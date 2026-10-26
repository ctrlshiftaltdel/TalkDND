#!/usr/bin/env python3
"""One scan = registry → evidence → lessons → report (#49).

The pipeline is deliberately thin; every rule lives in its module:

``sources``  which sources are enabled, where the next window starts, how a
             cursor may move (only ``OK`` advances it, regressions are refused).
``collect``  read-only collection and honest ``BLOCKED`` / ``UNKNOWN`` /
             ``PARTIAL`` results.
``lessons``  fingerprints, dedup, conflicts, state transitions and rendering.

What the pipeline deliberately does **not** do: invent an analysis from a title,
create an Issue, adopt a lesson, or write to a source repository. A lesson only
exists once an explicit analysis is supplied (``analyses`` maps an evidence URL
to one), which is what keeps repeated scans idempotent instead of noisy.
"""
from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path

from . import collect, lessons, sources

class ScanOutcome:
    def __init__(self, report, registry, ingestions=()):
        self.report = report
        self.registry = registry
        self.ingestions = list(ingestions)

    def as_dict(self):
        payload = dict(self.report)
        payload['ingestions'] = [result.as_dict() for result in self.ingestions]
        return payload


def run_scan(registry, store, runner, *, now=None, offline=False, max_pages=None,
             analyses=None):
    """Scan every registered source once and return a report + the new registry.

    ``registry`` and ``store`` are not mutated; callers persist the returned
    registry only if they intend to move the cursor. ``analyses`` is an optional
    ``{evidence_url: analysis}`` map for explicit lesson drafting.
    """
    moment = _moment(now)
    timestamp = sources.format_timestamp(moment)
    collector = collect.Collector(runner) if max_pages is None else \
        collect.Collector(runner, max_pages=max_pages)
    updated_registry = registry
    source_reports = []
    decisions = []
    for source in registry.sources:
        result = collector.scan_source(source, now=timestamp, started_at=timestamp,
                                      offline=offline)
        cursor_before = source.get('scan_cursor')
        cursor_after = cursor_before
        status = result.status
        blocked_reason = result.blocked_reason
        if result.status != 'NOT_RUN':
            try:
                cursor_after = sources.advance_cursor(
                    cursor_before, status=result.status, started_at=result.started_at,
                    finished_at=result.finished_at, scanned_commit=result.watermark(),
                    blocked_reason=result.blocked_reason)
                updated_registry = sources.update_cursor(updated_registry,
                                                        source.get('source_id'), cursor_after)
            except sources.CursorRegression as exc:
                # sources-and-trust.md §4 rule 2: stop incremental scanning, keep the
                # old watermark, re-backfill by lookback and let a human confirm.
                status = 'UNKNOWN'
                blocked_reason = sources.UNKNOWN_CURSOR_REGRESSION
                cursor_after = cursor_before
                result.notes.append(f'cursor regression: {exc}')
                decisions.append(_decision('cursor_regression', source.get('source_id'),
                                           'scan start moved backwards; the cursor was not '
                                           'advanced and the next scan must re-backfill the '
                                           'lookback window'))
            except ValueError as exc:
                status = 'UNKNOWN'
                blocked_reason = 'no_commit_watermark'
                cursor_after = cursor_before
                result.notes.append(f'cursor not advanced: {exc}')
        if (status in ('BLOCKED', 'UNKNOWN')
                and source.get('access_mode') == 'authorized_private'):
            decisions.append(_decision('authorized_private_unavailable',
                                       source.get('source_id'),
                                       'private source could not be read; HD-01 requires an '
                                       'owner-approved access mechanism (never a personal token)'))
        report = result.report()
        report['scan_status'] = status
        report['blocked_reason'] = blocked_reason
        report['cursor_after'] = cursor_after
        source_reports.append(report)

    ingestions = _ingest(store, source_reports, analyses or {})
    for lesson in store.all():
        for conflict in lessons.open_conflicts(lesson):
            decisions.append(_decision('open_conflict', lesson['lesson_id'],
                                       f'OPEN conflict with {conflict.get("lesson_id")} blocks '
                                       'GENERALIZED / IMPLEMENTED; HD-04 decision required'))
    report = lessons.build_report(
        registry_summary=sources.summary(registry),
        source_reports=source_reports,
        lessons=store.all(),
        generated_at=timestamp,
        decisions_required=decisions,
        scan_mode='offline' if offline else 'live',
    )
    return ScanOutcome(report, updated_registry, ingestions)


def _ingest(store, source_reports, analyses):
    results = []
    for source_report in source_reports:
        for item in source_report.get('evidence') or []:
            analysis = analyses.get(item.get('url'))
            if not analysis:
                continue
            candidate = lessons.candidate_from_evidence(item)
            candidate['source_commit'] = item.get('source_commit')
            lesson = lessons.draft_lesson(
                candidate, analysis,
                existing_ids={existing['lesson_id'] for existing in store.all()},
            )
            results.append(store.ingest(lesson))
    return results


def _decision(kind, source_id, reason):
    return {'kind': kind, 'source_id': source_id, 'reason': reason,
            'decision': 'NEEDS_HUMAN_DECISION'}


def _moment(value):
    if value is None:
        return datetime.now(timezone.utc)
    if isinstance(value, datetime):
        return value.astimezone(timezone.utc)
    return sources.parse_timestamp(value)


def write_report(report, path):
    """Write the JSON report; never rewrites source content, only references it."""
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n',
                      encoding='utf-8')
    return target


def write_markdown(report, path):
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(lessons.render_markdown(report), encoding='utf-8')
    return target
