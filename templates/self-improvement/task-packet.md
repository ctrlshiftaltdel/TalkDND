# Self-Improvement Task Packet

> A task created by the self-improvement loop uses the same packet as any other task.
> `local` writes it to the task database, `github` writes it to an Issue — never both.
> The evidence item fields, `status` values and the G1–G6 rules are defined only in
> `kit/orchestration.md` 4.1; this template carries the fields and does not restate them.
> Small tasks write `N/A: <reason>` in a scenario field; a bare `N/A` is a missing field.

- **Task ID / Issue:**
- **Title:**
- **Source lesson:** `lesson_id` / fingerprint / evidence level
- **Trigger and edition:** `进行自我完善` / `local` | `github`
- **Goal / user-visible result:** (one independently acceptable outcome)
- **Context:** (what was observed, and what is explicitly out of scope)
- **Allowed paths:**
- **Forbidden paths:**
- **declared_resources:** (optional, `<id>:<mode>[:<ttl>]`, comma separated)
- **Dependencies (must be `VERIFIED`):**
- **Acceptance criteria:**
  - [ ] `AC-1`: <checkable statement> — verification: `<command or manual check>`
  - [ ] `AC-2`:
- **Invariants:** (what must stay true across modules and steps; or `N/A: <reason>`)
- **Negative cases:** (inputs / paths that must be rejected or stay unchanged; or
  `N/A: <reason>`)
- **Migration cases:** (old data / configuration / version paths and the no-migration
  behaviour; or `N/A: <reason>`)
- **Integration paths:** (callers, callees and protocol boundaries touched; for shared
  mutable state list every read-modify-write call site; or `N/A: <reason>`)
- **Unverified assumptions:** (what is not proven yet and would overturn the task; or
  `N/A: <reason>`)
- **Validation commands:**
- **Baseline branch / commit:**
- **Expected branch / Worktree:**
- **Evidence head SHA:**
- **Evidence matrix:**
  | ac_id | method | expected | actual | status | environment | commit_sha | evidence_ref | notes |
  |------|--------|----------|--------|--------|-------------|------------|--------------|-------|
- **Risks / human decisions:** (`HD-*` ids when a decision is open; unresolved decisions
  block dispatch)
- **Delivery evidence:** changed paths, `git status --short`, `git diff --name-only`, head
  SHA, command output / exit codes, incomplete items and risks.
- **Hard boundaries:** no scope expansion, no self-approval, no merge, no closing the
  Issue, no claiming `VERIFIED` by the execution agent.

## Fill rules

- **One goal per packet.** If the goal needs "and also", split it before creating an
  Issue / task.
- **AC ids are stable.** `AC-1 … AC-n` never renumber after dispatch; add new ids.
- **No unexecuted claim is a pass.** A command that was not run is `NOT_RUN`, and a
  blocked one is `BLOCKED`; neither counts as passing.
- **High-risk work is not silently dispatched.** Permission / security, locks, state
  machine, migration, external dependency, privacy, breaking compatibility and
  irreversible actions need an owner decision (`NEEDS_HUMAN_DECISION`) before a task
  packet exists.
- **One carrier per edition.** `local` uses `scripts/orchestrator.py`; `github` uses the
  Issue and a Draft PR. A missing carrier capability is reported, never simulated.
