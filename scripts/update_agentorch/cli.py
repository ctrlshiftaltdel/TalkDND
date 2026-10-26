"""Command line entry point: ``PLAN``/``APPLY`` one update and print the report.

There is deliberately no flag for a local zip, a URL or another repository: the
frozen contract only accepts an official GitHub Release, so a test or a human
cannot widen the trust boundary through the CLI. Tests drive the engine with an
injected in-memory source instead.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

from . import engine, sources
from . import report as report_module

EDITION_RE = re.compile(r'`?edition`?\s*[:：]\s*`?([^`\s]+)`?', re.IGNORECASE)
VALID_EDITIONS = ('local', 'github')


def read_edition(project: Path) -> str:
    """Read ``edition`` from docs/PROJECT.md; fail closed instead of guessing."""
    config = Path(project) / 'docs' / 'PROJECT.md'
    try:
        text = config.read_text(encoding='utf-8')
    except OSError as exc:
        raise ValueError(f'cannot read {config}: {exc}') from exc
    match = EDITION_RE.search(text)
    if not match:
        raise ValueError(f'{config} does not declare an edition (expected local or github)')
    value = match.group(1).strip().strip('`').strip()
    if value not in VALID_EDITIONS:
        raise ValueError(f'{config}: edition {value!r} is not one of {list(VALID_EDITIONS)}')
    return value


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog='update_agentorch',
        description='Verify an official AgentOrch release and plan/apply a safe update.',
    )
    parser.add_argument('--project', default='.', help='installed project root (default: .)')
    parser.add_argument('--mode', choices=('PLAN', 'APPLY'), default='PLAN',
                        help='PLAN only classifies; APPLY writes the conflict-free change set')
    parser.add_argument('--edition', choices=VALID_EDITIONS,
                        help='edition recorded in the report (default: read from docs/PROJECT.md)')
    parser.add_argument('--output', help='also write the JSON report to this path')
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    project = Path(args.project)
    try:
        edition = args.edition or read_edition(project)
    except ValueError as exc:
        print(f'update_agentorch: configuration error: {exc}', file=sys.stderr)
        return 1

    source = sources.GitHubReleaseSource()
    try:
        result = engine.run_update(project, mode=args.mode, source=source, edition=edition)
    except Exception as exc:  # noqa: BLE001 - the CLI must report, not traceback
        print(f'update_agentorch: no report produced: {type(exc).__name__}: {exc}', file=sys.stderr)
        return 1

    text = json.dumps(result, indent=2, ensure_ascii=False) + '\n'
    if args.output:
        try:
            Path(args.output).write_text(text, encoding='utf-8')
        except OSError as exc:
            print(f'update_agentorch: cannot write report to {args.output}: {exc}', file=sys.stderr)
            return 1
    sys.stdout.write(text)

    counts = result.get('classification', {}).get('counts', {})
    print(f"update_agentorch: {result.get('result')} "
          f"(UPSTREAM_UNCHANGED={counts.get('UPSTREAM_UNCHANGED', 0)}, "
          f"conflicts={counts.get('THREE_WAY_CONFLICT', 0)})", file=sys.stderr)

    problems = report_module.validate_report(result)
    if problems:
        for problem in problems:
            print(f'update_agentorch: report contract violation: {problem}', file=sys.stderr)
        return 5
    return engine.EXIT_CODES.get(result.get('result'), 1)


if __name__ == '__main__':
    raise SystemExit(main())
