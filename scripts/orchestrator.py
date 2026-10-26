#!/usr/bin/env python3
# Local single-user task registry and atomic path-lock manager.
# This tool manages governance state only; it does not launch agents or merge Git branches.
#
# Named resource locks implement the frozen contract docs/design/RESOURCE-LOCKS.md
# (v1). That file is the only authority for their semantics; this module must not
# restate or extend it. Path-lock behaviour is unchanged.
from __future__ import annotations
import argparse, hashlib, json, os, re, socket, sqlite3, sys, unicodedata
from datetime import datetime, timedelta, timezone
from pathlib import Path, PurePosixPath

ACTIVE = {'DISPATCHED','IN_PROGRESS','REVIEW_READY','MASTER_REVIEW','APPROVED','MERGED','BLOCKED','NEEDS_HUMAN_DECISION','FAILED'}
TERMINAL = {'VERIFIED','CANCELED'}
TRANSITIONS = {
 'PLANNED': {'READY','BLOCKED','NEEDS_HUMAN_DECISION','CANCELED'},
 'READY': {'DISPATCHED','BLOCKED','NEEDS_HUMAN_DECISION','CANCELED'},
 'DISPATCHED': {'IN_PROGRESS','FAILED','BLOCKED','NEEDS_HUMAN_DECISION','CANCELED'},
 'IN_PROGRESS': {'REVIEW_READY','FAILED','BLOCKED','NEEDS_HUMAN_DECISION','CANCELED'},
 'REVIEW_READY': {'MASTER_REVIEW','IN_PROGRESS','BLOCKED','NEEDS_HUMAN_DECISION'},
 'MASTER_REVIEW': {'APPROVED','IN_PROGRESS','BLOCKED','NEEDS_HUMAN_DECISION','CANCELED'},
 'APPROVED': {'MERGED','MASTER_REVIEW','BLOCKED'},
 'MERGED': {'VERIFIED','BLOCKED'},
 'VERIFIED': set(), 'BLOCKED': {'READY','IN_PROGRESS','MASTER_REVIEW','NEEDS_HUMAN_DECISION','FAILED','CANCELED'},
 'NEEDS_HUMAN_DECISION': {'READY','IN_PROGRESS','MASTER_REVIEW','CANCELED'},
 'FAILED': {'READY','CANCELED'}, 'CANCELED': set(),
}

def now(): return datetime.now(timezone.utc).isoformat(timespec='seconds')
def root_path(raw): return Path(raw).resolve()
def db_path(root): return root/'.orchestration'/'state.sqlite3'
def connect(root):
    p=db_path(root)
    if not p.exists(): raise RuntimeError(f'task database not initialized: {p}; run `init` first')
    con=sqlite3.connect(p, timeout=15, isolation_level=None)
    con.row_factory=sqlite3.Row
    con.execute('PRAGMA foreign_keys=ON')
    con.execute('PRAGMA busy_timeout=15000')
    # Migrate incrementally on open: a database created before resource locks
    # existed must gain the tables without being rebuilt (§8.2 rule 5).
    ensure_resource_schema(con)
    return con

# Review-fix retry cap precedence (see also read_retry_cap):
#   1. <root>/.orchestration/config.json key "retry_cap" when it is an integer >= 1.
#   2. Otherwise the built-in default RETRY_CAP_DEFAULT = 3.
# A missing file, a missing key, or an invalid value falls back to the default so
# the historical hard-coded behaviour (3 rounds) is preserved for existing roots.
RETRY_CAP_DEFAULT = 3
EDITIONS = ('local','github')
# `.orchestration/config.json` holds local CLI runtime parameters only. It never
# selects the edition: that key lives in <root>/docs/PROJECT.md, which is
# the single authority (see read_project_edition). Keeping the edition out of this
# file removes the second, silently-defaulting definition that used to exist.
RUNTIME_DEFAULTS = {'retry_cap': RETRY_CAP_DEFAULT, 'local_dir': '.local'}
PROJECT_CONFIG = 'docs/PROJECT.md'
# docs/PLAN.md is the plan of record (task registration, dispatch and phase
# close-out read it); templates/PLAN.md is only the seed copy that `init`
# copies once. A root-level PLAN.md is the legacy (<=1.3) location: init never
# touches it and never seeds docs/PLAN.md while it exists, so the design goals
# always keep a single authority. Both names are relative to the project root.
PLAN_DOC = 'docs/PLAN.md'
LEGACY_PLAN_DOC = 'PLAN.md'
PLAN_TEMPLATE = 'templates/plan.md'
EDITION_LINE = re.compile(r'^\s*[-*]\s*`?edition`?\s*[:：]\s*(?P<value>.*)$', re.M)

class EditionError(RuntimeError):
    """The authoritative project edition is missing, unreadable or illegal."""

def config_path(root): return root/'.orchestration'/'config.json'
def project_config_path(root): return root/PROJECT_CONFIG

def _runtime_config(root):
    """retry_cap / local_dir from config.json, with the historical silent fallback.

    Unchanged on purpose: a missing file, a missing key, a wrong type, an
    out-of-range value or malformed JSON yields the built-in default so existing
    local roots keep working. The edition is deliberately not read here, so an
    edition problem can never change or block the retry budget.
    """
    cfg=dict(RUNTIME_DEFAULTS)
    sources={key:'default' for key in RUNTIME_DEFAULTS}
    data=None; state='missing'
    try:
        data=json.loads(config_path(root).read_text(encoding='utf-8'))
        state='invalid' if not isinstance(data,dict) else 'ok'
    except FileNotFoundError:
        pass
    except Exception:
        state='invalid'
    if state=='ok':
        value=data.get('retry_cap')
        if not isinstance(value,bool) and isinstance(value,int) and value>=1:
            cfg['retry_cap']=value; sources['retry_cap']='config.json'
        value=data.get('local_dir')
        if isinstance(value,str) and value.strip():
            cfg['local_dir']=value.strip(); sources['local_dir']='config.json'
    cfg['sources']=sources
    cfg['config_state']=state
    cfg['recorded_edition']=data.get('edition') if state=='ok' else None
    return cfg

def read_project_edition(root):
    """Return the edition declared by <root>/docs/PROJECT.md.

    This key is the only authority for edition selection. A missing file, a
    missing or duplicated key, an empty value, a wrong type or an unsupported
    value raises EditionError naming the actual content and the file to fix.
    Nothing falls back to 'local'.
    """
    path=project_config_path(root)
    try:
        text=path.read_text(encoding='utf-8')
    except FileNotFoundError:
        raise EditionError(f'{PROJECT_CONFIG}: file not found at {path}; add the key `edition` with value local or github') from None
    except (OSError,UnicodeDecodeError) as exc:
        raise EditionError(f'{PROJECT_CONFIG}: cannot read {path} ({exc}); repair the file so its `edition` key is readable') from None
    matches=list(EDITION_LINE.finditer(text))
    if not matches:
        raise EditionError(f'{PROJECT_CONFIG}: missing required key `edition` in {path}; add one line "- `edition`: local" or "- `edition`: github"')
    if len(matches)>1:
        where=', '.join(str(text.count('\n',0,m.start())+1) for m in matches)
        raise EditionError(f'{PROJECT_CONFIG}: key `edition` is declared {len(matches)} times (lines {where}) in {path}; keep exactly one declaration')
    raw=matches[0].group('value').strip()
    value=raw.strip('`').strip()
    if not value:
        raise EditionError(f'{PROJECT_CONFIG}: key `edition` is empty in {path}; allowed values are local, github')
    if value not in EDITIONS:
        raise EditionError(f"{PROJECT_CONFIG}: key `edition` has unsupported value {raw!r} in {path}; "
                           f"allowed values are {', '.join(EDITIONS)} (write the value alone, with no extra text)")
    return value

def read_config(root):
    """Return the effective runtime configuration, its provenance and the edition.

    Runtime keys keep their historical silent fallback. The edition is read from
    docs/PROJECT.md and raises EditionError when missing or illegal, so
    routing can never silently run on the wrong task carrier.
    """
    cfg=_runtime_config(root)
    cfg['edition']=read_project_edition(root)
    cfg['sources']['edition']=PROJECT_CONFIG
    return cfg

def read_retry_cap(root):
    """Return the effective review-fix retry cap (integer >= 1, default 3).

    Precedence: config.json key "retry_cap" wins when present and valid;
    anything else (missing file/key, non-integer, bool, < 1, malformed JSON)
    silently yields RETRY_CAP_DEFAULT. Reads the same parser as read_config, so
    the two entry points can never disagree about the cap, while an edition
    problem cannot block this value.
    """
    return _runtime_config(root)['retry_cap']

def require_local_scheduler(root, action):
    """Refuse to write local task state unless the project declares the local edition.

    The CLI implements the local edition only. Under a github (or broken) edition
    it must fail loudly and name the next step instead of quietly behaving like
    local: github task state lives in Issues and PRs, and writing the SQLite
    registry would create a second, conflicting source of truth.
    """
    edition=read_project_edition(root)
    if edition=='local': return
    raise EditionError(
        f'edition is {edition!r} ({project_config_path(root)}); this CLI cannot run `{action}` for the {edition} edition: '
        f'it writes the local SQLite task registry, which is a read-only archive under {edition}. '
        'Next: create or update the task as a GitHub Issue using editions/github/issue-template.md, record the '
        'ORCH-TASK-REGISTRY block before starting, and drive the PR per editions/github/execution.md. '
        'init, backup, config show, task list, task show and task resources stay available.'
    )

def config_show(root):
    """Print the effective configuration, where each value came from, and the edition.

    Returns 0 when the project edition is legal, 1 when it is missing or illegal.
    The edition is reported from docs/PROJECT.md only; a leftover
    `edition` key in config.json is shown as ignored, never applied.
    """
    cfg=_runtime_config(root); path=config_path(root); exists=path.exists()
    edition=None; error=None
    try: edition=read_project_edition(root)
    except EditionError as exc: error=exc
    print(f'Config file: {path}' + ('' if exists else ' (not found; using built-in defaults)'))
    rows=(('retry_cap',cfg['retry_cap'],cfg['sources']['retry_cap']),
          ('local_dir',cfg['local_dir'],cfg['sources']['local_dir']),
          ('edition',edition or '(invalid)',PROJECT_CONFIG))
    for key,value,source in rows:
        print(f'  {key:<10} = {value!s:<10} (source: {source})')
    print(f'Edition is selected only by `edition` in {project_config_path(root)}; '
          f'{path.name} holds local CLI runtime parameters (retry_cap, local_dir) and never selects an edition.')
    if cfg['config_state']=='invalid':
        print(f'Note: {path} is not a readable JSON object; retry_cap and local_dir fall back to the built-in defaults.')
    if cfg['recorded_edition'] is not None:
        print(f'Note: {path} also records edition = {cfg["recorded_edition"]!r}; that legacy key is ignored.')
    print('Edit .orchestration/config.json to change retry_cap or local_dir; invalid values there fall back to defaults.')
    if error:
        print(f'ERROR: {error}',file=sys.stderr)
        return 1
    if edition!='local':
        print(f'Scheduling commands (task add/ready/dispatch/status/workspace/amend/unlock) are refused under the '
              f'{edition} edition; this CLI maintains the local task registry only (matrix: README.md 6.2).')
    return 0

def safe_rollback(con):
    """Roll back only when a transaction is actually open; never mask the real error.

    If BEGIN IMMEDIATE itself failed (for example because the database is locked)
    there is no transaction to roll back, and issuing ROLLBACK would raise
    'cannot rollback - no transaction is active', hiding the original failure.
    """
    try:
        if con.in_transaction:
            con.execute('ROLLBACK')
    except sqlite3.Error:
        pass

def event(con, task_id, action, from_status=None, to_status=None, note='', data=None):
    con.execute('INSERT INTO events(task_id,at,action,from_status,to_status,note,data_json) VALUES(?,?,?,?,?,?,?)',
        (task_id,now(),action,from_status,to_status,note,json.dumps(data or {},ensure_ascii=False)))

def init_plan(root):
    """Seed <root>/docs/PLAN.md from templates/PLAN.md exactly once.

    docs/PLAN.md is the plan of record, so an existing file is never
    overwritten (that would destroy the project owner's plan). A legacy root
    PLAN.md (<=1.3 location) is never touched either: while it exists,
    docs/PLAN.md is NOT seeded, because two design-goal authorities are worse
    than one stale path — init prints the migration step instead. A missing
    template is reported and skipped: `init` must still finish its task-store
    work and exit 0. A write failure is raised instead of being reported as
    success. Returns 'created', 'kept', 'legacy-present' or 'template-missing'.
    """
    target=root/PLAN_DOC
    legacy=root/LEGACY_PLAN_DOC
    if target.exists():
        print(f'PLAN.md already present, kept unchanged: {target}')
        print('Next: judge task direction from the design goals in docs/PLAN.md; templates/PLAN.md is only the initial copy.')
        return 'kept'
    if legacy.exists():
        print(f'Legacy root PLAN.md found; docs/PLAN.md not created: {legacy}')
        print('Next: keep a single design-goal authority — run `git mv PLAN.md docs/PLAN.md` (create docs/ first if needed), then rerun init; see README section 10.')
        return 'legacy-present'
    template=root/PLAN_TEMPLATE
    if not template.is_file():
        print(f'PLAN.md not created: template missing at {template}')
        print('Next: copy templates/PLAN.md into docs/PLAN.md (kit installed there), then rerun init.')
        return 'template-missing'
    (root/'docs').mkdir(parents=True,exist_ok=True)
    target.write_bytes(template.read_bytes())
    print(f'Created docs/PLAN.md from {template}')
    print('下一步：填写 docs/PLAN.md 的设计目标（为什么做、做到什么程度、原则与边界）；配图放 docs/images/，在正文中用相对路径引用，Agent 只按文中指引查看图片')
    return 'created'

def init(root):
    d=root/'.orchestration'; d.mkdir(parents=True,exist_ok=True)
    (d/'backups').mkdir(exist_ok=True)
    cfg=config_path(root)
    if not cfg.exists():
        cfg.write_text(json.dumps(RUNTIME_DEFAULTS)+'\n',encoding='utf-8')
    con=sqlite3.connect(db_path(root))
    con.executescript('''
    PRAGMA journal_mode=WAL;
    PRAGMA foreign_keys=ON;
    CREATE TABLE IF NOT EXISTS tasks(
      id TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
      paths_json TEXT NOT NULL, depends_json TEXT NOT NULL, acceptance_json TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'PLANNED', branch TEXT NOT NULL DEFAULT '', worktree TEXT NOT NULL DEFAULT '',
      head_sha TEXT NOT NULL DEFAULT '', approved_sha TEXT NOT NULL DEFAULT '', merged_sha TEXT NOT NULL DEFAULT '', retries INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_note TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE IF NOT EXISTS locks(
      path TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS locks_task_idx ON locks(task_id);
    CREATE TABLE IF NOT EXISTS events(
      id INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL, at TEXT NOT NULL,
      action TEXT NOT NULL, from_status TEXT, to_status TEXT, note TEXT NOT NULL DEFAULT '', data_json TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX IF NOT EXISTS events_task_idx ON events(task_id,id);
    ''')
    ensure_resource_schema(con)
    con.close()
    # Keep private, machine-local state out of the worktree and commits.
    gi=root/'.git'/'info'/'exclude'
    if (root/'.git').is_dir():
        gi.parent.mkdir(parents=True,exist_ok=True)
        current=gi.read_text(encoding='utf-8',errors='ignore') if gi.exists() else ''
        if '.orchestration/' not in current.splitlines():
            with gi.open('a',encoding='utf-8') as f: f.write('\n# Agent-Orchestration machine-local state\n.orchestration/\n')
    print(f'Initialized local task registry: {db_path(root)}')
    print(f'Review-fix retry cap: {read_retry_cap(root)} (edit key "retry_cap" in {cfg} to change it)')
    print(f'Local task registry only: the edition is read from {project_config_path(root)}, not from this config.')
    print('Back up .orchestration/ with your project backups.')
    init_plan(root)

def normalize_path(raw):
    s=raw.replace('\\','/').strip()
    p=PurePosixPath(s)
    if not s or p.is_absolute() or re.match(r'^[A-Za-z]:/', s) or any(part in ('..','.') for part in p.parts) or any(ch in s for ch in '*?[]'):
        raise ValueError(f'invalid relative path: {raw!r}')
    return str(p).rstrip('/') or '.'

def overlaps(a,b):
    if a==b or a=='.' or b=='.': return True
    return a.startswith(b.rstrip('/')+'/') or b.startswith(a.rstrip('/')+'/')

def get_task(con, task_id):
    row=con.execute('SELECT * FROM tasks WHERE id=?',(task_id,)).fetchone()
    if not row: raise RuntimeError(f'unknown task: {task_id}')
    return row

def task_add(root, args):
    require_local_scheduler(root,'task add')
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{1,63}',args.id): raise ValueError('task ID must be 2-64 chars: letters, digits, dot, underscore, hyphen')
    paths=sorted(set(normalize_path(p) for p in args.paths))
    if not paths: raise ValueError('at least one allowed path is required')
    if not args.acceptance: raise ValueError('at least one acceptance criterion is required')
    # Declared resources are optional; an empty declaration keeps the task outside
    # the resource-lock space entirely (contract §8.2 rule 1).
    resources=[f'{e["resource_id"]}:{e["mode"]}' + (f':{e["ttl"]}' if e['ttl'] else '')
               for e in parse_resource_specs(getattr(args,'resources',None) or [])]
    depends=sorted(set(args.depends or []))
    if args.id in depends: raise ValueError('task cannot depend on itself')
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE')
        for dep in depends:
            if not con.execute('SELECT 1 FROM tasks WHERE id=?',(dep,)).fetchone(): raise RuntimeError(f'dependency not found: {dep}')
        con.execute('INSERT INTO tasks(id,title,description,paths_json,depends_json,acceptance_json,resources_json,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)',
            (args.id,args.title,args.description or '',json.dumps(paths),json.dumps(depends),json.dumps(args.acceptance,ensure_ascii=False),json.dumps(resources),'PLANNED',now(),now()))
        # Detect cycles after insertion. Rollback avoids leaving a bad task behind.
        seen=set(); stack=set()
        def visit(tid):
            if tid in stack: raise RuntimeError('dependency cycle detected')
            if tid in seen: return
            stack.add(tid)
            row=con.execute('SELECT depends_json FROM tasks WHERE id=?',(tid,)).fetchone()
            for d in json.loads(row['depends_json']): visit(d)
            stack.remove(tid); seen.add(tid)
        visit(args.id)
        event(con,args.id,'TASK_CREATED',None,'PLANNED','Task created',{'paths':paths,'depends':depends,'declared_resources':resources})
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: created in PLANNED')
    if resources: print(f'{args.id}: declared resources {", ".join(resources)}')

def task_list(root, args):
    con=connect(root)
    try:
        rows=con.execute('SELECT * FROM tasks ORDER BY created_at,id').fetchall()
        for r in rows:
            if args.active_only and r['status'] in TERMINAL: continue
            paths=', '.join(json.loads(r['paths_json']))
            locks=[x['path'] for x in con.execute('SELECT path FROM locks WHERE task_id=? ORDER BY path',(r['id'],))]
            print(f"{r['id']}\t{r['status']}\t{r['title']}\n  paths: {paths}\n  locks: {', '.join(locks) if locks else '(none)'}\n  retries: {r['retries']}")
            # Resource locks are reported only when the task participates in them,
            # so a path-lock-only task prints exactly what it printed before.
            declared=declared_resources(r)
            held=held_resource_rows(con,r['id'])
            if declared or held:
                print(f"  resources: {', '.join(declared) if declared else '(none)'}")
                for row in held:
                    print(f"    {row['resource_id']} {row['mode']} {row['state']} "
                          f"lease_until {row['lease_until']} holder {row['holder']}"
                          + resource_blocking_note(row))
    finally: con.close()

def resource_blocking_note(row):
    """Short, honest conflict note for listings (no claim beyond the state)."""
    if row['state']=='RELEASED':
        return f" (released {row['released_at']}, reason {row['release_reason']})"
    if row['mode']=='exclusive':
        return ' (blocks every new claim on this resource until released)'
    return ' (blocks new exclusive claims on this resource until released)'

def task_show(root, task_id):
    con=connect(root)
    try:
        r=get_task(con,task_id)
        print(f"# {r['id']} — {r['title']}\n\nStatus: {r['status']}\nDescription: {r['description']}\nAllowed paths:\n"+'\n'.join(f'- {p}' for p in json.loads(r['paths_json']))+'\n\nDependencies:\n'+('\n'.join(f'- {d}' for d in json.loads(r['depends_json'])) or '- None')+'\n\nAcceptance criteria:\n'+'\n'.join(f'- [ ] {x}' for x in json.loads(r['acceptance_json']))+f"\n\nBranch: {r['branch'] or '(unset)'}\nWorktree: {r['worktree'] or '(unset)'}\nHead SHA: {r['head_sha'] or '(unset)'}\nApproved SHA: {r['approved_sha'] or '(unset)'}\nMerge SHA: {r['merged_sha'] or '(unset)'}\nRetries: {r['retries']}\nLast note: {r['last_note'] or '(none)'}")
        declared=declared_resources(r); held=held_resource_rows(con,task_id)
        lines=[f"- declared: {', '.join(declared)}" if declared else '- declared: (none)']
        for row in held:
            lines.append(f"- {row['resource_id']} {row['mode']} {row['state']} holder {row['holder']} "
                         f"lease_until {row['lease_until']} renewed_at {row['renewed_at']} "
                         f"lock_id {row['lock_id']}"+resource_blocking_note(row))
        denials=con.execute("SELECT at,resource_id,reason FROM resource_lock_events WHERE task_id=? "
                            "AND event_type IN ('LOCK_DENIED_CONFLICT','LOCK_SET_ROLLED_BACK','LOCK_QUARANTINED') "
                            'ORDER BY event_id DESC LIMIT 3',(task_id,)).fetchall()
        for denial in denials:
            lines.append(f"- blocking reason {denial['at']} {denial['resource_id']}: {denial['reason']}")
        print('\nResource locks (contract: docs/design/RESOURCE-LOCKS.md):\n'+'\n'.join(lines))
        print('\nEvents:')
        for e in con.execute('SELECT * FROM events WHERE task_id=? ORDER BY id',(task_id,)):
            print(f"- {e['at']} {e['action']} {e['from_status'] or '-'}→{e['to_status'] or '-'} {e['note']}")
    finally: con.close()

def dependency_check(con, row):
    for dep in json.loads(row['depends_json']):
        d=get_task(con,dep)
        if d['status']!='VERIFIED': raise RuntimeError(f'dependency {dep} is {d["status"]}, expected VERIFIED')

def task_ready(root, task_id):
    require_local_scheduler(root,'task ready')
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE'); r=get_task(con,task_id)
        if r['status'] not in {'PLANNED','BLOCKED','FAILED','NEEDS_HUMAN_DECISION'}: raise RuntimeError(f'cannot set READY from {r["status"]}')
        dependency_check(con,r)
        # Current locks must not overlap declared paths. It is OK that lock is not acquired until dispatch; this is a preflight only.
        paths=json.loads(r['paths_json'])
        for lock in con.execute('SELECT path,task_id FROM locks'):
            if lock['task_id']==task_id: continue
            for p in paths:
                if overlaps(p,lock['path']): raise RuntimeError(f'path conflict: {p} overlaps {lock["path"]}, held by {lock["task_id"]}')
        old=r['status']; con.execute("UPDATE tasks SET status='READY',updated_at=?,last_note=? WHERE id=?",(now(),'Dependencies and paths checked',task_id))
        event(con,task_id,'STATUS_CHANGED',old,'READY','Dependencies and paths checked')
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{task_id}: READY')

def default_holder():
    """A locatable default holder: this host and this process (§2.2)."""
    override=os.environ.get('ORCH_HOLDER','').strip()
    if override: return override
    return f'{socket.gethostname()}:{os.getpid()}'

def task_dispatch(root, task_id, options=None):
    require_local_scheduler(root,'task dispatch')
    # The holder is only validated when the task declares resources, so a
    # path-lock-only dispatch keeps its previous behaviour byte for byte.
    holder=getattr(options,'holder',None) or default_holder()
    con=connect(root)
    acquired=None
    try:
        con.execute('BEGIN IMMEDIATE'); r=get_task(con,task_id)
        if r['status']!='READY': raise RuntimeError(f'only READY tasks can be dispatched; current status is {r["status"]}')
        dependency_check(con,r)
        paths=json.loads(r['paths_json'])
        conflicts=[]
        locks=con.execute('SELECT path,task_id FROM locks WHERE task_id<>?',(task_id,)).fetchall()
        for p in paths:
            for lock in locks:
                if overlaps(p,lock['path']): conflicts.append((p,lock['path'],lock['task_id']))
        if conflicts:
            detail='; '.join(f'{p} overlaps {held} (task {tid})' for p,held,tid in conflicts)
            raise RuntimeError('dispatch refused; path conflict: '+detail)
        # Resource locks are an independent second gate (contract §1.2): the path
        # check above keeps its wording and runs first, then the resource set is
        # acquired in the same transaction. A task without a declaration never
        # touches the resource tables.
        specs=declared_resources(r)+list(getattr(options,'resource',None) or [])
        if specs:
            holder=validate_holder(holder)
            entries=resolve_resource_entries(root,specs)
            acquired=acquire_resource_set(con,task_id,entries,holder,
                                          actor=getattr(options,'actor',None) or 'orchestrator',
                                          reason='dispatch',evidence_ref=getattr(options,'evidence',None) or '(none)')
        for p in paths:
            con.execute('INSERT OR REPLACE INTO locks(path,task_id,created_at) VALUES(?,?,?)',(p,task_id,now()))
        old=r['status']; con.execute("UPDATE tasks SET status='DISPATCHED',updated_at=?,last_note=? WHERE id=?",(now(),'Atomic path locks acquired before dispatch',task_id))
        event(con,task_id,'TASK_DISPATCHED',old,'DISPATCHED','Atomic path locks acquired',
              {'paths':paths,'resources':acquired['entries'] if acquired else [],'resource_status':acquired['status'] if acquired else None,
               'holder':holder if acquired else None,'fingerprint':acquired['fingerprint'] if acquired else None})
        con.execute('COMMIT')
    except ResourceDenied:
        # The denial audit trail (and any quarantine transition) is already written
        # in this transaction; commit it so the evidence survives. No lock row was
        # inserted before the whole set passed, so resource_locks stays净空.
        con.execute('COMMIT'); raise
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{task_id}: DISPATCHED; locks acquired for {", ".join(paths)}')
    if acquired:
        wanted=', '.join(f'{e["resource_id"]}:{e["mode"]}' for e in acquired['entries'])
        print(f'{task_id}: resource locks {acquired["status"]} for {wanted} '
              f'(holder {holder}, fingerprint {acquired["fingerprint"][:12]})')
    print('Now pass the self-contained task packet to a real Execution Agent. This command does not launch agents.')

def set_workspace(root, args):
    require_local_scheduler(root,'task workspace')
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE'); r=get_task(con,args.id)
        if r['status'] in TERMINAL: raise RuntimeError('cannot update workspace for terminal task')
        approved=r['approved_sha']
        status=r['status']; oldsha=r['head_sha']
        newsha=args.head_sha or oldsha
        if status=='APPROVED' and newsha and newsha!=approved:
            con.execute("UPDATE tasks SET status='MASTER_REVIEW',approved_sha='',head_sha=?,branch=COALESCE(NULLIF(?,''),branch),worktree=COALESCE(NULLIF(?,''),worktree),updated_at=?,last_note=? WHERE id=?",
                (newsha,args.branch or '',args.worktree or '',now(),'head changed; previous approval invalidated',args.id))
            event(con,args.id,'APPROVAL_INVALIDATED','APPROVED','MASTER_REVIEW','head changed',{'old_head':oldsha,'new_head':newsha})
        else:
            con.execute('UPDATE tasks SET head_sha=?,branch=COALESCE(NULLIF(?,\'\'),branch),worktree=COALESCE(NULLIF(?,\'\'),worktree),updated_at=?,last_note=? WHERE id=?',
                (newsha,args.branch or '',args.worktree or '',now(),args.note or 'workspace updated',args.id))
            event(con,args.id,'WORKSPACE_UPDATED',status,status,args.note or 'workspace updated',{'branch':args.branch,'worktree':args.worktree,'head_sha':newsha})
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: workspace updated')

def task_status(root, args):
    require_local_scheduler(root,'task status')
    dest=args.status.upper()
    if dest not in TRANSITIONS: raise ValueError(f'unknown status: {dest}')
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE'); r=get_task(con,args.id); old=r['status']
        if dest in {'APPROVED','MERGED','VERIFIED','BLOCKED','NEEDS_HUMAN_DECISION','FAILED','CANCELED'} and not (args.note or '').strip():
            raise RuntimeError(f'{dest} requires an audit note via --note (include decision / evidence / recovery reason)')
        if dest not in TRANSITIONS[old]: raise RuntimeError(f'illegal transition {old} -> {dest}; allowed: {", ".join(sorted(TRANSITIONS[old])) or "none"}')
        if dest=='READY': raise RuntimeError('use `task ready <ID>` so dependency and path-conflict checks cannot be bypassed')
        if dest=='APPROVED':
            sha=args.head_sha or r['head_sha']
            if not sha: raise RuntimeError('APPROVED requires a current --head-sha')
            if r['head_sha'] and sha!=r['head_sha']: raise RuntimeError(f'approval SHA {sha} does not match current task head {r["head_sha"]}')
            con.execute('UPDATE tasks SET approved_sha=? WHERE id=?',(sha,args.id))
        elif old=='APPROVED' and dest=='MERGED':
            if not r['approved_sha'] or r['approved_sha'] != r['head_sha']:
                raise RuntimeError('approved SHA does not match current task head; return to MASTER_REVIEW')
            if not getattr(args, 'merge_sha', None):
                raise RuntimeError('MERGED requires --merge-sha for the resulting local merge commit')
            if not re.fullmatch(r'[0-9a-fA-F]{40}', args.merge_sha):
                raise RuntimeError('MERGED requires a full 40-character hex --merge-sha (abbreviated SHAs are refused)')
            args.merge_sha=args.merge_sha.lower()
        if dest=='IN_PROGRESS' and old=='MASTER_REVIEW':
            retry_cap=read_retry_cap(root)
            if r['retries'] >= retry_cap:
                dest='BLOCKED'
                retries=r['retries']
                args.note=(args.note + '; ' if args.note else '') + f'Retry cap reached after {retry_cap} review-fix rounds; manual decision required.'
            else:
                retries=r['retries']+1
                con.execute('UPDATE tasks SET retries=? WHERE id=?',(retries,args.id))
        else: retries=r['retries']
        new_head=args.head_sha or r['head_sha']
        if old=='APPROVED' and args.head_sha and dest!='MERGED' and args.head_sha!=r['approved_sha']:
            raise RuntimeError('head changed after approval; approval is invalid')
        note=args.note or ''
        if old=='APPROVED' and dest=='MASTER_REVIEW':
            con.execute("UPDATE tasks SET approved_sha='' WHERE id=?",(args.id,))
        if dest=='MERGED':
            con.execute('UPDATE tasks SET status=?,merged_sha=?,updated_at=?,last_note=? WHERE id=?',(dest,getattr(args, 'merge_sha', None),now(),note,args.id))
        else:
            con.execute('UPDATE tasks SET status=?,head_sha=?,updated_at=?,last_note=? WHERE id=?',(dest,new_head,now(),note,args.id))
        event(con,args.id,'STATUS_CHANGED',old,dest,note,{'head_sha':new_head,'merge_sha':getattr(args, 'merge_sha', None),'retries':retries})
        released_resources=[]
        if dest in TERMINAL:
            con.execute('DELETE FROM locks WHERE task_id=?',(args.id,))
            event(con,args.id,'LOCKS_RELEASED',dest,dest,'terminal task status; path locks released')
            # Only VERIFIED / CANCELED auto-release resource locks (contract §5.2).
            held=held_resource_rows(con,args.id)
            if held:
                release_reason='NORMAL_COMPLETE' if dest=='VERIFIED' else 'CANCELED'
                released_resources=release_resource_set(con,args.id,release_reason,
                                                        note or f'task {dest}: resource locks released',
                                                        'orchestrator','task status '+dest,rows=held)
        elif dest=='NEEDS_HUMAN_DECISION':
            # The holder may still be alive; the lock is frozen, not released (§5.2).
            for row in held_resource_rows(con,args.id):
                quarantine_resource_row(con,row,'orchestrator',
                                        f'task {args.id} is NEEDS_HUMAN_DECISION: {note or "awaiting human decision"}',
                                        'task status NEEDS_HUMAN_DECISION',now())
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: {old} -> {dest}' + (f' (head {args.head_sha})' if args.head_sha and dest!='MERGED' else f' (merge {args.merge_sha})' if getattr(args, 'merge_sha', None) else ''))
    if released_resources:
        print(f'{args.id}: released {len(released_resources)} resource lock(s): {", ".join(released_resources)}')

def task_amend(root, args):
    """Correct an already-recorded merge SHA on a MERGED / VERIFIED task.

    The only audited entry point for changing merged_sha after MERGED: requires a
    full 40-hex SHA and a --note reason, and records a RECORD_AMENDED event with
    the old/new values. No other command mutates merged_sha outside MERGED set-up.
    """
    require_local_scheduler(root,'task amend')
    sha=(args.merge_sha or '').strip()
    if not re.fullmatch(r'[0-9a-fA-F]{40}', sha):
        raise RuntimeError('amend requires a full 40-character hex --merge-sha')
    if not (args.note or '').strip():
        raise RuntimeError('amend requires --note explaining the merge SHA correction')
    new=sha.lower()
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE'); r=get_task(con,args.id)
        if r['status'] not in {'MERGED','VERIFIED'}:
            raise RuntimeError(f'merge SHA can only be amended on MERGED or VERIFIED tasks; current status is {r["status"]}')
        old=r['merged_sha']
        con.execute('UPDATE tasks SET merged_sha=?,updated_at=?,last_note=? WHERE id=?',(new,now(),args.note,args.id))
        event(con,args.id,'RECORD_AMENDED',r['status'],r['status'],args.note,{'field':'merged_sha','old':old,'new':new})
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: merge SHA amended {old or "(unset)"} -> {new}')

def find_task_resource_lock(con, task_id, reference):
    """Resolve `--resource` as a lock_id or a resource_id held by this task."""
    if '#' in reference:
        row=resource_row(con,reference)
        if row['task_id']!=task_id:
            raise ResourceError('LOCK_NOT_HELD',f'{reference} belongs to task {row["task_id"]}, not {task_id}')
        return row
    resource_id=normalize_resource_id(reference)
    rows=[row for row in held_resource_rows(con,task_id) if row['resource_id']==resource_id]
    if not rows:
        raise ResourceError('LOCK_NOT_HELD',f'task {task_id} holds no non-RELEASED lock on {resource_id}')
    return rows[0]

def unlock(root, args):
    require_local_scheduler(root,'task unlock')
    con=connect(root)
    resource=getattr(args,'resource',None)
    try:
        con.execute('BEGIN IMMEDIATE'); r=get_task(con,args.id)
        if r['status'] not in TERMINAL and not args.force:
            raise RuntimeError('task is not terminal; verify that its execution has stopped, then use --force with a reason if unlock is truly required')
        if resource:
            row=find_task_resource_lock(con,args.id,resource)
            evidence=getattr(args,'evidence',None) or '(none)'
            authorized_by=getattr(args,'authorized_by',None)
            if args.force:
                force_unlock_resource(con,row['lock_id'],args.reason,authorized_by or default_holder(),
                                      evidence,authorized_by=authorized_by)
                outcome='forced'
            else:
                release_reason='CANCELED' if r['status']=='CANCELED' else 'NORMAL_COMPLETE'
                release_resource_set(con,args.id,release_reason,args.reason,'orchestrator',evidence,rows=[row])
                outcome='released'
            event(con,args.id,'MANUAL_UNLOCK',r['status'],r['status'],args.reason,
                  {'resource':row['resource_id'],'mode':row['mode'],'lock_id':row['lock_id'],
                   'outcome':outcome,'force':bool(args.force),'authorized_by':authorized_by})
            con.execute('COMMIT')
            print(f'{args.id}: resource lock {row["resource_id"]} ({row["mode"]}) {outcome}; reason recorded')
            return
        rows=con.execute('SELECT path FROM locks WHERE task_id=?',(args.id,)).fetchall()
        con.execute('DELETE FROM locks WHERE task_id=?',(args.id,))
        event(con,args.id,'MANUAL_UNLOCK',r['status'],r['status'],args.reason,{'released_paths':[x['path'] for x in rows],'force':args.force})
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: released {len(rows)} path lock(s); reason recorded')

def lock_line(row, now_stamp=None):
    """One inspection line: id, mode, state, lease, holder, blocking effect (§4.3)."""
    overdue=''
    if row['state']=='ACTIVE' and now_stamp and row['lease_until']<=now_stamp:
        overdue=' lease OVERDUE (expiry pending)'
    line=(f"{row['resource_id']}\t{row['mode']}\t{row['state']}\tlease_until={row['lease_until']}\t"
          f"renewed_at={row['renewed_at']}\tholder={row['holder']}\ttask={row['task_id']}\t"
          f"backend={row['backend']}\tlock_id={row['lock_id']}{overdue}")
    if row['state']!='RELEASED':
        line+=('\tblocks=exclusive+shared' if row['mode']=='exclusive' else '\tblocks=exclusive')
    return line

def task_resources(root, args):
    """Inspect resource locks: every non-RELEASED row plus its blocking reason."""
    con=connect(root)
    edition=None
    try:
        edition=read_project_edition(root)
    except EditionError:
        edition=None                     # an unreadable edition must not hide the listing
    try:
        if edition=='local':
            # Expiring is a state transition, not a release; only the local edition
            # owns this database and may write it.
            con.execute('BEGIN IMMEDIATE'); expire_due_locks(con); con.execute('COMMIT')
        stamp=now()
        rows=con.execute("SELECT * FROM resource_locks WHERE state<>'RELEASED' ORDER BY resource_id, mode, task_id"
                         ).fetchall()
        if getattr(args,'all',False):
            rows=con.execute('SELECT * FROM resource_locks ORDER BY resource_id, mode, task_id').fetchall()
        if getattr(args,'resource',None):
            resource_id=normalize_resource_id(args.resource)
            rows=[row for row in rows if row['resource_id']==resource_id]
        if not rows:
            print('No resource locks held.')
            print(resource_backend_note(edition or 'local') if edition else
                  'the project edition cannot be read; this listing is read-only and no lease was migrated.')
            return
        print('resource_id\tmode\tstate\tlease\tholder\ttask\tbackend\tlock_id\tblocking')
        for row in rows:
            print(lock_line(row,stamp))
        print(f'Total: {len(rows)} row(s); generated with the frozen contract docs/design/RESOURCE-LOCKS.md.')
        print(resource_backend_note(edition or 'local') if edition else
              'the project edition cannot be read; this listing is read-only and no lease was migrated.')
    finally: con.close()

def task_renew(root, args):
    require_local_scheduler(root,'task renew')
    holder=validate_holder(getattr(args,'holder',None) or default_holder())
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE')
        ttl=resource_config(root,require_ttl=True)[LEASE_TTL_KEY]
        renewed=renew_resource_locks(con,args.id,holder,ttl,getattr(args,'reason','') or 'periodic renewal',
                                     getattr(args,'evidence',None) or '(none)',actor=holder)
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: renewed {len(renewed)} resource lock(s) for {ttl}s as holder {holder}')

def task_reclaim(root, args):
    require_local_scheduler(root,'task reclaim')
    grace=resource_config(root,require_grace=True)[LEASE_GRACE_KEY]
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE'); expire_due_locks(con)
        row=find_task_resource_lock(con,args.id,args.resource)
        reclaim_resource_lock(con,row['lock_id'],args.reason,getattr(args,'evidence',None) or '(none)',grace,
                              actor=getattr(args,'actor',None) or 'orchestrator')
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    print(f'{args.id}: reclaimed {row["resource_id"]} ({row["mode"]}) after a {grace}s cool-down; evidence recorded')

def task_reconcile(root, args):
    """Rebuild lock facts without trusting memory; quarantine what cannot be proven."""
    require_local_scheduler(root,'task reconcile')
    con=connect(root)
    try:
        con.execute('BEGIN IMMEDIATE')
        report=reconcile_resource_locks(con,getattr(args,'reason','cross-session recovery'),
                                        getattr(args,'evidence',None) or '(none)')
        con.execute('COMMIT')
    except Exception:
        safe_rollback(con); raise
    finally: con.close()
    if not report:
        print('No non-RELEASED resource locks to reconcile.')
        return
    print('lock_id\tverdict\taction')
    for item in report:
        print(f"{item['lock_id']}\t{item['verdict']}\t{item['action']}")
    print(f'Reconciled {len(report)} lock(s); verdicts come from the lock table and its audit events only.')

# --- named resource locks (contract: docs/design/RESOURCE-LOCKS.md, frozen v1) ---
# The contract owns the semantics: data model (§2), conflict matrix (§3), atomic
# set acquisition (§4), state machine (§5), recovery (§6), backend boundaries (§7).
# This section only implements it. Error codes below are reported verbatim so a
# caller can act on the outcome instead of parsing prose.
RESOURCE_MODES = ('shared', 'exclusive')
RESOURCE_STATES = ('ACTIVE', 'EXPIRED', 'QUARANTINED', 'RELEASED')
RESOURCE_RELEASE_REASONS = ('NORMAL_COMPLETE', 'CANCELED', 'FORCED_UNLOCK', 'LEASE_RECLAIMED',
                            'TRANSFER', 'MIGRATION_DROP')
RESOURCE_EVENT_TYPES = (
    'LOCK_REQUESTED', 'LOCK_GRANTED', 'LOCK_DENIED_CONFLICT', 'LOCK_SET_ROLLED_BACK',
    'LOCK_RENEWED', 'LOCK_EXPIRED', 'LOCK_RECLAIMED', 'LOCK_RELEASED', 'LOCK_FORCED',
    'LOCK_QUARANTINED', 'LOCK_ADOPTED', 'LOCK_TRANSITION_REJECTED', 'RESOURCE_REGISTERED',
)
# RELEASED is the only terminal state; a released record is never revived, a
# re-acquisition uses a new generation (§5.1).
RESOURCE_TRANSITIONS = {
    'ACTIVE': {'ACTIVE', 'EXPIRED', 'RELEASED', 'QUARANTINED'},
    'EXPIRED': {'ACTIVE', 'RELEASED', 'QUARANTINED'},
    'QUARANTINED': {'ACTIVE', 'RELEASED'},
    'RELEASED': set(),
}
RESOURCE_TYPE_RE = re.compile(r'[a-z][a-z0-9-]{0,15}')
RESOURCE_NAME_RE = re.compile(r'[a-z0-9][a-z0-9._/-]{0,127}')
# Characters that would silently turn one resource into several / a pattern.
# They are rejected rather than rewritten (§2.1 rule 1).
RESOURCE_REJECT_CHARS = (' ', '\t', '\r', '\n', '\\', '?', '*', '#', '[')
# A holder must be locatable; generic role names are refused (§2.2, §6.3).
HOLDER_GENERIC = {'', 'agent', 'subagent', 'assistant', 'ai', 'unknown', 'none', '(none)', 'n/a'}
LEASE_TTL_KEY = 'resource_lease_ttl'
LEASE_GRACE_KEY = 'resource_reclaim_grace'
LEASE_TTL_MAX = 604800          # 7 days: a lease longer than this is not a lease.
LEASE_GRACE_MAX = 86400         # 1 day of reclaim cool-down is already generous.
RESOURCE_SCHEMA = '''
CREATE TABLE IF NOT EXISTS resource_locks(
  lock_id TEXT PRIMARY KEY,
  resource_id TEXT NOT NULL,
  mode TEXT NOT NULL CHECK(mode IN ('shared','exclusive')),
  holder TEXT NOT NULL,
  task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  acquired_at TEXT NOT NULL,
  lease_until TEXT NOT NULL,
  renewed_at TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN ('ACTIVE','EXPIRED','QUARANTINED','RELEASED')),
  release_reason TEXT NOT NULL DEFAULT '(none)',
  released_at TEXT NOT NULL DEFAULT '(none)',
  reason TEXT NOT NULL,
  evidence_ref TEXT NOT NULL,
  backend TEXT NOT NULL CHECK(backend IN ('local','github')),
  generation INTEGER NOT NULL DEFAULT 0,
  UNIQUE(resource_id, task_id, mode, generation)
);
CREATE UNIQUE INDEX IF NOT EXISTS resource_locks_single_exclusive
  ON resource_locks(resource_id) WHERE mode='exclusive' AND state<>'RELEASED';
CREATE INDEX IF NOT EXISTS resource_locks_resource_idx ON resource_locks(resource_id, state);
CREATE INDEX IF NOT EXISTS resource_locks_task_idx2 ON resource_locks(task_id, state);
CREATE INDEX IF NOT EXISTS resource_locks_lease_idx ON resource_locks(lease_until);
CREATE TABLE IF NOT EXISTS resource_lock_events(
  event_id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_type TEXT NOT NULL,
  at TEXT NOT NULL,
  actor TEXT NOT NULL,
  task_id TEXT NOT NULL,
  resource_id TEXT NOT NULL,
  mode TEXT NOT NULL,
  prev_state TEXT NOT NULL,
  new_state TEXT NOT NULL,
  request_fingerprint TEXT NOT NULL,
  reason TEXT NOT NULL,
  evidence_ref TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS resource_lock_events_task_idx ON resource_lock_events(task_id, event_id);
CREATE INDEX IF NOT EXISTS resource_lock_events_resource_idx ON resource_lock_events(resource_id, event_id);
'''

class ResourceError(RuntimeError):
    """A resource-lock operation was refused; `code` is the machine-readable reason."""

    def __init__(self, code, message):
        self.code = code
        super().__init__(f'{code}: {message}')

class ResourceDenied(ResourceError):
    """Refusal raised *after* the denial audit trail was written in the same transaction.

    Callers must COMMIT (never roll back) so the LOCK_DENIED_CONFLICT /
    LOCK_SET_ROLLED_BACK / quarantine evidence survives, while resource_locks
    stays净空 because no lock row is written before every entry passed (§4.4).
    """

def _ts(value):
    return value.astimezone(timezone.utc).isoformat(timespec='seconds')

def add_seconds(stamp, seconds):
    return _ts(datetime.fromisoformat(stamp) + timedelta(seconds=int(seconds)))

def ensure_resource_schema(con):
    """Create the resource tables incrementally; an old database keeps working.

    Called from connect()/init() only, i.e. outside an explicit transaction:
    `executescript` would implicitly commit the caller's transaction. A missing
    table must never be read as "no conflict" (§8.2 rule 5), so this runs before
    any lock judgement.
    """
    tables = {row[0] for row in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    if 'tasks' not in tables:
        return
    columns = {row[1] for row in con.execute('PRAGMA table_info(tasks)')}
    need_tables = not {'resource_locks', 'resource_lock_events'} <= tables
    need_column = 'resources_json' not in columns
    if not (need_tables or need_column):
        return
    if need_tables:
        con.executescript(RESOURCE_SCHEMA)
    if need_column:
        con.execute("ALTER TABLE tasks ADD COLUMN resources_json TEXT NOT NULL DEFAULT '[]'")

def normalize_resource_id(raw):
    """Return the canonical `<type>:<name>` form or raise RESOURCE_ID_INVALID.

    Rules are applied to the raw string, never to a cleaned-up one: silently
    lowering case or trimming spaces would create two IDs for one resource.
    """
    if not isinstance(raw, str) or not raw:
        raise ResourceError('RESOURCE_ID_INVALID', f'resource ID must be a non-empty string, got {raw!r}')
    text = unicodedata.normalize('NFC', raw)
    if text != text.strip():
        raise ResourceError('RESOURCE_ID_INVALID', f'resource ID {raw!r} has leading/trailing whitespace; write it without spaces')
    if text != text.lower():
        raise ResourceError('RESOURCE_ID_INVALID', f'resource ID {raw!r} contains uppercase characters; only lowercase is accepted')
    for char in RESOURCE_REJECT_CHARS:
        if char in text:
            raise ResourceError('RESOURCE_ID_INVALID', f'resource ID {raw!r} contains forbidden character {char!r}')
    res_type, sep, name = text.partition(':')
    if not sep:
        raise ResourceError('RESOURCE_ID_INVALID', f'resource ID {raw!r} must look like <type>:<name> (for example db:test/mysql-main)')
    if not RESOURCE_TYPE_RE.fullmatch(res_type):
        raise ResourceError('RESOURCE_ID_INVALID', f'resource type {res_type!r} in {raw!r} must match {RESOURCE_TYPE_RE.pattern}')
    if not RESOURCE_NAME_RE.fullmatch(name):
        raise ResourceError('RESOURCE_ID_INVALID', f'resource name {name!r} in {raw!r} must match {RESOURCE_NAME_RE.pattern}')
    parts = name.split('/')
    if any(part in ('', '.', '..') for part in parts):
        raise ResourceError('RESOURCE_ID_INVALID', f'resource name {name!r} in {raw!r} has an empty or dots-only path segment')
    return text

def parse_resource_spec(spec):
    """Parse `<resource_id>:<mode>[:<ttl>]`; the ID itself may contain ':'."""
    if not isinstance(spec, str) or not spec.strip():
        raise ResourceError('RESOURCE_SPEC_INVALID', f'empty resource declaration; expected <resource_id>:<mode>[:<ttl>]')
    parts = spec.split(':')
    ttl = None
    if len(parts) >= 3 and parts[-2] in RESOURCE_MODES and parts[-1].isdigit():
        ttl = int(parts[-1]); parts = parts[:-1]
    if len(parts) < 2 or parts[-1] not in RESOURCE_MODES:
        raise ResourceError('RESOURCE_SPEC_INVALID',
                            f'resource declaration {spec!r} must be <resource_id>:<mode>[:<ttl>] with mode shared or exclusive')
    mode = parts[-1]
    entry = {'resource_id': normalize_resource_id(':'.join(parts[:-1])), 'mode': mode, 'ttl': ttl}
    if ttl is not None and not 1 <= ttl <= LEASE_TTL_MAX:
        raise ResourceError('RESOURCE_CONFIG_INVALID', f'lease ttl {ttl} in {spec!r} must be between 1 and {LEASE_TTL_MAX} seconds')
    return entry

def parse_resource_specs(specs):
    """Normalise a declaration list into one entry per resource_id (§4.1)."""
    entries, seen = [], {}
    for spec in specs or []:
        entry = parse_resource_spec(spec)
        rid = entry['resource_id']
        if rid in seen:
            if seen[rid] != entry['mode']:
                raise ResourceError('RESOURCE_SPEC_INVALID',
                                    f'resource {rid} is declared twice with different modes ({seen[rid]} and {entry["mode"]})')
            continue
        seen[rid] = entry['mode']
        entries.append(entry)
    return entries

def resource_config(root, require_ttl=False, require_grace=False):
    """Read resource_lease_ttl / resource_reclaim_grace from .orchestration/config.json.

    Both are explicit configuration: there is no implicit default (§6.2). A task
    that declares no resource never reads them, so existing roots are untouched.
    """
    data = {}
    try:
        raw = json.loads(config_path(root).read_text(encoding='utf-8'))
        if isinstance(raw, dict):
            data = raw
    except FileNotFoundError:
        pass
    except Exception:
        data = {}
    cfg, problems = {}, []
    for key, low, high, required in (
            (LEASE_TTL_KEY, 1, LEASE_TTL_MAX, require_ttl),
            (LEASE_GRACE_KEY, 0, LEASE_GRACE_MAX, require_grace)):
        value = data.get(key)
        valid = isinstance(value, int) and not isinstance(value, bool) and low <= value <= high
        if valid:
            cfg[key] = value
        elif required:
            problems.append(f'{key} must be an integer between {low} and {high} in {config_path(root)}')
        else:
            cfg[key] = None
    if problems:
        raise ResourceError('RESOURCE_CONFIG_INVALID',
                            'resource lock configuration is missing or illegal: ' + '; '.join(problems) +
                            '; add it to .orchestration/config.json (a lease without an explicit ttl or cool-down is refused)')
    return cfg

def resolve_resource_entries(root, specs):
    """Validate specs and fill every ttl, sorted by UTF-8 byte order (§4.2)."""
    entries = parse_resource_specs(specs)
    if not entries:
        return []
    cfg = resource_config(root, require_ttl=any(e['ttl'] is None for e in entries))
    for entry in entries:
        entry['ttl'] = entry['ttl'] if entry['ttl'] is not None else cfg[LEASE_TTL_KEY]
    return order_resource_entries(entries)

def order_resource_entries(entries):
    """Deterministic order: UTF-8 byte order of resource_id (§4.2)."""
    return sorted(entries, key=lambda e: (e['resource_id'].encode('utf-8'), e['mode']))

def resource_fingerprint(entries):
    """SHA-256 of the normalised, sorted `resource_id:mode` set (§2.3)."""
    payload = '\n'.join(f'{e["resource_id"]}:{e["mode"]}' for e in order_resource_entries(entries))
    return hashlib.sha256(payload.encode('utf-8')).hexdigest()

def validate_holder(holder):
    text = (holder or '').strip()
    if not text or text.lower() in HOLDER_GENERIC:
        raise ResourceError('HOLDER_INVALID',
                            f'holder {holder!r} cannot be located; pass a runtime instance ID, session ID or operator ID '
                            '(generic role names such as "agent" are refused)')
    return text

def resource_event(con, event_type, task_id, resource_id, mode, prev_state, new_state,
                   fingerprint, reason, evidence_ref, actor, at, event_id=None):
    """Append one immutable audit event; a repeated event_id is a no-op (§2.3)."""
    if event_type not in RESOURCE_EVENT_TYPES:
        raise ResourceError('RESOURCE_EVENT_INVALID', f'unregistered resource lock event type: {event_type}')
    if not (actor or '').strip():
        raise ResourceError('RESOURCE_EVENT_INVALID', f'{event_type} for {resource_id} requires a non-empty actor')
    if event_id is not None:
        existing = con.execute('SELECT * FROM resource_lock_events WHERE event_id=?', (event_id,)).fetchone()
        if existing:
            return existing['event_id']
        con.execute('INSERT INTO resource_lock_events(event_id,event_type,at,actor,task_id,resource_id,mode,'
                    'prev_state,new_state,request_fingerprint,reason,evidence_ref) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',
                    (event_id, event_type, at, actor, task_id, resource_id, mode, prev_state, new_state,
                     fingerprint, reason, evidence_ref))
        return event_id
    cursor = con.execute('INSERT INTO resource_lock_events(event_type,at,actor,task_id,resource_id,mode,'
                         'prev_state,new_state,request_fingerprint,reason,evidence_ref) VALUES(?,?,?,?,?,?,?,?,?,?,?)',
                         (event_type, at, actor, task_id, resource_id, mode, prev_state, new_state,
                          fingerprint, reason, evidence_ref))
    return cursor.lastrowid

def declared_resources(task_row):
    raw = task_row['resources_json'] if 'resources_json' in task_row.keys() else '[]'
    try:
        return list(json.loads(raw or '[]'))
    except Exception:
        raise ResourceError('RESOURCE_SPEC_INVALID', f'task {task_row["id"]} has unreadable declared_resources {raw!r}')

def held_resource_rows(con, task_id):
    return con.execute("SELECT * FROM resource_locks WHERE task_id=? AND state<>'RELEASED' "
                       "ORDER BY resource_id, mode", (task_id,)).fetchall()

def resource_row(con, lock_id):
    row = con.execute('SELECT * FROM resource_locks WHERE lock_id=?', (lock_id,)).fetchone()
    if not row:
        raise ResourceError('LOCK_NOT_FOUND', f'unknown resource lock: {lock_id}')
    return row

def transition_resource(con, row, new_state, event_type, actor, reason, evidence_ref, at,
                        fingerprint='', **updates):
    """Apply one whitelisted state transition and its audit event (§5.1)."""
    prev = row['state']
    if new_state not in RESOURCE_TRANSITIONS.get(prev, set()):
        resource_event(con, 'LOCK_TRANSITION_REJECTED', row['task_id'], row['resource_id'], row['mode'],
                       prev, new_state, fingerprint, f'{prev} -> {new_state} is not an allowed lock transition: {reason}',
                       evidence_ref, actor, at)
        raise ResourceError('TRANSITION_REJECTED',
                            f'{row["lock_id"]}: {prev} -> {new_state} is not allowed; the lock record is kept unchanged')
    assignments = {'state': new_state, 'reason': reason, 'evidence_ref': evidence_ref}
    assignments.update(updates)
    columns = ', '.join(f'{name}=?' for name in assignments)
    con.execute(f'UPDATE resource_locks SET {columns} WHERE lock_id=?',
                (*assignments.values(), row['lock_id']))
    resource_event(con, event_type, row['task_id'], row['resource_id'], row['mode'], prev, new_state,
                   fingerprint, reason, evidence_ref, actor, at)
    return new_state

def expire_due_locks(con, at=None, actor='orchestrator'):
    """ACTIVE -> EXPIRED for every lease that passed; expiry is not a release (§6.2)."""
    at = at or now()
    rows = con.execute("SELECT * FROM resource_locks WHERE state='ACTIVE' AND lease_until<=? "
                       "ORDER BY resource_id, task_id", (at,)).fetchall()
    expired = []
    for row in rows:
        transition_resource(con, row, 'EXPIRED', 'LOCK_EXPIRED', actor,
                            f'lease_until {row["lease_until"]} passed without renewal; the lock stays held and still conflicts',
                            row['evidence_ref'], at)
        expired.append(row['lock_id'])
    return expired

def conflicting_resource_row(con, resource_id, mode, task_id):
    """First non-RELEASED row blocking (resource_id, mode) for task_id (§3.1)."""
    for row in con.execute("SELECT * FROM resource_locks WHERE resource_id=? AND state<>'RELEASED' "
                           "AND task_id<>? ORDER BY task_id, mode", (resource_id, task_id)):
        if mode == 'exclusive' or row['mode'] == 'exclusive':
            return row
    return None

def _conflict_detail(row):
    return (f'resource {row["resource_id"]} is held by task {row["task_id"]} '
            f'(holder {row["holder"]}, mode {row["mode"]}, state {row["state"]}, lease_until {row["lease_until"]}); '
            'next: wait for that release, declare a different resource, or serialize the two tasks')

def _deny(con, task_id, entry, fingerprint, actor, at, evidence_ref, code, detail, attempted, total):
    """Write the denial evidence, then raise; caller COMMITs so it survives."""
    resource_event(con, 'LOCK_DENIED_CONFLICT', task_id, entry['resource_id'], entry['mode'],
                   '(none)', '(none)', fingerprint, f'DENIED {code}: {detail}', evidence_ref, actor, at)
    resource_event(con, 'LOCK_SET_ROLLED_BACK', task_id, entry['resource_id'], entry['mode'],
                   '(none)', '(none)', fingerprint,
                   f'whole set refused after {attempted} of {total} resource(s); no lock row was written for task {task_id}',
                   evidence_ref, actor, at)
    raise ResourceDenied(code, detail)

def quarantine_resource_row(con, row, actor, reason, evidence_ref, at, fingerprint=''):
    if row['state'] == 'QUARANTINED':
        return row['state']
    return transition_resource(con, row, 'QUARANTINED', 'LOCK_QUARANTINED', actor, reason, evidence_ref,
                               at, fingerprint)

def acquire_resource_set(con, task_id, entries, holder, actor='orchestrator', reason='dispatch',
                         evidence_ref='(none)', backend='local', at=None):
    """Atomically acquire one declared resource set; caller owns the transaction.

    Conflict reads and lock writes share the caller's BEGIN IMMEDIATE transaction
    (§3.2). No row is inserted before *every* entry passed the matrix, so a
    denial leaves resource_locks净空 while its events are committed (§4.4).
    """
    at = at or now()
    holder = validate_holder(holder)
    expire_due_locks(con, at)
    ordered = order_resource_entries(entries)
    fingerprint = resource_fingerprint(ordered)
    held = held_resource_rows(con, task_id)
    if held:
        holders = sorted({row['holder'] for row in held})
        if holder not in holders:
            for row in held:
                quarantine_resource_row(con, row, actor,
                                        f'duplicate ownership: task {task_id} already holds {row["resource_id"]} as '
                                        f'holder {row["holder"]}; new holder {holder} must not write (§6.3)',
                                        evidence_ref, at, fingerprint)
            raise ResourceDenied('DUPLICATE_OWNERSHIP',
                                 f'task {task_id} already holds resource locks as holder {", ".join(holders)}; '
                                 f'holder {holder} cannot continue; resolve the duplicate owner manually')
        wanted = {entry['resource_id']: entry for entry in ordered}
        missing = [entry for entry in ordered if entry['resource_id'] not in {r['resource_id'] for r in held}]
        dropped = sorted({row['resource_id'] for row in held} - set(wanted))
        mode_changed = sorted({row['resource_id'] for row in held
                               if row['resource_id'] in wanted and wanted[row['resource_id']]['mode'] != row['mode']})
        upgrade = sorted({row['resource_id'] for row in held
                          if row['resource_id'] in wanted and row['mode'] == 'shared'
                          and wanted[row['resource_id']]['mode'] == 'exclusive'})
        if upgrade:
            entry = wanted[upgrade[0]]
            _deny(con, task_id, entry, fingerprint, actor, at, evidence_ref, 'UPGRADE_FORBIDDEN',
                  f'{entry["resource_id"]} is already held as shared; in-place upgrade to exclusive is forbidden — '
                  'release the set and request it again as a whole', 1, len(ordered))
        if dropped or mode_changed:
            bad = dropped + mode_changed
            declared_text = '; '.join(f'{e["resource_id"]}:{e["mode"]}' for e in ordered) or '(empty)'
            for row in held:
                quarantine_resource_row(con, row, actor,
                                        f'LOCK_SET_MISMATCH: task {task_id} already holds '
                                        f'{row["resource_id"]}:{row["mode"]} but the new declaration is {declared_text}',
                                        evidence_ref, at, fingerprint)
            raise ResourceDenied('LOCK_SET_MISMATCH',
                                 f'task {task_id} already holds a different resource set (differs on {", ".join(bad)}); '
                                 'the held locks are now QUARANTINED and need a human decision')
        if not missing:
            for entry in ordered:
                row = next(r for r in held
                           if r['resource_id'] == entry['resource_id'] and r['mode'] == entry['mode'])
                if row['state'] != 'ACTIVE':
                    raise ResourceDenied('LOCK_NOT_HELD',
                                         f'{row["lock_id"]} is {row["state"]}, not ACTIVE; renew it or resolve the '
                                         'quarantine before dispatching this task again')
                resource_event(con, 'LOCK_GRANTED', task_id, entry['resource_id'], entry['mode'],
                               'ACTIVE', 'ACTIVE', fingerprint, f'ALREADY_HELD: {reason}',
                               evidence_ref, actor, at)
            return {'status': 'ALREADY_HELD', 'fingerprint': fingerprint, 'entries': ordered,
                    'lock_ids': sorted(row['lock_id'] for row in held)}
        ordered = missing
    granted = []
    for index, entry in enumerate(ordered, 1):
        resource_event(con, 'LOCK_REQUESTED', task_id, entry['resource_id'], entry['mode'], '(none)', 'ACTIVE',
                       fingerprint, f'requested {reason}', evidence_ref, actor, at)
        blocker = conflicting_resource_row(con, entry['resource_id'], entry['mode'], task_id)
        if blocker:
            _deny(con, task_id, entry, fingerprint, actor, at, evidence_ref, 'MODE_CONFLICT',
                  f'{entry["resource_id"]}:{entry["mode"]} conflicts with {_conflict_detail(blocker)}',
                  index, len(ordered))
        granted.append(entry)
    lock_ids = []
    for entry in granted:
        previous = con.execute('SELECT MAX(generation) FROM resource_locks WHERE resource_id=? AND task_id=? AND mode=?',
                               (entry['resource_id'], task_id, entry['mode'])).fetchone()[0]
        generation = 0 if previous is None else int(previous) + 1
        lock_id = f'{entry["resource_id"]}#{entry["mode"]}#{task_id}#{generation}'
        lease_until = add_seconds(at, entry['ttl'])
        con.execute('INSERT INTO resource_locks(lock_id,resource_id,mode,holder,task_id,acquired_at,lease_until,'
                    'renewed_at,state,release_reason,released_at,reason,evidence_ref,backend,generation) '
                    "VALUES(?,?,?,?,?,?,?,?,'ACTIVE','(none)','(none)',?,?,?,?)",
                    (lock_id, entry['resource_id'], entry['mode'], holder, task_id, at, lease_until, at,
                     reason, evidence_ref, backend, generation))
        resource_event(con, 'LOCK_GRANTED', task_id, entry['resource_id'], entry['mode'], '(none)', 'ACTIVE',
                       fingerprint, reason, evidence_ref, actor, at)
        lock_ids.append(lock_id)
    return {'status': 'GRANTED', 'fingerprint': fingerprint, 'entries': granted, 'lock_ids': lock_ids}

def release_resource_set(con, task_id, release_reason, reason, actor, evidence_ref, at=None, rows=None):
    """Release a whole set in deterministic order; one failure does not skip the rest (§4.5)."""
    if release_reason not in RESOURCE_RELEASE_REASONS:
        raise ResourceError('RELEASE_REASON_INVALID', f'release reason {release_reason!r} is not in {list(RESOURCE_RELEASE_REASONS)}')
    at = at or now()
    targets = list(rows) if rows is not None else list(held_resource_rows(con, task_id))
    released, problems = [], []
    for row in targets:
        row = resource_row(con, row['lock_id'])
        if row['state'] == 'RELEASED':
            resource_event(con, 'LOCK_TRANSITION_REJECTED', row['task_id'], row['resource_id'], row['mode'],
                           'RELEASED', 'RELEASED', '', f'ALREADY_RELEASED: {reason}', evidence_ref, actor, at)
            problems.append(f'{row["lock_id"]}: ALREADY_RELEASED (no new state transition)')
            continue
        try:
            transition_resource(con, row, 'RELEASED', 'LOCK_RELEASED', actor, reason, evidence_ref, at,
                                release_reason=release_reason, released_at=at)
            released.append(row['lock_id'])
        except Exception as exc:                     # keep going: report per item, then aggregate
            problems.append(f'{row["lock_id"]}: {exc}')
    if problems:
        raise ResourceError('RELEASE_INCOMPLETE', 'release was not fully applied: ' + '; '.join(problems))
    return released

def force_unlock_resource(con, lock_id, reason, actor, evidence_ref, authorized_by=None, at=None):
    """Manual override: reason + authorisation + audit; skips conflict checks (§6.4)."""
    at = at or now()
    if not (reason or '').strip():
        raise ResourceError('REASON_REQUIRED', 'forced unlock requires --reason explaining why the holder has stopped')
    if not (evidence_ref or '').strip():
        raise ResourceError('EVIDENCE_REQUIRED', 'forced unlock requires a locatable --evidence reference')
    row = resource_row(con, lock_id)
    task = con.execute('SELECT status FROM tasks WHERE id=?', (row['task_id'],)).fetchone()
    status = task['status'] if task else '(unknown)'
    if status not in TERMINAL and not (authorized_by or '').strip():
        raise ResourceError('AUTHORIZATION_REQUIRED',
                            f'task {row["task_id"]} is {status}, not terminal; forced unlock needs --authorized-by '
                            '(the person or role taking responsibility), or wait until the task is VERIFIED/CANCELED')
    whoever = (authorized_by or actor or '').strip()
    if not whoever:
        raise ResourceError('AUTHORIZATION_REQUIRED', 'forced unlock requires a locatable authoriser')
    transition_resource(con, row, 'RELEASED', 'LOCK_FORCED', whoever,
                        f'FORCED_UNLOCK by {whoever}: {reason}', evidence_ref, at, release_reason='FORCED_UNLOCK',
                        released_at=at)
    return row['lock_id']

def reclaim_resource_lock(con, lock_id, reason, evidence_ref, grace, actor='orchestrator', at=None):
    """EXPIRED -> RELEASED with terminal evidence and after the cool-down (§6.2)."""
    at = at or now()
    if not (reason or '').strip():
        raise ResourceError('REASON_REQUIRED', 'reclaim requires --reason stating the terminal evidence')
    if not (evidence_ref or '').strip():
        raise ResourceError('EVIDENCE_REQUIRED', 'reclaim requires a locatable --evidence reference')
    row = resource_row(con, lock_id)
    if row['state'] != 'EXPIRED':
        raise ResourceError('NOT_EXPIRED', f'{lock_id} is {row["state"]}; only an EXPIRED lock can be reclaimed '
                                           '(a live lease is never reclaimed)')
    ready_at = add_seconds(row['lease_until'], int(grace))
    if add_seconds(at, 0) < ready_at:
        raise ResourceError('GRACE_NOT_ELAPSED',
                            f'{lock_id} can be reclaimed at {ready_at} at the earliest (lease_until {row["lease_until"]} '
                            f'+ cool-down {grace}s); renew or raise a forced unlock instead')
    transition_resource(con, row, 'RELEASED', 'LOCK_RECLAIMED', actor, f'LEASE_RECLAIMED: {reason}',
                        evidence_ref, at, release_reason='LEASE_RECLAIMED', released_at=at)
    return lock_id

def renew_resource_locks(con, task_id, holder, ttl, reason, evidence_ref, actor=None, at=None):
    """Extend own lease only; a released/foreign lock yields LOCK_NOT_HELD (§5.2, §6.4)."""
    at = at or now()
    holder = validate_holder(holder)
    rows = [row for row in held_resource_rows(con, task_id) if row['state'] in ('ACTIVE', 'EXPIRED')]
    if not rows:
        raise ResourceError('LOCK_NOT_HELD', f'task {task_id} holds no ACTIVE or EXPIRED resource lock; '
                                            'nothing can be renewed (a forced unlock keeps this true)')
    for row in rows:
        if row['holder'] != holder:
            raise ResourceError('LOCK_NOT_HELD', f'{row["lock_id"]} is held by {row["holder"]}, not {holder}; '
                                                'a lock is only renewed by its own holder')
        transition_resource(con, row, 'ACTIVE', 'LOCK_RENEWED', actor or holder,
                            f'renewed: {reason}' if reason else 'renewed', evidence_ref, at,
                            renewed_at=at, lease_until=add_seconds(at, int(ttl)))
    return [row['lock_id'] for row in rows]

def reconcile_resource_locks(con, reason, evidence_ref, actor='orchestrator', at=None):
    """Rebuild the facts from the store, never from memory (§6.6).

    Classifies every non-RELEASED lock; a record without acquisition evidence,
    with an unlocatable holder, or belonging to a task in NEEDS_HUMAN_DECISION is
    quarantined instead of being trusted or treated as free.

    The evidence count is snapshotted *before* the expiry sweep: an EXPIRED event
    written by this very pass must not be able to justify a holding that has no
    acquisition event at all.
    """
    at = at or now()
    row_ids = [row['lock_id'] for row in con.execute(
        "SELECT lock_id FROM resource_locks WHERE state<>'RELEASED' ORDER BY resource_id, task_id")]
    evidence = {}
    for lock_id in row_ids:
        row = con.execute('SELECT * FROM resource_locks WHERE lock_id=?', (lock_id,)).fetchone()
        evidence[lock_id] = con.execute(
            "SELECT COUNT(*) FROM resource_lock_events WHERE task_id=? AND resource_id=? AND mode=? "
            "AND event_type IN ('LOCK_GRANTED','LOCK_ADOPTED')",
            (row['task_id'], row['resource_id'], row['mode'])).fetchone()[0]
    expire_due_locks(con, at, actor)
    report = []
    for lock_id in row_ids:
        row = con.execute('SELECT * FROM resource_locks WHERE lock_id=?', (lock_id,)).fetchone()
        events = evidence[lock_id]
        task = con.execute('SELECT status FROM tasks WHERE id=?', (row['task_id'],)).fetchone()
        status = task['status'] if task else '(unknown)'
        holder = (row['holder'] or '').strip()
        verdict, action = 'valid', 'kept as held; read-only for a new session'
        if not events:
            verdict, action = 'missing-evidence', 'quarantined: lock rows without events are not evidence'
        elif holder.lower() in HOLDER_GENERIC:
            verdict, action = 'unlocatable-holder', 'quarantined: the holder cannot be located'
        elif status == 'NEEDS_HUMAN_DECISION':
            verdict, action = 'human-decision', 'quarantined: task awaits a human decision'
        elif row['state'] == 'EXPIRED':
            verdict, action = 'expired', 'kept EXPIRED: reclaim after the cool-down or force with a reason'
        elif row['state'] == 'QUARANTINED':
            verdict, action = 'quarantined', 'kept quarantined until a human decides'
        else:
            heartbeat = datetime.fromisoformat(at) - datetime.fromisoformat(row['renewed_at'])
            lease = datetime.fromisoformat(row['lease_until']) - datetime.fromisoformat(row['acquired_at'])
            if lease.total_seconds() > 0 and heartbeat.total_seconds() > lease.total_seconds() / 2:
                verdict, action = 'stale-heartbeat', 'kept as held but flagged: renew before the lease runs out'
        if verdict in ('missing-evidence', 'unlocatable-holder', 'human-decision'):
            quarantine_resource_row(con, row, actor, f'{reason}: {verdict}', evidence_ref, at)
        report.append({'lock_id': row['lock_id'], 'resource_id': row['resource_id'], 'mode': row['mode'],
                       'state': row['state'], 'holder': row['holder'], 'lease_until': row['lease_until'],
                       'task_id': row['task_id'], 'task_status': status, 'events': events,
                       'verdict': verdict, 'action': action})
    return report

def resource_backend_note(backend='local'):
    """State the boundary honestly; `github` registration is advisory only (§7)."""
    if backend == 'github':
        return ('github stores resource claims as an advisory registration in the Issue ORCH-TASK-REGISTRY '
                'block; it is a coordination record, not an atomic lock, and two concurrent writers can '
                'overwrite each other (TOCTOU).')
    return ('local acquires named resource locks atomically inside one SQLite BEGIN IMMEDIATE transaction, '
            'scoped to this database file on this host; it claims no cross-host or shared-filesystem guarantee.')

def backup(root):
    src=db_path(root)
    if not src.exists(): raise RuntimeError('task database is not initialized')
    outdir=root/'.orchestration'/'backups'; outdir.mkdir(parents=True,exist_ok=True)
    name='state-'+datetime.now().strftime('%Y%m%d-%H%M%S')+'.sqlite3'; dest=outdir/name
    a=sqlite3.connect(src); b=sqlite3.connect(dest)
    try: a.backup(b)
    finally: b.close(); a.close()
    print(f'Backup created: {dest}')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',default='.',help='project root (default: current directory)')
    sub=parser.add_subparsers(dest='cmd',required=True)
    sub.add_parser('init',help='initialize the local task database and .orchestration/config.json (retry_cap, local_dir); seed docs/PLAN.md from templates/PLAN.md when absent')
    sub.add_parser('backup',help='create a consistent SQLite backup')
    cf=sub.add_parser('config',help='inspect runtime configuration'); cs=cf.add_subparsers(dest='configcmd',required=True)
    cs.add_parser('show',help='print effective config values and their source')
    t=sub.add_parser('task',help='task operations'); ts=t.add_subparsers(dest='taskcmd',required=True)
    a=ts.add_parser('add'); a.add_argument('--id',required=True); a.add_argument('--title',required=True); a.add_argument('--description',default=''); a.add_argument('--paths',nargs='+',required=True); a.add_argument('--depends',nargs='*',default=[]); a.add_argument('--acceptance',nargs='+',required=True)
    a.add_argument('--resources',nargs='*',default=[],
                   help='optional declared resource set: <resource_id>:<mode>[:<ttl>], comma-free list; empty means the task takes no resource lock')
    l=ts.add_parser('list'); l.add_argument('--active-only',action='store_true')
    sh=ts.add_parser('show'); sh.add_argument('id')
    rd=ts.add_parser('ready'); rd.add_argument('id')
    dp=ts.add_parser('dispatch'); dp.add_argument('id')
    dp.add_argument('--resource',action='append',default=[],
                    help='extra <resource_id>:<mode>[:<ttl>] claimed with the declared set (optional; default behaviour unchanged)')
    dp.add_argument('--holder',default='',help='locatable holder ID (default: host:pid or $ORCH_HOLDER)')
    dp.add_argument('--evidence',default='',help='locatable evidence reference recorded on the granted locks')
    st=ts.add_parser('status'); st.add_argument('id'); st.add_argument('status',choices=sorted(TRANSITIONS)); st.add_argument('--head-sha'); st.add_argument('--merge-sha',help='full 40-hex merge commit SHA (required for MERGED)'); st.add_argument('--note',default='')
    am=ts.add_parser('amend',help='correct the merge SHA of a MERGED/VERIFIED task (audited; requires --note)'); am.add_argument('id'); am.add_argument('--merge-sha',required=True); am.add_argument('--note',required=True)
    ws=ts.add_parser('workspace'); ws.add_argument('id'); ws.add_argument('--branch',default=''); ws.add_argument('--worktree',default=''); ws.add_argument('--head-sha',default=''); ws.add_argument('--note',default='')
    ul=ts.add_parser('unlock'); ul.add_argument('id'); ul.add_argument('--reason',required=True); ul.add_argument('--force',action='store_true')
    ul.add_argument('--resource',default='',help='release/force-unlock one named resource lock (id or lock_id) instead of the path locks')
    ul.add_argument('--evidence',default='',help='locatable evidence for a forced unlock')
    ul.add_argument('--authorized-by',dest='authorized_by',default='',help='authoriser for a forced unlock of a non-terminal task')
    rs=ts.add_parser('resources',help='inspect named resource locks, their leases and blocking effect')
    rs.add_argument('--resource',default='',help='only this resource_id'); rs.add_argument('--all',action='store_true',help='include RELEASED rows')
    rn=ts.add_parser('renew',help='renew this task\'s own resource leases'); rn.add_argument('id')
    rn.add_argument('--holder',default=''); rn.add_argument('--reason',default=''); rn.add_argument('--evidence',default='')
    rc=ts.add_parser('reclaim',help='reclaim one EXPIRED lock after the cool-down with terminal evidence')
    rc.add_argument('id'); rc.add_argument('--resource',required=True); rc.add_argument('--reason',required=True)
    rc.add_argument('--evidence',required=True); rc.add_argument('--actor',default='')
    rf=ts.add_parser('reconcile',help='rebuild resource lock facts from the store; quarantine what cannot be proven')
    rf.add_argument('--reason',default='cross-session recovery'); rf.add_argument('--evidence',default='')
    args=parser.parse_args(); root=root_path(args.root)
    try:
        if args.cmd=='init': init(root)
        elif args.cmd=='backup': backup(root)
        elif args.cmd=='config': return config_show(root)
        elif args.taskcmd=='add': task_add(root,args)
        elif args.taskcmd=='list': task_list(root,args)
        elif args.taskcmd=='show': task_show(root,args.id)
        elif args.taskcmd=='ready': task_ready(root,args.id)
        elif args.taskcmd=='dispatch': task_dispatch(root,args.id,args)
        elif args.taskcmd=='status': task_status(root,args)
        elif args.taskcmd=='amend': task_amend(root,args)
        elif args.taskcmd=='workspace': set_workspace(root,args)
        elif args.taskcmd=='unlock': unlock(root,args)
        elif args.taskcmd=='resources': task_resources(root,args)
        elif args.taskcmd=='renew': task_renew(root,args)
        elif args.taskcmd=='reclaim': task_reclaim(root,args)
        elif args.taskcmd=='reconcile': task_reconcile(root,args)
    except Exception as e:
        print(f'ERROR: {e}',file=sys.stderr); return 1
    return 0
if __name__=='__main__': raise SystemExit(main())
