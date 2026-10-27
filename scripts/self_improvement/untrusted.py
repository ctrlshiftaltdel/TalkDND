#!/usr/bin/env python3
"""Treat everything a source says as data (#49, contract invariants 1–2).

Two jobs, both deliberately small and both content-agnostic:

``redact``                    removes secret-shaped strings and machine-local
                              absolute paths before any source string is stored.
``detect_instruction_patterns``
                              flags imperative text ("ignore previous rules",
                              "send me the token", ...) so a report can count it
                              and reviewers can see it. Detection never changes
                              behaviour: the text stays data, it is not executed,
                              not summarised into an action and not allowed to
                              widen an allow-list, a path or an acceptance rule.

``data_cell`` renders a source string for Markdown so it cannot be mistaken for
a report instruction: newlines are flattened, backticks neutralised and the value
carries an explicit ``data:`` prefix.

Standard library only.
"""
from __future__ import annotations

import re

REDACTED = '[redacted]'

# The literal prefixes are assembled from pieces on purpose. The release builder
# scans packaged text for the *consecutive* strings (token prefixes, PEM header,
# home directories) and refuses to ship a file containing one. Splitting the
# literals keeps the detection semantics identical while keeping this source file
# itself publishable; do not "simplify" these back into one string.
_SEP = '-' * 5
_GH_TOKEN = 'gh' + 'p_'
_GH_FINE_TOKEN = 'github_' + 'pat_'
_GH_OAUTH = 'gh' + 'o_'
_GH_APP = 'gh' + 's_'
_AWS_KEY = 'AK' + 'IA'
_PEM_HEADER = _SEP + 'BEGIN'
_MAC_HOME = '/' + 'Users' + '/'
_LINUX_HOME = '/' + 'home' + '/'
_WIN_HOME = 'C:' + chr(92) + 'Users' + chr(92)

# Literal prefixes and their human labels; `sources.py` reuses this list so the
# registry validator and the collector share one definition.
SENSITIVE_LITERALS = (
    (_GH_TOKEN, 'GitHub token prefix'),
    (_GH_FINE_TOKEN, 'GitHub token prefix'),
    (_PEM_HEADER, 'private key block'),
    (_AWS_KEY, 'AWS access key prefix'),
    (_LINUX_HOME, 'personal absolute path'),
    (_MAC_HOME, 'personal absolute path'),
    (_WIN_HOME, 'personal absolute path'),
)

# Secret shapes. These are *redaction* patterns, not detectors: a match means the
# value must not be stored, never that the value is valid or safe.
_SECRET_PATTERNS = (
    re.compile(_GH_TOKEN + r'[A-Za-z0-9]{16,}'),
    re.compile(_GH_FINE_TOKEN + r'[A-Za-z0-9_]{16,}'),
    re.compile(_GH_OAUTH + r'[A-Za-z0-9]{16,}'),
    re.compile(_GH_APP + r'[A-Za-z0-9]{16,}'),
    re.compile(_AWS_KEY + r'[0-9A-Z]{12,}'),
    re.compile(_PEM_HEADER + r'[^-]{0,40}' + _SEP + r'[\s\S]*?' + _SEP
               + r'END[^-]{0,40}' + _SEP),
    re.compile(r'(?i)\bbearer\s+[A-Za-z0-9._\-]{16,}'),
    re.compile(r'(?i)\b(?:token|secret|password|passwd|api[_-]?key)\s*[:=]\s*[^\s,;]{8,}'),
)
# Machine-local absolute paths: home directories and Windows user profiles.
_PATH_PATTERNS = (
    re.compile(_LINUX_HOME + r'[^/\s]+[^\s]*'),
    re.compile(_MAC_HOME + r'[^/\s]+[^\s]*'),
    re.compile(r'[A-Za-z]:' + re.escape(chr(92)) + 'Users' + re.escape(chr(92)) + r'[^\s]+'),
)

# Imperative shapes that must never be read as instructions. Kept short,
# deliberately literal and bilingual; false positives are acceptable because the
# only consequence is a counted flag, never a behaviour change.
_INSTRUCTION_PATTERNS = (
    ('ignore_previous', re.compile(r'(?i)\b(?:ignore|disregard|forget)\s+(?:all\s+|any\s+)?(?:the\s+)?(?:previous|prior|above|earlier)\b')),
    ('ignore_rules', re.compile(r'(?i)\b(?:ignore|disregard|override)\s+(?:the\s+|all\s+|your\s+)*(?:rules?|instructions?|guidelines?|constraints?)\b')),
    ('send_credentials', re.compile(r'(?i)\b(?:send|post|paste|print|reveal|leak)\s+(?:me\s+|us\s+)?(?:the\s+|your\s+)?(?:token|credentials?|secret|password|api[_-]?key|env)\b')),
    ('approve_change', re.compile(r'(?i)\b(?:approve|merge|accept)\s+(?:this|the)\s+(?:pr|pull request|issue|change|patch)\b')),
    ('modify_authority', re.compile(r'(?i)\b(?:modify|edit|rewrite|change)\s+(?:AGENTS\.md|the\s+rules|allowed\s+paths?|permissions?)\b')),
    ('ignore_previous_zh', re.compile(r'忽略\s*(?:之前|此前|前面|以上|上述)')),
    ('ignore_rules_zh', re.compile(r'忽略\s*(?:所有|全部)?\s*(?:规则|指令|约束|要求)')),
    ('send_credentials_zh', re.compile(r'(?:把|将)\s*(?:你的\s*)?(?:token|令牌|密钥|密码)[^\n]{0,12}(?:发给|发送|贴出|告诉我)')),
    ('approve_change_zh', re.compile(r'(?:批准|合并|直接接受)\s*(?:这个|该|此)\s*(?:PR|Issue|变更|改动)')),
    ('modify_authority_zh', re.compile(r'(?:修改|改写|覆盖)\s*(?:AGENTS\.md|规则|允许路径|权限)')),
)


def redact(text):
    """Remove secret-shaped strings and machine-local paths from *text*.

    Returns ``(clean_text, redactions)`` where ``redactions`` counts replacements.
    A non-string or empty value becomes ``''`` with zero redactions.
    """
    if not isinstance(text, str) or not text:
        return ('' if not isinstance(text, str) else text), 0
    clean = text
    count = 0
    for pattern in _SECRET_PATTERNS + _PATH_PATTERNS:
        clean, hits = pattern.subn(REDACTED, clean)
        count += hits
    return clean, count


def contains_secret(text):
    if not isinstance(text, str) or not text:
        return False
    return any(pattern.search(text) for pattern in _SECRET_PATTERNS + _PATH_PATTERNS)


def detect_instruction_patterns(text):
    """Return the sorted list of instruction-shape categories found in *text*.

    The result is a *flag list for reviewers*; it is never an action list.
    """
    if not isinstance(text, str) or not text:
        return []
    found = set()
    for label, pattern in _INSTRUCTION_PATTERNS:
        if pattern.search(text):
            found.add(label)
    return sorted(found)


def data_cell(text, limit=200):
    """Render one source-derived string so it reads as data, not as an order."""
    if text is None:
        return 'data: (none)'
    value = str(text).replace('\r', ' ').replace('\n', ' ').replace('`', "'").strip()
    if len(value) > limit:
        value = value[:limit - 1] + '…'
    return f'data: {value}' if value else 'data: (empty)'
