# TalkDND Project Charter

Living charter for TalkDND. This file is the project charter AgentOrch looks up at `docs/PROJECT-CHARTER.md`. It is filled in. It has no placeholders.

This document states purpose, audience, goals, non-goals, scope, constraints, success criteria, and decisions. It does not contain game-save contents, DM secrets, or text from rulebooks or adventures.

Player guide: [README.md](../README.md). DM guide: [LetsPlay/AGENTS.md](../LetsPlay/AGENTS.md). Development contract: [AGENTS.md](../AGENTS.md) and [ORCHESTRATION-PROJECT.md](../ORCHESTRATION-PROJECT.md).

## Purpose

TalkDND is an AI dungeon-master play kit for Dungeons & Dragons, plus a Rust combat simulator.

An AI assistant runs the game in chat. It narrates, plays NPCs, and offers choices. The player plays their own character and may type any action. Dice are rolled by the local `combat-sim` program, and the full expression is shown. Combat (attack rolls, damage, conditions, concentration, movement, opportunity attacks) is resolved by that same Rust engine. Play state is written into a local save so a later session can resume.

The repository ships the program, original docs, and SRD-derived combat data. It does not ship the books the player reads.

## Audience

- Players who want an AI DM in a chat window and can run a command-line agent against `LetsPlay/`.
- The same players as rules owners: they supply their own books. The minimum is the System Reference Document for the ruleset they choose.
- Developers who change the engine, the combat data, or the development workflow from the repository root.

TalkDND is an unofficial fan project. It is not affiliated with or endorsed by Wizards of the Coast. "Dungeons & Dragons" and "D&D" are their trademarks.

## Goals

- Run a D&D session in chat: narration, NPCs, choices, free-text actions, and a written save the player can resume.
- Resolve dice and combat with `combat-sim`, a data-driven Rust engine that does not hard-code a campaign.
- Support the 2014 and 2024 rulesets, one ruleset per save, selected by that save's `ruleset` field.
- Keep copyrighted books and adventures, personal saves, and DM secrets out of Git, Issues, pull requests, commit messages, and CI logs.
- Publish prebuilt `combat-sim` binaries for the release targets in this charter, and keep a from-source build for everyone else.
- Record product decisions in this charter so later tasks do not drift from them.

## Non-goals

- Never distribute copyrighted rulebooks or adventures. Players supply their own. The minimum they add locally is the SRD for the chosen ruleset (SRD 5.1 for 2014, SRD 5.2.1 for 2024). Official books, when the player owns them, stay on that player's machine.
- Do not commit save contents, character sheets, session logs, or DM secrets. `LetsPlay/Saves/README.md` is the only save-tree file that belongs in Git.
- Do not mix 2014 and 2024 rules inside one save.
- Do not hard-code a campaign, an adventure, or a monster that is outside the SRD into the engine or into tracked combat data.
- Do not run a second task system. The `local` AgentOrch edition files stay in the tree only because kit validation requires them. They are not a source of truth.
- Do not add a network service, an account system, or a hosted game.

## Scope and layout

Play and development are separate roots.

| Tree | Role |
|------|------|
| `LetsPlay/` | Play. The DM agent starts here. `LetsPlay/AGENTS.md` is the DM guide. Players put books in `LetsPlay/Rules/2014/`, `LetsPlay/Rules/2024/`, and `LetsPlay/Adventures/`. Those directories are local except their README files. Combat data the engine loads lives in `LetsPlay/Rules/combat/`. Saves live in `LetsPlay/Saves/` and are not committed. Launchers live in `LetsPlay/bin/`. |
| Repository root | Development. `AGENTS.md` is the collaboration contract, not the DM guide. Tools live in `tools/combat-sim` (engine) and `tools/talkdnd-dev` (dev tools). AgentOrch files, CI, and this charter stay at the root. |

`tools/bin/combat-sim` is a local binary or a symlink to a local release build. It is not committed. The `LetsPlay/bin` wrappers find it from their own path, so the player's working directory does not matter.

## Supported rulesets

Both rulesets are supported. Each save picks one.

| Ruleset | Save field | Player books (local) | Engine data |
|---------|------------|----------------------|-------------|
| 5e (2014) | `ruleset: "2014"` | `LetsPlay/Rules/2014/`, at least SRD 5.1 | `LetsPlay/Rules/combat/common/` then `LetsPlay/Rules/combat/2014/` |
| 5.5e (2024) | `ruleset: "2024"` | `LetsPlay/Rules/2024/`, at least SRD 5.2.1 | `LetsPlay/Rules/combat/common/` then `LetsPlay/Rules/combat/2024/` |

The field is the save manifest `ruleset`. The engine loads `common/`, then the directory for that value, and ignores the other version directory. Shared rulings are `LetsPlay/Rules/rulings.md`. Version differences are `LetsPlay/Rules/rulings-2014.md` and `LetsPlay/Rules/rulings-2024.md`.

Tracked combat data comes only from the SRD (CC-BY-4.0). Attribution is in `NOTICE`. Data that exists only in a non-SRD book stays in ignored local files.

## Tech stack

Project programs, tools, and scripts are Rust.

- `tools/combat-sim` is the combat engine.
- `tools/talkdnd-dev` is the development tool (leak check and layout migration) and uses only the Rust standard library.
- Rules data is Markdown and YAML, not a third language runtime.

Accepted exceptions, and no others:

1. Vendored, unmodified AgentOrch Python kit scripts. They stay as upstream files and must not be edited: `scripts/orchestrator.py`, `scripts/validate_kit.py`, `tests/test_orchestrator.py`. They use the Python standard library only.
2. Tiny `LetsPlay/bin` launcher wrappers: `LetsPlay/bin/combat-sim` (shell) and `LetsPlay/bin/combat-sim.cmd`. They only locate the repo binary and exec it.
3. CI YAML under `.github/workflows/`, including the shell steps those workflow files contain.

New Rust crates, third-party Python packages, and external services beyond GitHub Actions need human approval before they are added.

`tools/combat-sim/src` stays non-writable. Make a file writable with `chmod u+w` before editing it, verify, then `chmod a-w`. Rebuild the release binary only when `src` changes.

## Privacy rule

Never put save contents, DM secrets, or copyrighted text into Issues, pull requests, commit messages, or CI logs.

The same ban covers chat dumps of those materials, orchestration notes, and backups. Local saves and player books stay untracked.

## Platform targets

Prebuilt `combat-sim` packages are published for these three targets:

| System | Rust target |
|--------|-------------|
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Windows x86_64 | `x86_64-pc-windows-msvc` |
| macOS arm64 (Apple silicon) | `aarch64-apple-darwin` |

There is no prebuilt package for Intel macOS (x86_64). Other machines build `tools/combat-sim` from source. Players do not need a compiler when a prebuilt package matches their system. Developers who run `tools/talkdnd-dev` need Rust.

## Development process

- Orchestration is AgentOrch, **github** edition. Issues are the task list. Evidence is the pull request, CI, and the head SHA. Do not run `python3 scripts/orchestrator.py init`. Do not keep a parallel local task database.
- The Execution Agent is a Grok Build session. It implements one dispatched task in its own worktree on a `task/<ID>-<name>` branch. It does not approve or merge its own work.
- The Master Agent plans, schedules, reviews, and merges. The Master does not implement a task unless the owner writes that exception into the task record.
- Review uses the merge gates in `AGENT-ORCHESTRATION.md`. A green CI run does not replace a criterion-by-criterion review.
- **Living charter.** The Master MUST update `docs/PROJECT-CHARTER.md` (the Decision Log and every affected section) in the same pull request whenever a task changes scope, goals, tech stack, rules support, or a user decision. Reviewers check this at the merge gate. If that update is missing, the review fails and the pull request is not merged.
- Agent bootstrap treats the charter as present when this file exists and has no placeholders. The path is recorded in `AGENTS.md` and `ORCHESTRATION-PROJECT.md`. Vendored kit files are not edited to retarget that lookup.

## Constraints and assumptions

- Platform: a local CLI. Linux, Windows, and macOS are the player platforms. The three prebuilt targets are listed above.
- Data and privacy: player books and saves are local. Tracked combat data is SRD-only. The privacy rule above is mandatory.
- Reliability: dice come from the operating system's entropy through `combat-sim`, not from the model inventing a roll.
- Assumptions: the player can obtain the SRD (or owns the books) and can run an agent that reads `LetsPlay/` and executes the launcher. GitHub is reachable for Issues, pull requests, and CI. One ruleset per save is enough.

## Success criteria

- A player can follow the root README, add their own SRD, start an agent in `LetsPlay/`, and play with real `combat-sim` rolls.
- A save's `ruleset` selects 2014 or 2024 data, and the engine does not load the other version's directory.
- `python3 scripts/validate_kit.py` and `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks` pass on a tree that has no player books and no saves.
- Release automation builds the three prebuilt targets named in this charter.
- A pull request that changes scope, goals, tech stack, rules support, or a user decision also updates this charter in that same pull request.

## Architecture decisions

Current choices. The dated record and the rejected alternatives are in the Decision Log.

- Play files live under `LetsPlay/`. Development files live at the repository root.
- The AgentOrch edition is `github`.
- Implementation language is Rust, plus the three exceptions in Tech stack.
- Prebuilt releases cover Linux x86_64, Windows x86_64, and macOS arm64.
- This file is a living charter, updated in the same pull request as the decision it records.

## Open decisions

None. A new product decision is written into the Decision Log, and into the sections it changes, before a task that depends on it is approved. The Master does not invent an answer in chat and leave this file stale.

## Initial milestone

The smallest end-to-end slice is already the play kit: a player supplies an SRD, starts in `LetsPlay/`, and uses `combat-sim` to roll dice and resolve one combat under a single `ruleset`. This charter is the signed description of that slice. Further work stays inside it until the Decision Log says otherwise.

Validation is the kit check and the leak check in Success criteria, plus the combat-sim and talkdnd-dev test commands in `ORCHESTRATION-PROJECT.md`. Rollback of a charter edit is a normal Git revert of that commit. Rollback of a product change follows the task's own rollback note and, when the decision changes, a new Decision Log entry.

## Decision Log

### 2026-10-20 — LetsPlay layout

Play content lives under `LetsPlay/`. The DM agent starts there and reads `LetsPlay/AGENTS.md`. Development contracts, tools, CI, and this charter stay at the repository root.

Alternative considered: keep play files and the development contract in one root. Rejected so a play session does not load the orchestration rules, and so local books and saves sit in directories the leak check already guards.

### 2026-10-20 — GitHub edition

AgentOrch runs in the `github` edition. Tasks are GitHub Issues. Evidence is the pull request, CI, and the head SHA. `editions/local/` remains in the tree because `scripts/validate_kit.py` requires both editions. It is not a second status source. Do not dual-write. Do not run `python3 scripts/orchestrator.py init`.

Alternative considered: the `local` edition with a SQLite task database. Rejected for this repository. The project owner chose GitHub Issues and CI as the record.

### 2026-10-20 — Rust-only, with named exceptions

All project programs, tools, and scripts are Rust. Markdown and YAML remain the rules data. Three exceptions are accepted and must not grow:

1. Vendored, unmodified AgentOrch Python kit scripts: `scripts/orchestrator.py`, `scripts/validate_kit.py`, `tests/test_orchestrator.py`.
2. Tiny `LetsPlay/bin` shell and `.cmd` launcher wrappers: `LetsPlay/bin/combat-sim` and `LetsPlay/bin/combat-sim.cmd`.
3. CI YAML under `.github/workflows/`, including the shell steps those workflow files contain.

Alternative considered: leave new dev scripts in Python or shell. Rejected. Leak checks and layout migration already live in `tools/talkdnd-dev`. The kit scripts stay only because they are upstream files kept as-is.

### 2026-10-20 — Release targets

Prebuilt `combat-sim` packages are Linux x86_64 (`x86_64-unknown-linux-gnu`), Windows x86_64 (`x86_64-pc-windows-msvc`), and macOS arm64 (`aarch64-apple-darwin`). No Intel macOS prebuilt package is published. Other platforms build from source.

Alternative considered: source-only installs, or an Intel macOS prebuilt package. Source-only was rejected because players should not need a compiler on the three targets above. An Intel macOS prebuilt package was not adopted.

### 2026-10-20 — Living charter

`docs/PROJECT-CHARTER.md` is the project charter. The Master MUST update the Decision Log and every affected section in the same pull request whenever a task changes scope, goals, tech stack, rules support, or a user decision. Reviewers check that update at the merge gate. A missing update fails review.

The charter path is declared in the project docs (`AGENTS.md` and `ORCHESTRATION-PROJECT.md`) so AgentOrch bootstrap resolves "charter missing" against this file. Vendored kit files are not modified to point at it.

Alternative considered: leave scope only inside `ORCHESTRATION-PROJECT.md` and skip a charter. Rejected. AgentOrch expects `docs/PROJECT-CHARTER.md`, and a config file is a poor decision log.

## Change history

| Date | Change |
|------|--------|
| 2026-10-20 | Initial living charter. Added purpose, audience, goals, non-goals, layout, rulesets, tech stack, privacy rule, release targets, development process, and the five decisions above. Pointed project docs at this path. |
