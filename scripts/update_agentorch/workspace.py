"""Project workspace inspection, staging, backup and rollback.

Everything in this module is read-only until :func:`atomic_write` is called. The
engine only calls the writers after the package is verified and a backup has been
created and verified. ``git stash`` / ``git checkout --`` / ``git clean`` are
never executed: uncommitted work is either left alone in an isolated worktree or
the run stops.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import tempfile
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath

SKIP_DIRS = frozenset({
    '.git', '.orchestration', '.local', '.worktrees', '.workbuddy', 'release',
    '__pycache__', 'node_modules', '.venv', '.idea', '.vscode',
    '.mypy_cache', '.pytest_cache', '.ruff_cache',
})


class UnsafePath(Exception):
    """A target path would leave the project or traverse a symlink."""


def utc_stamp(now: datetime) -> str:
    return now.astimezone(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')


# --- byte helpers -----------------------------------------------------------

def is_text(data: bytes) -> bool:
    if b'\x00' in data:
        return False
    try:
        data.decode('utf-8')
    except UnicodeDecodeError:
        return False
    return True


def normalize_line_endings(data: bytes) -> bytes:
    """Same folding rule the release builder uses before hashing."""
    if not is_text(data):
        return data
    return data.replace(b'\r\n', b'\n')


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(normalize_line_endings(data)).hexdigest()


# --- git state --------------------------------------------------------------

@dataclass
class GitState:
    is_repo: bool = False
    head_sha: str | None = None
    branch: str | None = None
    dirty_paths: list[str] = field(default_factory=list)
    isolated: bool = False
    commands: list[dict] = field(default_factory=list)

    @property
    def dirty(self) -> bool:
        return bool(self.dirty_paths)


def _run_git(root: Path, args: list[str], commands: list[dict], now: datetime) -> subprocess.CompletedProcess:
    commands.append({'command': ' '.join(args), 'exit_code': 0, 'ran_at': utc_stamp(now)})
    try:
        proc = subprocess.run(args, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                              text=True, encoding='utf-8', errors='replace',
                              env={**os.environ, 'GIT_TERMINAL_PROMPT': '0'})
    except OSError:
        commands[-1]['exit_code'] = 127
        raise
    commands[-1]['exit_code'] = proc.returncode
    return proc


def _porcelain_paths(output: str) -> list[str]:
    paths: list[str] = []
    records = output.split('\0')
    index = 0
    while index < len(records):
        record = records[index]
        index += 1
        if not record:
            continue
        status = record[:2]
        path = record[3:]
        if path:
            paths.append(path)
        if status[:1] in ('R', 'C') and index < len(records) and records[index]:
            paths.append(records[index])
            index += 1
    return sorted(set(paths))


def inspect_git(root: Path, now: datetime) -> GitState:
    """Read head / branch / uncommitted paths. Never mutates the repository."""
    root = Path(root)
    state = GitState()
    try:
        inside = _run_git(root, ['git', 'rev-parse', '--is-inside-work-tree'], state.commands, now)
    except OSError:
        return state
    if inside.returncode != 0 or inside.stdout.strip() != 'true':
        return state
    state.is_repo = True

    head = _run_git(root, ['git', 'rev-parse', 'HEAD'], state.commands, now)
    if head.returncode == 0:
        state.head_sha = head.stdout.strip() or None
    branch = _run_git(root, ['git', 'rev-parse', '--abbrev-ref', 'HEAD'], state.commands, now)
    if branch.returncode == 0:
        state.branch = branch.stdout.strip() or None
    status = _run_git(root, ['git', 'status', '--porcelain=v1', '-z', '--untracked-files=all'],
                      state.commands, now)
    if status.returncode == 0:
        state.dirty_paths = _porcelain_paths(status.stdout)

    git_dir = _run_git(root, ['git', 'rev-parse', '--git-dir'], state.commands, now)
    common_dir = _run_git(root, ['git', 'rev-parse', '--git-common-dir'], state.commands, now)
    if git_dir.returncode == 0 and common_dir.returncode == 0:
        def resolved(value: str) -> Path:
            candidate = Path(value.strip())
            return candidate if candidate.is_absolute() else (root / candidate)
        state.isolated = resolved(git_dir.stdout).resolve() != resolved(common_dir.stdout).resolve()
    return state


@dataclass
class Isolation:
    path: Path | None
    commands: list[dict] = field(default_factory=list)
    detail: str | None = None


def create_isolation_worktree(root: Path, name: str, now: datetime) -> Isolation:
    """Create a dedicated branch + worktree under ``.worktrees/`` for APPLY.

    The original working tree (including uncommitted changes) is not touched.
    """
    root = Path(root)
    relative = f'.worktrees/update-{name}'
    target = root / relative
    if target.exists():
        return Isolation(None, [], f'{relative} already exists')
    commands: list[dict] = []
    proc = _run_git(root, ['git', 'worktree', 'add', '-q', '-b', f'update/{name}', relative, 'HEAD'],
                    commands, now)
    if proc.returncode != 0:
        return Isolation(None, commands, 'git worktree add failed')
    return Isolation(target, commands, None)


# --- project scan -----------------------------------------------------------

def scan_project_files(root: Path) -> dict[str, str]:
    """Relative path -> normalized sha256 for every regular project file."""
    root = Path(root)
    found: dict[str, str] = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
        for name in filenames:
            if name == '.git':
                continue
            full = Path(dirpath) / name
            if full.is_symlink() or not full.is_file():
                continue
            rel = full.relative_to(root).as_posix()
            try:
                found[rel] = sha256_bytes(full.read_bytes())
            except OSError:
                continue
    return dict(sorted(found.items()))


# --- safe writes ------------------------------------------------------------

def safe_project_path(root: Path, relative: str) -> Path:
    """Return the write target, refusing symlinks and paths outside ``root``."""
    root = Path(root)
    candidate = root / relative
    resolved_root = root.resolve()
    current = root
    for part in PurePosixPath(relative).parts:
        current = current / part
        if current.is_symlink():
            raise UnsafePath(f'target path traverses a symlink: {relative}')
    if candidate.is_symlink():
        raise UnsafePath(f'target path is a symlink: {relative}')
    parent = candidate.parent.resolve()
    if parent != resolved_root and resolved_root not in parent.parents:
        raise UnsafePath(f'target path escapes the project: {relative}')
    return candidate


def target_problems(root: Path, paths: list[str]) -> list[str]:
    """Preflight the write set: refuse symlinks, directories and paths that escape."""
    problems: list[str] = []
    for relative in sorted(set(paths)):
        try:
            target = safe_project_path(root, relative)
        except UnsafePath as exc:
            problems.append(str(exc))
            continue
        if target.exists() and not target.is_file():
            problems.append(f'{relative}: target exists but is not a regular file')
    return problems


def atomic_write(path: Path, data: bytes) -> list[Path]:
    """Write ``data`` via a sibling temp file + ``os.replace``; return created dirs."""
    created: list[Path] = []
    probe = path.parent
    while not probe.exists() and probe != probe.parent:
        created.append(probe)
        probe = probe.parent
    path.parent.mkdir(parents=True, exist_ok=True)
    created = list(reversed(created))
    handle, temp_name = tempfile.mkstemp(dir=str(path.parent), prefix='.agentorch-write-')
    try:
        with os.fdopen(handle, 'wb') as stream:
            stream.write(data)
        os.replace(temp_name, path)
    except BaseException:
        try:
            os.unlink(temp_name)
        except OSError:
            pass
        raise
    return created


# --- backup / restore -------------------------------------------------------

@dataclass
class Backup:
    location: Path
    created_at: str
    verified: bool
    entries: list[dict]
    metadata: dict

    @property
    def files_dir(self) -> Path:
        return self.location / 'files'


def _entry_for(root: Path, relative: str) -> dict:
    target = root / relative
    if target.is_file() and not target.is_symlink():
        data = target.read_bytes()
        return {
            'path': relative,
            'present': True,
            'sha256': sha256_bytes(data),
            'size': len(data),
        }
    return {'path': relative, 'present': False, 'sha256': None, 'size': None}


def create_backup(root: Path, paths: list[str], *, now: datetime, metadata: dict) -> Backup:
    """Copy every path that may be rewritten into a project-external directory."""
    root = Path(root)
    location = Path(tempfile.mkdtemp(prefix='agentorch-update-backup-'))
    files_dir = location / 'files'
    files_dir.mkdir(parents=True, exist_ok=True)
    entries = []
    for relative in sorted(set(paths)):
        entry = _entry_for(root, relative)
        if entry['present']:
            target = files_dir / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((root / relative).read_bytes())
        entries.append(entry)
    payload = {
        'backup_schema_version': 1,
        'created_at': utc_stamp(now),
        'files': entries,
        **metadata,
    }
    (location / 'backup.json').write_text(json.dumps(payload, indent=2, sort_keys=True) + '\n',
                                          encoding='utf-8')
    verified = verify_backup(location, entries)
    return Backup(location=location, created_at=payload['created_at'], verified=verified,
                  entries=entries, metadata=payload)


def verify_backup(location: Path, entries: list[dict]) -> bool:
    location = Path(location)
    try:
        payload = json.loads((location / 'backup.json').read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return False
    recorded = {entry['path']: entry for entry in payload.get('files') or [] if isinstance(entry, dict)}
    if set(recorded) != {entry['path'] for entry in entries}:
        return False
    for entry in entries:
        backup_copy = location / 'files' / entry['path']
        if entry['present']:
            if not backup_copy.is_file():
                return False
            if sha256_bytes(backup_copy.read_bytes()) != entry['sha256']:
                return False
            if backup_copy.stat().st_size != entry['size']:
                return False
        elif backup_copy.exists():
            return False
    return True


def restore_backup(root: Path, backup: Backup) -> list[str]:
    """Restore every backed-up path; return human-readable problems (empty = clean)."""
    root = Path(root)
    problems: list[str] = []
    for entry in backup.entries:
        relative = entry['path']
        try:
            target = safe_project_path(root, relative)
        except UnsafePath as exc:
            problems.append(str(exc))
            continue
        try:
            if entry['present']:
                data = (backup.files_dir / relative).read_bytes()
                atomic_write(target, data)
            elif target.is_file() or target.is_symlink():
                target.unlink()
        except OSError:
            problems.append(f'cannot restore {relative}')
    return problems


def verify_restore(root: Path, backup: Backup) -> list[str]:
    """Re-read the restored paths and compare them with the backup record."""
    root = Path(root)
    problems: list[str] = []
    for entry in backup.entries:
        target = root / entry['path']
        exists = target.is_file() and not target.is_symlink()
        if exists != entry['present']:
            problems.append(f'{entry["path"]}: presence differs after rollback')
            continue
        if not entry['present']:
            continue
        try:
            digest = sha256_bytes(target.read_bytes())
        except OSError:
            problems.append(f'{entry["path"]}: unreadable after rollback')
            continue
        if digest != entry['sha256']:
            problems.append(f'{entry["path"]}: sha256 differs after rollback')
    return problems
