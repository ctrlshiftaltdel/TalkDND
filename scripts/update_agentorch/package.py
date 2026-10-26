"""Release package verification (frozen order, fail closed).

The ten checks and their order come from ``docs/self-improvement/update-safety.md``
§3. Every check executes in a project-external staging area / in memory; a
failure never writes a project file. Unchecked steps after a failure are reported
as ``NOT_RUN`` -- they are never implied to pass.

Resource limits for hostile archives (T-15: symlinks, hard links, device files,
zip bombs, oversized packages) are explicit constants in :class:`PackageLimits`.
The frozen contract leaves the values to the implementation; they are recorded
in ``docs/self-improvement/update-engine-implementation.md``.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
import re
import stat
import zipfile
from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath

from . import sources, versions

MANIFEST_NAME = 'MANIFEST.json'
MANIFEST_SCHEMA_VERSION = 1
SUPPORTED_MANIFEST_SCHEMAS = (MANIFEST_SCHEMA_VERSION,)

CHECK_ORDER = (
    'SOURCE_ALLOWLIST',
    'RELEASE_CHANNEL',
    'ASSET_SHA256',
    'MANIFEST_SCHEMA',
    'MANIFEST_SHA256',
    'MEMBER_PATHS',
    'SYMLINKS',
    'FILE_SET',
    'EXPECTED_LAYOUT',
    'VERSION_MATCH',
)

UPSTREAM_CATEGORIES = ('core', 'edition', 'template', 'project-profile')
PROFILE_PATHS = (
    'ORCHESTRATION-PROJECT.md',
    'docs/PROJECT.md',
    'PLAN.md',
    'docs/PLAN.md',
    'DESIGN.md',
    'docs/DESIGN.md',
)

PATH_RE = re.compile(r'^[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*$')
SHA256_RE = re.compile(r'^[a-f0-9]{64}$')
DRIVE_RE = re.compile(r'^[A-Za-z]:')
NESTED_ARCHIVE_SUFFIXES = ('.zip', '.tar', '.tgz', '.gz', '.bz2', '.xz', '.7z', '.rar', '.jar')
FORBIDDEN_MANIFEST_KEYS = ('generated_at', 'generated_on', 'timestamp', 'built_at', 'date')


@dataclass(frozen=True)
class PackageLimits:
    """Hard ceilings applied before anything is extracted.

    The defaults are deliberately generous for a rules package (a few hundred
    text files) and still bound a decompression bomb. They are values of this
    implementation, not frozen contract constants.
    """

    max_members: int = 2000
    max_total_uncompressed: int = 64 * 1024 * 1024
    max_member_uncompressed: int = 32 * 1024 * 1024
    max_compression_ratio: int = 1000
    max_manifest_bytes: int = 1024 * 1024
    max_path_length: int = 512


DEFAULT_LIMITS = PackageLimits()


@dataclass
class PackageResult:
    ok: bool
    status: str  # VERIFIED / UNSAFE_PACKAGE / NOT_RUN
    checks: list[dict] = field(default_factory=list)
    manifest: dict | None = None
    manifest_bytes: bytes | None = None
    manifest_sha256: str | None = None
    files: dict[str, bytes] = field(default_factory=dict)
    top_dir: str | None = None
    version: str | None = None
    format_version: str | None = None
    suite: str | None = None
    failed_check: str | None = None
    detail: str | None = None


# --- manifest schema (frozen update-manifest.schema.json) -------------------

def classify_upstream(path: str) -> str:
    if path.startswith('editions/'):
        return 'edition'
    if path.startswith('templates/'):
        return 'template'
    if path in PROFILE_PATHS:
        return 'project-profile'
    return 'core'


def package_path_errors(path: object, *, max_length: int = 512) -> list[str]:
    if not isinstance(path, str) or not path:
        return ['path must be a non-empty string']
    errors = []
    if len(path) > max_length:
        errors.append(f'path is longer than {max_length} characters')
    if '\\' in path:
        errors.append('path must use "/" separators (no backslash)')
    if path.startswith('/'):
        errors.append('path must be relative (no leading "/")')
    if DRIVE_RE.match(path):
        errors.append('path must not carry a drive letter')
    if not PATH_RE.match(path):
        errors.append('path must match [A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*')
    if any(segment in ('.', '..') for segment in path.split('/')):
        errors.append('path must not contain "." or ".." segments')
    if path == MANIFEST_NAME:
        errors.append('MANIFEST.json must not list itself')
    return errors


def manifest_schema_errors(manifest: object) -> list[str]:
    """Structural errors of a package / baseline manifest (no filesystem access)."""
    if not isinstance(manifest, dict):
        return ['manifest must be a JSON object']
    errors: list[str] = []
    allowed_top = {'schema_version', 'suite', 'format_version', 'files'}
    for key in sorted(manifest):
        if key in FORBIDDEN_MANIFEST_KEYS:
            errors.append(f'manifest must not carry {key!r}: timestamps break reproducibility')
        elif key not in allowed_top:
            errors.append(f'manifest carries unknown top-level key: {key!r}')
    for key in ('schema_version', 'suite', 'format_version', 'files'):
        if key not in manifest:
            errors.append(f'manifest is missing top-level key: {key!r}')

    schema = manifest.get('schema_version')
    if not isinstance(schema, int) or isinstance(schema, bool):
        errors.append(f'manifest schema_version must be an integer, got {schema!r}')
    elif schema not in SUPPORTED_MANIFEST_SCHEMAS:
        errors.append(f'manifest schema_version {schema!r} is not supported (known: {list(SUPPORTED_MANIFEST_SCHEMAS)})')

    suite = manifest.get('suite')
    if not isinstance(suite, str) or not suite.strip():
        errors.append('manifest suite must be a non-empty string')
    elif len(suite) > 200:
        errors.append('manifest suite is longer than 200 characters')

    format_version = manifest.get('format_version')
    if not isinstance(format_version, str) or not re.match(r'^[0-9]+\.[0-9]+$', format_version):
        errors.append(f'manifest format_version must look like <major>.<minor>, got {format_version!r}')

    entries = manifest.get('files')
    if not isinstance(entries, list):
        errors.append('manifest files must be a list')
        return errors

    seen: set[str] = set()
    previous: str | None = None
    for index, entry in enumerate(entries):
        label = f'manifest files[{index}]'
        if not isinstance(entry, dict):
            errors.append(f'{label} must be an object')
            continue
        allowed = {'path', 'category', 'sha256', 'size'}
        for key in sorted(entry):
            if key not in allowed:
                errors.append(f'{label} carries unknown key: {key!r}')
        for key in ('path', 'category', 'sha256', 'size'):
            if key not in entry:
                errors.append(f'{label} is missing key: {key!r}')
        path = entry.get('path')
        for problem in package_path_errors(path, max_length=512):
            errors.append(f'{label}: {problem}')
        if isinstance(path, str):
            if path in seen:
                errors.append(f'{label} duplicates path: {path}')
            if previous is not None and path < previous:
                errors.append(f'{label} is not sorted by path: {path}')
            previous = path
            seen.add(path)
            category = entry.get('category')
            if category not in UPSTREAM_CATEGORIES:
                errors.append(f'{label} has an unknown category: {category!r}')
            elif category != classify_upstream(path):
                errors.append(f'{label} category {category!r} does not match path {path!r} '
                              f'({classify_upstream(path)!r})')
        digest = entry.get('sha256')
        if not isinstance(digest, str) or not SHA256_RE.match(digest):
            errors.append(f'{label} sha256 must be 64 lowercase hex characters')
        size = entry.get('size')
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            errors.append(f'{label} size must be a non-negative integer')
    return errors


# --- verification -----------------------------------------------------------

def _check(name: str, result: str, detail: str | None = None) -> dict:
    if detail is not None and len(detail) > 1000:
        detail = detail[:997] + '...'
    return {'check': name, 'result': result, 'detail': detail}


def _abort(checks: list[dict], failed: str, detail: str, *, status: str = 'UNSAFE_PACKAGE',
           **kwargs) -> PackageResult:
    checks.append(_check(failed, 'FAIL', detail))
    for name in CHECK_ORDER[CHECK_ORDER.index(failed) + 1:]:
        checks.append(_check(name, 'NOT_RUN', f'not executed: stopped at {failed}'))
    return PackageResult(ok=False, status=status, checks=checks, failed_check=failed,
                         detail=detail, **kwargs)


def _passed(checks: list[dict], name: str, detail: str | None = None) -> None:
    checks.append(_check(name, 'PASS', detail))


def verify_package(release: sources.ReleaseInfo, payload: bytes, *,
                   expected_suite: str | None = None,
                   expected_manifest_sha256: str | None = None,
                   limits: PackageLimits = DEFAULT_LIMITS) -> PackageResult:
    """Run the ten frozen checks against downloaded bytes. Never touches the project."""
    checks: list[dict] = []

    # 1 + 2: source allowlist and release channel
    try:
        asset = sources.select_asset(release)
    except sources.SourceError as exc:
        if exc.check == 'RELEASE_CHANNEL':
            _passed(checks, 'SOURCE_ALLOWLIST', 'official repository and release URL')
            return _abort(checks, 'RELEASE_CHANNEL', exc.detail)
        return _abort(checks, 'SOURCE_ALLOWLIST', exc.detail)
    _passed(checks, 'SOURCE_ALLOWLIST', 'ctrlshiftaltdel/AgentOrch Releases only')
    _passed(checks, 'RELEASE_CHANNEL', 'stable release')

    # 3: asset digest
    actual_digest = _sha256(payload)
    recorded = sources.digest_hex(asset.digest)
    if recorded is None:
        return _abort(checks, 'ASSET_SHA256', 'release record does not carry a sha256 asset digest')
    if recorded != actual_digest:
        return _abort(checks, 'ASSET_SHA256',
                      f'downloaded asset sha256 {actual_digest} != recorded {recorded}')
    _passed(checks, 'ASSET_SHA256', f'sha256={actual_digest}')

    # 4: MANIFEST.json exists, parses and matches the frozen schema
    try:
        archive = zipfile.ZipFile(io.BytesIO(payload))
    except (zipfile.BadZipFile, OSError, RuntimeError) as exc:
        return _abort(checks, 'MANIFEST_SCHEMA', f'asset is not a readable zip: {exc}')

    with archive:
        try:
            infos = archive.infolist()
        except (zipfile.BadZipFile, OSError, RuntimeError) as exc:
            return _abort(checks, 'MANIFEST_SCHEMA', f'cannot list archive members: {exc}')

        manifest_members = [info for info in infos
                            if not info.is_dir() and PurePosixPath(info.filename).name == MANIFEST_NAME]
        if len(manifest_members) != 1:
            return _abort(checks, 'MANIFEST_SCHEMA',
                          f'package must contain exactly one {MANIFEST_NAME}, found {len(manifest_members)}')
        manifest_info = manifest_members[0]
        parts = manifest_info.filename.split('/')
        if len(parts) != 2 or not parts[0] or not PATH_RE.match(parts[0]) \
                or parts[0] in ('.', '..'):
            return _abort(checks, 'MANIFEST_SCHEMA',
                          f'{MANIFEST_NAME} must sit directly under a single top-level directory')
        top_dir = parts[0]
        if manifest_info.file_size > limits.max_manifest_bytes:
            return _abort(checks, 'MANIFEST_SCHEMA',
                          f'{MANIFEST_NAME} is larger than the {limits.max_manifest_bytes} byte limit')
        try:
            with archive.open(manifest_info) as handle:
                manifest_bytes = handle.read(limits.max_manifest_bytes + 1)
        except (zipfile.BadZipFile, OSError, RuntimeError) as exc:
            return _abort(checks, 'MANIFEST_SCHEMA', f'cannot read {MANIFEST_NAME}: {exc}')
        if len(manifest_bytes) > limits.max_manifest_bytes:
            return _abort(checks, 'MANIFEST_SCHEMA', f'{MANIFEST_NAME} exceeds the size limit')
        try:
            manifest = json.loads(manifest_bytes.decode('utf-8'))
        except (UnicodeDecodeError, ValueError) as exc:
            return _abort(checks, 'MANIFEST_SCHEMA', f'{MANIFEST_NAME} is not valid JSON: {exc}')
        errors = manifest_schema_errors(manifest)
        if errors:
            return _abort(checks, 'MANIFEST_SCHEMA', '; '.join(errors[:5]))
        _passed(checks, 'MANIFEST_SCHEMA', f'schema_version={manifest["schema_version"]}')

        # 5: manifest bytes digest (independent record when supplied, stable re-read otherwise)
        manifest_sha = _sha256(manifest_bytes)
        try:
            with archive.open(manifest_info) as handle:
                second_read = handle.read(limits.max_manifest_bytes + 1)
        except (zipfile.BadZipFile, OSError, RuntimeError) as exc:
            return _abort(checks, 'MANIFEST_SHA256', f'cannot re-read {MANIFEST_NAME}: {exc}')
        if second_read != manifest_bytes:
            return _abort(checks, 'MANIFEST_SHA256', f'{MANIFEST_NAME} changed between reads')
        expected_manifest = expected_manifest_sha256 or release.manifest_sha256
        if expected_manifest is not None:
            if expected_manifest != manifest_sha:
                return _abort(checks, 'MANIFEST_SHA256',
                              f'manifest sha256 {manifest_sha} != recorded {expected_manifest}')
            _passed(checks, 'MANIFEST_SHA256', f'sha256={manifest_sha} (matches release record)')
        else:
            _passed(checks, 'MANIFEST_SHA256',
                    f'sha256={manifest_sha} (bytes stable across re-read; no independent release digest)')

        # 6: member paths + resource limits
        problem = _member_paths_problem(infos, limits)
        if problem:
            return _abort(checks, 'MEMBER_PATHS', problem)
        _passed(checks, 'MEMBER_PATHS',
                f'{sum(1 for i in infos if not i.is_dir())} files, no traversal or absolute paths')

        # 7: symlinks / hard links / device files
        problem = _special_member_problem(infos)
        if problem:
            return _abort(checks, 'SYMLINKS', problem)
        _passed(checks, 'SYMLINKS', 'no symlink, device or other special members')

        # 8: file set equality + per-member hash/size (the manifest is the transfer contract)
        files, problem = _file_set(archive, infos, manifest, top_dir)
        if problem:
            return _abort(checks, 'FILE_SET', problem)
        _passed(checks, 'FILE_SET', 'manifest entries == archive members; every sha256/size matches')

        # 9: expected layout
        problem = _layout_problem(infos, manifest, top_dir)
        if problem:
            return _abort(checks, 'EXPECTED_LAYOUT', problem)
        _passed(checks, 'EXPECTED_LAYOUT', f'single top-level directory {top_dir!r}, no executables')

        # 10: version / suite corroboration
        if not versions.tag_matches_format(release.tag_name, manifest['format_version']):
            return _abort(checks, 'VERSION_MATCH',
                          f'release tag {release.tag_name!r} does not match manifest '
                          f'format_version {manifest["format_version"]!r}')
        if expected_suite is not None and manifest['suite'] != expected_suite:
            return _abort(checks, 'VERSION_MATCH',
                          f'package suite {manifest["suite"]!r} != installed suite {expected_suite!r}')
        version = versions.normalize_version(release.tag_name)
        _passed(checks, 'VERSION_MATCH',
                f'{release.tag_name} -> {version}, format {manifest["format_version"]}')

    return PackageResult(
        ok=True,
        status='VERIFIED',
        checks=checks,
        manifest=manifest,
        manifest_bytes=manifest_bytes,
        manifest_sha256=manifest_sha,
        files=files,
        top_dir=top_dir,
        version=version,
        format_version=manifest['format_version'],
        suite=manifest['suite'],
    )


def _sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _member_paths_problem(infos, limits: PackageLimits) -> str | None:
    if len(infos) > limits.max_members:
        return f'archive has {len(infos)} members, limit is {limits.max_members}'
    total = 0
    seen: set[str] = set()
    for info in infos:
        name = info.filename
        stripped = name[:-1] if name.endswith('/') else name
        for problem in package_path_errors(stripped, max_length=limits.max_path_length):
            return f'member {name!r}: {problem}'
        if name in seen:
            return f'duplicate archive member: {name!r}'
        seen.add(name)
        if info.file_size > limits.max_member_uncompressed:
            return (f'member {name!r} is {info.file_size} bytes uncompressed, '
                    f'limit is {limits.max_member_uncompressed}')
        total += info.file_size
        if total > limits.max_total_uncompressed:
            return f'archive expands past the {limits.max_total_uncompressed} byte limit'
        if info.file_size > 0:
            ratio = info.file_size / max(info.compress_size, 1)
            if ratio > limits.max_compression_ratio:
                return (f'member {name!r} compression ratio {ratio:.0f} exceeds '
                        f'the {limits.max_compression_ratio} limit')
    return None


def _special_member_problem(infos) -> str | None:
    for info in infos:
        if info.create_system != 3:
            continue
        mode = info.external_attr >> 16
        if not mode:
            continue
        file_type = stat.S_IFMT(mode)
        if file_type and file_type not in (stat.S_IFREG, stat.S_IFDIR):
            return f'member {info.filename!r} is not a regular file (mode {oct(mode)})'
    return None


def _file_set(archive, infos, manifest, top_dir) -> tuple[dict, str | None]:
    members = {info.filename for info in infos if not info.is_dir()}
    expected = {f'{top_dir}/{entry["path"]}' for entry in manifest['files']}
    expected.add(f'{top_dir}/{MANIFEST_NAME}')
    missing = sorted(expected - members)
    extra = sorted(members - expected)
    if missing:
        return {}, f'archive is missing manifest entries: {", ".join(missing[:5])}'
    if extra:
        return {}, f'archive carries files absent from the manifest: {", ".join(extra[:5])}'
    files = {}
    for entry in manifest['files']:
        name = f'{top_dir}/{entry["path"]}'
        try:
            data = archive.read(name)
        except (zipfile.BadZipFile, OSError, RuntimeError) as exc:
            return {}, f'cannot read {name!r}: {exc}'
        if len(data) != entry['size']:
            return {}, f'{name!r} size {len(data)} != manifest size {entry["size"]}'
        digest = _sha256(data)
        if digest != entry['sha256']:
            return {}, f'{name!r} sha256 {digest} != manifest {entry["sha256"]}'
        files[entry['path']] = data
    return files, None


def _layout_problem(infos, manifest, top_dir) -> str | None:
    for info in infos:
        name = info.filename
        if not name.startswith(f'{top_dir}/'):
            return f'member {name!r} is not under the single top-level directory {top_dir!r}'
    files = [info for info in infos if not info.is_dir()]
    paths = [entry['path'] for entry in manifest['files']]
    for path in paths:
        if path.lower().endswith(NESTED_ARCHIVE_SUFFIXES):
            return f'nested archive is not allowed in the package: {path!r}'
    lowered: dict[str, str] = {}
    for path in paths + [MANIFEST_NAME]:
        key = path.lower()
        if key in lowered and lowered[key] != path:
            return f'member paths differ only by case: {lowered[key]!r} / {path!r}'
        lowered[key] = path
    for info in files:
        mode = info.external_attr >> 16
        if info.create_system == 3 and mode & 0o111:
            return f'executable bit is not expected on {info.filename!r}'
    # a file cannot also be used as a directory
    files_set = set(paths) | {MANIFEST_NAME}
    for path in files_set:
        parts = path.split('/')
        for index in range(1, len(parts)):
            prefix = '/'.join(parts[:index])
            if prefix in files_set:
                return f'package path {prefix!r} is both a file and a directory'
    return None


def extract_package(pkg: PackageResult, directory: str | os.PathLike) -> Path:
    """Write verified members under ``directory`` and return the package root.

    Member paths have already passed ``MEMBER_PATHS``; the containment check here
    is a second, independent guard so a future verifier regression cannot turn
    into a filesystem write outside the staging directory.
    """
    root = Path(directory) / pkg.top_dir
    root.mkdir(parents=True, exist_ok=True)
    for rel, data in sorted(pkg.files.items()):
        target = root / rel
        parent = target.parent
        parent.mkdir(parents=True, exist_ok=True)
        resolved_parent = parent.resolve()
        if resolved_parent != root.resolve() and root.resolve() not in resolved_parent.parents:
            raise ValueError(f'extraction escaped the staging root: {rel!r}')
        target.write_bytes(data)
    (root / MANIFEST_NAME).write_bytes(pkg.manifest_bytes)
    return root
