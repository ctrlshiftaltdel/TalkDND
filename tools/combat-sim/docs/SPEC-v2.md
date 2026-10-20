# combat-sim v2 — implementation spec (companion to DESIGN-v2.md)

Goal: a GENERIC 5e (2024) combat engine. **No content names in src/** (no spell, feature, monster,
mastery, condition-specific or campaign-specific identifiers/branches). All content is data.
Core 5e rules that every game shares stay in code: d20 tests, attack/AC, saves, damage types &
resist/immune/vuln, temp HP, crits, initiative, turns, actions/bonus/reaction economy, movement,
death saves (kind=pc), opportunity attacks, cover math, basic actions (dash/disengage/dodge/hide/
help/ready-as-manual), concentration (one at a time, CON save DC max(10, dmg/2) on damage).
Conditions are DATA (their mechanical effects are declared with the vocabulary below); the engine
only knows generic condition *flags* (incapacitated, speed_zero, no_reactions,
no_opportunity_attacks, stand_up_half_speed) and modifiers.

Target ~80% automation. Anything not expressible → `manual:` text + `note` effect (logged as
"MANUAL: ..."), DM resolves with generic commands (roll/damage/heal/condition/move...).
Do NOT build a scripting language.

## 1. Content files & search path
Formats: YAML (`.yaml/.yml`) and JSON (`.json`). YAML crate: `serde_norway` (maintained
serde_yaml fork; fall back to `serde_yaml` 0.9 if needed).
A content file is a map with any of these top-level keys (each a list):
`conditions, masteries, abilities, creatures, ai_templates, scenes, encounters` plus an optional
single map `rules` (core-rule wiring, see §5b).
Markdown files (`.md`) contribute content via fenced blocks whose info string is
` ```combat yaml ` or ` ```combat json `; a block holds ONE creature (map with `id`), or a map with
the top-level keys above. Everything else in the .md is ignored.

Content sources, loaded in order (later overrides earlier by `id` within the same kind):
1. library dirs (`--lib DIR`, repeatable) — recursive scan
2. adventure dirs (`--adventure DIR`, repeatable)
3. character files/dirs (`--chars PATH`, repeatable; .md/.yaml/.json)
Or a campaign manifest `--campaign FILE` (also env `COMBAT_SIM_CAMPAIGN`):
```yaml
# LetsPlay/Saves/2026-10-19-01/combat/campaign.yaml  (paths relative to this file)
name: TalkDND save 2026-10-19-01
library: [../../../Rules/combat]          # LetsPlay/Rules/combat; Rules moves with Saves
adventures: [../../../Adventures/keys-from-the-golden-vault/combat]
characters: [../Characters, ../NPCs]        # dirs: all *.md with combat blocks
state_dir: .                                # default dir for --state names without '/'
```
Explicit flags add to the manifest. `combat-sim content list [--kind K]` and
`combat-sim content show <id>` print what was loaded (and from which file) — useful for
debugging. `combat-sim content check` validates all refs and reports errors/warnings (TODO notes
are listed as warnings so they stay visible).
The encounter state file stores the resolved content snapshot it needs (creatures, abilities,
conditions, masteries, templates, scene) so step commands work without re-reading content;
`--campaign` is only needed for `new`, `simulate`, `content`.

IDs: lowercase-hyphen strings, unique per kind. References are by id.

## 2. Expressions
Numbers or dice strings with `{var}` substitution, e.g. `"1d10+{level}"`, `"8+{attack_mod}+{prof}"`,
`"2+{slot_level}"`, `"{param.dice}"`. Vars: `str dex con int wis cha` (modifiers), `prof`, `level`,
`spell_mod`, `spell_attack`, `spell_dc`, `slot_level`, `attack_mod` (ability mod of the attack
in progress), `param.<name>` (params given where the ability is referenced). Evaluate → DiceExpr.
Logs show the substituted formula and rolls.

## 3. Creature (library monster, adventure creature, or PC block)
```yaml
id: guard
name: Guard
kind: monster            # monster | pc   (pc => death saves, stays at 0 HP)
base: guard              # optional: inherit from another creature id, then override fields
side: enemy              # default; encounters may override
size: medium
creature_type: humanoid
level: 1                 # for {level}
ac: 16
hp: 11                   # max HP (current HP may be given as hp_current)
hp_current: 11
speed: {walk: 30, climb: 0, fly: 0, swim: 0}
init_bonus: 1
prof: 2
abilities: {str: 13, dex: 12, con: 12, int: 10, wis: 11, cha: 10}
saves: {}                # total bonuses; missing => modifier
skills: {perception: 2}
spellcasting: {ability: int, attack: 5, dc: 13}   # optional
resistances: [], immunities: [], vulnerabilities: [], condition_immunities: []
senses: "Passive Perception 12"
resources:               # named pools
  spell-slot-1: {max: 2, recharge: long}
  second-wind: {max: 2, recharge: long, short_rest_regain: 1}
  # recharge: turn | round | short | long | never | "recharge 5-6"(rolled at start of turn)
attacks:                 # shorthand for weapon/natural attack abilities
  - id: spear
    name: Spear
    kind: melee          # melee | ranged | melee_or_ranged
    ability: str         # sets {attack_mod}
    to_hit: 3
    damage: [{dice: "1d6+1", type: piercing}]
    reach: 5
    range: [20, 60]
    properties: [thrown, versatile]     # finesse, light, thrown, ranged, versatile, heavy ...
    mastery: sap         # id from masteries
    on_hit: []           # extra EFFECTs
actions: [ {ref: magic-missile}, {ref: second-wind}, {inline ability...} ]  # abilities usable
features: [ {ref: sneak-attack, params: {dice: 1d6}} ]                     # passives/triggers
multiattack: [slam, slam]   # ids of attacks/actions, used by AI and `attack --multi`
conditions: []
dormant: false              # dormant creatures don't act/aren't targeted until activated
ai: {template: brute, params: {}}
notes: "..."
todo: ["Guard AC: OCR 15 vs 16 by gear"]
```
A `{ref, params, cost, name}` entry may override the referenced ability's `cost`/`name`.

## 4. Ability (spells, features, item/monster actions, traps' effects)
```yaml
id: fire-bolt
name: Fire Bolt
tags: [spell, cantrip]           # free tags; AI and predicates may use `spell`, `heal` ...
activation: action               # action | bonus | reaction | passive | free
cost: {resource: spell-slot-1, amount: 1}     # optional; slot casts set {slot_level}=1
slot_level: 1                    # optional explicit
range: 120                       # ft; 0 = self; 5 = touch
target: {type: creature, side: enemy, count: 1}   # type: creature|self|zone ; side: enemy|ally|any
area: {radius: 5}                # optional AoE: zone mode = target zone (+ adjacent zones whose edge distance <= radius); grid = cells within radius
concentration: false
duration: {rounds: 10}
effects: [EFFECT...]
triggers: [TRIGGER...]           # passive/reaction behaviour
modifiers: [MODIFIER...]         # always-on while owner has this feature
ai: {role: damage}               # damage | heal | self_heal | buff | control | defense | none
manual: "what DM must adjudicate"
source: "PHB 2024 p.xxx"
todo: []
```
EFFECT (a map with exactly one primary key; small fixed set):
- `attack: {to_hit: 5 | "{spell_attack}", kind: melee|ranged, damage: [{dice, type}], on_hit: [E], on_miss: [E], properties: [], mastery: id}`
- `save: {ability: dex, dc: 13|"{spell_dc}", half: false, ignore_cover: false, on_fail: [E], on_success: [E]}` (damage inside on_fail is halved for on-success when half=true)
- `damage: {dice: "1d4+1", type: force | weapon, repeat: 1, crit: inherit|never}` (`weapon` = type of triggering attack; inside on_hit, crit doubles dice unless crit: never)
- `heal: {dice: "2d8+{spell_mod}"}`
- `apply_condition: {condition: id, to: target|self, duration: DURATION, link: true, save_ends: {ability, dc, at: end_of_turn}}`
- `remove_condition: {condition: id, from: target|self}`
- `forced_move: {distance: 10, direction: away|toward, max_size: large}` (zone mode: moves to the adjacent zone farthest from/closest to source if edge distance <= distance; grid: straight line)
- `use: <ability-or-attack-id>` (perform another of the owner's abilities, e.g. multiattack)
- `activate: {creature: <instance name | tag>}` (wake dormant creatures; they roll initiative and join)
- `check: {skill|ability, dc, who: actor, on_success: [E], on_fail: [E]}`
- `basic_action: dash|disengage|dodge|hide|help`
- `choice: [[E...], [E...]]` (CLI `--option N`; AI picks first useful)
- `note: "text"` → logs `MANUAL: text`
DURATION: `{rounds: N}` (ends at end of affected creature's Nth turn) | `{until: end_of_source_next_turn | start_of_source_next_turn | end_of_target_next_turn}` | `{minutes: 1}` (=10 rounds) | `{permanent: true}`; optional `ends_on: [damage_taken, own_attack, own_spell, deals_damage]`.

MODIFIER:
```yaml
{applies: attack_roll | incoming_attack | save | check | ac | damage_roll | damage_taken | speed | healing_done | healing_received,
 effect: advantage | disadvantage | bonus | auto_fail | auto_crit | resistance | immunity,
 value: 1 | "1d4" | "-10" | "2+{slot_level}",
 filter: {ability: [str, dex], damage_type: [fire], attack_kind: [melee], tags: [spell]},
 when: PREDICATE, once_per: turn, consume: true}   # consume: removed (condition ends) after first use
```
PREDICATE (combine with `all: [...]`, `any: [...]`, `not: P`):
`has_advantage`, `has_disadvantage`, `ally_within_5_of_target` (non-incapacitated ally of the actor),
`target_has_condition: id`, `self_has_condition: id`, `attacker_within: ft`, `attacker_beyond: ft`,
`linked_is_attacker` / `linked_is_target` (condition instance's linked creature),
`attack_has_property: [finesse, ranged]` (any listed property matches; a ranged-kind attack
counts as `ranged`), `attack_kind: melee|ranged`,
`self_hp_below_pct: N`, `target_hp_below_pct: N`, `is_spell`, `slot_level_min: N`,
`damage_dealt` (>0 in this hit), `would_miss_with_ac_bonus: N` (for reactions like Shield),
`target_size_max: large`.

TRIGGER: `{on: on_hit | on_miss | on_hit_taken | on_damage_taken | start_of_turn | end_of_turn | on_enter_area | on_interact | on_heal_cast,
 when: PREDICATE, once_per: turn|round, reaction: true, cost: COST, effects: [E]}`
Reactions are offered automatically in step mode and sim (AI decides: use if predicate true and
template allows; `--no-reactions` disables); one reaction per round.

## 5. Conditions & masteries are data (shared library)
```yaml
conditions:
  - id: prone
    flags: [stand_up_half_speed]
    implies: []
    modifiers:
      - {applies: attack_roll, effect: disadvantage}
      - {applies: incoming_attack, effect: advantage, when: {attacker_within: 5}}
      - {applies: incoming_attack, effect: disadvantage, when: {attacker_beyond: 5}}
masteries:
  - id: sap
    triggers: [{on: on_hit, effects: [{apply_condition: {condition: sapped, to: target, duration: {until: start_of_source_next_turn}}}]}]
```
Engine-known flags only: `incapacitated, speed_zero, no_reactions, no_opportunity_attacks,
stand_up_half_speed, unaware`. Dying PCs automatically get the condition ids listed in engine
config `dying_conditions` (default `[unconscious]`, resolved from data).

## 5b. `rules` map (data wiring for core rules, in the shared library)
```yaml
rules:
  dying_conditions: [unconscious]      # applied when a pc drops to 0 HP; removed on heal
  dead_monster_condition: null
  basic_actions:                       # condition ids the basic actions apply to the actor
    dodge: dodging
    disengage: disengaged
    hide: hidden
    help: helped                       # applied to the helped ally's/target
  cover: {half: 2, three_quarters: 5}
  concentration_condition: concentrating
```
The engine reads condition ids only from here; it never hardcodes them.

## 6. Space
### Zone graph (default)
```yaml
scenes:
  - id: v12
    name: Prehistoric Predators
    mode: zones            # zones | grid
    light: dim
    zones:
      - {id: floor, cover: none, elevation: 0, light: dim, tags: [], description: "..."}
      - {id: vent, elevation: 10, cover: half}
    edges:
      - {a: floor, b: vent, distance: 10, climb: {skill: athletics, dc: 10}}   # climb check on entering (step mode: rolled automatically, fail = movement stops)
      - {a: floor, b: hall, distance: 20, door: {locked: false}, difficult: false, one_way: false}
    objects:
      - {id: controls, zone: display, name: "...", interact: {check: {skill: arcana, dc: 10}, on_success: [E], on_fail: [E]}, manual: "..."}
    triggers:
      - {on: on_enter_area, zone: statue, side: party, once: true, effects: [{activate: {creature: "Satyr Statue"}}]}
```
- Distance between zones = shortest path (edge distances; difficult edges cost double for
  movement only). Same zone ⇒ within 5 ft. Vertical: effective distance for reach/range adds
  |elevation difference| (so a melee 5-ft reach can't hit someone in a 10-ft-up vent).
- Melee reach: same zone (and same elevation) or adjacent zone with distance <= reach.
- Cover: a creature in a zone with cover gets that cover vs attacks/Dex saves originating from a
  DIFFERENT zone: half +2 AC/Dex save, three_quarters +5, total = can't be targeted. Abilities
  with `ignore_cover` skip it. Light: `dark` ⇒ attacks into it have disadvantage unless attacker
  has darkvision/blindsight (parse `senses`), attacks from it have advantage (unseen) — keep simple.
- Opportunity attacks: when a creature leaves a zone (move/AI movement) that contains a hostile
  with reaction available, melee attack, not incapacitated, no `no_opportunity_attacks` flag, and
  the mover didn't Disengage ⇒ hostile makes one melee attack (auto in sim, auto in step mode
  unless `--no-oa`).
- on_enter_area triggers fire when a creature enters the zone (filter by side optional, `once`).
### Optional grid (per scene, `mode: grid`)
```yaml
    grid:
      cell: 5
      legend: {"#": {wall: true}, ".": {}, "~": {difficult: true}, "h": {cover: half}, "H": {cover: three_quarters}, "^": {elevation: 10}, "S": {zone: statue}}
      rows: ["#######", "#..S..#", "#.~~h.#", "#######"]
      zones: {statue: "S"}   # optional: legend chars that map to zone ids for triggers
```
Positions `[x, y]`. Distance: 5 ft per step incl. diagonals (2024), plus
|elevation(from) - elevation(to)| for attack range/reach (movement stays
cell-based). Movement: BFS/Dijkstra with
difficult = 2x, walls impassable. Line of sight: Bresenham through cell centers; wall blocks;
max cover among intervening cover cells (and target cell's own cover property if attacker not
adjacent). Adjacent (≤5 ft) for melee/OA. Area radius = cells within radius. Keep it modest.
Both modes implement one `Space` trait: distance(a,b), can_reach(a,b,reach), cover(attacker,target),
move_toward/away, adjacent creatures, creatures in area, enter-triggers.

## 7. Encounters
```yaml
encounters:
  - id: v12-satyr-and-guards
    scene: v12
    party_zone: east-door        # default zone for PCs added with --party
    members:
      - {creature: guard, count: 2, zone: floor, ai: {template: guardian, params: {guard_zone: floor}}}
      - {creature: satyr-statue, zone: statue, dormant: true}
```
CLI: `new --campaign C --encounter ID [--party will,rina,milo,sera] [--party-zone Z] [--add id[:count][@zone]]... --state NAME`.
Positions in grid mode: `zone` means "any free cell tagged with that zone char", or explicit `pos: [x,y]`.

## 8. AI templates (data)
```yaml
ai_templates:
  - id: brute
    params: {targeting: nearest, range_pref: melee, keep_distance: 0, self_heal_below_pct: 50,
             heal_ally_below_pct: 0, heal_downed: false, resource_threshold: 1.5, flee_below_pct: 0,
             guard_zone: null, leash: null, attack_downed: false, opening: [], avoid_targets_with: [sanctuary-warded]}
```
Param meaning (fixed set, all optional): `targeting` nearest|lowest_hp|highest_damage|random;
`range_pref` melee|ranged|any; `keep_distance` ft (ranged/casters back off if enemy adjacent,
using disengage-like abilities if available as bonus); `self_heal_below_pct`; `heal_ally_below_pct`;
`heal_downed`; `resource_threshold` (use costed damage abilities only if expected damage >= X ×
best at-will); `flee_below_pct` (move away from enemies / toward optional `flee_zone`, dash);
`guard_zone`+`leash` (don't leave guard_zone unless enemy within leash ft); `attack_downed`;
`opening` (ability ids to use on first turn if available & sensible, e.g. a buff);
`avoid_targets_with` (condition ids — prefer other targets). Creature `ai.params` override.
Turn flow: dying → death save; incapacitated → skip; flee check; bonus self-heal; heal ally/downed;
opening buff; choose target; choose best ability/attack/multiattack by expected value (hit
chance incl. adv/dis & cover, save chance, crit, resist/vuln/immune); move (respect speed,
OAs, climb checks, leash); attack; bonus-action options (abilities with activation bonus and
ai.role matching need).

## 9. State / CLI / sim (keep v1 behaviour)
- Step mode: OsRng per roll; simulate: OsRng-seeded StdRng per thread; `--seed` for tests.
- Logs show full formulas (`1d20(14)+6+2(cover)=..`, modifiers named by source, e.g.
  `+1d4(3) bless`, `adv: vexed`).
- Commands from v1 remain (new, init, status, attack, damage, heal, save, check, roll, cast,
  use, condition, move, deathsave, next, log, rest, simulate) adjusted for content refs; add
  `interact <who> <object> [--option N]`, `activate <name>`, `content list|show|check`,
  `scene` (print zones/edges/objects/positions compactly as text, no map drawing).
- `--state NAME` without a slash resolves to `<manifest.state_dir>/NAME.state.json`.
- Simulate: `simulate --encounter ID [--party a,b,c] [--from-state NAME] [--runs N]
  [--max-rounds 100] [--threads T] [--json]`. `--encounter`, `--from-state` and a bare
  `--state` source are mutually exclusive. `runs=1` prints the play-by-play and a summary;
  `runs>1` uses rayon with one prepared template/AI plan set, per-worker seeded RNG and a
  `NullSink`, and reports runs/sec, wins/draws/timeouts, rounds and HP statistics. Reaching
  `--max-rounds` without an outcome is a timeout; no living side is a draw.
- Engine logs use a `LogSink`; a disabled sink (Monte Carlo) performs no formatting.

## 10. Tests
Synthetic fixtures only under tests/fixtures/ (generic names like "brute-a", "test-sword").
Real-content smoke test (ignored if paths missing): load the TalkDND campaign manifest, run
`content check`, and simulate each adventure encounter once.
Grep test: a test that scans src/ for a deny-list of content words (sneak, wind, missile, sap,
vex, topple, graze, nick, cleave, guard, mimic, allosaurus, scarecrow, will, rina, milo, sera,
bless, sanctuary, shield, prone, frightened, paralyzed, unconscious, grappled, poisoned ...) as
whole identifiers/strings and fails if found (allow generic words via explicit allow-list
where unavoidable, e.g. `unconscious` only via data-config default string — prefer config).

## 11. Additions found while writing the real content (binding)
- Every content item (creature, attack, ability, condition, mastery, scene, zone, object, trigger,
  encounter, template) may carry optional `name`, `notes`, `manual`, `source`, `todo: [..]`, `description`.
- Creature: `tags: [..]`; `side` may be any string (e.g. `party`, `enemy`, `rampage`); every side
  is hostile to every other side. Combat ends when ≤1 side has active (non-dormant, conscious)
  members. Encounter member fields: `creature, count, zone, pos, side, dormant, ai, name`.
- `features:`/`actions:` entries may be `{ref: id, params, cost, name}` OR a full inline ability.
- Condition fields: `id, name, flags, implies, modifiers, ends_on, duration_default, manual, todo`.
  Extra engine flag: `no_opportunity_attacks_against` (movement of this creature provokes no OA).
- `apply_condition` may carry `ends_on: [damage_taken, own_attack, own_spell, deals_damage]`.
- New effect `toggle_trigger: {trigger: <scene trigger id>, enabled: false|true}`.
- `activate: {creature: X}` matches instance name, creature id, or creature tag (all matches).
- Scene trigger fields: `id, on, zone | zones: [..], side (only creatures of that side fire it),
  once, enabled (default true), affects: entering | zone_occupants (default entering; with
  zone_occupants the effects apply to every creature currently in `zones`), effects`.
- Object `interact`: either `{check, on_success, on_fail}` / `{effects}` directly, or
  `{options: [{name, check?, on_success?, on_fail?, effects?}]}` (CLI `interact <who> <object> --option N`, 1-based).
  `check: {skill: arcana | ability: dex, dc: N}` is rolled for the interacting creature.
- Grid legend cell props: `wall, difficult, cover, elevation, zone` (zone ⇒ that cell belongs to
  the zone id, so triggers/encounter placement by zone work in grid scenes). `rows` are strings,
  `rows[0]` is y=0, x = column index.
- Attack `kind: melee_or_ranged` = usable as melee (reach) or ranged/thrown (range).
- Ability with `target.type: zone` + `area.radius`: AI picks the enemy-densest reachable zone.
