"""Fixture builders shared by the update-engine tests.

A fixture is always a *synthetic* project (a temp directory) plus a *synthetic*
release package (bytes in memory) that follows the frozen manifest schema. No
fixture downloads anything, and none contains real project data.
"""
from __future__ import annotations

import hashlib
import importlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / 'scripts'
_ENGINE_DIR = SCRIPTS / 'update_agentorch'
# Load the engine under a private alias: when `unittest discover -s tests` runs,
# the test package itself is imported as top-level `update_agentorch`, so the
# production package cannot be addressed by that name in every invocation.
_ENGINE_ALIAS = 'agentorch_update_engine_under_test'


def _load_engine():
    if _ENGINE_ALIAS in sys.modules:
        return sys.modules[_ENGINE_ALIAS]
    spec = importlib.util.spec_from_file_location(
        _ENGINE_ALIAS, _ENGINE_DIR / '__init__.py',
        submodule_search_locations=[str(_ENGINE_DIR)])
    module = importlib.util.module_from_spec(spec)
    sys.modules[_ENGINE_ALIAS] = module
    spec.loader.exec_module(module)
    return module


_load_engine()
sources = importlib.import_module(f'{_ENGINE_ALIAS}.sources')
versions = importlib.import_module(f'{_ENGINE_ALIAS}.versions')
package = importlib.import_module(f'{_ENGINE_ALIAS}.package')
classify = importlib.import_module(f'{_ENGINE_ALIAS}.classify')
workspace = importlib.import_module(f'{_ENGINE_ALIAS}.workspace')
report = importlib.import_module(f'{_ENGINE_ALIAS}.report')
engine = importlib.import_module(f'{_ENGINE_ALIAS}.engine')
cli = importlib.import_module(f'{_ENGINE_ALIAS}.cli')

SUITE = 'Agent-Orchestration Starter Kit'
PROFILE_PATHS = (
    'ORCHESTRATION-PROJECT.md',
    'docs/PROJECT.md',
    'PLAN.md',
    'docs/PLAN.md',
    'DESIGN.md',
    'docs/DESIGN.md',
)


# --- byte helpers -----------------------------------------------------------

def is_text(data: bytes) -> bool:
    if b'\x00' in data:
        return False
    try:
        data.decode('utf-8')
    except UnicodeDecodeError:
        return False
    return True


def normalize(data: bytes) -> bytes:
    """Same LF folding rule the release builder uses before hashing/packing."""
    if not is_text(data):
        return data
    return data.replace(b'\r\n', b'\n')


def sha256(data: bytes) -> str:
    return hashlib.sha256(normalize(data)).hexdigest()


def category(path: str) -> str:
    if path.startswith('editions/'):
        return 'edition'
    if path.startswith('templates/'):
        return 'template'
    if path in PROFILE_PATHS:
        return 'project-profile'
    return 'core'


# --- manifests --------------------------------------------------------------

def build_manifest(files: dict[str, bytes], *, suite: str = SUITE,
                   format_version: str = '1.5') -> dict:
    entries = []
    for path in sorted(files):
        data = normalize(files[path])
        entries.append({
            'path': path,
            'category': category(path),
            'sha256': hashlib.sha256(data).hexdigest(),
            'size': len(data),
        })
    return {
        'schema_version': 1,
        'suite': suite,
        'format_version': format_version,
        'files': entries,
    }


def manifest_bytes(manifest: dict) -> bytes:
    return json.dumps(manifest, indent=2, sort_keys=True, ensure_ascii=False).encode('utf-8')


# --- release packages -------------------------------------------------------

def write_member(archive: zipfile.ZipFile, name: str, data: bytes, mode: int = 0o644) -> None:
    info = zipfile.ZipInfo(filename=name)
    info.external_attr = mode << 16
    info.create_system = 3
    info.compress_type = zipfile.ZIP_DEFLATED
    archive.writestr(info, data)


def make_zip(files: dict[str, bytes], manifest: dict | None = None, *,
             top: str | None = 'agentorch-20260101',
             include_manifest: bool = True,
             manifest_override: bytes | None = None,
             raw_members: tuple[tuple[str, bytes, int], ...] = ()) -> bytes:
    """Build a package archive from synthetic members.

    ``raw_members`` injects deliberately hostile entries (traversal, symlinks,
    extra files) without going through the manifest helper.
    """
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w', zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(files):
            name = f'{top}/{path}' if top else path
            write_member(archive, name, normalize(files[path]))
        if include_manifest:
            data = manifest_override if manifest_override is not None else manifest_bytes(manifest or {})
            name = f'{top}/MANIFEST.json' if top else 'MANIFEST.json'
            write_member(archive, name, data)
        for name, data, mode in raw_members:
            write_member(archive, name, data, mode)
    return buffer.getvalue()


def release_info(tag: str = 'v1.6', payload: bytes | None = None, *,
                 repository: str = sources.OFFICIAL_REPOSITORY,
                 prerelease: bool = False, draft: bool = False,
                 asset_name: str = 'agentorch-20260101.zip',
                 digest: str | None = 'auto',
                 extra_assets: tuple[dict, ...] = (),
                 html_url: str | None = None,
                 manifest_sha256: str | None = None) -> sources.ReleaseInfo:
    if digest == 'auto':
        digest = 'sha256:' + hashlib.sha256(payload or b'').hexdigest() if payload is not None else None
    assets = [sources.ReleaseAsset(
        name=asset_name,
        size=len(payload or b''),
        digest=digest,
        download_url=f'https://github.com/{repository}/releases/download/{tag}/{asset_name}',
    )]
    return sources.ReleaseInfo(
        repository=repository,
        tag_name=tag,
        prerelease=prerelease,
        draft=draft,
        assets=tuple(assets) + tuple(sources.ReleaseAsset(**entry) for entry in extra_assets),
        html_url=html_url or f'https://github.com/{repository}/releases/tag/{tag}',
        manifest_sha256=manifest_sha256,
    )


class FakeReleaseSource:
    """In-memory ReleaseSource replacement; counts calls so tests can prove zero network."""

    def __init__(self, release: sources.ReleaseInfo, payload: bytes | None = None, *,
                 fetch_error: Exception | None = None,
                 download_error: Exception | None = None):
        self.release = release
        self.payload = payload
        self.fetch_error = fetch_error
        self.download_error = download_error
        self.fetch_count = 0
        self.download_count = 0

    def fetch_release(self) -> sources.ReleaseInfo:
        self.fetch_count += 1
        if self.fetch_error:
            raise self.fetch_error
        return self.release

    def download_asset(self, asset: sources.ReleaseAsset) -> bytes:
        self.download_count += 1
        if self.download_error:
            raise self.download_error
        assert self.payload is not None
        return self.payload


# --- synthetic project -------------------------------------------------------

class ProjectTree:
    """A throwaway project directory; ``install`` writes files plus a baseline manifest."""

    def __init__(self, root):
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)

    def path(self, rel: str) -> Path:
        return self.root / rel

    def write(self, rel: str, data: bytes | str) -> None:
        target = self.root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(data, str):
            data = data.encode('utf-8')
        target.write_bytes(data)

    def read(self, rel: str) -> bytes:
        return (self.root / rel).read_bytes()

    def exists(self, rel: str) -> bool:
        return (self.root / rel).exists()

    def install(self, files: dict[str, bytes], *, format_version: str = '1.5',
                suite: str = SUITE, write_manifest: bool = True) -> dict:
        for rel in sorted(files):
            self.write(rel, files[rel])
        manifest = build_manifest(files, suite=suite, format_version=format_version)
        if write_manifest:
            self.write('MANIFEST.json', manifest_bytes(manifest))
        return manifest

    def snapshot(self) -> dict[str, bytes]:
        """Every regular file except VCS / worktree metadata (stable ordering)."""
        found = {}
        for dirpath, dirnames, filenames in os.walk(self.root):
            dirnames[:] = sorted(d for d in dirnames if d not in {'.git', '.worktrees'})
            for name in filenames:
                full = Path(dirpath) / name
                if full.is_symlink() or not full.is_file():
                    continue
                found[full.relative_to(self.root).as_posix()] = full.read_bytes()
        return dict(sorted(found.items()))

    def hashes(self) -> dict[str, str]:
        return {rel: hashlib.sha256(data).hexdigest() for rel, data in self.snapshot().items()}


# --- git helpers ------------------------------------------------------------

def git(root, *args, check: bool = True) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        ['git', '-c', 'user.email=update-test@example.invalid',
         '-c', 'user.name=Update Test', *args],
        cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        env={**os.environ, 'GIT_CONFIG_NOSYSTEM': '1', 'GIT_TERMINAL_PROMPT': '0'},
    )
    if check and proc.returncode != 0:
        raise AssertionError(f'git {" ".join(args)} failed: {proc.stderr}')
    return proc


def git_init(root) -> str:
    git(root, 'init', '-q')
    git(root, 'add', '-A')
    git(root, 'commit', '-qm', 'baseline')
    return git(root, 'rev-parse', 'HEAD').stdout.strip()


# --- fault injection --------------------------------------------------------

class FaultPlan:
    """Deterministic fault injection used only by tests (never wired into the CLI)."""

    def __init__(self, *, fail_after_writes: int | None = None,
                 fail_before_backup: bool = False,
                 corrupt_after_restore: str | None = None,
                 corrupt_to: bytes = b'corrupted by test\n'):
        self.fail_after_writes = fail_after_writes
        self.fail_before_backup = fail_before_backup
        self.corrupt_after_restore = corrupt_after_restore
        self.corrupt_to = corrupt_to
        self.writes = 0
        self.restored = False

    def before_backup(self) -> None:
        if self.fail_before_backup:
            raise OSError('synthetic backup failure')

    def after_write(self, path: str) -> None:
        self.writes += 1
        if self.fail_after_writes is not None and self.writes >= self.fail_after_writes:
            raise OSError(f'synthetic write failure after {path}')

    def after_restore(self, root) -> None:
        self.restored = True
        if self.corrupt_after_restore:
            (Path(root) / self.corrupt_after_restore).write_bytes(self.corrupt_to)
