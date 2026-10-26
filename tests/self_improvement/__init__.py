"""Allow ``python3 -m unittest tests.self_improvement`` to run this package.

``unittest`` does not discover submodules when given a package name, so the
package registers its own test modules explicitly. ``python3 -m unittest discover
-s tests`` picks the same modules up through ``test_*.py`` naming.
"""
import importlib

_TEST_MODULES = ('test_registry', 'test_lesson_lifecycle', 'test_collection', 'test_pipeline',
                 'test_routing', 'test_planning', 'test_orchestration',
                 'test_acceptance_report')


def load_tests(loader, standard_tests, pattern):
    for name in _TEST_MODULES:
        module = importlib.import_module(f'{__name__}.{name}')
        standard_tests.addTests(loader.loadTestsFromModule(module))
    return standard_tests
