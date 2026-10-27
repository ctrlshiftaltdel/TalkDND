"""Natural-language entry for "更新 AgentOrch" (Issue #52).

The entry is a thin *router and carrier adapter*:

1. route the user message with :func:`scripts.self_improvement.planning.route_request`
   -- the same deterministic table the self-improvement loop uses, so there is no
   second trigger vocabulary;
2. inspect the project (edition, installed Format / version, baseline manifest,
   workspace state) before any network or write;
3. call the #51 engine (``engine.run_update``) unchanged -- the entry never
   re-classifies files, relaxes verification or decides a rollback;
4. shape the *delivery* for the current edition:
   ``local`` prepares a task-database / branch delivery packet, ``github``
   prepares an Issue / Draft PR delivery packet. The entry itself writes neither
   carrier: no SQLite mutation and no ``gh`` call. Both shapes carry the same
   gate record (review required, no self-approval, no merge).

Command line::

    python3 -m scripts.update_agentorch.entry --project <dir> \
        --request "更新 AgentOrch" --mode PLAN

Exit codes add ``6`` ("the message did not start an update") to the engine's
codes in :data:`scripts.update_agentorch.engine.EXIT_CODES`. Only the Python
standard library is used.
"""
from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

from . import classify, cli, engine, report as report_module, sources, workspace

try:  # repository checkout / package root on sys.path
    from scripts.self_improvement import planning
except ImportError:  # ``scripts/`` itself on sys.path
    from self_improvement import planning  # type: ignore[no-redef]

EXIT_NOT_UPDATE = 6

TRIGGER_EXAMPLES = (
    '更新 AgentOrch',
    '升级 AgentOrch',
    '把项目中的 AgentOrch 更新到最新版',
)
NON_TRIGGER_EXAMPLES = (
    '更新一下文档',
    '更新依赖',
    '更新这个 Issue',
    'AgentOrch 最新版本是多少',
)

UPDATE_CLARIFICATION = ('你是想更新 AgentOrch 套件本身，还是更新项目内容'
                        '（文档 / 依赖 / Issue）？')
SELF_IMPROVEMENT_MESSAGE = ('这条消息属于“进行自我完善”闭环，不属于更新引擎；'
                            '请按自我完善流程处理，不能在这里启动套件更新。')


class EntryError(ValueError):
    """Invalid project or request; the entry reports it instead of guessing."""


# --------------------------------------------------------------------------- #
# Routing
# --------------------------------------------------------------------------- #

def route(text):
    """Return the routing decision as a dict with the update-specific flags.

    ``starts_update`` is the only green light for this entry. It is deliberately
    narrower than ``route == UPDATE_AGENTORCH``: an ambiguous wording
    ("更新 AgentOrch 的文档") asks once instead of starting an update.
    """
    decision = planning.route_request(text)
    payload = decision.as_dict()
    payload['starts_update'] = bool(
        decision.route == planning.ROUTE_UPDATE_AGENTORCH
        and decision.execution_intent
        and not decision.needs_clarification)
    payload['starts_self_improvement'] = bool(decision.starts_workflow)
    if payload['starts_update']:
        payload['clarification_question'] = None
    elif decision.route == planning.ROUTE_UPDATE_AGENTORCH and decision.needs_clarification:
        payload['clarification_question'] = UPDATE_CLARIFICATION
    else:
        payload['clarification_question'] = None
    return payload


# --------------------------------------------------------------------------- #
# Environment inspection (read-only, before any network call)
# --------------------------------------------------------------------------- #

def inspect_environment(project):
    """Read edition, installed Format / version, baseline and workspace state."""
    project = Path(project)
    if not project.is_dir():
        raise EntryError(f'project root does not exist or is not a directory: {project}')

    edition = None
    edition_error = None
    try:
        edition = cli.read_edition(project)
    except ValueError as exc:
        edition_error = f'edition could not be read: {exc}'

    baseline = engine._read_baseline(project)  # same reader the engine uses
    git = workspace.inspect_git(project, datetime.now(timezone.utc))

    return {
        'project': str(project.resolve()),
        'edition': edition,
        'edition_error': edition_error,
        'suite': baseline.suite if baseline.status == 'VALID' else None,
        'installed_version': (baseline.installed_version
                              if baseline.status == 'VALID' else None),
        'format_version': (baseline.format_version
                           if baseline.status == 'VALID' else None),
        'baseline_status': baseline.status,
        'baseline_notes': baseline.notes,
        'workspace': {
            'is_git_repo': git.is_repo,
            'head_sha': git.head_sha,
            'branch': git.branch,
            'dirty': git.dirty,
            'dirty_paths': list(git.dirty_paths),
            'isolated': git.isolated,
        },
    }


# --------------------------------------------------------------------------- #
# Delivery shape (edition-specific) and shared gates
# --------------------------------------------------------------------------- #

def _applied_and_kept(report):
    applied, kept = [], []
    for item in report.get('classification', {}).get('files', []):
        if item.get('decision') == 'APPLIED':
            applied.append(item['path'])
        elif item.get('class') in classify.CONFLICT_CLASSES:
            kept.append(item['path'])
    return sorted(applied), sorted(kept)


def _suggested_branch(report, edition):
    worktree = (report.get('operation') or {}).get('worktree')
    if worktree:
        name = Path(worktree).name
        if name:
            return f'task/update-agentorch-{name}'
    target = (report.get('target') or {}).get('version') or 'latest'
    return f'task/update-agentorch-{target.replace(".", "-")}-{edition}'


def gates(environment, report, mode):
    """The same gate record for both editions; only the carrier differs."""
    problems = report_module.validate_report(report)
    return {
        'edition': environment['edition'],
        'mode': mode,
        'engine_result': report.get('result'),
        'requires_review': True,
        'self_approval_allowed': False,
        'auto_merge': False,
        'conflicts': sorted(
            item['path'] for item in report.get('classification', {}).get('files', [])
            if item.get('class') in classify.CONFLICT_CLASSES),
        'human_decisions': [entry.get('decision_id')
                            for entry in report.get('human_decisions') or []],
        'evidence_head_sha': report.get('head_sha_after'),
        'report_valid': not problems,
        'report_contract_errors': problems,
    }


def _task_packet(environment, report, applied, kept, branch):
    target = (report.get('target') or {}).get('version')
    installed = environment.get('format_version') or '(unknown)'
    return {
        'task id': '(assigned by scripts/orchestrator.py at dispatch)',
        'title': f'Apply the verified AgentOrch update {installed} -> {target}',
        'goal / user-visible result': (
            'The project runs the verified upstream AgentOrch files from the official '
            'release while every project-owned file keeps its exact bytes.'),
        'context': (
            'Entry routed "更新 AgentOrch" to the #51 safe update engine. The engine '
            'verified the official release and classified every path; this packet only '
            'carries the conflict-free change set for review.'),
        'allowed paths': applied,
        'forbidden paths': [
            'every path not listed under allowed paths',
            'project-owned paths (docs/, .github/, .local/, .orchestration/, src/, release/)',
        ],
        'dependencies (must be VERIFIED)': [],
        'acceptance criteria': [
            'AC-1: every applied path matches the verified package bytes',
            'AC-2: every project-owned / unknown / conflicting path keeps its exact bytes',
            'AC-3: the project verification commands pass on the resulting branch',
        ],
        'validation commands': [
            'python scripts/validate_kit.py',
            'python -m unittest discover -s tests',
        ],
        'baseline branch / commit': environment['workspace'].get('branch') or '(detached)',
        'expected branch / worktree': branch,
        'evidence head sha': (report.get('head_sha_after')
                              or environment['workspace'].get('head_sha') or '(none)'),
        'evidence matrix': [],
        'risks / human decisions': [entry.get('decision_id')
                                    for entry in report.get('human_decisions') or []],
        'delivery evidence': 'engine report, applied / kept path list, pre/post hashes',
        'hard boundaries': 'no self-approval, no merge, no main-branch write',
        'kept paths': kept,
    }


def _local_delivery(environment, report, mode):
    applied, kept = _applied_and_kept(report)
    branch = _suggested_branch(report, 'local')
    return {
        'carrier': 'local_task_database',
        'automation': 'manual',
        'writes': [],
        'note': ('The entry did not write the local task database. The Master registers '
                 'the packet and dispatches the task; the engine changes stay in the '
                 'branch / Worktree named below.'),
        'branch': branch,
        'worktree': (report.get('operation') or {}).get('worktree'),
        'task_packet': _task_packet(environment, report, applied, kept, branch),
        'next_steps': [
            'python scripts/orchestrator.py task add <TASK-ID> --title "<title>" '
            '--paths <allowed paths> --acceptance "<AC-1>" "<AC-2>"',
            'python scripts/orchestrator.py task ready <TASK-ID>',
            'python scripts/orchestrator.py task dispatch <TASK-ID>',
            'pass the packet and the engine report to the Execution Agent',
        ],
    }


def _issue_body(environment, report, applied, kept):
    target = (report.get('target') or {}).get('version')
    lines = [
        '## 目标', '',
        f'把项目中的 AgentOrch 从 `{environment.get("format_version") or "unknown"}` '
        f'更新到 `{target or "unknown"}`，只应用官方 Release 校验通过的无冲突变更。', '',
        '## 入口识别结果', '',
        f'- edition: `{environment.get("edition")}`',
        f'- installed Format: `{environment.get("format_version")}`',
        f'- baseline: `{environment.get("baseline_status")}`',
        f'- engine result: `{report.get("result")}`',
        f'- verification: `{(report.get("target") or {}).get("verification", {}).get("status")}`',
        f'- head SHA: `{report.get("head_sha_after")}`', '',
        '## 无冲突变更（待审查）', '',
    ]
    lines.extend(f'- `{path}`' for path in applied) if applied else lines.append('(none)')
    lines.extend(['', '## 保留 / 需人工决策的路径', ''])
    lines.extend(f'- `{path}`' for path in kept) if kept else lines.append('(none)')
    lines.extend(['', '## 人工决策', ''])
    decisions = report.get('human_decisions') or []
    lines.extend(f'- `{entry.get("decision_id")}`: {entry.get("question")}'
                 for entry in decisions) if decisions else lines.append('(none)')
    lines.extend([
        '', '## 边界', '',
        '- 本 Issue 由入口生成，不代表已批准、已合并或已验证。',
        '- 不覆盖项目自有配置 / 业务代码；冲突逐文件保留。',
        '- 证据矩阵与 G1–G6 判定仍由 `kit/orchestration.md` 与其 4.1 节定义。',
    ])
    return '\n'.join(lines) + '\n'


def _github_delivery(environment, report, mode):
    applied, kept = _applied_and_kept(report)
    branch = _suggested_branch(report, 'github')
    target = (report.get('target') or {}).get('version')
    return {
        'carrier': 'issue_and_draft_pr',
        'automation': 'manual',
        'writes': [],
        'note': ('The entry did not call gh and did not create anything. Create the Issue '
                 'and Draft PR from these payloads; the Draft PR must link the Issue and '
                 'stay unmerged until Master review.'),
        'issue': {
            'title': f'S5/update: apply verified AgentOrch {target or "latest"}',
            'body': _issue_body(environment, report, applied, kept),
        },
        'draft_pr': {
            'base': 'main',
            'head': branch,
            'title': f'task: apply verified AgentOrch {target or "latest"}',
            'body': ('Draft PR prepared by the "更新 AgentOrch" entry. It links the update '
                     'Issue, carries the engine report and does not merge itself.\n'),
        },
        'next_steps': [
            'gh issue create --title "<title>" --body-file <issue-body.md>',
            'gh pr create --draft --base main --head <branch> --title "<title>" '
            '--body-file <pr-body.md>',
            'link the PR back to the Issue and record the engine report as evidence',
        ],
    }


# --------------------------------------------------------------------------- #
# Public entry
# --------------------------------------------------------------------------- #

def prepare_update(project, request, *, mode='PLAN', source=None, edition=None,
                   isolation=True, now=None):
    """Route one message, inspect the project, run the engine, shape the carrier.

    The function performs no carrier write: ``local`` returns a task packet and
    ``github`` returns Issue / Draft PR payloads. Use the returned ``status`` to
    decide whether to continue; ``NOT_UPDATE`` and ``NEEDS_CLARIFICATION`` mean
    no engine call happened.
    """
    if mode not in ('PLAN', 'APPLY'):
        raise EntryError(f'mode must be PLAN or APPLY, got {mode!r}')

    routing = route(request)
    if routing['route'] == planning.ROUTE_SELF_IMPROVEMENT:
        return {
            'status': 'NOT_UPDATE',
            'route': routing,
            'message': SELF_IMPROVEMENT_MESSAGE,
            'delivery': None,
            'gates': None,
            'engine_report': None,
            'environment': None,
        }
    if not routing['starts_update']:
        status = 'NEEDS_CLARIFICATION' if routing['needs_clarification'] else 'NOT_UPDATE'
        return {
            'status': status,
            'route': routing,
            'message': routing.get('clarification_question')
            or 'the message does not start an AgentOrch update',
            'delivery': None,
            'gates': None,
            'engine_report': None,
            'environment': None,
        }

    environment = inspect_environment(project)
    if environment['edition_error']:
        return {
            'status': 'STOPPED_INVALID_EDITION',
            'route': routing,
            'message': environment['edition_error'],
            'delivery': None,
            'gates': None,
            'engine_report': None,
            'environment': environment,
        }

    effective_edition = edition or environment['edition']
    if effective_edition not in ('local', 'github'):
        raise EntryError(f'edition must be local or github, got {effective_edition!r}')

    source = source if source is not None else sources.GitHubReleaseSource()
    report = engine.run_update(project, mode=mode, source=source,
                               edition=effective_edition, now=now,
                               isolation=isolation)
    environment['edition'] = effective_edition

    gate_record = gates(environment, report, mode)
    if effective_edition == 'local':
        delivery = _local_delivery(environment, report, mode)
    else:
        delivery = _github_delivery(environment, report, mode)

    engine_result = report.get('result')
    status = 'PREPARED' if engine_result in ('PLANNED', 'APPLIED') else engine_result
    return {
        'status': status,
        'route': routing,
        'message': None,
        'delivery': delivery,
        'gates': gate_record,
        'engine_report': report,
        'environment': environment,
    }


def build_parser():
    parser = argparse.ArgumentParser(
        prog='update_agentorch.entry',
        description='Route "更新 AgentOrch" and prepare a safe update for the current edition.',
    )
    parser.add_argument('--project', default='.', help='installed project root (default: .)')
    parser.add_argument('--request', required=True,
                        help='the user message to route, e.g. "更新 AgentOrch"')
    parser.add_argument('--mode', choices=('PLAN', 'APPLY'), default='PLAN',
                        help='PLAN inspects only; APPLY writes the conflict-free change set')
    parser.add_argument('--edition', choices=('local', 'github'),
                        help='override the edition read from docs/PROJECT.md')
    parser.add_argument('--no-isolation', action='store_true',
                        help='do not create an isolated worktree on a dirty APPLY')
    parser.add_argument('--output', help='also write the JSON payload to this path')
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        payload = prepare_update(args.project, args.request, mode=args.mode,
                                 edition=args.edition,
                                 isolation=not args.no_isolation)
    except (EntryError, ValueError) as exc:
        print(f'update_agentorch.entry: configuration error: {exc}', file=sys.stderr)
        return 1

    text = json.dumps(payload, indent=2, ensure_ascii=False) + '\n'
    if args.output:
        try:
            Path(args.output).write_text(text, encoding='utf-8')
        except OSError as exc:
            print(f'update_agentorch.entry: cannot write report to {args.output}: {exc}',
                  file=sys.stderr)
            return 1
    sys.stdout.write(text)

    if payload['status'] == 'NOT_UPDATE':
        print(f"update_agentorch.entry: not an update request ({payload['message']})",
              file=sys.stderr)
        return EXIT_NOT_UPDATE
    if payload['status'] == 'NEEDS_CLARIFICATION':
        print(f"update_agentorch.entry: clarification required: {payload['message']}",
              file=sys.stderr)
        return EXIT_NOT_UPDATE
    if payload['status'] in ('PREPARED', 'PLANNED', 'APPLIED'):
        return 0
    return engine.EXIT_CODES.get((payload.get('engine_report') or {}).get('result'), 1)


if __name__ == '__main__':
    raise SystemExit(main())
