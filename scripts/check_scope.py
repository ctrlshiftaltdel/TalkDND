#!/usr/bin/env python3
"""Read-only G2 scope check: changed paths vs the allowed paths of one task.

Merge gate G2 asks whether every changed path is inside the allowed paths of the
task under review. The historical evidence was a human comparison of
``git diff --name-only`` + ``git status --short``. That comparison has no base
SHA: a bare ``git diff`` compares worktree to index, so it ignores work already
committed on the branch, and it silently drops untracked files. This tool turns
the same question into a set judgement:

* **base** - an explicit ``--base <40-hex>`` wins; otherwise the selected task's
  ``approved_sha``, then its ``head_sha``. When both are empty the answer is
  "undeterminable" (exit 1), never a silent pass.
* **changes** - three sources, unioned: committed paths in ``base...HEAD``
  (three-dot: only this branch's commits), worktree/staged edits against ``HEAD``,
  and every untracked file (``--exclude-standard``). Rename detection is off, so
  a rename reports both the deleted and the added path, and a deletion is never
  lost.
* **allowed paths** - ``paths_json`` of one task in the local task database.
  "One task" is resolved as: ``--task <id>`` when given (any status, so an
  archived task can be audited), otherwise the single task whose status is in
  ``scripts.orchestrator.ACTIVE``. Zero or several active tasks is exit 1 with
  the candidate ids; the activity set is imported, never re-declared here.

Exit codes mirror ``scripts/check_drift.py``:

* ``0`` - every change is inside the allowed paths (``in-scope``)
* ``2`` - at least one change is outside them (``out-of-scope``)
* ``1`` - invalid input, unreadable task database, Git error, or a base that
  cannot be resolved to a commit (``error``); undeterminable is never a pass

Read-only guarantee: the task database is copied into a private temporary
directory and read there, so not even SQLite's ``-shm`` sidecar appears next to
the original registry, and a live ``-wal`` is replayed on the copy instead of
being ignored. The Git calls are ``diff`` / ``ls-files`` / ``rev-parse`` /
``cat-file -e`` / ``--show-toplevel``. Nothing under the project root is
written, nothing is staged, committed, checked out or cleaned, no hook is
installed, and ``.git/config`` is never modified. ``ACTIVE`` and ``overlaps``
are imported from ``scripts/orchestrator`` so the activity set and the
path-overlap rule have one definition each.

Only the standard library is used.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

try:  # repository root on sys.path: `python -m unittest`, CI, module import
    from scripts.orchestrator import ACTIVE, overlaps
except ImportError:  # `python scripts/check_scope.py`: only scripts/ is on sys.path
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from scripts.orchestrator import ACTIVE, overlaps

REPORT_SCHEMA_VERSION = 1

EXIT_IN_SCOPE = 0
EXIT_ERROR = 1
EXIT_OUT_OF_SCOPE = 2

STATUS_IN_SCOPE = 'in-scope'
STATUS_OUT_OF_SCOPE = 'out-of-scope'
STATUS_ERROR = 'error'

# A base must be a full commit SHA. Short SHAs, revision expressions (`main`,
# `HEAD~2`) and prefixes are rejected on purpose: the base is evidence, and a
# moving name is not reproducible.
SHA_RE = re.compile(r'^[0-9a-fA-F]{40}$')

DB_DIR = '.orchestration'
DB_NAME = 'state.sqlite3'
TASK_COLUMNS = 'id,status,title,paths_json,approved_sha,head_sha'


class ScopeError(Exception):
    """Invalid input, unreadable state, or a Git failure (exit code 1)."""


# --- task database (read-only) ----------------------------------------------

def db_path(root):
    return Path(root) / DB_DIR / DB_NAME


def load_tasks(root):
    """Every task in <root>/.orchestration/state.sqlite3 as a dict keyed by id.

    The registry is read from a copy in a private temporary directory. Opening
    the original in place would make SQLite create a ``-shm`` sidecar next to it
    (WAL databases need shared memory even for read-only readers), which would
    break the promise that this check writes nothing under the project root. The
    copy also lets SQLite replay a pending ``-wal``, so the newest committed rows
    are visible. ``orchestrator.connect()` is deliberately not used: it migrates
    the database on open.
    """
    path = db_path(root)
    if not path.is_file():
        raise ScopeError(
            f'task database not found: {path}; this check reads the local registry '
            f'({DB_DIR}/{DB_NAME}) and never creates it')
    try:
        with tempfile.TemporaryDirectory(prefix='check_scope-') as tmp:
            copy = Path(tmp) / DB_NAME
            shutil.copyfile(path, copy)
            wal = path.with_name(path.name + '-wal')
            if wal.is_file():
                shutil.copyfile(wal, copy.with_name(copy.name + '-wal'))
            return _read_tasks(copy, path)
    except OSError as exc:
        raise ScopeError(f'cannot read the task database {path}: {exc}')


def _read_tasks(path, display=None):
    label = display or path
    try:
        con = sqlite3.connect(path)
    except sqlite3.Error as exc:
        raise ScopeError(f'cannot read the task database {label}: {exc}')
    try:
        con.row_factory = sqlite3.Row
        rows = con.execute(f'SELECT {TASK_COLUMNS} FROM tasks').fetchall()
    except sqlite3.Error as exc:
        raise ScopeError(f'cannot read the task database {label}: {exc}')
    finally:
        con.close()

    tasks = {}
    for row in rows:
        try:
            paths = json.loads(row['paths_json'])
        except (TypeError, ValueError) as exc:
            raise ScopeError(f'task {row["id"]} has an unreadable paths_json: {exc}')
        if not isinstance(paths, list) or not all(isinstance(p, str) and p for p in paths):
            raise ScopeError(f'task {row["id"]} has no usable allowed paths')
        tasks[row['id']] = {
            'id': row['id'],
            'status': row['status'],
            'title': row['title'],
            'allowed_paths': list(paths),
            'approved_sha': (row['approved_sha'] or '').strip(),
            'head_sha': (row['head_sha'] or '').strip(),
        }
    return tasks


def select_task(tasks, task_id=None):
    """Return (task, notes). Never guesses when the answer is ambiguous."""
    if task_id:
        if task_id not in tasks:
            raise ScopeError(
                f'unknown task {task_id!r} in the task database; known ids: '
                + (', '.join(sorted(tasks)) or '(none)'))
        task = tasks[task_id]
        notes = []
        if task['status'] not in ACTIVE:
            notes.append(f'task {task["id"]} is {task["status"]}, not an active status; '
                         'it was checked because it was named explicitly')
        return task, notes

    active = [task for task in tasks.values() if task['status'] in ACTIVE]
    if not active:
        raise ScopeError(
            'no active task in the task database: every task is terminal or the '
            'registry is empty, so no allowed paths can be attributed; pass '
            '--task <id> to name the task (a terminal task is accepted there)')
    if len(active) > 1:
        listed = ', '.join(f'{task["id"]} ({task["status"]})' for task in sorted(
            active, key=lambda item: item['id']))
        raise ScopeError(
            f'multiple active tasks: {listed}; pass --task <id> to select exactly '
            'one set of allowed paths (this check never unions them)')
    return active[0], []


# --- base resolution ---------------------------------------------------------

def _checked_sha(value, label):
    text = (value or '').strip()
    if not SHA_RE.match(text):
        raise ScopeError(f'{label} must be a 40-hex commit SHA, got {value!r}')
    return text.lower()


def resolve_base(explicit, task):
    """Return (sha, source). Undeterminable is an error, never an empty base."""
    if explicit is not None:
        return _checked_sha(explicit, '--base'), '--base'
    for source in ('approved_sha', 'head_sha'):
        value = task.get(source) or ''
        if value:
            return _checked_sha(value, f'task {task["id"]} {source}'), source
    raise ScopeError(
        f'task {task["id"]} has neither approved_sha nor head_sha; pass --base '
        '<40-hex commit sha> so the diff has a fixed anchor')


# --- git plumbing (read-only) ------------------------------------------------

def _decode(raw):
    return raw.decode('utf-8', 'surrogateescape')


def _git(root, *args):
    try:
        proc = subprocess.run(['git', '-C', str(root), *args], capture_output=True)
    except OSError as exc:
        raise ScopeError(f'cannot run git: {exc}')
    if proc.returncode != 0:
        stderr = _decode(proc.stderr).strip()
        raise ScopeError(f'git {" ".join(args)} failed (exit {proc.returncode})'
                         + (f': {stderr}' if stderr else ''))
    return proc.stdout


def _z_paths(raw):
    return [path for path in _decode(raw).split('\0') if path]


def work_tree_root(root):
    """Resolve and require the top level of a Git work tree."""
    root_path = Path(root)
    if not root_path.exists():
        raise ScopeError(f'project root does not exist: {root}')
    if not root_path.is_dir():
        raise ScopeError(f'project root is not a directory: {root}')
    root_path = root_path.resolve()
    top = _decode(_git(root_path, 'rev-parse', '--show-toplevel')).strip()
    if not top:
        raise ScopeError(f'not inside a git work tree: {root_path}')
    top_path = Path(top).resolve()
    if top_path != root_path:
        raise ScopeError(
            f'{root_path} is not the top level of its git work tree ({top_path}); '
            'allowed paths are repository-relative, so pass the repository root')
    return root_path


def require_commit(root, sha, source):
    try:
        proc = subprocess.run(['git', '-C', str(root), 'cat-file', '-e', f'{sha}^{{commit}}'],
                              capture_output=True)
    except OSError as exc:
        raise ScopeError(f'cannot run git: {exc}')
    if proc.returncode != 0:
        raise ScopeError(f'base {sha} (from {source}) is not a commit in this '
                         'repository; pass a full 40-hex SHA of a commit that exists here')


def collect_changes(root, base):
    """{path: [sources]} for committed, worktree and untracked changes."""
    changes = {}

    def add(paths, source):
        for path in paths:
            sources = changes.setdefault(path, [])
            if source not in sources:
                sources.append(source)

    # Three-dot: only the commits on this branch, not the reverse of commits that
    # landed on the base branch after the fork point.
    add(_z_paths(_git(root, 'diff', '--name-only', '--no-renames', '-z', f'{base}...HEAD')),
        'committed')
    # Staged + unstaged edits against HEAD. Deletions appear here as well.
    add(_z_paths(_git(root, 'diff', '--name-only', '--no-renames', '-z', 'HEAD')), 'worktree')
    # Every untracked, non-ignored file counts: "no untracked baggage" is exactly
    # what G2 wants to keep visible.
    add(_z_paths(_git(root, 'ls-files', '--others', '--exclude-standard', '-z')), 'untracked')
    return changes


# --- report ------------------------------------------------------------------

def _empty_report(root):
    return {
        'tool': 'check_scope',
        'report_schema_version': REPORT_SCHEMA_VERSION,
        'root': str(root),
        'status': STATUS_ERROR,
        'exit_code': EXIT_ERROR,
        'task': None,
        'base': {'sha': None, 'source': None},
        'changes': [],
        'in_scope': [],
        'out_of_scope': [],
        'summary': {'committed': 0, 'worktree': 0, 'untracked': 0,
                    'total': 0, 'in_scope': 0, 'out_of_scope': 0},
        'notes': [],
    }


def check_scope(root='.', task_id=None, base=None):
    """Return the stable report dict. Never writes anything."""
    root_path = work_tree_root(root)
    tasks = load_tasks(root_path)
    task, notes = select_task(tasks, task_id)
    base_sha, base_source = resolve_base(base, task)
    require_commit(root_path, base_sha, base_source)
    changes = collect_changes(root_path, base_sha)

    report = _empty_report(root_path)
    report['task'] = {'id': task['id'], 'status': task['status'],
                      'title': task['title'], 'allowed_paths': task['allowed_paths']}
    report['base'] = {'sha': base_sha, 'source': base_source}

    entries = []
    for path in sorted(changes):
        in_scope = any(overlaps(path, allowed) for allowed in task['allowed_paths'])
        entries.append({'path': path, 'sources': changes[path], 'in_scope': in_scope})
        (report['in_scope'] if in_scope else report['out_of_scope']).append(path)
    report['changes'] = entries

    summary = report['summary']
    summary['total'] = len(entries)
    summary['in_scope'] = len(report['in_scope'])
    summary['out_of_scope'] = len(report['out_of_scope'])
    for entry in entries:
        for source in entry['sources']:
            summary[source] += 1

    out_of_scope = bool(report['out_of_scope'])
    report['status'] = STATUS_OUT_OF_SCOPE if out_of_scope else STATUS_IN_SCOPE
    report['exit_code'] = EXIT_OUT_OF_SCOPE if out_of_scope else EXIT_IN_SCOPE
    report['notes'] = notes
    if not entries:
        report['notes'].append('no changes against the base: nothing to judge')
    return report


def error_report(root, message):
    try:
        resolved = str(Path(root).resolve())
    except OSError:
        resolved = str(root)
    report = _empty_report(resolved)
    report['error'] = message
    return report


def render_report(report, stream):
    """Human-readable evidence: base SHA, task, changed paths, verdict."""
    print(f"status: {report['status']} (exit {report['exit_code']})", file=stream)
    if report.get('error'):
        print(f"error: {report['error']}", file=stream)
    task = report['task']
    if task:
        print(f"task: {task['id']} ({task['status']}), allowed: "
              + (', '.join(task['allowed_paths']) or '(none)'), file=stream)
    base = report['base']
    if base['sha']:
        print(f"base: {base['sha']} (source: {base['source']})", file=stream)
    summary = report['summary']
    print(f"changes: {summary['total']} (committed {summary['committed']}, "
          f"worktree {summary['worktree']}, untracked {summary['untracked']}); "
          f"in scope {summary['in_scope']}, out of scope {summary['out_of_scope']}",
          file=stream)
    for entry in report['changes']:
        label = 'in-scope' if entry['in_scope'] else 'out-of-scope'
        print(f"  [{label}] {entry['path']} ({', '.join(entry['sources'])})", file=stream)
    for note in report['notes']:
        print(f'note: {note}', file=stream)


# --- CLI ---------------------------------------------------------------------

class _Parser(argparse.ArgumentParser):
    def error(self, message):
        # argparse would exit 2; here 2 means "out of scope", so a bad invocation
        # must not be mistaken for an out-of-scope verdict.
        raise ScopeError(f'{message} (see --help)')


def build_parser():
    parser = _Parser(
        description='Read-only G2 scope check: changed paths vs one task\'s allowed paths.',
        epilog='Exit codes: 0 in scope, 2 out of scope, 1 invalid input or undeterminable.')
    parser.add_argument('--root', default='.',
                        help='repository root (default: current directory)')
    parser.add_argument('--task', default=None,
                        help='task id to read allowed paths from; required when more '
                             'than one task is active')
    parser.add_argument('--base', default=None,
                        help='40-hex commit SHA to diff against; defaults to the '
                             'task approved_sha, then its head_sha')
    parser.add_argument('--json', action='store_true',
                        help='print one JSON document on stdout; human summary goes to stderr')
    return parser


def main(argv=None):
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
    except ScopeError as exc:
        print(f'ERROR: {exc}', file=sys.stderr)
        return EXIT_ERROR

    try:
        report = check_scope(args.root, args.task, args.base)
    except ScopeError as exc:
        print(f'ERROR: {exc}', file=sys.stderr)
        if args.json:
            print(json.dumps(error_report(args.root, str(exc)), indent=2,
                             sort_keys=True, ensure_ascii=False))
        return EXIT_ERROR

    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False))
        render_report(report, sys.stderr)
    else:
        render_report(report, sys.stdout)
    return report['exit_code']


if __name__ == '__main__':
    sys.exit(main())
