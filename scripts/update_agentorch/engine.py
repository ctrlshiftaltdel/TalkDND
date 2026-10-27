"""The safe update engine: PLAN / APPLY orchestration.

Order of operations (all fail closed):

1. inspect the workspace (git head / branch / uncommitted paths);
2. for APPLY on a dirty tree, create an isolated branch + worktree and leave the
   original tree untouched (never ``stash`` / ``checkout --`` / ``clean``);
3. read the installed baseline manifest (missing / unreadable -> stop, no network);
4. fetch and verify the official release package in memory / outside the project;
5. classify baseline vs current vs target;
6. PLAN stops with a report; APPLY backs up, writes only ``UPSTREAM_UNCHANGED``
   paths + the package manifest, verifies, and rolls back on any failure.

The report is assembled in :mod:`update_agentorch.report`'s shape and can be
validated with ``report.validate_report``.
"""
from __future__ import annotations

import hashlib
import json
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from . import classify, package, report as report_module, sources, versions, workspace

EXIT_CODES = {
    'PLANNED': 0,
    'APPLIED': 0,
    'STOPPED_CONFLICT': 2,
    'STOPPED_UNSAFE_PACKAGE': 2,
    'STOPPED_INVALID_BASELINE': 2,
    'STOPPED_DIRTY_WORKTREE': 2,
    'BLOCKED': 3,
    'ROLLED_BACK': 4,
}


@dataclass
class Baseline:
    status: str
    manifest: dict | None = None
    manifest_sha256: str | None = None
    format_version: str | None = None
    installed_version: str | None = None
    suite: str | None = None
    notes: str | None = None


def _raw_sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _stamp(now: datetime) -> str:
    return now.astimezone(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')


def _report_id(project: Path, mode: str, now: datetime) -> str:
    fingerprint = hashlib.sha256(f'{project.resolve()}|{mode}|{now.isoformat()}'.encode()).hexdigest()[:6]
    return f'UPD-{now.astimezone(timezone.utc).strftime("%Y%m%dT%H%M%SZ")}-{fingerprint}'


def _zero_counts() -> dict[str, int]:
    return {name: 0 for name in classify.CLASSES}


def _read_baseline(root: Path) -> Baseline:
    path = Path(root) / package.MANIFEST_NAME
    if not path.is_file():
        return Baseline(
            status='MISSING',
            notes=('no MANIFEST.json: the installed baseline is unknown. Run a read-only '
                   'diagnosis and migrate manually (HD-08); no file is overwritten.'),
        )
    try:
        data = path.read_bytes()
    except OSError:
        return Baseline(status='INVALID_BASELINE',
                        notes='cannot read MANIFEST.json (I/O or permission error)')
    try:
        manifest = json.loads(data.decode('utf-8'))
    except (UnicodeDecodeError, ValueError) as exc:
        return Baseline(status='INVALID_BASELINE', notes=f'MANIFEST.json is not valid JSON: {exc}')
    errors = package.manifest_schema_errors(manifest)
    if errors:
        return Baseline(status='INVALID_BASELINE',
                        notes='MANIFEST.json is invalid: ' + '; '.join(errors[:5]))
    return Baseline(
        status='VALID',
        manifest=manifest,
        manifest_sha256=_raw_sha256(data),
        format_version=manifest['format_version'],
        installed_version=manifest['format_version'],
        suite=manifest['suite'],
    )


def _decision(decision_id: str, question: str, options: list[str]) -> dict:
    return {
        'decision_id': decision_id,
        'question': question,
        'options': options,
        'status': 'OPEN',
    }


HD_DECISIONS = {
    'HD-09': lambda: _decision(
        'HD-09',
        'Upstream removed or renamed a file that still exists in the project. Keep or clean it?',
        ['keep the old file (default)', 'approve deletion/rename in a separate decision']),
    'HD-10': lambda: _decision(
        'HD-10',
        'A file was modified both locally and upstream. Which version should win?',
        ['keep the local file (default)', 'merge manually', 'accept upstream after review']),
    'HD-11': lambda: _decision(
        'HD-11',
        'Rollback verification failed. How should the damaged workspace be recovered?',
        ['restore from the backup manually', 'stop and investigate before any retry']),
    'HD-12': lambda: _decision(
        'HD-12',
        'Uncommitted changes could not be isolated. How should the update proceed?',
        ['commit/stash manually, then rerun', 'run the update inside a dedicated worktree']),
    'HD-16': lambda: _decision(
        'HD-16',
        'The target version is not newer than the installed version. Reinstall or downgrade?',
        ['stay on the installed version (default)', 'approve reinstall/downgrade explicitly']),
}


def _skeleton(*, project, mode, edition, now, report_id, git, baseline, dirty_paths,
              action, worktree) -> dict:
    branch = git.branch if git.branch and git.branch != 'HEAD' else None
    return {
        'schema_version': report_module.REPORT_SCHEMA_VERSION,
        'report_id': report_id,
        'generated_at': _stamp(now),
        'operation': {
            'command': 'update_agentorch',
            'edition': edition,
            'mode': mode,
            'branch': branch,
            'worktree': worktree,
        },
        'baseline': {
            'installed_version': baseline.installed_version if baseline.status == 'VALID' else None,
            'installed_format_version': baseline.format_version if baseline.status == 'VALID' else None,
            'manifest_sha256': baseline.manifest_sha256 if baseline.status == 'VALID' else None,
            'head_sha': git.head_sha,
            'baseline_status': baseline.status,
            'notes': baseline.notes,
        },
        'target': {
            'repository': sources.OFFICIAL_REPOSITORY,
            'release_tag': None,
            'version': None,
            'asset_name': None,
            'asset_sha256': None,
            'manifest_sha256': None,
            'retrieved_at': None,
            'verification': {'status': 'NOT_RUN', 'checks': []},
        },
        'workspace': {
            'dirty': bool(dirty_paths),
            'dirty_paths': list(dirty_paths),
            'action': action,
        },
        'classification': {'counts': _zero_counts(), 'files': []},
        'backup': {'required': False, 'location': None, 'created_at': None, 'verified': False},
        'rollback': {'required': False, 'attempted': False, 'result': 'NOT_NEEDED', 'evidence': None},
        'result': 'BLOCKED',
        'post_apply_checks': [],
        'commands': list(git.commands),
        'human_decisions': [],
        'head_sha_before': git.head_sha,
        'head_sha_after': git.head_sha,
    }


def _not_run_checks(detail: str) -> list[dict]:
    return [{'check': name, 'result': 'NOT_RUN', 'detail': detail} for name in package.CHECK_ORDER]


def _record_command(report: dict, command: str, exit_code: int, now: datetime) -> None:
    """Record a real operation (git runs and release transfers) with its outcome."""
    report['commands'].append({
        'command': command,
        'exit_code': exit_code,
        'ran_at': _stamp(now),
    })


def _merge_manifest_entry(result: classify.Classification, baseline: Baseline,
                          pkg: package.PackageResult, mode: str) -> None:
    same = baseline.manifest_sha256 == pkg.manifest_sha256
    klass = 'ALREADY_CURRENT' if same else 'UPSTREAM_UNCHANGED'
    if same:
        decision = 'NOT_APPLICABLE'
    else:
        decision = 'APPLIED' if mode == 'APPLY' else 'DEFERRED'
    entry = {
        'path': package.MANIFEST_NAME,
        'class': klass,
        'baseline_sha256': baseline.manifest_sha256,
        'current_sha256': baseline.manifest_sha256,
        'target_sha256': pkg.manifest_sha256,
        'decision': decision,
        'notes': 'package manifest: baseline bookkeeping for the next update',
    }
    result.files.append(entry)
    result.files.sort(key=lambda item: item['path'])
    result.counts[klass] += 1


def _rollback_failed(report: dict, problems: list[str]) -> None:
    report['rollback'] = {
        'required': True,
        'attempted': True,
        'result': 'FAILED',
        'evidence': ('rollback verification failed: ' + '; '.join(problems[:3]))[:1000],
    }
    report['result'] = 'BLOCKED'
    report['human_decisions'] = [HD_DECISIONS['HD-11']()]
    report['post_apply_checks'] = [{'name': 'rollback_verification', 'status': 'FAIL',
                                    'command': None, 'exit_code': None}]


def run_update(project, *, mode: str, source, edition: str,
               now: datetime | None = None, fault_injector=None,
               limits: package.PackageLimits | None = None,
               isolation: bool = True) -> dict:
    """Run one PLAN or APPLY against ``project`` and return the report dict."""
    if mode not in ('PLAN', 'APPLY'):
        raise ValueError(f'mode must be PLAN or APPLY, got {mode!r}')
    if edition not in ('local', 'github'):
        raise ValueError(f'edition must be local or github, got {edition!r}')
    project = Path(project)
    if not project.is_dir():
        raise ValueError(f'project root does not exist or is not a directory: {project}')
    now = now or datetime.now(timezone.utc)
    limits = limits or package.DEFAULT_LIMITS
    report_id = _report_id(project, mode, now)

    git = workspace.inspect_git(project, now)
    dirty_paths = list(git.dirty_paths)
    action = 'NONE'
    worktree_rel = None
    target_root = project

    if git.dirty and mode == 'APPLY':
        if isolation and git.is_repo:
            created = workspace.create_isolation_worktree(project, report_id, now)
            git.commands.extend(created.commands)
            if created.path is not None:
                target_root = created.path
                action = 'ISOLATED_BRANCH'
                worktree_rel = created.path.relative_to(project).as_posix()
                target_git = workspace.inspect_git(target_root, now)
                git.commands.extend(target_git.commands)
                git = _combine_isolation(git, target_git)
            # fall through: baseline is read below (a failure stops before writes)
        # a dirty tree without a usable isolation is stopped after the report skeleton

    baseline = _read_baseline(target_root)
    report = _skeleton(project=project, mode=mode, edition=edition, now=now,
                       report_id=report_id, git=git, baseline=baseline,
                       dirty_paths=dirty_paths, action=action, worktree=worktree_rel)

    if git.dirty and mode == 'APPLY' and action != 'ISOLATED_BRANCH':
        report['result'] = 'STOPPED_DIRTY_WORKTREE'
        report['workspace']['action'] = 'STOPPED_DIRTY_WORKTREE'
        report['human_decisions'] = [HD_DECISIONS['HD-12']()]
        return report

    if baseline.status != 'VALID':
        # Read-only inventory so the report can point at a manual migration route
        # without touching (or trusting) anything in the tree.
        try:
            inventory = workspace.scan_project_files(target_root)
        except OSError:
            inventory = {}
        note = (baseline.notes or '') + f' Read-only inventory: {len(inventory)} regular files.'
        report['baseline']['notes'] = note[:1000]
        report['result'] = 'STOPPED_INVALID_BASELINE'
        return report

    # --- fetch + verify (never writes to the project) ----------------------
    try:
        release = source.fetch_release()
        _record_command(report, 'github:releases/latest ctrlshiftaltdel/AgentOrch', 0, now)
        asset = sources.select_asset(release)
    except sources.SourceError as exc:
        _record_command(report, 'github:releases/latest ctrlshiftaltdel/AgentOrch', 1, now)
        report['target']['verification'] = {
            'status': 'UNSAFE_PACKAGE',
            'checks': _source_error_checks(exc),
        }
        report['result'] = 'STOPPED_UNSAFE_PACKAGE'
        return report
    except (OSError, TimeoutError, ValueError) as exc:
        _record_command(report, 'github:releases/latest ctrlshiftaltdel/AgentOrch', 1, now)
        report['target']['verification'] = {
            'status': 'NOT_RUN',
            'checks': _not_run_checks(f'release could not be retrieved: {type(exc).__name__}'),
        }
        report['result'] = 'BLOCKED'
        return report

    try:
        payload = source.download_asset(asset)
        _record_command(report, f'github:release-asset {asset.name}', 0, now)
    except sources.SourceError as exc:
        _record_command(report, f'github:release-asset {asset.name}', 1, now)
        report['target'].update(_target_fields(release, asset, now))
        report['target']['verification'] = {'status': 'UNSAFE_PACKAGE',
                                            'checks': _source_error_checks(exc)}
        report['result'] = 'STOPPED_UNSAFE_PACKAGE'
        return report
    except (OSError, TimeoutError, ValueError) as exc:
        _record_command(report, f'github:release-asset {asset.name}', 1, now)
        report['target'].update(_target_fields(release, asset, now))
        report['target']['verification'] = {
            'status': 'NOT_RUN',
            'checks': _not_run_checks(f'asset could not be downloaded: {type(exc).__name__}'),
        }
        report['result'] = 'BLOCKED'
        return report

    pkg = package.verify_package(release, payload, expected_suite=baseline.suite,
                                 limits=limits)
    report['target'].update(_target_fields(release, asset, now))
    report['target']['asset_sha256'] = _raw_sha256(payload)
    report['target']['manifest_sha256'] = pkg.manifest_sha256
    report['target']['verification'] = {'status': pkg.status, 'checks': pkg.checks}
    if not pkg.ok:
        report['result'] = 'STOPPED_UNSAFE_PACKAGE'
        return report

    # --- version monotonicity ---------------------------------------------
    target_key = versions.version_key(pkg.version)
    installed_key = versions.version_key(baseline.format_version)
    if target_key <= installed_key:
        report['result'] = 'BLOCKED'
        report['human_decisions'] = [HD_DECISIONS['HD-16']()]
        return report

    # --- classify (staging directory is outside the project) --------------
    with tempfile.TemporaryDirectory(prefix='agentorch-update-') as staging:
        try:
            extracted = package.extract_package(pkg, staging)
            target_bytes = {rel: (extracted / rel).read_bytes() for rel in pkg.files}
        except (OSError, ValueError) as exc:
            report['target']['verification'] = {
                'status': 'UNSAFE_PACKAGE',
                'checks': _fail_check_after_verification(pkg, f'package could not be staged: {exc}'),
            }
            report['result'] = 'STOPPED_UNSAFE_PACKAGE'
            return report

        expected_hashes = {entry['path']: entry['sha256'] for entry in pkg.manifest['files']}
        if any(workspace.sha256_bytes(target_bytes[rel]) != expected
               for rel, expected in expected_hashes.items()):
            report['result'] = 'BLOCKED'
            report['rollback']['evidence'] = 'staged bytes do not match the manifest after extraction'
            return report

        baseline_files = {entry['path']: entry['sha256'] for entry in baseline.manifest['files']}
        current_files = workspace.scan_project_files(target_root)
        current_files.pop(package.MANIFEST_NAME, None)
        target_hashes = {rel: workspace.sha256_bytes(data) for rel, data in target_bytes.items()}
        classification = classify.classify_files(baseline_files, current_files, target_hashes,
                                                mode=mode)
        _merge_manifest_entry(classification, baseline, pkg, mode)
        report['classification'] = {'counts': classification.counts,
                                    'files': classification.files}

        if classification.blocked:
            report['result'] = 'STOPPED_CONFLICT'
            decisions = []
            if classification.removals or classification.renames:
                decisions.append(HD_DECISIONS['HD-09']())
            if classification.conflicts:
                decisions.append(HD_DECISIONS['HD-10']())
            report['human_decisions'] = decisions
            return report

        if mode == 'PLAN':
            report['result'] = 'PLANNED'
            report['head_sha_after'] = workspace.inspect_git(target_root, now).head_sha
            return report

        write_entries = [entry for entry in classification.files
                         if entry['decision'] == 'APPLIED']
        if not write_entries:
            # Nothing to write: the project is already current. Report it as a plan,
            # never as an "applied" change set with an empty backup.
            report['result'] = 'PLANNED'
            return report

        write_data = {entry['path']: target_bytes[entry['path']]
                      for entry in write_entries if entry['path'] != package.MANIFEST_NAME}
        write_data[package.MANIFEST_NAME] = pkg.manifest_bytes

        # --- backup -------------------------------------------------------
        problems = workspace.target_problems(target_root, list(write_data))
        if problems:
            report['result'] = 'BLOCKED'
            report['rollback']['evidence'] = ('unsafe write target: ' + '; '.join(problems[:3]))[:1000]
            return report

        shall_fail_backup = getattr(fault_injector, 'before_backup', None)
        if callable(shall_fail_backup):
            try:
                shall_fail_backup()
            except Exception as exc:  # noqa: BLE001 - a test hook must not leak
                report['result'] = 'BLOCKED'
                report['rollback']['evidence'] = f'backup could not be created: {type(exc).__name__}'
                return report

        try:
            backup = workspace.create_backup(
                target_root, list(write_data), now=now,
                metadata={
                    'installed_version': baseline.installed_version,
                    'target_version': pkg.version,
                    'head_sha': git.head_sha,
                    'dirty_paths': dirty_paths,
                    'old_manifest_sha256': baseline.manifest_sha256,
                },
            )
        except OSError as exc:
            report['result'] = 'BLOCKED'
            report['rollback']['evidence'] = f'backup could not be created: {type(exc).__name__}'
            return report
        report['backup'] = {
            'required': True,
            'location': str(backup.location),
            'created_at': backup.created_at,
            'verified': backup.verified,
        }
        if not backup.verified:
            report['result'] = 'BLOCKED'
            report['rollback']['evidence'] = 'backup verification failed; no project file was written'
            return report

        # --- apply --------------------------------------------------------
        created_dirs: list[Path] = []
        written: list[str] = []
        failure: BaseException | None = None
        try:
            for relative in sorted(write_data):
                target_path = workspace.safe_project_path(target_root, relative)
                created_dirs.extend(workspace.atomic_write(target_path, write_data[relative]))
                written.append(relative)
                hook = getattr(fault_injector, 'after_write', None)
                if callable(hook):
                    hook(relative)
            for relative in written:
                expected = workspace.sha256_bytes(write_data[relative])
                actual = workspace.sha256_bytes((target_root / relative).read_bytes())
                if actual != expected:
                    raise RuntimeError(f'post-write hash mismatch: {relative}')
        except BaseException as exc:  # noqa: BLE001 - every failure must roll back
            failure = exc

        if failure is not None:
            problems = workspace.restore_backup(target_root, backup)
            hook = getattr(fault_injector, 'after_restore', None)
            if callable(hook):
                hook(target_root)
            problems.extend(workspace.verify_restore(target_root, backup))
            if not problems:
                for directory in sorted(set(created_dirs), key=lambda item: len(str(item)), reverse=True):
                    try:
                        directory.rmdir()
                    except OSError:
                        pass
                report['rollback'] = {
                    'required': True,
                    'attempted': True,
                    'result': 'SUCCEEDED',
                    'evidence': 'restored every written path and re-verified its sha256',
                }
                report['result'] = 'ROLLED_BACK'
                report['post_apply_checks'] = [{'name': 'rollback_verification', 'status': 'PASS',
                                                'command': None, 'exit_code': None}]
            else:
                _rollback_failed(report, problems)
            report['head_sha_after'] = workspace.inspect_git(target_root, now).head_sha
            if not isinstance(failure, Exception):
                raise failure
            return report

        report['post_apply_checks'] = [
            {'name': 'applied_file_hashes', 'status': 'PASS', 'command': None, 'exit_code': None},
            {'name': 'project verification (delegated to #52)', 'status': 'NOT_RUN',
             'command': None, 'exit_code': None},
        ]
        report['rollback'] = {'required': False, 'attempted': False,
                              'result': 'NOT_NEEDED', 'evidence': None}
        report['result'] = 'APPLIED'
        report['head_sha_after'] = workspace.inspect_git(target_root, now).head_sha
        return report


def _combine_isolation(original: workspace.GitState,
                       isolated: workspace.GitState) -> workspace.GitState:
    """Keep the original dirty paths / head, but report the isolated head."""
    return workspace.GitState(
        is_repo=isolated.is_repo,
        head_sha=isolated.head_sha or original.head_sha,
        branch=isolated.branch,
        dirty_paths=original.dirty_paths,
        isolated=True,
        commands=original.commands,
    )


def _target_fields(release: sources.ReleaseInfo, asset: sources.ReleaseAsset,
                   now: datetime) -> dict:
    return {
        'repository': sources.OFFICIAL_REPOSITORY,
        'release_tag': release.tag_name,
        'version': versions.normalize_version(release.tag_name),
        'asset_name': asset.name,
        'asset_sha256': None,
        'manifest_sha256': None,
        'retrieved_at': _stamp(now),
    }


def _source_error_checks(exc: sources.SourceError) -> list[dict]:
    check = exc.check if exc.check in package.CHECK_ORDER else 'SOURCE_ALLOWLIST'
    index = package.CHECK_ORDER.index(check)
    checks = [{'check': name, 'result': 'PASS', 'detail': None}
              for name in package.CHECK_ORDER[:index]]
    checks.append({'check': check, 'result': 'FAIL',
                   'detail': exc.detail[:997] + '...' if len(exc.detail) > 1000 else exc.detail})
    checks.extend({'check': name, 'result': 'NOT_RUN',
                   'detail': f'not executed: stopped at {check}'}
                  for name in package.CHECK_ORDER[index + 1:])
    return checks


def _fail_check_after_verification(pkg: package.PackageResult, detail: str) -> list[dict]:
    checks = []
    for entry in pkg.checks:
        if entry['check'] == 'FILE_SET':
            entry = dict(entry, result='FAIL', detail=detail[:1000])
        checks.append(entry)
    return checks
