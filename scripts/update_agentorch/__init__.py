"""Safe update engine for an installed AgentOrch project (Issue #51).

Public API::

    from update_agentorch import run_update
    report = run_update(project, mode='PLAN', source=GitHubReleaseSource(), edition='github')

Command line (from the repository root)::

    python3 -m scripts.update_agentorch --project <dir> --mode PLAN

or with ``scripts/`` on ``PYTHONPATH``::

    python3 -m update_agentorch --project <dir> --mode PLAN

Only the Python standard library is used. The engine never merges, never writes
the main branch on its own and never claims that an applied update has been
reviewed.
"""
from .engine import EXIT_CODES, run_update
from .sources import OFFICIAL_REPOSITORY, GitHubReleaseSource

__all__ = ['EXIT_CODES', 'OFFICIAL_REPOSITORY', 'GitHubReleaseSource', 'run_update']
