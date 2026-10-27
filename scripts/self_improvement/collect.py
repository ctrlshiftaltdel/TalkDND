#!/usr/bin/env python3
"""Read-only evidence collection through the ``gh`` CLI (#49).

Boundary rules (sources-and-trust.md §2–§3, §6):

* **read-only.** Every command is built here as ``gh api --method GET <path>``.
  Nothing in this module can create, edit, comment on or close anything in a
  source repository, and the command builder is the only place a command is
  assembled.
* **no source text bodies.** Only locatable URLs, titles, labels, states,
  timestamps and commit SHAs are kept. Review bodies, logs, prompts and diffs are
  deliberately not downloaded; a title that happens to contain a secret or a home
  path is redacted before it is stored.
* **honest failure.** 404 / 403 / 429 / no ``gh`` / network / bad JSON and
  timeouts become ``BLOCKED`` / ``UNKNOWN`` with a machine-readable reason. A
  scan that did not read its scope never reports ``OK`` and never says "no new
  evidence".
* **offline is not a scan.** ``offline=True`` yields ``NOT_RUN`` for every
  source; callers still get their existing records.

The runner is injected, so the whole module is testable without a network or a
GitHub token. Standard library only.
"""
from __future__ import annotations

import json
import re
import subprocess
from datetime import datetime, timezone

from . import sources as sources_module
from . import untrusted

PER_PAGE = 30
DEFAULT_MAX_PAGES = 3
COMMAND_TIMEOUT_SECONDS = 60

# Evidence links are rendered into Markdown and stored in lesson records, so a
# non-HTTP(S) or angle-bracket/backtick URL is treated as unusable rather than
# escaped-and-hoped: it would otherwise be able to break out of the link syntax.
EVIDENCE_URL_RE = re.compile(r'^https://[^\s<>`]+$|^http://[^\s<>`]+$')

# Field extraction per evidence kind. Endpoints are data, not per-source branches:
# the same table serves every registered repository.
ENDPOINTS = {
    'pull_request': 'repos/{repository}/pulls?state=all&sort=updated&direction=desc&per_page={per_page}&page={page}',
    'issue': 'repos/{repository}/issues?state=all&since={since}&per_page={per_page}&page={page}',
    'review': 'repos/{repository}/pulls/comments?since={since}&per_page={per_page}&page={page}',
    'commit': 'repos/{repository}/commits?since={since}&per_page={per_page}&page={page}',
    'ci_run': 'repos/{repository}/actions/runs?created=%3E%3D{since}&per_page={per_page}&page={page}',
}

REASON_NOT_FOUND = sources_module.BLOCKED_NOT_FOUND
REASON_FORBIDDEN = sources_module.BLOCKED_FORBIDDEN
REASON_RATE_LIMITED = sources_module.BLOCKED_RATE_LIMITED
REASON_NETWORK = sources_module.BLOCKED_NETWORK
REASON_TOOL_UNAVAILABLE = sources_module.BLOCKED_TOOL_UNAVAILABLE
REASON_MALFORMED = sources_module.UNKNOWN_MALFORMED
REASON_TIMEOUT = sources_module.UNKNOWN_TIMEOUT
REASON_UNEXPECTED = 'unexpected_command_failure'
REASON_OFFLINE = 'offline'
REASON_DISABLED = 'disabled'

BLOCKING_REASONS = (REASON_NOT_FOUND, REASON_FORBIDDEN, REASON_RATE_LIMITED,
                    REASON_NETWORK, REASON_TOOL_UNAVAILABLE)
UNKNOWN_REASONS = (REASON_MALFORMED, REASON_TIMEOUT, REASON_UNEXPECTED)

FILTER_KINDS = ('label', 'title')
DEFAULT_MAX_TITLE_FILTER_LENGTH = 200


class CollectionError(Exception):
    """A scan could not be attempted (bad filter configuration, bad fixture)."""


class CommandFailure(CollectionError):
    """One read command failed; carries the report-level status and reason."""

    def __init__(self, status, reason, detail=''):
        self.status = status
        self.reason = reason
        self.detail = detail
        super().__init__(f'{reason}: {detail}' if detail else reason)


class CommandRunner:
    """Runs one argv list and returns ``(returncode, stdout, stderr)``."""

    def run(self, args):  # pragma: no cover - interface
        raise NotImplementedError


class SubprocessRunner(CommandRunner):
    """``gh`` via ``subprocess``, no shell, no inherited secrets on the command line."""

    def __init__(self, gh_binary='gh', timeout=COMMAND_TIMEOUT_SECONDS):
        self.gh_binary = gh_binary
        self.timeout = timeout

    def run(self, args):
        argv = list(args)
        if argv and argv[0] == 'gh':
            argv[0] = self.gh_binary
        try:
            proc = subprocess.run(argv, capture_output=True, text=True,
                                  encoding='utf-8', errors='replace',
                                  timeout=self.timeout, shell=False)
        except FileNotFoundError as exc:
            raise CommandFailure('BLOCKED', REASON_TOOL_UNAVAILABLE,
                                 f'{self.gh_binary} is not available: {exc}') from exc
        except subprocess.TimeoutExpired as exc:
            raise CommandFailure('UNKNOWN', REASON_TIMEOUT, str(exc)) from exc
        return proc.returncode, proc.stdout, proc.stderr


def build_command(repository, kind, since, page=1):
    """The only command builder: always ``gh api --method GET``."""
    if kind not in ENDPOINTS:
        raise CollectionError(f'unsupported evidence kind {kind!r}')
    path = ENDPOINTS[kind].format(repository=repository, since=since, per_page=PER_PAGE,
                                  page=page)
    return ['gh', 'api', '--method', 'GET', path]


def classify_failure(returncode, stderr):
    """Map one failed command to ``(status, reason)``.

    Order matters: a 403 that mentions rate limits is still a permission answer,
    and a transport failure must not be reported as "no evidence".
    """
    text = (stderr or '').lower()
    if 'timeout' in text or 'timed out' in text:
        return 'UNKNOWN', REASON_TIMEOUT
    if re.search(r'\b404\b', text) or 'not found' in text:
        return 'BLOCKED', REASON_NOT_FOUND
    if re.search(r'\b403\b', text) or 'forbidden' in text or 'resource not accessible' in text:
        return 'BLOCKED', REASON_FORBIDDEN
    if re.search(r'\b429\b', text) or 'rate limit' in text or 'secondary rate' in text:
        return 'BLOCKED', REASON_RATE_LIMITED
    if any(token in text for token in ('could not resolve', 'connection', 'network',
                                       'proxy', 'no such host', 'nonetwork')):
        return 'BLOCKED', REASON_NETWORK
    if returncode == 127:
        return 'BLOCKED', REASON_TOOL_UNAVAILABLE
    return 'UNKNOWN', REASON_UNEXPECTED


def parse_filters(specs, field):
    """Compile ``kind:value`` filter strings; unknown kinds are configuration errors."""
    compiled = []
    for spec in specs or []:
        if not isinstance(spec, str) or ':' not in spec:
            raise CollectionError(f'{field} entry {spec!r} must look like "<kind>:<value>"')
        kind, value = spec.split(':', 1)
        kind = kind.strip().lower()
        value = value.strip()
        if kind not in FILTER_KINDS:
            raise CollectionError(f'{field} entry {spec!r} uses unknown filter kind {kind!r}; '
                                  f'supported: {", ".join(FILTER_KINDS)}')
        if not value:
            raise CollectionError(f'{field} entry {spec!r} has an empty value')
        if len(value) > DEFAULT_MAX_TITLE_FILTER_LENGTH:
            raise CollectionError(f'{field} entry {spec!r} is longer than '
                                  f'{DEFAULT_MAX_TITLE_FILTER_LENGTH} characters')
        compiled.append((kind, value.casefold()))
    return compiled


def _matches(item, filters):
    for kind, value in filters:
        if kind == 'label':
            if value in [str(label).casefold() for label in item.get('labels') or []]:
                return True
        elif kind == 'title':
            if value in str(item.get('title') or '').casefold():
                return True
    return False


class ScanResult:
    def __init__(self, source, status, *, blocked_reason=None, missing=(), evidence=(),
                 excluded_count=0, inclusion_misses=0, suspicious=(), started_at=None,
                 finished_at=None, window=None, truncated=(), notes=()):
        self.source = source
        self.status = status
        self.blocked_reason = blocked_reason
        self.missing = list(missing)
        self.evidence = list(evidence)
        self.excluded_count = excluded_count
        self.inclusion_misses = inclusion_misses
        self.suspicious = list(suspicious)
        self.started_at = started_at
        self.finished_at = finished_at
        self.window = window
        self.truncated = list(truncated)
        self.notes = list(notes)

    @property
    def evidence_count(self):
        return len(self.evidence)

    def watermark(self):
        """Newest commit SHA the scan actually read, or ``None``.

        GitHub returns commits newest-first, so the first commit row is the
        watermark; if commits were not collected, the first evidence row that
        carries a SHA stands in. A SHA is never invented.
        """
        for item in self.evidence:
            if item.get('evidence_kind') == 'commit' and item.get('source_commit'):
                return item['source_commit']
        for item in self.evidence:
            if item.get('source_commit'):
                return item['source_commit']
        return None

    def report(self):
        return {
            'source_id': self.source.get('source_id'),
            'repository': self.source.get('repository'),
            'enabled': self.source.get('enabled'),
            'scan_status': self.status,
            'blocked_reason': self.blocked_reason,
            'scan_started_at': self.started_at,
            'scan_finished_at': self.finished_at,
            'window': dict(self.window or {}),
            'cursor_before': self.source.get('scan_cursor'),
            'cursor_after': None,  # filled by the pipeline
            'evidence_count': self.evidence_count,
            'excluded_count': self.excluded_count,
            'inclusion_misses': self.inclusion_misses,
            'missing_evidence': list(self.missing),
            'truncated': list(self.truncated),
            'suspicious_instruction_count': len(self.suspicious),
            'suspicious_evidence_urls': list(self.suspicious),
            'evidence_scope': list(self.source.get('evidence_scope') or []),
            'inclusion_filters': list(self.source.get('inclusion_filters') or []),
            'exclusion_filters': list(self.source.get('exclusion_filters') or []),
            'evidence': list(self.evidence),
            'notes': list(self.notes),
        }


def _list_payload(stdout):
    try:
        payload = json.loads(stdout or 'null')
    except json.JSONDecodeError as exc:
        raise CommandFailure('UNKNOWN', REASON_MALFORMED, str(exc)) from exc
    # `gh api` returns the endpoint body; review comments and runs are wrapped.
    if isinstance(payload, dict):
        if 'workflow_runs' in payload:
            return payload['workflow_runs']
        if isinstance(payload.get('items'), list):
            return payload['items']
    if not isinstance(payload, list):
        raise CommandFailure('UNKNOWN', REASON_MALFORMED, 'expected a JSON array')
    return payload


def _iso(value):
    if not isinstance(value, str) or not value:
        return None
    text = value.strip()
    if text.endswith('Z'):
        text = text[:-1] + '+00:00'
    try:
        moment = datetime.fromisoformat(text)
    except ValueError:
        return None
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return sources_module.format_timestamp(moment)


def _labels(item):
    labels = []
    for label in item.get('labels') or []:
        name = label.get('name') if isinstance(label, dict) else label
        if isinstance(name, str) and name:
            labels.append(name)
    return sorted(labels)


def _evidence_item(source, kind, raw, captured_at, today_since):
    repository = source.get('repository')
    source_id = source.get('source_id')
    item = {
        'source_id': source_id,
        'source_repository': repository,
        'evidence_kind': kind,
        'url': None,
        'title': '',
        'observed_at': None,
        'captured_at': captured_at,
        'source_commit': None,
        'state': None,
        'merged': None,
        'labels': [],
        'evidence_gaps': [],
        'instruction_flags': [],
        'suggested_evidence_level': 'E0_CLAIM',
        'redactions': 0,
    }
    gaps = item['evidence_gaps']
    if kind == 'pull_request':
        item['url'] = raw.get('html_url')
        item['title'] = raw.get('title') or ''
        item['observed_at'] = _iso(raw.get('updated_at') or raw.get('created_at'))
        item['source_commit'] = (raw.get('head') or {}).get('sha')
        item['state'] = raw.get('state')
        item['merged'] = bool(raw.get('merged_at'))
        item['labels'] = _labels(raw)
        if item['state'] == 'closed' and not item['merged']:
            gaps.append('closed_without_merge_evidence')
        item['suggested_evidence_level'] = 'E1_SOURCE_ARTIFACT'
    elif kind == 'issue':
        if 'pull_request' in raw:
            return None  # the issues endpoint also returns PRs
        item['url'] = raw.get('html_url')
        item['title'] = raw.get('title') or ''
        item['observed_at'] = _iso(raw.get('updated_at') or raw.get('created_at'))
        item['state'] = raw.get('state')
        item['labels'] = _labels(raw)
        item['suggested_evidence_level'] = 'E0_CLAIM'
    elif kind == 'review':
        pull_number = raw.get('pull_request_url', '').rstrip('/').split('/')[-1]
        item['url'] = raw.get('html_url')
        item['title'] = f'review comment on pull request #{pull_number}' if pull_number else 'review comment'
        item['observed_at'] = _iso(raw.get('updated_at') or raw.get('created_at'))
        item['source_commit'] = raw.get('commit_id')
        item['state'] = 'review_comment'
        item['suggested_evidence_level'] = 'E1_SOURCE_ARTIFACT'
    elif kind == 'commit':
        commit = raw.get('commit') or {}
        author = commit.get('author') or {}
        item['url'] = raw.get('html_url')
        message = str(commit.get('message') or '').splitlines()
        item['title'] = message[0].strip() if message else ''
        item['observed_at'] = _iso(author.get('date'))
        item['source_commit'] = raw.get('sha')
        item['state'] = 'commit'
        item['suggested_evidence_level'] = 'E1_SOURCE_ARTIFACT'
    elif kind == 'ci_run':
        item['url'] = raw.get('html_url')
        name = raw.get('name') or 'workflow'
        item['title'] = f'{name}: {raw.get("status") or "unknown"}/{raw.get("conclusion") or "unknown"}'
        item['observed_at'] = _iso(raw.get('created_at') or raw.get('updated_at'))
        item['source_commit'] = raw.get('head_sha')
        item['state'] = raw.get('conclusion') or raw.get('status')
        item['suggested_evidence_level'] = 'E1_SOURCE_ARTIFACT'
    if not item['url'] or not EVIDENCE_URL_RE.match(str(item['url'])):
        if item['url']:
            gaps.append('invalid_evidence_url')
        item['url'] = None
        item['suggested_evidence_level'] = 'E0_CLAIM'
    clean_title, redactions = untrusted.redact(item['title'])
    item['title'] = clean_title
    item['redactions'] = redactions
    item['instruction_flags'] = untrusted.detect_instruction_patterns(item['title'])
    if item['observed_at'] and item['observed_at'] < today_since:
        return None
    return item


class Collector:
    """Scans one registered source and returns an honest :class:`ScanResult`."""

    def __init__(self, runner, *, per_page=PER_PAGE, max_pages=DEFAULT_MAX_PAGES):
        self.runner = runner
        self.per_page = per_page
        self.max_pages = max_pages

    def scan_source(self, source, *, now, started_at=None, offline=False):
        started_at = started_at or now
        window = sources_module.scan_window(source, _parse(now))
        if offline:
            return ScanResult(source, 'NOT_RUN', blocked_reason=REASON_OFFLINE,
                              started_at=started_at, finished_at=now, window=window,
                              notes=['offline mode: no collection attempted; existing records only'])
        if not source.get('enabled'):
            return ScanResult(source, 'NOT_RUN', blocked_reason=REASON_DISABLED,
                              started_at=started_at, finished_at=now, window=window,
                              notes=['source is disabled in the registry; not scanned'])
        inclusion = parse_filters(source.get('inclusion_filters'), 'inclusion_filters')
        exclusion = parse_filters(source.get('exclusion_filters'), 'exclusion_filters')
        evidence = []
        missing = []
        truncated = []
        excluded_count = 0
        inclusion_misses = 0
        suspicious = []
        failures = []
        for kind in [k for k in sources_module.EVIDENCE_KINDS
                     if k in (source.get('evidence_scope') or [])]:
            try:
                items, page_full = self._collect_kind(source, kind, window['since'])
            except CommandFailure as exc:
                failures.append(exc.reason)
                missing.append({'evidence_kind': kind, 'reason': exc.reason})
                continue
            for raw in items:
                item = _evidence_item(source, kind, raw, now, window['since'])
                if item is None:
                    continue
                if exclusion and _matches(item, exclusion):
                    excluded_count += 1
                    continue
                if inclusion and not _matches(item, inclusion):
                    inclusion_misses += 1
                    continue
                if item['instruction_flags']:
                    suspicious.append(item['url'])
                evidence.append(item)
            if page_full:
                truncated.append(kind)
        # CI evidence gap per merged PR (lesson-lifecycle.md §9).
        ci_shas = {item['source_commit'] for item in evidence
                   if item['evidence_kind'] == 'ci_run' and item.get('source_commit')}
        ci_in_scope = 'ci_run' in (source.get('evidence_scope') or [])
        for item in evidence:
            if item['evidence_kind'] != 'pull_request' or not item.get('merged'):
                continue
            if not ci_in_scope:
                item['evidence_gaps'].append('ci_not_in_scope')
            elif item.get('source_commit') not in ci_shas:
                item['evidence_gaps'].append('missing_ci_evidence')

        if not evidence and failures:
            reason = failures[0]
            status = 'BLOCKED' if reason in BLOCKING_REASONS else 'UNKNOWN'
            if any(r in UNKNOWN_REASONS for r in failures) and all(r in UNKNOWN_REASONS for r in failures):
                status = 'UNKNOWN'
            return ScanResult(source, status, blocked_reason=reason, missing=missing,
                              excluded_count=excluded_count, inclusion_misses=inclusion_misses,
                              suspicious=suspicious, started_at=started_at, finished_at=now,
                              window=window, truncated=truncated,
                              notes=[f'{len(failures)} evidence kind(s) could not be read'])
        if failures or truncated:
            reason = failures[0] if failures else 'page_limit'
            return ScanResult(source, 'PARTIAL', blocked_reason=None, missing=missing,
                              evidence=evidence, excluded_count=excluded_count,
                              inclusion_misses=inclusion_misses, suspicious=suspicious,
                              started_at=started_at, finished_at=now, window=window,
                              truncated=truncated,
                              notes=[f'partial scan: {reason}; the cursor is not advanced'])
        watermark = ScanResult(source, 'OK', evidence=evidence).watermark()
        if watermark is None:
            if 'commit' not in (source.get('evidence_scope') or []):
                missing.append({'evidence_kind': 'commit',
                                'reason': 'watermark_not_in_scope'})
                return ScanResult(source, 'PARTIAL', blocked_reason=None, missing=missing,
                                  excluded_count=excluded_count,
                                  inclusion_misses=inclusion_misses, suspicious=suspicious,
                                  started_at=started_at, finished_at=now, window=window,
                                  truncated=truncated,
                                  notes=['no commit evidence in scope: the OK watermark '
                                         'cannot be established'])
            return ScanResult(source, 'UNKNOWN', blocked_reason='no_commit_watermark',
                              missing=missing, excluded_count=excluded_count,
                              inclusion_misses=inclusion_misses, suspicious=suspicious,
                              started_at=started_at, finished_at=now, window=window,
                              truncated=truncated,
                              notes=['the source returned no commit SHA; the watermark is unknown'])
        return ScanResult(source, 'OK', evidence=evidence, excluded_count=excluded_count,
                          inclusion_misses=inclusion_misses, suspicious=suspicious,
                          started_at=started_at, finished_at=now, window=window,
                          truncated=truncated)

    def _collect_kind(self, source, kind, since):
        """Read up to ``max_pages`` pages.

        Returns ``(items, truncated)``; ``truncated`` is true only when the last
        page read was completely full at the page cap, i.e. more data probably
        exists. A page that happens to fill exactly is not called truncated when
        the next page was read and came back short.
        """
        repository = source.get('repository')
        items = []
        truncated = False
        for page in range(1, self.max_pages + 1):
            command = build_command(repository, kind, since, page=page)
            try:
                returncode, stdout, stderr = self.runner.run(command)
            except CommandFailure:
                raise
            except FileNotFoundError as exc:
                raise CommandFailure('BLOCKED', REASON_TOOL_UNAVAILABLE, str(exc)) from exc
            except OSError as exc:
                raise CommandFailure('BLOCKED', REASON_NETWORK, str(exc)) from exc
            if returncode != 0:
                status, reason = classify_failure(returncode, stderr)
                raise CommandFailure(status, reason, (stderr or 'no stderr').strip())
            page_items = _list_payload(stdout)
            items.extend(page_items)
            truncated = len(page_items) >= self.per_page
            if not truncated:
                break
        return items, truncated


def _parse(value):
    if isinstance(value, datetime):
        return value
    return sources_module.parse_timestamp(value)


def candidate_evidence(result):
    """Evidence rows that can be distilled into lessons (never mutated in place)."""
    return [item for item in result.evidence if item.get('url')]
