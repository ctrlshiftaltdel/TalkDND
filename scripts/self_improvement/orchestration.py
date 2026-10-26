"""The self-improvement loop: trigger → restore → scan → plan → register → report (#50).

The loop is an orchestrator, not a new governance layer. It:

1. routes the user message (:func:`planning.route_request`) and stops unless the
   workflow is actually requested;
2. restores state from an injected snapshot and refuses to guess when a required
   fact (edition, branch, head SHA) is missing;
3. reads one finished scan report (``scripts/self_improvement/pipeline``) and
   reports ``NO_SOURCES`` / ``NOT_RUN`` / ``INCOMPLETE`` honestly instead of
   calling an unfinished scan "no new evidence";
4. gates every lesson through :mod:`planning`, which needs an explicit spec and
   escalates conflicts / high-risk surfaces;
5. hands only ``READY`` proposals to **one** carrier matching the edition, and
6. writes a closure report with the current head SHA, the human decisions and
   every command it did not run (``NOT_RUN``).

Deliberate omissions: this module never creates an Issue / task, never runs a
command, never adopts a lesson and never approves or merges anything. The two
built-in carriers are dry-run records: they return the exact payload a Master
would apply (``executed: false``), not a fabricated task number. Real carriers
are injected by the caller and are still subject to the same gates.

Standard library only; no network, no subprocess.
"""
from __future__ import annotations

import json
import re
import shlex
from pathlib import Path

from . import lessons, planning, untrusted

LOOP_SCHEMA_VERSION = 1

CARRIER_KINDS = ('local', 'github')
STATUS_NOT_TRIGGERED = 'NOT_TRIGGERED'
STATUS_CLARIFICATION_REQUIRED = 'CLARIFICATION_REQUIRED'
STATUS_BLOCKED = 'BLOCKED'
STATUS_NO_SOURCES = 'NO_SOURCES'
STATUS_SCAN_INCOMPLETE = 'SCAN_INCOMPLETE'
STATUS_NO_NEW_CANDIDATES = 'NO_NEW_CANDIDATES'
STATUS_NO_ACTIONABLE_CANDIDATES = 'NO_ACTIONABLE_CANDIDATES'
STATUS_NEEDS_HUMAN_DECISION = 'NEEDS_HUMAN_DECISION'
STATUS_COMPLETED = 'COMPLETED'

SCAN_CONCLUSIONS = ('NO_SOURCES', 'NOT_RUN', 'INCOMPLETE', 'NO_NEW_CANDIDATES',
                    'CANDIDATES_FOUND')

SHA_RE = re.compile(r'^[0-9a-f]{40}$')

LIMITATIONS = (
    'Trigger routing is a documented phrase/verb table, not a language model; the '
    'synthetic tests prove the table and the report, not every user phrasing.',
    'A scan that did not reach its declared scope is reported as NOT_RUN / INCOMPLETE; '
    'it is never rewritten as "no new evidence".',
    'Source strings are untrusted data: they are quoted and counted, never executed and '
    'never used to change allowed paths or acceptance criteria.',
    'This loop does not create Issues or tasks, run validation commands, approve, merge, '
    'or close anything; the carriers here are dry-run records.',
    'High-risk surfaces always end in NEEDS_HUMAN_DECISION here; an owner decision is '
    're-planned as a normal task, not auto-unlocked by a flag.',
)


# --------------------------------------------------------------------------- #
# Errors, carriers and registrations
# --------------------------------------------------------------------------- #

class LoopError(Exception):
    def __init__(self, message, details=()):
        super().__init__(message)
        self.details = list(details)


class CarrierError(LoopError):
    pass


class Registration:
    """What one carrier recorded for one proposal. ``executed`` is explicit."""

    def __init__(self, *, carrier, ref, status='PLANNED', executed=False, payload=None):
        self.carrier = carrier
        self.ref = ref
        self.status = status
        self.executed = executed
        self.payload = payload or {}

    def as_dict(self):
        return {'carrier': self.carrier, 'ref': self.ref, 'status': self.status,
                'executed': self.executed, 'payload': self.payload}


def _carrier_precheck(carrier, proposal):
    """Gates shared by both built-in carriers (and expected of injected ones)."""
    if proposal.get('status') != 'READY':
        raise CarrierError(f'carrier received a {proposal.get("status")} proposal; only READY '
                           'proposals may be registered')
    if proposal.get('edition') != carrier.edition:
        raise CarrierError(f'proposal edition {proposal.get("edition")!r} does not match '
                           f'carrier edition {carrier.edition!r}')
    errors = planning.validate_task_packet(proposal)
    if errors:
        raise CarrierError('carrier refused an incomplete task packet', errors)


class _DryRunCarrier:
    """Base carrier: records the payload, never executes it."""

    kind = None

    def __init__(self, edition):
        if edition != self.kind:
            raise CarrierError(f'{self.kind} carrier cannot write a {edition} edition')
        self.edition = edition

    def register(self, proposal):
        _carrier_precheck(self, proposal)
        return Registration(carrier=self.kind,
                            ref=f'dry-run:{proposal.get("lesson_id")}',
                            status='PLANNED', executed=False,
                            payload=self.payload(proposal))

    def payload(self, proposal):  # pragma: no cover - overridden
        raise NotImplementedError


class LocalCarrier(_DryRunCarrier):
    """local edition: the payload is the task-database command sequence, unexecuted."""

    kind = 'local'

    def __init__(self, edition='local'):
        super().__init__(edition)

    def payload(self, proposal):
        task_id = 'TASK-<assigned>'
        arguments = ['python', 'scripts/orchestrator.py', 'task', 'add', '--id', task_id,
                     '--title', proposal.get('title') or 'untitled']
        for path in proposal.get('allowed_paths') or []:
            arguments.extend(['--paths', str(path)])
        for criterion in proposal.get('acceptance_criteria') or []:
            if isinstance(criterion, dict):
                arguments.extend(['--acceptance',
                                  f"{criterion.get('id')}: {criterion.get('statement')}"])
        add = ' '.join(shlex.quote(str(item)) for item in arguments)
        commands = [add,
                    f'python scripts/orchestrator.py task ready {task_id}',
                    f'python scripts/orchestrator.py task dispatch {task_id}']
        return {'task_id': task_id, 'state': 'PLANNED', 'commands': commands,
                'packet': planning.render_local_task_packet(proposal)}


class GithubCarrier(_DryRunCarrier):
    """github edition: the payload is the Issue body + create command, unexecuted."""

    kind = 'github'

    def __init__(self, edition='github'):
        super().__init__(edition)

    def payload(self, proposal):
        return {'issue_number': None, 'state': 'PLANNED',
                'commands': ['gh issue create --title <title> --body-file <issue-body>'],
                'issue_body': planning.render_github_issue_body(proposal)}


def carrier_for(edition):
    if edition == 'local':
        return LocalCarrier()
    if edition == 'github':
        return GithubCarrier()
    raise LoopError(f'unknown edition {edition!r}; expected one of {CARRIER_KINDS}')


# --------------------------------------------------------------------------- #
# State and scan
# --------------------------------------------------------------------------- #

def restore_state(state, *, edition):
    """Validate the injected snapshot. Returns ``(normalized, errors)``.

    The caller is responsible for the real commands; the loop only refuses to
    proceed when the facts it needs are absent or contradictory.
    """
    errors = []
    state = dict(state or {})
    if edition not in CARRIER_KINDS:
        errors.append(f'edition must be one of {CARRIER_KINDS}, got {edition!r}')
    declared = state.get('edition')
    if declared is None:
        errors.append('state.edition is missing; read docs/PROJECT.md instead of assuming')
    elif declared != edition:
        errors.append(f'state.edition {declared!r} disagrees with the routed edition '
                      f'{edition!r}; stop and reconcile the carrier before dispatching')
    head_sha = state.get('head_sha')
    if not isinstance(head_sha, str) or not SHA_RE.match(head_sha):
        errors.append('state.head_sha must be the full 40-character commit SHA')
    branch = state.get('branch')
    if not isinstance(branch, str) or not branch.strip():
        errors.append('state.branch is missing')
    workspace = state.get('workspace') or {}
    normalized = {
        'edition': declared,
        'branch': branch,
        'head_sha': head_sha,
        'workspace_clean': workspace.get('clean'),
        'workspace_status': list(workspace.get('status_short') or []),
        'active_tasks': list(state.get('active_tasks') or []),
        'open_pull_requests': list(state.get('open_pull_requests') or []),
    }
    if normalized['workspace_clean'] is None:
        normalized['workspace_clean'] = not normalized['workspace_status']
    return normalized, errors


def _scan_payload(scan_report):
    if scan_report is None:
        return None
    if hasattr(scan_report, 'as_dict'):
        return scan_report.as_dict()
    if hasattr(scan_report, 'report'):
        payload = dict(scan_report.report)
        ingestions = getattr(scan_report, 'ingestions', None)
        if ingestions is not None:
            payload['ingestions'] = [item.as_dict() for item in ingestions]
        return payload
    if isinstance(scan_report, dict):
        return dict(scan_report)
    raise LoopError('scan_report must be a dict, a ScanOutcome or None')


def summarize_scan(scan_report, lesson_records=()):
    """Classify one scan without upgrading a gap into "no new evidence"."""
    payload = _scan_payload(scan_report)
    if payload is None:
        return {'conclusion': 'NOT_RUN', 'reason': 'no scan was supplied',
                'sources': [], 'new_candidates': 0, 'ingestions_available': False}
    sources = list(payload.get('sources') or [])
    ingestions = payload.get('ingestions')
    ingestions_available = ingestions is not None
    created = [item.get('lesson_id') for item in (ingestions or [])
               if item.get('kind') == 'created']
    base = {'sources': sources, 'new_candidates': len(created),
            'ingestions_available': ingestions_available,
            'scan_mode': payload.get('scan_mode'),
            'counts': dict(payload.get('counts') or {})}
    if not sources:
        base.update(conclusion='NO_SOURCES',
                    reason='无来源可扫描 (no sources registered); no scan could have run')
        return base
    statuses = [source.get('scan_status') for source in sources]
    reached = [status for status in statuses if status in ('OK', 'PARTIAL')]
    incomplete = [source.get('source_id') for source in sources
                  if source.get('scan_status') not in ('OK',)]
    base['incomplete_sources'] = incomplete
    if not reached:
        conclusion = ('NOT_RUN' if all(status == 'NOT_RUN' for status in statuses)
                      else 'INCOMPLETE')
        base.update(conclusion=conclusion,
                    reason='未完成扫描 (scan incomplete): no source reached OK/PARTIAL; a gap '
                           'is not "no new evidence"')
        return base
    if created or lesson_records:
        base.update(conclusion='CANDIDATES_FOUND',
                    reason='new or previously captured candidates are available')
    else:
        base.update(conclusion='NO_NEW_CANDIDATES',
                    reason='无新候选 (no new candidates): the scan reached its scope and '
                           'produced no candidate')
    if incomplete:
        base['reason'] += '; incomplete sources: ' + ', '.join(
            str(name) for name in incomplete if name)
    return base


# --------------------------------------------------------------------------- #
# The loop
# --------------------------------------------------------------------------- #

class LoopOutcome:
    def __init__(self, report):
        self.report = report

    @property
    def status(self):
        return self.report['status']

    @property
    def markdown(self):
        return self.report['markdown']

    def as_dict(self):
        return self.report


def _validation_rows(plan, command_runs, head_sha, environment):
    rows = []
    seen = []
    for proposal in plan.ready:
        for command in proposal.get('validation_commands') or []:
            if command not in seen:
                seen.append(command)
    for command in seen:
        run = (command_runs or {}).get(command)
        if run is None:
            rows.append({'command': command, 'status': 'NOT_RUN', 'exit_code': None,
                         'actual': 'not executed in this loop run',
                         'environment': environment, 'commit_sha': head_sha,
                         'notes': 'never counts as passing'})
            continue
        if isinstance(run, dict):
            exit_code = run.get('exit_code')
            actual = run.get('output') or ''
        else:
            exit_code = run
            actual = ''
        status = 'PASS' if exit_code == 0 else 'FAIL'
        rows.append({'command': command, 'status': status, 'exit_code': exit_code,
                     'actual': untrusted.data_cell(str(actual)) if actual else '',
                     'environment': environment, 'commit_sha': head_sha,
                     'notes': '(none)'})
    return rows


def run_loop(*, request_text, edition, state, carrier=None, scan_report=None,
             lesson_records=(), specs=None, coverage=None, dependency_state=None,
             existing_tasks=None, command_runs=None, environment='', now=None):
    """Run one self-improvement loop pass and return a closure report."""
    if edition not in CARRIER_KINDS:
        raise LoopError(f'unknown edition {edition!r}; expected one of {CARRIER_KINDS}')
    routing = planning.route_request(request_text)
    generated_at = now or lessons.now_utc()
    report = {
        'loop_schema_version': LOOP_SCHEMA_VERSION,
        'generated_at': generated_at,
        'trigger': {'text': untrusted.data_cell(request_text, limit=200),
                    **routing.as_dict()},
        'edition': edition,
        'state': {},
        'scan': {},
        'plan': {'proposals': [], 'counts': {}},
        'registrations': [],
        'decisions_required': [],
        'blocked': [],
        'validation': [],
        'not_run': [],
        'counts': {},
        'limitations': list(LIMITATIONS),
    }

    if routing.route == planning.ROUTE_UPDATE_AGENTORCH:
        report['status'] = STATUS_NOT_TRIGGERED
        report['reason'] = 'AgentOrch update requests belong to the update engine'
        report['markdown'] = render_markdown(report)
        return LoopOutcome(report)
    if routing.route in (planning.ROUTE_EXPLAIN_ONLY, planning.ROUTE_UNRELATED):
        report['status'] = STATUS_NOT_TRIGGERED
        report['reason'] = routing.reason
        report['markdown'] = render_markdown(report)
        return LoopOutcome(report)
    if not routing.starts_workflow:
        report['status'] = STATUS_CLARIFICATION_REQUIRED
        report['reason'] = routing.reason
        report['decisions_required'].append({
            'decision_id': None, 'kind': 'clarify_trigger',
            'lesson_id': None, 'reason': routing.reason})
        report['markdown'] = render_markdown(report)
        return LoopOutcome(report)

    normalized_state, state_errors = restore_state(state, edition=edition)
    report['state'] = normalized_state
    if state_errors:
        report['status'] = STATUS_BLOCKED
        report['reason'] = 'state could not be restored from real facts'
        report['blocked'].extend({'kind': 'state', 'reason': error}
                                 for error in state_errors)
        report['markdown'] = render_markdown(report)
        return LoopOutcome(report)

    kind = getattr(carrier, 'kind', None)
    if kind != edition:
        raise LoopError(f'carrier kind {kind!r} does not match edition {edition!r}; '
                        'one edition writes one carrier, never both')
    register = getattr(carrier, 'register', None)
    if not callable(register):
        raise LoopError('carrier must expose register(proposal)')

    scan = summarize_scan(scan_report, lesson_records)
    report['scan'] = scan
    for decision in (_scan_payload(scan_report) or {}).get('decisions_required') or []:
        report['decisions_required'].append(dict(decision))

    def stop(status, reason):
        # A scan-level decision (open conflict, private source, cursor regression)
        # must stay visible even when no candidate is actionable.
        report['status'] = (STATUS_NEEDS_HUMAN_DECISION
                            if report['decisions_required'] else status)
        report['reason'] = reason
        report['markdown'] = render_markdown(report)
        return LoopOutcome(report)

    if scan['conclusion'] == 'NO_SOURCES':
        return stop(STATUS_NO_SOURCES, scan['reason'])
    if scan['conclusion'] in ('NOT_RUN', 'INCOMPLETE'):
        return stop(STATUS_SCAN_INCOMPLETE, scan['reason'])
    if scan['conclusion'] == 'NO_NEW_CANDIDATES':
        return stop(STATUS_NO_NEW_CANDIDATES, scan['reason'])

    plan = planning.plan_tasks(
        lesson_records=list(lesson_records), specs=specs or {}, edition=edition,
        dependency_state=dependency_state or {}, coverage=coverage or {},
        existing_tasks=existing_tasks or {})
    report['plan'] = plan.as_dict()

    for proposal in plan.proposals:
        if proposal['status'] == 'NEEDS_HUMAN_DECISION':
            report['decisions_required'].append({
                'decision_id': proposal.get('required_human_decision'),
                'kind': 'high_risk_or_conflict',
                'lesson_id': proposal.get('lesson_id'),
                'reason': proposal.get('status_reason'),
            })
        elif proposal['status'] == 'BLOCKED':
            report['blocked'].append({'kind': 'proposal', 'lesson_id': proposal.get(
                'lesson_id'), 'reason': proposal.get('status_reason')})

    for proposal in plan.ready:
        try:
            registration = register(proposal)
        except CarrierError as exc:
            report['blocked'].append({'kind': 'carrier', 'lesson_id': proposal.get(
                'lesson_id'), 'reason': str(exc), 'details': exc.details})
            continue
        if not isinstance(registration, Registration):
            raise LoopError('carrier must return a Registration')
        if registration.carrier != edition:
            raise LoopError(f'carrier returned {registration.carrier!r} for edition '
                            f'{edition!r}; refusing a cross-edition write')
        report['registrations'].append(registration.as_dict())

    report['validation'] = _validation_rows(
        plan, command_runs, normalized_state['head_sha'], environment)
    report['not_run'] = [row['command'] for row in report['validation']
                         if row['status'] == 'NOT_RUN']
    report['counts'] = {
        'lessons': len(plan.proposals),
        'ready': len(plan.ready),
        'registered': len(report['registrations']),
        'blocked': len(report['blocked']),
        'decisions': len(report['decisions_required']),
        'not_run_commands': len(report['not_run']),
        **{f'proposal_{key}': value for key, value in plan.counts().items()},
    }

    if report['decisions_required']:
        report['status'] = STATUS_NEEDS_HUMAN_DECISION
        report['reason'] = ('some candidates need a human decision; no high-risk proposal '
                            'was registered')
    elif report['registrations']:
        report['status'] = STATUS_COMPLETED
        report['reason'] = ('planned and registered; independent review, approval and merge '
                            'remain outside this loop')
    elif report['blocked']:
        report['status'] = STATUS_BLOCKED
        report['reason'] = 'no candidate could be registered'
    else:
        report['status'] = STATUS_NO_ACTIONABLE_CANDIDATES
        report['reason'] = 'candidates exist but none is eligible for a task'

    report['markdown'] = render_markdown(report)
    return LoopOutcome(report)


# --------------------------------------------------------------------------- #
# Report rendering
# --------------------------------------------------------------------------- #

def render_markdown(report):
    lines = ['# Self-improvement loop report', '',
             f"- status: `{report.get('status')}`",
             f"- generated_at: `{report.get('generated_at')}`",
             f"- edition: `{report.get('edition')}`",
             f"- reason: {untrusted.data_cell(report.get('reason'), limit=300)}", '']
    trigger = report.get('trigger') or {}
    lines.extend(['## Trigger', '',
                  f"- route: `{trigger.get('route')}`",
                  f"- matched: `{trigger.get('matched') or '(none)'}`",
                  f"- starts_workflow: `{trigger.get('starts_workflow')}`",
                  f"- request (data): {trigger.get('text') or '(empty)'}", ''])
    state = report.get('state') or {}
    if state:
        lines.extend(['## Restored state', '',
                      f"- branch: `{state.get('branch')}`",
                      f"- head_sha: `{state.get('head_sha')}`",
                      f"- workspace_clean: `{state.get('workspace_clean')}`",
                      f"- active_tasks: `{len(state.get('active_tasks') or [])}`", ''])
    scan = report.get('scan') or {}
    if scan:
        lines.extend(['## Scan', '',
                      f"- conclusion: `{scan.get('conclusion')}`",
                      f"- reason: {untrusted.data_cell(scan.get('reason'))}",
                      f"- new_candidates: `{scan.get('new_candidates', 0)}`",
                      '- sources:'])
        for source in scan.get('sources') or []:
            lines.append(
                f"  - `{source.get('source_id')}` `{source.get('scan_status')}` "
                f"`{source.get('blocked_reason') or '(none)'}`")
        if not scan.get('sources'):
            lines.append('  - (none)')
        lines.append('')
    proposals = (report.get('plan') or {}).get('proposals') or []
    if proposals:
        lines.extend(['## Plan', '',
                      '| lesson_id | status | reason | decision |',
                      '|---|---|---|---|'])
        for proposal in proposals:
            lines.append('| ' + ' | '.join([
                f"`{proposal.get('lesson_id')}`",
                f"`{proposal.get('status')}`",
                untrusted.data_cell(proposal.get('status_reason')),
                f"`{proposal.get('required_human_decision') or '(none)'}`",
            ]) + ' |')
        lines.append('')
    registrations = report.get('registrations') or []
    lines.extend(['## Registered tasks', ''])
    if registrations:
        lines.append('| carrier | ref | status | executed |')
        lines.append('|---|---|---|---|')
        for registration in registrations:
            lines.append(f"| `{registration.get('carrier')}` | `{registration.get('ref')}` | "
                         f"`{registration.get('status')}` | `{registration.get('executed')}` |")
    else:
        lines.append('- (none)')
    lines.append('')
    decisions = report.get('decisions_required') or []
    lines.extend(['## Decisions required', ''])
    if decisions:
        for decision in decisions:
            lines.append(f"- `{decision.get('decision_id') or 'clarify'}` "
                         f"{untrusted.data_cell(decision.get('lesson_id'))}: "
                         f"{untrusted.data_cell(decision.get('reason'))}")
    else:
        lines.append('- (none)')
    lines.append('')
    blocked = report.get('blocked') or []
    lines.extend(['## Blocked', ''])
    if blocked:
        for item in blocked:
            lines.append(f"- `{item.get('kind')}` {untrusted.data_cell(item.get('lesson_id'))}: "
                         f"{untrusted.data_cell(item.get('reason'))}")
    else:
        lines.append('- (none)')
    lines.append('')
    validation = report.get('validation') or []
    lines.extend(['## Validation commands', ''])
    if validation:
        lines.append('| command | status | exit_code | commit_sha |')
        lines.append('|---|---|---|---|')
        for row in validation:
            lines.append(f"| `{untrusted.data_cell(row.get('command'))}` | `{row.get('status')}` | "
                         f"`{row.get('exit_code')}` | `{row.get('commit_sha')}` |")
    else:
        lines.append('- (none)')
    lines.append('')
    lines.extend(['## Not run', ''])
    if report.get('not_run'):
        for command in report['not_run']:
            lines.append(f"- `{untrusted.data_cell(command)}` — NOT_RUN")
    else:
        lines.append('- (none)')
    lines.extend(['', '## Limitations', ''])
    lines.extend(f'- {limitation}' for limitation in report.get('limitations') or [])
    lines.append('')
    return '\n'.join(lines)


def write_report(report, path):
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n',
                      encoding='utf-8')
    return target


def write_markdown(report, path):
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(report.get('markdown') or render_markdown(report), encoding='utf-8')
    return target
