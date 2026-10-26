"""AC-3 / AC-4: evidence, conflict and risk gates; complete task packets only.

Every refusal path is exercised because "no task" is the correct answer more
often than "create a task". The positive control at the end proves the gates did
not simply reject everything.
"""
import unittest

from . import loop_support, support
from agentorch_self_improvement import planning  # noqa: E402


def plan_one(record, spec=None, **kwargs):
    kwargs.setdefault('edition', 'local')
    return planning.plan_tasks(
        lesson_records=[record], specs={record['lesson_id']: spec if spec is not None
                                        else loop_support.spec()},
        **kwargs)


class EligibilityGateTests(unittest.TestCase):
    def test_confirmed_fact_alone_cannot_become_a_task(self):
        record = support.verified_lesson()
        plan = plan_one(record, loop_support.spec())
        self.assertEqual(plan.proposals[0]['status'], 'NO_TASK')
        self.assertIn('GENERAL_LESSON', plan.proposals[0]['status_reason'])

    def test_unverified_assumption_never_becomes_a_task(self):
        record = support.lesson(content_class='UNVERIFIED_ASSUMPTION',
                                evidence_level='E1_SOURCE_ARTIFACT',
                                status='NEEDS_VERIFICATION')
        proposal = plan_one(record, loop_support.spec()).proposals[0]
        self.assertEqual(proposal['status'], 'NO_TASK')
        self.assertIsNone(proposal['required_human_decision'])

    def test_duplicate_lesson_does_not_get_a_second_task(self):
        record = loop_support.general_lesson(duplicate_of='LSN-TALKDND-000')
        proposal = plan_one(record, loop_support.spec()).proposals[0]
        self.assertEqual(proposal['status'], 'NO_TASK')
        self.assertIn('duplicate', proposal['status_reason'])

    def test_open_conflict_requires_hd04_even_with_a_complete_packet(self):
        record = loop_support.open_conflict_lesson()
        proposal = plan_one(record, loop_support.spec()).proposals[0]
        self.assertEqual(proposal['status'], 'NEEDS_HUMAN_DECISION')
        self.assertEqual(proposal['required_human_decision'], 'HD-04')
        self.assertIn('OPEN conflict', proposal['status_reason'])

    def test_already_covered_lesson_creates_no_duplicate_rule(self):
        record = loop_support.general_lesson()
        coverage = {record['lesson_id']: {'status': 'COVERED',
                                          'evidence': 'kit/orchestration.md 4.1'}}
        proposal = plan_one(record, loop_support.spec(), coverage=coverage).proposals[0]
        self.assertEqual(proposal['status'], 'NO_TASK')
        self.assertIn('already covers', proposal['status_reason'])

    def test_existing_task_is_reused_instead_of_duplicated(self):
        record = loop_support.general_lesson()
        proposal = plan_one(
            record, loop_support.spec(),
            existing_tasks={record['lesson_id']: 'TASK-0007'}).proposals[0]
        self.assertEqual(proposal['status'], 'ALREADY_REGISTERED')
        self.assertIn('TASK-0007', proposal['status_reason'])


class RiskGateTests(unittest.TestCase):
    def test_frozen_governance_path_needs_hd05(self):
        spec = loop_support.spec(allowed_paths=['kit/orchestration.md'])
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'NEEDS_HUMAN_DECISION')
        self.assertEqual(proposal['required_human_decision'], 'HD-05')
        self.assertEqual(proposal['risk_level'], 'HIGH')

    def test_state_machine_wording_escalates(self):
        spec = loop_support.spec(goal='修改任务状态机以允许跳过审查。')
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'NEEDS_HUMAN_DECISION')
        self.assertEqual(proposal['required_human_decision'], 'HD-05')

    def test_high_risk_wins_over_an_incomplete_packet(self):
        spec = loop_support.spec(allowed_paths=['kit/orchestration.md'])
        spec.pop('validation_commands')
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'NEEDS_HUMAN_DECISION')
        self.assertEqual(proposal['required_human_decision'], 'HD-05')

    def test_contract_schema_wording_maps_to_hd13(self):
        spec = loop_support.spec(goal='改变 lesson schema 的枚举取值。')
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'NEEDS_HUMAN_DECISION')
        self.assertEqual(proposal['required_human_decision'], 'HD-13')


class PacketCompletenessTests(unittest.TestCase):
    def test_missing_validation_commands_blocks_readiness(self):
        spec = loop_support.spec()
        spec.pop('validation_commands')
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'BLOCKED')
        self.assertIn('validation_commands', proposal['status_reason'])

    def test_acceptance_without_verification_blocks_readiness(self):
        spec = loop_support.spec(acceptance_criteria=[
            {'id': 'AC-1', 'statement': '有验收但没有验证命令。'}])
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'BLOCKED')
        self.assertIn('task packet is incomplete', proposal['status_reason'])
        self.assertTrue(any('verification' in error
                            for error in proposal['packet_errors']))

    def test_bare_na_is_not_a_reason(self):
        errors = planning.validate_task_packet(
            loop_support.spec(negative_cases='N/A'))
        self.assertTrue(any('bare N/A' in error for error in errors))

    def test_na_with_reason_is_accepted(self):
        packet = loop_support.spec(
            migration_cases='N/A: 不改变任何已持久化格式。', negative_cases='N/A: 只读报告。')
        self.assertEqual(planning.validate_task_packet(packet), [])
        proposal = plan_one(loop_support.general_lesson(), packet).proposals[0]
        self.assertEqual(proposal['status'], 'READY')

    def test_unresolved_dependency_blocks_readiness(self):
        spec = loop_support.spec(dependencies=['TASK-0002'])
        proposal = plan_one(loop_support.general_lesson(), spec).proposals[0]
        self.assertEqual(proposal['status'], 'BLOCKED')
        self.assertIn('TASK-0002', proposal['status_reason'])

    def test_verified_dependency_is_allowed(self):
        spec = loop_support.spec(dependencies=['TASK-0002'])
        proposal = plan_one(loop_support.general_lesson(), spec,
                            dependency_state={'TASK-0002': 'VERIFIED'}).proposals[0]
        self.assertEqual(proposal['status'], 'READY')
        self.assertEqual(proposal['dependencies'], ['TASK-0002'])

    def test_wildcard_allowed_path_is_rejected(self):
        errors = planning.validate_task_packet(
            loop_support.spec(allowed_paths=['scripts/**']))
        self.assertTrue(any('wildcard' in error for error in errors))


class CompletePacketTests(unittest.TestCase):
    def test_ready_proposal_carries_every_required_field(self):
        proposal = plan_one(loop_support.general_lesson(),
                            loop_support.spec()).proposals[0]
        self.assertEqual(proposal['status'], 'READY')
        self.assertTrue(proposal['allowed_paths'])
        self.assertIn('dependencies', proposal)
        self.assertTrue(proposal['acceptance_criteria'])
        self.assertTrue(proposal['negative_cases'])
        self.assertTrue(proposal['validation_commands'])
        self.assertEqual(planning.validate_task_packet(proposal), [])

    def test_rendered_packets_carry_the_same_fields_for_both_editions(self):
        proposal = plan_one(loop_support.general_lesson(),
                            loop_support.spec()).proposals[0]
        local_packet = planning.render_local_task_packet(proposal)
        github_body = planning.render_github_issue_body(proposal)
        for text in (local_packet, github_body):
            self.assertIn('AC-1', text)
            self.assertIn('negative_cases', text)
            self.assertIn('migration_cases', text)
            self.assertIn('validation_commands', text)
            self.assertIn('4.1', text)
        self.assertIn('## 可改路径', github_body)
        self.assertIn('- **Allowed paths:**', local_packet)

    def test_lesson_without_a_spec_is_not_invented(self):
        empty = planning.plan_tasks(
            lesson_records=[loop_support.general_lesson()], specs={},
            edition='local').proposals[0]
        self.assertEqual(empty['status'], 'BLOCKED')
        self.assertIn('task spec is incomplete', empty['status_reason'])


class LessonGuardTests(unittest.TestCase):
    def test_planning_does_not_mutate_the_lesson(self):
        record = loop_support.general_lesson()
        snapshot = str(record)
        plan_one(record, loop_support.spec())
        self.assertEqual(str(record), snapshot)


if __name__ == '__main__':
    unittest.main()
