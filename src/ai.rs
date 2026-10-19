//! Simple per-creature combat AI (SPEC §5).
//!
//! The AI keeps a precomputed plan per combatant so the simulation hot path can
//! evaluate attacks and heals without parsing dice expressions or allocating
//! strings. Step-mode-style calls use the regular engine entry points so their
//! log output stays readable; simulation calls use the quiet engine path.

use anyhow::Result;

use crate::dice::{DiceRng, RollMode};
use crate::engine::{self, AttackOptions, AttackSituation, LogSink, QuietAttack, QuietHeal};
use crate::model::{Attack, AttackKind, HealAction, Kind, Mastery};
use crate::state::CombatantState;

const AI_OPTIONS: AttackOptions = AttackOptions {
    advantage: false,
    disadvantage: false,
    no_sneak: false,
    no_reactions: false,
    multi: false,
};

/// Precomputed AI plans for an encounter.
#[derive(Debug, Clone)]
pub struct Ai {
    combatants: Vec<CombatantAi>,
}

#[derive(Debug, Clone)]
struct CombatantAi {
    attacks: Vec<AttackAi>,
    /// Indices into `attacks` for the multiattack sequence.
    multiattack: Option<Vec<usize>>,
    heals: Vec<QuietHeal>,
    bonus_heal: Option<usize>,
    action_heals: Vec<usize>,
}

#[derive(Debug, Clone)]
struct AttackAi {
    plan: QuietAttack,
    /// Damage estimates, one per combatant index.
    profiles: Vec<AttackProfile>,
}

/// Average damage figures for one attack against one target.
#[derive(Debug, Clone, Copy, Default)]
struct AttackProfile {
    hit: f64,
    crit: f64,
    vs_hit: f64,
    vs_crit: f64,
    graze: f64,
    sneak_hit: f64,
    sneak_crit: f64,
}

/// Range information derived from an attack entry.
#[derive(Debug, Clone, Copy)]
struct RangeProfile {
    reach: i32,
    normal: i32,
    long: i32,
    ranged_kind: bool,
}

impl RangeProfile {
    fn from_attack(attack: &Attack) -> Self {
        match attack.kind {
            AttackKind::Melee => {
                let reach = attack.reach.unwrap_or(5);
                let (normal, long) = attack
                    .range
                    .map(|range| (range[0], range[1]))
                    .unwrap_or((reach, reach));
                Self {
                    reach,
                    normal,
                    long,
                    ranged_kind: false,
                }
            }
            AttackKind::Ranged => {
                let (normal, long) = attack
                    .range
                    .map(|range| (range[0], range[1]))
                    .unwrap_or((0, 0));
                Self {
                    reach: 0,
                    normal,
                    long,
                    ranged_kind: true,
                }
            }
            AttackKind::SpellAttack => {
                if let Some(range) = attack.range {
                    Self {
                        reach: 0,
                        normal: range[0],
                        long: range[1],
                        ranged_kind: true,
                    }
                } else {
                    let reach = attack.reach.unwrap_or(5);
                    Self {
                        reach,
                        normal: reach,
                        long: reach,
                        ranged_kind: false,
                    }
                }
            }
            AttackKind::Save | AttackKind::Auto | AttackKind::Unknown => {
                let long = attack.range.map(|range| range[1]).unwrap_or(i32::MAX);
                Self {
                    reach: 0,
                    normal: long,
                    long,
                    ranged_kind: false,
                }
            }
        }
    }

    fn situation(&self, distance: i32) -> Option<AttackSituation> {
        if self.reach > 0 && distance <= self.reach {
            return Some(AttackSituation {
                ranged: false,
                long_range: false,
                distance,
            });
        }
        if distance <= self.long {
            return Some(AttackSituation {
                ranged: self.ranged_kind || self.reach > 0,
                long_range: distance > self.normal,
                distance,
            });
        }
        None
    }

    fn in_range(&self, distance: i32) -> bool {
        self.situation(distance).is_some()
    }
}

impl Ai {
    /// Builds plans by parsing every attack and heal expression once.
    pub fn new(state: &crate::state::EncounterState) -> Result<Self> {
        let mut combatants = Vec::with_capacity(state.combatants.len());
        for attacker in &state.combatants {
            let mut attacks = Vec::with_capacity(attacker.spec.attacks.len());
            for attack in &attacker.spec.attacks {
                let plan = QuietAttack::new(attack.clone(), &attacker.spec.features)?;
                let profiles = state
                    .combatants
                    .iter()
                    .map(|target| attack_profile(&plan, attacker, target))
                    .collect();
                attacks.push(AttackAi { plan, profiles });
            }

            let multiattack = if attacker.spec.multiattack.is_empty() {
                None
            } else {
                let mut indices = Vec::with_capacity(attacker.spec.multiattack.len());
                for name in &attacker.spec.multiattack {
                    match attacker
                        .spec
                        .attacks
                        .iter()
                        .position(|attack| attack.name.eq_ignore_ascii_case(name))
                    {
                        Some(index) => indices.push(index),
                        None => {
                            indices.clear();
                            break;
                        }
                    }
                }
                (!indices.is_empty()).then_some(indices)
            };

            let mut heals = Vec::with_capacity(attacker.spec.heals.len());
            for heal in &attacker.spec.heals {
                heals.push(QuietHeal::new(heal.clone())?);
            }
            let bonus_heal = attacker
                .spec
                .heals
                .iter()
                .position(|heal| heal.action == HealAction::Bonus);
            let action_heals: Vec<usize> = attacker
                .spec
                .heals
                .iter()
                .enumerate()
                .filter(|(_, heal)| heal.action == HealAction::Action)
                .map(|(index, _)| index)
                .collect();

            combatants.push(CombatantAi {
                attacks,
                multiattack,
                heals,
                bonus_heal,
                action_heals,
            });
        }
        Ok(Self { combatants })
    }

    /// Takes the turn of `state.current_index()`.
    pub fn take_turn<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        rng: &mut R,
        sink: &mut L,
    ) -> Result<()> {
        let Some(index) = state.current_index() else {
            return Ok(());
        };
        if index >= self.combatants.len() || !can_ai_act(&state.combatants[index]) {
            return Ok(());
        }

        self.bonus_self_heal(state, index, rng, sink)?;
        if self.action_heal(state, index, rng, sink)? {
            return Ok(());
        }

        let Some(target) = self.choose_target(state, index) else {
            return Ok(());
        };
        self.attack_target(state, index, target, rng, sink)
    }

    // -- healing ----------------------------------------------------------

    fn bonus_self_heal<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        index: usize,
        rng: &mut R,
        sink: &mut L,
    ) -> Result<bool> {
        let combatant = &state.combatants[index];
        if combatant.bonus_action_used || combatant.hp * 2 > combatant.spec.max_hp {
            return Ok(false);
        }
        let Some(heal_index) = self.combatants[index].bonus_heal else {
            return Ok(false);
        };
        if !self.heal_affordable(state, index, heal_index) {
            return Ok(false);
        }
        let plan = &self.combatants[index].heals[heal_index];
        self.heal_self(state, index, plan, rng, sink)?;
        Ok(true)
    }

    fn heal_self<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        index: usize,
        plan: &QuietHeal,
        rng: &mut R,
        sink: &mut L,
    ) -> Result<()> {
        if sink.enabled() {
            let name = state.combatants[index].name().to_string();
            engine::use_entry(state, &name, &plan.heal().name, None, rng, sink)?;
        } else {
            engine::quiet_use_heal(state, index, plan, rng)?;
        }
        Ok(())
    }

    fn action_heal<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        index: usize,
        rng: &mut R,
        sink: &mut L,
    ) -> Result<bool> {
        for &heal_index in &self.combatants[index].action_heals {
            let plan = &self.combatants[index].heals[heal_index];
            if plan.heal().self_only || !self.heal_affordable(state, index, heal_index) {
                continue;
            }
            let Some(ally) = choose_ally(state, index) else {
                continue;
            };
            let distance = engine::distance(state, index, ally);
            if distance > plan.heal().range {
                let speed = state.combatants[index].speed_left;
                if plan.heal().range <= 0 || distance > speed {
                    continue;
                }
                let zone = state.combatants[ally].zone.clone();
                self.move_to(state, index, &zone, sink)?;
            }
            if sink.enabled() {
                let caster = state.combatants[index].name().to_string();
                let target = state.combatants[ally].name().to_string();
                engine::cast(
                    state,
                    &caster,
                    &plan.heal().name,
                    &[target],
                    &AI_OPTIONS,
                    rng,
                    sink,
                )?;
            } else {
                engine::quiet_cast_heal(state, index, ally, plan, rng)?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn heal_affordable(
        &self,
        state: &crate::state::EncounterState,
        index: usize,
        heal_index: usize,
    ) -> bool {
        let heal = &self.combatants[index].heals[heal_index];
        let Some(cost) = &heal.heal().cost else {
            return true;
        };
        let Ok(amount) = i32::try_from(cost.amount) else {
            return false;
        };
        state.combatants[index]
            .resources
            .get(&cost.resource)
            .is_some_and(|resource| resource.current >= amount)
    }

    // -- targeting --------------------------------------------------------

    fn choose_target(&self, state: &crate::state::EncounterState, index: usize) -> Option<usize> {
        let attacker = &state.combatants[index];
        let mut conscious: Vec<usize> = (0..state.combatants.len())
            .filter(|&other| {
                other != index
                    && state.combatants[other].side() != attacker.side()
                    && state.combatants[other].is_conscious()
            })
            .collect();
        sort_targets(state, index, &mut conscious);

        if let Some(&first) = conscious.first() {
            let reachable = conscious
                .iter()
                .any(|&other| self.any_attack_in_range(state, index, other));
            if reachable || attacker.kind() != Kind::Monster {
                return Some(first);
            }
        }

        if attacker.kind() == Kind::Monster {
            let mut downed: Vec<usize> = (0..state.combatants.len())
                .filter(|&other| {
                    other != index
                        && state.combatants[other].side() != attacker.side()
                        && state.combatants[other].hp <= 0
                        && !state.combatants[other].death_saves.dead
                })
                .collect();
            sort_targets(state, index, &mut downed);
            if let Some(&first) = downed.first() {
                return Some(first);
            }
        }

        conscious.first().copied()
    }

    fn any_attack_in_range(
        &self,
        state: &crate::state::EncounterState,
        attacker: usize,
        target: usize,
    ) -> bool {
        let distance = engine::distance(state, attacker, target);
        self.combatants[attacker]
            .attacks
            .iter()
            .any(|attack| RangeProfile::from_attack(attack.plan.attack()).in_range(distance))
    }

    // -- attacking --------------------------------------------------------

    fn attack_target<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        attacker: usize,
        target: usize,
        rng: &mut R,
        sink: &mut L,
    ) -> Result<()> {
        let mut distance = engine::distance(state, attacker, target);
        if !self.any_attack_in_range(state, attacker, target) {
            let zone_distance = distance;
            let speed = state.combatants[attacker].speed_left;
            if zone_distance <= speed && zone_distance > 0 {
                let zone = state.combatants[target].zone.clone();
                self.move_to(state, attacker, &zone, sink)?;
                distance = 0;
            } else {
                self.dash_toward(state, attacker, target, sink)?;
                return Ok(());
            }
        }

        let Some(choice) = self.choose_attack(state, attacker, target, distance) else {
            return Ok(());
        };
        self.resolve_choice(state, attacker, target, choice, rng, sink)
    }

    fn choose_attack(
        &self,
        state: &crate::state::EncounterState,
        attacker: usize,
        target: usize,
        distance: i32,
    ) -> Option<AttackChoice> {
        let combatant = &self.combatants[attacker];
        let mut at_will: Option<(f64, AttackChoice)> = None;
        let mut resource: Option<(f64, AttackChoice)> = None;

        for (index, attack) in combatant.attacks.iter().enumerate() {
            let Some(score) = self.score_attack(state, attacker, target, index, distance) else {
                continue;
            };
            let choice = AttackChoice::Single(index);
            let slot = if attack.plan.attack().cost.is_some() {
                if !self.cost_affordable(state, attacker, index) {
                    continue;
                }
                &mut resource
            } else {
                &mut at_will
            };
            if slot.as_ref().is_none_or(|(best, _)| score > *best) {
                *slot = Some((score, choice));
            }
        }

        if let Some(sequence) = &combatant.multiattack {
            let mut total = 0.0;
            let mut usable = true;
            let mut has_cost = false;
            for &index in sequence {
                let Some(score) = self.score_attack(state, attacker, target, index, distance)
                else {
                    usable = false;
                    break;
                };
                if combatant.attacks[index].plan.attack().cost.is_some() {
                    has_cost = true;
                    if !self.cost_affordable(state, attacker, index) {
                        usable = false;
                        break;
                    }
                }
                total += score;
            }
            if usable {
                let slot = if has_cost {
                    &mut resource
                } else {
                    &mut at_will
                };
                if slot.as_ref().is_none_or(|(best, _)| total > *best) {
                    *slot = Some((total, AttackChoice::Multi));
                }
            }
        }

        let best_at_will = at_will.as_ref().map(|(score, _)| *score);
        let allowed_resource = resource
            .as_ref()
            .filter(|(score, _)| best_at_will.is_none_or(|best| *score >= best * 1.5))
            .map(|(score, choice)| (*score, choice.clone()));

        match (at_will, allowed_resource) {
            (Some((score, choice)), Some((resource_score, resource_choice))) => {
                if resource_score > score {
                    Some(resource_choice)
                } else {
                    Some(choice)
                }
            }
            (Some((_, choice)), None) => Some(choice),
            (None, Some((_, choice))) => Some(choice),
            (None, None) => None,
        }
    }

    fn score_attack(
        &self,
        state: &crate::state::EncounterState,
        attacker: usize,
        target: usize,
        attack_index: usize,
        distance: i32,
    ) -> Option<f64> {
        let attack = &self.combatants[attacker].attacks[attack_index];
        let entry = attack.plan.attack();
        let range = RangeProfile::from_attack(entry);
        let situation = range.situation(distance)?;
        Some(expected_damage(
            state, attacker, target, attack, entry, &situation,
        ))
    }

    fn cost_affordable(
        &self,
        state: &crate::state::EncounterState,
        attacker: usize,
        attack_index: usize,
    ) -> bool {
        let Some(cost) = &self.combatants[attacker].attacks[attack_index]
            .plan
            .attack()
            .cost
        else {
            return true;
        };
        let Ok(amount) = i32::try_from(cost.amount) else {
            return false;
        };
        state.combatants[attacker]
            .resources
            .get(&cost.resource)
            .is_some_and(|resource| resource.current >= amount)
    }

    fn resolve_choice<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        attacker: usize,
        target: usize,
        choice: AttackChoice,
        rng: &mut R,
        sink: &mut L,
    ) -> Result<()> {
        match choice {
            AttackChoice::Single(index) => {
                let plan = &self.combatants[attacker].attacks[index].plan;
                if sink.enabled() {
                    let attacker_name = state.combatants[attacker].name().to_string();
                    let target_name = state.combatants[target].name().to_string();
                    engine::attack(
                        state,
                        &attacker_name,
                        &target_name,
                        &plan.attack().name,
                        &AI_OPTIONS,
                        rng,
                        sink,
                    )?;
                } else {
                    engine::quiet_attack(state, attacker, target, plan, &AI_OPTIONS, rng)?;
                }
            }
            AttackChoice::Multi => {
                let sequence = self.combatants[attacker]
                    .multiattack
                    .as_ref()
                    .expect("multiattack choice requires a sequence");
                if sink.enabled() {
                    let attacker_name = state.combatants[attacker].name().to_string();
                    let target_name = state.combatants[target].name().to_string();
                    engine::multiattack(
                        state,
                        &attacker_name,
                        &target_name,
                        &AI_OPTIONS,
                        rng,
                        sink,
                    )?;
                } else {
                    for &index in sequence {
                        if state.combatants[target].death_saves.dead {
                            break;
                        }
                        let plan = &self.combatants[attacker].attacks[index].plan;
                        engine::quiet_attack(state, attacker, target, plan, &AI_OPTIONS, rng)?;
                    }
                }
            }
        }
        Ok(())
    }

    // -- movement ---------------------------------------------------------

    fn move_to<L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        index: usize,
        zone: &str,
        sink: &mut L,
    ) -> Result<()> {
        if sink.enabled() {
            let name = state.combatants[index].name().to_string();
            engine::move_to(state, &name, zone, sink)?;
        } else {
            engine::quiet_move_to(state, index, zone)?;
        }
        Ok(())
    }

    fn dash_toward<L: LogSink + ?Sized>(
        &self,
        state: &mut crate::state::EncounterState,
        attacker: usize,
        target: usize,
        sink: &mut L,
    ) -> Result<bool> {
        let speed = state.combatants[attacker].speed_left;
        if speed <= 0 {
            return Ok(false);
        }
        let current_position = zone_position(state, &state.combatants[attacker].zone);
        let target_position = zone_position(state, &state.combatants[target].zone);
        let current_distance = (current_position - target_position).abs();

        let mut best: Option<(i32, i32, String)> = None;
        for (zone, position) in &state.zones {
            let cost = (position - current_position).abs();
            if cost > speed {
                continue;
            }
            let new_distance = (position - target_position).abs();
            if new_distance >= current_distance {
                continue;
            }
            let better = match &best {
                None => true,
                Some((best_distance, best_cost, _)) => {
                    new_distance < *best_distance
                        || (new_distance == *best_distance && cost < *best_cost)
                }
            };
            if better {
                best = Some((new_distance, cost, zone.clone()));
            }
        }

        if let Some((_, _, zone)) = best {
            self.move_to(state, attacker, &zone, sink)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Convenience entry point that builds plans for one call.
pub fn take_turn<R: DiceRng + ?Sized, L: LogSink + ?Sized>(
    state: &mut crate::state::EncounterState,
    rng: &mut R,
    sink: &mut L,
) -> Result<()> {
    let ai = Ai::new(state)?;
    ai.take_turn(state, rng, sink)
}

#[derive(Debug, Clone)]
enum AttackChoice {
    Single(usize),
    Multi,
}

fn can_ai_act(combatant: &CombatantState) -> bool {
    combatant.hp > 0
        && !combatant.death_saves.dead
        && !combatant.has_condition("unconscious")
        && !combatant.has_condition("paralyzed")
        && !combatant.has_condition("incapacitated")
}

fn zone_position(state: &crate::state::EncounterState, zone: &str) -> i32 {
    state.zones.get(zone).copied().unwrap_or(0)
}

fn sort_targets(state: &crate::state::EncounterState, attacker: usize, targets: &mut [usize]) {
    targets.sort_by_key(|&target| {
        (
            engine::distance(state, attacker, target),
            state.combatants[target].hp,
            target,
        )
    });
}

fn choose_ally(state: &crate::state::EncounterState, index: usize) -> Option<usize> {
    let side = state.combatants[index].side();
    let mut allies: Vec<usize> = (0..state.combatants.len())
        .filter(|&other| {
            other != index
                && state.combatants[other].side() == side
                && !state.combatants[other].death_saves.dead
                && (state.combatants[other].hp <= 0
                    || state.combatants[other].hp * 10 <= state.combatants[other].spec.max_hp * 3)
        })
        .collect();
    allies.sort_by_key(|&other| {
        (
            if state.combatants[other].hp <= 0 {
                0
            } else {
                1
            },
            state.combatants[other].hp,
            other,
        )
    });
    allies.first().copied()
}

fn attack_profile(
    plan: &QuietAttack,
    attacker: &CombatantState,
    target: &CombatantState,
) -> AttackProfile {
    let attack = plan.attack();
    let hit = parts_damage(plan, target, false, false);
    let crit = parts_damage(plan, target, true, false);
    let (vs_hit, vs_crit) = if attack
        .vs_condition
        .as_ref()
        .is_some_and(|vs| vs.damage.is_some())
    {
        (
            parts_damage(plan, target, false, true),
            parts_damage(plan, target, true, true),
        )
    } else {
        (hit, crit)
    };
    let graze = if attack.mastery == Some(Mastery::Graze) {
        let flat = (attack.to_hit.unwrap_or(0) - attacker.spec.proficiency_bonus).max(0) as f64;
        flat * defense_multiplier(target, attack.damage_type.as_deref())
    } else {
        0.0
    };
    let (sneak_hit, sneak_crit) = match plan.sneak() {
        Some(sneak) => {
            let multiplier = defense_multiplier(target, attack.damage_type.as_deref());
            (
                sneak.average() * multiplier,
                sneak.average_doubled() * multiplier,
            )
        }
        None => (0.0, 0.0),
    };
    AttackProfile {
        hit,
        crit,
        vs_hit,
        vs_crit,
        graze,
        sneak_hit,
        sneak_crit,
    }
}

fn parts_damage(
    plan: &QuietAttack,
    target: &CombatantState,
    critical: bool,
    vs_active: bool,
) -> f64 {
    let attack = plan.attack();
    let base = if vs_active {
        plan.vs_base().or_else(|| plan.base())
    } else {
        plan.base()
    };
    let mut total = 0.0;
    if let Some(base) = base {
        let value = if critical {
            base.average_doubled()
        } else {
            base.average()
        };
        total += value * defense_multiplier(target, attack.damage_type.as_deref());
    }
    for (expression, damage_type) in plan.extras() {
        let value = if critical {
            expression.average_doubled()
        } else {
            expression.average()
        };
        total += value * defense_multiplier(target, damage_type.as_deref());
    }
    total
}

fn defense_multiplier(target: &CombatantState, damage_type: Option<&str>) -> f64 {
    let Some(damage_type) = damage_type else {
        return 1.0;
    };
    if target
        .spec
        .immunities
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type))
    {
        return 0.0;
    }
    let resistant = target
        .spec
        .resistances
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    let vulnerable = target
        .spec
        .vulnerabilities
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    match (resistant, vulnerable) {
        (true, true) => 1.0,
        (true, false) => 0.5,
        (false, true) => 2.0,
        (false, false) => 1.0,
    }
}

fn expected_damage(
    state: &crate::state::EncounterState,
    attacker: usize,
    target: usize,
    attack: &AttackAi,
    entry: &Attack,
    situation: &AttackSituation,
) -> f64 {
    let profile = &attack.profiles[target];
    let target_state = &state.combatants[target];
    let count = f64::from(entry.count.max(1));

    let vs_active = entry
        .vs_condition
        .as_ref()
        .is_some_and(|vs| target_state.has_condition(&vs.condition));
    let (hit_damage, crit_damage) = if vs_active {
        (profile.vs_hit, profile.vs_crit)
    } else {
        (profile.hit, profile.crit)
    };

    match entry.kind {
        AttackKind::Auto => count * hit_damage,
        AttackKind::Save => {
            let Some(save) = &entry.save else {
                return 0.0;
            };
            let fail_chance = save_fail_chance(target_state, &save.ability, save.dc);
            let on_fail = fail_chance * hit_damage;
            let on_success = if save.half_on_success {
                (1.0 - fail_chance) * hit_damage * 0.5
            } else {
                0.0
            };
            count * (on_fail + on_success)
        }
        _ => {
            let mode =
                engine::attack_roll_mode(state, attacker, target, entry, &AI_OPTIONS, situation).0;
            let (mut hit_chance, mut crit_chance) =
                hit_probabilities(entry.to_hit.unwrap_or(0), target_state.ac(), mode);
            let auto_crit = situation.distance <= 5
                && (target_state.has_condition("paralyzed")
                    || target_state.has_condition("unconscious"));
            if auto_crit {
                crit_chance = hit_chance;
            }
            if hit_chance < crit_chance {
                hit_chance = crit_chance;
            }
            let mut hit_damage = hit_damage;
            let mut crit_damage = crit_damage;
            if engine::sneak_qualifies(state, attacker, target, entry, mode, false)
                && attack.plan.sneak().is_some()
            {
                hit_damage += profile.sneak_hit;
                crit_damage += profile.sneak_crit;
            }
            let miss_chance = (1.0 - hit_chance).max(0.0);
            let non_crit = (hit_chance - crit_chance).max(0.0);
            count * (non_crit * hit_damage + crit_chance * crit_damage)
                + count * miss_chance * profile.graze
        }
    }
}

fn hit_probabilities(to_hit: i32, ac: i32, mode: RollMode) -> (f64, f64) {
    let mut hits = 0_i32;
    for die in 1..=20 {
        if die == 1 {
            continue;
        }
        if die == 20 || die + to_hit >= ac {
            hits += 1;
        }
    }
    let normal = f64::from(hits) / 20.0;
    let (hit_chance, crit_chance) = match mode {
        RollMode::Normal => (normal, 1.0 / 20.0),
        RollMode::Advantage => {
            let miss = 1.0 - normal;
            (1.0 - miss * miss, 1.0 - (19.0 / 20.0) * (19.0 / 20.0))
        }
        RollMode::Disadvantage => (normal * normal, 1.0 / 400.0),
    };
    (hit_chance.max(crit_chance), crit_chance)
}

fn save_fail_chance(target: &CombatantState, ability: &str, dc: i32) -> f64 {
    let auto_fail = matches!(ability.to_ascii_lowercase().as_str(), "str" | "dex")
        && (target.hp <= 0
            || target.has_condition("paralyzed")
            || target.has_condition("unconscious"));
    if auto_fail {
        return 1.0;
    }
    let bonus = target.save_bonus(ability);
    let mut fails = 0_i32;
    for die in 1..=20 {
        if die + bonus < dc {
            fails += 1;
        }
    }
    f64::from(fails) / 20.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::engine;
    use crate::model::{Cost, Heal, HealAction, ResetType, Resource, Side};
    use crate::state::{CombatantState, EncounterState};

    struct FixedRng(i32);

    impl DiceRng for FixedRng {
        fn roll_die(&mut self, sides: u32) -> u32 {
            (self.0.max(1) as u32).min(sides)
        }
    }

    fn spec(name: &str, side: Side, kind: Kind, hp: i32, ac: i32) -> crate::model::CombatantSpec {
        let mut abilities = BTreeMap::new();
        for ability in ["str", "dex", "con", "int", "wis", "cha"] {
            abilities.insert(ability.to_string(), 10);
        }
        crate::model::CombatantSpec {
            name: name.to_string(),
            side,
            kind,
            hp,
            max_hp: hp,
            temp_hp: 0,
            ac,
            init_bonus: 0,
            speed: 30,
            proficiency_bonus: 2,
            abilities,
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

    fn melee(name: &str, to_hit: i32, damage: &str, damage_type: &str) -> Attack {
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

    fn state_of(specs: Vec<crate::model::CombatantSpec>) -> EncounterState {
        let combatants: Vec<CombatantState> = specs
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

    fn set_current(state: &mut EncounterState, index: usize) {
        state.initiative = vec![index];
        state.turn = 0;
        state.round = 1;
        state.combatants[index].speed_left = state.combatants[index].effective_speed();
    }

    #[test]
    fn bonus_self_heal_triggers_at_half_hp() {
        let mut hero = spec("Will", Side::Party, Kind::Pc, 12, 18);
        hero.heals.push(Heal {
            name: "second_wind".to_string(),
            amount: "10".to_string(),
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
            Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 11, 16);
        let mut state = state_of(vec![hero, enemy]);
        state.combatants[0].hp = 4;
        set_current(&mut state, 0);

        let mut rng = FixedRng(5);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();

        assert_eq!(state.combatants[0].hp, 12);
        assert!(state.combatants[0].bonus_action_used);
        assert_eq!(state.combatants[0].resources["second_wind"].current, 0);
    }

    #[test]
    fn healer_revives_a_downed_ally_instead_of_attacking() {
        let mut healer = spec("Sera", Side::Party, Kind::Pc, 10, 18);
        healer.heals.push(Heal {
            name: "cure_wounds".to_string(),
            amount: "10".to_string(),
            range: 5,
            action: HealAction::Action,
            cost: Some(Cost {
                resource: "slot1".to_string(),
                amount: 1,
                notes: None,
            }),
            self_only: false,
            notes: None,
        });
        healer.resources.insert(
            "slot1".to_string(),
            Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let ally = spec("Will", Side::Party, Kind::Pc, 12, 18);
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 11, 16);
        let mut state = state_of(vec![healer, ally, enemy]);
        state.combatants[1].hp = 0;
        set_current(&mut state, 0);

        let mut rng = FixedRng(5);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();

        assert_eq!(state.combatants[1].hp, 10);
        assert!(state.combatants[0].action_used);
        assert_eq!(state.combatants[0].resources["slot1"].current, 0);
        assert_eq!(state.combatants[2].hp, 11, "healer should not attack");
    }

    #[test]
    fn targets_the_nearest_enemy_first() {
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 20, 18);
        let mut attack = melee("sword", 20, "5", "slashing");
        attack.range = Some([60, 120]);
        hero.attacks.push(attack);
        let near = spec("Near", Side::Enemy, Kind::Monster, 30, 10);
        let far = spec("Far", Side::Enemy, Kind::Monster, 30, 10);
        let mut state = state_of(vec![hero, near, far]);
        state.zones.insert("near".to_string(), 10);
        state.zones.insert("far".to_string(), 60);
        state.combatants[1].zone = "near".to_string();
        state.combatants[2].zone = "far".to_string();
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();

        assert_eq!(state.combatants[1].hp, 25);
        assert_eq!(state.combatants[2].hp, 30);
    }

    #[test]
    fn targets_the_lowest_hp_when_distance_ties() {
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 20, 18);
        hero.attacks.push(melee("sword", 20, "5", "slashing"));
        let healthy = spec("Healthy", Side::Enemy, Kind::Monster, 30, 10);
        let wounded = spec("Wounded", Side::Enemy, Kind::Monster, 8, 10);
        let mut state = state_of(vec![hero, healthy, wounded]);
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();

        assert_eq!(state.combatants[2].hp, 3);
        assert_eq!(state.combatants[1].hp, 30);
    }

    #[test]
    fn resource_attack_only_when_strong_enough() {
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 20, 18);
        hero.attacks.push(melee("at_will", 20, "5", "slashing"));
        let mut resource_attack = melee("resource", 20, "4", "slashing");
        resource_attack.cost = Some(Cost {
            resource: "slot1".to_string(),
            amount: 1,
            notes: None,
        });
        hero.attacks.push(resource_attack);
        hero.resources.insert(
            "slot1".to_string(),
            Resource {
                current: 1,
                max: 1,
                reset: ResetType::Long,
                notes: None,
            },
        );
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 50, 10);
        let mut state = state_of(vec![hero, enemy]);
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(state.combatants[0].resources["slot1"].current, 1);
        assert_eq!(state.combatants[1].hp, 45);

        state.combatants[0].spec.attacks[1].damage = Some("20".to_string());
        state.combatants[0].action_used = false;
        set_current(&mut state, 0);
        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(state.combatants[0].resources["slot1"].current, 0);
        assert_eq!(state.combatants[1].hp, 25);
    }

    #[test]
    fn incapacitated_creatures_skip_their_turn() {
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 20, 18);
        hero.attacks.push(melee("sword", 20, "5", "slashing"));
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 11, 10);
        let mut state = state_of(vec![hero, enemy]);
        state.combatants[0]
            .conditions
            .push(crate::state::ActiveCondition {
                condition: "paralyzed".to_string(),
                source: None,
                escape_dc: None,
                duration: None,
                expires_round: None,
                ends_with_turn_of: None,
                notes: None,
            });
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();

        assert!(log.is_empty());
        assert_eq!(state.combatants[1].hp, 11);
        assert!(!state.combatants[0].action_used);
    }

    #[test]
    fn monsters_fall_back_to_downed_enemies_when_nothing_else_is_in_range() {
        let mut monster = spec("Monster", Side::Enemy, Kind::Monster, 30, 15);
        monster.attacks.push(melee("claw", 20, "5", "slashing"));
        let downed = spec("Downed", Side::Party, Kind::Pc, 12, 18);
        let far_hero = spec("Far", Side::Party, Kind::Pc, 12, 18);
        let mut state = state_of(vec![monster, downed, far_hero]);
        state.zones.insert("far".to_string(), 60);
        state.combatants[2].zone = "far".to_string();
        state.combatants[1].hp = 0;
        state.combatants[1]
            .conditions
            .push(crate::state::ActiveCondition {
                condition: "unconscious".to_string(),
                source: None,
                escape_dc: None,
                duration: None,
                expires_round: None,
                ends_with_turn_of: None,
                notes: None,
            });
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();

        // Auto-crit against an unconscious target within 5 ft: 2 failures.
        assert_eq!(state.combatants[1].death_saves.failures, 2);
        assert_eq!(state.combatants[2].hp, 12);
    }

    #[test]
    fn ranged_attackers_stay_put_while_melee_attackers_move() {
        let mut archer = spec("Archer", Side::Party, Kind::Pc, 20, 14);
        let mut bow = melee("bow", 20, "5", "piercing");
        bow.kind = AttackKind::Ranged;
        bow.reach = None;
        bow.range = Some([30, 120]);
        archer.attacks.push(bow);
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 30, 10);
        let mut state = state_of(vec![archer, enemy]);
        state.zones.insert("front".to_string(), 20);
        state.combatants[1].zone = "front".to_string();
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(state.combatants[0].zone, "main");
        assert_eq!(state.combatants[1].hp, 25);

        let mut fighter = spec("Fighter", Side::Party, Kind::Pc, 20, 18);
        fighter.attacks.push(melee("sword", 20, "5", "slashing"));
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 30, 10);
        let mut state = state_of(vec![fighter, enemy]);
        state.zones.insert("front".to_string(), 20);
        state.combatants[1].zone = "front".to_string();
        set_current(&mut state, 0);

        let mut rng = FixedRng(15);
        let mut log = Vec::new();
        take_turn(&mut state, &mut rng, &mut log).unwrap();
        assert_eq!(state.combatants[0].zone, "front");
        assert_eq!(state.combatants[1].hp, 25);
    }

    #[test]
    fn engine_and_quiet_paths_agree_on_basic_attacks() {
        let mut hero = spec("Hero", Side::Party, Kind::Pc, 20, 18);
        hero.attacks.push(melee("sword", 20, "5", "slashing"));
        let enemy = spec("Guard", Side::Enemy, Kind::Monster, 30, 10);

        let mut loud_state = state_of(vec![hero.clone(), enemy.clone()]);
        let mut quiet_state = state_of(vec![hero, enemy]);
        let mut sink = Vec::new();
        engine::attack(
            &mut loud_state,
            "Hero",
            "Guard",
            "sword",
            &AI_OPTIONS,
            &mut FixedRng(15),
            &mut sink,
        )
        .unwrap();
        let plan = QuietAttack::new(
            quiet_state.combatants[0].spec.attacks[0].clone(),
            &BTreeMap::new(),
        )
        .unwrap();
        engine::quiet_attack(
            &mut quiet_state,
            0,
            1,
            &plan,
            &AI_OPTIONS,
            &mut FixedRng(15),
        )
        .unwrap();
        assert_eq!(loud_state.combatants[1].hp, quiet_state.combatants[1].hp);
    }
}
