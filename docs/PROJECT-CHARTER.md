# TalkDND Project Charter

Living charter for TalkDND. This file is the project charter AgentOrch looks up at `docs/PROJECT-CHARTER.md`. It is filled in. It has no placeholders.

This document states purpose, audience, goals, non-goals, scope, constraints, success criteria, and decisions. It does not contain game-save contents, DM secrets, or text from rulebooks or adventures.

Player guide: [LetsPlay/README.md](../LetsPlay/README.md). DM guide: [LetsPlay/AGENTS.md](../LetsPlay/AGENTS.md). Development contract: [AGENTS.md](../AGENTS.md) and [ORCHESTRATION-PROJECT.md](../ORCHESTRATION-PROJECT.md). The root [README.md](../README.md) is for developers.

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
- Let the DM leave a published adventure only within a per-save, creation-locked deviation scope and ratio, with every original element registered in a DM-only ledger.
- Resume a campaign across sessions with a recoverable state history (checkpoints and structured diffs), stable entity and relation memory, optional in-world time, and fast local navigation of the player's own books, all inside the existing save and under the same privacy and spoiler discipline.

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
| `LetsPlay/` | Play. The DM agent starts here. `LetsPlay/README.md` is the player guide. `LetsPlay/AGENTS.md` is the core DM guide (command table first, under 150 lines); details live in `LetsPlay/docs/*.md`, read on demand. Players put book PDFs in `LetsPlay/Rules/2014/`, `LetsPlay/Rules/2024/`, and `LetsPlay/Adventures/`, one folder per book (`<slug>/` with the PDF, its converted `index/` and, for adventures, `combat/`; `talkdnd start` tidies loose files into place). Those directories are local except their README files. Combat data the engine loads lives in `LetsPlay/Rules/combat/`. Saves live in `LetsPlay/Saves/` and are not committed. The programs they run are the gitignored binaries `LetsPlay/bin/combat-sim` and `LetsPlay/bin/talkdnd` (Windows: `.exe`). |
| Repository root | Development only. `AGENTS.md` is the collaboration contract, not the DM guide. Tools live in `tools/combat-sim` (engine), `tools/talkdnd` (play helper) and `tools/talkdnd-dev` (dev tools). AgentOrch files, CI, and this charter stay at the root. Nothing required to play lives here. |

Release archives contain the whole git-tracked `LetsPlay/` tree plus the binary at `LetsPlay/bin/`; extracting one gives a ready play folder. `build-artifacts` artifacts contain only the binary at the archive root; extracting one into `LetsPlay/bin/` is the install. No launcher script points outside `LetsPlay/`.

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
- `tools/talkdnd` is the play helper: resume digest, new save, character creation checks, random companions, checkpoints. It rolls through the combat-sim dice engine (library dependency) so all dice stay unified.
- `tools/talkdnd-dev` is the development tool (leak check, LetsPlay path check, layout migration, binary install) and uses only the Rust standard library.
- Rules data is Markdown and YAML, not a third language runtime.

Accepted exceptions, and no others:

1. AgentOrch Python kit scripts, maintained as part of this project: `scripts/orchestrator.py`, `scripts/validate_kit.py`, `tests/test_orchestrator.py`. They use the Python standard library only and run through `uv run --no-project`.
2. CI YAML under `.github/workflows/`, including the shell steps those workflow files contain.
3. `LetsPlay/tools/pdf2md/`, the Python PDF → Markdown extractor needed for play (decision "pdf2md" below).

New Rust crates, third-party Python packages, and external services beyond GitHub Actions need human approval before they are added.

`tools/combat-sim/src` stays non-writable. Make a file writable with `chmod u+w` before editing it, verify, then `chmod a-w`. When `src` is unchanged, install the prebuilt binary with `talkdnd-dev fetch-binary`. Rebuild and copy into `LetsPlay/bin/` only when `src` changes (`talkdnd-dev install-local`).

## Privacy rule

Never put save contents, DM secrets, or copyrighted text into Issues, pull requests, commit messages, or CI logs.

The same ban covers chat dumps of those materials, orchestration notes, and backups. Local saves and player books stay untracked.

## Platform targets

Prebuilt packages (`combat-sim` and `talkdnd`) are published for these three targets:

| System | Rust target |
|--------|-------------|
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Windows x86_64 | `x86_64-pc-windows-msvc` |
| macOS arm64 (Apple silicon) | `aarch64-apple-darwin` |

There is no prebuilt package for Intel macOS (x86_64). Other machines build `tools/combat-sim` from source and copy the binary into `LetsPlay/bin/`. Players do not need a compiler when a prebuilt package matches their system. Developers who run `tools/talkdnd-dev` need Rust.

## Development process

- Orchestration is AgentOrch, **github** edition. Issues are the task list. Evidence is the pull request, CI, and the head SHA. Do not run `uv run --no-project python scripts/orchestrator.py init`. Do not keep a parallel local task database.
- The Execution Agent is a Grok Build session. It implements one dispatched task in its own worktree on a `task/<ID>-<name>` branch. It does not approve or merge its own work.
- The Master Agent plans, schedules, reviews, and merges. The Master does not implement a task unless the owner writes that exception into the task record.
- Review uses the merge gates in `AGENT-ORCHESTRATION.md`. A green CI run does not replace a criterion-by-criterion review.
- **Living charter.** The Master MUST update `docs/PROJECT-CHARTER.md` (the Decision Log and every affected section) in the same pull request whenever a task changes scope, goals, tech stack, rules support, or a user decision. Reviewers check this at the merge gate. If that update is missing, the review fails and the pull request is not merged.
- Agent bootstrap treats the charter as present when this file exists and has no placeholders. The path is recorded in `AGENTS.md` and `ORCHESTRATION-PROJECT.md`. Vendored kit files are not edited to retarget that lookup.

## Constraints and assumptions

- Platform: a local CLI. Linux, Windows, and macOS are the player platforms. The three prebuilt targets are listed above.
- Data and privacy: player books and saves are local. Tracked combat data is SRD-only. The privacy rule above is mandatory.
- Reliability: dice come from the operating system's entropy through `combat-sim`, not from the model inventing a roll.
- Assumptions: the player can obtain the SRD (or owns the books) and can run an agent that reads `LetsPlay/` and executes `LetsPlay/bin/combat-sim`. GitHub is reachable for Issues, pull requests, and CI. One ruleset per save is enough.

## Success criteria

- A player can follow `LetsPlay/README.md`, add their own SRD, start an agent in `LetsPlay/`, and play with real `combat-sim` rolls from `LetsPlay/bin/combat-sim`.
- A save's `ruleset` selects 2014 or 2024 data, and the engine does not load the other version's directory.
- `uv run --no-project python scripts/validate_kit.py`, `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks`, and `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay` pass on a tree that has no player books and no saves.
- Release automation builds the three prebuilt targets named in this charter.
- A pull request that changes scope, goals, tech stack, rules support, or a user decision also updates this charter in that same pull request.
- Once v0.12 ships, a session interrupted mid-play can be resumed from the last good checkpoint, and the structured diff since the previous session is available without rereading the whole log.

## Architecture decisions

Current choices. The dated record and the rejected alternatives are in the Decision Log.

- Play files live under `LetsPlay/`. Development files live at the repository root.
- The AgentOrch edition is `github`.
- Implementation language is Rust, plus the two exceptions in Tech stack.
- The play directory is self-contained. The binary lives in `LetsPlay/bin/` and is not committed.
- Prebuilt releases cover Linux x86_64, Windows x86_64, and macOS arm64.
- This file is a living charter, updated in the same pull request as the decision it records.
- Adventure deviation is per save, chosen with `talkdnd new --scope/--ratio`, stored in `save-meta.yaml`, and cannot be changed after creation. Original content is registered in the DM-only `original.yaml` before it is used, and its type, rarity and chapter budget are checked by the tool.
- The campaign-continuity data contract is `LetsPlay/docs/campaign-continuity.md`. It is the single authority matrix for the P00–P08 program; implementation tasks do not drift from it.
- The main save schema rises to 8 only in P03. All other new save state is versioned sidecars (`memory.yaml`, `entities.yaml`, `relations.yaml`, `world-time.yaml`, `checkpoints/*/manifest.yaml`, `index/structured.json`): a missing file means the feature is off, a newer version is refused.
- Checkpoints are the DM-local recovery path, not a player-facing view. `checkpoint snapshot` rewinds mechanical state plus revealed/identified knowledge and nothing else (never append-only logs, DM working files, derived files, or binary handout exports); `checkpoint restore` backs the whole save up before replacing anything, appends a restore line to the session log without truncating history, refreshes the integrity manifest and audit baseline, migrates a snapshot whose schema is older than the current main schema and refuses a newer one, and keeps the last ten; `snapshot diff` reports structured changes only.
- World time is per save, off by default, and moves only on an explicit command. No narration or scene change advances it.
- The Oracle produces inspiration only. It writes no save file, never invents a roll, and resolves every die through the combat-sim dice engine.
- The live encounter state `Saves/<slot>/combat/*.state.json` is authoritative save data, not merely replayable scratch: it is covered by the integrity manifest when present, every normal combat-sim write refreshes the baseline on success (a failed write never advances it), a missing state file is not an error, and hand edits are reported and accepted like any other tracked file.
- Tool proficiency checks read proficiency only from the structured `Saves/<slot>/sheets/<id>.yaml` `tools` list, never from Markdown prose or the tool name. The checked ability is a mandatory explicit `--ability`; an unproficient or unknown but well-formed tool still rolls with only the ability modifier (plus explicit extras) and is marked not proficient, and the proficiency bonus comes from the combat block's `prof` (derived from `level` when absent).

## Open decisions

None. A new product decision is written into the Decision Log, and into the sections it changes, before a task that depends on it is approved. The Master does not invent an answer in chat and leave this file stale.

## Initial milestone

The smallest end-to-end slice is already the play kit: a player supplies an SRD, starts in `LetsPlay/`, and uses `combat-sim` to roll dice and resolve one combat under a single `ruleset`. This charter is the signed description of that slice. Further work stays inside it until the Decision Log says otherwise. The 2026-10-24 Decision Log authorizes the P00–P08 campaign-continuity program as the next scope, shipping in v0.12–v0.14.

Validation is the kit check and the leak check in Success criteria, plus the combat-sim and talkdnd-dev test commands in `ORCHESTRATION-PROJECT.md`. Rollback of a charter edit is a normal Git revert of that commit. Rollback of a product change follows the task's own rollback note and, when the decision changes, a new Decision Log entry.

## Decision Log

### 2026-10-24 — Tool proficiency checks (TASK-0068)

`talkdnd check` supports tool proficiency checks: `talkdnd check <who> tool:<name> --ability <str|dex|con|int|wis|cha> [--adv|--dis] [--extra N] [--dc N]`. Proficiency is read only from the structured sheet data — the `tools` list of `Saves/<slot>/sheets/<id>.yaml`; a missing or tool-less structured file means nothing is proficient. `--ability` is mandatory and is never inferred from the tool name. A proficient tool adds the combat block's proficiency bonus (`prof`, derived from `level` when absent) on top of the ability modifier; an unproficient tool still rolls, adds only the ability modifier and any explicit extras, and the output and session log explicitly say `not proficient (no proficiency bonus added)`. An unknown but well-formed tool name is not silently treated as proficient: it takes the same unproficient path. Tool names match case-, space-, apostrophe- and hyphen-insensitively with a small alias table, and malformed arguments are clear errors. `LetsPlay/docs/dice.md` documents the form in the same pull request.

Alternative considered: infer the checked ability from the tool name (e.g. thieves' tools → dex). Rejected: the same tool is legitimately used with different abilities in different scenes, so a name-based mapping would silently produce wrong modifiers; the issue requires the ability to be user-specified.

### 2026-10-24 — Live combat state is authoritative and integrity-tracked (TASK-0066)

The live encounter state files `Saves/<slot>/combat/*.state.json` are now treated as authoritative save data, not merely replayable scratch: once a fight is under way, the JSON on disk is the record of that fight. They join the integrity manifest when present (a save without state files reports nothing), and every normal combat-sim write of a state file — creating or updating — refreshes the integrity baseline on success, so the tool's own writes are never reported as hand edits. A failed write leaves the baseline untouched, and states written outside a save directory stay untracked scratch. Hand edits to a tracked state file are reported like any other save file and accepted with `talkdnd integrity accept --reason`. `LetsPlay/docs/table-integrity.md` and the authority matrix plus integrity rules in `LetsPlay/docs/campaign-continuity.md` are updated in the same pull request.

Alternative considered: keep `*.state.json` out of integrity because an encounter can be re-simulated from `campaign.yaml`. Rejected: replaying is a DM convenience, not a substitute for the file of record; the mid-fight state the tools write is exactly what the integrity system exists to protect, and excluding it would let manual edits to a live fight go unnoticed while making every legitimate engine write a false alarm once tracking existed without baseline maintenance.

### 2026-10-24 — Campaign-continuity contract clarification (TASK-0056 review)

Master review of the frozen P00 contract (`LetsPlay/docs/campaign-continuity.md`) found one contradiction and two undefined restore edges, plus an unclear visibility statement; the contract is amended in place, in the same file and the same pull request. `identified.yaml` joins the checkpoint copy set (it is small and must stay consistent with `reveals.yaml`), so a checkpoint rewinds mechanical state plus revealed/identified knowledge — never append-only logs (`Sessions/`, `rolls.log`), DM working files, derived files, or binary handout exports — and checkpoints themselves are DM-local recovery data, not a player-facing view. On restore: a snapshot whose `schema` is older than the current main schema is restored and then migrated normally (whole-save backup plus session-log line), a newer one is refused; after a restore the current session log gains a restore line and is never truncated, and the integrity manifest and audit baseline are refreshed so `talkdnd check` stays clean.

Alternative considered: keep `identified.yaml` out of checkpoints and rebuild identification from `reveals.yaml` after a restore. Rejected: `identified.yaml` is its own versioned sidecar with DM-only appearance overrides; rebuilding would silently drop state and break the one-writer matrix.

### 2026-10-24 — Campaign continuity program P00–P08

The owner approved the adjusted P00–P08 program. P00 freezes the data contract in `LetsPlay/docs/campaign-continuity.md`: an authority matrix that gives every existing save store exactly one writer, the exact formats (fields, versions, defaults, corruption behavior) for the new sidecars (`checkpoints/`, `memory.yaml`, `entities.yaml`, `relations.yaml`, `world-time.yaml`, `index/structured.json`), the session and in-world time model, the P01–P08 command surface, visibility rules, non-goals, and success metrics. The former P02 (snapshot diff) is folded into P01 (checkpoints and restore): both need the same snapshot machinery and both touch `start.rs`. The former P05 (relation graph) is folded into P04 (entities): relations reference entity ids. P09 (second-screen display and player input devices) is removed from the roadmap; it would introduce a network service and needs its own charter change and security assessment before any implementation.

Schema strategy: the main save schema (`save-meta.yaml` `schema`, currently 7) rises to 8 only in P03 (per-save DM style settings), with the usual whole-save backup migration. Every other new file is a versioned sidecar: a missing file means the feature is off, a newer version is refused. Save integrity covers the new sidecars when they exist but never `Sessions/` or checkpoint data copies. World time is off by default and advances only on an explicit command; the Oracle produces inspiration only, writes no authoritative state, and rolls only through the combat-sim dice engine.

Release slices: v0.12 = P00 + P00a + P00b + P01 + P07 (reliability and navigation); v0.13 = P03 + P04 (memory and entities); v0.14 = P06 + P08 (world time and DM aids).

Alternative considered: keep the former P01–P09 split and add P00 as a standalone design phase. Rejected: the fold-ins remove two migration windows and two `lib.rs` conflicts, and a network-service feature must not ride along on an approved reliability program.

### 2026-10-24 — Play-time page fixes and split-volume consolidation (v0.11.0)

Two play-table gaps. First, when converted Markdown contradicts the PDF mid-session, the DM can fix that one page immediately: `pdf2md fix` writes `index/fixes/pNNNN.md` with the PDF sha256 and rebuilds the book when no worker is running; `--no-apply` defers the rebuild. The PDF stays authoritative, text is not patched from memory, and the fix stays local. Second, `Adventures/` may hold both a full book and its split volumes (`<slug>-a`, `-b`, `-2`, ...). `talkdnd library merge <slug>` archives the parts under `.talkdnd/book-backups/merge-<slug>/` (never deletes), moves a part's `combat/` only when the full book has none, and repoints save paths to the full book. `library tidy` only warns and prints the command; it never moves parts by itself.

Alternative considered: automatic merging whenever a new PDF arrives. Rejected: deciding that two PDFs are the same book, and choosing which content survives, is a data-loss risk; the explicit command with a backup keeps the owner in control.

### 2026-10-20 — LetsPlay layout

Play content lives under `LetsPlay/`. The DM agent starts there and reads `LetsPlay/AGENTS.md`. Development contracts, tools, CI, and this charter stay at the repository root.

Alternative considered: keep play files and the development contract in one root. Rejected so a play session does not load the orchestration rules, and so local books and saves sit in directories the leak check already guards.

### 2026-10-20 — GitHub edition

AgentOrch runs in the `github` edition. Tasks are GitHub Issues. Evidence is the pull request, CI, and the head SHA. `editions/local/` remains in the tree because `scripts/validate_kit.py` requires both editions. It is not a second status source. Do not dual-write. Do not run `uv run --no-project python scripts/orchestrator.py init`.

Alternative considered: the `local` edition with a SQLite task database. Rejected for this repository. The project owner chose GitHub Issues and CI as the record.

### 2026-10-20 — Rust-only, with named exceptions

All project programs, tools, and scripts are Rust. Markdown and YAML remain the rules data. Two exceptions are accepted and must not grow:

1. AgentOrch Python kit scripts: `scripts/orchestrator.py`, `scripts/validate_kit.py`, `tests/test_orchestrator.py` (maintained in-project since 2026-10-21).
2. CI YAML under `.github/workflows/`, including the shell steps those workflow files contain.

The launcher-wrapper exception from the first draft of this decision was removed the same day. See "Play directory is self-contained" below.

Alternative considered: leave new dev scripts in Python or shell. Rejected. Leak checks and layout migration already live in `tools/talkdnd-dev`. The kit scripts stay only because they are upstream files kept as-is.

### 2026-10-20 — Release targets

Prebuilt `combat-sim` packages are Linux x86_64 (`x86_64-unknown-linux-gnu`), Windows x86_64 (`x86_64-pc-windows-msvc`), and macOS arm64 (`aarch64-apple-darwin`). No Intel macOS prebuilt package is published. Other platforms build from source.

Alternative considered: source-only installs, or an Intel macOS prebuilt package. Source-only was rejected because players should not need a compiler on the three targets above. An Intel macOS prebuilt package was not adopted.

### 2026-10-20 — Living charter

`docs/PROJECT-CHARTER.md` is the project charter. The Master MUST update the Decision Log and every affected section in the same pull request whenever a task changes scope, goals, tech stack, rules support, or a user decision. Reviewers check that update at the merge gate. A missing update fails review.

The charter path is declared in the project docs (`AGENTS.md` and `ORCHESTRATION-PROJECT.md`) so AgentOrch bootstrap resolves "charter missing" against this file. Vendored kit files are not modified to point at it.

Alternative considered: leave scope only inside `ORCHESTRATION-PROJECT.md` and skip a charter. Rejected. AgentOrch expects `docs/PROJECT-CHARTER.md`, and a config file is a poor decision log.

### 2026-10-20 — Play directory is self-contained

Everything needed to play lives under `LetsPlay/`. The repository root holds development content only: source, development docs, CI, and the AgentOrch kit.

The program a player runs is the binary `LetsPlay/bin/combat-sim` (Windows: `LetsPlay/bin/combat-sim.exe`). It is gitignored. There is no shell or `.cmd` launcher, and no tracked file under `LetsPlay/` may reference a path that leaves that directory. `talkdnd-dev check-letsplay` fails the build when one does. A save manifest's `../../../Rules` path, resolved from `Saves/<slot>/combat/`, stays inside `LetsPlay/` and passes.

The `build-artifacts` artifacts contain the binary at the archive root. Extracting one into `LetsPlay/bin/` installs it (release archives now ship the whole folder; see "Release bundles the play folder"). `talkdnd-dev fetch-binary` installs the same file. A local release build is copied there by `talkdnd-dev install-local` (`cargo build --release`, then copy). The Rust-only exception list no longer includes launcher wrappers.

Alternative considered: keep thin `LetsPlay/bin` wrappers that exec a binary under `tools/`. Rejected. A play session must not depend on a path outside `LetsPlay/`.

### 2026-10-20 — Release bundles the play folder

Each GitHub Release asset (`talkdnd-letsplay-<tag>-<target>`, `.zip` on Windows, `.tar.gz` elsewhere, each with `.sha256`) is the git-tracked `LetsPlay/` tree plus `LICENSE`, `NOTICE`, and the prebuilt binary at `LetsPlay/bin/`. CI stages it from `git archive` only and fails if the archive holds anything outside that list, any PDF, any `dm-secrets.md`, or anything under `Saves/`, `Rules/2014/`, `Rules/2024/`, or `Adventures/` besides their README. `v0.1.0` is the first release. `fetch-binary` still installs the binary alone from `build-artifacts`.

Alternative considered: commit binaries into `LetsPlay/bin/`, or ship the binary alone. Rejected: committed binaries bloat history; a binary-only asset leaves players to assemble the folder.

### 2026-10-20 — One dice path for play

Every die rolled in play goes through `bin/combat-sim roll` (or a combat command that uses the same engine). The DM guide forbids mental, invented, or other-RNG rolls and requires every roll, as printed, in the session log. The Python fallback was removed from play docs. `talkdnd-dev check-letsplay` now fails if a tracked play file names another dice source.

The `roll` grammar is: terms joined by `+`/`-`; dice `NdS` with optional `khN`, `klN`, `dhN`, `dlN`; `d%` for d100; flat integers. Accepted dice are d4, d6, d8, d10, d12, d20, d100 and d2/d3 (which the rules mention); others are rejected. A term has 1 to 100 dice, an expression at most 20 terms. `--adv`/`--dis` apply only to one 1d20 plus flat modifiers. `--crit` doubles the dice count of every damage term and adds modifiers once (identical in 2014 and 2024); it rejects d20 and keep/drop terms. `--dc` and `--json` work on every expression. Dropped dice print in brackets. The data-file `DiceExpr` and its output are unchanged. The owner approved this change to the read-only combat-sim source.

Alternative considered: allow any die size. Rejected: no D&D rule rolls a d7, and rejecting it catches typos.

### 2026-10-20 — Book is canon, presentation is improvised

The owner wants the adventure book followed faithfully for facts, structure, and mechanics (plot beats, NPC motives and secrets, clues, maps, stat blocks, DCs) while scenes, dialogue, NPC personality, and reactions are improvised rather than read aloud. The DM guide (`LetsPlay/AGENTS.md` section 5) now says so: paraphrase boxed text and never recite it; react to player choices through world logic; invent minor details freely and record them so they become canon (public ones in `Saves/<slot>/canon.md` or `NPCs/` (story NPCs), hidden ones in `dm-secrets.md`); keep improvisation consistent with book secrets; give each companion a distinct voice; manage pacing; end scenes with suggested options plus free input; follow the save's ruleset strictly and rule only where the rules are silent; use the adventure PDF maps. The guidance names no particular agent.

Alternative considered: read boxed text verbatim. Rejected by the owner, and it would put copyrighted text into session logs.

### 2026-10-21 — pdf2md: background PDF → Markdown, PDF authoritative

Players only add PDFs to `LetsPlay/Rules/2014/`, `LetsPlay/Rules/2024/` and `LetsPlay/Adventures/`. `LetsPlay/tools/pdf2md` (Python, a scoped exception to the Rust-only rule) converts them incrementally in the background into a hidden `.md/<PDF stem>/` folder next to each PDF. That folder holds `<PDF stem>.md` with PDF-page anchors, `index.md`/`index.json`, `manifest.json`, chunk files and page images. Work is done in 10-page chunks with a SHA-256 manifest, a single-worker lock, a time or page budget, and low CPU priority. Order: the current save's adventure, then that ruleset's SRD, then its other books, then other adventures, then the other ruleset. Pages without a text layer are OCR'd. RapidOCR (rapidocr-onnxruntime) is the default engine and PaddleOCR 3.x is an optional extra (`--ocr-engine paddle`). Repair passes are ported from the owner's ConvertPDF under MIT (credited in `NOTICE`); book-specific corrections were dropped. Stat blocks are not rebuilt. Their pages are flagged low quality and the DM reads the PDF, as for maps (linked page images, never embedded). Outputs, state and environments are gitignored, and `check-leaks` fails if they are tracked. Old hand-made `.md` files stay for now (superseded 2026-10-23: one folder per book, hand-made texts deleted, see below).

Dependencies are not bundled. PyMuPDF and pymupdf4llm (AGPL-3.0, pinned 2.0; chosen over 0.3.x because 0.3.x lost table structure on SRD tables) and RapidOCR are installed by `uv sync`. The DM guide tells any agent to install uv and run `uv sync` plus `pdf2md setup` on first use.

Box benchmark on five scanned 2014 PHB pages rendered at 200 dpi, CPU only:
- **RapidOCR:** 2.1 s/page, 99.0% word-list hits.
- **PaddleOCR 3.3** (mobile PP-OCRv5, oneDNN off because it crashes): 23 s/page, 99.3% hits.
- **Environment size:** about 0.5 GB with RapidOCR, about 1.5 GB with Paddle.

RapidOCR stays the default, and CI tests only RapidOCR.

Alternative considered: one-off whole-book conversion like ConvertPDF. Rejected: it blocks play and cannot resume.

### 2026-10-21 — All Python through uv

Every Python invocation goes through uv: `uv sync` / `uv run --project <dir>` for projects (pdf2md has `pyproject.toml` and `uv.lock`), and `uv run --no-project python scripts/...` for the stdlib AgentOrch kit scripts. CI uses `astral-sh/setup-uv`. Raw `pip`, `python -m venv` and bare `python3` are not used in docs or CI.

### 2026-10-21 — AgentOrch is part of the project; uv everywhere

AgentOrch is no longer treated as a vendored, read-only kit. Its docs (`AGENT-BOOTSTRAP.md`, `AGENT-ORCHESTRATION.md`, `editions/`, `prompts/`, `templates/`) and scripts may be changed through normal tasks. Every Python command in them is `uv run --no-project python scripts/...`. `validate_kit.py` skips local-only trees: `.venv`, `.worktrees`, pdf2md `.md/` output, player books and saves. CI runs the kit's unit tests through uv.

Alternative considered: keep the kit read-only and only document uv at the project level. Rejected by the owner.


### 2026-10-21 — 2014 combat monsters and party spells

`LetsPlay/Rules/combat/2014/monsters/` now holds SRD 5.1 creatures needed for keyed adventures that share bases with the 2024 set: Guard, Animated Armor, Mimic (tracked). Allosaurus and Scarecrow are **not** in SRD 5.1, so their 2014 YAML stays local and gitignored (same pattern as 2024 Scarecrow). Edition notes: 2014 Mimic speed 15 ft; 2014 Animated Armor lists Blinded among condition immunities; Scarecrow (local) uses MM 2014 numbers (HP 36, Multiattack, claw WIS save frighten, nonmagical B/P/S resistance).

2014 `spells.yaml` gains Guidance, Mending, Healing Word, Command, and Mage Armor from SRD 5.1 so party casters resolve those refs. `common/ai-templates.yaml` gains a `support` template (buff/heal ally bias) usable by both rulesets.

Alternative considered: commit Allosaurus under 2014 because 2024 already tracks it (SRD 5.2.1). Rejected: 2014 tracked data must stay SRD 5.1 only.


### 2026-10-21 — pdf2md review soft gate and local snippets

Pilot rejects on cleaned-up stat-block pages were mostly tiny recall dips (about 0.01–0.05) while unknown/1k improved. The gate now allows that soft recall drop when unknown/1k improves, words stay ≥ 90% of before, badness does not worsen, and the page has a `statblock` flag or structure markers; unknown/1k rising or words below 50% still hard-reject. Grok Build max-turns default is 60, with one short-prompt retry on max-turns. Prompts inject name-matched snippets from converted local SRD Markdown (web search stays off). `pdf2md review --rejudge` promotes prior soft rejects without an agent call. Thresholds and pilot numbers live in `LetsPlay/tools/pdf2md/docs/LESSONS.md` (no book text).

### 2026-10-22 — Per-save adventure deviation and original-content ledger

Each save locks how far the DM may leave the published adventure. `talkdnd new` asks for or accepts `--scope` (0 = gap filling only, 1 = minor original encounters/NPCs/items/locations, 2 = also side quests) and, for scope 1/2, `--ratio` (5–25%). The pair is stored as `deviation` in `Saves/<slot>/save-meta.yaml`, schema 4, and there is deliberately no setter command. Old saves migrate to scope 0 with the usual whole-save backup and a session-log line.

Original content goes into the DM-only `Saves/<slot>/original.yaml` before it appears in play (`talkdnd original add/list/status`). Types are gated by scope; rarity is capped by both the scope and the party level (1–4 uncommon, 5–10 rare, 11–16 very rare, 17+ legendary), taking the lower cap. The per-chapter budget is `original / (original + canon progress) <= ratio`, checked after adding; canon progress counts `xp_log` entries and revealed facts since the last major milestone (all campaign units when there is none). Going over refuses the registration and tells the DM to return to the main line. Key anchors — main villain, core secrets, ending direction — never change at any scope. Main-line player freedom is unaffected, adopted player hooks count as original, and players are never told which elements are original. `talkdnd start` shows the DM the scope, ratio, caps and budget; `talkdnd check` reports manual edits to `save-meta.yaml`/`original.yaml`, out-of-scope ledger entries, and inventory items marked original without a ledger entry.

Alternative considered: a global setting or a setter command. Rejected because deviation is a deliberate per-campaign decision and must not be relaxed mid-play.

### 2026-10-23 — One folder per book, one paged Markdown edition (v0.10.0)

The owner found `Adventures/` and `Rules/` mixed: some books were loose PDFs with a hidden `.md/<stem>/` next to them, one adventure had its own folder, and hand-made Markdown files sat beside them. The owner asked for one standard and for stray books to be moved. Decisions:

- **One folder per book.** `Adventures/<slug>/` holds the PDF, the pdf2md output in `index/`, and `combat/`. `Rules/<edition>/<slug>/` holds the PDF and `index/`. Program data (`Rules/chargen/`, `Rules/combat/`, `Rules/defaults/`, `Rules/rulings*.md`) stays outside the book folders.
- **Deterministic slug** shared by `talkdnd` and pdf2md: ASCII letters and digits lowercased, `'`/`’` dropped, every other run becomes one `-`, ends trimmed; no ASCII letter or digit gives `book-<FNV-1a 32 hex>`.
- **Automatic tidy** on every `talkdnd start` and on `talkdnd library tidy [--dry-run]`: loose or mis-named PDFs and their legacy `.md/<stem>/` move into place, every move is reported and logged in `.talkdnd/library-moves.yaml`, rulebooks of unknown edition are only warned about, book files are never deleted, a collision keeps both with a warning, and a second run changes nothing. It does nothing while pdf2md is converting.
- **One Markdown edition.** pdf2md 0.6.0 writes only `<stem>.pages.md` (PDF page anchors). The page-less reader copy is deleted where the paged edition exists, by tidy (bytes reported) and by the pdf2md upgrade, with `index.md`/`index.json` repointed. The upgrade from 0.5.0 redoes no chunks; readers still accept the old layout, so nothing is reconverted.
- **Saves:** schema 6 → 7 rewrites book paths in save text files to the new layout, with a full backup and integrity update; later tidy moves are applied to saves at `start`.
- **Hand-made Markdown** copies of books are not a supported format. The owner's old hand-converted texts were deleted after checking nothing depended on them; tidy warns about any new one, and the READMEs only ask for PDFs.
- **Rulings:** `Rules/defaults/rulings*.md` are the shipped defaults (overwritten by updates, not read at the table); `Rules/rulings*.md` are the DM's copies and the only ones that count. An unedited copy follows the default automatically; an edited one is kept with a merge hint.

Alternatives considered: no folders at all (rejected: `combat/` and the converted index need a per-book home, and multi-PDF books would collide); keeping both Markdown editions (rejected by the owner: same text twice, about 18 MB on the owner's library); moving hand-made texts into book folders (rejected by the owner: they duplicate the PDF and the paged index).

### 2026-10-23 — Companions are party members (v0.9.0)

The owner noticed that DM-run companions were stored in `Saves/<slot>/NPCs/` (non-playable characters), which misdescribes them: they are party members. All party members now live in `Characters/`, and `sheets/<id>.yaml` carries `controller: player` or `controller: dm`, shown on the rendered sheet. `NPCs/` is kept for story NPCs only. `talkdnd companion random|sheet` replaces `talkdnd npc …` (the old name still works with a note). Save schema 6 moves companion sheets and `_party.md` on `start`/`migrate` after the usual whole-save backup, edits `sheets/*.yaml` textually so nothing is lost, keeps both files on a name clash, and carries pre-existing hand-edit flags to the new paths.

### 2026-10-23 — Information discipline, secret guard and dice display (v0.8.0)

The DM now reads converted adventures one heading at a time (`talkdnd adventure section`; future/DM-only sections need a logged `--reason`), builds narration and options only from `talkdnd known`, and checks drafts with `talkdnd guard` against local secret terms (`dm-secrets.md`, `dm-plan.md`, future chapter headings, original `--hidden`, manual terms). Unidentified monsters/items are shown to players by appearance (`talkdnd identify`, combat-sim `--player`), while DM views keep real names. Per-save `settings.dice_display` (default on, changeable mid-game and logged) controls whether dice values, DCs and modifiers are spoken; every roll stays program-made and lands in `rolls.log`. `LetsPlay/AGENTS.md` stays at 150 lines; details live in `option-writing.md`, `anti-spoiler.md` and `dice.md`.

Alternative considered: rely on DM discipline alone. Rejected: the new tools make the information boundary checkable without blocking play, and the DM can still proceed with a logged reason where a scene must be read ahead.

## Change history

| Date | Change |
|------|--------|
| 2026-10-20 | Initial living charter. Added purpose, audience, goals, non-goals, layout, rulesets, tech stack, privacy rule, release targets, development process, and the five decisions above. Pointed project docs at this path. |
| 2026-10-20 | Play directory is self-contained. The binary lives in `LetsPlay/bin/` and is gitignored. Launcher wrappers are gone, so the Rust-only exception list is the kit scripts and CI YAML. |
| 2026-10-20 | Release assets bundle the tracked `LetsPlay/` folder with the binary, guarded by a CI content gate. First release `v0.1.0`. |
| 2026-10-20 | Dice: one roll path through `combat-sim roll`, extended grammar, check-letsplay dice-source lint. |
| 2026-10-20 | DM guidance: book is canon for facts and mechanics, presentation improvised; `canon.md` added to the save layout. |
| 2026-10-21 | pdf2md background extractor (Python exception, OCR, uv); all Python via uv. |
| 2026-10-21 | AgentOrch kit maintained in-project; all kit commands via `uv run`. |
| 2026-10-21 | pdf2md audit + local page-fix layer + optional agent review (Grok Build on the box; agent-agnostic), accepted under the LESSONS soft/hard gate. |
| 2026-10-21 | pdf2md review soft gate, max-turns retry, local SRD snippet injection, `--rejudge`. |
| 2026-10-21 | 2014 `monsters/` (SRD Guard/Animated Armor/Mimic), local Allosaurus/Scarecrow gitignore, five SRD spells, `support` AI template. |
| 2026-10-21 | `talkdnd` play helper (start digest, new save, chargen, npc, checkpoint, log) built and shipped with combat-sim; release assets use plain platform names (`linux-x86_64`, `windows-x86_64`, `macos-arm64`). |
| 2026-10-21 | DM guide split: short `LetsPlay/AGENTS.md` (player commands 开新局 / 继续游戏 / 存档, mandatory step-by-step character creation, core rules) plus `LetsPlay/docs/`. No CLAUDE.md / GEMINI.md pointers (current agents read AGENTS.md). |
| 2026-10-21 | chargen validation: adventure start-level step, 2014 level-1 subclass rule, armor proficiency from class+subclass, cantrip/spell count checks, species traits in the sheet and combat yaml. |
| 2026-10-22 | Per-save adventure deviation (scope 0–2, ratio 5–25%) locked in `save-meta.yaml` schema 4; DM-only `original.yaml` ledger with scope/rarity/chapter-budget checks; `char item add` source check; `start`/`check` surfacing; v0.7.0. |
| 2026-10-23 | v0.8.0 information discipline: scene-scoped reading, known list, secret terms/guard, hidden original aspects, player-facing name masking, option-writing rules, per-save dice display and roll log; schema 5. |
| 2026-10-23 | v0.9.0 party companions live in `Characters/` next to the PC, told apart by `controller: player|dm` in `sheets/<id>.yaml`; `talkdnd companion`; `NPCs/` is for story NPCs only; schema 6 migration. |

### 2026-10-21 — pdf2md audit, page fixes and agent review

`pdf2md audit` ports the owner's evaluation method into the tool: each page is scored against the PDF text layer (word recall, extra words and unknown words per 1000 via the word list, body-like headings, table structure, empty/repeated/interleaved text, low-quality flags, and `inferred` / `check PDF` review markers), written worst-first to `.md/<stem>/audit.json`. Corrections live in `.md/<stem>/fixes/pNNNN.md` (header records the PDF SHA-256 and pdf2md version) and replace that page whenever the book is assembled, so they survive reconversion; a fix for a different PDF hash is ignored. `pdf2md review` hands an agent the page Markdown, the text layer and a rendered image with strict rules (fix OCR/spacing, restore missing text, rebuild tables and stat blocks, keep anchors, add nothing not on the page). For uncertain spots the agent first consults the page image, the converted SRDs and other local books, and cross-references in the same book; if still unresolved it infers from context and marks `<!-- inferred: reason -->`, using `<!-- check PDF -->` only for truly unreadable text. A fix is kept under the review gate in `docs/LESSONS.md` (hard reject on rising unknown/1k or heavy deletion; soft recall allowance on structured pages); rejections are kept under `fixes/rejected/` and logged. The review role is agent-agnostic (`--engine cmd:<command>`); Grok Build (`--engine grok`) is the choice on the owner's box. All of this stays local and gitignored; tests use synthetic PDFs and a fake engine. The step is optional for players.

### 2026-10-21 — talkdnd play helper

Agents were slow to start (they read the whole DM guide, logs and sheets) and one agent built a character for the player without asking. Mechanical steps that need no model now live in `tools/talkdnd`, a Rust program shipped next to combat-sim in `LetsPlay/bin/`. `talkdnd start` checks content, starts pdf2md in the background and writes a compact `digest.md`; `talkdnd new` and `talkdnd chargen` record each player decision in `chargen.json` and check it against SRD-only option data (2014 species increases, 2024 background increases, 27-point buy, pool assignment, HP, AC, skills); `chargen sheet` writes the sheet and its `combat yaml` block from one set of numbers. Ability rolls use the combat-sim dice engine as a library and are logged and final. `talkdnd npc random` drafts SRD companions; the DM adds personality after the player confirms. Options outside the SRD are allowed with a warning, because validation must not need the player's books. Release archives are now named by plain platform (`talkdnd-letsplay-<tag>-linux-x86_64.tar.gz`, `-windows-x86_64.zip`, `-macos-arm64.tar.gz`).

### 2026-10-21 — Short DM guide with player commands

Agents auto-read `AGENTS.md`, some only its beginning, so it now opens with a player command table (`开新局`, `继续游戏 <存档>`, `存档`) and a mandatory character-creation flow: one decision per turn, the player chooses everything unless they say random, ability rolls via `talkdnd chargen roll` and assignment by the player, companions shown for confirmation. Core table rules follow; everything else moved to `LetsPlay/docs/` (chargen, dice, combat, narration, pdf2md, layout). Resuming reads the core guide plus `talkdnd start`'s digest instead of the whole manual, rulings, logs and every sheet. Separate CLAUDE.md / GEMINI.md pointer files were rejected by the owner because current Claude Code and Gemini read AGENTS.md.

### 2026-10-21 — chargen validation: start level, subclass, armor, spells, species traits

`talkdnd chargen` checks more of the character against SRD data before the sheet is written:

- A tracked `tools/talkdnd/data/adventures.yaml` maps an adventure slug to a suggested start-level range (`start_level_min`, `start_level_max`, `note`; facts only). `chargen` adds a `level` step after `class` that prints that range; `validate` errors when the level is outside it (unknown adventure: warning only), and the level must be 1..20.
- 2014 cleric/sorcerer/warlock must pick a subclass at level 1; other classes at their `subclass_level` from the class data. A missing subclass at or above that level is an error, and a `subclass` step appears in `next_step` when needed.
- Armor and shield proficiency come from the class plus the subclass (a new `armor_proficiency` field on subclass data; 2014 Life Domain grants heavy armor). Wearing unproficient armor or a shield is an error.
- Cantrip count and known/prepared spell count are checked against class data at the chosen level (2014 prepared = spellcasting modifier + level, minimum 1; 2024 uses the table): too many is an error, too few a warning.
- Species data gains darkvision, resistances, and short own-words trait summaries (SRD species only), written to the sheet's new 种族特性 section and to the `combat yaml` `senses`/`resistances`.

Alternative considered: require the player to split cantrips and leveled spells into two fields. Rejected: it adds a field and breaks existing saves; a built-in SRD cantrip list splits the one spell list.
