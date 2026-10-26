"""AC-5: synthetic old-version project upgraded through the natural-language entry.

The fixture is a *synthetic* project plus a *synthetic* in-memory release package.
No network, token or published artifact is involved. The test proves the entry
identifies edition / Format / baseline, applies only conflict-free upstream files,
and keeps project-owned / unknown files byte-identical.
"""
from __future__ import annotations

import importlib
import tempfile
import unittest
from datetime import datetime, timezone

from . import support

entry = importlib.import_module(f'{support._ENGINE_ALIAS}.entry')

NOW = datetime(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc)
CONFIG = b'# project config\n\n- `edition`: github\n'

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
    'docs/PROJECT.md': CONFIG,
    'src/app.py': b'print("project source")\n',
    'custom/keep.txt': b'project-specific note\n',
    '.local/notes.txt': b'private machine note\n',
}


def prepare(tmp, *, baseline_format='1.5', old=None, target=None,
            target_format='1.6', manifest=True, git=False):
    project = support.ProjectTree(tmp)
    old = dict(UPSTREAM_15 if old is None else old)
    target = dict(UPSTREAM_16 if target is None else target)
    project.install(old, format_version=baseline_format, write_manifest=manifest)
    for rel, data in PROJECT_FILES.items():
        if rel != 'docs/PROJECT.md':
            project.write(rel, data)
    project.write('docs/PROJECT.md', (old.get('docs/PROJECT.md') or CONFIG))
    if git:
        support.git_init(project.root)
    manifest_target = support.build_manifest(target, format_version=target_format)
    payload = support.make_zip(target, manifest_target)
    source = support.FakeReleaseSource(
        support.release_info('v1.6', payload), payload)
    return project, source, target, manifest_target


def run_entry(project, source, *, mode='APPLY', **kwargs):
    return entry.prepare_update(
        project.root, '更新 AgentOrch', mode=mode, source=source, now=NOW, **kwargs)


def file_entry(report, path):
    for item in report['classification']['files']:
        if item['path'] == path:
            return item
    raise AssertionError(f'{path} not in classification: '
                         f'{[item["path"] for item in report["classification"]["files"]]}')


class CleanOldProjectTests(unittest.TestCase):
    def assert_project_files_unchanged(self, project, before):
        for rel in PROJECT_FILES:
            self.assertEqual(project.read(rel), before[rel], rel)

    def test_synthetic_1_5_project_upgrades_and_keeps_project_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, target, manifest = prepare(tmp)
            before = project.snapshot()
            payload = run_entry(project, source)

            self.assertEqual(payload['status'], 'PREPARED')
            self.assertEqual(payload['engine_report']['result'], 'APPLIED')
            self.assertEqual(payload['environment']['format_version'], '1.5')
            for rel, data in target.items():
                self.assertEqual(project.read(rel), data, rel)
            self.assert_project_files_unchanged(project, before)
            self.assertEqual(project.read('MANIFEST.json'),
                             support.manifest_bytes(manifest))
            self.assertEqual(file_entry(payload['engine_report'], 'AGENTS.md')['decision'],
                             'APPLIED')
            self.assertIn(file_entry(payload['engine_report'], 'docs/PROJECT.md'),
                          payload['engine_report']['classification']['files'])
            self.assertTrue(payload['gates']['report_valid'])
            self.assertTrue(payload['gates']['requires_review'])

    def test_synthetic_1_4_project_upgrades(self):
        with tempfile.TemporaryDirectory() as tmp:
            old = dict(UPSTREAM_15)
            old['PLAN.md'] = b'# legacy root plan\n'
            project, source, target, _ = prepare(tmp, baseline_format='1.4', old=old)
            before = project.snapshot()
            payload = run_entry(project, source)
            self.assertEqual(payload['engine_report']['result'], 'APPLIED')
            self.assertEqual(payload['environment']['format_version'], '1.4')
            self.assertEqual(project.read('PLAN.md'), b'# legacy root plan\n')
            self.assertEqual(project.read('AGENTS.md'), target['AGENTS.md'])
            self.assert_project_files_unchanged(project, before)

    def test_1_3_style_root_readme_is_a_removal_and_is_never_deleted(self):
        with tempfile.TemporaryDirectory() as tmp:
            old = dict(UPSTREAM_15)
            old['README.md'] = b'# legacy root readme\n'
            project, source, target, _ = prepare(tmp, baseline_format='1.3', old=old)
            before = project.snapshot()
            payload = run_entry(project, source)

            self.assertEqual(payload['engine_report']['result'], 'STOPPED_CONFLICT')
            self.assertEqual(payload['engine_report']['classification']['counts']
                             ['UPSTREAM_REMOVAL'], 1)
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(project.read('README.md'), b'# legacy root readme\n')
            self.assertEqual(payload['gates']['conflicts'], ['README.md'])
            self.assertIn('HD-09', payload['gates']['human_decisions'])


class StopConditionsTests(unittest.TestCase):
    def test_local_upstream_conflict_stops_with_zero_writes(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            project.write('AGENTS.md', b'# local AGENTS edit\n')
            before = project.snapshot()
            payload = run_entry(project, source)

            self.assertEqual(payload['engine_report']['result'], 'STOPPED_CONFLICT')
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(project.read('AGENTS.md'), b'# local AGENTS edit\n')
            self.assertEqual(file_entry(payload['engine_report'], 'AGENTS.md')['class'],
                             'THREE_WAY_CONFLICT')
            self.assertIn('HD-10', payload['gates']['human_decisions'])

    def test_missing_baseline_stops_before_any_network_call(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, manifest=False)
            before = project.snapshot()
            payload = run_entry(project, source)

            self.assertEqual(payload['engine_report']['result'], 'STOPPED_INVALID_BASELINE')
            self.assertEqual(source.fetch_count, 0)
            self.assertEqual(project.snapshot(), before)
            self.assertEqual(payload['environment']['baseline_status'], 'MISSING')

    def test_dirty_worktree_stops_when_isolation_is_disabled(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp, git=True)
            project.write('src/app.py', b'print("uncommitted edit")\n')
            before = project.snapshot()
            payload = run_entry(project, source, isolation=False)

            self.assertEqual(payload['engine_report']['result'], 'STOPPED_DIRTY_WORKTREE')
            self.assertEqual(source.fetch_count, 0)
            self.assertEqual(project.snapshot(), before)
            self.assertIn('src/app.py',
                          payload['engine_report']['workspace']['dirty_paths'])
            self.assertIn('HD-12', payload['gates']['human_decisions'])

    def test_upstream_rename_is_reported_and_not_applied(self):
        with tempfile.TemporaryDirectory() as tmp:
            old = dict(UPSTREAM_15)
            old['kit/old.md'] = b'# moved verbatim\n'
            target = dict(UPSTREAM_16)
            target['kit/new.md'] = b'# moved verbatim\n'
            project, source, _, _ = prepare(tmp, old=old, target=target)
            before = project.snapshot()
            payload = run_entry(project, source)

            self.assertEqual(payload['engine_report']['result'], 'STOPPED_CONFLICT')
            self.assertEqual(project.snapshot(), before)
            self.assertTrue(project.exists('kit/old.md'))
            self.assertFalse(project.exists('kit/new.md'))
            self.assertIn('HD-09', payload['gates']['human_decisions'])

    def test_network_failure_is_blocked_and_checks_stay_not_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source, _, _ = prepare(tmp)
            source.fetch_error = OSError('synthetic network unavailable')
            before = project.snapshot()
            payload = run_entry(project, source)

            report = payload['engine_report']
            self.assertEqual(report['result'], 'BLOCKED')
            self.assertEqual(report['target']['verification']['status'], 'NOT_RUN')
            self.assertTrue(all(check['result'] == 'NOT_RUN'
                                for check in report['target']['verification']['checks']))
            self.assertEqual(project.snapshot(), before)
            self.assertTrue(payload['gates']['report_valid'])


if __name__ == '__main__':
    unittest.main()
