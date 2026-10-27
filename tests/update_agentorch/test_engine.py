"""End-to-end engine tests on synthetic projects and synthetic release packages.

No test reaches the network: the engine always receives an in-memory release
source. Every assertion about "zero writes" compares a full byte snapshot of the
project before and after the run.
"""
from __future__ import annotations

import json
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest import mock

from . import support

engine = support.engine
report_module = support.report


NOW = datetime(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc)

UPSTREAM_15 = {
    'AGENTS.md': b'# AGENTS v1.5\n',
    'kit/orchestration.md': b'# orchestration v1.5\n',
    'templates/task-packet.md': b'# packet v1.5\n',
    'editions/github/master.md': b'# master v1.5\n',
}

UPSTREAM_16 = {
    'AGENTS.md': b'# AGENTS v1.6\n',
    'kit/orchestration.md': b'# orchestration v1.6\n',
    'templates/task-packet.md': b'# packet v1.6\n',
    'editions/github/master.md': b'# master v1.6\n',
    'kit/new-in-16.md': b'# new in 1.6\n',
}

PROJECT_FILES = {
    'docs/PROJECT.md': b'# project config (project owned)\n',
    'src/app.py': b'print("project source")\n',
}


def prepare(tmp, *, upstream=None, project_files=None, target=None,
            baseline_format='1.5', target_format='1.6', tag='v1.6',
            manifest=True, git=False, baseline_extra=None, digest='auto'):
    upstream = dict(UPSTREAM_15 if upstream is None else upstream)
    target = dict(UPSTREAM_16 if target is None else target)
    project = support.ProjectTree(tmp)
    project.install(upstream, format_version=baseline_format, write_manifest=False)
    for rel, data in (PROJECT_FILES if project_files is None else project_files).items():
        project.write(rel, data)
    baseline_source = dict(upstream)
    baseline_source.update(baseline_extra or {})
    if manifest:
        project.write('MANIFEST.json',
                      support.manifest_bytes(support.build_manifest(
                          baseline_source, format_version=baseline_format)))
    if git:
        support.git_init(project.root)
    manifest_target = support.build_manifest(target, format_version=target_format)
    payload = support.make_zip(target, manifest_target)
    source = support.FakeReleaseSource(support.release_info(tag, payload, digest=digest), payload)
    return project, source, target, manifest_target


def run(project, source, *, mode='APPLY', **kwargs):
    return engine.run_update(project.root, mode=mode, source=source,
                             edition='github', now=NOW, **kwargs)


def file_entry(report, path):
    for item in report['classification']['files']:
        if item['path'] == path:
            return item
    raise AssertionError(f'{path} not in classification: '
                         f'{[i["path"] for i in report["classification"]["files"]]}')


class CleanUpgradeTests(unittest.TestCase):
    def test_clean_upgrade_applies_and_keeps_project_files_byte_identical(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, target, manifest = prepare(tmp)
            before = project.hashes()
            before_project_files = {path: project.read(path) for path in PROJECT_FILES}
            result = run(project, source)

            self.assertEqual(result['result'], 'APPLIED')
            for path, data in target.items():
                self.assertEqual(project.read(path), data, path)
            for path, data in before_project_files.items():
                self.assertEqual(project.read(path), data, path)
            self.assertEqual({path: project.hashes()[path] for path in PROJECT_FILES},
                             {path: before[path] for path in PROJECT_FILES})
            self.assertEqual(json.loads(project.read('MANIFEST.json')), manifest)
            self.assertEqual(file_entry(result, 'AGENTS.md')['class'], 'UPSTREAM_UNCHANGED')
            self.assertEqual(file_entry(result, 'AGENTS.md')['decision'], 'APPLIED')
            self.assertEqual(file_entry(result, 'docs/PROJECT.md')['class'], 'PROJECT_OWNED')
            self.assertEqual(file_entry(result, 'docs/PROJECT.md')['decision'], 'KEPT_LOCAL')
            self.assertEqual(file_entry(result, 'src/app.py')['class'], 'PROJECT_OWNED')
            self.assertEqual(result['workspace']['action'], 'NONE')
            self.assertTrue(result['backup']['required'])
            self.assertTrue(result['backup']['verified'])
            self.assertIsNotNone(result['backup']['location'])
            self.assertEqual(result['rollback']['result'], 'NOT_NEEDED')
            self.assertEqual(report_module.validate_report(result), [])
            commands = {entry['command']: entry['exit_code'] for entry in result['commands']}
            self.assertEqual(commands.get('github:releases/latest ctrlshiftaltdel/AgentOrch'), 0)
            self.assertEqual(commands.get('github:release-asset agentorch-20260101.zip'), 0)

    def test_plan_mode_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, target, _ = prepare(tmp)
            before = project.snapshot()
            result = run(project, source, mode='PLAN')

            self.assertEqual(result['result'], 'PLANNED')
            self.assertEqual(project.snapshot(), before)
            decisions = {item['decision'] for item in result['classification']['files']}
            self.assertNotIn('APPLIED', decisions)
            self.assertFalse(result['backup']['required'])
            self.assertFalse(result['rollback']['attempted'])
            self.assertEqual(source.download_count, 1)
            self.assertEqual(report_module.validate_report(result), [])

    def test_new_upstream_file_is_created(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            self.assertFalse(project.exists('kit/new-in-16.md'))
            result = run(project, source)
            self.assertEqual(result['result'], 'APPLIED')
            self.assertEqual(project.read('kit/new-in-16.md'), b'# new in 1.6\n')
            self.assertEqual(file_entry(result, 'kit/new-in-16.md')['class'], 'UPSTREAM_UNCHANGED')

    def test_unknown_local_file_is_kept_and_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            project.write('scripts/local-tool.py', b'# local tool\n')
            result = run(project, source)
            self.assertEqual(result['result'], 'APPLIED')
            self.assertEqual(project.read('scripts/local-tool.py'), b'# local tool\n')
            item = file_entry(result, 'scripts/local-tool.py')
            self.assertEqual(item['class'], 'UNKNOWN_FILE')
            self.assertEqual(item['decision'], 'KEPT_LOCAL')

    def test_writes_stay_inside_the_project(self):
        with tempfile.TemporaryDirectory() as tmp:
            nested = Path(tmp) / 'project'
            project, source, _, _ = prepare(nested)
            run(project, source)
            self.assertEqual(sorted(p.name for p in Path(tmp).iterdir()), ['project'])


class ClassificationBlockTests(unittest.TestCase):
    def test_two_sided_conflict_stops_with_zero_writes(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, target, _ = prepare(tmp)
            project.write('AGENTS.md', b'# local edit\n')
            before = project.snapshot()
            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_CONFLICT')
            self.assertEqual(project.snapshot(), before)
            item = file_entry(result, 'AGENTS.md')
            self.assertEqual(item['class'], 'THREE_WAY_CONFLICT')
            self.assertEqual(item['decision'], 'CONFLICT_REPORTED')
            self.assertEqual(result['classification']['counts']['THREE_WAY_CONFLICT'], 1)
            decision_ids = {entry['decision_id'] for entry in result['human_decisions']}
            self.assertIn('HD-10', decision_ids)
            self.assertEqual(report_module.validate_report(result), [])

    def test_project_owned_file_is_never_overwritten_even_if_baseline_lists_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, target, manifest = prepare(
                tmp,
                target={**UPSTREAM_16, 'docs/PROJECT.md': b'# upstream tries to own this\n'},
                baseline_extra={'docs/PROJECT.md': PROJECT_FILES['docs/PROJECT.md']},
            )
            before = project.read('docs/PROJECT.md')
            result = run(project, source)
            self.assertEqual(result['result'], 'APPLIED')
            self.assertEqual(project.read('docs/PROJECT.md'), before)
            item = file_entry(result, 'docs/PROJECT.md')
            self.assertEqual(item['class'], 'PROJECT_OWNED')
            self.assertEqual(item['decision'], 'KEPT_LOCAL')
            self.assertIn('PROJECT_OWNED', result['classification']['counts'])

    def test_upstream_removal_blocks_and_keeps_the_file(self):
        target = {path: data for path, data in UPSTREAM_16.items()
                  if path != 'templates/task-packet.md'}
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, target=target)
            before = project.snapshot()
            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_CONFLICT')
            self.assertEqual(project.snapshot(), before)
            item = file_entry(result, 'templates/task-packet.md')
            self.assertEqual(item['class'], 'UPSTREAM_REMOVAL')
            self.assertEqual(item['decision'], 'KEPT_LOCAL')
            self.assertIn('HD-09', {entry['decision_id'] for entry in result['human_decisions']})

    def test_upstream_rename_is_reported_and_not_applied(self):
        upstream = {'kit/old.md': b'# same bytes\n'}
        target = {'kit/new.md': b'# same bytes\n'}
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, upstream=upstream, target=target)
            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_CONFLICT')
            self.assertTrue(project.exists('kit/old.md'))
            self.assertFalse(project.exists('kit/new.md'))
            self.assertEqual(file_entry(result, 'kit/old.md')['class'], 'UPSTREAM_RENAME')
            self.assertEqual(file_entry(result, 'kit/new.md')['class'], 'UPSTREAM_RENAME')
            self.assertIn('HD-09', {entry['decision_id'] for entry in result['human_decisions']})

    def test_missing_baseline_stops_before_any_download(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, manifest=False)
            before = project.snapshot()
            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_INVALID_BASELINE')
            self.assertEqual(result['baseline']['baseline_status'], 'MISSING')
            self.assertIn('inventory', result['baseline']['notes'])
            self.assertEqual(source.fetch_count, 0)
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(report_module.validate_report(result), [])

    def test_unreadable_baseline_is_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            project.write('MANIFEST.json', b'{not json')
            result = run(project, source)
            self.assertEqual(result['result'], 'STOPPED_INVALID_BASELINE')
            self.assertEqual(result['baseline']['baseline_status'], 'INVALID_BASELINE')
            self.assertEqual(source.fetch_count, 0)

    def test_unsupported_baseline_schema_is_invalid(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            manifest = json.loads(project.read('MANIFEST.json'))
            manifest['schema_version'] = 2
            project.write('MANIFEST.json', support.manifest_bytes(manifest))
            result = run(project, source)
            self.assertEqual(result['result'], 'STOPPED_INVALID_BASELINE')


class UnsafePackageTests(unittest.TestCase):
    def test_bad_manifest_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = dict(UPSTREAM_16)
            project = support.ProjectTree(Path(tmp) / 'project')
            project.install(UPSTREAM_15, format_version='1.5', write_manifest=False)
            project.write('MANIFEST.json', support.manifest_bytes(
                support.build_manifest(UPSTREAM_15)))
            manifest = support.build_manifest(target)
            manifest['schema_version'] = 2
            payload = support.make_zip(target, manifest)
            source = support.FakeReleaseSource(support.release_info('v1.6', payload), payload)
            before = project.snapshot()

            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_UNSAFE_PACKAGE')
            self.assertEqual(result['target']['verification']['status'], 'UNSAFE_PACKAGE')
            checks = {c['check']: c['result'] for c in result['target']['verification']['checks']}
            self.assertEqual(checks['MANIFEST_SCHEMA'], 'FAIL')
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(report_module.validate_report(result), [])

    def test_path_traversal_member_writes_nothing_outside(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, _, _, _ = prepare(tmp)
            files = dict(UPSTREAM_16)
            manifest = support.build_manifest(files)
            payload = support.make_zip(
                files, manifest,
                raw_members=(('agentorch-20260101/../escape.txt', b'evil', 0o644),))
            source = support.FakeReleaseSource(support.release_info('v1.6', payload), payload)
            before = project.snapshot()

            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_UNSAFE_PACKAGE')
            checks = {c['check']: c['result'] for c in result['target']['verification']['checks']}
            self.assertEqual(checks['MEMBER_PATHS'], 'FAIL')
            self.assertEqual(project.snapshot(), before)
            self.assertFalse((Path(tmp) / 'escape.txt').exists())

    def test_symlink_member_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, _, _, _ = prepare(tmp)
            files = dict(UPSTREAM_16)
            manifest = support.build_manifest(files)
            payload = support.make_zip(
                files, manifest,
                raw_members=(('agentorch-20260101/kit/link', b'../../AGENTS.md', 0o120777),))
            source = support.FakeReleaseSource(support.release_info('v1.6', payload), payload)
            result = run(project, source)
            self.assertEqual(result['result'], 'STOPPED_UNSAFE_PACKAGE')
            checks = {c['check']: c['result'] for c in result['target']['verification']['checks']}
            self.assertEqual(checks['SYMLINKS'], 'FAIL')

    def test_non_official_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, _, target, manifest = prepare(tmp)
            payload = support.make_zip(target, manifest)
            source = support.FakeReleaseSource(
                support.release_info('v1.6', payload, repository='someone/AgentOrch'), payload)
            before = project.snapshot()
            result = run(project, source)

            self.assertEqual(result['result'], 'STOPPED_UNSAFE_PACKAGE')
            checks = {c['check']: c['result'] for c in result['target']['verification']['checks']}
            self.assertEqual(checks['SOURCE_ALLOWLIST'], 'FAIL')
            self.assertEqual(project.snapshot(), before)

    def test_network_failure_is_blocked_and_not_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            source.fetch_error = OSError('synthetic dns failure')
            before = project.snapshot()
            result = run(project, source)

            self.assertEqual(result['result'], 'BLOCKED')
            self.assertEqual(result['target']['verification']['status'], 'NOT_RUN')
            self.assertTrue(all(entry['result'] == 'NOT_RUN'
                                for entry in result['target']['verification']['checks']))
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(result['commands'][-1]['exit_code'], 1)
            self.assertEqual(report_module.validate_report(result), [])

    def test_downgrade_is_blocked(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, tag='v1.4', target_format='1.4')
            before = project.snapshot()
            result = run(project, source)

            self.assertEqual(result['result'], 'BLOCKED')
            self.assertEqual(project.snapshot(), before)
            self.assertIn('HD-16', {entry['decision_id'] for entry in result['human_decisions']})
            self.assertEqual(report_module.validate_report(result), [])

    def test_equal_version_is_blocked_as_a_reinstall(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, tag='v1.5', target_format='1.5')
            result = run(project, source)
            self.assertEqual(result['result'], 'BLOCKED')
            self.assertIn('HD-16', {entry['decision_id'] for entry in result['human_decisions']})

    def test_two_and_three_part_tags_apply(self):
        for tag in ('v1.6', 'v1.6.0'):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as tmp:
                project, source, target, _ = prepare(tmp, tag=tag)
                result = run(project, source)
                self.assertEqual(result['result'], 'APPLIED')
                self.assertEqual(result['target']['release_tag'], tag)
                self.assertEqual(result['target']['version'], '1.6.0')


class WorkspaceTests(unittest.TestCase):
    def test_dirty_git_worktree_is_isolated_and_left_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, target, _ = prepare(tmp, git=True)
            project.write('docs/PROJECT.md', b'# local uncommitted edit\n')
            head = support.git(project.root, 'rev-parse', 'HEAD').stdout.strip()
            result = run(project, source)

            self.assertEqual(result['result'], 'APPLIED')
            self.assertTrue(result['workspace']['dirty'])
            self.assertIn('docs/PROJECT.md', result['workspace']['dirty_paths'])
            self.assertEqual(result['workspace']['action'], 'ISOLATED_BRANCH')
            # the original working tree is untouched, including the dirty file
            self.assertEqual(project.read('docs/PROJECT.md'), b'# local uncommitted edit\n')
            self.assertEqual(project.read('AGENTS.md'), UPSTREAM_15['AGENTS.md'])
            self.assertEqual(support.git(project.root, 'stash', 'list').stdout, '')
            # the isolated worktree carries the update
            worktree = project.root / result['operation']['worktree']
            self.assertTrue(worktree.is_dir())
            self.assertEqual((worktree / 'AGENTS.md').read_bytes(), target['AGENTS.md'])
            self.assertEqual(result['head_sha_before'], head)
            self.assertEqual(report_module.validate_report(result), [])

    def test_clean_git_worktree_records_head_sha_and_commands(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, git=True)
            head = support.git(project.root, 'rev-parse', 'HEAD').stdout.strip()
            result = run(project, source)

            self.assertEqual(result['result'], 'APPLIED')
            self.assertEqual(result['workspace']['action'], 'NONE')
            self.assertEqual(result['baseline']['head_sha'], head)
            self.assertEqual(result['head_sha_before'], head)
            self.assertEqual(result['head_sha_after'], head)
            self.assertTrue(result['commands'])
            for entry in result['commands']:
                self.assertIsInstance(entry['exit_code'], int)
            self.assertEqual(report_module.validate_report(result), [])

    def test_half_update_rolls_back_and_verifies(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, git=True)
            before = project.snapshot()
            fault = support.FaultPlan(fail_after_writes=2)
            result = run(project, source, fault_injector=fault)

            self.assertEqual(result['result'], 'ROLLED_BACK')
            self.assertTrue(result['rollback']['attempted'])
            self.assertEqual(result['rollback']['result'], 'SUCCEEDED')
            self.assertTrue(result['backup']['verified'])
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(report_module.validate_report(result), [])

    def test_rollback_failure_is_blocked(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, git=True)
            fault = support.FaultPlan(fail_after_writes=2,
                                      corrupt_after_restore='AGENTS.md')
            result = run(project, source, fault_injector=fault)

            self.assertEqual(result['result'], 'BLOCKED')
            self.assertEqual(result['rollback']['result'], 'FAILED')
            self.assertIn('HD-11', {entry['decision_id'] for entry in result['human_decisions']})
            self.assertEqual(report_module.validate_report(result), [])

    def test_backup_failure_stops_before_any_write(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, git=True)
            before = project.snapshot()
            fault = support.FaultPlan(fail_before_backup=True)
            result = run(project, source, fault_injector=fault)

            self.assertEqual(result['result'], 'BLOCKED')
            self.assertEqual(project.snapshot(), before)
            self.assertFalse(result['rollback']['attempted'])
            self.assertEqual(report_module.validate_report(result), [])

    def test_backup_creation_error_blocks_without_writes(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            before = project.snapshot()
            with mock.patch.object(support.workspace, 'create_backup',
                                   side_effect=OSError('synthetic disk full')):
                result = run(project, source)
            self.assertEqual(result['result'], 'BLOCKED')
            self.assertEqual(project.snapshot(), before)
            self.assertFalse(result['rollback']['attempted'])
            self.assertIn('backup', result['rollback']['evidence'])
            self.assertEqual(report_module.validate_report(result), [])

    def test_symlinked_write_target_is_blocked_and_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            outside = Path(tmp) / 'outside.txt'
            outside.write_bytes(b'outside data\n')
            link = project.path('kit/new-in-16.md')
            link.symlink_to(outside)
            before = project.snapshot()

            result = run(project, source)

            self.assertEqual(result['result'], 'BLOCKED')
            self.assertTrue(link.is_symlink())
            self.assertEqual(outside.read_bytes(), b'outside data\n')
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(report_module.validate_report(result), [])


class CliTests(unittest.TestCase):
    def test_read_edition_accepts_both_colon_styles(self):
        with tempfile.TemporaryDirectory() as tmp:
            project = Path(tmp)
            (project / 'docs').mkdir()
            (project / 'docs' / 'PROJECT.md').write_text(
                '- `edition`：`github`\n', encoding='utf-8')
            self.assertEqual(support.cli.read_edition(project), 'github')
            (project / 'docs' / 'PROJECT.md').write_text(
                '- edition: local\n', encoding='utf-8')
            self.assertEqual(support.cli.read_edition(project), 'local')

    def test_missing_or_invalid_edition_fails_closed(self):
        import contextlib
        import io as _io

        with tempfile.TemporaryDirectory() as tmp:
            project = Path(tmp)
            with contextlib.redirect_stderr(_io.StringIO()):
                self.assertEqual(support.cli.main(['--project', tmp]), 1)
            (project / 'docs').mkdir()
            (project / 'docs' / 'PROJECT.md').write_text(
                '- `edition`: `1`\n', encoding='utf-8')
            with contextlib.redirect_stderr(_io.StringIO()):
                self.assertEqual(support.cli.main(['--project', tmp]), 1)


class ReportEvidenceTests(unittest.TestCase):
    def test_report_carries_versions_sha_commands_and_rollback_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, git=True)
            result = run(project, source)

            self.assertEqual(result['baseline']['installed_version'], '1.5')
            self.assertEqual(result['baseline']['installed_format_version'], '1.5')
            self.assertEqual(result['target']['release_tag'], 'v1.6')
            self.assertEqual(result['target']['version'], '1.6.0')
            self.assertEqual(result['target']['repository'], 'ctrlshiftaltdel/AgentOrch')
            self.assertEqual(len(result['target']['asset_sha256']), 64)
            self.assertEqual(len(result['target']['manifest_sha256']), 64)
            self.assertEqual(sum(result['classification']['counts'].values()),
                             len(result['classification']['files']))
            self.assertEqual(result['classification']['counts']['THREE_WAY_CONFLICT'], 0)
            self.assertTrue(result['commands'])
            self.assertTrue(result['backup']['location'])
            self.assertIsNotNone(result['backup']['created_at'])
            self.assertEqual(result['rollback']['result'], 'NOT_NEEDED')
            self.assertTrue(result['post_apply_checks'])
            self.assertEqual(report_module.validate_report(result), [])

    def test_engine_never_opens_a_network_socket_with_an_injected_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            with mock.patch('socket.socket', side_effect=AssertionError('network used')):
                result = run(project, source)
            self.assertEqual(result['result'], 'APPLIED')
            self.assertEqual(source.fetch_count, 1)
            self.assertEqual(source.download_count, 1)

    def test_plan_and_apply_reports_use_distinct_ids(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            plan = run(project, source, mode='PLAN')
            apply = run(project, source)
            self.assertNotEqual(plan['report_id'], apply['report_id'])
            self.assertRegex(plan['report_id'], r'^UPD-[0-9]{8}T[0-9]{6}Z-[a-f0-9]{6}$')


if __name__ == '__main__':
    unittest.main()
