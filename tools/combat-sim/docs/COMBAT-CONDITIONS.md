# COMBAT-CONDITIONS — Bloodied, Exhaustion, Charmed and Frightened

This document describes the v1.5b combat-condition layer in `combat-sim`. The
engine stays data-driven: condition effects, exhaustion thresholds and
per-level penalties come from `LetsPlay/Rules/combat/{common,2014,2024}/`, and
the engine only supplies the shared query and enforcement code.

## Data model

### Conditions

`ConditionDef` keeps its existing shape (`flags`, `implies`, `modifiers`,
`duration_default`, `ends_on`, `manual`). v1.5b adds an optional
`rules.condition_behaviors` map instead of new condition fields, so existing
condition definitions and direct `ConditionDef` literals keep working.

```yaml
rules:
  condition_behaviors:
    charmed: {cannot_target_source: true, ends_when_source_dies: true}
    frightened: {cannot_move_closer_to_source: true, ends_when_source_dies: true}
    hidden: {hidden: true}
    invisible: {hidden: true}
```

- `cannot_target_source` — while the holder is affected, it cannot attack the
  recorded source or target it with a damaging ability.
- `cannot_move_closer_to_source` — willing movement that would reduce the
  distance to the recorded, visible source is refused.
- `ends_when_source_dies` — the condition is removed from every holder when its
  recorded source dies.
- `hidden` — the holder is not visible for source-line-of-sight checks.

The source is the normal `ActiveCondition.source` field written by
`condition add --source`; `linked` is accepted as a fallback for older data.

### Exhaustion

Exhaustion has two parts:

1. An additive `CombatantState.exhaustion_level` (`u32`, serde default `0`) is
   the source of truth. Old combat states omit it and load with level 0.
2. The normal `exhaustion` condition marker keeps the existing condition
   display, immunity and modifier paths working. It is created and removed with
   the level.

`rules.exhaustion` holds the edition parameters:

```yaml
# common
exhaustion:
  condition: exhaustion
  max_level: 6
  death_level: 6
  long_rest_reduction: 1

# 2014
exhaustion:
  check_disadvantage_from: 1
  speed_halved_from: 2
  attack_save_disadvantage_from: 3
  hp_max_halved_from: 4
  speed_zero_from: 5

# 2024
exhaustion:
  d20_penalty_per_level: 2
  speed_penalty_per_level: 5
```

The engine applies the 2014 thresholds to Speed, HP maximum and death, and the
2014 disadvantage modifiers are declared on the `exhaustion` condition itself
with `when: {exhaustion_level_min: N}`. For 2024 the engine adds a flat
`-2 × level` bonus term (labelled `exhaustion`) to every d20 Test that goes
through the attack, save, check or ability-check paths, and subtracts
`5 × level` feet from Speed. Level 6 kills the creature.

A long rest lowers the level by `long_rest_reduction`; healing and rest use the
effective HP maximum, so a 2014 level-4 maximum is respected until the level
drops.

### Bloodied

Bloodied is derived, never stored: `engine::is_bloodied(state, index)` is
`hp <= effective_max_hp / 2`. It is therefore always current after damage,
healing and temporary-HP changes, and at 0 HP. The `Effective HP maximum`
includes 2014 level-4 exhaustion.

Content uses the predicates:

```yaml
- applies: attack_roll
  effect: advantage
  when: {bloodied: true}

# or, for a feature that inspects another creature:
when: {target_bloodied: true}
```

`bloodied` and `target_bloodied` are booleans (`true` / `false`). The older
`self_hp_below_pct` / `target_hp_below_pct` predicates now use the effective
maximum as well.

## CLI

The old forms are unchanged:

```sh
combat-sim --state fight.state.json condition add Hero hidden
combat-sim --state fight.state.json condition remove Hero hidden
```

New optional arguments:

```sh
# Exhaustion level 1-6 (or the ruleset max).
combat-sim --state fight.state.json condition add Hero exhaustion --level 3
combat-sim --state fight.state.json condition remove Hero exhaustion

# Source-based conditions.
combat-sim --state fight.state.json condition add Hero charmed --source "Court Mage"
combat-sim --state fight.state.json condition add Hero frightened --source "Young Dragon"
```

`--level` is exclusive to exhaustion and rejects 0, values above the ruleset
maximum, and non-exhaustion conditions. `condition remove exhaustion` clears
the level and marker. `status --json` exposes `effective_max_hp`, `bloodied`,
and a `level` field on the exhaustion condition entry.

Logs name the condition, its source (when present), the level and the
effect/removal reason. No save text or DM-only material is added to a log.

## Enforcement points

- **Attacks**: `attack`, `--unarmed`, `--offhand` and ability attack effects
  check the charmed source before spending the action.
- **Damaging abilities**: `use` / `cast` checks explicit creature targets; an
  ability is damaging when any reachable effect contains damage.
- **Movement**: both the legacy zone/grid router and the spatial planner
  preflight every step against visible frightened sources. Forced movement is
  not willing movement and is not blocked.
- **Line of sight**: `source_in_line_of_sight` uses the existing spatial LOS
  kernel on grids, including wall and height blockers, and falls back to the
  zone cover flag when no metric geometry exists.
- **Hidden sources**: the `hidden` flag on a condition behaviour makes a source
  invisible to this check; `hidden` and `invisible` use it in both editions.
- **Source death**: `on_death` removes all conditions with
  `ends_when_source_dies` from every holder and logs the reason.

## Compatibility

- `STATE_SCHEMA_VERSION` stays **1**.
- `exhaustion_level` is additive and serde-defaulted; a state without it loads
  with no exhaustion.
- No `save-meta` schema or `talkdnd` migration is added.
- Old `condition add/remove` commands without the new flags keep their previous
  behavior.
- Content packs without `rules.exhaustion` or `rules.condition_behaviors` load
  unchanged; the engine's exhaustion condition defaults to `exhaustion` and
  level effects remain absent unless the data provides them.

## Known limits

- Charmed social advantage is manual: the engine does not roll social ability
  checks.
- A zone scene has no geometric sight blockers; `source_in_line_of_sight`
  falls back to the zone cover flag there. Grid scenes use the full LOS kernel.
- An area ability that includes the charmer in its area is not re-targeted; the
  explicit-target guard covers single-creature targeting.
- `talkdnd`'s out-of-combat party rest is a separate party-state path; the
  exhaustion level is carried by the combat state and is reduced by
  `combat-sim rest`.
