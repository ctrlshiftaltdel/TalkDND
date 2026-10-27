import contextlib, io, json, os, sqlite3, subprocess, sys, tempfile, time, unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock
import importlib.util

SCRIPT=Path(__file__).resolve().parents[1]/'scripts'/'orchestrator.py'
spec=importlib.util.spec_from_file_location('orch',SCRIPT)
orch=importlib.util.module_from_spec(spec); spec.loader.exec_module(orch)

# Worker for the amend --paths race test: two blocked tasks try to claim the same
# new path at a synchronised instant, so the lock rewrite and the conflict check
# in one BEGIN IMMEDIATE must let exactly one of them through.
AMEND_PATHS_WORKER = r'''
import importlib.util, json, sys, time
from pathlib import Path
from types import SimpleNamespace
spec = importlib.util.spec_from_file_location('orch_amend_worker', sys.argv[1])
orch = importlib.util.module_from_spec(spec); spec.loader.exec_module(orch)
root, task, path, start = Path(sys.argv[2]), sys.argv[3], sys.argv[4], float(sys.argv[5])
while time.time() < start:
    time.sleep(0.005)
out = {'task': task, 'path': path}
try:
    orch.task_amend(root, SimpleNamespace(id=task, merge_sha=None,
                                          note='re-scope after block; authorised in the task thread',
                                          paths=[path]))
    out['status'] = 'AMENDED'
except Exception as exc:
    out['status'] = 'REFUSED'; out['error'] = f'{type(exc).__name__}: {exc}'
print(json.dumps(out))
'''

class LocalOrchestratorTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.root=Path(self.tmp.name)
        # The edition is read from docs/PROJECT.md only, and is required:
        # every local-CLI test root therefore declares it explicitly.
        self.write_edition('local')
        orch.init(self.root)
    def tearDown(self): self.tmp.cleanup()
    def write_edition(self, value, key='edition'):
        (self.root/'docs').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs').mkdir(parents=True, exist_ok=True)
        (self.root/'docs/PROJECT.md').write_text(
            f'# Project\n\n## 运行配置\n\n- `{key}`：`{value}`\n',encoding='utf-8')
    def add(self,tid,paths,depends=None):
        class A: pass
        a=A(); a.id=tid; a.title=tid; a.description=''; a.paths=paths; a.depends=depends or []; a.acceptance=['works']
        orch.task_add(self.root,a)
    def test_parent_child_lock_conflict(self):
        self.add('TASK-0001',['src/']); self.add('TASK-0002',['src/main.py'])
        orch.task_ready(self.root,'TASK-0001')
        class A: pass
        a=A(); a.active_only=False
        orch.task_dispatch(self.root,'TASK-0001')
        with self.assertRaises(RuntimeError): orch.task_ready(self.root,'TASK-0002')
    def test_independent_paths_can_dispatch(self):
        self.add('TASK-0001',['src/a.py']); self.add('TASK-0002',['src/b.py'])
        orch.task_ready(self.root,'TASK-0001'); orch.task_dispatch(self.root,'TASK-0001')
        orch.task_ready(self.root,'TASK-0002'); orch.task_dispatch(self.root,'TASK-0002')
    def test_dependency_must_be_verified(self):
        self.add('TASK-0001',['a.py']); self.add('TASK-0002',['b.py'],['TASK-0001'])
        with self.assertRaises(RuntimeError): orch.task_ready(self.root,'TASK-0002')
    def test_retry_cap(self):
        self.add('TASK-0001',['a.py']); orch.task_ready(self.root,'TASK-0001'); orch.task_dispatch(self.root,'TASK-0001')
        class A: pass
        for dest in ['IN_PROGRESS','REVIEW_READY','MASTER_REVIEW','IN_PROGRESS','REVIEW_READY','MASTER_REVIEW','IN_PROGRESS','REVIEW_READY','MASTER_REVIEW','IN_PROGRESS','REVIEW_READY','MASTER_REVIEW']:
            x=A(); x.id='TASK-0001'; x.status=dest; x.head_sha=None; x.note=''; orch.task_status(self.root,x)
        x=A(); x.id='TASK-0001'; x.status='IN_PROGRESS'; x.head_sha=None; x.note=''; orch.task_status(self.root,x)
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0001',)).fetchone()['status'],'BLOCKED')
        finally: con.close()
    def test_full_approval_merge_verification_and_lock_release(self):
        self.add('TASK-0001',['src/app.py'])
        orch.task_ready(self.root,'TASK-0001'); orch.task_dispatch(self.root,'TASK-0001')
        class A: pass
        def go(status, head_sha=None, merge_sha=None, note=''):
            a=A(); a.id='TASK-0001'; a.status=status; a.head_sha=head_sha; a.merge_sha=merge_sha; a.note=note or ('evidence recorded' if status in {'APPROVED','MERGED','VERIFIED','BLOCKED','NEEDS_HUMAN_DECISION','FAILED','CANCELED'} else '')
            orch.task_status(self.root,a)
        go('IN_PROGRESS')
        go('REVIEW_READY',head_sha='abc123')
        go('MASTER_REVIEW')
        go('APPROVED',head_sha='abc123')
        go('MERGED',merge_sha='d'*40)
        go('VERIFIED')
        con=orch.connect(self.root)
        try:
            row=con.execute('SELECT status,head_sha,approved_sha,merged_sha FROM tasks WHERE id=?',('TASK-0001',)).fetchone()
            self.assertEqual(row['status'],'VERIFIED')
            self.assertEqual(row['head_sha'],'abc123')
            self.assertEqual(row['approved_sha'],'abc123')
            self.assertEqual(row['merged_sha'],'d'*40)
            self.assertEqual(con.execute('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0001',)).fetchone()[0],0)
        finally: con.close()

    def test_normalize_rejects_parent_path(self):
        with self.assertRaises(ValueError): orch.normalize_path('../secret')
        with self.assertRaises(ValueError): orch.normalize_path('src/**/*.py')
        with self.assertRaises(ValueError): orch.normalize_path('C:\\private\\file.txt')

    # --- Shared helpers for acceptance-gate tests ---
    def advance(self, tid, dest, head_sha=None, merge_sha=None, note=''):
        class A: pass
        a=A(); a.id=tid; a.status=dest; a.head_sha=head_sha; a.merge_sha=merge_sha
        a.note=note or ('evidence recorded' if dest in {'APPROVED','MERGED','VERIFIED','BLOCKED','NEEDS_HUMAN_DECISION','FAILED','CANCELED'} else '')
        orch.task_status(self.root,a)
    def workspace(self, tid, head_sha='', branch='', worktree='', note=''):
        class A: pass
        a=A(); a.id=tid; a.head_sha=head_sha; a.branch=branch; a.worktree=worktree; a.note=note
        orch.set_workspace(self.root,a)
    def count(self, sql, params=()):
        con=orch.connect(self.root)
        try: return con.execute(sql,params).fetchone()[0]
        finally: con.close()
    def drive_to_master_review(self, tid, path, head_sha):
        self.add(tid,[path]); orch.task_ready(self.root,tid); orch.task_dispatch(self.root,tid)
        self.advance(tid,'IN_PROGRESS'); self.advance(tid,'REVIEW_READY',head_sha=head_sha)
        self.advance(tid,'MASTER_REVIEW')
    def drive_to_approved(self, tid, path, head_sha):
        self.drive_to_master_review(tid,path,head_sha); self.advance(tid,'APPROVED',head_sha=head_sha)

    # Gate B: dispatch only from a READY activity state.
    def test_non_ready_dispatch_is_refused(self):
        self.add('TASK-0101',['src/'])
        with self.assertRaises(RuntimeError) as cm: orch.task_dispatch(self.root,'TASK-0101')
        self.assertIn('only READY tasks can be dispatched',str(cm.exception))
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0101',)),0)
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0101',)).fetchone()['status'],'PLANNED')
        finally: con.close()

    # Gate C: APPROVED must be bound to the exact current head SHA.
    def test_approved_head_sha_mismatch_is_refused(self):
        head='a'*40; other='b'*40
        self.drive_to_master_review('TASK-0102','src/',head)
        with self.assertRaises(RuntimeError) as cm: self.advance('TASK-0102','APPROVED',head_sha=other)
        self.assertIn('does not match current task head',str(cm.exception))
        self.advance('TASK-0102','APPROVED',head_sha=head)
        con=orch.connect(self.root)
        try:
            row=con.execute('SELECT status,approved_sha FROM tasks WHERE id=?',('TASK-0102',)).fetchone()
            self.assertEqual(row['status'],'APPROVED')
            self.assertEqual(row['approved_sha'],head)
        finally: con.close()

    # Gate C: MERGED is not allowed without the resulting merge commit SHA.
    def test_merged_without_merge_sha_is_refused(self):
        head='c'*40
        self.drive_to_approved('TASK-0103','src/',head)
        with self.assertRaises(RuntimeError) as cm: self.advance('TASK-0103','MERGED',note='merge ready')
        self.assertIn('MERGED requires --merge-sha',str(cm.exception))
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0103',)).fetchone()['status'],'APPROVED')
        finally: con.close()

    # Gate C: any head change after APPROVED must invalidate the approval.
    def test_head_change_invalidates_approval(self):
        head='d'*40; new='e'*40
        self.drive_to_approved('TASK-0104','src/',head)
        self.workspace('TASK-0104',head_sha=new)
        con=orch.connect(self.root)
        try:
            row=con.execute('SELECT status,approved_sha FROM tasks WHERE id=?',('TASK-0104',)).fetchone()
            self.assertEqual(row['status'],'MASTER_REVIEW')
            self.assertEqual(row['approved_sha'],'')
            hit=con.execute("SELECT COUNT(*) FROM events WHERE task_id=? AND action='APPROVAL_INVALIDATED'",('TASK-0104',)).fetchone()[0]
            self.assertEqual(hit,1)
        finally: con.close()

    # Gate C: VERIFIED cannot be reached without an audit note.
    def test_verified_requires_audit_note(self):
        head='f'*40
        self.drive_to_approved('TASK-0109','src/',head)
        self.advance('TASK-0109','MERGED',merge_sha='9'*40)
        class A: pass
        a=A(); a.id='TASK-0109'; a.status='VERIFIED'; a.head_sha=None; a.merge_sha=None; a.note=''
        with self.assertRaises(RuntimeError) as cm: orch.task_status(self.root,a)
        self.assertIn('VERIFIED requires an audit note',str(cm.exception))
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0109',)).fetchone()['status'],'MERGED')
        finally: con.close()

    # Gate C: an illegal transition is refused and the audit trail is left intact.
    def test_illegal_task_transition_is_refused_and_events_are_kept(self):
        head='1'*40
        self.drive_to_approved('TASK-0110','src/',head)
        self.advance('TASK-0110','MERGED',merge_sha='2'*40)
        before=self.count('SELECT COUNT(*) FROM events WHERE task_id=?',('TASK-0110',))
        class A: pass
        a=A(); a.id='TASK-0110'; a.status='MERGED'; a.head_sha=None; a.merge_sha='3'*40; a.note='illegal back'
        with self.assertRaises(RuntimeError) as cm: orch.task_status(self.root,a)
        self.assertIn('illegal transition MERGED -> MERGED',str(cm.exception))
        self.assertEqual(self.count('SELECT COUNT(*) FROM events WHERE task_id=?',('TASK-0110',)),before)
        con=orch.connect(self.root)
        try:
            row=con.execute('SELECT status,merged_sha FROM tasks WHERE id=?',('TASK-0110',)).fetchone()
            self.assertEqual(row['status'],'MERGED')
            self.assertEqual(row['merged_sha'],'2'*40)
            actions=[r['action'] for r in con.execute('SELECT action FROM events WHERE task_id=? ORDER BY id',('TASK-0110',))]
        finally: con.close()
        self.assertEqual(actions.count('TASK_CREATED'),1)
        self.assertEqual(actions.count('TASK_DISPATCHED'),1)
        self.assertEqual(actions.count('STATUS_CHANGED'),6)

    # Gate B: FAILED keeps the lock; only an explicit reasoned unlock releases it.
    def test_failed_keeps_lock_and_force_unlock(self):
        self.add('TASK-0105',['src/app.py']); orch.task_ready(self.root,'TASK-0105'); orch.task_dispatch(self.root,'TASK-0105')
        self.advance('TASK-0105','FAILED',note='simulated crash; state unknown')
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0105',)),1)
        class A: pass
        u=A(); u.id='TASK-0105'; u.reason='cleanup'; u.force=False
        with self.assertRaises(RuntimeError) as cm: orch.unlock(self.root,u)
        self.assertIn('use --force with a reason',str(cm.exception))
        u=A(); u.id='TASK-0105'; u.reason='execution confirmed stopped; releasing'; u.force=True
        orch.unlock(self.root,u)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0105',)),0)
        con=orch.connect(self.root)
        try:
            row=con.execute("SELECT data_json FROM events WHERE task_id=? AND action='MANUAL_UNLOCK'",('TASK-0105',)).fetchone()
            self.assertIsNotNone(row)
            self.assertTrue(json.loads(row['data_json'])['force'])
        finally: con.close()

    # Gate B: a multi-path task with any single conflict is refused entirely; no half locks.
    def test_multi_path_conflict_refuses_whole_dispatch(self):
        self.add('TASK-0106-A',['src/','docs/']); self.add('TASK-0106-B',['docs/x.md','tests/'])
        orch.task_ready(self.root,'TASK-0106-A'); orch.task_ready(self.root,'TASK-0106-B')
        orch.task_dispatch(self.root,'TASK-0106-A')
        with self.assertRaises(RuntimeError) as cm: orch.task_dispatch(self.root,'TASK-0106-B')
        self.assertIn('path conflict',str(cm.exception))
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0106-B',)),0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0106-A',)),2)

    # Gate A: task add rejects self dependency and unknown dependency without dirty rows.
    def test_task_add_rejects_self_and_missing_dependency(self):
        with self.assertRaises(ValueError) as cm: self.add('TASK-0107',['src/'],['TASK-0107'])
        self.assertIn('cannot depend on itself',str(cm.exception))
        with self.assertRaises(RuntimeError) as cm: self.add('TASK-0108',['src/'],['TASK-9999'])
        self.assertIn('dependency not found',str(cm.exception))
        self.assertEqual(self.count('SELECT COUNT(*) FROM tasks WHERE id=?',('TASK-0107',)),0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM tasks WHERE id=?',('TASK-0108',)),0)

    # --- review-fix retry cap configurable via <root>/.orchestration/config.json ---
    def test_retry_cap_is_configurable_from_config(self):
        self.add('TASK-0201',['cfg.py']); orch.task_ready(self.root,'TASK-0201'); orch.task_dispatch(self.root,'TASK-0201')
        (self.root/'.orchestration'/'config.json').write_text(json.dumps({'retry_cap':1}),encoding='utf-8')
        self.assertEqual(orch.read_retry_cap(self.root),1)
        self.advance('TASK-0201','IN_PROGRESS')
        self.advance('TASK-0201','REVIEW_READY'); self.advance('TASK-0201','MASTER_REVIEW')
        # With cap 1 the first MASTER_REVIEW -> IN_PROGRESS is allowed (retries 0 -> 1).
        self.advance('TASK-0201','IN_PROGRESS')
        self.assertEqual(self.count('SELECT retries FROM tasks WHERE id=?',('TASK-0201',)),1)
        self.advance('TASK-0201','REVIEW_READY'); self.advance('TASK-0201','MASTER_REVIEW')
        # The second round must hit the configured cap and divert to BLOCKED.
        self.advance('TASK-0201','IN_PROGRESS',note='round 2')
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0201',)).fetchone()['status'],'BLOCKED')
        finally: con.close()

    def test_retry_cap_defaults_for_missing_or_invalid_config(self):
        cfg=self.root/'.orchestration'/'config.json'
        self.assertEqual(orch.read_retry_cap(self.root),3)  # init wrote the default
        cfg.unlink()                                        # missing file
        self.assertEqual(orch.read_retry_cap(self.root),3)
        cfg.write_text(json.dumps({'other':5}),encoding='utf-8')  # key missing
        self.assertEqual(orch.read_retry_cap(self.root),3)
        for bad in [0,-1,'3',2.5,True,None]:
            cfg.write_text(json.dumps({'retry_cap':bad}),encoding='utf-8')
            self.assertEqual(orch.read_retry_cap(self.root),3,bad)
        cfg.write_text('{ not json',encoding='utf-8')       # malformed file
        self.assertEqual(orch.read_retry_cap(self.root),3)

    def test_retry_cap_missing_config_keeps_default_budget(self):
        (self.root/'.orchestration'/'config.json').unlink()
        self.add('TASK-0202',['cfg2.py']); orch.task_ready(self.root,'TASK-0202'); orch.task_dispatch(self.root,'TASK-0202')
        # 4 review rounds: the first IN_PROGRESS is DISPATCHED->IN_PROGRESS and does
        # not consume the retry budget, so retries reaches the default cap of 3 only
        # after the fourth IN_PROGRESS from MASTER_REVIEW.
        for dest in ['IN_PROGRESS','REVIEW_READY','MASTER_REVIEW']*4:
            self.advance('TASK-0202',dest)
        self.assertEqual(self.count('SELECT retries FROM tasks WHERE id=?',('TASK-0202',)),3)
        self.advance('TASK-0202','IN_PROGRESS')
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0202',)).fetchone()['status'],'BLOCKED')
        finally: con.close()

    # --- MERGED requires a full 40-hex merge commit SHA ---
    def test_merged_requires_full_40_hex_merge_sha(self):
        head='a'*40
        self.drive_to_approved('TASK-0301','one.py',head)
        with self.assertRaises(RuntimeError) as cm:
            self.advance('TASK-0301','MERGED',merge_sha='def456',note='merge ready')
        self.assertIn('40',str(cm.exception))
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0301',)),'')
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0301',)).fetchone()['status'],'APPROVED')
        finally: con.close()
        # A valid 40-hex value is accepted (mixed case normalised to lower case).
        self.advance('TASK-0301','MERGED',merge_sha='B'*40,note='merge ready')
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0301',)),'b'*40)

    # --- task amend corrects merge_sha and records a RECORD_AMENDED event ---
    def amend(self, tid, merge_sha=None, note=''):
        class A: pass
        a=A(); a.id=tid; a.merge_sha=merge_sha; a.note=note
        orch.task_amend(self.root,a)
    def merge(self, tid, path, head, merge_sha):
        self.drive_to_approved(tid,path,head)
        self.advance(tid,'MERGED',merge_sha=merge_sha,note='merge ready')
    def test_amend_corrects_merge_sha_and_records_event(self):
        old='a'*40; new='b'*40; third='c'*40
        self.merge('TASK-0302','two.py',old,old)
        self.amend('TASK-0302',merge_sha=new,note='backfill full SHA after short value')
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0302',)),new)
        # VERIFIED is also a valid amendment target.
        self.advance('TASK-0302','VERIFIED',note='verification complete')
        self.amend('TASK-0302',merge_sha=third,note='second correction')
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0302',)),third)
        con=orch.connect(self.root)
        try:
            rows=con.execute("SELECT note,data_json FROM events WHERE task_id=? AND action='RECORD_AMENDED' ORDER BY id",('TASK-0302',)).fetchall()
            self.assertEqual(len(rows),2)
            self.assertIn('backfill',rows[0]['note'])
            first=json.loads(rows[0]['data_json'])
            self.assertEqual(first['old'],old); self.assertEqual(first['new'],new)
            second=json.loads(rows[1]['data_json'])
            self.assertEqual(second['old'],new); self.assertEqual(second['new'],third)
        finally: con.close()
    def test_amend_rejects_non_terminal_missing_note_and_bad_sha(self):
        head='c'*40
        # A non-terminal task cannot be amended.
        self.drive_to_approved('TASK-0303','three.py',head)
        with self.assertRaises(RuntimeError) as cm:
            self.amend('TASK-0303',merge_sha='e'*40,note='too early')
        self.assertIn('MERGED',str(cm.exception))
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0303',)),'')
        # --note is mandatory.
        self.merge('TASK-0304','four.py',head,head)
        with self.assertRaises(RuntimeError) as cm:
            self.amend('TASK-0304',merge_sha='e'*40)
        self.assertIn('--note',str(cm.exception))
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0304',)),head)
        # The SHA must be a full 40-hex string.
        with self.assertRaises(RuntimeError) as cm:
            self.amend('TASK-0304',merge_sha='abc123',note='short')
        self.assertIn('40',str(cm.exception))
        with self.assertRaises(RuntimeError):
            self.amend('TASK-0304',merge_sha='z'*40,note='non-hex')
        self.assertEqual(self.count("SELECT merged_sha FROM tasks WHERE id=?",('TASK-0304',)),head)

    # --- task amend --paths: re-scope a non-active task and rewrite its locks ---
    AMEND_PATH_REFUSED_STATES=('PLANNED','READY','DISPATCHED','IN_PROGRESS','REVIEW_READY',
                               'MASTER_REVIEW','APPROVED','MERGED','VERIFIED','CANCELED')
    def drive_to_state(self, tid, dest, head='a'*40, merge='d'*40):
        """Drive a fresh task to `dest`; return the allowed path it was created with."""
        path=f'{tid.lower()}/file.py'
        self.add(tid,[path])
        if dest=='PLANNED': return path
        if dest=='CANCELED':
            self.advance(tid,'CANCELED',note='scope withdrawn'); return path
        orch.task_ready(self.root,tid)
        if dest=='READY': return path
        orch.task_dispatch(self.root,tid)
        chains={
            'DISPATCHED':[],
            'IN_PROGRESS':[('IN_PROGRESS',{})],
            'REVIEW_READY':[('IN_PROGRESS',{}),('REVIEW_READY',{'head_sha':head})],
            'MASTER_REVIEW':[('IN_PROGRESS',{}),('REVIEW_READY',{'head_sha':head}),
                             ('MASTER_REVIEW',{})],
            'APPROVED':[('IN_PROGRESS',{}),('REVIEW_READY',{'head_sha':head}),
                        ('MASTER_REVIEW',{}),('APPROVED',{'head_sha':head})],
            'MERGED':[('IN_PROGRESS',{}),('REVIEW_READY',{'head_sha':head}),
                      ('MASTER_REVIEW',{}),('APPROVED',{'head_sha':head}),
                      ('MERGED',{'merge_sha':merge})],
            'VERIFIED':[('IN_PROGRESS',{}),('REVIEW_READY',{'head_sha':head}),
                        ('MASTER_REVIEW',{}),('APPROVED',{'head_sha':head}),
                        ('MERGED',{'merge_sha':merge}),('VERIFIED',{})],
            'NEEDS_HUMAN_DECISION':[('NEEDS_HUMAN_DECISION',{})],
            'FAILED':[('FAILED',{})],
        }
        for step,kwargs in chains[dest]: self.advance(tid,step,**kwargs)
        return path
    def amend_paths(self, tid, paths, note=''):
        orch.task_amend(self.root,SimpleNamespace(id=tid,merge_sha=None,note=note,paths=paths))
    def snapshot(self, tid):
        """(paths_json verbatim, sorted lock paths) for one task."""
        con=orch.connect(self.root)
        try:
            paths_json=con.execute('SELECT paths_json FROM tasks WHERE id=?',(tid,)).fetchone()['paths_json']
            locks=[x['path'] for x in con.execute('SELECT path FROM locks WHERE task_id=? ORDER BY path',(tid,))]
        finally: con.close()
        return paths_json,locks
    def block_after_dispatch(self, tid, path):
        self.add(tid,[path]); orch.task_ready(self.root,tid); orch.task_dispatch(self.root,tid)
        self.advance(tid,'BLOCKED',note='scope under review: re-scope requested')

    # --- AC-1: every state outside BLOCKED / NEEDS_HUMAN_DECISION / FAILED is refused ---
    def test_amend_paths_is_refused_outside_the_three_recoverable_states(self):
        for index,dest in enumerate(self.AMEND_PATH_REFUSED_STATES,1):
            tid=f'TASK-06{index:02d}'
            with self.subTest(state=dest):
                path=self.drive_to_state(tid,dest)
                before=self.snapshot(tid)
                self.assertEqual(json.loads(before[0]),[path])
                with self.assertRaises(RuntimeError) as cm:
                    self.amend_paths(tid,[f'{tid.lower()}/new.py'],
                                     note='re-scope; authorised by the project owner')
                message=str(cm.exception)
                for needle in ('BLOCKED','NEEDS_HUMAN_DECISION','FAILED',dest):
                    self.assertIn(needle,message)
                self.assertEqual(self.snapshot(tid),before)
                self.assertEqual(self.count("SELECT COUNT(*) FROM events WHERE task_id=? "
                                            "AND action='RECORD_AMENDED'",(tid,)),0)

    # --- AC-2 / AC-6: success path rewrites paths + locks and stays auditable ---
    def test_amend_paths_on_blocked_task_rewrites_paths_locks_and_shows_before_after(self):
        self.block_after_dispatch('TASK-0620','old/dir')
        self.assertEqual(self.snapshot('TASK-0620'),(json.dumps(['old/dir']),['old/dir']))
        note='re-scope after block: split work; authorised by the project owner in Issue #100'
        with contextlib.redirect_stdout(io.StringIO()):
            self.amend_paths('TASK-0620',['new/a.py','new/b.py'],note=note)
        paths_json,locks=self.snapshot('TASK-0620')
        self.assertEqual(json.loads(paths_json),['new/a.py','new/b.py'])
        self.assertEqual(locks,['new/a.py','new/b.py'])   # exactly the new set, no leftovers
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0620',)),2)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE path=?',('old/dir',)),0)
        con=orch.connect(self.root)
        try:
            rows=con.execute("SELECT note,data_json FROM events WHERE task_id=? AND action='RECORD_AMENDED' "
                             'ORDER BY id',('TASK-0620',)).fetchall()
        finally: con.close()
        self.assertEqual(len(rows),1)
        self.assertEqual(rows[0]['note'],note)
        payload=json.loads(rows[0]['data_json'])
        self.assertEqual(payload['field'],'paths')
        self.assertEqual(payload['before'],['old/dir'])
        self.assertEqual(payload['after'],['new/a.py','new/b.py'])
        self.assertEqual(payload['reason'],note)
        # AC-6: the before/after summary is visible to a human in `task show`.
        out=io.StringIO()
        with contextlib.redirect_stdout(out): orch.task_show(self.root,'TASK-0620')
        shown=out.getvalue()
        self.assertIn('[paths: old/dir -> new/a.py, new/b.py]',shown)
        self.assertIn('Allowed paths:\n- new/a.py\n- new/b.py',shown)
        # Shrinking uses the same channel and deletes the dropped lock row.
        with contextlib.redirect_stdout(io.StringIO()):
            self.amend_paths('TASK-0620',['new/b.py'],
                             note='narrow scope: new/a.py moved to another task')
        self.assertEqual(self.snapshot('TASK-0620'),(json.dumps(['new/b.py']),['new/b.py']))
        # The amended registry is what `task ready` now checks against.
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root,'TASK-0620')
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0620',)),1)
        self.assertEqual(self.count('SELECT status FROM tasks WHERE id=?',('TASK-0620',)),'READY')

    # --- BLOCKED task that never held a lock gains exactly the new set ---
    def test_amend_paths_creates_the_lock_set_for_a_blocked_task_that_held_none(self):
        self.add('TASK-0623',['preblock'])
        self.advance('TASK-0623','BLOCKED',note='blocked before dispatch')
        self.assertEqual(self.snapshot('TASK-0623'),(json.dumps(['preblock']),[]))
        with contextlib.redirect_stdout(io.StringIO()):
            self.amend_paths('TASK-0623',['after'],note='re-scope while blocked without locks')
        self.assertEqual(self.snapshot('TASK-0623'),(json.dumps(['after']),['after']))

    # --- AC-3: a conflicting new path refuses the whole rewrite ---
    def test_amend_paths_conflicting_with_another_active_task_is_refused_atomically(self):
        self.block_after_dispatch('TASK-0621','holder/')
        self.block_after_dispatch('TASK-0622','mover/')
        before_holder=self.snapshot('TASK-0621'); before_mover=self.snapshot('TASK-0622')
        with self.assertRaises(RuntimeError) as cm:
            self.amend_paths('TASK-0622',['mover/','holder/child.py'],
                             note='would overlap the holder lock')
        message=str(cm.exception)
        self.assertIn('path conflict',message)
        self.assertIn('holder/child.py',message)
        self.assertIn('TASK-0621',message)
        self.assertEqual(self.snapshot('TASK-0621'),before_holder)
        self.assertEqual(self.snapshot('TASK-0622'),before_mover)
        self.assertEqual(self.count("SELECT COUNT(*) FROM events WHERE task_id=? "
                                    "AND action='RECORD_AMENDED'",('TASK-0622',)),0)

    # --- AC-4: NEEDS_HUMAN_DECISION / FAILED behave like BLOCKED ---
    def test_amend_paths_is_allowed_on_needs_human_decision_and_failed(self):
        for tid,dest in (('TASK-0630','NEEDS_HUMAN_DECISION'),('TASK-0631','FAILED')):
            with self.subTest(state=dest):
                self.drive_to_state(tid,dest)
                wanted=[f'{tid.lower()}/rescoped.py']
                with contextlib.redirect_stdout(io.StringIO()):
                    self.amend_paths(tid,wanted,note=f're-scope while {dest}')
                self.assertEqual(self.snapshot(tid),(json.dumps(wanted),wanted))
                self.assertEqual(self.count('SELECT status FROM tasks WHERE id=?',(tid,)),dest)

    # --- AC-5: the merge-sha channel keeps its success and error behaviour ---
    def test_merge_sha_amend_channel_keeps_its_behaviour(self):
        old='a'*40; new='b'*40
        self.merge('TASK-0640','file640.py',old,old)
        self.amend('TASK-0640',merge_sha=new,note='backfill full SHA')
        self.assertEqual(self.count('SELECT merged_sha FROM tasks WHERE id=?',('TASK-0640',)),new)
        con=orch.connect(self.root)
        try:
            row=con.execute("SELECT data_json FROM events WHERE task_id=? AND action='RECORD_AMENDED'",
                            ('TASK-0640',)).fetchone()
        finally: con.close()
        payload=json.loads(row['data_json'])
        self.assertEqual((payload['field'],payload['old'],payload['new']),('merged_sha',old,new))
        with self.assertRaises(RuntimeError) as cm: self.amend('TASK-0640',merge_sha='c'*40)
        self.assertIn('--note',str(cm.exception))
        with self.assertRaises(RuntimeError) as cm: self.amend('TASK-0640',merge_sha='abc123',note='short')
        self.assertIn('40',str(cm.exception))
        with self.assertRaises(RuntimeError) as cm: self.amend('TASK-0640',merge_sha='z'*40,note='non-hex')
        self.assertIn('40',str(cm.exception))
        self.assertEqual(self.count('SELECT merged_sha FROM tasks WHERE id=?',('TASK-0640',)),new)
        # Illegal-state text is unchanged for the merge-sha channel.
        self.add('TASK-0641',['file641.py'])
        with self.assertRaises(RuntimeError) as cm: self.amend('TASK-0641',merge_sha='e'*40,note='too early')
        self.assertEqual(str(cm.exception),
                         'merge SHA can only be amended on MERGED or VERIFIED tasks; current status is PLANNED')
        # The two modes are mutually exclusive; refusing both leaves the row untouched.
        before=self.snapshot('TASK-0641')
        with self.assertRaises(RuntimeError) as cm:
            orch.task_amend(self.root,SimpleNamespace(id='TASK-0641',merge_sha='e'*40,
                                                      note='ambiguous',paths=['file641.py']))
        self.assertIn('not both',str(cm.exception))
        self.assertEqual(self.snapshot('TASK-0641'),before)
    def test_cli_amend_without_merge_sha_keeps_the_missing_argument_error(self):
        self.add('TASK-0642',['file642.py'])
        result=subprocess.run([sys.executable,str(SCRIPT),'--root',str(self.root),
                               'task','amend','TASK-0642'],
                              capture_output=True,text=True)
        self.assertEqual(result.returncode,2,result.stderr)
        self.assertIn('the following arguments are required: --merge-sha',result.stderr)
        self.assertEqual(self.count("SELECT COUNT(*) FROM events WHERE task_id=?",('TASK-0642',)),1)

    # --- AC-7: a real two-process race cannot leave two owners on one path ---
    def test_concurrent_amend_paths_cannot_leave_two_tasks_holding_one_path(self):
        self.block_after_dispatch('TASK-0650','race0600/one/')
        self.block_after_dispatch('TASK-0651','race0600/two/')
        worker=self.root/'amend_paths_worker.py'
        worker.write_text(AMEND_PATHS_WORKER,encoding='utf-8')
        start=time.time()+1.2
        processes=[
            subprocess.Popen([sys.executable,str(worker),str(SCRIPT),str(self.root),tid,
                              'race0600/shared/x.py',str(start)],
                             stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            for tid in ('TASK-0650','TASK-0651')
        ]
        results=[]
        for process in processes:
            stdout,stderr=process.communicate(timeout=120)
            self.assertEqual(process.returncode,0,stderr)
            results.append(json.loads(stdout.strip().splitlines()[-1]))
        self.assertEqual(sorted(r['status'] for r in results),['AMENDED','REFUSED'],results)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE path=?',
                                    ('race0600/shared/x.py',)),1)
        con=orch.connect(self.root)
        try:
            owners=[(r['task_id'],r['path']) for r in
                    con.execute('SELECT task_id,path FROM locks ORDER BY task_id,path')]
            tasks={r['id']:json.loads(r['paths_json'])
                   for r in con.execute('SELECT id,paths_json FROM tasks')}
        finally: con.close()
        # Every task's locks are exactly its declared paths...
        for tid in ('TASK-0650','TASK-0651'):
            self.assertEqual(sorted(p for t,p in owners if t==tid),sorted(tasks[tid]))
        # ...and no two tasks hold overlapping paths.
        for left in owners:
            for right in owners:
                if left[0]!=right[0]:
                    self.assertFalse(orch.overlaps(left[1],right[1]),f'{left} overlaps {right}')
        amended=[r for r in results if r['status']=='AMENDED'][0]
        refused=[r for r in results if r['status']=='REFUSED'][0]
        self.assertEqual(tasks[amended['task']],['race0600/shared/x.py'])
        self.assertNotIn('race0600/shared/x.py',tasks[refused['task']])
        self.assertIn('path conflict',refused['error'])

    # --- a failing BEGIN must not be masked by a spurious ROLLBACK ---
    def test_begin_failure_propagates_original_error(self):
        real=orch.connect(self.root)
        class BeginFails:
            def __init__(self,con): self._c=con
            @property
            def in_transaction(self): return self._c.in_transaction
            def execute(self,sql,*a,**k):
                if sql.strip().upper().startswith('BEGIN'):
                    raise sqlite3.OperationalError('database is locked')
                return self._c.execute(sql,*a,**k)
            def close(self): self._c.close()
            def __getattr__(self,name): return getattr(self._c,name)
        self.add('TASK-0401',['lock.py']); orch.task_ready(self.root,'TASK-0401'); orch.task_dispatch(self.root,'TASK-0401')
        with mock.patch.object(orch,'connect',return_value=BeginFails(real)):
            with self.assertRaises(sqlite3.OperationalError) as cm:
                self.advance('TASK-0401','IN_PROGRESS')
        msg=str(cm.exception)
        self.assertIn('database is locked',msg)
        self.assertNotIn('cannot rollback',msg)

    # --- effective runtime config and `config show` provenance ---
    def test_config_defaults_when_file_missing(self):
        cfg=self.root/'.orchestration'/'config.json'
        cfg.unlink()
        conf=orch.read_config(self.root)
        self.assertEqual((conf['retry_cap'],conf['local_dir']),(3,'.local'))
        for key in ('retry_cap','local_dir'):
            self.assertEqual(conf['sources'][key],'default',key)
        # The edition never comes from config.json: it is the project declaration.
        self.assertEqual(conf['edition'],'local')
        self.assertEqual(conf['sources']['edition'],'docs/PROJECT.md')
        # A root with no project file has no edition at all; it must not default to local.
        with tempfile.TemporaryDirectory() as other:
            with self.assertRaises(orch.EditionError) as cm: orch.read_config(Path(other))
            self.assertIn('docs/PROJECT.md',str(cm.exception))
        # init writes the runtime defaults only; the edition stays out of this file.
        orch.init(self.root)
        self.assertTrue(cfg.exists())
        self.assertEqual(json.loads(cfg.read_text(encoding='utf-8')),{'retry_cap':3,'local_dir':'.local'})
        conf=orch.read_config(self.root)
        self.assertEqual((conf['retry_cap'],conf['edition'],conf['local_dir']),(3,'local','.local'))

    def test_runtime_keys_come_from_config_json_but_edition_does_not(self):
        (self.root/'.orchestration'/'config.json').write_text(
            json.dumps({'retry_cap':1,'edition':'github','local_dir':'priv'}),encoding='utf-8')
        conf=orch.read_config(self.root)
        self.assertEqual(conf['retry_cap'],1)
        self.assertEqual(conf['local_dir'],'priv')
        for key in ('retry_cap','local_dir'):
            self.assertEqual(conf['sources'][key],'config.json',key)
        # A leftover `edition` key in config.json is a record, not a routing input:
        # the project declaration wins and no second definition is honoured.
        self.assertEqual(conf['edition'],'local')
        self.assertEqual(conf['sources']['edition'],'docs/PROJECT.md')
        self.assertEqual(orch.read_retry_cap(self.root),1)
        # Illegal runtime values fall back to their own default, not to the whole file.
        (self.root/'.orchestration'/'config.json').write_text(
            json.dumps({'retry_cap':0,'local_dir':''}),encoding='utf-8')
        conf=orch.read_config(self.root)
        self.assertEqual((conf['retry_cap'],conf['local_dir']),(3,'.local'))
        self.assertEqual(conf['sources']['retry_cap'],'default')

    def test_config_show_runs(self):
        out=io.StringIO()
        with contextlib.redirect_stdout(out): rc=orch.config_show(self.root)
        text=out.getvalue()
        self.assertEqual(rc,0)
        for key in ('retry_cap','local_dir','edition','config.json','docs/PROJECT.md'):
            self.assertIn(key,text)
        (self.root/'.orchestration'/'config.json').write_text(
            json.dumps({'retry_cap':2,'edition':'github','local_dir':'priv'}),encoding='utf-8')
        out=io.StringIO()
        with contextlib.redirect_stdout(out): rc=orch.config_show(self.root)
        text=out.getvalue()
        self.assertEqual(rc,0)
        self.assertIn('retry_cap  = 2',text)
        self.assertIn('local_dir  = priv',text)
        self.assertIn('source: config.json',text)
        # The printed edition is the project declaration, not the legacy config.json key.
        self.assertIn('edition    = local',text)
        self.assertIn('source: docs/PROJECT.md',text)
        self.assertIn('legacy key is ignored',text)

class PathConflictEventTests(unittest.TestCase):
    """A refused path check leaves an audit event — and still no lock row.

    `task dispatch` records `CONFLICT_RECORDED` (`data.kind=path`) inside the
    same transaction and commits it before re-raising, which is the event M4
    reads. `task ready` only preflights, so it records its own
    `PATH_PREFLIGHT_REFUSED` and must never look like a dispatch conflict in
    M4. The refusal text, the exit code and the task status stay byte for byte
    what they were before the events existed.
    """

    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.root=Path(self.tmp.name)
        (self.root/'docs').mkdir(parents=True)
        (self.root/'docs/PROJECT.md').write_text(
            '# Project\n\n## 运行配置\n\n- `edition`：`local`\n',encoding='utf-8')
        orch.init(self.root)

    def tearDown(self):
        try: self.tmp.cleanup()
        except PermissionError:                     # Windows may still hold the WAL file
            pass

    def add(self,tid,paths):
        with contextlib.redirect_stdout(io.StringIO()):
            orch.task_add(self.root,SimpleNamespace(id=tid,title=tid,description='',
                                                    paths=list(paths),depends=[],acceptance=['works']))

    def ready(self,tid):
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root,tid)

    def dispatch(self,tid):
        with contextlib.redirect_stdout(io.StringIO()): orch.task_dispatch(self.root,tid)

    def count(self,sql,params=()):
        con=orch.connect(self.root)
        try: return con.execute(sql,params).fetchone()[0]
        finally: con.close()

    def status(self,tid):
        con=orch.connect(self.root)
        try: return con.execute('SELECT status FROM tasks WHERE id=?',(tid,)).fetchone()['status']
        finally: con.close()

    def events(self,tid,action=None):
        con=orch.connect(self.root)
        try:
            rows=[dict(r) for r in con.execute('SELECT * FROM events WHERE task_id=? ORDER BY id',(tid,))]
        finally: con.close()
        parsed=[dict(row,data=json.loads(row['data_json'] or '{}')) for row in rows]
        return [row for row in parsed if action is None or row['action']==action]

    def hold_src(self):
        """TASK-HOLDER owns `src/`; TASK-SUBJECT declares a child path."""
        self.add('TASK-HOLDER',['src/']); self.add('TASK-SUBJECT',['src/main.py'])
        self.ready('TASK-SUBJECT'); self.ready('TASK-HOLDER'); self.dispatch('TASK-HOLDER')

    # --- AC-1: the refused dispatch is auditable and inserted no lock ----------
    def test_dispatch_conflict_writes_conflict_recorded_and_no_locks(self):
        self.hold_src()
        with self.assertRaises(RuntimeError) as cm: self.dispatch('TASK-SUBJECT')
        self.assertEqual(str(cm.exception),
                         'dispatch refused; path conflict: src/main.py overlaps src (task TASK-HOLDER)')
        recorded=self.events('TASK-SUBJECT','CONFLICT_RECORDED')
        self.assertEqual(len(recorded),1)
        self.assertEqual(recorded[0]['task_id'],'TASK-SUBJECT')
        data=recorded[0]['data']
        self.assertEqual(data['kind'],'path')
        self.assertEqual(data['phase'],'dispatch')
        # The payload locates both sides of the conflict, not just its kind.
        self.assertEqual(data['conflicts'],
                         [{'path':'src/main.py','held_path':'src','holder_task':'TASK-HOLDER'}])
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-SUBJECT',)),0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks'),1)   # only the holder's lock
        self.assertEqual(self.status('TASK-SUBJECT'),'READY')          # not DISPATCHED

    # --- AC-2: the observable refusal is unchanged; the event is the only delta
    def test_cli_dispatch_conflict_keeps_exit_code_and_error_text(self):
        self.hold_src()
        result=subprocess.run([sys.executable,str(SCRIPT),'--root',str(self.root),
                               'task','dispatch','TASK-SUBJECT'],
                              capture_output=True,text=True)
        self.assertEqual(result.returncode,1)
        self.assertEqual(result.stderr,
                         'ERROR: dispatch refused; path conflict: src/main.py overlaps src '
                         '(task TASK-HOLDER)\n')
        self.assertEqual(self.status('TASK-SUBJECT'),'READY')
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-SUBJECT',)),0)
        self.assertEqual(len(self.events('TASK-SUBJECT','CONFLICT_RECORDED')),1)

    # --- AC-3: the ready preflight refusal is auditable under its own name -----
    def test_ready_preflight_conflict_is_audited_with_its_own_event(self):
        self.add('TASK-HOLDER',['src/']); self.add('TASK-SUBJECT',['src/main.py'])
        self.ready('TASK-HOLDER'); self.dispatch('TASK-HOLDER')
        with self.assertRaises(RuntimeError) as cm: self.ready('TASK-SUBJECT')
        self.assertEqual(str(cm.exception),
                         'path conflict: src/main.py overlaps src, held by TASK-HOLDER')
        refused=self.events('TASK-SUBJECT','PATH_PREFLIGHT_REFUSED')
        self.assertEqual(len(refused),1)
        self.assertEqual(refused[0]['data']['kind'],'path')
        self.assertEqual(refused[0]['data']['conflicts'],
                         [{'path':'src/main.py','held_path':'src','holder_task':'TASK-HOLDER'}])
        # It is a preflight record, not a dispatch conflict: M4 must not see it.
        self.assertEqual(self.events('TASK-SUBJECT','CONFLICT_RECORDED'),[])
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-SUBJECT',)),0)
        self.assertEqual(self.status('TASK-SUBJECT'),'PLANNED')        # never became READY

class ProjectEditionTests(unittest.TestCase):
    """The project declaration is the single edition authority and fails closed."""
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.root=Path(self.tmp.name)
    def tearDown(self): self.tmp.cleanup()
    def project(self, body):
        (self.root/'docs').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs').mkdir(parents=True, exist_ok=True)
        (self.root/'docs/PROJECT.md').write_text(f'# Project\n\n## 运行配置\n\n{body}\n',encoding='utf-8')
    def config(self, payload):
        target=self.root/'.orchestration'; target.mkdir(exist_ok=True)
        (target/'config.json').write_text(payload,encoding='utf-8')

    def test_project_file_is_the_only_edition_authority(self):
        self.project('- `edition`：`local`')
        self.config(json.dumps({'edition':'github','retry_cap':2}))
        conf=orch.read_config(self.root)
        self.assertEqual(conf['edition'],'local')
        self.assertEqual(conf['sources']['edition'],'docs/PROJECT.md')
        self.assertEqual(conf['retry_cap'],2)
        self.project('- `edition`：`github`')
        self.assertEqual(orch.read_config(self.root)['edition'],'github')

    def test_missing_project_file_blocks(self):
        with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
        msg=str(cm.exception)
        self.assertIn('docs/PROJECT.md',msg)
        self.assertIn('file not found',msg)
        self.assertIn(str(self.root),msg)

    def test_missing_edition_key_blocks(self):
        self.project('- `local_dir`：`.local`')
        with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
        msg=str(cm.exception)
        self.assertIn('missing required key `edition`',msg)
        self.assertIn(str(self.root/'docs/PROJECT.md'),msg)

    def test_illegal_edition_strings_block(self):
        for bad in ('GITHUB','cloud','local edition','local|github'):
            with self.subTest(value=bad):
                self.project(f'- `edition`：`{bad}`')
                with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
                msg=str(cm.exception)
                self.assertIn(bad,msg)
                self.assertIn('allowed values are local, github',msg)
                self.assertIn(str(self.root/'docs/PROJECT.md'),msg)

    def test_wrong_type_edition_blocks(self):
        for bad in ('1','true','"local"','[local]'):
            with self.subTest(value=bad):
                self.project(f'- `edition`：{bad}')
                with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
                msg=str(cm.exception)
                self.assertIn(bad,msg)
                self.assertIn('unsupported value',msg)

    def test_empty_edition_value_blocks(self):
        self.project('- `edition`：   ')
        with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
        self.assertIn('is empty',str(cm.exception))

    def test_duplicate_edition_declarations_block(self):
        self.project('- `edition`：`local`\n- `edition`：`github`')
        with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
        self.assertIn('declared 2 times',str(cm.exception))

    def test_unreadable_project_file_blocks(self):
        (self.root/'docs').mkdir(parents=True, exist_ok=True)
        (self.root/'docs/PROJECT.md').write_bytes(b'- `edition`: \xff\xfe local\n')
        with self.assertRaises(orch.EditionError) as cm: orch.read_project_edition(self.root)
        self.assertIn('cannot read',str(cm.exception))

    def test_github_declaration_refuses_task_add_with_guidance(self):
        self.project('- `edition`：`github`')
        orch.init(self.root)
        a=SimpleNamespace(id='TASK-0501',title='t',description='',paths=['a.py'],depends=[],acceptance=['works'])
        with self.assertRaises(orch.EditionError) as cm: orch.task_add(self.root,a)
        msg=str(cm.exception)
        self.assertIn("'github'",msg)
        self.assertIn('read-only archive',msg)
        self.assertIn('issue-template.md',msg)
        self.assertIn('execution.md',msg)
        con=orch.connect(self.root)
        try: self.assertEqual(con.execute('SELECT COUNT(*) FROM tasks').fetchone()[0],0)
        finally: con.close()

    def test_every_scheduling_command_is_refused_under_github(self):
        self.project('- `edition`：`local`'); orch.init(self.root)
        a=SimpleNamespace(id='TASK-0502',title='t',description='',paths=['a.py'],depends=[],acceptance=['works'])
        orch.task_add(self.root,a); orch.task_ready(self.root,'TASK-0502'); orch.task_dispatch(self.root,'TASK-0502')
        self.project('- `edition`：`github`')
        cases=[
            ('task ready',lambda: orch.task_ready(self.root,'TASK-0502')),
            ('task dispatch',lambda: orch.task_dispatch(self.root,'TASK-0502')),
            ('task status',lambda: orch.task_status(self.root,SimpleNamespace(id='TASK-0502',status='IN_PROGRESS',head_sha=None,merge_sha=None,note=''))),
            ('task workspace',lambda: orch.set_workspace(self.root,SimpleNamespace(id='TASK-0502',head_sha='',branch='',worktree='',note=''))),
            ('task amend',lambda: orch.task_amend(self.root,SimpleNamespace(id='TASK-0502',merge_sha='a'*40,note='x'))),
            ('task unlock',lambda: orch.unlock(self.root,SimpleNamespace(id='TASK-0502',reason='x',force=False))),
        ]
        for label,call in cases:
            with self.subTest(command=label):
                with self.assertRaises(orch.EditionError) as cm: call()
                self.assertIn(f'`{label}`',str(cm.exception))
        # Refusals happen before any write: the archived registry is unchanged.
        con=orch.connect(self.root)
        try:
            self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?',('TASK-0502',)).fetchone()['status'],'DISPATCHED')
            self.assertEqual(con.execute('SELECT COUNT(*) FROM locks WHERE task_id=?',('TASK-0502',)).fetchone()[0],1)
        finally: con.close()

    def test_readonly_and_maintenance_commands_stay_available_under_github(self):
        self.project('- `edition`：`local`'); orch.init(self.root)
        a=SimpleNamespace(id='TASK-0503',title='archived',description='',paths=['a.py'],depends=[],acceptance=['works'])
        orch.task_add(self.root,a)
        self.project('- `edition`：`github`')
        out=io.StringIO()
        with contextlib.redirect_stdout(out): rc=orch.config_show(self.root)
        self.assertEqual(rc,0)
        text=out.getvalue()
        self.assertIn('edition    = github',text)
        self.assertIn('source: docs/PROJECT.md',text)
        with contextlib.redirect_stdout(io.StringIO()): orch.task_list(self.root,SimpleNamespace(active_only=False))
        with contextlib.redirect_stdout(io.StringIO()): orch.task_show(self.root,'TASK-0503')
        with contextlib.redirect_stdout(io.StringIO()): orch.backup(self.root)
        self.assertTrue(list((self.root/'.orchestration'/'backups').glob('*.sqlite3')))

    def test_config_show_reports_invalid_edition_and_returns_nonzero(self):
        self.project('- `edition`：`cloud`')
        out,err=io.StringIO(),io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err): rc=orch.config_show(self.root)
        self.assertEqual(rc,1)
        self.assertIn('(invalid)',out.getvalue())
        self.assertIn('retry_cap',out.getvalue())
        self.assertIn('cloud',err.getvalue())
        self.assertIn('docs/PROJECT.md',err.getvalue())

    def test_retry_cap_is_never_blocked_by_an_edition_problem(self):
        self.project('- `edition`：`cloud`')
        self.config('{ not json')
        self.assertEqual(orch.read_retry_cap(self.root),3)
        self.config(json.dumps({'retry_cap':5,'edition':'local'}))
        self.assertEqual(orch.read_retry_cap(self.root),5)

class PlanDocumentInitTests(unittest.TestCase):
    """`init` seeds docs/PLAN.md once; docs/PLAN.md stays the plan of record.

    A legacy root PLAN.md (<=1.3) is never touched and blocks seeding, so the
    design goals always keep a single authority.
    """
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.root=Path(self.tmp.name)
        (self.root/'docs').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs').mkdir(parents=True, exist_ok=True)
        (self.root/'docs/PROJECT.md').write_text('# Project\n\n- `edition`：`local`\n',encoding='utf-8')
    def tearDown(self): self.tmp.cleanup()
    def write_template(self, data):
        target=self.root/'templates'; target.mkdir(exist_ok=True)
        (target/'plan.md').write_bytes(data)
    def run_init(self):
        out,err=io.StringIO(),io.StringIO()
        with mock.patch.object(sys,'argv',['orchestrator.py','--root',str(self.root),'init']), \
             contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc=orch.main()
        return rc,out.getvalue(),err.getvalue()

    def test_init_creates_plan_byte_for_byte_from_template(self):
        # CRLF + non-ASCII bytes prove the copy is a byte copy, not a re-render.
        data='# 计划\r\n\r\n- **Plan name:** 演示\r\n'.encode('utf-8')
        self.write_template(data)
        rc,out,err=self.run_init()
        self.assertEqual(rc,0,err)
        self.assertEqual((self.root/'docs'/'PLAN.md').read_bytes(),data)
        self.assertEqual((self.root/'templates'/'plan.md').read_bytes(),data)
        self.assertIn('Created docs/PLAN.md',out)
        self.assertIn('下一步：填写 docs/PLAN.md 的设计目标（为什么做、做到什么程度、原则与边界）',out)

    def test_existing_docs_plan_is_kept_and_repeated_init_is_idempotent(self):
        self.write_template(b'# Implementation Plan\n\ntemplate\n')
        sentinel=b'# my plan\n\nSENTINEL: owner content, never overwrite\n'
        (self.root/'docs').mkdir(parents=True, exist_ok=True); (self.root/'docs'/'PLAN.md').write_bytes(sentinel)
        config=self.root/'.orchestration'/'config.json'
        rc,out,err=self.run_init(); self.assertEqual(rc,0,err)
        self.assertIn('kept unchanged',out)
        self.assertIn('judge task direction from the design goals',out)
        self.assertNotIn('Created docs/PLAN.md',out)
        first=(self.root/'docs'/'PLAN.md').read_bytes(), config.read_bytes()
        rc,out,err=self.run_init(); self.assertEqual(rc,0,err)
        self.assertEqual((self.root/'docs'/'PLAN.md').read_bytes(),sentinel)
        self.assertEqual((self.root/'docs'/'PLAN.md').read_bytes(),first[0])
        self.assertEqual(config.read_bytes(),first[1])          # runtime config untouched
        self.assertTrue(orch.db_path(self.root).is_file())      # task store still initialised

    def test_legacy_root_plan_blocks_seeding_and_is_never_touched(self):
        # A root PLAN.md is the <=1.3 location. init must not move it, must not
        # seed docs/PLAN.md next to it, and must point at the migration step.
        self.write_template(b'# Implementation Plan\n\ntemplate\n')
        sentinel=b'# legacy design goals\n\nSENTINEL: never auto-migrated\n'
        (self.root/'PLAN.md').write_bytes(sentinel)
        rc,out,err=self.run_init(); self.assertEqual(rc,0,err)
        self.assertFalse((self.root/'docs'/'PLAN.md').exists())
        self.assertEqual((self.root/'PLAN.md').read_bytes(),sentinel)
        self.assertIn('Legacy root PLAN.md found',out)
        self.assertIn('git mv PLAN.md docs/PLAN.md',out)
        self.assertNotIn('Created docs/PLAN.md',out)
        # Rerunning stays deterministic: no second authority appears.
        rc,out,err=self.run_init(); self.assertEqual(rc,0,err)
        self.assertFalse((self.root/'docs'/'PLAN.md').exists())

    def test_missing_template_is_reported_and_init_still_succeeds(self):
        rc,out,err=self.run_init()
        self.assertEqual(rc,0,err)
        self.assertFalse((self.root/'docs'/'PLAN.md').exists())
        self.assertFalse((self.root/'docs'/'PLAN.md').exists())   # no PLAN is fabricated
        self.assertIn('PLAN.md not created: template missing',out)
        self.assertTrue(orch.db_path(self.root).is_file())

    def test_unwritable_plan_is_an_error_not_a_silent_success(self):
        self.write_template(b'# Implementation Plan\n\ntemplate\n')
        with mock.patch.object(Path,'write_bytes',side_effect=OSError('read-only file system')):
            with self.assertRaises(OSError):
                orch.init(self.root)
            rc,out,err=self.run_init()
        self.assertEqual(rc,1)
        self.assertIn('ERROR:',err)
        self.assertIn('read-only file system',err)
        self.assertFalse((self.root/'docs'/'PLAN.md').exists())

# Real multi-process competitor: it waits for a shared start time, then opens its
# own SQLite connection and races the same resource set. It never retries an
# *acquisition*, so an outcome is either a grant or a recorded denial. It does
# reopen the connection when Windows refuses the first write attempt on a WAL
# database ("attempt to write a readonly database"), which is a per-connection
# artefact of two processes creating the -shm file at the same instant, not a
# lock decision.
CONCURRENCY_WORKER = r'''
import importlib.util, json, sys, time
from pathlib import Path
spec = importlib.util.spec_from_file_location('orch_worker', sys.argv[1])
orch = importlib.util.module_from_spec(spec); spec.loader.exec_module(orch)
root, task, holder, declaration, start = Path(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5], float(sys.argv[6])
while time.time() < start:
    time.sleep(0.005)
out = {'task': task, 'holder': holder, 'attempts': 0}
con = None
try:
    for attempt in range(1, 41):
        if con is not None:
            con.close()
        con = orch.connect(root)
        out['attempts'] = attempt
        try:
            con.execute('BEGIN IMMEDIATE')
            break
        except Exception as exc:
            if 'readonly' not in str(exc) and 'locked' not in str(exc):
                raise
            time.sleep(0.05)
    else:
        raise RuntimeError('no writable connection after 40 attempts')
    entries = orch.resolve_resource_entries(root, [declaration])
    result = orch.acquire_resource_set(con, task, entries, holder)
    con.execute('COMMIT')
    out['status'] = result['status']; out['lock_ids'] = result['lock_ids']
except orch.ResourceDenied as exc:
    con.execute('COMMIT'); out['status'] = 'DENIED'; out['code'] = exc.code
except Exception as exc:
    out['status'] = 'ERROR'; out['error'] = f'{type(exc).__name__}: {exc}'
finally:
    if con is not None:
        con.close()
print(json.dumps(out))
'''


class ResourceIdTests(unittest.TestCase):
    """§2.1: canonical form, hard rejections, exact matching only."""

    def test_accepts_canonical_ids(self):
        for raw in ('db:test/mysql-main', 'env:build/windows-01', 'svc:api', 'vol:data.1/0'):
            with self.subTest(raw=raw):
                self.assertEqual(orch.normalize_resource_id(raw), raw)

    def test_rejects_uppercase_space_and_wildcards_without_rewriting(self):
        for bad in ('DB:test/main', 'db:Test/main', ' db:test/main', 'db:test/main ',
                    'db:test main', 'db:test\tmain', 'db:test/*', 'db:test?x', 'db:test#1',
                    'db:test\\main'):
            with self.subTest(raw=bad):
                with self.assertRaises(orch.ResourceError) as cm: orch.normalize_resource_id(bad)
                self.assertEqual(cm.exception.code, 'RESOURCE_ID_INVALID')

    def test_rejects_empty_segments_and_parent_segments(self):
        for bad in ('db:test/../main', 'db:test//main', 'db:/main', 'db:test/', 'db:..', 'db:.',
                    'db:', ':name', 'db', '', 'db:test/main/./x'):
            with self.subTest(raw=bad):
                with self.assertRaises(orch.ResourceError) as cm: orch.normalize_resource_id(bad)
                self.assertEqual(cm.exception.code, 'RESOURCE_ID_INVALID')

    def test_unicode_is_nfc_normalized_before_validation(self):
        # NFC normalisation runs before the ASCII-only pattern is applied, so both
        # spellings of the same visual ID are rejected identically instead of one
        # of them slipping through as a "different" resource.
        composed, decomposed = 'db:caf\u00e9', 'db:cafe\u0301'
        for raw in (composed, decomposed):
            with self.subTest(raw=raw):
                with self.assertRaises(orch.ResourceError) as cm: orch.normalize_resource_id(raw)
                self.assertEqual(cm.exception.code, 'RESOURCE_ID_INVALID')
        self.assertEqual(orch.normalize_resource_id('db:cafe'), 'db:cafe')

    def test_sibling_names_in_one_namespace_do_not_conflict(self):
        self.assertNotEqual(orch.normalize_resource_id('db:test/a'), orch.normalize_resource_id('db:test/b'))
        # No prefix derivation: a parent name is a different resource, not a group.
        self.assertNotEqual(orch.normalize_resource_id('db:test'), orch.normalize_resource_id('db:test/a'))

    def test_spec_parsing_keeps_the_id_colons_and_rejects_bad_modes(self):
        entry = orch.parse_resource_spec('db:test/main:shared:120')
        self.assertEqual((entry['resource_id'], entry['mode'], entry['ttl']), ('db:test/main', 'shared', 120))
        entry = orch.parse_resource_spec('db:test/main:exclusive')
        self.assertEqual((entry['mode'], entry['ttl']), ('exclusive', None))
        for bad in ('db:test/main', 'db:test/main:write', 'db:test/main:shared:0', ''):
            with self.subTest(spec=bad):
                with self.assertRaises(orch.ResourceError): orch.parse_resource_spec(bad)

    def test_duplicate_resource_in_one_request_is_rejected_or_merged(self):
        merged = orch.parse_resource_specs(['db:a:shared', 'db:a:shared'])
        self.assertEqual(len(merged), 1)
        with self.assertRaises(orch.ResourceError) as cm: orch.parse_resource_specs(['db:a:shared', 'db:a:exclusive'])
        self.assertEqual(cm.exception.code, 'RESOURCE_SPEC_INVALID')


class ResourceLockTests(unittest.TestCase):
    """Named resource locks, implemented against docs/design/resource-locks.md."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.root = Path(self.tmp.name)
        (self.root / 'docs').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs' / 'PROJECT.md').write_text('# Project\n\n- `edition`：`local`\n', encoding='utf-8')
        with contextlib.redirect_stdout(io.StringIO()): orch.init(self.root)
        config = self.root / '.orchestration' / 'config.json'
        data = json.loads(config.read_text(encoding='utf-8'))
        data.update({'resource_lease_ttl': 600, 'resource_reclaim_grace': 30})
        config.write_text(json.dumps(data), encoding='utf-8')

    def tearDown(self):
        try:
            self.tmp.cleanup()
        except PermissionError:                     # Windows may still hold the WAL file
            pass

    # --- helpers ------------------------------------------------------------
    def config(self, **values):
        path = self.root / '.orchestration' / 'config.json'
        data = json.loads(path.read_text(encoding='utf-8'))
        data.update(values)
        path.write_text(json.dumps(data), encoding='utf-8')

    def add(self, tid, paths=('src/',), resources=None, depends=None):
        args = SimpleNamespace(id=tid, title=tid, description='', paths=list(paths), depends=depends or [],
                               acceptance=['works'], resources=resources or [])
        with contextlib.redirect_stdout(io.StringIO()): orch.task_add(self.root, args)

    def dispatch(self, tid, holder='hostA:100', extra=None, evidence='unit-test'):
        options = SimpleNamespace(id=tid, holder=holder, resource=list(extra or []), evidence=evidence, actor='orchestrator')
        out = io.StringIO()
        with contextlib.redirect_stdout(out): orch.task_dispatch(self.root, tid, options)
        return out.getvalue()

    def ready_then_dispatch(self, tid, holder='hostA:100', extra=None):
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, tid)
        return self.dispatch(tid, holder, extra)

    def advance(self, tid, dest, note='', head_sha=None, merge_sha=None):
        args = SimpleNamespace(id=tid, status=dest, head_sha=head_sha, merge_sha=merge_sha,
                               note=note or ('evidence recorded' if dest in {'APPROVED', 'MERGED', 'VERIFIED',
                                                                             'BLOCKED', 'NEEDS_HUMAN_DECISION',
                                                                             'FAILED', 'CANCELED'} else ''))
        with contextlib.redirect_stdout(io.StringIO()): orch.task_status(self.root, args)

    def count(self, sql, params=()):
        con = orch.connect(self.root)
        try: return con.execute(sql, params).fetchone()[0]
        finally: con.close()

    def rows(self, sql, params=()):
        con = orch.connect(self.root)
        try: return [dict(r) for r in con.execute(sql, params)]
        finally: con.close()

    def unlock(self, tid, resource='', reason='cleanup', force=False, evidence='', authorized_by=''):
        args = SimpleNamespace(id=tid, resource=resource, reason=reason, force=force,
                               evidence=evidence, authorized_by=authorized_by)
        with contextlib.redirect_stdout(io.StringIO()): orch.unlock(self.root, args)

    def events(self, task_id=None, kinds=None):
        sql = 'SELECT * FROM resource_lock_events'
        params = ()
        if task_id:
            sql += ' WHERE task_id=?'; params = (task_id,)
        sql += ' ORDER BY event_id'
        rows = self.rows(sql, params)
        return [r for r in rows if not kinds or r['event_type'] in kinds]

    # --- AC-1: schema -------------------------------------------------------
    def test_schema_has_every_contract_field_and_is_created_incrementally(self):
        columns = {r['name'] for r in self.rows('PRAGMA table_info(resource_locks)')}
        self.assertEqual(columns, {
            'lock_id', 'resource_id', 'mode', 'holder', 'task_id', 'acquired_at', 'lease_until', 'renewed_at',
            'state', 'release_reason', 'released_at', 'reason', 'evidence_ref', 'backend', 'generation'})
        event_columns = {r['name'] for r in self.rows('PRAGMA table_info(resource_lock_events)')}
        self.assertEqual(event_columns, {
            'event_id', 'event_type', 'at', 'actor', 'task_id', 'resource_id', 'mode', 'prev_state', 'new_state',
            'request_fingerprint', 'reason', 'evidence_ref'})

    def test_old_database_gains_the_tables_without_rebuild(self):
        # Simulate a database written before issue #6: no resource tables at all.
        self.add('TASK-OLD1', ['old.py'])
        con = sqlite3.connect(orch.db_path(self.root))
        con.executescript('DROP TABLE resource_lock_events; DROP TABLE resource_locks;')
        con.commit(); con.close()
        con = orch.connect(self.root)                     # opening must migrate, not fail
        try:
            tables = {r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            self.assertIn('resource_locks', tables)
            self.assertIn('resource_lock_events', tables)
            self.assertEqual(con.execute("SELECT COUNT(*) FROM resource_locks").fetchone()[0], 0)
            # The pre-existing task is intact and still dispatchable through the path gate.
            self.assertEqual(con.execute('SELECT status FROM tasks WHERE id=?', ('TASK-OLD1',)).fetchone()[0], 'PLANNED')
        finally: con.close()
        self.ready_then_dispatch('TASK-OLD1')

    # --- AC-2/AC-3: conflict matrix ----------------------------------------
    def test_shared_shared_is_allowed(self):
        self.add('TASK-S1', ['a.py'], ['db:test/main:shared'])
        self.add('TASK-S2', ['b.py'], ['db:test/main:shared'])
        self.ready_then_dispatch('TASK-S1', 'hostA:1')
        self.ready_then_dispatch('TASK-S2', 'hostB:2')
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE resource_id='db:test/main' "
                                    "AND state='ACTIVE'"), 2)
        self.assertEqual({r['task_id'] for r in self.rows('SELECT task_id FROM resource_locks')},
                         {'TASK-S1', 'TASK-S2'})

    def test_shared_then_exclusive_is_denied_with_holder_details(self):
        self.add('TASK-X1', ['a.py'], ['db:test/main:shared'])
        self.add('TASK-X2', ['b.py'], ['db:test/main:exclusive'])
        self.ready_then_dispatch('TASK-X1', 'hostA:1')
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, 'TASK-X2')
        with self.assertRaises(orch.ResourceError) as cm: self.dispatch('TASK-X2', 'hostB:2')
        message = str(cm.exception)
        self.assertIn('MODE_CONFLICT', message)
        self.assertIn('TASK-X1', message)          # owning task
        self.assertIn('hostA:1', message)          # owning holder
        self.assertIn('lease_until', message)      # lease of the blocker
        self.assertIn('next:', message)            # actionable advice, not just "conflict"
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE task_id='TASK-X2'"), 0)

    def test_exclusive_then_shared_and_exclusive_are_both_denied(self):
        self.add('TASK-E1', ['a.py'], ['svc:build/line1:exclusive'])
        self.add('TASK-E2', ['b.py'], ['svc:build/line1:shared'])
        self.add('TASK-E3', ['c.py'], ['svc:build/line1:exclusive'])
        self.ready_then_dispatch('TASK-E1', 'hostA:1')
        for tid in ('TASK-E2', 'TASK-E3'):
            with self.subTest(task=tid):
                with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, tid)
                with self.assertRaises(orch.ResourceError) as cm: self.dispatch(tid, 'hostB:2')
                self.assertIn('MODE_CONFLICT', str(cm.exception))
                self.assertEqual(self.count('SELECT COUNT(*) FROM resource_locks WHERE task_id=?', (tid,)), 0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks WHERE task_id=?', ('TASK-E1',)), 1)

    def test_upgrade_shared_to_exclusive_is_forbidden_in_place(self):
        self.add('TASK-U1', ['a.py'], ['db:test/main:shared'])
        self.ready_then_dispatch('TASK-U1', 'hostA:1')
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            with self.assertRaises(orch.ResourceDenied) as cm:
                orch.acquire_resource_set(con, 'TASK-U1', orch.resolve_resource_entries(self.root, ['db:test/main:exclusive']), 'hostA:1')
            self.assertEqual(cm.exception.code, 'UPGRADE_FORBIDDEN')
            con.execute('COMMIT')
        finally: con.close()
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE task_id='TASK-U1'"), 1)
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-U1'"), 'ACTIVE')
        self.assertTrue(self.events('TASK-U1', {'LOCK_DENIED_CONFLICT'}))

    # --- AC-4: atomic set, deterministic order, idempotency ----------------
    def test_partial_conflict_gives_up_the_whole_set_without_half_locks(self):
        self.add('TASK-B1', ['a.py'], ['db:b:exclusive'])
        self.add('TASK-B2', ['b.py'], ['db:a:shared', 'db:b:shared', 'db:c:shared'])
        self.ready_then_dispatch('TASK-B1', 'hostA:1')
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, 'TASK-B2')
        with self.assertRaises(orch.ResourceError) as cm: self.dispatch('TASK-B2', 'hostB:2')
        self.assertIn('MODE_CONFLICT', str(cm.exception))
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE task_id='TASK-B2' "
                                    "AND state<>'RELEASED'"), 0)         # invariant 5: zero residue
        self.assertEqual(self.count("SELECT COUNT(*) FROM locks WHERE task_id='TASK-B2'"), 0)
        self.assertEqual(self.count("SELECT status FROM tasks WHERE id='TASK-B2'"), 'READY')
        rollback = self.events('TASK-B2', {'LOCK_SET_ROLLED_BACK'})
        self.assertEqual(len(rollback), 1)
        self.assertIn('2 of 3', rollback[0]['reason'])
        self.assertEqual(len(self.events('TASK-B2', {'LOCK_DENIED_CONFLICT'})), 1)

    def test_acquisition_order_and_fingerprint_ignore_caller_order(self):
        entries_a = orch.order_resource_entries(orch.parse_resource_specs(['db:z:shared', 'db:a:exclusive', 'db:m:shared']))
        entries_b = orch.order_resource_entries(orch.parse_resource_specs(['db:m:shared', 'db:z:shared', 'db:a:exclusive']))
        self.assertEqual([e['resource_id'] for e in entries_a], ['db:a', 'db:m', 'db:z'])
        self.assertEqual(orch.resource_fingerprint(entries_a), orch.resource_fingerprint(entries_b))
        self.assertEqual(len(orch.resource_fingerprint(entries_a)), 64)

    def test_repeat_dispatch_of_the_same_set_is_idempotent(self):
        self.add('TASK-I1', ['a.py'], ['db:test/main:shared'])
        self.ready_then_dispatch('TASK-I1', 'hostA:1')
        before = self.rows("SELECT lock_id,lease_until,renewed_at,generation FROM resource_locks WHERE task_id='TASK-I1'")
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            result = orch.acquire_resource_set(con, 'TASK-I1', orch.resolve_resource_entries(self.root, ['db:test/main:shared']), 'hostA:1')
            con.execute('COMMIT')
        finally: con.close()
        self.assertEqual(result['status'], 'ALREADY_HELD')
        after = self.rows("SELECT lock_id,lease_until,renewed_at,generation FROM resource_locks WHERE task_id='TASK-I1'")
        self.assertEqual(before, after)                       # no new row, no refreshed lease
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE task_id='TASK-I1'"), 1)
        self.assertTrue(any('ALREADY_HELD' in e['reason'] for e in self.events('TASK-I1', {'LOCK_GRANTED'})))

    def test_smaller_set_mismatch_is_refused_and_quarantined(self):
        self.add('TASK-M1', ['a.py'], ['db:a:shared', 'db:b:shared'])
        self.ready_then_dispatch('TASK-M1', 'hostA:1')
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            with self.assertRaises(orch.ResourceDenied) as cm:
                orch.acquire_resource_set(con, 'TASK-M1', orch.resolve_resource_entries(self.root, ['db:a:shared']), 'hostA:1')
            self.assertEqual(cm.exception.code, 'LOCK_SET_MISMATCH')
            con.execute('COMMIT')
        finally: con.close()
        states = {r['resource_id']: r['state'] for r in self.rows("SELECT * FROM resource_locks WHERE task_id='TASK-M1'")}
        self.assertEqual(states, {'db:a': 'QUARANTINED', 'db:b': 'QUARANTINED'})
        self.assertEqual(len(self.events('TASK-M1', {'LOCK_QUARANTINED'})), 2)
        self.assertFalse({r['resource_id'] for r in self.rows("SELECT resource_id FROM resource_locks WHERE state='RELEASED'")})

    def test_growing_set_extends_and_keeps_the_original_locks(self):
        self.add('TASK-G1', ['a.py'], ['db:a:shared'])
        self.add('TASK-G2', ['b.py'], ['db:b:exclusive'])
        self.ready_then_dispatch('TASK-G1', 'hostA:1')
        self.ready_then_dispatch('TASK-G2', 'hostB:2')
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            with self.assertRaises(orch.ResourceDenied) as cm:
                orch.acquire_resource_set(con, 'TASK-G1',
                                          orch.resolve_resource_entries(self.root, ['db:a:shared', 'db:b:shared']), 'hostA:1')
            self.assertEqual(cm.exception.code, 'MODE_CONFLICT')
            con.execute('COMMIT')
        finally: con.close()
        remaining = self.rows("SELECT resource_id,state FROM resource_locks WHERE task_id='TASK-G1'")
        self.assertEqual(remaining, [{'resource_id': 'db:a', 'state': 'ACTIVE'}])   # extension left no partial lock
        self.assertIn('1 of 1', [e for e in self.events('TASK-G1', {'LOCK_SET_ROLLED_BACK'})][-1]['reason'])

    # --- AC-5: lifecycle ----------------------------------------------------
    def test_verified_and_canceled_release_but_failed_and_blocked_keep(self):
        for tid, dest, want in (('TASK-L1', 'VERIFIED', 0), ('TASK-L2', 'CANCELED', 0),
                                ('TASK-L3', 'FAILED', 1), ('TASK-L4', 'BLOCKED', 1)):
            with self.subTest(task=tid, dest=dest):
                self.add(tid, [f'{tid}.py'], ['db:lifecycle:shared'])
                self.ready_then_dispatch(tid, f'host{tid}:1')
                self.advance(tid, 'IN_PROGRESS')
                if dest == 'VERIFIED':
                    self.advance(tid, 'REVIEW_READY'); self.advance(tid, 'MASTER_REVIEW')
                    self.advance(tid, 'APPROVED', head_sha='a' * 40)
                    self.advance(tid, 'MERGED', merge_sha='b' * 40)
                    self.advance(tid, dest, note='terminal evidence')
                elif dest == 'CANCELED':
                    self.advance(tid, dest, note='cancelled by the owner')
                else:
                    self.advance(tid, dest, note='stopped without proof of release')
                self.assertEqual(self.count('SELECT COUNT(*) FROM resource_locks WHERE task_id=? '
                                            "AND state<>'RELEASED'", (tid,)), want)
        releases = self.rows("SELECT task_id,release_reason,reason FROM resource_locks WHERE state='RELEASED'")
        self.assertEqual({r['task_id']: r['release_reason'] for r in releases},
                         {'TASK-L1': 'NORMAL_COMPLETE', 'TASK-L2': 'CANCELED'})
        self.assertEqual(len(self.events('TASK-L1', {'LOCK_RELEASED'})), 1)

    def test_needs_human_decision_quarantines_instead_of_releasing(self):
        self.add('TASK-Q1', ['a.py'], ['db:q:exclusive'])
        self.ready_then_dispatch('TASK-Q1', 'hostA:1')
        self.advance('TASK-Q1', 'NEEDS_HUMAN_DECISION', note='state unknown; stop dispatching')
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-Q1'"), 'QUARANTINED')
        self.assertEqual(self.count('SELECT COUNT(*) FROM resource_locks WHERE task_id=? '
                                    "AND state='RELEASED'", ('TASK-Q1',)), 0)
        self.assertTrue(self.events('TASK-Q1', {'LOCK_QUARANTINED'}))

    def test_expired_lease_still_conflicts_and_is_not_released(self):
        self.config(resource_lease_ttl=1)
        self.add('TASK-P1', ['a.py'], ['db:exp:exclusive'])
        self.add('TASK-P2', ['b.py'], ['db:exp:exclusive'])
        self.ready_then_dispatch('TASK-P1', 'hostA:1')
        time.sleep(1.2)
        with contextlib.redirect_stdout(io.StringIO()):
            self.rows('SELECT 1')                        # plain read does not expire anything
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE'); orch.expire_due_locks(con); con.execute('COMMIT')
        finally: con.close()
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-P1'"), 'EXPIRED')
        self.assertEqual(self.count('SELECT COUNT(*) FROM resource_locks WHERE task_id=? '
                                    "AND state='RELEASED'", ('TASK-P1',)), 0)
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, 'TASK-P2')
        with self.assertRaises(orch.ResourceError) as cm: self.dispatch('TASK-P2', 'hostB:2')
        self.assertIn('MODE_CONFLICT', str(cm.exception))
        self.assertIn('EXPIRED', str(cm.exception))
        self.assertTrue(self.events('TASK-P1', {'LOCK_EXPIRED'}))

    def test_renewal_extends_only_own_locks_and_reactivates_expired(self):
        self.config(resource_lease_ttl=1)
        self.add('TASK-R1', ['a.py'], ['db:ren:shared'])
        self.add('TASK-R2', ['b.py'], ['db:ren:shared'])
        self.ready_then_dispatch('TASK-R1', 'hostA:1')
        self.ready_then_dispatch('TASK-R2', 'hostB:2')
        time.sleep(1.2)
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE'); orch.expire_due_locks(con)
            renewed = orch.renew_resource_locks(con, 'TASK-R1', 'hostA:1', 600, 'heartbeat restored', 'ops-log')
            con.execute('COMMIT')
        finally: con.close()
        self.assertEqual(len(renewed), 1)
        states = {r['task_id']: r['state'] for r in self.rows('SELECT task_id,state FROM resource_locks')}
        self.assertEqual(states, {'TASK-R1': 'ACTIVE', 'TASK-R2': 'EXPIRED'})
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            with self.assertRaises(orch.ResourceError) as cm:
                orch.renew_resource_locks(con, 'TASK-R2', 'hostA:1', 600, 'not mine', 'ops-log')
            self.assertEqual(cm.exception.code, 'LOCK_NOT_HELD')
            con.execute('ROLLBACK')
        finally: con.close()

    def test_reclaim_needs_expiry_cool_down_and_evidence(self):
        self.config(resource_lease_ttl=1, resource_reclaim_grace=3600)
        self.add('TASK-C1', ['a.py'], ['db:rec:exclusive'])
        self.ready_then_dispatch('TASK-C1', 'hostA:1')
        args = SimpleNamespace(id='TASK-C1', resource='db:rec', reason='holder process gone',
                               evidence='ops-log 9', actor='ops')
        # A live lease is never reclaimed, not even with a reason and evidence.
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            with self.assertRaises(orch.ResourceError) as cm:
                orch.reclaim_resource_lock(con, 'db:rec#exclusive#TASK-C1#0', 'holder gone', 'ops-log 9', 3600)
            self.assertEqual(cm.exception.code, 'NOT_EXPIRED')
            con.execute('ROLLBACK')
        finally: con.close()
        time.sleep(1.2)
        # task reclaim expires first, then refuses because the cool-down has not passed.
        with self.assertRaises(orch.ResourceError) as cm: orch.task_reclaim(self.root, args)
        self.assertEqual(cm.exception.code, 'GRACE_NOT_ELAPSED')
        # A refused reclaim rolls back as a whole: nothing is released and no
        # reclaim event survives, so the lock cannot be lost by trying too early.
        self.assertNotEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-C1'"), 'RELEASED')
        self.assertFalse(self.events('TASK-C1', {'LOCK_RECLAIMED'}))
        self.config(resource_reclaim_grace=0)
        with contextlib.redirect_stdout(io.StringIO()): orch.task_reclaim(self.root, args)
        row = self.rows("SELECT state,release_reason,evidence_ref FROM resource_locks WHERE task_id='TASK-C1'")[0]
        self.assertEqual((row['state'], row['release_reason']), ('RELEASED', 'LEASE_RECLAIMED'))
        self.assertEqual(row['evidence_ref'], 'ops-log 9')
        self.assertTrue(self.events('TASK-C1', {'LOCK_RECLAIMED'}))

    # --- AC-5: forced unlock ------------------------------------------------
    def test_forced_unlock_requires_reason_and_authorization_and_blocks_later_writes(self):
        self.add('TASK-F1', ['a.py'], ['db:force:exclusive'])
        self.ready_then_dispatch('TASK-F1', 'hostA:1')
        with self.assertRaises(orch.ResourceError) as cm:
            self.unlock('TASK-F1', resource='db:force', reason='   ', force=True, evidence='log')
        self.assertEqual(cm.exception.code, 'REASON_REQUIRED')
        with self.assertRaises(orch.ResourceError) as cm:
            self.unlock('TASK-F1', resource='db:force', reason='holder gone', force=True, evidence='log')
        self.assertEqual(cm.exception.code, 'AUTHORIZATION_REQUIRED')
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-F1'"), 'ACTIVE')
        self.unlock('TASK-F1', resource='db:force', reason='holder crashed; process 8842 gone',
                    force=True, evidence='ops-log 12', authorized_by='operator-csad')
        row = self.rows("SELECT state,release_reason,reason,evidence_ref FROM resource_locks WHERE task_id='TASK-F1'")[0]
        self.assertEqual((row['state'], row['release_reason']), ('RELEASED', 'FORCED_UNLOCK'))
        self.assertIn('process 8842 gone', row['reason'])
        forced = self.events('TASK-F1', {'LOCK_FORCED'})[0]
        self.assertEqual(forced['actor'], 'operator-csad')
        self.assertIn('operator-csad', forced['reason'])
        # A forced unlock must stop the old task from writing again.
        with self.assertRaises(orch.ResourceError) as cm:
            orch.task_renew(self.root, SimpleNamespace(id='TASK-F1', holder='hostA:1', reason='x', evidence='x'))
        self.assertEqual(cm.exception.code, 'LOCK_NOT_HELD')

    def test_release_twice_is_reported_without_a_new_transition(self):
        self.add('TASK-D1', ['a.py'], ['db:dup:shared'])
        self.ready_then_dispatch('TASK-D1', 'hostA:1')
        self.advance('TASK-D1', 'CANCELED', note='cancelled by owner')
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            row = orch.resource_row(con, 'db:dup#shared#TASK-D1#0')
            with self.assertRaises(orch.ResourceError) as cm:
                orch.release_resource_set(con, 'TASK-D1', 'CANCELED', 'again', 'orchestrator', 'log', rows=[row])
            self.assertIn('ALREADY_RELEASED', str(cm.exception))
            con.execute('COMMIT')
        finally: con.close()
        rejected = self.events('TASK-D1', {'LOCK_TRANSITION_REJECTED'})
        self.assertEqual(len(rejected), 1)
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE task_id='TASK-D1' "
                                    "AND release_reason='CANCELED'"), 1)

    def test_illegal_transition_is_rejected_and_the_record_is_kept(self):
        self.add('TASK-T1', ['a.py'], ['db:tr:shared'])
        self.ready_then_dispatch('TASK-T1', 'hostA:1')
        con = orch.connect(self.root)
        try:
            con.execute('BEGIN IMMEDIATE')
            row = orch.resource_row(con, 'db:tr#shared#TASK-T1#0')
            orch.transition_resource(con, row, 'RELEASED', 'LOCK_RELEASED', 'ops', 'manual', 'log', orch.now(),
                                     release_reason='NORMAL_COMPLETE', released_at=orch.now())
            released = orch.resource_row(con, 'db:tr#shared#TASK-T1#0')
            with self.assertRaises(orch.ResourceError) as cm:
                orch.transition_resource(con, released, 'ACTIVE', 'LOCK_RENEWED', 'ops', 'revive', 'log', orch.now())
            self.assertEqual(cm.exception.code, 'TRANSITION_REJECTED')
            con.execute('COMMIT')
        finally: con.close()
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-T1'"), 'RELEASED')
        self.assertTrue(self.events('TASK-T1', {'LOCK_TRANSITION_REJECTED'}))

    def test_event_id_is_idempotent(self):
        self.add('TASK-V1', ['a.py'], ['db:ev:shared'])
        con = orch.connect(self.root)
        try:
            first = orch.resource_event(con, 'RESOURCE_REGISTERED', 'TASK-V1', 'db:ev', 'shared', '(none)', '(none)',
                                        'f' * 64, 'first write', 'log', 'ops', orch.now(), event_id=4242)
            second = orch.resource_event(con, 'RESOURCE_REGISTERED', 'TASK-V1', 'db:ev', 'shared', '(none)', '(none)',
                                         'f' * 64, 'duplicate write', 'log', 'ops', orch.now(), event_id=4242)
        finally: con.close()
        self.assertEqual((first, second), (4242, 4242))
        self.assertEqual(self.count('SELECT COUNT(*) FROM resource_lock_events WHERE event_id=4242'), 1)
        self.assertEqual(self.events('TASK-V1', {'RESOURCE_REGISTERED'})[0]['reason'], 'first write')

    def test_generic_holder_is_refused(self):
        self.add('TASK-H1', ['a.py'], ['db:holder:shared'])
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, 'TASK-H1')
        with self.assertRaises(orch.ResourceError) as cm: self.dispatch('TASK-H1', 'agent')
        self.assertEqual(cm.exception.code, 'HOLDER_INVALID')
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE task_id='TASK-H1'"), 0)

    def test_missing_lease_ttl_refuses_dispatch_without_side_effects(self):
        self.config(resource_lease_ttl=None)
        self.add('TASK-N1', ['a.py'], ['db:cfg:shared'])
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, 'TASK-N1')
        with self.assertRaises(orch.ResourceError) as cm: self.dispatch('TASK-N1')
        self.assertEqual(cm.exception.code, 'RESOURCE_CONFIG_INVALID')
        self.assertIn('resource_lease_ttl', str(cm.exception))
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks"), 0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks'), 0)

    def test_task_without_resources_never_touches_the_resource_tables(self):
        self.add('TASK-Z1', ['only.py'])
        self.ready_then_dispatch('TASK-Z1', 'hostA:1')
        self.assertEqual(self.count('SELECT COUNT(*) FROM resource_lock_events'), 0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM resource_locks'), 0)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks'), 1)

    # --- AC-6: inspection ---------------------------------------------------
    def test_listing_show_and_resources_expose_lease_holder_and_blocking(self):
        self.add('TASK-I2', ['a.py'], ['db:visible/main:exclusive'])
        self.add('TASK-I3', ['b.py'], ['db:visible/main:shared'])
        self.ready_then_dispatch('TASK-I2', 'hostA:1')
        with contextlib.redirect_stdout(io.StringIO()): orch.task_ready(self.root, 'TASK-I3')
        with self.assertRaises(orch.ResourceError):
            with contextlib.redirect_stdout(io.StringIO()): self.dispatch('TASK-I3', 'hostB:2')
        out = io.StringIO()
        with contextlib.redirect_stdout(out): orch.task_resources(self.root, SimpleNamespace(resource='', all=False))
        listing = out.getvalue()
        for needle in ('db:visible/main', 'exclusive', 'ACTIVE', 'lease_until=', 'holder=hostA:1', 'task=TASK-I2',
                       'blocks=exclusive+shared'):
            self.assertIn(needle, listing)
        out = io.StringIO()
        with contextlib.redirect_stdout(out): orch.task_list(self.root, SimpleNamespace(active_only=False))
        self.assertIn('resources: db:visible/main:exclusive', out.getvalue())
        out = io.StringIO()
        with contextlib.redirect_stdout(out): orch.task_show(self.root, 'TASK-I3')
        self.assertIn('blocking reason', out.getvalue())
        self.assertIn('MODE_CONFLICT', out.getvalue())

    def test_github_backend_wording_is_advisory_only(self):
        self.assertIn('not an atomic lock', orch.resource_backend_note('github'))
        self.assertIn('advisory registration', orch.resource_backend_note('github'))
        self.assertIn('SQLite BEGIN IMMEDIATE', orch.resource_backend_note('local'))

    # --- AC-9: real multi-process race --------------------------------------
    def test_two_processes_racing_one_exclusive_resource(self):
        self.add('TASK-RACE1', ['race1/'], ['db:race/main:exclusive'])
        self.add('TASK-RACE2', ['race2/'], ['db:race/main:exclusive'])
        worker = self.root / 'race_worker.py'
        worker.write_text(CONCURRENCY_WORKER, encoding='utf-8')
        start = time.time() + 1.2
        script = Path(__file__).resolve().parents[1] / 'scripts' / 'orchestrator.py'
        processes = [
            subprocess.Popen([sys.executable, str(worker), str(script), str(self.root), tid, f'race-{index}:{index}',
                              'db:race/main:exclusive', str(start)],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            for index, tid in enumerate(('TASK-RACE1', 'TASK-RACE2'), 1)
        ]
        results = []
        for process in processes:
            stdout, stderr = process.communicate(timeout=120)
            self.assertEqual(process.returncode, 0, stderr)
            results.append(json.loads(stdout.strip().splitlines()[-1]))
        granted = [r for r in results if r['status'] == 'GRANTED']
        denied = [r for r in results if r['status'] == 'DENIED']
        self.assertEqual(len(granted), 1, results)
        self.assertEqual(len(denied), 1, results)
        self.assertEqual(denied[0]['code'], 'MODE_CONFLICT')
        self.assertEqual(self.count("SELECT COUNT(*) FROM resource_locks WHERE resource_id='db:race/main' "
                                    "AND state<>'RELEASED'"), 1)                 # one owner, no duplicate
        self.assertEqual(self.count('SELECT COUNT(*) FROM resource_locks WHERE task_id=?', (denied[0]['task'],)), 0)
        self.assertEqual(self.count("SELECT COUNT(DISTINCT holder) FROM resource_locks "
                                    "WHERE resource_id='db:race/main' AND state<>'RELEASED'"), 1)
        self.assertEqual(self.count('SELECT COUNT(*) FROM locks'), 0)          # neither wrote a path lock
        self.assertTrue(self.events(denied[0]['task'], {'LOCK_SET_ROLLED_BACK'}))

    # --- AC-10: recovery ----------------------------------------------------
    def test_reconcile_quarantines_lock_rows_without_events(self):
        self.add('TASK-W1', ['a.py'], ['db:rec2:shared'])
        self.ready_then_dispatch('TASK-W1', 'hostA:1')
        con = sqlite3.connect(orch.db_path(self.root))
        con.execute('DELETE FROM resource_lock_events')     # simulate lost evidence
        con.commit(); con.close()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            orch.task_reconcile(self.root, SimpleNamespace(reason='new session took over', evidence='session-2'))
        self.assertIn('missing-evidence', out.getvalue())
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-W1'"), 'QUARANTINED')
        self.assertTrue(self.events('TASK-W1', {'LOCK_QUARANTINED'}))

    def test_reconcile_does_not_let_its_own_expiry_event_count_as_evidence(self):
        self.config(resource_lease_ttl=1)
        self.add('TASK-W2', ['b.py'], ['db:rec3:shared'])
        self.ready_then_dispatch('TASK-W2', 'hostA:1')
        time.sleep(1.2)
        con = sqlite3.connect(orch.db_path(self.root))
        con.execute('DELETE FROM resource_lock_events')     # only the lease survived
        con.commit(); con.close()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            orch.task_reconcile(self.root, SimpleNamespace(reason='session take-over', evidence='session-3'))
        self.assertIn('missing-evidence', out.getvalue())
        self.assertNotIn('kept EXPIRED', out.getvalue())
        self.assertEqual(self.count("SELECT state FROM resource_locks WHERE task_id='TASK-W2'"), 'QUARANTINED')


class ResourceLifecycleOnStatusTests(unittest.TestCase):
    """The terminal release is driven by task status, not by a separate command."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.root = Path(self.tmp.name)
        (self.root / 'docs').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs' / 'PROJECT.md').write_text('# Project\n\n- `edition`：`local`\n', encoding='utf-8')
        with contextlib.redirect_stdout(io.StringIO()): orch.init(self.root)
        config = self.root / '.orchestration' / 'config.json'
        data = json.loads(config.read_text(encoding='utf-8')); data['resource_lease_ttl'] = 600
        config.write_text(json.dumps(data), encoding='utf-8')

    def tearDown(self):
        try: self.tmp.cleanup()
        except PermissionError: pass

    def test_path_locks_only_task_still_releases_exactly_once(self):
        args = SimpleNamespace(id='TASK-ONLY', title='t', description='', paths=['one.py'], depends=[],
                               acceptance=['works'], resources=[])
        with contextlib.redirect_stdout(io.StringIO()):
            orch.task_add(self.root, args); orch.task_ready(self.root, 'TASK-ONLY')
            orch.task_dispatch(self.root, 'TASK-ONLY', SimpleNamespace(holder='h:1', resource=[], evidence='x'))
            orch.task_status(self.root, SimpleNamespace(id='TASK-ONLY', status='IN_PROGRESS', head_sha=None,
                                                        merge_sha=None, note=''))
            orch.task_status(self.root, SimpleNamespace(id='TASK-ONLY', status='CANCELED', head_sha=None,
                                                        merge_sha=None, note='cancel smoke'))
        con = orch.connect(self.root)
        try:
            self.assertEqual(con.execute('SELECT COUNT(*) FROM locks').fetchone()[0], 0)
            self.assertEqual(con.execute('SELECT COUNT(*) FROM resource_locks').fetchone()[0], 0)
            actions = [r[0] for r in con.execute("SELECT action FROM events WHERE task_id='TASK-ONLY'")]
            self.assertEqual(actions.count('LOCKS_RELEASED'), 1)
        finally: con.close()


class TaskAddRepeatableOptionTests(unittest.TestCase):
    """Issue #93: repeating a multi-value option must merge, not overwrite.

    The defect lived in argparse (`nargs` with the default `store` action), so
    these tests drive the real CLI entry point instead of calling task_add().
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.root = Path(self.tmp.name)
        (self.root / 'docs').mkdir(parents=True, exist_ok=True)
        (self.root / 'docs' / 'PROJECT.md').write_text(
            '# Project\n\n## 运行配置\n\n- `edition`：`local`\n', encoding='utf-8')
        with contextlib.redirect_stdout(io.StringIO()): orch.init(self.root)

    def tearDown(self): self.tmp.cleanup()

    def cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, 'argv', ['orchestrator.py', '--root', str(self.root), *argv]):
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = orch.main()
        return code, out.getvalue(), err.getvalue()

    def add(self, tid, *extra):
        return self.cli('task', 'add', '--id', tid, '--title', tid, *extra)

    def row(self, tid):
        con = orch.connect(self.root)
        try: return con.execute('SELECT * FROM tasks WHERE id=?', (tid,)).fetchone()
        finally: con.close()

    def show(self, tid):
        code, out, err = self.cli('task', 'show', tid)
        self.assertEqual(code, 0, err)
        return out

    def test_repeated_acceptance_keeps_every_criterion_in_order(self):
        code, out, err = self.add('TASK-9301', '--paths', 'src/',
                                  '--acceptance', 'AC-1: first', '--acceptance', 'AC-2: second')
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(self.row('TASK-9301')['acceptance_json']),
                         ['AC-1: first', 'AC-2: second'])
        shown = self.show('TASK-9301')
        self.assertIn('- [ ] AC-1: first', shown)
        self.assertIn('- [ ] AC-2: second', shown)
        self.assertLess(shown.index('AC-1: first'), shown.index('AC-2: second'))

    def test_single_acceptance_occurrence_with_multiple_values_still_works(self):
        code, out, err = self.add('TASK-9302', '--paths', 'src/', '--acceptance', 'AC-1', 'AC-2')
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(self.row('TASK-9302')['acceptance_json']), ['AC-1', 'AC-2'])

    def test_dispatch_keeps_every_acceptance_in_the_task_packet(self):
        code, out, err = self.add('TASK-9303', '--paths', 'src/',
                                  '--acceptance', 'AC-1: first', '--acceptance', 'AC-2: second')
        self.assertEqual(code, 0, err)
        for cmd in (('task', 'ready', 'TASK-9303'), ('task', 'dispatch', 'TASK-9303')):
            code, out, err = self.cli(*cmd)
            self.assertEqual(code, 0, err)
        shown = self.show('TASK-9303')
        self.assertIn('Status: DISPATCHED', shown)
        self.assertIn('- [ ] AC-1: first', shown)
        self.assertIn('- [ ] AC-2: second', shown)

    def test_repeated_paths_are_merged_sorted_and_deduplicated(self):
        code, out, err = self.add('TASK-9304', '--paths', 'tests/', '--paths', 'src/', '--paths', 'src/',
                                  '--acceptance', 'AC-1')
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(self.row('TASK-9304')['paths_json']), ['src', 'tests'])

    def test_single_paths_occurrence_with_multiple_values_still_works(self):
        code, out, err = self.add('TASK-9305', '--paths', 'tests/', 'src/', '--acceptance', 'AC-1')
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(self.row('TASK-9305')['paths_json']), ['src', 'tests'])

    def test_invalid_path_is_still_rejected(self):
        code, out, err = self.add('TASK-9306', '--paths', 'src/', '--paths', '../secret', '--acceptance', 'AC-1')
        self.assertEqual(code, 1)
        self.assertIn('invalid relative path', err)
        self.assertIsNone(self.row('TASK-9306'))

    def test_repeated_depends_keeps_every_dependency(self):
        for dep in ('TASK-9307-A', 'TASK-9307-B'):
            code, out, err = self.add(dep, '--paths', dep.lower() + '/', '--acceptance', 'AC-1')
            self.assertEqual(code, 0, err)
        code, out, err = self.add('TASK-9307', '--paths', 'src/', '--acceptance', 'AC-1',
                                  '--depends', 'TASK-9307-B', '--depends', 'TASK-9307-A')
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(self.row('TASK-9307')['depends_json']),
                         ['TASK-9307-A', 'TASK-9307-B'])

    def test_repeated_resources_keeps_every_declaration(self):
        code, out, err = self.add('TASK-9308', '--paths', 'src/', '--acceptance', 'AC-1',
                                  '--resources', 'db:issue93/a:shared', '--resources', 'db:issue93/b:exclusive')
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(self.row('TASK-9308')['resources_json']),
                         ['db:issue93/a:shared', 'db:issue93/b:exclusive'])

    def test_invalid_resource_is_still_rejected(self):
        code, out, err = self.add('TASK-9309', '--paths', 'src/', '--acceptance', 'AC-1',
                                  '--resources', 'db:issue93/a:shared', '--resources', 'DB:Issue93/B:shared')
        self.assertEqual(code, 1)
        self.assertIn('uppercase', err)
        self.assertIsNone(self.row('TASK-9309'))

    def test_empty_depends_and_resources_stay_empty(self):
        code, out, err = self.add('TASK-9310', '--paths', 'src/', '--acceptance', 'AC-1',
                                  '--depends', '--resources')
        self.assertEqual(code, 0, err)
        row = self.row('TASK-9310')
        self.assertEqual(json.loads(row['depends_json']), [])
        self.assertEqual(json.loads(row['resources_json']), [])


if __name__=='__main__': unittest.main()
