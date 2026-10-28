# COMBAT-READY — the Ready action

This document describes the v1.5c Ready layer in `combat-sim`. Ready is a
structured action, not a free-text note: the engine stores one response and one
trigger description, matches automatic triggers deterministically, spends the
reaction once, and resolves the response through the normal attack, ability and
movement paths.

## State model

`CombatantState` gains one additive field:

```rust
pub ready: Option<ReadyState>,

pub struct ReadyState {
    pub trigger: ReadyTrigger,      // manual | enter_reach | attacked | ability | turn_start
    pub description: String,        // the perceivable circumstance, for the audit log
    pub response: ReadyResponse,    // attack | unarmed | ability | move
    pub set_round: u32,
    pub expires_round: u32,
}
```

`ReadyResponse` is tagged by `kind`:

```json
{"kind": "attack", "attack": "longsword", "target": "Ogre"}
{"kind": "unarmed", "target": "Ogre"}
{"kind": "ability", "ability": "sacred-flame", "target": "Ogre"}
{"kind": "move", "zone": "cellar"}
{"kind": "move", "cell": [3, 2]}
```

The field is serde-defaulted and skipped when empty. Old combat states load
with no ready, `STATE_SCHEMA_VERSION` stays **1**, and no `talkdnd` migration is
added.

## CLI

```sh
# Weapon attack response. With no --target the trigger source is the target.
combat-sim --state fight.state.json ready Hero \
  --on attacked --trigger "an enemy attacks" --attack longsword

# Unarmed Strike response.
combat-sim --state fight.state.json ready Hero \
  --on enter-reach --trigger "a foe comes within reach" --unarmed

# Action ability response. --target is required for an area ability; a
# creature-targeted ability defaults to the triggering creature.
combat-sim --state fight.state.json ready Hero \
  --on ability --trigger "the cultist casts" --ability sacred-flame --target Cultist

# Movement response to a reachable zone or grid cell.
combat-sim --state fight.state.json ready Hero \
  --on manual --trigger "the door opens" --zone cellar
combat-sim --state fight.state.json ready Hero \
  --on manual --trigger "the door opens" --to 3,2

# Fire it explicitly (DM trigger). --source names the creature that caused it.
combat-sim --state fight.state.json trigger Hero --source Ogre

# Clear it without triggering.
combat-sim --state fight.state.json ready-clear Hero
```

The `ready` command is only legal on the holder's own turn and has no
`--force` escape hatch. `trigger` and `ready-clear` are out-of-turn commands.
`status --json` exposes the full structured ready entry under each combatant;
the text status appends `| ready: ...` only when one is prepared.

## Trigger kinds

| `--on` | Fires when |
|--------|------------|
| `manual` | Only an explicit DM `trigger` command. |
| `enter-reach` | An enemy crosses from outside the response's reach/range to inside it during movement (normal or forced). |
| `attacked` | An enemy resolves an attack. The ready reaction happens after that attack finishes. |
| `ability` | An enemy resolves an ability or spell. The reaction happens after the ability finishes. |
| `turn-start` | An enemy starts its turn. The holder's own ready expires at the holder's turn start instead. |

An explicit `trigger` works for every kind, including `manual`. Automatic
matching uses the structured kind; the description is audit text only.

The watch distance for `enter-reach` is the response's own reach or range:
melee reach, normal ranged range, ability range (or 5 ft for a self/movement
response). A move that starts inside the reach does not count as entering it.

## Timing and cleanup

- Setting Ready spends the holder's action and stores exactly one response.
- Setting an **ability** response also pays its `Ability.cost` (spell slot or
  resource) immediately and, when the ability requires concentration, begins
  that concentration immediately. This is RAW: setting Ready casts and holds
  the spell. A failed affordability/legality check spends no action, cost or
  concentration.
- A trigger releases the held ability without paying its cost again or
  starting concentration again; it spends the holder's reaction exactly once.
  A reaction already used this round refuses the trigger and keeps the ready.
- Expiry is at the start of the holder's next turn. `start_turn` clears the
  ready before the new turn's triggers run.
- `ready-clear` clears it immediately.
- Death, 0 HP, incapacitation and dormancy clear it. The death path logs the
  loss before concentration cleanup.
- An untriggered, expired or cleared ready never refunds the slot/resource.
  If the Ready started concentration, clearing/expiry/death/0 HP/incapacitation/
  dormancy ends **that** concentration (matched by ability id); an unrelated
  concentration is left untouched.
- Starting another concentration replaces a held readied spell's concentration
  and clears the ready (no refund), because the new concentration ends the old
  one. A concentration broken by damage leaves the stored spell unusable: the
  trigger is refused and the slot/resource is still not refunded.
- A legal refusal (dead target, spent reaction, out-of-range target, stale
  expiry) logs the reason and keeps the ready, its paid resource and its
  concentration; no reaction is spent.
- If a readied move is blocked by a locked door or wall, the existing movement
  path stops at the blocker and logs it. The response counts as triggered and
  spends the reaction; the creature simply stops where the route ended.

## Resolution paths

The response never teleports and never bypasses a normal check:

- **Weapon attack** resolves through `resolve_attack` with the normal reach,
  range, line-of-sight, cover, charmed-source, target and condition checks. It
  is one attack; Extra Attack slots do not apply to a reaction.
- **Unarmed Strike** builds the same unarmed profile as the Attack action and
  resolves through `resolve_attack`.
- **Ability** resolves through the normal `use_resolved_ability` path with the
  action spend suppressed (the ready already spent the action) and with the
  cost/concentration marked as prepaid. Activation must be `action`;
  bonus-action, reaction, passive and free abilities are refused when the ready
  is set. Target, area, self and range checks still run at release.
- **Movement** uses `move_to_at`, so step costs, opportunity attacks, frightened
  movement restrictions and scene triggers are unchanged.

Automatic ready reactions are suppressed while another ready reaction is
resolving, so a ready response cannot recursively trigger a second ready.

## Data seam

`rules.ready` is optional:

```yaml
rules:
  ready:
    max_responses: 1
    expires: start_of_next_turn
```

Both editions share the same Ready shape, so there is no edition branch in the
engine. The common defaults apply when the data is absent: one response per
combatant and expiry at the holder's next turn start. `content check` rejects
`max_responses != 1`. Existing content without `rules.ready` loads unchanged.

## Compatibility

- `STATE_SCHEMA_VERSION` stays **1**; `ready` is additive and serde-defaulted.
- No new persistent field is needed for prepayment: the stored ability id
  identifies the concentration that belongs to the ready, and the cost is paid
  when the ready is written. A released state has no ready field; only a state
  written by the pre-RAW revision of this unmerged change could contain an
  unpaid ready, and re-setting that ready is enough.
- Old combat states load with no ready.
- Old `attack --force` and `use` behavior is unchanged; the automatic hooks do
  nothing when no combatant has a ready.
- No dependency, `space.rs`, AI or loader change is required.
- Logs contain only the holder's name, the DM-supplied description (whitespace
  collapsed and capped at 160 characters), the trigger kind, the response
  summary and the refusal/expiry reason. No save text or book text is added.

## Known limits

- A readied spell is prepaid at setup. If its concentration is broken or
  replaced before release, the stored spell cannot be released and the slot or
  resource is not refunded; a replacement clears the ready, while a broken
  concentration leaves it until expiry and refuses every trigger.
- Ready only stores actions, not bonus actions or reactions.
- Nested ready chains are deliberately suppressed to keep resolution bounded.
- The engine has no `remove combatant` command; death and `ready-clear` cover
  the removal cleanup. A dormant combatant loses its ready on the next turn
  start or on an explicit trigger attempt.
- A readied move that is partially blocked stops at the blocker and still
  spends the reaction (structured log, not a rollback).
- An explicit `trigger` that is refused still exits successfully after saving
  the refusal to the combat log; the refusal reason is the actionable output.

## Test coverage

`tools/combat-sim/tests/combat_ready.rs` covers both editions for: set/repeat/
no action/incapacitated/no reactions, expiry at turn start and by expiry round,
explicit trigger, all four automatic triggers, reaction already spent, dead and
out-of-range targets, blocked movement, movement execution, death/incapacitation/
dormancy cleanup, old state/schema, old `attack --force`/`use`, CLI help and
JSON round-trip, `rules.ready` validation, slot/resource payment at setup with
no double payment at release, no refund on clear/expiry/0 HP, concentration at
setup with no re-begin at release, concentration cleanup on clear/expiry,
replacement by a new concentration, insufficient resource/illegal response
before spending, and refused triggers keeping the paid resource and
concentration.
