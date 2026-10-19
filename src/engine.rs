//! Generic, data-driven 5e combat engine.
//!
//! Rules that every 5e game shares live here. Campaign-specific behaviour is
//! read from the content snapshot stored in [`EncounterState`]: conditions,
//! masteries, abilities, creature attacks and the core-rule wiring in
//! `rules`. No content ids or display names appear in this module.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, bail, Context, Result};

use crate::content::expr::ExprContext;
use crate::content::schema::{
    Ability, AbilityCost, AbilityName, Activation, Attack, AttackEffect, AttackKind,
    AttackProperty, BasicAction, CheckEffect, CheckWho, ConditionFlag, CritMode, DamageEffect,
    DamagePart, Direction, Duration, DurationUntil, Effect, EndsOn, ExprValue, ForcedMoveEffect,
    Modifier, ModifierApplies, ModifierEffect, ModifierFilter, OncePer, Predicate, Recharge,
    SaveEffect, SaveEndsAt, Size, TargetRef, TargetSide, TargetType, ToggleTriggerEffect, Trigger,
    TriggerWhen,
};
use crate::content::{ContentRegistry, ResolvedAbility};
use crate::dice::{seeded_rng, D20Roll, DiceExpr, DiceRng, RollMode, RollRecord};
use crate::space::Space;
use crate::state::{
    ability_from_text, active_condition_ids, active_duration, ActiveCondition, ActiveDuration,
    ActiveSaveEnds, CombatOutcome, CombatantState, DeathSaves, EncounterState,
};

/// Attack command options.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttackOptions {
    pub advantage: bool,
    pub disadvantage: bool,
    pub multi: bool,
    pub no_reactions: bool,
}

/// Ability/use command options.
#[derive(Debug, Clone, Default)]
pub struct UseOptions {
    pub targets: Vec<String>,
    pub option: Option<usize>,
    pub slot_level: Option<i32>,
    pub advantage: bool,
    pub disadvantage: bool,
    pub no_reactions: bool,
}

/// A character's condition-duration override supplied by the CLI.
#[derive(Debug, Clone, Default)]
pub struct AddConditionOptions {
    pub rounds: Option<u32>,
    pub minutes: Option<u32>,
    pub permanent: bool,
    pub until: Option<DurationUntil>,
    pub until_subject: Option<String>,
    pub source: Option<String>,
    pub linked: Option<String>,
    pub save_ends: Option<(AbilityName, i32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestKind {
    Short,
    Long,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn score_context(combatant: &CombatantState) -> ExprContext {
    let scores = combatant.creature.abilities.clone().unwrap_or_default();
    let mut context = ExprContext::new()
        .with_ability_scores(&scores)
        .with_prof(combatant.creature.proficiency_bonus())
        .with_level(combatant.creature.level());
    if let Some(spellcasting) = &combatant.creature.spellcasting {
        let modifier = combatant.creature.ability_modifier(spellcasting.ability);
        let attack = spellcasting
            .attack
            .unwrap_or(combatant.creature.proficiency_bonus() + modifier);
        let dc = spellcasting
            .dc
            .unwrap_or(8 + combatant.creature.proficiency_bonus() + modifier);
        context = context.with_spellcasting(Some(modifier), Some(attack), Some(dc));
    }
    context
}

fn source_context(
    state: &EncounterState,
    actor: usize,
    source: Option<&EffectSource>,
    attack_mod: Option<i32>,
    slot_level: Option<i32>,
) -> ExprContext {
    let mut context = score_context(&state.combatants[actor]);
    if let Some(source) = source {
        context = context.with_params(source.params.clone());
        if let Some(slot) = slot_level.or(source.slot_level) {
            context = context.with_slot_level(slot);
        }
    } else if let Some(slot) = slot_level {
        context = context.with_slot_level(slot);
    }
    if let Some(attack_mod) = attack_mod {
        context = context.with_attack_mod(attack_mod);
    }
    context
}

fn evaluate_value(context: &ExprContext, value: &ExprValue) -> Result<(String, DiceExpr)> {
    context.evaluate(&value.as_text())
}

fn roll_value(
    context: &ExprContext,
    value: &ExprValue,
    crit: bool,
    rng: &mut dyn DiceRng,
) -> Result<(String, RollRecord)> {
    let (formula, expression) = evaluate_value(context, value)?;
    let record = if crit {
        expression.roll_doubled(rng)
    } else {
        expression.roll(rng)
    };
    Ok((formula, record))
}

fn ability_name_from_attack_kind(
    kind: AttackKind,
    creature: &CombatantState,
) -> Option<AbilityName> {
    match kind {
        AttackKind::Melee => Some(AbilityName::Str),
        AttackKind::Ranged => Some(AbilityName::Dex),
        AttackKind::MeleeOrRanged => {
            if creature.creature.ability_modifier(AbilityName::Str)
                >= creature.creature.ability_modifier(AbilityName::Dex)
            {
                Some(AbilityName::Str)
            } else {
                Some(AbilityName::Dex)
            }
        }
    }
}

fn has_flag(state: &EncounterState, index: usize, flag: ConditionFlag) -> bool {
    let combatant = &state.combatants[index];
    active_condition_ids(&state.content, combatant)
        .iter()
        .any(|id| {
            state
                .content
                .conditions
                .get(id)
                .is_some_and(|def| def.flags.contains(&flag))
        })
}

fn is_incapacitated(state: &EncounterState, index: usize) -> bool {
    has_flag(state, index, ConditionFlag::Incapacitated)
}

fn ensure_can_act(state: &EncounterState, index: usize) -> Result<()> {
    let combatant = &state.combatants[index];
    if combatant.dormant {
        bail!(
            "`{}` is dormant and cannot act until activated",
            combatant.name
        );
    }
    if combatant.is_dead() {
        bail!("`{}` is dead", combatant.name);
    }
    if combatant.hp <= 0 {
        bail!("`{}` has 0 HP and cannot act", combatant.name);
    }
    if is_incapacitated(state, index) {
        bail!("`{}` is incapacitated and cannot act", combatant.name);
    }
    Ok(())
}

fn ensure_targetable(state: &EncounterState, index: usize) -> Result<()> {
    let combatant = &state.combatants[index];
    if combatant.dormant {
        bail!(
            "`{}` is dormant and cannot be targeted until activated",
            combatant.name
        );
    }
    if combatant.is_dead() {
        bail!("`{}` is already dead", combatant.name);
    }
    Ok(())
}

fn hostile_to(state: &EncounterState, a: usize, b: usize) -> bool {
    state.combatants[a].side != state.combatants[b].side
}

fn effective_speed(state: &EncounterState, index: usize) -> i32 {
    if has_flag(state, index, ConditionFlag::SpeedZero) {
        return 0;
    }
    let base = state.combatants[index].base_speed();
    let modifiers = collect_modifiers(
        state,
        index,
        ModifierApplies::Speed,
        &PredicateContext::for_owner(index),
        None,
    );
    let mut speed = base;
    for modifier in modifiers {
        if modifier.modifier.effect == ModifierEffect::Bonus {
            let context = score_context(&state.combatants[index]);
            if let Some(value) = &modifier.modifier.value {
                if let Ok((_, expression)) = evaluate_value(&context, value) {
                    speed += expression.modifier;
                }
            }
        }
    }
    speed.max(0)
}

fn size_rank(size: Size) -> u8 {
    match size {
        Size::Tiny => 0,
        Size::Small => 1,
        Size::Medium => 2,
        Size::Large => 3,
        Size::Huge => 4,
        Size::Gargantuan => 5,
        Size::Unknown => 2,
    }
}

fn check_size_limit(actual: Size, maximum: Option<Size>) -> bool {
    maximum.is_none_or(|maximum| size_rank(actual) <= size_rank(maximum))
}

// ---------------------------------------------------------------------------
// Predicates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AttackFacts {
    kind: AttackKind,
    properties: Vec<AttackProperty>,
    ability: Option<AbilityName>,
    tags: Vec<String>,
}

impl Default for AttackFacts {
    fn default() -> Self {
        Self {
            kind: AttackKind::Melee,
            properties: Vec::new(),
            ability: None,
            tags: Vec::new(),
        }
    }
}

impl AttackFacts {
    fn has_property(&self, property: AttackProperty) -> bool {
        if self.properties.contains(&property) {
            return true;
        }
        property == AttackProperty::Ranged && self.kind == AttackKind::Ranged
    }
}

#[derive(Debug, Clone)]
struct PredicateContext {
    /// Owner of the modifier/condition being evaluated.
    self_idx: usize,
    /// Creature performing the current action.
    actor: usize,
    target: Option<usize>,
    attacker: Option<usize>,
    linked: Option<String>,
    attack: AttackFacts,
    has_advantage: bool,
    has_disadvantage: bool,
    damage_dealt: i32,
    slot_level: Option<i32>,
    current_attack_total: Option<i32>,
    current_ac: Option<i32>,
    ability: Option<AbilityName>,
    damage_type: Option<String>,
    source_tags: Vec<String>,
}

impl PredicateContext {
    fn for_owner(index: usize) -> Self {
        Self {
            self_idx: index,
            actor: index,
            target: None,
            attacker: None,
            linked: None,
            attack: AttackFacts::default(),
            has_advantage: false,
            has_disadvantage: false,
            damage_dealt: 0,
            slot_level: None,
            current_attack_total: None,
            current_ac: None,
            ability: None,
            damage_type: None,
            source_tags: Vec::new(),
        }
    }
}

fn eval_predicate(
    state: &EncounterState,
    predicate: &Predicate,
    context: &PredicateContext,
) -> bool {
    match predicate {
        Predicate::All(items) => items
            .iter()
            .all(|item| eval_predicate(state, item, context)),
        Predicate::Any(items) => items
            .iter()
            .any(|item| eval_predicate(state, item, context)),
        Predicate::Not(item) => !eval_predicate(state, item, context),
        Predicate::HasAdvantage(flag) => context.has_advantage == flag.get(),
        Predicate::HasDisadvantage(flag) => context.has_disadvantage == flag.get(),
        Predicate::AllyWithin5OfTarget(flag) => {
            let value = ally_within_5(state, context.actor, context.target);
            value == flag.get()
        }
        Predicate::TargetHasCondition(condition) => context.target.is_some_and(|target| {
            state.combatants[target].has_condition(&state.content, condition)
        }),
        Predicate::SelfHasCondition(condition) => {
            state.combatants[context.self_idx].has_condition(&state.content, condition)
        }
        Predicate::AttackerWithin(feet) => {
            distance_between(state, context.attacker, context.target)
                .is_some_and(|distance| distance <= *feet)
        }
        Predicate::AttackerBeyond(feet) => {
            distance_between(state, context.attacker, context.target)
                .is_some_and(|distance| distance > *feet)
        }
        Predicate::LinkedIsAttacker(flag) => {
            let value = match (&context.linked, context.attacker) {
                (Some(linked), Some(attacker)) => {
                    state.combatants[attacker].name.eq_ignore_ascii_case(linked)
                }
                _ => false,
            };
            value == flag.get()
        }
        Predicate::LinkedIsTarget(flag) => {
            let value = match (&context.linked, context.target) {
                (Some(linked), Some(target)) => {
                    state.combatants[target].name.eq_ignore_ascii_case(linked)
                }
                _ => false,
            };
            value == flag.get()
        }
        Predicate::AttackHasProperty(properties) => properties
            .iter()
            .any(|property| context.attack.has_property(*property)),
        Predicate::AttackKind(kind) => context.attack.kind == *kind,
        Predicate::SelfHpBelowPct(percent) => {
            hp_below_pct(&state.combatants[context.self_idx], *percent)
        }
        Predicate::TargetHpBelowPct(percent) => context
            .target
            .is_some_and(|target| hp_below_pct(&state.combatants[target], *percent)),
        Predicate::IsSpell(flag) => {
            let is_spell = context
                .source_tags
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case("spell"));
            is_spell == flag.get()
        }
        Predicate::SlotLevelMin(minimum) => context.slot_level.unwrap_or(0) >= *minimum,
        Predicate::DamageDealt(flag) => (context.damage_dealt > 0) == flag.get(),
        Predicate::WouldMissWithAcBonus(bonus) => {
            match (context.current_attack_total, context.current_ac) {
                (Some(total), Some(ac)) => total < ac.saturating_add(*bonus),
                _ => false,
            }
        }
        Predicate::TargetSizeMax(maximum) => context.target.is_some_and(|target| {
            check_size_limit(state.combatants[target].creature.size(), Some(*maximum))
        }),
    }
}

fn hp_below_pct(combatant: &CombatantState, percent: i32) -> bool {
    if combatant.max_hp <= 0 {
        return combatant.hp <= 0;
    }
    combatant.hp.saturating_mul(100) <= combatant.max_hp.saturating_mul(percent)
}

fn distance_between(state: &EncounterState, a: Option<usize>, b: Option<usize>) -> Option<i32> {
    let (a, b) = (a?, b?);
    let space = state.space()?;
    space.distance(&state.combatants[a].zone, &state.combatants[b].zone)
}

fn ally_within_5(state: &EncounterState, actor: usize, target: Option<usize>) -> bool {
    let Some(target) = target else {
        return false;
    };
    let Some(space) = state.space() else {
        return false;
    };
    let target_zone = &state.combatants[target].zone;
    state
        .combatants
        .iter()
        .enumerate()
        .any(|(index, combatant)| {
            index != actor
                && index != target
                && combatant.side == state.combatants[actor].side
                && combatant.is_active()
                && !is_incapacitated(state, index)
                && space
                    .distance(&combatant.zone, target_zone)
                    .is_some_and(|distance| distance <= 5)
        })
}

// ---------------------------------------------------------------------------
// Modifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CollectedModifier {
    modifier: Modifier,
    label: String,
    key: String,
    origin_condition: Option<String>,
}

fn modifier_filter_matches(filter: &ModifierFilter, context: &PredicateContext) -> bool {
    if let Some(abilities) = &filter.ability {
        let matches = context
            .ability
            .is_some_and(|ability| abilities.contains(&ability));
        if !matches {
            return false;
        }
    }
    if let Some(types) = &filter.damage_type {
        let matches = context.damage_type.as_ref().is_some_and(|damage_type| {
            types
                .iter()
                .any(|candidate| candidate.as_str().eq_ignore_ascii_case(damage_type))
        });
        if !matches {
            return false;
        }
    }
    if let Some(kinds) = &filter.attack_kind {
        if !kinds.contains(&context.attack.kind) {
            return false;
        }
    }
    if let Some(tags) = &filter.tags {
        let matches = tags.iter().any(|tag| {
            context
                .source_tags
                .iter()
                .any(|source| source.eq_ignore_ascii_case(tag))
        });
        if !matches {
            return false;
        }
    }
    true
}

fn collect_modifiers(
    state: &EncounterState,
    owner: usize,
    applies: ModifierApplies,
    context: &PredicateContext,
    source: Option<&EffectSource>,
) -> Vec<CollectedModifier> {
    let combatant = &state.combatants[owner];
    let mut out = Vec::new();
    let active = combatant.conditions.clone();
    for (instance_index, instance) in active.iter().enumerate() {
        let mut scoped = context.clone();
        scoped.linked = instance.linked.clone();
        let mut condition_ids = Vec::new();
        collect_condition_chain(&state.content, &instance.condition, &mut condition_ids);
        for condition_id in condition_ids {
            if state.combatants[owner].is_immune_to_condition(&condition_id) {
                continue;
            }
            let Some(def) = state.content.conditions.get(&condition_id) else {
                continue;
            };
            for (modifier_index, modifier) in def.modifiers.iter().enumerate() {
                if modifier.applies != applies {
                    continue;
                }
                if !modifier_filter_matches_opt(&modifier.filter, &scoped) {
                    continue;
                }
                if let Some(when) = &modifier.when {
                    if !eval_predicate(state, when, &scoped) {
                        continue;
                    }
                }
                out.push(CollectedModifier {
                    modifier: modifier.clone(),
                    label: def.display_name().to_string(),
                    key: format!(
                        "condition:{}:{}:mod:{modifier_index}",
                        instance.condition, instance_index
                    ),
                    origin_condition: Some(instance.condition.clone()),
                });
            }
        }
    }

    if let Ok(abilities) = combatant.owned_abilities(&state.content) {
        for resolved in abilities {
            if resolved.ability.activation != Activation::Passive {
                continue;
            }
            for (modifier_index, modifier) in resolved.ability.modifiers.iter().enumerate() {
                if modifier.applies != applies {
                    continue;
                }
                if !modifier_filter_matches_opt(&modifier.filter, context) {
                    continue;
                }
                if let Some(when) = &modifier.when {
                    if !eval_predicate(state, when, context) {
                        continue;
                    }
                }
                out.push(CollectedModifier {
                    modifier: modifier.clone(),
                    label: resolved.ability.display_name().to_string(),
                    key: format!("ability:{}:mod:{modifier_index}", resolved.ability.id),
                    origin_condition: None,
                });
            }
        }
    }
    if let Some(source) = source {
        for (modifier_index, modifier) in source.modifiers.iter().enumerate() {
            if modifier.applies != applies {
                continue;
            }
            if !modifier_filter_matches_opt(&modifier.filter, context) {
                continue;
            }
            if let Some(when) = &modifier.when {
                if !eval_predicate(state, when, context) {
                    continue;
                }
            }
            out.push(CollectedModifier {
                modifier: modifier.clone(),
                label: source.name.clone(),
                key: format!("source:{}:mod:{modifier_index}", source.id),
                origin_condition: None,
            });
        }
    }
    out
}

fn modifier_filter_matches_opt(
    filter: &Option<ModifierFilter>,
    context: &PredicateContext,
) -> bool {
    filter
        .as_ref()
        .is_none_or(|filter| modifier_filter_matches(filter, context))
}

fn collect_condition_chain(content: &ContentRegistry, id: &str, out: &mut Vec<String>) {
    if out.iter().any(|known| known.eq_ignore_ascii_case(id)) {
        return;
    }
    out.push(id.to_string());
    if let Some(def) = content.conditions.get(id) {
        for implied in &def.implies {
            collect_condition_chain(content, implied, out);
        }
    }
}

fn modifier_available(
    state: &EncounterState,
    combatant: &CombatantState,
    collected: &CollectedModifier,
    once_per: Option<OncePer>,
) -> bool {
    match once_per {
        Some(OncePer::Turn) => !combatant.once_used.contains_key(&turn_key(&collected.key)),
        Some(OncePer::Round) => combatant
            .once_used
            .get(&round_key(&collected.key))
            .is_none_or(|round| *round != state.round),
        None => true,
    }
}

fn mark_once(combatant: &mut CombatantState, key: &str, once_per: Option<OncePer>, round: u32) {
    match once_per {
        Some(OncePer::Turn) => {
            combatant.once_used.insert(turn_key(key), round);
        }
        Some(OncePer::Round) => {
            combatant.once_used.insert(round_key(key), round);
        }
        None => {}
    }
}

fn turn_key(key: &str) -> String {
    format!("turn:{key}")
}

fn round_key(key: &str) -> String {
    format!("round:{key}")
}

fn consume_modifier_source(
    state: &mut EncounterState,
    owner: usize,
    collected: &CollectedModifier,
    log: &mut Vec<String>,
) {
    if let Some(condition) = &collected.origin_condition {
        let name = state.combatants[owner].name.clone();
        let before = state.combatants[owner].conditions.len();
        state.combatants[owner]
            .conditions
            .retain(|active| !active.condition.eq_ignore_ascii_case(condition));
        if state.combatants[owner].conditions.len() != before {
            log.push(format!("{name} is no longer {condition}"));
        }
    }
    mark_once(
        &mut state.combatants[owner],
        &collected.key,
        None,
        state.round,
    );
}

fn finish_collected_modifiers(
    state: &mut EncounterState,
    owner: usize,
    applies: ModifierApplies,
    context: &PredicateContext,
    source: Option<&EffectSource>,
    log: &mut Vec<String>,
) {
    let collected = collect_modifiers(state, owner, applies, context, source);
    for modifier in collected {
        if let Some(once_per) = modifier.modifier.once_per {
            mark_once(
                &mut state.combatants[owner],
                &modifier.key,
                Some(once_per),
                state.round,
            );
        }
        if modifier.modifier.consume {
            consume_modifier_source(state, owner, &modifier, log);
        }
    }
}

#[derive(Debug, Clone)]
struct BonusTerm {
    label: String,
    record: RollRecord,
}

fn roll_bonus_terms(
    state: &EncounterState,
    owner: usize,
    modifiers: &[CollectedModifier],
    rng: &mut dyn DiceRng,
) -> Result<Vec<BonusTerm>> {
    let context = score_context(&state.combatants[owner]);
    let mut terms = Vec::new();
    for collected in modifiers {
        let modifier = &collected.modifier;
        let value = modifier
            .value
            .as_ref()
            .ok_or_else(|| anyhow!("modifier from `{}` has no value", collected.label))?;
        let (_, expression) = evaluate_value(&context, value)?;
        let record = expression.roll(rng);
        terms.push(BonusTerm {
            label: collected.label.clone(),
            record,
        });
    }
    Ok(terms)
}

fn damage_roll_bonus(
    state: &EncounterState,
    owner: usize,
    source: Option<&EffectSource>,
    damage_type: Option<&str>,
    rng: &mut dyn DiceRng,
) -> Result<Option<BonusTerm>> {
    let context = PredicateContext {
        self_idx: owner,
        actor: owner,
        target: None,
        attacker: None,
        linked: None,
        attack: AttackFacts::default(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: source.and_then(|source| source.slot_level),
        current_attack_total: None,
        current_ac: None,
        ability: None,
        damage_type: damage_type.map(ToString::to_string),
        source_tags: source.map(|source| source.tags.clone()).unwrap_or_default(),
    };
    let modifiers = collect_modifiers(state, owner, ModifierApplies::DamageRoll, &context, source);
    let context_expr = score_context(&state.combatants[owner]);
    let mut total = 0;
    let mut labels = Vec::new();
    for collected in &modifiers {
        if collected.modifier.effect != ModifierEffect::Bonus {
            continue;
        }
        if let Some(value) = &collected.modifier.value {
            let (_, expression) = evaluate_value(&context_expr, value)?;
            let record = expression.roll(rng);
            total += record.total;
            labels.push(format!("{}[{}]", record.total, collected.label));
        }
    }
    if total == 0 {
        Ok(None)
    } else {
        Ok(Some(BonusTerm {
            label: labels.join(","),
            record: RollRecord {
                formula: "bonus".to_string(),
                dice: Vec::new(),
                modifier: total,
                total,
                text: format!("+{total}"),
            },
        }))
    }
}

// ---------------------------------------------------------------------------
// Initiative and turns
// ---------------------------------------------------------------------------

/// Rolls initiative for every non-dormant combatant.
pub fn init(
    state: &mut EncounterState,
    overrides: &[(usize, i32)],
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    state.initiative.clear();
    state.round = 1;
    state.turn = 0;
    state.outcome = None;

    let mut rows: Vec<(usize, i32, bool)> = Vec::new();
    for (index, combatant) in state.combatants.iter_mut().enumerate() {
        combatant.initiative_total = None;
        if combatant.dormant || combatant.is_dead() {
            continue;
        }
        if let Some((_, total)) = overrides.iter().find(|(target, _)| *target == index) {
            combatant.initiative_total = Some(*total);
            rows.push((index, *total, true));
            continue;
        }
        let roll = D20Roll::roll(combatant.creature.init_bonus(), RollMode::Normal, rng);
        combatant.initiative_total = Some(roll.total);
        log.push(format!("Initiative {}: {}", combatant.name, roll.text()));
        rows.push((index, roll.total, false));
    }

    rows.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| {
                state.combatants[b.0]
                    .creature
                    .init_bonus()
                    .cmp(&state.combatants[a.0].creature.init_bonus())
            })
            .then_with(|| {
                state.combatants[b.0]
                    .creature
                    .ability_modifier(AbilityName::Dex)
                    .cmp(
                        &state.combatants[a.0]
                            .creature
                            .ability_modifier(AbilityName::Dex),
                    )
            })
            .then_with(|| a.0.cmp(&b.0))
    });
    state.initiative = rows.iter().map(|(index, _, _)| *index).collect();
    if state.initiative.is_empty() {
        bail!("no non-dormant combatants to roll initiative for");
    }
    for (index, total, set) in rows {
        if set {
            log.push(format!(
                "Initiative {}: {total} (set)",
                state.combatants[index].name
            ));
        }
    }

    let first = state.initiative[0];
    start_turn(state, first, rng, log);
    if let Some(combatant) = state.combatants.get(first) {
        log.push(format!("Round {} - {}'s turn", state.round, combatant.name));
    }
    Ok(())
}

fn eligible_for_turn(combatant: &CombatantState) -> bool {
    !combatant.dormant && !combatant.is_dead()
}

/// Ends the current turn and starts the next eligible one.
pub fn next_turn(
    state: &mut EncounterState,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    if state.initiative.is_empty() {
        bail!("initiative has not been rolled; run `init` first");
    }
    let current = state
        .initiative
        .get(state.turn)
        .copied()
        .ok_or_else(|| anyhow!("turn index is outside the initiative order"))?;
    end_turn(state, current, rng, log)?;

    let len = state.initiative.len();
    let mut position = state.turn;
    let mut wrapped = false;
    let mut next = None;
    for _ in 0..len {
        position += 1;
        if position >= len {
            position = 0;
            if !wrapped {
                wrapped = true;
                state.round = state.round.saturating_add(1);
                reset_round_state(state, rng, log);
            }
        }
        let candidate = state.initiative[position];
        if eligible_for_turn(&state.combatants[candidate]) {
            next = Some((position, candidate));
            break;
        }
    }
    let Some((position, next_index)) = next else {
        bail!("no eligible combatant remains in the initiative order");
    };
    state.turn = position;
    if !wrapped && state.round == 0 {
        state.round = 1;
    }
    start_turn(state, next_index, rng, log);
    let name = state.combatants[next_index].name.clone();
    log.push(format!("Round {} - {name}'s turn", state.round));
    log_outcome(state, log);
    Ok(())
}

fn reset_round_state(state: &mut EncounterState, rng: &mut dyn DiceRng, log: &mut Vec<String>) {
    for index in 0..state.combatants.len() {
        state.combatants[index]
            .once_used
            .retain(|key, _| !key.starts_with("round:"));
    }
    for index in 0..state.combatants.len() {
        recharge_resources(state, index, RechargePhase::Round, rng, log);
    }
}

fn start_turn(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) {
    let speed = effective_speed(state, index);
    {
        let combatant = &mut state.combatants[index];
        combatant.action_used = false;
        combatant.bonus_action_used = false;
        combatant.speed_left = speed;
        combatant
            .once_used
            .retain(|key, _| !key.starts_with("turn:"));
    }
    if state.combatants[index].is_dead() || state.combatants[index].dormant {
        return;
    }
    expire_start_phases(state, index, log);
    recharge_resources(state, index, RechargePhase::Turn, rng, log);
    if state.combatants[index].is_dying() {
        let name = state.combatants[index].name.clone();
        log.push(format!(
            "{name} is dying and must make a death saving throw"
        ));
        let _ = death_save_at(state, index, rng, log);
    }
    run_owned_triggers(state, index, TriggerWhen::StartOfTurn, None, rng, log);
}

fn end_turn(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    run_owned_triggers(state, index, TriggerWhen::EndOfTurn, None, rng, log);
    expire_conditions(state, index, rng, log)?;
    Ok(())
}

fn expire_conditions(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let name = state.combatants[index].name.clone();
    let mut remove = Vec::new();
    let mut still_active = Vec::new();
    let conditions = state.combatants[index].conditions.clone();
    for (position, mut instance) in conditions.into_iter().enumerate() {
        let mut expired = false;
        if let Some(save_ends) = instance.save_ends.clone() {
            let bonus = state.combatants[index].save_bonus(save_ends.ability);
            let roll = D20Roll::roll(bonus, RollMode::Normal, rng);
            log.push(format!(
                "{} save to end {}: {}",
                name,
                instance.condition,
                roll.vs_dc(save_ends.dc)
            ));
            if roll.total >= save_ends.dc {
                expired = true;
            }
        }
        if !expired {
            match instance.duration.as_mut() {
                Some(ActiveDuration::Rounds { remaining }) => {
                    if *remaining > 0 {
                        *remaining -= 1;
                    }
                    if *remaining == 0 {
                        expired = true;
                    }
                }
                Some(ActiveDuration::Minutes { remaining }) => {
                    if *remaining > 0 {
                        *remaining -= 1;
                    }
                    if *remaining == 0 {
                        expired = true;
                    }
                }
                Some(ActiveDuration::Until {
                    until,
                    subject,
                    end_round,
                }) => {
                    let is_end_phase = matches!(
                        until,
                        DurationUntil::EndOfSourceNextTurn | DurationUntil::EndOfTargetNextTurn
                    );
                    if is_end_phase
                        && subject.eq_ignore_ascii_case(&name)
                        && state.round >= *end_round
                    {
                        expired = true;
                    }
                }
                Some(ActiveDuration::Permanent) | None => {}
            }
        }
        if expired {
            remove.push(position);
            log.push(format!("{name} is no longer {}", instance.condition));
            if instance.concentration {
                break_concentration(state, index, log);
            }
        } else {
            instance.applied_round = instance.applied_round.min(state.round);
            still_active.push(instance);
        }
    }
    let _ = remove;
    state.combatants[index].conditions = still_active;
    Ok(())
}

fn expire_start_phases(state: &mut EncounterState, index: usize, log: &mut Vec<String>) {
    let name = state.combatants[index].name.clone();
    let mut keep = Vec::new();
    for instance in state.combatants[index].conditions.clone() {
        let expired = match &instance.duration {
            Some(ActiveDuration::Until {
                until: DurationUntil::StartOfSourceNextTurn,
                subject,
                end_round,
            }) => subject.eq_ignore_ascii_case(&name) && state.round >= *end_round,
            _ => false,
        };
        if expired {
            log.push(format!("{name} is no longer {}", instance.condition));
        } else {
            keep.push(instance);
        }
    }
    state.combatants[index].conditions = keep;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RechargePhase {
    Turn,
    Round,
}

fn recharge_resources(
    state: &mut EncounterState,
    index: usize,
    phase: RechargePhase,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) {
    let name = state.combatants[index].name.clone();
    let ids: Vec<String> = state.combatants[index].resources.keys().cloned().collect();
    for id in ids {
        let resource = state.combatants[index]
            .resources
            .get(&id)
            .expect("resource id came from the same map");
        if resource.is_full() {
            continue;
        }
        let recharge = resource.recharge.clone();
        let applies = matches!(
            (phase, &recharge),
            (RechargePhase::Turn, Recharge::Turn) | (RechargePhase::Round, Recharge::Round)
        );
        if applies {
            restore_resource(state, index, &id, log);
            continue;
        }
        let in_range = match (&recharge, phase) {
            (Recharge::Single(minimum), RechargePhase::Turn) => rng.roll_die(6) >= *minimum,
            (Recharge::Range { min, max }, RechargePhase::Turn) => {
                let roll = rng.roll_die(6);
                roll >= *min && roll <= *max
            }
            _ => false,
        };
        if in_range {
            log.push(format!("{name} recharges {id}"));
            restore_resource(state, index, &id, log);
        }
    }
}

fn restore_resource(state: &mut EncounterState, index: usize, id: &str, _log: &mut Vec<String>) {
    if let Some(resource) = state.combatants[index].resources.get_mut(id) {
        resource.current = resource.max;
    }
}

// ---------------------------------------------------------------------------
// Combat outcome
// ---------------------------------------------------------------------------

pub fn combat_outcome(state: &EncounterState) -> Option<CombatOutcome> {
    let active_sides: BTreeSet<&str> = state
        .combatants
        .iter()
        .filter(|combatant| combatant.is_active())
        .map(|combatant| combatant.side.as_str())
        .collect();
    match active_sides.len() {
        0 => Some(CombatOutcome::Draw),
        1 => Some(CombatOutcome::Winner(
            active_sides.iter().next().expect("one side").to_string(),
        )),
        _ => None,
    }
}

fn log_outcome(state: &mut EncounterState, log: &mut Vec<String>) {
    let outcome = combat_outcome(state);
    if outcome.is_some() && state.outcome != outcome {
        if let Some(outcome) = &outcome {
            log.push(format!("Outcome: {outcome}"));
        }
    }
    state.outcome = outcome;
}

// ---------------------------------------------------------------------------
// Attacks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AttackProfile {
    id: String,
    name: String,
    kind: AttackKind,
    ability: Option<AbilityName>,
    to_hit: ExprValue,
    damage: Vec<DamagePart>,
    reach: i32,
    range: Option<[i32; 2]>,
    properties: Vec<AttackProperty>,
    mastery: Option<String>,
    on_hit: Vec<Effect>,
    on_miss: Vec<Effect>,
    tags: Vec<String>,
}

impl AttackProfile {
    fn from_attack(attack: &Attack) -> Self {
        Self {
            id: attack.id.clone(),
            name: attack.display_name().to_string(),
            kind: attack.kind,
            ability: attack.ability,
            to_hit: attack.to_hit.clone().unwrap_or(ExprValue::Number(0)),
            damage: attack.damage.clone(),
            reach: attack.reach.unwrap_or(5),
            range: attack.range,
            properties: attack.properties.clone(),
            mastery: attack.mastery.clone(),
            on_hit: attack.on_hit.clone(),
            on_miss: Vec::new(),
            tags: Vec::new(),
        }
    }

    fn from_effect(effect: &AttackEffect, source: &EffectSource, range: Option<i32>) -> Self {
        let (reach, ranged) = match effect.kind {
            AttackKind::Melee => (range.unwrap_or(5), None),
            AttackKind::Ranged => (5, range.map(|value| [value, value])),
            AttackKind::MeleeOrRanged => (5, range.map(|value| [value, value])),
        };
        Self {
            id: source.id.clone(),
            name: source.name.clone(),
            kind: effect.kind,
            ability: None,
            to_hit: effect.to_hit.clone(),
            damage: effect.damage.clone(),
            reach,
            range: ranged,
            properties: effect.properties.clone(),
            mastery: effect.mastery.clone(),
            on_hit: effect.on_hit.clone(),
            on_miss: effect.on_miss.clone(),
            tags: source.tags.clone(),
        }
    }

    fn facts(&self) -> AttackFacts {
        AttackFacts {
            kind: self.kind,
            properties: self.properties.clone(),
            ability: self.ability,
            tags: self.tags.clone(),
        }
    }

    fn attack_modifier(&self, combatant: &CombatantState) -> i32 {
        self.ability
            .map(|ability| combatant.creature.ability_modifier(ability))
            .unwrap_or_else(|| {
                ability_name_from_attack_kind(self.kind, combatant)
                    .map(|ability| combatant.creature.ability_modifier(ability))
                    .unwrap_or(0)
            })
    }
}

#[derive(Debug, Clone)]
struct EffectSource {
    id: String,
    name: String,
    tags: Vec<String>,
    slot_level: Option<i32>,
    concentration: bool,
    params: BTreeMap<String, String>,
    duration: Option<Duration>,
    range: Option<i32>,
    modifiers: Vec<Modifier>,
}

impl EffectSource {
    fn from_ability(ability: &Ability, params: &BTreeMap<String, ExprValue>) -> Self {
        Self {
            id: ability.id.clone(),
            name: ability.display_name().to_string(),
            tags: ability.tags.clone(),
            slot_level: ability.slot_level,
            concentration: ability.concentration,
            params: params
                .iter()
                .map(|(key, value)| (key.clone(), value.as_text()))
                .collect(),
            duration: ability.duration.clone(),
            range: ability.range,
            modifiers: ability.modifiers.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct D20Result {
    dice: Vec<i32>,
    kept: i32,
    base: i32,
    bonus_terms: Vec<BonusTerm>,
    total: i32,
    nat1: bool,
    nat20: bool,
    mode: RollMode,
    advantage_sources: Vec<String>,
    disadvantage_sources: Vec<String>,
}

impl D20Result {
    fn text(&self) -> String {
        let suffix = match self.mode {
            RollMode::Normal => String::new(),
            RollMode::Advantage => format!(" adv->{}", self.kept),
            RollMode::Disadvantage => format!(" dis->{}", self.kept),
        };
        let dice = self
            .dice
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut text = format!(
            "1d20({dice}{suffix}){}{}",
            signed(self.base),
            bonus_text(&self.bonus_terms)
        );
        text.push('=');
        text.push_str(&self.total.to_string());
        let mut annotations = Vec::new();
        if !self.advantage_sources.is_empty() {
            annotations.push(format!("adv: {}", self.advantage_sources.join(", ")));
        }
        if !self.disadvantage_sources.is_empty() {
            annotations.push(format!("dis: {}", self.disadvantage_sources.join(", ")));
        }
        if !annotations.is_empty() {
            text.push_str(&format!(" [{}]", annotations.join("; ")));
        }
        text
    }
}

fn signed(value: i32) -> String {
    if value == 0 {
        String::new()
    } else {
        format!("{value:+}")
    }
}

fn bonus_text(terms: &[BonusTerm]) -> String {
    let mut text = String::new();
    for term in terms {
        let base = term
            .record
            .text
            .split('=')
            .next()
            .unwrap_or(&term.record.text);
        if base.parse::<i32>().is_ok() {
            text.push_str(&format!("{}[{}]", signed(term.record.total), term.label));
        } else {
            text.push_str(&format!("+{base}[{}]", term.label));
        }
    }
    text
}

fn roll_d20(
    _state: &EncounterState,
    _owner: usize,
    base: i32,
    mode: RollMode,
    bonuses: Vec<BonusTerm>,
    advantage_sources: Vec<String>,
    disadvantage_sources: Vec<String>,
    rng: &mut dyn DiceRng,
) -> D20Result {
    let roll = D20Roll::roll(base, mode, rng);
    let bonus_total: i32 = bonuses.iter().map(|term| term.record.total).sum();
    D20Result {
        dice: roll.dice.clone(),
        kept: roll.kept,
        base,
        bonus_terms: bonuses,
        total: roll.total + bonus_total,
        nat1: roll.nat1,
        nat20: roll.nat20,
        mode,
        advantage_sources,
        disadvantage_sources,
    }
}

fn effective_ac(
    state: &EncounterState,
    target: usize,
    attacker: Option<usize>,
    attack: &AttackFacts,
) -> Result<(i32, Vec<(i32, String)>)> {
    let combatant = &state.combatants[target];
    let mut ac = combatant.creature.ac();
    let mut terms = Vec::new();
    let context = PredicateContext {
        self_idx: target,
        actor: target,
        target: Some(target),
        attacker,
        linked: None,
        attack: attack.clone(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: None,
        current_attack_total: None,
        current_ac: None,
        ability: attack.ability,
        damage_type: None,
        source_tags: attack.tags.clone(),
    };
    for collected in collect_modifiers(state, target, ModifierApplies::Ac, &context, None) {
        if collected.modifier.effect == ModifierEffect::Bonus {
            if let Some(value) = &collected.modifier.value {
                let expr_context = score_context(combatant);
                let (_, expression) = evaluate_value(&expr_context, value)?;
                let bonus = expression.modifier;
                ac += bonus;
                terms.push((bonus, collected.label));
            }
        }
    }
    if let Some(attacker) = attacker {
        if let Some(space) = state.space() {
            if let Some(cover) = space.cover_bonus(
                &state.combatants[attacker].zone,
                &state.combatants[target].zone,
                &state.content.rules,
            ) {
                if cover != 0 {
                    ac += cover;
                    terms.push((cover, "cover".to_string()));
                }
            } else {
                bail!(
                    "`{}` has total cover from `{}`",
                    combatant.name,
                    state.combatants[attacker].name
                );
            }
        }
    }
    Ok((ac, terms))
}

fn ac_text(total: i32, terms: &[(i32, String)]) -> String {
    let bonus: i32 = terms.iter().map(|(value, _)| *value).sum();
    let base = total - bonus;
    let mut text = format!("AC {base}");
    for (value, label) in terms {
        text.push_str(&format!("{}[{label}]", signed(*value)));
    }
    if !terms.is_empty() {
        text.push_str(&format!("={total}"));
    }
    text
}

/// Resolves `<attacker> <target> <attack-or-ability>`.
pub fn attack(
    state: &mut EncounterState,
    attacker: &str,
    target: &str,
    attack_name: &str,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let attacker_index = state
        .find_index(attacker)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target_index = state
        .find_index(target)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, attacker_index)?;
    ensure_targetable(state, target_index)?;
    if attacker_index == target_index {
        bail!("a creature cannot attack itself");
    }
    if !hostile_to(state, attacker_index, target_index) {
        bail!("`{target}` is not hostile to `{attacker}`");
    }
    if options.multi {
        let entries = state.combatants[attacker_index]
            .creature
            .multiattack
            .clone()
            .ok_or_else(|| anyhow!("`{attacker}` has no multiattack"))?;
        spend_action(state, attacker_index, Activation::Action, log)?;
        log.push(format!("{attacker} uses multiattack"));
        for entry in entries {
            resolve_named_entry(
                state,
                attacker_index,
                target_index,
                &entry,
                options,
                true,
                rng,
                log,
            )?;
        }
        log_outcome(state, log);
        return Ok(());
    }

    spend_action(state, attacker_index, Activation::Action, log)?;
    resolve_named_entry(
        state,
        attacker_index,
        target_index,
        attack_name,
        options,
        true,
        rng,
        log,
    )?;
    log_outcome(state, log);
    Ok(())
}

fn resolve_named_entry(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    name: &str,
    options: &AttackOptions,
    auto: bool,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let combatant = &state.combatants[attacker];
    if let Ok(attack) = combatant.find_attack(name) {
        let profile = AttackProfile::from_attack(attack);
        return resolve_attack(
            state, attacker, target, &profile, None, options, rng, log, false,
        );
    }
    let resolved = combatant
        .find_owned_ability(&state.content, name)
        .with_context(|| format!("`{}` cannot use `{name}`", combatant.name))?;
    let source = EffectSource::from_ability(&resolved.ability, &resolved.params);
    use_resolved_ability(
        state,
        attacker,
        &resolved,
        &source,
        &[target],
        options,
        None,
        None,
        auto,
        rng,
        log,
    )
}

fn spend_action(
    state: &mut EncounterState,
    index: usize,
    activation: Activation,
    log: &mut Vec<String>,
) -> Result<()> {
    match activation {
        Activation::Action => {
            if state.combatants[index].action_used {
                bail!(
                    "`{}` has already used an action this turn",
                    state.combatants[index].name
                );
            }
            state.combatants[index].action_used = true;
        }
        Activation::Bonus => {
            if state.combatants[index].bonus_action_used {
                bail!(
                    "`{}` has already used a bonus action this turn",
                    state.combatants[index].name
                );
            }
            state.combatants[index].bonus_action_used = true;
        }
        Activation::Reaction => {
            spend_reaction(state, index, log)?;
        }
        Activation::Free | Activation::Passive => {}
    }
    Ok(())
}

fn spend_reaction(state: &mut EncounterState, index: usize, _log: &mut Vec<String>) -> Result<()> {
    if state.combatants[index].reaction_round == Some(state.round) {
        bail!(
            "`{}` has already used a reaction this round",
            state.combatants[index].name
        );
    }
    if has_flag(state, index, ConditionFlag::NoReactions) {
        bail!("`{}` cannot use reactions", state.combatants[index].name);
    }
    state.combatants[index].reaction_round = Some(state.round);
    Ok(())
}

fn resolve_attack(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    profile: &AttackProfile,
    source: Option<&EffectSource>,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
    economy_spent: bool,
) -> Result<()> {
    if economy_spent {
        // Caller already spent the action (multiattack / ability effect).
    }
    ensure_can_act(state, attacker)?;
    ensure_targetable(state, target)?;

    let attacker_name = state.combatants[attacker].name.clone();
    let target_name = state.combatants[target].name.clone();
    let mut mode = RollMode::from_flags(options.advantage, options.disadvantage);
    let mut advantage_sources = Vec::new();
    let mut disadvantage_sources = Vec::new();
    if options.advantage {
        advantage_sources.push("command".to_string());
    }
    if options.disadvantage {
        disadvantage_sources.push("command".to_string());
    }

    let facts = profile.facts();
    let attack_ability = profile
        .ability
        .or_else(|| ability_name_from_attack_kind(profile.kind, &state.combatants[attacker]));
    let attacker_context = PredicateContext {
        self_idx: attacker,
        actor: attacker,
        target: Some(target),
        attacker: Some(attacker),
        linked: None,
        attack: facts.clone(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: source.and_then(|source| source.slot_level),
        current_attack_total: None,
        current_ac: None,
        ability: attack_ability,
        damage_type: None,
        source_tags: profile.tags.clone(),
    };
    let target_context = PredicateContext {
        self_idx: target,
        actor: target,
        target: Some(target),
        attacker: Some(attacker),
        linked: None,
        attack: facts.clone(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: source.and_then(|source| source.slot_level),
        current_attack_total: None,
        current_ac: None,
        ability: attack_ability,
        damage_type: None,
        source_tags: profile.tags.clone(),
    };

    for collected in collect_modifiers(
        state,
        attacker,
        ModifierApplies::AttackRoll,
        &attacker_context,
        source,
    ) {
        match collected.modifier.effect {
            ModifierEffect::Advantage => {
                advantage_sources.push(collected.label.clone());
                mode = add_advantage(mode);
            }
            ModifierEffect::Disadvantage => {
                disadvantage_sources.push(collected.label.clone());
                mode = add_disadvantage(mode);
            }
            _ => {}
        }
    }
    for collected in collect_modifiers(
        state,
        target,
        ModifierApplies::IncomingAttack,
        &target_context,
        None,
    ) {
        match collected.modifier.effect {
            ModifierEffect::Advantage => {
                advantage_sources.push(collected.label.clone());
                mode = add_advantage(mode);
            }
            ModifierEffect::Disadvantage => {
                disadvantage_sources.push(collected.label.clone());
                mode = add_disadvantage(mode);
            }
            _ => {}
        }
    }

    // Range and reach.
    let space = state
        .space()
        .ok_or_else(|| anyhow!("this state has no scene"))?;
    let distance = space
        .distance(
            &state.combatants[attacker].zone,
            &state.combatants[target].zone,
        )
        .ok_or_else(|| anyhow!("no path between the combatants' zones"))?;
    let mut kind = profile.kind;
    if kind == AttackKind::MeleeOrRanged {
        kind = if space.can_reach(
            &state.combatants[attacker].zone,
            &state.combatants[target].zone,
            profile.reach,
        ) {
            AttackKind::Melee
        } else {
            AttackKind::Ranged
        };
    }
    match kind {
        AttackKind::Melee => {
            if !space.can_reach(
                &state.combatants[attacker].zone,
                &state.combatants[target].zone,
                profile.reach,
            ) {
                bail!(
                    "`{target_name}` is out of reach ({distance} ft; reach {})",
                    profile.reach
                );
            }
        }
        AttackKind::Ranged | AttackKind::MeleeOrRanged => {
            let Some([normal, long]) = profile.range else {
                bail!("`{}` has no range", profile.name);
            };
            if distance > long {
                bail!("`{target_name}` is out of range ({distance} ft; long range {long})");
            }
            if distance > normal {
                disadvantage_sources.push("long range".to_string());
                mode = add_disadvantage(mode);
            }
            if hostile_within_5(state, attacker) {
                disadvantage_sources.push("ranged in melee".to_string());
                mode = add_disadvantage(mode);
            }
        }
    }

    // Attack roll bonuses.
    let mut bonuses = Vec::new();
    for collected in collect_modifiers(
        state,
        attacker,
        ModifierApplies::AttackRoll,
        &attacker_context,
        source,
    ) {
        if collected.modifier.effect == ModifierEffect::Bonus
            && modifier_available(state, &state.combatants[attacker], &collected, None)
        {
            bonuses.extend(roll_bonus_terms(
                state,
                attacker,
                std::slice::from_ref(&collected),
                rng,
            )?);
            mark_once(
                &mut state.combatants[attacker],
                &collected.key,
                None,
                state.round,
            );
        }
    }
    for collected in collect_modifiers(
        state,
        target,
        ModifierApplies::IncomingAttack,
        &target_context,
        None,
    ) {
        if collected.modifier.effect == ModifierEffect::Bonus {
            bonuses.extend(roll_bonus_terms(
                state,
                target,
                std::slice::from_ref(&collected),
                rng,
            )?);
        }
    }

    let (base_ac, ac_terms) = effective_ac(state, target, Some(attacker), &facts)?;
    let attack_mod = profile.attack_modifier(&state.combatants[attacker]);
    let context = source_context(state, attacker, source, Some(attack_mod), None);
    let (to_hit_formula, to_hit_expr) = evaluate_value(&context, &profile.to_hit)
        .with_context(|| format!("attack `{}` to_hit", profile.name))?;
    let base = to_hit_expr.modifier;
    let result = roll_d20(
        state,
        attacker,
        base,
        mode,
        bonuses,
        advantage_sources,
        disadvantage_sources,
        rng,
    );
    let _ = to_hit_formula;
    let mut hit = result.nat20 || (!result.nat1 && result.total >= base_ac);
    let mut crit = result.nat20;
    let mut current_ac = base_ac;
    let mut current_terms = ac_terms.clone();
    log.push(format!(
        "{attacker_name} attacks {target_name} with {}: {} vs {} {}",
        profile.name,
        result.text(),
        ac_text(base_ac, &current_terms),
        if hit { "HIT" } else { "MISS" }
    ));

    if hit && !options.no_reactions {
        let converted = try_hit_reactions(
            state,
            attacker,
            target,
            profile,
            &facts,
            result.total,
            current_ac,
            source,
            rng,
            log,
        )?;
        if let Some((new_ac, new_terms)) = converted {
            current_ac = new_ac;
            current_terms = new_terms;
            if result.total < current_ac {
                hit = false;
                log.push(format!(
                    "{} now misses: {} vs {} MISS",
                    attacker_name,
                    result.total,
                    ac_text(current_ac, &current_terms)
                ));
            }
        }
    }

    if hit {
        for collected in collect_modifiers(
            state,
            target,
            ModifierApplies::IncomingAttack,
            &target_context,
            None,
        ) {
            if collected.modifier.effect == ModifierEffect::AutoCrit {
                crit = true;
            }
        }
        let mut attack_context = EffectContext {
            actor: attacker,
            target: Some(target),
            source: source.cloned(),
            attack: Some(profile.clone()),
            option: None,
            crit,
            damage_dealt: 0,
            damage_scale: 1.0,
            has_advantage: result.mode == RollMode::Advantage,
            has_disadvantage: result.mode == RollMode::Disadvantage,
        };
        attack_context.damage_dealt =
            apply_attack_damage_parts(state, &attack_context, &profile.damage, rng, log)?;
        run_mastery_triggers(
            state,
            attacker,
            target,
            profile,
            &attack_context,
            TriggerWhen::OnHit,
            rng,
            log,
        )?;
        apply_effects(state, &attack_context, &profile.on_hit, rng, log)?;
        run_owned_triggers_for(
            state,
            attacker,
            TriggerWhen::OnHit,
            Some(target),
            profile,
            &attack_context,
            rng,
            log,
        )?;
    } else {
        let attack_context = EffectContext {
            actor: attacker,
            target: Some(target),
            source: source.cloned(),
            attack: Some(profile.clone()),
            option: None,
            crit: false,
            damage_dealt: 0,
            damage_scale: 1.0,
            has_advantage: result.mode == RollMode::Advantage,
            has_disadvantage: result.mode == RollMode::Disadvantage,
        };
        run_mastery_triggers(
            state,
            attacker,
            target,
            profile,
            &attack_context,
            TriggerWhen::OnMiss,
            rng,
            log,
        )?;
        apply_effects(state, &attack_context, &profile.on_miss, rng, log)?;
        run_owned_triggers_for(
            state,
            attacker,
            TriggerWhen::OnMiss,
            Some(target),
            profile,
            &attack_context,
            rng,
            log,
        )?;
    }

    // Ends-on events tied to attacking / dealing damage.
    fire_ends_on(state, attacker, EndsOn::OwnAttack, log);
    if hit {
        fire_ends_on(state, attacker, EndsOn::DealsDamage, log);
        fire_ends_on(state, target, EndsOn::DamageTaken, log);
    }
    finish_collected_modifiers(
        state,
        attacker,
        ModifierApplies::AttackRoll,
        &attacker_context,
        source,
        log,
    );
    finish_collected_modifiers(
        state,
        target,
        ModifierApplies::IncomingAttack,
        &target_context,
        None,
        log,
    );
    Ok(())
}

fn add_advantage(mode: RollMode) -> RollMode {
    match mode {
        RollMode::Normal => RollMode::Advantage,
        RollMode::Disadvantage => RollMode::Normal,
        RollMode::Advantage => RollMode::Advantage,
    }
}

fn add_disadvantage(mode: RollMode) -> RollMode {
    match mode {
        RollMode::Normal => RollMode::Disadvantage,
        RollMode::Advantage => RollMode::Normal,
        RollMode::Disadvantage => RollMode::Disadvantage,
    }
}

fn hostile_within_5(state: &EncounterState, index: usize) -> bool {
    let Some(space) = state.space() else {
        return false;
    };
    let zone = &state.combatants[index].zone;
    state
        .combatants
        .iter()
        .enumerate()
        .any(|(other, combatant)| {
            other != index
                && combatant.is_active()
                && combatant.side != state.combatants[index].side
                && space
                    .distance(zone, &combatant.zone)
                    .is_some_and(|distance| distance <= 5)
        })
}

#[allow(clippy::too_many_arguments)]
fn try_hit_reactions(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    profile: &AttackProfile,
    facts: &AttackFacts,
    attack_total: i32,
    current_ac: i32,
    source: Option<&EffectSource>,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<Option<(i32, Vec<(i32, String)>)>> {
    let abilities = state.combatants[target].owned_abilities(&state.content)?;
    let mut converted = None;
    let mut new_ac = current_ac;
    for resolved in abilities {
        for (trigger_index, trigger) in resolved.ability.triggers.iter().enumerate() {
            if trigger.on != TriggerWhen::OnHitTaken
                || (!trigger.reaction && resolved.ability.activation != Activation::Reaction)
            {
                continue;
            }
            if state.combatants[target].reaction_round == Some(state.round) {
                continue;
            }
            if has_flag(state, target, ConditionFlag::NoReactions)
                || is_incapacitated(state, target)
                || state.combatants[target].dormant
            {
                continue;
            }
            let key = format!(
                "trigger:{}:{}:{}",
                target, resolved.ability.id, trigger_index
            );
            if !trigger_available(state, target, trigger, &key) {
                continue;
            }
            if !trigger_affordable(state, target, trigger) {
                continue;
            }
            let context = PredicateContext {
                self_idx: target,
                actor: target,
                target: Some(target),
                attacker: Some(attacker),
                linked: None,
                attack: facts.clone(),
                has_advantage: false,
                has_disadvantage: false,
                damage_dealt: 0,
                slot_level: resolved.ability.slot_level,
                current_attack_total: Some(attack_total),
                current_ac: Some(new_ac),
                ability: profile.ability,
                damage_type: None,
                source_tags: resolved.ability.tags.clone(),
            };
            if let Some(when) = &trigger.when {
                if !eval_predicate(state, when, &context) {
                    continue;
                }
            }
            if let Some(cost) = trigger
                .cost
                .clone()
                .or_else(|| resolved.ability.cost.clone())
            {
                if !spend_cost(state, target, &cost, log)? {
                    continue;
                }
            }
            spend_reaction(state, target, log)?;
            mark_trigger_used(state, target, trigger, &key);
            let owner_name = state.combatants[target].name.clone();
            log.push(format!(
                "{owner_name} uses {} (reaction)",
                resolved.ability.display_name()
            ));
            let effect_source = EffectSource::from_ability(&resolved.ability, &resolved.params);
            let effect_context = EffectContext {
                actor: target,
                target: Some(target),
                source: Some(effect_source),
                attack: Some(profile.clone()),
                option: None,
                crit: false,
                damage_dealt: 0,
                damage_scale: 1.0,
                has_advantage: false,
                has_disadvantage: false,
            };
            apply_effects(state, &effect_context, &trigger.effects, rng, log)?;
            let (ac_after, terms_after) = effective_ac(state, target, Some(attacker), facts)?;
            new_ac = ac_after;
            converted = Some((ac_after, terms_after));
            if attack_total < new_ac {
                break;
            }
        }
        if attack_total < new_ac {
            break;
        }
    }
    let _ = source;
    Ok(converted)
}

fn trigger_available(state: &EncounterState, owner: usize, trigger: &Trigger, key: &str) -> bool {
    match trigger.once_per {
        Some(OncePer::Turn) => !state.combatants[owner]
            .once_used
            .contains_key(&turn_key(key)),
        Some(OncePer::Round) => state.combatants[owner]
            .once_used
            .get(&round_key(key))
            .is_none_or(|round| *round != state.round),
        None => true,
    }
}

fn mark_trigger_used(state: &mut EncounterState, owner: usize, trigger: &Trigger, key: &str) {
    let round = state.round;
    mark_once(&mut state.combatants[owner], key, trigger.once_per, round);
}

fn trigger_affordable(state: &EncounterState, owner: usize, trigger: &Trigger) -> bool {
    match &trigger.cost {
        Some(cost) => can_afford(state, owner, cost),
        None => true,
    }
}

fn can_afford(state: &EncounterState, owner: usize, cost: &AbilityCost) -> bool {
    state.combatants[owner]
        .resources
        .get(&cost.resource)
        .is_some_and(|resource| resource.current >= i32::try_from(cost.amount).unwrap_or(i32::MAX))
}

fn spend_cost(
    state: &mut EncounterState,
    owner: usize,
    cost: &AbilityCost,
    log: &mut Vec<String>,
) -> Result<bool> {
    let Some(resource) = state.combatants[owner].resources.get_mut(&cost.resource) else {
        bail!(
            "`{}` has no resource `{}`",
            state.combatants[owner].name,
            cost.resource
        );
    };
    let amount = i32::try_from(cost.amount).unwrap_or(i32::MAX);
    if resource.current < amount {
        return Ok(false);
    }
    resource.current -= amount;
    let name = state.combatants[owner].name.clone();
    log.push(format!(
        "{name} spends {} {}{}",
        cost.amount,
        cost.resource,
        if cost.amount == 1 { "" } else { "s" }
    ));
    Ok(true)
}

fn apply_attack_damage_parts(
    state: &mut EncounterState,
    context: &EffectContext,
    parts: &[DamagePart],
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<i32> {
    let Some(target) = context.target else {
        return Ok(0);
    };
    if parts.is_empty() {
        return Ok(0);
    }
    let attack_mod = context
        .attack
        .as_ref()
        .map(|attack| attack.attack_modifier(&state.combatants[context.actor]))
        .unwrap_or(0);
    let expr_context = source_context(
        state,
        context.actor,
        context.source.as_ref(),
        Some(attack_mod),
        None,
    );
    let mut rolls = Vec::new();
    for part in parts {
        let (_, record) = roll_value(&expr_context, &part.dice, context.crit, rng)?;
        rolls.push((record, part.damage_type.as_str().to_string()));
    }
    if let Some(first) = parts.first() {
        if let Some(bonus) = damage_roll_bonus(
            state,
            context.actor,
            context.source.as_ref(),
            Some(first.damage_type.as_str()),
            rng,
        )? {
            if let Some((record, _)) = rolls.first_mut() {
                record.total += bonus.record.total;
                record.text.push_str(&format!(" +{}", bonus.record.total));
            }
        }
    }
    let tags = context
        .source
        .as_ref()
        .map(|source| source.tags.clone())
        .unwrap_or_default();
    apply_damage_rolls(state, target, context.actor, rolls, &tags, rng, log)
}

fn run_mastery_triggers(
    state: &mut EncounterState,
    attacker: usize,
    _target: usize,
    profile: &AttackProfile,
    base_context: &EffectContext,
    when: TriggerWhen,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(mastery_id) = &profile.mastery else {
        return Ok(());
    };
    let Some(mastery) = state.content.masteries.get(mastery_id).cloned() else {
        return Ok(());
    };
    for (index, trigger) in mastery.triggers.iter().enumerate() {
        if trigger.on != when {
            continue;
        }
        let key = format!("mastery:{}:{index}", mastery.id);
        if !trigger_available(state, attacker, trigger, &key) {
            continue;
        }
        if !trigger_affordable(state, attacker, trigger) {
            continue;
        }
        let predicate_context = predicate_from_effect(state, base_context, profile.facts());
        if let Some(predicate) = &trigger.when {
            if !eval_predicate(state, predicate, &predicate_context) {
                continue;
            }
        }
        mark_trigger_used(state, attacker, trigger, &key);
        let source = EffectSource {
            id: mastery.id.clone(),
            name: mastery.display_name().to_string(),
            tags: Vec::new(),
            slot_level: None,
            concentration: false,
            params: BTreeMap::new(),
            duration: None,
            range: None,
            modifiers: mastery.modifiers.clone(),
        };
        let mut context = base_context.clone();
        context.source = Some(source);
        apply_effects(state, &context, &trigger.effects, rng, log)?;
    }
    Ok(())
}

fn predicate_from_effect(
    _state: &EncounterState,
    context: &EffectContext,
    facts: AttackFacts,
) -> PredicateContext {
    let target = context.target;
    let attacker = if context.attack.is_some() {
        Some(context.actor)
    } else {
        None
    };
    PredicateContext {
        self_idx: context.actor,
        actor: context.actor,
        target,
        attacker,
        linked: None,
        attack: facts,
        has_advantage: context.has_advantage,
        has_disadvantage: context.has_disadvantage,
        damage_dealt: context.damage_dealt,
        slot_level: context.source.as_ref().and_then(|source| source.slot_level),
        current_attack_total: None,
        current_ac: None,
        ability: context.attack.as_ref().and_then(|attack| attack.ability),
        damage_type: None,
        source_tags: context
            .source
            .as_ref()
            .map(|source| source.tags.clone())
            .unwrap_or_default(),
    }
}

fn run_owned_triggers_for(
    state: &mut EncounterState,
    owner: usize,
    when: TriggerWhen,
    target: Option<usize>,
    profile: &AttackProfile,
    base_context: &EffectContext,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let abilities = state.combatants[owner].owned_abilities(&state.content)?;
    for resolved in abilities {
        for (index, trigger) in resolved.ability.triggers.iter().enumerate() {
            if trigger.on != when {
                continue;
            }
            if trigger.reaction {
                continue;
            }
            if !trigger_affordable(state, owner, trigger) {
                continue;
            }
            let key = format!("trigger:{}:{}:{}", owner, resolved.ability.id, index);
            if !trigger_available(state, owner, trigger, &key) {
                continue;
            }
            let mut predicate_context = predicate_from_effect(state, base_context, profile.facts());
            predicate_context.target = target;
            predicate_context.self_idx = owner;
            predicate_context.actor = owner;
            if let Some(predicate) = &trigger.when {
                if !eval_predicate(state, predicate, &predicate_context) {
                    continue;
                }
            }
            if let Some(cost) = trigger
                .cost
                .clone()
                .or_else(|| resolved.ability.cost.clone())
            {
                if !spend_cost(state, owner, &cost, log).unwrap_or(false) {
                    continue;
                }
            }
            mark_trigger_used(state, owner, trigger, &key);
            let mut context = base_context.clone();
            context.actor = owner;
            context.target = target;
            context.source = Some(EffectSource::from_ability(
                &resolved.ability,
                &resolved.params,
            ));
            apply_effects(state, &context, &trigger.effects, rng, log)?;
        }
    }
    Ok(())
}

fn run_owned_triggers(
    state: &mut EncounterState,
    owner: usize,
    when: TriggerWhen,
    _target: Option<usize>,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) {
    let Ok(abilities) = state.combatants[owner].owned_abilities(&state.content) else {
        return;
    };
    for resolved in abilities {
        for (index, trigger) in resolved.ability.triggers.iter().enumerate() {
            if trigger.on != when {
                continue;
            }
            let key = format!("trigger:{}:{}:{}", owner, resolved.ability.id, index);
            if !trigger_available(state, owner, trigger, &key) {
                continue;
            }
            if !trigger_affordable(state, owner, trigger) {
                continue;
            }
            let context = PredicateContext {
                self_idx: owner,
                actor: owner,
                target: None,
                attacker: None,
                linked: None,
                attack: AttackFacts::default(),
                has_advantage: false,
                has_disadvantage: false,
                damage_dealt: 0,
                slot_level: resolved.ability.slot_level,
                current_attack_total: None,
                current_ac: None,
                ability: None,
                damage_type: None,
                source_tags: resolved.ability.tags.clone(),
            };
            if let Some(predicate) = &trigger.when {
                if !eval_predicate(state, predicate, &context) {
                    continue;
                }
            }
            let cost = trigger
                .cost
                .clone()
                .or_else(|| resolved.ability.cost.clone());
            if let Some(cost) = cost {
                if !spend_cost(state, owner, &cost, log).unwrap_or(false) {
                    continue;
                }
            }
            mark_trigger_used(state, owner, trigger, &key);
            let effect_source = EffectSource::from_ability(&resolved.ability, &resolved.params);
            let effect_context = EffectContext {
                actor: owner,
                target: None,
                source: Some(effect_source),
                attack: None,
                option: None,
                crit: false,
                damage_dealt: 0,
                damage_scale: 1.0,
                has_advantage: false,
                has_disadvantage: false,
            };
            let _ = apply_effects(state, &effect_context, &trigger.effects, rng, log);
        }
    }
}

fn fire_ends_on(state: &mut EncounterState, owner: usize, event: EndsOn, log: &mut Vec<String>) {
    let name = state.combatants[owner].name.clone();
    let mut kept = Vec::new();
    for instance in state.combatants[owner].conditions.drain(..) {
        let ends = instance.ends_on.contains(&event)
            || state
                .content
                .conditions
                .get(&instance.condition)
                .is_some_and(|def| def.ends_on.contains(&event));
        if ends {
            log.push(format!("{name} is no longer {}", instance.condition));
            if instance.concentration {
                // The caller may also clear concentration; this is best effort.
            }
        } else {
            kept.push(instance);
        }
    }
    state.combatants[owner].conditions = kept;
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EffectContext {
    actor: usize,
    target: Option<usize>,
    source: Option<EffectSource>,
    attack: Option<AttackProfile>,
    option: Option<usize>,
    crit: bool,
    damage_dealt: i32,
    damage_scale: f64,
    has_advantage: bool,
    has_disadvantage: bool,
}

fn apply_effects(
    state: &mut EncounterState,
    context: &EffectContext,
    effects: &[Effect],
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    for effect in effects {
        apply_effect(state, context, effect, rng, log)?;
    }
    Ok(())
}

fn apply_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &Effect,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    match effect {
        Effect::Attack(effect) => {
            let Some(target) = context.target else {
                bail!("attack effect needs a target");
            };
            let source = context.source.clone().unwrap_or(EffectSource {
                id: "attack".to_string(),
                name: "attack".to_string(),
                tags: Vec::new(),
                slot_level: None,
                concentration: false,
                params: BTreeMap::new(),
                duration: None,
                range: None,
                modifiers: Vec::new(),
            });
            let profile = AttackProfile::from_effect(effect, &source, source.range);
            let options = AttackOptions {
                advantage: context.has_advantage,
                disadvantage: context.has_disadvantage,
                multi: false,
                no_reactions: false,
            };
            resolve_attack(
                state,
                context.actor,
                target,
                &profile,
                Some(&source),
                &options,
                rng,
                log,
                true,
            )
        }
        Effect::Save(effect) => apply_save_effect(state, context, effect, rng, log),
        Effect::Damage(effect) => apply_damage_effect(state, context, effect, rng, log),
        Effect::Heal(effect) => apply_heal_effect(state, context, effect, rng, log),
        Effect::ApplyCondition(effect) => {
            let target = match effect.to {
                TargetRef::SelfTarget => context.actor,
                TargetRef::Target => context
                    .target
                    .ok_or_else(|| anyhow!("apply_condition needs a target"))?,
            };
            let linked = if effect.link {
                Some(state.combatants[context.actor].name.clone())
            } else {
                None
            };
            let save_ends = effect
                .save_ends
                .as_ref()
                .map(|save_ends| {
                    let attack_mod = context
                        .attack
                        .as_ref()
                        .map(|attack| attack.attack_modifier(&state.combatants[context.actor]));
                    let expr_context = source_context(
                        state,
                        context.actor,
                        context.source.as_ref(),
                        attack_mod,
                        context.source.as_ref().and_then(|source| source.slot_level),
                    );
                    let (_, dc) = evaluate_value(&expr_context, &save_ends.dc)?;
                    Ok::<_, anyhow::Error>((save_ends.ability, dc.modifier, save_ends.at))
                })
                .transpose()?;
            let duration = effect.duration.clone().or_else(|| {
                context
                    .source
                    .as_ref()
                    .and_then(|source| source.duration.clone())
            });
            let mut ends_on = effect.ends_on.clone();
            if let Some(duration) = &duration {
                ends_on.extend(duration.ends_on.iter().copied());
            }
            apply_condition_internal(
                state,
                target,
                &effect.condition,
                Some(context.actor),
                linked,
                duration,
                save_ends,
                ends_on,
                context.source.as_ref().map(|source| source.id.clone()),
                context
                    .source
                    .as_ref()
                    .is_some_and(|source| source.concentration),
                rng,
                log,
            )
        }
        Effect::RemoveCondition(effect) => {
            let target = match effect.from {
                TargetRef::SelfTarget => context.actor,
                TargetRef::Target => context
                    .target
                    .ok_or_else(|| anyhow!("remove_condition needs a target"))?,
            };
            remove_condition_from(state, target, &effect.condition, log);
            Ok(())
        }
        Effect::ForcedMove(effect) => apply_forced_move(state, context, effect, log),
        Effect::Use(reference) => {
            let resolved = state.combatants[context.actor]
                .find_owned_ability(&state.content, reference)
                .or_else(|_| {
                    state.combatants[context.actor]
                        .find_attack(reference)
                        .map(|attack| ResolvedAbility {
                            ability: AttackProfile::from_attack(attack).into_ability(),
                            params: BTreeMap::new(),
                        })
                })?;
            if let Some(target) = context.target {
                if state.combatants[context.actor]
                    .find_attack(reference)
                    .is_ok()
                {
                    let attack = state.combatants[context.actor].find_attack(reference)?;
                    let profile = AttackProfile::from_attack(attack);
                    return resolve_attack(
                        state,
                        context.actor,
                        target,
                        &profile,
                        None,
                        &AttackOptions::default(),
                        rng,
                        log,
                        true,
                    );
                }
            }
            let source = EffectSource::from_ability(&resolved.ability, &resolved.params);
            let targets: Vec<usize> = context.target.into_iter().collect();
            use_resolved_ability(
                state,
                context.actor,
                &resolved,
                &source,
                &targets,
                &AttackOptions::default(),
                context.option,
                context.source.as_ref().and_then(|source| source.slot_level),
                true,
                rng,
                log,
            )
        }
        Effect::Activate(effect) => {
            activate(state, &effect.creature, rng, log)?;
            Ok(())
        }
        Effect::Check(effect) => apply_check_effect(state, context, effect, rng, log),
        Effect::BasicAction(action) => apply_basic_action(state, context, *action, log),
        Effect::Choice(options) => {
            if options.is_empty() {
                return Ok(());
            }
            let index = match context.option {
                Some(option) if option >= 1 && option <= options.len() => option - 1,
                Some(option) => {
                    bail!(
                        "choice option {option} is out of range; choose 1-{}",
                        options.len()
                    )
                }
                None if options.len() == 1 => 0,
                None => {
                    let listing = options
                        .iter()
                        .enumerate()
                        .map(|(index, option)| {
                            let first_note = option.iter().find_map(|effect| match effect {
                                Effect::Note(text) => Some(text.clone()),
                                _ => None,
                            });
                            format!(
                                "{}: {}",
                                index + 1,
                                first_note.unwrap_or_else(|| "effect".to_string())
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    bail!(
                        "this ability requires --option N (1-{}): {listing}",
                        options.len()
                    )
                }
            };
            apply_effects(state, context, &options[index], rng, log)
        }
        Effect::Note(text) => {
            log.push(format!("MANUAL: {text}"));
            Ok(())
        }
        Effect::ToggleTrigger(effect) => {
            toggle_scene_trigger(state, effect, log);
            Ok(())
        }
    }
}

impl AttackProfile {
    fn into_ability(self) -> Ability {
        Ability {
            id: self.id,
            name: Some(self.name),
            tags: self.tags,
            activation: Activation::Action,
            cost: None,
            slot_level: None,
            range: self.range.map(|range| range[0]),
            target: None,
            area: None,
            concentration: false,
            duration: None,
            effects: vec![Effect::Attack(AttackEffect {
                to_hit: self.to_hit,
                kind: self.kind,
                damage: self.damage,
                on_hit: self.on_hit,
                on_miss: self.on_miss,
                properties: self.properties,
                mastery: self.mastery,
            })],
            triggers: Vec::new(),
            modifiers: Vec::new(),
            ai: None,
            manual: None,
            source: None,
            todo: Vec::new(),
            notes: None,
            description: None,
        }
    }
}

fn apply_save_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &SaveEffect,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(target) = context.target else {
        bail!("save effect needs a target");
    };
    let attack_mod = context
        .attack
        .as_ref()
        .map(|attack| attack.attack_modifier(&state.combatants[context.actor]));
    let expr_context = source_context(
        state,
        context.actor,
        context.source.as_ref(),
        attack_mod,
        context.source.as_ref().and_then(|source| source.slot_level),
    );
    let (_, dc) = evaluate_value(&expr_context, &effect.dc)?;
    let dc = dc.modifier;
    let (mode, bonuses, adv_sources, dis_sources, auto_fail) =
        save_modifiers(state, target, effect.ability, &expr_context, rng)?;
    if auto_fail {
        log.push(format!(
            "{} automatically fails the {} save (DC {dc})",
            state.combatants[target].name,
            effect.ability.as_str().to_uppercase()
        ));
        let fail_context = EffectContext {
            damage_scale: context.damage_scale,
            ..context.clone()
        };
        apply_effects(state, &fail_context, &effect.on_fail, rng, log)?;
        return Ok(());
    }
    let base = state.combatants[target].save_bonus(effect.ability);
    let result = roll_d20(
        state,
        target,
        base,
        mode,
        bonuses,
        adv_sources,
        dis_sources,
        rng,
    );
    let success = result.total >= dc;
    let name = state.combatants[target].name.clone();
    log.push(format!(
        "{name} {} save: {} vs DC {dc} {}",
        effect.ability.as_str().to_uppercase(),
        result.text(),
        if success { "PASS" } else { "FAIL" }
    ));
    if success {
        if effect.half {
            let mut half_context = context.clone();
            half_context.damage_scale = 0.5;
            for effect in &effect.on_fail {
                if matches!(effect, Effect::Damage(_)) {
                    apply_effect(state, &half_context, effect, rng, log)?;
                }
            }
        }
        apply_effects(state, context, &effect.on_success, rng, log)?;
    } else {
        apply_effects(state, context, &effect.on_fail, rng, log)?;
    }
    Ok(())
}

fn save_modifiers(
    state: &EncounterState,
    target: usize,
    ability: AbilityName,
    _expr_context: &ExprContext,
    rng: &mut dyn DiceRng,
) -> Result<(RollMode, Vec<BonusTerm>, Vec<String>, Vec<String>, bool)> {
    let combatant = &state.combatants[target];
    let context = PredicateContext {
        self_idx: target,
        actor: target,
        target: Some(target),
        attacker: None,
        linked: None,
        attack: AttackFacts::default(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: None,
        current_attack_total: None,
        current_ac: None,
        ability: Some(ability),
        damage_type: None,
        source_tags: Vec::new(),
    };
    let mut mode = RollMode::Normal;
    let mut advantages = Vec::new();
    let mut disadvantages = Vec::new();
    let mut bonuses = Vec::new();
    let mut auto_fail = false;
    for collected in collect_modifiers(state, target, ModifierApplies::Save, &context, None) {
        match collected.modifier.effect {
            ModifierEffect::Advantage => {
                advantages.push(collected.label.clone());
                mode = add_advantage(mode);
            }
            ModifierEffect::Disadvantage => {
                disadvantages.push(collected.label.clone());
                mode = add_disadvantage(mode);
            }
            ModifierEffect::AutoFail => auto_fail = true,
            ModifierEffect::Bonus => {
                let expr_context = score_context(combatant);
                let value = collected
                    .modifier
                    .value
                    .as_ref()
                    .ok_or_else(|| anyhow!("save bonus has no value"))?;
                let (_, expression) = evaluate_value(&expr_context, value)?;
                bonuses.push(BonusTerm {
                    label: collected.label,
                    record: expression.roll(rng),
                });
            }
            _ => {}
        }
    }
    Ok((mode, bonuses, advantages, disadvantages, auto_fail))
}

fn apply_damage_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &DamageEffect,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(target) = context.target else {
        bail!("damage effect needs a target");
    };
    let expr_context = source_context(
        state,
        context.actor,
        context.source.as_ref(),
        context
            .attack
            .as_ref()
            .map(|attack| attack.attack_modifier(&state.combatants[context.actor])),
        context.source.as_ref().and_then(|source| source.slot_level),
    );
    let crit = context.crit && effect.crit == CritMode::Inherit;
    let mut damage_type = effect.damage_type.as_str().to_string();
    if damage_type.eq_ignore_ascii_case("weapon") {
        damage_type = context
            .attack
            .as_ref()
            .and_then(|attack| attack.damage.first())
            .map(|part| part.damage_type.as_str().to_string())
            .unwrap_or_else(|| "untyped".to_string());
    }
    let mut rolls = Vec::new();
    for _ in 0..effect.repeat.max(1) {
        let (_, record) = roll_value(&expr_context, &effect.dice, crit, rng)?;
        let damage = ((record.total as f64) * context.damage_scale).floor() as i32;
        let text = if (context.damage_scale - 1.0).abs() > f64::EPSILON {
            format!("{} x{}", record.text, context.damage_scale)
        } else {
            record.text.clone()
        };
        let mut text = text;
        if let Some(source) = &context.source {
            text.push_str(&format!(" [{}]", source.name));
        }
        rolls.push((
            RollRecord {
                total: damage,
                text,
                ..record
            }
            .clone(),
            damage_type.clone(),
        ));
    }
    if let Some(bonus) = damage_roll_bonus(
        state,
        context.actor,
        context.source.as_ref(),
        Some(&damage_type),
        rng,
    )? {
        if let Some((record, _)) = rolls.first_mut() {
            record.total += bonus.record.total;
            record.text.push_str(&format!(" +{}", bonus.record.total));
        }
    }
    let tags = context
        .source
        .as_ref()
        .map(|source| source.tags.clone())
        .unwrap_or_default();
    let dealt = apply_damage_rolls(state, target, context.actor, rolls, &tags, rng, log)?;
    if dealt > 0 {
        fire_ends_on(state, target, EndsOn::DamageTaken, log);
        fire_ends_on(state, context.actor, EndsOn::DealsDamage, log);
    }
    Ok(())
}

fn apply_heal_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &crate::content::schema::HealEffect,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(target) = context.target else {
        bail!("heal effect needs a target");
    };
    let expr_context = source_context(
        state,
        context.actor,
        context.source.as_ref(),
        None,
        context.source.as_ref().and_then(|source| source.slot_level),
    );
    let (_, record) = roll_value(&expr_context, &effect.dice, false, rng)?;
    let mut amount = record.total;
    let mut terms = Vec::new();
    let healing_context = PredicateContext {
        self_idx: context.actor,
        actor: context.actor,
        target: Some(target),
        attacker: None,
        linked: None,
        attack: AttackFacts::default(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: context.source.as_ref().and_then(|source| source.slot_level),
        current_attack_total: None,
        current_ac: None,
        ability: None,
        damage_type: None,
        source_tags: context
            .source
            .as_ref()
            .map(|source| source.tags.clone())
            .unwrap_or_default(),
    };
    for collected in collect_modifiers(
        state,
        context.actor,
        ModifierApplies::HealingDone,
        &healing_context,
        context.source.as_ref(),
    ) {
        if collected.modifier.effect == ModifierEffect::Bonus {
            if let Some(value) = &collected.modifier.value {
                let (_, expression) = evaluate_value(&expr_context, value)?;
                let extra = expression.roll(rng).total;
                amount += extra;
                terms.push(format!("+{extra}[{}]", collected.label));
            }
        }
    }
    let received_context = PredicateContext {
        self_idx: target,
        ..healing_context.clone()
    };
    for collected in collect_modifiers(
        state,
        target,
        ModifierApplies::HealingReceived,
        &received_context,
        None,
    ) {
        if collected.modifier.effect == ModifierEffect::Bonus {
            if let Some(value) = &collected.modifier.value {
                let (_, expression) = evaluate_value(&expr_context, value)?;
                let extra = expression.roll(rng).total;
                amount += extra;
                terms.push(format!("+{extra}[{}]", collected.label));
            }
        }
    }
    let name = state.combatants[target].name.clone();
    let before = state.combatants[target].hp;
    let max_hp = state.combatants[target].max_hp;
    state.combatants[target].hp = (before + amount).min(max_hp);
    let healed = state.combatants[target].hp - before;
    log.push(format!(
        "{name} regains {healed} HP ({} {})",
        record.text,
        terms.join(" ")
    ));
    if before == 0 && state.combatants[target].hp > 0 && !state.combatants[target].is_dead() {
        revive_from_zero(state, target, log);
    }
    Ok(())
}

fn revive_from_zero(state: &mut EncounterState, target: usize, log: &mut Vec<String>) {
    let dying = state
        .content
        .rules
        .dying_conditions
        .clone()
        .unwrap_or_default();
    let name = state.combatants[target].name.clone();
    for condition in dying {
        let before = state.combatants[target].conditions.len();
        state.combatants[target]
            .conditions
            .retain(|active| !active.condition.eq_ignore_ascii_case(&condition));
        if state.combatants[target].conditions.len() != before {
            log.push(format!("{name} is no longer {condition}"));
        }
    }
    state.combatants[target].death_saves = DeathSaves::default();
}

fn apply_forced_move(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &ForcedMoveEffect,
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(target) = context.target else {
        bail!("forced_move needs a target");
    };
    let source_zone = state.combatants[context.actor].zone.clone();
    let target_zone = state.combatants[target].zone.clone();
    if !check_size_limit(state.combatants[target].creature.size(), effect.max_size) {
        return Ok(());
    }
    let Some(space) = state.space() else {
        return Ok(());
    };
    let mut candidates = Vec::new();
    for (zone, cost) in space.neighbours(&target_zone) {
        if cost > effect.distance.max(0) {
            continue;
        }
        let Some(distance) = space.distance(&source_zone, &zone) else {
            continue;
        };
        candidates.push((zone, distance));
    }
    let chosen = match effect.direction {
        Direction::Away => candidates.into_iter().max_by_key(|(_, distance)| *distance),
        Direction::Toward => candidates.into_iter().min_by_key(|(_, distance)| *distance),
    };
    if let Some((zone, _)) = chosen {
        let name = state.combatants[target].name.clone();
        let from = state.combatants[target].zone.clone();
        state.combatants[target].zone = zone.clone();
        log.push(format!("{name} is moved {from} -> {zone}"));
    }
    Ok(())
}

fn apply_check_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &CheckEffect,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let who = match effect.who {
        CheckWho::Actor => context.actor,
        CheckWho::Target => context
            .target
            .ok_or_else(|| anyhow!("check effect needs a target"))?,
    };
    let (label, base) = if let Some(skill) = &effect.skill {
        (skill.clone(), state.combatants[who].skill_bonus(skill))
    } else if let Some(ability) = effect.ability {
        (
            ability.as_str().to_string(),
            state.combatants[who].creature.ability_modifier(ability),
        )
    } else {
        bail!("check effect needs `skill` or `ability`");
    };
    let expr_context = score_context(&state.combatants[context.actor]);
    let (_, dc) = evaluate_value(&expr_context, &effect.dc)?;
    let dc = dc.modifier;
    let (mode, advantages, disadvantages) = check_modifiers(state, who, effect.ability, &label);
    let result = roll_d20(
        state,
        who,
        base,
        mode,
        Vec::new(),
        advantages,
        disadvantages,
        rng,
    );
    let success = result.total >= dc;
    let name = state.combatants[who].name.clone();
    log.push(format!(
        "{name} {label} check: {} vs DC {dc} {}",
        result.text(),
        if success { "PASS" } else { "FAIL" }
    ));
    if success {
        apply_effects(state, context, &effect.on_success, rng, log)
    } else {
        apply_effects(state, context, &effect.on_fail, rng, log)
    }
}

fn check_modifiers(
    state: &EncounterState,
    who: usize,
    ability: Option<AbilityName>,
    skill: &str,
) -> (RollMode, Vec<String>, Vec<String>) {
    let context = PredicateContext {
        self_idx: who,
        actor: who,
        target: None,
        attacker: None,
        linked: None,
        attack: AttackFacts::default(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: None,
        current_attack_total: None,
        current_ac: None,
        ability: ability.or_else(|| ability_from_text(skill)),
        damage_type: None,
        source_tags: Vec::new(),
    };
    let mut mode = RollMode::Normal;
    let mut advantages = Vec::new();
    let mut disadvantages = Vec::new();
    for collected in collect_modifiers(state, who, ModifierApplies::Check, &context, None) {
        match collected.modifier.effect {
            ModifierEffect::Advantage => {
                advantages.push(collected.label.clone());
                mode = add_advantage(mode);
            }
            ModifierEffect::Disadvantage => {
                disadvantages.push(collected.label.clone());
                mode = add_disadvantage(mode);
            }
            _ => {}
        }
    }
    (mode, advantages, disadvantages)
}

fn apply_basic_action(
    state: &mut EncounterState,
    context: &EffectContext,
    action: BasicAction,
    log: &mut Vec<String>,
) -> Result<()> {
    let actor = context.actor;
    let name = state.combatants[actor].name.clone();
    let key = match action {
        BasicAction::Dash => "dash",
        BasicAction::Disengage => "disengage",
        BasicAction::Dodge => "dodge",
        BasicAction::Hide => "hide",
        BasicAction::Help => "help",
    };
    log.push(format!("{name} takes the {key} action"));
    if action == BasicAction::Dash {
        let speed = effective_speed(state, actor);
        state.combatants[actor].speed_left += speed;
        return Ok(());
    }
    let condition_id = state
        .content
        .rules
        .basic_actions
        .as_ref()
        .and_then(|actions| actions.get(key))
        .cloned();
    if let Some(condition_id) = condition_id {
        let target = if action == BasicAction::Help {
            context.target.unwrap_or(actor)
        } else {
            actor
        };
        let duration = state
            .content
            .conditions
            .get(&condition_id)
            .and_then(|def| def.duration_default.clone())
            .or_else(|| {
                Some(Duration {
                    rounds: None,
                    until: Some(DurationUntil::StartOfSourceNextTurn),
                    minutes: None,
                    permanent: None,
                    ends_on: Vec::new(),
                })
            });
        apply_condition_internal(
            state,
            target,
            &condition_id,
            Some(actor),
            None,
            duration,
            None,
            Vec::new(),
            None,
            false,
            &mut seeded_rng(0),
            log,
        )?;
    } else {
        log.push(format!("MANUAL: resolve the {key} action"));
    }
    Ok(())
}

fn toggle_scene_trigger(
    state: &mut EncounterState,
    effect: &ToggleTriggerEffect,
    log: &mut Vec<String>,
) {
    let key = state
        .scene
        .as_ref()
        .and_then(|scene| {
            scene
                .triggers
                .iter()
                .enumerate()
                .find(|(_, trigger)| trigger.id.as_deref() == Some(effect.trigger.as_str()))
                .map(|(index, trigger)| EncounterState::trigger_key(index, trigger))
        })
        .unwrap_or_else(|| effect.trigger.clone());
    state.set_trigger_enabled(&key, effect.enabled);
    log.push(format!(
        "Scene trigger {} is now {}",
        effect.trigger,
        if effect.enabled {
            "enabled"
        } else {
            "disabled"
        }
    ));
}

// ---------------------------------------------------------------------------
// Conditions
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn apply_condition_internal(
    state: &mut EncounterState,
    target: usize,
    condition_id: &str,
    source: Option<usize>,
    linked: Option<String>,
    duration: Option<Duration>,
    save_ends: Option<(AbilityName, i32, SaveEndsAt)>,
    ends_on: Vec<EndsOn>,
    created_by: Option<String>,
    concentration: bool,
    _rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(def) = state.content.conditions.get(condition_id) else {
        bail!("unknown condition `{condition_id}`");
    };
    let target_name = state.combatants[target].name.clone();
    if state.combatants[target].is_immune_to_condition(condition_id) {
        log.push(format!("{target_name} is immune to {condition_id}"));
        return Ok(());
    }
    let def = def.clone();
    let source_name = source.map(|index| state.combatants[index].name.clone());
    let duration = duration.or_else(|| def.duration_default.clone());
    let active_duration = duration.as_ref().map(|duration| {
        if let Some(until) = duration.until {
            let subject = match until {
                DurationUntil::EndOfTargetNextTurn => target_name.clone(),
                DurationUntil::EndOfSourceNextTurn | DurationUntil::StartOfSourceNextTurn => {
                    source_name.clone().unwrap_or_else(|| target_name.clone())
                }
            };
            let end_round = next_named_turn_round(state, &subject);
            ActiveDuration::Until {
                until,
                subject,
                end_round,
            }
        } else {
            active_duration(duration, &target_name, state.round)
        }
    });
    let mut ends = ends_on;
    ends.extend(def.ends_on.iter().copied());
    ends.sort_by_key(|event| format!("{event:?}"));
    ends.dedup();
    let active = ActiveCondition {
        condition: condition_id.to_string(),
        source: source_name,
        linked,
        duration: active_duration,
        save_ends: save_ends.map(|(ability, dc, at)| ActiveSaveEnds { ability, dc, at }),
        ends_on: ends,
        created_by,
        concentration,
        applied_round: state.round,
        notes: None,
    };
    if let Some(existing) = state.combatants[target]
        .conditions
        .iter_mut()
        .find(|existing| existing.condition.eq_ignore_ascii_case(condition_id))
    {
        *existing = active;
        log.push(format!("{target_name}'s {condition_id} is refreshed"));
    } else {
        state.combatants[target].conditions.push(active);
        log.push(format!("{target_name} is now {condition_id}"));
    }
    Ok(())
}

fn next_named_turn_round(state: &EncounterState, subject: &str) -> u32 {
    if state.round == 0 || state.initiative.is_empty() {
        return state.round.saturating_add(1);
    }
    let Some(index) = state
        .combatants
        .iter()
        .position(|combatant| combatant.name.eq_ignore_ascii_case(subject))
    else {
        return state.round.saturating_add(1);
    };
    let Some(position) = state.initiative.iter().position(|entry| *entry == index) else {
        return state.round.saturating_add(1);
    };
    if position > state.turn {
        state.round
    } else {
        state.round.saturating_add(1)
    }
}

fn remove_condition_from(
    state: &mut EncounterState,
    target: usize,
    condition_id: &str,
    log: &mut Vec<String>,
) {
    let name = state.combatants[target].name.clone();
    let before = state.combatants[target].conditions.len();
    state.combatants[target]
        .conditions
        .retain(|condition| !condition.condition.eq_ignore_ascii_case(condition_id));
    if state.combatants[target].conditions.len() != before {
        log.push(format!("{name} is no longer {condition_id}"));
        let concentration_condition = state.content.rules.concentration_condition.clone();
        if concentration_condition
            .as_deref()
            .is_some_and(|condition| condition.eq_ignore_ascii_case(condition_id))
        {
            break_concentration(state, target, log);
        }
    }
}

/// CLI `condition add`.
pub fn add_condition(
    state: &mut EncounterState,
    who: &str,
    condition_id: &str,
    options: &AddConditionOptions,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let target = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    if options.rounds.is_none()
        && options.minutes.is_none()
        && options.until.is_none()
        && !options.permanent
        && state.content.conditions.get(condition_id).is_none()
    {
        bail!("unknown condition `{condition_id}`");
    }
    let duration = if let Some(rounds) = options.rounds {
        Some(Duration {
            rounds: Some(rounds),
            until: None,
            minutes: None,
            permanent: None,
            ends_on: Vec::new(),
        })
    } else if let Some(minutes) = options.minutes {
        Some(Duration {
            rounds: None,
            until: None,
            minutes: Some(minutes),
            permanent: None,
            ends_on: Vec::new(),
        })
    } else if let Some(until) = options.until {
        let subject = options
            .until_subject
            .clone()
            .unwrap_or_else(|| state.combatants[target].name.clone());
        let end_round = next_named_turn_round(state, &subject);
        let _ = end_round;
        Some(Duration {
            rounds: None,
            until: Some(until),
            minutes: None,
            permanent: None,
            ends_on: Vec::new(),
        })
    } else if options.permanent {
        Some(Duration {
            rounds: None,
            until: None,
            minutes: None,
            permanent: Some(true),
            ends_on: Vec::new(),
        })
    } else {
        None
    };
    let source = options
        .source
        .as_deref()
        .map(|name| {
            state
                .find_index(name)
                .map_err(|error| anyhow!(error.to_string()))
        })
        .transpose()?;
    let linked = options
        .linked
        .as_deref()
        .map(|name| {
            state
                .find_index(name)
                .map(|index| state.combatants[index].name.clone())
                .map_err(|error| anyhow!(error.to_string()))
        })
        .transpose()?;
    let save_ends = options
        .save_ends
        .map(|(ability, dc)| (ability, dc, SaveEndsAt::EndOfTurn));
    apply_condition_internal(
        state,
        target,
        condition_id,
        source,
        linked,
        duration,
        save_ends,
        Vec::new(),
        None,
        false,
        rng,
        log,
    )
}

/// CLI `condition remove`.
pub fn remove_condition(
    state: &mut EncounterState,
    who: &str,
    condition_id: &str,
    log: &mut Vec<String>,
) -> Result<()> {
    let target = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    remove_condition_from(state, target, condition_id, log);
    Ok(())
}

// ---------------------------------------------------------------------------
// Ability use
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn use_resolved_ability(
    state: &mut EncounterState,
    actor: usize,
    resolved: &ResolvedAbility,
    source: &EffectSource,
    targets: &[usize],
    options: &AttackOptions,
    option: Option<usize>,
    slot_level: Option<i32>,
    auto: bool,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let ability = &resolved.ability;
    ensure_can_act(state, actor)?;
    let target_spec = ability.target.clone().unwrap_or_default();
    match target_spec.target_type {
        TargetType::Zone => bail!("zone/area targeting arrives in step 3"),
        TargetType::SelfTarget => {
            if !targets.is_empty() && targets.iter().any(|target| *target != actor) {
                bail!("`{}` targets self", ability.display_name());
            }
        }
        TargetType::Creature => {
            if targets.is_empty() {
                bail!("`{}` needs a target", ability.display_name());
            }
            let max = target_spec.count.unwrap_or(1).max(1) as usize;
            if targets.len() > max {
                bail!(
                    "`{}` targets at most {max} creature(s)",
                    ability.display_name()
                );
            }
            for target in targets {
                if !target_matches_side(state, actor, *target, target_spec.side) {
                    bail!(
                        "`{}` cannot target `{}` (target side restriction)",
                        ability.display_name(),
                        state.combatants[*target].name
                    );
                }
                ensure_targetable(state, *target)?;
            }
        }
    }
    if let Some(cost) = &ability.cost {
        if !can_afford(state, actor, cost) {
            bail!(
                "`{}` cannot afford {} {}",
                state.combatants[actor].name,
                cost.amount,
                cost.resource
            );
        }
    }
    if !auto {
        spend_action(state, actor, ability.activation, log)?;
    }
    if let Some(cost) = &ability.cost {
        let _ = spend_cost(state, actor, cost, log)?;
    }
    if ability.concentration {
        begin_concentration(state, actor, ability, log);
    }
    if let Some(manual) = &ability.manual {
        log.push(format!("MANUAL: {manual}"));
    }
    let slot_level = slot_level.or(ability.slot_level);
    let mut effect_source = source.clone();
    if let Some(slot_level) = slot_level {
        effect_source.slot_level = Some(slot_level);
    }
    let source_is_spell = effect_source
        .tags
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case("spell"));
    let context_base = EffectContext {
        actor,
        target: None,
        source: Some(effect_source),
        attack: None,
        option,
        crit: false,
        damage_dealt: 0,
        damage_scale: 1.0,
        has_advantage: options.advantage,
        has_disadvantage: options.disadvantage,
    };
    log.push(format!(
        "{} uses {}",
        state.combatants[actor].name,
        ability.display_name()
    ));
    match target_spec.target_type {
        TargetType::SelfTarget => {
            let context = EffectContext {
                target: Some(actor),
                ..context_base
            };
            apply_effects(state, &context, &ability.effects, rng, log)?;
        }
        TargetType::Creature => {
            for target in targets {
                let context = EffectContext {
                    target: Some(*target),
                    ..context_base.clone()
                };
                apply_effects(state, &context, &ability.effects, rng, log)?;
            }
        }
        TargetType::Zone => unreachable!("zone targeting was rejected above"),
    }
    if source_is_spell {
        fire_ends_on(state, actor, EndsOn::OwnSpell, log);
    }
    let _ = options;
    Ok(())
}

fn target_matches_side(
    state: &EncounterState,
    actor: usize,
    target: usize,
    side: Option<TargetSide>,
) -> bool {
    match side {
        None | Some(TargetSide::Any) => true,
        Some(TargetSide::Enemy) => hostile_to(state, actor, target),
        Some(TargetSide::Ally) => state.combatants[actor].side == state.combatants[target].side,
    }
}

/// CLI `cast`/`use` for an owned ability.
pub fn use_ability(
    state: &mut EncounterState,
    actor_name: &str,
    ability_name: &str,
    options: &UseOptions,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let actor = state
        .find_index(actor_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, actor)?;
    let resolved = state.combatants[actor].find_owned_ability(&state.content, ability_name)?;
    let source = EffectSource::from_ability(&resolved.ability, &resolved.params);
    let mut targets = Vec::new();
    for name in &options.targets {
        let index = state
            .find_index(name)
            .map_err(|error| anyhow!(error.to_string()))?;
        targets.push(index);
    }
    let attack_options = AttackOptions {
        advantage: options.advantage,
        disadvantage: options.disadvantage,
        multi: false,
        no_reactions: options.no_reactions,
    };
    use_resolved_ability(
        state,
        actor,
        &resolved,
        &source,
        &targets,
        &attack_options,
        options.option,
        options.slot_level,
        false,
        rng,
        log,
    )?;
    log_outcome(state, log);
    Ok(())
}

/// CLI `use <who> <attack-or-ability>`: falls back to a creature attack.
pub fn use_entry(
    state: &mut EncounterState,
    actor_name: &str,
    name: &str,
    targets: &[String],
    options: &UseOptions,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let actor = state
        .find_index(actor_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    if state.combatants[actor].find_attack(name).is_ok() {
        let Some(target_name) = targets.first() else {
            bail!("attack `{name}` needs a target");
        };
        let target = state
            .find_index(target_name)
            .map_err(|error| anyhow!(error.to_string()))?;
        ensure_can_act(state, actor)?;
        ensure_targetable(state, target)?;
        spend_action(state, actor, Activation::Action, log)?;
        let attack = state.combatants[actor].find_attack(name)?.clone();
        let profile = AttackProfile::from_attack(&attack);
        let options = AttackOptions {
            advantage: options.advantage,
            disadvantage: options.disadvantage,
            multi: false,
            no_reactions: options.no_reactions,
        };
        resolve_attack(
            state, actor, target, &profile, None, &options, rng, log, false,
        )?;
        log_outcome(state, log);
        return Ok(());
    }
    use_ability(state, actor_name, name, options, rng, log)
}

// ---------------------------------------------------------------------------
// Concentration
// ---------------------------------------------------------------------------

fn begin_concentration(
    state: &mut EncounterState,
    actor: usize,
    ability: &Ability,
    log: &mut Vec<String>,
) {
    let name = state.combatants[actor].name.clone();
    if state.concentration.contains_key(&name) {
        break_concentration(state, actor, log);
    }
    state.concentration.insert(
        name.clone(),
        crate::state::ConcentrationState {
            ability: Some(ability.id.clone()),
            round: state.round,
        },
    );
    if let Some(condition_id) = state.content.rules.concentration_condition.clone() {
        let mut rng = seeded_rng(0);
        let _ = apply_condition_internal(
            state,
            actor,
            &condition_id,
            Some(actor),
            None,
            None,
            None,
            Vec::new(),
            Some(ability.id.clone()),
            true,
            &mut rng,
            log,
        );
    }
    log.push(format!(
        "{name} is concentrating on {}",
        ability.display_name()
    ));
}

fn break_concentration(state: &mut EncounterState, actor: usize, log: &mut Vec<String>) {
    let name = state.combatants[actor].name.clone();
    if state.concentration.remove(&name).is_none() {
        return;
    }
    log.push(format!("{name}'s concentration ends"));
    for combatant in &mut state.combatants {
        let mut kept = Vec::new();
        for condition in combatant.conditions.drain(..) {
            if condition.concentration && condition.source.as_deref() == Some(name.as_str()) {
                log.push(format!(
                    "{} is no longer {}",
                    combatant.name, condition.condition
                ));
            } else {
                kept.push(condition);
            }
        }
        combatant.conditions = kept;
    }
    if let Some(condition_id) = state.content.rules.concentration_condition.clone() {
        state.combatants[actor]
            .conditions
            .retain(|condition| !condition.condition.eq_ignore_ascii_case(&condition_id));
    }
}

fn concentration_damage_check(
    state: &mut EncounterState,
    target: usize,
    damage: i32,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let name = state.combatants[target].name.clone();
    if !state.concentration.contains_key(&name) || damage <= 0 {
        return Ok(());
    }
    let dc = 10.max(damage / 2);
    let (mode, bonuses, advantages, disadvantages, auto_fail) = save_modifiers(
        state,
        target,
        AbilityName::Con,
        &score_context(&state.combatants[target]),
        rng,
    )?;
    if auto_fail {
        log.push(format!(
            "{name} automatically fails the concentration save (DC {dc})"
        ));
        break_concentration(state, target, log);
        return Ok(());
    }
    let bonus = state.combatants[target].save_bonus(AbilityName::Con);
    let result = roll_d20(
        state,
        target,
        bonus,
        mode,
        bonuses,
        advantages,
        disadvantages,
        rng,
    );
    log.push(format!(
        "{name} concentration save: {} vs DC {dc} {}",
        result.text(),
        if result.total >= dc { "PASS" } else { "FAIL" }
    ));
    if result.total < dc {
        break_concentration(state, target, log);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Damage and healing
// ---------------------------------------------------------------------------

fn apply_damage_rolls(
    state: &mut EncounterState,
    target: usize,
    source: usize,
    rolls: Vec<(RollRecord, String)>,
    source_tags: &[String],
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<i32> {
    if rolls.is_empty() {
        return Ok(0);
    }
    let name = state.combatants[target].name.clone();
    let mut grouped: BTreeMap<String, (i32, Vec<String>)> = BTreeMap::new();
    for (record, damage_type) in &rolls {
        let mitigated = mitigate(
            state,
            target,
            source,
            record.total,
            damage_type,
            source_tags,
        )?;
        let entry = grouped
            .entry(damage_type.clone())
            .or_insert_with(|| (0, Vec::new()));
        entry.0 += mitigated;
        entry.1.push(record.text.clone());
    }
    let mut total = 0;
    let mut parts = Vec::new();
    for (damage_type, (amount, formulas)) in grouped {
        total += amount;
        parts.push(format!("{amount} {damage_type} ({})", formulas.join(", ")));
    }
    if total <= 0 {
        log.push(format!("{name} takes no damage ({})", parts.join("; ")));
        return Ok(0);
    }

    let temp = state.combatants[target].temp_hp;
    let absorbed = temp.min(total);
    if absorbed > 0 {
        state.combatants[target].temp_hp -= absorbed;
    }
    let hp_damage = total - absorbed;
    let before = state.combatants[target].hp;
    state.combatants[target].hp = (before - hp_damage).max(0);
    log.push(format!(
        "{name} takes {total} damage ({}){}",
        parts.join("; "),
        if absorbed > 0 {
            format!("; {absorbed} absorbed by temporary HP")
        } else {
            String::new()
        }
    ));

    // Massive damage.
    let overflow = hp_damage.saturating_sub(before.max(0));
    let max_hp = state.combatants[target].max_hp;
    if state.combatants[target].hp == 0 && overflow >= max_hp {
        log.push(format!("{name} dies from massive damage"));
        state.combatants[target].dead = true;
    }

    if state.combatants[target].hp == 0 && !state.combatants[target].is_dead() {
        if state.combatants[target].is_pc() {
            apply_dying(state, target, log);
        } else {
            state.combatants[target].dead = true;
            if let Some(condition_id) = state.content.rules.dead_monster_condition.clone() {
                let mut rng = seeded_rng(0);
                let _ = apply_condition_internal(
                    state,
                    target,
                    &condition_id,
                    Some(source),
                    None,
                    Some(Duration {
                        rounds: None,
                        until: None,
                        minutes: None,
                        permanent: Some(true),
                        ends_on: Vec::new(),
                    }),
                    None,
                    Vec::new(),
                    None,
                    false,
                    &mut rng,
                    log,
                );
            }
            log.push(format!("{name} is dead"));
        }
    }

    // Damage-taken triggers and concentration.
    concentration_damage_check(state, target, total, rng, log)?;
    run_damage_taken_triggers(state, target, source, total, rng, log);
    Ok(total)
}

fn mitigate(
    state: &EncounterState,
    target: usize,
    source: usize,
    amount: i32,
    damage_type: &str,
    source_tags: &[String],
) -> Result<i32> {
    let combatant = &state.combatants[target];
    let mut multiplier = 1.0f64;
    let static_immune = combatant
        .creature
        .immunities
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    if static_immune {
        return Ok(0);
    }
    if combatant
        .creature
        .resistances
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type))
    {
        multiplier *= 0.5;
    }
    if combatant
        .creature
        .vulnerabilities
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type))
    {
        multiplier *= 2.0;
    }
    let context = PredicateContext {
        self_idx: target,
        actor: target,
        target: Some(target),
        attacker: Some(source),
        linked: None,
        attack: AttackFacts::default(),
        has_advantage: false,
        has_disadvantage: false,
        damage_dealt: 0,
        slot_level: None,
        current_attack_total: None,
        current_ac: None,
        ability: None,
        damage_type: Some(damage_type.to_string()),
        source_tags: source_tags.to_vec(),
    };
    for collected in collect_modifiers(state, target, ModifierApplies::DamageTaken, &context, None)
    {
        match collected.modifier.effect {
            ModifierEffect::Immunity => return Ok(0),
            ModifierEffect::Resistance => multiplier *= 0.5,
            ModifierEffect::Vulnerability => multiplier *= 2.0,
            _ => {}
        }
    }
    Ok(((amount as f64) * multiplier).floor() as i32)
}

fn apply_dying(state: &mut EncounterState, target: usize, log: &mut Vec<String>) {
    let name = state.combatants[target].name.clone();
    state.combatants[target].death_saves.stable = false;
    log.push(format!(
        "{name} drops to 0 HP ({}/{} death saves)",
        state.combatants[target].death_saves.successes,
        state.combatants[target].death_saves.failures
    ));
    let conditions = state
        .content
        .rules
        .dying_conditions
        .clone()
        .unwrap_or_default();
    for condition in conditions {
        let mut rng = seeded_rng(0);
        let _ = apply_condition_internal(
            state,
            target,
            &condition,
            None,
            None,
            Some(Duration {
                rounds: None,
                until: None,
                minutes: None,
                permanent: Some(true),
                ends_on: Vec::new(),
            }),
            None,
            Vec::new(),
            None,
            false,
            &mut rng,
            log,
        );
    }
}

fn run_damage_taken_triggers(
    state: &mut EncounterState,
    target: usize,
    source: usize,
    damage: i32,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) {
    let Ok(abilities) = state.combatants[target].owned_abilities(&state.content) else {
        return;
    };
    for resolved in abilities {
        for (index, trigger) in resolved.ability.triggers.iter().enumerate() {
            if trigger.on != TriggerWhen::OnDamageTaken {
                continue;
            }
            let key = format!("trigger:{}:{}:{}", target, resolved.ability.id, index);
            if !trigger_available(state, target, trigger, &key) {
                continue;
            }
            if !trigger_affordable(state, target, trigger) {
                continue;
            }
            let context = PredicateContext {
                self_idx: target,
                actor: target,
                target: Some(target),
                attacker: Some(source),
                linked: None,
                attack: AttackFacts::default(),
                has_advantage: false,
                has_disadvantage: false,
                damage_dealt: damage,
                slot_level: resolved.ability.slot_level,
                current_attack_total: None,
                current_ac: None,
                ability: None,
                damage_type: None,
                source_tags: resolved.ability.tags.clone(),
            };
            if let Some(predicate) = &trigger.when {
                if !eval_predicate(state, predicate, &context) {
                    continue;
                }
            }
            if let Some(cost) = trigger
                .cost
                .clone()
                .or_else(|| resolved.ability.cost.clone())
            {
                if !spend_cost(state, target, &cost, log).unwrap_or(false) {
                    continue;
                }
            }
            mark_trigger_used(state, target, trigger, &key);
            let effect_context = EffectContext {
                actor: target,
                target: Some(source),
                source: Some(EffectSource::from_ability(
                    &resolved.ability,
                    &resolved.params,
                )),
                attack: None,
                option: None,
                crit: false,
                damage_dealt: damage,
                damage_scale: 1.0,
                has_advantage: false,
                has_disadvantage: false,
            };
            let _ = apply_effects(state, &effect_context, &trigger.effects, rng, log);
        }
    }
}

/// CLI `damage`.
pub fn damage(
    state: &mut EncounterState,
    target_name: &str,
    expression: &str,
    damage_type: Option<&str>,
    crit: bool,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let target = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_targetable(state, target)?;
    let parsed = expression
        .parse::<DiceExpr>()
        .with_context(|| format!("invalid damage expression `{expression}`"))?;
    let record = if crit {
        parsed.roll_doubled(rng)
    } else {
        parsed.roll(rng)
    };
    let damage_type = damage_type.unwrap_or("untyped");
    // A manual damage command has no attacker; use the target as the source
    // index for context-only lookups.
    apply_damage_rolls(
        state,
        target,
        target,
        vec![(record, damage_type.to_string())],
        &[],
        rng,
        log,
    )?;
    log_outcome(state, log);
    Ok(())
}

/// CLI `heal`.
pub fn heal(
    state: &mut EncounterState,
    target_name: &str,
    expression: &str,
    from: Option<&str>,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let target = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let from_index = match from {
        Some(name) => Some(
            state
                .find_index(name)
                .map_err(|error| anyhow!(error.to_string()))?,
        ),
        None => None,
    };
    let parsed = expression
        .parse::<DiceExpr>()
        .with_context(|| format!("invalid healing expression `{expression}`"))?;
    let record = parsed.roll(rng);
    let mut amount = record.total;
    let mut terms = Vec::new();
    if let Some(source) = from_index {
        let context = PredicateContext {
            self_idx: source,
            actor: source,
            target: Some(target),
            attacker: None,
            linked: None,
            attack: AttackFacts::default(),
            has_advantage: false,
            has_disadvantage: false,
            damage_dealt: 0,
            slot_level: None,
            current_attack_total: None,
            current_ac: None,
            ability: None,
            damage_type: None,
            source_tags: Vec::new(),
        };
        for collected in
            collect_modifiers(state, source, ModifierApplies::HealingDone, &context, None)
        {
            if collected.modifier.effect == ModifierEffect::Bonus {
                if let Some(value) = &collected.modifier.value {
                    let (_, expression) =
                        evaluate_value(&score_context(&state.combatants[source]), value)?;
                    let extra = expression.roll(rng).total;
                    amount += extra;
                    terms.push(format!("+{extra}[{}]", collected.label));
                }
            }
        }
    }
    let name = state.combatants[target].name.clone();
    let before = state.combatants[target].hp;
    let max_hp = state.combatants[target].max_hp;
    state.combatants[target].hp = (before + amount).min(max_hp);
    let healed = state.combatants[target].hp - before;
    log.push(format!(
        "{name} regains {healed} HP ({} {})",
        record.text,
        terms.join(" ")
    ));
    if before == 0 && state.combatants[target].hp > 0 && !state.combatants[target].is_dead() {
        revive_from_zero(state, target, log);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Saves, checks and rolls
// ---------------------------------------------------------------------------

/// CLI `save`.
pub fn save(
    state: &mut EncounterState,
    who: &str,
    ability: &str,
    dc: i32,
    mode: RollMode,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let ability = ability_from_text(ability).ok_or_else(|| {
        anyhow!("unknown ability `{ability}`; use str, dex, con, int, wis or cha")
    })?;
    let (modifier_mode, bonuses, advantages, disadvantages, auto_fail) = save_modifiers(
        state,
        index,
        ability,
        &score_context(&state.combatants[index]),
        rng,
    )?;
    let mode = combine_modes(mode, modifier_mode);
    if auto_fail {
        log.push(format!(
            "{} automatically fails the {} save (DC {dc})",
            state.combatants[index].name,
            ability.as_str().to_uppercase()
        ));
        return Ok(());
    }
    let base = state.combatants[index].save_bonus(ability);
    let result = roll_d20(
        state,
        index,
        base,
        mode,
        bonuses,
        advantages,
        disadvantages,
        rng,
    );
    log.push(format!(
        "{} {} save: {} vs DC {dc} {}",
        state.combatants[index].name,
        ability.as_str().to_uppercase(),
        result.text(),
        if result.total >= dc { "PASS" } else { "FAIL" }
    ));
    Ok(())
}

fn combine_modes(a: RollMode, b: RollMode) -> RollMode {
    match (a, b) {
        (RollMode::Normal, other) | (other, RollMode::Normal) => other,
        (RollMode::Advantage, RollMode::Advantage) => RollMode::Advantage,
        (RollMode::Disadvantage, RollMode::Disadvantage) => RollMode::Disadvantage,
        _ => RollMode::Normal,
    }
}

/// CLI `check`.
pub fn check(
    state: &mut EncounterState,
    who: &str,
    skill_or_ability: &str,
    dc: i32,
    mode: RollMode,
    bonus: i32,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let ability = ability_from_text(skill_or_ability);
    let base = if ability.is_some()
        && state.combatants[index]
            .creature
            .skills
            .as_ref()
            .is_none_or(|skills| {
                !skills
                    .keys()
                    .any(|skill| skill.eq_ignore_ascii_case(skill_or_ability))
            }) {
        ability
            .map(|ability| state.combatants[index].creature.ability_modifier(ability))
            .unwrap_or(0)
    } else {
        state.combatants[index].skill_bonus(skill_or_ability)
    };
    let (modifier_mode, advantages, disadvantages) =
        check_modifiers(state, index, ability, skill_or_ability);
    let mode = combine_modes(mode, modifier_mode);
    let result = roll_d20(
        state,
        index,
        base + bonus,
        mode,
        Vec::new(),
        advantages,
        disadvantages,
        rng,
    );
    log.push(format!(
        "{} {} check: {} vs DC {dc} {}",
        state.combatants[index].name,
        skill_or_ability,
        result.text(),
        if result.total >= dc { "PASS" } else { "FAIL" }
    ));
    Ok(())
}

/// CLI `roll`.
pub fn roll_expression(
    expression: &str,
    mode: RollMode,
    dc: Option<i32>,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let parsed = expression
        .parse::<DiceExpr>()
        .with_context(|| format!("invalid dice expression `{expression}`"))?;
    if mode != RollMode::Normal && parsed.terms.len() == 1 && parsed.terms[0] == (1, 20) {
        let roll = D20Roll::roll(parsed.modifier, mode, rng);
        let text = match dc {
            Some(dc) => roll.vs_dc(dc),
            None => roll.text(),
        };
        log.push(text);
    } else {
        let record = parsed.roll(rng);
        let text = match dc {
            Some(dc) => format!(
                "{} vs DC {dc} {}",
                record.text,
                if record.total >= dc { "PASS" } else { "FAIL" }
            ),
            None => record.text,
        };
        log.push(text);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Movement, activation, death saves, rests
// ---------------------------------------------------------------------------

/// CLI `move`.
pub fn move_to(
    state: &mut EncounterState,
    who: &str,
    zone: &str,
    log: &mut Vec<String>,
) -> Result<()> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    if state.combatants[index].dormant {
        bail!(
            "`{}` is dormant and cannot move",
            state.combatants[index].name
        );
    }
    let Some(space) = state.space() else {
        bail!("this state has no scene");
    };
    if !space.has_zone(zone) {
        bail!("unknown zone `{zone}`");
    }
    let from = state.combatants[index].zone.clone();
    if from == zone {
        log.push(format!(
            "{} is already in {zone}",
            state.combatants[index].name
        ));
        return Ok(());
    }
    let distance = space
        .travel_distance(&from, zone)
        .ok_or_else(|| anyhow!("no path from `{from}` to `{zone}`"))?;
    if distance > state.combatants[index].speed_left {
        bail!(
            "`{}` needs {distance} ft of movement but only {} ft remain",
            state.combatants[index].name,
            state.combatants[index].speed_left
        );
    }
    state.combatants[index].speed_left -= distance;
    state.combatants[index].zone = zone.to_string();
    log.push(format!(
        "{} moves {from} -> {zone} ({distance} ft)",
        state.combatants[index].name
    ));
    Ok(())
}

/// `activate` command and effect. Matches instance name, creature id or tag.
pub fn activate(
    state: &mut EncounterState,
    query: &str,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let matches = state.find_activatable(query);
    if matches.is_empty() {
        bail!("no dormant creature matches `{query}`");
    }
    for index in matches {
        if !state.combatants[index].dormant {
            log.push(format!(
                "{} is already active",
                state.combatants[index].name
            ));
            continue;
        }
        let roll = D20Roll::roll(
            state.combatants[index].creature.init_bonus(),
            RollMode::Normal,
            rng,
        );
        state.combatants[index].dormant = false;
        state.combatants[index].initiative_total = Some(roll.total);
        log.push(format!(
            "{} activates: {}",
            state.combatants[index].name,
            roll.text()
        ));
        insert_into_initiative(state, index);
    }
    log_outcome(state, log);
    Ok(())
}

fn insert_into_initiative(state: &mut EncounterState, index: usize) {
    if state.initiative.is_empty() {
        state.initiative.push(index);
        return;
    }
    let total = state.combatants[index].initiative_total.unwrap_or(0);
    let position = state
        .initiative
        .iter()
        .position(|candidate| {
            let other = state.combatants[*candidate].initiative_total.unwrap_or(0);
            total > other
        })
        .unwrap_or(state.initiative.len());
    state.initiative.insert(position, index);
    if position <= state.turn && !state.initiative.is_empty() {
        state.turn = state.turn.saturating_add(1);
    }
    if state.turn >= state.initiative.len() {
        state.turn = state.initiative.len().saturating_sub(1);
    }
}

/// CLI `deathsave`.
pub fn death_save(
    state: &mut EncounterState,
    who: &str,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    death_save_at(state, index, rng, log)
}

fn death_save_at(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut Vec<String>,
) -> Result<()> {
    let name = state.combatants[index].name.clone();
    if !state.combatants[index].is_pc() {
        bail!("`{name}` is not a player character");
    }
    if state.combatants[index].is_dead() {
        bail!("`{name}` is already dead");
    }
    if state.combatants[index].hp > 0 {
        bail!("`{name}` is not dying");
    }
    if state.combatants[index].death_saves.stable {
        log.push(format!("{name} is stable"));
        return Ok(());
    }
    let roll = D20Roll::roll(0, RollMode::Normal, rng);
    log.push(format!("{name} death save: {}", roll.text()));
    if roll.nat20 {
        state.combatants[index].hp = 1;
        state.combatants[index].death_saves = DeathSaves::default();
        revive_from_zero(state, index, log);
        log.push(format!("{name} regains 1 HP"));
        return Ok(());
    }
    if roll.nat1 {
        state.combatants[index].death_saves.failures = state.combatants[index]
            .death_saves
            .failures
            .saturating_add(2);
        log.push(format!("{name} critical failure (2 death save failures)"));
    } else if roll.total >= 10 {
        state.combatants[index].death_saves.successes = state.combatants[index]
            .death_saves
            .successes
            .saturating_add(1);
        log.push(format!("{name} succeeds on the death save"));
    } else {
        state.combatants[index].death_saves.failures = state.combatants[index]
            .death_saves
            .failures
            .saturating_add(1);
        log.push(format!("{name} fails the death save"));
    }
    if state.combatants[index].death_saves.failures >= 3 {
        state.combatants[index].death_saves.dead = true;
        state.combatants[index].dead = true;
        log.push(format!("{name} dies"));
    } else if state.combatants[index].death_saves.successes >= 3 {
        state.combatants[index].death_saves.stable = true;
        state.combatants[index].death_saves.successes = 0;
        state.combatants[index].death_saves.failures = 0;
        log.push(format!("{name} is stable"));
    }
    Ok(())
}

/// CLI `rest short|long`.
pub fn rest(
    state: &mut EncounterState,
    kind: RestKind,
    who: Option<&str>,
    log: &mut Vec<String>,
) -> Result<()> {
    let targets: Vec<usize> = match who {
        Some(name) if name.eq_ignore_ascii_case("all") => (0..state.combatants.len()).collect(),
        Some(name) => vec![state
            .find_index(name)
            .map_err(|error| anyhow!(error.to_string()))?],
        None => (0..state.combatants.len()).collect(),
    };
    for index in targets {
        let name = state.combatants[index].name.clone();
        match kind {
            RestKind::Short => {
                for resource in state.combatants[index].resources.values_mut() {
                    if resource.recharge == Recharge::Short {
                        resource.current = resource.max;
                    } else if let Some(regain) = resource.short_rest_regain {
                        resource.restore(regain);
                    }
                }
                log.push(format!("{name} takes a short rest"));
            }
            RestKind::Long => {
                for resource in state.combatants[index].resources.values_mut() {
                    resource.current = resource.max;
                }
                if state.combatants[index].death_saves.dead {
                    log.push(format!(
                        "{name} is dead and cannot benefit from a long rest"
                    ));
                    continue;
                }
                state.combatants[index].hp = state.combatants[index].max_hp;
                state.combatants[index].temp_hp = 0;
                state.combatants[index].dead = false;
                state.combatants[index].death_saves = DeathSaves::default();
                let dying = state
                    .content
                    .rules
                    .dying_conditions
                    .clone()
                    .unwrap_or_default();
                state.combatants[index].conditions.retain(|condition| {
                    !dying
                        .iter()
                        .any(|id| id.eq_ignore_ascii_case(&condition.condition))
                });
                log.push(format!("{name} takes a long rest"));
            }
        }
    }
    log_outcome(state, log);
    Ok(())
}

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

/// Armour class including active condition modifiers (but no cover bonus).
pub fn armor_class(state: &EncounterState, index: usize) -> i32 {
    effective_ac(state, index, None, &AttackFacts::default())
        .map(|(ac, _)| ac)
        .unwrap_or_else(|_| state.combatants[index].creature.ac())
}

pub fn status_lines(state: &EncounterState) -> Vec<String> {
    let mut lines = Vec::new();
    match state.current() {
        Some(combatant) => lines.push(format!("Round {} - {}'s turn", state.round, combatant.name)),
        None => lines.push(format!("Round {} - initiative not rolled", state.round)),
    }
    for index in status_order(state) {
        lines.push(combatant_status_line(
            state,
            index,
            state.current_index() == Some(index),
        ));
    }
    lines
}

pub fn status_order(state: &EncounterState) -> Vec<usize> {
    let mut order = Vec::new();
    for &index in &state.initiative {
        if index < state.combatants.len() && !order.contains(&index) {
            order.push(index);
        }
    }
    for index in 0..state.combatants.len() {
        if !order.contains(&index) {
            order.push(index);
        }
    }
    order
}

fn combatant_status_line(state: &EncounterState, index: usize, current: bool) -> String {
    let combatant = &state.combatants[index];
    let marker = if current { "> " } else { "  " };
    let initiative = combatant
        .initiative_total
        .map_or_else(|| "-".to_string(), |total| total.to_string());
    let hp = if combatant.temp_hp > 0 {
        format!(
            "{}/{} (+{})",
            combatant.hp, combatant.max_hp, combatant.temp_hp
        )
    } else {
        format!("{}/{}", combatant.hp, combatant.max_hp)
    };
    let conditions = if combatant.conditions.is_empty() {
        "none".to_string()
    } else {
        combatant
            .conditions
            .iter()
            .map(|condition| match &condition.duration {
                Some(duration) => format!("{}({})", condition.condition, duration.describe()),
                None => condition.condition.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let resources = if combatant.resources.is_empty() {
        "none".to_string()
    } else {
        combatant
            .resources
            .iter()
            .map(|(name, resource)| format!("{name} {}/{}", resource.current, resource.max))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let saves = if !combatant.is_pc() {
        "-".to_string()
    } else if combatant.death_saves.dead {
        "dead".to_string()
    } else if combatant.death_saves.stable {
        "stable".to_string()
    } else {
        format!(
            "{}S/{}F",
            combatant.death_saves.successes, combatant.death_saves.failures
        )
    };
    let dormant = if combatant.dormant { " DORMANT" } else { "" };
    let concentration = if state.concentration.contains_key(&combatant.name) {
        " CONCENTRATING"
    } else {
        ""
    };
    format!(
        "{marker}{initiative} {} {} HP {hp} AC {} {}{dormant}{concentration} | conditions: {conditions} | resources: {resources} | saves: {saves}",
        combatant.name,
        combatant.side.as_str(),
        armor_class(state, index),
        combatant.zone
    )
}
