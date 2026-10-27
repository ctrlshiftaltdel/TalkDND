#!/usr/bin/env python3
"""The single authoritative experience-source registry (#49).

Frozen semantics live in ``docs/self-improvement/sources-and-trust.md`` and the
machine definition in ``docs/contracts/schemas/source-registry.schema.json``
(both frozen by #47). This module is the only code that reads, validates or
rewrites that registry.

Design rules that follow from the contract:

* **Config, not code.** Adding, disabling, re-enabling or removing a source is a
  data edit. No function here branches on a particular ``source_id`` and the
  default source list never appears in code.
* **Fail closed.** A missing, unreadable, unparsable, unsupported or invalid
  registry is an error. It is never replaced by defaults and never partially
  accepted, because a partially accepted registry looks scanned when it is not.
* **Empty is legal.** ``sources: []`` means "nothing to scan right now"; it is a
  valid registry, not a configuration error.
* **Cursor is state, not proof.** Only a scan that read its whole declared scope
  may advance the watermark. A cached ``blocked_reason`` is required before a
  ``BLOCKED`` / ``UNKNOWN`` cursor may be written, and a start timestamp that
  moves backwards is refused instead of silently trusted.

Only the Python standard library is used.
"""
from __future__ import annotations

import json
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path

from . import untrusted

SCHEMA_VERSION = '1.0'
SUPPORTED_SCHEMA_VERSIONS = ('1.0',)

SOURCE_TYPES = ('downstream_project', 'upstream_release', 'documentation')
ACCESS_MODES = ('public', 'authorized_private')
EVIDENCE_KINDS = ('pull_request', 'issue', 'review', 'commit', 'ci_run')
SCAN_STATUSES = ('OK', 'PARTIAL', 'BLOCKED', 'UNKNOWN')

# Machine-readable reasons a source could not be read. The schema only requires a
# non-empty string (the list in sources-and-trust.md §6 is illustrative), so the
# extra values are implementation vocabulary, documented in
# docs/self-improvement/sources-implementation.md.
BLOCKED_NOT_FOUND = 'not_found'
BLOCKED_FORBIDDEN = 'forbidden'
BLOCKED_RATE_LIMITED = 'rate_limited'
BLOCKED_NETWORK = 'network_unavailable'
BLOCKED_TOOL_UNAVAILABLE = 'tool_unavailable'
UNKNOWN_MALFORMED = 'malformed_response'
UNKNOWN_TIMEOUT = 'timeout'
UNKNOWN_CURSOR_REGRESSION = 'cursor_regressed'

TRUST_CONSTANTS = {
    'content_trust': 'untrusted_data',
    'redaction_required': True,
    'may_override_rules': False,
}
ALLOWED_USE = ('evidence_only', 'synthetic_fixture_source')
MIN_LOOKBACK_DAYS = 1
MAX_LOOKBACK_DAYS = 365

SOURCE_ID_RE = re.compile(r'^[a-z][a-z0-9-]{1,31}$')
REPOSITORY_RE = re.compile(r'^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$')
SHA1_RE = re.compile(r'^[a-f0-9]{40}$')

SOURCE_REQUIRED = ('source_id', 'repository', 'enabled', 'source_type', 'access_mode',
                   'evidence_scope', 'lookback_days', 'scan_cursor', 'trust')
SOURCE_OPTIONAL = ('inclusion_filters', 'exclusion_filters', 'owner_approved_notes')
CURSOR_REQUIRED = ('last_scan_status', 'last_scan_started_at')
CURSOR_OPTIONAL = ('last_successful_scan_at', 'last_scanned_commit', 'blocked_reason')
TRUST_REQUIRED = tuple(TRUST_CONSTANTS) + ('allowed_use',)
TRUST_OPTIONAL = ('notes',)

# Secrets and machine-local paths are refused in registry strings (contract §3.3).
# A registry that needs them is a registry that has to be fixed, not redacted.
# The literal list lives in ``untrusted`` so the registry validator and the
# collector cannot drift apart (and so neither file has to spell out a prefix the
# release scan forbids).
_SENSITIVE_STRINGS = untrusted.SENSITIVE_LITERALS


class RegistryError(Exception):
    """Registry cannot be used. ``errors`` is a list of ``{path, keyword, message}``."""

    def __init__(self, message, errors=()):
        self.errors = list(errors)
        detail = ''
        if self.errors:
            detail = '; '.join(f"{e['path']}: {e['message']}" for e in self.errors[:8])
            if len(self.errors) > 8:
                detail += f' (+{len(self.errors) - 8} more)'
        super().__init__(f'{message}: {detail}' if detail else message)


class CursorRegression(Exception):
    """A scan start timestamp moved backwards; the watermark is not trustworthy."""


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


def _sensitive(value):
    for token, label in _SENSITIVE_STRINGS:
        if token in value:
            return label
    return None


def _validate_string(value, errors, path, *, required=False, pattern=None,
                     min_length=None, max_length=None, keyword='type'):
    if value is None and not required:
        return
    if not isinstance(value, str):
        _err(errors, path, keyword, 'expected a string')
        return
    if required and not value:
        _err(errors, path, 'minLength', 'must not be empty')
        return
    if pattern is not None and value and not pattern.match(value):
        _err(errors, path, 'pattern', f'{value!r} does not match {pattern.pattern}')
    if min_length is not None and len(value) < min_length:
        _err(errors, path, 'minLength', f'must be at least {min_length} characters')
    if max_length is not None and len(value) > max_length:
        _err(errors, path, 'maxLength', f'must be at most {max_length} characters')
    sensitive = _sensitive(value)
    if sensitive:
        _err(errors, path, 'privacy', f'contains a {sensitive}; registries must not carry secrets or personal paths')


def _validate_enum(value, errors, path, allowed):
    if value not in allowed:
        _err(errors, path, 'enum', f'{value!r} is not one of {", ".join(allowed)}')


def _validate_datetime(value, errors, path, *, required=False, nullable=False):
    if value is None and nullable:
        return
    if value is None:
        if required:
            _err(errors, path, 'type', 'expected an RFC 3339 timestamp')
        return
    if not isinstance(value, str):
        _err(errors, path, 'type', 'expected an RFC 3339 timestamp string')
        return
    try:
        parse_timestamp(value)
    except ValueError as exc:
        _err(errors, path, 'format', f'not an RFC 3339 timestamp ({exc})')


def validate_cursor(cursor, path='scan_cursor'):
    errors = []
    if cursor is None:
        return errors
    if not isinstance(cursor, dict):
        _err(errors, path, 'type', 'scan_cursor must be null or an object')
        return errors
    for key in cursor:
        if key not in CURSOR_REQUIRED + CURSOR_OPTIONAL:
            _err(errors, f'{path}/{key}', 'additionalProperties', 'unknown scan_cursor field')
    for key in CURSOR_REQUIRED:
        if key not in cursor:
            _err(errors, f'{path}/{key}', 'required', 'missing required scan_cursor field')
    status = cursor.get('last_scan_status')
    if 'last_scan_status' in cursor:
        _validate_enum(status, errors, f'{path}/last_scan_status', SCAN_STATUSES)
    _validate_datetime(cursor.get('last_scan_started_at'), errors,
                       f'{path}/last_scan_started_at', required=True)
    _validate_datetime(cursor.get('last_successful_scan_at'), errors,
                       f'{path}/last_successful_scan_at', nullable=True)
    commit = cursor.get('last_scanned_commit')
    if commit is not None and not (isinstance(commit, str) and SHA1_RE.match(commit)):
        _err(errors, f'{path}/last_scanned_commit', 'pattern',
             'last_scanned_commit must be a 40-character lowercase hex SHA or null')
    reason = cursor.get('blocked_reason')
    if reason is not None:
        _validate_string(reason, errors, f'{path}/blocked_reason', max_length=500)

    if status == 'OK':
        if cursor.get('last_successful_scan_at') is None:
            _err(errors, f'{path}/last_successful_scan_at', 'required',
                 'an OK cursor must record last_successful_scan_at')
        if cursor.get('last_scanned_commit') is None:
            _err(errors, f'{path}/last_scanned_commit', 'required',
                 'an OK cursor must record last_scanned_commit')
    if status in ('BLOCKED', 'UNKNOWN'):
        if not cursor.get('blocked_reason'):
            _err(errors, f'{path}/blocked_reason', 'required',
                 f'{status} requires a machine-readable blocked_reason')

    started = cursor.get('last_scan_started_at')
    succeeded = cursor.get('last_successful_scan_at')
    if isinstance(started, str) and isinstance(succeeded, str):
        # Only a scan that *started* after the stored success can be the newer
        # success; the schema does not constrain the reverse (a failed scan starts
        # later than the last success it preserves).
        try:
            if parse_timestamp(succeeded) > parse_timestamp(started):
                _err(errors, f'{path}/last_successful_scan_at', 'timestamp_order',
                     'last_successful_scan_at is later than the latest scan start')
        except ValueError:
            pass
    return errors


def validate_trust(trust, path='trust'):
    errors = []
    if not isinstance(trust, dict):
        _err(errors, path, 'type', 'trust must be an object')
        return errors
    for key in trust:
        if key not in TRUST_REQUIRED + TRUST_OPTIONAL:
            _err(errors, f'{path}/{key}', 'additionalProperties', 'unknown trust field')
    for key in TRUST_REQUIRED:
        if key not in trust:
            _err(errors, f'{path}/{key}', 'required', 'missing required trust field')
    for key, expected in TRUST_CONSTANTS.items():
        if key not in trust:
            continue
        actual = trust[key]
        if isinstance(expected, bool):
            if not isinstance(actual, bool) or actual != expected:
                _err(errors, f'{path}/{key}', 'const', f'{key} must be {expected!r}')
        elif actual != expected:
            _err(errors, f'{path}/{key}', 'const', f'{key} must be {expected!r}')
    allowed_use = trust.get('allowed_use')
    if 'allowed_use' in trust:
        if not isinstance(allowed_use, list) or not allowed_use:
            _err(errors, f'{path}/allowed_use', 'minItems', 'allowed_use must be a non-empty array')
        else:
            if len(set(allowed_use)) != len(allowed_use):
                _err(errors, f'{path}/allowed_use', 'uniqueItems', 'allowed_use must not repeat values')
            for index, item in enumerate(allowed_use):
                if item not in ALLOWED_USE:
                    _err(errors, f'{path}/allowed_use/{index}', 'enum',
                         f'{item!r} is not one of {", ".join(ALLOWED_USE)}')
    if 'notes' in trust:
        _validate_string(trust['notes'], errors, f'{path}/notes', max_length=500)
    return errors


def validate_source(source, path='sources/0'):
    errors = []
    if not isinstance(source, dict):
        _err(errors, path, 'type', 'a source entry must be an object')
        return errors
    for key in source:
        if key not in SOURCE_REQUIRED + SOURCE_OPTIONAL:
            _err(errors, f'{path}/{key}', 'additionalProperties', 'unknown source field')
    for key in SOURCE_REQUIRED:
        if key not in source:
            _err(errors, f'{path}/{key}', 'required', 'missing required source field')
    if 'source_id' in source:
        _validate_string(source['source_id'], errors, f'{path}/source_id',
                         required=True, pattern=SOURCE_ID_RE)
    if 'repository' in source:
        _validate_string(source['repository'], errors, f'{path}/repository',
                         required=True, pattern=REPOSITORY_RE)
        repository = source['repository']
        if isinstance(repository, str) and (
                '://' in repository or '@' in repository or ' ' in repository
                or repository.startswith('-') or repository.startswith('/')):
            _err(errors, f'{path}/repository', 'pattern',
                 'repository must be a bare owner/repo, not a URL, credential or CLI argument')
    if 'enabled' in source and not isinstance(source['enabled'], bool):
        _err(errors, f'{path}/enabled', 'type', 'enabled must be a boolean')
    if 'source_type' in source:
        _validate_enum(source['source_type'], errors, f'{path}/source_type', SOURCE_TYPES)
    if 'access_mode' in source:
        _validate_enum(source['access_mode'], errors, f'{path}/access_mode', ACCESS_MODES)
    if 'evidence_scope' in source:
        scope = source['evidence_scope']
        if not isinstance(scope, list) or not scope:
            _err(errors, f'{path}/evidence_scope', 'minItems',
                 'evidence_scope must be a non-empty array')
        elif len(set(scope)) != len(scope):
            _err(errors, f'{path}/evidence_scope', 'uniqueItems',
                 'evidence_scope must not repeat values')
        else:
            for index, item in enumerate(scope):
                if item not in EVIDENCE_KINDS:
                    _err(errors, f'{path}/evidence_scope/{index}', 'enum',
                         f'{item!r} is not one of {", ".join(EVIDENCE_KINDS)}')
    if 'lookback_days' in source:
        value = source['lookback_days']
        if isinstance(value, bool) or not isinstance(value, int):
            _err(errors, f'{path}/lookback_days', 'type', 'lookback_days must be an integer')
        elif not MIN_LOOKBACK_DAYS <= value <= MAX_LOOKBACK_DAYS:
            _err(errors, f'{path}/lookback_days', 'minimum',
                 f'lookback_days must be between {MIN_LOOKBACK_DAYS} and {MAX_LOOKBACK_DAYS}')
    if 'scan_cursor' in source:
        errors.extend(validate_cursor(source['scan_cursor'], f'{path}/scan_cursor'))
    if 'trust' in source:
        errors.extend(validate_trust(source['trust'], f'{path}/trust'))
    for field in ('inclusion_filters', 'exclusion_filters'):
        if field in source:
            value = source[field]
            if not isinstance(value, list):
                _err(errors, f'{path}/{field}', 'type', f'{field} must be an array')
            else:
                for index, item in enumerate(value):
                    _validate_string(item, errors, f'{path}/{field}/{index}',
                                     required=True, max_length=200)
    if 'owner_approved_notes' in source:
        _validate_string(source['owner_approved_notes'], errors,
                         f'{path}/owner_approved_notes', max_length=2000)
    return errors


def validate_registry(data):
    """Full schema-subset + semantic validation. Returns a list of errors."""
    errors = []
    if not isinstance(data, dict):
        return [{'path': '$', 'keyword': 'type', 'message': 'the registry must be a JSON object'}]
    for key in data:
        if key not in ('schema_version', 'registry_updated_at', 'sources'):
            _err(errors, f'/{key}', 'additionalProperties', 'unknown registry field')
    if 'schema_version' not in data:
        _err(errors, '/schema_version', 'required', 'missing required registry field')
    elif data['schema_version'] not in SUPPORTED_SCHEMA_VERSIONS:
        _err(errors, '/schema_version', 'const',
             f"unsupported schema_version {data['schema_version']!r}; supported: "
             f"{', '.join(SUPPORTED_SCHEMA_VERSIONS)}. Upgrade the registry or the tool "
             '(no downgrade guess is made)')
    if 'registry_updated_at' in data:
        _validate_datetime(data['registry_updated_at'], errors, '/registry_updated_at')
    if 'sources' not in data:
        _err(errors, '/sources', 'required', 'missing required registry field')
        return errors
    sources = data['sources']
    if not isinstance(sources, list):
        _err(errors, '/sources', 'type', 'sources must be an array (an empty array is legal)')
        return errors
    seen = {}
    for index, source in enumerate(sources):
        errors.extend(validate_source(source, f'/sources/{index}'))
        if isinstance(source, dict):
            source_id = source.get('source_id')
            if isinstance(source_id, str):
                if source_id in seen:
                    _err(errors, f'/sources/{index}/source_id', 'uniqueItems',
                         f'duplicate source_id {source_id!r} (also at /sources/{seen[source_id]})')
                else:
                    seen[source_id] = index
    return errors


def _assert_valid(data, context='source registry'):
    errors = validate_registry(data)
    if errors:
        raise RegistryError(f'invalid {context}', errors)


class Registry:
    """Validated, immutable-by-convention view over one registry document."""

    def __init__(self, data, path=None):
        _assert_valid(data)
        self.path = Path(path) if path is not None else None
        self._data = json.loads(json.dumps(data))  # deep copy, keep it JSON only

    @classmethod
    def load(cls, path):
        path = Path(path)
        if not path.is_file():
            raise RegistryError(
                f'source registry not found: {path}',
                [{'path': str(path), 'keyword': 'required',
                  'message': 'registry file is missing; fail closed instead of guessing default sources'}])
        try:
            raw = path.read_text(encoding='utf-8')
        except OSError as exc:
            raise RegistryError(f'source registry unreadable: {path}', [
                {'path': str(path), 'keyword': 'readable', 'message': str(exc)}]) from exc
        try:
            data = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise RegistryError(f'source registry is not valid JSON: {path}', [
                {'path': str(path), 'keyword': 'parse', 'message': str(exc)}]) from exc
        registry = cls(data, path=path)
        registry._data['sources'] = [_normalize_source(s) for s in registry._data['sources']]
        return registry

    @property
    def data(self):
        return json.loads(json.dumps(self._data))

    @property
    def sources(self):
        return self._data['sources']

    def enabled_sources(self):
        return [s for s in self._data['sources'] if s.get('enabled')]

    def get(self, source_id):
        for source in self._data['sources']:
            if source.get('source_id') == source_id:
                return source
        return None

    def to_dict(self):
        return self.data

    def with_sources(self, sources, updated_at=None):
        data = self.data
        data['sources'] = list(sources)
        if updated_at is not None:
            data['registry_updated_at'] = updated_at
        return Registry(data, path=self.path)

    def save(self, path=None, updated_at=None):
        target = Path(path) if path is not None else self.path
        if target is None:
            raise RegistryError('no registry path to save to')
        data = self.data
        if updated_at is not None:
            data['registry_updated_at'] = updated_at
        _assert_valid(data)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        self.path = target
        self._data = data
        return target


def _normalize_source(source):
    """Fill schema defaults without inventing values the caller must decide."""
    if not isinstance(source, dict):
        return source
    normalized = dict(source)
    normalized.setdefault('inclusion_filters', [])
    normalized.setdefault('exclusion_filters', [])
    return normalized


def make_source(source_id, repository, *, evidence_scope=('pull_request', 'issue', 'commit'),
                lookback_days=90, source_type='downstream_project', access_mode='public',
                enabled=True, scan_cursor=None, inclusion_filters=None, exclusion_filters=None,
                owner_approved_notes=None, trust=None):
    """Build a source entry as configuration data; validated before it is used."""
    source = {
        'source_id': source_id,
        'repository': repository,
        'enabled': enabled,
        'source_type': source_type,
        'access_mode': access_mode,
        'evidence_scope': list(evidence_scope),
        'lookback_days': lookback_days,
        'scan_cursor': scan_cursor,
        'trust': trust if trust is not None else default_trust(),
        'inclusion_filters': list(inclusion_filters or []),
        'exclusion_filters': list(exclusion_filters or []),
    }
    if owner_approved_notes is not None:
        source['owner_approved_notes'] = owner_approved_notes
    errors = validate_source(source)
    if errors:
        raise RegistryError(f'invalid source {source_id!r}', errors)
    return source


def default_trust(allowed_use=('evidence_only', 'synthetic_fixture_source'), notes=None):
    trust = dict(TRUST_CONSTANTS)
    trust['allowed_use'] = list(allowed_use)
    if notes is not None:
        trust['notes'] = notes
    return trust


def add_source(registry, source, updated_at=None):
    """Append one entry. A source_id may not be re-pointed at another repository."""
    _assert_valid_source_entry(source)
    existing = registry.get(source['source_id'])
    if existing is not None:
        if existing.get('repository') != source.get('repository'):
            raise RegistryError(
                f'source_id {source["source_id"]!r} is already registered',
                [{'path': f'/sources/{source["source_id"]}/repository', 'keyword': 'uniqueItems',
                  'message': ('re-pointing a source_id is forbidden; add a new source and '
                              'disable the old one (sources-and-trust.md §2)')}])
        raise RegistryError(f'source_id {source["source_id"]!r} is already registered', [
            {'path': f'/sources/{source["source_id"]}', 'keyword': 'uniqueItems',
             'message': 'duplicate source_id'}])
    return registry.with_sources(registry.sources + [_normalize_source(source)], updated_at)


def _assert_valid_source_entry(source):
    errors = validate_source(source)
    if errors:
        raise RegistryError('invalid source entry', errors)


def set_enabled(registry, source_id, enabled):
    """Enable or disable one entry, keeping it (and its history) in place."""
    sources = registry.sources
    matches = [s for s in sources if s.get('source_id') == source_id]
    if not matches:
        raise RegistryError(f'unknown source_id {source_id!r}', [
            {'path': '/sources', 'keyword': 'required',
             'message': f'no registered source {source_id!r}'}])
    updated = []
    for source in sources:
        if source.get('source_id') == source_id:
            changed = dict(source)
            changed['enabled'] = bool(enabled)
            updated.append(changed)
        else:
            updated.append(source)
    return registry.with_sources(updated)


def disable_source(registry, source_id):
    return set_enabled(registry, source_id, False)


def enable_source(registry, source_id):
    return set_enabled(registry, source_id, True)


def remove_source(registry, source_id):
    """Delete one entry. Removing every entry is legal and yields ``sources: []``."""
    sources = registry.sources
    if not any(s.get('source_id') == source_id for s in sources):
        raise RegistryError(f'unknown source_id {source_id!r}', [
            {'path': '/sources', 'keyword': 'required',
             'message': f'no registered source {source_id!r}'}])
    return registry.with_sources([s for s in sources if s.get('source_id') != source_id])


def update_cursor(registry, source_id, cursor):
    sources = registry.sources
    if not any(s.get('source_id') == source_id for s in sources):
        raise RegistryError(f'unknown source_id {source_id!r}', [
            {'path': '/sources', 'keyword': 'required',
             'message': f'no registered source {source_id!r}'}])
    updated = []
    for source in sources:
        if source.get('source_id') == source_id:
            changed = dict(source)
            changed['scan_cursor'] = cursor
            updated.append(changed)
        else:
            updated.append(source)
    return registry.with_sources(updated)


def scan_window(source, now):
    """Where the next scan starts, and why. Cursor first, lookback only as fallback.

    Returns ``{'since', 'until', 'mode', 'lookback_days'}``. ``mode`` is one of
    ``first_scan`` (cursor null), ``incremental`` (last OK watermark),
    ``retry`` (non-OK cursor that still has an earlier success) or
    ``lookback_after_failed_scan`` (non-OK cursor with no success at all, and the
    cursor-loss path). Overlap is intentional: the same window may be read twice
    and deduplicated by fingerprint.
    """
    until = now
    lookback_days = int(source.get('lookback_days') or 1)
    cursor = source.get('scan_cursor')
    if not cursor:
        return _window(until - timedelta(days=lookback_days), until, 'first_scan', lookback_days)
    last_success = cursor.get('last_successful_scan_at')
    if cursor.get('last_scan_status') == 'OK' and last_success:
        return _window(parse_timestamp(last_success), until, 'incremental', lookback_days)
    if last_success:
        return _window(parse_timestamp(last_success), until, 'retry', lookback_days)
    return _window(until - timedelta(days=lookback_days), until,
                   'lookback_after_failed_scan', lookback_days)


def _window(since, until, mode, lookback_days):
    return {'since': format_timestamp(since), 'until': format_timestamp(until),
            'mode': mode, 'lookback_days': lookback_days}


def advance_cursor(cursor, *, status, started_at, finished_at=None, scanned_commit=None,
                   blocked_reason=None):
    """Compute the next cursor. Only ``OK`` moves the success watermark.

    A start timestamp earlier than the stored one raises :class:`CursorRegression`
    (sources-and-trust.md §4 rule 2): the caller must stop incremental scanning,
    fall back to the lookback window and record the decision; this function will
    not paper over it by writing a newer-looking cursor.
    """
    if status not in SCAN_STATUSES:
        raise ValueError(f'unknown scan status {status!r}')
    started = parse_timestamp(started_at)
    previous = cursor or None
    if previous and previous.get('last_scan_started_at'):
        previous_started = parse_timestamp(previous['last_scan_started_at'])
        if started < previous_started:
            raise CursorRegression(
                'scan start moved backwards: '
                f'{format_timestamp(started)} < {format_timestamp(previous_started)}')
    next_cursor = {'last_scan_status': status,
                   'last_scan_started_at': format_timestamp(started)}
    prior_success = previous.get('last_successful_scan_at') if previous else None
    prior_commit = previous.get('last_scanned_commit') if previous else None
    if status == 'OK':
        if not scanned_commit or not SHA1_RE.match(str(scanned_commit)):
            raise ValueError('an OK cursor needs a 40-character lowercase hex last_scanned_commit')
        next_cursor['last_successful_scan_at'] = format_timestamp(
            parse_timestamp(finished_at) if finished_at else started)
        next_cursor['last_scanned_commit'] = str(scanned_commit)
        next_cursor['blocked_reason'] = None
    else:
        next_cursor['last_successful_scan_at'] = prior_success
        next_cursor['last_scanned_commit'] = prior_commit
        if status in ('BLOCKED', 'UNKNOWN'):
            if not blocked_reason:
                raise ValueError(f'{status} cursor requires a blocked_reason')
            next_cursor['blocked_reason'] = str(blocked_reason)
        else:
            next_cursor['blocked_reason'] = blocked_reason or None
    errors = validate_cursor(next_cursor)
    if errors:
        raise RegistryError('invalid cursor', errors)
    return next_cursor


def summary(registry):
    """Small, report-safe view of the registry (no notes, no paths)."""
    return {
        'schema_version': registry.data.get('schema_version'),
        'registry_updated_at': registry.data.get('registry_updated_at'),
        'source_count': len(registry.sources),
        'enabled_count': len(registry.enabled_sources()),
        'sources': [
            {
                'source_id': s.get('source_id'),
                'repository': s.get('repository'),
                'enabled': s.get('enabled'),
                'evidence_scope': list(s.get('evidence_scope') or []),
                'lookback_days': s.get('lookback_days'),
                'scan_cursor': s.get('scan_cursor'),
            }
            for s in registry.sources
        ],
    }
