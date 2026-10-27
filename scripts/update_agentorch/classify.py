"""Three-way classification of baseline / current / target files.

The table in ``docs/self-improvement/update-safety.md`` §4 is the authority.
This module never touches the filesystem: it consumes sha256 maps and returns the
per-file classification plus counts. Only ``UPSTREAM_UNCHANGED`` may ever be
marked ``APPLIED``; every project-owned, unknown, removed, renamed or conflicting
path is kept and reported.
"""
from __future__ import annotations

from dataclasses import dataclass, field

CLASSES = (
    'UPSTREAM_UNCHANGED',
    'ALREADY_CURRENT',
    'LOCAL_MODIFICATION',
    'THREE_WAY_CONFLICT',
    'PROJECT_OWNED',
    'UNKNOWN_FILE',
    'UPSTREAM_REMOVAL',
    'UPSTREAM_RENAME',
    'UNSAFE_PACKAGE',
    'INVALID_BASELINE',
)

# Classes that forbid an APPLIED result (mirrors update-report.schema.json).
CONFLICT_CLASSES = (
    'THREE_WAY_CONFLICT',
    'UPSTREAM_REMOVAL',
    'UPSTREAM_RENAME',
    'UNSAFE_PACKAGE',
    'INVALID_BASELINE',
)

AUTO_APPLY_CLASS = 'UPSTREAM_UNCHANGED'

# docs/self-improvement/update-safety.md §5: project-owned paths, never overwritten
# even when the old manifest claims them.
PROJECT_OWNED_FILES = frozenset({
    'ORCHESTRATION-PROJECT.md',
    'docs/PROJECT.md',
    'PLAN.md',
    'docs/PLAN.md',
    'DESIGN.md',
    'docs/DESIGN.md',
})
PROJECT_OWNED_PREFIXES = (
    '.github/',
    '.local/',
    '.orchestration/',
    '.worktrees/',
    '.workbuddy/',
    'docs/',
    'src/',
    'release/',
)


def is_project_owned(path: str) -> bool:
    if path in PROJECT_OWNED_FILES:
        return True
    return any(path.startswith(prefix) for prefix in PROJECT_OWNED_PREFIXES)


@dataclass
class Classification:
    files: list[dict] = field(default_factory=list)
    counts: dict[str, int] = field(default_factory=dict)
    conflicts: list[str] = field(default_factory=list)
    removals: list[str] = field(default_factory=list)
    renames: list[str] = field(default_factory=list)

    @property
    def blocked(self) -> bool:
        return any(self.counts[name] for name in CONFLICT_CLASSES)


def _empty_counts() -> dict[str, int]:
    return {name: 0 for name in CLASSES}


def _entry(path: str, klass: str, decision: str, *, baseline=None, current=None,
           target=None, notes=None) -> dict:
    return {
        'path': path,
        'class': klass,
        'baseline_sha256': baseline,
        'current_sha256': current,
        'target_sha256': target,
        'decision': decision,
        'notes': notes,
    }


def _rename_pairs(baseline: dict[str, str], target: dict[str, str]) -> dict[str, str]:
    """Map an old upstream path to a new one when the bytes moved verbatim."""
    removed = sorted(set(baseline) - set(target))
    added = sorted(set(target) - set(baseline))
    pairs: dict[str, str] = {}
    used: set[str] = set()
    for old in removed:
        for new in added:
            if new in used:
                continue
            if target[new] == baseline[old]:
                pairs[old] = new
                used.add(new)
                break
    return pairs


def classify_files(baseline: dict[str, str], current: dict[str, str], target: dict[str, str],
                   *, mode: str = 'APPLY') -> Classification:
    """Classify every path seen in any of the three snapshots.

    ``baseline`` / ``current`` / ``target`` map relative path -> normalized
    sha256. A missing key means "absent" (not "unchanged").
    """
    if mode not in ('PLAN', 'APPLY'):
        raise ValueError(f'mode must be PLAN or APPLY, got {mode!r}')
    upstream_decision = 'APPLIED' if mode == 'APPLY' else 'DEFERRED'
    rename_map = _rename_pairs(baseline, target)
    target_side = {new: old for old, new in rename_map.items()}
    result = Classification(counts=_empty_counts())

    for path in sorted(set(baseline) | set(current) | set(target)):
        b, c, t = baseline.get(path), current.get(path), target.get(path)

        if is_project_owned(path):
            klass, decision = 'PROJECT_OWNED', 'KEPT_LOCAL'
            notes = 'project-owned path: never overwritten by a kit update (report only)'
        elif path in rename_map:
            klass, decision = 'UPSTREAM_RENAME', 'KEPT_LOCAL'
            notes = (f'upstream renamed this path to {rename_map[path]!r} with identical bytes; '
                     'kept local, deletion/rename needs HD-09')
        elif path in target_side:
            klass, decision = 'UPSTREAM_RENAME', 'KEPT_LOCAL'
            notes = (f'target side of an upstream rename from {target_side[path]!r}; '
                     'not created, rename needs HD-09')
        elif b is not None:
            klass, decision, notes = _baseline_case(b, c, t, upstream_decision)
        else:
            klass, decision, notes = _new_case(c, t, upstream_decision)

        entry = _entry(path, klass, decision, baseline=b, current=c, target=t, notes=notes)
        result.files.append(entry)
        result.counts[klass] += 1
        if klass == 'THREE_WAY_CONFLICT':
            result.conflicts.append(path)
        elif klass == 'UPSTREAM_REMOVAL':
            result.removals.append(path)
        elif klass == 'UPSTREAM_RENAME' and path in rename_map:
            result.renames.append(path)

    return result


def _baseline_case(b: str, c: str | None, t: str | None,
                   upstream_decision: str) -> tuple[str, str, str | None]:
    if c is None:
        if t is None:
            return ('ALREADY_CURRENT', 'NOT_APPLICABLE',
                    'absent locally and no longer shipped upstream; nothing to do')
        if t == b:
            return ('LOCAL_MODIFICATION', 'KEPT_LOCAL',
                    'deleted locally; upstream file is unchanged, keep the local decision')
        return ('THREE_WAY_CONFLICT', 'CONFLICT_REPORTED',
                'deleted locally but changed upstream; manual decision required (HD-10)')
    if t is None:
        return ('UPSTREAM_REMOVAL', 'KEPT_LOCAL',
                'no longer shipped upstream; kept, deletion requires HD-09')
    if c == t:
        return ('ALREADY_CURRENT', 'NOT_APPLICABLE', 'local bytes already equal the target')
    if c == b:
        return ('UPSTREAM_UNCHANGED', upstream_decision,
                'local copy untouched since the baseline; safe to update')
    if t == b:
        return ('LOCAL_MODIFICATION', 'KEPT_LOCAL',
                'locally modified, upstream unchanged; keep the local edit')
    return ('THREE_WAY_CONFLICT', 'CONFLICT_REPORTED',
            'modified locally and upstream; manual merge required (HD-10)')


def _new_case(c: str | None, t: str | None,
              upstream_decision: str) -> tuple[str, str, str | None]:
    if t is None:
        return ('UNKNOWN_FILE', 'KEPT_LOCAL',
                'local file unknown to both manifests; never treated as overwritable')
    if c is None:
        return ('UPSTREAM_UNCHANGED', upstream_decision,
                'new upstream file; safe to add')
    if c == t:
        return ('ALREADY_CURRENT', 'NOT_APPLICABLE',
                'local bytes already equal the new upstream file')
    return ('UNKNOWN_FILE', 'KEPT_LOCAL',
            'upstream now ships this path but a local file already exists; kept and reported')
