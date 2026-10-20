# combat-sim — SPEC v1

A headless D&D 5e (2024 rules) combat helper for the TalkDND text campaign. No UI, no map
rendering. The DM bot calls it from the shell. Rust, binary name `combat-sim`, edition 2021.
Toolchain: rustup stable at ~/.cargo/bin (put it first in PATH). Allowed crates: clap (derive),
serde, serde_json, toml, rand (0.9) + rand_chacha if needed, anyhow/thiserror, rayon. Nothing else
without good reason.

## 1. Dice (src/dice.rs)
- Expression parser: `NdM`, `NdM+K`, `NdM-K`, `K`, sums like `2d8+1d6+3`, `d20` (= 1d20).
  Errors on garbage. Struct `DiceExpr { terms: Vec<(count, sides)>, modifier }`.
- RNG: trait-object or generic `R: rand::RngCore`. Default for step-by-step commands =
  `rand::rngs::OsRng` (OS entropy, every roll). `simulate` = a fresh CSPRNG (StdRng/ChaCha) seeded
  from OsRng once per process/thread (still OS entropy, but fast). `--seed N` (global flag) only
  for tests/reproducibility -> seeded StdRng everywhere. Never seed by default.
- Every roll returns a record with: formula, raw dice values, modifier, total, and a formatted
  string, e.g. `1d20(14)+6=20 vs AC 15 HIT`, `1d20(3,17 adv→17)+5=22 vs DC 14 PASS`,
  `2d8(3,6)+3=12 slashing`.
- d20 with advantage/disadvantage (both cancel). Nat 20 on an attack = crit (auto hit, roll all
  damage dice twice, modifier once). Nat 1 = auto miss.

## 2. Data model (src/model.rs)
Combatant files are JSON (`.json`) or TOML (`.toml`); a file holds one combatant object.
Fields (serde, most optional with defaults):
```json
{
  "name": "Will", "side": "party", "kind": "pc",          // kind: pc | monster ; pc => death saves
  "hp": 12, "max_hp": 12, "temp_hp": 0, "ac": 18, "init_bonus": 2, "speed": 30,
  "abilities": {"str":18,"dex":14,"con":14,"int":16,"wis":10,"cha":12},
  "saves": {"str":6,"con":4},          // missing ability => plain modifier
  "skills": {"athletics":6,"perception":2},
  "creature_type": "humanoid",
  "zone": "front",                      // optional default zone
  "attacks": [ {
      "name": "longsword", "kind": "melee",          // melee | ranged | spell_attack (uses to_hit) | save | auto (auto-hit, e.g. magic missile)
      "to_hit": 6, "damage": "1d8+4", "damage_type": "slashing",
      "extra_damage": [{"dice":"1d8","type":"acid"}],
      "reach": 5, "range": [30,120],                  // range = normal/long feet for ranged/thrown
      "mastery": "sap",                               // sap|slow|nick|vex|topple|graze|push|cleave (optional)
      "sneak_eligible": true,                         // finesse or ranged weapon
      "count": 1,                                     // e.g. magic missile darts = 3 (each rolled separately)
      "cost": {"resource":"slot1","amount":1},        // optional limited use
      "save": {"ability":"dex","dc":13,"half_on_success":false},   // for kind=save
      "on_hit_conditions": [{"condition":"grappled","escape_dc":13,"duration":null}],
      "on_fail_conditions": [{"condition":"frightened","duration":{"until":"end_of_source_next_turn"}}],
      "vs_condition": {"condition":"grappled","advantage":true,"damage":"2d8+3"}  // mimic bite
  } ],
  "multiattack": ["slam","slam"],       // AI/`attack --multi` uses this sequence
  "heals": [ {"name":"cure_wounds","amount":"2d8+6","range":5,"action":"action","cost":{"resource":"slot1","amount":1},"self_only":false},
             {"name":"second_wind","amount":"1d10+1","range":0,"action":"bonus","cost":{"resource":"second_wind","amount":1},"self_only":true} ],
  "reactions": [ {"name":"shield","ac_bonus":5,"cost":{"resource":"shield_free","amount":1}} ],
  "resources": {"slot1":{"current":2,"max":2,"reset":"long"}},
  "features": {"sneak_attack":"1d6"},
  "resistances": [], "immunities": [], "vulnerabilities": [], "condition_immunities": [],
  "conditions": [],
  "notes": "free text, TODOs"
}
```
Zones: abstract positions on a line, in feet. An encounter file (JSON) lists zones and members:
```json
{ "name":"party vs 2 guards",
  "zones": {"front":0, "hall":30, "far":60},
  "members": [ {"file":"../party/will.json","zone":"front"},
               {"file":"../monsters/guard.json","count":2,"zone":"hall"} ] }
```
Distance between two combatants = |pos(zoneA)-pos(zoneB)|. Same zone = 0 ft = within 5 ft
(engaged/melee reach). Default when no zones: everyone in zone "main" (pos 0).
Duplicates get names "Guard 1", "Guard 2". Name lookup on the CLI is case-insensitive, accepts
a unique prefix; ambiguous => error listing candidates.

## 3. Encounter state (src/state.rs) persisted to JSON (`--state path`)
Contains: combatants (full runtime state: hp, temp_hp, conditions with durations, resources,
death saves {successes, failures, stable, dead}, per-turn flags: reaction_used,
sneak_used_this_turn, action/bonus used, speed_left, sapped/vexed markers), zones, initiative
order, current turn index, round number, and the full log (Vec<String>). Load -> mutate -> save on
every command. Round-trip must be lossless.

## 4. CLI (clap). Global flags: `--state <file>` (default ./combat-state.json), `--seed <u64>`, `--json`.
- `new <spec>...` / `new --encounter enc.json` : spec = `file.json[:count][@zone]`.
- `init [--set Name=total ...]` roll initiative (d20+init_bonus, tie-break by bonus then dex),
  sort, set round 1, turn 0. Log each roll.
- `status [--json]` compact one line per combatant: marker for current turn, init, name, side,
  HP/max(+temp), AC, zone, conditions(with remaining), resources, death saves. Round + whose turn.
- `attack <attacker> <target> <attack> [--adv] [--dis] [--no-sneak] [--multi]` resolves to-hit
  (or save for kind=save, or auto), auto-applies adv/dis from conditions (prone/frightened/
  poisoned/blinded/restrained/paralyzed/unconscious/sapped/vexed, ranged attack while an enemy
  is within 5 ft = dis, long range = dis, out of range = error), crits (paralyzed/unconscious
  target hit from within 5 ft = auto crit), resistances/immunities/vulnerabilities, temp HP,
  sneak attack once per turn (advantage, or an allied conscious creature in target's zone, and
  no disadvantage, attack sneak_eligible), the target's Shield reaction (auto if it turns a hit
  into a miss and resource available, unless `--no-reactions`), mastery effects (Sap: target dis
  on its next attack roll before attacker's next turn; Vex: attacker adv on next attack vs that
  target before end of its next turn; Topple: target Con save DC 8+attack ability mod+prof
  → Prone; Slow: target speed -10 until attacker's next turn; Graze: on miss deal attack ability
  modifier damage; Nick/Push/Cleave: log a reminder only), on-hit conditions, cost spending.
  Damage to 0: monster dies; PC goes unconscious (Prone+Unconscious) and starts death saves;
  damage while at 0 = 1 death-save failure (2 on crit); massive damage (leftover >= max_hp) = dead.
- `damage <target> <expr|N> [--type t] [--crit]` ; `heal <target> <expr|N> [--from <healer>]`
  (heal from 0 HP clears unconscious & death saves). Life Domain bonus is already baked in data.
- `save <who> <ability> <dc> [--adv] [--dis]` (paralyzed/unconscious auto-fail str/dex).
- `check <who> <skill|ability> <dc> [--adv] [--dis] [--bonus N]`.
- `roll <expr> [--adv] [--dis] [--dc N]` generic roll.
- `cast <who> <name> [targets...] [--adv] [--dis]` : runs a named attack/save/heal entry from the
  combatant data that has a cost (spends resource; error if empty). `use <who> <resource|heal-name>
  [--amount N]` spends a resource; if the name is a heal entry (e.g. second_wind) it runs it.
- `condition add <who> <cond> [--rounds N] [--until-end-of-turn <name>] [--source <name>]`,
  `condition remove <who> <cond>`. `--rounds N` = expires at the end of the affected creature's
  Nth turn. Respect condition_immunities.
- `move <who> <zone>` (logs distance; warns if more than speed_left, does not block).
- `deathsave <who>` manual roll. Rules: d20 >=10 success, <10 fail, nat1 = 2 fails, nat20 =
  regain 1 HP & conscious. 3 successes = stable, 3 failures = dead.
- `next` : end current turn (tick "end of turn" durations, expire Sap/Vex etc.), advance to next
  living combatant (skip dead monsters and dead PCs; PCs at 0 still get turns for death saves:
  auto-roll the death save at the start of their turn and log it), wrap → round+1. Reset reaction,
  sneak, action flags, speed_left at start of the creature's turn. Print new round/turn + status
  line for the active creature. Report if one side has no conscious members (combat over).
- `log [--tail N]`.
- `rest short|long <who|all>` restore resources by reset type (short restores short ones; long
  restores all + full HP).
- `simulate` (see §5).
All human-readable output is concise plain text, one roll per line. `--json` gives machine output
for `status`, `simulate`.

## 5. Auto mode (src/ai.rs, src/sim.rs)
`simulate [<spec>... | --encounter enc.json | --from-state] [--runs N] [--max-rounds 100]
[--seed S] [--json] [--threads T]`
- runs==1 (default): full play-by-play log printed (same format as step mode), then a summary.
- runs>1: Monte Carlo, no string formatting in the hot loop (logging behind a flag / no-op sink),
  parallel with rayon, each thread its own RNG seeded from OsRng (or from --seed + thread index).
  Report: runs, win rate per side (and draws/timeouts), avg/median rounds, avg HP left per side and
  per combatant, per-combatant chance of ending at 0 HP ("down") and dead, elapsed time, runs/sec.
- Simple AI per creature turn:
  1. If dead/at 0 → death save (PC) / skip.
  2. Bonus-action self heal (e.g. Second Wind) if HP <= 50% and available.
  3. If it has a heal and an ally is at 0 HP (dying) or <= 30% HP and reachable (move first if
     needed), heal the most hurt one (action heal) — at most one action heal per turn.
  4. Otherwise pick target: valid enemies (conscious; for monsters also may attack downed PCs only
     if no conscious enemy within reach — prefer conscious), nearest first, then lowest HP.
  5. Choose the attack (or multiattack sequence) with best expected damage vs that target (hit
     chance from AC/save, resist/vuln/immune, crit chance); resource-costing attacks are used only
     if expected damage >= 1.5x best at-will option; ranged attackers stay put if in range, melee
     move to the target zone (speed limit: move up to speed; dash if can't reach this turn).
  6. Reactions: Shield auto as in step mode.
  7. Conditions honored: paralyzed/unconscious/incapacitated creatures skip turn; frightened (dis);
     grappled = speed 0.
- Combat ends when one side has no conscious members, or max rounds (timeout).
- Must be fast: target >= 50k runs/sec for a party-vs-2-guards fight on 8 cores.

## 6. Data files (data/)
`data/party/{will,rina,milo,sera}.json`, `data/monsters/{guard,animated-armor,scarecrow,mimic,
allosaurus,allosaurus-animatronic}.json`, `data/encounters/{party-vs-2-guards,party-vs-satyr-statue,
party-vs-mimic,party-vs-animatronic-allosaurus,party-vs-scarecrow}.json`.
These files ALREADY EXIST (written by the DM from the books; see docs/DATA.md). The Rust schema must load
them unchanged. Any object (combatant, attack, heal, resource, ...) may carry an optional `notes` string.
Do not change the numbers.

## 7. Tests (cargo test)
Dice parsing (valid/invalid), crit doubling, nat-1 auto miss, adv/dis selection (seeded), state
JSON round trip, death save rules, a full `simulate` run (seeded) finishes with a winner, Monte
Carlo of 1000 runs works. `cargo build --release` and `cargo clippy --all-targets -- -D warnings`
must be clean.

## 8. Docs
README.md (Chinese) with usage examples & cheat sheet; docs/SPEC.md (this) ; docs/DATA.md.
