# Combat senses and obscurement (v1.5d)

TASK-0118 adds structured senses, light and obscurement to `combat-sim`. The
engine answers one question — can this observer see that target, and how well —
and uses the same answer for attacks, ability attacks and the AI's forecasts.

The raw `Creature.senses` string stays unchanged. `combat-sim` parses it at
runtime; there is no new persisted creature field and no save/state schema
change.

## Data

### Creature senses

`Creature.senses` is still free-form text, for example:

```yaml
senses: "Blindsight 60 ft.; Darkvision 120 ft.; Passive perception 21"
```

`combat_sim::engine::parse_senses` reads, case-insensitively:

- `darkvision`
- `blindsight`
- `tremorsense`
- `truesight`
- `passive perception`

Ranges may use `60 ft.`, `60 feet` or `60`. Segments may be separated by
commas or semicolons. A missing, zero, negative or malformed range becomes
`None`; parsing never panics. If the same sense appears twice, the largest
valid range wins. The raw string remains visible in `status --json` and is not
rewritten.

### Scene and zone obscurement

`Scene` and `Zone` accept an additive `obscured` value:

```yaml
scenes:
  - id: foggy-road
    mode: zones
    light: dim
    obscured: light
    zones:
      - {id: open, light: bright}
      - {id: fog-bank, light: bright, obscured: heavy}
```

Allowed values are `none`, `light` and `heavy`. The field is optional and
defaults to `none`, so existing content loads unchanged. In a zone scene the
zone value wins and the scene value is the fallback. In a grid scene the
scene-level value applies to every cell; `content check` warns when a grid
scene also sets zone-level `obscured`, because that value is ignored.

## Visibility model

`engine::can_see(state, observer, target)` returns a structured result:

```text
CanSee {
    visibility: clear | lightly_obscured | heavily_obscured | blocked,
    reason,
    source,
    light,
    obscurement,
    distance_ft,
}
```

Light and obscurement combine, taking the worse of the two:

| Light at target | Obscurement | Visibility |
|-----------------|-------------|------------|
| `bright` | none | clear |
| `dim` | none | lightly obscured |
| `bright` | light | lightly obscured |
| `dark` | none | heavily obscured |
| `bright` | heavy | heavily obscured |
| `dark` | light | heavily obscured |

Only heavy obscurement or `blocked` imposes attack disadvantage. Dim light and
light obscurement do not; they matter to Perception, which the engine does not
roll automatically.

## Senses

All ranges are measured with the existing spatial distance query. A sense is
usable at a distance less than or equal to its range.

- **Darkvision** treats `dark` as `dim` and `dim` as `bright`. It does not
  penetrate light or heavy obscurement: fog, smoke or similar physical
  obscurement remains. Beyond its range it does nothing.
- **Blindsight** ignores light and obscurement within range, and perceives
  hidden and invisible creatures. It still respects total cover / blocked line
  of sight.
- **Tremorsense** perceives creatures in contact with the ground within range,
  regardless of light, obscurement or cover. A creature at altitude 0 but in a
  flying/hovering state is not in ground contact. Creatures with an
  `incorporeal` tag are not sensed. It also perceives hidden creatures.
- **Truesight** sees through normal darkness and the `invisible` condition
  within range and respects total cover. It does **not** defeat ordinary
  hiding.
- **`hidden`** blocks normal vision and darkvision at any range. Blindsight and
  tremorsense perceive a hidden creature within range; truesight does not.
- **`invisible`** blocks normal vision and darkvision. Blindsight, tremorsense
  (ground contact) and truesight perceive the creature.

Magical darkness is out of scope. It is not modelled as a separate light or
obscurement source; a scene can still be authored as `dark` plus `heavy`
obscurement to represent an area that even darkvision cannot penetrate, but the
engine does not know whether a darkness effect is magical.

## Attacks

`resolve_attack_inner` and the two forecast paths (`forecast_profile` and
`simple_attack_forecast`, used by the AI) call `can_see` in both directions:

- attacker cannot see the target → disadvantage, source
  `darkness (cannot see target; ...)`, `heavy obscurement (...)`, `unseen (...)`
  or the hidden/invisible reason;
- target cannot see the attacker → advantage, source
  `unseen (target cannot see attacker; ...)`;
- both → the existing `RollMode::from_flags` cancellation applies.

Total cover is not counted a second time as "cannot see": cover remains the
attack-legality and AC rule. The hidden/invisible condition modifiers are
suppressed when the observer has a sense that defeats that condition, so
truesight/blindsight/tremorsense do not double-count the condition and normal
vision keeps the existing condition behaviour.

A creature without a `senses` string uses normal vision. There is no automatic
Perception roll; a DM who wants an active search resolves it manually.

## Status

`status --json` adds two fields per combatant:

```json
{
  "senses": "Darkvision 60 ft.; Passive Perception 12",
  "vision": {
    "light": "dim",
    "effective_light": "bright",
    "obscurement": "none",
    "darkvision_ft": 60,
    "passive_perception": 12
  }
}
```

`effective_light` applies truesight or darkvision to the light at the
combatant's own position. Range fields are omitted when absent. `--player`
masking still applies only to names; it does not hide the vision fields.

Text `status` adds `| lightly obscured`, `| heavily obscured` and `| unseen`
markers. It stays one line per combatant.

## Compatibility

- `STATE_SCHEMA_VERSION` stays 1 and no combat-state field is added.
- `obscured` is optional in `Scene` and `Zone`; old content and old state files
  load with `none`.
- With no `obscured` value and no `senses`, attacks use normal vision exactly
  as before.
- No `save-meta` schema change and no `talkdnd` migration.

## Verification

```bash
CARGO_BUILD_JOBS=2 cargo test --manifest-path tools/combat-sim/Cargo.toml
CARGO_BUILD_JOBS=2 cargo clippy --manifest-path tools/combat-sim/Cargo.toml --all-targets -- -D warnings
cargo run --manifest-path tools/combat-sim/Cargo.toml -- --campaign tools/combat-sim/tests/fixtures/rulesets/real-2014.yaml content check
cargo run --manifest-path tools/combat-sim/Cargo.toml -- --campaign tools/combat-sim/tests/fixtures/rulesets/real-2024.yaml content check
```

The dedicated suite is
`tools/combat-sim/tests/combat_senses.rs`.
