# Self-Improvement Proposal

> One proposal per lesson. It records *why* a change is worth making and which gate
> it must pass; it is not a task, an approval or an Issue.
> The task packet fields that a later task must carry (`invariants`, `negative_cases`,
> `migration_cases`, `integration_paths`, `unverified_assumptions`) and the evidence
> rules are defined only in `kit/orchestration.md` 4.1. This template does not restate
> them; write `N/A: <reason>` when a surface truly does not apply.

## 1. Identity

- **Proposal ID:**
- **Lesson ID / fingerprint:**
- **Trigger and edition:** `进行自我完善` / `local` | `github`
- **Current head SHA:**
- **Source evidence (links) and evidence level (E0–E4):**

## 2. What was observed

- **Observed fact (scoped to the evidence):**
- **Root cause:**
- **Impact:**
- **Why the existing mechanism did not catch it:**
- **Content class:** `CONFIRMED_FACT` | `SOURCE_SPECIFIC_EXPERIENCE` | `GENERAL_LESSON` |
  `UNVERIFIED_ASSUMPTION`
- **Can this lesson generate a task?** yes only for `GENERAL_LESSON` + `GENERALIZED`
  with no `OPEN` conflict; otherwise `no — <reason>`

## 3. Coverage check

- **Existing contract / code / test / process:** `<path or section>` — `COVERED` |
  `PARTIAL` | `NOT_COVERED`
- **If covered:** stop here and record the evidence; do not add a duplicate rule or a new
  test merely to look busy.

## 4. Options and cost / benefit

At least two options are required; "do nothing" is a valid option.

| Option | Benefit | Cost | Risk | Reversible? |
|---|---|---|---|---|
| A. Do nothing / keep watching | | | | |
| B. <Narrow change> | | | | |
| C. <Wider change> | | | | |

- **Recommended option and why:**
- **Non-goals:**

## 5. Risk and human decision

- **High-risk surface touched?** permission / security boundary, lock, state machine,
  data migration, external dependency, privacy, breaking compatibility, irreversible
  action, frozen schema → **yes / no**.
- **If yes:** required decision id from the catalogue in
  `docs/self-improvement/entrypoints-and-decisions.md` §5 (`HD-*`), the open question and
  the options. A proposal in this state is not dispatched.
- **Remaining risks after the decision:**
- **Rollback / migration plan:**

## 6. Draft task shape (input to the task template)

- **Proposed allowed paths:**
- **Forbidden paths:**
- **Dependencies (must be `VERIFIED`):**
- **Draft acceptance criteria:** `AC-1 …`
- **Validation commands:**
- **Negative scenarios:**
- **Status:** `DRAFT` | `NEEDS_HUMAN_DECISION` | `READY_FOR_TASK_PACKET`

> A `READY_FOR_TASK_PACKET` proposal still needs a complete task packet
> (`templates/self-improvement/task-packet.md`), independent review and the normal merge
> gates. The trigger phrase never authorises a change by itself.
