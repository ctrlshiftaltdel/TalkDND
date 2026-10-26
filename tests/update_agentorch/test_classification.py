"""Three-way classification tests (pure, no filesystem, no package)."""
from __future__ import annotations

import unittest

from . import support

classify = support.classify


A = 'a' * 64
B = 'b' * 64
C = 'c' * 64


def entry(result, path):
    for item in result.files:
        if item['path'] == path:
            return item
    raise AssertionError(f'{path} not classified')


class ClassificationTests(unittest.TestCase):
    def classify(self, baseline, current, target, mode='APPLY'):
        return classify.classify_files(baseline, current, target, mode=mode)

    def test_upstream_unchanged_is_the_only_auto_applied_class(self):
        result = self.classify({'kit/a.md': A}, {'kit/a.md': A}, {'kit/a.md': B})
        self.assertEqual(entry(result, 'kit/a.md')['class'], 'UPSTREAM_UNCHANGED')
        self.assertEqual(entry(result, 'kit/a.md')['decision'], 'APPLIED')
        self.assertEqual(result.counts['UPSTREAM_UNCHANGED'], 1)

    def test_plan_mode_defers_the_write(self):
        result = self.classify({'kit/a.md': A}, {'kit/a.md': A}, {'kit/a.md': B}, mode='PLAN')
        self.assertEqual(entry(result, 'kit/a.md')['decision'], 'DEFERRED')
        self.assertNotIn('APPLIED', [item['decision'] for item in result.files])

    def test_already_current(self):
        result = self.classify({'kit/a.md': A}, {'kit/a.md': B}, {'kit/a.md': B})
        item = entry(result, 'kit/a.md')
        self.assertEqual(item['class'], 'ALREADY_CURRENT')
        self.assertEqual(item['decision'], 'NOT_APPLICABLE')

    def test_local_modification_is_kept(self):
        result = self.classify({'kit/a.md': A}, {'kit/a.md': B}, {'kit/a.md': A})
        item = entry(result, 'kit/a.md')
        self.assertEqual(item['class'], 'LOCAL_MODIFICATION')
        self.assertEqual(item['decision'], 'KEPT_LOCAL')

    def test_three_way_conflict(self):
        result = self.classify({'kit/a.md': A}, {'kit/a.md': B}, {'kit/a.md': C})
        item = entry(result, 'kit/a.md')
        self.assertEqual(item['class'], 'THREE_WAY_CONFLICT')
        self.assertEqual(item['decision'], 'CONFLICT_REPORTED')
        self.assertEqual(result.conflicts, ['kit/a.md'])
        self.assertTrue(result.blocked)

    def test_project_owned_wins_even_over_the_baseline(self):
        result = self.classify({'docs/PROJECT.md': A}, {'docs/PROJECT.md': A},
                               {'docs/PROJECT.md': C})
        item = entry(result, 'docs/PROJECT.md')
        self.assertEqual(item['class'], 'PROJECT_OWNED')
        self.assertEqual(item['decision'], 'KEPT_LOCAL')

    def test_project_owned_prefixes(self):
        for path in ('docs/PROJECT.md', 'docs/PLAN.md', '.github/workflows/ci.yml',
                     '.local/notes.md', 'src/app.py', 'release/README.md'):
            with self.subTest(path=path):
                self.assertTrue(classify.is_project_owned(path))

    def test_kit_owned_namespaces_are_not_blanket_project_owned(self):
        for path in ('scripts/orchestrator.py', 'tests/test_orchestrator.py',
                     'kit/orchestration.md', 'AGENTS.md'):
            with self.subTest(path=path):
                self.assertFalse(classify.is_project_owned(path))

    def test_unknown_file(self):
        result = self.classify({'kit/a.md': A}, {'kit/a.md': A, 'scripts/local.py': B},
                               {'kit/a.md': B})
        item = entry(result, 'scripts/local.py')
        self.assertEqual(item['class'], 'UNKNOWN_FILE')
        self.assertEqual(item['decision'], 'KEPT_LOCAL')
        self.assertFalse(result.blocked)

    def test_upstream_removal_is_kept(self):
        result = self.classify({'kit/old.md': A}, {'kit/old.md': A}, {})
        item = entry(result, 'kit/old.md')
        self.assertEqual(item['class'], 'UPSTREAM_REMOVAL')
        self.assertEqual(item['decision'], 'KEPT_LOCAL')
        self.assertEqual(result.removals, ['kit/old.md'])
        self.assertTrue(result.blocked)

    def test_upstream_rename_by_content(self):
        result = self.classify({'kit/old.md': A}, {'kit/old.md': A}, {'kit/new.md': A})
        old = entry(result, 'kit/old.md')
        new = entry(result, 'kit/new.md')
        self.assertEqual(old['class'], 'UPSTREAM_RENAME')
        self.assertEqual(new['class'], 'UPSTREAM_RENAME')
        self.assertEqual(old['decision'], 'KEPT_LOCAL')
        self.assertEqual(new['decision'], 'KEPT_LOCAL')
        self.assertTrue(result.blocked)
        self.assertIn('kit/new.md', old['notes'])

    def test_new_upstream_file_is_upstream_unchanged(self):
        result = self.classify({}, {}, {'kit/new.md': B})
        item = entry(result, 'kit/new.md')
        self.assertEqual(item['class'], 'UPSTREAM_UNCHANGED')
        self.assertEqual(item['decision'], 'APPLIED')
        self.assertIsNone(item['baseline_sha256'])
        self.assertIsNone(item['current_sha256'])

    def test_new_upstream_path_that_exists_locally_is_unknown(self):
        result = self.classify({}, {'kit/new.md': A}, {'kit/new.md': B})
        item = entry(result, 'kit/new.md')
        self.assertEqual(item['class'], 'UNKNOWN_FILE')
        self.assertEqual(item['decision'], 'KEPT_LOCAL')

    def test_local_deletion_with_unchanged_upstream_is_kept(self):
        result = self.classify({'kit/a.md': A}, {}, {'kit/a.md': A})
        item = entry(result, 'kit/a.md')
        self.assertEqual(item['class'], 'LOCAL_MODIFICATION')
        self.assertEqual(item['decision'], 'KEPT_LOCAL')

    def test_local_deletion_with_changed_upstream_is_a_conflict(self):
        result = self.classify({'kit/a.md': A}, {}, {'kit/a.md': B})
        item = entry(result, 'kit/a.md')
        self.assertEqual(item['class'], 'THREE_WAY_CONFLICT')
        self.assertEqual(item['decision'], 'CONFLICT_REPORTED')

    def test_already_gone_upstream_and_locally(self):
        result = self.classify({'kit/a.md': A}, {}, {})
        item = entry(result, 'kit/a.md')
        self.assertEqual(item['class'], 'ALREADY_CURRENT')
        self.assertEqual(item['decision'], 'NOT_APPLICABLE')

    def test_counts_sum_to_the_number_of_files(self):
        result = self.classify(
            {'kit/a.md': A, 'kit/old.md': A, 'docs/PROJECT.md': A},
            {'kit/a.md': A, 'kit/old.md': A, 'docs/PROJECT.md': B, 'scripts/x.py': B},
            {'kit/a.md': B, 'kit/new.md': B},
        )
        self.assertEqual(sum(result.counts.values()), len(result.files))
        self.assertEqual(set(result.counts), set(classify.CLASSES))

    def test_never_applies_anything_but_upstream_unchanged(self):
        result = self.classify(
            {'kit/a.md': A, 'kit/b.md': A, 'kit/old.md': A, 'kit/ren.md': A,
             'docs/PROJECT.md': A, 'kit/c.md': A},
            {'kit/a.md': A, 'kit/b.md': B, 'kit/old.md': A, 'kit/ren.md': A,
             'docs/PROJECT.md': A, 'kit/c.md': A, 'scripts/local.py': B},
            {'kit/a.md': B, 'kit/b.md': C, 'kit/new.md': B, 'docs/PROJECT.md': C},
        )
        for item in result.files:
            if item['decision'] == 'APPLIED':
                self.assertEqual(item['class'], 'UPSTREAM_UNCHANGED', item)
        for item in result.files:
            if item['class'] in ('PROJECT_OWNED', 'UNKNOWN_FILE', 'UPSTREAM_REMOVAL',
                                 'UPSTREAM_RENAME', 'THREE_WAY_CONFLICT'):
                self.assertNotEqual(item['decision'], 'APPLIED', item)

    def test_every_class_is_emitted_by_at_least_one_scenario(self):
        emitted = set()
        emitted |= {entry(self.classify({'kit/a.md': A}, {'kit/a.md': A}, {'kit/a.md': B}),
                           'kit/a.md')['class']}
        emitted |= {entry(self.classify({'kit/a.md': A}, {'kit/a.md': B}, {'kit/a.md': B}),
                           'kit/a.md')['class']}
        emitted |= {entry(self.classify({'kit/a.md': A}, {'kit/a.md': B}, {'kit/a.md': A}),
                           'kit/a.md')['class']}
        emitted |= {entry(self.classify({'kit/a.md': A}, {'kit/a.md': B}, {'kit/a.md': C}),
                           'kit/a.md')['class']}
        emitted |= {entry(self.classify({'docs/PROJECT.md': A}, {'docs/PROJECT.md': A},
                                        {'docs/PROJECT.md': A}), 'docs/PROJECT.md')['class']}
        emitted |= {entry(self.classify({}, {'scripts/x.py': A}, {}), 'scripts/x.py')['class']}
        emitted |= {entry(self.classify({'kit/old.md': A}, {'kit/old.md': A}, {}),
                           'kit/old.md')['class']}
        emitted |= {entry(self.classify({'kit/old.md': A}, {'kit/old.md': A},
                                        {'kit/new.md': A}), 'kit/old.md')['class']}
        self.assertEqual(emitted, set(classify.CLASSES) - {'UNSAFE_PACKAGE', 'INVALID_BASELINE'})


if __name__ == '__main__':
    unittest.main()
