# TalkDND Project Charter

Living charter for TalkDND. This file is the project charter AgentOrch looks up at `docs/PROJECT-CHARTER.md`. It is filled in. It has no placeholders.

This document states purpose, audience, goals, non-goals, scope, constraints, success criteria, and decisions. It does not contain game-save contents, DM secrets, or text from rulebooks or adventures.

Player guide: [LetsPlay/README.md](../LetsPlay/README.md). DM guide: [LetsPlay/AGENTS.md](../LetsPlay/AGENTS.md). Development contract: [AGENTS.md](../AGENTS.md), project configuration [docs/PROJECT.md](PROJECT.md), design goals [docs/PLAN.md](PLAN.md). The root [README.md](../README.md) is for developers.

## Purpose

TalkDND is an AI dungeon-master play kit for Dungeons & Dragons, plus a Rust combat simulator.

An AI assistant runs the game in chat. It narrates, plays NPCs, and offers choices. The player plays their own character and may type any action. Dice are rolled by the local `combat-sim` program, and the full expression is shown. Combat (attack rolls, damage, conditions, concentration, movement, opportunity attacks) is resolved by that same Rust engine. Play state is written into a local save so a later session can resume.

The repository ships the program, original docs, the SRD 5.1 / 5.2.1 content packs, and SRD-derived combat data. Starting with v1.2 it also ships licensed PHB/DMG/MM content packs that stay locked until the player supplies the matching PDF; it does not ship the printed books themselves.

## Audience

- Players who want an AI DM in a chat window and can run a command-line agent against `LetsPlay/`.
- The same players as rules owners: they supply their own books. The minimum is the System Reference Document for the ruleset they choose.
- Developers who change the engine, the combat data, or the development workflow from the repository root.

TalkDND is an unofficial fan project. It is not affiliated with or endorsed by Wizards of the Coast. "Dungeons & Dragons" and "D&D" are their trademarks.

## Goals

- Run a D&D session in chat: narration, NPCs, choices, free-text actions, and a written save the player can resume.
- Resolve dice and combat with `combat-sim`, a data-driven Rust engine that does not hard-code a campaign.
- Support four player-visible rulesets — **5.5e FULL, 5e FULL, 5.5e SRD, 5e SRD** — one per save, selected by that save's `ruleset` field. SRD loads only the edition's SRD layer; FULL adds the player's unlocked full-book packs for that edition.
- Keep copyrighted books and adventures, personal saves, and DM secrets out of Git, Issues, pull requests, commit messages, and CI logs.
- Publish prebuilt `combat-sim` binaries for the release targets in this charter, and keep a from-source build for everyone else.
- Record product decisions in this charter so later tasks do not drift from them.
- Let the DM leave a published adventure only within a per-save, creation-locked deviation scope and ratio, with every original element registered in a DM-only ledger.
- Resume a campaign across sessions with a recoverable state history (checkpoints and structured diffs), stable entity and relation memory, optional in-world time, and fast local navigation of the player's own books, all inside the existing save and under the same privacy and spoiler discipline.
- Resolve spatial rules through one kernel: reach uses the target's volume, line of sight and cover use the same blocker volumes, areas intersect target volumes, movement is stepwise, and flight/falls are explicit engine commands with edition-specific triggers.

## Non-goals

- Never distribute copyrighted adventures. Rulebook content ships only under the owner's licence: SRD 5.1 / 5.2.1 packs under CC-BY-4.0, and (from v1.2) PHB/DMG/MM full packs unlocked by the player's own PDF. Players add PDFs only when they want page images, PDF fallback, or full-book content; their books stay on their machine.
- Do not commit save contents, character sheets, session logs, or DM secrets. `LetsPlay/Saves/README.md` is the only save-tree file that belongs in Git.
- Do not mix 5e and 5.5e rules inside one save.
- Do not hard-code a campaign, an adventure, or a monster that is outside the SRD into the engine or into tracked combat data.
- Do not run a second task system. The `local` AgentOrch edition files stay in the tree only because kit validation requires them. They are not a source of truth.
- Do not add a network service, an account system, or a hosted game.

## Scope and layout

Play and development are separate roots.

| Tree | Role |
|------|------|
| `LetsPlay/` | Play. The DM agent starts here. `LetsPlay/README.md` is the player guide. `LetsPlay/AGENTS.md` is the core DM guide (command table first, under 150 lines); details live in `LetsPlay/docs/*.md`, read on demand. Players put book PDFs in `LetsPlay/Rules/2014/`, `LetsPlay/Rules/2024/`, and `LetsPlay/Adventures/`, one folder per book (`<slug>/` with the PDF, its converted `index/` and, for adventures, `combat/`; `talkdnd start` tidies loose files into place). Those directories are local except their README files. Combat data the engine loads lives in `LetsPlay/Rules/combat/`. Saves live in `LetsPlay/Saves/` and are not committed. The programs they run are the gitignored binaries `LetsPlay/bin/combat-sim` and `LetsPlay/bin/talkdnd` (Windows: `.exe`). |
| Repository root | Development only. `AGENTS.md` is the collaboration contract, not the DM guide. Tools live in `tools/combat-sim` (engine), `tools/talkdnd` (play helper) and `tools/talkdnd-dev` (dev tools). AgentOrch core is `AGENTS.md` plus `kit/`, `editions/`, `templates/`, `scripts/`, `tests/` and `MANIFEST.json`; project configuration is `docs/PROJECT.md`, design goals are `docs/PLAN.md`, and the living charter is `docs/PROJECT-CHARTER.md`. Nothing required to play lives here. |

Release archives contain the whole git-tracked `LetsPlay/` tree plus the binary at `LetsPlay/bin/`; extracting one gives a ready play folder. `build-artifacts` artifacts contain only the binary at the archive root; extracting one into `LetsPlay/bin/` is the install. No launcher script points outside `LetsPlay/`. Full-book packs are a separate release asset, not part of that archive: they are built locally from the owner's pdf2md indexes and are never committed.

## Supported rulesets

Each save picks exactly one of four player-visible rulesets. The save manifest field is `ruleset`; values are case- and whitespace-insensitive on read, and the old `2014` / `2024` values are read as `5e-srd` / `5.5e-srd` and migrated on the next manifest write.

| Player option | `ruleset` value | Engine edition / layers | Content loaded |
|---------------|-----------------|-------------------------|----------------|
| 5.5e FULL | `5.5e-full` | 2024 | `common/` + `2024/srd/` + `2024/**/srd-*` SRD overlays, then unlocked `full-2024-*` packs, then `2024/` non-SRD hand-written content, adventure and campaign overrides |
| 5e FULL | `5e-full` | 2014 | `common/` + `2014/srd/` + `2014/**/srd-*` SRD overlays, then unlocked `full-2014-*` packs, then `2014/` non-SRD hand-written content, adventure and campaign overrides |
| 5.5e SRD | `5.5e-srd` | 2024 | `LetsPlay/Rules/combat/common/` then all of `2024/`; no full packs |
| 5e SRD | `5e-srd` | 2014 | `LetsPlay/Rules/combat/common/` then all of `2014/`; no full packs |

The engine still selects the on-disk layer with the edition (`2014` or `2024`) and ignores the other edition's directory. SRD is always the base layer. `*-srd` loads the whole edition layer (`common/` plus `2014/` or `2024/`) in its existing order. `*-full` loads `common/` plus `<edition>/srd/` and SRD-only `<edition>/**/srd-*` overlays (for example `2024/monsters/srd-spatial.yaml`) as the SRD base, then the unlocked full packs, then the rest of `<edition>/` (monsters, spells and other hand-written files), then adventure and character overrides. A FULL save requires at least one unlocked full pack for its edition: when the pack is missing, locked or invalid, `talkdnd start` and `combat-sim content check` fail explicitly, name the missing book and explain how to add the PDF/pack. They never silently fall back to SRD. One save never mixes editions.

The migration is idempotent: it preserves every other manifest field and the player's comments, rewrites only the `ruleset` line, and records the old/new values in a comment next to it. Shared rulings are `LetsPlay/Rules/rulings.md`. Version differences are `LetsPlay/Rules/rulings-2014.md` and `LetsPlay/Rules/rulings-2024.md`; the file names use the edition, not the player option.

Tracked combat data comes only from the SRD (CC-BY-4.0). Attribution is in `NOTICE`. Data that exists only in a non-SRD book stays in ignored local files.

## Tech stack

Project programs, tools, and scripts are Rust.

- `tools/combat-sim` is the combat engine.
- `tools/talkdnd` is the play helper: resume digest, new save, character creation checks, random companions, checkpoints. It rolls through the combat-sim dice engine (library dependency) so all dice stay unified.
- `tools/talkdnd-dev` is the development tool (leak check, LetsPlay path check, layout migration, binary install) and uses only the Rust standard library.
- Rules data is Markdown and YAML, not a third language runtime.

Accepted exceptions, and no others:

1. AgentOrch Python kit under `scripts/` and `tests/`, maintained as part of this project: `orchestrator.py`, `validate_kit.py`, `check_drift.py`, `check_scope.py`, `report_metrics.py`, `run_delivery_scenarios.py`, `scripts/architecture/`, `scripts/self_improvement/`, `scripts/update_agentorch/`, and their tests. They use the Python standard library only and run through `uv run --no-project`.
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
- Review uses the merge gates in `kit/orchestration.md`. A green CI run does not replace a criterion-by-criterion review.
- **Living charter.** The Master MUST update `docs/PROJECT-CHARTER.md` (the Decision Log and every affected section) in the same pull request whenever a task changes scope, goals, tech stack, rules support, or a user decision. Reviewers check this at the merge gate. If that update is missing, the review fails and the pull request is not merged.
- Agent bootstrap treats the charter as present when this file exists and has no placeholders. The path is recorded in `AGENTS.md` and `docs/PROJECT.md`. The kit is maintained in-project, but the charter lookup stays at `docs/PROJECT-CHARTER.md`; no kit file hard-codes a different path.

## Constraints and assumptions

- Platform: a local CLI. Linux, Windows, and macOS are the player platforms. The three prebuilt targets are listed above.
- Data and privacy: player books and saves are local. Tracked combat data is SRD-only. The privacy rule above is mandatory. Full-book packs are built locally from pdf2md indexes, are never in Git, and are published only as separate release assets; CI uses synthetic fixtures and never builds or downloads them.
- Reliability: dice come from the operating system's entropy through `combat-sim`, not from the model inventing a roll.
- Assumptions: the player can run an agent that reads `LetsPlay/` and executes `LetsPlay/bin/combat-sim`. The SRD packs ship with the program; adding the SRD PDFs is optional. GitHub is reachable for Issues, pull requests, and CI. One of the four rulesets per save is enough.

## Success criteria

- A player can follow `LetsPlay/README.md`, add their own SRD, start an agent in `LetsPlay/`, and play with real `combat-sim` rolls from `LetsPlay/bin/combat-sim`.
- A save's `ruleset` selects one of the four options: `*-srd` loads only the edition's SRD layer, `*-full` requires an unlocked full pack for that edition, and the engine does not load the other edition's directory.
- `uv run --no-project python scripts/validate_kit.py --project-ready`, `uv run --no-project python -m unittest discover -s tests`, `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-leaks`, and `cargo run --manifest-path tools/talkdnd-dev/Cargo.toml -- check-letsplay` pass on a tree that has no player books and no saves.
- Release automation builds the three prebuilt targets named in this charter.
- Full-book packs build from the owner's local pdf2md indexes, validate as `distribution: full`, and are published as separate release assets without entering Git, the LetsPlay archive or CI logs.
- A pull request that changes scope, goals, tech stack, rules support, or a user decision also updates this charter in that same pull request.
- A grid encounter that explicitly carries P1 spatial state resolves reach, line of sight, cover, areas, stepwise movement, opportunity threats and flight/falls through the unified kernel; a save without those fields keeps the legacy 2D result.
- As of v1.0.0, a session interrupted mid-play can be resumed from the last good checkpoint, and the structured diff since the previous session is available without rereading the whole log.

## Architecture decisions

Current choices. The dated record and the rejected alternatives are in the Decision Log.

- Play files live under `LetsPlay/`. Development files live at the repository root.
- The AgentOrch edition is `github`.
- Player-visible rulesets are the four save values `5.5e-full`, `5e-full`, `5.5e-srd`, `5e-srd`. The edition layer stays `2024` / `2014`; the suffix controls whether unlocked full packs load. FULL fails closed when its edition has no unlocked pack.
- Implementation language is Rust, plus the named exceptions in Tech stack.
- The play directory is self-contained. The binary lives in `LetsPlay/bin/` and is not committed.
- Prebuilt releases cover Linux x86_64, Windows x86_64, and macOS arm64.
- This file is a living charter, updated in the same pull request as the decision it records.
- Adventure deviation is per save, chosen with `talkdnd new --scope/--ratio`, stored in `save-meta.yaml`, and cannot be changed after creation. Original content is registered in the DM-only `original.yaml` before it is used, and its type, rarity and chapter budget are checked by the tool.
- The campaign-continuity data contract is `LetsPlay/docs/campaign-continuity.md`. It is the single authority matrix for the P00–P08 program; implementation tasks do not drift from it.
- The main save schema rises to 8 only in P03. All other new save state is versioned sidecars (`memory.yaml`, `entities.yaml`, `relations.yaml`, `world-time.yaml`, `checkpoints/*/manifest.yaml`, `index/structured.json`): a missing file means the feature is off, a newer version is refused.
- Checkpoints are the DM-local recovery path, not a player-facing view. `checkpoint snapshot` rewinds mechanical state plus revealed/identified knowledge and nothing else (never append-only logs, DM working files, derived files, or binary handout exports); `checkpoint restore` backs the whole save up before replacing anything and never reuses an existing or partial backup directory (a same-second name collision takes a `-2`, `-3`, … suffix and a new empty directory, the same disambiguation checkpoint ids and migration backups already use), appends a restore line to the session log without truncating history, refreshes the integrity manifest and audit baseline, migrates a snapshot whose schema is older than the current main schema and refuses a newer one, and keeps the last ten; a restore refuses before changing anything when the save contains an entry the whole-save backup cannot reproduce (symlink or other special file), and a mirror rollback keeps the runtime locks `.talkdnd.lock` and `.talkdnd.integrity.lock` in place; a failure after the commit (schema migration, restore log append, integrity re-baseline) is not automatically rolled back and reports the stage, checkpoint id, pre-restore whole-save backup and that the save may already have changed; `snapshot diff` reports structured changes only.
- World time is per save, off by default, and moves only on an explicit command. No narration or scene change advances it.
- World-time events come due by exact label match (`due == date`, pending only). `time event list` defaults to the player view (dm_only events hidden); `--all` is the DM view marked `[DM]`, `start` shows only player-visible due events, and `time show` counts only player-visible events (dm_only is excluded from both its total and pending counts).
- The Oracle produces inspiration only. It writes no save file (not even the roll log or the session log), never invents a roll, resolves every die through the combat-sim dice engine behind the shared `--seed` RNG, never registers original content or bypasses the deviation rules, and `original plan` adds the six optional dm-plan template sections (当前局面, 参与者目标, 冲突, 线索, 可能行动, 不介入后果) only when `dm-plan.md` does not exist yet — an existing plan is never overwritten.
- The live encounter state `Saves/<slot>/combat/*.state.json` is authoritative save data, not merely replayable scratch: it is covered by the integrity manifest when present, every normal combat-sim write refreshes the baseline on success (a failed write never advances it), a missing state file is not an error, and hand edits are reported and accepted like any other tracked file.
- Tool proficiency checks read proficiency only from the structured `Saves/<slot>/sheets/<id>.yaml` `tools` list, never from Markdown prose or the tool name. The checked ability is a mandatory explicit `--ability`; an unproficient or unknown but well-formed tool still rolls with only the ability modifier (plus explicit extras) and is marked not proficient, and the proficiency bonus comes from the combat block's `prof` (derived from `level` when absent).
- Mid-encounter reinforcement is `combat-sim add`, never a rebuild. New combatants join awake and enter the live initiative order by total (higher totals first, ties after the combatants already in the order); one inserted at or before the current turn position does not act in the running round (the turn cursor shifts so the current actor and round progress are preserved), while one inserted after it acts later in the same round. Existing combatants' HP, resources, conditions, action counters, positions, initiative and the encounter log are only ever appended to, party-side additions sync through the existing party-state write-back, and the write goes through the normal state-save path so the integrity baseline advances only on success.
- Secret protection stays literal. `secret scan` is read-only and reports marker coverage as counts and line numbers, never file content; `secret import` reads only DM-reviewed explicit terms, defaults to a dry-run preview, and writes only on an explicit `--apply` after deduplication and format validation; `talkdnd start` warns when `dm-secrets.md` has content but zero recognized markers. There is no semantic spoiler detection: unmarked narrative is never auto-imported, and the guard's matching strategy (case-insensitive English word boundaries, Chinese substrings) is unchanged.
- Spatial rules have one kernel. A save with explicit P1 spatial fields (a pose, vertical state, `SpatialState`, `cell_height_ft` or terrain) resolves reach, line of sight, cover, areas and movement through `space.rs`; a save without them keeps the legacy 2D queries. `GridCell.height_ft` is a solid obstacle above the cell floor and `cell_height_ft` is a ceiling above the local floor. `combat-sim fly` owns altitude changes and `combat-sim fall` owns R-09 and liquid check rolling; losing flight support through a condition drops the creature. Liquid landing is only used when a caller supplies a `FallMap` with a liquid column, because the grid scene has no water flag.

## Open decisions

None. A new product decision is written into the Decision Log, and into the sections it changes, before a task that depends on it is approved. The Master does not invent an answer in chat and leave this file stale.

## Initial milestone

The smallest end-to-end slice is already the play kit: a player supplies an SRD, starts in `LetsPlay/`, and uses `combat-sim` to roll dice and resolve one combat under a single `ruleset`. This charter is the signed description of that slice. Further work stays inside it until the Decision Log says otherwise. The 2026-10-24 Decision Log authorizes the P00–P08 campaign-continuity program as the next scope, shipping as v1.0.0 on 2026-10-25 (see the 2026-10-25 Decision Log entry).

Validation is the kit check, the full kit test suite and the leak check in Success criteria, plus the combat-sim and talkdnd-dev test commands in `docs/PROJECT.md`. Rollback of a charter edit is a normal Git revert of that commit. Rollback of a product change follows the task's own rollback note and, when the decision changes, a new Decision Log entry.

## Decision Log

### 2026-10-28 — Unified spatial rules in the engine and CLI (TASK-0090, P5)

The owner's spatial roadmap reaches user-visible play. P1–P4 added additive types, geometry, movement and flight/fall seams; P5 wires them into the existing turn commands without changing `Position`, the save schema or the AI layer. A save that explicitly carries P1 spatial state (a combatant `pose`/`vertical`, an encounter `SpatialState`, `Scene.cell_height_ft` or terrain volumes) now resolves attacks, abilities and movement through one kernel: footprint-aware reach, blocker-volume line of sight and cover, area cells intersected with target volumes, stepwise P3 movement (including forced pushes that spend no Speed and provoke no opportunity attack), 3D opportunity threats and boundary-only zone triggers. A save without those fields keeps the legacy 2D results, which the regression tests pin.

User-visible additions are `combat-sim fly <who> <height>` and `combat-sim fall <who>`. `fly` moves the vertical axis, stops below a ceiling and keeps `SpatialPose.cell.z` (absolute elevation) in sync with `VerticalState.altitude_ft` (height above the local floor). `fall` applies P4 damage and landing, rolls the R-09 DC 10 Acrobatics house-rule check for damaging falls of 10 ft or less, and rolls the 2024 liquid-landing reaction when a caller-supplied `FallMap` has a liquid column. A grid scene has no water flag, so the CLI never invents liquid from scene data. Losing flight support through a condition (for example 2014 `stunned` with `speed_zero`, or 2024 Incapacitated) now drops the creature immediately; the shared 2014 `stunned` data gained the missing `speed_zero` flag, and a 2024 flyer still falls through Incapacitated.

`GridCell.height_ft` is now a solid vertical obstacle above the cell floor: it blocks ground movement and line of sight when tall, gives cover when low, and forms a landing surface in `FallMap`; `cell_height_ft` remains a ceiling above the local floor. Movement is validated before any state change and re-checked before every step, so a blocked route, a stopped reaction or a killed mover can never teleport to the goal. Opportunity attacks use 3D reach, and an observer's reaction is consumed once per round. Enter-area scene triggers fire on a zone boundary, not on every cell.

No schema field, dependency, crate version or save migration is added. Known limits, recorded rather than hidden: `deprived_of_movement` / `held_aloft_by_magic` remain low-level `FlightSubject` inputs because no engine/content flag expresses them; scene-derived falls never carry liquid; and `interact` uses the object's zone as its reach proxy because scene objects have no cell coordinate. Alternatives considered: rewriting `Position` into a 3D enum (rejected — P1 is additive and old saves must load unchanged), adding a scene water flag or new condition flags (rejected — schema is frozen for P5; follow-ups need their own task), and auto-detecting height obstacles in the legacy router (rejected — legacy saves must keep their results).

### 2026-10-27 — Four player-visible rulesets (TASK-0107)

The owner's 2026-10-27 ruleset decision is implemented as four save-visible values: **5.5e FULL** (`5.5e-full`), **5e FULL** (`5e-full`), **5.5e SRD** (`5.5e-srd`) and **5e SRD** (`5e-srd`). Read parsing trims whitespace and ignores case; any other value is rejected with an error that lists all four options and the legacy aliases. `2014` is still read as `5e-srd` and `2024` as `5.5e-srd`, so old saves load. The next manifest write (including `talkdnd start`, `talkdnd migrate`, and any `chargen` field that writes the manifest) rewrites only the `ruleset` line to the canonical value, preserves other fields and comments, records the old/new values in a comment, and is idempotent.

The edition layer stays `2014` / `2024`. `*-srd` loads the whole edition layer (`common/` plus `2014/` or `2024/`) in its existing order and ignores full packs, even if an old manifest still lists one. `*-full` loads `common/` plus `<edition>/srd/` and SRD-only `<edition>/**/srd-*` overlays as the SRD base, then the unlocked `full-<edition>-{phb,dmg,mm}` packs (same ids override the SRD base), then the rest of `<edition>/` (monsters, spells and other hand-written files), then adventure and character overrides. A full pack for the other edition is a hard error; one save never mixes editions. FULL requires at least one unlocked full pack for its edition; when no matching pack is loaded, `talkdnd start` and `combat-sim content check` fail with the missing book and the PDF/pack instructions instead of falling back to SRD. A save may switch between FULL and SRD; switching to SRD removes the managed full-pack entries on the next start.

`talkdnd start`'s digest, `talkdnd new` / `chargen` prompts, errors and the player docs use 5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD; `2014` / `2024` remain only as edition labels and legacy values. No save schema, pack schema, dependency or crate version changes. Tests cover the four values, normalization, illegal values, legacy migration and idempotency, SRD-only layering, full-pack layering and same-id override, the other-edition rejection and the fail-closed start/content-check paths. Rollback is a normal revert; old values remain accepted.

### 2026-10-27 — Full-book pack builder and release packaging (TASK-0102)

The owner's 2026-10-27 licence decision (below) is implemented as a local builder plus a separate release asset. `combat-sim content pack build --edition <2014|2024> --book <phb|dmg|mm> --index <pdf2md-index> --out <dir>` reads one book's local pdf2md output and writes a `ContentPack` with `distribution: full`, schema version 1, the normal content hash, the owner-licence string, the book label, the pdf2md tool and the source PDF SHA-256 prefix as provenance, and the pdf2md creation time. Content is one `rule_section` per Markdown heading with text, page, chapter, heading level and page anchor, section cross-references resolved from `#pN` links, and a pack index of sections plus relative figure/art/map paths. Absolute, `..` and drive-qualified image paths are skipped and reported; a pack never carries a local path.

Chapter starts come from `index.json`, the manifest intake audit, or chapter-like headings. The command reports only counts and gaps (missing/empty pages, low-quality pages, stat-block pages, skipped headings and images) and never section text. `combat-sim content pack release` re-validates a directory of full packs, rejects non-full packs, duplicate ids and empty directories, and writes a deterministic `full-packs-manifest.json` plus copies of the packs; `--dry-run` writes nothing. The release workflow attaches a separate `talkdnd-full-packs-<tag>` archive only when the release runner already has locally built packs. Hosted CI has no books and never builds or downloads full packs; tests use synthetic pdf2md fixtures only.

The source SHA-256 is provenance, not an unlock check: TASK-0103 identifies a book by its content and does not verify a hash. If the licence scope changes, the full packs are withdrawn or replaced. No pack schema, loader, save schema, dependency or crate version changes.

### 2026-10-27 — AgentOrch 1.6.3 update and HD-10 resolution (TASK-0109)

The owner asked to update AgentOrch. The 1.6.3 release (`AgentOrch-1.6.3.zip`, SHA256 `dcce5b242997ccac44ee1dea225ca13c2e9af6583dc6bca3d5bdc35acb046885`, 92 `REQUIRED` + `MANIFEST.json`) includes the upstream fixes for the downstream docs/manifest/path issues (#85). The update entry reported `STOPPED_CONFLICT` on five files; the owner approved the recommended `HD-10` resolution: accept upstream for `kit/orchestration.md`, `scripts/check_drift.py`, and the downstream mode in `scripts/validate_kit.py`; manually merge `AGENTS.md` (TalkDND section + upstream #102 reading discipline); keep only the local-book/generated-output skip in `scripts/validate_kit.py` and its regression test. Local drift drops from six to three files (`AGENTS.md`, `scripts/validate_kit.py`, `tests/test_validate_kit.py`). New `scripts/check_scope.py` and `tests/test_check_scope.py` ship with 1.6.3. No product code, rules data, save schema, crate version or player-visible behavior changes. The 1.6.3 `MANIFEST.json` is the new baseline; it is not regenerated from this tree.

### 2026-10-27 — Validator ignores local books and caches (TASK-0101)

The post-merge check on a developer machine with the player's local books exposed that the 1.6 global Markdown link scan read `LetsPlay/Adventures/*/index/` and `LetsPlay/Rules/*/*/index/` and failed on generated cross-links. The 1.1 validator had a local-only skip; 1.6 lost it. The validator now skips gitignored player trees (except their tracked README), pdf2md `.md/` output, `.worktrees`, `target`, `.pytest_cache`, `.codegraph` and `.opencode`; tracked documentation links are still checked. A regression test covers both sides. No product code or save schema change. CI has no books, so the test and the new skip are the durable guard.

### 2026-10-27 — AgentOrch 1.6 upgrade with an official baseline (TASK-0100)

The owner asked on 2026-10-27 to bring the project's AgentOrch to the latest release and to open an upstream issue first when a downstream gap blocks it. The kit is upgraded from Format 1.1 to **Format 1.6** (`AgentOrch-1.6.zip`, SHA256 `bc0a64e35f0d70b937a4a40d923f03638376c97af5d3c55026084d12c04f8d57`) and the official `MANIFEST.json` is installed as the baseline for future updates. This is a manual 1.1 → 1.6 migration: the release page has no v1.1 asset or manifest, so the update entry cannot verify a 1.1 baseline (`STOPPED_INVALID_BASELINE`); after this migration the 1.6 baseline is `VALID` and a future 1.7 release can use the update entry. The layout migration is the 1.5 one plus the 1.6 additions: `AGENT-BOOTSTRAP.md` + `AGENT-ORCHESTRATION.md` + `AGENT-ORCHESTRATION-ACCEPTANCE.md` become `kit/orchestration.md`; `ORCHESTRATION-PROJECT.md` becomes `docs/PROJECT.md`; `docs/PLAN.md` is added as the design-goal authority; editions shrink from 14 to 7 lowercase files; templates become the 1.6 lowercase set; `prompts/` is removed; `VERSION.txt`/`LICENSE.txt` become `VERSION.md`/`LICENSE.md`; the 1.6 scripts, self-improvement/update/architecture modules, and tests are installed.

Several downstream gaps in the 1.6 package are handled locally and reported upstream (AgentOrch issue #85): `validate_kit.py` now treats a tree with `docs/` but no `docs/upgrade.md` as a downstream project, checks `docs/PROJECT.md` / `docs/PLAN.md` and the manifest structure, and leaves manifest hash drift to `check_drift.py`; the shipped tests gate upstream-only `docs/README.md` / `docs/upgrade.md` assertions on those files and add downstream-mode tests; stale `MASTER.md` / `START-MASTER-*.md` references are corrected to `master.md` and its sections; `check_drift.py` skips Rust build output and local tool caches. The expected `check_drift.py` result is six modified files (`AGENTS.md`, `kit/orchestration.md`, `scripts/check_drift.py`, `scripts/validate_kit.py`, `templates/project-charter.md`, `tests/test_validate_kit.py`) and two project-profile files (`docs/PROJECT.md`, `docs/PLAN.md`). The manifest is not regenerated from this tree, and the update engine must stop on `THREE_WAY_CONFLICT` rather than overwrite local files.

No product code, rule data, save schema, crate version or player-visible behavior changes. Existing GitHub Issues keep the old task-packet shape; the 1.6 evidence matrix applies to new tasks. Rollback is a normal revert of the merge commit; no data migration is needed.

Alternative considered: stay on 1.1 or install 1.6 without the manifest. Rejected because the project would not get the 1.6 self-improvement/update/architecture tooling, and because a 1.6 tree without the baseline could not use the update entry at all. Alternative considered: generate a fresh manifest from the customized tree. Rejected because it would record TalkDND's local edits as upstream and make future three-way classification lie. Alternative considered: block the upgrade on an upstream 1.6.1. Rejected for this task: the local patch is small, tested, and reversible; the upstream issue tracks the clean fix.

### 2026-10-27 — v1.1.0 content platform (TASK-0097)

The owner ships the content platform as TalkDND 1.1.0. The behavior baseline is main `f34d523fd0af8c900e67a07ffc5b4cac0423569b` (PR #197). It contains PR #188 (TASK-0098: ContentPack v1 format, loader, pack CLI, metadata and integrity hash), PR #192 (TASK-0104: item/class/subclass/species/background/feat/rule-section/table domain entities in packs), PR #193 (TASK-0099: full SRD 5.1 and 5.2.1 packs generated from 5e-database commit `a6212beb4b278917c2cff41c2b04c4ef4f3e0d6f`: 2014 eight packs, 2024 seven packs), PR #195 (TASK-0106: 2024 rules prose extracted from the local pdf2md index into a 213-section pack) and PR #197 (TASK-0085: spatial rules matrix and contract v1). On `f34d523`, combat-sim 235, talkdnd 279 and talkdnd-dev 45 tests passed, and `validate_kit.py`, `check-leaks` and `check-letsplay` exited 0. The release commit changes release metadata only — the three crate versions to 1.1.0 (with the three `Cargo.lock` files), the CHANGELOG v1.1.0 section, this charter and `LetsPlay/README.md` — with no source, test or CI behavior change beyond what those merged PRs already introduced. The tag builds from the merged main tree. No save-schema change and no migration (schema stays 8); the `ruleset` values `2014` and `2024` remain valid. The bundled content is SRD-only and ships under CC-BY-4.0; full PHB/DMG/MM packs, the four player-visible rulesets (5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD) and PDF-based unlocking are v1.2 work (TASK-0102 #189, TASK-0103 #190, TASK-0107 #196). Known limitations recorded in the changelog: the SRD is a condensed text (no class introductions, no separate 2024 squeezing clause), 2024 rules prose depends on pdf2md heading structure, content adds about 9.9 MB and 0.14–0.26 s per load, one ruleset per save, and the spatial contract records Tiny sharing, creature cover degree and zone distance as project approximations pending confirmation.

### 2026-10-26 — v1.0.3 quality patch (TASK-0083)

The owner asked on 2026-10-26 to ship the checkpoint and integrity fixes merged after 1.0.2 as TalkDND 1.0.3. The behavior baseline is main `b45b749483c9530bd07384f23d9d6bd82a95c20e` (PR #164). It contains PR #154 (merge `2d150d7`, TASK-0077: restore and migration manifest writes join `.talkdnd.integrity.lock`), PR #158 (merge `49783e9`, TASK-0078: rollback keeps the integrity lock and refuses a save the whole-save backup cannot reproduce), PR #160 (merge `7ecb624`, TASK-0079: post-commit restore failures name the stage, checkpoint and backup), and PR #164 (merge `b45b749`, TASK-0081: a same-second restore allocates a new empty backup directory instead of reusing the previous one). PR #161 (merge `99ea061`, TASK-0080) only stabilizes a combat-sim concurrency test and changes no player-visible behavior; it is on main, so the tag includes it. On `b45b749`, combat-sim 208, talkdnd 279 and talkdnd-dev 45 tests passed, and `validate_kit.py`, `check-leaks` and `check-letsplay` exited 0. The release commit changes release metadata only — the three crate versions to 1.0.3 (with the three `Cargo.lock` files), the CHANGELOG v1.0.3 section and this charter — with no source, test, CI or behavior change beyond what those merged PRs already introduced. The tag builds from the merged main tree. No save-schema change and no migration (schema stays 8); pdf2md stays 0.6.0. Known limitations recorded for this release: a save that contains a symlink or other special file is refused before restore changes anything; two processes writing the same protected file still surface the later writer as an outside edit, cleared with `integrity accept`; checkpoints still do not cover memory, entities, relations, world-time, session logs or DM working files (frozen contract §4.1). The 1.0.1/1.0.2 same-second backup-name collision and the 1.0.2 note that restore and migration manifest writes bypass the integrity lock are fixed in this release; those older changelog entries stay as history. The Success criteria "As of v1.0.0" line is not edited.

Alternative considered: holding the fixes for a 1.1.0 minor release. Rejected: there is no new command, save schema or migration, so `docs/versioning.md` makes this a patch. A minor bump would imply a feature change that is not in the tree.

### 2026-10-25 — A same-second restore does not reuse its backup directory (TASK-0081)

An independent check of `checkpoint restore` confirmed that the pre-restore backup name is `{slot}-{stamp}-restore-{id}` with a one-second stamp, and that `copy_tree` creates the destination if needed and then copies entries without removing ones already there. Two restores of the same checkpoint in the same second therefore merge the second backup into the first directory. A deterministic test forces that collision with a test-only stamp (`#[doc(hidden)]`; production `restore` still passes `None` and reads the clock). The save contains a covered file A before the first restore, so the first backup contains A; the successful restore deletes A; the second restore uses the same stamp and fails after a partial commit, then runs the real mirror rollback. On the pre-fix code that rollback reported 「已回滚到恢复前状态」 while putting A back, because the reused directory still held A and the mirror treats the backup as this operation's whole tree. The fix creates a new empty directory and, when the name exists, takes the `-2`, `-3`, … suffix already used for checkpoint ids and for migration backups under `Saves/_backups/`. The copy never fills a directory this call did not just create, so an existing or partial backup cannot be silently treated as this restore's complete backup. The player command is unchanged: a second restore in the same second still runs, and each one has its own complete backup. Backup failure still leaves the save untouched; a failed rollback still names that restore's backup. No save-schema change, no new dependency, no version bump, no tag.

Alternative considered: refuse the restore when the backup path already exists. Rejected: that would turn a same-second second restore into a new failure the contract does not require, while the contract's requirement is a complete backup of the current save and a rollback to that tree. A new directory satisfies both, and it is the disambiguation this codebase already uses for the same one-second stamp. The v1.0.1 and v1.0.2 changelog entries that record the collision as a known limitation of those releases are left as history; the unreleased changelog records the fix.

### 2026-10-25 — Post-commit restore failures name the stage, checkpoint and backup (TASK-0079)

An independent verification of `checkpoint restore` covered the three stages that run after the covered files are committed: the migration of an older checkpoint schema, the append-only restore log line, and the integrity re-baseline. Each used `?` directly, so a failure returned only the underlying error: no checkpoint id, no stage, no pre-restore backup path, and no statement that the save had already been replaced. The behavior was reproduced deterministically with test-only fault injection (`#[doc(hidden)]`, production always `None`) for all three stages; no scenario was left to static analysis alone. The minimal fix wraps each stage with one message naming the stage, the checkpoint id, the pre-restore whole-save backup and the fact that the current save may already have been replaced or changed and was not automatically rolled back. Automatic reverse rollback remains out of scope because a partial migration or log append cannot be undone safely on the same argument as the commit rollback. A regression test per stage asserts those four facts and fails when the wrapping is removed. No save-schema change, no new dependency, no version bump, no tag; the successful restore path and its output are unchanged.

### 2026-10-25 — Restore rollback preserves runtime locks and refuses an unbackupable save (TASK-0078)

An independent check of `checkpoint restore` found two reachable defects on its failure paths. First, a concurrent tool command can create and lock `.talkdnd.integrity.lock` after restore took its pre-restore whole-save backup; the mirror rollback then removed that lock file as an entry the backup lacks. The next writer created a fresh lock file and acquired the manifest lock while the concurrent holder still had the old one, silently breaking the per-save integrity mutex. The rollback now excludes both runtime lock files (`.talkdnd.lock`, `.talkdnd.integrity.lock`) and `.cache`, as contract §4.1 already states for excluded runtime data; a deterministic test plays the concurrent holder through a test-only after-backup hook, asserts a second handle's `try_lock()` still returns `WouldBlock`, then that a new writer can take the lock after release. Second, `copy_tree` silently skipped symlinks and other non-file entries when building the whole-save backup, so a rollback deleted a pre-existing symlink (the backup lacked it) and still claimed 「回滚到恢复前状态」. Restore now refuses before staging when any entry cannot be reproduced by the backup, naming the entry type and stating the save is unchanged; a Unix test creates a symlink and proves the refusal leaves the save untouched (Windows cannot create symlinks without a privilege, so that fixture is Unix-only while the check itself is portable). The cleanup scan no longer drops per-entry directory-iteration errors: they are reported as leftovers, so a rollback cannot claim success it did not verify; no deterministic per-entry iteration error is constructible on the tested platforms, so that branch rests on the documented `ReadDir` item type rather than a regression test. No save-schema change, no new dependency, no version bump, no tag; restore behavior for ordinary saves is unchanged.

Alternative considered: adding a full-tree hash verification after a rollback. Rejected: the contract derives the rollback result from the mirror operation and its error states, and the two confirmed defects are concrete (a held lock deleted, a symlink deleted); the minimal fixes remove both without adding a second verification system.

### 2026-10-25 — Restore and migration manifest writes join the integrity lock (TASK-0077)

The residual concurrency gap left by TASK-0075 is closed: `checkpoint restore`'s final integrity manifest re-baseline (load → disk snapshot → reconciliation baseline → store) and the schema migration's `carry_dirty_flags` (moving a hand-edited companion's stale fingerprint from `NPCs/` to `Characters/`) were still unlocked manifest read-modify-writes. A concurrent tool process committing between the read and the store could be clobbered — restore kept the checkpoint's stale fingerprint for a file the concurrent command had just rewritten, and because dirty files are never laundered the false hand-edit report would persist (issue #154; a recorded risk, not an observed play defect). Both sections now run inside the same per-save `.talkdnd.integrity.lock` and re-read the manifest inside it. The lock order is fixed and one-way: restore already holds the checkpoint lock `.talkdnd.lock` and takes the integrity lock only after it; no code path holds the integrity lock and then takes the checkpoint lock (the integrity-lock holders — audit, after_command, accept — only read checkpoint data), and the migration's manifest writes (`after_write`, `carry_dirty_flags`) run strictly one after another, so the two locks never nest. The lock covers only the manifest section, not the file replacements, backup or log writes. The fingerprint algorithm, the `integrity.yaml` schema, `talkdnd check` exit codes and wording, and the dirty-files-are-never-laundered semantics are unchanged; deterministic regression tests prove the lock is held after acquisition (a second handle's `try_lock` must return `WouldBlock`) and then inject a committed concurrent writer before the in-lock read, at the re-baseline point for restore and at the carry point for the migration; both fail when the lock is removed. `LetsPlay/docs/table-integrity.md` now states the lock as closed instead of a known limitation. No save-schema change, no version bump, no tag.

Alternative considered: proving the two sections serialized by an outer lock instead of locking them (restore holds `.talkdnd.lock`, but other tool commands never take that lock, so it serializes restores only against each other). Rejected: the manifest read-modify-write window is what loses fingerprints and the per-save integrity lock already exists, so joining it is the same mechanism TASK-0075 adopted and needs no new locking scheme.

### 2026-10-25 — v1.0.2 quality patch (TASK-0076)

The owner ships the two fixes merged after 1.0.1 as TalkDND 1.0.2: checkpoint restore's partial-commit rollback now mirrors the pre-restore backup (PR #150, merge `4ae33c1`, TASK-0074) and the integrity manifest read-modify-write is serialized per save with an OS-released advisory lock (PR #151, merge `fe5480d`, TASK-0075). Each PR passed review against its exact head (`42b0b10` and `fd254a0`), all CI jobs were green on three platforms, and the merges were re-verified on main: 208 combat-sim, 271 talkdnd and 45 talkdnd-dev tests, `validate_kit.py`, `check-leaks` and `check-letsplay` exit 0. The release commit changes release metadata only — the three crate versions to 1.0.2 (with the three `Cargo.lock` files), the CHANGELOG v1.0.2 section and this charter — with no source, test, CI or behavior change beyond what the two merged PRs already introduced; the tag builds from the merged main tree, so it contains both fixes. No save-schema change and no migration (schema stays 8); pdf2md stays 0.6.0. Known limitations recorded in the changelog: the inline manifest writes at the end of `checkpoint restore` and inside `talkdnd migrate` still bypass the new `.talkdnd.integrity.lock` (out of TASK-0075's allowed paths; restore keeps its own `.talkdnd.lock`), and two processes concurrently writing the same protected file conservatively flag the later writer as an outside edit, resolved with `integrity accept`; the 1.0.1 limitations on backup-name collisions and checkpoint coverage still hold.

Alternative considered: holding both fixes for a larger 1.1.0. Rejected: both are user-visible defects with no new commands, schema or migration, so a patch release is the correct vehicle under `docs/versioning.md`; bundling them into a minor release would delay the fix and imply a feature change that does not exist.

### 2026-10-25 — Integrity manifest writes are serialized per save (TASK-0075)

Concurrent `talkdnd`/`combat-sim` processes interleaving the integrity manifest read-modify-write made the later writer store a manifest that it had loaded before the earlier writer committed, silently dropping the earlier writer's fingerprints; because dirty files are never laundered, the loss surfaced as a permanent false hand-edit report on `talkdnd check` (issue #149, observed in play). Every manifest read-modify-write — the post-write fingerprint record, the reconciliation baseline write-back in `talkdnd check`/`start`, `integrity accept`, and the first baseline for old saves — now runs inside a per-save advisory lock file `Saves/<slot>/.talkdnd.integrity.lock` (`std::fs::File::lock`, blocking, no new dependency, distinct from checkpoint's `.talkdnd.lock`), re-reads the manifest inside the lock, and releases the lock by RAII on every error and panic path; the OS releases the lock when the process exits, so there are no stale locks and the lock file itself is never fingerprinted. The fingerprint algorithm, the `integrity.yaml` schema, `talkdnd check` exit codes and wording, and the dirty-files-are-never-laundered semantics are unchanged. Known limitation, recorded in `LetsPlay/docs/table-integrity.md`: the inline manifest writes at the end of `checkpoint restore` and inside the schema migration still bypass this lock (restore serializes itself through `.talkdnd.lock` but can still interleave with another process's manifest write). No save-schema change, no version bump, no tag.

Alternative considered: guarding only the individual `store` calls, or adding a compare-and-swap re-check before storing. Rejected: fingerprints are lost at the read-modify-write granularity, so only serializing the whole load → merge → store section — with the read performed inside the critical section — closes the window; a CAS-style retry would add complexity to code that a blocking file lock already serializes.

### 2026-10-25 — v1.0.1 quality patch (TASK-0073)

The owner ships the #143 quality fixes (merged as PR #146, TASK-0071) as TalkDND 1.0.1, the first patch release. The behavior baseline is the candidate verified in #144: a real Windows-terminal run of 9 scenarios / 90 steps (0 failures, 0 unexpected writes, 0 data leaks, scenarios 8–9 covering the new cross-ref audit and restore failure paths) against candidate commit `140e081` (main HEAD after the #146 merge), whose Windows ZIP SHA-256 is `f39b11843c8cffd432d8502eb40e2bca34563a44c7384d0c598e47648ed71dcd`. The release commit changes release metadata only — the three crate versions to 1.0.1 (with the three `Cargo.lock` files), the CHANGELOG v1.0.1 section, and this charter — with no source, test, CI or behavior change, so shipped binaries behave as the verified candidate. No save-schema change and no migration (schema stays 8); pdf2md stays 0.6.0. Known limitations recorded in the changelog: the whole-save restore backup name carries a second-resolution timestamp, so restoring the same checkpoint twice within one second collides on the backup directory name (v1.0.0 behavior, recorded by #144, deliberately not fixed in 1.0.1); checkpoints still do not cover memory/entities/relations/world-time, session logs or DM working files (the frozen contract §4.1 design); and the new `cross-ref:` warnings may carry dm_only entity names/aliases and relation ids/types, so they are DM-view output and must not be pasted to players. The Success criteria "As of v1.0.0" checkpoint-resume line and the Initial milestone's v1.0.0 milestone statement remain accurate for this patch, so they are not edited.

Alternative considered: rebuilding the release binaries so the final tag build matches the verified candidate byte-for-byte. The owner decided on 2026-10-25 to accept the version-only delta: the #144 run on candidate `140e081` is the behavior baseline, and the release commit differs only in self-reported versions, CHANGELOG and this charter, with no behavior/source change. No rebuild/retest is required for v1.0.1; if a future release policy requires byte-identical artifacts, rebuild and retest before tagging.

### 2026-10-25 — Pre-release quality audit: contract §3.4 cross-reference warnings and restore failure reporting (TASK-0071)

The v1.0.0 pre-release audit (#143) confirmed one contract gap and one error-reporting defect; the owner's task package authorizes these minimal fixes and no others. First, `talkdnd check` (no arguments) now implements the frozen campaign-continuity contract §3.4 cross-reference audit that was specified but never wired up: dangling `relations.yaml` endpoints (exact id comparison, one line per endpoint with the relation id), clashing entity names/aliases (case-insensitive full-ledger scan, one line per shared key, plus name-equals-own-alias), world-time events with `status: pending` and `due` equal to the current `date` (reported with an explicit 世界时间未启用 note when the feature is disabled), and unusable checkpoints (missing or damaged manifest, failing copy verification — the same `problem` as `checkpoint list`). They surface as `cross-ref:` lines on `audit::Report` (counted by `clean()`/`issue_count()`/`lines()`), are warnings only — `check` still exits 0 and integrity semantics are untouched — DM-view output that may carry entity names/aliases (including dm_only entities) and relation ids/types but never file content, so it must not be pasted into player-visible material, and an unloadable sidecar (invalid YAML, newer schema) becomes one warning naming file and error instead of aborting the remaining checks. Second, `checkpoint restore` no longer claims success it cannot prove: after a commit failure the rollback is verified — "已从 … 回滚到恢复前状态" only when the whole-save `copy_tree` rollback actually succeeded, otherwise 回滚失败 with the save possibly incomplete, naming the pre-restore backup, the commit error and the rollback error; temp-file cleanup failures after a staging or commit failure are listed instead of silently ignored. A test-only fault injection inside `checkpoint.rs` (production always passes `None`; no production switch) makes the partial-commit and rollback-failure paths deterministically testable on every platform without permission tricks. Third, documentation-only: the `secret import` dry-run preview is now documented as what it already was — a DM-only local review listing at most 20 pending terms on stdout, writing nothing (not `secret-terms.yaml`, not the session log), never for player-visible material — while `scan` remains the report that never quotes content; the preview behavior itself is unchanged. The audit also recorded two non-defects without code changes: checkpoints not covering memory/entities/relations/world-time/logs/DM files is the frozen §4.1 design, and the unsigned v1.0.0 tag matches the project's release policy (no tag rewrite, no signing policy exists). No save-schema, version or tag change; one existing test assertion (`tests/worldtime.rs`, a due-today fixture asserting the old clean `check` output) was updated to expect the contract-mandated warning.

Alternative considered: treating the missing §3.4 audit and the ignored rollback result as acceptable for a 1.0.x patch. Rejected: the cross-reference audit is a frozen contract requirement ("全部是警告不是错误"), and an unverified "已回滚" claim can leave a DM restoring over a half-broken save believing it is whole — exactly the loss the whole-save backup exists to prevent.

### 2026-10-25 — TalkDND 1.0.0 (first stable release)

The owner decided on 2026-10-25 to ship the approved P00–P08 campaign-continuity program together with the 2026-10-24/25 combat quality hardening (#127–#131) as one milestone: TalkDND 1.0.0, the first stable release. This replaces the release slicing in the 2026-10-24 P00–P08 decision (v0.12 = P00 + P00a + P00b + P01 + P07; v0.13 = P03 + P04; v0.14 = P06 + P08); every feature of those three slices is already merged on main. The save schema rises to 8 exactly as that decision authorized (only P03's `settings.style`, whole-save backup migration). TASK-0070 itself changes release metadata only — the three crate versions to 1.0.0, the CHANGELOG v1.0.0 section, this charter, and the release-slice line in `LetsPlay/docs/campaign-continuity.md` §9 — with no source, test, CI or behavior change.

Alternative considered: continue shipping the three slices as v0.12–v0.14. Rejected: the owner decided on 2026-10-25 to close the milestone as 1.0.0; all slice content is already on main, so three separate releases would add tag and migration overhead without any additional change.

### 2026-10-25 — Improvisation oracle and the dm-plan template (TASK-0064)

`talkdnd oracle` implements the frozen campaign-continuity contract (§5.6): `oracle yes-no [<question>]` prints exactly `yes` or `no` (1d6: 1–3 yes, 4–6 no, the question is never echoed back), `oracle event [--focus F]` prints one event-focus phrase (prefixed `<focus>：<phrase>` when a non-empty focus is given) and `oracle meaning [--chaos 1-9]` prints `<phrase>（混乱 <N>）` with chaos clamped to 1–9 by clap (default 5, anything else is a usage error with exit code 2). All three draw from module-internal phrase tables written for this task — no published book or random-table text — through the shared DiceRng passed down from `lib.rs`, so the hidden global `--seed` reproduces every output exactly. The oracle is strictly read-only: it writes no save file — not progress, canon, reveals, original, save-meta, dm-plan, rolls.log, integrity, the session log or anything else — never calls the atomic writer or the log appender, and cannot register original content or bypass the deviation/original rules, which makes it safe at any deviation scope including 0. `talkdnd original plan` creates the DM-only `dm-plan.md` only when it does not exist, and the new template adds six optional sections (当前局面, 参与者目标, 冲突, 线索, 可能行动, 不介入后果) alongside 计划 and 伏笔与回收; an existing `dm-plan.md` is never overwritten, rewritten or parsed, so old plans stay byte-identical and fully readable. The obsolete scaffold stub test that asserted the oracle still bail!s is removed with the execution package's authorization. No version bump, no tag: the release ship remains 1.0.0 after P06 and P08.

Alternative considered: letting the oracle read the save (scene, entities, adventure position) to tailor its suggestions. Rejected: the contract freezes the oracle as pure inspiration drawn from module-internal tables, and any save-reading risks pulling dm-only facts into its output or coupling a suggestion tool to the deviation lock.

### 2026-10-25 — World time command surface and event visibility (TASK-0063)

`talkdnd time` implements the frozen campaign-continuity contract (§2.4, §4.5, §5.4): `world-time.yaml` (version 1) is off by default, every write command is refused while disabled (a refused command never creates or changes the file), and only `time set` / `time advance` move the clock — `advance` is pure addition with `--hours` folded in as h/24, rounded to 9 decimals on write and displayed with at most 3. Events carry sequential, never-reused `EV-` ids; due checking stays exact label equality against the current date with `status: pending`, and done/skipped are terminal. Per the owner decision of 2026-10-25, `time event list` defaults to the player view (dm_only events hidden), `--all` is the DM view with a `[DM]` marker, the `start` digest shows only player-visible due events, and `time show` counts only player-visible events (dm_only events are excluded from both its total and pending counts, so its numbers cannot reveal that hidden events exist; DMs see everything through `time event list --all`). This task changes no version and tags nothing: the release ship is 1.0.0 after P06 and P08.

Alternative considered: keeping `time event list` an unconditional full listing (the contract's original wording). Rejected: §6.1 puts the command's default output in the player view, so a full listing would leak dm_only events and contradict the contract's own filter rule; the owner resolved the conflict on 2026-10-25 in favor of the player-view default with an explicit `--all` DM view.

### 2026-10-24 — Adding combatants mid-encounter (TASK-0067)

`combat-sim add --creature ID[:N][@ZONE] [--side SIDE] [--initiative N]` adds new combatants to an encounter that already exists, without rebuilding it or losing fight progress. All validation (creature id, count, zone, side, initiative) happens before any persistence: an invalid command leaves the state file byte-identical. New combatants join awake with full default state (HP, resources, conditions, action counters, position) and reuse the same placement and naming rules as `new --add`. Once initiative has been rolled, they enter the live order by total — higher totals first, ties after the combatants already in the order — with automatic rolls logged as `(rolled)` and explicit `--initiative` values as `(set)`, so the initiative source is auditable. A combatant inserted at or before the current turn position does not act in the running round: the turn cursor shifts so the current actor and round progress are preserved, and the combatant's first turn comes when the order wraps around; a combatant inserted after the current position acts later in the same round. Existing combatants' HP, resources, conditions, action counters, positions, initiative and the encounter log are never reset, recomputed or rewritten — the command only appends. Party-side additions sync through the existing party-state write-back, and the save goes through the normal state-save path so the integrity baseline advances only on success. A fight that has already ended (`outcome` set) refuses additions; before `init` there is no order to insert into, so `--initiative` is refused there and `init` rolls for every combatant, added ones included.

Alternative considered: rebuilding the encounter with `new --add` when reinforcements arrive. Rejected: it resets the HP, resources, conditions and combat log of the running fight — exactly the loss of progress this decision prevents. Also considered letting a higher-initiative addition act immediately in the running round; rejected as a retroactive action that corrupts round bookkeeping and the audit trail.

### 2026-10-24 — Secret marker scan, explicit import and start warning (TASK-0069)

`talkdnd secret scan [--from <file>]` is a read-only coverage report for a DM notes file (default: the save's `dm-secrets.md`): it counts the recognized explicit markers (`- term:`, `- 秘密词:`, `[[词]]`), lists the content lines that carry none by 1-based line number (capped listing), and reports a coverage summary. The source file is never modified, the report holds counts and line numbers only — no file content — and nothing it prints is written to the session log, so no secret body text can reach player-visible output or the public log through it. `talkdnd secret import --from <file> [--dry-run | --apply]` imports DM-reviewed explicit terms, one per line or in the existing marker forms. The default (with or without an explicit `--dry-run`) is a preview that writes nothing; only an explicit `--apply` writes `secret-terms.yaml`, after deduplicating case-insensitively against the import file itself and the existing manual list, and validating format by line number (an empty marker value or a term shorter than two characters is reported as an error and not imported). Applying invalidates the secret-term cache, and the existing revealed-term filtering is unchanged. `talkdnd start` warns in the digest when `dm-secrets.md` has content but zero recognized markers, including the unmarked-line count; a missing, empty or comment-only file stays quiet.

Boundary: everything stays literal. Scan and import recognize only explicit markers; they do not perform semantic spoiler detection, never turn narrative sentences or paragraphs into terms automatically, and the guard's matching strategy is unchanged. Synonym, paraphrase and semantic coverage remains the DM's job.

Alternative considered: automatic semantic spoiler detection that turns unmarked narrative into secret terms. Rejected: it would flood the guard with false positives, over-block normal narration, and promise a guarantee that literal matching cannot deliver; the issue explicitly rules it out.

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

The charter path is declared in the project docs (`AGENTS.md` and `docs/PROJECT.md`) so AgentOrch bootstrap resolves "charter missing" against this file. The kit is maintained in-project, but the charter lookup stays at `docs/PROJECT-CHARTER.md`; no kit file hard-codes a different path.

Alternative considered: leave scope only inside `docs/PROJECT.md` and skip a charter. Rejected. AgentOrch expects `docs/PROJECT-CHARTER.md`, and a config file is a poor decision log.

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

AgentOrch is no longer treated as a vendored, read-only kit. Its files (`AGENTS.md`, `kit/orchestration.md`, `editions/`, `templates/`, `scripts/`, `tests/`) may be changed through normal tasks. Every Python command in them is `uv run --no-project python ...`. The 1.6 validator has a documented downstream mode: a tree with `docs/` but no `docs/upgrade.md` checks project docs and the manifest structure, and `check_drift.py` reports local drift. CI runs the kit's full unittest suite through uv.

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
| 2026-10-27 | AgentOrch Format 1.1 → 1.6 upgrade (TASK-0100): `kit/orchestration.md`, `docs/PROJECT.md`, `docs/PLAN.md`, 1.6 scripts/tests, official `MANIFEST.json` baseline, downstream validator mode; no product behavior change. |
| 2026-10-27 | TASK-0101 post-merge fix: validator skips local player books/saves, pdf2md output and build/cache dirs; tracked doc links still checked. |
| 2026-10-27 | TASK-0109: AgentOrch 1.6.3; HD-10 resolved by accepting upstream fixes and keeping only the local-book validator skip; local drift 6 → 3. |
| 2026-10-27 | TASK-0099: SRD 5.1 / 5.2.1 full content packs ship with the program (CC-BY-4.0, generated from 5e-database); players no longer need to add the SRD PDFs. PHB/DMG/MM are still not shipped. |
| 2026-10-27 | Owner confirmed the licence to distribute full PHB/DMG/MM content (legal review). Full packs ship from v1.2 but stay locked until the player supplies the matching PDF; no hash check, book identification is by content. If the licence scope changes, the full packs are withdrawn or replaced. Player-visible rulesets are 5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD (TASK-0107). |

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
