"""Rust dependency adapter: capability-gated lexical `use` scanning.

The frozen design (docs/architecture/architecture-contract.md §8) does not claim
Rust support. This adapter is deliberately conservative:

- availability is probed with `shutil.which` for `rustc` and `cargo`; when either
  is missing the capability is reported as `UNKNOWN` and every Rust-relevant rule
  is `NOT_RUN`, never `PASS`;
- even with the toolchain present the scan is **lexical**, not rustc-driven: it
  reads `use` declarations to find first-party module references. Braced /
  glob / macro-generated imports are reported as `UNKNOWN`;
- references into crates other than the current one (`use serde::...`) are not
  project dependencies and are ignored.

The toolchain only gates whether the rules may run at all. The checker never
invokes cargo, so there is no network access and no third-party runtime
dependency.
"""
from __future__ import annotations

import re
import shutil
from pathlib import Path

SKIP_DIRS = frozenset({
    '.git', '.hg', '.svn', '.worktrees', '.orchestration', '.venv', 'venv',
    'node_modules', '__pycache__', 'target', '.local',
})

USE_RE = re.compile(r'^\s*(?:pub\s+)?use\s+(?P<path>[^;]+);')
COMPLEX_RE = re.compile(r'[{}*\n]')


def toolchain():
    """Absolute paths of the Rust tools this adapter gates on, or None."""
    return {'rustc': shutil.which('rustc'), 'cargo': shutil.which('cargo')}


def available():
    probes = toolchain()
    return all(probes.values()), probes


def _rust_files(project_root):
    root = Path(project_root)
    for path in sorted(root.rglob('*.rs')):
        if not path.is_file():
            continue
        relative = path.relative_to(root)
        if SKIP_DIRS & set(relative.parts):
            continue
        yield relative.as_posix(), path


def _module_path(relative):
    """`crates/core/src/util.rs` -> ['util']; `src/lib.rs` -> []."""
    parts = relative[:-len('.rs')].split('/')
    if parts and parts[-1] in ('lib', 'main', 'mod'):
        parts = parts[:-1]
    if 'src' in parts:
        parts = parts[parts.index('src') + 1:]
    return parts


class ModuleIndex:
    def __init__(self, files):
        self.by_path = {}
        for relative in files:
            parts = _module_path(relative)
            if parts:
                self.by_path.setdefault(tuple(parts), relative)

    def resolve(self, parts):
        """Longest declared prefix wins: crate::a::b::Thing -> a/b.rs or a.rs."""
        candidate = list(parts)
        while candidate:
            found = self.by_path.get(tuple(candidate))
            if found:
                return found
            candidate = candidate[:-1]
        return None


def scan(project_root):
    """Return (records, unknowns, info); availability is the caller's concern."""
    ready, probes = available()
    info = {'available': ready, 'toolchain': probes, 'files': 0, 'mode': 'lexical-use-scan'}
    if not ready:
        info['reason'] = ('rustc/cargo not found on PATH: Rust rules are NOT_RUN and the '
                          'capability is UNKNOWN')
        return [], [], info

    records, unknowns = [], []
    files = list(_rust_files(project_root))
    info['files'] = len(files)
    index = ModuleIndex([relative for relative, _ in files])
    for relative, path in files:
        try:
            lines = path.read_text(encoding='utf-8').splitlines()
        except (OSError, UnicodeDecodeError) as exc:
            unknowns.append({'file': relative, 'line': None,
                             'message': f'file could not be read: {exc}'})
            continue
        base = _module_path(relative)
        for lineno, line in enumerate(lines, 1):
            match = USE_RE.match(line)
            if not match:
                continue
            raw = match.group('path').strip()
            if COMPLEX_RE.search(raw):
                unknowns.append({'file': relative, 'line': lineno,
                                 'message': f'complex use declaration {raw!r}: braced/glob '
                                            'imports are not resolved; edge is UNKNOWN'})
                continue
            target = _resolve_use(raw, base, index)
            if target is None:
                continue
            records.append({'source_file': relative, 'line': lineno, 'target_file': target,
                            'dotted': raw, 'text': f'use {raw};', 'conditional': False,
                            'dynamic': False})
    return records, unknowns, info


def _resolve_use(raw, base, index):
    parts = [part for part in raw.split('::') if part]
    if not parts:
        return None
    if parts[0] == 'crate':
        return index.resolve(parts[1:])
    if parts[0] == 'super':
        levels = 0
        while levels < len(parts) and parts[levels] == 'super':
            levels += 1
        prefix = base[:max(0, len(base) - levels)]
        return index.resolve(prefix + parts[levels:])
    if parts[0] == 'self':
        return index.resolve(base + parts[1:])
    # Anything else is another crate (std / serde / ...): not a project edge.
    return None
