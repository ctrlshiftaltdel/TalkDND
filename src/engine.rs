//! Combat rules resolution for SPEC §4.
//!
//! Every public operation mutates an [`EncounterState`] and writes concise
//! human-readable lines to a [`LogSink`]. Reports carry the same information in
//! structured form so the CLI and the AI can inspect results without parsing
//! log strings.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};

use crate::dice::{D20Roll, DiceExpr, DiceRng, RollMode, RollRecord};
use crate::model::{
    Attack, AttackKind, CombatantSpec, ConditionApplication, Cost, DurationSpec, Heal, HealAction,
    Kind, Mastery, Reaction, ResetType, Side,
};
use crate::state::{ActiveCondition, CombatantState, EffectMarker, EncounterState, MarkerKind};

/// Destination for engine log lines.
///
/// Callers that do not need a log use [`NullSink`]; the default [`LogSink::log`]
/// implementation checks [`LogSink::enabled`] before formatting, so disabled
/// sinks avoid the formatting cost.
pub trait LogSink {
    fn enabled(&self) -> bool;
    fn push(&mut self, line: &str);
    fn log(&mut self, args: std::fmt::Arguments<'_>) {
        if self.enabled() {
            self.push(&format!("{args}"));
        }
    }
}

impl LogSink for Vec<String> {
    fn enabled(&self) -> bool {
        true
    }

    fn push(&mut self, line: &str) {
        Vec::push(self, line.to_string());
    }
}

/// A sink that discards everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

impl LogSink for NullSink {
    fn enabled(&self) -> bool {
        false
    }

    fn push(&mut self, _line: &str) {}
}

/// Options shared by `attack` and `cast`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttackOptions {
    pub advantage: bool,
    pub disadvantage: bool,
    pub no_sneak: bool,
    pub no_reactions: bool,
    pub multi: bool,
}

/// Options for manual condition application.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConditionOptions {
    /// Expire at the end of the affected creature's Nth turn.
    pub rounds: Option<u32>,
    /// Expire at the end of this creature's next turn.
    pub until_end_of_turn: Option<String>,
    pub source: Option<String>,
}

/// Options for skill checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckOptions {
    pub mode: RollMode,
    pub bonus: i32,
}

/// Which side won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatOutcome {
    PartyWins,
    EnemyWins,
    Draw,
}

/// Short or long rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestKind {
    Short,
    Long,
}

/// One damage type's contribution to an attack or damage command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageComponentReport {
    pub expression: String,
    pub damage_type: Option<String>,
    /// Damage rolled before half-on-success and defenses.
    pub raw: i32,
    /// Damage after defenses (before temporary HP).
    pub dealt: i32,
    pub immune: bool,
    pub resistant: bool,
    pub vulnerable: bool,
}

/// Result of applying damage to one creature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageReport {
    pub target: String,
    /// Damage before defenses.
    pub requested: i32,
    /// Damage after defenses (before temporary HP).
    pub amount: i32,
    pub temp_absorbed: i32,
    pub hp_before: i32,
    pub hp_after: i32,
    pub temp_hp_before: i32,
    pub temp_hp_after: i32,
    pub damage_type: Option<String>,
    pub critical: bool,
    pub downed: bool,
    pub dead: bool,
    pub massive: bool,
    pub death_save_failures: u8,
    pub components: Vec<DamageComponentReport>,
}

/// Result of healing one creature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealReport {
    pub target: String,
    pub rolled: i32,
    /// HP actually restored (capped at maximum).
    pub healed: i32,
    pub hp_before: i32,
    pub hp_after: i32,
    pub revived: bool,
}

/// Result of a saving throw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    pub who: String,
    pub ability: String,
    pub dc: i32,
    pub mode: RollMode,
    pub roll: Option<D20Roll>,
    pub total: i32,
    pub success: bool,
    pub auto_fail: bool,
}

/// Result of a skill or ability check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub who: String,
    pub skill: String,
    pub dc: i32,
    pub mode: RollMode,
    pub bonus: i32,
    pub roll: D20Roll,
    pub total: i32,
    pub success: bool,
}

/// Result of a generic `roll` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollReport {
    pub expression: String,
    pub mode: RollMode,
    pub roll: Option<D20Roll>,
    pub total: i32,
    pub dc: Option<i32>,
    pub success: Option<bool>,
    pub text: String,
}

/// Result of one attack entry, which may contain several instances (multiattack
/// or `count > 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackReport {
    pub attacker: String,
    pub target: String,
    pub attack: String,
    pub attack_kind: AttackKind,
    pub hit: bool,
    pub critical: bool,
    pub natural_1: bool,
    pub natural_20: bool,
    pub attack_roll: Option<D20Roll>,
    pub attack_total: Option<i32>,
    pub target_ac: i32,
    pub save: Option<SaveReport>,
    pub shield: bool,
    pub sneak: bool,
    pub graze: bool,
    pub mastery: Option<Mastery>,
    pub damage: Option<DamageReport>,
    pub total_damage: i32,
    pub hp_before: i32,
    pub hp_after: i32,
    pub downed: bool,
    pub dead: bool,
    pub instances: Vec<AttackReport>,
}

/// Result of casting a named attack or heal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastReport {
    pub caster: String,
    pub name: String,
    pub cost_spent: Option<String>,
    pub attacks: Vec<AttackReport>,
    pub heals: Vec<HealReport>,
}

/// Result of the `use` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseReport {
    pub who: String,
    pub name: String,
    pub cost_spent: Option<String>,
    pub healed: Option<HealReport>,
}

/// Result of movement between zones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveReport {
    pub who: String,
    pub from: String,
    pub to: String,
    pub distance: i32,
    pub speed_left_before: i32,
    pub speed_left_after: i32,
}

/// Result of a death saving throw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeathSaveReport {
    pub who: String,
    pub roll: i32,
    pub total: i32,
    pub successes: u8,
    pub failures: u8,
    pub stable: bool,
    pub dead: bool,
    pub revived: bool,
}

/// Result of advancing to the next turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    pub round: u32,
    pub turn: usize,
    pub active: String,
    pub skipped: Vec<String>,
    pub death_save: Option<DeathSaveReport>,
    pub outcome: Option<CombatOutcome>,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn parse_expr(text: &str) -> Result<DiceExpr> {
    text.parse()
        .map_err(|error| anyhow!("invalid dice expression `{text}`: {error}"))
}

fn zone_position(state: &EncounterState, zone: &str) -> i32 {
    state.zones.get(zone).copied().unwrap_or(0)
}

fn distance(state: &EncounterState, first: usize, second: usize) -> i32 {
    let first_zone = &state.combatants[first].zone;
    let second_zone = &state.combatants[second].zone;
    (zone_position(state, first_zone) - zone_position(state, second_zone)).abs()
}

fn can_act(combatant: &CombatantState) -> bool {
    combatant.hp > 0
        && !combatant.death_saves.dead
        && !combatant.has_condition("unconscious")
        && !combatant.has_condition("paralyzed")
        && !combatant.has_condition("incapacitated")
}

fn is_dead(combatant: &CombatantState) -> bool {
    combatant.death_saves.dead
}

fn ensure_can_act(state: &EncounterState, index: usize) -> Result<()> {
    let combatant = &state.combatants[index];
    if is_dead(combatant) {
        bail!("{} is dead", combatant.name());
    }
    if !can_act(combatant) {
        bail!("{} cannot act", combatant.name());
    }
    Ok(())
}

fn ensure_target(state: &EncounterState, index: usize) -> Result<()> {
    let combatant = &state.combatants[index];
    if is_dead(combatant) {
        bail!("{} is dead", combatant.name());
    }
    Ok(())
}

fn find_reaction<'a>(spec: &'a CombatantSpec, name: &str) -> Option<&'a Reaction> {
    spec.reactions
        .iter()
        .find(|reaction| reaction.name.eq_ignore_ascii_case(name))
}

fn attack_candidates(spec: &CombatantSpec) -> String {
    if spec.attacks.is_empty() {
        "(none)".to_string()
    } else {
        spec.attacks
            .iter()
            .map(|attack| attack.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn heal_candidates(spec: &CombatantSpec) -> String {
    if spec.heals.is_empty() {
        "(none)".to_string()
    } else {
        spec.heals
            .iter()
            .map(|heal| heal.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn resource_candidates(combatant: &CombatantState) -> String {
    if combatant.resources.is_empty() {
        "(none)".to_string()
    } else {
        combatant
            .resources
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn find_attack<'a>(spec: &'a CombatantSpec, name: &str) -> Result<&'a Attack> {
    spec.attacks
        .iter()
        .find(|attack| attack.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            anyhow!(
                "{} has no attack named `{name}` (known attacks: {})",
                spec.name,
                attack_candidates(spec)
            )
        })
}

fn find_heal<'a>(spec: &'a CombatantSpec, name: &str) -> Option<&'a Heal> {
    spec.heals
        .iter()
        .find(|heal| heal.name.eq_ignore_ascii_case(name))
}

fn cost_amount(cost: &Cost) -> Result<i32> {
    i32::try_from(cost.amount).map_err(|_| anyhow!("cost amount is too large"))
}

fn check_cost(state: &EncounterState, index: usize, cost: &Cost) -> Result<()> {
    let amount = cost_amount(cost)?;
    let combatant = &state.combatants[index];
    let resource = combatant.resources.get(&cost.resource).ok_or_else(|| {
        anyhow!(
            "{} has no resource `{}` (known resources: {})",
            combatant.name(),
            cost.resource,
            resource_candidates(combatant)
        )
    })?;
    if resource.current < amount {
        bail!(
            "{} has only {} of `{}` ({} required)",
            combatant.name(),
            resource.current,
            cost.resource,
            amount
        );
    }
    Ok(())
}

fn can_afford(state: &EncounterState, index: usize, cost: &Cost) -> bool {
    let Ok(amount) = i32::try_from(cost.amount) else {
        return false;
    };
    state.combatants[index]
        .resources
        .get(&cost.resource)
        .is_some_and(|resource| resource.current >= amount)
}

fn spend_cost<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    index: usize,
    cost: &Cost,
    sink: &mut L,
) -> Result<()> {
    check_cost(state, index, cost)?;
    let amount = cost_amount(cost)?;
    let combatant = &mut state.combatants[index];
    let resource = combatant
        .resources
        .get_mut(&cost.resource)
        .expect("cost was checked immediately before spending");
    let before = resource.current;
    resource.current -= amount;
    let after = resource.current;
    let name = combatant.name().to_string();
    sink.log(format_args!(
        "{name} spends {amount} {} ({before} -> {after})",
        cost.resource
    ));
    Ok(())
}

/// Round in which a named creature's next turn ends.
fn next_turn_expiry(state: &EncounterState, creature: usize) -> u32 {
    let round = state.round.max(1);
    let Some(current) = state.current_index() else {
        return round;
    };
    if creature == current {
        return round + 1;
    }
    let position = state.initiative.iter().position(|&index| index == creature);
    let current_position = state.initiative.iter().position(|&index| index == current);
    match (position, current_position) {
        (Some(position), Some(current_position)) if position > current_position => round,
        _ => round + 1,
    }
}

fn hostile_within_5ft(state: &EncounterState, attacker: usize) -> bool {
    let side = state.combatants[attacker].side();
    state
        .combatants
        .iter()
        .enumerate()
        .any(|(index, combatant)| {
            index != attacker
                && combatant.side() != side
                && combatant.hp > 0
                && !combatant.death_saves.dead
                && !combatant.has_condition("incapacitated")
                && !combatant.has_condition("paralyzed")
                && !combatant.has_condition("unconscious")
                && distance(state, attacker, index) <= 5
        })
}

fn sneak_ally_near(state: &EncounterState, attacker: usize, target: usize) -> bool {
    let side = state.combatants[attacker].side();
    state
        .combatants
        .iter()
        .enumerate()
        .any(|(index, combatant)| {
            index != attacker
                && combatant.side() == side
                && combatant.is_conscious()
                && !combatant.has_condition("incapacitated")
                && distance(state, index, target) <= 5
        })
}

fn condition_immunity(state: &EncounterState, index: usize, condition: &str) -> bool {
    state.combatants[index].is_immune_to_condition(condition)
}

// ---------------------------------------------------------------------------
// Initiative and turn flow
// ---------------------------------------------------------------------------

/// Rolls initiative for every combatant, sorts it, and starts round 1.
pub fn init<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    rng: &mut R,
    sink: &mut L,
) -> Result<()> {
    init_with_overrides(state, &[], rng, sink)
}

/// Rolls initiative, honouring fixed totals supplied by `init --set`.
///
/// Overridden combatants do not consume RNG. The same tie-breaks apply:
/// total, initiative bonus, Dexterity modifier, then creation order.
pub fn init_with_overrides<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    overrides: &[(usize, i32)],
    rng: &mut R,
    sink: &mut L,
) -> Result<()> {
    if state.combatants.is_empty() {
        bail!("there are no combatants to roll initiative for");
    }

    let mut fixed = BTreeMap::new();
    for &(index, total) in overrides {
        if index >= state.combatants.len() {
            bail!("initiative override refers to unknown combatant index {index}");
        }
        fixed.insert(index, total);
    }

    let mut rolls: Vec<(usize, i32, i32, i32)> = Vec::with_capacity(state.combatants.len());
    for (index, combatant) in state.combatants.iter().enumerate() {
        let bonus = combatant.spec.init_bonus;
        let total = if let Some(&total) = fixed.get(&index) {
            if sink.enabled() {
                sink.log(format_args!(
                    "Initiative {0}: {total} (set)",
                    combatant.name()
                ));
            }
            total
        } else {
            let roll = D20Roll::roll(bonus, RollMode::Normal, rng);
            if sink.enabled() {
                sink.log(format_args!(
                    "Initiative {0}: {1}",
                    combatant.name(),
                    roll.text()
                ));
            }
            roll.total
        };
        rolls.push((index, total, bonus, combatant.ability_modifier("dex")));
    }

    rolls.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| right.3.cmp(&left.3))
            .then_with(|| left.0.cmp(&right.0))
    });

    for &(index, total, ..) in &rolls {
        state.combatants[index].initiative_total = Some(total);
    }
    state.initiative = rolls.into_iter().map(|(index, ..)| index).collect();
    state.round = 1;
    state.turn = 0;

    let active = state.initiative[0];
    start_turn(state, active);
    if sink.enabled() {
        sink.log(format_args!(
            "Round 1 — {}'s turn",
            state.combatants[active].name()
        ));
    }
    Ok(())
}

fn start_turn(state: &mut EncounterState, index: usize) {
    let combatant = &mut state.combatants[index];
    combatant.reaction_used = false;
    combatant.sneak_used_this_turn = false;
    combatant.action_used = false;
    combatant.bonus_action_used = false;
    combatant.speed_left = combatant.effective_speed();
}

fn end_turn<L: LogSink + ?Sized>(state: &mut EncounterState, current: usize, sink: &mut L) {
    enum Expiry {
        Rounds,
        Turn(String),
    }

    let round = state.round;
    let current_name = state.combatants[current].name().to_string();

    for holder in 0..state.combatants.len() {
        let holder_name = state.combatants[holder].name().to_string();
        let mut expired_conditions: Vec<(String, Expiry)> = Vec::new();
        let mut expired_markers: Vec<(MarkerKind, String)> = Vec::new();
        {
            let combatant = &mut state.combatants[holder];
            combatant.conditions.retain_mut(|condition| {
                let mut keep = true;
                if holder == current {
                    if let Some(rounds) = condition
                        .duration
                        .as_mut()
                        .and_then(|duration| duration.rounds.as_mut())
                    {
                        if *rounds <= 1 {
                            keep = false;
                            expired_conditions.push((condition.condition.clone(), Expiry::Rounds));
                        } else {
                            *rounds -= 1;
                        }
                    }
                }
                if keep {
                    if let Some(ends_with) = &condition.ends_with_turn_of {
                        let reached = match condition.expires_round {
                            Some(expires) => round >= expires,
                            None => true,
                        };
                        if ends_with.eq_ignore_ascii_case(&current_name) && reached {
                            keep = false;
                            expired_conditions.push((
                                condition.condition.clone(),
                                Expiry::Turn(current_name.clone()),
                            ));
                        }
                    }
                }
                keep
            });
            combatant.markers.retain(|marker| {
                if marker.ends_with_turn_of.eq_ignore_ascii_case(&current_name)
                    && round >= marker.expires_round
                {
                    expired_markers.push((marker.kind, marker.source.clone()));
                    false
                } else {
                    true
                }
            });
        }
        for (condition, expiry) in expired_conditions {
            match expiry {
                Expiry::Rounds => sink.log(format_args!(
                    "{holder_name}: {condition} expires (rounds elapsed)"
                )),
                Expiry::Turn(name) => sink.log(format_args!(
                    "{holder_name}: {condition} expires (end of {name}'s turn)"
                )),
            }
        }
        for (kind, source) in expired_markers {
            sink.log(format_args!(
                "{holder_name}: {kind:?} marker from {source} expires"
            ));
        }
    }
}

/// Ends the current turn, advances to the next living combatant, and starts it.
pub fn next_turn<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    rng: &mut R,
    sink: &mut L,
) -> Result<TurnReport> {
    if state.initiative.is_empty() {
        bail!("initiative has not been rolled");
    }
    let Some(current) = state.current_index() else {
        bail!("current turn index is out of range");
    };

    end_turn(state, current, sink);

    let length = state.initiative.len();
    let mut skipped = Vec::new();
    let mut death_save = None;

    for _ in 0..=length {
        state.turn += 1;
        if state.turn >= length {
            state.turn = 0;
            state.round = state.round.saturating_add(1);
        }
        let index = state.initiative[state.turn];
        let combatant = &state.combatants[index];
        if combatant.death_saves.dead || (combatant.kind() != Kind::Pc && combatant.hp <= 0) {
            skipped.push(combatant.name().to_string());
            continue;
        }

        start_turn(state, index);

        if state.combatants[index].kind() == Kind::Pc
            && state.combatants[index].hp <= 0
            && !state.combatants[index].death_saves.stable
            && !state.combatants[index].death_saves.dead
        {
            let report = death_save_at(state, index, rng, sink)?;
            let died = report.dead;
            death_save = Some(report);
            if died {
                skipped.push(state.combatants[index].name().to_string());
                continue;
            }
        }

        let active = state.combatants[index].name().to_string();
        sink.log(format_args!("Round {} — {active}'s turn", state.round));
        let outcome = combat_outcome(state);
        if let Some(outcome) = outcome {
            log_outcome(sink, outcome);
        }
        return Ok(TurnReport {
            round: state.round,
            turn: state.turn,
            active,
            skipped,
            death_save,
            outcome,
        });
    }

    bail!("no living combatants remain")
}

fn log_outcome<L: LogSink + ?Sized>(sink: &mut L, outcome: CombatOutcome) {
    let text = match outcome {
        CombatOutcome::PartyWins => "Combat over: the party wins",
        CombatOutcome::EnemyWins => "Combat over: the enemies win",
        CombatOutcome::Draw => "Combat over: draw (no conscious combatants)",
    };
    sink.log(format_args!("{text}"));
}

/// Which side, if any, has been reduced to zero conscious members.
pub fn combat_outcome(state: &EncounterState) -> Option<CombatOutcome> {
    let mut party_alive = false;
    let mut enemy_alive = false;
    for combatant in &state.combatants {
        if combatant.is_conscious() {
            if combatant.side() == Side::Party {
                party_alive = true;
            } else {
                enemy_alive = true;
            }
        }
    }
    match (party_alive, enemy_alive) {
        (true, false) => Some(CombatOutcome::PartyWins),
        (false, true) => Some(CombatOutcome::EnemyWins),
        (false, false) => Some(CombatOutcome::Draw),
        (true, true) => None,
    }
}

// ---------------------------------------------------------------------------
// Attacks
// ---------------------------------------------------------------------------

struct ResolveRequest<'a> {
    attacker: usize,
    target: usize,
    attack: &'a Attack,
    options: &'a AttackOptions,
    spend_cost: bool,
    count_override: Option<u32>,
}

struct InstanceRequest<'a> {
    attacker: usize,
    target: usize,
    attack: &'a Attack,
    options: &'a AttackOptions,
    ranged: bool,
    long_range: bool,
    distance: i32,
}

/// Resolves a named attack, or the attacker's multiattack sequence when
/// `options.multi` is set.
pub fn attack<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    attacker: &str,
    target: &str,
    attack_name: &str,
    options: &AttackOptions,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    let attacker_index = state.find_index(attacker)?;
    let target_index = state.find_index(target)?;
    if options.multi {
        return multiattack(state, attacker, target, options, rng, sink);
    }

    let attack = find_attack(&state.combatants[attacker_index].spec, attack_name)?.clone();
    let request = ResolveRequest {
        attacker: attacker_index,
        target: target_index,
        attack: &attack,
        options,
        spend_cost: true,
        count_override: None,
    };
    resolve_named_attack(state, &request, rng, sink)
}

/// Resolves the attacker's full `multiattack` sequence against one target.
pub fn multiattack<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    attacker: &str,
    target: &str,
    options: &AttackOptions,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    let attacker_index = state.find_index(attacker)?;
    let target_index = state.find_index(target)?;
    multiattack_at(state, attacker_index, target_index, options, rng, sink)
}

fn multiattack_at<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    attacker_index: usize,
    target_index: usize,
    options: &AttackOptions,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    ensure_can_act(state, attacker_index)?;
    ensure_target(state, target_index)?;
    let sequence = state.combatants[attacker_index].spec.multiattack.clone();
    if sequence.is_empty() {
        bail!(
            "{} has no multiattack sequence",
            state.combatants[attacker_index].name()
        );
    }

    let attacker_name = state.combatants[attacker_index].name().to_string();
    let target_name = state.combatants[target_index].name().to_string();
    let target_ac = state.combatants[target_index].ac();
    let hp_before = state.combatants[target_index].hp;
    let mut report = new_report(
        &attacker_name,
        &target_name,
        "multiattack",
        AttackKind::Unknown,
        target_ac,
        hp_before,
    );

    for name in sequence {
        if is_dead(&state.combatants[target_index]) {
            sink.log(format_args!("Multiattack stops: {target_name} is down"));
            break;
        }
        let attack = find_attack(&state.combatants[attacker_index].spec, &name)?.clone();
        let mut inner_options = *options;
        inner_options.multi = false;
        let request = ResolveRequest {
            attacker: attacker_index,
            target: target_index,
            attack: &attack,
            options: &inner_options,
            spend_cost: true,
            count_override: None,
        };
        let result = resolve_named_attack(state, &request, rng, sink)?;
        if result.instances.is_empty() {
            report.instances.push(result);
        } else {
            report.instances.extend(result.instances);
        }
    }

    summarize(&mut report, state, target_index);
    Ok(report)
}

fn new_report(
    attacker: &str,
    target: &str,
    attack: &str,
    kind: AttackKind,
    target_ac: i32,
    hp_before: i32,
) -> AttackReport {
    AttackReport {
        attacker: attacker.to_string(),
        target: target.to_string(),
        attack: attack.to_string(),
        attack_kind: kind,
        hit: false,
        critical: false,
        natural_1: false,
        natural_20: false,
        attack_roll: None,
        attack_total: None,
        target_ac,
        save: None,
        shield: false,
        sneak: false,
        graze: false,
        mastery: None,
        damage: None,
        total_damage: 0,
        hp_before,
        hp_after: hp_before,
        downed: false,
        dead: false,
        instances: Vec::new(),
    }
}

fn summarize(report: &mut AttackReport, state: &EncounterState, target: usize) {
    if report.instances.is_empty() {
        return;
    }
    report.hit = report.instances.iter().any(|instance| instance.hit);
    report.critical = report.instances.iter().any(|instance| instance.critical);
    report.natural_1 = report.instances.iter().all(|instance| instance.natural_1);
    report.natural_20 = report.instances.iter().any(|instance| instance.natural_20);
    report.sneak = report.instances.iter().any(|instance| instance.sneak);
    report.shield = report.instances.iter().any(|instance| instance.shield);
    report.total_damage = report
        .instances
        .iter()
        .map(|instance| instance.total_damage)
        .sum();
    if let Some(first) = report.instances.first() {
        report.hp_before = first.hp_before;
    }
    let combatant = &state.combatants[target];
    report.hp_after = combatant.hp;
    report.downed = combatant.hp <= 0 && !combatant.death_saves.dead;
    report.dead = combatant.death_saves.dead;
}

fn check_range(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    attack: &Attack,
) -> Result<(bool, bool)> {
    let distance = distance(state, attacker, target);
    let attacker_name = state.combatants[attacker].name();
    let target_name = state.combatants[target].name();
    match attack.kind {
        AttackKind::Melee => {
            let reach = attack.reach.unwrap_or(5);
            if distance <= reach {
                Ok((false, false))
            } else if let Some([normal, long]) = attack.range {
                if distance > long {
                    bail!(
                        "{attacker_name} cannot reach {target_name} with {} ({distance} ft, long range {long} ft)",
                        attack.name
                    );
                }
                Ok((true, distance > normal))
            } else {
                bail!(
                    "{attacker_name} cannot reach {target_name} with {} ({distance} ft, reach {reach} ft)",
                    attack.name
                );
            }
        }
        AttackKind::Ranged => {
            let [normal, long] = attack
                .range
                .ok_or_else(|| anyhow!("{} is a ranged attack but has no range", attack.name))?;
            if distance > long {
                bail!(
                    "{attacker_name} cannot reach {target_name} with {} ({distance} ft, long range {long} ft)",
                    attack.name
                );
            }
            Ok((true, distance > normal))
        }
        AttackKind::SpellAttack => {
            if let Some([normal, long]) = attack.range {
                if distance > long {
                    bail!(
                        "{attacker_name} cannot reach {target_name} with {} ({distance} ft, long range {long} ft)",
                        attack.name
                    );
                }
                Ok((true, distance > normal))
            } else {
                let reach = attack.reach.unwrap_or(5);
                if distance > reach {
                    bail!(
                        "{attacker_name} cannot reach {target_name} with {} ({distance} ft, reach {reach} ft)",
                        attack.name
                    );
                }
                Ok((false, false))
            }
        }
        AttackKind::Save | AttackKind::Auto | AttackKind::Unknown => {
            if let Some([_, long]) = attack.range {
                if distance > long {
                    bail!(
                        "{attacker_name} cannot reach {target_name} with {} ({distance} ft, long range {long} ft)",
                        attack.name
                    );
                }
            }
            Ok((false, false))
        }
    }
}

fn resolve_named_attack<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    request: &ResolveRequest<'_>,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    ensure_can_act(state, request.attacker)?;
    ensure_target(state, request.target)?;
    if request.attacker == request.target {
        bail!("a creature cannot attack itself");
    }

    let (ranged, long_range) =
        check_range(state, request.attacker, request.target, request.attack)?;
    if request.attack.kind == AttackKind::Save && request.attack.save.is_none() {
        bail!("{} is a save attack but has no save", request.attack.name);
    }
    if request.spend_cost {
        if let Some(cost) = &request.attack.cost {
            spend_cost(state, request.attacker, cost, sink)?;
        }
    }
    state.combatants[request.attacker].action_used = true;

    let count = request
        .count_override
        .unwrap_or(request.attack.count)
        .max(1);
    let distance = distance(state, request.attacker, request.target);
    let instance = InstanceRequest {
        attacker: request.attacker,
        target: request.target,
        attack: request.attack,
        options: request.options,
        ranged,
        long_range,
        distance,
    };

    let attacker_name = state.combatants[request.attacker].name().to_string();
    let target_name = state.combatants[request.target].name().to_string();
    let target_ac = state.combatants[request.target].ac();
    let hp_before = state.combatants[request.target].hp;
    let mut base = new_report(
        &attacker_name,
        &target_name,
        &request.attack.name,
        request.attack.kind,
        target_ac,
        hp_before,
    );

    match request.attack.kind {
        AttackKind::Auto => {
            let result = auto_instance(state, &instance, count, rng, sink)?;
            return Ok(result);
        }
        AttackKind::Save => {
            for _ in 0..count {
                if is_dead(&state.combatants[request.target]) {
                    break;
                }
                let result = save_instance(state, &instance, rng, sink)?;
                base.instances.push(result);
            }
        }
        _ => {
            for _ in 0..count {
                if is_dead(&state.combatants[request.target]) {
                    sink.log(format_args!(
                        "{} is already down; remaining attacks stop",
                        target_name
                    ));
                    break;
                }
                let result = attack_instance(state, &instance, rng, sink)?;
                base.instances.push(result);
            }
        }
    }

    if base.instances.len() == 1 {
        return Ok(base.instances.remove(0));
    }
    summarize(&mut base, state, request.target);
    Ok(base)
}

fn attack_instance<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    request: &InstanceRequest<'_>,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    ensure_can_act(state, request.attacker)?;
    ensure_target(state, request.target)?;

    let target_name = state.combatants[request.target].name().to_string();
    let attacker_name = state.combatants[request.attacker].name().to_string();
    let target_ac = state.combatants[request.target].ac();
    let hp_before = state.combatants[request.target].hp;
    let mut report = new_report(
        &attacker_name,
        &target_name,
        &request.attack.name,
        request.attack.kind,
        target_ac,
        hp_before,
    );

    let mut has_advantage = request.options.advantage;
    let mut has_disadvantage = request.options.disadvantage || request.long_range;
    if request.ranged && hostile_within_5ft(state, request.attacker) {
        has_disadvantage = true;
    }

    let mut vex_applies = false;
    {
        let attacker = &state.combatants[request.attacker];
        let target = &state.combatants[request.target];
        for condition in ["blinded", "frightened", "poisoned", "prone", "restrained"] {
            if attacker.has_condition(condition) {
                has_disadvantage = true;
            }
        }
        if attacker.has_marker(MarkerKind::Sap) {
            has_disadvantage = true;
        }
        for condition in ["blinded", "paralyzed", "restrained", "unconscious"] {
            if target.has_condition(condition) {
                has_advantage = true;
            }
        }
        if target.has_condition("prone") {
            if request.distance <= 5 {
                has_advantage = true;
            } else {
                has_disadvantage = true;
            }
        }
        if let Some(vs_condition) = &request.attack.vs_condition {
            if vs_condition.advantage && target.has_condition(&vs_condition.condition) {
                has_advantage = true;
            }
        }
        if attacker.markers.iter().any(|marker| {
            marker.kind == MarkerKind::Vex
                && marker
                    .target
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&target_name))
        }) {
            has_advantage = true;
            vex_applies = true;
        }
    }

    let mode = RollMode::from_flags(has_advantage, has_disadvantage);
    let to_hit = request.attack.to_hit.unwrap_or(0);
    let roll = D20Roll::roll(to_hit, mode, rng);
    report.natural_1 = roll.nat1;
    report.natural_20 = roll.nat20;
    let mut hit;
    let mut critical;
    if roll.nat1 {
        hit = false;
        critical = false;
    } else if roll.nat20 {
        hit = true;
        critical = true;
    } else {
        hit = roll.total >= target_ac;
        critical = false;
    }
    if hit
        && request.distance <= 5
        && (state.combatants[request.target].has_condition("paralyzed")
            || state.combatants[request.target].has_condition("unconscious"))
    {
        critical = true;
    }

    {
        let attacker = &mut state.combatants[request.attacker];
        attacker
            .markers
            .retain(|marker| marker.kind != MarkerKind::Sap);
        if vex_applies {
            attacker.markers.retain(|marker| {
                !(marker.kind == MarkerKind::Vex
                    && marker
                        .target
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&target_name)))
            });
        }
    }

    if hit && !roll.nat20 && !request.options.no_reactions {
        let shield = find_reaction(&state.combatants[request.target].spec, "shield").cloned();
        if let Some(shield) = shield {
            let reaction_used = state.combatants[request.target].reaction_used;
            let affordable = shield
                .cost
                .as_ref()
                .is_none_or(|cost| can_afford(state, request.target, cost));
            let would_miss = roll.total < target_ac + shield.ac_bonus;
            let target_can_react = can_act(&state.combatants[request.target]);
            if !reaction_used && target_can_react && affordable && would_miss {
                if let Some(cost) = &shield.cost {
                    spend_cost(state, request.target, cost, sink)?;
                }
                state.combatants[request.target].reaction_used = true;
                hit = false;
                critical = false;
                report.shield = true;
                sink.log(format_args!(
                    "Shield! {target_name} raises AC by {}; the attack misses",
                    shield.ac_bonus
                ));
            }
        }
    }

    report.attack_roll = Some(roll.clone());
    report.attack_total = Some(roll.total);
    if sink.enabled() {
        sink.log(format_args!(
            "{attacker_name} attacks {target_name} with {}: {}",
            request.attack.name,
            roll.vs_ac(target_ac, hit)
        ));
    }

    if !hit {
        report.hit = false;
        if request.attack.mastery == Some(Mastery::Graze) {
            report.graze = true;
            let to_hit = request.attack.to_hit.unwrap_or(0);
            let graze_damage =
                (to_hit - state.combatants[request.attacker].spec.proficiency_bonus).max(0);
            if graze_damage > 0 {
                if sink.enabled() {
                    sink.log(format_args!("Graze: {graze_damage} damage"));
                }
                let component = ComponentRoll {
                    expression: graze_damage.to_string(),
                    damage_type: request.attack.damage_type.clone(),
                    raw: graze_damage,
                };
                let damage =
                    apply_components(state, request.target, &[component], false, false, sink)?;
                copy_damage(&mut report, damage);
            }
        }
        return Ok(report);
    }

    let mut sneak = None;
    if request.attack.sneak_eligible
        && !request.options.no_sneak
        && mode != RollMode::Disadvantage
        && !state.combatants[request.attacker].sneak_used_this_turn
    {
        if let Some(text) = state.combatants[request.attacker]
            .spec
            .features
            .get("sneak_attack")
        {
            if let Ok(expression) = text.parse::<DiceExpr>() {
                if mode == RollMode::Advantage
                    || sneak_ally_near(state, request.attacker, request.target)
                {
                    sneak = Some(expression);
                }
            }
        }
    }
    if sneak.is_some() {
        state.combatants[request.attacker].sneak_used_this_turn = true;
    }
    report.sneak = sneak.is_some();

    let components = attack_components(
        state,
        request.target,
        request.attack,
        critical,
        sneak.as_ref(),
        rng,
        sink,
    )?;
    let damage = apply_components(state, request.target, &components, critical, false, sink)?;
    copy_damage(&mut report, damage);

    if !is_dead(&state.combatants[request.target]) {
        for application in &request.attack.on_hit_conditions {
            apply_application(
                state,
                request.target,
                application,
                Some(&attacker_name),
                sink,
            )?;
        }
        if let Some(mastery) = request.attack.mastery {
            report.mastery = Some(mastery);
            apply_mastery(
                state,
                request.attacker,
                request.target,
                request.attack,
                rng,
                sink,
            )?;
        }
    }

    report.hit = true;
    report.critical = critical;
    report.natural_1 = roll.nat1;
    report.natural_20 = roll.nat20;
    Ok(report)
}

fn copy_damage(report: &mut AttackReport, damage: DamageReport) {
    report.total_damage = damage.amount;
    report.hp_before = damage.hp_before;
    report.hp_after = damage.hp_after;
    report.downed = damage.downed;
    report.dead = damage.dead;
    report.damage = Some(damage);
}

fn save_instance<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    request: &InstanceRequest<'_>,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    let save_spec = request
        .attack
        .save
        .as_ref()
        .ok_or_else(|| anyhow!("{} is a save attack but has no save", request.attack.name))?;
    let attacker_name = state.combatants[request.attacker].name().to_string();
    let target_name = state.combatants[request.target].name().to_string();
    let target_ac = state.combatants[request.target].ac();
    let hp_before = state.combatants[request.target].hp;
    let mut report = new_report(
        &attacker_name,
        &target_name,
        &request.attack.name,
        request.attack.kind,
        target_ac,
        hp_before,
    );

    let save_report = save_at(
        state,
        request.target,
        &save_spec.ability,
        save_spec.dc,
        RollMode::Normal,
        rng,
        sink,
    )?;
    let success = save_report.success;
    report.save = Some(save_report);
    report.hit = !success;

    if !success || save_spec.half_on_success {
        let components = attack_components(
            state,
            request.target,
            request.attack,
            false,
            None,
            rng,
            sink,
        )?;
        let damage = apply_components(state, request.target, &components, false, success, sink)?;
        copy_damage(&mut report, damage);
    }

    if !success && !is_dead(&state.combatants[request.target]) {
        for application in &request.attack.on_fail_conditions {
            apply_application(
                state,
                request.target,
                application,
                Some(&attacker_name),
                sink,
            )?;
        }
    }

    Ok(report)
}

fn auto_instance<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    request: &InstanceRequest<'_>,
    count: u32,
    rng: &mut R,
    sink: &mut L,
) -> Result<AttackReport> {
    let attacker_name = state.combatants[request.attacker].name().to_string();
    let target_name = state.combatants[request.target].name().to_string();
    let target_ac = state.combatants[request.target].ac();
    let hp_before = state.combatants[request.target].hp;
    let mut report = new_report(
        &attacker_name,
        &target_name,
        &request.attack.name,
        request.attack.kind,
        target_ac,
        hp_before,
    );
    report.hit = true;

    let mut components = Vec::new();
    for dart in 1..=count {
        if count > 1 && sink.enabled() {
            sink.log(format_args!(
                "{} (instance {dart}/{count})",
                request.attack.name
            ));
        }
        let rolled = attack_components(
            state,
            request.target,
            request.attack,
            false,
            None,
            rng,
            sink,
        )?;
        components.extend(rolled);
    }

    if !components.is_empty() {
        let damage = apply_components(state, request.target, &components, false, false, sink)?;
        copy_damage(&mut report, damage);
    }

    if !is_dead(&state.combatants[request.target]) {
        for application in &request.attack.on_hit_conditions {
            apply_application(
                state,
                request.target,
                application,
                Some(&attacker_name),
                sink,
            )?;
        }
    }

    Ok(report)
}

fn apply_mastery<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    attack: &Attack,
    rng: &mut R,
    sink: &mut L,
) -> Result<()> {
    match attack.mastery {
        Some(Mastery::Sap) => {
            add_marker(state, target, MarkerKind::Sap, attacker, None, sink);
            sink.log(format_args!(
                "Sap: {} has disadvantage on its next attack",
                state.combatants[target].name()
            ));
        }
        Some(Mastery::Vex) => {
            add_marker(
                state,
                attacker,
                MarkerKind::Vex,
                attacker,
                Some(target),
                sink,
            );
            sink.log(format_args!(
                "Vex: {} has advantage on its next attack against {}",
                state.combatants[attacker].name(),
                state.combatants[target].name()
            ));
        }
        Some(Mastery::Topple) => {
            let dc = 8 + attack.to_hit.unwrap_or(0);
            let save_report = save_at(state, target, "con", dc, RollMode::Normal, rng, sink)?;
            if !save_report.success {
                add_condition_at(
                    state,
                    target,
                    "prone",
                    &ConditionOptions {
                        source: Some(state.combatants[attacker].name().to_string()),
                        ..ConditionOptions::default()
                    },
                    None,
                    None,
                    sink,
                )?;
            }
        }
        Some(Mastery::Slow) => {
            let already_slowed = state.combatants[target].has_marker(MarkerKind::Slow);
            add_marker(state, target, MarkerKind::Slow, attacker, None, sink);
            let combatant = &mut state.combatants[target];
            if !already_slowed {
                combatant.speed_left = (combatant.speed_left - 10).max(0);
            }
            sink.log(format_args!(
                "Slow: {}'s speed is reduced by 10 ft",
                combatant.name()
            ));
        }
        Some(Mastery::Graze) => {}
        Some(Mastery::Nick) => {
            sink.log(format_args!("Nick: remember the extra light-weapon attack"))
        }
        Some(Mastery::Push) => sink.log(format_args!("Push: remember to push the target 10 ft")),
        Some(Mastery::Cleave) => sink.log(format_args!("Cleave: remember the follow-up attack")),
        Some(Mastery::Unknown) | None => {}
    }
    Ok(())
}

fn add_marker<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    holder: usize,
    kind: MarkerKind,
    source: usize,
    target: Option<usize>,
    sink: &mut L,
) {
    let expires_round = next_turn_expiry(state, source);
    let source_name = state.combatants[source].name().to_string();
    let ends_with_turn_of = source_name.clone();
    let target_name = target.map(|index| state.combatants[index].name().to_string());
    {
        let combatant = &mut state.combatants[holder];
        combatant.markers.retain(|marker| {
            !(marker.kind == kind
                && marker.target.as_deref().map(str::to_ascii_lowercase)
                    == target_name.as_deref().map(str::to_ascii_lowercase))
        });
        combatant.markers.push(EffectMarker {
            kind,
            source: source_name,
            target: target_name,
            expires_round,
            ends_with_turn_of,
            notes: None,
        });
    }
    let holder_name = state.combatants[holder].name().to_string();
    sink.log(format_args!(
        "{kind:?} marker on {holder_name} expires at end of round {expires_round}"
    ));
}

struct ComponentRoll {
    expression: String,
    damage_type: Option<String>,
    raw: i32,
}

fn roll_component<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    expr_text: &str,
    damage_type: Option<&str>,
    critical: bool,
    label: Option<&str>,
    rng: &mut R,
    sink: &mut L,
) -> Result<ComponentRoll> {
    let expression = parse_expr(expr_text)?;
    let record = if critical {
        expression.roll_doubled(rng)
    } else {
        expression.roll(rng)
    };
    if sink.enabled() {
        let suffix = damage_type.unwrap_or("");
        if let Some(label) = label {
            sink.log(format_args!("{label}: {} {suffix}", record.text));
        } else if suffix.is_empty() {
            sink.log(format_args!("{}", record.text));
        } else {
            sink.log(format_args!("{} {suffix}", record.text));
        }
    }
    Ok(ComponentRoll {
        expression: record.formula.clone(),
        damage_type: damage_type.map(str::to_string),
        raw: record.total,
    })
}

fn attack_components<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &EncounterState,
    target: usize,
    attack: &Attack,
    critical: bool,
    sneak: Option<&DiceExpr>,
    rng: &mut R,
    sink: &mut L,
) -> Result<Vec<ComponentRoll>> {
    let target_combatant = &state.combatants[target];
    let vs_active = attack
        .vs_condition
        .as_ref()
        .is_some_and(|vs| target_combatant.has_condition(&vs.condition));
    let base_damage = if vs_active {
        attack
            .vs_condition
            .as_ref()
            .and_then(|vs| vs.damage.as_deref())
            .or(attack.damage.as_deref())
    } else {
        attack.damage.as_deref()
    };
    let damage_type = attack.damage_type.as_deref();
    let mut components = Vec::new();

    if let Some(base) = base_damage {
        components.push(roll_component(
            base,
            damage_type,
            critical,
            None,
            rng,
            sink,
        )?);
    }
    for extra in &attack.extra_damage {
        let extra_type = if extra.damage_type.is_empty() {
            None
        } else {
            Some(extra.damage_type.as_str())
        };
        components.push(roll_component(
            &extra.dice,
            extra_type,
            critical,
            None,
            rng,
            sink,
        )?);
    }
    if let Some(sneak) = sneak {
        let text = sneak.to_string();
        components.push(roll_component(
            &text,
            damage_type,
            critical,
            Some("Sneak attack"),
            rng,
            sink,
        )?);
    }
    Ok(components)
}

fn mitigate(
    state: &EncounterState,
    target: usize,
    amount: i32,
    damage_type: Option<&str>,
) -> (i32, bool, bool, bool) {
    let Some(damage_type) = damage_type else {
        return (amount, false, false, false);
    };
    let combatant = &state.combatants[target];
    let immune = combatant
        .spec
        .immunities
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    if immune {
        return (0, true, false, false);
    }
    let resistant = combatant
        .spec
        .resistances
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    let vulnerable = combatant
        .spec
        .vulnerabilities
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    let dealt = if resistant && vulnerable {
        amount
    } else if resistant {
        amount.div_euclid(2)
    } else if vulnerable {
        amount.saturating_mul(2)
    } else {
        amount
    };
    (dealt, false, resistant, vulnerable)
}

struct DamageGroup {
    damage_type: Option<String>,
    key: Option<String>,
    raw: i32,
    expressions: Vec<String>,
}

fn apply_components<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: usize,
    components: &[ComponentRoll],
    critical: bool,
    half: bool,
    sink: &mut L,
) -> Result<DamageReport> {
    let mut requested = 0;
    let mut groups: Vec<DamageGroup> = Vec::new();
    for component in components {
        let rolled = component.raw.max(0);
        requested += rolled;
        let key = component
            .damage_type
            .as_deref()
            .map(str::to_ascii_lowercase);
        match groups.iter_mut().find(|group| group.key == key) {
            Some(group) => {
                group.raw += rolled;
                group.expressions.push(component.expression.clone());
            }
            None => groups.push(DamageGroup {
                damage_type: component.damage_type.clone(),
                key,
                raw: rolled,
                expressions: vec![component.expression.clone()],
            }),
        }
    }

    let mut total = 0;
    let mut reports = Vec::with_capacity(groups.len());
    for group in &groups {
        let after_half = if half {
            group.raw.div_euclid(2)
        } else {
            group.raw
        };
        let (dealt, immune, resistant, vulnerable) =
            mitigate(state, target, after_half, group.damage_type.as_deref());
        total += dealt;
        reports.push(DamageComponentReport {
            expression: group.expressions.join(" + "),
            damage_type: group.damage_type.clone(),
            raw: group.raw,
            dealt,
            immune,
            resistant,
            vulnerable,
        });
    }

    let mut report = apply_damage(state, target, total, None, critical, sink)?;
    report.requested = requested;
    report.components = reports;
    report.damage_type = if groups.len() == 1 {
        groups[0].damage_type.clone()
    } else {
        None
    };
    Ok(report)
}

fn apply_damage<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: usize,
    amount: i32,
    damage_type: Option<&str>,
    critical: bool,
    sink: &mut L,
) -> Result<DamageReport> {
    let combatant = &state.combatants[target];
    if combatant.death_saves.dead {
        bail!("{} is already dead", combatant.name());
    }
    let name = combatant.name().to_string();
    let kind = combatant.kind();
    let max_hp = combatant.spec.max_hp;
    let hp_before = combatant.hp;
    let temp_before = combatant.temp_hp;
    let amount = amount.max(0);

    let temp_absorbed = temp_before.min(amount);
    let after_temp = amount - temp_absorbed;
    let mut downed = false;
    let mut dead = false;
    let mut massive = false;

    {
        let combatant = &mut state.combatants[target];
        combatant.temp_hp -= temp_absorbed;

        if hp_before <= 0 && kind == Kind::Pc {
            combatant.hp = 0;
            if after_temp > 0 {
                combatant.death_saves.stable = false;
                if after_temp >= max_hp {
                    combatant.death_saves.dead = true;
                    dead = true;
                    massive = true;
                } else {
                    let increase = if critical { 2 } else { 1 };
                    combatant.death_saves.failures = combatant
                        .death_saves
                        .failures
                        .saturating_add(increase)
                        .min(3);
                    if combatant.death_saves.failures >= 3 {
                        combatant.death_saves.dead = true;
                        dead = true;
                    }
                }
            }
        } else {
            let raw_hp = hp_before - after_temp;
            if kind == Kind::Pc {
                if raw_hp <= 0 {
                    let overflow = -raw_hp;
                    if overflow >= max_hp {
                        combatant.hp = 0;
                        combatant.death_saves.dead = true;
                        dead = true;
                        massive = true;
                    } else {
                        combatant.hp = 0;
                        combatant.death_saves = Default::default();
                        downed = true;
                    }
                } else {
                    combatant.hp = raw_hp;
                }
            } else if raw_hp <= 0 {
                combatant.hp = 0;
                combatant.death_saves.dead = true;
                dead = true;
            } else {
                combatant.hp = raw_hp;
            }
        }

        if downed {
            ensure_condition(combatant, "unconscious");
            ensure_condition(combatant, "prone");
        }
    }
    let hp_after = state.combatants[target].hp;

    if amount > 0 {
        if temp_absorbed > 0 {
            sink.log(format_args!(
                "{name} takes {amount} damage (temp {temp_absorbed}, {hp_before} -> {hp_after})"
            ));
        } else {
            sink.log(format_args!(
                "{name} takes {amount} damage ({hp_before} -> {hp_after})"
            ));
        }
    }
    if massive {
        sink.log(format_args!("{name} dies from massive damage"));
    } else if dead {
        sink.log(format_args!("{name} dies"));
    } else if downed {
        sink.log(format_args!("{name} falls unconscious"));
    }
    if !dead && kind == Kind::Pc && hp_before <= 0 && after_temp > 0 {
        sink.log(format_args!(
            "{name} fails a death save ({}/3)",
            state.combatants[target].death_saves.failures
        ));
    }

    let combatant = &state.combatants[target];
    Ok(DamageReport {
        target: name,
        requested: amount,
        amount,
        temp_absorbed,
        hp_before,
        hp_after: combatant.hp,
        temp_hp_before: temp_before,
        temp_hp_after: combatant.temp_hp,
        damage_type: damage_type.map(str::to_string),
        critical,
        downed,
        dead,
        massive,
        death_save_failures: combatant.death_saves.failures,
        components: Vec::new(),
    })
}

fn ensure_condition(combatant: &mut CombatantState, condition: &str) {
    if !combatant.has_condition(condition) {
        combatant.conditions.push(ActiveCondition {
            condition: condition.to_string(),
            source: None,
            escape_dc: None,
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });
    }
}

/// Applies damage from an expression or a flat number.
pub fn damage<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: &str,
    expression: &str,
    damage_type: Option<&str>,
    critical: bool,
    rng: &mut R,
    sink: &mut L,
) -> Result<DamageReport> {
    let target_index = state.find_index(target)?;
    ensure_target(state, target_index)?;
    let component = roll_component(expression, damage_type, critical, None, rng, sink)?;
    if component.raw < 0 {
        bail!("damage amount cannot be negative");
    }
    apply_components(state, target_index, &[component], critical, false, sink)
}

/// Heals a creature, waking it from 0 HP if applicable.
pub fn heal<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: &str,
    expression: &str,
    source: Option<&str>,
    rng: &mut R,
    sink: &mut L,
) -> Result<HealReport> {
    let target_index = state.find_index(target)?;
    heal_at(state, target_index, expression, source, rng, sink)
}

fn heal_at<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: usize,
    expression: &str,
    source: Option<&str>,
    rng: &mut R,
    sink: &mut L,
) -> Result<HealReport> {
    if is_dead(&state.combatants[target]) {
        bail!(
            "{} is dead and cannot be healed",
            state.combatants[target].name()
        );
    }
    let expr = parse_expr(expression)?;
    let record = expr.roll(rng);
    if record.total <= 0 {
        bail!("healing must be positive");
    }
    let name = state.combatants[target].name().to_string();
    if sink.enabled() {
        sink.log(format_args!("{} healing", record.text));
    }
    let combatant = &mut state.combatants[target];
    let hp_before = combatant.hp;
    let max_hp = combatant.spec.max_hp;
    let revived = hp_before <= 0;
    let healed = record.total.min((max_hp - hp_before).max(0));
    combatant.hp = hp_before + healed;
    if revived {
        combatant
            .conditions
            .retain(|condition| !condition.condition.eq_ignore_ascii_case("unconscious"));
        combatant.death_saves = Default::default();
    }
    if let Some(source) = source {
        sink.log(format_args!(
            "{source} heals {name} for {healed} HP ({hp_before} -> {})",
            combatant.hp
        ));
    } else {
        sink.log(format_args!(
            "{name} heals {healed} HP ({hp_before} -> {})",
            combatant.hp
        ));
    }
    Ok(HealReport {
        target: name,
        rolled: record.total,
        healed,
        hp_before,
        hp_after: combatant.hp,
        revived,
    })
}

// ---------------------------------------------------------------------------
// Saves, checks and generic rolls
// ---------------------------------------------------------------------------

/// Rolls a saving throw for one combatant.
pub fn save<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    ability: &str,
    dc: i32,
    mode: RollMode,
    rng: &mut R,
    sink: &mut L,
) -> Result<SaveReport> {
    let index = state.find_index(who)?;
    save_at(state, index, ability, dc, mode, rng, sink)
}

fn save_at<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    index: usize,
    ability: &str,
    dc: i32,
    mode: RollMode,
    rng: &mut R,
    sink: &mut L,
) -> Result<SaveReport> {
    let combatant = &state.combatants[index];
    let who = combatant.name().to_string();
    let ability = ability.to_string();
    let auto_fail = matches!(ability.to_ascii_lowercase().as_str(), "str" | "dex")
        && (combatant.hp <= 0
            || combatant.has_condition("paralyzed")
            || combatant.has_condition("unconscious"));

    if auto_fail {
        sink.log(format_args!(
            "{who} auto-fails the {ability} save (paralyzed or unconscious)"
        ));
        return Ok(SaveReport {
            who,
            ability,
            dc,
            mode,
            roll: None,
            total: 0,
            success: false,
            auto_fail: true,
        });
    }

    let bonus = combatant.save_bonus(&ability);
    let roll = D20Roll::roll(bonus, mode, rng);
    let success = roll.total >= dc;
    if sink.enabled() {
        sink.log(format_args!("{who} {ability} save: {}", roll.vs_dc(dc)));
    }
    Ok(SaveReport {
        who,
        ability,
        dc,
        mode,
        total: roll.total,
        success,
        auto_fail: false,
        roll: Some(roll),
    })
}

/// Rolls a skill or ability check.
pub fn check<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    skill_or_ability: &str,
    dc: i32,
    options: &CheckOptions,
    rng: &mut R,
    sink: &mut L,
) -> Result<CheckReport> {
    let index = state.find_index(who)?;
    let combatant = &state.combatants[index];
    let has_skill = combatant
        .spec
        .skills
        .keys()
        .any(|key| key.eq_ignore_ascii_case(skill_or_ability));
    let has_ability = combatant
        .spec
        .abilities
        .keys()
        .any(|key| key.eq_ignore_ascii_case(skill_or_ability));
    if !has_skill && !has_ability {
        bail!(
            "{} has no skill or ability named `{skill_or_ability}`",
            combatant.name()
        );
    }
    let base = combatant.skill_bonus(skill_or_ability);
    let bonus = base + options.bonus;
    let roll = D20Roll::roll(bonus, options.mode, rng);
    let success = roll.total >= dc;
    if sink.enabled() {
        sink.log(format_args!(
            "{} {skill_or_ability} check: {}",
            combatant.name(),
            roll.vs_dc(dc)
        ));
    }
    Ok(CheckReport {
        who: combatant.name().to_string(),
        skill: skill_or_ability.to_string(),
        dc,
        mode: options.mode,
        bonus,
        total: roll.total,
        success,
        roll,
    })
}

/// Rolls an arbitrary expression for the `roll` command.
pub fn roll_expression<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    expression: &str,
    mode: RollMode,
    dc: Option<i32>,
    rng: &mut R,
    sink: &mut L,
) -> Result<RollReport> {
    let expr = parse_expr(expression)?;
    let single_d20 = expr.terms.len() == 1 && expr.terms[0] == (1, 20);
    if mode != RollMode::Normal && !single_d20 {
        bail!("advantage and disadvantage only apply to a single d20 expression");
    }

    if single_d20 {
        let roll = D20Roll::roll(expr.modifier, mode, rng);
        let success = dc.map(|dc| roll.total >= dc);
        let text = match dc {
            Some(dc) => roll.vs_dc(dc),
            None => roll.text(),
        };
        if sink.enabled() {
            sink.log(format_args!("{text}"));
        }
        return Ok(RollReport {
            expression: expr.to_string(),
            mode,
            total: roll.total,
            dc,
            success,
            text,
            roll: Some(roll),
        });
    }

    let record: RollRecord = expr.roll(rng);
    let success = dc.map(|dc| record.total >= dc);
    let text = match (dc, success) {
        (Some(dc), Some(success)) => format!(
            "{} vs DC {dc} {}",
            record.text,
            if success { "PASS" } else { "FAIL" }
        ),
        _ => record.text.clone(),
    };
    if sink.enabled() {
        sink.log(format_args!("{text}"));
    }
    Ok(RollReport {
        expression: expr.to_string(),
        mode,
        total: record.total,
        dc,
        success,
        text,
        roll: None,
    })
}

// ---------------------------------------------------------------------------
// Casting, resources and conditions
// ---------------------------------------------------------------------------

/// Finds the target indices and validates a cast's range checks.
fn validate_cast_targets(
    state: &EncounterState,
    caster: usize,
    targets: &[String],
    attack: &Attack,
) -> Result<Vec<usize>> {
    if targets.is_empty() {
        bail!("{} requires a target", attack.name);
    }
    let mut indices = Vec::with_capacity(targets.len());
    for target in targets {
        let index = state.find_index(target)?;
        ensure_can_act(state, caster)?;
        ensure_target(state, index)?;
        if index == caster {
            bail!("a creature cannot attack itself");
        }
        check_range(state, caster, index, attack)?;
        indices.push(index);
    }
    Ok(indices)
}

/// Casts a named attack or heal entry from a combatant's data.
///
/// Attack entries win over heal entries when both share a name. For attacks
/// with multiple targets, the attack's `count` instances are distributed as
/// evenly as possible; it is an error when there are fewer instances than
/// targets.
pub fn cast<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    caster: &str,
    name: &str,
    targets: &[String],
    options: &AttackOptions,
    rng: &mut R,
    sink: &mut L,
) -> Result<CastReport> {
    let caster_index = state.find_index(caster)?;
    ensure_can_act(state, caster_index)?;
    let caster_name = state.combatants[caster_index].name().to_string();

    if let Some(attack) = find_attack(&state.combatants[caster_index].spec, name)
        .ok()
        .cloned()
    {
        let target_indices = validate_cast_targets(state, caster_index, targets, &attack)?;
        let count = attack.count.max(1);
        let target_count =
            u32::try_from(target_indices.len()).map_err(|_| anyhow!("too many targets"))?;
        if target_count > count {
            bail!(
                "{} has only {count} instance(s) for {target_count} targets",
                attack.name
            );
        }
        if let Some(cost) = &attack.cost {
            check_cost(state, caster_index, cost)?;
        }

        let base_count = count / target_count;
        let extra = count % target_count;
        if let Some(cost) = &attack.cost {
            spend_cost(state, caster_index, cost, sink)?;
        }
        let cost_spent = attack
            .cost
            .as_ref()
            .map(|cost| format!("{} x{}", cost.resource, cost.amount));

        let mut inner_options = *options;
        inner_options.multi = false;
        let mut attacks = Vec::new();
        for (position, target) in target_indices.into_iter().enumerate() {
            let instances = base_count
                + if u32::try_from(position).unwrap_or(0) < extra {
                    1
                } else {
                    0
                };
            let request = ResolveRequest {
                attacker: caster_index,
                target,
                attack: &attack,
                options: &inner_options,
                spend_cost: false,
                count_override: Some(instances),
            };
            attacks.push(resolve_named_attack(state, &request, rng, sink)?);
        }
        state.combatants[caster_index].action_used = true;
        return Ok(CastReport {
            caster: caster_name,
            name: attack.name,
            cost_spent,
            attacks,
            heals: Vec::new(),
        });
    }

    let heal_entry = find_heal(&state.combatants[caster_index].spec, name)
        .cloned()
        .ok_or_else(|| {
            let spec = &state.combatants[caster_index].spec;
            anyhow!(
                "{caster_name} has no attack or heal named `{name}` (known attacks: {}; known heals: {})",
                attack_candidates(spec),
                heal_candidates(spec)
            )
        })?;

    let target_indices: Vec<usize> = if targets.is_empty() {
        vec![caster_index]
    } else {
        let mut indices = Vec::with_capacity(targets.len());
        for target in targets {
            let index = state.find_index(target)?;
            if is_dead(&state.combatants[index]) {
                bail!(
                    "{} is dead and cannot be healed",
                    state.combatants[index].name()
                );
            }
            if heal_entry.self_only && index != caster_index {
                bail!("{} can only target its caster", heal_entry.name);
            }
            if index != caster_index {
                let distance = distance(state, caster_index, index);
                if heal_entry.range <= 0 || distance > heal_entry.range {
                    bail!(
                        "{} is out of range for {} ({distance} ft, range {} ft)",
                        state.combatants[index].name(),
                        heal_entry.name,
                        heal_entry.range
                    );
                }
            }
            indices.push(index);
        }
        indices
    };

    if let Some(cost) = &heal_entry.cost {
        check_cost(state, caster_index, cost)?;
    }
    if let Some(cost) = &heal_entry.cost {
        spend_cost(state, caster_index, cost, sink)?;
    }
    let cost_spent = heal_entry
        .cost
        .as_ref()
        .map(|cost| format!("{} x{}", cost.resource, cost.amount));

    match heal_entry.action {
        HealAction::Action => state.combatants[caster_index].action_used = true,
        HealAction::Bonus => state.combatants[caster_index].bonus_action_used = true,
        HealAction::Unknown => {}
    }

    let mut heals = Vec::new();
    for target in target_indices {
        heals.push(heal_at(
            state,
            target,
            &heal_entry.amount,
            Some(&caster_name),
            rng,
            sink,
        )?);
    }
    Ok(CastReport {
        caster: caster_name,
        name: heal_entry.name,
        cost_spent,
        attacks: Vec::new(),
        heals,
    })
}

/// Spends a resource, or runs a self-heal when `name` matches a heal entry.
pub fn use_entry<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    name: &str,
    amount: Option<u32>,
    rng: &mut R,
    sink: &mut L,
) -> Result<UseReport> {
    let index = state.find_index(who)?;
    ensure_can_act(state, index)?;
    let who_name = state.combatants[index].name().to_string();

    if let Some(heal_entry) = find_heal(&state.combatants[index].spec, name).cloned() {
        if is_dead(&state.combatants[index]) {
            bail!("{who_name} is dead");
        }
        if let Some(cost) = &heal_entry.cost {
            check_cost(state, index, cost)?;
        }
        if let Some(cost) = &heal_entry.cost {
            spend_cost(state, index, cost, sink)?;
        }
        let cost_spent = heal_entry
            .cost
            .as_ref()
            .map(|cost| format!("{} x{}", cost.resource, cost.amount));
        match heal_entry.action {
            HealAction::Action => state.combatants[index].action_used = true,
            HealAction::Bonus => state.combatants[index].bonus_action_used = true,
            HealAction::Unknown => {}
        }
        let healed = heal_at(state, index, &heal_entry.amount, Some(&who_name), rng, sink)?;
        return Ok(UseReport {
            who: who_name,
            name: heal_entry.name,
            cost_spent,
            healed: Some(healed),
        });
    }

    let requested = amount.unwrap_or(1);
    if requested == 0 {
        bail!("amount must be at least 1");
    }
    let requested = i32::try_from(requested).map_err(|_| anyhow!("amount is too large"))?;
    let key = state.combatants[index]
        .resources
        .keys()
        .find(|key| key.eq_ignore_ascii_case(name))
        .cloned()
        .ok_or_else(|| {
            let combatant = &state.combatants[index];
            anyhow!(
                "{who_name} has no resource or heal named `{name}` (known resources: {}; known heals: {})",
                resource_candidates(combatant),
                heal_candidates(&combatant.spec)
            )
        })?;
    {
        let resource = state.combatants[index]
            .resources
            .get_mut(&key)
            .expect("resource key was found immediately before");
        if resource.current < requested {
            bail!(
                "{who_name} has only {} of `{key}` ({requested} required)",
                resource.current
            );
        }
        let before = resource.current;
        resource.current -= requested;
        sink.log(format_args!(
            "{who_name} uses {requested} {key} ({before} -> {})",
            resource.current
        ));
    }
    Ok(UseReport {
        who: who_name,
        name: key.clone(),
        cost_spent: Some(format!("{key} x{requested}")),
        healed: None,
    })
}

/// Adds a condition, replacing any existing condition of the same name.
pub fn add_condition<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    condition: &str,
    options: &ConditionOptions,
    sink: &mut L,
) -> Result<bool> {
    if options.rounds == Some(0) {
        bail!("--rounds must be at least 1");
    }
    if options.rounds.is_some() && options.until_end_of_turn.is_some() {
        bail!("--rounds and --until-end-of-turn are mutually exclusive");
    }
    let index = state.find_index(who)?;
    add_condition_at(state, index, condition, options, None, None, sink)
}

fn add_condition_at<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: usize,
    condition: &str,
    options: &ConditionOptions,
    escape_dc: Option<i32>,
    notes: Option<&str>,
    sink: &mut L,
) -> Result<bool> {
    if condition.trim().is_empty() {
        bail!("condition name cannot be empty");
    }
    if condition_immunity(state, target, condition) {
        sink.log(format_args!(
            "{} is immune to {condition}",
            state.combatants[target].name()
        ));
        return Ok(false);
    }

    let mut duration = None;
    let mut expires_round = None;
    let mut ends_with_turn_of = None;
    if let Some(rounds) = options.rounds {
        if rounds == 0 {
            bail!("rounds must be at least 1");
        }
        duration = Some(DurationSpec {
            until: None,
            rounds: Some(rounds),
            notes: None,
        });
    } else if let Some(name) = &options.until_end_of_turn {
        let source_index = state.find_index(name)?;
        let canonical = state.combatants[source_index].name().to_string();
        let expiry = next_turn_expiry(state, source_index);
        duration = Some(DurationSpec {
            until: Some("end_of_source_next_turn".to_string()),
            rounds: None,
            notes: None,
        });
        expires_round = Some(expiry);
        ends_with_turn_of = Some(canonical);
    }

    let target_name = state.combatants[target].name().to_string();
    let combatant = &mut state.combatants[target];
    combatant
        .conditions
        .retain(|active| !active.condition.eq_ignore_ascii_case(condition));
    combatant.conditions.push(ActiveCondition {
        condition: condition.to_string(),
        source: options.source.clone(),
        escape_dc,
        duration,
        expires_round,
        ends_with_turn_of: ends_with_turn_of.clone(),
        notes: notes.map(str::to_string),
    });

    if let Some(rounds) = options.rounds {
        sink.log(format_args!(
            "{target_name} is now {condition} ({rounds} round{})",
            if rounds == 1 { "" } else { "s" }
        ));
    } else if let Some(ends) = ends_with_turn_of {
        sink.log(format_args!(
            "{target_name} is now {condition} (until end of {ends}'s next turn)"
        ));
    } else {
        sink.log(format_args!("{target_name} is now {condition}"));
    }
    Ok(true)
}

fn apply_application<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: usize,
    application: &ConditionApplication,
    source: Option<&str>,
    sink: &mut L,
) -> Result<bool> {
    let mut options = ConditionOptions {
        source: source.map(str::to_string),
        ..ConditionOptions::default()
    };
    if let Some(duration) = &application.duration {
        if let Some(rounds) = duration.rounds {
            options.rounds = Some(rounds);
        } else if let Some(until) = &duration.until {
            if until.eq_ignore_ascii_case("end_of_source_next_turn") {
                if let Some(source) = source {
                    options.until_end_of_turn = Some(source.to_string());
                }
            } else {
                options.until_end_of_turn = Some(until.clone());
            }
        }
    }
    add_condition_at(
        state,
        target,
        &application.condition,
        &options,
        application.escape_dc,
        application.notes.as_deref(),
        sink,
    )
}

/// Removes a condition by name.
pub fn remove_condition<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    condition: &str,
    sink: &mut L,
) -> Result<bool> {
    let index = state.find_index(who)?;
    remove_condition_at(state, index, condition, sink)
}

fn remove_condition_at<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    target: usize,
    condition: &str,
    sink: &mut L,
) -> Result<bool> {
    let name = state.combatants[target].name().to_string();
    let combatant = &mut state.combatants[target];
    let before = combatant.conditions.len();
    combatant
        .conditions
        .retain(|active| !active.condition.eq_ignore_ascii_case(condition));
    let removed = combatant.conditions.len() != before;
    if removed {
        sink.log(format_args!("{name} is no longer {condition}"));
    }
    Ok(removed)
}

/// Moves a combatant to another zone.
pub fn move_to<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    zone: &str,
    sink: &mut L,
) -> Result<MoveReport> {
    let index = state.find_index(who)?;
    if !state.zones.contains_key(zone) {
        bail!(
            "unknown zone `{zone}` (known zones: {})",
            state.zones.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    let name = state.combatants[index].name().to_string();
    let from = state.combatants[index].zone.clone();
    let from_position = zone_position(state, &from);
    let to_position = zone_position(state, zone);
    let distance = (from_position - to_position).abs();
    let combatant = &mut state.combatants[index];
    let speed_before = combatant.speed_left;
    combatant.zone = zone.to_string();
    combatant.speed_left = (speed_before - distance).max(0);
    sink.log(format_args!(
        "{name} moves {from} -> {zone} ({distance} ft)"
    ));
    if distance > speed_before {
        sink.log(format_args!(
            "warning: {name} moved {distance} ft with only {speed_before} ft of speed left"
        ));
    }
    Ok(MoveReport {
        who: name,
        from,
        to: zone.to_string(),
        distance,
        speed_left_before: speed_before,
        speed_left_after: combatant.speed_left,
    })
}

/// Rolls a death saving throw manually.
pub fn death_save<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    who: &str,
    rng: &mut R,
    sink: &mut L,
) -> Result<DeathSaveReport> {
    let index = state.find_index(who)?;
    death_save_at(state, index, rng, sink)
}

fn death_save_at<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut EncounterState,
    index: usize,
    rng: &mut R,
    sink: &mut L,
) -> Result<DeathSaveReport> {
    let combatant = &state.combatants[index];
    let who = combatant.name().to_string();
    if combatant.kind() != Kind::Pc {
        bail!("{who} is not a player character");
    }
    if combatant.death_saves.dead {
        bail!("{who} is dead");
    }
    if combatant.hp > 0 {
        bail!("{who} is not at 0 HP");
    }
    if combatant.death_saves.stable {
        bail!("{who} is stable and does not roll death saves");
    }

    let roll = D20Roll::roll(0, RollMode::Normal, rng);
    if sink.enabled() {
        sink.log(format_args!("{who} death save: {}", roll.text()));
    }

    let combatant = &mut state.combatants[index];
    let mut revived = false;
    if roll.nat20 {
        combatant.hp = 1;
        combatant.death_saves = Default::default();
        combatant
            .conditions
            .retain(|condition| !condition.condition.eq_ignore_ascii_case("unconscious"));
        revived = true;
        sink.log(format_args!("{who} regains 1 HP and is conscious"));
    } else {
        if roll.nat1 {
            combatant.death_saves.failures =
                combatant.death_saves.failures.saturating_add(2).min(3);
        } else if roll.total >= 10 {
            combatant.death_saves.successes =
                combatant.death_saves.successes.saturating_add(1).min(3);
        } else {
            combatant.death_saves.failures =
                combatant.death_saves.failures.saturating_add(1).min(3);
        }
        if combatant.death_saves.failures >= 3 {
            combatant.death_saves.dead = true;
            sink.log(format_args!("{who} dies"));
        } else if combatant.death_saves.successes >= 3 {
            combatant.death_saves.stable = true;
            sink.log(format_args!("{who} stabilizes"));
        } else {
            sink.log(format_args!(
                "{who}: {} success(es), {} failure(s)",
                combatant.death_saves.successes, combatant.death_saves.failures
            ));
        }
    }

    let death_saves = state.combatants[index].death_saves;
    Ok(DeathSaveReport {
        who,
        roll: roll.kept,
        total: roll.total,
        successes: death_saves.successes,
        failures: death_saves.failures,
        stable: death_saves.stable,
        dead: death_saves.dead,
        revived,
    })
}

/// Applies a short or long rest.
pub fn rest<L: LogSink + ?Sized>(
    state: &mut EncounterState,
    kind: RestKind,
    who: Option<&str>,
    sink: &mut L,
) -> Result<()> {
    let explicit = match who {
        None => false,
        Some(name) => !name.eq_ignore_ascii_case("all"),
    };
    let indices: Vec<usize> = match who {
        Some(name) if !name.eq_ignore_ascii_case("all") => vec![state.find_index(name)?],
        _ => (0..state.combatants.len()).collect(),
    };

    for index in indices {
        let combatant = &state.combatants[index];
        if combatant.death_saves.dead {
            if explicit {
                bail!("{} is dead and cannot rest", combatant.name());
            }
            continue;
        }
        let name = combatant.name().to_string();
        let was_at_zero = combatant.hp <= 0;
        let combatant = &mut state.combatants[index];
        match kind {
            RestKind::Short => {
                for resource in combatant.resources.values_mut() {
                    if resource.reset == ResetType::Short {
                        resource.current = resource.max;
                    }
                }
                sink.log(format_args!("{name} finishes a short rest"));
            }
            RestKind::Long => {
                for resource in combatant.resources.values_mut() {
                    resource.current = resource.max;
                }
                combatant.hp = combatant.spec.max_hp;
                combatant.death_saves = Default::default();
                combatant.temp_hp = 0;
                if was_at_zero && combatant.kind() == Kind::Pc {
                    combatant.conditions.retain(|condition| {
                        !condition.condition.eq_ignore_ascii_case("unconscious")
                    });
                }
                sink.log(format_args!(
                    "{name} finishes a long rest ({}/{})",
                    combatant.hp, combatant.spec.max_hp
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use std::collections::BTreeMap;

    use crate::model::{ExtraDamage, SaveSpec};
    use crate::state::EncounterState;

    struct ScriptRng {
        values: VecDeque<u32>,
    }

    impl ScriptRng {
        fn new(values: &[u32]) -> Self {
            Self {
                values: values.iter().copied().collect(),
            }
        }
    }

    impl DiceRng for ScriptRng {
        fn roll_die(&mut self, sides: u32) -> u32 {
            let value = self
                .values
                .pop_front()
                .expect("scripted RNG ran out of values");
            assert!(
                value >= 1 && value <= sides,
                "scripted value {value} is not a d{sides}"
            );
            value
        }
    }

    fn abilities(str: i32, dex: i32, con: i32) -> BTreeMap<String, i32> {
        BTreeMap::from([
            ("str".to_string(), str),
            ("dex".to_string(), dex),
            ("con".to_string(), con),
            ("int".to_string(), 10),
            ("wis".to_string(), 10),
            ("cha".to_string(), 10),
        ])
    }

    fn spec(
        name: &str,
        side: Side,
        kind: Kind,
        hp: i32,
        ac: i32,
        init_bonus: i32,
        dex: i32,
    ) -> CombatantSpec {
        CombatantSpec {
            name: name.to_string(),
            side,
            kind,
            hp,
            max_hp: hp,
            temp_hp: 0,
            ac,
            init_bonus,
            speed: 30,
            proficiency_bonus: 2,
            abilities: abilities(10, dex, 10),
            saves: BTreeMap::new(),
            skills: BTreeMap::new(),
            creature_type: String::new(),
            zone: None,
            attacks: Vec::new(),
            multiattack: Vec::new(),
            heals: Vec::new(),
            reactions: Vec::new(),
            resources: BTreeMap::new(),
            features: BTreeMap::new(),
            resistances: Vec::new(),
            immunities: Vec::new(),
            vulnerabilities: Vec::new(),
            condition_immunities: Vec::new(),
            conditions: Vec::new(),
            notes: None,
        }
    }

    fn make_attack(name: &str, to_hit: i32, damage: &str, damage_type: &str) -> Attack {
        Attack {
            name: name.to_string(),
            kind: AttackKind::Melee,
            to_hit: Some(to_hit),
            damage: Some(damage.to_string()),
            damage_type: Some(damage_type.to_string()),
            extra_damage: Vec::new(),
            reach: Some(5),
            range: None,
            mastery: None,
            sneak_eligible: false,
            count: 1,
            cost: None,
            save: None,
            on_hit_conditions: Vec::new(),
            on_fail_conditions: Vec::new(),
            vs_condition: None,
            notes: None,
        }
    }

    fn repo_path(relative: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    fn state_with(specs: Vec<CombatantSpec>) -> EncounterState {
        let combatants = specs
            .into_iter()
            .map(|spec| CombatantState::from_spec(spec, "main"))
            .collect();
        EncounterState {
            name: None,
            zones: BTreeMap::from([("main".to_string(), 0)]),
            combatants,
            initiative: Vec::new(),
            turn: 0,
            round: 0,
            log: Vec::new(),
        }
    }

    fn run_attack(
        state: &mut EncounterState,
        attacker: &str,
        target: &str,
        name: &str,
        options: &AttackOptions,
        rng: &mut ScriptRng,
    ) -> AttackReport {
        let mut log = Vec::new();
        attack(state, attacker, target, name, options, rng, &mut log).unwrap()
    }

    #[test]
    fn critical_doubles_dice_and_resistance_halves() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 5, "1d8+4", "slashing"));
        let mut target = spec("Slime", Side::Enemy, Kind::Monster, 30, 10, 0, 10);
        target.resistances.push("slashing".to_string());
        let mut state = state_with(vec![attacker, target]);

        let mut rng = ScriptRng::new(&[20, 3, 5]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Slime",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );

        assert!(report.hit);
        assert!(report.critical);
        let damage = report.damage.expect("attack dealt damage");
        assert_eq!(damage.requested, 12);
        assert_eq!(damage.amount, 6);
        assert_eq!(state.combatants[1].hp, 24);
    }

    #[test]
    fn natural_one_auto_misses() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 10, "1d8+4", "slashing"));
        let target = spec("Guard", Side::Enemy, Kind::Monster, 11, 10, 0, 10);
        let mut state = state_with(vec![attacker, target]);

        let mut rng = ScriptRng::new(&[1]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );

        assert!(!report.hit);
        assert!(report.natural_1);
        assert_eq!(report.total_damage, 0);
        assert_eq!(state.combatants[1].hp, 11);
    }

    #[test]
    fn advantage_and_disadvantage_cancel_to_normal() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 5, "1d8+4", "slashing"));
        let target = spec("Guard", Side::Enemy, Kind::Monster, 11, 15, 0, 10);
        let mut state = state_with(vec![attacker, target]);

        let options = AttackOptions {
            advantage: true,
            disadvantage: true,
            ..AttackOptions::default()
        };
        let mut rng = ScriptRng::new(&[12, 4]);
        let report = run_attack(&mut state, "Fighter", "Guard", "sword", &options, &mut rng);
        assert_eq!(
            report.attack_roll.as_ref().map(|roll| roll.mode),
            Some(RollMode::Normal)
        );
        assert_eq!(report.attack_total, Some(17));
        assert!(report.hit);
    }

    #[test]
    fn advantage_keeps_the_higher_die() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 5, "1d8+4", "slashing"));
        let target = spec("Guard", Side::Enemy, Kind::Monster, 11, 15, 0, 10);
        let mut state = state_with(vec![attacker, target]);

        let options = AttackOptions {
            advantage: true,
            ..AttackOptions::default()
        };
        let mut rng = ScriptRng::new(&[3, 17, 4]);
        let report = run_attack(&mut state, "Fighter", "Guard", "sword", &options, &mut rng);
        let roll = report.attack_roll.expect("attack roll");
        assert_eq!(roll.dice, vec![3, 17]);
        assert_eq!(roll.kept, 17);
        assert_eq!(report.attack_total, Some(22));
    }

    #[test]
    fn resistance_immunity_and_temporary_hp() {
        let mut resistant = spec("Resistant", Side::Enemy, Kind::Monster, 20, 10, 0, 10);
        resistant.resistances.push("fire".to_string());
        let mut immune = spec("Immune", Side::Enemy, Kind::Monster, 20, 10, 0, 10);
        immune.immunities.push("fire".to_string());
        let mut tough = spec("Tough", Side::Enemy, Kind::Monster, 20, 10, 0, 10);
        tough.temp_hp = 4;
        let mut state = state_with(vec![resistant, immune, tough]);

        let mut log = Vec::new();
        let mut rng = ScriptRng::new(&[]);
        let report = damage(
            &mut state,
            "Resistant",
            "7",
            Some("fire"),
            false,
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.amount, 3);
        assert_eq!(state.combatants[0].hp, 17);

        let report = damage(
            &mut state,
            "Immune",
            "7",
            Some("fire"),
            false,
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.amount, 0);
        assert_eq!(state.combatants[1].hp, 20);

        let report = damage(&mut state, "Tough", "10", None, false, &mut rng, &mut log).unwrap();
        assert_eq!(report.temp_absorbed, 4);
        assert_eq!(state.combatants[2].temp_hp, 0);
        assert_eq!(state.combatants[2].hp, 14);
    }

    #[test]
    fn shield_turns_a_hit_into_a_miss_and_spends_a_reaction() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 6, "1d8+4", "slashing"));
        let mut target = spec("Mage", Side::Enemy, Kind::Pc, 20, 16, 0, 10);
        target.reactions.push(Reaction {
            name: "shield".to_string(),
            ac_bonus: 5,
            cost: Some(Cost {
                resource: "shield_free".to_string(),
                amount: 1,
                notes: None,
            }),
            notes: None,
        });
        target.resources.insert(
            "shield_free".to_string(),
            crate::model::Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let mut state = state_with(vec![attacker, target]);

        let mut rng = ScriptRng::new(&[12]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Mage",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(!report.hit);
        assert!(report.shield);
        assert!(state.combatants[1].reaction_used);
        assert_eq!(state.combatants[1].resources["shield_free"].current, 0);
    }

    #[test]
    fn natural_twenty_is_not_shielded() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 6, "1d8+4", "slashing"));
        let mut target = spec("Mage", Side::Enemy, Kind::Pc, 20, 16, 0, 10);
        target.reactions.push(Reaction {
            name: "shield".to_string(),
            ac_bonus: 5,
            cost: None,
            notes: None,
        });
        let mut state = state_with(vec![attacker, target]);

        let mut rng = ScriptRng::new(&[20, 2, 2]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Mage",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.hit);
        assert!(report.critical);
        assert!(!report.shield);
        assert!(!state.combatants[1].reaction_used);
    }

    fn sneak_attacker() -> CombatantSpec {
        let mut attacker = spec("Rogue", Side::Party, Kind::Pc, 20, 14, 0, 16);
        let mut dagger = make_attack("dagger", 5, "1d4+3", "piercing");
        dagger.sneak_eligible = true;
        attacker.attacks.push(dagger);
        attacker
            .features
            .insert("sneak_attack".to_string(), "1d6".to_string());
        attacker
    }

    #[test]
    fn sneak_attack_applies_once_per_turn() {
        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let mut state = state_with(vec![sneak_attacker(), target]);
        let options = AttackOptions {
            advantage: true,
            ..AttackOptions::default()
        };

        let mut rng = ScriptRng::new(&[15, 10, 3, 4]);
        let first = run_attack(&mut state, "Rogue", "Guard", "dagger", &options, &mut rng);
        assert!(first.sneak);
        assert_eq!(first.total_damage, 10);
        assert!(state.combatants[0].sneak_used_this_turn);

        let mut rng = ScriptRng::new(&[15, 10, 4]);
        let second = run_attack(&mut state, "Rogue", "Guard", "dagger", &options, &mut rng);
        assert!(!second.sneak);
        assert_eq!(second.total_damage, 7);
    }

    #[test]
    fn sneak_attack_not_on_miss_or_disadvantage() {
        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let mut state = state_with(vec![sneak_attacker(), target]);
        let advantage = AttackOptions {
            advantage: true,
            ..AttackOptions::default()
        };

        let mut rng = ScriptRng::new(&[1, 1]);
        let miss = run_attack(&mut state, "Rogue", "Guard", "dagger", &advantage, &mut rng);
        assert!(!miss.hit);
        assert!(!miss.sneak);
        assert!(!state.combatants[0].sneak_used_this_turn);

        let disadvantage = AttackOptions {
            disadvantage: true,
            ..AttackOptions::default()
        };
        let mut rng = ScriptRng::new(&[15, 10, 4]);
        let hit = run_attack(
            &mut state,
            "Rogue",
            "Guard",
            "dagger",
            &disadvantage,
            &mut rng,
        );
        assert!(hit.hit);
        assert!(!hit.sneak);
    }

    fn death_state() -> EncounterState {
        let mut pc = spec("Hero", Side::Party, Kind::Pc, 12, 15, 0, 10);
        pc.hp = 0;
        let mut state = state_with(vec![pc]);
        state.combatants[0].hp = 0;
        state.combatants[0].conditions.push(ActiveCondition {
            condition: "unconscious".to_string(),
            source: None,
            escape_dc: None,
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });
        state
    }

    #[test]
    fn death_save_nat_twenty_revives() {
        let mut state = death_state();
        let mut rng = ScriptRng::new(&[20]);
        let mut log = Vec::new();
        let report = death_save(&mut state, "Hero", &mut rng, &mut log).unwrap();
        assert!(report.revived);
        assert_eq!(state.combatants[0].hp, 1);
        assert!(!state.combatants[0].has_condition("unconscious"));
        assert_eq!(state.combatants[0].death_saves.failures, 0);
    }

    #[test]
    fn death_save_nat_one_and_three_failures_kill() {
        let mut state = death_state();
        let mut rng = ScriptRng::new(&[1]);
        let mut log = Vec::new();
        let report = death_save(&mut state, "Hero", &mut rng, &mut log).unwrap();
        assert_eq!(report.failures, 2);

        let mut rng = ScriptRng::new(&[2]);
        let report = death_save(&mut state, "Hero", &mut rng, &mut log).unwrap();
        assert!(report.dead);
        assert_eq!(report.failures, 3);
    }

    #[test]
    fn death_save_three_successes_stabilize() {
        let mut state = death_state();
        let mut log = Vec::new();
        for _ in 0..3 {
            let mut rng = ScriptRng::new(&[15]);
            death_save(&mut state, "Hero", &mut rng, &mut log).unwrap();
        }
        assert!(state.combatants[0].death_saves.stable);
        assert!(!state.combatants[0].death_saves.dead);
    }

    #[test]
    fn damage_at_zero_adds_failures_and_massive_damage_kills() {
        let mut state = death_state();
        let mut log = Vec::new();
        let mut rng = ScriptRng::new(&[]);
        let report = damage(&mut state, "Hero", "3", None, false, &mut rng, &mut log).unwrap();
        assert_eq!(report.death_save_failures, 1);
        assert!(!report.dead);

        let report = damage(&mut state, "Hero", "3", None, true, &mut rng, &mut log).unwrap();
        assert_eq!(report.death_save_failures, 3);
        assert!(report.dead);

        let mut pc = spec("Other", Side::Party, Kind::Pc, 5, 15, 0, 10);
        pc.max_hp = 10;
        let mut state = state_with(vec![pc]);
        let report = damage(&mut state, "Other", "15", None, false, &mut rng, &mut log).unwrap();
        assert!(report.massive);
        assert!(report.dead);
        assert_eq!(state.combatants[0].hp, 0);
        assert_eq!(report.hp_after, 0);
    }

    #[test]
    fn monsters_die_at_zero_hp() {
        let mut state = state_with(vec![spec(
            "Guard",
            Side::Enemy,
            Kind::Monster,
            5,
            16,
            1,
            10,
        )]);
        let mut log = Vec::new();
        let mut rng = ScriptRng::new(&[]);
        let report = damage(&mut state, "Guard", "9", None, false, &mut rng, &mut log).unwrap();
        assert!(report.dead);
        assert_eq!(state.combatants[0].hp, 0);
        assert!(state.combatants[0].death_saves.dead);
    }

    #[test]
    fn condition_rounds_expire_at_end_of_own_turn() {
        let mut state = state_with(vec![
            spec("A", Side::Party, Kind::Pc, 10, 10, 0, 10),
            spec("B", Side::Enemy, Kind::Monster, 10, 10, 0, 10),
        ]);
        state.initiative = vec![0, 1];
        state.round = 1;
        state.turn = 0;
        let mut log = Vec::new();
        add_condition(
            &mut state,
            "A",
            "frightened",
            &ConditionOptions {
                rounds: Some(2),
                ..ConditionOptions::default()
            },
            &mut log,
        )
        .unwrap();

        let mut rng = ScriptRng::new(&[]);
        next_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(
            state.combatants[0].conditions[0]
                .duration
                .as_ref()
                .and_then(|duration| duration.rounds),
            Some(1)
        );

        next_turn(&mut state, &mut rng, &mut log).unwrap();
        next_turn(&mut state, &mut rng, &mut log).unwrap();
        assert!(state.combatants[0].conditions.is_empty());
    }

    #[test]
    fn end_of_source_next_turn_expiry_round_is_computed() {
        let mut state = state_with(vec![
            spec("A", Side::Party, Kind::Pc, 10, 10, 0, 10),
            spec("B", Side::Enemy, Kind::Monster, 10, 10, 0, 10),
        ]);
        state.initiative = vec![0, 1];
        state.round = 1;
        state.turn = 0;
        let mut log = Vec::new();
        add_condition(
            &mut state,
            "A",
            "frightened",
            &ConditionOptions {
                until_end_of_turn: Some("B".to_string()),
                source: Some("B".to_string()),
                ..ConditionOptions::default()
            },
            &mut log,
        )
        .unwrap();
        assert_eq!(state.combatants[0].conditions[0].expires_round, Some(1));
        assert_eq!(
            state.combatants[0].conditions[0]
                .ends_with_turn_of
                .as_deref(),
            Some("B")
        );

        let mut rng = ScriptRng::new(&[]);
        next_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(state.combatants[0].conditions.len(), 1);

        next_turn(&mut state, &mut rng, &mut log).unwrap();
        assert!(state.combatants[0].conditions.is_empty());
    }

    #[test]
    fn auto_crit_and_save_auto_fail() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 20, 15, 0, 10);
        attacker
            .attacks
            .push(make_attack("sword", 5, "1d8+4", "slashing"));
        let mut paralyzed = spec("Paralyzed", Side::Enemy, Kind::Monster, 30, 10, 0, 10);
        paralyzed.condition_immunities.push("paralyzed".to_string());
        let mut state = state_with(vec![attacker, paralyzed]);
        state.combatants[1].conditions.push(ActiveCondition {
            condition: "paralyzed".to_string(),
            source: None,
            escape_dc: None,
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });

        let mut rng = ScriptRng::new(&[15, 2, 3, 4]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Paralyzed",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.hit);
        assert!(report.critical);
        assert_eq!(
            report.damage.as_ref().map(|damage| damage.requested),
            Some(11)
        );

        let mut rng = ScriptRng::new(&[]);
        let mut log = Vec::new();
        let save_report = save(
            &mut state,
            "Paralyzed",
            "dex",
            15,
            RollMode::Normal,
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert!(save_report.auto_fail);
        assert!(!save_report.success);
    }

    #[test]
    fn multiattack_runs_every_sequence_entry() {
        let mut attacker = spec("Armor", Side::Enemy, Kind::Monster, 33, 18, 0, 10);
        attacker
            .attacks
            .push(make_attack("slam", 4, "1d6+2", "bludgeoning"));
        attacker.multiattack = vec!["slam".to_string(), "slam".to_string()];
        let target = spec("Hero", Side::Party, Kind::Pc, 30, 10, 0, 10);
        let mut state = state_with(vec![attacker, target]);

        let mut rng = ScriptRng::new(&[10, 4, 12, 5]);
        let options = AttackOptions {
            multi: true,
            ..AttackOptions::default()
        };
        let report = run_attack(&mut state, "Armor", "Hero", "slam", &options, &mut rng);
        assert_eq!(report.instances.len(), 2);
        assert_eq!(report.total_damage, 13);
        assert_eq!(state.combatants[1].hp, 17);
    }

    #[test]
    fn initiative_orders_by_total_then_bonus_then_dex() {
        let mut state = state_with(vec![
            spec("A", Side::Party, Kind::Pc, 10, 10, 2, 14),
            spec("B", Side::Party, Kind::Pc, 10, 10, 2, 12),
            spec("C", Side::Enemy, Kind::Monster, 10, 10, 0, 16),
        ]);
        let mut rng = ScriptRng::new(&[10, 10, 10]);
        let mut log = Vec::new();
        init(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(state.initiative, vec![0, 1, 2]);
        assert_eq!(state.combatants[0].initiative_total, Some(12));
        assert_eq!(state.combatants[1].initiative_total, Some(12));
        assert_eq!(state.combatants[2].initiative_total, Some(10));
        assert_eq!(state.round, 1);
        assert_eq!(state.turn, 0);
    }

    #[test]
    fn initiative_overrides_skip_rng_and_set_totals() {
        let mut state = state_with(vec![
            spec("A", Side::Party, Kind::Pc, 10, 10, 2, 14),
            spec("B", Side::Party, Kind::Pc, 10, 10, 0, 10),
        ]);
        let mut rng = ScriptRng::new(&[10]);
        let mut log = Vec::new();
        init_with_overrides(&mut state, &[(0, 30)], &mut rng, &mut log).unwrap();
        assert_eq!(state.initiative, vec![0, 1]);
        assert_eq!(state.combatants[0].initiative_total, Some(30));
        assert_eq!(state.combatants[1].initiative_total, Some(10));
        assert!(log.iter().any(|line| line.contains("(set)")));
    }

    #[test]
    fn next_turn_skips_dead_and_resets_flags() {
        let mut state = state_with(vec![
            spec("A", Side::Party, Kind::Pc, 10, 10, 0, 10),
            spec("B", Side::Enemy, Kind::Monster, 10, 10, 0, 10),
            spec("C", Side::Party, Kind::Pc, 10, 10, 0, 10),
        ]);
        state.initiative = vec![0, 1, 2];
        state.round = 1;
        state.turn = 0;
        state.combatants[1].death_saves.dead = true;
        state.combatants[2].reaction_used = true;
        state.combatants[2].sneak_used_this_turn = true;
        state.combatants[2].action_used = true;
        state.combatants[2].speed_left = 0;

        let mut rng = ScriptRng::new(&[]);
        let mut log = Vec::new();
        let report = next_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(report.active, "C");
        assert_eq!(report.turn, 2);
        assert_eq!(report.skipped, vec!["B"]);
        assert!(!state.combatants[2].reaction_used);
        assert!(!state.combatants[2].sneak_used_this_turn);
        assert!(!state.combatants[2].action_used);
        assert_eq!(state.combatants[2].speed_left, 30);
    }

    #[test]
    fn rest_short_and_long_restore_the_right_things() {
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 12, 15, 0, 10);
        hero.resources.insert(
            "short".to_string(),
            crate::model::Resource {
                current: 0,
                max: 2,
                reset: ResetType::Short,
                notes: None,
            },
        );
        hero.resources.insert(
            "long".to_string(),
            crate::model::Resource {
                current: 0,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let mut state = state_with(vec![hero]);
        state.combatants[0].hp = 4;

        let mut log = Vec::new();
        rest(&mut state, RestKind::Short, None, &mut log).unwrap();
        assert_eq!(state.combatants[0].resources["short"].current, 2);
        assert_eq!(state.combatants[0].resources["long"].current, 0);
        assert_eq!(state.combatants[0].hp, 4);

        rest(&mut state, RestKind::Long, Some("all"), &mut log).unwrap();
        assert_eq!(state.combatants[0].resources["short"].current, 2);
        assert_eq!(state.combatants[0].resources["long"].current, 1);
        assert_eq!(state.combatants[0].hp, 12);
    }

    #[test]
    fn combat_outcome_reports_winners_and_draws() {
        let mut state = state_with(vec![
            spec("A", Side::Party, Kind::Pc, 10, 10, 0, 10),
            spec("B", Side::Enemy, Kind::Monster, 10, 10, 0, 10),
        ]);
        assert_eq!(combat_outcome(&state), None);
        state.combatants[1].hp = 0;
        state.combatants[1].death_saves.dead = true;
        assert_eq!(combat_outcome(&state), Some(CombatOutcome::PartyWins));
        state.combatants[0].hp = 0;
        state.combatants[0].death_saves.dead = true;
        assert_eq!(combat_outcome(&state), Some(CombatOutcome::Draw));
    }

    #[test]
    fn cast_and_use_spend_resources() {
        let mut caster = spec("Mage", Side::Party, Kind::Pc, 20, 12, 0, 10);
        let mut missile = Attack {
            name: "magic_missile".to_string(),
            kind: AttackKind::Auto,
            to_hit: None,
            damage: Some("1d4+1".to_string()),
            damage_type: Some("force".to_string()),
            extra_damage: Vec::new(),
            reach: None,
            range: Some([120, 120]),
            mastery: None,
            sneak_eligible: false,
            count: 3,
            cost: Some(Cost {
                resource: "slot1".to_string(),
                amount: 1,
                notes: None,
            }),
            save: None,
            on_hit_conditions: Vec::new(),
            on_fail_conditions: Vec::new(),
            vs_condition: None,
            notes: None,
        };
        missile.extra_damage.push(ExtraDamage {
            dice: "1d4".to_string(),
            damage_type: "force".to_string(),
            notes: None,
        });
        caster.attacks.push(missile);
        caster.resources.insert(
            "slot1".to_string(),
            crate::model::Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let target = spec("Guard", Side::Enemy, Kind::Monster, 30, 10, 0, 10);
        let mut state = state_with(vec![caster, target]);

        let mut rng = ScriptRng::new(&[2, 3, 1, 2, 2, 4, 1, 1, 1]);
        let mut log = Vec::new();
        let report = cast(
            &mut state,
            "Mage",
            "magic_missile",
            &["Guard".to_string()],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.attacks.len(), 1);
        assert_eq!(state.combatants[0].resources["slot1"].current, 0);

        let mut rng = ScriptRng::new(&[]);
        let error = cast(
            &mut state,
            "Mage",
            "magic_missile",
            &["Guard".to_string()],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .expect_err("second cast should be out of slots");
        assert!(error.to_string().contains("slot1"));

        state.combatants[0]
            .resources
            .get_mut("slot1")
            .expect("slot1 exists")
            .current = 1;
        let mut rng = ScriptRng::new(&[]);
        let use_report = use_entry(&mut state, "Mage", "slot1", None, &mut rng, &mut log).unwrap();
        assert_eq!(use_report.cost_spent.as_deref(), Some("slot1 x1"));
        assert!(state.combatants[0].resources["slot1"].current == 0);
    }

    #[test]
    fn heal_at_zero_clears_unconscious_and_death_saves() {
        let mut state = death_state();
        state.combatants[0].death_saves.failures = 2;
        state.combatants[0].death_saves.stable = false;
        let mut rng = ScriptRng::new(&[]);
        let mut log = Vec::new();
        let report = heal(&mut state, "Hero", "5", Some("Cleric"), &mut rng, &mut log).unwrap();
        assert_eq!(report.hp_before, 0);
        assert_eq!(report.hp_after, 5);
        assert!(report.revived);
        assert!(!state.combatants[0].has_condition("unconscious"));
        assert_eq!(state.combatants[0].death_saves.failures, 0);
        assert!(!state.combatants[0].death_saves.stable);
    }

    #[test]
    fn range_errors_and_long_range_disadvantage() {
        let mut archer = spec("Archer", Side::Party, Kind::Pc, 20, 14, 0, 10);
        let mut bow = make_attack("bow", 5, "1d6+3", "piercing");
        bow.kind = AttackKind::Ranged;
        bow.reach = None;
        bow.range = Some([30, 120]);
        archer.attacks.push(bow);
        let target = spec("Guard", Side::Enemy, Kind::Monster, 20, 10, 0, 10);
        let mut state = state_with(vec![archer, target]);
        state.zones.insert("far".to_string(), 60);
        state.combatants[1].zone = "far".to_string();

        let mut rng = ScriptRng::new(&[3, 17, 4]);
        let report = run_attack(
            &mut state,
            "Archer",
            "Guard",
            "bow",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(report.attack_roll.as_ref().map(|roll| roll.kept), Some(3));
        assert_eq!(
            report.attack_roll.as_ref().map(|roll| roll.mode),
            Some(RollMode::Disadvantage)
        );

        state.zones.insert("too_far".to_string(), 200);
        state.combatants[1].zone = "too_far".to_string();
        let mut rng = ScriptRng::new(&[]);
        let mut log = Vec::new();
        let error = attack(
            &mut state,
            "Archer",
            "Guard",
            "bow",
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .expect_err("out-of-range attack should fail");
        assert!(error.to_string().contains("cannot reach"));
    }

    #[test]
    fn ranged_attack_adjacent_to_an_enemy_has_disadvantage() {
        let mut archer = spec("Archer", Side::Party, Kind::Pc, 20, 14, 0, 10);
        let mut bow = make_attack("bow", 5, "1d6+3", "piercing");
        bow.kind = AttackKind::Ranged;
        bow.reach = None;
        bow.range = Some([30, 120]);
        archer.attacks.push(bow);
        let target = spec("Guard", Side::Enemy, Kind::Monster, 20, 5, 0, 10);
        let mut state = state_with(vec![archer, target]);

        let mut rng = ScriptRng::new(&[15, 2, 3]);
        let report = run_attack(
            &mut state,
            "Archer",
            "Guard",
            "bow",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(
            report.attack_roll.as_ref().map(|roll| roll.mode),
            Some(RollMode::Disadvantage)
        );
    }

    #[test]
    fn mastery_sap_vex_and_slow_apply_markers() {
        let mut fighter = spec("Fighter", Side::Party, Kind::Pc, 30, 15, 0, 10);
        let mut sword = make_attack("sword", 5, "1d8+4", "slashing");
        sword.mastery = Some(Mastery::Sap);
        fighter.attacks.push(sword);
        let mut guard = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        guard
            .attacks
            .push(make_attack("spear", 3, "1d6+1", "piercing"));
        let mut state = state_with(vec![fighter, guard]);

        let mut rng = ScriptRng::new(&[15, 4]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(report.mastery, Some(Mastery::Sap));
        assert!(state.combatants[1].has_marker(MarkerKind::Sap));

        let mut rng = ScriptRng::new(&[15, 10, 4]);
        let guard_attack = run_attack(
            &mut state,
            "Guard",
            "Fighter",
            "spear",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(
            guard_attack.attack_roll.as_ref().map(|roll| roll.mode),
            Some(RollMode::Disadvantage)
        );
        assert!(!state.combatants[1].has_marker(MarkerKind::Sap));

        state.combatants[0].markers.clear();
        let mut rng = ScriptRng::new(&[15, 4]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(report.mastery, Some(Mastery::Sap));
        assert!(state.combatants[1].has_marker(MarkerKind::Sap));
    }

    #[test]
    fn mastery_topple_and_graze() {
        let mut fighter = spec("Fighter", Side::Party, Kind::Pc, 30, 15, 0, 10);
        let mut sword = make_attack("sword", 5, "1d8+4", "slashing");
        sword.mastery = Some(Mastery::Topple);
        fighter.attacks.push(sword);
        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let mut state = state_with(vec![fighter, target]);

        let mut rng = ScriptRng::new(&[15, 4, 2]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(report.mastery, Some(Mastery::Topple));
        assert!(state.combatants[1].has_condition("prone"));

        state.combatants[1].conditions.clear();
        state.combatants[0].spec.attacks[0].mastery = Some(Mastery::Graze);
        state.combatants[0].spec.attacks[0].to_hit = Some(5);
        let mut rng = ScriptRng::new(&[1]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(!report.hit);
        assert!(report.graze);
        assert_eq!(report.total_damage, 3);
    }

    #[test]
    fn slow_marker_reduces_speed_immediately_and_at_turn_start() {
        let mut fighter = spec("Fighter", Side::Party, Kind::Pc, 30, 15, 0, 10);
        let mut sword = make_attack("sword", 5, "1d8+4", "slashing");
        sword.mastery = Some(Mastery::Slow);
        fighter.attacks.push(sword);
        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let mut state = state_with(vec![fighter, target]);
        state.combatants[1].speed_left = 30;

        let mut rng = ScriptRng::new(&[15, 4]);
        run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(state.combatants[1].has_marker(MarkerKind::Slow));
        assert_eq!(state.combatants[1].speed_left, 20);
        assert_eq!(state.combatants[1].effective_speed(), 20);
    }

    #[test]
    fn invalid_condition_durations_are_rejected() {
        let mut state = state_with(vec![spec("A", Side::Party, Kind::Pc, 10, 10, 0, 10)]);
        let mut log = Vec::new();
        let error = add_condition(
            &mut state,
            "A",
            "prone",
            &ConditionOptions {
                rounds: Some(0),
                ..ConditionOptions::default()
            },
            &mut log,
        )
        .expect_err("zero rounds is invalid");
        assert!(error.to_string().contains("at least 1"));
    }
    #[test]
    fn paralyzed_target_beyond_five_feet_is_not_auto_crit() {
        let mut attacker = spec("Archer", Side::Party, Kind::Pc, 20, 14, 0, 10);
        let mut bow = make_attack("bow", 5, "1d6+3", "piercing");
        bow.kind = AttackKind::Ranged;
        bow.reach = None;
        bow.range = Some([30, 120]);
        attacker.attacks.push(bow);
        let target = spec("Paralyzed", Side::Enemy, Kind::Monster, 30, 10, 0, 10);
        let mut state = state_with(vec![attacker, target]);
        state.zones.insert("far".to_string(), 10);
        state.combatants[1].zone = "far".to_string();
        state.combatants[1].conditions.push(ActiveCondition {
            condition: "paralyzed".to_string(),
            source: None,
            escape_dc: None,
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });

        let mut rng = ScriptRng::new(&[15, 10, 4]);
        let report = run_attack(
            &mut state,
            "Archer",
            "Paralyzed",
            "bow",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.hit);
        assert!(!report.critical);
    }

    #[test]
    fn condition_immunity_blocks_condition_application() {
        let mut target = spec("Mimic", Side::Enemy, Kind::Monster, 20, 12, 0, 10);
        target.condition_immunities.push("prone".to_string());
        let mut state = state_with(vec![target]);
        let mut log = Vec::new();
        let applied = add_condition(
            &mut state,
            "Mimic",
            "prone",
            &ConditionOptions::default(),
            &mut log,
        )
        .unwrap();
        assert!(!applied);
        assert!(!state.combatants[0].has_condition("prone"));
    }

    #[test]
    fn use_heal_entry_runs_a_self_heal_and_spends_the_cost() {
        let mut hero = spec("Will", Side::Party, Kind::Pc, 20, 18, 0, 10);
        hero.heals.push(Heal {
            name: "second_wind".to_string(),
            amount: "1d10+1".to_string(),
            range: 0,
            action: HealAction::Bonus,
            cost: Some(Cost {
                resource: "second_wind".to_string(),
                amount: 1,
                notes: None,
            }),
            self_only: true,
            notes: None,
        });
        hero.resources.insert(
            "second_wind".to_string(),
            crate::model::Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let mut state = state_with(vec![hero]);
        state.combatants[0].hp = 5;

        let mut rng = ScriptRng::new(&[5]);
        let mut log = Vec::new();
        let report =
            use_entry(&mut state, "Will", "second_wind", None, &mut rng, &mut log).unwrap();
        assert_eq!(report.name, "second_wind");
        assert_eq!(report.healed.unwrap().healed, 6);
        assert_eq!(state.combatants[0].hp, 11);
        assert_eq!(state.combatants[0].resources["second_wind"].current, 0);
        assert!(state.combatants[0].bonus_action_used);
    }

    #[test]
    fn cast_heal_checks_range_and_can_target_self_by_default() {
        let mut healer = spec("Cleric", Side::Party, Kind::Pc, 20, 18, 0, 10);
        healer.heals.push(Heal {
            name: "cure_wounds".to_string(),
            amount: "1d8+3".to_string(),
            range: 5,
            action: HealAction::Action,
            cost: None,
            self_only: false,
            notes: None,
        });
        let ally = spec("Ally", Side::Party, Kind::Pc, 20, 12, 0, 10);
        let mut state = state_with(vec![healer, ally]);
        state.zones.insert("far".to_string(), 10);
        state.combatants[1].zone = "far".to_string();
        state.combatants[1].hp = 10;

        let mut rng = ScriptRng::new(&[]);
        let mut log = Vec::new();
        let error = cast(
            &mut state,
            "Cleric",
            "cure_wounds",
            &["Ally".to_string()],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .expect_err("heal target is out of range");
        assert!(error.to_string().contains("out of range"));

        state.combatants[0].hp = 5;
        let mut rng = ScriptRng::new(&[4]);
        let report = cast(
            &mut state,
            "Cleric",
            "cure_wounds",
            &[],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.heals.len(), 1);
        assert_eq!(report.heals[0].healed, 7);
        assert_eq!(report.heals[0].hp_after, 12);
    }

    #[test]
    fn roll_expression_validates_advantage_and_reports_dc() {
        let mut rng = ScriptRng::new(&[]);
        let mut log = Vec::new();
        let error = roll_expression("2d6", RollMode::Advantage, None, &mut rng, &mut log)
            .expect_err("advantage is only valid for one d20");
        assert!(error.to_string().contains("single d20"));

        let mut rng = ScriptRng::new(&[3, 17]);
        let report =
            roll_expression("1d20+5", RollMode::Advantage, Some(15), &mut rng, &mut log).unwrap();
        assert_eq!(report.total, 22);
        assert_eq!(report.success, Some(true));
        assert!(report.text.contains("PASS"));
    }

    #[test]
    fn movement_updates_zone_and_remaining_speed() {
        let mut state = state_with(vec![spec("Hero", Side::Party, Kind::Pc, 10, 10, 0, 10)]);
        state.zones.insert("hall".to_string(), 20);
        state.zones.insert("far".to_string(), 60);
        let mut log = Vec::new();

        let report = move_to(&mut state, "Hero", "hall", &mut log).unwrap();
        assert_eq!(report.distance, 20);
        assert_eq!(report.speed_left_after, 10);
        assert_eq!(state.combatants[0].zone, "hall");

        let report = move_to(&mut state, "Hero", "far", &mut log).unwrap();
        assert_eq!(report.distance, 40);
        assert_eq!(report.speed_left_before, 10);
        assert_eq!(report.speed_left_after, 0);
        assert_eq!(state.combatants[0].zone, "far");

        let error = move_to(&mut state, "Hero", "nowhere", &mut log)
            .expect_err("unknown zones should be rejected");
        assert!(error.to_string().contains("nowhere"));
    }

    #[test]
    fn next_turn_auto_rolls_a_death_save_for_a_downed_pc() {
        let monster = spec("Guard", Side::Enemy, Kind::Monster, 11, 16, 0, 10);
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 12, 15, 0, 10);
        hero.hp = 0;
        let mut state = state_with(vec![monster, hero]);
        state.combatants[1].hp = 0;
        state.combatants[1].conditions.push(ActiveCondition {
            condition: "unconscious".to_string(),
            source: None,
            escape_dc: None,
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });
        state.initiative = vec![0, 1];
        state.round = 1;
        state.turn = 0;

        let mut rng = ScriptRng::new(&[20]);
        let mut log = Vec::new();
        let report = next_turn(&mut state, &mut rng, &mut log).unwrap();
        let death_save = report.death_save.expect("downed PC should roll");
        assert!(death_save.revived);
        assert_eq!(report.active, "Hero");
        assert_eq!(state.combatants[1].hp, 1);
    }

    #[test]
    fn sneak_attack_needs_advantage_or_a_nearby_ally() {
        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let ally = spec("Ally", Side::Party, Kind::Pc, 10, 10, 0, 10);
        let mut state = state_with(vec![sneak_attacker(), target, ally]);

        let mut rng = ScriptRng::new(&[15, 3, 4]);
        let report = run_attack(
            &mut state,
            "Rogue",
            "Guard",
            "dagger",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.sneak);
        assert_eq!(report.total_damage, 10);

        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let mut state = state_with(vec![sneak_attacker(), target]);
        let mut rng = ScriptRng::new(&[15, 3]);
        let report = run_attack(
            &mut state,
            "Rogue",
            "Guard",
            "dagger",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(!report.sneak);
        assert_eq!(report.total_damage, 6);
    }

    #[test]
    fn cast_with_multiple_targets_distributes_count() {
        let mut caster = spec("Mage", Side::Party, Kind::Pc, 20, 12, 0, 10);
        caster.attacks.push(Attack {
            name: "magic_missile".to_string(),
            kind: AttackKind::Auto,
            to_hit: None,
            damage: Some("1d4+1".to_string()),
            damage_type: Some("force".to_string()),
            extra_damage: Vec::new(),
            reach: None,
            range: Some([120, 120]),
            mastery: None,
            sneak_eligible: false,
            count: 3,
            cost: Some(Cost {
                resource: "slot1".to_string(),
                amount: 1,
                notes: None,
            }),
            save: None,
            on_hit_conditions: Vec::new(),
            on_fail_conditions: Vec::new(),
            vs_condition: None,
            notes: None,
        });
        caster.resources.insert(
            "slot1".to_string(),
            crate::model::Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let guards: Vec<CombatantSpec> = (1..=3)
            .map(|index| {
                spec(
                    &format!("Guard {index}"),
                    Side::Enemy,
                    Kind::Monster,
                    20,
                    10,
                    0,
                    10,
                )
            })
            .collect();
        let mut combatants = vec![caster];
        combatants.extend(guards);
        let mut state = state_with(combatants);

        let mut rng = ScriptRng::new(&[3, 2, 4]);
        let mut log = Vec::new();
        let report = cast(
            &mut state,
            "Mage",
            "magic_missile",
            &[
                "Guard 1".to_string(),
                "Guard 2".to_string(),
                "Guard 3".to_string(),
            ],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.attacks.len(), 3);
        assert_eq!(state.combatants[0].resources["slot1"].current, 0);
        assert_eq!(state.combatants[1].hp, 16);
        assert_eq!(state.combatants[2].hp, 17);
        assert_eq!(state.combatants[3].hp, 15);
    }
    #[test]
    fn real_data_attacks_casts_and_saves_resolve() {
        let mut state = EncounterState::from_encounter_file(repo_path(
            "data/encounters/party-vs-2-guards.json",
        ))
        .unwrap();
        let mut log = Vec::new();
        move_to(&mut state, "Guard 1", "front", &mut log).unwrap();

        let mut rng = ScriptRng::new(&[15, 5]);
        let report = run_attack(
            &mut state,
            "Will",
            "Guard 1",
            "longsword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.hit);
        assert_eq!(state.get("Guard 1").unwrap().hp, 2);

        let mut rng = ScriptRng::new(&[15, 4]);
        let report = run_attack(
            &mut state,
            "Will",
            "Guard 1",
            "shocking_grasp",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.hit);
        assert!(state.get("Guard 1").unwrap().death_saves.dead);

        let mut rng = ScriptRng::new(&[2, 3, 1]);
        let report = cast(
            &mut state,
            "Milo",
            "magic_missile",
            &["Guard 2".to_string()],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.attacks.len(), 1);
        assert_eq!(report.attacks[0].total_damage, 9);
        assert_eq!(state.get("Milo").unwrap().resources["slot1"].current, 1);
        assert_eq!(state.get("Guard 2").unwrap().hp, 2);

        let mut rng = ScriptRng::new(&[2, 7]);
        let report = cast(
            &mut state,
            "Sera",
            "sacred_flame",
            &["Guard 2".to_string()],
            &AttackOptions::default(),
            &mut rng,
            &mut log,
        )
        .unwrap();
        assert_eq!(report.attacks.len(), 1);
        assert!(report.attacks[0]
            .save
            .as_ref()
            .is_some_and(|save| !save.success));
        assert!(state.get("Guard 2").unwrap().death_saves.dead);
        assert_eq!(combat_outcome(&state), Some(CombatOutcome::PartyWins));
    }
    #[test]
    fn on_hit_conditions_record_source_and_expiry() {
        let mut attacker = spec("Mimic", Side::Enemy, Kind::Monster, 30, 12, 0, 10);
        let mut bite = make_attack("bite", 5, "1d8+3", "piercing");
        bite.on_hit_conditions.push(ConditionApplication {
            condition: "grappled".to_string(),
            escape_dc: Some(13),
            duration: Some(DurationSpec {
                until: Some("end_of_source_next_turn".to_string()),
                rounds: None,
                notes: None,
            }),
            notes: None,
        });
        attacker.attacks.push(bite);
        let target = spec("Hero", Side::Party, Kind::Pc, 30, 10, 0, 10);
        let mut state = state_with(vec![attacker, target]);
        state.initiative = vec![0, 1];
        state.round = 1;
        state.turn = 0;

        let mut rng = ScriptRng::new(&[15, 3]);
        run_attack(
            &mut state,
            "Mimic",
            "Hero",
            "bite",
            &AttackOptions::default(),
            &mut rng,
        );
        let condition = state.combatants[1]
            .conditions
            .iter()
            .find(|condition| condition.condition == "grappled")
            .expect("grappled condition");
        assert_eq!(condition.source.as_deref(), Some("Mimic"));
        assert_eq!(condition.escape_dc, Some(13));
        assert_eq!(condition.ends_with_turn_of.as_deref(), Some("Mimic"));
        assert_eq!(condition.expires_round, Some(2));
    }

    #[test]
    fn vs_condition_replaces_base_damage_and_adds_advantage() {
        let mut attacker = spec("Mimic", Side::Enemy, Kind::Monster, 30, 12, 0, 10);
        let mut bite = make_attack("bite", 5, "1d8+3", "piercing");
        bite.vs_condition = Some(crate::model::VsCondition {
            condition: "grappled".to_string(),
            advantage: true,
            damage: Some("2d8+3".to_string()),
            notes: None,
        });
        attacker.attacks.push(bite);
        let target = spec("Hero", Side::Party, Kind::Pc, 30, 10, 0, 10);
        let mut state = state_with(vec![attacker, target]);
        state.combatants[1].conditions.push(ActiveCondition {
            condition: "grappled".to_string(),
            source: Some("Mimic".to_string()),
            escape_dc: Some(13),
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });

        let mut rng = ScriptRng::new(&[15, 10, 3, 6]);
        let report = run_attack(
            &mut state,
            "Mimic",
            "Hero",
            "bite",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(
            report.attack_roll.as_ref().map(|roll| roll.mode),
            Some(RollMode::Advantage)
        );
        assert_eq!(
            report.damage.as_ref().map(|damage| damage.requested),
            Some(12)
        );
    }

    #[test]
    fn vex_grants_one_advantage_attack_against_its_target() {
        let mut attacker = spec("Fighter", Side::Party, Kind::Pc, 30, 15, 0, 10);
        let mut sword = make_attack("sword", 5, "1d8+4", "slashing");
        sword.mastery = Some(Mastery::Vex);
        attacker.attacks.push(sword);
        let target = spec("Guard", Side::Enemy, Kind::Monster, 60, 10, 0, 10);
        let mut state = state_with(vec![attacker, target]);

        let mut rng = ScriptRng::new(&[15, 4]);
        run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(state.combatants[0].has_marker(MarkerKind::Vex));

        // Drop the mastery so the second hit does not immediately reapply Vex.
        state.combatants[0].spec.attacks[0].mastery = None;
        let mut rng = ScriptRng::new(&[15, 10, 4]);
        let report = run_attack(
            &mut state,
            "Fighter",
            "Guard",
            "sword",
            &AttackOptions::default(),
            &mut rng,
        );
        assert_eq!(
            report.attack_roll.as_ref().map(|roll| roll.mode),
            Some(RollMode::Advantage)
        );
        assert!(!state.combatants[0].has_marker(MarkerKind::Vex));
    }

    #[test]
    fn save_attacks_apply_half_damage_and_failed_save_conditions() {
        let mut caster = spec("Caster", Side::Party, Kind::Pc, 20, 12, 0, 10);
        caster.attacks.push(Attack {
            name: "flame".to_string(),
            kind: AttackKind::Save,
            to_hit: None,
            damage: Some("2d6".to_string()),
            damage_type: Some("fire".to_string()),
            extra_damage: Vec::new(),
            reach: None,
            range: Some([60, 60]),
            mastery: None,
            sneak_eligible: false,
            count: 1,
            cost: None,
            save: Some(SaveSpec {
                ability: "dex".to_string(),
                dc: 13,
                half_on_success: true,
                notes: None,
            }),
            on_hit_conditions: Vec::new(),
            on_fail_conditions: vec![ConditionApplication {
                condition: "prone".to_string(),
                escape_dc: None,
                duration: None,
                notes: None,
            }],
            vs_condition: None,
            notes: None,
        });
        let target = spec("Guard", Side::Enemy, Kind::Monster, 20, 10, 0, 10);
        let mut state = state_with(vec![caster, target]);
        state.zones.insert("far".to_string(), 10);
        state.combatants[1].zone = "far".to_string();

        let mut rng = ScriptRng::new(&[15, 4, 2]);
        let report = run_attack(
            &mut state,
            "Caster",
            "Guard",
            "flame",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.save.as_ref().is_some_and(|save| save.success));
        assert_eq!(report.total_damage, 3);
        assert!(!state.combatants[1].has_condition("prone"));

        let mut rng = ScriptRng::new(&[2, 4, 2]);
        let report = run_attack(
            &mut state,
            "Caster",
            "Guard",
            "flame",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.save.as_ref().is_some_and(|save| !save.success));
        assert_eq!(report.total_damage, 6);
        assert!(state.combatants[1].has_condition("prone"));
    }
    #[test]
    fn same_type_components_are_mitigated_after_grouping() {
        let mut rogue = spec("Rogue", Side::Party, Kind::Pc, 20, 14, 0, 16);
        let mut dagger = make_attack("dagger", 5, "5", "slashing");
        dagger.sneak_eligible = true;
        rogue.attacks.push(dagger);
        rogue
            .features
            .insert("sneak_attack".to_string(), "3".to_string());
        let mut target = spec("Slime", Side::Enemy, Kind::Monster, 30, 10, 0, 10);
        target.resistances.push("slashing".to_string());
        let mut state = state_with(vec![rogue, target]);

        let options = AttackOptions {
            advantage: true,
            ..AttackOptions::default()
        };
        let mut rng = ScriptRng::new(&[15, 10]);
        let report = run_attack(&mut state, "Rogue", "Slime", "dagger", &options, &mut rng);

        assert!(report.sneak);
        let damage = report.damage.expect("attack dealt damage");
        assert_eq!(damage.requested, 8);
        assert_eq!(damage.amount, 4);
        assert_eq!(damage.components.len(), 1);
        assert_eq!(damage.components[0].raw, 8);
        assert_eq!(damage.components[0].dealt, 4);
        assert_eq!(state.combatants[1].hp, 26);
    }

    #[test]
    fn half_on_success_groups_same_type_before_resistance() {
        let mut caster = spec("Caster", Side::Party, Kind::Pc, 20, 12, 0, 10);
        caster.attacks.push(Attack {
            name: "flame".to_string(),
            kind: AttackKind::Save,
            to_hit: None,
            damage: Some("5".to_string()),
            damage_type: Some("fire".to_string()),
            extra_damage: vec![ExtraDamage {
                dice: "3".to_string(),
                damage_type: "fire".to_string(),
                notes: None,
            }],
            reach: None,
            range: Some([60, 60]),
            mastery: None,
            sneak_eligible: false,
            count: 1,
            cost: None,
            save: Some(SaveSpec {
                ability: "dex".to_string(),
                dc: 10,
                half_on_success: true,
                notes: None,
            }),
            on_hit_conditions: Vec::new(),
            on_fail_conditions: Vec::new(),
            vs_condition: None,
            notes: None,
        });
        let mut target = spec("Guard", Side::Enemy, Kind::Monster, 20, 10, 0, 10);
        target.resistances.push("fire".to_string());
        let mut state = state_with(vec![caster, target]);
        state.zones.insert("far".to_string(), 10);
        state.combatants[1].zone = "far".to_string();

        let mut rng = ScriptRng::new(&[15]);
        let report = run_attack(
            &mut state,
            "Caster",
            "Guard",
            "flame",
            &AttackOptions::default(),
            &mut rng,
        );

        assert!(report.save.as_ref().is_some_and(|save| save.success));
        let damage = report.damage.expect("half damage on a successful save");
        assert_eq!(damage.requested, 8);
        assert_eq!(damage.amount, 2);
        assert_eq!(damage.components.len(), 1);
        assert_eq!(damage.components[0].raw, 8);
        assert_eq!(damage.components[0].dealt, 2);
        assert_eq!(state.combatants[1].hp, 18);
    }

    #[test]
    fn combat_outcome_ignores_positive_hp_unconscious_creatures() {
        let hero = spec("Hero", Side::Party, Kind::Pc, 10, 15, 0, 10);
        let guard = spec("Guard", Side::Enemy, Kind::Monster, 10, 16, 0, 10);
        let mut state = state_with(vec![hero, guard]);
        assert_eq!(combat_outcome(&state), None);

        state.combatants[0].conditions.push(ActiveCondition {
            condition: "unconscious".to_string(),
            source: None,
            escape_dc: None,
            duration: None,
            expires_round: None,
            ends_with_turn_of: None,
            notes: None,
        });
        assert_eq!(combat_outcome(&state), Some(CombatOutcome::EnemyWins));
    }

    #[test]
    fn sneak_attack_does_not_count_an_unconscious_ally() {
        let target = spec("Guard", Side::Enemy, Kind::Monster, 40, 10, 0, 10);
        let mut ally = spec("Ally", Side::Party, Kind::Pc, 10, 10, 0, 10);
        ally.conditions.push(ConditionApplication {
            condition: "unconscious".to_string(),
            escape_dc: None,
            duration: None,
            notes: None,
        });
        let mut state = state_with(vec![sneak_attacker(), target, ally]);

        let mut rng = ScriptRng::new(&[15, 3]);
        let report = run_attack(
            &mut state,
            "Rogue",
            "Guard",
            "dagger",
            &AttackOptions::default(),
            &mut rng,
        );
        assert!(report.hit);
        assert!(!report.sneak);
        assert_eq!(report.total_damage, 6);
        assert!(!state.combatants[0].sneak_used_this_turn);
    }
}
