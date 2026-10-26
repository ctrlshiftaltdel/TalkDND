"""Structural tests for scripts/validate_kit.py (evidence matrix contract).

The field names / status enum are read back from kit/orchestration.md, so these
tests fail if the authoritative definition and the validator drift apart.
"""
import importlib.util, json, re, subprocess, sys, tempfile, unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'validate_kit.py'
spec = importlib.util.spec_from_file_location('validate_kit', SCRIPT)
validate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validate)

ROOT = SCRIPT.parents[1]
HEAD = 'a' * 40


def canonical():
    core = (ROOT / validate.CORE_DOC).read_text(encoding='utf-8')
    return validate.canonical_evidence_fields(core)


class TaskPacketMatrixTests(unittest.TestCase):
    def setUp(self):
        self.evidence_fields, self.status_values = canonical()
        self.structured = validate.canonical_packet_fields(
            (ROOT / validate.CORE_DOC).read_text(encoding='utf-8'))

    # --- builders -----------------------------------------------------------
    def header(self):
        return '| ' + ' | '.join(self.evidence_fields) + ' |'

    def row(self, ac='AC-1', status='PASS', actual='3 tests passed', sha=HEAD,
            notes='(none)', ref='task record: evidence log 1'):
        cells = {
            'ac_id': ac,
            'method': '`python -m unittest`',
            'expected': 'tests pass',
            'actual': actual,
            'status': status,
            'environment': 'Linux, Python 3.12, local edition',
            'commit_sha': sha,
            'evidence_ref': ref,
            'notes': notes,
        }
        return '  | ' + ' | '.join(cells[name] for name in self.evidence_fields) + ' |'

    def packet(self, acs=None, rows=None, structured=None, head=HEAD, omit=(), resources=None):
        acs = acs if acs is not None else [('x', 'AC-1', 'dry run writes nothing'),
                                           ('x', 'AC-2', 'existing behaviour unchanged')]
        rows = rows if rows is not None else [self.row('AC-1'), self.row('AC-2', actual='12 tests passed')]
        structured = structured if structured is not None else {
            name: 'N/A: covered by the criteria above' for name in self.structured}
        lines = []
        skipped = {validate.field_label(name) for name in omit}

        def add(label, value=''):
            if validate.field_label(label) in skipped:
                return
            lines.append(f'- **{label}:**' + (f' {value}' if value else ''))

        add('Task ID', 'SAMPLE-9001')
        add('Title', 'Add a dry-run flag')
        add('Goal / user-visible result', 'the export command can report without writing')
        add('Context', 'single command, no persisted state')
        add('Allowed paths', '`src/cli/export.py`')
        add('Forbidden paths', 'every other path in the repository')
        # `resources=None` means "field absent"; '' means "field present but empty".
        if resources is not None:
            add('declared_resources', resources)
        add('Dependencies (must be VERIFIED)', 'none')
        add('Acceptance criteria')
        for mark, ac_id, text in acs:
            lines.append(f'  - [{mark}] {ac_id}: {text}')
        for name in self.structured:
            add(name, structured.get(name, ''))
        add('Validation commands', '`python -m unittest`')
        add('Baseline branch / commit', '`main` @ `1111111`')
        add('Expected branch / Worktree', '`task/9001-dry-run` / `.worktrees/9001-dry-run`')
        add('Evidence head SHA', head)
        add('Evidence matrix')
        if 'evidence matrix' not in omit:
            lines.append(f'  {self.header()}')
            lines.append('  |' + '---|' * len(self.evidence_fields))
            lines.extend(rows)
        add('Risks / human decisions', 'none identified')
        add('Delivery evidence', 'changed paths, Diff, head SHA, exit codes')
        add('Hard boundaries', 'no scope expansion, no self-approval, no merge')
        return '\n'.join(lines) + '\n'

    def errors(self, text):
        return validate.validate_task_packet(text, self.evidence_fields, self.status_values,
                                             label='packet', structured_fields=self.structured)

    # --- contract extraction ------------------------------------------------
    def test_canonical_fields_are_read_from_the_core_document(self):
        self.assertEqual(self.evidence_fields, [
            'ac_id', 'method', 'expected', 'actual', 'status',
            'environment', 'commit_sha', 'evidence_ref', 'notes',
        ])
        self.assertEqual(set(self.status_values), set(validate.STATUS_VALUES))
        self.assertEqual(self.structured, [
            'invariants', 'negative_cases', 'migration_cases',
            'integration_paths', 'unverified_assumptions',
        ])

    # --- legal packets ------------------------------------------------------
    def test_legal_packet_passes(self):
        self.assertEqual(self.errors(self.packet()), [])

    def test_legal_packet_with_not_applicable_evidence_passes(self):
        acs = [('x', 'AC-1', 'dry run writes nothing'),
               (' ', 'AC-2', 'only applies when a remote target is configured')]
        rows = [self.row('AC-1'),
                self.row('AC-2', status='NOT_APPLICABLE', actual='n/a', sha='(none)',
                         notes='the flag cannot affect this path')]
        self.assertEqual(self.errors(self.packet(acs=acs, rows=rows)), [])

    def test_legal_packet_with_unchecked_criterion_and_not_run_passes(self):
        acs = [('x', 'AC-1', 'migration fills defaults'), (' ', 'AC-2', 'v1 saves migrate too')]
        rows = [self.row('AC-1'),
                self.row('AC-2', status='NOT_RUN', actual='not executed: fixture missing',
                         ref='task record: no run recorded')]
        self.assertEqual(self.errors(self.packet(acs=acs, rows=rows)), [])

    # --- missing / empty required content -----------------------------------
    def test_missing_structured_field_is_reported(self):
        errors = self.errors(self.packet(omit=('negative_cases',)))
        self.assertTrue(any('missing structured field: negative_cases' in e for e in errors), errors)

    def test_missing_required_field_is_reported(self):
        errors = self.errors(self.packet(omit=('allowed paths',)))
        self.assertTrue(any('missing required field: allowed paths' in e for e in errors), errors)

    def test_empty_required_field_is_reported(self):
        errors = self.errors(self.packet().replace('- **Title:** Add a dry-run flag', '- **Title:**'))
        self.assertTrue(any('required field is empty: title' in e for e in errors), errors)

    def test_bare_na_without_reason_is_reported(self):
        errors = self.errors(self.packet(structured={'negative_cases': 'N/A'}))
        self.assertTrue(any('must give a reason after "N/A:"' in e for e in errors), errors)

    def test_empty_acceptance_criterion_is_reported(self):
        acs = [('x', 'AC-1', '')]
        errors = self.errors(self.packet(acs=acs, rows=[self.row('AC-1')]))
        self.assertTrue(any('has no checkable statement' in e for e in errors), errors)

    def test_acceptance_criterion_without_id_is_reported(self):
        errors = self.errors(self.packet().replace('  - [x] AC-1:', '  - [x] '))
        self.assertTrue(any('without an AC-<n> id' in e for e in errors), errors)

    def test_duplicate_acceptance_id_is_reported(self):
        acs = [('x', 'AC-1', 'one'), ('x', 'AC-1', 'two')]
        errors = self.errors(self.packet(acs=acs, rows=[self.row('AC-1')]))
        self.assertTrue(any('duplicate acceptance criterion id: AC-1' in e for e in errors), errors)

    # --- evidence mapping ---------------------------------------------------
    def test_acceptance_criterion_without_evidence_is_reported(self):
        acs = [('x', 'AC-1', 'first'), ('x', 'AC-2', 'second')]
        errors = self.errors(self.packet(acs=acs, rows=[self.row('AC-1'), self.row('AC-9')]))
        self.assertTrue(any('AC-2 has no evidence item' in e for e in errors), errors)
        self.assertTrue(any('references unknown acceptance criterion: AC-9' in e for e in errors), errors)

    def test_matrix_without_rows_is_reported(self):
        errors = self.errors(self.packet(rows=[]))
        self.assertTrue(any('has no evidence items' in e for e in errors), errors)

    def test_wrong_matrix_columns_are_reported(self):
        text = self.packet()
        text = text.replace(self.header(), '| ac_id | status |')
        self.assertTrue(any('columns must be exactly' in e for e in self.errors(text)))

    def test_evidence_item_with_missing_field_is_reported(self):
        rows = [self.row('AC-1').replace(' | (none) |', ' |'), self.row('AC-2')]
        errors = self.errors(self.packet(rows=rows))
        self.assertTrue(any('fields, expected' in e for e in errors), errors)

    # --- "not executed" must not become "passing" ---------------------------
    def test_not_run_cannot_satisfy_a_checked_criterion(self):
        rows = [self.row('AC-1'),
                self.row('AC-2', status='NOT_RUN', actual='not executed')]
        errors = self.errors(self.packet(rows=rows))
        self.assertTrue(any('AC-2 is marked as met but has NOT_RUN evidence' in e for e in errors), errors)

    def test_pass_status_with_not_run_text_is_reported(self):
        for phrasing in ('not run: environment missing', 'not executed: fixture missing'):
            with self.subTest(actual=phrasing):
                rows = [self.row('AC-1', actual=phrasing), self.row('AC-2')]
                errors = self.errors(self.packet(rows=rows))
                self.assertTrue(any('is PASS but reports not-run' in e for e in errors), errors)

    def test_unknown_status_is_reported(self):
        rows = [self.row('AC-1', status='OK'), self.row('AC-2')]
        self.assertTrue(any('unknown status: OK' in e for e in self.errors(self.packet(rows=rows))))

    def test_not_applicable_without_reason_is_reported(self):
        rows = [self.row('AC-1'), self.row('AC-2', status='NOT_APPLICABLE', actual='n/a', sha='(none)')]
        errors = self.errors(self.packet(rows=rows))
        self.assertTrue(any('NOT_APPLICABLE without a reason' in e for e in errors), errors)

    # --- evidence binding to the head SHA -----------------------------------
    def test_empty_actual_is_reported(self):
        rows = [self.row('AC-1', actual=''), self.row('AC-2')]
        self.assertTrue(any('empty actual' in e for e in self.errors(self.packet(rows=rows))))

    def test_empty_commit_sha_is_reported(self):
        rows = [self.row('AC-1', sha=''), self.row('AC-2')]
        self.assertTrue(any('empty commit_sha' in e for e in self.errors(self.packet(rows=rows))))

    def test_sha_mismatch_is_reported(self):
        rows = [self.row('AC-1', sha='b' * 40), self.row('AC-2')]
        errors = self.errors(self.packet(rows=rows))
        self.assertTrue(any('does not match the declared evidence head SHA' in e for e in errors), errors)

    def test_abbreviated_head_sha_is_reported(self):
        errors = self.errors(self.packet(head='abc1234'))
        self.assertTrue(any('must be a full 40-hex' in e for e in errors), errors)

    # --- repository integration --------------------------------------------
    def test_template_samples_are_valid(self):
        text = (ROOT / validate.TASK_PACKET_DOC).read_text(encoding='utf-8')
        samples = validate.extract_packet_samples(text)
        self.assertGreaterEqual(len(samples), 2)
        for sample in samples:
            with self.subTest(sample=sample['name']):
                self.assertEqual(self.errors(sample['text']), [])

    def test_both_edition_templates_carry_every_field(self):
        for rel in (validate.TASK_PACKET_DOC, validate.ISSUE_TEMPLATE_DOC):
            text = (ROOT / rel).read_text(encoding='utf-8')
            for name in self.evidence_fields + self.structured:
                self.assertIn(f'`{name}`', text, f'{rel} is missing `{name}`')

    def test_kit_validation_passes_on_this_repository(self):
        errors, sample_count = validate.validate_kit(ROOT)
        self.assertEqual(errors, [])
        self.assertEqual(sample_count, 2)

    def test_cli_task_packet_accepts_legal_and_rejects_broken_packet(self):
        legal = self.packet()
        broken = self.packet(rows=[self.row('AC-1', sha='c' * 40), self.row('AC-2')])
        with tempfile.TemporaryDirectory() as tmp:
            ok = Path(tmp) / 'ok.md'
            bad = Path(tmp) / 'bad.md'
            ok.write_text(legal, encoding='utf-8')
            bad.write_text(broken, encoding='utf-8')
            good = subprocess.run([sys.executable, str(SCRIPT), '--task-packet', str(ok)],
                                  capture_output=True, text=True)
            fail = subprocess.run([sys.executable, str(SCRIPT), '--task-packet', str(bad)],
                                  capture_output=True, text=True)
        self.assertEqual(good.returncode, 0, good.stdout + good.stderr)
        self.assertIn('Task packet validation passed', good.stdout)
        self.assertEqual(fail.returncode, 1, fail.stdout + fail.stderr)
        self.assertIn('does not match the declared evidence head SHA', fail.stdout)


class ResourceDeclarationTests(unittest.TestCase):
    """The optional `declared_resources` field: shape only, empty is never a failure."""

    def setUp(self):
        self.evidence_fields, self.status_values = canonical()
        self.structured = validate.canonical_packet_fields(
            (ROOT / validate.CORE_DOC).read_text(encoding='utf-8'))
        # Reuse the canonical packet builder instead of duplicating the layout.
        self.builder = TaskPacketMatrixTests('test_legal_packet_passes')
        self.builder.evidence_fields = self.evidence_fields
        self.builder.structured = self.structured

    def errors(self, text):
        return validate.validate_task_packet(text, self.evidence_fields, self.status_values,
                                             label='packet', structured_fields=self.structured)

    def test_absent_and_empty_declarations_are_both_legal(self):
        self.assertEqual(self.errors(self.builder.packet()), [])                 # field absent
        self.assertEqual(self.errors(self.builder.packet(resources='')), [])     # field present, empty
        self.assertEqual(self.errors(self.builder.packet(resources='   ')), [])

    def test_well_formed_declaration_passes(self):
        value = 'db:test/mysql-main:exclusive,svc:build/win-01:shared:600,vol:data.1/0:shared'
        self.assertEqual(self.errors(self.builder.packet(resources=f'`{value}`')), [])

    def test_malformed_entries_are_reported(self):
        cases = {
            'db:test/main': 'mode shared or exclusive',
            'db:test/main:write': 'mode shared or exclusive',
            'DB:test/main:shared': 'mode shared or exclusive',
            'db:test/main:exclusive:0': 'mode shared or exclusive',
            'db:test main:shared': 'mode shared or exclusive',
            'db:test/*:shared': 'mode shared or exclusive',
            'db:test/../main:shared': 'empty or dots-only path segment',
            'db:test//main:shared': 'empty or dots-only path segment',
        }
        for value, needle in cases.items():
            with self.subTest(value=value):
                errors = self.errors(self.builder.packet(resources=value))
                self.assertTrue(any(needle in e for e in errors), errors)

    def test_trailing_comma_and_duplicates_are_reported(self):
        errors = self.errors(self.builder.packet(resources='db:a:shared,'))
        self.assertTrue(any('empty entry' in e for e in errors), errors)
        errors = self.errors(self.builder.packet(resources='db:a:shared,db:a:exclusive'))
        self.assertTrue(any('declares db:a more than once' in e for e in errors), errors)

    def test_templates_document_the_optional_fields(self):
        packet = (ROOT / validate.TASK_PACKET_DOC).read_text(encoding='utf-8')
        issue = (ROOT / validate.ISSUE_TEMPLATE_DOC).read_text(encoding='utf-8')
        self.assertIn(f'`{validate.RESOURCE_FIELD}`', packet)
        self.assertIn(f'`{validate.LOCKED_RESOURCE_FIELD}`', issue)
        self.assertIn('optional', packet.lower())
        # A filled-in sample must stay readable by the same validator.
        filled = self.builder.packet(resources='db:test/main:exclusive')
        self.assertEqual(validate.resource_declaration_errors(filled, 'packet'), [])

    def test_cli_rejects_a_broken_resource_declaration(self):
        broken = self.builder.packet(resources='db:Test main:write')
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'bad.md'
            path.write_text(broken, encoding='utf-8')
            result = subprocess.run([sys.executable, str(SCRIPT), '--task-packet', str(path)],
                                    capture_output=True, text=True)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('declared_resources', result.stdout)


class PlanLayerSplitTests(unittest.TestCase):
    """PLAN.md carries the design goals; phases and task blocks live in a docs plan."""

    def test_implementation_plan_template_is_in_the_release_manifest(self):
        self.assertIn('templates/implementation-plan.md', validate.REQUIRED)
        self.assertTrue((ROOT / 'templates' / 'implementation-plan.md').is_file())

    def test_design_template_points_to_the_separate_phase_plan(self):
        text = (ROOT / 'templates' / 'plan.md').read_text(encoding='utf-8')
        self.assertIn('templates/implementation-plan.md', text)
        for needle in ('Allowed paths', 'Depends on', 'AC-1'):
            self.assertNotIn(needle, text,
                             f'templates/plan.md must not carry the task block field {needle!r}')

    def test_legacy_root_plan_is_rejected_with_migration_hint(self):
        """Since 1.4 the design-goal authority is docs/PLAN.md; a root PLAN.md
        means two competing authorities, so validation fails closed with the
        exact migration command instead of silently accepting the old layout."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # A minimal tree: only the legacy file matters for this check.
            (root / 'PLAN.md').write_text('# legacy design goals\n', encoding='utf-8')
            errors, _ = validate.validate_kit(root)
            self.assertTrue(any('legacy root PLAN.md' in e and 'git mv PLAN.md docs/PLAN.md' in e
                                for e in errors), errors)

    def test_phase_plan_template_carries_phases_and_task_blocks(self):
        text = (ROOT / 'templates' / 'implementation-plan.md').read_text(encoding='utf-8')
        for needle in ('Phase', 'Goal', 'Allowed paths', 'Depends on', 'AC-1',
                       '当前状态', '维护规则'):
            self.assertIn(needle, text)

    @unittest.skipUnless((ROOT / 'docs').is_dir(), 'repository-only docs are not shipped')
    def test_documented_required_file_count_matches_the_manifest(self):
        readme_path = ROOT / 'docs' / 'README.md'
        if not readme_path.is_file():
            self.skipTest('docs/README.md is repository-only and absent inside a release package')
        readme = readme_path.read_text(encoding='utf-8')
        numbers = set(re.findall(r'(\d+)\s*项必需文件', readme))
        numbers |= set(re.findall(r'(\d+)\s*required files', readme))
        self.assertEqual(numbers, {str(len(validate.REQUIRED))})


class ContractTemplateTests(unittest.TestCase):
    """The reusable contract templates are structure-checked, not trusted.

    A deleted heading silently removes a question nobody will then answer, so a
    missing section must fail validation instead of yielding a shorter document.
    """

    NEW_TEMPLATES = ('templates/milestone-audit.md', 'templates/contract-decision-log.md')

    def copy_templates(self, root):
        """A throwaway root holding the real contract templates."""
        (root / 'templates').mkdir()
        for rel in validate.TEMPLATE_SECTIONS:
            (root / rel).write_text((ROOT / rel).read_text(encoding='utf-8'), encoding='utf-8')

    def test_new_templates_are_required_and_present(self):
        for rel in self.NEW_TEMPLATES:
            self.assertIn(rel, validate.REQUIRED)
            self.assertTrue((ROOT / rel).is_file(), f'{rel} is missing')

    def test_repository_templates_are_structurally_complete(self):
        self.assertEqual(validate.template_structure_errors(ROOT), [])

    def test_kit_validation_covers_the_templates(self):
        errors, _ = validate.validate_kit(ROOT)
        self.assertEqual(errors, [])

    def test_missing_section_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.copy_templates(root)
            target = root / 'templates' / 'milestone-audit.md'
            text = target.read_text(encoding='utf-8')
            self.assertIn('## 7. Recoverability', text)
            target.write_text(text.replace('## 7. Recoverability', '## Restart notes'),
                              encoding='utf-8')
            errors = validate.template_structure_errors(root)
        self.assertTrue(
            any('templates/milestone-audit.md is missing required section: Recoverability' in e
                for e in errors), errors)

    def test_missing_template_file_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'templates').mkdir()
            errors = validate.template_structure_errors(root)
            kit_errors, _ = validate.validate_kit(root)
        for rel in validate.TEMPLATE_SECTIONS:
            self.assertTrue(any(f'missing required template: {rel}' in e for e in errors), errors)
        for rel in self.NEW_TEMPLATES:
            self.assertTrue(any(f'missing required file: {rel}' in e for e in kit_errors), kit_errors)

    def test_broken_relative_link_inside_a_template_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.copy_templates(root)
            target = root / 'templates' / 'design.md'
            target.write_text(target.read_text(encoding='utf-8')
                              + '\nSee [missing decision](does-not-exist.md).\n', encoding='utf-8')
            errors = validate.template_structure_errors(root)
        self.assertTrue(any('templates/design.md: broken relative link: does-not-exist.md' in e
                            for e in errors), errors)

    def test_design_contract_states_freeze_is_not_permanent(self):
        text = (ROOT / 'templates' / 'design.md').read_text(encoding='utf-8')
        self.assertIn('not permanence', text)
        self.assertIn('contract-decision-log.md', text)
        for needle in ('N/A: <reason>', 'kit/orchestration.md'):
            self.assertIn(needle, text)

    def test_templates_reference_instead_of_restating_the_core_contract(self):
        for rel in ('templates/design.md', 'templates/milestone-audit.md'):
            text = (ROOT / rel).read_text(encoding='utf-8')
            self.assertIn('kit/orchestration.md', text)
            for state in ('DISPATCHED', 'MASTER_REVIEW', 'APPROVED', 'VERIFIED'):
                self.assertNotIn(state, text, f'{rel} must not copy the task lifecycle')

    def test_milestone_audit_keeps_the_verdict_classes_apart(self):
        text = (ROOT / 'templates' / 'milestone-audit.md').read_text(encoding='utf-8')
        for needle in ('BLOCKING', 'NON_BLOCKING_HARDENING', 'OPTIONAL',
                       'NEEDS_HUMAN_DECISION', 'N/A: <reason>'):
            self.assertIn(needle, text)
        self.assertIn('must not be marked complete', text)

    def test_decision_log_separates_frozen_open_and_revisitable(self):
        text = (ROOT / 'templates' / 'contract-decision-log.md').read_text(encoding='utf-8')
        for status in ('`OPEN`', '`FROZEN`', '`REVISITABLE`', '`SUPERSEDED`'):
            self.assertIn(status, text)
        for column in ('Decider', 'Affected tasks', 'Before', 'After', 'Rationale', 'Evidence'):
            self.assertIn(column, text)

    @unittest.skipUnless((ROOT / 'docs' / 'README.md').is_file(),
                         'upstream README is not vendored downstream')
    def test_readme_and_acceptance_programme_point_at_the_templates(self):
        readme = (ROOT / 'docs' / 'README.md').read_text(encoding='utf-8')
        for rel in ('templates/design.md', 'templates/contract-decision-log.md',
                    'templates/milestone-audit.md'):
            self.assertIn(rel, readme)
        acceptance = (ROOT / 'kit/orchestration.md').read_text(encoding='utf-8')
        for rel in ('templates/milestone-audit.md', 'templates/contract-decision-log.md'):
            self.assertIn(rel, acceptance)


class CapabilityAndMetricsTemplateTests(unittest.TestCase):
    """The capability register and the metric definitions are contracts too.

    A dropped capability row silently claims a capability nobody probed; a
    dropped metric column leaves the start / end event, timezone, exclusion or
    missing-data rule undefined. Both fail validation instead of yielding a
    shorter document that still looks authoritative.
    """

    CAPABILITY = 'templates/runtime-capability.md'
    METRICS = 'templates/metrics.md'

    def test_templates_are_required_and_present(self):
        for rel in (self.CAPABILITY, self.METRICS):
            self.assertIn(rel, validate.REQUIRED)
            self.assertTrue((ROOT / rel).is_file(), f'{rel} is missing')

    def test_kit_validation_covers_both_templates(self):
        self.assertEqual(validate.template_structure_errors(ROOT), [])
        errors, _ = validate.validate_kit(ROOT)
        self.assertEqual(errors, [])

    def test_capability_template_covers_the_contract_and_the_fine_boundaries(self):
        text = (ROOT / self.CAPABILITY).read_text(encoding='utf-8')
        for capability in ('| C1 |', '| C2 |', '| C3 |', '| C4 |', '| C5 |', '| C6 |', '| C7 |'):
            self.assertIn(capability, text)
        for needle in ('UNKNOWN', 're-probe', 'spawned-subagent', 'manual-handoff',
                       'Probe date (UTC)', 'evidence_ref', 'Limits'):
            self.assertIn(needle, text)
        # A record must not become a permission grant or a second definition of C1-C4.
        self.assertIn('not a permission grant', text)
        self.assertIn('kit/orchestration.md', text)

    def test_capability_template_refuses_a_stale_pass(self):
        text = (ROOT / self.CAPABILITY).read_text(encoding='utf-8')
        self.assertIn('Downgrade a verdict to `UNKNOWN`', text)
        self.assertIn('never unlocks the sub-agent path', text)

    def test_metrics_template_defines_seven_metrics_with_precise_columns(self):
        text = (ROOT / self.METRICS).read_text(encoding='utf-8')
        header = next(line for line in text.splitlines() if line.startswith('| ID | Metric'))
        for column in ('Start event', 'End event', 'Timezone', 'Exclusions',
                       'Missing handling'):
            self.assertIn(column, header)
        for mid in ('**M1**', '**M2**', '**M3**', '**M4**', '**M5**', '**M6**', '**M7**'):
            self.assertIn(mid, text)

    def test_metrics_template_forbids_context_free_ranking(self):
        text = (ROOT / self.METRICS).read_text(encoding='utf-8')
        self.assertIn('No context-free ranking', text)
        self.assertIn('sample size', text)
        self.assertIn('Missing is not zero', text)

    def test_metrics_gold_sample_is_valid_json_and_covers_every_case(self):
        text = (ROOT / self.METRICS).read_text(encoding='utf-8')
        start = text.index('```metrics-sample') + len('```metrics-sample')
        end = text.index('```', start)
        payload = json.loads(text[start:end])
        self.assertEqual(len(payload['tasks']), 9)
        statuses = {event.get('to_status') for event in payload['events']}
        for terminal in ('VERIFIED', 'FAILED', 'CANCELED', 'BLOCKED', 'MERGED', 'APPROVED'):
            self.assertIn(terminal, statuses)

    def test_missing_section_in_either_template_is_reported(self):
        cases = ((self.METRICS, '## Gold sample', '## Sample notes'),
                 (self.CAPABILITY, '## UNKNOWN and re-probe', '## Re-probe notes'))
        for rel, old, new in cases:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.copy_templates(root)
                target = root / rel
                text = target.read_text(encoding='utf-8')
                self.assertIn(old, text)
                target.write_text(text.replace(old, new), encoding='utf-8')
                errors = validate.template_structure_errors(root)
            expected = f'{rel} is missing required section: {"Gold sample" if "Gold" in old else "UNKNOWN and re-probe"}'
            self.assertTrue(any(expected in error for error in errors), errors)

    @unittest.skipUnless((ROOT / 'docs' / 'README.md').is_file(),
                         'upstream README is not vendored downstream')
    def test_readme_and_acceptance_point_at_the_new_artifacts(self):
        readme = (ROOT / 'docs' / 'README.md').read_text(encoding='utf-8')
        for rel in (self.CAPABILITY, self.METRICS, 'scripts/report_metrics.py'):
            self.assertIn(rel, readme)
        acceptance = (ROOT / 'kit/orchestration.md').read_text(encoding='utf-8')
        for rel in (self.CAPABILITY, self.METRICS):
            self.assertIn(rel, acceptance)

    def copy_templates(self, root):
        (root / 'templates').mkdir()
        for rel in validate.TEMPLATE_SECTIONS:
            (root / rel).write_text((ROOT / rel).read_text(encoding='utf-8'), encoding='utf-8')


class AuditConclusionTests(unittest.TestCase):
    """A filled audit may only be read as a closeout when no verdict forbids it.

    The four verdict classes come from templates/milestone-audit.md; this check
    only reads the classification back, it never judges whether a verdict is
    justified.
    """

    HEADER = '| 检查项 | verdict | 依据 |\n|---|---|---|\n'

    def audit(self, rows):
        return validate.audit_conclusion_errors('## 阶段审计结论\n\n' + self.HEADER + rows)

    def test_clean_closeout_passes(self):
        rows = ('| functional completeness | PASS | task 1, head aaaa |\n'
                '| residual items | NON_BLOCKING_HARDENING | tracked |\n'
                '| performance baseline | OPTIONAL | not measured |\n'
                '| legacy data migration | NOT_APPLICABLE | N/A: no legacy data |\n')
        self.assertEqual(self.audit(rows), [])

    def test_blocking_forbids_the_closeout_claim(self):
        errors = self.audit('| permissions and security | BLOCKING | probe missing |\n')
        self.assertTrue(any('BLOCKING' in e and 'must not be' in e for e in errors), errors)

    def test_needs_human_decision_forbids_the_closeout_claim(self):
        errors = self.audit('| legacy data migration | NEEDS_HUMAN_DECISION | owner undecided |\n')
        self.assertTrue(any('NEEDS_HUMAN_DECISION' in e and 'must not be' in e for e in errors), errors)

    def test_not_run_is_not_counted_as_passing(self):
        errors = self.audit('| end-to-end path | NOT_RUN | environment missing |\n')
        self.assertTrue(any('not counted as passing' in e for e in errors), errors)

    def test_unknown_verdict_is_rejected(self):
        errors = self.audit('| recoverability | LOOKS_FINE | trust me |\n')
        self.assertTrue(any('unknown verdict' in e for e in errors), errors)

    def test_table_without_verdict_rows_is_rejected(self):
        errors = validate.audit_conclusion_errors('## 阶段审计结论\n\n全部通过。\n')
        self.assertTrue(any('no readable verdict rows' in e for e in errors), errors)


class DeliveryScenarioIntegrationTests(unittest.TestCase):
    """The scenario runner and its catalogue must be part of the kit's own checks."""

    RUNNER = 'scripts/run_delivery_scenarios.py'
    RUNNER_TEST = 'tests/test_run_delivery_scenarios.py'
    SCENARIO_DOC = 'docs/testing/delivery-regression-scenarios.md'

    def test_runner_and_its_test_ship_with_the_kit(self):
        self.assertIn(self.RUNNER, validate.REQUIRED)
        self.assertIn(self.RUNNER_TEST, validate.REQUIRED)
        for rel in (self.RUNNER, self.RUNNER_TEST):
            self.assertTrue((ROOT / rel).is_file(), f'{rel} is missing')

    @unittest.skipUnless((ROOT / 'docs').is_dir(), 'repository-only docs are not shipped')
    def test_scenario_document_is_a_repository_only_requirement(self):
        # It cannot be in REQUIRED: build_release.py refuses any path with a
        # `docs` segment, so shipping it would fail the release self-check.
        self.assertIn(self.SCENARIO_DOC, validate.UPSTREAM_DOCS)
        self.assertNotIn(self.SCENARIO_DOC, validate.REQUIRED)

    @unittest.skipUnless((ROOT / 'docs' / 'upgrade.md').is_file(),
                         'upstream repository docs are not vendored downstream')
    def test_scenario_document_is_structurally_complete(self):
        self.assertEqual(validate.required_doc_errors(ROOT, upstream=True), [])

    def test_missing_scenario_document_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            errors = validate.required_doc_errors(root, upstream=True)
        self.assertTrue(any(f'missing required document: {self.SCENARIO_DOC}' in e
                            for e in errors), errors)

    def test_document_without_a_required_section_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            target = root / self.SCENARIO_DOC
            target.parent.mkdir(parents=True)
            target.write_text('# Scenarios\n\n## 场景矩阵\n\nno other section\n', encoding='utf-8')
            errors = validate.required_doc_errors(root, upstream=True)
        self.assertTrue(any('is missing required section' in e for e in errors), errors)

    def test_document_check_is_skipped_inside_a_release_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(validate.required_doc_errors(Path(tmp)), [])

    @unittest.skipUnless((ROOT / 'docs' / 'README.md').is_file(),
                         'upstream README is not vendored downstream')
    def test_readme_and_acceptance_point_at_the_scenario_runner(self):
        readme = (ROOT / 'docs' / 'README.md').read_text(encoding='utf-8')
        acceptance = (ROOT / 'kit/orchestration.md').read_text(encoding='utf-8')
        self.assertIn(self.RUNNER, readme)
        self.assertIn(self.RUNNER, acceptance)
        self.assertIn(self.SCENARIO_DOC, readme)


class DownstreamManifestModeTests(unittest.TestCase):
    """A downstream project has docs/ but not the upstream docs/upgrade.md."""

    def test_downstream_tree_detection(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.assertFalse(validate.downstream_tree(root))
            (root / 'docs').mkdir()
            self.assertTrue(validate.downstream_tree(root))
            (root / 'docs' / 'upgrade.md').write_text('# 升级\n', encoding='utf-8')
            self.assertFalse(validate.downstream_tree(root))

    def test_project_docs_are_checked_downstream(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            errors = validate.required_doc_errors(root)
            self.assertTrue(any('docs/PROJECT.md' in e for e in errors), errors)
            self.assertTrue(any('docs/PLAN.md' in e for e in errors), errors)
            self.assertFalse(any('docs/upgrade.md' in e for e in errors), errors)

    def test_upstream_docs_are_checked_in_the_upstream_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            (root / 'docs' / 'upgrade.md').write_text('# 升级\n', encoding='utf-8')
            errors = validate.required_doc_errors(root)
            self.assertTrue(any('docs/README.md' in e for e in errors), errors)
            self.assertTrue(any('docs/testing/delivery-regression-scenarios.md' in e
                                for e in errors), errors)

    def _manifest_root(self, tmp):
        root = Path(tmp)
        manifest = ROOT / validate.MANIFEST_DOC
        self.assertTrue(manifest.is_file(), 'the 1.6 baseline manifest is missing')
        (root / validate.MANIFEST_DOC).write_bytes(manifest.read_bytes())
        (root / 'AGENTS.md').write_text('locally customized\n', encoding='utf-8')
        return root

    def test_manifest_hashes_are_strict_in_a_release_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._manifest_root(tmp)
            errors = validate.check_release_manifest(root)
            self.assertTrue(any('sha256 does not match the file: AGENTS.md' in e
                                for e in errors), errors)

    def test_manifest_hashes_are_diagnostic_downstream(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self._manifest_root(tmp)
            (root / 'docs').mkdir()
            errors = validate.check_release_manifest(root)
            self.assertEqual(errors, [])


class MarkdownLinkScanTests(unittest.TestCase):
    """Links inside code spans / fences are literal text, not live links.

    The image convention is taught by example (`![图](images/x.png)` inside
    backticks), so the scan must not report those as broken; a real link to a
    missing file is still flagged, and a link to a real binary (an image)
    resolves.
    """
    def test_code_spans_are_skipped_and_real_breaks_are_flagged(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'AGENTS.md').write_text(
                '# A\n\n用 `![图](images/x.png)` 引用；另有 [坏链](nope/missing.md)。\n',
                encoding='utf-8')
            errors, _ = validate.validate_kit(root)
            self.assertFalse(any('broken relative link: images/x.png' in e for e in errors), errors)
            self.assertTrue(any('broken relative link: nope/missing.md' in e for e in errors), errors)

    def test_image_links_to_real_binaries_resolve(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs' / 'images').mkdir(parents=True)
            (root / 'docs' / 'images' / 'a.png').write_bytes(b'\x89PNG\r\n\x1a\n')
            (root / 'docs' / 'PLAN.md').write_text('![图 1](images/a.png) 配图说明：架构总览。\n',
                                                   encoding='utf-8')
            errors, _ = validate.validate_kit(root)
            self.assertFalse(any('broken relative link' in e for e in errors), errors)

    def test_local_books_and_generated_dirs_are_not_scanned(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            book = root / 'LetsPlay' / 'Adventures' / 'example' / 'index' / 'index.md'
            book.parent.mkdir(parents=True)
            book.write_text('[broken](missing.md)\n', encoding='utf-8')
            rulebook = root / 'LetsPlay' / 'Rules' / '2014' / 'example' / 'index' / 'index.md'
            rulebook.parent.mkdir(parents=True)
            rulebook.write_text('[broken](missing.md)\n', encoding='utf-8')
            generated = root / '.md' / 'example' / 'page.md'
            generated.parent.mkdir(parents=True)
            generated.write_text('[broken](missing.md)\n', encoding='utf-8')
            (root / 'README.md').write_text('[broken](missing.md)\n', encoding='utf-8')
            saved_readme = root / 'LetsPlay' / 'Saves' / 'README.md'
            saved_readme.parent.mkdir(parents=True)
            saved_readme.write_text('[broken](missing.md)\n', encoding='utf-8')
            errors, _ = validate.validate_kit(root)
            messages = '\n'.join(errors)
            self.assertIn('README.md: broken relative link: missing.md', messages)
            self.assertIn('LetsPlay/Saves/README.md: broken relative link: missing.md', messages)
            self.assertNotIn('LetsPlay/Adventures', messages)
            self.assertNotIn('LetsPlay/Rules', messages)
            self.assertNotIn('.md/example', messages)


if __name__ == '__main__':
    unittest.main()
