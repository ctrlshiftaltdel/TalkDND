"""Acceptance tests for the final self-improvement / update scenarios (#53).

Two things are pinned here:

1. the synthetic delivery runner really registers, runs and cleans up the final
   scenarios for self-improvement, safe update, dependency direction and metric
   honesty — removing a scenario from the registry or from the scenario
   catalogue fails this module even when every remaining scenario is green;
2. the delivery-metric report keeps the semantics the task depends on: a window,
   a sample size and a missing rate / coverage on every metric, ``no data`` for
   an empty input, and no invented coupling / cohesion score.

Everything is synthetic and standard-library only; no model API, credential or
private source is used. The repository-only catalogue check is skipped inside a
release package, which never ships ``docs/``.
"""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / 'scripts'
RUNNER_PATH = SCRIPTS / 'run_delivery_scenarios.py'
METRICS_PATH = SCRIPTS / 'report_metrics.py'
SCENARIO_DOC = ROOT / 'docs' / 'testing' / 'delivery-regression-scenarios.md'

FINAL_SCENARIOS = (
    'A8', 'A9', 'A10', 'A11', 'A12', 'A13', 'A14', 'A15',
    'B5', 'B6', 'B7', 'B8', 'B9', 'B10', 'B11', 'B12', 'B13', 'B14',
    'C19', 'C20', 'D8', 'D9',
)
FORBIDDEN_SCORE_TERMS = ('coupling', 'cohesion', '内聚', '耦合')
METRIC_CONTEXT_KEYS = ('sample_size', 'eligible', 'missing', 'missing_rate', 'coverage')


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


RUNNER = _load('acceptance_delivery_runner', RUNNER_PATH)
METRICS = _load('acceptance_report_metrics', METRICS_PATH)


class RegistryTests(unittest.TestCase):
    def test_final_scenarios_are_registered_with_unique_ids(self):
        ids = RUNNER.scenario_ids()
        self.assertEqual(len(ids), len(set(ids)))
        missing = [item for item in FINAL_SCENARIOS if item not in ids]
        self.assertEqual(missing, [], f'final scenarios are not registered: {missing}')

    def test_every_final_scenario_declares_gate_evidence_and_cleanup(self):
        by_id = {item.id: item for item in RUNNER.SCENARIOS}
        for scenario_id in FINAL_SCENARIOS:
            scenario = by_id[scenario_id]
            for field in ('precondition', 'defect', 'gate', 'evidence', 'cleanup'):
                self.assertTrue(getattr(scenario, field),
                                f'{scenario_id} has an empty {field}')
            self.assertIn(scenario.kind, ('positive', 'negative'))
            self.assertIn(scenario.group, 'ABCD')

    def test_each_new_group_keeps_a_positive_control(self):
        by_id = {item.id: item for item in RUNNER.SCENARIOS}
        positives = [item for item in FINAL_SCENARIOS if by_id[item].kind == 'positive']
        self.assertEqual(positives, ['A8', 'A15', 'B5', 'C19', 'D8'])

    @unittest.skipUnless(SCENARIO_DOC.is_file(), 'repository-only document is not shipped')
    def test_scenario_catalogue_lists_every_final_scenario(self):
        text = SCENARIO_DOC.read_text(encoding='utf-8')
        for scenario_id in FINAL_SCENARIOS:
            self.assertIn(f'| {scenario_id} |', text,
                          f'{scenario_id} is missing from {SCENARIO_DOC.relative_to(ROOT)}')


class ExecutionTests(unittest.TestCase):
    """The final scenarios are executed, not just declared."""

    @classmethod
    def setUpClass(cls):
        cls.workdir = tempfile.TemporaryDirectory()
        cls.vk = RUNNER._load_module('acceptance_validate_kit', SCRIPTS / 'validate_kit.py')
        cls.orch = RUNNER._load_module('acceptance_orchestrator', SCRIPTS / 'orchestrator.py')
        cls.results = RUNNER.run_scenarios(
            RUNNER.select_scenarios(list(FINAL_SCENARIOS)), cls.workdir.name,
            vk=cls.vk, orch=cls.orch)

    @classmethod
    def tearDownClass(cls):
        cls.workdir.cleanup()

    def test_all_final_scenarios_pass(self):
        broken = [f'{result.scenario.id}: {result.status} '
                  f'{result.error.get("message", "")}'
                  for result in self.results if result.status != 'PASS']
        self.assertEqual(broken, [])

    def test_every_final_scenario_leaves_locatable_evidence(self):
        for result in self.results:
            self.assertTrue(result.evidence, f'{result.scenario.id} left no evidence')

    def test_the_run_cleans_up_its_sandboxes(self):
        self.assertEqual(list(Path(self.workdir.name).iterdir()), [])

    def test_exit_code_is_zero_for_the_final_set(self):
        self.assertEqual(RUNNER._exit_code(self.results), RUNNER.EXIT_PASS)


class MetricsContractTests(unittest.TestCase):
    def setUp(self):
        self.dataset = METRICS.load_gold_sample()
        self.window = METRICS.resolve_window(self.dataset['events'], '2026-01-01',
                                             '2026-02-01', METRICS.datetime.now(
                                                 METRICS.timezone.utc))
        self.report = METRICS.build_report(self.dataset, self.window, 3, 14.0)

    def test_every_metric_carries_a_window_sample_missing_rate_and_coverage(self):
        self.assertEqual(self.report['window']['timezone'], 'UTC')
        self.assertIsNotNone(self.report['window']['start'])
        self.assertIsNotNone(self.report['window']['end'])
        self.assertEqual(len(self.report['metrics']), 7)
        for metric in self.report['metrics']:
            for key in METRIC_CONTEXT_KEYS:
                self.assertIn(key, metric, f'{metric["id"]} is missing {key}')
            self.assertIsNotNone(metric['missing_rate'], metric['id'])
            self.assertIsNotNone(metric['coverage'], metric['id'])
            self.assertGreaterEqual(metric['sample_size'], 0)

    def test_empty_input_is_no_data_not_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'events.json'
            path.write_text(json.dumps({'events': []}), encoding='utf-8')
            dataset = METRICS.load_fixture(path)
            window = METRICS.resolve_window(dataset['events'], '2026-01-01', '2026-02-01',
                                            METRICS.datetime.now(METRICS.timezone.utc))
            report = METRICS.build_report(dataset, window, 3, 14.0)
        self.assertEqual(report['summary']['headline'], 'no data')
        for metric in report['metrics']:
            self.assertEqual(metric['status'], 'no_data', metric['id'])
            self.assertIsNone(metric['value'], metric['id'])
            self.assertEqual(metric['coverage'], 0.0, metric['id'])
        self.assertIn('no data', METRICS.render(report))

    def test_no_coupling_or_cohesion_score_is_invented(self):
        payload = json.dumps(self.report, ensure_ascii=False).lower()
        for term in FORBIDDEN_SCORE_TERMS:
            self.assertNotIn(term, payload, f'the report invented a score term: {term}')
        self.assertEqual([metric['id'] for metric in self.report['metrics']],
                         ['M1', 'M2', 'M3', 'M4', 'M5', 'M6', 'M7'])

    def test_report_states_it_is_not_a_ranking(self):
        rendered = METRICS.render(self.report)
        self.assertIn('not a ranking', rendered)
        self.assertIn('coverage:', rendered)


if __name__ == '__main__':
    unittest.main()
