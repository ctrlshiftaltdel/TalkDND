# Runtime Capability Record

> Filled in **after** a real probe of one runtime, one version, one edition and one execution mode. One record per probed combination: a runtime upgrade, a different edition, or a different delivery mode is a different record.
>
> **Contract:** C1–C4 are defined once in [kit/orchestration.md](../kit/orchestration.md) §5. This template records **what was measured** for that contract plus the finer boundaries a scheduler actually depends on. It does not redefine C1–C4 and does not replace them.
>
> **UNKNOWN is a valid answer.** A capability that was not probed, whose evidence expired, or whose evidence cannot be located is written as `UNKNOWN`. `UNKNOWN` never means "probably works", and it never unlocks the sub-agent path. Only `PASS` on C1–C4 does.
>
> **This record is not a permission grant.** Recording `PASS` for "can write files" or "can run CI commands" says what the runtime *was observed to do*; it does not authorize this task, project or person to do it. Authorization stays with [AGENTS.md](../AGENTS.md), the project access control and the merge gates.

## When to use

- Before claiming that this runtime can spawn / isolate / wait / collect a sub-agent or an execution instance, and before a task is routed to a runtime you have not used yet.
- Again whenever the runtime, its exact version, the edition, the execution mode or the way results are collected changes — that is a new record, not an edit.
- When a previous verdict has to be questioned: an expired probe date, a dead evidence link, or a probe whose result was ambiguous.
- Before a stage closes: a stage that reports "we used sub-agents" without a record here has an unverified claim, not a capability.


## Record identity

| Field | Value | Rules |
|-------|-------|-------|
| Runtime name | | Product / CLI / service name, not a nickname |
| Runtime version | | Exact version string as reported by the runtime; "latest" is not a version |
| Edition (`local` / `github`) | | Must match `docs/PROJECT.md` |
| Execution mode | | `spawned-subagent` / `manual-handoff` / `scheduled-job` / other exact mode |
| Delivery artifacts | | What actually comes back: diff, commit SHA, file, PR, log, exit code |
| Probe date (UTC) | | `YYYY-MM-DDThh:mm:ssZ`; local time is not acceptable |
| Probe task / run ID | | The task record or Issue that holds the probe, addressable by anyone |
| Probed by | | Role + account, so a later reader can ask a follow-up |
| Supersedes | | Previous record (path or task ID), or `(none)` |
| Status | | `COMPLETE` / `PARTIAL` / `STALE` — `PARTIAL` and `STALE` cannot unlock the sub-agent path |

## Capability matrix

One row per capability. `Verdict` uses only `PASS` / `FAIL` / `UNKNOWN` / `NOT_APPLICABLE`; `NOT_APPLICABLE` requires a reason in `Limits`. A row without a probe date and an addressable `evidence_ref` is `UNKNOWN`, not `PASS`.

| ID | Capability | What must be shown | Probe performed | Verdict | Probe date (UTC) | evidence_ref | Limits |
|----|------------|--------------------|-----------------|---------|------------------|--------------|--------|
| C1 | Spawn and handle | A start call returns an addressable handle (ID / URL / session id) that resolves back to that instance | Start one read-only probe, record the handle, resolve it again | | | | |
| C2 | Context isolation | Two instances cannot see each other's writable intermediate state | Start two read-only probes, check each cannot see the other's output | | | | |
| C3 | Wait and collect | Waiting returns status, artifacts and evidence (files, command output, exit code, head SHA) | Have the probe produce a known output, wait, collect and compare | | | | |
| C4 | Terminal classification | Success, failure and cancellation are distinguishable; empty / unknown results are never success | Produce three terminal outcomes and classify each | | | | |
| C5 | Cancellation | A running instance can be stopped, and the stop is reported as cancelled rather than failed-or-succeeded | Start a long probe, cancel it, read back the terminal state | | | | |
| C6 | File / command / CI permissions | Which paths it may write, which commands it may run, whether it may read CI logs, trigger or read checks | Attempt one allowed and one denied action per class; record the observed enforcement | | | | |
| C7 | Tools and test environment | Language runtimes, package managers, network access, test runner, available disk / CPU, whether the environment is reproducible | Run the project's real validation command in that environment | | | | |
| C8 | Probe metadata | Probe time, runtime identity and evidence are recorded where a later reader can find them | Locate this record from the task record / Issue | | | | |

`Verdict` aggregation: the sub-agent path requires `PASS` on **all** of C1–C4. `C5`–`C8` may be `UNKNOWN` for a manual-handoff path, but then the handoff, timeouts and cancellation must be handled by the operator and written into `Limits`.

## Extended boundaries

Answer each line; `UNKNOWN` or `N/A: <reason>` are the only alternatives to a measured answer.

- **Spawn:** how many instances can run at once, what the handle looks like, what happens when the handle is stale, whether a failed spawn is distinguishable from a queued one.
- **Isolation:** whether working directories, environment variables, task state and credentials are shared; what leaks between two concurrent instances; whether isolation is per instance, per session or per machine.
- **Wait / collect:** timeout behaviour, partial output, whether artifacts survive after the instance ends, which of status / artifact / evidence is genuinely retrievable and which is only summarised.
- **Terminal classification:** the exact strings or codes for success, failure, cancellation, timeout and "unknown"; how an empty result is classified.
- **Cancellation:** who may cancel, what the cancelled run leaves behind (locks, branches, temp files), whether cancellation is recorded as a terminal state.
- **File and CI permissions:** writable roots, whether writes outside the worktree are possible, ability to read CI logs / checks, ability to trigger a run, ability to approve or merge (should be none for an execution instance).
- **Tools and test environment:** installed toolchain versions, network access, ability to install dependencies, test parallelism, whether two instances share one test database or build cache.
- **Probe time and evidence:** the exact commands run and where their output lives; whether the evidence is still addressable after the probe branch is deleted.

## UNKNOWN and re-probe

- A capability is `UNKNOWN` when it was not probed, when the probe predates the current runtime version, when the runtime or edition changed, when the evidence link no longer resolves, or when the probe result was ambiguous.
- Re-probe the affected capabilities — not just the changed one — when any of these happens: runtime upgrade or downgrade, edition switch (`local` ↔ `github`), execution-mode change, collection mechanism change (logs, artifacts, API), a failed or cancelled probe, or this record being older than the project's stated review interval.
- An unrelated capability may keep its old verdict only if its own probe date and evidence are still valid; "the rest still applies" is acceptable, "everything still applies" is not.
- Downgrade a verdict to `UNKNOWN` first and re-probe second. Do not leave a stale `PASS` in place while the re-probe is pending.
- After a downgrade, the routing decision falls back to manual handoff until the capability is `PASS` again; the record states the fallback, not just the missing capability.

## Permission scope

- A `PASS` here means "the runtime was observed to be able to do this", not "you may do this". Task scope ([templates/task-packet.md](task-packet.md) allowed / forbidden paths), project policy ([AGENTS.md](../AGENTS.md)) and the merge gates still decide.
- Capability records never grant credentials, never widen allowed paths and never replace the CI checks of the merge gate G4.
- The record must not contain tokens, session cookies, prompt text or customer data. Reference the probe by task ID / URL, not by pasting the exchange.

## Evidence and limits

- Every `PASS` row points to evidence that a second person can open without asking the author: a task record, an Issue comment, a CI run URL, a commit SHA plus file path, or a log file committed for the probe.
- Evidence that only exists inside one chat session is not evidence. If the only copy is in a transcript, write `UNKNOWN`.
- `Limits` is not optional prose: it carries what the probe did **not** cover, known flakiness, environment differences (OS, network, credentials) and what would invalidate the verdict.
- Keep the record where the task carrier can find it (`local`: task record or `docs/`; `github`: Issue comment linked from the task). A record nobody can locate is equivalent to `UNKNOWN`.

```capability-record example
- Runtime name: example-runtime-cli
- Runtime version: 2.4.1
- Edition: local
- Execution mode: spawned-subagent
- Probe date (UTC): 2026-01-12T09:30:00Z
- Probe task / run ID: TASK-0007 (task record: events 41-58)
- Status: PARTIAL
- C1 PASS  handle `run-7f3a` resolved twice (evidence: TASK-0007 events 42, 44)
- C2 PASS  two probes could not see each other's scratch file (events 45-47)
- C3 PASS  status + diff + exit code 0 collected (events 50-52)
- C4 UNKNOWN  cancellation produced an empty result that could not be classified
- C5 FAIL  cancel returned success; see events 55-56
- C6 PASS  write outside the worktree was refused (events 57)
- C7 PASS  project validation command ran in 92s (events 58)
- Limits: C4/C5 block the sub-agent path for cancelled runs; fallback is manual handoff for
  any task that may need cancellation; re-probe required after 2.5.x.
```
