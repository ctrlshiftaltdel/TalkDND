"""Version and release-tag normalization.

Frozen rule (docs/self-improvement/update-safety.md §2): a release tag is
``v<major>.<minor>`` or ``v<major>.<minor>.<patch>``. A missing patch is
normalized to ``0`` for comparison; the original tag text is kept for display.
"""
from __future__ import annotations

import re

_TAG_RE = re.compile(r'^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:\.(0|[1-9][0-9]*))?$')
_VERSION_RE = re.compile(r'^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:\.(0|[1-9][0-9]*))?$')
_FORMAT_RE = re.compile(r'^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$')


def _parts(match) -> tuple[int, int, int]:
    major, minor, patch = match.group(1), match.group(2), match.group(3)
    return int(major), int(minor), int(patch or 0)


def parse_tag(tag: object) -> tuple[int, int, int] | None:
    """Return ``(major, minor, patch)`` for a valid release tag, else ``None``."""
    if not isinstance(tag, str):
        return None
    match = _TAG_RE.match(tag)
    return _parts(match) if match else None


def parse_version(text: object) -> tuple[int, int, int] | None:
    """Return ``(major, minor, patch)`` for a version string (tag prefix allowed)."""
    if not isinstance(text, str):
        return None
    value = text[1:] if text.startswith('v') else text
    match = _VERSION_RE.match(value)
    return _parts(match) if match else None


def version_key(text: object) -> tuple[int, int, int]:
    """Sortable key for a version; raises ``ValueError`` for an invalid value."""
    parsed = parse_version(text)
    if parsed is None:
        raise ValueError(f'not a supported version: {text!r}')
    return parsed


def normalize_version(text: object) -> str | None:
    """Canonical ``x.y.z`` form; ``v1.6`` and ``1.6`` both become ``1.6.0``."""
    parsed = parse_version(text)
    if parsed is None:
        return None
    return '{}.{}.{}'.format(*parsed)


def tag_matches_format(tag: object, format_version: object) -> bool:
    """True when the release tag's major/minor equals a manifest format version."""
    if not isinstance(format_version, str):
        return False
    match = _FORMAT_RE.match(format_version)
    if not match:
        return False
    parsed = parse_tag(tag)
    if parsed is None:
        return False
    major, minor, _ = parsed
    return (major, minor) == (int(match.group(1)), int(match.group(2)))
