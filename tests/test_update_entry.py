"""AC-1 / AC-2: the natural-language "更新 AgentOrch" entry.

Routing is delegated to ``scripts/self_improvement/planning.py::route_request``;
these tests pin the update-specific boundary (triggers, ambiguous wording, plain
requests and the self-improvement split) and the two edition delivery shapes.
No test reaches the network: the engine receives an in-memory release source.
"""
from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TESTS = Path(__file__).resolve().parent
SCRIPTS = ROOT / 'scripts'
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
if str(TESTS) not in sys.path:
    sys.path.insert(0, str(TESTS))


def _load(name, path, *, package_dir=None):
    spec = importlib.util.spec_from_file_location(
        name, path, submodule_search_locations=[str(package_dir)] if package_dir else None)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


_entry_alias = 'agentorch_update_entry_under_test'
if _entry_alias not in sys.modules:
    _load(_entry_alias, SCRIPTS / 'update_agentorch' / '__init__.py',
          package_dir=SCRIPTS / 'update_agentorch')
entry = importlib.import_module(f'{_entry_alias}.entry')

if 'agentorch_update_entry_support' not in sys.modules:
    _load('agentorch_update_entry_support', TESTS / 'update_agentorch' / 'support.py')
support = sys.modules['agentorch_update_entry_support']

planning = importlib.import_module('scripts.self_improvement.planning')

NOW = datetime(2026, 1, 2, 3, 4, 5, tzinfo=timezone.utc)
EDITION_CONFIG = {
    'local': '# project config\n\n- `edition`: local\n',
    'github': '# project config\n\n- `edition`: github\n',
}

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
    'src/app.py': b'print("project source")\n',
    '.local/notes.txt': b'private machine note\n',
}


def prepare_project(tmp, edition):
    project = support.ProjectTree(tmp)
    files = dict(UPSTREAM_15)
    files['docs/PROJECT.md'] = EDITION_CONFIG[edition].encode('utf-8')
    project.install(files, format_version='1.5')
    for rel, data in PROJECT_FILES.items():
        project.write(rel, data)
    manifest = support.build_manifest(UPSTREAM_16, format_version='1.6')
    payload = support.make_zip(UPSTREAM_16, manifest)
    source = support.FakeReleaseSource(support.release_info('v1.6', payload), payload)
    return project, source


class TriggerRoutingTests(unittest.TestCase):
    def test_trigger_phrases_start_an_update(self):
        for text in ('更新 AgentOrch', '升级 AgentOrch',
                     '把项目中的 AgentOrch 更新到最新版',
                     '把项目里的 AgentOrch 升级到最新版本',
                     'update AgentOrch', 'upgrade agentorch'):
            with self.subTest(text=text):
                decision = entry.route(text)
                self.assertEqual(decision['route'], planning.ROUTE_UPDATE_AGENTORCH)
                self.assertTrue(decision['starts_update'])
                self.assertFalse(decision['needs_clarification'])
                self.assertFalse(decision['starts_self_improvement'])

    def test_plain_requests_do_not_start_an_update(self):
        for text in ('更新一下文档', '更新依赖', '更新这个 Issue', '帮我更新 README',
                     '更新项目文档', '今天天气怎么样'):
            with self.subTest(text=text):
                decision = entry.route(text)
                self.assertFalse(decision['starts_update'])
                self.assertEqual(decision['route'], planning.ROUTE_UNRELATED)

    def test_ambiguous_object_asks_once_instead_of_starting(self):
        for text in ('更新 AgentOrch 的文档', '更新 AgentOrch 的依赖',
                     '升级 AgentOrch 版本号'):
            with self.subTest(text=text):
                decision = entry.route(text)
                self.assertEqual(decision['route'], planning.ROUTE_UPDATE_AGENTORCH)
                self.assertTrue(decision['needs_clarification'])
                self.assertFalse(decision['starts_update'])
                self.assertIn('套件本身', decision['clarification_question'])

    def test_version_question_is_explained_only(self):
        decision = entry.route('AgentOrch 最新版本是多少？')
        self.assertEqual(decision['route'], planning.ROUTE_EXPLAIN_ONLY)
        self.assertFalse(decision['starts_update'])

    def test_self_improvement_is_a_different_route(self):
        decision = entry.route('进行自我完善')
        self.assertEqual(decision['route'], planning.ROUTE_SELF_IMPROVEMENT)
        self.assertTrue(decision['starts_self_improvement'])
        self.assertFalse(decision['starts_update'])

    def test_prepare_does_not_start_an_engine_for_a_different_route(self):
        class ExplodingSource:
            fetch_count = 0

            def fetch_release(self):
                self.fetch_count += 1
                raise AssertionError('the update engine must not be called')

        with tempfile.TemporaryDirectory() as tmp:
            project = support.ProjectTree(tmp)
            project.write('docs/PROJECT.md', EDITION_CONFIG['github'])
            before = project.snapshot()
            source = ExplodingSource()
            for text in ('进行自我完善', '更新一下文档', '更新 AgentOrch 的文档'):
                with self.subTest(text=text):
                    result = entry.prepare_update(project.root, text, source=source)
                    self.assertIn(result['status'], ('NOT_UPDATE', 'NEEDS_CLARIFICATION'))
                    self.assertIsNone(result['engine_report'])
                    self.assertIsNone(result['delivery'])
                    self.assertIsNone(result['gates'])
            self.assertEqual(source.fetch_count, 0)
            self.assertEqual(project.snapshot(), before)


class EnvironmentInspectionTests(unittest.TestCase):
    def test_reads_edition_format_baseline_and_workspace(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source = prepare_project(tmp, 'github')
            environment = entry.inspect_environment(project.root)
            self.assertEqual(environment['edition'], 'github')
            self.assertIsNone(environment['edition_error'])
            self.assertEqual(environment['format_version'], '1.5')
            self.assertEqual(environment['installed_version'], '1.5')
            self.assertEqual(environment['baseline_status'], 'VALID')
            self.assertFalse(environment['workspace']['is_git_repo'])
            self.assertFalse(environment['workspace']['dirty'])

    def test_missing_edition_fails_closed_before_the_engine(self):
        with tempfile.TemporaryDirectory() as tmp:
            project = support.ProjectTree(tmp)
            project.install(UPSTREAM_15, format_version='1.5')
            source = support.FakeReleaseSource(support.release_info('v1.6', b''), b'')
            result = entry.prepare_update(project.root, '更新 AgentOrch', source=source)
            self.assertEqual(result['status'], 'STOPPED_INVALID_EDITION')
            self.assertIn('edition', result['message'])
            self.assertEqual(source.fetch_count, 0)
            self.assertIsNone(result['engine_report'])


class EditionDeliveryTests(unittest.TestCase):
    def _prepare(self, tmp, edition):
        project, source = prepare_project(tmp, edition)
        return project, entry.prepare_update(
            project.root, '更新 AgentOrch', mode='PLAN', source=source, now=NOW)

    def test_local_delivery_is_a_task_packet_and_never_a_pr(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, payload = self._prepare(tmp, 'local')
            self.assertEqual(payload['status'], 'PREPARED')
            self.assertEqual(payload['engine_report']['result'], 'PLANNED')
            self.assertEqual(payload['environment']['edition'], 'local')
            delivery = payload['delivery']
            self.assertEqual(delivery['carrier'], 'local_task_database')
            self.assertEqual(delivery['automation'], 'manual')
            self.assertEqual(delivery['writes'], [])
            self.assertIn('task_packet', delivery)
            self.assertNotIn('draft_pr', delivery)
            self.assertNotIn('issue', delivery)
            self.assertIn('orchestrator.py', delivery['task_packet']['task id'])
            self.assertTrue(delivery['next_steps'])
            self.assertIn('orchestrator.py', ' '.join(delivery['next_steps']))

    def test_github_delivery_is_an_issue_and_draft_pr_and_never_the_task_db(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, payload = self._prepare(tmp, 'github')
            self.assertEqual(payload['status'], 'PREPARED')
            self.assertEqual(payload['engine_report']['result'], 'PLANNED')
            self.assertEqual(payload['environment']['edition'], 'github')
            delivery = payload['delivery']
            self.assertEqual(delivery['carrier'], 'issue_and_draft_pr')
            self.assertEqual(delivery['automation'], 'manual')
            self.assertEqual(delivery['writes'], [])
            self.assertIn('issue', delivery)
            self.assertIn('draft_pr', delivery)
            self.assertNotIn('task_packet', delivery)
            self.assertEqual(delivery['draft_pr']['base'], 'main')
            self.assertTrue(delivery['draft_pr']['head'].startswith('task/'))
            self.assertNotIn('orchestrator.py', ' '.join(delivery['next_steps']))

    def test_both_editions_share_the_same_gate_record(self):
        with tempfile.TemporaryDirectory() as local_tmp, \
                tempfile.TemporaryDirectory() as github_tmp:
            _, local = self._prepare(local_tmp, 'local')
            _, github = self._prepare(github_tmp, 'github')
            local_gates, github_gates = local['gates'], github['gates']
            self.assertEqual(set(local_gates), set(github_gates))
            self.assertTrue(local_gates['requires_review'])
            self.assertFalse(local_gates['self_approval_allowed'])
            self.assertFalse(local_gates['auto_merge'])
            self.assertTrue(local_gates['report_valid'])
            for key in ('mode', 'engine_result', 'conflicts', 'human_decisions',
                        'requires_review', 'self_approval_allowed', 'auto_merge',
                        'report_valid'):
                with self.subTest(key=key):
                    self.assertEqual(local_gates[key], github_gates[key])
            self.assertEqual(local_gates['edition'], 'local')
            self.assertEqual(github_gates['edition'], 'github')

    def test_cli_payload_can_be_written_to_a_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source = prepare_project(tmp, 'github')
            output = Path(tmp) / 'entry.json'
            # The CLI would create a real GitHubReleaseSource; exercise the same
            # serialization path by rendering the dict the CLI prints.
            payload = entry.prepare_update(project.root, '更新 AgentOrch', source=source, now=NOW)
            import json
            output.write_text(json.dumps(payload, ensure_ascii=False), encoding='utf-8')
            self.assertIn('"status": "PREPARED"', output.read_text(encoding='utf-8'))


if __name__ == '__main__':
    unittest.main()
