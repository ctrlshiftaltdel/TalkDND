"""Synthetic tests for the safe update engine (Issue #51).

Everything here is built at runtime from synthetic bytes: no real GitHub
Release, credential, network call or production project is involved. The
fixtures are declarative (dict of relative path -> bytes) so the same shapes can
be reused by #53's regression scenarios.

``python3 -m unittest tests.update_agentorch`` runs this whole package; the
repository-wide ``unittest discover -s tests`` finds the modules directly.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

_ROOT = Path(__file__).resolve().parents[2]
_SCRIPTS = _ROOT / 'scripts'
if str(_SCRIPTS) not in sys.path:  # `python -m unittest tests.update_agentorch...`
    sys.path.insert(0, str(_SCRIPTS))


def load_tests(loader, tests, pattern):
    return loader.discover(start_dir=str(Path(__file__).resolve().parent),
                           pattern=pattern or 'test_*.py',
                           top_level_dir=str(_ROOT))
