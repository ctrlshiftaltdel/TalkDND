#!/usr/bin/env python3
"""Lesson records, evidence levels, state machine, dedup and report output (#49).

Frozen semantics: ``docs/self-improvement/lesson-lifecycle.md`` (content classes,
E0–E4, transitions T1–T12, fingerprint ``sha256-normalized-v1``, conflicts) and
``docs/contracts/schemas/lesson.schema.json``. This module implements them and
adds only what the contract leaves to the implementation:

* **affected_surface derivation.** The schema has no dedicated field for
  ``affected_surface``, so it is derived deterministically from
  ``applicability.applies_to`` (sorted, joined with ``|``). ``suggested_action_target``
  comes from ``suggested_action.target``. Both are documented in
  ``docs/self-improvement/lessons-implementation.md``.
* **dedup index.** ``LessonStore`` keeps three indexes: full fingerprint →
  lesson_id, ``(source_id, evidence_url)`` → lesson_id (so a corrected
  fingerprint cannot create a second lesson), and the lesson_id map itself.
* **candidates are not lessons.** Text distilled from a source cannot become a
  lesson by itself: :func:`candidate_from_evidence` returns a candidate that
  still needs an explicit analysis, and :func:`draft_lesson` refuses to invent a
  root cause. This keeps the pipeline from degrading into title matching, which
  lesson-lifecycle.md §9 forbids.

Only the Python standard library is used.
"""
from __future__ import annotations

import hashlib
import json
import re
import unicodedata
from datetime import datetime, timezone
from pathlib import Path

from . import untrusted

SCHEMA_VERSION = '1.0'

CONTENT_CLASSES = ('CONFIRMED_FACT', 'UNVERIFIED_ASSUMPTION',
                   'SOURCE_SPECIFIC_EXPERIENCE', 'GENERAL_LESSON')
STATUSES = ('OBSERVED', 'NEEDS_VERIFICATION', 'VERIFIED', 'REJECTED',
            'GENERALIZED', 'IMPLEMENTED', 'REGRESSION_WATCH')
EVIDENCE_LEVELS = ('E0_CLAIM', 'E1_SOURCE_ARTIFACT', 'E2_REPRODUCED',
                   'E3_FIXED_WITH_TEST', 'E4_REGRESSION_COVERED')
EVIDENCE_RANK = {level: rank for rank, level in enumerate(EVIDENCE_LEVELS)}
TARGETS = ('CONTRACT', 'CODE', 'TEST', 'DOCUMENTATION', 'PROCESS', 'METRIC')
SCALES = ('LOW', 'MEDIUM', 'HIGH')
EVIDENCE_KINDS = ('pull_request', 'issue', 'review', 'commit', 'ci_run', 'documentation')
CONFLICT_TYPES = ('CONTRADICTS', 'SUPERSEDES', 'OVERLAPS')
RESOLUTIONS = ('OPEN', 'RESOLVED_BY_EVIDENCE', 'RESOLVED_BY_OWNER')
DECISION_OUTCOMES = ('ADOPT', 'DEFER', 'REJECT')

FINGERPRINT_ALGORITHM = 'sha256-normalized-v1'
FINGERPRINT_FIELDS = ('root_cause', 'impact', 'affected_surface', 'suggested_action_target')
TRAILING_PUNCTUATION = '.,;:!?。，；：！？'
SURFACE_FIELDS = ('affected_surface', 'suggested_action_target')
_SURFACE_ALLOWED = re.compile(r'[^a-z0-9_./-]')
_WHITESPACE = re.compile(r'\s+')

LESSON_ID_RE = re.compile(r'^LSN-[A-Z0-9]{2,12}-[0-9]{3}$')
SHA1_RE = re.compile(r'^[a-f0-9]{40}$')
SHA256_RE = re.compile(r'^[a-f0-9]{64}$')
URL_RE = re.compile(r'^https?://[^\s<>`]+$')

# lesson-lifecycle.md §2.1 as data. Both tables are the semantic authority; the
# schema's allOf is their machine projection.
ALLOWED_EVIDENCE_LEVELS = {
    'CONFIRMED_FACT': ('E1_SOURCE_ARTIFACT', 'E2_REPRODUCED', 'E3_FIXED_WITH_TEST',
                       'E4_REGRESSION_COVERED'),
    'UNVERIFIED_ASSUMPTION': ('E0_CLAIM', 'E1_SOURCE_ARTIFACT'),
    'SOURCE_SPECIFIC_EXPERIENCE': ('E1_SOURCE_ARTIFACT', 'E2_REPRODUCED',
                                   'E3_FIXED_WITH_TEST', 'E4_REGRESSION_COVERED'),
    'GENERAL_LESSON': ('E2_REPRODUCED', 'E3_FIXED_WITH_TEST', 'E4_REGRESSION_COVERED'),
}
ALLOWED_STATUSES = {
    'CONFIRMED_FACT': ('OBSERVED', 'NEEDS_VERIFICATION', 'VERIFIED', 'REJECTED',
                       'REGRESSION_WATCH'),
    'UNVERIFIED_ASSUMPTION': ('OBSERVED', 'NEEDS_VERIFICATION', 'REJECTED'),
    'SOURCE_SPECIFIC_EXPERIENCE': ('OBSERVED', 'NEEDS_VERIFICATION', 'VERIFIED', 'REJECTED',
                                   'REGRESSION_WATCH'),
    'GENERAL_LESSON': ('VERIFIED', 'GENERALIZED', 'IMPLEMENTED', 'REJECTED',
                       'REGRESSION_WATCH'),
}
# States that claim a conclusion and therefore need reproducible evidence.
CONCLUDING_STATUSES = ('VERIFIED', 'GENERALIZED', 'IMPLEMENTED', 'REGRESSION_WATCH')
TASK_ELIGIBLE_STATUSES = ('GENERALIZED',)

TRANSITION_TABLE = {
    ('OBSERVED', 'NEEDS_VERIFICATION'): 'T2',
    ('OBSERVED', 'VERIFIED'): 'T3',
    ('OBSERVED', 'REJECTED'): 'T4',
    ('NEEDS_VERIFICATION', 'REJECTED'): 'T4',
    ('NEEDS_VERIFICATION', 'VERIFIED'): 'T5',
    ('VERIFIED', 'GENERALIZED'): 'T6',
    ('VERIFIED', 'REJECTED'): 'T7',
    ('GENERALIZED', 'IMPLEMENTED'): 'T8',
    ('IMPLEMENTED', 'REGRESSION_WATCH'): 'T9',
    ('REGRESSION_WATCH', 'NEEDS_VERIFICATION'): 'T10',
    ('GENERALIZED', 'VERIFIED'): 'T12',
}

# Explicit rejections from lesson-lifecycle.md §3.3, checked before the table so
# the error names the rule instead of only "not allowed".
ILLEGAL_PATTERNS = (
    ({'OBSERVED', 'NEEDS_VERIFICATION'}, {'GENERALIZED', 'IMPLEMENTED'},
     'skips verification and generalisation'),
    ({'REGRESSION_WATCH'}, {'VERIFIED', 'GENERALIZED', 'IMPLEMENTED'},
     'a regression watcher must go back through NEEDS_VERIFICATION'),
)


class LessonError(Exception):
    """A lesson or transition violates the frozen contract."""

    def __init__(self, message, errors=()):
        self.errors = list(errors)
        detail = ''
        if self.errors:
            detail = '; '.join(f"{e['path']}: {e['message']}" for e in self.errors[:8])
        super().__init__(f'{message}: {detail}' if detail else message)


class TransitionError(LessonError):
    """The requested state transition is not in the frozen transition table."""


def _err(errors, path, keyword, message):
    errors.append({'path': path, 'keyword': keyword, 'message': message})


def parse_timestamp(value):
    if not isinstance(value, str) or not value.strip():
        raise ValueError('not an RFC 3339 timestamp')
    text = value.strip()
    if text.endswith('Z'):
        text = text[:-1] + '+00:00'
    parsed = datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        raise ValueError('timestamp must carry a UTC offset')
    return parsed.astimezone(timezone.utc)


def format_timestamp(moment):
    return moment.astimezone(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')


def now_utc():
    return format_timestamp(datetime.now(timezone.utc))


# --------------------------------------------------------------------------- #
# Fingerprint: sha256-normalized-v1 (lesson-lifecycle.md §4.1)
# --------------------------------------------------------------------------- #

def normalize_text(field, text):
    """Normalise one fingerprint field exactly as §4.1 prescribes."""
    value = '' if text is None else str(text)
    value = unicodedata.normalize('NFKC', value).casefold()
    value = _WHITESPACE.sub(' ', value).strip()
    value = value.rstrip(TRAILING_PUNCTUATION).strip()
    if field in SURFACE_FIELDS:
        value = _SURFACE_ALLOWED.sub('', value)
        value = _WHITESPACE.sub(' ', value).strip()
    return value


def fingerprint_inputs(lesson):
    """The four field values the algorithm can consume, derived from the record."""
    observation = lesson.get('observation') or {}
    applicability = lesson.get('applicability') or {}
    suggested = lesson.get('suggested_action') or {}
    applies_to = applicability.get('applies_to') or []
    surface = '|'.join(sorted(str(item) for item in applies_to))
    return {
        'root_cause': observation.get('root_cause'),
        'impact': observation.get('impact'),
        'affected_surface': surface,
        'suggested_action_target': suggested.get('target'),
    }


def fingerprint_value(values, fields):
    """``sha256-normalized-v1`` digest for *values* limited to *fields*."""
    selected = sorted(fields)
    canonical = json.dumps(
        [[name, normalize_text(name, values.get(name))] for name in selected],
        ensure_ascii=False, separators=(',', ':'), sort_keys=False,
    ).encode('utf-8')
    return hashlib.sha256(canonical).hexdigest()


def compute_fingerprint(values, fields=FINGERPRINT_FIELDS):
    """Return the schema-shaped fingerprint object for *values*."""
    selected = list(fields)
    errors = []
    _validate_normalized_fields(selected, errors, '/fingerprint/normalized_fields')
    if errors:
        raise LessonError('invalid fingerprint fields', errors)
    return {'algorithm': FINGERPRINT_ALGORITHM,
            'value': fingerprint_value(values, selected),
            'normalized_fields': selected}


def _validate_normalized_fields(fields, errors, path):
    if not isinstance(fields, list) or len(fields) < 3:
        _err(errors, path, 'minItems', 'normalized_fields needs at least three field names')
        return
    if len(set(fields)) != len(fields):
        _err(errors, path, 'uniqueItems', 'normalized_fields must not repeat names')
    for index, field in enumerate(fields):
        if field not in FINGERPRINT_FIELDS:
            _err(errors, f'{path}/{index}', 'enum',
                 f'{field!r} is not one of {", ".join(FINGERPRINT_FIELDS)}')


# --------------------------------------------------------------------------- #
# Lesson validation (schema subset + the semantic checks the schema cannot express)
# --------------------------------------------------------------------------- #

def _string(value, errors, path, *, required=True, min_length=None, max_length=None,
            pattern=None, nullable=False):
    if value is None and nullable:
        return
    if not isinstance(value, str):
        _err(errors, path, 'type', 'expected a string')
        return
    if required and not value:
        _err(errors, path, 'minLength', 'must not be empty')
    if min_length is not None and value and len(value) < min_length:
        _err(errors, path, 'minLength', f'must be at least {min_length} characters')
    if max_length is not None and len(value) > max_length:
        _err(errors, path, 'maxLength', f'must be at most {max_length} characters')
    if pattern is not None and value and not pattern.match(value):
        _err(errors, path, 'pattern', f'{value!r} does not match {pattern.pattern}')
    if untrusted.contains_secret(value):
        _err(errors, path, 'privacy',
             'contains a secret-shaped value or a personal absolute path')


def _datetime(value, errors, path, *, nullable=False):
    if value is None and nullable:
        return
    if not isinstance(value, str):
        _err(errors, path, 'type', 'expected an RFC 3339 timestamp')
        return
    try:
        parse_timestamp(value)
    except ValueError as exc:
        _err(errors, path, 'format', f'not an RFC 3339 timestamp ({exc})')


def _enum(value, errors, path, allowed):
    if value not in allowed:
        _err(errors, path, 'enum', f'{value!r} is not one of {", ".join(allowed)}')


def validate_lesson(lesson):
    """Return a list of ``{path, keyword, message}`` errors. Empty means valid."""
    errors = []
    if not isinstance(lesson, dict):
        return [{'path': '$', 'keyword': 'type', 'message': 'a lesson must be a JSON object'}]
    required = ('schema_version', 'lesson_id', 'title', 'content_class', 'status',
                'evidence_level', 'source', 'observation', 'fingerprint', 'applicability',
                'suggested_action', 'created_at', 'updated_at')
    allowed = set(required) | {'duplicate_of', 'conflicts_with', 'links', 'decision'}
    for key in lesson:
        if key not in allowed:
            _err(errors, f'/{key}', 'additionalProperties', 'unknown lesson field')
    for key in required:
        if key not in lesson:
            _err(errors, f'/{key}', 'required', 'missing required lesson field')
    if lesson.get('schema_version') not in (None, SCHEMA_VERSION):
        _err(errors, '/schema_version', 'const', f'schema_version must be {SCHEMA_VERSION!r}')
    _string(lesson.get('lesson_id'), errors, '/lesson_id', pattern=LESSON_ID_RE)
    _string(lesson.get('title'), errors, '/title', min_length=8, max_length=160)
    content_class = lesson.get('content_class')
    status = lesson.get('status')
    level = lesson.get('evidence_level')
    if content_class is not None:
        _enum(content_class, errors, '/content_class', CONTENT_CLASSES)
    if status is not None:
        _enum(status, errors, '/status', STATUSES)
    if level is not None:
        _enum(level, errors, '/evidence_level', EVIDENCE_LEVELS)
    _datetime(lesson.get('created_at'), errors, '/created_at')
    _datetime(lesson.get('updated_at'), errors, '/updated_at')
    created = lesson.get('created_at')
    updated = lesson.get('updated_at')
    if isinstance(created, str) and isinstance(updated, str):
        try:
            if parse_timestamp(updated) < parse_timestamp(created):
                _err(errors, '/updated_at', 'timestamp_order',
                     'updated_at precedes created_at')
        except ValueError:
            pass
    if 'duplicate_of' in lesson:
        _string(lesson['duplicate_of'], errors, '/duplicate_of', nullable=True,
                pattern=LESSON_ID_RE)

    errors.extend(_validate_source_ref(lesson.get('source')))
    errors.extend(_validate_observation(lesson.get('observation')))
    errors.extend(_validate_fingerprint(lesson.get('fingerprint'), lesson))
    errors.extend(_validate_applicability(lesson.get('applicability')))
    errors.extend(_validate_suggested_action(lesson.get('suggested_action')))
    errors.extend(_validate_links(lesson.get('links')))
    errors.extend(_validate_decision(lesson.get('decision')))
    errors.extend(_validate_conflicts(lesson.get('conflicts_with')))

    # §2.1 class ↔ status ↔ evidence-level coherence.
    if content_class in ALLOWED_EVIDENCE_LEVELS and level in EVIDENCE_LEVELS:
        if level not in ALLOWED_EVIDENCE_LEVELS[content_class]:
            _err(errors, '/evidence_level', 'enum',
                 f'{content_class} allows only {", ".join(ALLOWED_EVIDENCE_LEVELS[content_class])}')
    if content_class in ALLOWED_STATUSES and status in STATUSES:
        if status not in ALLOWED_STATUSES[content_class]:
            _err(errors, '/status', 'enum',
                 f'{content_class} allows only {", ".join(ALLOWED_STATUSES[content_class])}')
    if status == 'GENERALIZED' or status == 'IMPLEMENTED':
        if content_class not in (None, 'GENERAL_LESSON'):
            _err(errors, '/content_class', 'const',
                 f'{status} requires content_class GENERAL_LESSON')
        if 'decision' not in lesson or lesson.get('decision') is None:
            _err(errors, '/decision', 'required', f'{status} requires a decision')
    if status == 'REJECTED' and not lesson.get('decision'):
        _err(errors, '/decision', 'required', 'REJECTED requires a decision')
    if status in CONCLUDING_STATUSES and level in EVIDENCE_LEVELS:
        if EVIDENCE_RANK[level] < EVIDENCE_RANK['E2_REPRODUCED']:
            _err(errors, '/evidence_level', 'enum',
                 f'{status} needs E2_REPRODUCED or stronger evidence')
    applicability = lesson.get('applicability')
    if isinstance(applicability, dict):
        scope = applicability.get('scope')
        if content_class == 'SOURCE_SPECIFIC_EXPERIENCE' and scope not in (None, 'source_only'):
            _err(errors, '/applicability/scope', 'const',
                 'SOURCE_SPECIFIC_EXPERIENCE requires applicability.scope source_only')
        if content_class == 'GENERAL_LESSON' and scope not in (None, 'agentorch_general'):
            _err(errors, '/applicability/scope', 'const',
                 'GENERAL_LESSON requires applicability.scope agentorch_general')
    if status in ('GENERALIZED', 'IMPLEMENTED'):
        applicability = lesson.get('applicability')
        if isinstance(applicability, dict) and applicability.get('scope') != 'agentorch_general':
            _err(errors, '/applicability/scope', 'const',
                 f'{status} requires applicability.scope agentorch_general')
        for conflict in lesson.get('conflicts_with') or []:
            if isinstance(conflict, dict) and conflict.get('resolution') == 'OPEN':
                _err(errors, '/conflicts_with', 'not',
                     f'{status} is blocked by an OPEN conflict with '
                     f'{conflict.get("lesson_id")!r}')
    if status == 'IMPLEMENTED':
        links = lesson.get('links') or {}
        if not (links.get('issues') or links.get('pull_requests')):
            _err(errors, '/links', 'required',
                 'IMPLEMENTED must reference the implementing Issue or PR')
    return errors


def _validate_source_ref(source):
    errors = []
    if not isinstance(source, dict):
        return [{'path': '/source', 'keyword': 'type', 'message': 'source must be an object'}]
    required = ('source_id', 'evidence_kind', 'evidence_urls', 'observed_at', 'captured_at')
    allowed = set(required) | {'source_commit'}
    for key in source:
        if key not in allowed:
            _err(errors, f'/source/{key}', 'additionalProperties', 'unknown source field')
    for key in required:
        if key not in source:
            _err(errors, f'/source/{key}', 'required', 'missing required source field')
    _string(source.get('source_id'), errors, '/source/source_id',
            pattern=re.compile(r'^[a-z][a-z0-9-]{1,31}$'))
    if 'evidence_kind' in source:
        _enum(source['evidence_kind'], errors, '/source/evidence_kind', EVIDENCE_KINDS)
    urls = source.get('evidence_urls')
    if not isinstance(urls, list) or not urls:
        _err(errors, '/source/evidence_urls', 'minItems', 'evidence_urls must not be empty')
    else:
        if len(set(urls)) != len(urls):
            _err(errors, '/source/evidence_urls', 'uniqueItems', 'evidence_urls repeat a value')
        for index, url in enumerate(urls):
            _string(url, errors, f'/source/evidence_urls/{index}', pattern=URL_RE)
    _datetime(source.get('observed_at'), errors, '/source/observed_at')
    _datetime(source.get('captured_at'), errors, '/source/captured_at')
    if 'source_commit' in source:
        commit = source['source_commit']
        if commit is not None and not (isinstance(commit, str) and SHA1_RE.match(commit)):
            _err(errors, '/source/source_commit', 'pattern',
                 'source_commit must be a 40-character lowercase hex SHA or null')
    return errors


def _validate_observation(observation):
    errors = []
    if not isinstance(observation, dict):
        return [{'path': '/observation', 'keyword': 'type',
                 'message': 'observation must be an object'}]
    required = ('observed_fact', 'repro_conditions', 'root_cause', 'impact', 'why_missed')
    for key in observation:
        if key not in required:
            _err(errors, f'/observation/{key}', 'additionalProperties', 'unknown observation field')
    for key in required:
        if key not in observation:
            _err(errors, f'/observation/{key}', 'required', 'missing required observation field')
    _string(observation.get('observed_fact'), errors, '/observation/observed_fact',
            min_length=16, max_length=2000)
    for key in ('repro_conditions', 'root_cause', 'impact', 'why_missed'):
        _string(observation.get(key), errors, f'/observation/{key}',
                min_length=8, max_length=2000)
    return errors


def _validate_fingerprint(fingerprint, lesson):
    errors = []
    if not isinstance(fingerprint, dict):
        return [{'path': '/fingerprint', 'keyword': 'type',
                 'message': 'fingerprint must be an object'}]
    required = ('algorithm', 'value', 'normalized_fields')
    for key in fingerprint:
        if key not in required:
            _err(errors, f'/fingerprint/{key}', 'additionalProperties', 'unknown fingerprint field')
    for key in required:
        if key not in fingerprint:
            _err(errors, f'/fingerprint/{key}', 'required', 'missing required fingerprint field')
    if fingerprint.get('algorithm') != FINGERPRINT_ALGORITHM:
        _err(errors, '/fingerprint/algorithm', 'const',
             f'algorithm must be {FINGERPRINT_ALGORITHM!r}')
    value = fingerprint.get('value')
    if not (isinstance(value, str) and SHA256_RE.match(value)):
        _err(errors, '/fingerprint/value', 'pattern', 'value must be 64 lowercase hex characters')
    fields = fingerprint.get('normalized_fields')
    _validate_normalized_fields(fields, errors, '/fingerprint/normalized_fields')
    if not errors and isinstance(lesson, dict):
        try:
            expected = fingerprint_value(fingerprint_inputs(lesson), list(fields))
        except Exception:  # pragma: no cover - defensive, inputs are simple values
            expected = None
        if expected is not None and value != expected:
            _err(errors, '/fingerprint/value', 'const',
                 'value does not match sha256-normalized-v1 over the declared fields '
                 '(recompute it instead of hand-editing)')
    return errors


def _validate_applicability(applicability):
    errors = []
    if not isinstance(applicability, dict):
        return [{'path': '/applicability', 'keyword': 'type',
                 'message': 'applicability must be an object'}]
    required = ('scope', 'applies_to', 'excluded_contexts')
    for key in applicability:
        if key not in required:
            _err(errors, f'/applicability/{key}', 'additionalProperties', 'unknown applicability field')
    for key in required:
        if key not in applicability:
            _err(errors, f'/applicability/{key}', 'required', 'missing required applicability field')
    scope = applicability.get('scope')
    if scope not in (None, 'source_only', 'agentorch_general'):
        _err(errors, '/applicability/scope', 'enum', 'scope must be source_only or agentorch_general')
    applies_to = applicability.get('applies_to')
    if not isinstance(applies_to, list) or not applies_to:
        _err(errors, '/applicability/applies_to', 'minItems', 'applies_to must not be empty')
    else:
        for index, item in enumerate(applies_to):
            _string(item, errors, f'/applicability/applies_to/{index}')
    excluded = applicability.get('excluded_contexts')
    if not isinstance(excluded, list):
        _err(errors, '/applicability/excluded_contexts', 'type',
             'excluded_contexts must be an array')
    else:
        for index, item in enumerate(excluded):
            _string(item, errors, f'/applicability/excluded_contexts/{index}')
    return errors


def _validate_suggested_action(action):
    errors = []
    if not isinstance(action, dict):
        return [{'path': '/suggested_action', 'keyword': 'type',
                 'message': 'suggested_action must be an object'}]
    required = ('target', 'summary', 'cost', 'risk')
    allowed = set(required) | {'validation_command'}
    for key in action:
        if key not in allowed:
            _err(errors, f'/suggested_action/{key}', 'additionalProperties',
                 'unknown suggested_action field')
    for key in required:
        if key not in action:
            _err(errors, f'/suggested_action/{key}', 'required',
                 'missing required suggested_action field')
    if 'target' in action:
        _enum(action['target'], errors, '/suggested_action/target', TARGETS)
    _string(action.get('summary'), errors, '/suggested_action/summary',
            min_length=8, max_length=1000)
    for key in ('cost', 'risk'):
        if key in action:
            _enum(action[key], errors, f'/suggested_action/{key}', SCALES)
    if 'validation_command' in action:
        _string(action['validation_command'], errors, '/suggested_action/validation_command',
                nullable=True)
    return errors


def _validate_links(links):
    errors = []
    if links is None:
        return errors
    if not isinstance(links, dict):
        return [{'path': '/links', 'keyword': 'type', 'message': 'links must be an object'}]
    for key in links:
        if key not in ('issues', 'pull_requests', 'modules'):
            _err(errors, f'/links/{key}', 'additionalProperties', 'unknown links field')
    for key in ('issues', 'pull_requests'):
        if key in links:
            value = links[key]
            if not isinstance(value, list):
                _err(errors, f'/links/{key}', 'type', f'{key} must be an array of integers')
                continue
            for index, item in enumerate(value):
                if isinstance(item, bool) or not isinstance(item, int) or item < 1:
                    _err(errors, f'/links/{key}/{index}', 'minimum',
                         'expected an integer >= 1')
    if 'modules' in links:
        value = links['modules']
        if not isinstance(value, list):
            _err(errors, '/links/modules', 'type', 'modules must be an array of strings')
        else:
            for index, item in enumerate(value):
                _string(item, errors, f'/links/modules/{index}')
    return errors


def _validate_decision(decision):
    errors = []
    if decision is None:
        return errors
    if not isinstance(decision, dict):
        return [{'path': '/decision', 'keyword': 'type',
                 'message': 'decision must be an object or null'}]
    required = ('decided_by', 'decided_at', 'outcome', 'rationale')
    for key in decision:
        if key not in required:
            _err(errors, f'/decision/{key}', 'additionalProperties', 'unknown decision field')
    for key in required:
        if key not in decision:
            _err(errors, f'/decision/{key}', 'required', 'missing required decision field')
    _string(decision.get('decided_by'), errors, '/decision/decided_by')
    _datetime(decision.get('decided_at'), errors, '/decision/decided_at')
    if 'outcome' in decision:
        _enum(decision['outcome'], errors, '/decision/outcome', DECISION_OUTCOMES)
    _string(decision.get('rationale'), errors, '/decision/rationale',
            min_length=8, max_length=2000)
    return errors


def _validate_conflicts(conflicts):
    errors = []
    if conflicts is None:
        return errors
    if not isinstance(conflicts, list):
        return [{'path': '/conflicts_with', 'keyword': 'type',
                 'message': 'conflicts_with must be an array'}]
    seen = set()
    for index, conflict in enumerate(conflicts):
        path = f'/conflicts_with/{index}'
        if not isinstance(conflict, dict):
            _err(errors, path, 'type', 'a conflict must be an object')
            continue
        required = ('lesson_id', 'conflict_type', 'resolution')
        for key in conflict:
            if key not in required + ('resolution_evidence',):
                _err(errors, f'{path}/{key}', 'additionalProperties', 'unknown conflict field')
        for key in required:
            if key not in conflict:
                _err(errors, f'{path}/{key}', 'required', 'missing required conflict field')
        _string(conflict.get('lesson_id'), errors, f'{path}/lesson_id', pattern=LESSON_ID_RE)
        if 'conflict_type' in conflict:
            _enum(conflict['conflict_type'], errors, f'{path}/conflict_type', CONFLICT_TYPES)
        resolution = conflict.get('resolution')
        if resolution is not None:
            _enum(resolution, errors, f'{path}/resolution', RESOLUTIONS)
        if resolution in ('RESOLVED_BY_EVIDENCE', 'RESOLVED_BY_OWNER'):
            evidence = conflict.get('resolution_evidence')
            if not isinstance(evidence, str) or not evidence:
                _err(errors, f'{path}/resolution_evidence', 'required',
                     f'{resolution} requires locatable resolution evidence')
        if 'resolution_evidence' in conflict:
            _string(conflict['resolution_evidence'], errors,
                    f'{path}/resolution_evidence', nullable=True, max_length=1000)
        key = (conflict.get('lesson_id'), conflict.get('conflict_type'))
        if key in seen:
            _err(errors, path, 'uniqueItems', 'the same conflict is listed twice')
        seen.add(key)
    return errors


def _assert_valid(lesson, context='lesson'):
    errors = validate_lesson(lesson)
    if errors:
        raise LessonError(f'invalid {context}', errors)


# --------------------------------------------------------------------------- #
# Construction and state machine
# --------------------------------------------------------------------------- #

def make_decision(decided_by, outcome, rationale, decided_at=None):
    if outcome not in DECISION_OUTCOMES:
        raise LessonError(f'unknown decision outcome {outcome!r}')
    return {'decided_by': decided_by,
            'decided_at': decided_at or now_utc(),
            'outcome': outcome,
            'rationale': rationale}


def make_conflict(lesson_id, conflict_type='OVERLAPS', resolution='OPEN',
                  resolution_evidence=None):
    conflict = {'lesson_id': lesson_id, 'conflict_type': conflict_type,
                'resolution': resolution}
    if resolution_evidence is not None:
        conflict['resolution_evidence'] = resolution_evidence
    return conflict


def build_lesson(*, lesson_id, title, content_class, evidence_level, source, observation,
                 applicability, suggested_action, fingerprint_fields=FINGERPRINT_FIELDS,
                 status='OBSERVED', duplicate_of=None, conflicts_with=None, links=None,
                 decision=None, created_at=None, updated_at=None):
    """Assemble a record and compute its fingerprint (T1's guard).

    Raises :class:`LessonError` when the record or its fingerprint fields are
    invalid. T1's guard — non-empty ``evidence_urls`` and a computed
    ``fingerprint`` — is enforced here rather than by an optional caller.
    """
    timestamp = created_at or now_utc()
    lesson = {
        'schema_version': SCHEMA_VERSION,
        'lesson_id': lesson_id,
        'title': title,
        'content_class': content_class,
        'status': status,
        'evidence_level': evidence_level,
        'source': source,
        'observation': observation,
        'fingerprint': {'algorithm': FINGERPRINT_ALGORITHM, 'value': '', 'normalized_fields': list(fingerprint_fields)},
        'duplicate_of': duplicate_of,
        'conflicts_with': list(conflicts_with or []),
        'applicability': applicability,
        'suggested_action': suggested_action,
        'links': links or {},
        'decision': decision,
        'created_at': timestamp,
        'updated_at': updated_at or timestamp,
    }
    lesson['fingerprint'] = compute_fingerprint(fingerprint_inputs(lesson), fingerprint_fields)
    _assert_valid(lesson, 'new lesson')
    return lesson


def allowed_transitions(lesson):
    """Legal target statuses for *lesson*, as a sorted list of transition ids."""
    status = lesson.get('status')
    return sorted(f'{tid}->{to}' for (frm, to), tid in TRANSITION_TABLE.items() if frm == status)


def _is_illegal(status, to_status, content_class, conflicts, scan_status):
    reasons = []
    for sources_set, targets_set, reason in ILLEGAL_PATTERNS:
        if status in sources_set and to_status in targets_set:
            reasons.append(reason)
    if content_class == 'UNVERIFIED_ASSUMPTION' and to_status in (
            'VERIFIED', 'GENERALIZED', 'IMPLEMENTED'):
        reasons.append('UNVERIFIED_ASSUMPTION cannot be verified or adopted')
    if content_class == 'SOURCE_SPECIFIC_EXPERIENCE' and to_status in ('GENERALIZED', 'IMPLEMENTED'):
        reasons.append('SOURCE_SPECIFIC_EXPERIENCE must be reclassified to GENERAL_LESSON '
                       'with a recorded decision first')
    if status == 'REJECTED':
        reasons.append('REJECTED is terminal; re-evaluate with a new lesson that references it')
    if to_status in ('GENERALIZED', 'IMPLEMENTED'):
        for conflict in conflicts or []:
            if isinstance(conflict, dict) and conflict.get('resolution') == 'OPEN':
                reasons.append(f'OPEN conflict with {conflict.get("lesson_id")!r} blocks {to_status}')
    if scan_status in ('NOT_RUN', 'UNKNOWN', 'BLOCKED', 'PARTIAL') and to_status in CONCLUDING_STATUSES:
        reasons.append(f'evidence state {scan_status} cannot be treated as verified')
    return reasons


def transition(lesson, to_status, *, decision=None, links=None, new_evidence_urls=(),
               observation_window=None, content_class=None, now=None, scan_status=None):
    """Apply one transition from the frozen table, or refuse it.

    ``decision``, ``links`` and ``new_evidence_urls`` are the guards' inputs.
    ``content_class`` is only accepted for T12 (return a GENERAL_LESSON to
    CONFIRMED_FACT / SOURCE_SPECIFIC_EXPERIENCE). Evidence that the last scan is
    ``NOT_RUN`` / ``UNKNOWN`` / ``BLOCKED`` / ``PARTIAL`` also blocks any
    concluding transition: a gap in collection is not a conclusion.
    """
    _assert_valid(lesson, 'current lesson')
    if to_status not in STATUSES:
        raise TransitionError(f'unknown status {to_status!r}')
    status = lesson['status']
    if status == to_status:
        raise TransitionError(f'lesson is already {status}')
    illegal = _is_illegal(status, to_status, lesson['content_class'],
                          lesson.get('conflicts_with'), scan_status)
    if illegal:
        raise TransitionError(f'illegal transition {status} -> {to_status}: ' + '; '.join(illegal))

    entry = TRANSITION_TABLE.get((status, to_status))
    if entry is None:
        raise TransitionError(
            f'no transition {status} -> {to_status} in the frozen table; '
            f'allowed: {allowed_transitions(lesson) or ["(terminal)"]}')

    updated = json.loads(json.dumps(lesson))
    if entry in ('T3', 'T5'):
        if EVIDENCE_RANK[lesson['evidence_level']] < EVIDENCE_RANK['E2_REPRODUCED']:
            raise TransitionError(f'{entry} needs E2_REPRODUCED or stronger evidence; '
                                  f'record has {lesson["evidence_level"]}')
        if lesson['content_class'] not in ('CONFIRMED_FACT', 'SOURCE_SPECIFIC_EXPERIENCE'):
            raise TransitionError(f'{entry} requires CONFIRMED_FACT or '
                                  'SOURCE_SPECIFIC_EXPERIENCE')
    if entry == 'T6':
        if lesson['content_class'] != 'GENERAL_LESSON':
            raise TransitionError('T6 requires content_class GENERAL_LESSON; reclassify first')
        if not decision or decision.get('outcome') != 'ADOPT':
            raise TransitionError('T6 requires a decision with outcome ADOPT')
        if (lesson.get('applicability') or {}).get('scope') != 'agentorch_general':
            raise TransitionError('T6 requires applicability.scope agentorch_general')
    if entry in ('T4', 'T6', 'T7', 'T8', 'T12') and not decision:
        raise TransitionError(f'{entry} requires a recorded decision')
    if entry == 'T4' and decision.get('outcome') != 'REJECT':
        raise TransitionError('T4 requires decision.outcome REJECT')
    if entry == 'T8':
        links = links or lesson.get('links') or {}
        if not (links.get('issues') or links.get('pull_requests')):
            raise TransitionError('T8 requires the implementing Issue or PR number in links')
    if entry == 'T9':
        if not observation_window:
            raise TransitionError('T9 requires a named observation window / check '
                                  '(observation_window)')
    if entry == 'T10':
        if not new_evidence_urls:
            raise TransitionError('T10 requires at least one new evidence URL')
    if entry == 'T12':
        if content_class not in ('CONFIRMED_FACT', 'SOURCE_SPECIFIC_EXPERIENCE'):
            raise TransitionError('T12 needs content_class CONFIRMED_FACT or '
                                  'SOURCE_SPECIFIC_EXPERIENCE')
    elif content_class is not None:
        raise TransitionError(f'content_class may only change through T12 or reclassify(), '
                              f'not through {entry}')

    if decision is not None:
        updated['decision'] = decision
    if links is not None:
        updated['links'] = links
    if new_evidence_urls:
        merged = list(dict.fromkeys(list(updated['source']['evidence_urls']) + list(new_evidence_urls)))
        updated['source']['evidence_urls'] = merged
        updated['source']['captured_at'] = now or now_utc()
    if content_class is not None:
        updated['content_class'] = content_class
        # T12 walks a general lesson back: it is no longer general, so its scope
        # cannot stay agentorch_general.
        updated['applicability']['scope'] = 'source_only'
    if observation_window:
        updated['links'] = dict(updated.get('links') or {})
        modules = list(updated['links'].get('modules') or [])
        marker = f'window:{observation_window}'
        if marker not in modules:
            modules.append(marker)
        updated['links']['modules'] = modules
    updated['status'] = to_status
    updated['updated_at'] = now or now_utc()
    _assert_valid(updated, f'transition {entry}')
    return updated


def reclassify(lesson, to_class, *, decision, scope=None, now=None):
    """Explicit content-class change (§2.1 rule 3), never a silent field edit.

    Promotion to ``GENERAL_LESSON`` needs E2+, an ADOPT decision, no OPEN
    conflict, and forces ``scope=agentorch_general``. Demotion to
    ``SOURCE_SPECIFIC_EXPERIENCE`` forces ``scope=source_only``.
    """
    _assert_valid(lesson, 'current lesson')
    if to_class not in CONTENT_CLASSES:
        raise LessonError(f'unknown content class {to_class!r}')
    if to_class == lesson['content_class']:
        raise LessonError(f'lesson is already {to_class}')
    if not decision:
        raise LessonError('reclassify requires a recorded decision')
    if to_class == 'GENERAL_LESSON':
        if EVIDENCE_RANK[lesson['evidence_level']] < EVIDENCE_RANK['E2_REPRODUCED']:
            raise LessonError('promoting to GENERAL_LESSON needs E2_REPRODUCED or stronger')
        if decision.get('outcome') != 'ADOPT':
            raise LessonError('promoting to GENERAL_LESSON requires decision.outcome ADOPT')
        open_conflicts = [c for c in lesson.get('conflicts_with') or []
                          if c.get('resolution') == 'OPEN']
        if open_conflicts:
            raise LessonError('promoting to GENERAL_LESSON is blocked by an OPEN conflict: '
                              + ', '.join(c.get('lesson_id', '?') for c in open_conflicts))
        # A pre-GENERAL_LESSON record may still be OBSERVED/NEEDS_VERIFICATION, which
        # the class table does not allow; a promotion is only meaningful once verified.
        if lesson['status'] not in ('VERIFIED', 'GENERALIZED'):
            raise LessonError('promoting to GENERAL_LESSON requires status VERIFIED first')
    updated = json.loads(json.dumps(lesson))
    updated['content_class'] = to_class
    updated['decision'] = decision
    updated['updated_at'] = now or now_utc()
    if scope is not None:
        updated['applicability']['scope'] = scope
    elif to_class == 'GENERAL_LESSON':
        updated['applicability']['scope'] = 'agentorch_general'
    elif to_class == 'SOURCE_SPECIFIC_EXPERIENCE':
        updated['applicability']['scope'] = 'source_only'
    _assert_valid(updated, 'reclassified lesson')
    return updated


def demote(lesson, *, reason, decided_by, now=None, to_status='NEEDS_VERIFICATION',
           evidence_level='E0_CLAIM'):
    """T11: evidence withdrawn → back to UNVERIFIED_ASSUMPTION with E0/E1."""
    _assert_valid(lesson, 'current lesson')
    if evidence_level not in ('E0_CLAIM', 'E1_SOURCE_ARTIFACT'):
        raise LessonError('T11 may only demote to E0_CLAIM or E1_SOURCE_ARTIFACT')
    if to_status not in ('OBSERVED', 'NEEDS_VERIFICATION'):
        raise LessonError('T11 must land in OBSERVED or NEEDS_VERIFICATION')
    if not reason:
        raise LessonError('T11 requires a reason')
    updated = json.loads(json.dumps(lesson))
    updated['content_class'] = 'UNVERIFIED_ASSUMPTION'
    updated['evidence_level'] = evidence_level
    updated['status'] = to_status
    updated['decision'] = make_decision(decided_by, 'DEFER', reason,
                                        decided_at=now or now_utc())
    updated['updated_at'] = now or now_utc()
    if (updated.get('applicability') or {}).get('scope') == 'agentorch_general':
        updated['applicability']['scope'] = 'source_only'
    _assert_valid(updated, 'demoted lesson')
    return updated


def add_conflict(lesson, conflict):
    """Attach a conflict; unresolved by default (no side is chosen automatically)."""
    _assert_valid(lesson, 'current lesson')
    errors = _validate_conflicts([conflict])
    if errors:
        raise LessonError('invalid conflict', errors)
    if lesson.get('status') in ('GENERALIZED', 'IMPLEMENTED') and \
            conflict.get('resolution') == 'OPEN':
        raise LessonError(
            'an OPEN conflict cannot be added to a GENERALIZED/IMPLEMENTED lesson '
            '(lesson.schema.json forbids it); re-open verification first (T12 or demote)')
    updated = json.loads(json.dumps(lesson))
    conflicts = list(updated.get('conflicts_with') or [])
    if any(c.get('lesson_id') == conflict['lesson_id']
           and c.get('conflict_type') == conflict['conflict_type'] for c in conflicts):
        raise LessonError(f'conflict with {conflict["lesson_id"]} already recorded')
    updated['conflicts_with'] = conflicts + [conflict]
    updated['updated_at'] = now_utc()
    _assert_valid(updated, 'lesson with conflict')
    return updated


def resolve_conflict(lesson, other_lesson_id, resolution, evidence):
    """Resolve an OPEN conflict only with locatable evidence or an owner decision."""
    _assert_valid(lesson, 'current lesson')
    if resolution not in ('RESOLVED_BY_EVIDENCE', 'RESOLVED_BY_OWNER'):
        raise LessonError('resolution must be RESOLVED_BY_EVIDENCE or RESOLVED_BY_OWNER')
    if not evidence:
        raise LessonError('resolving a conflict requires locatable evidence')
    updated = json.loads(json.dumps(lesson))
    found = False
    conflicts = []
    for conflict in updated.get('conflicts_with') or []:
        if conflict.get('lesson_id') == other_lesson_id:
            found = True
            changed = dict(conflict)
            changed['resolution'] = resolution
            changed['resolution_evidence'] = evidence
            conflicts.append(changed)
        else:
            conflicts.append(conflict)
    if not found:
        raise LessonError(f'no conflict with {other_lesson_id!r} recorded')
    updated['conflicts_with'] = conflicts
    updated['updated_at'] = now_utc()
    _assert_valid(updated, 'lesson with resolved conflict')
    return updated


def open_conflicts(lesson):
    return [c for c in lesson.get('conflicts_with') or [] if c.get('resolution') == 'OPEN']


def can_generate_task(lesson):
    """True only for GENERAL_LESSON + GENERALIZED + no OPEN conflict (§7)."""
    if lesson.get('content_class') != 'GENERAL_LESSON':
        return False, f'content_class is {lesson.get("content_class")}, not GENERAL_LESSON'
    if lesson.get('status') not in TASK_ELIGIBLE_STATUSES:
        return False, f'status is {lesson.get("status")}, not GENERALIZED'
    conflicts = open_conflicts(lesson)
    if conflicts:
        return False, ('blocked by OPEN conflict with '
                       + ', '.join(c.get('lesson_id', '?') for c in conflicts))
    if lesson.get('duplicate_of'):
        return False, f'record is a duplicate of {lesson["duplicate_of"]}'
    return True, 'GENERAL_LESSON + GENERALIZED with no OPEN conflict'


# --------------------------------------------------------------------------- #
# Candidates, drafting and dedup
# --------------------------------------------------------------------------- #

def candidate_from_evidence(evidence):
    """A candidate pointer, *not* a lesson: text alone cannot fix a root cause."""
    return {
        'kind': 'CANDIDATE',
        'source_id': evidence.get('source_id'),
        'evidence_kind': evidence.get('evidence_kind'),
        'evidence_url': evidence.get('url'),
        'title': evidence.get('title'),
        'observed_at': evidence.get('observed_at'),
        'captured_at': evidence.get('captured_at'),
        'suggested_evidence_level': evidence.get('suggested_evidence_level'),
        'instruction_flags': list(evidence.get('instruction_flags') or []),
        'requires_analysis': True,
        'blocking_reasons': [
            'no root cause / impact / why-missed analysis yet; a title or a link '
            'alone is E0–E1 and cannot become a lesson',
        ],
    }


ANALYSIS_FIELDS = ('title', 'content_class', 'evidence_level', 'observation',
                   'applicability', 'suggested_action')


def draft_lesson(candidate, analysis, *, lesson_id=None, existing_ids=(), fingerprint_fields=FINGERPRINT_FIELDS,
                 status='OBSERVED', conflicts_with=None, links=None, decision=None, now=None):
    """Turn one candidate + explicit analysis into a lesson record.

    ``analysis`` must supply title, content_class, evidence_level, observation,
    applicability and suggested_action. Nothing is inferred from the source text:
    a missing root cause is a missing field, not a placeholder that would make
    unrelated findings collide on one fingerprint.
    """
    missing = [field for field in ANALYSIS_FIELDS if field not in analysis]
    if missing:
        raise LessonError('analysis is incomplete; a lesson is not created from source '
                          'text alone', [{'path': f'/analysis/{field}', 'keyword': 'required',
                                          'message': 'missing analysis field'} for field in missing])
    timestamp = now or now_utc()
    source_id = candidate['source_id']
    source = {
        'source_id': source_id,
        'evidence_kind': candidate['evidence_kind'],
        'evidence_urls': [candidate['evidence_url']],
        'observed_at': candidate.get('observed_at') or timestamp,
        'captured_at': candidate.get('captured_at') or timestamp,
        'source_commit': candidate.get('source_commit'),
    }
    if source['source_commit'] is None:
        source.pop('source_commit')
    return build_lesson(
        lesson_id=lesson_id or next_lesson_id(source_id, existing_ids),
        title=analysis['title'],
        content_class=analysis['content_class'],
        evidence_level=analysis['evidence_level'],
        source=source,
        observation=analysis['observation'],
        applicability=analysis['applicability'],
        suggested_action=analysis['suggested_action'],
        fingerprint_fields=fingerprint_fields,
        status=status,
        conflicts_with=conflicts_with,
        links=links,
        decision=decision,
        created_at=timestamp,
        updated_at=timestamp,
    )


def next_lesson_id(source_id, existing_ids):
    """``LSN-<SOURCE>-NNN``; never reuses a number, even after removal."""
    prefix = 'LSN-' + re.sub(r'[^A-Z0-9]', '', source_id.upper())[:12]
    highest = 0
    for lesson_id in existing_ids:
        match = re.match(rf'^{re.escape(prefix)}-([0-9]+)$', lesson_id)
        if match:
            highest = max(highest, int(match.group(1)))
    return f'{prefix}-{highest + 1:03d}'


class IngestResult:
    """Outcome of one candidate hitting the store."""

    def __init__(self, kind, lesson_id, lesson, duplicate_of=None):
        self.kind = kind              # created | duplicate | idempotent | fingerprint_updated
        self.lesson_id = lesson_id
        self.lesson = lesson
        self.duplicate_of = duplicate_of

    def as_dict(self):
        return {'kind': self.kind, 'lesson_id': self.lesson_id,
                'duplicate_of': self.duplicate_of,
                'evidence_urls': list(self.lesson['source']['evidence_urls'])}

    def __repr__(self):  # pragma: no cover - debugging aid
        return f'IngestResult({self.kind!r}, {self.lesson_id!r})'


class LessonStore:
    """In-memory lesson records plus the dedup indexes from §4.2–§4.3."""

    def __init__(self, lessons=()):
        self._lessons = {}
        self._fingerprints = {}
        self._urls = {}
        for lesson in lessons:
            self.add(lesson)

    @classmethod
    def load(cls, path):
        path = Path(path)
        if not path.is_file():
            return cls()
        data = json.loads(path.read_text(encoding='utf-8'))
        if isinstance(data, dict):
            lessons = data.get('lessons', [])
        else:
            lessons = data
        return cls(lessons)

    def save(self, path):
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {'schema_version': SCHEMA_VERSION, 'lessons': self.all()}
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n',
                        encoding='utf-8')
        return path

    def __len__(self):
        return len(self._lessons)

    def all(self):
        return [self._lessons[key] for key in sorted(self._lessons)]

    def get(self, lesson_id):
        return self._lessons.get(lesson_id)

    def by_fingerprint(self, value):
        lesson_id = self._fingerprints.get(value)
        return self._lessons.get(lesson_id) if lesson_id else None

    def add(self, lesson):
        _assert_valid(lesson, 'lesson')
        lesson_id = lesson['lesson_id']
        if lesson_id in self._lessons:
            raise LessonError(f'lesson_id {lesson_id!r} already exists (IDs are never reused)')
        fingerprint = lesson['fingerprint']['value']
        owner = self._fingerprints.get(fingerprint)
        if owner and owner != lesson_id:
            raise LessonError(f'fingerprint {fingerprint[:12]}… already belongs to {owner!r}; '
                              'one fingerprint is one lesson (lesson-lifecycle.md §4.2)')
        for url in lesson['source']['evidence_urls']:
            key = (lesson['source']['source_id'], url)
            if key in self._urls:
                raise LessonError(f'evidence {url!r} is already attached to '
                                  f'{self._urls[key]!r}; use ingest() to merge')
        self._lessons[lesson_id] = lesson
        self._fingerprints[lesson['fingerprint']['value']] = lesson_id
        for url in lesson['source']['evidence_urls']:
            self._urls[(lesson['source']['source_id'], url)] = lesson_id
        return lesson_id

    def ingest(self, candidate_lesson, *, now=None):
        """Insert or merge one already-validated lesson record.

        Rules (lesson-lifecycle.md §4.2–§4.3):

        * same ``(source_id, evidence_url, fingerprint)`` → idempotent no-op;
        * same fingerprint, new evidence → merge into the earliest lesson; the
          result is reported as ``duplicate`` and never yields a second record;
        * same evidence URL, corrected fingerprint → update in place and keep the
          old fingerprint as an alias (a fingerprint fix is not a new lesson);
        * anything else → create, with a fresh ``lesson_id`` when none was given.
        """
        _assert_valid(candidate_lesson, 'candidate lesson')
        timestamp = now or now_utc()
        source_id = candidate_lesson['source']['source_id']
        urls = candidate_lesson['source']['evidence_urls']
        fingerprint = candidate_lesson['fingerprint']['value']

        existing_id = self._fingerprints.get(fingerprint)
        if existing_id:
            existing = self._lessons[existing_id]
            if any((source_id, url) in self._urls for url in urls):
                return IngestResult('idempotent', existing_id, existing)
            merged = json.loads(json.dumps(existing))
            merged['source']['evidence_urls'] = list(dict.fromkeys(
                existing['source']['evidence_urls'] + urls))
            merged['source']['captured_at'] = timestamp
            merged['updated_at'] = timestamp
            _assert_valid(merged, 'merged lesson')
            self._lessons[existing_id] = merged
            for url in urls:
                self._urls[(source_id, url)] = existing_id
            return IngestResult('duplicate', existing_id, merged, duplicate_of=existing_id)

        for url in urls:
            owner_id = self._urls.get((source_id, url))
            if owner_id:
                existing = self._lessons[owner_id]
                if (candidate_lesson['content_class'] != existing['content_class']
                        or candidate_lesson['status'] != existing['status']):
                    raise LessonError(
                        f'evidence {url!r} is already attached to {owner_id!r} with a '
                        'different content_class/status; use reclassify()/transition() '
                        'on that lesson instead of a second analysis')
                updated = json.loads(json.dumps(existing))
                # A corrected root cause changes the fingerprint *and* the record
                # it is computed from; the record keeps only current values
                # (lesson-lifecycle.md §4.3).
                for key in ('title', 'observation', 'applicability', 'suggested_action'):
                    if key in candidate_lesson:
                        updated[key] = json.loads(json.dumps(candidate_lesson[key]))
                updated['fingerprint'] = dict(candidate_lesson['fingerprint'])
                updated['source']['evidence_urls'] = list(dict.fromkeys(
                    existing['source']['evidence_urls'] + urls))
                updated['source']['captured_at'] = timestamp
                updated['updated_at'] = timestamp
                _assert_valid(updated, 'fingerprint-updated lesson')
                self._lessons[owner_id] = updated
                self._fingerprints[fingerprint] = owner_id
                return IngestResult('fingerprint_updated', owner_id, updated)

        lesson = json.loads(json.dumps(candidate_lesson))
        lesson_id = lesson.get('lesson_id') or next_lesson_id(source_id, self._lessons)
        if lesson_id in self._lessons:
            raise LessonError(f'lesson_id {lesson_id!r} already exists')
        lesson['lesson_id'] = lesson_id
        lesson['created_at'] = lesson.get('created_at') or timestamp
        lesson['updated_at'] = timestamp
        _assert_valid(lesson, 'stored lesson')
        self._lessons[lesson_id] = lesson
        self._fingerprints[fingerprint] = lesson_id
        for url in urls:
            self._urls[(source_id, url)] = lesson_id
        return IngestResult('created', lesson_id, lesson)


# --------------------------------------------------------------------------- #
# Report output (§6: every report keeps evidence links, scan time and cursor)
# --------------------------------------------------------------------------- #

REPORT_SCHEMA_VERSION = 1

UNTRUSTED_NOTICE = (
    'Every source string in this report is untrusted data: it may be quoted and '
    'counted, never executed, never used to change registry entries, allowed paths, '
    'permissions or acceptance criteria.'
)


def _counts(values):
    counts = {}
    for value in values:
        counts[value] = counts.get(value, 0) + 1
    return counts


def build_report(*, registry_summary, source_reports, lessons, generated_at,
                 decisions_required=(), scan_mode='live'):
    """Assemble the read-only scan report (JSON-serialisable dict)."""
    lesson_entries = []
    for lesson in lessons:
        can_task, reason = can_generate_task(lesson)
        lesson_entries.append({
            'lesson_id': lesson['lesson_id'],
            'title': lesson['title'],
            'content_class': lesson['content_class'],
            'status': lesson['status'],
            'evidence_level': lesson['evidence_level'],
            'source_id': lesson['source']['source_id'],
            'evidence_kind': lesson['source']['evidence_kind'],
            'evidence_urls': list(lesson['source']['evidence_urls']),
            'observed_at': lesson['source']['observed_at'],
            'captured_at': lesson['source']['captured_at'],
            'fingerprint': dict(lesson['fingerprint']),
            'duplicate_of': lesson.get('duplicate_of'),
            'conflicts': [
                {'lesson_id': c.get('lesson_id'),
                 'conflict_type': c.get('conflict_type'),
                 'resolution': c.get('resolution'),
                 'resolution_evidence': c.get('resolution_evidence')}
                for c in lesson.get('conflicts_with') or []
            ],
            'open_conflicts': [c.get('lesson_id') for c in open_conflicts(lesson)],
            'scope': (lesson.get('applicability') or {}).get('scope'),
            'can_generate_task': can_task,
            'task_blocking_reason': None if can_task else reason,
        })
    scan_statuses = [s.get('scan_status') for s in source_reports]
    counts = {
        'sources_total': len(source_reports),
        'sources_enabled': sum(1 for s in source_reports if s.get('enabled')),
        'sources_ok': scan_statuses.count('OK'),
        'sources_partial': scan_statuses.count('PARTIAL'),
        'sources_blocked': scan_statuses.count('BLOCKED'),
        'sources_unknown': scan_statuses.count('UNKNOWN'),
        'sources_not_run': scan_statuses.count('NOT_RUN'),
        'evidence_items': sum(int(s.get('evidence_count') or 0) for s in source_reports),
        'exclusions': sum(int(s.get('excluded_count') or 0) for s in source_reports),
        'suspicious_evidence': sum(int(s.get('suspicious_instruction_count') or 0)
                                   for s in source_reports),
        'lessons_total': len(lesson_entries),
        'lessons_by_content_class': _counts(e['content_class'] for e in lesson_entries),
        'lessons_by_status': _counts(e['status'] for e in lesson_entries),
        'duplicates': sum(1 for e in lesson_entries if e['duplicate_of']),
        'open_conflicts': sum(len(e['open_conflicts']) for e in lesson_entries),
        'task_eligible': sum(1 for e in lesson_entries if e['can_generate_task']),
    }
    report = {
        'report_schema_version': REPORT_SCHEMA_VERSION,
        'generated_at': generated_at,
        'scan_mode': scan_mode,
        'untrusted_data': {
            'content_trust': 'untrusted_data',
            'may_override_rules': False,
            'redaction_required': True,
            'notice': UNTRUSTED_NOTICE,
        },
        'registry': registry_summary,
        'sources': list(source_reports),
        'lessons': lesson_entries,
        'counts': counts,
        'decisions_required': list(decisions_required),
    }
    return report


def render_markdown(report):
    """Human-readable rendering. Source text is emitted only through ``data_cell``."""
    lines = []
    counts = report.get('counts', {})
    lines.append('# Experience scan report')
    lines.append('')
    lines.append(f"- generated_at: `{report.get('generated_at')}`")
    lines.append(f"- scan_mode: `{report.get('scan_mode')}`")
    lines.append(f"- untrusted data: {report.get('untrusted_data', {}).get('notice', '')}")
    lines.append('')
    lines.append('## Sources')
    lines.append('')
    lines.append('| source_id | repository | scan_status | blocked_reason | scanned_at | window | cursor_before | cursor_after | evidence | excluded |')
    lines.append('|---|---|---|---|---|---|---|---|---|---|')
    for source in report.get('sources', []):
        cursor_before = source.get('cursor_before')
        cursor_after = source.get('cursor_after')
        lines.append('| ' + ' | '.join([
            f"`{source.get('source_id')}`",
            f"`{source.get('repository')}`",
            f"`{source.get('scan_status')}`",
            f"`{source.get('blocked_reason') or '(none)'}`",
            f"`{source.get('scan_started_at')}`",
            f"`{(source.get('window') or {}).get('since')}` → `{(source.get('window') or {}).get('until')}` ({(source.get('window') or {}).get('mode')})",
            f"`{_cursor_label(cursor_before)}`",
            f"`{_cursor_label(cursor_after)}`",
            str(source.get('evidence_count', 0)),
            str(source.get('excluded_count', 0)),
        ]) + ' |')
    if not report.get('sources'):
        lines.append('| (no sources registered) | — | — | — | — | — | — | — | 0 | 0 |')
    lines.append('')
    lines.append('## Evidence')
    lines.append('')
    lines.append('| source_id | kind | observed_at | title (data) | evidence_url |')
    lines.append('|---|---|---|---|---|')
    for source in report.get('sources', []):
        for item in source.get('evidence') or []:
            lines.append('| ' + ' | '.join([
                f"`{item.get('source_id')}`",
                f"`{item.get('evidence_kind')}`",
                f"`{item.get('observed_at')}`",
                untrusted.data_cell(item.get('title')),
                f"<{item.get('url')}>" if item.get('url') else '(no usable link)',
            ]) + ' |')
    lines.append('')
    lines.append('## Lessons')
    lines.append('')
    lines.append('| lesson_id | content_class | status | evidence_level | evidence_urls | updated_at | duplicate_of | open_conflicts | task? |')
    lines.append('|---|---|---|---|---|---|---|---|---|')
    for lesson in report.get('lessons', []):
        urls = ' '.join(f'<{url}>' for url in lesson.get('evidence_urls') or [])
        lines.append('| ' + ' | '.join([
            f"`{lesson.get('lesson_id')}`",
            f"`{lesson.get('content_class')}`",
            f"`{lesson.get('status')}`",
            f"`{lesson.get('evidence_level')}`",
            urls or '(none)',
            f"`{lesson.get('captured_at')}`",
            f"`{lesson.get('duplicate_of')}`" if lesson.get('duplicate_of') else '(none)',
            ', '.join(lesson.get('open_conflicts') or []) or '(none)',
            'yes' if lesson.get('can_generate_task') else f"no — {untrusted.data_cell(lesson.get('task_blocking_reason'))}",
        ]) + ' |')
    if not report.get('lessons'):
        lines.append('| (no lessons recorded) | — | — | — | — | — | — | — | — |')
    lines.append('')
    lines.append('## Counts')
    lines.append('')
    for key in ('sources_total', 'sources_ok', 'sources_partial', 'sources_blocked',
                'sources_unknown', 'sources_not_run', 'evidence_items', 'exclusions',
                'suspicious_evidence', 'lessons_total', 'duplicates', 'open_conflicts',
                'task_eligible'):
        lines.append(f'- {key}: {counts.get(key, 0)}')
    for key in ('lessons_by_content_class', 'lessons_by_status'):
        rendered = ', '.join(f'{name}={value}' for name, value in sorted((counts.get(key) or {}).items()))
        lines.append(f'- {key}: {rendered or "(none)"}')
    lines.append('')
    lines.append('## Decisions required')
    lines.append('')
    decisions = report.get('decisions_required') or []
    if decisions:
        for decision in decisions:
            lines.append(f"- `{decision.get('kind')}` {untrusted.data_cell(decision.get('source_id'))}: "
                         f"{untrusted.data_cell(decision.get('reason'))}")
    else:
        lines.append('- (none)')
    lines.append('')
    lines.append('## Limitations')
    lines.append('')
    lines.append('- A scan that did not read its declared scope advances no cursor; `BLOCKED` / '
                 '`UNKNOWN` / `PARTIAL` / `NOT_RUN` never means "no new evidence".')
    lines.append('- Source text is untrusted data; this report never turns it into instructions.')
    lines.append('- Lessons are candidates only until an explicit analysis and decision exist; '
                 'no Issue or task is created by this pipeline.')
    lines.append('')
    return '\n'.join(lines)


def _cursor_label(cursor):
    if not cursor:
        return 'never-scanned'
    status = cursor.get('last_scan_status')
    success = cursor.get('last_successful_scan_at') or 'never'
    return f"{status} @ {success}"
