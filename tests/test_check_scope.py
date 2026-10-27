"""Tests for the read-only G2 scope check (`scripts/check_scope.py`).

The tool answers one question: are every changed path (committed since a base
SHA, plus working-tree edits, plus untracked files) inside the allowed paths of
one active task in the local task database? Exit codes mirror
`scripts/check_drift.py`: 0 in scope, 2 out of scope, 1 undeterminable.

Every test builds its own throwaway Git repository and task database; the real
`.orchestration/` database is never opened for writing and the repository's
Git configuration is never touched.
"""
import contextlib
import hashlib
import io
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
# Import the module the way a reviewer does: `from scripts.orchestrator import
# ACTIVE` has to resolve, so the repository root must be importable.
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts import check_scope as check  # noqa: E402
from scripts import orchestrator  # noqa: E402

NEVER = '0' * 40


class GitFixture:
    """A minimal repository with a real task database built by orchestrator.init."""

    def __init__(self, tmp):
        self.root = Path(tmp).resolve()
        self.git('init', '-q')
        self.git('config', 'user.name', 'Scope Test')
        self.git('config', 'user.email', 'scope@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        self.write('.gitignore', '.orchestration/\n')
        self.write('docs/PROJECT.md', '# Project\n\n- `edition`: `local`\n')
        self.base = self.commit('base')

    # --- git plumbing -------------------------------------------------------
    def git(self, *args, check_ok=True):
        return subprocess.run(['git', '-C', str(self.root), *args],
                              capture_output=True, text=True, check=check_ok)

    def commit(self, message='change'):
        self.git('add', '-A')
        self.git('commit', '-q', '-m', message)
        return self.git('rev-parse', 'HEAD').stdout.strip()

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')
        return path

    def remove(self, rel):
        (self.root / rel).unlink()

    # --- task database ------------------------------------------------------
    @property
    def db_path(self):
        return self.root / '.orchestration' / 'state.sqlite3'

    def init_db(self):
        with contextlib.redirect_stdout(io.StringIO()):
            orchestrator.init(self.root)
        return self

    def add_task(self, task_id, paths, status='DISPATCHED', head_sha='', approved_sha=''):
        con = sqlite3.connect(self.db_path)
        try:
            con.execute(
                'INSERT INTO tasks(id,title,description,paths_json,depends_json,'
                'acceptance_json,status,head_sha,approved_sha,created_at,updated_at) '
                'VALUES(?,?,?,?,?,?,?,?,?,?,?)',
                (task_id, task_id, '', json.dumps(paths), '[]', '["works"]', status,
                 head_sha, approved_sha, '2026-10-27T00:00:00+00:00',
                 '2026-10-27T00:00:00+00:00'))
            con.commit()
        finally:
            con.close()
        return self

    # --- snapshots ----------------------------------------------------------
    def tree_snapshot(self):
        return sorted(p.relative_to(self.root).as_posix()
                      for p in self.root.rglob('*') if p.is_file())

    def db_digest(self):
        return hashlib.sha256(self.db_path.read_bytes()).hexdigest()


class CheckScopeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.fx = GitFixture(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    # --- helpers ------------------------------------------------------------
    def run_tool(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = check.main(['--root', str(self.fx.root), *args])
        return code, out.getvalue(), err.getvalue()

    def run_json(self, *args):
        code, out, err = self.run_tool('--json', *args)
        return code, json.loads(out), err

    # --- AC-1 / AC-2: base resolution and exit codes ------------------------
    def test_explicit_base_with_in_scope_change_returns_zero(self):
        self.fx.init_db()
        self.fx.write('src/app.py', 'print("ok")\n')
        self.fx.commit('add app')
        self.fx.write('src/app.py', 'print("ok")\nprint("more")\n')
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 0, report)
        self.assertEqual(report['status'], 'in-scope')
        self.assertEqual(report['base']['source'], '--base')
        self.assertIn('src/app.py', report['in_scope'])
        self.assertEqual(report['out_of_scope'], [])

    def test_out_of_scope_change_returns_two(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        self.fx.write('docs/notes.md', 'unrelated\n')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 2)
        self.assertEqual(report['status'], 'out-of-scope')
        self.assertEqual(report['out_of_scope'], ['docs/notes.md'])
        self.assertEqual(report['exit_code'], 2)

    def test_missing_base_everywhere_returns_one(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'])
        self.fx.write('src/app.py', 'x\n')
        code, out, err = self.run_tool('--task', 'T-1')
        self.assertEqual(code, 1)
        self.assertIn('pass --base', err)

    def test_base_falls_back_to_approved_sha_then_head_sha(self):
        self.fx.init_db()
        self.fx.write('src/app.py', 'one\n')
        approved = self.fx.commit('app v1')
        self.fx.write('src/app.py', 'one\ntwo\n')
        head = self.fx.commit('app v2')
        self.fx.write('src/app.py', 'one\ntwo\nthree\n')

        self.fx.add_task('T-APPROVED', ['src/'], status='APPROVED',
                         head_sha=head, approved_sha=approved)
        code, report, _ = self.run_json('--task', 'T-APPROVED')
        self.assertEqual(code, 0, report)
        self.assertEqual(report['base']['sha'], approved)
        self.assertEqual(report['base']['source'], 'approved_sha')

        self.fx.add_task('T-HEAD', ['src/'], head_sha=approved)
        code, report, _ = self.run_json('--task', 'T-HEAD')
        self.assertEqual(code, 0, report)
        self.assertEqual(report['base']['sha'], approved)
        self.assertEqual(report['base']['source'], 'head_sha')

    def test_base_that_is_not_a_full_sha_returns_one(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        code, out, err = self.run_tool('--task', 'T-1', '--base', 'deadbeef')
        self.assertEqual(code, 1)
        self.assertIn('40-hex', err)

        for bad in ('main', self.fx.base[:39], self.fx.base + 'a', 'g' * 40):
            code, _, err = self.run_tool('--task', 'T-1', '--base', bad)
            self.assertEqual(code, 1, bad)
            self.assertIn('40-hex', err)

    def test_base_sha_absent_from_the_repository_returns_one(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        code, out, err = self.run_tool('--task', 'T-1', '--base', NEVER)
        self.assertEqual(code, 1)
        self.assertIn('not a commit', err)

    def test_invalid_arguments_return_one_instead_of_argparse_two(self):
        with contextlib.redirect_stderr(io.StringIO()):
            code = check.main(['--root', str(self.fx.root), '--nonsense'])
        self.assertEqual(code, 1)

    # --- AC-3: activity set comes from scripts.orchestrator.ACTIVE -----------
    def test_active_set_is_the_orchestrator_object(self):
        self.assertIs(check.ACTIVE, orchestrator.ACTIVE)
        for status in ('DISPATCHED', 'IN_PROGRESS', 'REVIEW_READY', 'MASTER_REVIEW',
                       'APPROVED', 'MERGED', 'BLOCKED', 'NEEDS_HUMAN_DECISION', 'FAILED'):
            self.assertIn(status, check.ACTIVE)
        for status in ('PLANNED', 'READY', 'VERIFIED', 'CANCELED'):
            self.assertNotIn(status, check.ACTIVE)

    def test_approved_only_task_is_still_active(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], status='APPROVED',
                         head_sha=self.fx.base, approved_sha=self.fx.base)
        self.fx.write('src/app.py', 'a\nb\n')
        code, report, _ = self.run_json()
        self.assertEqual(code, 0, report)
        self.assertEqual(report['task']['id'], 'T-1')
        self.assertEqual(report['task']['status'], 'APPROVED')

    def test_no_active_task_returns_one_instead_of_silent_zero(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], status='VERIFIED',
                         head_sha=self.fx.base, approved_sha=self.fx.base)
        self.fx.write('src/app.py', 'x\n')
        code, out, err = self.run_tool()
        self.assertEqual(code, 1)
        self.assertIn('no active task', err)

    def test_multiple_active_tasks_require_an_explicit_task(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        self.fx.add_task('T-2', ['docs/'], status='BLOCKED', head_sha=self.fx.base)
        self.fx.write('src/app.py', 'x\n')

        code, out, err = self.run_tool()
        self.assertEqual(code, 1)
        self.assertIn('multiple active tasks', err)
        self.assertIn('T-1', err)
        self.assertIn('T-2', err)

        code, report, _ = self.run_json('--task', 'T-1')
        self.assertEqual(code, 0, report)
        self.assertEqual(report['task']['id'], 'T-1')

    def test_unknown_task_returns_one(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        code, out, err = self.run_tool('--task', 'T-NOPE')
        self.assertEqual(code, 1)
        self.assertIn('unknown task', err)

    # --- AC-4: coverage of the change collection ----------------------------
    def test_parent_directory_prefix_overlap(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        self.fx.write('src/deep/nested/mod.py', 'x\n')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 0, report)
        self.assertIn('src/deep/nested/mod.py', report['in_scope'])

    def test_sibling_file_outside_a_single_file_path_is_out_of_scope(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/core.py'], head_sha=self.fx.base)
        self.fx.write('src/other.py', 'x\n')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 2, report)
        self.assertEqual(report['out_of_scope'], ['src/other.py'])

    def test_untracked_files_are_changes(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        self.fx.write('src/new.py', 'x\n')
        self.fx.write('notes.txt', 'x\n')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 2, report)
        self.assertIn('src/new.py', report['in_scope'])
        self.assertEqual(report['out_of_scope'], ['notes.txt'])
        sources = {item['path']: item['sources'] for item in report['changes']}
        self.assertEqual(sources['notes.txt'], ['untracked'])

    def test_deleted_files_are_changes(self):
        self.fx.write('src/old.py', 'x\n')
        self.fx.commit('add old')
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        self.fx.remove('src/old.py')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 0, report)
        self.assertIn('src/old.py', report['in_scope'])
        sources = {item['path']: item['sources'] for item in report['changes']}
        self.assertIn('worktree', sources['src/old.py'])

        self.fx.write('docs/gone.md', 'x\n')
        self.fx.commit('add gone')
        self.fx.remove('docs/gone.md')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 2, report)
        self.assertEqual(report['out_of_scope'], ['docs/gone.md'])

    def test_rename_reports_both_sides(self):
        self.fx.write('src/a.py', 'x\n')
        self.fx.commit('add a')
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        (self.fx.root / 'other').mkdir()
        self.fx.git('mv', 'src/a.py', 'other/a.py')
        code, report, _ = self.run_json('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 2, report)
        self.assertIn('src/a.py', report['in_scope'])
        self.assertEqual(report['out_of_scope'], ['other/a.py'])

    def test_task_database_missing_returns_one(self):
        self.fx.write('src/app.py', 'x\n')
        code, out, err = self.run_tool('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 1)
        self.assertIn('task database', err)

    def test_task_database_corrupt_returns_one(self):
        self.fx.init_db()
        self.fx.db_path.write_bytes(b'not a sqlite database at all')
        code, out, err = self.run_tool('--task', 'T-1', '--base', self.fx.base)
        self.assertEqual(code, 1)
        self.assertIn('task database', err)

    def test_root_outside_a_git_work_tree_returns_one(self):
        outside = Path(self.tmp.name) / 'plain'
        outside.mkdir()
        (outside / '.orchestration').mkdir()
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = check.main(['--root', str(outside), '--base', self.fx.base])
        self.assertEqual(code, 1)
        self.assertIn('git', err.getvalue().lower())

    # --- AC-5: read-only, no hooks, no .git/config changes -------------------
    def test_run_never_writes_or_touches_git_configuration(self):
        self.fx.init_db()
        self.fx.add_task('T-1', ['src/'], head_sha=self.fx.base)
        self.fx.write('src/app.py', 'x\n')

        config_path = self.fx.root / '.git' / 'config'
        config_before = config_path.read_bytes()
        hooks_before = sorted(p.name for p in (self.fx.root / '.git' / 'hooks').iterdir())
        hooks_path_before = self.fx.git('config', '--get', 'core.hooksPath', check_ok=False)
        tree_before = self.fx.tree_snapshot()
        status_before = self.fx.git('status', '--porcelain').stdout
        digest_before = self.fx.db_digest()

        self.assertEqual(self.run_tool('--task', 'T-1')[0], 0)
        self.assertEqual(self.run_tool('--task', 'T-1', '--base', 'bad')[0], 1)

        self.assertEqual(config_path.read_bytes(), config_before)
        self.assertEqual(sorted(p.name for p in (self.fx.root / '.git' / 'hooks').iterdir()),
                         hooks_before)
        hooks_path_after = self.fx.git('config', '--get', 'core.hooksPath', check_ok=False)
        self.assertEqual(hooks_path_after.returncode, hooks_path_before.returncode)
        self.assertNotEqual(hooks_path_before.returncode, 0,
                            'the fixture repository must not define core.hooksPath')
        self.assertEqual(hooks_path_after.stdout, '')
        self.assertEqual(self.fx.tree_snapshot(), tree_before)
        self.assertEqual(self.fx.git('status', '--porcelain').stdout, status_before)
        self.assertEqual(self.fx.db_digest(), digest_before)

    # --- integration: a task created by the orchestrator CLI -----------------
    def test_task_created_through_the_orchestrator_api_is_readable(self):
        self.fx.init_db()
        args = SimpleNamespace(id='TASK-0001', title='scope', description='',
                               paths=['src/'], depends=[], acceptance=['works'])
        with contextlib.redirect_stdout(io.StringIO()):
            orchestrator.task_add(self.fx.root, args)
            orchestrator.task_ready(self.fx.root, 'TASK-0001')
            orchestrator.task_dispatch(self.fx.root, 'TASK-0001')
        self.fx.write('src/app.py', 'x\n')
        code, report, _ = self.run_json('--base', self.fx.base)
        self.assertEqual(code, 0, report)
        self.assertEqual(report['task']['id'], 'TASK-0001')
        self.assertEqual(report['task']['status'], 'DISPATCHED')
        self.assertEqual(report['task']['allowed_paths'], ['src'])


if __name__ == '__main__':
    unittest.main()
