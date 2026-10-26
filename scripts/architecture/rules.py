"""Rule execution for the Architecture Contract checker.

Implemented hard rules (may block under `ENFORCE`, never under
`OFF` / `DIAGNOSTIC_ONLY` / `WARN`):

- `NO_CYCLE`        module dependency cycles (Tarjan SCC over static edges);
- `FORBIDDEN_EDGE`  an edge listed in `dependencies.forbidden` (supports `*`);
- `LAYER_DIRECTION` reverse edges against an explicit `params.order`;
- `PRIVATE_ACCESS`  a cross-module import of another module's `private_paths`.

Soft metrics (`MODULE_FAN_OUT`, `MODULE_SIZE`, `INTERFACE_GROWTH`) are always
reported as `WARNING`, even when the contract declares `severity: ERROR`:
the frozen design forbids blocking on arbitrary thresholds.

Everything the checker cannot prove is reported as `UNKNOWN` / `NOT_RUN`:
unparsable files, dynamic imports, an adapter that is disabled or whose external
toolchain is missing, a rule kind with no implementation here, and a rule that
needs params the contract did not provide.
"""
from __future__ import annotations

from datetime import date

from . import contract as contract_module
from .adapters import python_ast, rust

GRAPH_KINDS = ('NO_CYCLE', 'FORBIDDEN_EDGE', 'LAYER_DIRECTION', 'PRIVATE_ACCESS')
SOFT_KINDS = ('MODULE_FAN_OUT', 'MODULE_SIZE', 'INTERFACE_GROWTH')
SOFT_NOTE = 'static metric: reported as WARNING, never blocking'
UNIMPLEMENTED_KINDS = ('DUPLICATE_BUSINESS_RULE', 'CONTRACT_TEST', 'CUSTOM')
SOURCE_SUFFIXES = ('.py', '.rs', '.ts', '.tsx', '.js', '.jsx', '.go', '.java', '.kt',
                   '.cs', '.rb', '.php', '.swift', '.c', '.cc', '.cpp', '.h', '.hpp')

STATUS_ORDER = {'ERROR': 0, 'WARNING': 1, 'UNKNOWN': 2, 'NOT_RUN': 3}


def module_prefixes(modules):
    """[(prefix, module_id)] longest first, for path -> module attribution."""
    prefixes = []
    for module in modules:
        module_id = module.get('module_id')
        if not isinstance(module_id, str):
            continue
        for path in module.get('paths') or []:
            if isinstance(path, str) and contract_module.relative_path_ok(path):
                prefixes.append((path.rstrip('/'), module_id))
    prefixes.sort(key=lambda item: len(item[0]), reverse=True)
    return prefixes


def module_of(prefixes, relative):
    for prefix, module_id in prefixes:
        if prefix and (relative == prefix or relative.startswith(prefix + '/')):
            return module_id
    return None


def _finding(status, rule, message, module_id=None, target_module=None, file=None,
             line=None, modules=None, waiver_id=None):
    return {
        'status': status,
        'rule_id': (rule or {}).get('rule_id'),
        'kind': (rule or {}).get('kind'),
        'severity': (rule or {}).get('severity'),
        'module_id': module_id,
        'target_module': target_module,
        'modules': list(modules) if modules else ([module_id] if module_id else []),
        'file': file,
        'line': line,
        'message': message,
        'waiver_id': waiver_id,
    }


def _files_by_module(project_root, prefixes, suffix):
    mapping = {}
    for relative, _ in python_ast.iter_files(project_root, suffix):
        module_id = module_of(prefixes, relative)
        if module_id:
            mapping.setdefault(module_id, []).append(relative)
    return mapping


def _all_module_files(project_root, prefixes):
    """module_id -> sorted relative paths for every source-like file under its paths."""
    mapping = {}
    for suffix in SOURCE_SUFFIXES:
        for relative, _ in python_ast.iter_files(project_root, suffix):
            module_id = module_of(prefixes, relative)
            if module_id and relative not in mapping.setdefault(module_id, []):
                mapping[module_id].append(relative)
    for files in mapping.values():
        files.sort()
    return mapping


def collect_edges(project_root, modules, adapters_enabled):
    """Static dependency edges plus capability / unknown records."""
    prefixes = module_prefixes(modules)
    python_files = _files_by_module(project_root, prefixes, '.py')
    rust_files = _files_by_module(project_root, prefixes, '.rs')
    roots = sorted({prefix.split('/')[0] for prefix, _ in prefixes if prefix})
    capabilities, edges, unknowns = {}, [], []

    if 'python' in adapters_enabled:
        records, python_unknowns, info = python_ast.scan(project_root, roots)
        for record in records:
            source_module = module_of(prefixes, record['source_file'])
            if source_module is None:
                continue  # file outside every declared module: out of contract scope
            target_module = module_of(prefixes, record['target_file'])
            if target_module is None:
                unknowns.append({
                    'file': record['source_file'], 'line': record['line'],
                    'message': f'import target {record["target_file"]!r} belongs to no '
                               'declared module; the edge is UNKNOWN',
                })
                continue
            if target_module == source_module:
                continue
            edges.append({
                'module_id': source_module, 'target_module': target_module,
                'file': record['source_file'], 'line': record['line'],
                'target_file': record['target_file'],
                'text': record['text'], 'conditional': record['conditional'],
                'adapter': 'python',
            })
        unknowns.extend(python_unknowns)
        capabilities['python'] = {
            'status': 'AVAILABLE',
            'files_scanned': info['files'],
            'unparsable': info['unparsable'],
            'notes': ['static AST scan; dynamic imports and unresolved targets are UNKNOWN'],
        }
        examined_python = set(python_files)
    else:
        capabilities['python'] = {
            'status': 'UNKNOWN', 'files_scanned': 0, 'unparsable': [],
            'notes': ['adapter disabled by --disable-adapter python; python edges are NOT_RUN'],
        }
        examined_python = set()

    if 'rust' in adapters_enabled:
        records, rust_unknowns, info = rust.scan(project_root)
        capabilities['rust'] = {
            'status': 'AVAILABLE' if info['available'] else 'UNKNOWN',
            'files_scanned': info['files'],
            'toolchain': info['toolchain'],
            'notes': [info['reason']] if info.get('reason') else
                     ['lexical `use` scan only; braced/glob imports are UNKNOWN'],
        }
        if info['available']:
            for record in records:
                source_module = module_of(prefixes, record['source_file'])
                target_module = module_of(prefixes, record['target_file'])
                if source_module is None or target_module is None:
                    continue
                if source_module == target_module:
                    continue
                edges.append({
                    'module_id': source_module, 'target_module': target_module,
                    'file': record['source_file'], 'line': record['line'],
                    'target_file': record['target_file'],
                    'text': record['text'], 'conditional': False, 'adapter': 'rust',
                })
            unknowns.extend(rust_unknowns)
            examined_rust = set(rust_files)
        else:
            examined_rust = set()
    else:
        capabilities['rust'] = {
            'status': 'UNKNOWN', 'files_scanned': 0,
            'toolchain': rust.toolchain(),
            'notes': ['adapter disabled by --disable-adapter rust; Rust rules are NOT_RUN '
                      'and the capability is UNKNOWN'],
        }
        examined_rust = set()

    uncovered = []
    for module in modules:
        module_id = module.get('module_id')
        if not isinstance(module_id, str):
            continue
        has_python = module_id in python_files
        has_rust = module_id in rust_files
        if (has_python or has_rust) and module_id not in examined_python | examined_rust:
            uncovered.append(module_id)
    return {
        'edges': edges,
        'unknowns': unknowns,
        'capabilities': capabilities,
        'uncovered_modules': sorted(uncovered),
        'examined_modules': sorted(examined_python | examined_rust),
    }


# --- hard rules -------------------------------------------------------------

def _cycle_components(edges):
    adjacency = {}
    for edge in edges:
        adjacency.setdefault(edge['module_id'], set()).add(edge['target_module'])
    nodes = sorted(set(adjacency) | {t for targets in adjacency.values() for t in targets})
    index, low, stack, on_stack, components = {}, {}, [], set(), []
    counter = [0]

    def strong(node):
        index[node] = low[node] = counter[0]
        counter[0] += 1
        stack.append(node)
        on_stack.add(node)
        for target in sorted(adjacency.get(node, ())):
            if target not in index:
                strong(target)
                low[node] = min(low[node], low[target])
            elif target in on_stack:
                low[node] = min(low[node], index[target])
        if low[node] == index[node]:
            component = []
            while True:
                member = stack.pop()
                on_stack.discard(member)
                component.append(member)
                if member == node:
                    break
            if len(component) > 1:
                components.append(sorted(component))

    for node in nodes:
        if node not in index:
            strong(node)
    return sorted(components)


def _cycle_path(component, edges):
    allowed = set(component)
    adjacency = {}
    for edge in edges:
        if edge['module_id'] in allowed and edge['target_module'] in allowed:
            adjacency.setdefault(edge['module_id'], set()).add(edge['target_module'])
    start = component[0]
    stack = [(start, [start])]
    while stack:
        node, path = stack.pop()
        for target in sorted(adjacency.get(node, ())):
            if target == start:
                return path + [start]
            if target in path:
                continue
            stack.append((target, path + [target]))
    return component + [component[0]]


def _cycle_findings(rule, edges):
    findings = []
    for component in _cycle_components(edges):
        path = _cycle_path(component, edges)
        inside = [e for e in edges
                  if e['module_id'] in component and e['target_module'] in component]
        inside.sort(key=lambda e: (e['file'], e['line'] or 0))
        first = inside[0] if inside else {}
        findings.append(_finding(
            'ERROR', rule,
            'dependency cycle between modules: ' + ' -> '.join(path),
            module_id=path[0], target_module=path[1], modules=component,
            file=first.get('file'), line=first.get('line')))
    return findings


def _forbidden_findings(rule, forbidden, edges):
    findings, seen = [], set()
    for entry in forbidden:
        if entry.get('rule_id') != rule.get('rule_id'):
            continue
        for edge in edges:
            if not _edge_matches(entry, edge['module_id'], edge['target_module']):
                continue
            key = (edge['module_id'], edge['target_module'], edge['file'], edge['line'])
            if key in seen:
                continue
            seen.add(key)
            findings.append(_finding(
                'ERROR', rule,
                f'forbidden dependency edge: {edge["module_id"]} -> '
                f'{edge["target_module"]} ({entry.get("reason", "")})',
                module_id=edge['module_id'], target_module=edge['target_module'],
                file=edge['file'], line=edge['line']))
    return findings


def _edge_matches(entry, source, target):
    def matches(pattern, value):
        return pattern == '*' or pattern == value
    return matches(entry.get('from'), source) and matches(entry.get('to'), target)


def _layer_findings(rule, edges):
    params = rule.get('params') or {}
    order = params.get('order')
    if not isinstance(order, list) or len(order) < 2:
        return [_finding('UNKNOWN', rule,
                         'LAYER_DIRECTION requires params.order with at least two module ids; '
                         'rule not evaluated')]
    direction = params.get('direction', 'downward')
    position = {module_id: index for index, module_id in enumerate(order)}
    findings = []
    for edge in edges:
        source, target = edge['module_id'], edge['target_module']
        if source not in position or target not in position:
            continue
        backwards = position[target] < position[source] if direction == 'downward' \
            else position[target] > position[source]
        if not backwards:
            continue
        findings.append(_finding(
            'ERROR', rule,
            f'reverse layer dependency: {source} -> {target} (layer order: '
            + ' -> '.join(order) + ')',
            module_id=source, target_module=target, file=edge['file'], line=edge['line']))
    return findings


def _private_findings(rule, edges, modules_by_id):
    findings = []
    for edge in edges:
        module = modules_by_id.get(edge['target_module'])
        if not module:
            continue
        for private in module.get('private_paths') or []:
            if not isinstance(private, str):
                continue
            prefix = private.rstrip('/')
            target_file = edge.get('target_file') or ''
            if target_file == prefix or target_file.startswith(prefix + '/'):
                findings.append(_finding(
                    'ERROR', rule,
                    f'cross-module private access: {edge["module_id"]} imports '
                    f'{target_file} which is inside {edge["target_module"]} private path '
                    f'{private!r}',
                    module_id=edge['module_id'], target_module=edge['target_module'],
                    file=edge['file'], line=edge['line']))
                break
    return findings


def _soft_fan_out(rule, edges):
    params = rule.get('params') or {}
    maximum = params.get('max')
    if not isinstance(maximum, int) or isinstance(maximum, bool):
        return [_finding('UNKNOWN', rule,
                         'MODULE_FAN_OUT requires params.max (integer); rule not evaluated')]
    by_source = {}
    for edge in edges:
        by_source.setdefault(edge['module_id'], set()).add(edge['target_module'])
    findings = []
    for module_id, targets in sorted(by_source.items()):
        if len(targets) <= maximum:
            continue
        first = next(e for e in edges if e['module_id'] == module_id)
        findings.append(_finding(
            'WARNING', rule,
            f'module {module_id} depends on {len(targets)} modules (budget {maximum}); '
            + SOFT_NOTE,
            module_id=module_id, file=first['file'], line=first['line']))
    return findings


def _soft_size(rule, files_by_module, project_root):
    params = rule.get('params') or {}
    maximum = params.get('max_lines')
    if not isinstance(maximum, int) or isinstance(maximum, bool) or maximum < 1:
        return [_finding('UNKNOWN', rule,
                         'MODULE_SIZE requires params.max_lines (integer >= 1); '
                         'rule not evaluated')]
    findings = []
    for module_id, files in sorted(files_by_module.items()):
        for relative in files:
            path = project_root / relative
            try:
                lines = len(path.read_text(encoding='utf-8', errors='replace').splitlines())
            except OSError as exc:
                findings.append(_finding('UNKNOWN', rule,
                                         f'{relative} could not be read ({exc}); size unknown',
                                         module_id=module_id, file=relative))
                continue
            if lines > maximum:
                findings.append(_finding(
                    'WARNING', rule,
                    f'module {module_id}: {relative} has {lines} lines (budget {maximum}); '
                    + SOFT_NOTE, module_id=module_id, file=relative, line=1))
    return findings


def _soft_growth(rule, modules):
    params = rule.get('params') or {}
    baseline = params.get('baseline')
    if not isinstance(baseline, dict) or not baseline:
        return [_finding('UNKNOWN', rule,
                         'INTERFACE_GROWTH requires params.baseline (module -> count); '
                         'no recorded baseline, rule not evaluated')]
    findings = []
    for module in modules:
        module_id = module.get('module_id')
        if module_id not in baseline:
            continue
        actual = len(module.get('public_interfaces') or [])
        recorded = baseline[module_id]
        if not isinstance(recorded, int) or isinstance(recorded, bool):
            continue
        if actual > recorded:
            findings.append(_finding(
                'WARNING', rule,
                f'module {module_id} exposes {actual} public interfaces, baseline {recorded}; '
                + SOFT_NOTE, module_id=module_id))
    return findings


# --- waivers ----------------------------------------------------------------

def _scope_matches(scope, finding):
    for entry in scope:
        if not isinstance(entry, str):
            continue
        entry = entry.strip()
        if not entry or entry == '*':
            continue
        if entry == finding.get('rule_id') or entry == finding.get('module_id') \
                or entry == finding.get('target_module'):
            return True
        target = finding.get('file')
        if target and (target == entry or target.startswith(entry.rstrip('/') + '/')):
            return True
    return False


def apply_waivers(finding, rule, today=None):
    """Downgrade a violation covered by an ACTIVE, unexpired waiver."""
    if finding.get('status') != 'ERROR':
        return
    today = today or date.today()
    for waiver in rule.get('waivers') or []:
        if waiver.get('status') != 'ACTIVE':
            continue
        try:
            expires = date.fromisoformat(str(waiver.get('expires_at')))
        except ValueError:
            continue
        if expires < today:
            if _scope_matches(waiver.get('scope') or [], finding):
                finding['message'] += (f' [waiver {waiver.get("waiver_id")} expired on '
                                       f'{expires.isoformat()}; blocking restored]')
            continue
        if _scope_matches(waiver.get('scope') or [], finding):
            finding['status'] = 'WARNING'
            finding['waiver_id'] = waiver.get('waiver_id')
            finding['message'] += (f' [waived by {waiver.get("waiver_id")}: '
                                   f'{waiver.get("reason", "")}]')
            return


# --- invariant coverage -----------------------------------------------------

def _rule_statuses(findings):
    """rule_id -> worst reported status for that rule."""
    statuses = {}
    for finding in findings:
        rule_id = finding.get('rule_id')
        status = finding.get('status')
        if not rule_id or status is None:
            continue
        current = statuses.get(rule_id)
        if current is None or STATUS_ORDER.get(status, 9) < STATUS_ORDER.get(current, 9):
            statuses[rule_id] = status
    return statuses


def invariant_coverage(contract_data, findings):
    """Per-invariant coverage derived from the rules that actually ran.

    A declared invariant is not evidence by itself. For `checked_by:
    STATIC_RULE` the reported status is the worst status of the rules it names,
    so a missing, disabled, skipped or unparsable rule can never be read as
    coverage. `CONTRACT_TEST` and `MANUAL_REVIEW` are reported as `UNKNOWN`:
    this checker runs neither the project test suite nor a human review, and
    claiming otherwise would turn a declaration into a pass.

    This is additive report data. It does not create findings, does not change
    `blocking`, and does not alter the four report statuses of the frozen
    contract (docs/architecture/architecture-contract.md §4.2).
    """
    statuses = _rule_statuses(findings)
    # An UNKNOWN that belongs to no rule (dynamic import, unparsable file,
    # import target outside the declared modules) means at least one file was
    # never proven. A rule that "found nothing" over an incomplete scan is not
    # coverage, so a static-rule invariant degrades to UNKNOWN instead of PASS.
    unattributed = [finding for finding in findings
                    if finding.get('rule_id') is None and finding.get('status') != 'PASS']
    entries = []
    for invariant in contract_data.get('invariants') or []:
        if not isinstance(invariant, dict):
            continue
        invariant_id = invariant.get('invariant_id')
        checked_by = invariant.get('checked_by')
        rule_ids = [rule_id for rule_id in invariant.get('rule_ids') or []
                    if isinstance(rule_id, str)]
        entry = {'invariant_id': invariant_id, 'checked_by': checked_by,
                 'rule_ids': rule_ids, 'status': 'UNKNOWN', 'message': ''}
        if checked_by == 'STATIC_RULE':
            if not rule_ids:
                entry['message'] = ('checked_by=STATIC_RULE with an empty rule_ids: no rule can '
                                    'prove it, so it is not covered')
            else:
                observed = {rule_id: statuses[rule_id] for rule_id in rule_ids
                            if rule_id in statuses}
                missing = [rule_id for rule_id in rule_ids if rule_id not in statuses]
                if missing or not observed:
                    entry['message'] = ('no executed result for rule(s) ' + ', '.join(missing)
                                        + ': the invariant is not covered')
                else:
                    worst = None
                    for status in observed.values():
                        if worst is None or STATUS_ORDER.get(status, 9) < STATUS_ORDER.get(worst, 9):
                            worst = status
                    detail = ', '.join(f'{rule_id}={observed[rule_id]}'
                                       for rule_id in sorted(observed))
                    if unattributed:
                        entry['status'] = 'UNKNOWN'
                        entry['message'] = (
                            f'worst status of {detail}, but {len(unattributed)} finding(s) '
                            'belong to no rule (dynamic import, unparsable file or target '
                            'outside the declared modules); the scan is incomplete, so this is '
                            'not coverage')
                    else:
                        entry['status'] = worst
                        entry['message'] = f'worst status of {detail}'
        elif checked_by == 'CONTRACT_TEST':
            entry['message'] = ('checked_by=CONTRACT_TEST: this checker does not run the project '
                                'test suite, so the shared contract test result has to be attached '
                                'as its own evidence; not covered here')
        elif checked_by == 'MANUAL_REVIEW':
            entry['message'] = ('checked_by=MANUAL_REVIEW: a reviewer must record the outcome; '
                                'the checker cannot cover it')
        else:
            entry['message'] = (f'checked_by={checked_by!r}: no automatic check is declared, so '
                                'the invariant is not covered (never read as PASS)')
        entries.append(entry)
    entries.sort(key=lambda item: item['invariant_id'] or '')
    return entries


# --- entry point ------------------------------------------------------------

def run(contract_data, project_root, adapters_enabled=('python', 'rust')):
    """Execute every declared rule and return findings + capabilities."""
    modules = [m for m in contract_data.get('modules') or [] if isinstance(m, dict)]
    modules_by_id = {m.get('module_id'): m for m in modules if isinstance(m.get('module_id'), str)}
    forbidden = (contract_data.get('dependencies') or {}).get('forbidden') or []

    if contract_data.get('enforcement_mode') == 'OFF':
        # OFF means disabled: report the declared rules as NOT_RUN instead of
        # pretending they passed or scanning anyway.
        findings = [_finding('NOT_RUN', rule, 'enforcement_mode=OFF: rule not executed')
                    for rule in contract_data.get('rules') or [] if isinstance(rule, dict)]
        return {
            'findings': findings,
            'edges': [],
            'capabilities': {
                name: {'status': 'UNKNOWN', 'files_scanned': 0,
                       'notes': [f'not executed: enforcement_mode=OFF']}
                for name in ('python', 'rust')
            },
            'invariants': invariant_coverage(contract_data, findings),
            'uncovered_modules': [],
            'examined_modules': [],
        }
    collected = collect_edges(project_root, modules, adapters_enabled)
    edges = collected['edges']
    uncovered = collected['uncovered_modules']
    files_by_module = _all_module_files(project_root, module_prefixes(modules))
    findings = []

    for unknown in collected['unknowns']:
        findings.append(_finding(
            'UNKNOWN', None, unknown['message'], file=unknown['file'],
            line=unknown['line']))

    for rule in contract_data.get('rules') or []:
        if not isinstance(rule, dict):
            continue
        kind = rule.get('kind')
        if not rule.get('enabled'):
            findings.append(_finding('NOT_RUN', rule,
                                     'rule is disabled in the contract (enabled: false); '
                                     'not executed'))
            continue
        if kind in GRAPH_KINDS and uncovered:
            findings.append(_finding(
                'NOT_RUN', rule,
                'no available adapter examined modules: ' + ', '.join(uncovered)
                + ' (missing Python parser or Rust toolchain); rule not run',
                modules=uncovered))
        if kind in GRAPH_KINDS and modules and not collected['examined_modules']:
            findings.append(_finding(
                'NOT_RUN', rule,
                'no source file matched the declared module paths, so there is nothing to '
                'prove; rule not run (this is not a pass)',
                modules=[m.get('module_id') for m in modules
                         if isinstance(m.get('module_id'), str)]))
        if kind == 'NO_CYCLE':
            findings.extend(_cycle_findings(rule, edges))
        elif kind == 'FORBIDDEN_EDGE':
            findings.extend(_forbidden_findings(rule, forbidden, edges))
        elif kind == 'LAYER_DIRECTION':
            findings.extend(_layer_findings(rule, edges))
        elif kind == 'PRIVATE_ACCESS':
            findings.extend(_private_findings(rule, edges, modules_by_id))
        elif kind == 'MODULE_FAN_OUT':
            findings.extend(_soft_fan_out(rule, edges))
        elif kind == 'MODULE_SIZE':
            findings.extend(_soft_size(rule, files_by_module, project_root))
        elif kind == 'INTERFACE_GROWTH':
            findings.extend(_soft_growth(rule, modules))
        elif kind in UNIMPLEMENTED_KINDS:
            findings.append(_finding(
                'UNKNOWN', rule,
                f'rule kind {kind} has no implementation in this checker; it requires a '
                'contract test or manual review and is not counted as passing'))
        else:
            findings.append(_finding(
                'UNKNOWN', rule, f'unknown rule kind {kind!r}; not evaluated'))
        if not any(f['rule_id'] == rule.get('rule_id') for f in findings):
            # Every declared rule reports a status: a rule that ran and found
            # nothing is PASS, never an omitted line that reads as success.
            findings.append(_finding('PASS', rule, 'no violation found for this rule'))

    for finding in findings:
        rule = next((r for r in contract_data.get('rules') or []
                     if isinstance(r, dict) and r.get('rule_id') == finding['rule_id']), None)
        if rule is not None:
            apply_waivers(finding, rule)

    findings.sort(key=lambda f: (STATUS_ORDER.get(f['status'], 9),
                                 f['rule_id'] or '', f['file'] or '',
                                 f['line'] or 0, f['message']))
    return {
        'findings': findings,
        'edges': edges,
        'capabilities': collected['capabilities'],
        'invariants': invariant_coverage(contract_data, findings),
        'uncovered_modules': uncovered,
        'examined_modules': collected['examined_modules'],
    }
