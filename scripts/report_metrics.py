#!/usr/bin/env python3
"""Read-only delivery metrics for the Agent-Orchestration kit.

The seven metric definitions live in ``templates/metrics.md``; this module
implements them and prints the metric ``id`` on every row so a report can be
traced back to that definition. It reads task-library events and evidence
references only:

* no ``note`` column, no prompt text, no message bodies, no credentials;
* no inference: a task without the events a metric needs is ``missing``, a
  record kind the source does not collect makes the metric ``unknown``, and an
  empty dataset makes it ``no data`` -- never ``0``.

Sources:

* a SQLite task library (``.orchestration/state.sqlite3``) opened read-only;
* a JSON event sample (``--gold-sample`` reads the fixed sample embedded in
  ``templates/metrics.md``).

Exit codes: ``0`` for a produced report (including "no data"), ``2`` when the
source cannot be read.
"""
from pathlib import Path
import argparse, json, sqlite3, sys
from datetime import datetime, timedelta, timezone
from statistics import median

SCHEMA_VERSION = '1.0'
TIMEZONE = 'UTC'
GOLD_SAMPLE_TEMPLATE = Path(__file__).resolve().parents[1] / 'templates' / 'metrics.md'
GOLD_SAMPLE_FENCE = 'metrics-sample'

STATUS_CHANGED = 'STATUS_CHANGED'
CHECK_RECORDED = 'CHECK_RECORDED'
ACCEPTANCE_RECORDED = 'ACCEPTANCE_RECORDED'
CONFLICT_RECORDED = 'CONFLICT_RECORDED'
PATH_PREFLIGHT_REFUSED = 'PATH_PREFLIGHT_REFUSED'
REGRESSION_REPORTED = 'REGRESSION_REPORTED'
APPROVAL_INVALIDATED = 'APPROVAL_INVALIDATED'
TASK_DISPATCHED = 'TASK_DISPATCHED'

KNOWN_ACTIONS = frozenset({
    'TASK_CREATED', STATUS_CHANGED, TASK_DISPATCHED, 'WORKSPACE_UPDATED',
    'LOCKS_RELEASED', 'MANUAL_UNLOCK', 'RECORD_AMENDED', APPROVAL_INVALIDATED,
    CHECK_RECORDED, ACCEPTANCE_RECORDED, CONFLICT_RECORDED, PATH_PREFLIGHT_REFUSED,
    REGRESSION_REPORTED,
})
# Record kinds the report can consume. `local` task libraries carry the
# resource-conflict subset, plus the path conflicts a refused dispatch now
# writes; the rest stay `unknown` until a task records them, which is exactly
# what the report must say instead of printing 0.
COLLECTION_KINDS = ('checks', 'path_conflicts', 'resource_conflicts',
                    'regressions', 'acceptances')
SQLITE_COLLECTION = {
    'checks': False,
    # A refused dispatch writes CONFLICT_RECORDED kind=path, but libraries from
    # before that event hold none: the flag stays False here and `load_sqlite`
    # promotes it only when the dataset really carries a path conflict, so a
    # library without one stays `unknown` instead of being reported as 0.
    'path_conflicts': False,
    'resource_conflicts': True,   # resource_lock_events LOCK_DENIED_CONFLICT
    'regressions': False,
    'acceptances': False,
}
PASS_FAIL = ('PASS', 'FAIL')
AC_PASSING = ('PASS', 'NOT_APPLICABLE')
REWORK_TARGETS = ('IN_PROGRESS', 'BLOCKED', 'FAILED')

LIMITS = (
    'Values describe this window and this sample only; they are not a ranking of agents, '
    'people, models or runtimes.',
    'A metric with low coverage or a high missing rate is reported but cannot support a conclusion.',
    'Records the source does not collect make a metric `unknown`, never 0.',
    'Durations are wall-clock hours, not effort; the window start and end change them.',
)


class SourceError(Exception):
    """The source cannot be read at all; the caller exits with code 2."""


# --- time handling -----------------------------------------------------------

def parse_at(raw):
    """(aware UTC datetime, was_naive) for an ISO-8601 string, or (None, False)."""
    if not isinstance(raw, str) or not raw.strip():
        return None, False
    text = raw.strip()
    if text.endswith(('Z', 'z')):
        text = text[:-1] + '+00:00'
    try:
        parsed = datetime.fromisoformat(text)
    except ValueError:
        return None, False
    naive = parsed.tzinfo is None
    if naive:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc), naive


def parse_bound(raw, label):
    """Window bound: a date means 00:00 UTC of that day."""
    parsed, _ = parse_at(raw)
    if parsed is None:
        raise SourceError(f'{label} is not an ISO-8601 date or timestamp: {raw!r}')
    return parsed


def iso(value):
    return None if value is None else value.astimezone(timezone.utc).isoformat().replace('+00:00', 'Z')


def hours(delta):
    return delta.total_seconds() / 3600.0


def r4(value):
    return None if value is None else round(float(value), 4)


def p90(values):
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, int(round(0.9 * (len(ordered) - 1)))))
    return ordered[index]


# --- sources -----------------------------------------------------------------

def _normalise_event(raw, index):
    """A report event, or (None, reason) when the record cannot be used."""
    if not isinstance(raw, dict):
        return None, 'not an object'
    task_id = raw.get('task_id') or raw.get('task')
    if not isinstance(task_id, str) or not task_id.strip():
        return None, 'missing task_id'
    action = raw.get('action')
    if not isinstance(action, str) or not action.strip():
        return None, 'missing action'
    at, naive = parse_at(raw.get('at'))
    if at is None:
        return None, 'unparsable timestamp'
    data = raw.get('data') if isinstance(raw.get('data'), dict) else {}
    return {
        'seq': index,
        'task_id': task_id.strip(),
        'at': at,
        'naive': naive,
        'action': action.strip(),
        'from_status': (raw.get('from_status') or '').strip() or None,
        'to_status': (raw.get('to_status') or '').strip() or None,
        'data': data,
        'ref': (raw.get('ref') or raw.get('evidence_ref') or '') or '',
    }, None


def build_dataset(kind, path, label, tasks, raw_events, collection):
    events, ignored, naive = [], 0, 0
    for index, raw in enumerate(raw_events):
        event, _reason = _normalise_event(raw, index)
        if event is None:
            ignored += 1
            continue
        naive += 1 if event['naive'] else 0
        if event['action'] not in KNOWN_ACTIONS:
            ignored += 1
            continue
        events.append(event)
    events.sort(key=lambda e: (e['at'], e['seq']))
    task_ids = {e['task_id'] for e in events}
    declared = []
    for item in tasks or []:
        if isinstance(item, dict) and item.get('id'):
            declared.append(str(item['id']))
        elif isinstance(item, str):
            declared.append(item)
    return {
        'kind': kind,
        'path': str(path),
        'label': label,
        'events': events,
        'tasks': sorted(set(declared) | task_ids),
        'tasks_with_events': len(task_ids),
        'records_ignored': ignored,
        'naive_timestamps': naive,
        'collection': collection,
    }


def load_fixture(path):
    try:
        payload = json.loads(Path(path).read_text(encoding='utf-8'))
    except (OSError, ValueError) as exc:
        raise SourceError(f'cannot read fixture {path}: {exc}')
    if not isinstance(payload, dict) or not isinstance(payload.get('events'), list):
        raise SourceError(f'fixture {path} must be an object with an "events" array')
    declared = payload.get('collects') if isinstance(payload.get('collects'), dict) else {}
    collection = {kind: bool(declared.get(kind, True)) for kind in COLLECTION_KINDS}
    return build_dataset('fixture', path, payload.get('label') or Path(path).name,
                         payload.get('tasks'), payload['events'], collection)


def load_sqlite(path):
    db = Path(path)
    if not db.is_file():
        raise SourceError(f'task library not found: {path}')
    uri = f'file:{db.resolve().as_posix()}?mode=ro'
    try:
        con = sqlite3.connect(uri, uri=True)
    except sqlite3.Error as exc:
        raise SourceError(f'cannot open task library {path} read-only: {exc}')
    con.row_factory = sqlite3.Row
    try:
        rows = con.execute(
            'SELECT task_id, at, action, from_status, to_status, data_json FROM events ORDER BY id'
        ).fetchall()
        tasks = [{'id': r['id']} for r in con.execute('SELECT id FROM tasks').fetchall()]
    except sqlite3.Error as exc:
        con.close()
        raise SourceError(f'task library {path} has no readable events/tasks table: {exc}')
    raw = []
    for row in rows:
        try:
            data = json.loads(row['data_json'] or '{}')
        except ValueError:
            data = {}
        raw.append({
            'task_id': row['task_id'], 'at': row['at'], 'action': row['action'],
            'from_status': row['from_status'], 'to_status': row['to_status'],
            'data': data if isinstance(data, dict) else {},
            'ref': f'task/{row["task_id"]}',
        })
    collection = dict(SQLITE_COLLECTION)
    # The event is the evidence: only a library that actually holds a path
    # conflict is reported as collecting it. Older libraries keep the legacy
    # False and stay `unknown`, never a fabricated 0.
    if any(entry['action'] == CONFLICT_RECORDED
           and str(entry['data'].get('kind') or '').lower() == 'path' for entry in raw):
        collection['path_conflicts'] = True
    try:
        conflicts = con.execute(
            "SELECT task_id, at, resource_id FROM resource_lock_events "
            "WHERE event_type='LOCK_DENIED_CONFLICT'"
        ).fetchall()
    except sqlite3.Error:
        conflicts = []
        collection['resource_conflicts'] = False
    for row in conflicts:
        raw.append({
            'task_id': row['task_id'], 'at': row['at'], 'action': CONFLICT_RECORDED,
            'data': {'kind': 'resource', 'resource_id': row['resource_id']},
            'ref': f'task/{row["task_id"]}/resource-lock',
        })
    con.close()
    return build_dataset('sqlite', path, db.name, tasks, raw, collection)


def gold_sample_text(root=GOLD_SAMPLE_TEMPLATE):
    """The fixed synthetic dataset embedded in templates/metrics.md."""
    try:
        text = Path(root).read_text(encoding='utf-8')
    except OSError as exc:
        raise SourceError(f'cannot read the metrics template: {exc}')
    lines, i = text.splitlines(), 0
    while i < len(lines):
        if lines[i].strip().startswith(f'```{GOLD_SAMPLE_FENCE}'):
            body, i = [], i + 1
            while i < len(lines) and lines[i].strip() != '```':
                body.append(lines[i])
                i += 1
            return '\n'.join(body)
        i += 1
    raise SourceError(f'no ```{GOLD_SAMPLE_FENCE} block found in {root}')


def load_gold_sample(root=GOLD_SAMPLE_TEMPLATE):
    try:
        payload = json.loads(gold_sample_text(root))
    except ValueError as exc:
        raise SourceError(f'the gold sample is not valid JSON: {exc}')
    if not isinstance(payload, dict) or not isinstance(payload.get('events'), list):
        raise SourceError('the gold sample must be an object with an "events" array')
    declared = payload.get('collects') if isinstance(payload.get('collects'), dict) else {}
    collection = {kind: bool(declared.get(kind, True)) for kind in COLLECTION_KINDS}
    return build_dataset('fixture', root, payload.get('label') or 'gold-sample',
                         payload.get('tasks'), payload['events'], collection)


# --- metric scaffolding ------------------------------------------------------

def blank_metric(mid, name, unit):
    return {
        'id': mid, 'name': name, 'unit': unit, 'status': 'no_data', 'value': None,
        'sample_size': 0, 'eligible': 0, 'missing': 0, 'missing_rate': None,
        'coverage': 0.0, 'excluded': 0, 'detail': {}, 'notes': [],
    }


def finish(metric, sample, eligible, missing, excluded=0):
    metric['sample_size'] = sample
    metric['eligible'] = eligible
    metric['missing'] = missing
    metric['excluded'] = excluded
    metric['missing_rate'] = r4(missing / eligible) if eligible else 0.0
    metric['coverage'] = r4(sample / eligible) if eligible else 0.0
    if metric['status'] == 'unknown':
        metric['value'] = None
        metric['coverage'] = 0.0
        return metric
    if sample == 0:
        # `no_data` means nothing was eligible; eligible-but-all-missing is a
        # coverage result, not an empty window.
        metric['status'] = 'no_data' if eligible == 0 else 'partial'
        metric['value'] = None
        return metric
    metric['status'] = 'ok' if missing == 0 and excluded == 0 else 'partial'
    return metric


def mark_unknown(metric, reason, eligible=0, missing=0):
    metric['status'] = 'unknown'
    metric['notes'].append(reason)
    return finish(metric, 0, eligible, missing)


def by_task(events):
    grouped = {}
    for event in events:
        grouped.setdefault(event['task_id'], []).append(event)
    return grouped


# --- metrics -----------------------------------------------------------------

def metric_ready_to_verified(grouped):
    m = blank_metric('M1', 'ready_to_verified', 'hours')
    durations, missing_tasks, excluded_tasks = [], [], []
    eligible = 0
    for task_id, events in grouped.items():
        readies = [e for e in events if e['to_status'] == 'READY']
        if not readies:
            continue
        eligible += 1
        verified = [e for e in events if e['to_status'] == 'VERIFIED']
        pair = None
        for ready in reversed(readies):
            end = next((v for v in verified if v['at'] >= ready['at']), None)
            if end is not None:
                pair = (ready, end)
                break
        if pair is None:
            missing_tasks.append(task_id)
            continue
        value = hours(pair[1]['at'] - pair[0]['at'])
        if value < 0:
            excluded_tasks.append(task_id)
            continue
        durations.append(value)
    sample = len(durations)
    m['value'] = r4(median(durations)) if durations else None
    m['detail'] = {
        'mean_hours': r4(sum(durations) / sample) if sample else None,
        'median_hours': m['value'],
        'p90_hours': r4(p90(durations)),
        'max_hours': r4(max(durations)) if durations else None,
        'missing_tasks': sorted(missing_tasks),
        'excluded_tasks': sorted(excluded_tasks),
    }
    if eligible == 0:
        m['notes'].append('no task entered READY inside the window')
    return finish(m, sample, eligible, len(missing_tasks), len(excluded_tasks))


def metric_first_check(grouped, collection):
    m = blank_metric('M2', 'first_check_pass_rate', 'ratio')
    passed = failed = 0
    missing_tasks, by_kind = [], {}
    reviewed = {tid for tid, events in grouped.items()
                if any(e['to_status'] == 'MASTER_REVIEW' for e in events)}
    records = any(e['action'] == CHECK_RECORDED
                  for events in grouped.values() for e in events)
    if not (collection['checks'] or records):
        return mark_unknown(m, 'check results are not recorded by this source',
                            eligible=len(reviewed))
    for task_id, events in grouped.items():
        usable = [e for e in events if e['action'] == CHECK_RECORDED
                  and str(e['data'].get('result', '')).upper() in PASS_FAIL]
        if usable:
            first = usable[0]
            result = str(first['data'].get('result')).upper()
            kind = str(first['data'].get('kind') or 'unspecified')
            bucket = by_kind.setdefault(kind, {'PASS': 0, 'FAIL': 0})
            bucket[result] += 1
            if result == 'PASS':
                passed += 1
            else:
                failed += 1
        elif task_id in reviewed:
            missing_tasks.append(task_id)
    sample = passed + failed
    eligible = sample + len(missing_tasks)
    m['value'] = r4(passed / sample) if sample else None
    m['detail'] = {
        'passed': passed, 'failed': failed, 'by_kind': by_kind,
        'missing_tasks': sorted(missing_tasks),
    }
    if eligible == 0:
        m['notes'].append('no task reached MASTER_REVIEW or recorded a check in the window')
    elif missing_tasks:
        m['notes'].append('tasks with no first-check record are counted as missing, not as passing')
    return finish(m, sample, eligible, len(missing_tasks))


def metric_review_rounds(grouped, retry_cap):
    m = blank_metric('M3', 'review_fix_rounds', 'rounds')
    rounds, missing_tasks = [], []
    reviewed = 0
    for task_id, events in grouped.items():
        entries = [e for e in events if e['to_status'] == 'MASTER_REVIEW']
        if not entries:
            continue
        reviewed += 1
        returns = sum(1 for e in events
                      if e['action'] == STATUS_CHANGED
                      and e['from_status'] == 'MASTER_REVIEW'
                      and e['to_status'] in REWORK_TARGETS)
        invalidated = sum(1 for e in events if e['action'] == APPROVAL_INVALIDATED)
        exits = any(e['action'] == STATUS_CHANGED
                    and e['from_status'] == 'MASTER_REVIEW'
                    and e['to_status'] != 'MASTER_REVIEW' for e in events)
        exits = exits or any(e['to_status'] in ('APPROVED', 'CANCELED', 'FAILED')
                             for e in events)
        if not exits:
            missing_tasks.append(task_id)
            continue
        rounds.append(returns + invalidated)
    sample = len(rounds)
    m['value'] = r4(median(rounds)) if rounds else None
    m['detail'] = {
        'mean_rounds': r4(sum(rounds) / sample) if sample else None,
        'median_rounds': m['value'],
        'max_rounds': max(rounds) if rounds else None,
        'at_or_above_cap': sum(1 for r in rounds if r >= retry_cap),
        'retry_cap': retry_cap,
        'missing_tasks': sorted(missing_tasks),
    }
    if reviewed == 0:
        m['notes'].append('no task entered MASTER_REVIEW inside the window')
    elif missing_tasks:
        m['notes'].append('a review with no observed outcome is missing, not zero rounds')
    return finish(m, sample, reviewed, len(missing_tasks))


def metric_conflicts(grouped, events, collection):
    m = blank_metric('M4', 'path_resource_conflicts', 'count')
    dispatched = {tid for tid, evs in grouped.items()
                  if any(e['action'] == TASK_DISPATCHED for e in evs)}
    counts = {'path': 0, 'resource': 0, 'other': 0}
    for event in events:
        if event['action'] != CONFLICT_RECORDED:
            continue
        kind = str(event['data'].get('kind') or 'other').lower()
        counts[kind if kind in ('path', 'resource') else 'other'] += 1
    supported = {
        'path': collection['path_conflicts'] or counts['path'] > 0,
        'resource': collection['resource_conflicts'] or counts['resource'] > 0,
    }
    total = counts['path'] + counts['resource']
    m['detail'] = {
        'path': counts['path'], 'resource': counts['resource'], 'other': counts['other'],
        'dispatched_tasks': len(dispatched),
        'per_dispatched_task': r4(total / len(dispatched)) if dispatched else None,
        'path_supported': supported['path'], 'resource_supported': supported['resource'],
    }
    # M4 keeps TASK_DISPATCHED as its denominator (owner decision 2026-10-27):
    # a refused conflict can involve a task that was never dispatched, so the
    # figure is a count over dispatched tasks, not a comparable rate.
    m['notes'].append('per_dispatched_task is not a comparable rate: a refused conflict can '
                      'involve a task that was never dispatched, so it can exceed 1')
    if not all(supported.values()):
        missing_kinds = [k for k, ok in supported.items() if not ok]
        return mark_unknown(
            m, f'conflict kinds not collected by this source: {", ".join(missing_kinds)}',
            eligible=len(dispatched))
    m['value'] = total
    if len(dispatched) == 0:
        m['notes'].append('no task was dispatched inside the window')
        return finish(m, 0, 0, 0)
    return finish(m, len(dispatched), len(dispatched), 0)


def metric_blocked_hours(grouped):
    m = blank_metric('M5', 'blocked_hours', 'hours')
    totals, open_blocks, excluded = {}, 0, 0
    for task_id, events in grouped.items():
        started = None
        for event in events:
            if event['action'] != STATUS_CHANGED:
                continue
            if event['to_status'] == 'BLOCKED':
                started = event['at']
            elif event['from_status'] == 'BLOCKED' and started is not None:
                value = hours(event['at'] - started)
                if value < 0:
                    excluded += 1
                else:
                    totals[task_id] = totals.get(task_id, 0.0) + value
                started = None
        if started is not None:
            open_blocks += 1
    values = list(totals.values())
    sample = len(values)
    m['value'] = r4(median(values)) if values else None
    m['detail'] = {
        'total_hours': r4(sum(values)),
        'mean_hours': r4(sum(values) / sample) if sample else None,
        'median_hours': m['value'],
        'max_hours': r4(max(values)) if values else None,
        'open_blocks': open_blocks,
    }
    if open_blocks:
        m['notes'].append('a block still open at the window end is missing; its duration is not extrapolated')
    return finish(m, sample, sample + open_blocks, open_blocks, excluded)


def metric_post_merge_regressions(grouped, collection, observation_days, window_end):
    m = blank_metric('M6', 'post_merge_regressions', 'ratio')
    merges = {}
    for task_id, events in grouped.items():
        merged = [e for e in events if e['to_status'] == 'MERGED']
        if merged:
            merges[task_id] = merged[0]['at']
    if not (collection['regressions'] or any(e['action'] == REGRESSION_REPORTED
                                             for e in sum(grouped.values(), []))):
        return mark_unknown(m, 'regression reports are not recorded by this source',
                            eligible=len(merges))
    cohort, observing = [], []
    for task_id, merged_at in merges.items():
        if merged_at + timedelta(days=observation_days) <= window_end:
            cohort.append(task_id)
        else:
            observing.append(task_id)
    regressed = []
    for task_id in cohort:
        merged_at = merges[task_id]
        if any(e['action'] == REGRESSION_REPORTED and e['at'] >= merged_at
               for e in grouped[task_id]):
            regressed.append(task_id)
    sample = len(cohort)
    m['value'] = r4(len(regressed) / sample) if sample else None
    m['detail'] = {
        'merged_tasks': len(merges), 'observation_days': observation_days,
        'regressed_tasks': sorted(regressed), 'still_observing': sorted(observing),
    }
    if observing:
        m['notes'].append('merges still inside the observation window are missing, not clean')
    return finish(m, sample, len(merges), len(observing))


def metric_acceptance(grouped, collection):
    m = blank_metric('M7', 'e2e_acceptance_pass_rate', 'ratio')
    records = {}
    for task_id, events in grouped.items():
        for event in events:
            if event['action'] != ACCEPTANCE_RECORDED:
                continue
            ac_id = str(event['data'].get('ac_id') or '').strip()
            status = str(event['data'].get('status') or '').strip().upper()
            if ac_id and status:
                records.setdefault(task_id, {})[ac_id] = status
    if not (collection['acceptances'] or records):
        verified = {tid for tid, evs in grouped.items()
                    if any(e['to_status'] == 'VERIFIED' for e in evs)}
        return mark_unknown(m, 'acceptance records are not collected by this source',
                            eligible=len(verified))
    passed, failed, passing_counts = [], [], {}
    for task_id, acs in records.items():
        statuses = list(acs.values())
        for status in statuses:
            passing_counts[status] = passing_counts.get(status, 0) + 1
        if 'PASS' in statuses and all(s in AC_PASSING for s in statuses):
            passed.append(task_id)
        else:
            failed.append(task_id)
    verified = {tid for tid, evs in grouped.items()
                if any(e['to_status'] == 'VERIFIED' for e in evs)}
    missing_tasks = sorted(verified - set(records))
    sample = len(records)
    m['value'] = r4(len(passed) / sample) if sample else None
    m['detail'] = {
        'passed': len(passed), 'failed': len(failed),
        'by_status': passing_counts, 'missing_tasks': missing_tasks,
    }
    if missing_tasks:
        m['notes'].append('a VERIFIED task without acceptance records is missing, not passing')
    return finish(m, sample, sample + len(missing_tasks), len(missing_tasks))


# --- report ------------------------------------------------------------------

def compute(events, collection, window, retry_cap=3, observation_days=14.0):
    grouped = by_task(events)
    metrics = [
        metric_ready_to_verified(grouped),
        metric_first_check(grouped, collection),
        metric_review_rounds(grouped, retry_cap),
        metric_conflicts(grouped, events, collection),
        metric_blocked_hours(grouped),
        metric_post_merge_regressions(grouped, collection, observation_days, window['end_dt']),
        metric_acceptance(grouped, collection),
    ]
    with_data = [m for m in metrics if m['status'] in ('ok', 'partial') and m['sample_size']]
    overall = r4(sum(m['coverage'] for m in metrics) / len(metrics)) if metrics else 0.0
    return {
        'schema_version': SCHEMA_VERSION,
        'generated_at': iso(datetime.now(timezone.utc)),
        'source': None,
        'window': {
            'start': iso(window['start_dt']), 'end': iso(window['end_dt']),
            'timezone': TIMEZONE, 'derived': window['derived'],
        },
        'dataset': None,
        'collection': {kind: 'supported' if collection[kind] else 'unsupported'
                       for kind in COLLECTION_KINDS},
        'metrics': metrics,
        'summary': {
            'metrics_total': len(metrics),
            'metrics_with_data': len(with_data),
            'overall_coverage': overall,
            'headline': 'no data' if not with_data else f'{len(with_data)}/{len(metrics)} metrics with data',
        },
        'limits': list(LIMITS),
    }


def resolve_window(events, since, until, now):
    derived = until is None
    if until:
        end_dt = parse_bound(until, '--until')
    else:
        # An open-ended window still has to contain every recorded event, even
        # when a clock-skewed timestamp is in the future.
        latest = max((e['at'] for e in events), default=None)
        end_dt = max(now, latest + timedelta(seconds=1)) if latest else now
    if since:
        start_dt = parse_bound(since, '--since')
    else:
        start_dt = min((e['at'] for e in events), default=None)
    return {'start_dt': start_dt, 'end_dt': end_dt, 'derived': derived}


def in_window(event, window):
    start_dt = window['start_dt']
    if start_dt is not None and event['at'] < start_dt:
        return False
    return event['at'] < window['end_dt']


def build_report(dataset, window, retry_cap, observation_days):
    events = [e for e in dataset['events'] if in_window(e, window)]
    report = compute(events, dataset['collection'], window, retry_cap, observation_days)
    tasks_in_window = len({e['task_id'] for e in events})
    report['source'] = {'kind': dataset['kind'], 'path': dataset['path'], 'label': dataset['label']}
    report['dataset'] = {
        'tasks': len(dataset['tasks']),
        'tasks_with_events': tasks_in_window,
        'events': len(events),
        'records_ignored': dataset['records_ignored'],
        'naive_timestamps': dataset['naive_timestamps'],
    }
    return report


def format_value(metric):
    if metric['value'] is None:
        return 'none'
    if metric['unit'] == 'ratio':
        return f'{metric["value"]:.4f}'
    if metric['unit'] == 'count':
        return str(int(metric['value']))
    return f'{metric["value"]:.4f}'


def render(report):
    window = report['window']
    dataset = report['dataset']
    start = window['start'] or '(no events)'
    lines = [
        'AgentOrch delivery metrics',
        f"source: {report['source']['kind']} {report['source']['label']} ({report['source']['path']})",
        f"window: {start} .. {window['end']} ({window['timezone']}"
        f"{', derived' if window['derived'] else ', explicit'})",
        f"dataset: {dataset['tasks']} tasks ({dataset['tasks_with_events']} with events), "
        f"{dataset['events']} events, {dataset['records_ignored']} ignored, "
        f"{dataset['naive_timestamps']} naive timestamps",
        '',
    ]
    for metric in report['metrics']:
        lines.append(f"{metric['id']} {metric['name']} ({metric['unit']})")
        if metric['status'] == 'unknown':
            line = (f"  unknown | sample {metric['sample_size']}/{metric['eligible']} eligible "
                    f"| coverage 0.0")
        elif metric['status'] == 'no_data':
            line = ('  no data | sample 0/0 eligible | coverage 0.0 | '
                    'nothing in this window was eligible')
        else:
            value = format_value(metric)
            line = (f"  value {value} | sample {metric['sample_size']}/"
                    f"{metric['eligible']} eligible | missing {metric['missing']} "
                    f"(rate {metric['missing_rate']:.4f}) | coverage {metric['coverage']:.4f}")
        lines.append(line)
        detail = {k: v for k, v in metric['detail'].items() if v not in (None, {}, [])}
        if detail:
            lines.append(f'  detail {json.dumps(detail, ensure_ascii=False, sort_keys=True)}')
        for note in metric['notes']:
            lines.append(f'  note {note}')
        lines.append('')
    summary = report['summary']
    lines.append(f"coverage: {summary['metrics_with_data']}/{summary['metrics_total']} metrics "
                 f"with data; overall {summary['overall_coverage']:.4f}")
    lines.append('scope: descriptive for this window and sample only; '
                 'not a ranking of agents, people, models or runtimes.')
    return '\n'.join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description='Read-only delivery metrics from task-library events and evidence references.')
    parser.add_argument('source', nargs='?',
                        help='task library (.sqlite3 / .db) or JSON event sample')
    parser.add_argument('--gold-sample', action='store_true',
                        help='use the fixed sample embedded in templates/metrics.md')
    parser.add_argument('--json', action='store_true', help='print the stable JSON report')
    parser.add_argument('--since', help='window start (ISO date or timestamp, UTC)')
    parser.add_argument('--until', help='window end, exclusive (ISO date or timestamp, UTC)')
    parser.add_argument('--now', help='reference "now" for the window end (default: system UTC)')
    parser.add_argument('--regression-window-days', type=float, default=14.0,
                        help='observation period before a merge can count (default 14)')
    parser.add_argument('--retry-cap', type=int, default=3,
                        help='review-fix cap reported in M3 detail (default 3)')
    args = parser.parse_args(argv)

    try:
        if args.gold_sample:
            dataset = load_gold_sample()
        elif args.source:
            lowered = args.source.lower()
            dataset = (load_sqlite(args.source) if lowered.endswith(('.sqlite3', '.db', '.sqlite'))
                       else load_fixture(args.source))
        else:
            parser.error('a source path or --gold-sample is required')
        now = parse_bound(args.now, '--now') if args.now else datetime.now(timezone.utc)
        window = resolve_window(dataset['events'], args.since, args.until, now)
    except SourceError as exc:
        print(f'ERROR {exc}', file=sys.stderr)
        return 2

    report = build_report(dataset, window, args.retry_cap, args.regression_window_days)
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print(render(report))
    return 0


if __name__ == '__main__':
    sys.exit(main())
