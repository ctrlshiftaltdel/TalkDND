# Milestone Audit

> Run at milestone closeout, by someone other than the executor of the last task, before the milestone is called done. A per-task review (see [REVIEW-CHECKLIST.md](review-checklist.md)) only proves that each task met its own acceptance criteria; it says nothing about whether the parts hold together. This template covers that gap.
>
> It does not restate the merge gates G1–G6, the task lifecycle or the evidence matrix — those are defined once in [kit/orchestration.md](../kit/orchestration.md). It also does not decide what "done" means for the milestone: that is fixed by the project charter and the frozen contracts ([DESIGN.md](design.md), [CONTRACT-DECISION-LOG.md](contract-decision-log.md)).

## When to use

- A milestone (or a phase of the plan) is about to be declared complete.
- A frozen contract changed and the blast radius is not obvious from the task records.
- Ownership is handed over and the receiving side needs the system-level state, not the task-level one.

Not for: reviewing a single task's delivery (use [REVIEW-CHECKLIST.md](review-checklist.md)) or accepting a single PR.

## How to fill

- Every conclusion carries a task ID, an evidence pointer and the head SHA it was taken at. A conclusion without all three is `NEEDS_HUMAN_DECISION`, not `PASS`.
- **Lightweight mode:** small projects keep every heading and answer the ones that do not apply with `N/A: <reason>` (a bare `N/A` is not acceptable). Skipping a heading is not the same as "not applicable".
- **Unverified is not passing.** A check whose evidence is `NOT_RUN`, `BLOCKED` or missing must not be recorded as `PASS`, and must not be counted in the pass totals.
- Copying the state machine or the gate definitions into this file is a defect: link them instead.

## Verdict classes

| Class | Meaning | Blocks milestone closeout? |
| --- | --- | --- |
| `PASS` | Checked against real evidence at the recorded head SHA; nothing to fix. | No (it is not a finding). |
| `BLOCKING` | The milestone's promised outcome is not met, or a frozen invariant / permission boundary is violated. | Yes. Must be fixed and re-audited on a new head. |
| `NON_BLOCKING_HARDENING` | Works, but the audit found a robustness / maintainability gap worth fixing soon. | No, but it is carried forward as residual work with a task ID. |
| `OPTIONAL` | Improvement with no correctness impact; may be dropped. | No. |
| `NEEDS_HUMAN_DECISION` | Requires an owner call (scope, risk acceptance, contract change, product trade-off). | Yes. The milestone stays open until it is decided and recorded. |

A finding is one of the four classes above; a verified-clean check is `PASS`. While any `BLOCKING` or `NEEDS_HUMAN_DECISION` row exists, the milestone must not be marked complete and must not be reported as delivered — neither in the plan, nor in the task carrier, nor to the owner.

## 1. Functional completeness

<!-- Does the milestone deliver the promised user-visible outcome end to end, and is each claim traceable to a verified acceptance criterion? -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 2. Cross-module invariants

<!-- Do the invariants frozen in the design contracts still hold across modules, on the integrated head rather than per task? -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 3. Permissions and security

<!-- Authorization checks, trust boundaries, secrets and personal data. Anything unverified here is blocking by default. -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 4. Legacy data migration

<!-- Was old data read, migrated and rolled back on real or representative data, or only on an empty fixture? -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 5. Real and synthetic data validation

<!-- Which conclusions rest on synthetic fixtures, and what would change if real data behaves differently? -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 6. End-to-end path

<!-- The main user path exercised on the integrated branch, not the sum of unit tests. -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 7. Recoverability

<!-- Failure, restart, rollback and backup/restore: what the system does when the happy path stops, and what it leaves behind. -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 8. Performance baseline

<!-- The measured baseline and the conditions it was measured under, or `N/A: <reason>` when the milestone makes no performance claim. -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 9. Public content and privacy

<!-- Anything published (release notes, README, docs, examples, logs) checked for secrets, personal paths and private data. -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## 10. Residual items

<!-- Everything knowingly left undone, blocked or deferred, with the owner who accepted it. Empty is a conclusion too — write "none" plus its evidence. -->
- Verdict:
- Task / evidence / head SHA:
- Notes:

## Audit conclusion

| Verdict class | Count | Items |
| --- | --- | --- |
| `BLOCKING` | | |
| `NEEDS_HUMAN_DECISION` | | |
| `NON_BLOCKING_HARDENING` | | |
| `OPTIONAL` | | |

- Unverified / blocked checks that must not be counted as passing:
- Milestone may be declared complete: `yes` / `no` — `no` while any `BLOCKING` or `NEEDS_HUMAN_DECISION` row above exists
- Contract changes recorded since the last audit (decision log IDs):
- Auditor / date / audited head SHA / scope covered:
- Residual work carried into the next milestone (task IDs):
- Where this audit lives (task record, PR comment, document path):
