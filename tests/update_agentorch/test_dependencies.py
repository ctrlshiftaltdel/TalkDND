"""Dependency and scope guard for the update engine / entry (AC-6).

The update **engine** must stay pure standard library and must never touch the
existing scheduler / task state machine. Since #52 the package also carries
``entry.py``: it is a router that must call ``scripts/self_improvement/planning.py``
(the single trigger table), so it is exempt from the engine-purity scan while a
separate check keeps it away from the scheduler.
"""
from __future__ import annotations

import ast
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / 'scripts' / 'update_agentorch'
ENTRY = PACKAGE / 'entry.py'
ENGINE_MODULES = sorted(path for path in PACKAGE.glob('*.py') if path.name != 'entry.py')


def imported_roots(path):
    tree = ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
    roots = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            roots.extend(alias.name.split('.')[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            if node.level:
                continue  # relative import inside the package
            if node.module:
                roots.append(node.module.split('.')[0])
    return roots


def imports_scheduler(path):
    tree = ast.parse(path.read_text(encoding='utf-8'), filename=str(path))
    for node in ast.walk(tree):
        names = []
        if isinstance(node, ast.Import):
            names = [alias.name for alias in node.names]
        elif isinstance(node, ast.ImportFrom) and node.module:
            names = [node.module]
        if any('orchestrator' in name for name in names):
            return True
    return False


class DependencyTests(unittest.TestCase):
    def test_package_exists_and_has_sources(self):
        self.assertTrue(PACKAGE.is_dir())
        self.assertTrue(list(PACKAGE.glob('*.py')))
        self.assertTrue(ENTRY.is_file(), 'the #52 entry must ship in the package')

    def test_engine_modules_import_only_standard_library_and_own_package(self):
        allowed = set(sys.stdlib_module_names) | {'update_agentorch'}
        violations = []
        for path in ENGINE_MODULES:
            for module in imported_roots(path):
                if module not in allowed:
                    violations.append(f'{path.name}: {module}')
        self.assertEqual(violations, [])

    def test_entry_adds_only_the_shared_routing_module(self):
        allowed = (set(sys.stdlib_module_names)
                   | {'update_agentorch', 'scripts', 'self_improvement'})
        violations = sorted({module for module in imported_roots(ENTRY)
                             if module not in allowed})
        self.assertEqual(violations, [])

    def test_engine_modules_do_not_reference_the_task_state_machine(self):
        for path in ENGINE_MODULES:
            text = path.read_text(encoding='utf-8')
            self.assertNotIn('orchestrator.py', text, path.name)

    def test_entry_does_not_import_the_task_state_machine(self):
        # The entry prints task-registration commands for the local carrier, but
        # it must never import or drive the scheduler itself.
        self.assertFalse(imports_scheduler(ENTRY))

    def test_no_third_party_requirement_files_are_added(self):
        for name in ('requirements.txt', 'pyproject.toml', 'setup.py', 'Pipfile'):
            self.assertFalse((PACKAGE / name).exists(), name)


if __name__ == '__main__':
    unittest.main()
