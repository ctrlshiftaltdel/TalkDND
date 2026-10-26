# Design Contract

> Filled in by the Execution Agent before implementation starts, reviewed section by section, and frozen by the Master before downstream tasks may depend on it. Keep it to one page where you can: answer what a later task could otherwise get wrong, not everything that is true.
>
> **When to use:** the change freezes a schema / API / event shape, crosses more than one module, touches authorization or stored data, or is expensive to reverse.
>
> **Lightweight mode:** a small change does not have to produce a long document, but it must not delete headings. Keep every heading and answer the ones that do not apply with `N/A: <reason>` (a bare `N/A` is not acceptable, and a missing heading is not the same as "not applicable").
>
> **Freeze is not permanence.** "Frozen" means downstream tasks may not rewrite the contract on their own; it does not mean the contract can never change. Any change is made by recording a decision in [CONTRACT-DECISION-LOG.md](contract-decision-log.md) with the affected tasks, the before/after text and the evidence, then updating this file.
>
> This template deliberately does **not** restate the task state machine, the merge gates G1–G6 or the evidence matrix. Those are defined once in [kit/orchestration.md](../kit/orchestration.md); reference them, never copy them here.

## Scope and non-goals

<!-- What this contract covers, and — explicitly — what it does not. A written non-goal is what stops a later task from widening scope. -->
- In scope:
- Non-goals:
- Consumers / modules affected:
- Owner (who may freeze this contract):

## Schema and interfaces

<!-- The frozen shapes: fields, types, required / optional, units, defaults, error codes. One minimal example is worth ten adjectives. -->
- Entities / payloads and their fields:
- Field types, required vs optional, defaults, units:
- API / event signatures (request → response):
- Stability per item (`stable` / `additive-only` / `experimental`):
- Minimal valid example (one payload or call):

## State transitions

<!-- Only the states THIS design introduces — not the task lifecycle. Say where the authoritative definition lives instead of repeating it. -->
- States introduced by this design:
- Legal transitions (from → to, trigger, guard):
- Illegal transitions and how they are rejected:
- Authoritative definition (file / document section):

## Invariants

<!-- Statements that must hold at every moment, including across modules. An invariant without a way to check it is a wish. -->
| ID | Invariant | Enforced where | Checked by |
| --- | --- | --- | --- |
| INV-1 | | | |

## Failure semantics

<!-- What happens when the happy path fails: who sees what, what is retried, what is left behind. -->
- Failure classes and the user-visible outcome of each:
- Retry / idempotency rules:
- Partial-write, rollback and cleanup behaviour:
- Error codes / messages and how stable they are:

## Compatibility and migration

- Previous contract and what stays compatible:
- Additive vs breaking changes in this design:
- Data / state migration steps and the way back:
- Readers and writers that may lag one version:
- Old data that must remain readable:

## Permission and trust boundaries

- Callers and what each of them may do:
- Trusted vs untrusted input (where the boundary is enforced):
- Secrets / personal data handled, and where they must never appear:
- Authorization checks and their location:

## Shared resource locks

<!-- Only when this design depends on resources that are not identified by file
     paths (test database, shared build environment, serial test slot, fixture
     volume). The semantics are frozen in docs/design/resource-locks.md —
     reference it, do not restate it. Otherwise answer `N/A: file paths cover it`. -->
- Named resources this design needs (`<resource_id>` + mode `shared` / `exclusive`):
- Why a path lock alone is not enough:
- What happens on lock conflict, lease expiry and holder crash:
- Backend limits acknowledged (`local` atomic within one database file; `github` is an advisory registration, never an atomic lock):

## Verification strategy

<!-- How each of the above becomes evidence. Command-level, not "tested manually". -->
- Contract / invariant checks and their commands:
- Negative cases that must fail:
- Migration test on real or representative data:
- End-to-end path exercised:
- Performance / resource baseline (or `N/A: <reason>`):
- Evidence carrier (task record, PR comment) and head SHA:

## Frozen decisions

<!-- The items downstream tasks may not rewrite. One row per decision; the full record lives in the decision log. -->
| Decision ID | Item frozen | Affects tasks | Rationale | Evidence |
| --- | --- | --- | --- | --- |
| DC-001 | | | | |

- Items frozen by this document:
- Items still open at freeze time (each with who decides and what is blocked):
- Items frozen now but expected to change (record them as revisitable in [CONTRACT-DECISION-LOG.md](contract-decision-log.md)):

## Open decisions

- Question / options / recommendation:
- Who decides:
- What is blocked until it is decided:
- Where the answer will be recorded:

## Versioning and change policy

- Contract version and where it is recorded:
- What counts as a breaking change:
- Who may freeze or unfreeze, and what record is required (decision log ID, affected tasks, new head SHA):
- How downstream tasks learn about a change:
