# Automation switches (v1.3 / TASK-0111)

`talkdnd settings automation` exposes the DM automation switch board. It is
per save: one save-wide default per fixed category, plus optional overrides for
one scene and one encounter. Effective order is **encounter > scene > save
default**. The settings live in `Saves/<slot>/save-meta.yaml` under
`settings.automation`; there is no second settings file.

The switches describe *how much the DM does on its own*. They do **not** change
combat rules, dice, the legacy combat AI default, or the save schema beyond the
versioned block below.

## Data shape (schema 9)

```yaml
settings:
  dice_display: true            # unchanged from schema 5
  style: { ... }                # unchanged from schema 8
  automation:
    version: 1
    default:
      resolution: auto
      combat: auto
      npc: assisted
      player_choice: assisted
      reaction: assisted
      near_limit: assisted
    scenes:
      crypt-entry:              # partial: only overridden categories
        npc: manual
    encounters:
      goblin-ambush:
        combat: assisted
```

- `version` must be `1`; a newer version is refused.
- `default` is the save-wide value per category. A missing category falls back
  to the charter default below; an unknown category, unknown level or unknown
  field fails closed when the save loads.
- `scenes` / `encounters` are keyed by content id and hold only the categories
  they override. Unknown ids are refused when setting; an empty override map,
  a duplicate YAML key and a control-character / path-like id are refused.
- `player_choice` and `reaction` may be `manual` or `assisted`, never `auto`.

## Fixed categories and defaults

| Category | Meaning | Default |
|----------|---------|---------|
| `resolution` | Checks, damage, HP/resource bookkeeping with no player choice | `auto` |
| `combat` | Initiative, monster turns, conditions, movement | `auto` |
| `npc` | NPC dialogue, voice and improvised reactions | `assisted` |
| `player_choice` | Any decision belonging to a player character | `assisted` |
| `reaction` | Shield, Luck, Inspiration, opportunity attacks, similar reactions | `assisted` |
| `near_limit` | Low HP, almost-empty slots/resources, exhaustion or encumbrance near a limit | `assisted` |

Levels:

- `manual` — only when the table explicitly asks; no prompt.
- `assisted` — pause and ask, or warn before proceeding.
- `auto` — resolve without asking and log it.

`auto` is rejected for `player_choice` and `reaction` at every scope. The
program never rolls a player's choice or spends a player's reaction for them.

## CLI

```bash
talkdnd settings automation                                  # defaults + overrides
talkdnd settings automation show --scene crypt-entry
talkdnd settings automation show --encounter goblin-ambush
talkdnd settings automation set npc manual --scene crypt-entry --reason "players narrate here"
talkdnd settings automation set combat assisted --encounter goblin-ambush --reason "ask before monster turns"
talkdnd settings automation reset npc --scene crypt-entry --reason "table changed its mind"
talkdnd settings automation reset all --encounter goblin-ambush --reason "back to defaults"
talkdnd settings automation reset resolution --reason "back to charter default"
```

- `show` without a context lists the save defaults and every override. With
  `--scene` and/or `--encounter` it prints one effective value per category and
  names the source (`存档默认` / `场景 <id>` / `遭遇 <id>`). Passing
  `--encounter` also applies the scene that encounter links to.
- `set` and `reset` accept `--scene` **or** `--encounter`, never both. A plain
  command targets the save default.
- `--reason` is required, non-empty and at most 200 characters. It is written
  to the current session log with the old value, new value, scope and time.
- `reset <category>` on a scene/encounter removes that override; on the save
  default it restores the charter default. `reset all` does the same for every
  category at that scope.
- `settings` and `settings dice on|off` keep working exactly as before; the
  legacy `settings.dice_display` field is preserved.

`talkdnd start` writes one automation line into the digest and the printed
summary: effective values at the current scene, override counts, and a pointer
to `talkdnd settings automation` for the per-value source.

### Known ids

Scene and encounter ids come from the save, not from the CLI:

- the current scene label in `progress.yaml`;
- `combat/*.state.json` (`scene.id` and `encounter_name`);
- the adventure `combat/` directories named by `combat/campaign.yaml`
  (`scenes[].id`, `encounters[].id`, and single-item files).

An override for an id that is not known is refused before anything is written.
Ids are 1–80 letters, digits, `-`, `_` or `.`; whitespace, control
characters, path separators, `.` and `..` are refused.

## Validation and failure behavior

`migrate::load` validates `settings.automation` on every read. `write_meta`
validates it again before every write. The following fail closed with an
actionable error and never touch the save:

- unknown category, level or field;
- `auto` on `player_choice` / `reaction`;
- unknown scene/encounter id, duplicate id, empty override map;
- automation block version other than 1;
- `save-meta.yaml` schema newer than this binary.

A failed `set`/`reset` writes no meta and no log line. A command that would
not change anything reports "未改动、未记日志" and also writes nothing.

## Schema 8 -> 9 migration

`CURRENT_SCHEMA` is 9. `talkdnd start` and `talkdnd migrate`:

1. back the whole save up to `Saves/_backups/<slot>-<stamp>-schema8`;
2. write the charter defaults for every missing category, preserving any
   existing `settings.dice_display`, `settings.style` and partial automation
   values;
3. append the schema migration record (8 -> 9) and a session-log line naming
   the backup and the automation defaults;
4. refresh the integrity manifest for files that were clean before.

A schema-8 save without automation therefore behaves exactly like the charter
defaults before and after migration. The backup is the rollback: restore it to
get the schema-8 `save-meta.yaml` back. A failed write leaves the old file in
place; the retry migrates normally. A save whose schema is newer than 9 is
refused, not downgraded.

## Privacy

The commands read only `save-meta.yaml`, `progress.yaml`, campaign/state
metadata and adventure combat ids. Audit lines contain only category, level,
scope id and the DM-supplied reason; save contents, character sheets, secrets
and book text are never read or logged.

## Code map and tests

- `src/migrate.rs` — `Automation`, `Level`, `AutomationCategory`, schema 9 and
  the 8 -> 9 migration.
- `src/settings.rs` — known ids, precedence, `show` / `set` / `reset`, audit.
- `src/start.rs` — the digest line.
- `src/lib.rs` — `settings automation` CLI wiring.
- `tests/automation.rs` — defaults, sources, precedence, overrides/reset,
  illegal values, unknown/duplicate ids, migration/equivalence, newer-schema
  refusal, CLI, digest and audit. Unit tests in `settings.rs` cover the
  precedence helper and the player-owned `auto` ban.
