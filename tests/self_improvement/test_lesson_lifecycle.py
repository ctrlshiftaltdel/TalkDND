"""AC-3 / AC-4: lesson classes, evidence levels, transitions, dedup and conflicts.

AC-4 — the four content classes are distinguishable and gated: an unverified
assumption cannot be verified, a source-specific experience cannot be generalised
without an explicit reclassification, and only a GENERAL_LESSON in GENERALIZED
with no OPEN conflict may become an improvement task.

AC-3 — the same finding scanned twice never produces a second record: the store
is idempotent per ``(source_id, evidence_url, fingerprint)``, merges new evidence
into the earliest lesson, and treats a corrected fingerprint as an update rather
than a new lesson.
"""
import unittest

from . import support
from agentorch_self_improvement import lessons


class FingerprintTests(unittest.TestCase):
    def test_normalization_follows_the_frozen_steps(self):
        self.assertEqual(lessons.normalize_text('root_cause', '  Ａ Ｒoot   CAUSE。  '),
                         'a root cause')
        self.assertEqual(lessons.normalize_text('impact', 'breaks the gate!!!'),
                         'breaks the gate')
        self.assertEqual(lessons.normalize_text('affected_surface', 'Scripts/Self Improvement.py'),
                         'scripts/selfimprovement.py')
        self.assertEqual(lessons.normalize_text('suggested_action_target', 'TEST'), 'test')
        self.assertEqual(lessons.normalize_text('root_cause', None), '')

    def test_value_is_stable_and_independent_of_field_order(self):
        values = lessons.fingerprint_inputs(support.lesson())
        forward = lessons.fingerprint_value(values, lessons.FINGERPRINT_FIELDS)
        backward = lessons.fingerprint_value(values, list(reversed(lessons.FINGERPRINT_FIELDS)))
        self.assertEqual(forward, backward)
        self.assertRegex(forward, r'^[a-f0-9]{64}$')

    def test_rewording_the_root_cause_changes_the_fingerprint(self):
        first = support.lesson()
        second = support.lesson(observation=support.observation(
            root_cause='合并门只看 CI 状态，不要求新增测试覆盖该路径。'))
        self.assertNotEqual(first['fingerprint']['value'], second['fingerprint']['value'])

    def test_fewer_than_three_fields_is_refused(self):
        with self.assertRaises(lessons.LessonError):
            lessons.compute_fingerprint(lessons.fingerprint_inputs(support.lesson()),
                                        ['root_cause', 'impact'])

    def test_hand_edited_fingerprint_is_refused(self):
        record = support.lesson()
        record['fingerprint']['value'] = '0' * 64
        errors = lessons.validate_lesson(record)
        self.assertTrue(any(error['path'] == '/fingerprint/value' for error in errors))

    def test_lesson_without_evidence_is_refused(self):
        source = dict(support.lesson()['source'])
        source['evidence_urls'] = []
        with self.assertRaises(lessons.LessonError):
            support.lesson(source=source)


class ClassAndLevelTests(unittest.TestCase):
    def test_unverified_assumption_cannot_hold_reproduced_evidence(self):
        errors = lessons.validate_lesson(support.raw_lesson(
            content_class='UNVERIFIED_ASSUMPTION', evidence_level='E2_REPRODUCED'))
        self.assertTrue(any(error['path'] == '/evidence_level' for error in errors))

    def test_general_lesson_cannot_be_observed(self):
        errors = lessons.validate_lesson(support.raw_lesson(
            content_class='GENERAL_LESSON', evidence_level='E2_REPRODUCED',
            status='OBSERVED'))
        self.assertTrue(any(error['path'] == '/status' for error in errors))

    def test_source_specific_lesson_must_use_source_only_scope(self):
        errors = lessons.validate_lesson(support.raw_lesson(
            content_class='SOURCE_SPECIFIC_EXPERIENCE',
            applicability={'scope': 'agentorch_general',
                           'applies_to': ['scripts/self_improvement/lessons.py'],
                           'excluded_contexts': []}))
        self.assertTrue(any(error['path'] == '/applicability/scope' for error in errors))

    def test_four_classes_are_mutually_distinguishable_in_the_report(self):
        records = [
            support.lesson(lesson_id='LSN-TALKDND-001'),
            support.lesson(lesson_id='LSN-TALKDND-002',
                           content_class='UNVERIFIED_ASSUMPTION',
                           evidence_level='E0_CLAIM',
                           source={'source_id': 'talkdnd', 'evidence_kind': 'issue',
                                   'evidence_urls': ['https://github.com/ctrlshiftaltdel/TalkDND/issues/7'],
                                   'observed_at': support.T1, 'captured_at': support.T2}),
            support.lesson(lesson_id='LSN-TALKDND-003',
                           content_class='SOURCE_SPECIFIC_EXPERIENCE',
                           status='VERIFIED', evidence_level='E2_REPRODUCED'),
            support.lesson(lesson_id='LSN-TALKDND-004',
                           content_class='GENERAL_LESSON', status='VERIFIED',
                           evidence_level='E2_REPRODUCED',
                           applicability={'scope': 'agentorch_general',
                                          'applies_to': ['scripts/validate_kit.py'],
                                          'excluded_contexts': ['没有 CI 的项目']}),
        ]
        for record in records:
            self.assertEqual(lessons.validate_lesson(record), [])
        report = lessons.build_report(
            registry_summary={}, source_reports=[], lessons=records,
            generated_at=support.T2)
        self.assertEqual(report['counts']['lessons_by_content_class'],
                         {'CONFIRMED_FACT': 1, 'UNVERIFIED_ASSUMPTION': 1,
                          'SOURCE_SPECIFIC_EXPERIENCE': 1, 'GENERAL_LESSON': 1})
        self.assertFalse(report['lessons'][0]['can_generate_task'])

    def test_only_a_generalized_general_lesson_can_generate_a_task(self):
        general = support.lesson(
            lesson_id='LSN-TALKDND-001', content_class='GENERAL_LESSON', status='VERIFIED',
            evidence_level='E2_REPRODUCED',
            applicability={'scope': 'agentorch_general',
                           'applies_to': ['scripts/validate_kit.py'],
                           'excluded_contexts': []})
        self.assertFalse(lessons.can_generate_task(general)[0])
        generalized = lessons.reclassify(
            support.verified_lesson(), 'GENERAL_LESSON', decision=support.decision())
        generalized = lessons.transition(generalized, 'GENERALIZED',
                                         decision=support.decision())
        can_task, reason = lessons.can_generate_task(generalized)
        self.assertTrue(can_task, reason)
        # Defensive: a record carrying an OPEN conflict is never task-eligible, even
        # if it somehow reached GENERALIZED (the schema normally prevents that).
        import copy
        conflicted = copy.deepcopy(generalized)
        conflicted['conflicts_with'] = [lessons.make_conflict('LSN-TALKDND-002')]
        allowed, why = lessons.can_generate_task(conflicted)
        self.assertFalse(allowed)
        self.assertIn('OPEN conflict', why)


class TransitionTests(unittest.TestCase):
    def test_observed_to_needs_verification(self):
        updated = lessons.transition(support.lesson(), 'NEEDS_VERIFICATION')
        self.assertEqual(updated['status'], 'NEEDS_VERIFICATION')

    def test_observed_to_verified_needs_e2(self):
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(support.lesson(), 'VERIFIED')
        updated = lessons.transition(support.verified_lesson(status='OBSERVED'), 'VERIFIED')
        self.assertEqual(updated['status'], 'VERIFIED')

    def test_needs_verification_to_verified(self):
        record = lessons.transition(support.verified_lesson(status='NEEDS_VERIFICATION'),
                                    'VERIFIED')
        self.assertEqual(record['status'], 'VERIFIED')

    def test_unverified_assumption_cannot_be_verified(self):
        record = support.lesson(content_class='UNVERIFIED_ASSUMPTION',
                                evidence_level='E1_SOURCE_ARTIFACT')
        with self.assertRaises(lessons.TransitionError) as caught:
            lessons.transition(record, 'VERIFIED')
        self.assertIn('UNVERIFIED_ASSUMPTION', str(caught.exception))

    def test_skipping_verification_is_refused(self):
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(support.lesson(), 'GENERALIZED')
        record = lessons.transition(support.lesson(), 'NEEDS_VERIFICATION')
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(record, 'IMPLEMENTED')

    def test_source_specific_cannot_jump_to_generalized(self):
        record = support.lesson(content_class='SOURCE_SPECIFIC_EXPERIENCE',
                                status='VERIFIED', evidence_level='E2_REPRODUCED',
                                applicability={'scope': 'source_only',
                                               'applies_to': ['scripts/validate_kit.py'],
                                               'excluded_contexts': []})
        with self.assertRaises(lessons.TransitionError) as caught:
            lessons.transition(record, 'GENERALIZED', decision=support.decision())
        self.assertIn('SOURCE_SPECIFIC_EXPERIENCE', str(caught.exception))

    def test_generalization_needs_a_decision_and_an_explicit_reclassification(self):
        verified = support.verified_lesson()
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(verified, 'GENERALIZED')
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(verified, 'GENERALIZED', decision=support.decision())
        reclassified = lessons.reclassify(verified, 'GENERAL_LESSON',
                                          decision=support.decision())
        self.assertEqual(reclassified['applicability']['scope'], 'agentorch_general')
        self.assertEqual(lessons.validate_lesson(reclassified), [])
        generalized = lessons.transition(reclassified, 'GENERALIZED',
                                         decision=support.decision())
        self.assertEqual(generalized['status'], 'GENERALIZED')

    def test_promotion_requires_e2_and_adopt(self):
        # A valid E1 record cannot be promoted: the evidence floor is E2.
        with self.assertRaises(lessons.LessonError):
            lessons.reclassify(support.lesson(), 'GENERAL_LESSON',
                               decision=support.decision())
        with self.assertRaises(lessons.LessonError):
            lessons.reclassify(support.verified_lesson(), 'GENERAL_LESSON',
                               decision=support.decision(outcome='DEFER'))

    def test_open_conflict_blocks_generalization_and_implementation(self):
        first = support.verified_lesson()
        first = lessons.add_conflict(first, lessons.make_conflict('LSN-TALKDND-002'))
        self.assertEqual(first['conflicts_with'][0]['resolution'], 'OPEN')
        with self.assertRaises(lessons.LessonError):
            lessons.reclassify(first, 'GENERAL_LESSON', decision=support.decision())
        generalized = lessons.transition(
            lessons.reclassify(support.verified_lesson(), 'GENERAL_LESSON',
                               decision=support.decision()),
            'GENERALIZED', decision=support.decision())
        # The schema forbids attaching a newly discovered OPEN conflict to an
        # already-general lesson: it must be re-opened for verification first.
        with self.assertRaises(lessons.LessonError):
            lessons.add_conflict(generalized,
                                 lessons.make_conflict('LSN-TALKDND-002', 'CONTRADICTS'))
        conflicted_verified = lessons.add_conflict(
            support.verified_lesson(), lessons.make_conflict('LSN-TALKDND-002', 'CONTRADICTS'))
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(conflicted_verified, 'GENERALIZED',
                               decision=support.decision())

    def test_conflict_resolution_requires_locatable_evidence(self):
        record = lessons.add_conflict(support.verified_lesson(),
                                      lessons.make_conflict('LSN-TALKDND-002'))
        with self.assertRaises(lessons.LessonError):
            lessons.resolve_conflict(record, 'LSN-TALKDND-002', 'RESOLVED_BY_OWNER', '')
        resolved = lessons.resolve_conflict(record, 'LSN-TALKDND-002', 'RESOLVED_BY_OWNER',
                                            'Issue #49 comment 1234')
        self.assertEqual(lessons.open_conflicts(resolved), [])
        self.assertEqual(lessons.validate_lesson(resolved), [])

    def test_rejected_is_terminal_and_needs_a_decision(self):
        with self.assertRaises(lessons.LessonError):
            support.lesson(status='REJECTED')
        rejected = lessons.transition(support.lesson(), 'REJECTED',
                                      decision=support.decision('REJECT'))
        with self.assertRaises(lessons.TransitionError) as caught:
            lessons.transition(rejected, 'OBSERVED')
        self.assertIn('terminal', str(caught.exception))

    def test_implemented_needs_the_task_link(self):
        generalized = lessons.transition(
            lessons.reclassify(support.verified_lesson(), 'GENERAL_LESSON',
                               decision=support.decision()),
            'GENERALIZED', decision=support.decision())
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(generalized, 'IMPLEMENTED', decision=support.decision())
        implemented = lessons.transition(generalized, 'IMPLEMENTED',
                                         links={'issues': [51], 'pull_requests': [52]},
                                         decision=support.decision())
        self.assertEqual(implemented['status'], 'IMPLEMENTED')
        watched = lessons.transition(implemented, 'REGRESSION_WATCH',
                                     observation_window='two weekly scans')
        self.assertEqual(watched['status'], 'REGRESSION_WATCH')
        self.assertIn('window:two weekly scans', watched['links']['modules'])

    def test_t10_needs_new_evidence(self):
        watched = support.lesson(status='REGRESSION_WATCH', evidence_level='E4_REGRESSION_COVERED')
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(watched, 'NEEDS_VERIFICATION')
        regressed = lessons.transition(
            watched, 'NEEDS_VERIFICATION',
            new_evidence_urls=['https://github.com/ctrlshiftaltdel/TalkDND/issues/99'])
        self.assertEqual(regressed['status'], 'NEEDS_VERIFICATION')

    def test_t11_demotion_keeps_the_record_and_lowers_the_level(self):
        demoted = lessons.demote(support.verified_lesson(), reason='来源证据链接已失效',
                                 decided_by='project-owner')
        self.assertEqual(demoted['content_class'], 'UNVERIFIED_ASSUMPTION')
        self.assertEqual(demoted['evidence_level'], 'E0_CLAIM')
        self.assertEqual(demoted['status'], 'NEEDS_VERIFICATION')
        self.assertEqual(lessons.validate_lesson(demoted), [])

    def test_t12_walks_a_general_lesson_back(self):
        generalized = lessons.transition(
            lessons.reclassify(support.verified_lesson(), 'GENERAL_LESSON',
                               decision=support.decision()),
            'GENERALIZED', decision=support.decision())
        reverted = lessons.transition(generalized, 'VERIFIED',
                                      content_class='SOURCE_SPECIFIC_EXPERIENCE',
                                      decision=support.decision())
        self.assertEqual(reverted['content_class'], 'SOURCE_SPECIFIC_EXPERIENCE')
        self.assertEqual(reverted['applicability']['scope'], 'source_only')
        self.assertEqual(lessons.validate_lesson(reverted), [])

    def test_a_partial_scan_cannot_verify_anything(self):
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(support.verified_lesson(status='OBSERVED'), 'VERIFIED',
                               scan_status='PARTIAL')
        with self.assertRaises(lessons.TransitionError):
            lessons.transition(support.verified_lesson(status='OBSERVED'), 'VERIFIED',
                               scan_status='NOT_RUN')

    def test_allowed_transitions_lists_only_table_entries(self):
        self.assertEqual(lessons.allowed_transitions(support.lesson()),
                         ['T2->NEEDS_VERIFICATION', 'T3->VERIFIED', 'T4->REJECTED'])
        rejected = lessons.transition(support.lesson(), 'REJECTED',
                                      decision=support.decision('REJECT'))
        self.assertEqual(lessons.allowed_transitions(rejected), [])


class DedupTests(unittest.TestCase):
    def test_same_candidate_twice_is_idempotent(self):
        store = lessons.LessonStore()
        first = store.ingest(support.lesson())
        second = store.ingest(support.lesson())
        self.assertEqual(first.kind, 'created')
        self.assertEqual(second.kind, 'idempotent')
        self.assertEqual(second.lesson_id, first.lesson_id)
        self.assertEqual(len(store), 1)
        self.assertEqual(store.get(first.lesson_id)['source']['evidence_urls'],
                         ['https://github.com/ctrlshiftaltdel/TalkDND/pull/12'])

    def test_same_fingerprint_new_evidence_merges_into_the_earliest_lesson(self):
        store = lessons.LessonStore()
        first = store.ingest(support.lesson())
        replayed = support.lesson(
            lesson_id='LSN-TALKDND-900',
            source={'source_id': 'talkdnd', 'evidence_kind': 'pull_request',
                    'evidence_urls': ['https://github.com/ctrlshiftaltdel/TalkDND/pull/13'],
                    'observed_at': support.T1, 'captured_at': support.T2})
        second = store.ingest(replayed)
        self.assertEqual(second.kind, 'duplicate')
        self.assertEqual(second.lesson_id, first.lesson_id)
        self.assertEqual(second.duplicate_of, first.lesson_id)
        self.assertEqual(len(store), 1)
        self.assertEqual(store.get(first.lesson_id)['source']['evidence_urls'],
                         ['https://github.com/ctrlshiftaltdel/TalkDND/pull/12',
                          'https://github.com/ctrlshiftaltdel/TalkDND/pull/13'])
        # A duplicate never becomes a new task candidate.
        self.assertFalse(store.get(first.lesson_id).get('duplicate_of'))

    def test_corrected_fingerprint_updates_the_same_lesson(self):
        store = lessons.LessonStore()
        first = store.ingest(support.lesson())
        corrected = support.lesson(
            observation=support.observation(root_cause='合并门只检查 CI 状态，没有要求回归测试。'))
        result = store.ingest(corrected)
        self.assertEqual(result.kind, 'fingerprint_updated')
        self.assertEqual(result.lesson_id, first.lesson_id)
        self.assertEqual(len(store), 1)
        self.assertEqual(store.get(first.lesson_id)['fingerprint']['value'],
                         corrected['fingerprint']['value'])
        # The old fingerprint still resolves to the same lesson.
        self.assertEqual(store.by_fingerprint(first.lesson['fingerprint']['value'])
                         ['lesson_id'], first.lesson_id)

    def test_lesson_ids_increment_and_are_not_reused(self):
        store = lessons.LessonStore()
        store.ingest(support.lesson(lesson_id='LSN-TALKDND-004'))
        next_id = lessons.next_lesson_id('talkdnd', [record['lesson_id'] for record in store.all()])
        self.assertEqual(next_id, 'LSN-TALKDND-005')
        self.assertEqual(lessons.next_lesson_id('agentmgr', []), 'LSN-AGENTMGR-001')

    def test_store_roundtrips_through_a_file(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'lessons.json'
            store = lessons.LessonStore([support.lesson()])
            store.save(path)
            reloaded = lessons.LessonStore.load(path)
            self.assertEqual(reloaded.all(), store.all())

    def test_store_refuses_two_lessons_with_the_same_fingerprint(self):
        first = support.lesson(lesson_id='LSN-TALKDND-001')
        duplicate = support.lesson(
            lesson_id='LSN-TALKDND-002',
            source={'source_id': 'talkdnd', 'evidence_kind': 'pull_request',
                    'evidence_urls': ['https://github.com/ctrlshiftaltdel/TalkDND/pull/13'],
                    'observed_at': support.T1, 'captured_at': support.T2})
        with self.assertRaises(lessons.LessonError):
            lessons.LessonStore([first, duplicate])

    def test_a_lone_title_does_not_become_a_lesson(self):
        candidate = lessons.candidate_from_evidence({
            'source_id': 'talkdnd', 'evidence_kind': 'issue',
            'url': 'https://github.com/ctrlshiftaltdel/TalkDND/issues/7',
            'title': 'Flaky merge gate', 'observed_at': support.T1,
            'captured_at': support.T2, 'suggested_evidence_level': 'E0_CLAIM',
        })
        self.assertTrue(candidate['requires_analysis'])
        with self.assertRaises(lessons.LessonError):
            lessons.draft_lesson(candidate, {'title': 'Flaky merge gate'})


if __name__ == '__main__':
    unittest.main()
