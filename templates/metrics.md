# Delivery Metrics

> Filled in once per project, before anyone quotes a number. It defines **what is measured and from which recorded events**, so two people reading the same task library get the same figure.
>
> **What these numbers are not:** they are not a ranking of agents, people, models or runtimes. A metric calculated on four tasks, over two weeks, with a third of its events missing says nothing about who is "better". Any comparison must carry the sample size, the window and the missing rate that produced it — quoting a rate without them is a defect, not a summary.
>
> **Single source:** the seven metric definitions below are the authority. `scripts/report_metrics.py` implements them and prints an `id` for every row it emits so a report can always be traced back to this file. Changing a definition here without changing the script (or the reverse) is a defect.

## When to use

- When a delivery bottleneck has to be explained, and the answer must be a number with a sample size rather than an impression.
- When a stage is being closed and someone asks "how are we doing" — the report is the evidence, the sample size and missing rate are the caveats.
- When a project adopts this kit and has to state **which** metrics it can and cannot compute; the "current data sources" table in `docs/metrics.md` (repository only, not shipped) says what is still `unknown`.
- Before quoting any earlier figure: if the window, the sample or the definitions changed, the old number is not comparable and must be recomputed.


## Reading rules

1. **Sample size first.** Never print a metric without `sample_size` and the window it covers. A value computed on `n = 2` is an anecdote and must be labelled as one.
2. **Missing is not zero.** A task without the start event, without the end event, or without the record the metric needs is counted as `missing`, not as "fast", "passing" or "clean". Missing rate is `missing / eligible`.
3. **Coverage is part of the result.** `coverage = sample_size / eligible`. A metric with coverage below the project's threshold is reported, but not used to conclude anything.
4. **Not collected is `unknown`.** If the data source does not record the events a metric needs, the metric is `unknown` — never `0`. Zero conflicts and "we do not record conflicts" are different statements.
5. **No context-free ranking.** The report may compare two windows of the same project, or two stages of the same project. It must not turn these numbers into a leaderboard of agents, sessions or people.
6. **No reconstruction from chat.** Numbers come from recorded events and evidence references only. If the event is not there, the metric is missing; a chat log is not a data source.
7. **Nothing private.** The report reads event type, status, timestamp, task id and evidence reference. It must never read prompt text, message bodies, tokens or credentials.

## Common definitions

- **Timezone:** every timestamp is normalised to UTC. A timestamp with no offset is treated as UTC and counted in `dataset.naive_timestamps` (visible in the report, because a wrong assumption shifts everything).
- **Window:** half-open `[start, end)` in UTC. Defaults to the full span of the recorded events when `--since` / `--until` are omitted, in which case the report marks the window `derived`.
- **Task eligibility:** a task is eligible for a metric when it has the events the metric keys on (defined per metric). Eligible = sample + missing.
- **Task set:** a task is considered when it has at least one event inside the window; events outside the window never contribute, even if they are the only start or end event.
- **Duration:** wall-clock hours between two events, `(end - start) / 3600`, not working hours. A negative duration (clock skew) is excluded and counted in `excluded`.
- **Ratio:** numerator / sample, reported with four decimals; `0.6667` means 4 of 6.
- **Headline value:** for durations and round counts `value` is the **median**; `detail` carries mean / p90 / max / total. For ratios `value` is the rate. The unit is always printed next to the value.
- **Regression observation:** a merge is only counted after `--regression-window-days` (default `14`) have passed inside the window; a merge still inside its observation period is `missing`, not "clean".
- **Retry cap:** 3 review-fix rounds, read from `.orchestration/config.json` `retry_cap`. Rounds at or above the cap are counted separately in `detail.at_or_above_cap`.

## Metric definitions

Events come from the task library (`local`: `events` table; `github`: the same event names recorded in the Issue / PR timeline). Only the events named here are read.

| ID | Metric | Unit | Start event | End event | Timezone | Exclusions | Missing handling | Sample / eligibility |
|----|--------|------|-------------|-----------|------------|------------------|----------------------|
| **M1** | READY → VERIFIED elapsed | hours | Last `STATUS_CHANGED` into `READY` that precedes the end event | First `STATUS_CHANGED` into `VERIFIED` after the start event | UTC (earliest `READY`, latest `VERIFIED` boundary normalised to UTC) | Negative durations (clock skew) → `excluded`; events outside the window | Task with `READY` but no later `VERIFIED` (canceled, failed, still open) → `missing`; the elapsed time is **not** estimated | Eligible: tasks with a `READY` transition in the window. Sample: those also reaching `VERIFIED`. Detail: mean / p90 / max. |
| **M2** | First check pass rate | ratio | — | — | UTC (check timestamp compared with the window) | Checks whose `result` is neither `PASS` nor `FAIL` are unusable → `missing` | Task that reached `MASTER_REVIEW` but has no `CHECK_RECORDED` → `missing` (never assumed to pass) | Eligible: tasks with a first check result, plus reviewed tasks without one. Sample: tasks with a usable first `CHECK_RECORDED` in the window (first by timestamp, not by `attempt`). Detail: PASS/FAIL counts and a breakdown by `kind` (`ci` / `local` / other). |
| **M3** | Review-fix rounds | rounds | First `STATUS_CHANGED` into `MASTER_REVIEW` | Terminal review outcome (`APPROVED`, `CANCELED`, `FAILED`) | UTC (transition timestamps) | Nothing excluded by design; a review that never left `MASTER_REVIEW` is incomplete, not zero | Reviewed task with no observed exit from `MASTER_REVIEW` → `missing` (in flight) | Eligible: reviewed tasks. Sample: reviewed tasks with an observed exit. Round = `STATUS_CHANGED` from `MASTER_REVIEW` to `IN_PROGRESS` / `BLOCKED` / `FAILED`, plus each `APPROVAL_INVALIDATED`. Detail: mean / median / max / `at_or_above_cap`. |
| **M4** | Path / resource conflicts | count | — | — | UTC (conflict timestamp) | Records outside the window | A conflict kind the source does not record → metric status `partial` / `unknown` for that kind, **not** `0` | Eligible & sample: tasks dispatched (`TASK_DISPATCHED`) in the window. Count = `CONFLICT_RECORDED` events with `data.kind` = `path` or `resource`. Detail: per-kind count and per-task rate. |
| **M5** | Blocked time | hours | `STATUS_CHANGED` into `BLOCKED` | Next `STATUS_CHANGED` leaving `BLOCKED` | UTC (block start / end) | Negative intervals → `excluded` | Block still open at the window end → task counted in `missing` and listed in `detail.open_blocks`; the open interval is **not** extrapolated | Eligible: tasks with a closed block. Sample: same, per-task total. Detail: total / mean / max / open blocks. |
| **M6** | Post-merge regressions | ratio | `STATUS_CHANGED` into `MERGED` | `REGRESSION_REPORTED` carrying that task's `merge_sha`, or any `REGRESSION_REPORTED` after the merge | UTC (merge and regression timestamps; observation window counted in 24h days) | Regressions reported against a merge outside the window | Merge still inside the observation window (`--regression-window-days`) → `missing`; source that does not record regressions → `unknown` | Eligible: merges in the window. Sample: merges whose observation window has elapsed. Detail: merged count, observation days, regressed task ids. |
| **M7** | End-to-end acceptance pass rate | ratio | — | — | UTC (acceptance record timestamp) | — | Task reaching `VERIFIED` without any `ACCEPTANCE_RECORDED` → `missing`; source without acceptance records → `unknown` | Eligible: tasks with acceptance records plus verified tasks without them. Sample: tasks with records. Task passes only when every recorded AC is `PASS` / `NOT_APPLICABLE` **and** at least one is `PASS` (the G1 rule, not a new one). Detail: passed / failed / per-AC status counts. |

Event vocabulary read by the report (unknown actions are ignored and counted, never guessed):

| Event | Meaning | Fields read |
|-------|---------|-------------|
| `STATUS_CHANGED` | Task status transition | `from_status`, `to_status` |
| `TASK_DISPATCHED` | Locks acquired, task handed to an execution instance | — |
| `CHECK_RECORDED` | One validation / CI check result | `data.result` (`PASS` / `FAIL`), `data.kind`, `data.attempt` |
| `ACCEPTANCE_RECORDED` | One acceptance criterion result | `data.ac_id`, `data.status` |
| `CONFLICT_RECORDED` | A dispatch or lock conflict was refused | `data.kind` (`path` / `resource`), `data.resource_id` |
| `REGRESSION_REPORTED` | A defect traced back to a merged change | `data.merge_sha`, `data.severity` |
| `APPROVAL_INVALIDATED` | Head changed after `APPROVED`; the approval no longer holds | — |

## Missing data and coverage

Per-metric `status`:

| Status | Meaning | What the report prints |
|--------|---------|------------------------|
| `ok` | Every eligible task produced a value | value + sample + missing rate |
| `partial` | Some eligible tasks are missing records | value on the sample + explicit missing rate |
| `no_data` | No eligible task in the window | `no data` (never `0`) |
| `unknown` | The source does not record the events this metric needs | `unknown` + the missing collection item |

`collection` in the report states, per record kind, whether the source can produce it. The `local` task library currently records `resource` conflicts but **not** rejected path locks (a refused dispatch is rolled back and leaves no event) and does **not** record checks, acceptance rows or regressions. Those metrics therefore report `unknown` on a plain `local` library until a task records the event — the report never fills the gap by inference. `github` can supply all five from the PR timeline once wired; until then the same rule applies.

## Report contract

`python scripts/report_metrics.py <source> [--json]` prints a human summary; `--json` prints one object with these keys, always present, in this order:

| Key | Type | Content |
|-----|------|---------|
| `schema_version` | string | `1.0` — bumped when a field changes meaning |
| `generated_at` | string | ISO-8601 UTC |
| `source` | object | `kind` (`sqlite` / `fixture`), `path`, `label` |
| `window` | object | `start`, `end`, `timezone` (`UTC`), `derived` (bool) |
| `dataset` | object | `tasks`, `tasks_with_events`, `events`, `records_ignored`, `naive_timestamps` |
| `collection` | object | `checks`, `path_conflicts`, `resource_conflicts`, `regressions`, `acceptances` → `supported` / `unsupported` |
| `metrics` | array | Seven objects, always in M1–M7 order: `id`, `name`, `unit`, `status`, `value`, `sample_size`, `eligible`, `missing`, `missing_rate`, `coverage`, `excluded`, `detail`, `notes` |
| `summary` | object | `metrics_total`, `metrics_with_data`, `overall_coverage`, `headline` |
| `limits` | array | Fixed strings restating the coverage / no-ranking limits for anyone who reads the JSON alone |

`value` is `null` when `status` is `no_data` or `unknown`; `coverage` is `0.0`, never null. Exit code is `0` for a successful report (including `no data`) and `2` for an unreadable source — a report with no data is a result, not an error.

Input shape (the JSON event sample; a task library is read from the same event vocabulary):

| Level | Keys | Rules |
|-------|------|-------|
| top | `schema_version`, `kind`, `label`, `collects`, `tasks`, `events` | `collects` declares which record kinds the sample carries (`checks`, `path_conflicts`, `resource_conflicts`, `regressions`, `acceptances`); an omitted kind is treated as carried |
| `tasks[]` | `id`, `created_at`, `status` | Only `id` is used; the rest is context for a human reader |
| `events[]` | `task_id`, `at`, `action`, `from_status`, `to_status`, `data`, `ref` | `at` is ISO-8601 (naive means UTC and is counted); `data` carries only the fields listed in the event vocabulary; `ref` is an evidence locator, never message text |

A task library is read with the same field names (`events.action`, `from_status`, `to_status`, `at`, `data_json`), plus `resource_lock_events` rows of type `LOCK_DENIED_CONFLICT` for resource conflicts. The `note` column is never read.

## Gold sample

The block below is the fixed synthetic dataset the tests and the docs use. It contains a clean success, a first-check failure that was fixed, a cancellation, a failure, an open block, a task with no check record, a merge still inside its observation window, and a task that hit the retry cap. Nothing in it is read from a real project; nothing here proves a real project's numbers.

Reproduce it with `python scripts/report_metrics.py --gold-sample --since 2026-01-01 --until 2026-02-01`. Expected headline values (asserted in `tests/test_report_metrics.py`, so this paragraph cannot drift silently): M1 sample 5 / median 47.8333 h / missing 4; M2 `0.6667` (4 of 6, 1 missing); M3 max 4 rounds, `at_or_above_cap` 1; M4 count 2 (1 path, 1 resource) over 9 dispatched tasks; M5 total 9.0 h over 2 tasks, 1 open block; M6 `0.3333` (1 of 3 complete cohorts, 3 still observing); M7 `0.8000` (4 of 5, 1 missing).

```metrics-sample
{
  "schema_version": 1,
  "kind": "fixture",
  "label": "gold-sample-2026-01",
  "timezone": "UTC",
  "collects": {"checks": true, "path_conflicts": true, "resource_conflicts": true, "regressions": true, "acceptances": true},
  "tasks": [
    {"id": "T-1", "created_at": "2026-01-02T09:00:00Z", "status": "VERIFIED"},
    {"id": "T-2", "created_at": "2026-01-04T09:00:00Z", "status": "VERIFIED"},
    {"id": "T-3", "created_at": "2026-01-07T09:00:00Z", "status": "VERIFIED"},
    {"id": "T-4", "created_at": "2026-01-10T09:00:00Z", "status": "CANCELED"},
    {"id": "T-5", "created_at": "2026-01-11T09:00:00Z", "status": "FAILED"},
    {"id": "T-6", "created_at": "2026-01-12T09:00:00Z", "status": "BLOCKED"},
    {"id": "T-7", "created_at": "2026-01-20T09:00:00Z", "status": "VERIFIED"},
    {"id": "T-8", "created_at": "2026-01-30T09:00:00Z", "status": "MERGED"},
    {"id": "T-9", "created_at": "2026-01-22T09:00:00Z", "status": "VERIFIED"}
  ],
  "events": [
    {"task_id": "T-1", "at": "2026-01-02T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/1#1"},
    {"task_id": "T-1", "at": "2026-01-02T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/1#2"},
    {"task_id": "T-1", "at": "2026-01-02T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/1#3"},
    {"task_id": "T-1", "at": "2026-01-02T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/1#4"},
    {"task_id": "T-1", "at": "2026-01-02T10:00:00Z", "action": "CHECK_RECORDED", "data": {"result": "PASS", "kind": "ci", "attempt": 1}, "ref": "task/1#5"},
    {"task_id": "T-1", "at": "2026-01-02T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/1#6"},
    {"task_id": "T-1", "at": "2026-01-02T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/1#7"},
    {"task_id": "T-1", "at": "2026-01-02T11:30:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/1#8"},
    {"task_id": "T-1", "at": "2026-01-02T12:00:00Z", "action": "STATUS_CHANGED", "from_status": "APPROVED", "to_status": "MERGED", "ref": "task/1#9"},
    {"task_id": "T-1", "at": "2026-01-03T08:50:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-1", "status": "PASS"}, "ref": "task/1#10"},
    {"task_id": "T-1", "at": "2026-01-03T08:51:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-2", "status": "PASS"}, "ref": "task/1#11"},
    {"task_id": "T-1", "at": "2026-01-03T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "MERGED", "to_status": "VERIFIED", "ref": "task/1#12"},
    {"task_id": "T-2", "at": "2026-01-04T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/2#1"},
    {"task_id": "T-2", "at": "2026-01-04T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/2#2"},
    {"task_id": "T-2", "at": "2026-01-04T09:20:00Z", "action": "CONFLICT_RECORDED", "data": {"kind": "path", "resource_id": "src/core.py"}, "ref": "pr/24#conflict"},
    {"task_id": "T-2", "at": "2026-01-04T09:25:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/2#3"},
    {"task_id": "T-2", "at": "2026-01-04T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/2#4"},
    {"task_id": "T-2", "at": "2026-01-04T10:00:00Z", "action": "CHECK_RECORDED", "data": {"result": "FAIL", "kind": "ci", "attempt": 1}, "ref": "ci/1001"},
    {"task_id": "T-2", "at": "2026-01-04T10:10:00Z", "action": "CHECK_RECORDED", "data": {"result": "PASS", "kind": "local", "attempt": 2}, "ref": "task/2#6"},
    {"task_id": "T-2", "at": "2026-01-04T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/2#7"},
    {"task_id": "T-2", "at": "2026-01-04T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/2#8"},
    {"task_id": "T-2", "at": "2026-01-04T11:30:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "IN_PROGRESS", "ref": "task/2#9"},
    {"task_id": "T-2", "at": "2026-01-05T10:00:00Z", "action": "CHECK_RECORDED", "data": {"result": "PASS", "kind": "ci", "attempt": 2}, "ref": "ci/1002"},
    {"task_id": "T-2", "at": "2026-01-05T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/2#11"},
    {"task_id": "T-2", "at": "2026-01-05T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/2#12"},
    {"task_id": "T-2", "at": "2026-01-05T11:30:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/2#13"},
    {"task_id": "T-2", "at": "2026-01-05T12:00:00Z", "action": "STATUS_CHANGED", "from_status": "APPROVED", "to_status": "MERGED", "ref": "task/2#14"},
    {"task_id": "T-2", "at": "2026-01-06T08:55:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-1", "status": "PASS"}, "ref": "task/2#15"},
    {"task_id": "T-2", "at": "2026-01-06T08:56:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-2", "status": "PASS"}, "ref": "task/2#16"},
    {"task_id": "T-2", "at": "2026-01-06T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "MERGED", "to_status": "VERIFIED", "ref": "task/2#17"},
    {"task_id": "T-2", "at": "2026-01-09T10:00:00Z", "action": "REGRESSION_REPORTED", "data": {"severity": "high"}, "ref": "issue/31"},
    {"task_id": "T-3", "at": "2026-01-07T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/3#1"},
    {"task_id": "T-3", "at": "2026-01-07T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/3#2"},
    {"task_id": "T-3", "at": "2026-01-07T09:25:00Z", "action": "CONFLICT_RECORDED", "data": {"kind": "resource", "resource_id": "db:test-db"}, "ref": "task/3#lock"},
    {"task_id": "T-3", "at": "2026-01-07T09:30:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/3#3"},
    {"task_id": "T-3", "at": "2026-01-07T09:40:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/3#4"},
    {"task_id": "T-3", "at": "2026-01-07T10:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "BLOCKED", "ref": "task/3#5"},
    {"task_id": "T-3", "at": "2026-01-07T14:00:00Z", "action": "STATUS_CHANGED", "from_status": "BLOCKED", "to_status": "IN_PROGRESS", "ref": "task/3#6"},
    {"task_id": "T-3", "at": "2026-01-08T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "BLOCKED", "ref": "task/3#7"},
    {"task_id": "T-3", "at": "2026-01-08T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "BLOCKED", "to_status": "IN_PROGRESS", "ref": "task/3#8"},
    {"task_id": "T-3", "at": "2026-01-08T11:30:00Z", "action": "CHECK_RECORDED", "data": {"result": "PASS", "kind": "local", "attempt": 1}, "ref": "task/3#9"},
    {"task_id": "T-3", "at": "2026-01-08T13:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/3#10"},
    {"task_id": "T-3", "at": "2026-01-08T13:30:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/3#11"},
    {"task_id": "T-3", "at": "2026-01-08T14:00:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/3#12"},
    {"task_id": "T-3", "at": "2026-01-08T14:30:00Z", "action": "STATUS_CHANGED", "from_status": "APPROVED", "to_status": "MERGED", "ref": "task/3#13"},
    {"task_id": "T-3", "at": "2026-01-09T08:50:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-1", "status": "PASS"}, "ref": "task/3#14"},
    {"task_id": "T-3", "at": "2026-01-09T08:51:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-2", "status": "NOT_APPLICABLE"}, "ref": "task/3#15"},
    {"task_id": "T-3", "at": "2026-01-09T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "MERGED", "to_status": "VERIFIED", "ref": "task/3#16"},
    {"task_id": "T-4", "at": "2026-01-10T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/4#1"},
    {"task_id": "T-4", "at": "2026-01-10T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/4#2"},
    {"task_id": "T-4", "at": "2026-01-10T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/4#3"},
    {"task_id": "T-4", "at": "2026-01-10T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "CANCELED", "ref": "task/4#4"},
    {"task_id": "T-5", "at": "2026-01-11T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/5#1"},
    {"task_id": "T-5", "at": "2026-01-11T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/5#2"},
    {"task_id": "T-5", "at": "2026-01-11T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/5#3"},
    {"task_id": "T-5", "at": "2026-01-11T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/5#4"},
    {"task_id": "T-5", "at": "2026-01-11T10:00:00Z", "action": "CHECK_RECORDED", "data": {"result": "FAIL", "kind": "local", "attempt": 1}, "ref": "task/5#5"},
    {"task_id": "T-5", "at": "2026-01-11T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "FAILED", "ref": "task/5#6"},
    {"task_id": "T-5", "at": "2026-01-11T10:40:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-1", "status": "FAIL"}, "ref": "task/5#7"},
    {"task_id": "T-6", "at": "2026-01-12T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/6#1"},
    {"task_id": "T-6", "at": "2026-01-12T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/6#2"},
    {"task_id": "T-6", "at": "2026-01-12T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/6#3"},
    {"task_id": "T-6", "at": "2026-01-12T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/6#4"},
    {"task_id": "T-6", "at": "2026-01-12T10:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "BLOCKED", "ref": "task/6#5"},
    {"task_id": "T-7", "at": "2026-01-20T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/7#1"},
    {"task_id": "T-7", "at": "2026-01-20T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/7#2"},
    {"task_id": "T-7", "at": "2026-01-20T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/7#3"},
    {"task_id": "T-7", "at": "2026-01-20T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/7#4"},
    {"task_id": "T-7", "at": "2026-01-20T10:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/7#5"},
    {"task_id": "T-7", "at": "2026-01-20T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/7#6"},
    {"task_id": "T-7", "at": "2026-01-20T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/7#7"},
    {"task_id": "T-7", "at": "2026-01-20T11:30:00Z", "action": "STATUS_CHANGED", "from_status": "APPROVED", "to_status": "MERGED", "ref": "task/7#8"},
    {"task_id": "T-7", "at": "2026-01-21T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "MERGED", "to_status": "VERIFIED", "ref": "task/7#9"},
    {"task_id": "T-8", "at": "2026-01-30T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/8#1"},
    {"task_id": "T-8", "at": "2026-01-30T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/8#2"},
    {"task_id": "T-8", "at": "2026-01-30T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/8#3"},
    {"task_id": "T-8", "at": "2026-01-30T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/8#4"},
    {"task_id": "T-8", "at": "2026-01-30T10:00:00Z", "action": "CHECK_RECORDED", "data": {"result": "PASS", "kind": "ci", "attempt": 1}, "ref": "ci/1101"},
    {"task_id": "T-8", "at": "2026-01-30T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/8#6"},
    {"task_id": "T-8", "at": "2026-01-30T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/8#7"},
    {"task_id": "T-8", "at": "2026-01-30T11:30:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/8#8"},
    {"task_id": "T-8", "at": "2026-01-30T12:00:00Z", "action": "STATUS_CHANGED", "from_status": "APPROVED", "to_status": "MERGED", "ref": "task/8#9"},
    {"task_id": "T-9", "at": "2026-01-22T09:00:00Z", "action": "TASK_CREATED", "to_status": "PLANNED", "ref": "task/9#1"},
    {"task_id": "T-9", "at": "2026-01-22T09:10:00Z", "action": "STATUS_CHANGED", "from_status": "PLANNED", "to_status": "READY", "ref": "task/9#2"},
    {"task_id": "T-9", "at": "2026-01-22T09:20:00Z", "action": "TASK_DISPATCHED", "from_status": "READY", "to_status": "DISPATCHED", "ref": "task/9#3"},
    {"task_id": "T-9", "at": "2026-01-22T09:30:00Z", "action": "STATUS_CHANGED", "from_status": "DISPATCHED", "to_status": "IN_PROGRESS", "ref": "task/9#4"},
    {"task_id": "T-9", "at": "2026-01-22T10:00:00Z", "action": "CHECK_RECORDED", "data": {"result": "PASS", "kind": "ci", "attempt": 1}, "ref": "ci/1201"},
    {"task_id": "T-9", "at": "2026-01-22T10:30:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/9#6"},
    {"task_id": "T-9", "at": "2026-01-22T11:00:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/9#7"},
    {"task_id": "T-9", "at": "2026-01-22T11:30:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "IN_PROGRESS", "ref": "task/9#8"},
    {"task_id": "T-9", "at": "2026-01-22T14:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/9#9"},
    {"task_id": "T-9", "at": "2026-01-22T14:30:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/9#10"},
    {"task_id": "T-9", "at": "2026-01-22T15:00:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "IN_PROGRESS", "ref": "task/9#11"},
    {"task_id": "T-9", "at": "2026-01-23T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "BLOCKED", "ref": "task/9#12"},
    {"task_id": "T-9", "at": "2026-01-23T12:00:00Z", "action": "STATUS_CHANGED", "from_status": "BLOCKED", "to_status": "IN_PROGRESS", "ref": "task/9#13"},
    {"task_id": "T-9", "at": "2026-01-23T13:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/9#14"},
    {"task_id": "T-9", "at": "2026-01-23T13:30:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/9#15"},
    {"task_id": "T-9", "at": "2026-01-23T14:00:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "IN_PROGRESS", "ref": "task/9#16"},
    {"task_id": "T-9", "at": "2026-01-23T16:00:00Z", "action": "STATUS_CHANGED", "from_status": "IN_PROGRESS", "to_status": "REVIEW_READY", "ref": "task/9#17"},
    {"task_id": "T-9", "at": "2026-01-23T16:30:00Z", "action": "STATUS_CHANGED", "from_status": "REVIEW_READY", "to_status": "MASTER_REVIEW", "ref": "task/9#18"},
    {"task_id": "T-9", "at": "2026-01-23T17:00:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/9#19"},
    {"task_id": "T-9", "at": "2026-01-23T17:30:00Z", "action": "APPROVAL_INVALIDATED", "from_status": "APPROVED", "to_status": "MASTER_REVIEW", "ref": "task/9#20"},
    {"task_id": "T-9", "at": "2026-01-23T18:00:00Z", "action": "STATUS_CHANGED", "from_status": "MASTER_REVIEW", "to_status": "APPROVED", "ref": "task/9#21"},
    {"task_id": "T-9", "at": "2026-01-23T18:30:00Z", "action": "STATUS_CHANGED", "from_status": "APPROVED", "to_status": "MERGED", "ref": "task/9#22"},
    {"task_id": "T-9", "at": "2026-01-24T08:50:00Z", "action": "ACCEPTANCE_RECORDED", "data": {"ac_id": "AC-1", "status": "PASS"}, "ref": "task/9#23"},
    {"task_id": "T-9", "at": "2026-01-24T09:00:00Z", "action": "STATUS_CHANGED", "from_status": "MERGED", "to_status": "VERIFIED", "ref": "task/9#24"}
  ]
}
```

## Limits

- **Small samples by construction.** A project with a dozen tasks has a dozen samples at best; the numbers describe that project's last few weeks, not the quality of an agent or a model.
- **Not every metric is collected yet.** Checks, acceptance rows and regression reports are read only when a source records them. On a plain `local` task library they are `unknown`; `github` needs the PR timeline wired before those three become `ok`. The report says so instead of reporting a comfortable `0`.
- **Durations are wall-clock.** A task waiting overnight and a task waiting on a slow reviewer both count as hours. Do not convert them into "effort".
- **Window choice moves the numbers.** A window that ends right after a batch of merges understates regressions; one that starts mid-task understates elapsed time. Always print the window with the value.
- **The definitions are a product decision.** Changing a start / end event or an exclusion changes every historical figure; record the change here, with the date, before the next report is quoted.
