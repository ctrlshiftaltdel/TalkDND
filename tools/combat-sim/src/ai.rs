//! Generic, data-driven combat AI.
//!
//! Everything a creature decides is derived from content data: attack/ability
//! roles, ranges, damage/heal effects, condition flags and the optional
//! [`AiParams`] on an AI template or creature assignment. The module contains no
//! campaign, spell, condition or monster names.
//!
//! Plans are built once per simulation from the self-contained state snapshot,
//! then reused for every Monte Carlo clone. A plan holds resolved abilities and
//! pre-evaluated average damage/heal values, so a turn does not parse strings or
//! rebuild static data.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::content::schema::{
    AbilityCost, AbilityName, Activation, AiAssignment, AiParams, AiRole, AiTargeting, Attack,
    AttackEffect, AttackKind, BasicAction, ConditionDef, Effect, ExprValue, RangePreference,
    TargetRef, TargetSide, TargetType,
};
use crate::content::{ContentRegistry, ResolvedAbility};
use crate::dice::DiceRng;
use crate::engine::{self, AttackOptions, LogSink};
use crate::space::{Position, Space};
use crate::state::EncounterState;

/// Default ratio for costed damage abilities when a template omits it.
const DEFAULT_RESOURCE_THRESHOLD: f64 = 1.2;
const NEGATIVE: f64 = -1.0e9;

/// Which decision procedure a caller opts into.
///
/// The legacy heuristic remains the default until P8 (quality gate) decides
/// whether the utility planner can become the default. Keeping the switch in
/// `ai.rs` lets benchmarks compare both without touching the save schema.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AiStrategy {
    /// The pre-P6 heuristic. This is also the execution-failure fallback.
    #[default]
    LegacyHeuristic,
    /// Structured candidate generation and utility scoring.
    Utility,
}

/// Prepared AI plans for every combatant of one encounter template.
pub struct Ai {
    plans: Vec<CombatantPlan>,
    strategy: AiStrategy,
}

#[derive(Clone)]
struct CombatantPlan {
    params: AiParams,
    /// Action- and bonus-action abilities/attacks.
    actions: Vec<PlannedAction>,
    /// Entries of `multiattack`, resolved against the same action list.
    multiattack: Vec<PlannedAction>,
    /// Best raw damage from a free action, used by `resource_threshold`.
    best_at_will_damage: f64,
    /// Rough offensive threat, used by `targeting: highest_damage`.
    threat: f64,
    monster: bool,
    opening: Vec<String>,
}

#[derive(Clone)]
struct PlannedAction {
    id: String,
    kind: PlannedKind,
    role: AiRole,
    activation: Activation,
    cost: Option<AbilityCost>,
    range: Option<i32>,
    target_type: TargetType,
    target_side: Option<TargetSide>,
    target_count: u32,
    area_radius: i32,
    /// Effects for ability actions.
    effects: Vec<PlanEffect>,
    /// Shorthand-attack base damage.
    damage_parts: Vec<PlanDamage>,
    /// Shorthand-attack extra effects.
    on_hit: Vec<PlanEffect>,
    on_miss: Vec<PlanEffect>,
    /// Attack reach/range for shorthand attacks.
    reach: i32,
    attack_range: Option<[i32; 2]>,
    attack_kind: Option<AttackKind>,
    to_hit: i32,
    tags: Vec<String>,
    slot_level: Option<i32>,
    raw_damage: f64,
}

#[derive(Clone)]
enum PlannedKind {
    Attack(Box<Attack>),
    Ability(Box<ResolvedAbility>),
}

#[derive(Clone)]
enum PlanEffect {
    Attack {
        effect: Box<AttackEffect>,
        damages: Vec<PlanDamage>,
        on_hit: Vec<PlanEffect>,
        on_miss: Vec<PlanEffect>,
        tags: Vec<String>,
        params: BTreeMap<String, ExprValue>,
        slot_level: Option<i32>,
        range: Option<i32>,
        to_hit: i32,
    },
    Save {
        ability: AbilityName,
        dc: i32,
        half: bool,
        ignore_cover: bool,
        on_fail: Vec<PlanEffect>,
        on_success: Vec<PlanEffect>,
    },
    Damage(PlanDamage),
    Heal(f64),
    Condition {
        condition: String,
        on_self: bool,
        value: f64,
    },
    Choice(Vec<Vec<PlanEffect>>),
    BasicAction(BasicAction),
    Other,
}

#[derive(Clone, Copy, Default)]
struct PlanDamage {
    average: f64,
    dice_average: f64,
    repeat: u32,
    dtype: DType,
}

/// A tiny string storage for a damage type without allocating per score.
#[derive(Clone, Copy, Default)]
struct DType {
    len: usize,
    bytes: [u8; 16],
}

impl DType {
    fn new(text: &str) -> Self {
        let mut bytes = [0u8; 16];
        let mut len = 0;
        for byte in text.as_bytes().iter().take(16) {
            bytes[len] = byte.to_ascii_lowercase();
            len += 1;
        }
        Self { len, bytes }
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Default)]
struct Score {
    damage: f64,
    heal: f64,
    control: f64,
    buff: f64,
    defense: f64,
}

impl Score {
    fn total(&self) -> f64 {
        self.damage + self.heal + self.control + self.buff + self.defense
    }

    fn add(self, other: Score) -> Score {
        Score {
            damage: self.damage + other.damage,
            heal: self.heal + other.heal,
            control: self.control + other.control,
            buff: self.buff + other.buff,
            defense: self.defense + other.defense,
        }
    }

    fn scaled(self, factor: f64) -> Score {
        Score {
            damage: self.damage * factor,
            heal: self.heal * factor,
            control: self.control * factor,
            buff: self.buff * factor,
            defense: self.defense * factor,
        }
    }
}

#[derive(Clone, Copy)]
enum TargetChoice {
    Action(usize),
    Multiattack,
}

impl Ai {
    /// Builds one plan per combatant from the encounter template.
    pub fn prepare(state: &EncounterState) -> Self {
        Self::prepare_with(state, AiStrategy::LegacyHeuristic)
    }

    /// Builds plans and selects the decision procedure explicitly.
    pub fn prepare_with(state: &EncounterState, strategy: AiStrategy) -> Self {
        let plans = state
            .combatants
            .iter()
            .enumerate()
            .map(|(index, _)| build_plan(state, index))
            .collect();
        Self { plans, strategy }
    }

    /// The strategy this prepared AI uses.
    pub fn strategy(&self) -> AiStrategy {
        self.strategy
    }

    /// Plays the current combatant's turn through the generic engine.
    pub fn take_turn(
        &self,
        state: &mut EncounterState,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        if engine::combat_outcome(state).is_some() {
            return Ok(());
        }
        let Some(actor) = state.current_index() else {
            return Ok(());
        };
        if actor >= self.plans.len() {
            return Ok(());
        }
        {
            let combatant = &state.combatants[actor];
            if combatant.dormant || combatant.is_dead() || combatant.hp <= 0 {
                return Ok(());
            }
            // `start_turn` already rolled the automatic death save; a still
            // dying creature has no actions to take.
            if combatant.is_dying() {
                return Ok(());
            }
            if engine::has_condition_flag(
                state,
                actor,
                crate::content::schema::ConditionFlag::Incapacitated,
            ) {
                return Ok(());
            }
        }
        let plan = &self.plans[actor];
        if self.strategy == AiStrategy::Utility {
            match tactics::take_utility_turn(self, state, actor, plan, rng, log) {
                Ok(true) => {
                    state.combatants[actor].opening_used = true;
                    return Ok(());
                }
                Ok(false) => {
                    log.log(format_args!(
                        "tactical planner found no executable candidate for {}; using heuristic fallback",
                        state.combatants[actor].name
                    ));
                }
                Err(error) => {
                    log.log(format_args!(
                        "tactical plan failed for {}: {error:#}; falling back to heuristic",
                        state.combatants[actor].name
                    ));
                }
            }
        }
        let hp_pct = hp_percent(&state.combatants[actor]);
        let flee_below = plan.params.flee_below_pct.unwrap_or(0);
        if flee_below > 0 && hp_pct <= flee_below {
            self.flee(state, actor, plan, rng, log)?;
            state.combatants[actor].opening_used = true;
            return Ok(());
        }

        self.maybe_self_heal(state, actor, plan, rng, log)?;
        self.maybe_heal_ally(state, actor, plan, rng, log)?;
        self.maybe_opening(state, actor, plan, rng, log)?;

        if !state.combatants[actor].action_used {
            let target = { self.choose_target(state, actor, plan, rng) };
            if let Some(target) = target {
                {
                    self.move_for_action(state, actor, plan, target, rng, log)?;
                }
                {
                    self.take_action(state, actor, plan, target, rng, log)?;
                }
            }
        }
        state.combatants[actor].opening_used = true;
        Ok(())
    }

    fn maybe_self_heal(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        if state.combatants[actor].bonus_action_used {
            return Ok(());
        }
        let threshold = plan.params.self_heal_below_pct.unwrap_or(0);
        if threshold <= 0 || hp_percent(&state.combatants[actor]) > threshold {
            return Ok(());
        }
        let mut best: Option<(f64, usize)> = None;
        for (index, action) in plan.actions.iter().enumerate() {
            if action.activation != Activation::Bonus || action.role != AiRole::SelfHeal {
                continue;
            }
            if !affordable(state, actor, action) {
                continue;
            }
            let heal = self
                .score_effects(state, actor, actor, &action.effects)
                .heal;
            if heal > best.map(|(value, _)| value).unwrap_or(0.0) {
                best = Some((heal, index));
            }
        }
        if let Some((_, index)) = best {
            let action = &plan.actions[index];
            self.execute(state, actor, action, actor, None, rng, log)?;
        }
        Ok(())
    }

    fn maybe_heal_ally(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        if state.combatants[actor].action_used {
            return Ok(());
        }
        let threshold = plan.params.heal_ally_below_pct.unwrap_or(0);
        let heal_downed = plan.params.heal_downed.unwrap_or(false);
        if threshold <= 0 && !heal_downed {
            return Ok(());
        }
        let mut wounded: Vec<usize> = (0..state.combatants.len())
            .filter(|&index| {
                index != actor
                    && state.combatants[index].side == state.combatants[actor].side
                    && !state.combatants[index].dormant
                    && !state.combatants[index].is_dead()
            })
            .filter(|&index| {
                let combatant = &state.combatants[index];
                (heal_downed && combatant.hp <= 0)
                    || (combatant.hp > 0 && hp_percent(combatant) <= threshold)
            })
            .collect();
        wounded.sort_by_key(|&index| state.combatants[index].hp);
        let heal_actions: Vec<usize> = plan
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                action.activation == Activation::Action && action.role == AiRole::Heal
            })
            .map(|(index, _)| index)
            .collect();
        for ally in wounded {
            for &action_index in &heal_actions {
                let action = &plan.actions[action_index];
                if !affordable(state, actor, action) || !action_in_range(state, actor, action, ally)
                {
                    continue;
                }
                self.execute(state, actor, action, ally, None, rng, log)?;
                return Ok(());
            }
        }
        Ok(())
    }

    fn maybe_opening(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        if state.combatants[actor].opening_used || plan.opening.is_empty() {
            return Ok(());
        }
        for id in &plan.opening {
            let Some(action) = plan.actions.iter().find(|action| action.id == *id) else {
                continue;
            };
            if action.activation == Activation::Action && state.combatants[actor].action_used {
                continue;
            }
            if action.activation == Activation::Bonus && state.combatants[actor].bonus_action_used {
                continue;
            }
            if !affordable(state, actor, action) {
                continue;
            }
            let target = if action.target_type == TargetType::SelfTarget
                || action.target_side == Some(TargetSide::Ally)
            {
                actor
            } else {
                let Some(target) = self.choose_target(state, actor, plan, rng) else {
                    continue;
                };
                target
            };
            if action.target_type == TargetType::Creature
                && !action_in_range(state, actor, action, target)
            {
                continue;
            }
            let used_action = action.activation == Activation::Action;
            let used_bonus = action.activation == Activation::Bonus;
            self.execute(state, actor, action, target, None, rng, log)?;
            if used_action || used_bonus {
                break;
            }
        }
        Ok(())
    }

    fn choose_target(
        &self,
        state: &EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        rng: &mut dyn DiceRng,
    ) -> Option<usize> {
        let attack_downed = plan.params.attack_downed.unwrap_or(false) && plan.monster;
        let avoid = plan.params.avoid_targets_with.clone().unwrap_or_default();
        let build = |include_avoided: bool| -> Vec<usize> {
            (0..state.combatants.len())
                .filter(|&index| {
                    index != actor
                        && state.combatants[index].side != state.combatants[actor].side
                        && !state.combatants[index].dormant
                        && !state.combatants[index].is_dead()
                        && (state.combatants[index].hp > 0 || attack_downed)
                        && (include_avoided
                            || !avoid.iter().any(|condition| {
                                state.combatants[index].has_condition(&state.content, condition)
                            }))
                })
                .collect()
        };
        let mut candidates = build(false);
        if candidates.is_empty() {
            candidates = build(true);
        }
        if candidates.is_empty() {
            return None;
        }
        let space = state.space();
        match plan.params.targeting.unwrap_or(AiTargeting::Nearest) {
            AiTargeting::Random => {
                let roll = rng.roll_die(candidates.len() as u32) as usize;
                candidates.get(roll.saturating_sub(1)).copied()
            }
            AiTargeting::LowestHp => candidates.into_iter().min_by_key(|&index| {
                (
                    state.combatants[index].hp,
                    distance(space.as_ref(), state, actor, index),
                    index,
                )
            }),
            AiTargeting::HighestDamage => candidates.into_iter().max_by(|&a, &b| {
                let ta = self.plans.get(a).map(|plan| plan.threat).unwrap_or(0.0);
                let tb = self.plans.get(b).map(|plan| plan.threat).unwrap_or(0.0);
                ta.partial_cmp(&tb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        distance(space.as_ref(), state, b, actor).cmp(&distance(
                            space.as_ref(),
                            state,
                            a,
                            actor,
                        ))
                    })
            }),
            AiTargeting::Nearest => candidates.into_iter().min_by_key(|&index| {
                (
                    distance(space.as_ref(), state, actor, index),
                    state.combatants[index].hp,
                    index,
                )
            }),
        }
    }

    fn take_action(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        target: usize,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        if state.combatants[actor].action_used
            || state.combatants[actor].hp <= 0
            || state.combatants[actor].is_dead()
        {
            return Ok(());
        }
        let choice = { self.choose_action(state, actor, plan, target) };
        match choice {
            Some(TargetChoice::Action(index)) => {
                let action = &plan.actions[index];
                self.execute(state, actor, action, target, None, rng, log)
            }
            Some(TargetChoice::Multiattack) => engine::attack_at(
                state,
                actor,
                target,
                "",
                &AttackOptions {
                    multi: true,
                    ..AttackOptions::default()
                },
                rng,
                log,
            ),
            None => Ok(()),
        }
    }

    fn choose_action(
        &self,
        state: &EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        target: usize,
    ) -> Option<TargetChoice> {
        let mut best: Option<(f64, TargetChoice)> = None;
        for (index, action) in plan.actions.iter().enumerate() {
            if action.activation != Activation::Action
                || !action_in_range(state, actor, action, target)
            {
                continue;
            }
            if !affordable(state, actor, action) {
                continue;
            }
            if !self.resource_allows(state, plan, action) {
                continue;
            }
            let score = self.score_action(state, actor, action, target);
            if score > 0.0 && score > best.map(|(value, _)| value).unwrap_or(NEGATIVE) {
                best = Some((score, TargetChoice::Action(index)));
            }
        }
        if !plan.multiattack.is_empty()
            && plan
                .multiattack
                .iter()
                .all(|action| action_in_range(state, actor, action, target))
        {
            let score: f64 = plan
                .multiattack
                .iter()
                .map(|action| self.score_action(state, actor, action, target))
                .sum();
            if score > 0.0 && score > best.map(|(value, _)| value).unwrap_or(NEGATIVE) {
                best = Some((score, TargetChoice::Multiattack));
            }
        }
        best.map(|(_, choice)| choice)
    }

    fn resource_allows(
        &self,
        state: &EncounterState,
        plan: &CombatantPlan,
        action: &PlannedAction,
    ) -> bool {
        if action.cost.is_none() || action.role != AiRole::Damage {
            return true;
        }
        let threshold = plan
            .params
            .resource_threshold
            .unwrap_or(DEFAULT_RESOURCE_THRESHOLD);
        if plan.best_at_will_damage <= 0.0 {
            return true;
        }
        let _ = state;
        action.raw_damage + 1.0e-9 >= threshold * plan.best_at_will_damage
    }

    /// Raw, role-adjusted-free outcome of one action against one target.
    ///
    /// This is shared by the legacy score and the utility terms so both use
    /// the same damage/heal/control estimates.
    fn action_outcome(
        &self,
        state: &EncounterState,
        actor: usize,
        action: &PlannedAction,
        target: usize,
    ) -> Score {
        let mut score = match &action.kind {
            PlannedKind::Attack(attack) => {
                let forecast = if engine::combatant_has_modifier_sources(state, actor)
                    || engine::combatant_has_modifier_sources(state, target)
                {
                    engine::forecast_attack(state, actor, target, attack)
                } else {
                    engine::simple_attack_forecast(
                        state,
                        actor,
                        target,
                        action.to_hit,
                        attack.kind,
                        action.reach,
                        attack.range,
                    )
                };
                let hit = hit_chance(
                    forecast.to_hit,
                    forecast.ac,
                    forecast.advantage,
                    forecast.disadvantage,
                );
                let mut damage = 0.0;
                for part in &action.damage_parts {
                    let multiplier = engine::damage_multiplier(
                        state,
                        target,
                        Some(actor),
                        part.dtype.as_str(),
                        &action.tags,
                    );
                    damage +=
                        (part.average + part.dice_average * forecast.crit_chance) * multiplier;
                }
                let on_hit = self.score_effects(state, actor, target, &action.on_hit);
                let on_miss = self.score_effects(state, actor, target, &action.on_miss);
                Score {
                    damage: damage * hit + on_hit.damage * hit + on_miss.damage * (1.0 - hit),
                    heal: on_hit.heal * hit,
                    control: on_hit.control * hit,
                    buff: on_hit.buff * hit,
                    defense: on_hit.defense * hit,
                }
            }
            PlannedKind::Ability(_) => {
                let effect_target = if matches!(action.role, AiRole::Buff | AiRole::Defense) {
                    actor
                } else {
                    target
                };
                self.score_effects(state, actor, effect_target, &action.effects)
            }
        };
        if action.target_type == TargetType::Zone {
            let count = self.area_enemy_count(state, actor, action, target);
            if count > 1 {
                score.damage *= count as f64;
                score.control *= count as f64;
            }
        }
        score
    }

    fn score_action(
        &self,
        state: &EncounterState,
        actor: usize,
        action: &PlannedAction,
        target: usize,
    ) -> f64 {
        if action.role == AiRole::Heal || action.role == AiRole::SelfHeal {
            return 0.0;
        }
        let mut score = self.action_outcome(state, actor, action, target);
        match action.role {
            AiRole::Damage => {
                score.damage += score.control * 0.35;
                score.damage + score.heal * 0.2
            }
            AiRole::Control => {
                if score.control <= 0.0 {
                    0.0
                } else {
                    let target_value = 3.0
                        + self
                            .plans
                            .get(target)
                            .map(|plan| plan.threat)
                            .unwrap_or(0.0);
                    score.control * target_value + score.damage * 0.35
                }
            }
            AiRole::Buff => (score.buff + score.control).max(0.0),
            AiRole::Defense => {
                let threatened = hp_percent(&state.combatants[actor]) < 60
                    || nearest_hostile_distance(state, actor) <= 5;
                if threatened {
                    1.0 + score.defense + score.control
                } else {
                    (score.defense + score.control) * 0.25
                }
            }
            AiRole::Heal | AiRole::SelfHeal | AiRole::None => 0.0,
        }
    }

    fn score_effects(
        &self,
        state: &EncounterState,
        actor: usize,
        target: usize,
        effects: &[PlanEffect],
    ) -> Score {
        let mut total = Score::default();
        for effect in effects {
            total = total.add(self.score_effect(state, actor, target, effect));
        }
        total
    }

    fn score_effect(
        &self,
        state: &EncounterState,
        actor: usize,
        target: usize,
        effect: &PlanEffect,
    ) -> Score {
        match effect {
            PlanEffect::Attack {
                effect,
                damages,
                on_hit,
                on_miss,
                tags,
                params,
                slot_level,
                range,
                to_hit,
            } => {
                let forecast = if !engine::combatant_has_modifier_sources(state, actor)
                    && !engine::combatant_has_modifier_sources(state, target)
                {
                    engine::simple_attack_forecast(
                        state,
                        actor,
                        target,
                        *to_hit,
                        effect.kind,
                        5,
                        range.map(|range| [range, range]),
                    )
                } else {
                    engine::forecast_attack_effect(
                        state,
                        actor,
                        target,
                        effect,
                        tags,
                        params,
                        *slot_level,
                        *range,
                    )
                };
                let hit = hit_chance(
                    forecast.to_hit,
                    forecast.ac,
                    forecast.advantage,
                    forecast.disadvantage,
                );
                let mut damage = 0.0;
                for part in damages {
                    let multiplier = engine::damage_multiplier(
                        state,
                        target,
                        Some(actor),
                        part.dtype.as_str(),
                        tags,
                    );
                    damage += part.average * multiplier
                        + part.dice_average * multiplier * forecast.crit_chance;
                }
                let hit_score = self.score_effects(state, actor, target, on_hit);
                let miss_score = self.score_effects(state, actor, target, on_miss);
                Score {
                    damage: damage * hit + hit_score.damage * hit + miss_score.damage * (1.0 - hit),
                    heal: hit_score.heal * hit,
                    control: hit_score.control * hit,
                    buff: hit_score.buff * hit,
                    defense: hit_score.defense * hit,
                }
            }
            PlanEffect::Save {
                ability,
                dc,
                half,
                ignore_cover,
                on_fail,
                on_success,
            } => {
                let fail = engine::save_fail_chance(
                    state,
                    target,
                    *ability,
                    *dc,
                    *ignore_cover,
                    Some(actor),
                );
                let fail_score = self.score_effects(state, actor, target, on_fail);
                let mut success_score = self.score_effects(state, actor, target, on_success);
                if *half {
                    success_score.damage += fail_score.damage * 0.5;
                }
                fail_score
                    .scaled(fail)
                    .add(success_score.scaled(1.0 - fail))
            }
            PlanEffect::Damage(part) => {
                let multiplier =
                    engine::damage_multiplier(state, target, Some(actor), part.dtype.as_str(), &[]);
                let repeats = f64::from(part.repeat.max(1));
                Score {
                    damage: part.average * multiplier * repeats,
                    ..Score::default()
                }
            }
            PlanEffect::Heal(amount) => Score {
                heal: *amount,
                ..Score::default()
            },
            PlanEffect::Condition {
                condition,
                on_self,
                value,
            } => {
                if *on_self {
                    Score {
                        buff: *value,
                        ..Score::default()
                    }
                } else if state.combatants[target].has_condition(&state.content, condition) {
                    Score::default()
                } else {
                    Score {
                        control: *value,
                        ..Score::default()
                    }
                }
            }
            PlanEffect::Choice(options) => options
                .iter()
                .map(|option| self.score_effects(state, actor, target, option))
                .max_by(|a, b| {
                    a.total()
                        .partial_cmp(&b.total())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or_default(),
            PlanEffect::BasicAction(_) | PlanEffect::Other => Score::default(),
        }
    }

    fn area_enemy_count(
        &self,
        state: &EncounterState,
        actor: usize,
        action: &PlannedAction,
        target: usize,
    ) -> usize {
        let Some(space) = state.space() else {
            return 1;
        };
        let center = state.combatants[target].position();
        let owned;
        let positions: &[Position] = match space.area_slice(&center, action.area_radius) {
            Some(positions) => positions,
            None => {
                owned = space.area_positions(&center, action.area_radius);
                &owned
            }
        };
        state
            .combatants
            .iter()
            .enumerate()
            .filter(|(index, combatant)| {
                *index != actor
                    && combatant.side != state.combatants[actor].side
                    && !combatant.dormant
                    && !combatant.is_dead()
                    && positions.contains(&combatant.position())
            })
            .count()
            .max(1)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute(
        &self,
        state: &mut EncounterState,
        actor: usize,
        action: &PlannedAction,
        target: usize,
        option: Option<usize>,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        match &action.kind {
            PlannedKind::Attack(attack) => engine::attack_at(
                state,
                actor,
                target,
                &attack.id,
                &AttackOptions::default(),
                rng,
                log,
            ),
            PlannedKind::Ability(resolved) => {
                let targets = self.ability_targets(state, actor, action, target);
                let center = if action.target_type == TargetType::Zone {
                    Some(state.combatants[target].position())
                } else {
                    None
                };
                let option = option.or_else(|| best_option(state, actor, action, target));
                engine::use_planned_ability(
                    state,
                    actor,
                    resolved,
                    &targets,
                    center.as_ref(),
                    option,
                    action.slot_level,
                    rng,
                    log,
                )
            }
        }
    }

    fn ability_targets(
        &self,
        state: &EncounterState,
        actor: usize,
        action: &PlannedAction,
        primary: usize,
    ) -> Vec<usize> {
        if action.target_type == TargetType::SelfTarget {
            return vec![actor];
        }
        if action.target_type == TargetType::Zone {
            return Vec::new();
        }
        let count = action.target_count.max(1) as usize;
        let side = action.target_side.unwrap_or(TargetSide::Any);
        let mut targets = if side == TargetSide::Ally {
            if state.combatants[primary].side == state.combatants[actor].side {
                vec![primary]
            } else {
                vec![actor]
            }
        } else {
            vec![primary]
        };
        if count > 1 {
            let mut others: Vec<usize> = (0..state.combatants.len())
                .filter(|&index| {
                    index != targets[0]
                        && !state.combatants[index].dormant
                        && !state.combatants[index].is_dead()
                })
                .filter(|&index| match side {
                    TargetSide::Enemy => {
                        state.combatants[index].side != state.combatants[actor].side
                    }
                    TargetSide::Ally | TargetSide::Any => {
                        state.combatants[index].side == state.combatants[actor].side
                    }
                })
                .filter(|&index| action_in_range(state, actor, action, index))
                .collect();
            others.sort_by_key(|&index| state.combatants[index].hp);
            targets.extend(others.into_iter().take(count - 1));
        }
        if (action.role == AiRole::Buff || action.role == AiRole::Defense)
            && !targets.contains(&actor)
            && action.target_side == Some(TargetSide::Ally)
        {
            targets.insert(0, actor);
        }
        targets.truncate(count);
        targets
    }

    fn flee(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        let adjacent = nearest_hostile_distance(state, actor) <= 5;
        if !state.combatants[actor].bonus_action_used {
            let wanted = if adjacent {
                BasicAction::Disengage
            } else {
                BasicAction::Dash
            };
            if self.use_bonus_basic(state, actor, plan, wanted, rng, log)? && !adjacent {
                // Dash increased the movement budget; movement below sees it.
            } else if adjacent
                && self.use_bonus_basic(state, actor, plan, BasicAction::Dash, rng, log)?
            {
                // Disengage was unavailable; dash is still better than nothing.
            }
        }
        self.advance(state, actor, plan, None, 5, true, rng, log)
    }

    fn move_for_action(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        target: usize,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        let best = { self.best_action_for_position(state, actor, plan, target) };
        let Some(action) = best else {
            return self.advance(state, actor, plan, Some(target), 5, false, rng, log);
        };
        let desired = action_distance(action);
        let wants_ranged = plan.params.range_pref == Some(RangePreference::Ranged)
            || (plan.params.range_pref != Some(RangePreference::Melee)
                && action.attack_range.is_some()
                && action.attack_kind != Some(AttackKind::Melee)
                && !action_in_range(state, actor, action, target));
        let keep = plan.params.keep_distance.unwrap_or(0);
        let nearest = { nearest_hostile_distance(state, actor) };
        if keep > 0 && wants_ranged && nearest < keep {
            if nearest <= 5 && !state.combatants[actor].bonus_action_used {
                let _ =
                    self.use_bonus_basic(state, actor, plan, BasicAction::Disengage, rng, log)?;
            }
            return self.advance(state, actor, plan, Some(target), desired, false, rng, log);
        }
        if !action_in_range(state, actor, action, target) {
            return self.advance(state, actor, plan, Some(target), desired, false, rng, log);
        }
        Ok(())
    }

    fn best_action_for_position<'a>(
        &self,
        state: &EncounterState,
        actor: usize,
        plan: &'a CombatantPlan,
        _target: usize,
    ) -> Option<&'a PlannedAction> {
        // Position selection only needs an action's range and rough value.
        // Avoid a full accuracy forecast here; the chosen action is scored
        // once more after movement.
        plan.actions
            .iter()
            .filter(|action| {
                action.activation == Activation::Action
                    && matches!(action.role, AiRole::Damage | AiRole::Control)
                    && affordable(state, actor, action)
                    && self.resource_allows(state, plan, action)
            })
            .max_by(|a, b| {
                let value = |action: &PlannedAction| {
                    action.raw_damage
                        + if action.role == AiRole::Control {
                            0.5
                        } else {
                            0.0
                        }
                };
                value(a)
                    .partial_cmp(&value(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    fn use_bonus_basic(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        wanted: BasicAction,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<bool> {
        if state.combatants[actor].bonus_action_used {
            return Ok(false);
        }
        for action in &plan.actions {
            if action.activation != Activation::Bonus {
                continue;
            }
            if let Some(option) = option_for_basic_action(action, wanted) {
                if affordable(state, actor, action) {
                    self.execute(state, actor, action, actor, Some(option), rng, log)?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Moves according to posted-zone/leash, keep-distance, approach or flee rules.
    #[allow(clippy::too_many_arguments)]
    fn advance(
        &self,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        target: Option<usize>,
        desired_distance: i32,
        fleeing: bool,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        let Some(space) = state.space() else {
            return Ok(());
        };
        let budget = state.combatants[actor].speed_left;
        if budget <= 0 {
            return Ok(());
        }
        let start = state.combatants[actor].position();
        let target_position = target.map(|index| state.combatants[index].position());
        let visible_target = target_position
            .as_ref()
            .and_then(|position| space.distance(&start, position));
        let hostiles: Vec<Position> = state
            .combatants
            .iter()
            .enumerate()
            .filter(|(index, combatant)| {
                *index != actor
                    && combatant.side != state.combatants[actor].side
                    && !combatant.dormant
                    && !combatant.is_dead()
                    && combatant.hp > 0
            })
            .map(|(_, combatant)| combatant.position())
            .collect();
        let blocked: Vec<Position> = state
            .combatants
            .iter()
            .enumerate()
            .filter(|(index, combatant)| *index != actor && !combatant.is_dead())
            .map(|(_, combatant)| combatant.position())
            .collect();
        let start_ref = state.combatants[actor].position_ref();
        let nearest = hostiles
            .iter()
            .filter_map(|position| space.distance(start_ref.as_ref(), position))
            .min()
            .unwrap_or(9999);

        let guard_zone = plan.params.guard_zone.clone();
        let leash = plan.params.leash.unwrap_or(i32::MAX);
        let in_guard = guard_zone
            .as_deref()
            .is_none_or(|zone| space.zone_of(&start).as_deref() == Some(zone));
        if let Some(zone) = guard_zone.as_deref() {
            if !in_guard && nearest > leash {
                return self.move_toward_zone(state, actor, zone, rng, log);
            }
            if in_guard && nearest > leash {
                return Ok(());
            }
        }

        let flee_target = plan
            .params
            .flee_zone
            .as_deref()
            .and_then(|zone| zone_position(&space, zone));
        let keep = plan.params.keep_distance.unwrap_or(0);
        let range_pref = plan.params.range_pref;
        let owned_positions;
        let positions: &[Position] = match space.positions_slice() {
            Some(positions) => positions,
            None => {
                owned_positions = space.all_positions();
                &owned_positions
            }
        };
        let mut candidates: Vec<(i32, Position)> = Vec::new();
        for position in positions {
            if *position == start {
                candidates.push((0, position.clone()));
                continue;
            }
            if blocked.contains(position) {
                continue;
            }
            let Some(cost) = space.route_cost(&start, position, &blocked) else {
                continue;
            };
            if cost <= budget {
                candidates.push((cost, position.clone()));
            }
        }
        if candidates.is_empty() {
            return Ok(());
        }
        let mut best: Option<(f64, i32, Position)> = None;
        for (cost, position) in candidates {
            if !fleeing {
                if let Some(post) = guard_zone.as_deref() {
                    if nearest <= leash && !in_guard {
                        // Chasing is allowed, but do not wander away from the
                        // posted zone beyond the leash.
                    } else if space.zone_tag(&position) != Some(post) {
                        continue;
                    }
                }
            }
            let min_enemy = hostiles
                .iter()
                .filter_map(|enemy| space.distance(&position, enemy))
                .min()
                .unwrap_or(9999);
            let distance_target = target_position
                .as_ref()
                .and_then(|target| space.distance(&position, target))
                .unwrap_or(visible_target.unwrap_or(9999));
            let score = if fleeing {
                let flee_bonus = flee_target
                    .as_ref()
                    .and_then(|zone| space.distance(&position, zone))
                    .map(|distance| -f64::from(distance) * 0.4)
                    .unwrap_or(0.0);
                f64::from(min_enemy) * 10.0
                    + flee_bonus
                    + if position == start { -1.0 } else { 0.0 }
            } else if let Some(zone) = guard_zone.as_deref() {
                if !in_guard && nearest > leash {
                    let zone_distance = zone_position(&space, zone)
                        .and_then(|zone| space.distance(&position, &zone))
                        .unwrap_or(9999);
                    -f64::from(zone_distance) * 10.0 - f64::from(cost) * 0.01
                } else if target_position.is_some() {
                    approach_score(
                        keep,
                        range_pref,
                        min_enemy,
                        distance_target,
                        desired_distance,
                    )
                } else {
                    0.0
                }
            } else if target_position.is_some() {
                approach_score(
                    keep,
                    range_pref,
                    min_enemy,
                    distance_target,
                    desired_distance,
                )
            } else {
                -f64::from(cost) * 0.01
            };
            if best.as_ref().is_none_or(|(value, _, _)| score > *value) {
                best = Some((score, cost, position));
            }
        }
        let Some((_, _, destination)) = best else {
            return Ok(());
        };
        if destination == start {
            return Ok(());
        }
        let move_target = match &destination {
            Position::Cell { x, y } => engine::MoveTarget::Cell(*x, *y),
            Position::Zone { zone } => engine::MoveTarget::Zone(zone.clone()),
        };
        let _ = rng;
        engine::move_to_at(state, actor, &move_target, false, rng, log)
    }

    fn move_toward_zone(
        &self,
        state: &mut EncounterState,
        actor: usize,
        zone: &str,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        let Some(space) = state.space() else {
            return Ok(());
        };
        let start = state.combatants[actor].position();
        let budget = state.combatants[actor].speed_left;
        let blocked: Vec<Position> = state
            .combatants
            .iter()
            .enumerate()
            .filter(|(index, combatant)| *index != actor && !combatant.is_dead())
            .map(|(_, combatant)| combatant.position())
            .collect();
        let mut best: Option<(i32, Position)> = None;
        for position in space.zone_positions(zone) {
            if position == start || blocked.contains(&position) {
                continue;
            }
            let Some(cost) = space.route_cost(&start, &position, &blocked) else {
                continue;
            };
            if cost > budget {
                continue;
            }
            if best.as_ref().is_none_or(|(best_cost, _)| cost < *best_cost) {
                best = Some((cost, position));
            }
        }
        let Some((_, destination)) = best else {
            return Ok(());
        };
        let move_target = match &destination {
            Position::Cell { x, y } => engine::MoveTarget::Cell(*x, *y),
            Position::Zone { zone } => engine::MoveTarget::Zone(zone.clone()),
        };
        engine::move_to_at(state, actor, &move_target, false, rng, log)
    }
}

fn approach_score(
    keep: i32,
    range_pref: Option<RangePreference>,
    min_enemy: i32,
    distance_target: i32,
    desired_distance: i32,
) -> f64 {
    if keep > 0 && matches!(range_pref, Some(RangePreference::Ranged)) && min_enemy < keep {
        return f64::from(min_enemy.min(keep + 10)) * 10.0 - f64::from(distance_target) * 0.05;
    }
    if desired_distance > 5 {
        if distance_target > desired_distance {
            -f64::from(distance_target - desired_distance) * 10.0 - f64::from(min_enemy) * 0.05
        } else if distance_target < keep.max(5) && keep > 0 {
            f64::from(min_enemy.min(keep + 10)) * 10.0 - f64::from(distance_target) * 0.05
        } else {
            1000.0
                - (f64::from(distance_target - desired_distance)).abs()
                - f64::from(min_enemy) * 0.01
        }
    } else if distance_target <= 5 {
        1000.0 - f64::from(min_enemy)
    } else {
        -f64::from(distance_target) * 10.0 - f64::from(min_enemy) * 0.05
    }
}

fn action_distance(action: &PlannedAction) -> i32 {
    match action.attack_kind {
        Some(AttackKind::Melee) => action.reach.max(5),
        Some(AttackKind::Ranged) => action
            .attack_range
            .map(|[normal, _]| normal)
            .unwrap_or(30)
            .max(5),
        Some(AttackKind::MeleeOrRanged) => action
            .attack_range
            .map(|[normal, _]| normal)
            .unwrap_or(action.reach)
            .max(5),
        None => action.range.unwrap_or(5).max(5),
    }
}

fn build_plan(state: &EncounterState, actor: usize) -> CombatantPlan {
    let combatant = &state.combatants[actor];
    let params = merged_params(&state.content, &combatant.ai);
    let tags = combatant.creature.tags.clone().unwrap_or_default();
    let mut actions = Vec::new();
    if let Some(attacks) = &combatant.creature.attacks {
        for attack in attacks {
            actions.push(planned_attack(state, actor, attack));
        }
    }
    let mut entries = Vec::new();
    if let Some(actions_list) = &combatant.creature.actions {
        entries.extend(actions_list.iter());
    }
    if let Some(features) = &combatant.creature.features {
        entries.extend(features.iter());
    }
    for entry in entries {
        let Ok(resolved) = state.content.resolve_ability(entry) else {
            continue;
        };
        let ability = &resolved.ability;
        if matches!(
            ability.activation,
            Activation::Passive | Activation::Reaction
        ) {
            continue;
        }
        let role = ability
            .ai
            .as_ref()
            .map(|ai| ai.role)
            .unwrap_or(AiRole::None);
        let slot_level = ability.slot_level;
        let attack_mod = None;
        let effects = build_effects(
            state,
            actor,
            &ability.effects,
            &resolved.params,
            slot_level,
            attack_mod,
            &ability.tags,
        );
        let raw_damage = raw_effect_damage(&effects);
        actions.push(PlannedAction {
            id: ability.id.clone(),
            kind: PlannedKind::Ability(Box::new(resolved.clone())),
            role,
            activation: ability.activation,
            cost: ability.cost.clone(),
            range: ability.range,
            target_type: ability
                .target
                .as_ref()
                .map(|target| target.target_type)
                .unwrap_or(TargetType::SelfTarget),
            target_side: ability.target.as_ref().and_then(|target| target.side),
            target_count: ability
                .target
                .as_ref()
                .and_then(|target| target.count)
                .unwrap_or(1),
            area_radius: ability.area.as_ref().map(|area| area.radius).unwrap_or(0),
            effects,
            damage_parts: Vec::new(),
            on_hit: Vec::new(),
            on_miss: Vec::new(),
            reach: 5,
            attack_range: None,
            attack_kind: None,
            to_hit: 0,
            tags: ability.tags.clone(),
            slot_level,
            raw_damage,
        });
    }
    let multiattack = combatant
        .creature
        .multiattack
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter_map(|id| {
            actions
                .iter()
                .find(|action| action.id.eq_ignore_ascii_case(id))
                .cloned()
                .or_else(|| {
                    combatant
                        .creature
                        .attacks
                        .as_deref()
                        .unwrap_or(&[])
                        .iter()
                        .find(|attack| attack.id.eq_ignore_ascii_case(id))
                        .map(|attack| planned_attack(state, actor, attack))
                })
        })
        .collect::<Vec<_>>();
    let best_at_will_damage = actions
        .iter()
        .filter(|action| {
            action.activation == Activation::Action
                && action.role == AiRole::Damage
                && action.cost.is_none()
        })
        .map(|action| action.raw_damage)
        .fold(0.0, f64::max);
    let threat = actions
        .iter()
        .map(|action| action.raw_damage)
        .fold(0.0, f64::max)
        .max(1.0);
    let _ = tags;
    CombatantPlan {
        opening: params.opening.clone().unwrap_or_default(),
        params,
        actions,
        multiattack,
        best_at_will_damage,
        threat,
        monster: combatant.kind() == crate::content::schema::CreatureKind::Monster,
    }
}

fn planned_attack(state: &EncounterState, actor: usize, attack: &Attack) -> PlannedAction {
    let attack_mod = attack
        .ability
        .map(|ability| state.combatants[actor].creature.ability_modifier(ability));
    let params = BTreeMap::new();
    let damage_parts = attack
        .damage
        .iter()
        .map(|part| {
            plan_damage(
                state,
                actor,
                &part.dice,
                &part.damage_type,
                1,
                &params,
                attack_mod,
                None,
            )
        })
        .collect();
    let on_hit = build_effects(state, actor, &attack.on_hit, &params, None, attack_mod, &[]);
    let raw_damage = attack
        .damage
        .iter()
        .map(|part| {
            plan_damage(
                state,
                actor,
                &part.dice,
                &part.damage_type,
                1,
                &params,
                attack_mod,
                None,
            )
            .average
        })
        .sum::<f64>()
        + raw_effect_damage(&on_hit);
    PlannedAction {
        id: attack.id.clone(),
        kind: PlannedKind::Attack(Box::new(attack.clone())),
        role: AiRole::Damage,
        activation: Activation::Action,
        cost: attack.cost.clone(),
        range: attack.range.map(|range| range[1]),
        target_type: TargetType::Creature,
        target_side: Some(TargetSide::Enemy),
        target_count: 1,
        area_radius: 0,
        effects: Vec::new(),
        damage_parts,
        on_hit,
        on_miss: Vec::new(),
        reach: attack.reach.unwrap_or(5),
        attack_range: attack.range,
        attack_kind: Some(attack.kind),
        to_hit: engine::evaluate_for_actor(
            state,
            actor,
            &attack.to_hit.clone().unwrap_or(ExprValue::Number(0)),
            &BTreeMap::new(),
            attack_mod,
            None,
        )
        .map(|expression| expression.modifier)
        .unwrap_or(0),
        tags: Vec::new(),
        slot_level: None,
        raw_damage,
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_damage(
    state: &EncounterState,
    actor: usize,
    value: &ExprValue,
    damage_type: &crate::content::schema::DamageType,
    repeat: u32,
    params: &BTreeMap<String, ExprValue>,
    attack_mod: Option<i32>,
    slot_level: Option<i32>,
) -> PlanDamage {
    let expression =
        engine::evaluate_for_actor(state, actor, value, params, attack_mod, slot_level).ok();
    let (average, dice_average) = expression
        .as_ref()
        .map(|expression| {
            let dice: f64 = expression
                .terms
                .iter()
                .map(|&(count, sides)| f64::from(count) * (f64::from(sides) + 1.0) / 2.0)
                .sum();
            (expression.average(), dice)
        })
        .unwrap_or((0.0, 0.0));
    PlanDamage {
        average,
        dice_average,
        repeat,
        dtype: DType::new(damage_type.as_str()),
    }
}

fn build_effects(
    state: &EncounterState,
    actor: usize,
    effects: &[Effect],
    params: &BTreeMap<String, ExprValue>,
    slot_level: Option<i32>,
    attack_mod: Option<i32>,
    tags: &[String],
) -> Vec<PlanEffect> {
    effects
        .iter()
        .map(|effect| build_effect(state, actor, effect, params, slot_level, attack_mod, tags))
        .collect()
}

fn build_effect(
    state: &EncounterState,
    actor: usize,
    effect: &Effect,
    params: &BTreeMap<String, ExprValue>,
    slot_level: Option<i32>,
    attack_mod: Option<i32>,
    tags: &[String],
) -> PlanEffect {
    match effect {
        Effect::Attack(effect) => PlanEffect::Attack {
            effect: Box::new(effect.clone()),
            damages: effect
                .damage
                .iter()
                .map(|part| {
                    plan_damage(
                        state,
                        actor,
                        &part.dice,
                        &part.damage_type,
                        1,
                        params,
                        attack_mod,
                        slot_level,
                    )
                })
                .collect(),
            on_hit: build_effects(
                state,
                actor,
                &effect.on_hit,
                params,
                slot_level,
                attack_mod,
                tags,
            ),
            on_miss: build_effects(
                state,
                actor,
                &effect.on_miss,
                params,
                slot_level,
                attack_mod,
                tags,
            ),
            tags: tags.to_vec(),
            params: params.clone(),
            slot_level,
            range: None,
            to_hit: engine::evaluate_for_actor(
                state,
                actor,
                &effect.to_hit,
                params,
                attack_mod,
                slot_level,
            )
            .map(|expression| expression.modifier)
            .unwrap_or(0),
        },
        Effect::Save(effect) => {
            let dc = engine::average_expression(
                state, actor, &effect.dc, params, attack_mod, slot_level,
            )
            .unwrap_or(10.0) as i32;
            PlanEffect::Save {
                ability: effect.ability,
                dc,
                half: effect.half,
                ignore_cover: effect.ignore_cover,
                on_fail: build_effects(
                    state,
                    actor,
                    &effect.on_fail,
                    params,
                    slot_level,
                    attack_mod,
                    tags,
                ),
                on_success: build_effects(
                    state,
                    actor,
                    &effect.on_success,
                    params,
                    slot_level,
                    attack_mod,
                    tags,
                ),
            }
        }
        Effect::Damage(effect) => PlanEffect::Damage(plan_damage(
            state,
            actor,
            &effect.dice,
            &effect.damage_type,
            effect.repeat,
            params,
            attack_mod,
            slot_level,
        )),
        Effect::Heal(effect) => PlanEffect::Heal(
            engine::average_expression(state, actor, &effect.dice, params, attack_mod, slot_level)
                .unwrap_or(0.0),
        ),
        Effect::ApplyCondition(effect) => PlanEffect::Condition {
            condition: effect.condition.clone(),
            on_self: effect.to == TargetRef::SelfTarget,
            value: condition_value(&state.content, &effect.condition),
        },
        Effect::Choice(options) => PlanEffect::Choice(
            options
                .iter()
                .map(|option| {
                    build_effects(state, actor, option, params, slot_level, attack_mod, tags)
                })
                .collect(),
        ),
        Effect::BasicAction(action) => PlanEffect::BasicAction(*action),
        _ => PlanEffect::Other,
    }
}

fn raw_effect_damage(effects: &[PlanEffect]) -> f64 {
    effects
        .iter()
        .map(|effect| match effect {
            PlanEffect::Attack {
                damages,
                on_hit,
                on_miss,
                ..
            } => {
                damages.iter().map(|part| part.average).sum::<f64>()
                    + raw_effect_damage(on_hit) * 0.8
                    + raw_effect_damage(on_miss) * 0.2
            }
            PlanEffect::Save {
                on_fail,
                on_success,
                ..
            } => raw_effect_damage(on_fail) * 0.7 + raw_effect_damage(on_success) * 0.3,
            PlanEffect::Damage(part) => part.average * f64::from(part.repeat.max(1)),
            PlanEffect::Choice(options) => options
                .iter()
                .map(|option| raw_effect_damage(option))
                .fold(0.0, f64::max),
            _ => 0.0,
        })
        .sum()
}

fn merged_params(content: &ContentRegistry, assignment: &Option<AiAssignment>) -> AiParams {
    let mut params = AiParams::default();
    let Some(assignment) = assignment else {
        return params;
    };
    if let Some(template) = content.ai_templates.get(&assignment.template) {
        params = merge_params(&params, &template.params);
    }
    if let Some(overrides) = &assignment.params {
        params = merge_params(&params, overrides);
    }
    params
}

fn merge_params(base: &AiParams, overrides: &AiParams) -> AiParams {
    AiParams {
        targeting: overrides.targeting.or(base.targeting),
        range_pref: overrides.range_pref.or(base.range_pref),
        keep_distance: overrides.keep_distance.or(base.keep_distance),
        self_heal_below_pct: overrides.self_heal_below_pct.or(base.self_heal_below_pct),
        heal_ally_below_pct: overrides.heal_ally_below_pct.or(base.heal_ally_below_pct),
        heal_downed: overrides.heal_downed.or(base.heal_downed),
        resource_threshold: overrides.resource_threshold.or(base.resource_threshold),
        flee_below_pct: overrides.flee_below_pct.or(base.flee_below_pct),
        flee_zone: overrides
            .flee_zone
            .clone()
            .or_else(|| base.flee_zone.clone()),
        guard_zone: overrides
            .guard_zone
            .clone()
            .or_else(|| base.guard_zone.clone()),
        leash: overrides.leash.or(base.leash),
        attack_downed: overrides.attack_downed.or(base.attack_downed),
        opening: overrides.opening.clone().or_else(|| base.opening.clone()),
        avoid_targets_with: overrides
            .avoid_targets_with
            .clone()
            .or_else(|| base.avoid_targets_with.clone()),
    }
}

fn condition_value(content: &ContentRegistry, id: &str) -> f64 {
    let Some(definition) = content.conditions.get(id) else {
        return 0.2;
    };
    condition_def_value(definition)
}

fn condition_def_value(definition: &ConditionDef) -> f64 {
    use crate::content::schema::{ConditionFlag, ModifierApplies, ModifierEffect};
    let mut value: f64 = 0.0;
    for flag in &definition.flags {
        value += match flag {
            ConditionFlag::Incapacitated => 1.2,
            ConditionFlag::SpeedZero => 0.5,
            ConditionFlag::NoReactions => 0.25,
            ConditionFlag::NoOpportunityAttacks => 0.2,
            ConditionFlag::NoOpportunityAttacksAgainst => 0.15,
            ConditionFlag::StandUpHalfSpeed => 0.1,
            ConditionFlag::Unaware => 0.35,
            ConditionFlag::SpeedHalved => 0.25,
        };
    }
    for modifier in &definition.modifiers {
        match (modifier.applies, modifier.effect) {
            (ModifierApplies::AttackRoll, ModifierEffect::Disadvantage) => value += 0.35,
            (ModifierApplies::IncomingAttack, ModifierEffect::Advantage) => value += 0.3,
            (ModifierApplies::IncomingAttack, ModifierEffect::Disadvantage) => value += 0.3,
            (ModifierApplies::IncomingAttack, ModifierEffect::AutoCrit) => value += 0.3,
            (ModifierApplies::Save, ModifierEffect::AutoFail) => value += 0.45,
            (ModifierApplies::Save, ModifierEffect::Disadvantage) => value += 0.2,
            (ModifierApplies::Ac, ModifierEffect::Bonus) => value += 0.12,
            (ModifierApplies::DamageTaken, ModifierEffect::Resistance) => value += 0.2,
            (ModifierApplies::DamageTaken, ModifierEffect::Vulnerability) => value += 0.2,
            (ModifierApplies::Speed, ModifierEffect::Bonus) => value += 0.2,
            _ => {}
        }
    }
    value.clamp(0.0, 1.6)
}

fn affordable(state: &EncounterState, actor: usize, action: &PlannedAction) -> bool {
    let Some(cost) = &action.cost else {
        return true;
    };
    state.combatants[actor]
        .resources
        .get(&cost.resource)
        .is_some_and(|resource| resource.current >= cost.amount as i32)
}

fn action_in_range(
    state: &EncounterState,
    actor: usize,
    action: &PlannedAction,
    target: usize,
) -> bool {
    if action.target_type == TargetType::SelfTarget || target == actor {
        return true;
    }
    let Some(space) = state.space() else {
        return true;
    };
    let from = state.combatants[actor].position_ref();
    let to = state.combatants[target].position_ref();
    let (from, to) = (from.as_ref(), to.as_ref());
    let Some(distance) = space.distance(from, to) else {
        return false;
    };
    match action.attack_kind {
        Some(AttackKind::Melee) => space.can_reach(from, to, action.reach),
        Some(AttackKind::Ranged) => action
            .attack_range
            .is_some_and(|[_, long]| distance <= long),
        Some(AttackKind::MeleeOrRanged) => {
            space.can_reach(from, to, action.reach)
                || action
                    .attack_range
                    .is_some_and(|[_, long]| distance <= long)
        }
        None => action
            .range
            .is_none_or(|range| range <= 0 || distance <= range),
    }
}

fn hit_chance(to_hit: i32, ac: i32, advantage: bool, disadvantage: bool) -> f64 {
    let need = f64::from(ac - to_hit);
    let single = ((21.0 - need).clamp(0.0, 20.0)) / 20.0;
    match (advantage, disadvantage) {
        (true, false) => 1.0 - (1.0 - single) * (1.0 - single),
        (false, true) => single * single,
        _ => single,
    }
}

fn hp_percent(combatant: &crate::state::CombatantState) -> i32 {
    if combatant.max_hp <= 0 {
        return 0;
    }
    (combatant.hp.max(0) * 100) / combatant.max_hp
}

fn distance(
    space: Option<&crate::space::SpaceView<'_>>,
    state: &EncounterState,
    a: usize,
    b: usize,
) -> i32 {
    space
        .and_then(|space| {
            let from = state.combatants[a].position_ref();
            let to = state.combatants[b].position_ref();
            space.distance(from.as_ref(), to.as_ref())
        })
        .unwrap_or(9999)
}

fn nearest_hostile_distance(state: &EncounterState, actor: usize) -> i32 {
    let Some(space) = state.space() else {
        return 9999;
    };
    let position = state.combatants[actor].position_ref();
    state
        .combatants
        .iter()
        .enumerate()
        .filter(|(index, combatant)| {
            *index != actor
                && combatant.side != state.combatants[actor].side
                && !combatant.dormant
                && !combatant.is_dead()
                && combatant.hp > 0
        })
        .filter_map(|(_, combatant)| {
            let other = combatant.position_ref();
            space.distance(position.as_ref(), other.as_ref())
        })
        .min()
        .unwrap_or(9999)
}

fn zone_position(space: &crate::space::SpaceView<'_>, zone: &str) -> Option<Position> {
    if !space.has_zone(zone) {
        return None;
    }
    match space.mode() {
        crate::content::schema::SceneMode::Zones => Some(Position::Zone {
            zone: zone.to_string(),
        }),
        crate::content::schema::SceneMode::Grid => space.zone_positions(zone).first().cloned(),
    }
}

fn option_for_basic_action(action: &PlannedAction, wanted: BasicAction) -> Option<usize> {
    fn contains(effects: &[PlanEffect], wanted: BasicAction) -> bool {
        effects.iter().any(|effect| match effect {
            PlanEffect::BasicAction(action) => *action == wanted,
            PlanEffect::Choice(options) => options.iter().any(|option| contains(option, wanted)),
            _ => false,
        })
    }
    for (index, effect) in action.effects.iter().enumerate() {
        let _ = index;
        if contains(std::slice::from_ref(effect), wanted) {
            if let PlanEffect::Choice(options) = effect {
                for (option_index, option) in options.iter().enumerate() {
                    if contains(option, wanted) {
                        return Some(option_index + 1);
                    }
                }
            }
            return None;
        }
    }
    None
}

fn best_option(
    _state: &EncounterState,
    _actor: usize,
    action: &PlannedAction,
    _target: usize,
) -> Option<usize> {
    for effect in &action.effects {
        if let PlanEffect::Choice(options) = effect {
            if options.is_empty() {
                return None;
            }
            return Some(1);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// P6 tactical utility planner
// ---------------------------------------------------------------------------
//
// The planner is deliberately opt-in (`AiStrategy::Utility`). `Ai::prepare`
// keeps the legacy heuristic as the default and the legacy code stays the
// fallback when a utility candidate cannot be executed. Everything below is
// private except the structures the tests and P7 need to inspect.

pub use tactics::{
    BehaviourProfile, BehaviourRole, CandidateEvaluation, CandidateKind, MovePurpose,
    RejectedCandidate, TacticalPlan, TargetProfile, UtilityTerms, UtilityWeights,
    MAX_MOVEMENT_CANDIDATES,
};

pub mod tactics {
    use crate::content::schema::{
        Activation, AiRole, AiTargeting, AttackKind, ConditionFlag, Cover, Effect, Interact,
        RangePreference, SceneMode, TargetSide, TargetType,
    };
    use crate::dice::DiceRng;
    use crate::engine::{self, LogSink};
    use crate::space::{GridMap, Position, Space};
    use crate::state::EncounterState;

    use anyhow::{bail, Result};

    use super::{
        affordable, distance, hp_percent, option_for_basic_action, zone_position, Ai,
        CombatantPlan, PlanEffect, PlannedAction, PlannedKind,
    };
    use crate::content::schema::BasicAction;

    /// Hard cap on distinct movement waypoints generated in one turn.
    ///
    /// The planner never asks the geometry kernel for "all reachable cells": it
    /// builds a small set of tactical anchors (guard/flee/objective zones,
    /// target/ally rings, cover neighbours) and keeps the first
    /// `MAX_MOVEMENT_CANDIDATES` legal ones in deterministic order.
    pub const MAX_MOVEMENT_CANDIDATES: usize = 8;

    const MAX_CANDIDATES: usize = 128;
    const MAX_REJECTED: usize = 64;
    const RESCUE_MULTIPLIER: f64 = 2.5;

    /// Explicit strategy weights. P6 keeps these in code on purpose: the task
    /// forbids `content/schema.rs` changes, so a data-driven policy is a later
    /// task. No creature/class/spell name appears here.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct UtilityWeights {
        pub expected_damage: f64,
        pub ally_risk_reduction: f64,
        pub healing: f64,
        pub control: f64,
        pub objective_progress: f64,
        pub resource_opportunity_cost: f64,
        pub cover_positioning: f64,
        pub cooperation: f64,
        pub retreat_alert_cost: f64,
        pub behaviour_consistency: f64,
        pub movement_cost: f64,
    }

    impl UtilityWeights {
        pub const DEFAULT: Self = Self {
            expected_damage: 1.0,
            ally_risk_reduction: 1.4,
            healing: 1.2,
            control: 2.2,
            objective_progress: 6.0,
            resource_opportunity_cost: 1.5,
            cover_positioning: 2.0,
            cooperation: 1.2,
            retreat_alert_cost: 1.5,
            behaviour_consistency: 1.0,
            movement_cost: 0.05,
        };
    }

    impl Default for UtilityWeights {
        fn default() -> Self {
            Self::DEFAULT
        }
    }

    /// One candidate's utility decomposition. Positive terms add value; cost
    /// terms are subtracted by [`Self::score`].
    #[derive(Debug, Clone, Copy, Default, PartialEq)]
    pub struct UtilityTerms {
        pub expected_damage: f64,
        pub ally_risk_reduction: f64,
        pub healing: f64,
        pub control: f64,
        pub objective_progress: f64,
        pub resource_opportunity_cost: f64,
        pub cover_positioning: f64,
        pub cooperation: f64,
        pub retreat_alert_cost: f64,
        pub behaviour_consistency: f64,
        pub movement_cost: f64,
    }

    impl UtilityTerms {
        pub fn score(&self, weights: &UtilityWeights) -> f64 {
            weights.expected_damage * self.expected_damage
                + weights.ally_risk_reduction * self.ally_risk_reduction
                + weights.healing * self.healing
                + weights.control * self.control
                + weights.objective_progress * self.objective_progress
                - weights.resource_opportunity_cost * self.resource_opportunity_cost
                + weights.cover_positioning * self.cover_positioning
                + weights.cooperation * self.cooperation
                - weights.retreat_alert_cost * self.retreat_alert_cost
                + weights.behaviour_consistency * self.behaviour_consistency
                - weights.movement_cost * self.movement_cost
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BehaviourRole {
        Guardian,
        Escort,
        Retreat,
        Support,
        Controller,
        Skirmisher,
        Striker,
    }

    impl BehaviourRole {
        pub fn as_str(self) -> &'static str {
            match self {
                Self::Guardian => "guardian",
                Self::Escort => "escort",
                Self::Retreat => "retreat",
                Self::Support => "support",
                Self::Controller => "controller",
                Self::Skirmisher => "skirmisher",
                Self::Striker => "striker",
            }
        }
    }

    /// Targeting policy separated from the behaviour role.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TargetProfile {
        pub targeting: AiTargeting,
        pub avoid_conditions: Vec<String>,
        pub attack_downed: bool,
    }

    /// Read-only description of what this combatant is trying to do.
    #[derive(Debug, Clone, PartialEq)]
    pub struct BehaviourProfile {
        pub role: BehaviourRole,
        pub target: TargetProfile,
        pub guard_zone: Option<String>,
        pub flee_zone: Option<String>,
        pub leash: Option<i32>,
        pub range_preference: Option<RangePreference>,
        pub heal_ally_below_pct: Option<i32>,
        pub heal_downed: bool,
        pub self_heal_below_pct: Option<i32>,
        pub flee_below_pct: Option<i32>,
        pub resource_threshold: f64,
        pub opening: Vec<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MovePurpose {
        HoldPoint,
        Approach,
        Cover,
        Escort,
        Objective,
        Retreat,
        Reposition,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum CandidateKind {
        Action {
            action_id: String,
            target: Option<String>,
            role: AiRole,
            approach: Option<Position>,
        },
        Move {
            purpose: MovePurpose,
            to: Position,
        },
        Interact {
            object_id: String,
            approach: Option<Position>,
        },
        Wait,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct RejectedCandidate {
        pub action_id: String,
        pub target: Option<String>,
        pub reason: String,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Waypoint {
        position: Position,
        cost: i32,
        purpose: MovePurpose,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct CandidateEvaluation {
        pub kind: CandidateKind,
        pub terms: UtilityTerms,
        pub score: f64,
        action_index: Option<usize>,
        target_index: Option<usize>,
        option: Option<usize>,
        waypoint: Option<Waypoint>,
        multiattack: bool,
    }

    impl CandidateEvaluation {
        fn sort_key(&self) -> String {
            match &self.kind {
                CandidateKind::Action {
                    action_id,
                    target,
                    role,
                    approach,
                } => format!(
                    "0:{action_id}:{role:?}:{}:{}",
                    target.as_deref().unwrap_or(""),
                    position_key(approach.as_ref())
                ),
                CandidateKind::Move { purpose, to } => {
                    format!("1:{purpose:?}:{}", position_key(Some(to)))
                }
                CandidateKind::Interact {
                    object_id,
                    approach,
                } => format!("2:{object_id}:{}", position_key(approach.as_ref())),
                CandidateKind::Wait => "9:wait".to_string(),
            }
        }
    }

    fn position_key(position: Option<&Position>) -> String {
        match position {
            Some(Position::Cell { x, y }) => format!("cell:{x},{y}"),
            Some(Position::Zone { zone }) => format!("zone:{zone}"),
            None => "here".to_string(),
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct TacticalPlan {
        pub actor: usize,
        pub profile: BehaviourProfile,
        pub candidates: Vec<CandidateEvaluation>,
        pub rejected: Vec<RejectedCandidate>,
        pub selected_index: Option<usize>,
    }

    impl TacticalPlan {
        pub fn selected(&self) -> Option<&CandidateEvaluation> {
            self.selected_index
                .and_then(|index| self.candidates.get(index))
        }

        /// Number of distinct movement destinations referenced by candidates.
        /// This is the value bounded by [`MAX_MOVEMENT_CANDIDATES`].
        pub fn movement_waypoint_count(&self) -> usize {
            let mut positions: Vec<Position> = Vec::new();
            for candidate in &self.candidates {
                if let Some(waypoint) = &candidate.waypoint {
                    if !positions.contains(&waypoint.position) {
                        positions.push(waypoint.position.clone());
                    }
                }
            }
            positions.len()
        }
    }

    impl Ai {
        /// Plans the current turn without mutating state or consuming dice.
        pub fn preview_turn(&self, state: &EncounterState) -> Option<TacticalPlan> {
            let actor = state.current_index()?;
            let plan = self.plans.get(actor)?;
            if !can_act(state, actor) {
                return None;
            }
            Some(Planner::new(self, state, actor, plan).build())
        }

        /// Executes the best still-legal candidate from a previously built
        /// plan. Candidates are re-validated against the current state before
        /// every engine call.
        pub fn execute_tactical_plan(
            &self,
            state: &mut EncounterState,
            plan: &TacticalPlan,
            rng: &mut dyn DiceRng,
            log: &mut dyn LogSink,
        ) -> Result<bool> {
            if plan.actor >= self.plans.len() {
                return Ok(false);
            }
            let candidates: Vec<CandidateEvaluation> = plan.candidates.clone();
            execute_candidates(self, state, plan.actor, &candidates, rng, log)
        }
    }

    pub(super) fn take_utility_turn(
        ai: &Ai,
        state: &mut EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<bool> {
        let profile = build_profile(state, actor, plan);
        let fleeing = profile.flee_below_pct.is_some_and(|threshold| {
            threshold > 0 && hp_percent(&state.combatants[actor]) <= threshold
        });

        if fleeing {
            let preview = Planner::new(ai, state, actor, plan).build();
            let (defense, movement): (Vec<CandidateEvaluation>, Vec<CandidateEvaluation>) = preview
                .candidates
                .into_iter()
                .filter(is_retreat_candidate)
                .partition(is_defense_candidate);
            // Disengage/Dash first so the retreat path does not eat an
            // opportunity attack; then the movement candidate. Return whether
            // either phase actually executed so a fully failed retreat still
            // reaches the legacy fallback.
            let defense_ok = execute_candidates(ai, state, actor, &defense, rng, log)?;
            let movement_ok = execute_candidates(ai, state, actor, &movement, rng, log)?;
            return Ok(defense_ok || movement_ok);
        }

        // Bonus-action support/defence first; this keeps the legacy economy
        // (self-heal, disengage) without a second, hidden planner.
        if !state.combatants[actor].bonus_action_used {
            let preview = Planner::new(ai, state, actor, plan).build();
            let bonus: Vec<CandidateEvaluation> = preview
                .candidates
                .into_iter()
                .filter(|candidate| is_bonus_support(ai, state, actor, candidate))
                .collect();
            if !bonus.is_empty() {
                let _ = execute_candidates(ai, state, actor, &bonus, rng, log)?;
            }
        }

        let preview = Planner::new(ai, state, actor, plan).build();
        execute_candidates(ai, state, actor, &preview.candidates, rng, log)
    }

    fn is_retreat_candidate(candidate: &CandidateEvaluation) -> bool {
        match &candidate.kind {
            CandidateKind::Move {
                purpose: MovePurpose::Retreat,
                ..
            } => true,
            CandidateKind::Action { role, .. } => *role == AiRole::Defense,
            _ => false,
        }
    }

    fn is_defense_candidate(candidate: &CandidateEvaluation) -> bool {
        matches!(
            &candidate.kind,
            CandidateKind::Action {
                role: AiRole::Defense,
                ..
            }
        )
    }

    fn defense_option(action: &PlannedAction) -> Option<usize> {
        if action.role != AiRole::Defense {
            return None;
        }
        option_for_basic_action(action, BasicAction::Disengage)
            .or_else(|| option_for_basic_action(action, BasicAction::Dash))
    }

    fn is_bonus_support(
        ai: &Ai,
        state: &EncounterState,
        actor: usize,
        candidate: &CandidateEvaluation,
    ) -> bool {
        let Some(index) = candidate.action_index else {
            return false;
        };
        let Some(action) = ai.plans[actor].actions.get(index) else {
            return false;
        };
        if action.role == AiRole::Defense && !is_threatened(state, actor) {
            return false;
        }
        action.activation == Activation::Bonus
            && matches!(
                action.role,
                AiRole::SelfHeal | AiRole::Defense | AiRole::Buff
            )
            && candidate.score > 0.0
    }

    fn is_threatened(state: &EncounterState, actor: usize) -> bool {
        hp_percent(&state.combatants[actor]) < 60
            || state.space().is_some_and(|space| {
                state
                    .combatants
                    .iter()
                    .enumerate()
                    .any(|(index, combatant)| {
                        index != actor
                            && combatant.side != state.combatants[actor].side
                            && !combatant.dormant
                            && !combatant.is_dead()
                            && !is_unaware(state, index)
                            && combatant.hp > 0
                            && space
                                .distance(
                                    &state.combatants[actor].position(),
                                    &combatant.position(),
                                )
                                .is_some_and(|distance| distance <= 5)
                    })
            })
    }

    fn can_act(state: &EncounterState, actor: usize) -> bool {
        let combatant = &state.combatants[actor];
        !combatant.dormant
            && !combatant.is_dead()
            && combatant.hp > 0
            && !combatant.is_dying()
            && !engine::has_condition_flag(state, actor, ConditionFlag::Incapacitated)
    }

    /// Builds the behaviour/target profile for one actor.
    fn build_profile(
        state: &EncounterState,
        actor: usize,
        plan: &CombatantPlan,
    ) -> BehaviourProfile {
        let params = &plan.params;
        let guard_zone = params.guard_zone.clone();
        let fleeing = params.flee_below_pct.is_some_and(|threshold| {
            threshold > 0 && hp_percent(&state.combatants[actor]) <= threshold
        });
        let ally_in_guard = guard_zone.as_deref().is_some_and(|zone| {
            state
                .combatants
                .iter()
                .enumerate()
                .any(|(index, combatant)| {
                    index != actor
                        && combatant.side == state.combatants[actor].side
                        && !combatant.is_dead()
                        && combatant.zone == zone
                })
        });
        let healing =
            params.heal_ally_below_pct.unwrap_or(0) > 0 || params.heal_downed.unwrap_or(false);
        let controller = plan
            .actions
            .iter()
            .any(|action| action.role == AiRole::Control);
        let skirmisher = plan
            .actions
            .iter()
            .any(|action| action.role == AiRole::Defense)
            || params.range_pref == Some(RangePreference::Ranged);
        let role = if fleeing {
            BehaviourRole::Retreat
        } else if guard_zone.is_some() && ally_in_guard {
            BehaviourRole::Escort
        } else if guard_zone.is_some() {
            BehaviourRole::Guardian
        } else if healing {
            BehaviourRole::Support
        } else if controller {
            BehaviourRole::Controller
        } else if skirmisher {
            BehaviourRole::Skirmisher
        } else {
            BehaviourRole::Striker
        };
        BehaviourProfile {
            role,
            target: TargetProfile {
                targeting: params.targeting.unwrap_or(AiTargeting::Nearest),
                avoid_conditions: params.avoid_targets_with.clone().unwrap_or_default(),
                attack_downed: params.attack_downed.unwrap_or(false),
            },
            guard_zone,
            flee_zone: params.flee_zone.clone(),
            leash: params.leash,
            range_preference: params.range_pref,
            heal_ally_below_pct: params.heal_ally_below_pct,
            heal_downed: params.heal_downed.unwrap_or(false),
            self_heal_below_pct: params.self_heal_below_pct,
            flee_below_pct: params.flee_below_pct,
            resource_threshold: params
                .resource_threshold
                .unwrap_or(super::DEFAULT_RESOURCE_THRESHOLD),
            opening: params.opening.clone().unwrap_or_default(),
        }
    }

    struct Planner<'a> {
        ai: &'a Ai,
        state: &'a EncounterState,
        actor: usize,
        plan: &'a CombatantPlan,
        profile: BehaviourProfile,
        weights: UtilityWeights,
        waypoints: Vec<Waypoint>,
        candidates: Vec<CandidateEvaluation>,
        rejected: Vec<RejectedCandidate>,
    }

    impl<'a> Planner<'a> {
        fn new(
            ai: &'a Ai,
            state: &'a EncounterState,
            actor: usize,
            plan: &'a CombatantPlan,
        ) -> Self {
            Self {
                ai,
                state,
                actor,
                plan,
                profile: build_profile(state, actor, plan),
                weights: UtilityWeights::DEFAULT,
                waypoints: Vec::new(),
                candidates: Vec::new(),
                rejected: Vec::new(),
            }
        }

        fn build(mut self) -> TacticalPlan {
            self.collect_waypoints();
            self.generate_actions();
            self.generate_multiattack();
            self.generate_interactions();
            self.generate_moves();
            self.generate_wait();
            self.candidates.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.sort_key().cmp(&right.sort_key()))
            });
            let selected_index = self.candidates.iter().position(|candidate| {
                candidate.score > 0.0 || candidate.kind == CandidateKind::Wait
            });
            TacticalPlan {
                actor: self.actor,
                profile: self.profile,
                candidates: self.candidates,
                rejected: self.rejected,
                selected_index,
            }
        }

        fn fleeing(&self) -> bool {
            self.profile.flee_below_pct.is_some_and(|threshold| {
                threshold > 0 && hp_percent(&self.state.combatants[self.actor]) <= threshold
            })
        }

        fn current_position(&self) -> Position {
            self.state.combatants[self.actor].position()
        }

        fn blocked_positions(&self) -> Vec<Position> {
            self.state
                .combatants
                .iter()
                .enumerate()
                .filter(|(index, combatant)| *index != self.actor && !combatant.is_dead())
                .map(|(_, combatant)| combatant.position())
                .collect()
        }

        fn observable_enemies(&self) -> Vec<usize> {
            (0..self.state.combatants.len())
                .filter(|&index| {
                    index != self.actor
                        && self.state.combatants[index].side
                            != self.state.combatants[self.actor].side
                        && !self.state.combatants[index].dormant
                        && !self.state.combatants[index].is_dead()
                        && (self.state.combatants[index].hp > 0
                            || (self.profile.target.attack_downed && self.plan.monster))
                        && !is_unaware(self.state, index)
                })
                .collect()
        }

        fn observable_allies(&self) -> Vec<usize> {
            (0..self.state.combatants.len())
                .filter(|&index| {
                    index != self.actor
                        && self.state.combatants[index].side
                            == self.state.combatants[self.actor].side
                        && !self.state.combatants[index].dormant
                        && !self.state.combatants[index].is_dead()
                })
                .collect()
        }

        fn collect_waypoints(&mut self) {
            let Some(space) = self.state.space() else {
                return;
            };
            let start = self.current_position();
            let blocked = self.blocked_positions();
            let budget = self.state.combatants[self.actor].speed_left;
            let mut anchors: Vec<(Position, MovePurpose)> =
                vec![(start.clone(), MovePurpose::Reposition)];

            if let Some(zone) = self.profile.guard_zone.as_deref() {
                if let Some(position) = zone_position(&space, zone) {
                    anchors.push((position, MovePurpose::HoldPoint));
                }
            }
            if self.fleeing() {
                if let Some(position) = self
                    .profile
                    .flee_zone
                    .as_deref()
                    .and_then(|zone| zone_position(&space, zone))
                {
                    anchors.push((position, MovePurpose::Retreat));
                } else if let Some(position) = self.away_position(&space, &start, &blocked, budget)
                {
                    anchors.push((position, MovePurpose::Retreat));
                }
            }

            for object in self.objective_objects() {
                if let Some(position) = zone_position(&space, &object.zone) {
                    anchors.push((position, MovePurpose::Objective));
                }
            }

            if let Some(position) = self.cover_position(&space, &start) {
                anchors.push((position, MovePurpose::Cover));
            }

            let enemies = self.observable_enemies();
            for &enemy in enemies.iter().take(3) {
                let position = self.state.combatants[enemy].position();
                for anchor in neighbour_positions(&space, &position) {
                    anchors.push((anchor, MovePurpose::Approach));
                }
            }
            if let Some(&enemy) = enemies.first() {
                let target = self.state.combatants[enemy].position();
                if let Some(position) = toward_position(&space, &start, &target, budget) {
                    anchors.push((position, MovePurpose::Approach));
                }
            }

            for &ally in self.observable_allies().iter().take(2) {
                let position = self.state.combatants[ally].position();
                for anchor in neighbour_positions(&space, &position) {
                    anchors.push((anchor, MovePurpose::Escort));
                }
            }

            let mut seen: Vec<Position> = Vec::new();
            for (position, purpose) in anchors {
                if self.waypoints.len() >= MAX_MOVEMENT_CANDIDATES {
                    break;
                }
                if seen.contains(&position) {
                    continue;
                }
                seen.push(position.clone());
                let Some(cost) = space.route_cost(&start, &position, &blocked) else {
                    continue;
                };
                if cost > budget {
                    continue;
                }
                self.waypoints.push(Waypoint {
                    position,
                    cost,
                    purpose,
                });
            }
        }

        fn away_position(
            &self,
            space: &crate::space::SpaceView<'_>,
            start: &Position,
            blocked: &[Position],
            budget: i32,
        ) -> Option<Position> {
            let enemy = self
                .observable_enemies()
                .into_iter()
                .min_by_key(|&index| distance(Some(space), self.state, self.actor, index))?;
            let enemy_position = self.state.combatants[enemy].position();
            match (start, enemy_position) {
                (Position::Zone { zone }, _) => {
                    let scene = self.state.scene.as_deref()?;
                    scene
                        .zones
                        .iter()
                        .filter(|candidate| candidate.id != *zone)
                        .filter_map(|candidate| {
                            let position = Position::Zone {
                                zone: candidate.id.clone(),
                            };
                            space
                                .distance(start, &position)
                                .map(|distance| (distance, position))
                        })
                        .max_by_key(|(distance, _)| *distance)
                        .map(|(_, position)| position)
                }
                (Position::Cell { x, y }, Position::Cell { x: ex, y: ey }) => {
                    let steps = (budget / 5).max(1);
                    let dx = (x - ex).signum();
                    let dy = (y - ey).signum();
                    let destination = Position::cell(x + dx * steps, y + dy * steps);
                    space.route_cost(start, &destination, blocked)?;
                    Some(destination)
                }
                _ => None,
            }
        }

        fn cover_position(
            &self,
            space: &crate::space::SpaceView<'_>,
            start: &Position,
        ) -> Option<Position> {
            let (x, y) = start.cell_coords()?;
            let scene = self.state.scene.as_deref()?;
            let grid = GridMap::parse(scene)?;
            for (dx, dy) in NEIGHBOURS {
                let (nx, ny) = (x + dx, y + dy);
                let Some(cell) = grid.cell_at(nx, ny) else {
                    continue;
                };
                if cell.cover != Cover::None {
                    let position = Position::cell(nx, ny);
                    if space
                        .route_cost(start, &position, &self.blocked_positions())
                        .is_some()
                    {
                        return Some(position);
                    }
                }
            }
            None
        }

        fn objective_objects(&self) -> Vec<crate::content::schema::SceneObject> {
            self.state
                .scene
                .as_deref()
                .map(|scene| {
                    scene
                        .objects
                        .iter()
                        .filter(|object| object.interact.as_ref().is_some_and(has_objective_effect))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        }

        fn generate_actions(&mut self) {
            for (index, action) in self.plan.actions.iter().enumerate() {
                if self.candidates.len() >= MAX_CANDIDATES {
                    break;
                }
                if matches!(
                    action.activation,
                    Activation::Passive | Activation::Reaction
                ) {
                    continue;
                }
                if action.role == AiRole::None {
                    self.reject(
                        &action.id,
                        None,
                        "unsupported AI role (no valuation)".to_string(),
                    );
                    continue;
                }
                if let Some(reason) = unsupported_effects(action) {
                    self.reject(&action.id, None, format!("unsupported effect: {reason}"));
                    continue;
                }
                let targets = self.targets_for(action);
                if targets.is_empty() {
                    self.reject(
                        &action.id,
                        None,
                        "no observable target for this action".to_string(),
                    );
                    continue;
                }
                for target in targets {
                    if self.candidates.len() >= MAX_CANDIDATES {
                        break;
                    }
                    if !self.target_allowed(action, target) {
                        continue;
                    }
                    self.add_action_candidate(index, action, target);
                }
            }
        }

        fn generate_multiattack(&mut self) {
            if self.plan.multiattack.is_empty() || self.candidates.len() >= MAX_CANDIDATES {
                return;
            }
            let enemies = self.observable_enemies();
            for target in enemies {
                if self.candidates.len() >= MAX_CANDIDATES {
                    break;
                }
                if let Some(reason) = self.multiattack_rejection(target) {
                    self.reject("multiattack", Some(self.name(target)), reason);
                    continue;
                }
                let terms = self.multiattack_terms(target, None);
                if terms.score(&self.weights) <= 0.0 {
                    continue;
                }
                self.candidates.push(CandidateEvaluation {
                    kind: CandidateKind::Action {
                        action_id: "multiattack".to_string(),
                        target: Some(self.name(target)),
                        role: AiRole::Damage,
                        approach: None,
                    },
                    terms,
                    score: terms.score(&self.weights),
                    action_index: None,
                    target_index: Some(target),
                    option: None,
                    waypoint: None,
                    multiattack: true,
                });
            }
        }

        fn generate_interactions(&mut self) {
            let Some(scene) = self.state.scene.as_deref() else {
                return;
            };
            let space = self.state.space();
            for object in &scene.objects {
                if self.candidates.len() >= MAX_CANDIDATES {
                    break;
                }
                let Some(interact) = object.interact.as_ref() else {
                    if object.manual.is_some() {
                        self.reject(
                            &object.id,
                            None,
                            "manual scene interaction is not valued".to_string(),
                        );
                    }
                    continue;
                };
                if !has_objective_effect(interact) {
                    self.reject(
                        &object.id,
                        None,
                        "interaction has no valued objective effect".to_string(),
                    );
                    continue;
                }
                let mut waypoint = None;
                let mut legal = match &space {
                    Some(space) => same_zone(self.state, space, self.actor, object.zone.as_str()),
                    None => true,
                };
                if !legal {
                    if let Some(space) = space.as_ref() {
                        waypoint = self
                            .waypoints
                            .iter()
                            .find(|waypoint| {
                                zone_of_position(space, &waypoint.position).as_deref()
                                    == Some(object.zone.as_str())
                            })
                            .cloned();
                        legal = waypoint.is_some();
                    }
                }
                if !legal {
                    self.reject(
                        &object.id,
                        None,
                        format!("no reachable approach to object zone `{}`", object.zone),
                    );
                    continue;
                }
                let urgency = self.objective_urgency(&object.zone);
                let mut terms = UtilityTerms {
                    objective_progress: 1.0 + urgency,
                    behaviour_consistency: 0.8,
                    ..UtilityTerms::default()
                };
                if let Some(waypoint) = &waypoint {
                    terms.movement_cost = f64::from(waypoint.cost.max(0)) / 5.0;
                }
                let kind = CandidateKind::Interact {
                    object_id: object.id.clone(),
                    approach: waypoint.as_ref().map(|waypoint| waypoint.position.clone()),
                };
                self.candidates.push(CandidateEvaluation {
                    kind,
                    terms,
                    score: terms.score(&self.weights),
                    action_index: None,
                    target_index: None,
                    option: None,
                    waypoint,
                    multiattack: false,
                });
            }
        }

        fn generate_moves(&mut self) {
            let Some(space) = self.state.space() else {
                return;
            };
            let start = self.current_position();
            let current_cover = cover_value(self.state, &space, &start, self.actor);
            let current_threat = self.threat_at(&space, &start);
            for waypoint in self.waypoints.clone() {
                if self.candidates.len() >= MAX_CANDIDATES {
                    break;
                }
                if waypoint.cost == 0 && waypoint.purpose == MovePurpose::Reposition {
                    continue;
                }
                let cover = cover_value(self.state, &space, &waypoint.position, self.actor);
                let threat = self.threat_at(&space, &waypoint.position);
                let retreat = waypoint.purpose == MovePurpose::Retreat;
                let terms = UtilityTerms {
                    objective_progress: match waypoint.purpose {
                        MovePurpose::HoldPoint => 0.8,
                        MovePurpose::Retreat => 1.0,
                        MovePurpose::Objective => 0.7,
                        MovePurpose::Escort => 0.6,
                        MovePurpose::Cover => 0.4,
                        MovePurpose::Approach => 0.3,
                        MovePurpose::Reposition => 0.1,
                    },
                    cover_positioning: (cover - current_cover).clamp(-3.0, 3.0),
                    cooperation: self.ally_support_value(&space, &waypoint.position),
                    ally_risk_reduction: if retreat {
                        (current_threat - threat).max(0.0)
                    } else {
                        0.0
                    },
                    retreat_alert_cost: if retreat {
                        self.retreat_alert_cost(&space, &waypoint.position)
                    } else {
                        0.0
                    },
                    behaviour_consistency: self.move_consistency(&waypoint),
                    movement_cost: f64::from(waypoint.cost.max(0)) / 5.0,
                    ..UtilityTerms::default()
                };
                let kind = CandidateKind::Move {
                    purpose: waypoint.purpose,
                    to: waypoint.position.clone(),
                };
                self.candidates.push(CandidateEvaluation {
                    kind,
                    terms,
                    score: terms.score(&self.weights),
                    action_index: None,
                    target_index: None,
                    option: None,
                    waypoint: Some(waypoint),
                    multiattack: false,
                });
            }
        }

        fn generate_wait(&mut self) {
            let holding = self.profile.guard_zone.as_deref().is_some_and(|zone| {
                self.current_position().zone_id() == Some(zone)
                    && self.nearest_observable_enemy_distance()
                        > self.profile.leash.unwrap_or(i32::MAX)
            });
            let terms = UtilityTerms {
                objective_progress: if holding { 0.8 } else { 0.0 },
                behaviour_consistency: if holding { 1.0 } else { 0.0 },
                ..UtilityTerms::default()
            };
            self.candidates.push(CandidateEvaluation {
                kind: CandidateKind::Wait,
                terms,
                score: terms.score(&self.weights),
                action_index: None,
                target_index: None,
                option: None,
                waypoint: None,
                multiattack: false,
            });
        }

        fn targets_for(&self, action: &PlannedAction) -> Vec<usize> {
            if action.target_type == TargetType::SelfTarget || action.role == AiRole::SelfHeal {
                return vec![self.actor];
            }
            let allies = if action.role == AiRole::Heal
                || action.role == AiRole::Buff
                || action.role == AiRole::Defense
                || action.target_side == Some(TargetSide::Ally)
            {
                self.support_targets(action)
            } else {
                Vec::new()
            };
            let enemies = if action.target_side == Some(TargetSide::Ally) {
                Vec::new()
            } else {
                self.observable_enemies()
            };
            match action.target_type {
                TargetType::Zone => {
                    if action.target_side == Some(TargetSide::Ally) {
                        allies
                    } else {
                        enemies
                    }
                }
                TargetType::Creature | TargetType::SelfTarget => {
                    let mut targets = if action.role == AiRole::Heal
                        || action.role == AiRole::Buff
                        || action.role == AiRole::Defense
                    {
                        allies
                    } else {
                        enemies
                    };
                    if matches!(action.role, AiRole::Buff | AiRole::Defense)
                        && action.target_side == Some(TargetSide::Ally)
                        && !targets.contains(&self.actor)
                    {
                        targets.push(self.actor);
                    }
                    targets
                }
            }
        }

        fn support_targets(&self, action: &PlannedAction) -> Vec<usize> {
            let allies = self.observable_allies();
            if action.role == AiRole::Heal {
                return allies
                    .into_iter()
                    .filter(|&index| {
                        let hp = self.state.combatants[index].hp;
                        if hp <= 0 {
                            self.profile.heal_downed
                        } else {
                            let threshold = self.profile.heal_ally_below_pct.unwrap_or(100);
                            hp_percent(&self.state.combatants[index]) <= threshold
                        }
                    })
                    .collect();
            }
            allies
        }

        fn target_allowed(&self, action: &PlannedAction, target: usize) -> bool {
            if target == self.actor {
                return true;
            }
            if action.role == AiRole::Heal {
                let hp = self.state.combatants[target].hp;
                if hp <= 0 && !self.profile.heal_downed {
                    return false;
                }
            }
            true
        }

        fn add_action_candidate(
            &mut self,
            action_index: usize,
            action: &PlannedAction,
            target: usize,
        ) {
            let start = self.current_position();
            if let Err(reason) = self.action_precondition(action) {
                self.reject(&action.id, Some(self.name(target)), reason);
                return;
            }
            if self.action_legal_at(action, target, &start) {
                let terms = self.action_terms(action, target, None);
                let score = terms.score(&self.weights);
                if score > 0.0 {
                    self.candidates.push(CandidateEvaluation {
                        kind: CandidateKind::Action {
                            action_id: action.id.clone(),
                            target: Some(self.name(target)),
                            role: action.role,
                            approach: None,
                        },
                        terms,
                        score,
                        action_index: Some(action_index),
                        target_index: Some(target),
                        option: defense_option(action),
                        waypoint: None,
                        multiattack: false,
                    });
                }
                return;
            }
            let mut approaches: Vec<Waypoint> = Vec::new();
            for waypoint in self.waypoints.iter() {
                if waypoint.position == start {
                    continue;
                }
                if self.action_legal_at(action, target, &waypoint.position) {
                    approaches.push(waypoint.clone());
                }
                if approaches.len() >= 2 {
                    break;
                }
            }
            if approaches.is_empty() {
                self.reject(
                    &action.id,
                    Some(self.name(target)),
                    self.action_rejection(action, target, &start),
                );
                return;
            }
            for waypoint in approaches {
                let terms = self.action_terms(action, target, Some(&waypoint));
                let score = terms.score(&self.weights);
                if score <= 0.0 {
                    continue;
                }
                self.candidates.push(CandidateEvaluation {
                    kind: CandidateKind::Action {
                        action_id: action.id.clone(),
                        target: Some(self.name(target)),
                        role: action.role,
                        approach: Some(waypoint.position.clone()),
                    },
                    terms,
                    score,
                    action_index: Some(action_index),
                    target_index: Some(target),
                    option: defense_option(action),
                    waypoint: Some(waypoint),
                    multiattack: false,
                });
            }
        }

        fn action_precondition(&self, action: &PlannedAction) -> std::result::Result<(), String> {
            action_precondition(self.state, self.actor, self.plan, action)
        }

        fn action_legal_at(&self, action: &PlannedAction, target: usize, from: &Position) -> bool {
            action_legal_at_state(self.state, action, self.actor, Some(target), from)
        }

        fn action_rejection(
            &self,
            action: &PlannedAction,
            target: usize,
            from: &Position,
        ) -> String {
            if !affordable(self.state, self.actor, action) {
                return "resource not affordable".to_string();
            }
            if !self.ai.resource_allows(self.state, self.plan, action) {
                return "resource threshold".to_string();
            }
            let to = self.state.combatants[target].position();
            if let Some(space) = self.state.space() {
                if let Some(feet) = space.distance(from, &to) {
                    if action.target_type != TargetType::Zone
                        && space.cover(from, &to) == Cover::Total
                    {
                        return "total cover".to_string();
                    }
                    let limit = action
                        .attack_range
                        .map(|[_, long]| long)
                        .or(action.range)
                        .unwrap_or(action.reach);
                    if feet > limit {
                        return "out of range".to_string();
                    }
                }
            }
            "no legal approach".to_string()
        }

        fn action_terms(
            &self,
            action: &PlannedAction,
            target: usize,
            waypoint: Option<&Waypoint>,
        ) -> UtilityTerms {
            let mut terms = Self::terms_for_outcome(
                self.ai,
                self.state,
                self.actor,
                action,
                target,
                self.profile.target.targeting,
                &self.profile,
                waypoint,
                self.current_position(),
            );
            if action.role == AiRole::Defense && self.nearest_observable_enemy_distance() <= 5 {
                terms.ally_risk_reduction += 1.0;
            }
            terms
        }

        #[allow(clippy::too_many_arguments)]
        fn terms_for_outcome(
            ai: &Ai,
            state: &EncounterState,
            actor: usize,
            action: &PlannedAction,
            target: usize,
            targeting: AiTargeting,
            profile: &BehaviourProfile,
            waypoint: Option<&Waypoint>,
            current: Position,
        ) -> UtilityTerms {
            let outcome = ai.action_outcome(state, actor, action, target);
            let target_combatant = &state.combatants[target];
            let threat = ai.plans.get(target).map(|plan| plan.threat).unwrap_or(1.0);
            let hostile = target_combatant.side != state.combatants[actor].side;
            let hp = target_combatant.hp.max(0) + target_combatant.temp_hp.max(0);
            let expected_damage = if hostile {
                outcome.damage.max(0.0).min(f64::from(hp.max(1)))
            } else {
                0.0
            };
            let healing = if outcome.heal > 0.0 {
                let rescue = if target_combatant.hp <= 0 {
                    RESCUE_MULTIPLIER
                } else {
                    1.0
                };
                outcome.heal.max(0.0) * rescue
            } else {
                0.0
            };
            let control = if hostile {
                outcome.control.max(0.0) * (1.0 + threat)
            } else {
                0.0
            };
            let mut ally_risk_reduction = outcome.defense.max(0.0) + outcome.buff.max(0.0);
            if action.role == AiRole::Control {
                ally_risk_reduction += control * 0.2;
            }
            let resource_opportunity_cost = action
                .cost
                .as_ref()
                .map(|cost| {
                    let remaining = state.combatants[actor]
                        .resources
                        .get(&cost.resource)
                        .map(|resource| resource.current.max(1))
                        .unwrap_or(1);
                    f64::from(cost.amount) * (1.0 + 1.0 / f64::from(remaining))
                })
                .unwrap_or(0.0);
            let current_cover = state
                .space()
                .as_ref()
                .map(|space| cover_value(state, space, &current, actor))
                .unwrap_or(0.0);
            let cover_positioning = waypoint
                .map(|waypoint| {
                    state
                        .space()
                        .as_ref()
                        .map(|space| {
                            (cover_value(state, space, &waypoint.position, actor) - current_cover)
                                .clamp(-3.0, 3.0)
                        })
                        .unwrap_or(0.0)
                })
                .unwrap_or(0.0);
            let cooperation = if hostile {
                allies_near(state, actor, target) as f64 * 0.2
            } else {
                1.0 + (1.0 - hp_percent(target_combatant) as f64 / 100.0)
            };
            let consistency =
                action_consistency(state, actor, action, target, targeting, profile, waypoint);
            let movement_cost = waypoint
                .map(|waypoint| f64::from(waypoint.cost.max(0)) / 5.0)
                .unwrap_or(0.0);
            UtilityTerms {
                expected_damage,
                ally_risk_reduction,
                healing,
                control,
                objective_progress: 0.0,
                resource_opportunity_cost,
                cover_positioning,
                cooperation,
                retreat_alert_cost: 0.0,
                behaviour_consistency: consistency,
                movement_cost,
            }
        }

        fn multiattack_rejection(&self, target: usize) -> Option<String> {
            for action in &self.plan.multiattack {
                if !self.action_legal_at(action, target, &self.current_position()) {
                    return Some("multiattack entry out of range or blocked".to_string());
                }
                if !affordable(self.state, self.actor, action) {
                    return Some("multiattack entry not affordable".to_string());
                }
            }
            None
        }

        fn multiattack_terms(&self, target: usize, waypoint: Option<&Waypoint>) -> UtilityTerms {
            let mut total = UtilityTerms::default();
            for action in &self.plan.multiattack {
                let terms = Self::terms_for_outcome(
                    self.ai,
                    self.state,
                    self.actor,
                    action,
                    target,
                    self.profile.target.targeting,
                    &self.profile,
                    waypoint,
                    self.current_position(),
                );
                total.expected_damage += terms.expected_damage;
                total.ally_risk_reduction += terms.ally_risk_reduction;
                total.healing += terms.healing;
                total.control += terms.control;
                total.cooperation += terms.cooperation;
                total.behaviour_consistency += terms.behaviour_consistency;
                total.movement_cost = total.movement_cost.max(terms.movement_cost);
            }
            total
        }

        fn nearest_observable_enemy_distance(&self) -> i32 {
            let space = self.state.space();
            self.observable_enemies()
                .into_iter()
                .filter_map(|enemy| distance(space.as_ref(), self.state, self.actor, enemy).into())
                .min()
                .unwrap_or(9999)
        }

        fn threat_at(&self, space: &crate::space::SpaceView<'_>, at: &Position) -> f64 {
            self.observable_enemies()
                .into_iter()
                .map(|enemy| {
                    let position = self.state.combatants[enemy].position();
                    let distance = space.distance(at, &position).unwrap_or(9999);
                    let threat = self
                        .ai
                        .plans
                        .get(enemy)
                        .map(|plan| plan.threat)
                        .unwrap_or(1.0);
                    threat / (1.0 + f64::from(distance) / 5.0)
                })
                .sum()
        }

        fn ally_support_value(&self, space: &crate::space::SpaceView<'_>, at: &Position) -> f64 {
            self.observable_allies()
                .into_iter()
                .map(|ally| {
                    let position = self.state.combatants[ally].position();
                    let distance = space.distance(at, &position).unwrap_or(9999);
                    let mut value = (3.0 - f64::from(distance) / 10.0).max(0.0);
                    if self.state.combatants[ally].hp <= 0 && distance <= 5 {
                        value += 2.0;
                    }
                    value
                })
                .sum()
        }

        fn retreat_alert_cost(&self, space: &crate::space::SpaceView<'_>, at: &Position) -> f64 {
            let adjacent = self
                .observable_enemies()
                .into_iter()
                .filter(|&enemy| {
                    space
                        .distance(at, &self.state.combatants[enemy].position())
                        .is_some_and(|distance| distance <= 5)
                })
                .count() as f64;
            let guard_penalty =
                self.profile
                    .guard_zone
                    .as_deref()
                    .is_some_and(|zone| at.zone_id() != Some(zone)) as i32 as f64;
            adjacent * 0.5 + guard_penalty
        }

        fn move_consistency(&self, waypoint: &Waypoint) -> f64 {
            let mut value = 0.0;
            if let Some(zone) = self.profile.guard_zone.as_deref() {
                if waypoint.position.zone_id() == Some(zone) {
                    value += 0.8;
                }
            }
            if let Some(zone) = self.profile.flee_zone.as_deref() {
                if waypoint.purpose == MovePurpose::Retreat
                    && waypoint.position.zone_id() == Some(zone)
                {
                    value += 0.8;
                }
            }
            value
        }

        fn objective_urgency(&self, object_zone: &str) -> f64 {
            let Some(space) = self.state.space() else {
                return 0.0;
            };
            let Some(object_position) = zone_position(&space, object_zone) else {
                return 0.0;
            };
            let actor_distance = space
                .distance(&self.current_position(), &object_position)
                .unwrap_or(9999);
            let enemy_distance = self
                .observable_enemies()
                .into_iter()
                .filter_map(|enemy| {
                    space.distance(&self.state.combatants[enemy].position(), &object_position)
                })
                .min()
                .unwrap_or(9999);
            if enemy_distance < actor_distance {
                0.5
            } else {
                0.0
            }
        }

        fn name(&self, index: usize) -> String {
            self.state.combatants[index].name.clone()
        }

        fn reject(&mut self, action_id: &str, target: Option<String>, reason: String) {
            if self.rejected.len() >= MAX_REJECTED {
                return;
            }
            self.rejected.push(RejectedCandidate {
                action_id: action_id.to_string(),
                target,
                reason,
            });
        }
    }

    fn execute_candidates(
        ai: &Ai,
        state: &mut EncounterState,
        actor: usize,
        candidates: &[CandidateEvaluation],
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<bool> {
        for candidate in candidates {
            if candidate.score <= 0.0 && candidate.kind != CandidateKind::Wait {
                continue;
            }
            if candidate.kind == CandidateKind::Wait {
                return Ok(true);
            }
            match validate_candidate(ai, state, actor, candidate) {
                Ok(()) => {}
                Err(reason) => {
                    log.log(format_args!(
                        "tactical candidate skipped ({}): {reason}",
                        candidate.kind_label()
                    ));
                    continue;
                }
            }
            match execute_candidate(ai, state, actor, candidate, rng, log) {
                Ok(()) => return Ok(true),
                Err(error) => {
                    log.log(format_args!(
                        "tactical candidate failed ({}): {error:#}; trying next",
                        candidate.kind_label()
                    ));
                }
            }
        }
        Ok(false)
    }

    impl CandidateEvaluation {
        fn kind_label(&self) -> String {
            match &self.kind {
                CandidateKind::Action { action_id, .. } => action_id.clone(),
                CandidateKind::Move { purpose, .. } => format!("{purpose:?}"),
                CandidateKind::Interact { object_id, .. } => object_id.clone(),
                CandidateKind::Wait => "wait".to_string(),
            }
        }
    }

    fn validate_candidate(
        ai: &Ai,
        state: &EncounterState,
        actor: usize,
        candidate: &CandidateEvaluation,
    ) -> std::result::Result<(), String> {
        let plan = ai.plans.get(actor).ok_or("actor has no plan")?;
        match &candidate.kind {
            CandidateKind::Action { .. } => {
                if let Some(index) = candidate.action_index {
                    let action = plan.actions.get(index).ok_or("action no longer exists")?;
                    action_precondition(state, actor, plan, action)?;
                    if !target_still_legal(state, actor, action, candidate.target_index) {
                        return Err("target is no longer observable/legal".to_string());
                    }
                    if let Some(waypoint) = &candidate.waypoint {
                        if !movement_legal(state, actor, &waypoint.position) {
                            return Err("approach path is no longer legal".to_string());
                        }
                        if !action_legal_at_state(
                            state,
                            action,
                            actor,
                            candidate.target_index,
                            &waypoint.position,
                        ) {
                            return Err("action is no longer legal at the waypoint".to_string());
                        }
                    } else if !action_legal_at_state(
                        state,
                        action,
                        actor,
                        candidate.target_index,
                        &state.combatants[actor].position(),
                    ) {
                        return Err("action is no longer legal".to_string());
                    }
                } else if candidate.multiattack {
                    if state.combatants[actor].action_used {
                        return Err("action already used".to_string());
                    }
                    for action in &plan.multiattack {
                        if !action_legal_at_state(
                            state,
                            action,
                            actor,
                            candidate.target_index,
                            &state.combatants[actor].position(),
                        ) {
                            return Err("multiattack entry is no longer legal".to_string());
                        }
                        if !affordable(state, actor, action) {
                            return Err("multiattack entry is no longer affordable".to_string());
                        }
                    }
                }
                Ok(())
            }
            CandidateKind::Move { to, .. } => {
                if movement_legal(state, actor, to) {
                    Ok(())
                } else {
                    Err("movement is no longer legal".to_string())
                }
            }
            CandidateKind::Interact { object_id, .. } => {
                if let Some(waypoint) = &candidate.waypoint {
                    if !movement_legal(state, actor, &waypoint.position) {
                        return Err("interaction approach is no longer legal".to_string());
                    }
                }
                if interaction_legal(state, actor, object_id) {
                    Ok(())
                } else {
                    Err("scene object is no longer reachable".to_string())
                }
            }
            CandidateKind::Wait => Ok(()),
        }
    }

    fn action_precondition(
        state: &EncounterState,
        actor: usize,
        plan: &CombatantPlan,
        action: &PlannedAction,
    ) -> std::result::Result<(), String> {
        match action.activation {
            Activation::Action if state.combatants[actor].action_used => {
                return Err("action already used".to_string())
            }
            Activation::Bonus if state.combatants[actor].bonus_action_used => {
                return Err("bonus action already used".to_string())
            }
            _ => {}
        }
        if !affordable(state, actor, action) {
            return Err("resource not affordable".to_string());
        }
        if action.cost.is_some() && action.role == AiRole::Damage && plan.best_at_will_damage > 0.0
        {
            let threshold = plan
                .params
                .resource_threshold
                .unwrap_or(super::DEFAULT_RESOURCE_THRESHOLD);
            if action.raw_damage + 1.0e-9 < threshold * plan.best_at_will_damage {
                return Err("resource threshold".to_string());
            }
        }
        Ok(())
    }

    fn target_still_legal(
        state: &EncounterState,
        actor: usize,
        action: &PlannedAction,
        target: Option<usize>,
    ) -> bool {
        let Some(target) = target else {
            return true;
        };
        if target == actor || action.target_type == TargetType::SelfTarget {
            return true;
        }
        let combatant = &state.combatants[target];
        if combatant.dormant || combatant.is_dead() {
            return false;
        }
        if action.role == AiRole::Heal && combatant.hp <= 0 {
            // `heal_downed` is a profile gate applied during generation; a
            // target that went down after planning is still a valid rescue.
            return true;
        }
        if combatant.hp <= 0 {
            return false;
        }
        true
    }

    fn action_legal_at_state(
        state: &EncounterState,
        action: &PlannedAction,
        actor: usize,
        target: Option<usize>,
        from: &Position,
    ) -> bool {
        let Some(target) = target else {
            return true;
        };
        if target == actor || action.target_type == TargetType::SelfTarget {
            return true;
        }
        let to = state.combatants[target].position();
        if let Some(space) = state.space() {
            let Some(feet) = space.distance(from, &to) else {
                return false;
            };
            let in_range = match action.attack_kind {
                Some(AttackKind::Melee) => space.can_reach(from, &to, action.reach),
                Some(AttackKind::Ranged) => {
                    action.attack_range.is_some_and(|[_, long]| feet <= long)
                }
                Some(AttackKind::MeleeOrRanged) => {
                    space.can_reach(from, &to, action.reach)
                        || action.attack_range.is_some_and(|[_, long]| feet <= long)
                }
                None => action.range.is_none_or(|range| range <= 0 || feet <= range),
            };
            if !in_range {
                return false;
            }
            if action.target_type != TargetType::Zone
                && space.cover(from, &to) == Cover::Total
                && !action_ignores_cover(action)
            {
                return false;
            }
        }
        true
    }

    fn execute_candidate(
        ai: &Ai,
        state: &mut EncounterState,
        actor: usize,
        candidate: &CandidateEvaluation,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        if let Some(waypoint) = &candidate.waypoint {
            move_to(state, actor, &waypoint.position, rng, log)?;
        }
        match &candidate.kind {
            CandidateKind::Action { .. } => {
                if candidate.multiattack {
                    let target = candidate
                        .target_index
                        .ok_or_else(|| anyhow::anyhow!("multiattack candidate has no target"))?;
                    return engine::attack_at(
                        state,
                        actor,
                        target,
                        "",
                        &engine::AttackOptions {
                            multi: true,
                            ..engine::AttackOptions::default()
                        },
                        rng,
                        log,
                    );
                }
                let index = candidate
                    .action_index
                    .ok_or_else(|| anyhow::anyhow!("action candidate has no action"))?;
                let plan = &ai.plans[actor];
                let action = &plan.actions[index];
                let target = candidate
                    .target_index
                    .ok_or_else(|| anyhow::anyhow!("action candidate has no target"))?;
                if !action_legal_at_state(
                    state,
                    action,
                    actor,
                    Some(target),
                    &state.combatants[actor].position(),
                ) {
                    bail!("`{}` is no longer legal at this position", action.id);
                }
                ai.execute(state, actor, action, target, candidate.option, rng, log)
            }
            CandidateKind::Move { to, .. } => move_to(state, actor, to, rng, log),
            CandidateKind::Interact { object_id, .. } => {
                if !interaction_legal(state, actor, object_id) {
                    bail!("scene object `{object_id}` is no longer reachable");
                }
                let name = state.combatants[actor].name.clone();
                engine::interact(state, &name, object_id, None, rng, log)
            }
            CandidateKind::Wait => Ok(()),
        }
    }

    fn move_to(
        state: &mut EncounterState,
        actor: usize,
        position: &Position,
        rng: &mut dyn DiceRng,
        log: &mut dyn LogSink,
    ) -> Result<()> {
        let target = match position {
            Position::Cell { x, y } => engine::MoveTarget::Cell(*x, *y),
            Position::Zone { zone } => engine::MoveTarget::Zone(zone.clone()),
        };
        engine::move_to_at(state, actor, &target, false, rng, log)
    }

    fn movement_legal(state: &EncounterState, actor: usize, position: &Position) -> bool {
        let Some(space) = state.space() else {
            return true;
        };
        let start = state.combatants[actor].position();
        let blocked: Vec<Position> = state
            .combatants
            .iter()
            .enumerate()
            .filter(|(index, combatant)| *index != actor && !combatant.is_dead())
            .map(|(_, combatant)| combatant.position())
            .collect();
        space
            .route_cost(&start, position, &blocked)
            .is_some_and(|cost| cost <= state.combatants[actor].speed_left)
    }

    fn interaction_legal(state: &EncounterState, actor: usize, object_id: &str) -> bool {
        let Some(scene) = state.scene.as_deref() else {
            return false;
        };
        let Some(object) = scene
            .objects
            .iter()
            .find(|object| object.id.eq_ignore_ascii_case(object_id))
        else {
            return false;
        };
        if object.interact.is_none() {
            return false;
        }
        if scene.mode == SceneMode::Grid {
            let actor_zone = state
                .space()
                .and_then(|space| space.zone_of(&state.combatants[actor].position()));
            if actor_zone.as_deref() != Some(object.zone.as_str()) {
                return false;
            }
        }
        true
    }

    fn action_ignores_cover(action: &PlannedAction) -> bool {
        action.effects.iter().any(|effect| match effect {
            PlanEffect::Save { ignore_cover, .. } => *ignore_cover,
            _ => false,
        })
    }

    fn has_objective_effect(interact: &Interact) -> bool {
        fn effects_have(effects: &[Effect]) -> bool {
            effects.iter().any(|effect| match effect {
                Effect::ToggleTrigger(_) | Effect::Activate(_) => true,
                Effect::Choice(options) => options.iter().any(|option| effects_have(option)),
                _ => false,
            })
        }
        if let Some(effects) = &interact.effects {
            if effects_have(effects) {
                return true;
            }
        }
        if let Some(effects) = &interact.on_success {
            if effects_have(effects) {
                return true;
            }
        }
        if let Some(effects) = &interact.on_fail {
            if effects_have(effects) {
                return true;
            }
        }
        interact
            .options
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|option| {
                option.effects.as_deref().is_some_and(effects_have)
                    || option.on_success.as_deref().is_some_and(effects_have)
                    || option.on_fail.as_deref().is_some_and(effects_have)
            })
    }

    fn unsupported_effects(action: &PlannedAction) -> Option<String> {
        struct Support {
            supported: bool,
            unsupported: Vec<&'static str>,
        }
        fn walk(effects: &[Effect], support: &mut Support) {
            for effect in effects {
                match effect {
                    Effect::Attack(effect) => {
                        support.supported = true;
                        walk(&effect.on_hit, support);
                        walk(&effect.on_miss, support);
                    }
                    Effect::Save(effect) => {
                        support.supported = true;
                        walk(&effect.on_fail, support);
                        walk(&effect.on_success, support);
                    }
                    Effect::Damage(_) | Effect::Heal(_) | Effect::ApplyCondition(_) => {
                        support.supported = true;
                    }
                    Effect::BasicAction(_) => support.supported = true,
                    Effect::Choice(options) => {
                        for option in options {
                            walk(option, support);
                        }
                    }
                    Effect::RemoveCondition(_) => support.unsupported.push("remove_condition"),
                    Effect::ForcedMove(_) => support.unsupported.push("forced_move"),
                    Effect::Use(_) => support.unsupported.push("use"),
                    Effect::Activate(_) => support.unsupported.push("activate"),
                    Effect::Check(_) => support.unsupported.push("check"),
                    Effect::Note(_) => support.unsupported.push("note"),
                    Effect::ToggleTrigger(_) => support.unsupported.push("toggle_trigger"),
                }
            }
        }
        let mut support = Support {
            supported: matches!(action.kind, PlannedKind::Attack(_)),
            unsupported: Vec::new(),
        };
        match &action.kind {
            PlannedKind::Attack(attack) => {
                walk(&attack.on_hit, &mut support);
            }
            PlannedKind::Ability(ability) => walk(&ability.ability.effects, &mut support),
        }
        if !support.supported {
            return Some(if support.unsupported.is_empty() {
                "no valued effect".to_string()
            } else {
                support.unsupported.join(", ")
            });
        }
        None
    }

    fn action_consistency(
        state: &EncounterState,
        actor: usize,
        action: &PlannedAction,
        target: usize,
        targeting: AiTargeting,
        profile: &BehaviourProfile,
        waypoint: Option<&Waypoint>,
    ) -> f64 {
        let mut value = target_preference(state, actor, target, targeting);
        if let Some(preference) = profile.range_preference {
            let ranged = action
                .attack_kind
                .is_some_and(|kind| kind != AttackKind::Melee)
                || (action.attack_kind.is_none() && action.range.is_some_and(|range| range > 5));
            value += match preference {
                RangePreference::Melee if !ranged => 0.5,
                RangePreference::Melee => -0.5,
                RangePreference::Ranged if ranged => 0.5,
                RangePreference::Ranged => -0.5,
                RangePreference::Any => 0.0,
            };
        }
        if profile.opening.iter().any(|id| id == &action.id) {
            value += 2.0;
        }
        if profile.heal_downed && action.role == AiRole::Heal && state.combatants[target].hp <= 0 {
            value += 2.0;
        }
        if profile
            .target
            .avoid_conditions
            .iter()
            .any(|condition| state.combatants[target].has_condition(&state.content, condition))
        {
            value -= 4.0;
        }
        if let Some(waypoint) = waypoint {
            if let Some(zone) = profile.guard_zone.as_deref() {
                if waypoint.position.zone_id() != Some(zone) {
                    value -= 3.0;
                }
            }
        }
        value
    }

    fn target_preference(
        state: &EncounterState,
        actor: usize,
        target: usize,
        targeting: AiTargeting,
    ) -> f64 {
        let space = state.space();
        let distance = distance(space.as_ref(), state, actor, target);
        match targeting {
            AiTargeting::Nearest => 1.0 / (1.0 + f64::from(distance) / 5.0),
            AiTargeting::LowestHp => 1.0 - f64::from(hp_percent(&state.combatants[target])) / 100.0,
            AiTargeting::HighestDamage => 0.5,
            AiTargeting::Random => 0.5,
        }
    }

    fn cover_value(
        state: &EncounterState,
        space: &crate::space::SpaceView<'_>,
        at: &Position,
        actor: usize,
    ) -> f64 {
        (0..state.combatants.len())
            .filter(|&index| {
                state.combatants[index].side != state.combatants[actor].side
                    && !state.combatants[index].dormant
                    && !state.combatants[index].is_dead()
                    && state.combatants[index].hp > 0
            })
            .map(
                |index| match space.cover(&state.combatants[index].position(), at) {
                    Cover::None => 0.0,
                    Cover::Half => 1.0,
                    Cover::ThreeQuarters => 2.0,
                    Cover::Total => 3.0,
                },
            )
            .fold(0.0, f64::max)
    }

    fn allies_near(state: &EncounterState, actor: usize, target: usize) -> usize {
        state
            .combatants
            .iter()
            .enumerate()
            .filter(|(index, combatant)| {
                *index != actor
                    && combatant.side == state.combatants[actor].side
                    && !combatant.dormant
                    && !combatant.is_dead()
                    && distance(state.space().as_ref(), state, *index, target) <= 5
            })
            .count()
    }

    fn is_unaware(state: &EncounterState, index: usize) -> bool {
        state.combatants[index].conditions.iter().any(|active| {
            state
                .content
                .conditions
                .get(&active.condition)
                .is_some_and(|definition| definition.flags.contains(&ConditionFlag::Unaware))
        })
    }

    fn neighbour_positions(
        space: &crate::space::SpaceView<'_>,
        position: &Position,
    ) -> Vec<Position> {
        if let Some((x, y)) = position.cell_coords() {
            NEIGHBOURS
                .iter()
                .map(|(dx, dy)| Position::cell(x + dx, y + dy))
                .filter(|position| space.contains(position))
                .collect()
        } else {
            Vec::new()
        }
    }

    fn toward_position(
        space: &crate::space::SpaceView<'_>,
        from: &Position,
        to: &Position,
        budget: i32,
    ) -> Option<Position> {
        let (x, y) = from.cell_coords()?;
        let (tx, ty) = to.cell_coords()?;
        let steps = (budget / 5).max(1);
        let dx = (tx - x).signum();
        let dy = (ty - y).signum();
        let distance = (tx - x).abs().max((ty - y).abs());
        let step = steps.min(distance);
        let position = Position::cell(x + dx * step, y + dy * step);
        space.contains(&position).then_some(position)
    }

    fn zone_of_position(
        space: &crate::space::SpaceView<'_>,
        position: &Position,
    ) -> Option<String> {
        match position {
            Position::Zone { zone } => Some(zone.clone()),
            Position::Cell { .. } => space.zone_of(position),
        }
    }

    fn same_zone(
        state: &EncounterState,
        space: &crate::space::SpaceView<'_>,
        actor: usize,
        zone: &str,
    ) -> bool {
        space
            .zone_of(&state.combatants[actor].position())
            .as_deref()
            == Some(zone)
    }

    const NEIGHBOURS: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
}
