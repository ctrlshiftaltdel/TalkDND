# Local Task Packet

> This is the self-contained brief passed to one Execution Agent. The SQLite task record remains the state source of truth.
> Evidence item fields, `status` values and the G1 rules are defined once, in `kit/orchestration.md` 4.1; this template only carries the fields and does not restate their meaning.

- **Task ID:**
- **Title:**
- **Goal / user-visible result:**
- **Context:**
- **Allowed paths:**
- **Forbidden paths:**
- **declared_resources:**
- **Dependencies (must be VERIFIED):**
- **Acceptance criteria:**
  - [ ] AC-1:
- **Invariants:**
- **Negative cases:**
- **Migration cases:**
- **Integration paths:**
- **Unverified assumptions:**
- **Validation commands:**
- **Baseline branch / commit:**
- **Expected branch / Worktree:**
- **Evidence head SHA:**
- **Evidence matrix:**
  | ac_id | method | expected | actual | status | environment | commit_sha | evidence_ref | notes |
  |------|--------|----------|--------|--------|-------------|------------|--------------|-------|
- **Risks / human decisions:**
- **Delivery evidence:** changed paths, `git status --short`, `git diff --name-only`, head SHA, command output / exit codes, incomplete items and risks.
- **Hard boundaries:** no scope expansion, no self-approval, no merge.

## Field notes

- **One goal per packet.** `Goal / user-visible result` states a single independently acceptable outcome; if it needs "and also", split the packet.
- **AC numbering.** Acceptance criteria are numbered `AC-1`, `AC-2`, ...; one checkable statement each, and the ids never change after dispatch (add new ids instead of renumbering).
- **Structured scenario fields.** Both editions use the same five names: `invariants`, `negative_cases`, `migration_cases`, `integration_paths`, `unverified_assumptions`. Meaning, requiredness and value rules live in `kit/orchestration.md` 4.1; this template only carries the fields. Small tasks write `N/A: <reason>`; empty or a bare `N/A` counts as a missing field.
- **`declared_resources` is optional.** It is the task's named resource set, written as `<resource_id>:<mode>[:<ttl>]` separated by commas (`db:test/mysql-main:exclusive,svc:build/win-01:shared:600`). Empty (or omitted) means the task takes no resource lock and behaves exactly like a path-lock-only task. Semantics live only in `docs/design/resource-locks.md`; the shape here is only what the carrier needs to record. A resource set is acquired atomically at dispatch, so any conflict fails the whole dispatch instead of taking a partial set. `local` acquires these locks atomically inside one SQLite transaction; `github` only records an advisory reservation and claims no atomicity.
- **Evidence matrix.** Column order is fixed: `ac_id`, `method`, `expected`, `actual`, `status`, `environment`, `commit_sha`, `evidence_ref`, `notes` (meaning and rules: 4.1, not restated here). Every AC needs at least one row, and every `commit_sha` must equal `Evidence head SHA` except on `NOT_APPLICABLE` rows.
- **Why not always maximal.** What is enforced is "field present + rules apply", not volume: a field that truly does not apply gets `N/A: <reason>`, and scenarios must not be invented to fill the format.

### Cross-module tasks (shared state, locking, recovery)

Applies when the task touches shared mutable state, a lock file, a backup / rollback path, a multi-stage commit or a directory walk. Then:

- **Call-site inventory in `integration_paths`.** List every read-modify-write call site of the shared state — foreground, background, autosave, retry, migration — and the guard each one uses. One protected call site is not a complete answer, and a call-site list built by hand is only as good as the review of it.
- **Final-state assertions in `negative_cases`.** Every negative case names the file tree / state that must hold afterwards: baseline restored, no residue, the lock still exclusive, no false "rollback complete". "The call returned an error" is not an acceptance argument.
- **`invariants`** states what must stay true across modules and steps; **`migration_cases`** covers old lock files and old snapshot / manifest formats, plus what happens when they cannot be migrated (fail closed, `UNKNOWN`, or an explicit reviewer item).
- **Small tasks stay small.** When none of these surfaces exists, write `N/A: <reason>` with a concrete reason (as in Sample A above), never a bare `N/A`.
- The deeper rule set (R1–R6) lives in `docs/self-improvement/cross-module-state-and-recovery.md` in a repository checkout. It adds acceptance checkpoints only; it does not restate the state machine, the evidence fields or the merge gates.

## Task decomposition hard rules

- Each task carries exactly one independently acceptable goal.
- Allowed paths must be specified precisely down to concrete files or directories, never as vague wildcards or whole-repo access.
- Every acceptance criterion must be individually checkable, carry a unique `AC-<n>` id, and be paired with a verification command and its expected output.
- Granularity ceiling: a single review must be able to walk through all acceptance criteria item by item. If a task cannot be broken down finely enough, split it further; dispatching oversized tasks is not allowed.

## Verification before completion

- Delivery evidence must include real command output and exit codes from actual runs; inferred claims such as "this should pass" are not accepted.
- Every acceptance criterion must be mapped to at least one evidence item in the matrix; a criterion without evidence counts as not met.
- Tasks that change shared state, locks, backups or recovery must show the **final** state, not only the returned error: the evidence compares the ending file tree / state with the baseline (no residue), and states whether a rollback really happened.
- Tasks involving code follow red-green-refactor: first write a failing test and actually run it to prove it fails, then make it pass with a minimal implementation. Declaring completion without test evidence is prohibited.
- Any unverified item must be explicitly marked as incomplete (`NOT_RUN` / `BLOCKED`) and must not be counted as passing.

## Sample A — small task without migration (synthetic)

```task-packet sample-a-small-task
- **Task ID:** SAMPLE-1001
- **Title:** Add a `--dry-run` flag to the export command
- **Goal / user-visible result:** `export --dry-run` lists the files it would write and exits 0 without touching the filesystem.
- **Context:** Users currently run a real export just to see what would change.
- **Allowed paths:** `src/cli/export.py`, `tests/test_export.py`
- **Forbidden paths:** every other path in the repository
- **Dependencies (must be VERIFIED):** none
- **Acceptance criteria:**
  - [x] AC-1: `export --dry-run` prints the planned targets and exits 0 without creating or modifying files.
  - [x] AC-2: `export` without the flag keeps its previous output and exit codes.
- **Invariants:** N/A: one command, no shared state or cross-module contract is touched.
- **Negative cases:** N/A: the flag is read-only and adds no rejection path.
- **Migration cases:** N/A: no persisted data, configuration or file format changes.
- **Integration paths:** N/A: output goes to stdout only; no other module or service consumes it.
- **Unverified assumptions:** N/A: AC-1 and AC-2 cover the whole change; nothing is left open.
- **Validation commands:** `uv run python -m unittest tests.test_export`
- **Baseline branch / commit:** `main` @ `1111111`
- **Expected branch / Worktree:** `task/1001-dry-run` / `.worktrees/1001-dry-run`
- **Evidence head SHA:** 1111111111111111111111111111111111111111
- **Evidence matrix:**
  | ac_id | method | expected | actual | status | environment | commit_sha | evidence_ref | notes |
  |------|--------|----------|--------|--------|-------------|------------|--------------|-------|
  | AC-1 | `uv run python -m unittest tests.test_export.DryRunTests` | 4 tests pass; temporary directory stays empty | 4 tests passed; `du` on temp dir shows 0 bytes | PASS | Linux, Python 3.12, local edition, branch task/1001-dry-run | 1111111111111111111111111111111111111111 | task record: evidence log 12 | (none) |
  | AC-2 | `uv run python -m unittest tests.test_export` | pre-existing export tests still pass | 12 tests passed | PASS | same as above | 1111111111111111111111111111111111111111 | task record: evidence log 12 | (none) |
- **Risks / human decisions:** none identified; flag defaults to off.
- **Delivery evidence:** changed paths, Diff, head SHA, command output / exit codes.
- **Hard boundaries:** no scope expansion, no self-approval, no merge.
```

## Sample B — task with negative and migration scenarios (synthetic)

```task-packet sample-b-negative-migration
- **Task ID:** SAMPLE-1002
- **Title:** Make the save loader version-aware
- **Goal / user-visible result:** the save loader refuses payloads it cannot migrate and upgrades version 2 saves to version 3 on load.
- **Context:** Saves written before the format change still exist; an unknown version must never be half-loaded.
- **Allowed paths:** `src/saves/loader.py`, `src/saves/migrations.py`, `tests/saves/`
- **Forbidden paths:** `src/saves/writer.py`, `docs/`
- **Dependencies (must be VERIFIED):** SAMPLE-1001
- **Acceptance criteria:**
  - [x] AC-1: a version 3 save loads and round-trips without losing fields.
  - [x] AC-2: an unknown or truncated version is rejected with a typed error and the source file stays byte-identical.
  - [ ] AC-3: a version 2 save is migrated to version 3 with defaults filled and reported to the caller.
- **Invariants:** the source file is never overwritten before the migrated payload validates; the writer's on-disk format is unchanged.
- **Negative cases:** unknown version number; truncated payload; version 2 save whose optional block is missing; loading twice in a row must not mutate the file.
- **Migration cases:** version 2 to version 3 fills the new field with its documented default; a save with no version marker is treated as version 1 and must fail closed instead of guessing; rollback keeps the original file.
- **Integration paths:** loader is called by the CLI import path and by the autosave loop; both must surface the typed error unchanged.
- **Unverified assumptions:** autosave retries never run while a migration is in progress. Not yet verified; if false, AC-2's no-mutation guarantee needs a lock.
- **Validation commands:** `uv run python -m unittest tests.saves`
- **Baseline branch / commit:** `main` @ `2222222`
- **Expected branch / Worktree:** `task/1002-loader-versions` / `.worktrees/1002-loader-versions`
- **Evidence head SHA:** bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
- **Evidence matrix:**
  | ac_id | method | expected | actual | status | environment | commit_sha | evidence_ref | notes |
  |------|--------|----------|--------|--------|-------------|------------|--------------|-------|
  | AC-1 | `uv run python -m unittest tests.saves.test_loader.LoadTests` | 6 tests pass; round-trip diff empty | 6 tests passed; round-trip diff empty | PASS | Linux, Python 3.12, github edition, branch task/1002-loader-versions | bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb | PR comment 1 | (none) |
  | AC-2 | `uv run python -m unittest tests.saves.test_loader.UnknownVersionTests` | typed error raised; source file unchanged | 3 negative tests passed; mtime and bytes unchanged | PASS | same as above | bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb | PR comment 1 | also covers the truncated payload |
  | AC-3 | `uv run python -m unittest tests.saves.test_migration.V2ToV3Tests` | version 2 fixture migrates with defaults filled | not executed: version 2 fixture not written yet | NOT_RUN | same as above | bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb | task record: no run recorded | AC-3 left unchecked on purpose; must not be reported as passing |
- **Risks / human decisions:** autosave assumption above may require a lock; if so the task returns to planning instead of widening scope silently.
- **Delivery evidence:** changed paths, Diff, head SHA, command output / exit codes.
- **Hard boundaries:** no scope expansion, no self-approval, no merge.
```
