"""AC-1: trigger and synonym routing must not turn ordinary questions into work.

The table is exercised directly; the tests deliberately include the phrases the
contract names as non-triggers, not just the happy path.
"""
import unittest

from . import loop_support
from agentorch_self_improvement import planning  # noqa: E402


class SelfImprovementTriggerTests(unittest.TestCase):
    def assert_route(self, text, route, *, starts=None):
        decision = planning.route_request(text)
        self.assertEqual(decision.route, route, f'{text!r}: {decision.as_dict()}')
        if starts is not None:
            self.assertEqual(decision.starts_workflow, starts, decision.as_dict())
        return decision

    def test_imperative_synonyms_start_the_workflow(self):
        for text in ('进行自我完善',
                     '根据其他 AgentOrch 项目的经验改进本项目',
                     '总结近期开发教训并推进改进',
                     '现在开始进行自我完善。'):
            with self.subTest(text=text):
                self.assert_route(text, planning.ROUTE_SELF_IMPROVEMENT, starts=True)

    def test_spacing_case_and_punctuation_do_not_matter(self):
        for text in (' 进行  自我完善 ', '进行自我完善！',
                     '根据其他 agentorch 项目的经验改进本项目'):
            with self.subTest(text=text):
                decision = self.assert_route(
                    text, planning.ROUTE_SELF_IMPROVEMENT, starts=True)
                self.assertTrue(decision.execution_intent)
                self.assertFalse(decision.needs_clarification)

    def test_questions_about_the_mechanism_only_explain(self):
        for text in ('自我完善机制是什么？',
                     '自我完善的原理是什么',
                     '自我完善机制是怎么设计的？',
                     '能介绍一下自我完善的流程吗？'):
            with self.subTest(text=text):
                decision = self.assert_route(text, planning.ROUTE_EXPLAIN_ONLY, starts=False)
                self.assertIn('explanation', decision.reason)

    def test_how_do_i_run_it_is_still_a_question(self):
        decision = self.assert_route(
            '自我完善怎么进行？', planning.ROUTE_EXPLAIN_ONLY, starts=False)
        self.assertFalse(decision.execution_intent)

    def test_bare_noun_phrase_asks_once_before_doing_anything(self):
        decision = self.assert_route(
            '自我完善', planning.ROUTE_SELF_IMPROVEMENT, starts=False)
        self.assertTrue(decision.needs_clarification)
        self.assertIn('execution intent', decision.reason)

    def test_update_agentorch_belongs_to_another_feature(self):
        decision = self.assert_route(
            '更新 AgentOrch', planning.ROUTE_UPDATE_AGENTORCH, starts=False)
        self.assertEqual(decision.owner, 'update-engine')
        self.assertNotEqual(decision.route, planning.ROUTE_SELF_IMPROVEMENT)

    def test_update_with_ambiguous_object_is_not_executed(self):
        decision = self.assert_route(
            '更新 AgentOrch 的文档', planning.ROUTE_UPDATE_AGENTORCH, starts=False)
        self.assertTrue(decision.needs_clarification)
        self.assertIn('ambiguous', decision.reason)

    def test_ordinary_updates_are_not_agentorch_updates(self):
        for text in ('更新一下文档', '更新依赖', '更新这个 Issue'):
            with self.subTest(text=text):
                decision = planning.route_request(text)
                self.assertNotEqual(decision.route, planning.ROUTE_UPDATE_AGENTORCH)
                self.assertFalse(decision.starts_workflow)

    def test_version_question_is_not_a_trigger(self):
        decision = self.assert_route(
            'AgentOrch 最新版本是多少？', planning.ROUTE_EXPLAIN_ONLY, starts=False)
        self.assertEqual(decision.owner, 'update-engine')

    def test_unrelated_request_is_left_alone(self):
        for text in ('把登录页的颜色改成蓝色', '', '   '):
            with self.subTest(text=text):
                decision = planning.route_request(text)
                self.assertEqual(decision.route, planning.ROUTE_UNRELATED)
                self.assertFalse(decision.starts_workflow)

    def test_non_trigger_does_not_touch_the_loop(self):
        carrier = loop_support.RecordingCarrier()
        from agentorch_self_improvement import orchestration
        outcome = orchestration.run_loop(
            request_text='自我完善机制是什么', edition='local',
            state=loop_support.state(), carrier=carrier,
            scan_report=loop_support.scan_report())
        self.assertEqual(outcome.status, 'NOT_TRIGGERED')
        self.assertEqual(carrier.calls, [])
        self.assertEqual(outcome.report['plan']['proposals'], [])


if __name__ == '__main__':
    unittest.main()
