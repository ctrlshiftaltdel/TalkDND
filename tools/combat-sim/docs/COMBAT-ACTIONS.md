# Combat actions (v1.5a)

This document covers the five core actions added in TASK-0115: Grapple, Shove,
Unarmed Strike, Two-Weapon Fighting and Extra Attack. The engine resolves them
through the existing action, bonus-action, condition, forced-movement and
spatial paths. No creature, class or campaign name is hard-coded.

## Data

The common layer supplies the shared shape; the 2014 and 2024 rules files
override the fields that differ. The loader merges `common/rules.yaml`, then
`2014/rules.yaml` or `2024/rules.yaml`, field by field.

```yaml
rules:
  combat_actions:
    unarmed_strike:
      ability: str
      proficient: true
      damage: "1d3+{str}"   # edition-specific
      type: bludgeoning
      reach: 5
    grapple:
      resolution: save      # contest (2014) | save (2024)
      attacker_skill: athletics
      defender_skills: [athletics, acrobatics]
      save_abilities: [str, dex]
      dc: "8+{str}+{prof}"
      max_size_difference: 1
      condition: grappled
    shove:
      resolution: save
      attacker_skill: athletics
      defender_skills: [athletics, acrobatics]
      save_abilities: [str, dex]
      dc: "8+{str}+{prof}"
      max_size_difference: 1
      distance: 5
      prone_condition: prone
    two_weapon_fighting:
      require_light: true
      add_positive_ability_modifier: false
```

- `resolution: contest` uses the attacker skill against one of the target's
  listed skills. The defender wins ties. This is the 2014 form.
- `resolution: save` uses the target's best listed save unless the caller names
  one. The DC is a flat expression evaluated from the attacker's ability
  modifiers and proficiency. This is the 2024 form.
- `max_size_difference` limits the target to that many size ranks above the
  attacker.
- `condition` / `prone_condition` must name conditions that exist in the loaded
  content. The engine stores the source, a permanent duration (until escaped or
  removed), and an escape note. In 2024 the grappled condition is linked to the
  grappler so its existing "attack another target" modifier works.
- `add_positive_ability_modifier` stays false for both editions. A later
  feature that adds the modifier can set it true without changing the engine.
  The off-hand path removes the attack's positive ability modifier from its
  first damage part; content follows the existing convention of writing the
  ability modifier into weapon damage (for example `1d6+3`), so a negative
  modifier still applies.

`content check` validates the condition references, required fields, positive
distance/reach and flat DC expressions.

## Extra Attack

Extra attacks are data-driven:

```yaml
creatures:
  - id: veteran
    extra_attacks: 2        # three attacks with the Attack action

abilities:
  - id: extra-attack
    activation: passive
    extra_attacks: 1        # one additional attack
```

The engine uses the highest value from the creature or its passive features; it
never sums them, and it never reads a class name or level. A creature with no
value keeps the legacy one-attack behavior.

Extra attacks are resolved only when the `attack` command takes the Attack
action with a weapon or with `--unarmed`. They do not apply to `--offhand`,
`--multi` (a stat-block multiattack), or a `use` ability. Every attack rolls its
own hit, damage and triggers; a target that drops is skipped and the remaining
attacks are logged as skipped.

## Action economy

| Action | Cost | Requirement |
|--------|------|-------------|
| Grapple | one attack of the Attack action | target in reach, size legal, not immune |
| Shove | one attack of the Attack action | target in reach, size legal; condition immunity only for knock-down |
| Unarmed Strike | one attack of the Attack action (or the remaining attacks) | target in reach |
| Two-Weapon Fighting | bonus action | a prior Light melee main-hand Attack action and a different Light melee weapon |
| Extra Attack | remaining slots of the Attack action | `extra_attacks` data |

Every command validates the target, reach, size, immunity, the requested
defense and the resource before spending the action or bonus action. An invalid
defense is rejected before the action starts. A failed contest or save still
spends the attack (the attempt happened); a rejected attempt does not.

### Attack action budget

The engine stores Attack action slots in the existing per-turn `once_used` map
under `turn:attack-action:slot:<n>`; no state field or schema version is added.
The turn-start reset already clears every `turn:` key and resets `action_used`.

- The first `attack`, `attack --unarmed`, `grapple` or `shove` this turn starts
  the Attack action and gets `1 + extra_attacks` slots.
- Grapple and Shove consume exactly one slot. If slots remain, the Attack action
  stays open and a later `attack`/`attack --unarmed` resolves the remaining
  attacks.
- `attack` and `attack --unarmed` consume every remaining slot and resolve them
  in one command.
- `--offhand` is a bonus action and never consumes an attack slot.
- `--multi` and `use` are whole-action paths. They cannot be used while an
  Attack action is open, and they clear any stale slots when they start.
- A creature with one attack has one slot; after a grapple/shove it cannot take
  a second action that turn.

## CLI

```text
combat-sim attack <actor> <target> <attack> [--adv] [--dis] [--nonlethal]
combat-sim attack <actor> <target> --unarmed [--nonlethal]
combat-sim attack <actor> <target> <light-weapon> --offhand
combat-sim grapple <actor> <target> [--defense athletics|acrobatics|str|dex]
combat-sim shove <actor> <target> [--mode knock-down|away]
```

`--defense` is optional; when omitted the target uses its best legal defense and
the log says which one. A value that is not legal for the edition is rejected
before the action is spent. `shove --mode away` uses the existing forced-movement
path: it spends no Speed, provokes no opportunity attack, and stops at a wall,
occupied space, a drop or the scene boundary with the blocking reason logged.

## Compatibility

- `STATE_SCHEMA_VERSION` stays 1. Old state files and old content load with the
  new fields absent.
- Old `attack` and `use` commands keep their result when the creature and rules
  have no new data.
- The new fields are additive and skipped when empty, so existing content-pack
  hashes and state snapshots stay stable.
- The engine never writes DM-only text or save contents into action logs.

## Limits

- Grapple, Shove and Unarmed Strike resolve against the same target for the
  whole command; a target that drops is skipped and the remaining attacks are
  logged. Switching targets between attacks is a later task.
- 2024 grapple/shove are the Unarmed Strike options and use the save form. They
  do not roll a separate attack.
- The engine has no equipment inventory: Two-Weapon Fighting checks the
  creature's Light melee attack entries, not hand slots.
