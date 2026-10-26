"""Load and validate an optional Architecture Contract (schema version 1.0).

The machine definition of every field, enum and pattern lives in the frozen
schema `docs/contracts/schemas/architecture-contract.schema.json` (Issue #47).
This module does not restate the contract in prose: it re-implements only the
subset of JSON Schema that the frozen document actually uses (required keys,
enums, patterns, `additionalProperties: false`, a few `allOf` cases) so the
checker stays pure standard library.

Validation splits in two layers, both reported as `contract_errors`:

- schema errors: missing required field, unknown field, illegal enum, pattern
  violation, unsupported `schema_version`;
- semantic errors: duplicate ids, references to undeclared modules / rule ids,
  duplicate path ownership, a path escaping the project, an ACTIVE waiver whose
  scope is the whole-repository wildcard, unknown rule params.

Loading follows `docs/architecture/architecture-contract.md` §2:

- explicit `--contract` wins, then `docs/PROJECT.md`'s
  `architecture_contract_path`, then `docs/architecture/contract.json`;
- a path that is declared but absent is a config error (fail closed), **not**
  a silent fall back to "no contract";
- nothing declared and no default file means "no contract": callers must run in
  diagnostic-only mode and never invent boundaries.
"""
from __future__ import annotations

import json
import re
from datetime import date, datetime
from pathlib import Path

DEFAULT_CONTRACT_PATH = 'docs/architecture/contract.json'
PROJECT_CONFIG_PATH = 'docs/PROJECT.md'
CONTRACT_KEY = 'architecture_contract_path'

SUPPORTED_SCHEMA_VERSIONS = ('1.0',)
ENFORCEMENT_MODES = ('OFF', 'DIAGNOSTIC_ONLY', 'WARN', 'ENFORCE')
NON_BLOCKING_MODES = ('OFF', 'DIAGNOSTIC_ONLY', 'WARN')
SEVERITIES = ('ERROR', 'WARNING')
RULE_KINDS = (
    'NO_CYCLE', 'FORBIDDEN_EDGE', 'LAYER_DIRECTION', 'PRIVATE_ACCESS',
    'DUPLICATE_BUSINESS_RULE', 'MODULE_FAN_OUT', 'MODULE_SIZE',
    'INTERFACE_GROWTH', 'CONTRACT_TEST', 'CUSTOM',
)
INTERFACE_KINDS = ('python_module', 'cli_command', 'http_api', 'event', 'file')
STABILITIES = ('stable', 'additive_only', 'experimental')
BOUNDARY_KINDS = ('layer', 'domain', 'application', 'adapter', 'infrastructure',
                  'security', 'generated')
INVARIANT_CHECKERS = ('STATIC_RULE', 'CONTRACT_TEST', 'MANUAL_REVIEW', 'UNKNOWN')
WAIVER_STATUSES = ('ACTIVE', 'EXPIRED', 'REVOKED')

# Params this checker understands for each rule kind. Anything else is reported
# as an unsupported param instead of being silently ignored (frozen doc §3.3).
RULE_PARAMS = {
    'NO_CYCLE': (),
    'FORBIDDEN_EDGE': (),
    'LAYER_DIRECTION': ('order', 'direction'),
    'PRIVATE_ACCESS': (),
    'DUPLICATE_BUSINESS_RULE': (),
    'MODULE_FAN_OUT': ('max',),
    'MODULE_SIZE': ('max_lines',),
    'INTERFACE_GROWTH': ('baseline',),
    'CONTRACT_TEST': (),
    'CUSTOM': (),
}

MODULE_ID_RE = re.compile(r'^[a-z][a-z0-9_.-]{1,63}$')
MODULE_REF_RE = re.compile(r'^(\*|[a-z][a-z0-9_.-]{1,63})$')
RULE_ID_RE = re.compile(r'^ARCH-[A-Z0-9-]{2,32}$')
BOUNDARY_ID_RE = re.compile(r'^BND-[A-Z0-9-]{2,32}$')
INVARIANT_ID_RE = re.compile(r'^INV-[A-Z0-9-]{2,32}$')
WAIVER_ID_RE = re.compile(r'^WVR-[A-Z0-9-]{2,24}$')
SEMVER_RE = re.compile(r'^[0-9]+\.[0-9]+\.[0-9]+$')
RELATIVE_PATH_RE = re.compile(r'^[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*/?$')
DATE_RE = re.compile(r'^\d{4}-\d{2}-\d{2}$')
DATETIME_RE = re.compile(r'^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(Z|[+-]\d{2}:\d{2})$')

TOP_LEVEL_KEYS = ('schema_version', 'contract_version', 'project_name',
                  'enforcement_mode', 'modules', 'dependencies', 'boundaries',
                  'invariants', 'rules', 'baseline')
MODULE_KEYS = ('module_id', 'paths', 'responsibilities', 'owns_state',
               'public_interfaces', 'private_paths')
INTERFACE_KEYS = ('name', 'kind', 'stability')
DEPENDENCY_KEYS = ('allowed', 'forbidden')
ALLOWED_EDGE_KEYS = ('from', 'to', 'reason')
FORBIDDEN_EDGE_KEYS = ('from', 'to', 'reason', 'rule_id')
BOUNDARY_KEYS = ('boundary_id', 'kind', 'members', 'notes')
INVARIANT_KEYS = ('invariant_id', 'statement', 'checked_by', 'severity',
                  'rule_ids', 'notes')
RULE_KEYS = ('rule_id', 'kind', 'severity', 'enabled', 'description', 'params',
             'waivers')
WAIVER_KEYS = ('waiver_id', 'reason', 'owner', 'expires_at', 'status', 'scope')
BASELINE_KEYS = ('frozen_at', 'frozen_by', 'migration_notes')

DECLARED_RE = re.compile(
    r'^[ \t]*[-*][ \t]*`?' + CONTRACT_KEY + r'`?[ \t]*[:：][ \t]*`?([^`\n]+?)`?[ \t]*$',
    re.M)


def _unknown_keys(obj, allowed, label, errors):
    for key in sorted(set(obj) - set(allowed)):
        errors.append(f'{label}: unknown field {key!r} (additionalProperties is false)')


def _is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def _check_text(value, label, errors, minimum=1, maximum=500):
    if not isinstance(value, str):
        errors.append(f'{label}: must be a string')
        return False
    if not minimum <= len(value) <= maximum:
        errors.append(f'{label}: length must be {minimum}..{maximum}, got {len(value)}')
        return False
    return True


def relative_path_ok(value):
    """True when *value* is a project-relative path with no escape segment."""
    if not isinstance(value, str) or not RELATIVE_PATH_RE.match(value):
        return False
    return not any(part in ('.', '..') for part in value.split('/') if part)


def _check_relative_path(value, label, errors):
    if not relative_path_ok(value):
        errors.append(f'{label}: not a relative path inside the project: {value!r} '
                      '(absolute paths, backslashes and `..` segments are illegal)')


def _check_date(value, label, errors):
    if not isinstance(value, str) or not DATE_RE.match(value):
        errors.append(f'{label}: must be a YYYY-MM-DD date, got {value!r}')
        return False
    try:
        date.fromisoformat(value)
    except ValueError:
        errors.append(f'{label}: not a real calendar date: {value!r}')
        return False
    return True


def validate_contract(data):
    """Return schema + semantic errors for one parsed contract; empty means valid."""
    if not isinstance(data, dict):
        return ['contract: top level must be a JSON object']
    errors = []
    _unknown_keys(data, TOP_LEVEL_KEYS, 'contract', errors)

    for key in ('schema_version', 'contract_version', 'enforcement_mode',
                'modules', 'dependencies', 'rules', 'baseline'):
        if key not in data:
            errors.append(f'contract: missing required field {key!r}')

    schema_version = data.get('schema_version')
    if 'schema_version' in data and schema_version not in SUPPORTED_SCHEMA_VERSIONS:
        errors.append(f'contract: unsupported schema_version {schema_version!r} '
                      f'(supported: {", ".join(SUPPORTED_SCHEMA_VERSIONS)}); '
                      'fail closed and migrate the contract instead of guessing')

    version = data.get('contract_version')
    if 'contract_version' in data and (
            not isinstance(version, str) or not SEMVER_RE.match(version)):
        errors.append(f'contract: contract_version must be MAJOR.MINOR.PATCH, got {version!r}')

    if 'project_name' in data:
        _check_text(data['project_name'], 'contract.project_name', errors, 1, 120)

    mode = data.get('enforcement_mode')
    if 'enforcement_mode' in data and mode not in ENFORCEMENT_MODES:
        errors.append('contract: enforcement_mode must be one of '
                      f'{", ".join(ENFORCEMENT_MODES)}; got {mode!r}')

    modules = data.get('modules')
    module_ids = []
    declared_paths = {}
    if 'modules' in data:
        if not isinstance(modules, list):
            errors.append('contract.modules: must be an array')
            modules = []
        for index, module in enumerate(modules):
            module_ids.extend(_validate_module(module, index, errors))
            if isinstance(module, dict):
                for path in module.get('paths') or []:
                    if not relative_path_ok(path):
                        continue
                    normalised = path.rstrip('/')
                    if normalised in declared_paths:
                        other = declared_paths[normalised]
                        if other != module.get('module_id'):
                            errors.append(f'contract.modules: path {normalised!r} is declared '
                                          f'by both {other!r} and {module.get("module_id")!r}')
                    else:
                        declared_paths[normalised] = module.get('module_id')

    duplicates = sorted({mid for mid in module_ids if module_ids.count(mid) > 1})
    for mid in duplicates:
        errors.append(f'contract.modules: duplicate module_id {mid!r}')

    known_modules = set(module_ids)
    rule_ids = _validate_rules(data.get('rules'), known_modules, errors)
    known_rules = set(rule_ids)

    dependencies = data.get('dependencies')
    if 'dependencies' in data:
        _validate_dependencies(dependencies, known_modules, known_rules, errors)

    if 'boundaries' in data:
        _validate_boundaries(data['boundaries'], known_modules, errors)
    if 'invariants' in data:
        _validate_invariants(data['invariants'], known_rules, errors)
    if 'baseline' in data:
        _validate_baseline(data['baseline'], errors)
    return errors


def _validate_module(module, index, errors):
    label = f'contract.modules[{index}]'
    if not isinstance(module, dict):
        errors.append(f'{label}: must be an object')
        return []
    _unknown_keys(module, MODULE_KEYS, label, errors)
    for key in ('module_id', 'paths', 'responsibilities', 'public_interfaces'):
        if key not in module:
            errors.append(f'{label}: missing required field {key!r}')
    module_id = module.get('module_id')
    if 'module_id' in module and (
            not isinstance(module_id, str) or not MODULE_ID_RE.match(module_id)):
        errors.append(f'{label}.module_id: must match {MODULE_ID_RE.pattern}, got {module_id!r}')
    paths = module.get('paths')
    if 'paths' in module:
        if not isinstance(paths, list) or not paths:
            errors.append(f'{label}.paths: must be a non-empty array')
        else:
            for path_index, path in enumerate(paths):
                _check_relative_path(path, f'{label}.paths[{path_index}]', errors)
    responsibilities = module.get('responsibilities')
    if 'responsibilities' in module:
        if not isinstance(responsibilities, list) or not responsibilities:
            errors.append(f'{label}.responsibilities: must be a non-empty array')
        else:
            for item_index, item in enumerate(responsibilities):
                _check_text(item, f'{label}.responsibilities[{item_index}]', errors, 4, 300)
    owns_state = module.get('owns_state')
    if 'owns_state' in module:
        if not isinstance(owns_state, list):
            errors.append(f'{label}.owns_state: must be an array')
        else:
            for item_index, item in enumerate(owns_state):
                _check_text(item, f'{label}.owns_state[{item_index}]', errors, 1, 200)
    interfaces = module.get('public_interfaces')
    if 'public_interfaces' in module:
        if not isinstance(interfaces, list):
            errors.append(f'{label}.public_interfaces: must be an array')
        else:
            for item_index, interface in enumerate(interfaces):
                _validate_interface(interface, f'{label}.public_interfaces[{item_index}]', errors)
    private_paths = module.get('private_paths')
    if 'private_paths' in module:
        if not isinstance(private_paths, list):
            errors.append(f'{label}.private_paths: must be an array')
        else:
            for item_index, path in enumerate(private_paths):
                _check_relative_path(path, f'{label}.private_paths[{item_index}]', errors)
    return [module_id] if isinstance(module_id, str) else []


def _validate_interface(interface, label, errors):
    if not isinstance(interface, dict):
        errors.append(f'{label}: must be an object')
        return
    _unknown_keys(interface, INTERFACE_KEYS, label, errors)
    for key in ('name', 'kind'):
        if key not in interface:
            errors.append(f'{label}: missing required field {key!r}')
    if 'name' in interface:
        _check_text(interface['name'], f'{label}.name', errors, 1, 200)
    if 'kind' in interface and interface['kind'] not in INTERFACE_KINDS:
        errors.append(f'{label}.kind: must be one of {", ".join(INTERFACE_KINDS)}; '
                      f'got {interface["kind"]!r}')
    if 'stability' in interface and interface['stability'] not in STABILITIES:
        errors.append(f'{label}.stability: must be one of {", ".join(STABILITIES)}; '
                      f'got {interface["stability"]!r}')


def _validate_dependencies(dependencies, known_modules, known_rules, errors):
    label = 'contract.dependencies'
    if not isinstance(dependencies, dict):
        errors.append(f'{label}: must be an object')
        return
    _unknown_keys(dependencies, DEPENDENCY_KEYS, label, errors)
    for key in DEPENDENCY_KEYS:
        if key not in dependencies:
            errors.append(f'{label}: missing required field {key!r}')
    allowed = dependencies.get('allowed')
    if 'allowed' in dependencies:
        if not isinstance(allowed, list):
            errors.append(f'{label}.allowed: must be an array')
            allowed = []
        for index, edge in enumerate(allowed):
            edge_label = f'{label}.allowed[{index}]'
            if not isinstance(edge, dict):
                errors.append(f'{edge_label}: must be an object')
                continue
            _unknown_keys(edge, ALLOWED_EDGE_KEYS, edge_label, errors)
            for key in ALLOWED_EDGE_KEYS:
                if key not in edge:
                    errors.append(f'{edge_label}: missing required field {key!r}')
            for end in ('from', 'to'):
                value = edge.get(end)
                if end in edge and (not isinstance(value, str) or not MODULE_ID_RE.match(value)):
                    errors.append(f'{edge_label}.{end}: must be a module_id, got {value!r}')
                elif isinstance(value, str) and value not in known_modules:
                    errors.append(f'{edge_label}.{end}: unknown module {value!r} '
                                  '(no module declares it)')
            if 'reason' in edge:
                _check_text(edge['reason'], f'{edge_label}.reason', errors, 8, 300)
    forbidden = dependencies.get('forbidden')
    if 'forbidden' in dependencies:
        if not isinstance(forbidden, list):
            errors.append(f'{label}.forbidden: must be an array')
            forbidden = []
        for index, edge in enumerate(forbidden):
            edge_label = f'{label}.forbidden[{index}]'
            if not isinstance(edge, dict):
                errors.append(f'{edge_label}: must be an object')
                continue
            _unknown_keys(edge, FORBIDDEN_EDGE_KEYS, edge_label, errors)
            for key in FORBIDDEN_EDGE_KEYS:
                if key not in edge:
                    errors.append(f'{edge_label}: missing required field {key!r}')
            for end in ('from', 'to'):
                value = edge.get(end)
                if end in edge and (not isinstance(value, str) or not MODULE_REF_RE.match(value)):
                    errors.append(f'{edge_label}.{end}: must be a module_id or "*", '
                                  f'got {value!r}')
                elif isinstance(value, str) and value != '*' and value not in known_modules:
                    errors.append(f'{edge_label}.{end}: unknown module {value!r} '
                                  '(no module declares it)')
            if 'reason' in edge:
                _check_text(edge['reason'], f'{edge_label}.reason', errors, 8, 300)
            rule_id = edge.get('rule_id')
            if 'rule_id' in edge:
                if not isinstance(rule_id, str) or not RULE_ID_RE.match(rule_id):
                    errors.append(f'{edge_label}.rule_id: must match {RULE_ID_RE.pattern}, '
                                  f'got {rule_id!r}')
                elif rule_id not in known_rules:
                    errors.append(f'{edge_label}: unknown rule_id {rule_id!r} '
                                  '(no rule declares it)')


def _validate_boundaries(boundaries, known_modules, errors):
    label = 'contract.boundaries'
    if not isinstance(boundaries, list):
        errors.append(f'{label}: must be an array')
        return
    for index, boundary in enumerate(boundaries):
        item = f'{label}[{index}]'
        if not isinstance(boundary, dict):
            errors.append(f'{item}: must be an object')
            continue
        _unknown_keys(boundary, BOUNDARY_KEYS, item, errors)
        for key in ('boundary_id', 'kind', 'members'):
            if key not in boundary:
                errors.append(f'{item}: missing required field {key!r}')
        boundary_id = boundary.get('boundary_id')
        if 'boundary_id' in boundary and (
                not isinstance(boundary_id, str) or not BOUNDARY_ID_RE.match(boundary_id)):
            errors.append(f'{item}.boundary_id: must match {BOUNDARY_ID_RE.pattern}, '
                          f'got {boundary_id!r}')
        if 'kind' in boundary and boundary['kind'] not in BOUNDARY_KINDS:
            errors.append(f'{item}.kind: must be one of {", ".join(BOUNDARY_KINDS)}; '
                          f'got {boundary["kind"]!r}')
        members = boundary.get('members')
        if 'members' in boundary:
            if not isinstance(members, list) or not members:
                errors.append(f'{item}.members: must be a non-empty array')
            else:
                for member_index, member in enumerate(members):
                    if not isinstance(member, str) or not MODULE_ID_RE.match(member):
                        errors.append(f'{item}.members[{member_index}]: must be a module_id, '
                                      f'got {member!r}')
                    elif member not in known_modules:
                        errors.append(f'{item}.members[{member_index}]: unknown module '
                                      f'{member!r} (no module declares it)')
        if 'notes' in boundary:
            _check_text(boundary['notes'], f'{item}.notes', errors, 0, 500)


def _validate_invariants(invariants, known_rules, errors):
    label = 'contract.invariants'
    if not isinstance(invariants, list):
        errors.append(f'{label}: must be an array')
        return
    for index, invariant in enumerate(invariants):
        item = f'{label}[{index}]'
        if not isinstance(invariant, dict):
            errors.append(f'{item}: must be an object')
            continue
        _unknown_keys(invariant, INVARIANT_KEYS, item, errors)
        for key in ('invariant_id', 'statement', 'checked_by', 'severity'):
            if key not in invariant:
                errors.append(f'{item}: missing required field {key!r}')
        invariant_id = invariant.get('invariant_id')
        if 'invariant_id' in invariant and (
                not isinstance(invariant_id, str) or not INVARIANT_ID_RE.match(invariant_id)):
            errors.append(f'{item}.invariant_id: must match {INVARIANT_ID_RE.pattern}, '
                          f'got {invariant_id!r}')
        if 'statement' in invariant:
            _check_text(invariant['statement'], f'{item}.statement', errors, 8, 500)
        if 'checked_by' in invariant and invariant['checked_by'] not in INVARIANT_CHECKERS:
            errors.append(f'{item}.checked_by: must be one of '
                          f'{", ".join(INVARIANT_CHECKERS)}; got {invariant["checked_by"]!r}')
        if 'severity' in invariant and invariant['severity'] not in SEVERITIES:
            errors.append(f'{item}.severity: must be one of {", ".join(SEVERITIES)}; '
                          f'got {invariant["severity"]!r}')
        rule_ids = invariant.get('rule_ids')
        if 'rule_ids' in invariant:
            if not isinstance(rule_ids, list):
                errors.append(f'{item}.rule_ids: must be an array')
            else:
                for rule_index, rule_id in enumerate(rule_ids):
                    if not isinstance(rule_id, str) or not RULE_ID_RE.match(rule_id):
                        errors.append(f'{item}.rule_ids[{rule_index}]: must match '
                                      f'{RULE_ID_RE.pattern}, got {rule_id!r}')
                    elif rule_id not in known_rules:
                        errors.append(f'{item}.rule_ids[{rule_index}]: unknown rule_id '
                                      f'{rule_id!r} (no rule declares it)')
        if 'notes' in invariant:
            _check_text(invariant['notes'], f'{item}.notes', errors, 0, 500)


def _validate_rules(rules, known_modules, errors):
    label = 'contract.rules'
    if not isinstance(rules, list):
        errors.append(f'{label}: must be an array')
        return []
    if not rules:
        errors.append(f'{label}: must declare at least one rule')
    rule_ids, waiver_ids = [], []
    for index, rule in enumerate(rules):
        item = f'{label}[{index}]'
        if not isinstance(rule, dict):
            errors.append(f'{item}: must be an object')
            continue
        _unknown_keys(rule, RULE_KEYS, item, errors)
        for key in ('rule_id', 'kind', 'severity', 'enabled', 'description'):
            if key not in rule:
                errors.append(f'{item}: missing required field {key!r}')
        rule_id = rule.get('rule_id')
        if 'rule_id' in rule:
            if not isinstance(rule_id, str) or not RULE_ID_RE.match(rule_id):
                errors.append(f'{item}.rule_id: must match {RULE_ID_RE.pattern}, '
                              f'got {rule_id!r}')
            else:
                rule_ids.append(rule_id)
        kind = rule.get('kind')
        if 'kind' in rule and kind not in RULE_KINDS:
            errors.append(f'{item}.kind: must be one of {", ".join(RULE_KINDS)}; got {kind!r}')
        if 'severity' in rule and rule['severity'] not in SEVERITIES:
            errors.append(f'{item}.severity: must be one of {", ".join(SEVERITIES)}; '
                          f'got {rule["severity"]!r}')
        if 'enabled' in rule and not isinstance(rule['enabled'], bool):
            errors.append(f'{item}.enabled: must be a boolean')
        if 'description' in rule:
            _check_text(rule['description'], f'{item}.description', errors, 8, 500)
        params = rule.get('params')
        if 'params' in rule:
            if not isinstance(params, dict):
                errors.append(f'{item}.params: must be an object')
            elif kind in RULE_PARAMS:
                allowed = RULE_PARAMS[kind]
                for key in sorted(set(params) - set(allowed)):
                    suffix = f'(allowed: {", ".join(allowed)})' if allowed else '(takes no params)'
                    errors.append(f'{item} ({rule_id or kind}): unsupported param {key!r} '
                                  f'for kind {kind} {suffix}')
                _validate_rule_params(params, kind, item, rule_id, known_modules, errors)
        waivers = rule.get('waivers')
        if 'waivers' in rule:
            if not isinstance(waivers, list):
                errors.append(f'{item}.waivers: must be an array')
            else:
                for waiver_index, waiver in enumerate(waivers):
                    waiver_ids.extend(_validate_waiver(
                        waiver, f'{item}.waivers[{waiver_index}]', errors))
    duplicates = sorted({rid for rid in rule_ids if rule_ids.count(rid) > 1})
    for rid in duplicates:
        errors.append(f'{label}: duplicate rule_id {rid!r}')
    duplicate_waivers = sorted({wid for wid in waiver_ids if waiver_ids.count(wid) > 1})
    for wid in duplicate_waivers:
        errors.append(f'{label}: duplicate waiver_id {wid!r}')
    return rule_ids


def _validate_rule_params(params, kind, item, rule_id, known_modules, errors):
    """Type / cross-reference checks for the params this checker implements."""
    prefix = f'{item} ({rule_id or kind})'
    if kind == 'LAYER_DIRECTION':
        if 'order' in params:
            order = params['order']
            if not isinstance(order, list) or len(order) < 2:
                errors.append(f'{prefix}.params.order: must list at least two module ids')
            else:
                for index, member in enumerate(order):
                    if not isinstance(member, str) or not MODULE_ID_RE.match(member):
                        errors.append(f'{prefix}.params.order[{index}]: must be a module_id, '
                                      f'got {member!r}')
                    elif known_modules is not None and member not in known_modules:
                        errors.append(f'{prefix}.params.order[{index}]: unknown module '
                                      f'{member!r} (no module declares it)')
                if len(set(order)) != len(order):
                    errors.append(f'{prefix}.params.order: contains duplicate module ids')
        if 'direction' in params and params['direction'] not in ('downward', 'upward'):
            errors.append(f'{prefix}.params.direction: must be "downward" or "upward", '
                          f'got {params["direction"]!r}')
    if kind == 'MODULE_FAN_OUT' and 'max' in params:
        if not _is_int(params['max']) or params['max'] < 0:
            errors.append(f'{prefix}.params.max: must be an integer >= 0')
    if kind == 'MODULE_SIZE' and 'max_lines' in params:
        if not _is_int(params['max_lines']) or params['max_lines'] < 1:
            errors.append(f'{prefix}.params.max_lines: must be an integer >= 1')
    if kind == 'INTERFACE_GROWTH' and 'baseline' in params:
        baseline = params['baseline']
        if not isinstance(baseline, dict):
            errors.append(f'{prefix}.params.baseline: must be an object of module_id -> count')
        else:
            for module_id, count in baseline.items():
                if not isinstance(module_id, str) or not MODULE_ID_RE.match(module_id):
                    errors.append(f'{prefix}.params.baseline: illegal module id {module_id!r}')
                if not _is_int(count) or count < 0:
                    errors.append(f'{prefix}.params.baseline[{module_id!r}]: must be an '
                                  'integer >= 0')


def _validate_waiver(waiver, label, errors):
    if not isinstance(waiver, dict):
        errors.append(f'{label}: must be an object')
        return []
    _unknown_keys(waiver, WAIVER_KEYS, label, errors)
    for key in WAIVER_KEYS:
        if key not in waiver:
            errors.append(f'{label}: missing required field {key!r}')
    waiver_id = waiver.get('waiver_id')
    if 'waiver_id' in waiver:
        if not isinstance(waiver_id, str) or not WAIVER_ID_RE.match(waiver_id):
            errors.append(f'{label}.waiver_id: must match {WAIVER_ID_RE.pattern}, '
                          f'got {waiver_id!r}')
    if 'reason' in waiver:
        _check_text(waiver['reason'], f'{label}.reason', errors, 8, 500)
    if 'owner' in waiver:
        _check_text(waiver['owner'], f'{label}.owner', errors, 1, 120)
    if 'expires_at' in waiver:
        _check_date(waiver['expires_at'], f'{label}.expires_at', errors)
    status = waiver.get('status')
    if 'status' in waiver and status not in WAIVER_STATUSES:
        errors.append(f'{label}.status: must be one of {", ".join(WAIVER_STATUSES)}; '
                      f'got {status!r}')
    scope = waiver.get('scope')
    if 'scope' in waiver:
        if not isinstance(scope, list) or not scope:
            errors.append(f'{label}.scope: must be a non-empty array')
        else:
            for index, item in enumerate(scope):
                _check_text(item, f'{label}.scope[{index}]', errors, 1, 200)
            if status == 'ACTIVE' and any(
                    isinstance(item, str) and item.strip() == '*' for item in scope):
                errors.append(f'{label}: ACTIVE waiver scope must not be the whole repository '
                              "wildcard '*' (name the module, path or violation instance)")
    return [waiver_id] if isinstance(waiver_id, str) else []


def _validate_baseline(baseline, errors):
    label = 'contract.baseline'
    if not isinstance(baseline, dict):
        errors.append(f'{label}: must be an object')
        return
    _unknown_keys(baseline, BASELINE_KEYS, label, errors)
    for key in ('frozen_at', 'frozen_by'):
        if key not in baseline:
            errors.append(f'{label}: missing required field {key!r}')
    frozen_at = baseline.get('frozen_at')
    if 'frozen_at' in baseline:
        if not isinstance(frozen_at, str) or not DATETIME_RE.match(frozen_at):
            errors.append(f'{label}.frozen_at: must be an ISO-8601 date-time, got {frozen_at!r}')
        else:
            try:
                datetime.fromisoformat(frozen_at.replace('Z', '+00:00'))
            except ValueError:
                errors.append(f'{label}.frozen_at: not a real date-time: {frozen_at!r}')
    if 'frozen_by' in baseline:
        _check_text(baseline['frozen_by'], f'{label}.frozen_by', errors, 1, 120)
    if 'migration_notes' in baseline and baseline['migration_notes'] is not None:
        _check_text(baseline['migration_notes'], f'{label}.migration_notes', errors, 0, 2000)


def declared_path(project_root):
    """The `architecture_contract_path` value from docs/PROJECT.md, or None."""
    config = Path(project_root) / PROJECT_CONFIG_PATH
    if not config.is_file():
        return None
    match = DECLARED_RE.search(config.read_text(encoding='utf-8'))
    if not match:
        return None
    return match.group(1).strip().strip('`').strip() or None


def _inside_project(project_root, raw):
    """(relative posix path, error). Rejects absolute and escaping paths."""
    candidate = Path(raw)
    if candidate.is_absolute():
        return None, f'contract path {raw!r} is absolute; it must stay inside the project'
    normalised = Path(*[part for part in candidate.parts if part not in ('', '.')])
    if any(part == '..' for part in normalised.parts):
        return None, f'contract path {raw!r} points outside the project (`..` is illegal)'
    resolved = (project_root / normalised).resolve()
    try:
        resolved.relative_to(project_root)
    except ValueError:
        return None, f'contract path {raw!r} points outside the project'
    return normalised.as_posix(), None


def load(project_root, explicit=None):
    """Load the contract, if any.

    Returns a dict:
      contract     parsed + schema-valid contract, or None
      path         project-relative path used (None when nothing was configured)
      declared     True when the path came from --contract or docs/PROJECT.md
      errors       contract_errors (schema, semantic, or unreadable)
      notes        human-readable notes
      unreadable   True when a declared contract could not be read (fail closed)
      mode         declared enforcement_mode when readable
    """
    project_root = Path(project_root).resolve()
    result = {'contract': None, 'path': None, 'declared': False, 'errors': [],
              'notes': [], 'unreadable': False, 'mode': None, 'raw': None}
    if explicit:
        result['declared'] = True
        relative, error = _inside_project(project_root, explicit)
    else:
        configured = declared_path(project_root)
        if configured is not None:
            result['declared'] = True
            relative, error = _inside_project(project_root, configured)
        else:
            relative, error = DEFAULT_CONTRACT_PATH, None
    if error:
        result['errors'].append(error)
        result['unreadable'] = True
        return result
    result['path'] = relative
    target = project_root / relative
    if not target.is_file():
        if result['declared']:
            result['errors'].append(
                f'contract file is missing at {relative}: the path is declared but the file '
                'does not exist (fail closed; do not fall back to "no contract")')
            result['unreadable'] = True
        else:
            result['path'] = None
            result['notes'].append(
                'no architecture contract configured (docs/architecture/contract.json absent); '
                'diagnostic-only inventory, no rule executed and nothing is blocked')
        return result
    try:
        data = json.loads(target.read_text(encoding='utf-8'))
    except (OSError, ValueError) as exc:
        result['errors'].append(f'contract at {relative} could not be parsed: {exc}')
        result['unreadable'] = True
        return result
    result['raw'] = data
    if isinstance(data, dict) and data.get('enforcement_mode') in ENFORCEMENT_MODES:
        result['mode'] = data['enforcement_mode']
    result['errors'].extend(validate_contract(data))
    if not result['errors']:
        result['contract'] = data
    return result
