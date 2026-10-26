# Contract Decision Log

> One append-mostly log per project (or per milestone) that records every change to a frozen contract. The contract itself is [DESIGN.md](design.md); the closeout review that consumes these records is [MILESTONE-AUDIT.md](milestone-audit.md).
>
> A decision log answers three questions that a design document alone cannot: what is frozen, what is still undecided, and what is expected to change later. It is not a task list — execution state lives on the task carrier and follows the lifecycle in [kit/orchestration.md](../kit/orchestration.md), which this file does not restate.

## When to use

- Freezing a schema, API, event shape, invariant or permission boundary that downstream tasks must not rewrite.
- Changing or unfreezing such an item: a new record, not an edit to the old one.
- Leaving an item undecided at freeze time, so the next task does not fill the gap by guessing.
- Answering, at milestone closeout, which contract changes happened and what they touched.

## Status values

| Status | Meaning | May it change? |
| --- | --- | --- |
| `OPEN` | Not decided yet; nothing is frozen. The row says who decides and what is blocked. | Yes — record the decision as a new row, then mark this one `SUPERSEDED`. |
| `FROZEN` | Downstream tasks must follow it. A unilateral rewrite is a contract violation, not a refactor. | Only through a superseding record with affected tasks and evidence. |
| `REVISITABLE` | Frozen for now, but a change is already expected at a stated trigger. | Through a superseding record once the trigger is met. |
| `SUPERSEDED` | Replaced by another decision ID. Kept for history. | No — the replacement applies. |

`FROZEN` and `REVISITABLE` are both "already decided"; `OPEN` is "not decided". Keeping the three apart is the point: "we froze it", "we will change it", and "we have not decided" need different actions.

## Frozen decisions

| ID | Date | Decider | Affected tasks | Before | After | Rationale | Status | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DC-001 | | | | | | | `FROZEN` | |

## Open decisions

| ID | Date | Decider | Affected tasks | Question | Current default (if any) | What unblocks it | Status | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DC-002 | | | | | | | `OPEN` | |

## Revisitable items

| ID | Date | Decider | Affected tasks | Current decision | Trigger for change | Planned successor | Status | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DC-003 | | | | | | | `REVISITABLE` | |

## Superseded records

| ID | Superseded by | Date | What changed | Affected tasks |
| --- | --- | --- | --- | --- |
| | | | | |

## Recording rules

- IDs are sequential (`DC-<n>`), never reused and never renumbered; a correction is a new row.
- Every row names the decider and the affected tasks; "the team" or "everyone" is not a decider.
- `Before` / `After` are filled for a change: what the contract said, what it says now. For a first freeze, `Before` is `(none)`.
- Evidence is a resolvable pointer — task ID, PR or Issue comment — plus the head SHA it was recorded at. A decision without evidence is `OPEN`, not `FROZEN`.
- Do not delete rows. An obsolete decision becomes `SUPERSEDED` and stays visible.
- A change to a frozen item also updates the affected section of [DESIGN.md](design.md) in the same task; the log and the contract must not disagree.
- `OPEN` rows must say what unblocks them. Anything still `OPEN` at milestone closeout is reported as `NEEDS_HUMAN_DECISION`, not silently dropped.

### Example (fabricated, for shape only)

```text
| DC-004 | 2026-01-14 | owner | TASK-0012, TASK-0015 | balance was a float | balance is an integer in minor units | float drift breaks reconciliation | `FROZEN` | task TASK-0012 record, head 4f2c1ab |
```
