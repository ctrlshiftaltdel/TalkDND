"""Report assembly for the Architecture Contract checker.

The report is the only thing downstream tasks (#50, #53) may read. It always
binds:

- `head_sha` (git HEAD of the checked tree, an explicit override, or `null` with
  `head_sha_source: "unavailable"` — never silently omitted);
- `contract_version` and the contract path actually used;
- one of `ERROR` / `WARNING` / `UNKNOWN` / `NOT_RUN` / `PASS` per finding, with
  per-status counts so `UNKNOWN` and `NOT_RUN` can never be read as passing;
- `examined_modules` / `uncovered_modules`, so a `PASS` carries the scope it
  actually examined instead of an unqualified claim;
- `invariants[]`: per-invariant coverage derived from the rules that ran, never
  from the declaration itself (a `STATIC_RULE` invariant whose rule was skipped,
  disabled or unparsable is not coverage, and `CONTRACT_TEST` / `MANUAL_REVIEW`
  stay `UNKNOWN` here);
- `blocking`, computed from `enforcement_mode` and the findings, separately from
  `status` (a `DIAGNOSTIC_ONLY` run can report `ERROR` findings while blocking
  nothing).
"""
from __future__ import annotations

from pathlib import Path

from .adapters import python_ast

REPORT_SCHEMA_VERSION = 1
STATUSES = ('ERROR', 'WARNING', 'UNKNOWN', 'NOT_RUN', 'PASS')
NON_PASS_STATUSES = ('ERROR', 'WARNING', 'UNKNOWN', 'NOT_RUN')


def _no_contract_finding(notes):
    return {
        'status': 'NOT_RUN',
        'rule_id': None,
        'kind': None,
        'severity': None,
        'module_id': None,
        'target_module': None,
        'modules': [],
        'file': None,
        'line': None,
        'message': notes[0] if notes else 'no architecture contract configured',
        'waiver_id': None,
    }


def inventory(project_root):
    """Read-only inventory used only when no contract is configured."""
    project_root = Path(project_root)
    python_files = [rel for rel, _ in python_ast.iter_files(project_root, '.py')]
    rust_files = [rel for rel, _ in python_ast.iter_files(project_root, '.rs')]
    roots = sorted({rel.split('/')[0] for rel in python_files + rust_files if '/' in rel})
    return {
        'python_files': len(python_files),
        'rust_files': len(rust_files),
        'source_roots': roots[:20],
    }


def build_report(project_root, loaded, executed, head_sha, head_sha_source,
                 adapters_enabled):
    """Assemble the JSON-serialisable report dict."""
    contract = loaded.get('contract')
    mode = loaded.get('mode')
    errors = list(loaded.get('errors') or [])
    notes = list(loaded.get('notes') or [])

    findings = []
    capabilities = {}
    invariants = []
    examined_modules, uncovered_modules = [], []
    if executed is not None:
        findings = list(executed['findings'])
        capabilities = executed['capabilities']
        # Scope evidence: a rule only means what it examined. `examined_modules`
        # empty / `uncovered_modules` non-empty is why a rule can be NOT_RUN, and
        # both are surfaced so a `PASS` never travels without its scope.
        invariants = list(executed.get('invariants') or [])
        examined_modules = list(executed.get('examined_modules') or [])
        uncovered_modules = list(executed.get('uncovered_modules') or [])
    elif contract is None and not errors:
        findings = [_no_contract_finding(notes)]

    for capability in capabilities.values():
        for note in capability.get('notes') or []:
            if note not in notes:
                notes.append(note)

    counts = {status: 0 for status in STATUSES}
    for finding in findings:
        counts[finding['status']] = counts.get(finding['status'], 0) + 1
    counts['waived'] = sum(1 for finding in findings if finding.get('waiver_id'))

    blocking = False
    if errors:
        # Unreadable always fails closed. A schema/semantic error blocks in
        # ENFORCE and in every mode this checker cannot read (a corrupted
        # `enforcement_mode` must not silently downgrade the gate).
        blocking = bool(loaded.get('unreadable')
                        or mode not in ('OFF', 'DIAGNOSTIC_ONLY', 'WARN'))
    elif contract is not None:
        blocking = mode == 'ENFORCE' and counts['ERROR'] > 0

    if counts['ERROR']:
        status = 'ERROR'
    elif errors:
        status = 'UNKNOWN'
    elif counts['WARNING']:
        status = 'WARNING'
    elif counts['UNKNOWN']:
        status = 'UNKNOWN'
    elif counts['NOT_RUN']:
        status = 'NOT_RUN'
    else:
        status = 'PASS'

    if contract is None and not errors:
        mode = 'OFF'  # the frozen design: no contract is equivalent to OFF
    diagnostic_only = contract is None or mode in ('OFF', 'DIAGNOSTIC_ONLY')
    contract_version = None
    if contract is not None:
        contract_version = contract.get('contract_version')
    elif isinstance(loaded.get('raw'), dict):
        raw_version = loaded['raw'].get('contract_version')
        if isinstance(raw_version, str):
            contract_version = raw_version

    report = {
        'report_schema_version': REPORT_SCHEMA_VERSION,
        'tool': 'architecture-check',
        'project': str(Path(project_root).resolve()),
        'head_sha': head_sha,
        'head_sha_source': head_sha_source,
        'contract_path': loaded.get('path'),
        'contract_version': contract_version,
        'enforcement_mode': mode,
        'diagnostic_only': diagnostic_only,
        'blocking': blocking,
        'status': status,
        'counts': counts,
        'contract_errors': errors,
        'findings': findings,
        'invariants': invariants,
        'examined_modules': examined_modules,
        'uncovered_modules': uncovered_modules,
        'capabilities': capabilities,
        'adapters': list(adapters_enabled),
        'notes': notes,
    }
    if contract is None and not errors:
        report['inventory'] = inventory(project_root)
    return report


def render_text(report):
    """Human-readable rendering; one line per finding plus a summary."""
    lines = []
    contract = report.get('contract_path') or '(none)'
    version = report.get('contract_version') or '(unknown)'
    mode = report.get('enforcement_mode') or '(unknown)'
    lines.append(f'Architecture check: mode={mode} contract={contract} version={version}')
    source = report.get('head_sha_source')
    lines.append(f'head SHA: {report.get("head_sha") or "(unavailable)"} ({source})')
    if report.get('contract_errors'):
        lines.append('contract errors:')
        for error in report['contract_errors']:
            lines.append(f'  - {error}')
    for finding in report.get('findings') or []:
        location = ''
        if finding.get('file'):
            location = f'{finding["file"]}:{finding.get("line") or 0}'
        target = f' -> {finding["target_module"]}' if finding.get('target_module') else ''
        rule = finding.get('rule_id') or '-'
        kind = finding.get('kind') or '-'
        module = finding.get('module_id') or '-'
        lines.append(f'[{finding["status"]}] {rule} ({kind}) {module}{target} '
                     f'{location}: {finding["message"]}')
    counts = report.get('counts') or {}
    summary = ', '.join(f'{status} {counts.get(status, 0)}' for status in STATUSES)
    lines.append(f'status: {report["status"]} (blocking: {report["blocking"]}; {summary}; '
                 f'waived: {counts.get("waived", 0)})')
    examined = report.get('examined_modules') or []
    uncovered = report.get('uncovered_modules') or []
    if examined or uncovered:
        lines.append('scope: examined modules: ' + (', '.join(examined) or '(none)')
                     + '; uncovered modules: ' + (', '.join(uncovered) or '(none)'))
    for invariant in report.get('invariants') or []:
        lines.append(f'[coverage {invariant["status"]}] {invariant["invariant_id"]} '
                     f'({invariant["checked_by"]}): {invariant["message"]}')
    for note in report.get('notes') or []:
        lines.append(f'note: {note}')
    return '\n'.join(lines)
