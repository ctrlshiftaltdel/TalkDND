"""Zero-dependency structural + semantic validation of an update report.

The machine authority is ``docs/contracts/schemas/update-report.schema.json``;
the action semantics are in ``docs/self-improvement/update-safety.md``. Standard
JSON Schema can express most of the shape but not the cross-field rules (counts
sum, APPLIED vs conflict classes, rollback failure -> BLOCKED). This module
implements both so the engine can self-check before handing a report on.

Only the standard library is used, and no schema file is loaded at runtime: the
frozen schema is a document, not executable configuration.
"""
from __future__ import annotations

import re
from datetime import datetime

from . import classify, package, versions

REPORT_SCHEMA_VERSION = '1.0'

_REPORT_ID = re.compile(r'^UPD-[0-9]{8}T[0-9]{6}Z-[a-f0-9]{6}$')
_SHA256 = re.compile(r'^[a-f0-9]{64}$')
_HEAD = re.compile(r'^[a-f0-9]{40}$')
_TAG = re.compile(r'^v[0-9]+\.[0-9]+(\.[0-9]+)?$')
_VERSION = re.compile(r'^[0-9]+\.[0-9]+(\.[0-9]+)?$')
_FORMAT = re.compile(r'^[0-9]+\.[0-9]+$')
_ASSET = re.compile(r'^[A-Za-z0-9._-]+\.zip$')
_DECISION_ID = re.compile(r'^HD-[A-Z0-9-]{2,24}$')
_FILE_PATH = re.compile(r'^[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*$')

_RESULTS = (
    'PLANNED', 'APPLIED', 'STOPPED_CONFLICT', 'STOPPED_UNSAFE_PACKAGE',
    'STOPPED_INVALID_BASELINE', 'STOPPED_DIRTY_WORKTREE', 'ROLLED_BACK', 'BLOCKED',
)
_MODES = ('PLAN', 'APPLY')
_EDITIONS = ('local', 'github')
_VERIFICATION_STATUS = ('VERIFIED', 'UNSAFE_PACKAGE', 'NOT_RUN')
_CHECK_RESULTS = ('PASS', 'FAIL', 'NOT_RUN')
_BASELINE_STATUS = ('VALID', 'INVALID_BASELINE', 'MISSING')
_DECISIONS = ('APPLIED', 'KEPT_LOCAL', 'CONFLICT_REPORTED', 'DEFERRED', 'BLOCKED',
              'NOT_APPLICABLE')
_ROLLBACK_RESULTS = ('NOT_NEEDED', 'SUCCEEDED', 'FAILED', 'NOT_RUN')
_HD_STATUS = ('OPEN', 'DECIDED')


def _is_int(value) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _object(errors: list[str], value, label: str, required: set[str],
            optional: set[str] = frozenset()) -> dict | None:
    if not isinstance(value, dict):
        errors.append(f'{label}: must be an object')
        return None
    missing = sorted(required - set(value))
    extra = sorted(set(value) - required - set(optional))
    if missing:
        errors.append(f'{label}: missing required key(s) {missing}')
    if extra:
        errors.append(f'{label}: unknown key(s) {extra}')
    return value


def _string(errors: list[str], value, label: str, *, pattern: re.Pattern | None = None,
            choices: tuple[str, ...] | None = None, nullable: bool = False,
            min_length: int = 1, max_length: int | None = None) -> None:
    if value is None and nullable:
        return
    if not isinstance(value, str):
        errors.append(f'{label}: must be a string{" or null" if nullable else ""}')
        return
    if len(value) < min_length:
        errors.append(f'{label}: must not be empty')
    if max_length is not None and len(value) > max_length:
        errors.append(f'{label}: longer than {max_length} characters')
    if pattern is not None and not pattern.match(value):
        errors.append(f'{label}: does not match {pattern.pattern}')
    if choices is not None and value not in choices:
        errors.append(f'{label}: {value!r} is not one of {list(choices)}')


def _datetime(errors: list[str], value, label: str, *, nullable: bool = False) -> None:
    if value is None and nullable:
        return
    if not isinstance(value, str):
        errors.append(f'{label}: must be an ISO-8601 date-time string')
        return
    try:
        datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError:
        errors.append(f'{label}: not a valid ISO-8601 date-time: {value!r}')


def _list(errors: list[str], value, label: str) -> list | None:
    if not isinstance(value, list):
        errors.append(f'{label}: must be a list')
        return None
    return value


def _sha(errors: list[str], value, label: str, *, nullable: bool = False) -> None:
    _string(errors, value, label, pattern=_SHA256, nullable=nullable)


def validate_report(report: object) -> list[str]:
    """Return a list of contract violations; an empty list means conforming."""
    errors: list[str] = []
    if not isinstance(report, dict):
        return ['report: must be a JSON object']

    allowed = {
        'schema_version', 'report_id', 'generated_at', 'operation', 'baseline',
        'target', 'workspace', 'classification', 'backup', 'rollback', 'result',
        'post_apply_checks', 'commands', 'human_decisions',
        'head_sha_before', 'head_sha_after',
    }
    for key in sorted(set(report) - allowed):
        errors.append(f'report: unknown top-level key {key!r}')
    for key in ('schema_version', 'report_id', 'generated_at', 'operation', 'baseline',
                'target', 'workspace', 'classification', 'backup', 'rollback', 'result',
                'post_apply_checks'):
        if key not in report:
            errors.append(f'report: missing required key {key!r}')

    if report.get('schema_version') != REPORT_SCHEMA_VERSION:
        errors.append(f"report.schema_version: must be {REPORT_SCHEMA_VERSION!r}")

    _string(errors, report.get('report_id'), 'report.report_id', pattern=_REPORT_ID)
    _datetime(errors, report.get('generated_at'), 'report.generated_at')

    operation = _object(errors, report.get('operation'), 'report.operation',
                        {'command', 'edition', 'mode'}, {'branch', 'worktree'})
    if operation is not None:
        if operation.get('command') != 'update_agentorch':
            errors.append("report.operation.command: must be 'update_agentorch'")
        _string(errors, operation.get('edition'), 'report.operation.edition', choices=_EDITIONS)
        _string(errors, operation.get('mode'), 'report.operation.mode', choices=_MODES)
        _string(errors, operation.get('branch'), 'report.operation.branch',
                nullable=True, max_length=200)
        _string(errors, operation.get('worktree'), 'report.operation.worktree',
                nullable=True, max_length=500)

    baseline = _object(errors, report.get('baseline'), 'report.baseline',
                       {'installed_version', 'installed_format_version', 'manifest_sha256',
                        'head_sha', 'baseline_status'}, {'notes'})
    if baseline is not None:
        status = baseline.get('baseline_status')
        _string(errors, status, 'report.baseline.baseline_status', choices=_BASELINE_STATUS)
        _string(errors, baseline.get('installed_version'), 'report.baseline.installed_version',
                nullable=True, max_length=100)
        _string(errors, baseline.get('installed_format_version'),
                'report.baseline.installed_format_version', pattern=_FORMAT, nullable=True)
        _sha(errors, baseline.get('manifest_sha256'), 'report.baseline.manifest_sha256',
             nullable=True)
        _string(errors, baseline.get('head_sha'), 'report.baseline.head_sha',
                pattern=_HEAD, nullable=True)
        _string(errors, baseline.get('notes'), 'report.baseline.notes',
                nullable=True, min_length=0, max_length=1000)
        if status == 'VALID':
            if not baseline.get('installed_version'):
                errors.append('report.baseline.installed_version: required when baseline_status=VALID')
            _string(errors, baseline.get('installed_format_version'),
                    'report.baseline.installed_format_version', pattern=_FORMAT)
            _sha(errors, baseline.get('manifest_sha256'), 'report.baseline.manifest_sha256')

    target = _object(errors, report.get('target'), 'report.target',
                     {'repository', 'release_tag', 'version', 'asset_name', 'asset_sha256',
                      'manifest_sha256', 'retrieved_at', 'verification'})
    verification_status = None
    if target is not None:
        if target.get('repository') != 'ctrlshiftaltdel/AgentOrch':
            errors.append("report.target.repository: must be 'ctrlshiftaltdel/AgentOrch'")
        _string(errors, target.get('release_tag'), 'report.target.release_tag',
                pattern=_TAG, nullable=True)
        _string(errors, target.get('version'), 'report.target.version',
                pattern=_VERSION, nullable=True)
        _string(errors, target.get('asset_name'), 'report.target.asset_name',
                pattern=_ASSET, nullable=True)
        _sha(errors, target.get('asset_sha256'), 'report.target.asset_sha256', nullable=True)
        _sha(errors, target.get('manifest_sha256'), 'report.target.manifest_sha256', nullable=True)
        _datetime(errors, target.get('retrieved_at'), 'report.target.retrieved_at', nullable=True)
        verification = _object(errors, target.get('verification'), 'report.target.verification',
                               {'status', 'checks'})
        if verification is not None:
            verification_status = verification.get('status')
            _string(errors, verification_status, 'report.target.verification.status',
                    choices=_VERIFICATION_STATUS)
            checks = _list(errors, verification.get('checks'), 'report.target.verification.checks')
            if checks is not None:
                names = []
                for index, entry in enumerate(checks):
                    label = f'report.target.verification.checks[{index}]'
                    item = _object(errors, entry, label, {'check', 'result'}, {'detail'})
                    if item is None:
                        continue
                    _string(errors, item.get('check'), f'{label}.check',
                            choices=package.CHECK_ORDER)
                    _string(errors, item.get('result'), f'{label}.result', choices=_CHECK_RESULTS)
                    _string(errors, item.get('detail'), f'{label}.detail',
                            nullable=True, min_length=0, max_length=1000)
                    names.append(item.get('result'))
                if verification_status == 'VERIFIED':
                    if not checks:
                        errors.append('report.target.verification: VERIFIED requires at least one check')
                    if any(result in ('FAIL', 'NOT_RUN') for result in names):
                        errors.append('report.target.verification: VERIFIED must not contain FAIL/NOT_RUN checks')
                if verification_status == 'UNSAFE_PACKAGE' and 'FAIL' not in names:
                    errors.append('report.target.verification: UNSAFE_PACKAGE requires a FAIL check')

        tag, version = target.get('release_tag'), target.get('version')
        if tag is not None and version is not None:
            tag_key = versions.normalize_version(tag)
            version_key = versions.normalize_version(version)
            if tag_key != version_key:
                errors.append('report.target: release_tag and version do not normalize to the same value')

    workspace = _object(errors, report.get('workspace'), 'report.workspace',
                        {'dirty', 'dirty_paths', 'action'})
    if workspace is not None:
        dirty = workspace.get('dirty')
        if not isinstance(dirty, bool):
            errors.append('report.workspace.dirty: must be a boolean')
        _string(errors, workspace.get('action'), 'report.workspace.action',
                choices=('NONE', 'ISOLATED_BRANCH', 'STOPPED_DIRTY_WORKTREE'))
        paths = _list(errors, workspace.get('dirty_paths'), 'report.workspace.dirty_paths')
        if paths is not None:
            for index, path in enumerate(paths):
                _string(errors, path, f'report.workspace.dirty_paths[{index}]',
                        min_length=1, max_length=512)
            if dirty is True and not paths:
                errors.append('report.workspace.dirty_paths: required when dirty=true')
            if dirty is False and paths:
                errors.append('report.workspace.dirty_paths: must be empty when dirty=false')
        if workspace.get('action') == 'STOPPED_DIRTY_WORKTREE' and dirty is not True:
            errors.append('report.workspace: STOPPED_DIRTY_WORKTREE requires dirty=true')

    classification = _object(errors, report.get('classification'), 'report.classification',
                             {'counts', 'files'})
    counts = {}
    files = []
    if classification is not None:
        counts = _object(errors, classification.get('counts'),
                         'report.classification.counts', set(classify.CLASSES)) or {}
        for name in classify.CLASSES:
            value = counts.get(name)
            if not _is_int(value) or value < 0:
                errors.append(f'report.classification.counts.{name}: must be a non-negative integer')
        files = _list(errors, classification.get('files'), 'report.classification.files') or []
        for index, item in enumerate(files):
            label = f'report.classification.files[{index}]'
            entry = _object(errors, item, label,
                            {'path', 'class', 'baseline_sha256', 'current_sha256',
                             'target_sha256', 'decision'}, {'notes'})
            if entry is None:
                continue
            _string(errors, entry.get('path'), f'{label}.path', pattern=_FILE_PATH, max_length=512)
            _string(errors, entry.get('class'), f'{label}.class', choices=classify.CLASSES)
            for field_name in ('baseline_sha256', 'current_sha256', 'target_sha256'):
                _sha(errors, entry.get(field_name), f'{label}.{field_name}', nullable=True)
            _string(errors, entry.get('decision'), f'{label}.decision', choices=_DECISIONS)
            _string(errors, entry.get('notes'), f'{label}.notes',
                    nullable=True, min_length=0, max_length=1000)
            klass, decision = entry.get('class'), entry.get('decision')
            if decision == 'APPLIED' and klass != classify.AUTO_APPLY_CLASS:
                errors.append(f'{label}: only UPSTREAM_UNCHANGED may be APPLIED, got {klass!r}')
            if klass == 'UPSTREAM_UNCHANGED' and decision == 'APPLIED':
                if entry.get('target_sha256') is None:
                    errors.append(f'{label}: APPLIED requires target_sha256')
        if len(counts) == len(classify.CLASSES) and _is_int(sum(
                value for value in counts.values() if _is_int(value))):
            total = sum(value for value in counts.values() if _is_int(value))
            if total != len(files):
                errors.append(f'report.classification.counts: sum {total} != files length {len(files)}')

    backup = _object(errors, report.get('backup'), 'report.backup',
                     {'required', 'location', 'created_at', 'verified'})
    if backup is not None:
        required = backup.get('required')
        if not isinstance(required, bool):
            errors.append('report.backup.required: must be a boolean')
        if not isinstance(backup.get('verified'), bool):
            errors.append('report.backup.verified: must be a boolean')
        _string(errors, backup.get('location'), 'report.backup.location',
                nullable=True, max_length=500)
        _datetime(errors, backup.get('created_at'), 'report.backup.created_at', nullable=True)
        if required is True:
            if not backup.get('location'):
                errors.append('report.backup.location: required when required=true')
            if backup.get('created_at') is None:
                errors.append('report.backup.created_at: required when required=true')

    rollback = _object(errors, report.get('rollback'), 'report.rollback',
                       {'required', 'attempted', 'result'}, {'evidence'})
    if rollback is not None:
        if not isinstance(rollback.get('required'), bool):
            errors.append('report.rollback.required: must be a boolean')
        if not isinstance(rollback.get('attempted'), bool):
            errors.append('report.rollback.attempted: must be a boolean')
        _string(errors, rollback.get('result'), 'report.rollback.result',
                choices=_ROLLBACK_RESULTS)
        _string(errors, rollback.get('evidence'), 'report.rollback.evidence',
                nullable=True, min_length=0, max_length=1000)
        if rollback.get('attempted') is True and rollback.get('required') is not True:
            errors.append('report.rollback: attempted=true requires required=true')

    result = report.get('result')
    _string(errors, result, 'report.result', choices=_RESULTS)

    post_checks = _list(errors, report.get('post_apply_checks'),
                        'report.post_apply_checks') or []
    for index, item in enumerate(post_checks):
        label = f'report.post_apply_checks[{index}]'
        entry = _object(errors, item, label, {'name', 'status'}, {'command', 'exit_code'})
        if entry is None:
            continue
        _string(errors, entry.get('name'), f'{label}.name', max_length=300)
        _string(errors, entry.get('status'), f'{label}.status', choices=_CHECK_RESULTS)
        _string(errors, entry.get('command'), f'{label}.command',
                nullable=True, max_length=500)
        if entry.get('exit_code') is not None and not _is_int(entry.get('exit_code')):
            errors.append(f'{label}.exit_code: must be an integer or null')

    commands = report.get('commands', [])
    command_list = _list(errors, commands, 'report.commands') or []
    for index, item in enumerate(command_list):
        label = f'report.commands[{index}]'
        entry = _object(errors, item, label, {'command', 'exit_code'}, {'ran_at'})
        if entry is None:
            continue
        _string(errors, entry.get('command'), f'{label}.command', max_length=1000)
        if not _is_int(entry.get('exit_code')):
            errors.append(f'{label}.exit_code: must be an integer')
        _datetime(errors, entry.get('ran_at'), f'{label}.ran_at', nullable=True)

    decisions = report.get('human_decisions', [])
    decision_list = _list(errors, decisions, 'report.human_decisions') or []
    for index, item in enumerate(decision_list):
        label = f'report.human_decisions[{index}]'
        entry = _object(errors, item, label,
                        {'decision_id', 'question', 'options', 'status'},
                        {'decided_by', 'decided_at'})
        if entry is None:
            continue
        _string(errors, entry.get('decision_id'), f'{label}.decision_id', pattern=_DECISION_ID)
        _string(errors, entry.get('question'), f'{label}.question',
                min_length=8, max_length=500)
        options = _list(errors, entry.get('options'), f'{label}.options')
        if options is not None:
            if len(options) < 2:
                errors.append(f'{label}.options: at least two options are required')
            for option_index, option in enumerate(options):
                _string(errors, option, f'{label}.options[{option_index}]', max_length=300)
        _string(errors, entry.get('status'), f'{label}.status', choices=_HD_STATUS)
        _string(errors, entry.get('decided_by'), f'{label}.decided_by',
                nullable=True, min_length=0, max_length=120)
        _datetime(errors, entry.get('decided_at'), f'{label}.decided_at', nullable=True)

    for key in ('head_sha_before', 'head_sha_after'):
        _string(errors, report.get(key), f'report.{key}', pattern=_HEAD, nullable=True)

    # --- cross-field semantics ---------------------------------------------
    if result == 'APPLIED':
        if operation is not None and operation.get('mode') != 'APPLY':
            errors.append('report.result: APPLIED requires operation.mode=APPLY')
        if verification_status != 'VERIFIED':
            errors.append('report.result: APPLIED requires target.verification.status=VERIFIED')
        if workspace is not None and workspace.get('action') not in ('NONE', 'ISOLATED_BRANCH'):
            errors.append('report.result: APPLIED requires a non-stopped workspace action')
        for name in classify.CONFLICT_CLASSES:
            if counts.get(name):
                errors.append(f'report.result: APPLIED requires counts.{name}=0 (conflict)')
        for entry in files:
            if isinstance(entry, dict) and entry.get('class') in classify.CONFLICT_CLASSES:
                errors.append('report.result: APPLIED must not contain conflict/removal/rename classes')
                break
        if backup is not None and not (backup.get('required') is True and backup.get('verified') is True):
            errors.append('report.result: APPLIED requires a verified backup')
        for entry in decision_list:
            if isinstance(entry, dict) and entry.get('status') == 'OPEN':
                errors.append('report.result: APPLIED must not carry OPEN human decisions')
                break

    if rollback is not None and rollback.get('result') == 'FAILED' and result != 'BLOCKED':
        errors.append('report.rollback: result=FAILED requires report.result=BLOCKED')
    if result == 'STOPPED_UNSAFE_PACKAGE' and verification_status != 'UNSAFE_PACKAGE':
        errors.append('report.result: STOPPED_UNSAFE_PACKAGE requires verification.status=UNSAFE_PACKAGE')
    if result == 'STOPPED_DIRTY_WORKTREE' and workspace is not None \
            and workspace.get('action') != 'STOPPED_DIRTY_WORKTREE':
        errors.append('report.result: STOPPED_DIRTY_WORKTREE requires workspace.action=STOPPED_DIRTY_WORKTREE')
    if baseline is not None and baseline.get('baseline_status') == 'INVALID_BASELINE' \
            and result not in ('STOPPED_INVALID_BASELINE', 'BLOCKED'):
        errors.append('report.baseline: INVALID_BASELINE requires a stopped/BLOCKED result')
    if any(isinstance(entry, dict) and entry.get('class') == 'UNSAFE_PACKAGE' for entry in files) \
            and result not in ('STOPPED_UNSAFE_PACKAGE', 'BLOCKED'):
        errors.append('report.classification: UNSAFE_PACKAGE class requires a stopped/BLOCKED result')

    return errors
