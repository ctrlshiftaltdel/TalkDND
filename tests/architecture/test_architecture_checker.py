"""Behavioural tests for the optional Architecture Contract checker.

Every test materialises a throwaway project tree, runs the real CLI in a child
process and asserts on the exit code plus the machine-readable report. The
fixtures are declarative (a contract dict + a source tree), so #53 can reuse the
same shapes for synthetic regression without copying directories around.

Hard rules (NO_CYCLE / FORBIDDEN_EDGE / LAYER_DIRECTION / PRIVATE_ACCESS) each
have a positive and a negative fixture: the positive fixture must not block and
the negative one must exit non-zero and point at rule id, module and file.
"""
import ast
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / 'scripts' / 'architecture' / 'check.py'

MODULES = [
    {
        'module_id': 'app',
        'paths': ['src/app/'],
        'responsibilities': ['application entry points and wiring'],
        'public_interfaces': [{'name': 'src.app.main', 'kind': 'python_module',
                               'stability': 'stable'}],
    },
    {
        'module_id': 'domain',
        'paths': ['src/domain/'],
        'responsibilities': ['domain rules and state ownership'],
        'public_interfaces': [{'name': 'src.domain.model', 'kind': 'python_module',
                               'stability': 'stable'}],
    },
    {
        'module_id': 'infra',
        'paths': ['src/infra/'],
        'responsibilities': ['database and filesystem adapters'],
        'public_interfaces': [{'name': 'src.infra.db', 'kind': 'python_module',
                               'stability': 'additive_only'}],
    },
]

NO_CYCLE = {
    'rule_id': 'ARCH-NO-CYCLE',
    'kind': 'NO_CYCLE',
    'severity': 'ERROR',
    'enabled': True,
    'description': 'modules must not form dependency cycles',
}


def contract(**overrides):
    """A valid contract; keyword arguments replace top-level keys."""
    data = {
        'schema_version': '1.0',
        'contract_version': '1.0.0',
        'project_name': 'fixture',
        'enforcement_mode': 'ENFORCE',
        'modules': MODULES,
        'dependencies': {
            'allowed': [{'from': 'app', 'to': 'domain',
                         'reason': 'app orchestrates domain rules'}],
            'forbidden': [],
        },
        'rules': [NO_CYCLE],
        'baseline': {'frozen_at': '2026-01-01T00:00:00Z', 'frozen_by': 'fixture'},
    }
    data.update(overrides)
    return data


class CheckerTestBase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)

    def build(self, files, contract_data=None, project_md=None):
        """Write a project tree; a contract dict goes to the default path."""
        if contract_data is not None:
            files = dict(files)
            files['docs/architecture/contract.json'] = json.dumps(contract_data, indent=2)
        if project_md is not None:
            files = dict(files)
            files['docs/PROJECT.md'] = project_md
        for rel, text in files.items():
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding='utf-8')
        return self.root

    def run_check(self, *args):
        proc = subprocess.run(
            [sys.executable, str(CHECK), '--project', str(self.root), '--json', *args],
            capture_output=True, text=True)
        payload = None
        if proc.stdout.strip():
            try:
                payload = json.loads(proc.stdout)
            except ValueError:
                payload = None
        return proc.returncode, payload, proc

    def result(self, *args):
        code, payload, proc = self.run_check(*args)
        self.assertIsNotNone(payload, f'no JSON report on stdout\n{proc.stdout}\n{proc.stderr}')
        return code, payload

    @staticmethod
    def findings(payload, rule_id=None, status=None):
        found = payload.get('findings', [])
        if rule_id is not None:
            found = [f for f in found if f.get('rule_id') == rule_id]
        if status is not None:
            found = [f for f in found if f.get('status') == status]
        return found


class ContractOptionalityTests(CheckerTestBase):
    def test_no_contract_is_diagnostic_only_and_exit_zero(self):
        """AC-3: an unconfigured project gets a read-only diagnosis, never a gate."""
        self.build({'src/app/main.py': 'x = 1\n', 'src/domain/model.py': 'y = 2\n'})
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertIsNone(payload['contract_path'])
        self.assertEqual(payload['enforcement_mode'], 'OFF')
        self.assertTrue(payload['diagnostic_only'])
        self.assertFalse(payload['blocking'])
        self.assertEqual(payload['counts']['ERROR'], 0)
        self.assertEqual(payload['status'], 'NOT_RUN')
        self.assertTrue(any('no architecture contract' in note for note in payload['notes']),
                        payload['notes'])

    def test_declared_contract_that_is_missing_fails_closed(self):
        """A declared path that does not resolve is a config error, not OFF."""
        self.build(
            {'src/app/main.py': 'x = 1\n'},
            project_md='- `edition`: `github`\n'
                       '- `architecture_contract_path`: `docs/architecture/contract.json`\n')
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(payload['blocking'])
        self.assertTrue(any('contract.json' in e and 'missing' in e
                            for e in payload['contract_errors']), payload['contract_errors'])

    def test_project_md_override_path_is_used(self):
        files = {'src/app/main.py': 'x = 1\n',
                 'config/arch/contract.json': json.dumps(contract(), indent=2)}
        self.build(files, project_md='- `architecture_contract_path`: `config/arch/contract.json`\n')
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload['contract_path'], 'config/arch/contract.json')
        self.assertEqual(payload['contract_version'], '1.0.0')

    def test_contract_path_outside_the_project_is_rejected(self):
        # One sibling file per test instance: a fixed name (e.g. /tmp/escape.json)
        # is shared state, so concurrent runs would delete each other's fixture.
        # The file must stay a *sibling* of the project root, otherwise the path
        # would no longer escape the project and the rejection would go untested.
        with tempfile.NamedTemporaryFile(
                dir=self.root.parent, prefix='escape-', suffix='.json',
                mode='w', encoding='utf-8', delete=False) as handle:
            outside = Path(handle.name)
            handle.write(json.dumps(contract(), indent=2))
        self.addCleanup(outside.unlink, missing_ok=True)
        relative = (Path('..') / outside.name).as_posix()
        self.build({'src/app/main.py': 'x = 1\n'},
                   project_md=f'- `architecture_contract_path`: `{relative}`\n')
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(any('outside the project' in e for e in payload['contract_errors']),
                        payload['contract_errors'])

    def test_diagnostic_only_never_blocks_even_with_a_violation(self):
        files = {
            'src/app/a.py': 'from src.domain import b\n',
            'src/domain/b.py': 'from src.app import a\n',
        }
        self.build(files, contract(enforcement_mode='DIAGNOSTIC_ONLY'))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertFalse(payload['blocking'])
        self.assertEqual(payload['status'], 'ERROR')  # the violation stays visible
        self.assertTrue(self.findings(payload, 'ARCH-NO-CYCLE', 'ERROR'))


class ContractValidationTests(CheckerTestBase):
    def assert_contract_error(self, bad_contract, needle):
        self.build({'src/app/main.py': 'x = 1\n'}, bad_contract)
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(any(needle in error for error in payload['contract_errors']),
                        payload['contract_errors'])
        self.assertNotEqual(payload['status'], 'PASS')
        self.assertFalse(self.findings(payload, status='PASS'),
                         'a broken contract must never produce a PASS finding')

    def test_missing_required_field_is_a_config_error(self):
        bad = contract()
        del bad['rules']
        self.assert_contract_error(bad, 'rules')

    def test_unsupported_schema_version_fails_closed(self):
        self.assert_contract_error(contract(schema_version='0.9'), 'unsupported')

    def test_illegal_enum_is_a_config_error(self):
        self.assert_contract_error(contract(enforcement_mode='SOMETIMES'), 'enforcement_mode')

    def test_duplicate_module_id_is_a_semantic_error(self):
        modules = MODULES + [dict(MODULES[0])]
        self.assert_contract_error(contract(modules=modules), 'duplicate module_id')

    def test_module_path_traversal_is_rejected(self):
        modules = [dict(MODULES[0], paths=['../outside/'])] + MODULES[1:]
        self.assert_contract_error(contract(modules=modules), 'relative path')

    def test_forbidden_edge_referencing_an_unknown_rule_id_is_rejected(self):
        bad = contract(dependencies={
            'allowed': [],
            'forbidden': [{'from': 'app', 'to': 'infra', 'reason': 'no direct access',
                           'rule_id': 'ARCH-MISSING'}],
        })
        self.assert_contract_error(bad, 'unknown rule_id')

    def test_unknown_rule_param_is_not_silently_ignored(self):
        rule = dict(NO_CYCLE, params={'bogus': True})
        self.assert_contract_error(contract(rules=[rule]), 'unsupported param')

    def test_invalid_contract_in_diagnostic_mode_exits_zero_but_reports(self):
        bad = contract(enforcement_mode='DIAGNOSTIC_ONLY')
        del bad['rules']
        self.build({'src/app/main.py': 'x = 1\n'}, bad)
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertFalse(payload['blocking'])
        self.assertTrue(payload['contract_errors'])
        self.assertEqual(payload['status'], 'UNKNOWN')


class NoCycleRuleTests(CheckerTestBase):
    POSITIVE = {
        'src/app/main.py': 'from src.domain import model\n',
        'src/domain/model.py': 'VALUE = 1\n',
    }
    NEGATIVE = {
        'src/app/a.py': 'from src.domain import b\n',
        'src/domain/b.py': 'from src.app import a\n',
    }

    def test_positive_fixture_does_not_block(self):
        self.build(self.POSITIVE, contract())
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload['counts']['ERROR'], 0)
        self.assertEqual(payload['status'], 'PASS')

    def test_negative_fixture_blocks_and_locates_module_file_and_rule(self):
        self.build(self.NEGATIVE, contract())
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(payload['blocking'])
        hits = self.findings(payload, 'ARCH-NO-CYCLE', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        hit = hits[0]
        self.assertEqual(sorted(hit['modules']), ['app', 'domain'])
        self.assertIn(hit['file'], self.NEGATIVE)
        self.assertGreaterEqual(hit['line'], 1)
        self.assertIn('cycle', hit['message'])

    def test_human_output_names_the_rule_module_and_file(self):
        self.build(self.NEGATIVE, contract())
        proc = subprocess.run(
            [sys.executable, str(CHECK), '--project', str(self.root)],
            capture_output=True, text=True)
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertIn('ARCH-NO-CYCLE', proc.stdout)
        self.assertIn('app', proc.stdout)
        self.assertIn('src/', proc.stdout)


class ForbiddenEdgeRuleTests(CheckerTestBase):
    RULE = {
        'rule_id': 'ARCH-FORBID-INFRA',
        'kind': 'FORBIDDEN_EDGE',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'application code must not reach infrastructure directly',
    }

    def forbidden_contract(self, forbidden=None):
        return contract(
            dependencies={
                'allowed': [],
                'forbidden': forbidden if forbidden is not None else [
                    {'from': 'app', 'to': 'infra',
                     'reason': 'route through the domain module',
                     'rule_id': 'ARCH-FORBID-INFRA'}],
            },
            rules=[self.RULE])

    def test_positive_fixture_does_not_block(self):
        files = {'src/app/main.py': 'from src.domain import model\n',
                 'src/domain/model.py': 'from src.infra import db\n',
                 'src/infra/db.py': 'CONN = None\n'}
        self.build(files, self.forbidden_contract())
        code, payload = self.result()
        self.assertEqual(code, 0, payload)

    def test_negative_fixture_blocks_with_edge_location(self):
        files = {'src/app/main.py': 'from src.infra import db\n',
                 'src/infra/db.py': 'CONN = None\n'}
        self.build(files, self.forbidden_contract())
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        hits = self.findings(payload, 'ARCH-FORBID-INFRA', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertEqual(hits[0]['module_id'], 'app')
        self.assertEqual(hits[0]['target_module'], 'infra')
        self.assertEqual(hits[0]['file'], 'src/app/main.py')
        self.assertEqual(hits[0]['line'], 1)

    def test_wildcard_forbidden_edge_matches_any_target(self):
        bad = self.forbidden_contract(forbidden=[
            {'from': '*', 'to': 'infra',
             'reason': 'infrastructure is adapter-only',
             'rule_id': 'ARCH-FORBID-INFRA'}])
        files = {'src/domain/model.py': 'from src.infra import db\n',
                 'src/infra/db.py': 'CONN = None\n'}
        self.build(files, bad)
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(self.findings(payload, 'ARCH-FORBID-INFRA', 'ERROR'))


class LayerDirectionRuleTests(CheckerTestBase):
    RULE = {
        'rule_id': 'ARCH-LAYERS',
        'kind': 'LAYER_DIRECTION',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'dependencies may only point from outer to inner layers',
        'params': {'order': ['app', 'domain', 'infra'], 'direction': 'downward'},
    }

    def test_positive_fixture_does_not_block(self):
        files = {'src/app/main.py': 'from src.domain import model\n',
                 'src/domain/model.py': 'from src.infra import db\n',
                 'src/infra/db.py': 'CONN = None\n'}
        self.build(files, contract(rules=[self.RULE]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)

    def test_negative_fixture_blocks_on_reverse_layer(self):
        files = {'src/domain/model.py': 'from src.app import main\n',
                 'src/app/main.py': 'VALUE = 1\n'}
        self.build(files, contract(rules=[self.RULE]))
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        hits = self.findings(payload, 'ARCH-LAYERS', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertEqual(hits[0]['module_id'], 'domain')
        self.assertEqual(hits[0]['target_module'], 'app')
        self.assertEqual(hits[0]['file'], 'src/domain/model.py')
        self.assertIn('layer', hits[0]['message'])

    def test_missing_order_param_is_reported_as_unknown_not_pass(self):
        rule = dict(self.RULE)
        rule.pop('params')
        self.build({'src/app/main.py': 'x = 1\n'}, contract(rules=[rule]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        hits = self.findings(payload, 'ARCH-LAYERS', 'UNKNOWN')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertIn('order', hits[0]['message'])


class PrivateAccessRuleTests(CheckerTestBase):
    RULE = {
        'rule_id': 'ARCH-PRIVATE',
        'kind': 'PRIVATE_ACCESS',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'other modules must not import private implementation paths',
    }

    def private_contract(self):
        modules = [dict(m) for m in MODULES]
        modules[1]['private_paths'] = ['src/domain/internal/']
        return contract(modules=modules, rules=[self.RULE],
                        dependencies={'allowed': [], 'forbidden': []})

    def test_positive_fixture_imports_a_public_path(self):
        files = {'src/app/main.py': 'from src.domain import model\n',
                 'src/domain/model.py': 'from src.domain.internal import helper\n',
                 'src/domain/internal/helper.py': 'def h():\n    return 1\n'}
        self.build(files, self.private_contract())
        code, payload = self.result()
        self.assertEqual(code, 0, payload)

    def test_negative_fixture_blocks_cross_module_private_import(self):
        files = {'src/app/main.py': 'from src.domain.internal import helper\n',
                 'src/domain/internal/helper.py': 'def h():\n    return 1\n'}
        self.build(files, self.private_contract())
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        hits = self.findings(payload, 'ARCH-PRIVATE', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertEqual(hits[0]['module_id'], 'app')
        self.assertEqual(hits[0]['target_module'], 'domain')
        self.assertEqual(hits[0]['file'], 'src/app/main.py')
        self.assertIn('src/domain/internal', hits[0]['message'])


class CapabilityHonestyTests(CheckerTestBase):
    def test_dynamic_import_is_unknown_not_pass(self):
        files = {
            'src/app/main.py': (
                'import importlib\n\n'
                'def load(name):\n'
                '    return importlib.import_module(name)\n'
            ),
            'src/domain/model.py': 'VALUE = 1\n',
        }
        self.build(files, contract())
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        unknowns = self.findings(payload, status='UNKNOWN')
        self.assertTrue(any(f['file'] == 'src/app/main.py' for f in unknowns),
                        payload['findings'])
        self.assertNotEqual(payload['status'], 'PASS')

    def test_unparsable_python_file_is_unknown(self):
        files = {'src/app/broken.py': 'def broken(:\n', 'src/domain/model.py': 'VALUE = 1\n'}
        self.build(files, contract())
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertTrue(any(f['file'] == 'src/app/broken.py' and f['status'] == 'UNKNOWN'
                            for f in payload['findings']), payload['findings'])

    def test_rust_adapter_reports_not_run_when_toolchain_is_unavailable(self):
        rust_contract = contract(modules=[
            {'module_id': 'rustcore', 'paths': ['crates/core/'],
             'responsibilities': ['rust core crate'],
             'public_interfaces': [{'name': 'core', 'kind': 'python_module'}]},
        ], dependencies={'allowed': [], 'forbidden': []})
        files = {'crates/core/lib.rs': 'use crate::util::Thing;\n',
                 'crates/core/util.rs': 'pub struct Thing;\n'}
        self.build(files, rust_contract)
        code, payload = self.result('--disable-adapter', 'rust')
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload['capabilities']['rust']['status'], 'UNKNOWN')
        not_run = [f for f in payload['findings'] if f['status'] == 'NOT_RUN']
        self.assertTrue(not_run, payload['findings'])
        self.assertTrue(all(f['status'] != 'PASS' for f in payload['findings']))
        self.assertEqual(payload['status'], 'NOT_RUN')

    def test_unimplemented_rule_kind_is_unknown_not_pass(self):
        rule = {'rule_id': 'ARCH-CUSTOM', 'kind': 'CUSTOM', 'severity': 'ERROR',
                'enabled': True, 'description': 'project specific check needs a human'}
        self.build({'src/app/main.py': 'x = 1\n'}, contract(rules=[rule]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        hits = self.findings(payload, 'ARCH-CUSTOM')
        self.assertEqual([f['status'] for f in hits], ['UNKNOWN'])


class SoftMetricTests(CheckerTestBase):
    def test_soft_metrics_only_warn_even_when_declared_error(self):
        rule = {'rule_id': 'ARCH-SIZE', 'kind': 'MODULE_SIZE', 'severity': 'ERROR',
                'enabled': True, 'description': 'module size budget',
                'params': {'max_lines': 3}}
        files = {'src/app/main.py': ''.join(f'V{i} = {i}\n' for i in range(20)),
                 'src/domain/model.py': 'VALUE = 1\n'}
        self.build(files, contract(rules=[rule]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        hits = self.findings(payload, 'ARCH-SIZE')
        self.assertEqual([f['status'] for f in hits], ['WARNING'], payload['findings'])
        self.assertIn('static metric', hits[0]['message'])
        self.assertEqual(payload['counts']['ERROR'], 0)

    def test_fan_out_and_interface_growth_are_warning_only(self):
        rules = [
            {'rule_id': 'ARCH-FANOUT', 'kind': 'MODULE_FAN_OUT', 'severity': 'WARNING',
             'enabled': True, 'description': 'outgoing module fan-out budget',
             'params': {'max': 0}},
            {'rule_id': 'ARCH-GROWTH', 'kind': 'INTERFACE_GROWTH', 'severity': 'ERROR',
             'enabled': True, 'description': 'public interface growth budget',
             'params': {'baseline': {'app': 0}}},
        ]
        files = {'src/app/main.py': 'from src.domain import model\n',
                 'src/domain/model.py': 'VALUE = 1\n'}
        self.build(files, contract(rules=rules))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        for rule_id in ('ARCH-FANOUT', 'ARCH-GROWTH'):
            hits = self.findings(payload, rule_id)
            self.assertTrue(hits, rule_id)
            self.assertTrue(all(f['status'] == 'WARNING' for f in hits), hits)


class WaiverTests(CheckerTestBase):
    def waiver_contract(self, status):
        rule = {
            'rule_id': 'ARCH-FORBID-INFRA',
            'kind': 'FORBIDDEN_EDGE',
            'severity': 'ERROR',
            'enabled': True,
            'description': 'application code must not reach infrastructure directly',
            'waivers': [{
                'waiver_id': 'WVR-INFRA',
                'reason': 'legacy call site is being removed next sprint',
                'owner': 'fixture-owner',
                'expires_at': '2099-01-01',
                'status': status,
                'scope': ['app'],
            }],
        }
        return contract(
            dependencies={'allowed': [], 'forbidden': [
                {'from': 'app', 'to': 'infra', 'reason': 'route through the domain module',
                 'rule_id': 'ARCH-FORBID-INFRA'}]},
            rules=[rule])

    FILES = {'src/app/main.py': 'from src.infra import db\n',
             'src/infra/db.py': 'CONN = None\n'}

    def test_active_waiver_downgrades_to_warning(self):
        self.build(self.FILES, self.waiver_contract('ACTIVE'))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        hits = self.findings(payload, 'ARCH-FORBID-INFRA')
        self.assertEqual([f['status'] for f in hits], ['WARNING'], payload['findings'])
        self.assertEqual(hits[0]['waiver_id'], 'WVR-INFRA')
        self.assertFalse(payload['blocking'])

    def test_expired_waiver_restores_blocking(self):
        self.build(self.FILES, self.waiver_contract('EXPIRED'))
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        hits = self.findings(payload, 'ARCH-FORBID-INFRA')
        self.assertEqual([f['status'] for f in hits], ['ERROR'], payload['findings'])
        self.assertIsNone(hits[0]['waiver_id'])

    def test_active_waiver_with_wildcard_scope_is_rejected(self):
        bad = self.waiver_contract('ACTIVE')
        bad['rules'][0]['waivers'][0]['scope'] = ['*']
        self.build(self.FILES, bad)
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(any('scope' in error for error in payload['contract_errors']),
                        payload['contract_errors'])


class ReportBindingTests(CheckerTestBase):
    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, capture_output=True, text=True)

    def test_report_binds_head_sha_and_contract_version(self):
        self.build({'src/app/main.py': 'from src.domain import model\n',
                    'src/domain/model.py': 'VALUE = 1\n'}, contract())
        self.assertEqual(self.git('init', '-q').returncode, 0)
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.git('config', 'user.name', 'fixture')
        self.assertEqual(self.git('add', '-A').returncode, 0)
        self.assertEqual(self.git('commit', '-q', '-m', 'fixture').returncode, 0)
        expected = self.git('rev-parse', 'HEAD').stdout.strip()
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload['head_sha'], expected)
        self.assertEqual(payload['head_sha_source'], 'git')
        self.assertEqual(payload['contract_version'], '1.0.0')

    def test_head_sha_key_is_present_and_marked_unavailable_outside_git(self):
        self.build({'src/app/main.py': 'x = 1\n'})
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertIn('head_sha', payload)
        self.assertIsNone(payload['head_sha'])
        self.assertEqual(payload['head_sha_source'], 'unavailable')

    def test_explicit_head_sha_override_is_validated(self):
        self.build({'src/app/main.py': 'x = 1\n'})
        code, payload = self.result('--head-sha', 'a' * 40)
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload['head_sha'], 'a' * 40)
        self.assertEqual(payload['head_sha_source'], 'explicit')
        code, payload, proc = self.run_check('--head-sha', 'not-a-sha')
        self.assertEqual(code, 2, (payload, proc.stdout, proc.stderr))
        self.assertIsNone(payload)




class KitIntegrationTests(unittest.TestCase):
    """The architecture artifacts are part of the release manifest.

    `scripts/validate_kit.py:REQUIRED` is simultaneously the release payload, so
    a module that `check.py` imports but REQUIRED omits would ship a checker that
    crashes inside the package (and self-check h would fail). This test pins
    both directions: every file on disk ships, and every intra-package import
    resolves to a file that ships. It deliberately does not care *which* module
    layout is used, only that there is exactly one, complete one.
    """

    @staticmethod
    def manifest():
        path = ROOT / 'scripts' / 'validate_kit.py'
        spec = importlib.util.spec_from_file_location('validate_kit_for_architecture', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module.REQUIRED

    def architecture_files(self):
        shipped = []
        for directory in (ROOT / 'scripts' / 'architecture',
                          ROOT / 'templates' / 'architecture'):
            shipped += [p.relative_to(ROOT).as_posix() for p in directory.rglob('*')
                        if p.is_file() and '__pycache__' not in p.parts]
        shipped += [p.relative_to(ROOT).as_posix()
                    for p in (ROOT / 'tests' / 'architecture').glob('*.py')]
        return sorted(shipped)

    def test_every_architecture_artifact_ships_in_the_release(self):
        required = set(self.manifest())
        missing = [rel for rel in self.architecture_files() if rel not in required]
        self.assertEqual(missing, [],
                         f'architecture artifacts missing from REQUIRED: {missing}')

    def test_every_intra_package_import_resolves_to_a_shipped_file(self):
        base = ROOT / 'scripts' / 'architecture'
        required = set(self.manifest())
        problems = []
        for path in sorted(base.rglob('*.py')):
            tree = ast.parse(path.read_text(encoding='utf-8'))
            for node in ast.walk(tree):
                for candidates in self.import_groups(node):
                    resolved = next((candidate for candidate in candidates
                                     if (base / candidate).is_file()), None)
                    if resolved is None:
                        problems.append(f'{path.relative_to(ROOT)} imports missing module '
                                        f'{candidates[0]}')
                        continue
                    relative = (Path('scripts') / 'architecture' / resolved).as_posix()
                    if relative not in required:
                        problems.append(f'{path.relative_to(ROOT)} imports unshipped module '
                                        f'{relative}')
        self.assertEqual(problems, [], problems)

    @staticmethod
    def import_groups(node):
        """Candidate file groups a single import statement needs, in `architecture`."""
        if isinstance(node, ast.Import):
            groups = []
            for alias in node.names:
                if alias.name == 'architecture' or alias.name.startswith('architecture.'):
                    tail = alias.name[len('architecture'):].lstrip('.').replace('.', '/')
                    groups.append([f'{tail}.py', f'{tail}/__init__.py'] if tail
                                  else ['__init__.py'])
            return groups
        if not isinstance(node, ast.ImportFrom):
            return []
        module = (node.module or '')
        if node.level:  # relative: `from . import x` / `from .adapters import y`
            if not module:
                groups = []
                for alias in node.names:
                    groups.append([f'{alias.name}.py', f'{alias.name}/__init__.py'])
                return groups
            tail = module.replace('.', '/')
            return [[f'{tail}.py', f'{tail}/__init__.py']]
        if module == 'architecture':
            return [[f'{alias.name}.py', f'{alias.name}/__init__.py'] for alias in node.names]
        if module.startswith('architecture.'):
            tail = module[len('architecture.'):].replace('.', '/')
            return [[f'{tail}.py', f'{tail}/__init__.py']]
        return []


# --- architecture alignment fixtures derived from AgentMgr lessons (#56) ----- #
# The frozen contract already has the rule kinds these fixtures use; #56 adds no
# new kind and no new param. What it does add is the shape a reviewer can point
# at: a core module reaching into an adapter's *private* runtime path, a
# transport entry point that bypasses the shared application service, a
# declaration that must not become coverage by itself, and an unconfigured
# project that stays diagnostic-only. See docs/architecture/agentmgr-contract-rules.md.

AGM_MODULES = [
    {
        'module_id': 'domain',
        'paths': ['src/domain/'],
        'responsibilities': ['domain decisions and owned state'],
        'public_interfaces': [{'name': 'src.domain.rules', 'kind': 'python_module',
                               'stability': 'stable'}],
    },
    {
        'module_id': 'application',
        'paths': ['src/app/'],
        'responsibilities': ['single application service shared by every transport'],
        'public_interfaces': [{'name': 'src.app.service', 'kind': 'python_module',
                               'stability': 'stable'}],
    },
    {
        'module_id': 'adapter-hermes',
        'paths': ['src/adapters/hermes/'],
        'responsibilities': ['runtime adapter that hides its framework internals'],
        'public_interfaces': [{'name': 'src.adapters.hermes.api', 'kind': 'python_module',
                               'stability': 'additive_only'}],
        'private_paths': ['src/adapters/hermes/runtime/'],
    },
    {
        'module_id': 'entrypoint-api',
        'paths': ['src/entrypoints/api/'],
        'responsibilities': ['HTTP transport: protocol translation only'],
        'public_interfaces': [{'name': 'plan:http', 'kind': 'http_api', 'stability': 'stable'}],
    },
    {
        'module_id': 'entrypoint-mcp',
        'paths': ['src/entrypoints/mcp/'],
        'responsibilities': ['event transport: protocol translation only'],
        'public_interfaces': [{'name': 'plan:event', 'kind': 'event', 'stability': 'stable'}],
    },
]

AGM_RULES = [
    {
        'rule_id': 'ARCH-AGM-NO-CYCLE',
        'kind': 'NO_CYCLE',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'modules must not form dependency cycles',
    },
    {
        'rule_id': 'ARCH-AGM-ENTRY-SHARED',
        'kind': 'FORBIDDEN_EDGE',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'transport entry points call the shared application service, '
                       'never the domain or a concrete adapter',
    },
    {
        'rule_id': 'ARCH-AGM-ADAPTER-PRIVATE',
        'kind': 'PRIVATE_ACCESS',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'core modules never import an adapter private runtime path',
    },
]

AGM_BOUNDARIES = [
    {'boundary_id': 'BND-DOMAIN', 'kind': 'domain', 'members': ['domain']},
    {'boundary_id': 'BND-APP', 'kind': 'application', 'members': ['application']},
    {'boundary_id': 'BND-ADAPTER', 'kind': 'adapter', 'members': ['adapter-hermes']},
    {'boundary_id': 'BND-TRANSPORT', 'kind': 'layer',
     'members': ['entrypoint-api', 'entrypoint-mcp']},
]

AGM_INVARIANTS = [
    {
        'invariant_id': 'INV-AGM-ENTRY-EDGE',
        'statement': 'no transport entry point reaches the domain or a concrete adapter '
                     'directly; every request goes through the application service',
        'checked_by': 'STATIC_RULE',
        'severity': 'ERROR',
        'rule_ids': ['ARCH-AGM-ENTRY-SHARED'],
    },
    {
        'invariant_id': 'INV-AGM-ADAPTER-ISOLATION',
        'statement': 'the core modules never import an adapter private runtime path',
        'checked_by': 'STATIC_RULE',
        'severity': 'ERROR',
        'rule_ids': ['ARCH-AGM-ADAPTER-PRIVATE'],
    },
    {
        'invariant_id': 'INV-AGM-SHARED-DECISION',
        'statement': 'both transports return the same decision for the same request',
        'checked_by': 'CONTRACT_TEST',
        'severity': 'ERROR',
        'notes': 'the checker cannot run the project test suite; the shared contract test '
                 'result has to be attached as its own evidence',
    },
]

# The clean tree: both transports call the application service, the adapter keeps
# its framework session private, and nothing imports that private path.
AGM_CLEAN_FILES = {
    'src/domain/rules.py':
        'def decide(request):\n'
        '    return {"amount": request["amount"] * 2}\n',
    'src/app/service.py':
        'from src.domain import rules\n'
        '\n'
        '\n'
        'def plan(request):\n'
        '    return rules.decide(request)\n',
    'src/adapters/hermes/api.py':
        'from src.domain import rules\n'
        '\n'
        '\n'
        'class HermesAdapter:\n'
        '    def decide(self, request):\n'
        '        return rules.decide(request)\n',
    'src/adapters/hermes/runtime/session.py':
        'FRAMEWORK_SESSION = object()\n',
    'src/entrypoints/api/handler.py':
        'from src.app import service\n'
        '\n'
        '\n'
        'def handle(request):\n'
        '    return service.plan(request)\n',
    'src/entrypoints/mcp/handler.py':
        'from src.app import service\n'
        '\n'
        '\n'
        'def handle(request):\n'
        '    return service.plan(request)\n',
}


def agm_contract(**overrides):
    """The alignment fixture contract; keyword arguments replace top-level keys."""
    data = {
        'schema_version': '1.0',
        'contract_version': '1.0.0',
        'project_name': 'agentmgr-alignment-fixture',
        'enforcement_mode': 'ENFORCE',
        'modules': AGM_MODULES,
        'dependencies': {
            'allowed': [
                {'from': 'entrypoint-api', 'to': 'application',
                 'reason': 'the HTTP transport calls the shared application service'},
                {'from': 'entrypoint-mcp', 'to': 'application',
                 'reason': 'the event transport calls the shared application service'},
                {'from': 'application', 'to': 'domain',
                 'reason': 'the application service orchestrates domain rules'},
                {'from': 'adapter-hermes', 'to': 'domain',
                 'reason': 'the adapter implements the domain port'},
            ],
            'forbidden': [
                {'from': 'entrypoint-api', 'to': 'domain',
                 'reason': 'an entry point must not re-implement a business rule',
                 'rule_id': 'ARCH-AGM-ENTRY-SHARED'},
                {'from': 'entrypoint-mcp', 'to': 'domain',
                 'reason': 'an entry point must not re-implement a business rule',
                 'rule_id': 'ARCH-AGM-ENTRY-SHARED'},
                {'from': 'entrypoint-api', 'to': 'adapter-hermes',
                 'reason': 'an entry point must not bind a concrete adapter',
                 'rule_id': 'ARCH-AGM-ENTRY-SHARED'},
                {'from': 'entrypoint-mcp', 'to': 'adapter-hermes',
                 'reason': 'an entry point must not bind a concrete adapter',
                 'rule_id': 'ARCH-AGM-ENTRY-SHARED'},
            ],
        },
        'boundaries': AGM_BOUNDARIES,
        'invariants': AGM_INVARIANTS,
        'rules': AGM_RULES,
        'baseline': {'frozen_at': '2026-01-01T00:00:00Z', 'frozen_by': 'fixture'},
    }
    data.update(overrides)
    return data


class AgentMgrAlignmentTests(CheckerTestBase):
    """Positive control plus the dependency shapes the six rules describe."""

    def coverage(self, payload, invariant_id):
        entries = [entry for entry in payload.get('invariants') or []
                   if entry.get('invariant_id') == invariant_id]
        self.assertEqual(len(entries), 1, payload.get('invariants'))
        return entries[0]

    def test_positive_control_is_clean_and_reports_its_scope(self):
        self.build(dict(AGM_CLEAN_FILES), agm_contract())
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(payload['status'], 'PASS', payload['findings'])
        self.assertEqual(payload['counts']['ERROR'], 0)
        # Scope travels with the pass: every declared module was examined.
        self.assertEqual(sorted(payload['examined_modules']),
                         ['adapter-hermes', 'application', 'domain',
                          'entrypoint-api', 'entrypoint-mcp'])
        self.assertEqual(payload['uncovered_modules'], [])
        self.assertEqual(self.coverage(payload, 'INV-AGM-ENTRY-EDGE')['status'], 'PASS')
        self.assertEqual(self.coverage(payload, 'INV-AGM-ADAPTER-ISOLATION')['status'], 'PASS')
        # A declared contract test is NOT covered by this checker.
        self.assertEqual(self.coverage(payload, 'INV-AGM-SHARED-DECISION')['status'], 'UNKNOWN')

    def test_core_importing_an_adapter_private_symbol_blocks(self):
        files = dict(AGM_CLEAN_FILES)
        files['src/app/service.py'] = (
            'from src.adapters.hermes.runtime import session\n'
            'from src.domain import rules\n'
            '\n'
            '\n'
            'def plan(request):\n'
            '    return rules.decide(request)\n')
        self.build(files, agm_contract())
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        self.assertTrue(payload['blocking'])
        hits = self.findings(payload, 'ARCH-AGM-ADAPTER-PRIVATE', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertEqual(hits[0]['module_id'], 'application')
        self.assertEqual(hits[0]['target_module'], 'adapter-hermes')
        self.assertEqual(hits[0]['file'], 'src/app/service.py')
        self.assertIn('src/adapters/hermes/runtime', hits[0]['message'])
        self.assertEqual(self.coverage(payload, 'INV-AGM-ADAPTER-ISOLATION')['status'], 'ERROR')

    def test_entry_point_bypassing_the_shared_service_blocks(self):
        files = dict(AGM_CLEAN_FILES)
        files['src/entrypoints/mcp/handler.py'] = (
            'from src.domain import rules\n'
            '\n'
            '\n'
            'def handle(request):\n'
            '    return rules.decide(request)\n')
        self.build(files, agm_contract())
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        hits = self.findings(payload, 'ARCH-AGM-ENTRY-SHARED', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertEqual(hits[0]['module_id'], 'entrypoint-mcp')
        self.assertEqual(hits[0]['target_module'], 'domain')
        self.assertEqual(hits[0]['file'], 'src/entrypoints/mcp/handler.py')
        self.assertEqual(self.coverage(payload, 'INV-AGM-ENTRY-EDGE')['status'], 'ERROR')

    def test_entry_point_binding_a_concrete_adapter_blocks(self):
        files = dict(AGM_CLEAN_FILES)
        files['src/entrypoints/api/handler.py'] = (
            'from src.adapters.hermes import api\n'
            '\n'
            '\n'
            'def handle(request):\n'
            '    return api.HermesAdapter().decide(request)\n')
        self.build(files, agm_contract())
        code, payload = self.result()
        self.assertEqual(code, 1, payload)
        hits = self.findings(payload, 'ARCH-AGM-ENTRY-SHARED', 'ERROR')
        self.assertEqual(len(hits), 1, payload['findings'])
        self.assertEqual(hits[0]['target_module'], 'adapter-hermes')

    def test_project_without_a_contract_is_never_judged_by_inferred_rules(self):
        """AC-4: the same violations, no contract: diagnostic only, nothing blocked."""
        files = dict(AGM_CLEAN_FILES)
        files['src/app/service.py'] = (
            'from src.adapters.hermes.runtime import session\n'
            '\n'
            '\n'
            'def plan(request):\n'
            '    return session\n')
        files['src/entrypoints/mcp/handler.py'] = (
            'import src.domain.rules\n'
            '\n'
            '\n'
            'def handle(request):\n'
            '    return request\n')
        self.build(files)
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertIsNone(payload['contract_path'])
        self.assertEqual(payload['enforcement_mode'], 'OFF')
        self.assertTrue(payload['diagnostic_only'])
        self.assertFalse(payload['blocking'])
        self.assertEqual(payload['counts']['ERROR'], 0)
        self.assertEqual(payload['status'], 'NOT_RUN')
        self.assertEqual(payload['invariants'], [])
        self.assertEqual(payload['examined_modules'], [])
        self.assertFalse([finding for finding in payload['findings']
                          if finding['status'] in ('ERROR', 'WARNING')], payload['findings'])
        self.assertEqual(payload['inventory']['python_files'], len(files))


class InvariantCoverageTests(CheckerTestBase):
    """A declared invariant is not coverage; only an executed rule result is."""

    PASSING_RULE = {
        'rule_id': 'ARCH-AGM-ENTRY-SHARED',
        'kind': 'FORBIDDEN_EDGE',
        'severity': 'ERROR',
        'enabled': True,
        'description': 'transport entry points call the shared application service',
    }

    def contract_with(self, invariants, rules=None):
        return agm_contract(
            invariants=invariants,
            rules=rules if rules is not None else [self.PASSING_RULE,
                                                   AGM_RULES[0], AGM_RULES[2]])

    def invariant(self, invariant_id, checked_by, rule_ids=None, statement=None):
        entry = {
            'invariant_id': invariant_id,
            'statement': statement or 'the declared property still holds across the modules',
            'checked_by': checked_by,
            'severity': 'ERROR',
        }
        if rule_ids is not None:
            entry['rule_ids'] = rule_ids
        return entry

    def coverage(self, payload, invariant_id):
        entries = [entry for entry in payload.get('invariants') or []
                   if entry.get('invariant_id') == invariant_id]
        self.assertEqual(len(entries), 1, payload.get('invariants'))
        return entries[0]

    def test_static_rule_invariant_reports_the_rule_status(self):
        self.build(dict(AGM_CLEAN_FILES), self.contract_with([
            self.invariant('INV-AGM-STATIC', 'STATIC_RULE', ['ARCH-AGM-ENTRY-SHARED'])]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        entry = self.coverage(payload, 'INV-AGM-STATIC')
        self.assertEqual(entry['status'], 'PASS', entry)
        self.assertIn('ARCH-AGM-ENTRY-SHARED=PASS', entry['message'])

    def test_static_rule_invariant_over_an_unimplemented_rule_is_unknown(self):
        custom = {'rule_id': 'ARCH-AGM-CUSTOM', 'kind': 'CUSTOM', 'severity': 'ERROR',
                  'enabled': True, 'description': 'project specific check needs a human'}
        self.build(dict(AGM_CLEAN_FILES), self.contract_with(
            [self.invariant('INV-AGM-CUSTOM', 'STATIC_RULE', ['ARCH-AGM-CUSTOM'])],
            rules=[custom, self.PASSING_RULE, AGM_RULES[0], AGM_RULES[2]]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        entry = self.coverage(payload, 'INV-AGM-CUSTOM')
        self.assertEqual(entry['status'], 'UNKNOWN', entry)
        self.assertIn('ARCH-AGM-CUSTOM=UNKNOWN', entry['message'])

    def test_static_rule_invariant_over_a_disabled_rule_is_not_run(self):
        disabled = dict(self.PASSING_RULE, enabled=False)
        self.build(dict(AGM_CLEAN_FILES), self.contract_with(
            [self.invariant('INV-AGM-DISABLED', 'STATIC_RULE', ['ARCH-AGM-ENTRY-SHARED'])],
            rules=[disabled]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(self.coverage(payload, 'INV-AGM-DISABLED')['status'], 'NOT_RUN')

    def test_static_rule_invariant_without_a_rule_id_is_unknown(self):
        self.build(dict(AGM_CLEAN_FILES), self.contract_with([
            self.invariant('INV-AGM-UNBOUND', 'STATIC_RULE')]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        entry = self.coverage(payload, 'INV-AGM-UNBOUND')
        self.assertEqual(entry['status'], 'UNKNOWN', entry)
        self.assertIn('rule_ids', entry['message'])

    def test_contract_test_and_manual_review_invariants_stay_unknown(self):
        self.build(dict(AGM_CLEAN_FILES), self.contract_with([
            self.invariant('INV-AGM-CONTRACT', 'CONTRACT_TEST'),
            self.invariant('INV-AGM-REVIEW', 'MANUAL_REVIEW'),
            self.invariant('INV-AGM-DECLARED', 'UNKNOWN'),
        ]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(self.coverage(payload, 'INV-AGM-CONTRACT')['status'], 'UNKNOWN')
        self.assertIn('contract test', self.coverage(payload, 'INV-AGM-CONTRACT')['message'])
        self.assertEqual(self.coverage(payload, 'INV-AGM-REVIEW')['status'], 'UNKNOWN')
        self.assertIn('reviewer', self.coverage(payload, 'INV-AGM-REVIEW')['message'].lower())
        self.assertEqual(self.coverage(payload, 'INV-AGM-DECLARED')['status'], 'UNKNOWN')

    def test_incomplete_scan_cannot_cover_a_static_invariant(self):
        files = dict(AGM_CLEAN_FILES)
        files['src/app/loader.py'] = (
            'import importlib\n'
            '\n'
            '\n'
            'def load(name):\n'
            '    return importlib.import_module(name)\n')
        self.build(files, self.contract_with([
            self.invariant('INV-AGM-STATIC', 'STATIC_RULE', ['ARCH-AGM-ENTRY-SHARED'])]))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertTrue(self.findings(payload, status='UNKNOWN'), payload['findings'])
        entry = self.coverage(payload, 'INV-AGM-STATIC')
        self.assertEqual(entry['status'], 'UNKNOWN', entry)
        self.assertIn('scan is incomplete', entry['message'])

    def test_coverage_is_additive_and_changes_no_finding_or_blocking(self):
        violating = dict(AGM_CLEAN_FILES)
        violating['src/app/service.py'] = (
            'from src.adapters.hermes.runtime import session\n'
            'from src.domain import rules\n'
            '\n'
            '\n'
            'def plan(request):\n'
            '    return rules.decide(request)\n')
        self.build(dict(violating), agm_contract(invariants=[]))
        code_without, without = self.result()
        self.build(dict(violating), agm_contract())
        code_with, with_invariants = self.result()
        self.assertEqual((code_without, code_with), (1, 1))
        self.assertEqual(with_invariants['counts'], without['counts'])
        self.assertEqual(with_invariants['blocking'], without['blocking'])
        self.assertEqual(with_invariants['status'], without['status'])
        self.assertEqual(len(with_invariants['findings']), len(without['findings']))
        self.assertTrue(with_invariants['invariants'])

    def test_off_mode_reports_not_run_coverage(self):
        self.build(dict(AGM_CLEAN_FILES), agm_contract(enforcement_mode='OFF'))
        code, payload = self.result()
        self.assertEqual(code, 0, payload)
        self.assertEqual(self.coverage(payload, 'INV-AGM-ENTRY-EDGE')['status'], 'NOT_RUN')
        self.assertEqual(self.coverage(payload, 'INV-AGM-SHARED-DECISION')['status'], 'UNKNOWN')


if __name__ == '__main__':
    unittest.main()
