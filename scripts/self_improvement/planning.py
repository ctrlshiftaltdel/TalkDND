"""Self-improvement trigger routing and improvement planning (#50).

This module answers two questions and nothing else:

``route_request``
    Does a user message start the self-improvement workflow, ask for an
    explanation, ask for the *update engine* (a different feature) or none of
    those? The wording table is a routing rule set, not a language model: it is
    deliberately boring so a false positive is visible in the tests.

``plan_tasks``
    Given lesson records that already exist (``scripts/self_improvement/lessons``)
    and an explicit, caller-written task specification, produce an improvement
    proposal. A proposal is ``READY`` only when the lesson is eligible, the task
    packet is complete, dependencies are verified and no high-risk surface is
    touched. Everything else is ``NO_TASK`` / ``BLOCKED`` /
    ``NEEDS_HUMAN_DECISION``; nothing is invented from source text.

What the module does **not** do: adopt a lesson, invent a root cause or an
acceptance criterion, create an Issue / task, run a command, or approve a
high-risk change. Field meanings (``negative_cases`` ... ) and the G1–G6 gates
live only in ``kit/orchestration.md`` 4.1 and are referenced, never restated.

Standard library only; no network, no subprocess, no carrier.
"""
from __future__ import annotations

import re
import unicodedata

from . import lessons

# --------------------------------------------------------------------------- #
# Routing
# --------------------------------------------------------------------------- #

ROUTE_SELF_IMPROVEMENT = 'SELF_IMPROVEMENT'
ROUTE_UPDATE_AGENTORCH = 'UPDATE_AGENTORCH'
ROUTE_EXPLAIN_ONLY = 'EXPLAIN_ONLY'
ROUTE_UNRELATED = 'UNRELATED'

ROUTES = (ROUTE_SELF_IMPROVEMENT, ROUTE_UPDATE_AGENTORCH, ROUTE_EXPLAIN_ONLY,
          ROUTE_UNRELATED)

# Longest phrase first: "进行自我完善" contains "自我完善".
SELF_IMPROVEMENT_PHRASES = (
    ('进行自我完善', True),
    ('根据其他 agentorch 项目的经验改进本项目', True),
    ('总结近期开发教训并推进改进', True),
    ('自我完善', False),
)

UPDATE_VERBS = ('更新', '升级', 'update', 'upgrade')
AGENTORCH_REFERENTS = ('agentorch', 'agent-orch', 'agent orchestration',
                       'orchestration starter kit', '规则包', '套件本身')
# "更新 AgentOrch 的文档" / "更新这个 Issue" name a different object: ask once.
UPDATE_EXCLUDED_OBJECTS = ('文档', '依赖', 'issue', '版本号', '说明', '注释', '报告',
                           'readme', 'documentation')

EXECUTION_MARKERS = ('进行', '开始', '现在', '立即', '马上', '执行', '启动', '推进',
                     '去做', '跑一遍', '走一遍', '落地')
# A question ("怎么进行") is still an explanation unless the user is clearly
# asking for it *now*; minimising false positives is the point of the table.
STRONG_EXECUTION_MARKERS = ('现在', '立即', '马上', '执行', '启动', '去做', '跑一遍',
                            '走一遍', '落地')
QUESTION_MARKERS = ('什么', '怎么', '如何', '为什么', '原理', '机制', '吗', '多少',
                    '?', '？', '介绍', '解释', '说明一下')

VERSION_MARKERS = ('版本', 'version')

_WHITESPACE = re.compile(r'\s+')


def normalize_request(text):
    """NFKC + casefold + whitespace collapse. Punctuation is kept on purpose."""
    value = unicodedata.normalize('NFKC', text or '')
    return _WHITESPACE.sub(' ', value.casefold()).strip()


class RoutingDecision:
    """Outcome of one message. ``starts_workflow`` is the only green light."""

    def __init__(self, route, *, matched=None, reason='', execution_intent=False,
                 needs_clarification=False, owner=None):
        if route not in ROUTES:
            raise ValueError(f'unknown route {route!r}')
        self.route = route
        self.matched = matched
        self.reason = reason
        self.execution_intent = execution_intent
        self.needs_clarification = needs_clarification
        self.owner = owner

    @property
    def starts_workflow(self):
        return (self.route == ROUTE_SELF_IMPROVEMENT and self.execution_intent
                and not self.needs_clarification)

    def as_dict(self):
        return {
            'route': self.route,
            'matched': self.matched,
            'reason': self.reason,
            'execution_intent': self.execution_intent,
            'needs_clarification': self.needs_clarification,
            'owner': self.owner,
            'starts_workflow': self.starts_workflow,
        }


def _has_execution_intent(normalized):
    return any(marker in normalized for marker in EXECUTION_MARKERS)


def _has_strong_execution_intent(normalized):
    return any(marker in normalized for marker in STRONG_EXECUTION_MARKERS)


def _asks_explanation(normalized):
    return any(marker in normalized for marker in QUESTION_MARKERS)


def _match_self_improvement(normalized):
    for phrase, imperative in SELF_IMPROVEMENT_PHRASES:
        if phrase in normalized:
            return phrase, imperative
    return None, False


def _match_update(normalized):
    if not any(referent in normalized for referent in AGENTORCH_REFERENTS):
        return None
    for verb in UPDATE_VERBS:
        if verb in normalized:
            return verb
    return None


def _asks_version_only(normalized):
    return ('agentorch' in normalized
            and any(marker in normalized for marker in VERSION_MARKERS)
            and any(marker in normalized for marker in QUESTION_MARKERS + ('多少',)))


def route_request(text):
    """Route one user message.

    Boundary rules (``docs/self-improvement/entrypoints-and-decisions.md`` §1):

    * a matching phrase needs an explicit execution intent; a bare noun phrase or
      a question about the mechanism only explains;
    * ``更新 AgentOrch`` belongs to the update engine, not to this workflow;
    * unrelated requests are left alone.
    """
    normalized = normalize_request(text)
    if not normalized:
        return RoutingDecision(ROUTE_UNRELATED, reason='empty request')

    if _asks_version_only(normalized):
        return RoutingDecision(
            ROUTE_EXPLAIN_ONLY, matched='agentorch version',
            reason='AgentOrch version question only; the update engine is not started',
            owner='update-engine')

    verb = _match_update(normalized)
    if verb is not None:
        excluded = [name for name in UPDATE_EXCLUDED_OBJECTS if name in normalized]
        if excluded:
            return RoutingDecision(
                ROUTE_UPDATE_AGENTORCH, matched=verb,
                reason='update target is ambiguous: ' + ', '.join(sorted(set(excluded))),
                needs_clarification=True, owner='update-engine')
        return RoutingDecision(
            ROUTE_UPDATE_AGENTORCH, matched=verb,
            reason='AgentOrch update request; owned by the update engine, not this workflow',
            execution_intent=True, owner='update-engine')

    phrase, imperative = _match_self_improvement(normalized)
    if phrase is not None:
        execution_intent = imperative or _has_execution_intent(normalized)
        if _asks_explanation(normalized) and not _has_strong_execution_intent(normalized):
            return RoutingDecision(
                ROUTE_EXPLAIN_ONLY, matched=phrase,
                reason='explanation request about the mechanism, not an execution command')
        if not execution_intent:
            return RoutingDecision(
                ROUTE_SELF_IMPROVEMENT, matched=phrase,
                reason='trigger phrase without an execution intent; ask once before starting',
                needs_clarification=True)
        return RoutingDecision(
            ROUTE_SELF_IMPROVEMENT, matched=phrase,
            reason='execution intent recognised', execution_intent=True)

    return RoutingDecision(ROUTE_UNRELATED, reason='no self-improvement or update trigger')


# --------------------------------------------------------------------------- #
# Risk and human-decision gates
# --------------------------------------------------------------------------- #

DECISION_OPEN_CONFLICT = 'HD-04'
DECISION_HIGH_RISK = 'HD-05'
DECISION_CONTRACT = 'HD-13'

# Governance surfaces whose meaning is frozen elsewhere. Changing them is a
# human decision, never a routine self-improvement task.
HIGH_RISK_PATH_PREFIXES = (
    'AGENTS.md',
    'kit/',
    'editions/',
    'docs/contracts/',
    'docs/design/',
    '.orchestration/',
    'scripts/orchestrator.py',
    '.github/workflows/',
    'release/',
)

# (keyword, decision id). Terms follow AGENTS.md §5 and the HD-* catalogue; the
# catalogue itself is not redefined here.
HIGH_RISK_KEYWORDS = (
    ('权限', DECISION_HIGH_RISK),
    ('安全边界', DECISION_HIGH_RISK),
    ('安全模型', DECISION_HIGH_RISK),
    ('锁', DECISION_HIGH_RISK),
    ('状态机', DECISION_HIGH_RISK),
    ('数据迁移', DECISION_HIGH_RISK),
    ('迁移', DECISION_HIGH_RISK),
    ('外部依赖', DECISION_HIGH_RISK),
    ('隐私', DECISION_HIGH_RISK),
    ('凭据', DECISION_HIGH_RISK),
    ('破坏兼容', DECISION_HIGH_RISK),
    ('不兼容', DECISION_HIGH_RISK),
    ('不可逆', DECISION_HIGH_RISK),
    ('销毁', DECISION_HIGH_RISK),
    ('schema', DECISION_CONTRACT),
    ('architecture contract', DECISION_CONTRACT),
)


def _lesson_risk_text(lesson):
    observation = lesson.get('observation') or {}
    applicability = lesson.get('applicability') or {}
    suggested = lesson.get('suggested_action') or {}
    parts = [
        lesson.get('title') or '',
        observation.get('root_cause') or '',
        observation.get('impact') or '',
        suggested.get('summary') or '',
        ' '.join(applicability.get('applies_to') or []),
    ]
    return ' '.join(parts)


def assess_risk(proposal, lesson):
    """Return ``(level, decision_id, reasons)`` for one proposal.

    Two independent inputs: governance paths in ``allowed_paths`` and risky
    wording in the lesson / goal. The mapping to ``HD-*`` is a reference to the
    catalogue, not a second definition.
    """
    reasons = []
    decision_id = None
    for path in proposal.get('allowed_paths') or []:
        normalized = str(path).replace('\\', '/').strip()
        for prefix in HIGH_RISK_PATH_PREFIXES:
            if normalized == prefix or normalized.startswith(prefix):
                reasons.append(f'allowed path touches a frozen governance surface: {prefix}')
                decision_id = decision_id or DECISION_HIGH_RISK
    text = (_lesson_risk_text(lesson) + ' ' + str(proposal.get('goal') or '')).casefold()
    for keyword, mapped in HIGH_RISK_KEYWORDS:
        if keyword.casefold() in text:
            reasons.append(f'lesson or goal mentions high-risk area: {keyword}')
            decision_id = decision_id or mapped
    if reasons:
        return 'HIGH', decision_id or DECISION_HIGH_RISK, reasons
    return 'LOW', None, []


# --------------------------------------------------------------------------- #
# Task packet completeness
# --------------------------------------------------------------------------- #

PROPOSAL_STATUSES = ('READY', 'BLOCKED', 'NEEDS_HUMAN_DECISION', 'NO_TASK',
                     'ALREADY_REGISTERED')

SCENARIO_FIELDS = ('invariants', 'negative_cases', 'migration_cases',
                   'integration_paths', 'unverified_assumptions')
TEXT_REQUIRED_FIELDS = ('title', 'goal', 'context')
AC_ID_RE = re.compile(r'^AC-[0-9]+$')
NA_ONLY = re.compile(r'^n/?a\s*[:：]?$', re.I)


def _text(value):
    return isinstance(value, str) and bool(value.strip())


def _as_list(value):
    if value is None:
        return []
    if isinstance(value, str):
        return [value]
    if isinstance(value, (list, tuple)):
        return list(value)
    return [value]


def _scenario_errors(packet):
    errors = []
    for field in SCENARIO_FIELDS:
        value = packet.get(field)
        if isinstance(value, (list, tuple)):
            text = ' '.join(str(item) for item in value).strip()
        else:
            text = value.strip() if isinstance(value, str) else ''
        if not text:
            errors.append(f'{field}: required; write "N/A: <reason>" only when the surface '
                          'truly does not apply')
        elif NA_ONLY.match(text):
            errors.append(f'{field}: bare N/A is not allowed; write "N/A: <reason>"')
    return errors


def validate_task_packet(packet):
    """Structural check for one improvement task packet.

    It proves "the fields exist and no unexecuted claim is dressed as a result",
    not that the proposal is a good idea. Field meanings are in
    ``kit/orchestration.md`` 4.1 and are not restated here.
    """
    errors = []
    for field in TEXT_REQUIRED_FIELDS:
        if not _text(packet.get(field)):
            errors.append(f'{field}: required non-empty text')

    allowed = packet.get('allowed_paths')
    if not isinstance(allowed, (list, tuple)) or not allowed:
        errors.append('allowed_paths: at least one concrete file or directory is required')
    else:
        for path in allowed:
            if not _text(path):
                errors.append('allowed_paths: entries must be non-empty strings')
                continue
            stripped = path.strip()
            if stripped in ('.', './', '/', '*', '**'):
                errors.append(f'allowed_paths: {stripped!r} is too broad; name concrete paths')
            elif '*' in stripped:
                errors.append(f'allowed_paths: wildcard {stripped!r} is not allowed')

    forbidden = packet.get('forbidden_paths')
    if not _as_list(forbidden) or not any(_text(item) for item in _as_list(forbidden)):
        errors.append('forbidden_paths: required (use "every other path" when nothing else '
                      'may change)')

    dependencies = packet.get('dependencies')
    if not isinstance(dependencies, (list, tuple)):
        errors.append('dependencies: must be a list (write [] when there is no dependency)')

    criteria = packet.get('acceptance_criteria')
    if not isinstance(criteria, (list, tuple)) or not criteria:
        errors.append('acceptance_criteria: at least one AC is required')
    else:
        seen = set()
        for index, criterion in enumerate(criteria):
            if not isinstance(criterion, dict):
                errors.append(f'acceptance_criteria[{index}]: must be a mapping with id / '
                              'statement / verification')
                continue
            ac_id = criterion.get('id')
            if not isinstance(ac_id, str) or not AC_ID_RE.match(ac_id):
                errors.append(f'acceptance_criteria[{index}].id: must match AC-<n>')
            elif ac_id in seen:
                errors.append(f'acceptance_criteria[{index}].id: duplicate {ac_id}')
            else:
                seen.add(ac_id)
            if not _text(criterion.get('statement')):
                errors.append(f'acceptance_criteria[{index}].statement: required')
            if not _text(criterion.get('verification')):
                errors.append(f'acceptance_criteria[{index}].verification: a command or a '
                              'manual check is required')

    errors.extend(_scenario_errors(packet))

    commands = packet.get('validation_commands')
    if not isinstance(commands, (list, tuple)) or not commands:
        errors.append('validation_commands: at least one command is required')
    else:
        for command in commands:
            if not _text(command):
                errors.append('validation_commands: entries must be non-empty strings')

    return errors


def _missing_spec_fields(proposal):
    missing = []
    for field in ('goal', 'context', 'allowed_paths', 'forbidden_paths',
                  'acceptance_criteria', 'validation_commands') + SCENARIO_FIELDS:
        value = proposal.get(field)
        if value is None or (isinstance(value, str) and not value.strip()):
            missing.append(field)
        elif isinstance(value, (list, tuple)) and not value:
            missing.append(field)
    return missing


# --------------------------------------------------------------------------- #
# Proposals
# --------------------------------------------------------------------------- #

class PlanResult:
    def __init__(self, proposals):
        self.proposals = list(proposals)

    @property
    def ready(self):
        return [p for p in self.proposals if p['status'] == 'READY']

    def by_status(self, status):
        return [p for p in self.proposals if p['status'] == status]

    def counts(self):
        counts = {status: 0 for status in PROPOSAL_STATUSES}
        for proposal in self.proposals:
            counts[proposal['status']] = counts.get(proposal['status'], 0) + 1
        return counts

    def as_dict(self):
        return {'proposals': self.proposals, 'counts': self.counts()}


def _proposal_from_spec(lesson, spec, edition):
    spec = dict(spec or {})
    proposal = {
        'lesson_id': lesson.get('lesson_id'),
        'title': spec.get('title') or lesson.get('title'),
        'goal': spec.get('goal'),
        'context': spec.get('context'),
        'edition': edition,
        'allowed_paths': _as_list(spec.get('allowed_paths')),
        'forbidden_paths': _as_list(spec.get('forbidden_paths')),
        'dependencies': _as_list(spec.get('dependencies')),
        'acceptance_criteria': [
            dict(criterion) if isinstance(criterion, dict) else criterion
            for criterion in _as_list(spec.get('acceptance_criteria'))
        ],
        'declared_resources': _as_list(spec.get('declared_resources')),
        'evidence_urls': list((lesson.get('source') or {}).get('evidence_urls') or []),
        'risks': spec.get('risks') or '',
        'status': 'BLOCKED',
        'status_reason': '',
        'required_human_decision': None,
        'risk_level': 'UNKNOWN',
        'risk_reasons': [],
    }
    for field in SCENARIO_FIELDS:
        value = spec.get(field)
        proposal[field] = _as_list(value) if isinstance(value, (list, tuple)) else value
    proposal['validation_commands'] = _as_list(spec.get('validation_commands'))
    return proposal


def build_proposal(lesson, spec, *, edition, dependency_state=None, coverage=None,
                   existing_task=None):
    """Turn one lesson + one explicit spec into a gated proposal.

    Status order is deliberate: a conflict or a high-risk surface is escalated
    before any structural check, so a broken packet never hides a human decision.
    """
    if edition not in ('local', 'github'):
        raise ValueError(f'unknown edition {edition!r}')
    proposal = _proposal_from_spec(lesson, spec, edition)

    open_conflicts = lessons.open_conflicts(lesson)
    if open_conflicts:
        proposal['status'] = 'NEEDS_HUMAN_DECISION'
        proposal['status_reason'] = 'lesson has an OPEN conflict: ' + ', '.join(
            conflict.get('lesson_id', '?') for conflict in open_conflicts)
        proposal['required_human_decision'] = DECISION_OPEN_CONFLICT
        return proposal

    if existing_task:
        proposal['status'] = 'ALREADY_REGISTERED'
        proposal['status_reason'] = f'task already registered as {existing_task}'
        proposal['risk_level'] = 'UNKNOWN'
        return proposal

    coverage_entry = (coverage or {}).get(lesson.get('lesson_id')) or {}
    if coverage_entry.get('status') == 'COVERED':
        proposal['status'] = 'NO_TASK'
        proposal['status_reason'] = ('existing mechanism already covers this lesson: '
                                     + str(coverage_entry.get('evidence') or 'no reference'))
        return proposal

    eligible, eligibility_reason = lessons.can_generate_task(lesson)
    if not eligible:
        proposal['status'] = 'NO_TASK'
        proposal['status_reason'] = eligibility_reason
        return proposal

    missing = _missing_spec_fields(proposal)
    packet_errors = validate_task_packet(proposal)
    if packet_errors:
        proposal['packet_errors'] = packet_errors

    # Risk is assessed before any structural blocker so a high-risk surface is
    # never hidden behind "the packet is incomplete".
    level, decision_id, reasons = assess_risk(proposal, lesson)
    proposal['risk_level'] = level
    proposal['risk_reasons'] = reasons
    if level == 'HIGH':
        proposal['status'] = 'NEEDS_HUMAN_DECISION'
        proposal['status_reason'] = 'high-risk surface: ' + '; '.join(reasons)
        proposal['required_human_decision'] = decision_id
        return proposal

    if missing:
        proposal['status'] = 'BLOCKED'
        proposal['status_reason'] = 'task spec is incomplete: ' + ', '.join(missing)
        return proposal

    if packet_errors:
        proposal['status'] = 'BLOCKED'
        proposal['status_reason'] = 'task packet is incomplete'
        return proposal

    unresolved = [ref for ref in proposal['dependencies']
                  if (dependency_state or {}).get(str(ref)) != 'VERIFIED']
    if unresolved:
        proposal['status'] = 'BLOCKED'
        proposal['status_reason'] = 'dependencies are not VERIFIED: ' + ', '.join(
            str(ref) for ref in unresolved)
        return proposal

    proposal['status'] = 'READY'
    proposal['status_reason'] = 'eligible lesson, complete packet, verified dependencies, low risk'
    return proposal


def plan_tasks(*, lesson_records, specs=None, edition, dependency_state=None,
               coverage=None, existing_tasks=None):
    """Gate every lesson through :func:`build_proposal`."""
    specs = specs or {}
    existing_tasks = existing_tasks or {}
    proposals = []
    for lesson in lesson_records:
        lesson_id = lesson.get('lesson_id')
        proposals.append(build_proposal(
            lesson, specs.get(lesson_id), edition=edition,
            dependency_state=dependency_state, coverage=coverage,
            existing_task=existing_tasks.get(lesson_id)))
    return PlanResult(proposals)


# --------------------------------------------------------------------------- #
# Rendering (carrier-independent content)
# --------------------------------------------------------------------------- #

FIELD_REFERENCE = ('> Field meanings and the G1 evidence rules live only in '
                   '`kit/orchestration.md` 4.1; this packet carries the fields and does not '
                   'restate them.')
UNTRUSTED_NOTICE = ('> Lesson-derived text in this packet is untrusted data: quoted for a '
                    'human reviewer, never executed and never used to widen allowed paths or '
                    'acceptance criteria.')


def _render_acceptance(criteria):
    lines = []
    for criterion in criteria or []:
        if not isinstance(criterion, dict):
            continue
        lines.append(f"- [ ] {criterion.get('id')}: {criterion.get('statement')}")
        lines.append(f"  - 验证：`{criterion.get('verification')}`")
    return lines


def _render_scenarios(proposal):
    lines = []
    for field in SCENARIO_FIELDS:
        value = proposal.get(field)
        if isinstance(value, (list, tuple)):
            value = '; '.join(str(item) for item in value)
        lines.append(f'- **{field}:** {value or "(missing)"}')
    return lines


def render_local_task_packet(proposal):
    """Markdown in the shape of ``templates/task-packet.md`` (local carrier)."""
    lines = ['# Local Task Packet', '', FIELD_REFERENCE, UNTRUSTED_NOTICE, '',
             f"- **Task ID:** (assigned by `scripts/orchestrator.py` at dispatch)",
             f"- **Title:** {proposal.get('title')}",
             f"- **Goal / user-visible result:** {proposal.get('goal')}",
             f"- **Context:** {proposal.get('context')}",
             f"- **Lesson:** {proposal.get('lesson_id')}",
             '- **Evidence links:** ' + (' '.join(f'<{url}>' for url in proposal.get(
                 'evidence_urls') or []) or '(none)'),
             '- **Allowed paths:** ' + (', '.join(proposal.get('allowed_paths') or [])
                                        or '(missing)'),
             '- **Forbidden paths:** ' + (', '.join(proposal.get('forbidden_paths') or [])
                                          or '(missing)'),
             '- **declared_resources:** ' + (', '.join(
                 proposal.get('declared_resources') or []) or '(none)'),
             '- **Dependencies (must be VERIFIED):** ' + (', '.join(
                 str(item) for item in proposal.get('dependencies') or []) or '无'),
             '- **Acceptance criteria:**']
    lines.extend('  ' + line for line in _render_acceptance(
        proposal.get('acceptance_criteria')))
    lines.extend(_render_scenarios(proposal))
    lines.extend([
        '- **Validation commands:** ' + (', '.join(
            f'`{command}`' for command in proposal.get('validation_commands') or [])
            or '(missing)'),
        '- **Evidence matrix:**',
        '  | ac_id | method | expected | actual | status | environment | commit_sha | '
        'evidence_ref | notes |',
        '  |------|--------|----------|--------|--------|-------------|------------|'
        '--------------|-------|',
        '- **Risks / human decisions:** ' + (proposal.get('risks') or '(none recorded)'),
        '- **Hard boundaries:** no scope expansion, no self-approval, no merge.',
    ])
    return '\n'.join(lines) + '\n'


def render_github_issue_body(proposal):
    """Markdown in the shape of ``editions/github/issue-template.md``."""
    lines = ['## 目标', '',
             UNTRUSTED_NOTICE, '',
             str(proposal.get('goal')), '',
             str(proposal.get('context') or ''), '',
             f"- Lesson: `{proposal.get('lesson_id')}`",
             '- Evidence: ' + (' '.join(f'<{url}>' for url in proposal.get(
                 'evidence_urls') or []) or '(none)'), '',
             '## 验收标准', '']
    lines.extend(_render_acceptance(proposal.get('acceptance_criteria')))
    lines.extend(['', FIELD_REFERENCE, '',
                  '**证据矩阵**（执行时填写；列顺序与 4.1 一致）：', '',
                  '| ac_id | method | expected | actual | status | environment | commit_sha | '
                  'evidence_ref | notes |',
                  '|------|--------|----------|--------|--------|-------------|------------|'
                  '--------------|-------|', '',
                  '## 可改路径', ''])
    lines.extend(f'- `{path}`' for path in proposal.get('allowed_paths') or [])
    lines.extend(['', '## 禁改路径', ''])
    lines.extend(f'- `{path}`' for path in proposal.get('forbidden_paths') or [])
    lines.extend(['', '## 依赖', ''])
    lines.append(', '.join(str(item) for item in proposal.get('dependencies') or []) or '无')
    lines.extend(['', '## 结构化场景字段', ''])
    lines.extend(_render_scenarios(proposal))
    lines.extend(['', '## 验证命令', ''])
    lines.extend(f'- `{command}`' for command in proposal.get('validation_commands') or [])
    lines.extend(['', '## 风险与人工决策', '',
                  str(proposal.get('risks') or '(none recorded)'),
                  f"- required_human_decision: {proposal.get('required_human_decision') or '(none)'}"])
    return '\n'.join(lines) + '\n'


def render_packet(proposal, edition):
    if edition == 'local':
        return render_local_task_packet(proposal)
    if edition == 'github':
        return render_github_issue_body(proposal)
    raise ValueError(f'unknown edition {edition!r}')
