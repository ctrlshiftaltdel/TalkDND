#!/usr/bin/env python3
"""Read-only drift diagnosis between a project tree and a shipped MANIFEST.json.

Two jobs live here, and only here:

1. **Manifest generation** (`build_manifest` / `dumps_manifest`). The release
   builder imports this module instead of keeping its own copy of the rules, so
   the classification below is the single definition used by both the build and
   the downstream diagnosis (`build_release.py` imports `check_drift`).

2. **Drift diagnosis** (`diagnose`). Given a project root, compare the files on
   disk against the manifest that shipped with the kit and report:

   - `missing`  - a manifest entry that is absent from the tree (drift)
   - `modified` - a `core` / `edition` / `template` entry whose bytes changed
                  (drift); checks hash *and* size
   - `project_profile` - `docs/PROJECT.md` / `docs/PLAN.md` (current) or legacy root
                  or `docs/PLAN.md`) / `DESIGN.md` edits and additions;
                  expected after a project fills them in, so reported but
                  **never** counted as drift
   - `unknown`  - a local file that sits inside an upstream directory
                  (`editions/`, `templates/`, `scripts/`, `prompts/`, `tests/`)
                  but is not in the manifest; it may be a leftover from another
                  kit version or a local fork of a kit-owned path (drift)
   - `extensions` - every other local file (project source, docs, tests added
                  outside the upstream namespaces, ...); informational only

   `project-extension` therefore means "the project added this, the kit never
   owned that path"; `unknown` means "this path is inside a namespace the kit
   owns, yet the manifest does not know it". Neither is silently dropped, and
   neither is ever rewritten.

Read-only guarantee: this module only opens files for reading (`read_bytes`,
`read_text`), lists directories, and never creates, renames, deletes or writes
anything under the project root.

Exit codes: 0 clean, 2 drift (including "no MANIFEST.json: version unknown /
baseline registration required"), 1 invalid input or tool error. With `--json`,
stdout carries exactly one JSON document and the human summary goes to stderr,
so the output can be piped into another tool.

Byte normalization: hashes are computed over the same normalized bytes the
release archive ships (text = no NUL byte and valid UTF-8; CRLF folded to LF).
Without that, a Windows checkout with `core.autocrlf=true` would report every
text file as modified even though the kit is untouched.

Only the standard library is used.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path

VERSION_FILE = 'VERSION.md'
MANIFEST_NAME = 'MANIFEST.json'

# Schema of the report this tool prints. Bump only on an incompatible field
# change; consumers key off `report_schema_version`.
REPORT_SCHEMA_VERSION = 1
# Schema of MANIFEST.json. Bump when entries gain/lose required fields.
MANIFEST_SCHEMA_VERSION = 1
SUPPORTED_MANIFEST_SCHEMAS = (1,)

# Categories a manifest entry may carry. `project-extension` and `unknown` are
# diagnosis-only: they describe local files the manifest never lists.
UPSTREAM_CATEGORIES = ('core', 'edition', 'template', 'project-profile')
LOCAL_CATEGORIES = ('project-profile', 'project-extension', 'unknown')
CATEGORIES = UPSTREAM_CATEGORIES + ('project-extension', 'unknown')

# Per-project files. Their content is meant to change after installation, so a
# hash difference is reported as `project_profile`, never as drift. The root
# PLAN.md is the legacy (<=1.3) plan location; docs/PLAN.md is the current one.
PROJECT_PROFILE_PATHS = ('ORCHESTRATION-PROJECT.md', 'docs/PROJECT.md', 'PLAN.md',
                         'docs/PLAN.md', 'DESIGN.md', 'docs/DESIGN.md')
# Directories whose contents the manifest owns. A file that appears here without
# a manifest entry is `unknown` (drift), not a project extension.
UPSTREAM_NAMESPACES = ('editions/', 'templates/', 'scripts/', 'prompts/', 'tests/')
# Never walked: VCS metadata, machine-local state, caches and build output.
SKIP_DIRS = frozenset({
    '.git', '.orchestration', '.local', '.worktrees', '.workbuddy',
    '__pycache__', 'release', 'node_modules', '.venv', '.idea', '.vscode',
    # Build output and tool caches. A project that vendors a Rust crate, runs
    # pytest, or indexes itself with codegraph / opencode would otherwise report
    # thousands of generated files as `extension` (or `unknown` when the cache
    # sits inside an upstream namespace) and drown the real drift.
    'target', '.pytest_cache', '.codegraph', '.opencode',
})
# A manifest carrying any of these keys is rejected: a timestamp would make two
# builds of one tree differ and destroy byte-level reproducibility.
FORBIDDEN_MANIFEST_KEYS = ('generated_at', 'generated_on', 'timestamp', 'built_at', 'date')

STATUS_CLEAN = 'clean'
STATUS_DRIFT = 'drift'
STATUS_VERSION_UNKNOWN = 'version-unknown'
STATUS_ERROR = 'error'

EXIT_CLEAN = 0
EXIT_ERROR = 1
EXIT_DRIFT = 2

SHA256_RE = re.compile(r'^[0-9a-f]{64}$')


class DriftError(Exception):
    """Invalid input or an unreadable/invalid manifest (exit code 1)."""


# --- byte normalization (shared with scripts/build_release.py) ---------------

def is_text(data):
    """True when *data* should be treated as a text file.

    Text contains no NUL byte and decodes as UTF-8. Anything else is a binary
    payload and ships verbatim.
    """
    if b'\x00' in data:
        return False
    try:
        data.decode('utf-8')
    except UnicodeDecodeError:
        return False
    return True


def normalize_line_endings(data):
    """Return *data* with CRLF folded to LF when it is a text file.

    Hashes must not depend on the checkout's line endings: the same committed
    tree can be materialized as LF or CRLF, so the manifest is built from folded
    bytes and the downstream check folds local bytes the same way. Only CRLF is
    touched; a lone CR and a UTF-8 BOM are preserved.
    """
    if not is_text(data):
        return data
    return data.replace(b'\r\n', b'\n')


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def read_normalized(path):
    """Bytes of *path* as the release archive would ship them."""
    return normalize_line_endings(Path(path).read_bytes())


# --- classification (single definition) ------------------------------------

def classify_upstream(path):
    """Category of a file that belongs to the kit (listed in validate_kit.REQUIRED)."""
    if path.startswith('editions/'):
        return 'edition'
    if path.startswith('templates/'):
        return 'template'
    if path in PROJECT_PROFILE_PATHS:
        return 'project-profile'
    return 'core'


def classify_local(path):
    """Category of a file found in a project but absent from the manifest."""
    if path in PROJECT_PROFILE_PATHS:
        return 'project-profile'
    if path.startswith(UPSTREAM_NAMESPACES):
        return 'unknown'
    return 'project-extension'


# --- version + manifest generation -----------------------------------------

def read_version(root):
    """Parse VERSION.md -> (suite, format_version). Raises DriftError if unreadable."""
    path = Path(root) / VERSION_FILE
    try:
        text = path.read_text(encoding='utf-8')
    except OSError as exc:
        raise DriftError(f'cannot read {VERSION_FILE}: {exc}')
    suite, format_version = '', ''
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if line.startswith('#'):
            line = line.lstrip('#').strip()
        if line.startswith('- '):
            line = line[2:].strip()
        if not line:
            continue
        match = re.match(r'(?i)^format\s+version\s*[:：]\s*(\S+)$', line)
        if match:
            format_version = match.group(1)
            continue
        if not suite and ':' not in line:
            suite = line
    if not suite:
        raise DriftError(f'{VERSION_FILE} does not declare the suite name')
    if not format_version:
        raise DriftError(f'{VERSION_FILE} does not declare "Format version: <x.y>"')
    return suite, format_version


def build_manifest(files, suite, format_version):
    """Build the manifest payload from {relative_path: packaged_bytes}.

    Entries are sorted by path, keys are fixed, and no timestamp is recorded, so
    two builds of one tree produce byte-identical output. MANIFEST.json never
    lists itself (its own hash cannot be embedded).
    """
    entries = []
    for path in sorted(files):
        if path == MANIFEST_NAME:
            continue
        # Normalize here as well: the manifest must describe the bytes the
        # archive ships, whatever the caller's checkout line endings were.
        data = normalize_line_endings(files[path])
        entries.append({
            'path': path,
            'category': classify_upstream(path),
            'sha256': sha256_bytes(data),
            'size': len(data),
        })
    return {
        'schema_version': MANIFEST_SCHEMA_VERSION,
        'suite': suite,
        'format_version': format_version,
        'files': entries,
    }


def dumps_manifest(manifest):
    """Deterministic JSON text for a manifest payload."""
    return json.dumps(manifest, indent=2, sort_keys=True, ensure_ascii=False) + '\n'


# --- manifest validation ----------------------------------------------------

def manifest_structure_errors(manifest):
    """Structural/contract errors; no filesystem access.

    Used by the diagnosis (so a *tampered file* is reported as drift, not as an
    unreadable manifest) and, on top of the hash checks, by the build/validator.
    """
    errors = []
    if not isinstance(manifest, dict):
        return ['manifest must be a JSON object']
    allowed_top = {'schema_version', 'suite', 'format_version', 'files'}
    for key in sorted(manifest):
        if key in FORBIDDEN_MANIFEST_KEYS:
            errors.append(f'manifest must not carry {key!r}: timestamps break reproducibility')
        elif key not in allowed_top:
            errors.append(f'manifest carries unknown top-level key: {key}')
    for key in ('schema_version', 'suite', 'format_version', 'files'):
        if key not in manifest:
            errors.append(f'manifest is missing top-level key: {key}')
    schema = manifest.get('schema_version')
    if not isinstance(schema, int) or isinstance(schema, bool) or schema < 1:
        errors.append(f'manifest schema_version must be a positive integer, got {schema!r}')
    for key in ('suite', 'format_version'):
        value = manifest.get(key)
        if not isinstance(value, str) or not value.strip():
            errors.append(f'manifest {key} must be a non-empty string')
    files = manifest.get('files')
    if not isinstance(files, list) or not files:
        errors.append('manifest files must be a non-empty list')
        return errors

    seen = set()
    previous = None
    for index, entry in enumerate(files):
        label = f'manifest files[{index}]'
        if not isinstance(entry, dict):
            errors.append(f'{label} must be an object')
            continue
        allowed = {'path', 'category', 'sha256', 'size'}
        for key in sorted(entry):
            if key not in allowed:
                errors.append(f'{label} carries unknown key: {key}')
        for key in ('path', 'category', 'sha256', 'size'):
            if key not in entry:
                errors.append(f'{label} is missing key: {key}')
        path = entry.get('path')
        if not isinstance(path, str) or not path:
            errors.append(f'{label} path must be a non-empty string')
            continue
        if '\\' in path or path.startswith('/') or ':' in path:
            errors.append(f'{label} path must be a relative POSIX path: {path}')
        if '..' in path.split('/') or '.' in path.split('/'):
            errors.append(f'{label} path must not contain "." or ".." segments: {path}')
        if path == MANIFEST_NAME:
            errors.append(f'{label} must not list {MANIFEST_NAME} itself')
        if path in seen:
            errors.append(f'{label} duplicates path: {path}')
        if previous is not None and path < previous:
            errors.append(f'{label} is not sorted by path: {path}')
        previous = path
        seen.add(path)
        category = entry.get('category')
        if category not in CATEGORIES:
            errors.append(f'{label} has an unknown category: {category!r}')
        elif category != classify_upstream(path):
            errors.append(f'{label} category {category!r} does not match the '
                          f'single classification rule for {path} ({classify_upstream(path)!r})')
        digest = entry.get('sha256')
        if not isinstance(digest, str) or not SHA256_RE.match(digest):
            errors.append(f'{label} sha256 must be 64 lowercase hex characters')
        size = entry.get('size')
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            errors.append(f'{label} size must be a non-negative integer, got {size!r}')
    return errors


def verify_manifest_files(root, manifest):
    """Hash/size errors between manifest entries and the files under *root*."""
    errors = []
    root = Path(root)
    for entry in manifest.get('files', []):
        if not isinstance(entry, dict):
            continue
        path = entry.get('path')
        if not isinstance(path, str) or not path:
            continue
        target = root / path
        if not target.is_file():
            errors.append(f'manifest path is missing on disk: {path}')
            continue
        try:
            data = read_normalized(target)
        except OSError as exc:
            errors.append(f'cannot read {path}: {exc}')
            continue
        digest = sha256_bytes(data)
        if digest != entry.get('sha256'):
            errors.append(f'manifest sha256 does not match the file: {path}')
        if len(data) != entry.get('size'):
            errors.append(f'manifest size does not match the file: {path}')
    return errors


def validate_manifest(root, manifest):
    """Full manifest validation used by the build and by scripts/validate_kit.py."""
    return manifest_structure_errors(manifest) + verify_manifest_files(root, manifest)


# --- diagnosis --------------------------------------------------------------

def _empty_findings():
    return {'missing': [], 'modified': [], 'project_profile': [], 'unknown': [], 'extensions': []}


def _empty_report(root):
    return {
        'tool': 'check_drift',
        'report_schema_version': REPORT_SCHEMA_VERSION,
        'root': root,
        'status': STATUS_CLEAN,
        'exit_code': EXIT_CLEAN,
        'manifest': {'present': False, 'schema_version': None,
                     'suite': None, 'format_version': None},
        'version_span': {'manifest_schema_version': None,
                         'supported_schema_versions': list(SUPPORTED_MANIFEST_SCHEMAS),
                         'supported': None},
        'summary': {'checked': 0, 'missing': 0, 'modified': 0, 'project_profile': 0,
                    'unknown': 0, 'extensions': 0},
        'findings': _empty_findings(),
        'notes': [],
    }


def iter_local_files(root):
    """Sorted relative POSIX paths of every regular file under *root* (read-only walk)."""
    root = Path(root)
    found = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
        for name in filenames:
            full = Path(dirpath) / name
            if full.is_symlink() or not full.is_file():
                continue
            found.append(full.relative_to(root).as_posix())
    return sorted(found)


def load_manifest_structure(root):
    """Read + structurally validate MANIFEST.json; None when absent."""
    path = Path(root) / MANIFEST_NAME
    if not path.is_file():
        return None
    try:
        text = path.read_text(encoding='utf-8')
    except OSError as exc:
        raise DriftError(f'cannot read {MANIFEST_NAME}: {exc}')
    try:
        manifest = json.loads(text)
    except ValueError as exc:
        raise DriftError(f'{MANIFEST_NAME} is not valid JSON: {exc}')
    errors = manifest_structure_errors(manifest)
    if errors:
        raise DriftError(f'{MANIFEST_NAME} is invalid: ' + '; '.join(errors))
    return manifest


def diagnose(root):
    """Return the stable report dict for *root*. Never writes anything."""
    root_path = Path(root)
    if not root_path.exists():
        raise DriftError(f'project root does not exist: {root}')
    if not root_path.is_dir():
        raise DriftError(f'project root is not a directory: {root}')
    resolved = str(root_path.resolve())
    report = _empty_report(resolved)

    manifest = load_manifest_structure(root_path)
    if manifest is None:
        report['status'] = STATUS_VERSION_UNKNOWN
        report['exit_code'] = EXIT_DRIFT
        report['notes'].extend([
            f'no {MANIFEST_NAME} in this project: suite version unknown, no baselines to compare',
            'register a baseline: keep the manifest from the kit release you started from '
            '(or rebuild one with scripts/build_release.py), the tool never guesses a version',
            'this is a diagnosis only; reading and using the project is not blocked',
        ])
        return report

    report['manifest'] = {
        'present': True,
        'schema_version': manifest['schema_version'],
        'suite': manifest['suite'],
        'format_version': manifest['format_version'],
    }
    schema = manifest['schema_version']
    supported = schema in SUPPORTED_MANIFEST_SCHEMAS
    report['version_span'] = {
        'manifest_schema_version': schema,
        'supported_schema_versions': list(SUPPORTED_MANIFEST_SCHEMAS),
        'supported': supported,
    }
    if not supported:
        report['notes'].append(
            f'manifest schema_version {schema} is outside this tool\'s supported range '
            f'{list(SUPPORTED_MANIFEST_SCHEMAS)}; hash checks were still performed, '
            'update the kit to interpret the newer manifest')

    expected = {entry['path']: entry for entry in manifest['files']}
    findings = report['findings']
    report['summary']['checked'] = len(expected)
    for path in sorted(expected):
        entry = expected[path]
        target = root_path / path
        if not target.is_file():
            findings['missing'].append({'path': path, 'category': entry['category']})
            continue
        try:
            data = read_normalized(target)
        except OSError as exc:
            raise DriftError(f'cannot read project file {path}: {exc}')
        digest = sha256_bytes(data)
        if digest == entry['sha256'] and len(data) == entry['size']:
            continue
        item = {
            'path': path,
            'category': entry['category'],
            'expected_sha256': entry['sha256'],
            'actual_sha256': digest,
            'expected_size': entry['size'],
            'actual_size': len(data),
        }
        if entry['category'] == 'project-profile':
            findings['project_profile'].append(item)
        else:
            findings['modified'].append(item)

    for path in iter_local_files(root_path):
        if path in expected or path == MANIFEST_NAME:
            continue
        category = classify_local(path)
        if category == 'project-profile':
            findings['project_profile'].append({
                'path': path, 'category': category, 'status': 'added locally'})
        elif category == 'unknown':
            findings['unknown'].append({
                'path': path, 'category': category,
                'note': 'inside an upstream directory but absent from the manifest'})
        else:
            findings['extensions'].append({'path': path, 'category': category})

    for key in ('missing', 'modified', 'project_profile', 'unknown', 'extensions'):
        report['summary'][key] = len(findings[key])

    drift = bool(findings['missing'] or findings['modified'] or findings['unknown'] or not supported)
    report['status'] = STATUS_DRIFT if drift else STATUS_CLEAN
    report['exit_code'] = EXIT_DRIFT if drift else EXIT_CLEAN
    if findings['extensions']:
        report['notes'].append(
            f'{len(findings["extensions"])} local project file(s) are extensions; '
            'they are informational and never overwritten')
    if findings['project_profile']:
        report['notes'].append(
            f'{len(findings["project_profile"])} project-profile file(s) differ from the '
            'shipped template, which is expected after a project fills them in')
    return report


def error_report(root, message):
    """Same shape as a normal report, so JSON consumers see stable keys."""
    try:
        resolved = str(Path(root).resolve())
    except OSError:
        resolved = str(root)
    report = _empty_report(resolved)
    report['status'] = STATUS_ERROR
    report['exit_code'] = EXIT_ERROR
    report['error'] = message
    return report


def render_report(report, stream):
    """Human-readable summary (stdout in text mode, stderr in --json mode)."""
    print(f"status: {report['status']} (exit {report['exit_code']})", file=stream)
    manifest = report['manifest']
    if manifest['present']:
        span = report['version_span']
        print(f"manifest: schema {manifest['schema_version']} "
              f"(supported: {'yes' if span['supported'] else 'no'}), "
              f"suite {manifest['suite']!r}, format {manifest['format_version']}", file=stream)
    else:
        print(f"manifest: absent (no {MANIFEST_NAME})", file=stream)
    summary = report['summary']
    print(f"checked: {summary['checked']} manifest file(s); "
          f"missing {summary['missing']}, modified {summary['modified']}, "
          f"unknown {summary['unknown']}, project-profile {summary['project_profile']}, "
          f"extensions {summary['extensions']}", file=stream)
    labels = (('missing', 'missing'), ('modified', 'modified'),
              ('unknown', 'unknown'), ('project_profile', 'project-profile'),
              ('extensions', 'extension'))
    for key, label in labels:
        for item in report['findings'][key]:
            detail = item.get('note') or item.get('status') or item.get('category') or ''
            print(f"  [{label}] {item['path']}"
                  + (f' - {detail}' if detail else ''), file=stream)
    if report.get('error'):
        print(f"error: {report['error']}", file=stream)
    for note in report['notes']:
        print(f"note: {note}", file=stream)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description='Read-only drift diagnosis between a project tree and MANIFEST.json.')
    parser.add_argument('root', nargs='?', default='.',
                        help='project root to inspect (default: current directory)')
    parser.add_argument('--json', action='store_true',
                        help='print one JSON document on stdout; human summary goes to stderr')
    args = parser.parse_args(argv)

    try:
        report = diagnose(args.root)
    except DriftError as exc:
        report = error_report(args.root, str(exc))
        print(f'ERROR: {exc}', file=sys.stderr)
        if args.json:
            print(json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False))
        return EXIT_ERROR

    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True, ensure_ascii=False))
        render_report(report, sys.stderr)
    else:
        render_report(report, sys.stdout)
    return report['exit_code']


if __name__ == '__main__':
    sys.exit(main())
