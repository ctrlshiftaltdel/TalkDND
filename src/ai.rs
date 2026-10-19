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

/// Prepared AI plans for every combatant of one encounter template.
pub struct Ai {
    plans: Vec<CombatantPlan>,
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
    tags: Vec<String>,
    slot_level: Option<i32>,
    raw_damage: f64,
}

#[derive(Clone)]
enum PlannedKind {
    Attack(Attack),
    Ability(ResolvedAbility),
}

#[derive(Clone)]
enum PlanEffect {
    Attack {
        effect: AttackEffect,
        damages: Vec<PlanDamage>,
        on_hit: Vec<PlanEffect>,
        on_miss: Vec<PlanEffect>,
        tags: Vec<String>,
        params: BTreeMap<String, ExprValue>,
        slot_level: Option<i32>,
        range: Option<i32>,
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
        let plans = state
            .combatants
            .iter()
            .enumerate()
            .map(|(index, _)| build_plan(state, index))
            .collect();
        Self { plans }
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
            if engine::has_condition_flag(state, actor, crate::content::schema::ConditionFlag::Incapacitated)
            {
                return Ok(());
            }
        }
        let plan = &self.plans[actor];
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
            if let Some(target) = self.choose_target(state, actor, plan, rng) {
                self.move_for_action(state, actor, plan, target, rng, log)?;
                self.take_action(state, actor, plan, target, rng, log)?;
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
                if !affordable(state, actor, action)
                    || !action_in_range(state, actor, action, ally)
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
            let target = if action.target_type == TargetType::SelfTarget {
                actor
            } else if action.target_side == Some(TargetSide::Ally) {
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
                                state.combatants[index]
                                    .has_condition(&state.content, condition)
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
                        distance(space.as_ref(), state, b, actor)
                            .cmp(&distance(space.as_ref(), state, a, actor))
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
        if state.combatants[actor].action_used {
            return Ok(());
        }
        let choice = self.choose_action(state, actor, plan, target);
        match choice {
            Some(TargetChoice::Action(index)) => {
                let action = &plan.actions[index];
                self.execute(state, actor, action, target, None, rng, log)
            }
            Some(TargetChoice::Multiattack) => {
                let actor_name = state.combatants[actor].name.clone();
                let target_name = state.combatants[target].name.clone();
                let fallback = plan
                    .multiattack
                    .first()
                    .map(|action| action.id.clone())
                    .unwrap_or_default();
                let _ = fallback;
                engine::attack(
                    state,
                    &actor_name,
                    &target_name,
                    "",
                    &AttackOptions {
                        multi: true,
                        ..AttackOptions::default()
                    },
                    rng,
                    log,
                )
            }
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

    fn resource_allows(&self, state: &EncounterState, plan: &CombatantPlan, action: &PlannedAction) -> bool {
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
        let mut score = match &action.kind {
            PlannedKind::Attack(attack) => {
                let forecast = engine::forecast_attack(state, actor, target, attack);
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
                    damage += (part.average + part.dice_average * forecast.crit_chance)
                        * multiplier;
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
            } => {
                let forecast = engine::forecast_attack_effect(
                    state,
                    actor,
                    target,
                    effect,
                    tags,
                    params,
                    *slot_level,
                    *range,
                );
                let hit = hit_chance(
                    forecast.to_hit,
                    forecast.ac,
                    forecast.advantage,
                    forecast.disadvantage,
                );
                let mut damage = 0.0;
                for part in damages {
                    let multiplier =
                        engine::damage_multiplier(state, target, Some(actor), part.dtype.as_str(), tags);
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
                fail_score.scaled(fail).add(success_score.scaled(1.0 - fail))
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
        let positions = space.area_positions(&center, action.area_radius);
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
            PlannedKind::Attack(attack) => {
                let actor_name = state.combatants[actor].name.clone();
                let target_name = state.combatants[target].name.clone();
                engine::attack(
                    state,
                    &actor_name,
                    &target_name,
                    &attack.id,
                    &AttackOptions::default(),
                    rng,
                    log,
                )
            }
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
            } else if adjacent && self.use_bonus_basic(state, actor, plan, BasicAction::Dash, rng, log)? {
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
        let Some(action) = self.best_action_for_position(state, actor, plan, target) else {
            return self.advance(state, actor, plan, Some(target), 5, false, rng, log);
        };
        let desired = action_distance(action);
        let wants_ranged = plan.params.range_pref == Some(RangePreference::Ranged)
            || (plan.params.range_pref != Some(RangePreference::Melee)
                && action.attack_range.is_some()
                && action.attack_kind != Some(AttackKind::Melee)
                && !action_in_range(state, actor, action, target));
        let keep = plan.params.keep_distance.unwrap_or(0);
        let nearest = nearest_hostile_distance(state, actor);
        if keep > 0 && wants_ranged && nearest < keep {
            if nearest <= 5 && !state.combatants[actor].bonus_action_used {
                let _ = self.use_bonus_basic(
                    state,
                    actor,
                    plan,
                    BasicAction::Disengage,
                    rng,
                    log,
                )?;
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
        target: usize,
    ) -> Option<&'a PlannedAction> {
        plan.actions
            .iter()
            .filter(|action| {
                action.activation == Activation::Action
                    && matches!(action.role, AiRole::Damage | AiRole::Control)
                    && affordable(state, actor, action)
                    && self.resource_allows(state, plan, action)
            })
            .max_by(|a, b| {
                self.score_action(state, actor, a, target)
                    .partial_cmp(&self.score_action(state, actor, b, target))
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
            .filter(|(index, combatant)| {
                *index != actor && !combatant.dormant && !combatant.is_dead() && combatant.hp > 0
            })
            .map(|(_, combatant)| combatant.position())
            .collect();
        let nearest = hostiles
            .iter()
            .filter_map(|position| space.distance(&start, position))
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
        let mut candidates: Vec<(i32, Position)> = Vec::new();
        for position in space.all_positions() {
            if position == start {
                candidates.push((0, position));
                continue;
            }
            if blocked.contains(&position) {
                continue;
            }
            let Some(route) = space.move_route(&start, &position, &blocked) else {
                continue;
            };
            let cost: i32 = route.iter().map(|step| step.cost).sum();
            if cost <= budget {
                candidates.push((cost, position));
            }
        }
        if candidates.is_empty() {
            return Ok(());
        }
        let mut best: Option<(f64, i32, Position)> = None;
        for (cost, position) in candidates {
            let zone = space.zone_of(&position);
            if !fleeing {
                if let Some(post) = guard_zone.as_deref() {
                    if nearest <= leash && !in_guard {
                        // Chasing is allowed, but do not wander away from the
                        // posted zone beyond the leash.
                    } else if zone.as_deref() != Some(post) {
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
                approach_score(keep, range_pref, min_enemy, distance_target, desired_distance)
                } else {
                    0.0
                }
            } else if target_position.is_some() {
                approach_score(keep, range_pref, min_enemy, distance_target, desired_distance)
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
        let name = state.combatants[actor].name.clone();
        let _ = rng;
        engine::move_to(state, &name, &move_target, false, rng, log)
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
            .filter(|(index, combatant)| {
                *index != actor && !combatant.dormant && !combatant.is_dead() && combatant.hp > 0
            })
            .map(|(_, combatant)| combatant.position())
            .collect();
        let mut best: Option<(i32, Position)> = None;
        for position in space.zone_positions(zone) {
            if position == start || blocked.contains(&position) {
                continue;
            }
            let Some(route) = space.move_route(&start, &position, &blocked) else {
                continue;
            };
            let cost: i32 = route.iter().map(|step| step.cost).sum();
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
        let name = state.combatants[actor].name.clone();
        engine::move_to(state, &name, &move_target, false, rng, log)
    }
}

fn approach_score(
    keep: i32,
    range_pref: Option<RangePreference>,
    min_enemy: i32,
    distance_target: i32,
    desired_distance: i32,
) -> f64 {
    if keep > 0
        && matches!(range_pref, Some(RangePreference::Ranged))
        && min_enemy < keep
    {
        return f64::from(min_enemy.min(keep + 10)) * 10.0
            - f64::from(distance_target) * 0.05;
    }
    if desired_distance > 5 {
        if distance_target > desired_distance {
            -f64::from(distance_target - desired_distance) * 10.0
                - f64::from(min_enemy) * 0.05
        } else if distance_target < keep.max(5) && keep > 0 {
            f64::from(min_enemy.min(keep + 10)) * 10.0
                - f64::from(distance_target) * 0.05
        } else {
            1000.0 - (f64::from(distance_target - desired_distance)).abs()
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
        if matches!(ability.activation, Activation::Passive | Activation::Reaction) {
            continue;
        }
        let role = ability.ai.as_ref().map(|ai| ai.role).unwrap_or(AiRole::None);
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
            kind: PlannedKind::Ability(resolved.clone()),
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
        .map(|part| plan_damage(state, actor, &part.dice, &part.damage_type, 1, &params, attack_mod, None))
        .collect();
    let on_hit = build_effects(state, actor, &attack.on_hit, &params, None, attack_mod, &[]);
    let raw_damage = attack
        .damage
        .iter()
        .map(|part| {
            plan_damage(state, actor, &part.dice, &part.damage_type, 1, &params, attack_mod, None)
                .average
        })
        .sum::<f64>()
        + raw_effect_damage(&on_hit);
    PlannedAction {
        id: attack.id.clone(),
        kind: PlannedKind::Attack(attack.clone()),
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
        tags: Vec::new(),
        slot_level: None,
        raw_damage,
    }
}

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
    let expression = engine::evaluate_for_actor(state, actor, value, params, attack_mod, slot_level)
        .ok();
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
        .map(|effect| {
            build_effect(
                state,
                actor,
                effect,
                params,
                slot_level,
                attack_mod,
                tags,
            )
        })
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
            effect: effect.clone(),
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
        },
        Effect::Save(effect) => {
            let dc = engine::average_expression(state, actor, &effect.dc, params, attack_mod, slot_level)
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
            PlanEffect::Damage(part) => {
                part.average * f64::from(part.repeat.max(1))
            }
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
        self_heal_below_pct: overrides
            .self_heal_below_pct
            .or(base.self_heal_below_pct),
        heal_ally_below_pct: overrides
            .heal_ally_below_pct
            .or(base.heal_ally_below_pct),
        heal_downed: overrides.heal_downed.or(base.heal_downed),
        resource_threshold: overrides.resource_threshold.or(base.resource_threshold),
        flee_below_pct: overrides.flee_below_pct.or(base.flee_below_pct),
        flee_zone: overrides.flee_zone.clone().or_else(|| base.flee_zone.clone()),
        guard_zone: overrides.guard_zone.clone().or_else(|| base.guard_zone.clone()),
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

fn action_in_range(state: &EncounterState, actor: usize, action: &PlannedAction, target: usize) -> bool {
    if action.target_type == TargetType::SelfTarget || target == actor {
        return true;
    }
    let Some(space) = state.space() else {
        return true;
    };
    let from = state.combatants[actor].position();
    let to = state.combatants[target].position();
    let Some(distance) = space.distance(&from, &to) else {
        return false;
    };
    match action.attack_kind {
        Some(AttackKind::Melee) => space.can_reach(&from, &to, action.reach),
        Some(AttackKind::Ranged) => action
            .attack_range
            .is_some_and(|[_, long]| distance <= long),
        Some(AttackKind::MeleeOrRanged) => {
            space.can_reach(&from, &to, action.reach)
                || action
                    .attack_range
                    .is_some_and(|[_, long]| distance <= long)
        }
        None => action.range.is_none_or(|range| range <= 0 || distance <= range),
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
            space.distance(
                &state.combatants[a].position(),
                &state.combatants[b].position(),
            )
        })
        .unwrap_or(9999)
}

fn nearest_hostile_distance(state: &EncounterState, actor: usize) -> i32 {
    let Some(space) = state.space() else {
        return 9999;
    };
    let position = state.combatants[actor].position();
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
        .filter_map(|(_, combatant)| space.distance(&position, &combatant.position()))
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
