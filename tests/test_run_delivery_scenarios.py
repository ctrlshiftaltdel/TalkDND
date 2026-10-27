"""Tests for scripts/run_delivery_scenarios.py.

The runner is the acceptance step that proves the kit stops the failures it
documents, so the runner itself has to be tested: a scenario that crashes must
be recorded and must not end the run, and the exit code has to tell "all gates
held" apart from "a gate failed" and from "the runner could not run at all".
"""
import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'run_delivery_scenarios.py'
spec = importlib.util.spec_from_file_location('run_delivery_scenarios', SCRIPT)
runner = importlib.util.module_from_spec(spec)
# Register before exec: the runner uses `from __future__ import annotations`,
# and dataclass processing resolves KW_ONLY through sys.modules[__name__].
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)

ROOT = SCRIPT.parents[1]
SCENARIO_DOC = ROOT / runner.SCENARIO_DOC


def scenario(scenario_id, fn, kind='negative'):
    return runner.Scenario(scenario_id, 'D', kind, f'synthetic {scenario_id}',
                           'synthetic precondition', 'synthetic defect',
                           'synthetic gate', 'synthetic evidence',
                           'temporary workdir', fn)


def passing(ctx):
    return ['the gate held']


def crashing(ctx):
    raise ZeroDivisionError('synthetic non-assertion failure')


def failing(ctx):
    raise runner.ScenarioFailed('the expected gate did not fire')


class RegistryTests(unittest.TestCase):
    def test_scenario_ids_are_unique_and_groups_are_complete(self):
        ids = runner.scenario_ids()
        self.assertEqual(len(ids), len(set(ids)))
        for group in 'ABCD':
            self.assertTrue(any(item.group == group for item in runner.SCENARIOS),
                            f'group {group} has no scenario')

    def test_positive_controls_and_negative_groups_both_exist(self):
        kinds = {item.kind for item in runner.SCENARIOS}
        self.assertEqual(kinds, {'positive', 'negative'})
        positives = [item for item in runner.SCENARIOS if item.kind == 'positive']
        self.assertGreaterEqual(len(positives), 4)

    def test_every_scenario_declares_its_contract(self):
        for item in runner.SCENARIOS:
            for field in ('id', 'group', 'kind', 'title', 'precondition', 'defect',
                          'gate', 'evidence', 'cleanup'):
                self.assertTrue(getattr(item, field), f'{item.id} has an empty {field}')
            self.assertIn(item.kind, ('positive', 'negative'))
            self.assertIn(item.group, 'ABCD')

    def test_select_rejects_unknown_ids(self):
        with self.assertRaises(runner.ToolError):
            runner.select_scenarios(['NOPE'])

    @unittest.skipUnless(SCENARIO_DOC.is_file(), 'repository-only document is not shipped')
    def test_scenario_document_lists_every_scenario(self):
        text = SCENARIO_DOC.read_text(encoding='utf-8')
        for item in runner.SCENARIOS:
            self.assertIn(f'| {item.id} |', text,
                          f'{item.id} is missing from {runner.SCENARIO_DOC}')


class FullRunTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workdir = tempfile.TemporaryDirectory()
        cls.vk = runner._load_module('test_validate_kit', runner.SCRIPTS / 'validate_kit.py')
        cls.orch = runner._load_module('test_orchestrator', runner.SCRIPTS / 'orchestrator.py')
        cls.results = runner.run_scenarios(runner.SCENARIOS, cls.workdir.name,
                                           vk=cls.vk, orch=cls.orch)

    @classmethod
    def tearDownClass(cls):
        cls.workdir.cleanup()

    def test_every_scenario_passes(self):
        broken = [f'{r.scenario.id}: {r.status} {r.error.get("message", "")}'
                  for r in self.results if r.status != 'PASS']
        self.assertEqual(broken, [])

    def test_each_scenario_leaves_evidence(self):
        for result in self.results:
            self.assertTrue(result.evidence, f'{result.scenario.id} left no evidence')

    def test_sandboxes_are_removed_after_the_run(self):
        leftovers = list(Path(self.workdir.name).iterdir())
        self.assertEqual(leftovers, [], f'temporary sandboxes were not cleaned: {leftovers}')

    def test_exit_code_is_zero_when_all_pass(self):
        self.assertEqual(runner._exit_code(self.results), runner.EXIT_PASS)

    def test_report_totals_match_the_results(self):
        report = runner.build_report(self.results)
        totals = report['totals']
        self.assertEqual(totals['scenarios'], len(self.results))
        self.assertEqual(totals['passed'], len(self.results))
        self.assertEqual(totals['failed'], 0)
        self.assertEqual(totals['errors'], 0)
        self.assertEqual(totals['positive'] + totals['negative'], totals['scenarios'])
        self.assertEqual(sum(g['scenarios'] for g in report['groups'].values()),
                         totals['scenarios'])
        for item in report['scenarios']:
            self.assertEqual(item['status'], 'PASS')
            for key in ('id', 'group', 'kind', 'title', 'precondition',
                        'injected_defect', 'expected_gate', 'cleanup'):
                self.assertTrue(item[key], f'{item["id"]} report field {key} is empty')

    def test_human_summary_reports_the_counts(self):
        text = runner.format_human(runner.build_report(self.results))
        self.assertIn('Summary:', text)
        self.assertIn('0 failed', text)
        self.assertIn('all scenarios passed', text)


class IsolationTests(unittest.TestCase):
    """A broken scenario is recorded, and the scenarios after it still run."""

    def setUp(self):
        self.workdir = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.workdir.cleanup()

    def test_non_assertion_exception_does_not_stop_later_scenarios(self):
        results = runner.run_scenarios(
            [scenario('X1', passing), scenario('X2', crashing), scenario('X3', passing)],
            self.workdir.name)
        self.assertEqual([r.status for r in results], ['PASS', 'ERROR', 'PASS'])
        error = results[1].error
        self.assertEqual(error['type'], 'ZeroDivisionError')
        self.assertIn('synthetic non-assertion failure', error['message'])
        self.assertIn('Traceback', error['traceback'])
        self.assertEqual(results[2].scenario.id, 'X3')

    def test_expected_failure_is_reported_as_failed_not_errored(self):
        results = runner.run_scenarios(
            [scenario('Y1', failing), scenario('Y2', passing)], self.workdir.name)
        self.assertEqual([r.status for r in results], ['FAIL', 'PASS'])
        self.assertEqual(results[0].error['type'], 'ScenarioFailed')
        self.assertIn('did not fire', results[0].error['message'])
        self.assertIn('Traceback', results[0].error['traceback'])

    def test_failure_changes_the_exit_code(self):
        results = runner.run_scenarios(
            [scenario('Z1', failing), scenario('Z2', crashing)], self.workdir.name)
        self.assertEqual(runner._exit_code(results), runner.EXIT_SCENARIO_FAILURES)

    def test_report_keeps_the_error_for_every_failed_scenario(self):
        results = runner.run_scenarios(
            [scenario('W1', crashing), scenario('W2', passing)], self.workdir.name)
        report = runner.build_report(results)
        self.assertEqual(report['totals']['errors'], 1)
        self.assertEqual(report['totals']['passed'], 1)
        self.assertIn('Traceback', report['scenarios'][0]['error']['traceback'])
        self.assertIsNone(report['scenarios'][1]['error'])


class CommandLineTests(unittest.TestCase):
    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = runner.main(argv)
        return code, out.getvalue(), err.getvalue()

    def test_all_pass_exits_zero(self):
        code, out, _ = self.run_main(['--json'])
        self.assertEqual(code, runner.EXIT_PASS)
        report = json.loads(out)
        self.assertEqual(report['schema'], runner.REPORT_SCHEMA)
        self.assertEqual(report['totals']['scenarios'], len(runner.SCENARIOS))

    def test_json_is_machine_readable_and_matches_the_human_summary(self):
        code, out, _ = self.run_main(['--json', '--only', 'A1'])
        self.assertEqual(code, runner.EXIT_PASS)
        report = json.loads(out)
        self.assertEqual([item['id'] for item in report['scenarios']], ['A1'])
        self.assertEqual(report['totals']['passed'], 1)
        self.assertEqual(report['totals']['positive'], 1)

    def test_output_file_is_written_outside_the_kit(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / 'report.json'
            code, out, _ = self.run_main(['--json', '--only', 'C5', '--output', str(target)])
            self.assertEqual(code, runner.EXIT_PASS)
            self.assertTrue(target.is_file())
            written = json.loads(target.read_text(encoding='utf-8'))
            self.assertEqual(written, json.loads(out))

    def test_unknown_scenario_is_a_tool_error(self):
        code, _, err = self.run_main(['--only', 'NOPE'])
        self.assertEqual(code, runner.EXIT_TOOL_ERROR)
        self.assertIn('unknown scenario id', err)

    def test_unwritable_output_is_a_tool_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / 'missing-directory' / 'report.json'
            code, _, err = self.run_main(['--only', 'A1', '--output', str(target)])
        self.assertEqual(code, runner.EXIT_TOOL_ERROR)
        self.assertIn('cannot write --output', err)

    def test_list_prints_every_scenario(self):
        code, out, _ = self.run_main(['--list'])
        self.assertEqual(code, runner.EXIT_PASS)
        for scenario_id in runner.scenario_ids():
            self.assertIn(f'{scenario_id}\t', out)

    def test_exit_codes_are_distinct(self):
        self.assertEqual(len({runner.EXIT_PASS, runner.EXIT_SCENARIO_FAILURES,
                              runner.EXIT_TOOL_ERROR}), 3)


if __name__ == '__main__':
    unittest.main()
