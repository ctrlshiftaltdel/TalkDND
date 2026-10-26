"""AC-2 / AC-3 / AC-5 / AC-6: the loop, its carriers and the closure report.

The end-to-end case uses the real #49 pipeline with a synthetic ``gh`` runner, so
"trigger → restore → collect → plan → register → report" is exercised without a
network, a token, a real Issue or a real command execution.
"""
import json
import tempfile
import unittest
from pathlib import Path

from . import loop_support, support
from agentorch_self_improvement import lessons, orchestration, pipeline, sources  # noqa: E402


def run_loop(**overrides):
    payload = {
        'request_text': '进行自我完善',
        'edition': 'local',
        'state': loop_support.state(),
        'carrier': loop_support.RecordingCarrier('local'),
        'scan_report': loop_support.scan_report(),
        'lesson_records': (),
        'specs': {},
    }
    payload.update(overrides)
    return orchestration.run_loop(**payload)


def ready_case(**overrides):
    record = loop_support.general_lesson()
    payload = {
        'lesson_records': [record],
        'specs': {record['lesson_id']: loop_support.spec()},
        'scan_report': loop_support.scan_report(
            ingestions=[loop_support.created(record['lesson_id'])]),
    }
    payload.update(overrides)
    return payload


class NoNewEvidenceTests(unittest.TestCase):
    def test_ok_scan_without_candidates_reports_no_new_candidates_only(self):
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(carrier=carrier)
        self.assertEqual(outcome.status, 'NO_NEW_CANDIDATES')
        self.assertEqual(outcome.report['scan']['conclusion'], 'NO_NEW_CANDIDATES')
        self.assertIn('无新候选', outcome.markdown)
        self.assertEqual(carrier.calls, [])
        self.assertEqual(outcome.report['registrations'], [])
        self.assertEqual(outcome.report['plan']['proposals'], [])

    def test_scan_decision_is_not_hidden_by_no_new_candidates(self):
        report = loop_support.scan_report()
        report['decisions_required'] = [{
            'kind': 'open_conflict', 'source_id': 'talkdnd',
            'reason': 'HD-04 required', 'decision': 'NEEDS_HUMAN_DECISION'}]
        outcome = run_loop(scan_report=report)
        self.assertEqual(outcome.status, 'NEEDS_HUMAN_DECISION')
        self.assertTrue(outcome.report['decisions_required'])
        self.assertEqual(outcome.report['registrations'], [])

    def test_empty_registry_reports_no_sources(self):
        outcome = run_loop(scan_report=loop_support.scan_report(sources=[]))
        self.assertEqual(outcome.status, 'NO_SOURCES')
        self.assertIn('无来源可扫描', outcome.markdown)
        self.assertNotIn('无新候选', outcome.markdown)
        self.assertEqual(outcome.report['registrations'], [])

    def test_blocked_scan_is_incomplete_not_no_evidence(self):
        blocked = loop_support.source_report(status='BLOCKED', blocked_reason='forbidden')
        outcome = run_loop(scan_report=loop_support.scan_report(sources=[blocked]))
        self.assertEqual(outcome.status, 'SCAN_INCOMPLETE')
        self.assertIn('未完成扫描', outcome.markdown)
        self.assertNotIn('无新候选', outcome.markdown)
        self.assertEqual(outcome.report['registrations'], [])

    def test_offline_scan_is_not_run_and_creates_nothing(self):
        offline = loop_support.source_report(status='NOT_RUN', blocked_reason='offline',
                                             evidence_count=0)
        outcome = run_loop(scan_report=loop_support.scan_report(
            sources=[offline], scan_mode='offline'))
        self.assertEqual(outcome.status, 'SCAN_INCOMPLETE')
        self.assertEqual(outcome.report['scan']['conclusion'], 'NOT_RUN')
        self.assertEqual(outcome.report['registrations'], [])

    def test_ineligible_candidates_are_not_actionable(self):
        record = support.lesson(content_class='UNVERIFIED_ASSUMPTION',
                                evidence_level='E1_SOURCE_ARTIFACT',
                                status='NEEDS_VERIFICATION')
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(
            carrier=carrier, lesson_records=[record],
            specs={record['lesson_id']: loop_support.spec()},
            scan_report=loop_support.scan_report(
                ingestions=[loop_support.created(record['lesson_id'])]))
        self.assertEqual(outcome.status, 'NO_ACTIONABLE_CANDIDATES')
        self.assertEqual(outcome.report['plan']['proposals'][0]['status'], 'NO_TASK')
        self.assertEqual(carrier.calls, [])
        self.assertEqual(outcome.report['validation'], [])


class GateTests(unittest.TestCase):
    def test_high_risk_candidate_needs_a_human_and_is_not_registered(self):
        record = loop_support.general_lesson()
        spec = loop_support.spec(allowed_paths=['kit/orchestration.md'])
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(
            carrier=carrier, lesson_records=[record],
            specs={record['lesson_id']: spec},
            scan_report=loop_support.scan_report(
                ingestions=[loop_support.created(record['lesson_id'])]))
        self.assertEqual(outcome.status, 'NEEDS_HUMAN_DECISION')
        self.assertEqual(outcome.report['registrations'], [])
        self.assertEqual(carrier.calls, [])
        decisions = outcome.report['decisions_required']
        self.assertTrue(any(item.get('decision_id') == 'HD-05' for item in decisions))

    def test_incomplete_packet_blocks_the_loop(self):
        record = loop_support.general_lesson()
        spec = loop_support.spec()
        spec.pop('validation_commands')
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(carrier=carrier, lesson_records=[record],
                           specs={record['lesson_id']: spec},
                           scan_report=loop_support.scan_report(
                               ingestions=[loop_support.created(record['lesson_id'])]))
        self.assertEqual(outcome.status, 'BLOCKED')
        self.assertEqual(carrier.calls, [])
        self.assertTrue(outcome.report['blocked'])

    def test_missing_state_fails_closed_before_any_registration(self):
        state = loop_support.state()
        state.pop('head_sha')
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(carrier=carrier, state=state, **ready_case())
        self.assertEqual(outcome.status, 'BLOCKED')
        self.assertEqual(carrier.calls, [])
        self.assertTrue(any('head_sha' in item['reason']
                            for item in outcome.report['blocked']))

    def test_edition_disagreement_fails_closed(self):
        state = loop_support.state(edition='github')
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(carrier=carrier, state=state, **ready_case())
        self.assertEqual(outcome.status, 'BLOCKED')
        self.assertEqual(carrier.calls, [])
        self.assertTrue(any('disagrees' in item['reason']
                            for item in outcome.report['blocked']))

    def test_bare_trigger_asks_before_doing_work(self):
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(request_text='自我完善', carrier=carrier)
        self.assertEqual(outcome.status, 'CLARIFICATION_REQUIRED')
        self.assertEqual(carrier.calls, [])
        self.assertEqual(outcome.report['registrations'], [])

    def test_update_agentorch_never_enters_the_loop(self):
        carrier = loop_support.RecordingCarrier()
        outcome = run_loop(request_text='更新 AgentOrch', carrier=carrier)
        self.assertEqual(outcome.status, 'NOT_TRIGGERED')
        self.assertIn('update engine', outcome.report['reason'])
        self.assertEqual(carrier.calls, [])


class CarrierParityTests(unittest.TestCase):
    def test_both_editions_register_with_their_own_shape(self):
        results = {}
        for edition in ('local', 'github'):
            carrier = orchestration.carrier_for(edition)
            outcome = run_loop(edition=edition, **ready_case(
                state=loop_support.state(edition), carrier=carrier))
            self.assertEqual(outcome.status, 'COMPLETED')
            registration = outcome.report['registrations'][0]
            self.assertEqual(registration['carrier'], edition)
            self.assertFalse(registration['executed'])
            results[edition] = registration
        self.assertIn('task_id', results['local']['payload'])
        self.assertIn('issue_body', results['github']['payload'])
        self.assertIsNone(results['github']['payload']['issue_number'])
        self.assertNotIn('issue_body', results['local']['payload'])
        self.assertNotIn('task_id', results['github']['payload'])

    def test_the_same_gates_hold_for_both_editions(self):
        for edition in ('local', 'github'):
            with self.subTest(edition=edition):
                record = loop_support.general_lesson()
                spec = loop_support.spec(allowed_paths=['editions/'])
                carrier = orchestration.carrier_for(edition)
                outcome = run_loop(
                    edition=edition, carrier=carrier,
                    state=loop_support.state(edition),
                    lesson_records=[record], specs={record['lesson_id']: spec},
                    scan_report=loop_support.scan_report(
                        ingestions=[loop_support.created(record['lesson_id'])]))
                self.assertEqual(outcome.status, 'NEEDS_HUMAN_DECISION')
                self.assertEqual(outcome.report['registrations'], [])
                self.assertTrue(any(item.get('decision_id') == 'HD-05'
                                    for item in outcome.report['decisions_required']))

    def test_a_mismatched_carrier_is_refused_before_it_is_called(self):
        carrier = loop_support.RecordingCarrier('github')
        with self.assertRaises(orchestration.LoopError):
            run_loop(edition='local', carrier=carrier, **ready_case())
        self.assertEqual(carrier.calls, [])

    def test_carrier_refuses_a_non_ready_proposal(self):
        proposal = loop_support.spec()
        proposal.update({'lesson_id': 'LSN-TALKDND-001', 'edition': 'local',
                         'status': 'BLOCKED', 'status_reason': 'blocked'})
        with self.assertRaises(orchestration.CarrierError):
            orchestration.LocalCarrier().register(proposal)

    def test_one_edition_writes_one_carrier(self):
        local = loop_support.RecordingCarrier('local')
        github = loop_support.RecordingCarrier('github')
        run_loop(edition='local', carrier=local, **ready_case())
        self.assertEqual(github.calls, [])
        self.assertEqual(len(local.calls), 1)


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)

    def _eligible_record(self):
        registry = sources.Registry.load(support.registry_file(self.tmp))
        store = lessons.LessonStore()
        outcome = pipeline.run_scan(
            registry, store, support.FakeRunner(), now=support.T2,
            analyses={loop_support.PR_URL: support.analysis(
                evidence_level='E3_FIXED_WITH_TEST')})
        self.assertEqual([item.kind for item in outcome.ingestions], ['created'])
        record = store.all()[0]
        record = lessons.transition(record, 'VERIFIED')
        record = lessons.reclassify(record, 'GENERAL_LESSON',
                                    decision=support.decision('ADOPT'))
        record = lessons.transition(record, 'GENERALIZED',
                                    decision=support.decision('ADOPT'))
        return record, outcome

    def test_trigger_restore_collect_plan_register_report(self):
        record, scan = self._eligible_record()
        passed = 'python3 -m unittest tests.self_improvement.test_orchestration'
        unrun = 'python3 scripts/validate_kit.py'
        carrier = loop_support.RecordingCarrier('local')
        outcome = orchestration.run_loop(
            request_text='进行自我完善', edition='local',
            state=loop_support.state(),
            carrier=carrier, scan_report=scan,
            lesson_records=[record],
            specs={record['lesson_id']: loop_support.spec(
                validation_commands=[passed, unrun])},
            command_runs={passed: {'exit_code': 0, 'output': 'tests passed'}},
            environment='Linux; Python 3; synthetic fixture; local edition')

        report = outcome.report
        self.assertEqual(report['status'], 'COMPLETED')
        self.assertEqual(report['trigger']['route'], 'SELF_IMPROVEMENT')
        self.assertEqual(report['state']['head_sha'], loop_support.HEAD)
        self.assertEqual(report['scan']['conclusion'], 'CANDIDATES_FOUND')
        self.assertEqual(report['plan']['proposals'][0]['status'], 'READY')
        self.assertEqual(len(report['registrations']), 1)
        self.assertEqual(carrier.calls[0]['lesson_id'], record['lesson_id'])
        self.assertFalse(report['registrations'][0]['executed'])

        rows = {row['command']: row for row in report['validation']}
        self.assertEqual(rows[passed]['status'], 'PASS')
        self.assertEqual(rows[passed]['commit_sha'], loop_support.HEAD)
        self.assertEqual(rows[unrun]['status'], 'NOT_RUN')
        self.assertEqual(report['not_run'], [unrun])
        self.assertIn('NOT_RUN', outcome.markdown)

        json_path = orchestration.write_report(report, self.tmp / 'loop.json')
        md_path = orchestration.write_markdown(report, self.tmp / 'loop.md')
        reloaded = json.loads(json_path.read_text(encoding='utf-8'))
        self.assertEqual(reloaded['status'], 'COMPLETED')
        self.assertIn('Self-improvement loop report',
                      md_path.read_text(encoding='utf-8'))
        self.assertEqual(loop_support.HEAD, reloaded['state']['head_sha'])
        self.assertIn('limitations', reloaded)

    def test_repeated_trigger_does_not_duplicate_an_existing_task(self):
        record, scan = self._eligible_record()
        carrier = loop_support.RecordingCarrier('local')
        outcome = orchestration.run_loop(
            request_text='进行自我完善', edition='local',
            state=loop_support.state(), carrier=carrier, scan_report=scan,
            lesson_records=[record],
            specs={record['lesson_id']: loop_support.spec()},
            existing_tasks={record['lesson_id']: 'TASK-0003'})
        self.assertEqual(outcome.status, 'NO_ACTIONABLE_CANDIDATES')
        self.assertEqual(outcome.report['registrations'], [])
        self.assertEqual(outcome.report['plan']['proposals'][0]['status'],
                         'ALREADY_REGISTERED')
        self.assertEqual(carrier.calls, [])


if __name__ == '__main__':
    unittest.main()
