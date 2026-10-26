# Self-Improvement Closure Report

> Report one pass of the self-improvement loop. The report never replaces task state or a
> merge-gate decision: a task is done only when the normal gates G1–G6 (defined in
> `kit/orchestration.md` §4, not restated here) pass on the current head.
> Mark every unexecuted check `NOT_RUN` and every external blocker `BLOCKED`.

## 1. Trigger and context

- **Trigger phrase actually received:**
- **Route decision:** `SELF_IMPROVEMENT` | `EXPLAIN_ONLY` | `UPDATE_AGENTORCH` | `UNRELATED`
- **Edition:** `local` | `github`
- **Branch / Worktree:**
- **Head SHA at the start of the pass:**
- **Workspace status (`git status --short`):**
- **Active tasks / open PRs restored from the carrier:**

## 2. Scan and sources

- **Registry version / source ids:**
- **Cursor before / after, scan window, lookback:**
- **Filter counts, excluded items, suspicious instruction count:**
- **Per-source conclusion:** `OK` | `PARTIAL` | `BLOCKED` | `UNKNOWN` | `NOT_RUN`
- **If nothing was scanned:** say "no source to scan" / "scan incomplete"; never write
  "no new experience" for a scan that did not run.

## 3. Candidates and lessons

| lesson_id | content_class | evidence_level | status | duplicate_of | OPEN conflict | task-eligible |
|---|---|---|---|---|---|---|

- **New candidates this pass:**
- **Reused / previously captured candidates:**
- **Rejected candidates and why:**

## 4. Coverage and plan

- **Coverage check result per lesson:** `COVERED` | `PARTIAL` | `NOT_COVERED` with the
  contract / code / test reference.
- **Proposals created:** proposal id, recommended option, cost / benefit summary.
- **Blocked proposals:** reason (missing evidence, incomplete packet, dependency not
  `VERIFIED`, edge of allowed paths).
- **Human decisions required:** `HD-*` id, question, options, who decides; an open
  decision blocks the corresponding task.

## 5. Registered work

- **`local`:** task IDs, state, locked paths, Worktree / branch.
- **`github`:** Issue numbers, labels, `ORCH-TASK-REGISTRY` status, Draft PR numbers.
- **Dependencies and order:**
- **What was *not* created and why:** (no duplicate Issue / task, already covered, high
  risk without a decision)

## 6. Verification

- **Review status:** independent review performed? by whom? (`G1–G6` by reference)
- **Head SHA of the evidence:** (old-head results do not count)
- **Commands / exit codes / CI run links:**
- **Evidence matrix location:** task record | Issue comment | PR comment
- **Not run / blocked / unknown items and reasons:**
- **Merged?** merged by Master only; merge is never performed by the loop or the
  execution agent.

## 7. Residual risk and next step

- **Residual risks and accepted limitations:**
- **Assumptions still unverified:**
- **Next task / decision and who owns it:**
- **One-line conclusion:** `COMPLETED` | `NO_NEW_CANDIDATES` | `SCAN_INCOMPLETE` |
  `NO_ACTIONABLE_CANDIDATES` | `NEEDS_HUMAN_DECISION` | `BLOCKED` — with the reason.
