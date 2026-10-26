#!/usr/bin/env python3
"""Static validation for the Agent-Orchestration kit (core + local/github editions)."""
from pathlib import Path
import argparse, importlib.util, json, re, sys

ROOT = Path(__file__).resolve().parents[1]
# Release package manifest. It exists only in a built package (the repository
# root has none), so every check below is skipped when the file is absent.
MANIFEST_DOC = 'MANIFEST.json'
# Single source of truth for the shippable kit. This list has two jobs at once:
#   1. the structurally required files of this repository, and
#   2. the file manifest of the release package built by scripts/build_release.py.
# scripts/build_release.py parses THIS list to decide what to package, so there is
# no second manifest to maintain. Keep it in sync whenever a versioned artifact is
# added or removed. Files that must not ship (docs/, release/, the release builder
# itself, .orchestration/, .worktrees/, .workbuddy/) are simply not listed here.
REQUIRED = [
    'AGENTS.md', 'LICENSE.md', 'VERSION.md',
    'kit/orchestration.md',
    'editions/github/active-task-registry.md', 'editions/github/issue-template.md',
    'editions/github/execution.md', 'editions/github/master.md',
    'editions/github/pr-template.md',
    'editions/local/master.md', 'editions/local/execution.md',
    'templates/architecture/README.md', 'templates/architecture/contract.example.json',
    'templates/contract-decision-log.md', 'templates/design.md',
    'templates/implementation-plan.md', 'templates/metrics.md',
    'templates/milestone-audit.md', 'templates/plan.md', 'templates/project.md',
    'templates/project-charter.md', 'templates/review-checklist.md',
    'templates/runtime-capability.md', 'templates/task-packet.md',
    'templates/gitignore.snippet',
    'templates/self-improvement/README.md',
    'templates/self-improvement/closure-report.md',
    'templates/self-improvement/proposal.md',
    'templates/self-improvement/sources.example.json',
    'templates/self-improvement/task-packet.md',
    'scripts/architecture/__init__.py', 'scripts/architecture/check.py',
    'scripts/architecture/contract.py', 'scripts/architecture/report.py',
    'scripts/architecture/rules.py', 'scripts/architecture/adapters/__init__.py',
    'scripts/architecture/adapters/python_ast.py', 'scripts/architecture/adapters/rust.py',
    'scripts/check_drift.py', 'scripts/orchestrator.py', 'scripts/report_metrics.py',
    'scripts/run_delivery_scenarios.py', 'scripts/validate_kit.py',
    'scripts/self_improvement/__init__.py', 'scripts/self_improvement/cli.py',
    'scripts/self_improvement/collect.py', 'scripts/self_improvement/lessons.py',
    'scripts/self_improvement/orchestration.py', 'scripts/self_improvement/pipeline.py',
    'scripts/self_improvement/planning.py', 'scripts/self_improvement/sources.py',
    'scripts/self_improvement/untrusted.py',
    'scripts/update_agentorch/__init__.py', 'scripts/update_agentorch/__main__.py',
    'scripts/update_agentorch/classify.py', 'scripts/update_agentorch/cli.py',
    'scripts/update_agentorch/engine.py', 'scripts/update_agentorch/entry.py',
    'scripts/update_agentorch/package.py', 'scripts/update_agentorch/report.py',
    'scripts/update_agentorch/sources.py', 'scripts/update_agentorch/versions.py',
    'scripts/update_agentorch/workspace.py',
    'tests/architecture/__init__.py', 'tests/architecture/test_architecture_checker.py',
    'tests/self_improvement/__init__.py', 'tests/self_improvement/loop_support.py',
    'tests/self_improvement/support.py',
    'tests/self_improvement/test_acceptance_report.py',
    'tests/self_improvement/test_collection.py',
    'tests/self_improvement/test_lesson_lifecycle.py',
    'tests/self_improvement/test_orchestration.py',
    'tests/self_improvement/test_pipeline.py',
    'tests/self_improvement/test_planning.py',
    'tests/self_improvement/test_registry.py',
    'tests/self_improvement/test_routing.py',
    'tests/test_orchestrator.py', 'tests/test_release_drift.py',
    'tests/test_report_metrics.py', 'tests/test_run_delivery_scenarios.py',
    'tests/test_update_entry.py', 'tests/test_validate_kit.py',
    'tests/update_agentorch/__init__.py', 'tests/update_agentorch/support.py',
    'tests/update_agentorch/test_classification.py',
    'tests/update_agentorch/test_dependencies.py',
    'tests/update_agentorch/test_engine.py',
    'tests/update_agentorch/test_entry_release.py',
    'tests/update_agentorch/test_package.py',
    'tests/update_agentorch/test_versions_and_sources.py',
]

# --- Repository-only required documents --------------------------------------
# These files are required in a checkout but must NOT be added to REQUIRED:
# scripts/build_release.py refuses to package any path containing a `docs`
# segment (FORBIDDEN_SEGMENTS), so listing them above would turn every release
# self-check red. They are still required artifacts — a missing file or a
# deleted section fails validate_kit on a checkout — and the check is skipped
# when `docs/` is absent, which is exactly the case inside a release package.
#
# `tests/update_agentorch/test_report.py` is repository-only for the same
# reason: it reads the frozen fixtures under `docs/contracts/fixtures/`, which
# never ship. The package still tests the engine and the entry through the
# shipped `tests/update_agentorch/` modules listed in REQUIRED above.
# Downstream projects must carry docs/PROJECT.md and docs/PLAN.md; these are
# project-owned and checked in every tree that has a docs/ directory.
PROJECT_DOCS = {
    'docs/PROJECT.md': (),
    'docs/PLAN.md': ('设计目标',),
}
# Repository-only documents. They live in the upstream AgentOrch checkout and
# are never shipped in the release package. A downstream project has its own
# docs/ layout and is not expected to copy them; they are checked only when
# the tree is recognisably the upstream checkout (docs/upgrade.md is present).
UPSTREAM_DOCS = {
    'docs/README.md': (),
    'docs/implementation-plan.md': ('当前状态',),
    'docs/charter.md': (),
    'docs/upgrade.md': ('升级',),
    'docs/drift.md': (),
    'docs/metrics.md': (),
    'docs/design/resource-locks.md': ('冲突',),
    'docs/testing/delivery-regression-scenarios.md': (
        '场景矩阵', '运行方式', 'Fixture 规则', '正负组', '未验证范围', '清理',
    ),
}
# Backward-compatible union; validate_kit selects the subset per tree.
REQUIRED_DOCS = {**PROJECT_DOCS, **UPSTREAM_DOCS}

# --- Milestone-audit closeout contract ---------------------------------------
# A filled audit record may only be read as "stage complete" when no verdict
# forbids it. These values are the four classes from templates/milestone-audit.md
# plus PASS (verified) and the two non-pass markers; anything else is a typo or a
# second, invented vocabulary, so it is reported instead of counted.
AUDIT_VERDICTS = ('PASS', 'NON_BLOCKING_HARDENING', 'OPTIONAL', 'NEEDS_HUMAN_DECISION',
                  'BLOCKING', 'NOT_RUN', 'BLOCKED', 'NOT_APPLICABLE')
CLOSEOUT_BLOCKING_VERDICTS = ('BLOCKING', 'NEEDS_HUMAN_DECISION')
CLOSEOUT_UNVERIFIED_VERDICTS = ('NOT_RUN', 'BLOCKED')

# --- Reusable contract templates ---------------------------------------------
# DESIGN.md, MILESTONE-AUDIT.md and CONTRACT-DECISION-LOG.md are contracts in
# their own right: a deleted heading silently removes a question nobody will then
# answer, and a broken relative link makes the cross-reference unusable. Both are
# checked here, on structure only — whether the filled answers are true is a
# review decision, not something a static check can prove.
# Section names are matched case-insensitively as substrings of a heading, so a
# numbered heading ("### 1. Functional completeness") still matches.
TEMPLATE_SECTIONS = {
    'templates/design.md': (
        'Scope and non-goals', 'Schema and interfaces', 'State transitions', 'Invariants',
        'Failure semantics', 'Compatibility and migration', 'Permission and trust boundaries',
        'Verification strategy', 'Frozen decisions', 'Open decisions',
        'Versioning and change policy',
    ),
    'templates/milestone-audit.md': (
        'When to use', 'How to fill', 'Verdict classes',
        'Functional completeness', 'Cross-module invariants', 'Permissions and security',
        'Legacy data migration', 'Real and synthetic data validation', 'End-to-end path',
        'Recoverability', 'Performance baseline', 'Public content and privacy',
        'Residual items', 'Audit conclusion',
    ),
    'templates/contract-decision-log.md': (
        'When to use', 'Status values', 'Frozen decisions', 'Open decisions',
        'Revisitable items', 'Superseded records', 'Recording rules',
    ),
    # A capability record that loses a row silently claims a capability nobody
    # probed; a metric definition that loses a column leaves the start / end
    # event, exclusion or missing-data rule undefined. Both are contracts, so
    # only their structure is checked here, never whether the numbers are true.
    'templates/runtime-capability.md': (
        'When to use', 'Record identity', 'Capability matrix', 'Extended boundaries',
        'UNKNOWN and re-probe', 'Permission scope', 'Evidence and limits',
    ),
    'templates/metrics.md': (
        'When to use', 'Reading rules', 'Common definitions', 'Metric definitions',
        'Missing data and coverage', 'Report contract', 'Gold sample', 'Limits',
    ),
}
HEADING_RE = re.compile(r'^#{2,4}\s+(?P<text>.+?)\s*$', re.M)
MD_LINK_RE = re.compile(r'\[[^\]]*\]\(([^)]+)\)')


# Docs teach the link/image syntax itself (e.g. `![图](images/x.png)` inside
# backticks or fenced blocks). Those spans are literal text, not live links, so
# scanning them reported false "broken relative link" errors on every document
# that documents the convention.
CODE_BLOCK_RE = re.compile(r'```.*?```', re.S)
CODE_SPAN_RE = re.compile(r'`[^`\n]*`')


def markdown_links(text):
    """Markdown link targets outside fenced code blocks and inline code."""
    text = CODE_BLOCK_RE.sub('', text)
    text = CODE_SPAN_RE.sub('', text)
    return MD_LINK_RE.findall(text)
EXTERNAL_LINK_PREFIXES = ('http://', 'https://', 'mailto:', '#')

# --- Evidence matrix contract -------------------------------------------------
# The field names, the status enum and the G1 rules are defined once, in
# kit/orchestration.md 4.1. This module reads that definition back (it does not
# restate it) and enforces it on the local/github task templates. If the doc drifts
# away from the contract floor below, validation fails instead of silently
# accepting a second definition.
CORE_DOC = 'kit/orchestration.md'
TASK_PACKET_DOC = 'templates/task-packet.md'
ISSUE_TEMPLATE_DOC = 'editions/github/issue-template.md'
EVIDENCE_FIELD_ANCHOR = '证据条目（evidence item）字段'
PACKET_FIELD_ANCHOR = '任务包结构化扩展字段'
# Contract floor for `status`. A silently narrowed or extended enum is reported.
STATUS_VALUES = ('PASS', 'FAIL', 'NOT_RUN', 'BLOCKED', 'NOT_APPLICABLE')
NON_PASS_STATUSES = ('FAIL', 'NOT_RUN', 'BLOCKED')
# Text that claims the evidence was not produced. A row marked PASS while saying
# this is "not executed but counted as passing" and must be rejected.
NOT_RUN_MARKERS = re.compile(
    r'\bnot[ _-]?run\b|\bnot executed\b|\bnot verified\b|未执行|未运行|未验证|blocked by|被阻塞', re.I)
SHA_RE = re.compile(r'^[0-9a-f]{40}$')
# Fields every task packet must carry, on top of the structured scenario fields
# read from kit/orchestration.md. Names are the template labels, lower-cased.
PACKET_REQUIRED_FIELDS = (
    'task id', 'title', 'goal / user-visible result', 'context', 'allowed paths',
    'forbidden paths', 'dependencies (must be verified)', 'acceptance criteria',
    'validation commands', 'baseline branch / commit', 'expected branch / worktree',
    'evidence head sha', 'evidence matrix', 'risks / human decisions',
    'delivery evidence', 'hard boundaries',
)
# Fields whose content lives on the indented lines below the label.
PACKET_LIST_FIELDS = ('acceptance criteria', 'evidence matrix')

FIELD_LINE = re.compile(r'^- \*\*(?P<label>[^*]+?)\*\*\s*:?\s*(?P<value>.*)$')
CHECKBOX_LINE = re.compile(r'^\s*-\s*\[(?P<mark>[ xX])\]\s*(?P<body>.*)$')
AC_HEAD = re.compile(r'^(?P<id>AC-\d+)\s*[:：]\s*(?P<text>.*)$')
SEPARATOR_CELL = re.compile(r'^:?-{2,}:?$')

# --- Named resource declarations (shape only) ---------------------------------
# Semantics live in docs/design/RESOURCE-LOCKS.md; this file only checks that a
# filled carrier field can be read back. The field is OPTIONAL: absent or empty
# means "this task takes no named resource lock" and must never be a failure.
RESOURCE_FIELD = 'declared_resources'
LOCKED_RESOURCE_FIELD = 'locked_resources'
RESOURCE_SPEC_RE = re.compile(
    r'(?P<type>[a-z][a-z0-9-]{0,15}):(?P<name>[a-z0-9][a-z0-9._/-]{0,127})'
    r':(?P<mode>shared|exclusive)(?::(?P<ttl>[1-9][0-9]*))?')


def resource_declaration_errors(text, label, field=RESOURCE_FIELD):
    """Errors for a filled resource set; empty/absent values are legal."""
    fields = parse_task_packet(text)
    if field_label(field) not in fields:
        return []
    value = (field_value(fields, field) or '').strip().strip('`').strip()
    if not value:
        return []
    errors, seen = [], set()
    for raw in value.split(','):
        item = raw.strip().strip('`').strip()
        if not item:
            errors.append(f'{label}: {field} has an empty entry (remove the stray comma)')
            continue
        match = RESOURCE_SPEC_RE.fullmatch(item)
        if not match:
            errors.append(f'{label}: {field} entry {item!r} must be '
                          '<resource_id>:<mode>[:<ttl>] with mode shared or exclusive')
            continue
        if any(part in ('', '.', '..') for part in match.group('name').split('/')):
            errors.append(f'{label}: {field} entry {item!r} has an empty or dots-only path segment')
            continue
        resource_id = f'{match.group("type")}:{match.group("name")}'
        if resource_id in seen:
            errors.append(f'{label}: {field} declares {resource_id} more than once')
        seen.add(resource_id)
    return errors


def read_text(path):
    return Path(path).read_text(encoding='utf-8')


def _table_after(text, anchor):
    """Data rows of the first markdown table following the line containing anchor."""
    lines = text.splitlines()
    start = None
    for i, line in enumerate(lines):
        if anchor in line:
            start = i + 1
            break
    if start is None:
        return None
    j = start
    while j < len(lines) and not lines[j].strip().startswith('|'):
        j += 1
    rows = []
    while j < len(lines) and lines[j].strip().startswith('|'):
        rows.append([c.strip() for c in lines[j].strip().strip('|').split('|')])
        j += 1
    rows = [r for r in rows if not all(SEPARATOR_CELL.match(c) for c in r if c)]
    if len(rows) < 2:
        return None
    return rows[1:]  # drop the header row


def _backticked(cell):
    match = re.search(r'`([^`]+)`', cell or '')
    return match.group(1).strip() if match else None


def canonical_evidence_fields(core_text):
    """(field names, status values) as declared by the core orchestration doc."""
    rows = _table_after(core_text, EVIDENCE_FIELD_ANCHOR)
    if not rows:
        return [], []
    fields, statuses = [], []
    for row in rows:
        name = _backticked(row[0]) if row else None
        if not name:
            return [], []
        fields.append(name)
        if name == 'status' and len(row) > 2:
            statuses = re.findall(r'`([A-Z_]+)`', ' '.join(row[1:]))
    return fields, statuses


def canonical_packet_fields(core_text):
    """Structured task-packet field names declared by the core orchestration doc."""
    rows = _table_after(core_text, PACKET_FIELD_ANCHOR)
    if not rows:
        return []
    names = []
    for row in rows:
        name = _backticked(row[0]) if row else None
        if not name:
            return []
        names.append(name)
    return names


def parse_task_packet(text):
    """Map normalised field label -> {'inline': str, 'lines': [str]}."""
    fields = {}
    current = None
    for line in text.splitlines():
        match = FIELD_LINE.match(line)
        if match:
            current = field_label(match.group('label'))
            fields[current] = {'inline': match.group('value').strip(), 'lines': []}
            continue
        if current is not None:
            fields[current]['lines'].append(line)
    return fields


def field_label(name):
    """`unverified_assumptions`, `Unverified assumptions:` -> `unverified assumptions`."""
    label = re.sub(r'[\s_]+', ' ', (name or '').strip()).strip()
    return label.strip(':：').strip().lower()


def _clean(cell):
    return (cell or '').strip().strip('`').strip()


def field_value(fields, name):
    """Scalar value of a field: inline value, else the first non-empty sub-line."""
    item = fields.get(field_label(name))
    if item is None:
        return None
    if item['inline']:
        return item['inline']
    for line in item['lines']:
        if line.strip():
            return line.strip()
    return ''


def parse_matrix(lines):
    header, rows = None, []
    for raw in lines:
        line = raw.strip()
        if not line.startswith('|'):
            continue
        cells = [c.strip() for c in line.strip().strip('|').split('|')]
        if cells and all(SEPARATOR_CELL.match(c) for c in cells if c):
            continue
        if header is None:
            header = cells
            continue
        rows.append(cells)
    return header, rows


def _tables(text):
    """[(header, rows)] for every markdown table in text, in document order."""
    tables, header, rows = [], None, []
    for raw in text.splitlines():
        line = raw.strip()
        if not line.startswith('|'):
            if header is not None:
                tables.append((header, rows))
                header, rows = None, []
            continue
        cells = [c.strip() for c in line.strip().strip('|').split('|')]
        if all(SEPARATOR_CELL.match(c) for c in cells if c):
            continue
        if header is None:
            header = cells
        else:
            rows.append(cells)
    if header is not None:
        tables.append((header, rows))
    return tables


def audit_conclusion_errors(text, label='milestone audit'):
    """Findings that forbid reading a filled audit as a stage closeout.

    Only the *classification* is machine-checkable: a row whose verdict is
    BLOCKING or NEEDS_HUMAN_DECISION forbids "stage complete", and a NOT_RUN /
    BLOCKED row is not counted as passing. Whether the verdict is justified stays
    a review decision and is never claimed by this function.
    """
    errors, seen = [], 0
    for header, rows in _tables(text):
        labels = [field_label(cell) for cell in header]
        columns = [index for index, name in enumerate(labels)
                   if 'verdict' in name or '结论' in name]
        if not columns:
            continue
        column = columns[0]
        for index, cells in enumerate(rows, 1):
            if len(cells) <= column:
                errors.append(f'{label}: row {index} is shorter than its verdict column')
                continue
            verdict = _clean(cells[column]).upper()
            seen += 1
            if verdict not in AUDIT_VERDICTS:
                errors.append(f'{label}: row {index} has an unknown verdict '
                              f'{verdict or "(empty)"}; allowed: {", ".join(AUDIT_VERDICTS)}')
            elif verdict in CLOSEOUT_BLOCKING_VERDICTS:
                errors.append(f'{label}: row {index} is {verdict}; the stage must not be '
                              'claimed complete or passing')
            elif verdict in CLOSEOUT_UNVERIFIED_VERDICTS:
                errors.append(f'{label}: row {index} is {verdict}; it is not counted as passing')
    if not seen:
        errors.append(f'{label}: no readable verdict rows (expected a table with a '
                      'verdict / 结论 column)')
    return errors


def downstream_tree(root=ROOT):
    """True for a downstream project: docs/ exists without the upstream upgrade guide.

    A release package carries no docs/ at all; the upstream AgentOrch checkout
    carries docs/upgrade.md. Both are held to the strict manifest hash check.
    A downstream project necessarily carries docs/ (docs/PROJECT.md and
    docs/PLAN.md) but not the upstream repository documents; its core files may
    be customized, so drift is reported by check_drift instead of failed here.
    """
    root = Path(root)
    return (root / 'docs').is_dir() and not (root / 'docs' / 'upgrade.md').is_file()


def required_doc_errors(root=ROOT, upstream=None):
    """Missing or structurally incomplete documents.

    Skipped when the tree has no `docs/` directory: a release package never ships
    one. Project documents are always checked when `docs/` exists. Documents
    owned by the upstream AgentOrch repository are checked only when *upstream*
    is true; when it is None the tree is treated as upstream only if it carries
    `docs/upgrade.md`.
    """
    root = Path(root)
    if not (root / 'docs').is_dir():
        return []
    if upstream is None:
        upstream = (root / 'docs' / 'upgrade.md').is_file()
    docs = dict(PROJECT_DOCS)
    if upstream:
        docs.update(UPSTREAM_DOCS)
    errors = []
    for rel, sections in docs.items():
        path = root / rel
        if not path.is_file():
            errors.append(f'missing required document: {rel}')
            continue
        text = path.read_text(encoding='utf-8')
        headings = [match.group('text').lower() for match in HEADING_RE.finditer(text)]
        for section in sections:
            if not any(section.lower() in heading for heading in headings):
                errors.append(f'{rel} is missing required section: {section}')
    return errors


def extract_packet_samples(text):
    """Fenced ```task-packet blocks, used as structural test fixtures."""
    samples, lines, i = [], text.splitlines(), 0
    while i < len(lines):
        match = re.match(r'^```task-packet(?:\s+(?P<name>.*))?$', lines[i].strip())
        if not match:
            i += 1
            continue
        name = (match.group('name') or '').strip() or f'sample-{len(samples) + 1}'
        body, i = [], i + 1
        while i < len(lines) and lines[i].strip() != '```':
            body.append(lines[i])
            i += 1
        samples.append({'name': name, 'text': '\n'.join(body)})
        i += 1
    return samples


def validate_task_packet(text, canonical_fields, status_values, label='task packet', structured_fields=()):
    """Return structural errors for one filled task packet; empty list means valid."""
    if not canonical_fields:
        return [f'{label}: canonical evidence fields are unavailable in {CORE_DOC}']
    errors = []
    fields = parse_task_packet(text)

    for name in PACKET_REQUIRED_FIELDS:
        if field_label(name) not in fields:
            errors.append(f'{label}: missing required field: {name}')
        elif name not in PACKET_LIST_FIELDS and not field_value(fields, name):
            errors.append(f'{label}: required field is empty: {name}')

    for name in structured_fields:
        value = field_value(fields, name)
        if value is None:
            errors.append(f'{label}: missing structured field: {name}')
            continue
        if not value:
            errors.append(f'{label}: structured field is empty: {name}')
            continue
        if value.upper().startswith('N/A'):
            parts = re.split(r'[:：]', value, maxsplit=1)
            reason = parts[1].strip() if len(parts) > 1 else ''
            if not reason:
                errors.append(f'{label}: structured field {name} must give a reason after "N/A:"')

    acs = {}
    item = fields.get(field_label('acceptance criteria'))
    ac_source = ([item['inline']] if item and item['inline'] else []) + (item['lines'] if item else [])
    for line in ac_source:
        match = CHECKBOX_LINE.match(line)
        if not match:
            continue
        body = match.group('body').strip()
        head = AC_HEAD.match(body)
        if not head:
            errors.append(f'{label}: acceptance criterion without an AC-<n> id: {body or "(empty)"}')
            continue
        ac_id = head.group('id')
        if not head.group('text').strip():
            errors.append(f'{label}: acceptance criterion {ac_id} has no checkable statement')
        if ac_id in acs:
            errors.append(f'{label}: duplicate acceptance criterion id: {ac_id}')
        acs[ac_id] = match.group('mark').lower() == 'x'
    if not acs:
        errors.append(f'{label}: no acceptance criteria found (expected "- [ ] AC-1: ...")')

    head_sha = _clean(field_value(fields, 'evidence head sha') or '')
    if head_sha and not SHA_RE.match(head_sha.lower()):
        errors.append(f'{label}: evidence head SHA must be a full 40-hex commit SHA')

    matrix_lines = fields.get('evidence matrix', {}).get('lines', [])
    header, rows = parse_matrix(matrix_lines)
    item_rows = []
    if 'evidence matrix' in fields:
        if header is None:
            errors.append(f'{label}: evidence matrix table not found')
        else:
            if [_clean(c) for c in header] != list(canonical_fields):
                errors.append(f'{label}: evidence matrix columns must be exactly {list(canonical_fields)}')
            if not rows:
                errors.append(f'{label}: evidence matrix has no evidence items')
    for index, cells in enumerate(rows, 1):
        if len(cells) != len(canonical_fields):
            errors.append(f'{label}: evidence item #{index} has {len(cells)} fields, '
                          f'expected {len(canonical_fields)}')
            continue
        row = {name: _clean(cell) for name, cell in zip(canonical_fields, cells)}
        item_rows.append(row)
        status = row.get('status', '').upper()
        if status not in status_values:
            errors.append(f'{label}: evidence item #{index} has unknown status: '
                          f'{row.get("status") or "(empty)"}')
            continue
        ac_id = row.get('ac_id', '')
        if ac_id not in acs:
            errors.append(f'{label}: evidence item #{index} references unknown acceptance '
                          f'criterion: {ac_id or "(empty)"}')
        if status == 'NOT_APPLICABLE':
            notes = row.get('notes', '')
            if not notes or notes.lower() in ('(none)', 'none', 'n/a'):
                errors.append(f'{label}: evidence item #{index} is NOT_APPLICABLE without a reason in notes')
        else:
            if not row.get('actual'):
                errors.append(f'{label}: evidence item #{index} has empty actual for status {status}')
            if not row.get('commit_sha'):
                errors.append(f'{label}: evidence item #{index} has empty commit_sha for status {status}')
            elif head_sha and row['commit_sha'].lower() != head_sha.lower():
                errors.append(f'{label}: evidence item #{index} commit_sha does not match the '
                              f'declared evidence head SHA')
        if status == 'PASS' and NOT_RUN_MARKERS.search(f"{row.get('actual', '')} {row.get('notes', '')}"):
            errors.append(f'{label}: evidence item #{index} is PASS but reports not-run/blocked evidence')

    for ac_id, checked in acs.items():
        ac_rows = [r for r in item_rows if r.get('ac_id') == ac_id]
        if not ac_rows:
            errors.append(f'{label}: acceptance criterion {ac_id} has no evidence item')
            continue
        if not checked:
            continue
        bad = sorted({r.get('status', '').upper() for r in ac_rows
                      if r.get('status', '').upper() in NON_PASS_STATUSES})
        if bad:
            errors.append(f'{label}: acceptance criterion {ac_id} is marked as met but has '
                          f'{", ".join(bad)} evidence')
        if not any(r.get('status', '').upper() == 'PASS' for r in ac_rows):
            errors.append(f'{label}: acceptance criterion {ac_id} is marked as met without PASS evidence')
    errors.extend(resource_declaration_errors(text, label))
    return errors


def _drift_module():
    """Load scripts/check_drift.py by path (manifest rules live only there)."""
    path = Path(__file__).resolve().parent / 'check_drift.py'
    if not path.is_file():
        return None
    spec = importlib.util.spec_from_file_location('check_drift_for_validate_kit', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def check_release_manifest(root):
    """Validate MANIFEST.json when the tree carries one (a built package does).

    The repository root has no MANIFEST.json, so this returns no errors there and
    the pre-release workflow keeps working on a raw checkout. A present manifest
    must be well-formed and list exactly the REQUIRED files with legal
    categories. Hash/size checks are strict in a release package and in the
    upstream checkout; in a downstream project they are diagnostic only, because
    the project is allowed to customize core files. `check_drift.py` reports the
    drift, and the update engine classifies it as local modification / conflict.
    """
    manifest_path = Path(root) / MANIFEST_DOC
    if not manifest_path.is_file():
        return []
    drift = _drift_module()
    if drift is None:
        return [f'{MANIFEST_DOC} is present but scripts/check_drift.py is missing, '
                'so the manifest cannot be validated']
    try:
        manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as exc:
        return [f'{MANIFEST_DOC} is not valid JSON: {exc}']
    errors = [f'{MANIFEST_DOC}: {reason}'
              for reason in drift.manifest_structure_errors(manifest)]
    if not downstream_tree(root):
        errors += [f'{MANIFEST_DOC}: {reason}'
                   for reason in drift.verify_manifest_files(Path(root), manifest)]
    listed = [entry.get('path') for entry in manifest.get('files', [])
              if isinstance(entry, dict)] if isinstance(manifest, dict) else []
    for rel in sorted(set(REQUIRED) - set(listed)):
        errors.append(f'{MANIFEST_DOC} does not list required file: {rel}')
    for rel in sorted(set(listed) - set(REQUIRED)):
        errors.append(f'{MANIFEST_DOC} lists a file that is not in REQUIRED: {rel}')
    return errors


def template_structure_errors(root=ROOT):
    """Missing contract template, missing required heading, or a broken relative link.

    A missing file is reported here as well as by the REQUIRED loop: the two
    checks answer different questions ("is the artifact shipped?" vs "does it
    still ask the required questions?"), and a caller that only imports this
    function should not get a silent pass.
    """
    errors, root = [], Path(root)
    for rel, sections in TEMPLATE_SECTIONS.items():
        path = root / rel
        if not path.is_file():
            errors.append(f'missing required template: {rel}')
            continue
        text = path.read_text(encoding='utf-8')
        headings = [match.group('text').lower() for match in HEADING_RE.finditer(text)]
        for section in sections:
            if not any(section.lower() in heading for heading in headings):
                errors.append(f'{rel} is missing required section: {section}')
        for target in markdown_links(text):
            if target.startswith(EXTERNAL_LINK_PREFIXES):
                continue
            target = target.split('#', 1)[0]
            if target and not (path.parent / target).exists():
                errors.append(f'{rel}: broken relative link: {target}')
    return errors


def validate_kit(root=ROOT, project_ready=False):
    errors = []
    root = Path(root)

    for rel in REQUIRED:
        if not (root / rel).is_file():
            errors.append(f'missing required file: {rel}')

    # Since Format 1.4 the design-goal authority lives at docs/PLAN.md. A root
    # PLAN.md is the legacy (<=1.3) location: it must be moved, not kept, or the
    # project ends up with two competing design-goal authorities. init refuses
    # to seed docs/PLAN.md while it exists, so validation fails closed too.
    if (root / 'PLAN.md').is_file():
        errors.append("legacy root PLAN.md found: move it to docs/PLAN.md (`git mv PLAN.md docs/PLAN.md`) "
                      'so design goals keep a single authority — see README section 10')
    if (root / 'ORCHESTRATION-PROJECT.md').is_file():
        errors.append("legacy root ORCHESTRATION-PROJECT.md found: move it to docs/PROJECT.md "
                      '(`git mv ORCHESTRATION-PROJECT.md docs/PROJECT.md`) — see docs/upgrade.md')

    errors.extend(template_structure_errors(root))

    # Repository-only documents (scenario catalogue). Skipped inside a package,
    # which never carries docs/.
    errors.extend(required_doc_errors(root))

    # A built package also carries MANIFEST.json; validate it when present.
    errors.extend(check_release_manifest(root))

    # The two editions must expose the same public file set; github additionally
    # ships three templates. Comparing the *intersection* catches a file that
    # exists on only one side, while comparing each side against its expected set
    # catches strays.
    common_edition_files = ('master.md', 'execution.md')
    github_only_files = ('active-task-registry.md', 'issue-template.md', 'pr-template.md')
    edition_names = {}
    for edition in ('local', 'github'):
        directory = root / 'editions' / edition
        if not directory.is_dir():
            errors.append(f'missing edition directory: editions/{edition}')
            edition_names[edition] = set()
            continue
        actual = {p.name for p in directory.iterdir() if p.is_file()}
        edition_names[edition] = actual
        expected = set(common_edition_files) | (set(github_only_files) if edition == 'github' else set())
        for name in sorted(expected - actual):
            errors.append(f'editions/{edition} is missing file: {name}')
        for name in sorted(actual - expected):
            errors.append(f'editions/{edition} contains unexpected file: {name}')
    if edition_names.get('local') is not None and edition_names.get('github') is not None:
        shared = edition_names['local'] & edition_names['github']
        if shared != set(common_edition_files):
            errors.append('editions/local and editions/github disagree on the shared file set: '
                          f'intersection={sorted(shared)}')
    for name in github_only_files:
        if not (root / 'editions' / 'github' / name).is_file():
            errors.append(f'missing github-only template: editions/github/{name}')

    # The core orchestration document must really define the governance concepts it
    # is supposed to own: PR_OPEN (github-only state), merge gates G1-G6, runtime
    # capability contracts C1-C4 and the evidence matrix. This is a content check,
    # not a self-certification.
    core_path = root / CORE_DOC
    core_text = ''
    if core_path.is_file():
        core_text = core_path.read_text(encoding='utf-8')
        core_needles = ['PR_OPEN'] + [f'G{i}' for i in range(1, 7)] + [f'C{i}' for i in range(1, 5)]
        for needle in core_needles:
            if needle not in core_text:
                errors.append(f'{CORE_DOC} is missing required concept: {needle}')
    else:
        errors.append(f'missing core document: {CORE_DOC}')

    evidence_fields, status_values = canonical_evidence_fields(core_text)
    packet_fields = canonical_packet_fields(core_text)
    if not evidence_fields:
        errors.append(f'{CORE_DOC}: cannot read the evidence item field table '
                      f'(expected a table after "{EVIDENCE_FIELD_ANCHOR}")')
    if not status_values:
        errors.append(f'{CORE_DOC}: cannot read the allowed status values for evidence items')
    elif set(status_values) != set(STATUS_VALUES):
        errors.append(f'{CORE_DOC}: evidence status values {sorted(set(status_values))} '
                      f'do not match the contract {sorted(STATUS_VALUES)}')
    if not packet_fields:
        errors.append(f'{CORE_DOC}: cannot read the structured task packet field table '
                      f'(expected a table after "{PACKET_FIELD_ANCHOR}")')

    # Both edition templates must carry the same field names; the meaning is only
    # defined in the core document, so only the names are checked here.
    for rel in (TASK_PACKET_DOC, ISSUE_TEMPLATE_DOC):
        path = root / rel
        if not path.is_file():
            continue
        text = path.read_text(encoding='utf-8')
        for name in list(evidence_fields) + list(packet_fields):
            if f'`{name}`' not in text:
                errors.append(f'{rel} does not carry the field `{name}` declared in {CORE_DOC}')

    # The optional named-resource carrier fields must exist where they are
    # documented, and their value must be readable when a task fills them in.
    for rel, names in ((TASK_PACKET_DOC, (RESOURCE_FIELD,)),
                       (ISSUE_TEMPLATE_DOC, (LOCKED_RESOURCE_FIELD,))):
        path = root / rel
        if not path.is_file():
            continue
        text = path.read_text(encoding='utf-8')
        for name in names:
            if f'`{name}`' not in text:
                errors.append(f'{rel} does not carry the optional resource field `{name}`')

    samples = []
    packet_path = root / TASK_PACKET_DOC
    if packet_path.is_file():
        samples = extract_packet_samples(packet_path.read_text(encoding='utf-8'))
    if packet_path.is_file() and not samples:
        errors.append(f'{TASK_PACKET_DOC}: no ```task-packet samples found to validate')
    for sample in samples:
        errors.extend(validate_task_packet(
            sample['text'], evidence_fields, status_values,
            label=f'{TASK_PACKET_DOC} sample "{sample["name"]}"',
            structured_fields=packet_fields,
        ))

    # The edition selector is the routing key for the whole kit. It must be present
    # and explicit: silently defaulting would run a project on the wrong task carrier.
    config_path = root / 'docs/PROJECT.md'
    if config_path.is_file():
        config_text = config_path.read_text(encoding='utf-8')
        match = re.search(r'^\s*[-*]\s*`?edition`?\s*[:：]\s*`?([A-Za-z0-9_-]+)`?', config_text, re.M)
        if not match:
            errors.append('docs/PROJECT.md is missing the `edition` key '
                          "(expected `edition`: `local` or `github`)")
        else:
            edition_value = match.group(1)
            if edition_value not in ('local', 'github'):
                errors.append("docs/PROJECT.md `edition` must be 'local' or 'github', "
                              f"got {edition_value!r}")

    # Governance concepts are only meaningful in documentation. Scan document-class
    # files (*.md / *.txt) and skip code dirs and binaries so the check cannot be
    # satisfied by the validator's own source or by non-document assets.
    doc_suffixes = ('.md', '.txt')
    excluded_dirs = {'scripts', '.orchestration', '.git', '__pycache__'}
    doc_texts = []
    for p in root.rglob('*'):
        if not p.is_file():
            continue
        rel = p.relative_to(root)
        if rel.suffix.lower() not in doc_suffixes:
            continue
        if excluded_dirs & set(rel.parts):
            continue
        raw = p.read_text(encoding='utf-8', errors='replace')
        if '\x00' in raw:
            continue  # skip binary payloads
        doc_texts.append(raw)
    text_all = '\n'.join(doc_texts)
    for needle in ['PLANNED', 'DISPATCHED', 'MASTER_REVIEW', 'APPROVED', 'VERIFIED', 'NEEDS_HUMAN_DECISION']:
        if needle not in text_all:
            errors.append(f'missing expected governance concept: {needle}')
    # Relative Markdown links must resolve. Ignore anchors and external URLs.
    # The contract templates are already link-checked with template-scoped
    # messages by template_structure_errors(), so skip them here to avoid one
    # break being reported twice.
    for md in root.rglob('*.md'):
        if md.relative_to(root).as_posix() in TEMPLATE_SECTIONS:
            continue
        content = md.read_text(encoding='utf-8')
        for target in markdown_links(content):
            if target.startswith(('http://', 'https://', 'mailto:', '#')):
                continue
            target = target.split('#', 1)[0]
            if target and not (md.parent / target).exists():
                errors.append(f'{md.relative_to(root)}: broken relative link: {target}')
    if project_ready:
        cfg_path = root / 'docs' / 'PROJECT.md'
        if not cfg_path.is_file():
            errors.append('docs/PROJECT.md is missing: copy kit/templates/project.md and replace '
                          'every FILL_IN placeholder (edition, allowed paths, verification commands)')
        else:
            cfg = cfg_path.read_text(encoding='utf-8')
            missing = cfg.count('FILL_IN')
            if missing:
                errors.append(f'docs/PROJECT.md has {missing} FILL_IN placeholders')
    return errors, len(samples)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project-ready', action='store_true',
                        help='fail while docs/PROJECT.md still has FILL_IN placeholders')
    parser.add_argument('--task-packet', metavar='PATH',
                        help='validate one filled task packet instead of the whole kit')
    args = parser.parse_args(argv)

    if args.task_packet:
        core_text = read_text(ROOT / CORE_DOC) if (ROOT / CORE_DOC).is_file() else ''
        evidence_fields, status_values = canonical_evidence_fields(core_text)
        packet_fields = canonical_packet_fields(core_text)
        errors = validate_task_packet(
            read_text(args.task_packet), evidence_fields, status_values,
            label=args.task_packet, structured_fields=packet_fields,
        )
        if errors:
            print('TASK PACKET VALIDATION FAILED')
            print('\n'.join(f'- {e}' for e in errors))
            return 1
        print(f'Task packet validation passed: {args.task_packet} '
              f'({len(evidence_fields)} evidence fields, {len(packet_fields)} structured fields).')
        return 0

    errors, sample_count = validate_kit(ROOT, project_ready=args.project_ready)
    if errors:
        print('VALIDATION FAILED')
        print('\n'.join(f'- {e}' for e in errors))
        return 1
    if (Path(ROOT) / MANIFEST_DOC).is_file() and downstream_tree(ROOT):
        print('Manifest: downstream tree detected; local drift is diagnosed by '
              'scripts/check_drift.py and does not fail this check.')
    print(f'Kit validation passed ({len(REQUIRED)} required files checked; '
          f'editions local/github share 2 files, github adds 3 templates; '
          f'{len(PROJECT_DOCS)} project + {len(UPSTREAM_DOCS)} upstream document(s) defined; '
          f'{sample_count} task packet samples validated).')
    return 0


if __name__ == '__main__':
    sys.exit(main())
