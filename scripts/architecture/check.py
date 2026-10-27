#!/usr/bin/env python3
"""Architecture Contract checker (optional, standard library only).

Usage:
    python3 scripts/architecture/check.py [--project DIR] [--contract PATH]
                                          [--json] [--head-sha SHA]
                                          [--disable-adapter python|rust]

Exit codes:
    0  no blocking finding (including "no contract configured": diagnostic only)
    1  blocking finding, or a contract that cannot be read/validated under ENFORCE
    2  usage error (bad path, malformed --head-sha)

The checker never guesses a module layout. Without a contract it prints a
read-only inventory and exits 0; a contract that is *declared* but unreadable is
a config error and fails closed. See docs/architecture/architecture-contract.md
(frozen by #47) for the semantics and docs/self-improvement/architecture-checker.md
for this implementation's exact capability boundaries.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

if __package__ in (None, ''):  # executed as a script: make `architecture` importable
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from architecture import contract as contract_module  # noqa: E402
    from architecture import report as report_module  # noqa: E402
    from architecture import rules  # noqa: E402
else:  # imported as a package
    from . import contract as contract_module  # noqa: E402
    from . import report as report_module  # noqa: E402
    from . import rules  # noqa: E402

SHA_RE = re.compile(r'^[0-9a-f]{40}$')
ADAPTERS = ('python', 'rust')


def git_head(project_root):
    """HEAD SHA of the checked tree, or None when it is not a git checkout."""
    try:
        proc = subprocess.run(['git', '-C', str(project_root), 'rev-parse', 'HEAD'],
                              capture_output=True, text=True)
    except OSError:
        return None
    if proc.returncode != 0:
        return None
    value = proc.stdout.strip().lower()
    return value if SHA_RE.match(value) else None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--project', default='.', help='project root to check (default: .)')
    parser.add_argument('--contract', help='explicit contract path, overriding docs/PROJECT.md')
    parser.add_argument('--json', action='store_true', help='emit one JSON report on stdout')
    parser.add_argument('--head-sha', help='bind the report to this 40-hex SHA instead of git HEAD')
    parser.add_argument('--disable-adapter', action='append', choices=ADAPTERS, default=[],
                        help='disable a language adapter (repeatable); its rules become NOT_RUN')
    args = parser.parse_args(argv)

    project_root = Path(args.project)
    if not project_root.is_dir():
        print(f'error: --project is not a directory: {args.project}', file=sys.stderr)
        return 2
    if args.head_sha is not None and not SHA_RE.match(args.head_sha.lower()):
        print('error: --head-sha must be a 40-character hex commit SHA', file=sys.stderr)
        return 2

    if args.head_sha is not None:
        head_sha, head_sha_source = args.head_sha.lower(), 'explicit'
    else:
        head_sha = git_head(project_root)
        head_sha_source = 'git' if head_sha else 'unavailable'

    adapters = tuple(name for name in ADAPTERS if name not in args.disable_adapter)
    loaded = contract_module.load(project_root, args.contract)
    executed = None
    if loaded['contract'] is not None:
        executed = rules.run(loaded['contract'], project_root, adapters_enabled=adapters)

    report = report_module.build_report(
        project_root, loaded, executed, head_sha, head_sha_source, adapters)

    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=False, sort_keys=False))
    else:
        print(report_module.render_text(report))
    return 1 if report['blocking'] else 0


if __name__ == '__main__':
    sys.exit(main())
