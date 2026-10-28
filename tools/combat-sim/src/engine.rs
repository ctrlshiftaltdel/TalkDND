//! Generic, data-driven 5e combat engine.
//!
//! Rules that every 5e game shares live here. Campaign-specific behaviour is
//! read from the content snapshot stored in [`EncounterState`]: conditions,
//! masteries, abilities, creature attacks and the core-rule wiring in
//! `rules`. No content ids or display names appear in this module.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};

use crate::content::expr::ExprContext;
use crate::content::schema::{
    Ability, AbilityCost, AbilityEntry, AbilityName, ActionResolution, Activation, Attack,
    AttackEffect, AttackKind, AttackProperty, BasicAction, CheckEffect, CheckWho, ConditionFlag,
    Cover, CritMode, DamageEffect, DamagePart, DamageType, Direction, Duration, DurationUntil,
    Effect, EndsOn, ExprValue, ForcedMoveEffect, GrappleRule, Light, Modifier, ModifierApplies,
    ModifierEffect, ModifierFilter, OncePer, Predicate, Recharge, SaveEffect, SaveEndsAt,
    SceneMode, ShoveRule, Size, TargetRef, TargetSide, TargetType, ToggleTriggerEffect, Trigger,
    TriggerWhen, TwoWeaponRule, UnarmedStrikeRule,
};
use crate::content::{ContentRegistry, ResolvedAbility};
use crate::dice::{seeded_rng, D20Roll, DiceExpr, DiceRng, RollMode, RollRecord};
use crate::space::{
    AreaShape, AreaSpecV1, BlockerKind, CellPos, CoverResult, Explain, FallAbility, FallCause,
    FallMap, FallResolution, FallResolver, FallRuleKind, FlightCapability, FlightCheck,
    FlightConditions, FlightRules, FlightState, FlightSubject, Footprint, GridMap, HeightProfile,
    LandingCondition, LandingOutcome, LineOfSight, MovementCell, MovementMap, MovementPlanner,
    PlanOutcome, Position, Space, SpatialBlocker, SpatialBody, SpatialPose, SpatialProfile,
    TraversalCreature, OPEN_ZONE,
};
use crate::state::{
    ability_from_text, active_duration, ActiveCondition, ActiveDuration, ActiveSaveEnds,
    CombatOutcome, CombatantState, DeathSaves, EncounterState, ReadyResponse, ReadyState,
    ReadyTrigger,
};

/// Destination for engine log lines.
///
/// A disabled sink avoids formatting and allocation entirely, which keeps the
/// Monte Carlo hot path free of log-string work. `Vec<String>` keeps the
/// step-by-step command output unchanged.
pub trait LogSink {
    fn enabled(&self) -> bool;
    fn emit(&mut self, line: &str);

    fn log(&mut self, args: std::fmt::Arguments<'_>) {
        if self.enabled() {
            self.emit(&format!("{args}"));
        }
    }
}

impl LogSink for Vec<String> {
    fn enabled(&self) -> bool {
        true
    }

    fn emit(&mut self, line: &str) {
        self.push(line.to_string());
    }
}

/// Log sink that drops every line without formatting it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullSink;

impl LogSink for NullSink {
    fn enabled(&self) -> bool {
        false
    }

    fn emit(&mut self, _line: &str) {}
}

/// Attack command options.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttackOptions {
    pub advantage: bool,
    pub disadvantage: bool,
    pub multi: bool,
    pub no_reactions: bool,
    /// Knock the target out instead of killing it if this melee attack drops it to 0 HP.
    pub nonlethal: bool,
}

/// Unarmed Strike options (the attack itself, not the 2024 grapple/shove
/// replacements).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnarmedOptions {
    pub advantage: bool,
    pub disadvantage: bool,
    pub no_reactions: bool,
    pub nonlethal: bool,
    /// Resolve exactly one Unarmed Strike and leave the remaining Attack action
    /// slots open.
    pub one: bool,
}

/// Ready action setup. The response is stored as structured state and later
/// resolved through the normal attack/ability/move paths.
#[derive(Debug, Clone)]
pub struct ReadyOptions {
    pub trigger: ReadyTrigger,
    pub description: String,
    pub response: ReadyResponse,
}

/// Result of an explicit or automatic trigger attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyOutcome {
    /// The response resolved and the reaction was spent.
    Triggered,
    /// The ready is still held; the reason is logged and no reaction was spent.
    Refused(String),
    /// The ready was past its next-turn expiry or the holder was no longer
    /// able to use it, so it was cleared without spending a reaction.
    Expired,
}

/// Which parts of an ability's cost the caller has already settled. Normal
/// commands use the default (spend the action, pay the cost, begin
/// concentration). A readied ability is prepaid at Ready setup, so its release
/// uses `prepaid()` and only resolves the effects.
#[derive(Debug, Clone, Copy, Default)]
struct AbilitySettlement {
    /// The action was already spent by the caller (AI, nested effect, ready).
    action_spent: bool,
    /// The ability cost/resource was already paid (ready setup).
    cost_paid: bool,
    /// Concentration was already started (ready setup).
    concentration_started: bool,
}

impl AbilitySettlement {
    fn prepaid() -> Self {
        Self {
            action_spent: true,
            cost_paid: true,
            concentration_started: true,
        }
    }
}

/// Which outcome a successful shove chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShoveMode {
    #[default]
    KnockDown,
    Away,
}

/// Grapple options. `defense` is the target's chosen defense: an Athletics or
/// Acrobatics skill in 2014, or `str` / `dex` in 2024. `None` lets the target
/// use its best legal defense and is logged.
#[derive(Debug, Clone, Default)]
pub struct GrappleOptions {
    pub defense: Option<String>,
}

/// Shove options. `defense` follows [`GrappleOptions`].
#[derive(Debug, Clone, Default)]
pub struct ShoveOptions {
    pub defense: Option<String>,
    pub mode: ShoveMode,
}

/// Prefix for per-turn Attack action slots stored in `CombatantState::once_used`.
/// The `turn:` prefix is cleared by the existing turn-start reset.
const ATTACK_SLOT_PREFIX: &str = "turn:attack-action:slot:";

/// Ability/use command options.
#[derive(Debug, Clone, Default)]
pub struct UseOptions {
    pub targets: Vec<String>,
    pub option: Option<usize>,
    pub slot_level: Option<i32>,
    pub advantage: bool,
    pub disadvantage: bool,
    pub no_reactions: bool,
    /// Knock out instead of kill on a melee attack that drops the target to 0 HP.
    pub nonlethal: bool,
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
    /// Exhaustion level (1-6). Ignored for other conditions.
    pub level: Option<u32>,
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
    let empty = BTreeMap::new();
    let scores = combatant.creature.abilities.as_ref().unwrap_or(&empty);
    let mut context = ExprContext::new()
        .with_ability_scores(scores)
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
    if combatant.conditions.is_empty() {
        return false;
    }
    let mut stack: Vec<&str> = combatant
        .conditions
        .iter()
        .map(|active| active.condition.as_str())
        .collect();
    let mut seen: Vec<&str> = Vec::with_capacity(stack.len());
    while let Some(id) = stack.pop() {
        if seen.iter().any(|known| known.eq_ignore_ascii_case(id)) {
            continue;
        }
        seen.push(id);
        if let Some(def) = state.content.conditions.get(id) {
            if def.flags.contains(&flag) {
                return true;
            }
            stack.extend(def.implies.iter().map(String::as_str));
        }
    }
    false
}

fn is_incapacitated(state: &EncounterState, index: usize) -> bool {
    has_flag(state, index, ConditionFlag::Incapacitated)
}

/// The condition id used for the additive exhaustion level.
pub fn exhaustion_condition_id(state: &EncounterState) -> &str {
    state
        .content
        .rules
        .exhaustion
        .as_ref()
        .map(|rules| rules.condition_id())
        .unwrap_or("exhaustion")
}

fn exact_index(state: &EncounterState, name: &str) -> Option<usize> {
    state
        .combatants
        .iter()
        .position(|combatant| combatant.name.eq_ignore_ascii_case(name))
}

/// Effective HP maximum after 2014 level-4 exhaustion.
pub fn effective_max_hp(state: &EncounterState, index: usize) -> i32 {
    let base = state.combatants[index].max_hp.max(1);
    let halved_from = state
        .content
        .rules
        .exhaustion
        .as_ref()
        .and_then(|rules| rules.hp_max_halved_from);
    if halved_from.is_some_and(|level| state.combatants[index].exhaustion_level >= level) {
        (base / 2).max(1)
    } else {
        base
    }
}

/// HP at or below half of the effective maximum. This is a derived status:
/// it is always current after damage, healing or temporary-HP changes without
/// writing an extra state flag.
pub fn is_bloodied(state: &EncounterState, index: usize) -> bool {
    let max = effective_max_hp(state, index);
    state.combatants[index].hp <= max / 2
}

fn exhaustion_d20_penalty(state: &EncounterState, index: usize) -> i32 {
    let level = state.combatants[index].exhaustion_level;
    if level == 0 {
        return 0;
    }
    let per_level = state
        .content
        .rules
        .exhaustion
        .as_ref()
        .and_then(|rules| rules.d20_penalty_per_level)
        .unwrap_or(0)
        .max(0);
    -(per_level.saturating_mul(level as i32))
}

fn exhaustion_attack_save_disadvantage(state: &EncounterState, index: usize) -> bool {
    let level = state.combatants[index].exhaustion_level;
    state
        .content
        .rules
        .exhaustion
        .as_ref()
        .and_then(|rules| rules.attack_save_disadvantage_from)
        .is_some_and(|from| level >= from)
}

fn condition_behavior<'a>(
    state: &'a EncounterState,
    condition: &str,
) -> Option<&'a crate::content::schema::ConditionBehavior> {
    let behaviors = state.content.rules.condition_behaviors.as_ref()?;
    behaviors.get(condition).or_else(|| {
        behaviors
            .iter()
            .find(|(id, _)| id.eq_ignore_ascii_case(condition))
            .map(|(_, behavior)| behavior)
    })
}

fn combatant_is_hidden(state: &EncounterState, index: usize) -> bool {
    state.combatants[index].conditions.iter().any(|active| {
        condition_behavior(state, &active.condition).is_some_and(|behavior| behavior.hidden)
    })
}

/// Whether `source_name` is alive, not hidden and in line of sight of the
/// holder. Uses the same LOS kernel as the rest of the engine.
fn source_is_visible(state: &EncounterState, holder: usize, source_name: &str) -> bool {
    let Some(source) = exact_index(state, source_name) else {
        return false;
    };
    if state.combatants[source].is_dead() || state.combatants[source].hp <= 0 {
        return false;
    }
    if combatant_is_hidden(state, source) {
        return false;
    }
    if source == holder {
        return true;
    }
    if let Some(sight) = spatial_line_of_sight(state, holder, source) {
        return sight.clear;
    }
    let Some(space) = state.space() else {
        return true;
    };
    let from = state.combatants[holder].position_ref();
    let to = state.combatants[source].position_ref();
    space.cover(from.as_ref(), to.as_ref()) != Cover::Total
}

fn frightened_source_names(state: &EncounterState, holder: usize) -> Vec<String> {
    state.combatants[holder]
        .conditions
        .iter()
        .filter_map(|active| {
            if !condition_behavior(state, &active.condition)
                .is_some_and(|behavior| behavior.cannot_move_closer_to_source)
            {
                return None;
            }
            let name = active
                .source
                .as_deref()
                .or(active.linked.as_deref())?
                .to_string();
            source_is_visible(state, holder, &name).then_some(name)
        })
        .collect()
}

fn distance_from_source(state: &EncounterState, source: usize, position: &Position) -> Option<i32> {
    if spatial_query_enabled(state) {
        return spatial_distance_to_position(state, source, position);
    }
    let space = state.space()?;
    let source_position = state.combatants[source].position_ref();
    space.distance(source_position.as_ref(), position)
}

fn ensure_frightened_move(
    state: &EncounterState,
    holder: usize,
    from: &Position,
    to: &Position,
) -> Result<()> {
    for name in frightened_source_names(state, holder) {
        let Some(source) = exact_index(state, &name) else {
            continue;
        };
        let Some(from_ft) = distance_from_source(state, source, from) else {
            continue;
        };
        let Some(to_ft) = distance_from_source(state, source, to) else {
            continue;
        };
        if to_ft < from_ft {
            bail!(
                "`{}` can't willingly move closer to `{}` (source of fear)",
                state.combatants[holder].name,
                name
            );
        }
    }
    Ok(())
}

/// The charmer recorded by a `cannot_target_source` condition, if it is still
/// alive. Supports both the structured `source` and the older `linked` field.
fn charmed_source_name(state: &EncounterState, holder: usize) -> Option<String> {
    state.combatants[holder]
        .conditions
        .iter()
        .filter_map(|active| {
            if !condition_behavior(state, &active.condition)
                .is_some_and(|behavior| behavior.cannot_target_source)
            {
                return None;
            }
            let name = active
                .source
                .as_deref()
                .or(active.linked.as_deref())?
                .to_string();
            let source = exact_index(state, &name)?;
            (!state.combatants[source].is_dead()).then_some(name)
        })
        .next()
}

fn ensure_not_charmed_target(
    state: &EncounterState,
    holder: usize,
    target: usize,
    damaging: bool,
) -> Result<()> {
    if !damaging {
        return Ok(());
    }
    let Some(source) = charmed_source_name(state, holder) else {
        return Ok(());
    };
    if state.combatants[target].name.eq_ignore_ascii_case(&source) {
        bail!(
            "`{}` is charmed by `{}` and cannot attack or target it with damaging effects",
            state.combatants[holder].name,
            source
        );
    }
    Ok(())
}

fn effects_have_damage(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| match effect {
        Effect::Damage(_) => true,
        Effect::Attack(attack) => {
            !attack.damage.is_empty()
                || effects_have_damage(&attack.on_hit)
                || effects_have_damage(&attack.on_miss)
        }
        Effect::Save(save) => {
            effects_have_damage(&save.on_fail) || effects_have_damage(&save.on_success)
        }
        Effect::Choice(branches) => branches.iter().any(|branch| effects_have_damage(branch)),
        _ => false,
    })
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

// ---------------------------------------------------------------------------
// P5 spatial query bridge (TASK-0090)
// ---------------------------------------------------------------------------
//
// These helpers are the single engine-facing entry to the P2/P3/P4 kernel.
// They are only consulted when the save explicitly carries P1 spatial data;
// legacy saves keep the old `Space` trait path (contract §11, AC-5).

/// Whether this save opted into the additive spatial state at all.
fn spatial_query_enabled(state: &EncounterState) -> bool {
    spatial_flight_enabled(state)
}

fn spatial_body_for(state: &EncounterState, index: usize) -> Option<SpatialBody> {
    let combatant = state.combatants.get(index)?;
    let mut pose = combatant.spatial_pose()?;
    // A pre-P1 combatant has no explicit pose. `spatial_pose` derives z=0;
    // add the scene floor so an elevated cell is not treated as ground level.
    if combatant.pose.is_none() {
        if let Some(cell) = state
            .scene
            .as_deref()
            .and_then(GridMap::parse)
            .and_then(|grid| {
                grid.cell_at(pose.cell.x, pose.cell.y)
                    .map(|cell| cell.elevation.unwrap_or(0))
            })
        {
            pose.cell.z = cell.saturating_add(combatant.vertical_state().altitude_ft);
        }
    }
    let height = HeightProfile::default()
        .resolve(combatant.creature.size(), None)
        .ok()?
        .height_ft;
    Some(
        SpatialBody::new(combatant.name.clone(), pose, combatant.creature.size())
            .with_height(height),
    )
}

fn spatial_bodies(state: &EncounterState) -> Vec<(usize, SpatialBody)> {
    state
        .combatants
        .iter()
        .enumerate()
        .filter(|(_, combatant)| !combatant.dormant && !combatant.is_dead())
        .filter_map(|(index, _)| spatial_body_for(state, index).map(|body| (index, body)))
        .collect()
}

/// Blocker volumes for line of sight and cover. Walls use the scene ceiling as
/// their default height; `GridCell.height_ft` is a low-obstacle height above
/// the cell floor. Terrain volumes follow the same override rule as the fall
/// map: a non-empty `SpatialState` list replaces the scene list.
fn spatial_blockers(state: &EncounterState, profile: &SpatialProfile) -> Vec<SpatialBlocker> {
    let mut blockers = Vec::new();
    if let Some(scene) = state.scene.as_deref() {
        if let Some(grid) = GridMap::parse(scene) {
            let wall_height = scene.cell_height_ft.unwrap_or(10).max(1);
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let Some(cell) = grid.cell_at(x, y) else {
                        continue;
                    };
                    let floor = cell.elevation.unwrap_or(0);
                    let cell_pos = CellPos { x, y, z: floor };
                    if cell.wall {
                        let height = cell.height_ft.unwrap_or(wall_height);
                        if let Ok(blocker) = SpatialBlocker::wall(
                            format!("wall:{x},{y}"),
                            cell_pos,
                            Footprint::ONE,
                            height,
                        ) {
                            blockers.push(blocker);
                        }
                    } else if cell.height_ft.is_some() || cell.cover != Cover::None {
                        let height = cell.height_ft.unwrap_or(3);
                        let kind = if height >= 5 {
                            BlockerKind::Terrain
                        } else {
                            BlockerKind::LowObstacle
                        };
                        if let Ok(blocker) = SpatialBlocker::new(
                            format!("cell:{x},{y}"),
                            cell_pos,
                            Footprint::ONE,
                            height,
                            kind,
                        ) {
                            let cover = if cell.cover == Cover::None {
                                Cover::Half
                            } else {
                                cell.cover
                            };
                            blockers.push(blocker.with_cover(cover).with_sight(height >= 5));
                        }
                    }
                }
            }
        }
        let terrain = state
            .spatial
            .as_ref()
            .filter(|spatial| !spatial.terrain_volumes.is_empty())
            .map_or(scene.terrain_volumes.as_slice(), |spatial| {
                spatial.terrain_volumes.as_slice()
            });
        for volume in terrain {
            if let Ok(blocker) = SpatialBlocker::terrain(volume, Cover::Half) {
                blockers.push(blocker);
            }
        }
    } else if let Some(spatial) = state.spatial.as_ref() {
        for volume in &spatial.terrain_volumes {
            if let Ok(blocker) = SpatialBlocker::terrain(volume, Cover::Half) {
                blockers.push(blocker);
            }
        }
    }
    for (_, body) in spatial_bodies(state) {
        if let Ok(blocker) = SpatialBlocker::creature(&body, false) {
            blockers.push(blocker.with_cover(profile.creature_cover));
        }
    }
    blockers.sort_by(|left, right| left.id.cmp(&right.id));
    blockers
}

fn spatial_cover_result(
    state: &EncounterState,
    attacker: usize,
    target: usize,
) -> Option<CoverResult> {
    let from = spatial_body_for(state, attacker)?;
    let to = spatial_body_for(state, target)?;
    let profile = state.effective_spatial_profile();
    let blockers = spatial_blockers(state, &profile);
    Some(crate::space::cover_between(&from, &to, &blockers, &profile))
}

fn spatial_line_of_sight(
    state: &EncounterState,
    attacker: usize,
    target: usize,
) -> Option<LineOfSight> {
    let from = spatial_body_for(state, attacker)?;
    let to = spatial_body_for(state, target)?;
    let profile = state.effective_spatial_profile();
    let blockers = spatial_blockers(state, &profile);
    Some(crate::space::line_of_sight(&from, &to, &blockers, &profile))
}

fn ensure_spatial_line_of_sight(
    state: &EncounterState,
    attacker: usize,
    target: usize,
) -> Result<()> {
    let Some(sight) = spatial_line_of_sight(state, attacker, target) else {
        return Ok(());
    };
    if !sight.clear {
        bail!(
            "`{}` has no line of sight to `{}` (blocked by {})",
            state.combatants[attacker].name,
            state.combatants[target].name,
            sight.blocked_by.join(", ")
        );
    }
    Ok(())
}

fn spatial_body_distance(state: &EncounterState, from: usize, to: usize) -> Option<i32> {
    let from = spatial_body_for(state, from)?;
    let to = spatial_body_for(state, to)?;
    let profile = state.effective_spatial_profile();
    Some(crate::space::body_distance(&from, &to, &profile).feet)
}

fn spatial_can_reach(
    state: &EncounterState,
    from: usize,
    to: usize,
    reach_ft: i32,
) -> Option<bool> {
    let from = spatial_body_for(state, from)?;
    let to = spatial_body_for(state, to)?;
    let profile = state.effective_spatial_profile();
    Some(crate::space::can_reach_bodies(&from, &to, reach_ft, &profile).reachable)
}

/// Pose for an area centre. A named creature keeps its full pose (altitude and
/// facing); an explicit `x,y` centre is always the cell floor. This prevents a
/// flyer standing over a cell from lifting a ground-targeted area to its own
/// altitude.
fn spatial_pose_for_position(
    state: &EncounterState,
    position: &Position,
    creature: Option<usize>,
) -> Option<SpatialPose> {
    if let Some(index) = creature {
        return spatial_body_for(state, index).map(|body| body.pose);
    }
    match position {
        Position::Cell { x, y } => {
            let floor = state
                .scene
                .as_deref()
                .and_then(GridMap::parse)
                .and_then(|grid| grid.cell_at(*x, *y).map(|cell| cell.elevation.unwrap_or(0)))
                .unwrap_or(0);
            Some(SpatialPose {
                cell: CellPos {
                    x: *x,
                    y: *y,
                    z: floor,
                },
                footprint: Footprint::ONE,
                facing: None,
            })
        }
        Position::Zone { .. } => None,
    }
}

fn spatial_distance_to_position(state: &EncounterState, from: usize, to: &Position) -> Option<i32> {
    let from = spatial_body_for(state, from)?;
    let to = SpatialBody::new(
        "target-position",
        spatial_pose_for_position(state, to, None)?,
        Size::Medium,
    );
    let profile = state.effective_spatial_profile();
    Some(crate::space::body_distance(&from, &to, &profile).feet)
}

fn effective_speed(state: &EncounterState, index: usize) -> i32 {
    let base = state.combatants[index].base_speed();
    effective_speed_from(state, index, base)
}

/// Effective Fly Speed after `Speed` condition flags and modifiers. A Speed 0
/// or halved Speed applies to special speeds as well.
fn effective_fly_speed(state: &EncounterState, index: usize) -> i32 {
    let base = state.combatants[index].creature.speed().fly.max(0);
    effective_speed_from(state, index, base)
}

fn effective_speed_from(state: &EncounterState, index: usize, base: i32) -> i32 {
    let level = state.combatants[index].exhaustion_level;
    let exhaustion = state.content.rules.exhaustion.as_ref();
    if has_flag(state, index, ConditionFlag::SpeedZero)
        || exhaustion
            .and_then(|rules| rules.speed_zero_from)
            .is_some_and(|from| level >= from)
    {
        return 0;
    }
    let halved = has_flag(state, index, ConditionFlag::SpeedHalved)
        || exhaustion
            .and_then(|rules| rules.speed_halved_from)
            .is_some_and(|from| level >= from);
    let mut speed = base;
    if has_modifier_sources(state, index) {
        let modifiers = collect_modifiers(
            state,
            index,
            ModifierApplies::Speed,
            &PredicateContext::for_owner(index),
            None,
        );
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
    }
    if halved {
        speed /= 2;
    }
    if let Some(penalty) = exhaustion.and_then(|rules| rules.speed_penalty_per_level) {
        speed -= penalty.max(0).saturating_mul(level as i32);
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
    source: Option<String>,
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
            source: None,
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
        Predicate::SelfHpBelowPct(percent) => hp_below_pct(state, context.self_idx, *percent),
        Predicate::TargetHpBelowPct(percent) => context
            .target
            .is_some_and(|target| hp_below_pct(state, target, *percent)),
        Predicate::Bloodied(flag) => is_bloodied(state, context.self_idx) == flag.get(),
        Predicate::TargetBloodied(flag) => context
            .target
            .is_some_and(|target| is_bloodied(state, target) == flag.get()),
        Predicate::SourceInLineOfSight(flag) => {
            let visible = context
                .source
                .as_deref()
                .is_none_or(|source| source_is_visible(state, context.self_idx, source));
            visible == flag.get()
        }
        Predicate::ExhaustionLevelMin(minimum) => {
            state.combatants[context.self_idx].exhaustion_level >= *minimum
        }
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

fn hp_below_pct(state: &EncounterState, index: usize, percent: i32) -> bool {
    let max = effective_max_hp(state, index);
    if max <= 0 {
        return state.combatants[index].hp <= 0;
    }
    state.combatants[index].hp.saturating_mul(100) <= max.saturating_mul(percent)
}

fn distance_between(state: &EncounterState, a: Option<usize>, b: Option<usize>) -> Option<i32> {
    let (a, b) = (a?, b?);
    let space = state.space()?;
    let from = state.combatants[a].position_ref();
    let to = state.combatants[b].position_ref();
    space.distance(from.as_ref(), to.as_ref())
}

fn ally_within_5(state: &EncounterState, actor: usize, target: Option<usize>) -> bool {
    let Some(target) = target else {
        return false;
    };
    let Some(space) = state.space() else {
        return false;
    };
    let target_pos = state.combatants[target].position_ref();
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
                && {
                    let position = combatant.position_ref();
                    space
                        .distance(position.as_ref(), target_pos.as_ref())
                        .is_some_and(|distance| distance <= 5)
                }
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

fn modifier_once_available(
    state: &EncounterState,
    owner: usize,
    key: &str,
    once_per: Option<OncePer>,
) -> bool {
    match once_per {
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
        scoped.source = instance.source.clone();
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
                let key = format!(
                    "condition:{}:{}:mod:{modifier_index}",
                    instance.condition, instance_index
                );
                if !modifier_once_available(state, owner, &key, modifier.once_per) {
                    continue;
                }
                out.push(CollectedModifier {
                    modifier: modifier.clone(),
                    label: def.display_name().to_string(),
                    key,
                    origin_condition: Some(instance.condition.clone()),
                });
            }
        }
    }

    // Scan passive abilities without resolving (and cloning) every owned
    // action. Most creatures have no passive modifiers, so this avoids a large
    // amount of allocation on the attack hot path.
    let actions = combatant.creature.actions.as_deref().unwrap_or(&[]);
    let features = combatant.creature.features.as_deref().unwrap_or(&[]);
    for entry in actions.iter().chain(features.iter()) {
        let ability: &Ability = match entry {
            AbilityEntry::Inline(ability) => ability,
            AbilityEntry::Ref(reference) => {
                let Some(ability) = state.content.abilities.get(&reference.reference) else {
                    continue;
                };
                ability
            }
        };
        if ability.activation != Activation::Passive || ability.modifiers.is_empty() {
            continue;
        }
        for (modifier_index, modifier) in ability.modifiers.iter().enumerate() {
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
            let key = format!("ability:{}:mod:{modifier_index}", ability.id);
            if !modifier_once_available(state, owner, &key, modifier.once_per) {
                continue;
            }
            out.push(CollectedModifier {
                modifier: modifier.clone(),
                label: ability.display_name().to_string(),
                key,
                origin_condition: None,
            });
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
            let key = format!("source:{}:mod:{modifier_index}", source.id);
            if !modifier_once_available(state, owner, &key, modifier.once_per) {
                continue;
            }
            out.push(CollectedModifier {
                modifier: modifier.clone(),
                label: source.name.clone(),
                key,
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
    log: &mut dyn LogSink,
) {
    if let Some(condition) = &collected.origin_condition {
        let name = state.combatants[owner].name.clone();
        let before = state.combatants[owner].conditions.len();
        state.combatants[owner]
            .conditions
            .retain(|active| !active.condition.eq_ignore_ascii_case(condition));
        if state.combatants[owner].conditions.len() != before {
            log.log(format_args!("{name} is no longer {condition}"));
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
    log: &mut dyn LogSink,
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

fn flat_bonus_term(value: i32, label: impl Into<String>) -> BonusTerm {
    BonusTerm {
        label: label.into(),
        record: RollRecord {
            formula: value.to_string(),
            dice: Vec::new(),
            modifier: value,
            total: value,
            text: value.to_string(),
        },
    }
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
        source: None,
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
    log: &mut dyn LogSink,
) -> Result<()> {
    init_with_modes(state, overrides, &[], rng, log)
}

/// [`init`] with per-combatant roll modes. 2024 PHB: a Surprised creature
/// rolls initiative with Disadvantage; Invisible creatures roll with Advantage.
pub fn init_with_modes(
    state: &mut EncounterState,
    overrides: &[(usize, i32)],
    modes: &[(usize, RollMode)],
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
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
        let has_advantage = modes
            .iter()
            .any(|(target, mode)| *target == index && *mode == RollMode::Advantage);
        let has_disadvantage = modes
            .iter()
            .any(|(target, mode)| *target == index && *mode == RollMode::Disadvantage);
        let mode = RollMode::from_flags(has_advantage, has_disadvantage);
        let roll = D20Roll::roll(combatant.creature.init_bonus(), mode, rng);
        combatant.initiative_total = Some(roll.total);
        log.log(format_args!(
            "Initiative {}: {}",
            combatant.name,
            roll.text()
        ));
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
            log.log(format_args!(
                "Initiative {}: {total} (set)",
                state.combatants[index].name
            ));
        }
    }

    let first = state.initiative[0];
    let _ = start_turn(state, first, rng, log);
    if let Some(combatant) = state.combatants.get(first) {
        log.log(format_args!(
            "Round {} - {}'s turn",
            state.round, combatant.name
        ));
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
    log: &mut dyn LogSink,
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
    loop {
        let mut found = None;
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
                found = Some(candidate);
                break;
            }
        }
        let Some(next_index) = found else {
            bail!("no eligible combatant remains in the initiative order");
        };
        state.turn = position;
        if !wrapped && state.round == 0 {
            state.round = 1;
        }
        if !start_turn(state, next_index, rng, log) {
            // The automatic death save at the start of the turn killed this
            // combatant; keep advancing without running their turn.
            continue;
        }
        let name = state.combatants[next_index].name.clone();
        log.log(format_args!("Round {} - {name}'s turn", state.round));
        log_outcome(state, log);
        return Ok(());
    }
}

fn reset_round_state(state: &mut EncounterState, rng: &mut dyn DiceRng, log: &mut dyn LogSink) {
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
    log: &mut dyn LogSink,
) -> bool {
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
    if state.combatants[index].is_dead() {
        clear_ready_if_any(state, index, "dead", log);
        return false;
    }
    if state.combatants[index].dormant {
        clear_ready_if_any(state, index, "no longer active", log);
        return false;
    }
    expire_ready(state, index, log);
    expire_start_phases(state, index, log);
    expire_subject_conditions(state, index, UntilPhase::Start, log);
    recharge_resources(state, index, RechargePhase::Turn, rng, log);
    if state.combatants[index].is_dying() {
        let name = state.combatants[index].name.clone();
        log.log(format_args!(
            "{name} is dying and must make a death saving throw"
        ));
        let _ = death_save_at(state, index, rng, log);
        if state.combatants[index].is_dead() {
            clear_ready_if_any(state, index, "dead", log);
            return false;
        }
    }
    run_ready_triggers(state, ReadyTrigger::TurnStart, index, rng, log);
    if state.combatants[index].is_dead() {
        return false;
    }
    run_owned_triggers(state, index, TriggerWhen::StartOfTurn, None, rng, log);
    true
}

fn end_turn(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    run_owned_triggers(state, index, TriggerWhen::EndOfTurn, None, rng, log);
    expire_conditions(state, index, rng, log)?;
    expire_subject_conditions(state, index, UntilPhase::End, log);
    Ok(())
}

fn expire_conditions(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    if state.combatants[index].conditions.is_empty() {
        return Ok(());
    }
    let name = state.combatants[index].name.clone();
    let exhaustion_id = exhaustion_condition_id(state).to_string();
    let mut remove = Vec::new();
    let mut still_active = Vec::new();
    let mut concentration_broken = false;
    let conditions = state.combatants[index].conditions.clone();
    for (position, mut instance) in conditions.into_iter().enumerate() {
        let mut expired = false;
        if let Some(save_ends) = instance.save_ends.clone() {
            let bonus = state.combatants[index].save_bonus(save_ends.ability);
            let roll = D20Roll::roll(bonus, RollMode::Normal, rng);
            log.log(format_args!(
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
            log.log(format_args!("{name} is no longer {}", instance.condition));
            if instance.condition.eq_ignore_ascii_case(&exhaustion_id) {
                let previous = state.combatants[index].exhaustion_level;
                state.combatants[index].exhaustion_level = 0;
                log.log(format_args!(
                    "{name}'s exhaustion ends (level {previous} -> 0)"
                ));
            }
            if instance.concentration {
                concentration_broken = true;
            }
        } else {
            instance.applied_round = instance.applied_round.min(state.round);
            still_active.push(instance);
        }
    }
    let _ = remove;
    if concentration_broken {
        still_active.retain(|condition| !condition.concentration);
    }
    state.combatants[index].conditions = still_active;
    if concentration_broken {
        break_concentration(state, index, log);
    }
    Ok(())
}

fn expire_start_phases(state: &mut EncounterState, index: usize, log: &mut dyn LogSink) {
    if state.combatants[index].conditions.is_empty() {
        return;
    }
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
            log.log(format_args!("{name} is no longer {}", instance.condition));
        } else {
            keep.push(instance);
        }
    }
    state.combatants[index].conditions = keep;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UntilPhase {
    Start,
    End,
}

/// Ends `Until` conditions held by another combatant when the named subject
/// reaches the matching phase. Conditions held by the subject itself are
/// handled by `expire_start_phases`/`expire_conditions`.
fn expire_subject_conditions(
    state: &mut EncounterState,
    subject_index: usize,
    phase: UntilPhase,
    log: &mut dyn LogSink,
) {
    if state
        .combatants
        .iter()
        .all(|combatant| combatant.conditions.is_empty())
    {
        return;
    }
    let subject = state.combatants[subject_index].name.clone();
    for holder in 0..state.combatants.len() {
        if holder == subject_index {
            continue;
        }
        let holder_name = state.combatants[holder].name.clone();
        let mut concentration_broken = false;
        let mut keep = Vec::new();
        for instance in state.combatants[holder].conditions.clone() {
            let expired = match &instance.duration {
                Some(ActiveDuration::Until {
                    until,
                    subject: named,
                    end_round,
                }) => {
                    let matches_phase = match phase {
                        UntilPhase::Start => *until == DurationUntil::StartOfSourceNextTurn,
                        UntilPhase::End => matches!(
                            until,
                            DurationUntil::EndOfSourceNextTurn | DurationUntil::EndOfTargetNextTurn
                        ),
                    };
                    matches_phase
                        && named.eq_ignore_ascii_case(&subject)
                        && state.round >= *end_round
                }
                _ => false,
            };
            if expired {
                log.log(format_args!(
                    "{holder_name} is no longer {}",
                    instance.condition
                ));
                if instance.concentration {
                    concentration_broken = true;
                }
            } else {
                keep.push(instance);
            }
        }
        state.combatants[holder].conditions = keep;
        if concentration_broken {
            break_concentration(state, holder, log);
        }
    }
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
    log: &mut dyn LogSink,
) {
    if state.combatants[index].resources.is_empty() {
        return;
    }
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
            log.log(format_args!("{name} recharges {id}"));
            restore_resource(state, index, &id, log);
        }
    }
}

fn restore_resource(state: &mut EncounterState, index: usize, id: &str, _log: &mut dyn LogSink) {
    if let Some(resource) = state.combatants[index].resources.get_mut(id) {
        resource.current = resource.max;
    }
}

// ---------------------------------------------------------------------------
// Combat outcome
// ---------------------------------------------------------------------------

pub fn combat_outcome(state: &EncounterState) -> Option<CombatOutcome> {
    let mut side: Option<&str> = None;
    for combatant in state
        .combatants
        .iter()
        .filter(|combatant| combatant.is_active())
    {
        match side {
            None => side = Some(combatant.side.as_str()),
            Some(known) if known == combatant.side.as_str() => {}
            Some(_) => return None,
        }
    }
    match side {
        None => Some(CombatOutcome::Draw),
        Some(side) => Some(CombatOutcome::Winner(side.to_string())),
    }
}

fn log_outcome(state: &mut EncounterState, log: &mut dyn LogSink) {
    let outcome = combat_outcome(state);
    if outcome.is_some() && state.outcome != outcome {
        if let Some(outcome) = &outcome {
            log.log(format_args!("Outcome: {outcome}"));
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
    reroll_damage_below: Option<u32>,
    damage_die_min: Option<u32>,
    /// Flat adjustment applied once to the first damage part (off-hand
    /// attacks remove a positive ability modifier).
    damage_modifier: i32,
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
            reroll_damage_below: attack.reroll_damage_below,
            damage_die_min: attack.damage_die_min,
            damage_modifier: 0,
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
            reroll_damage_below: None,
            damage_die_min: None,
            damage_modifier: 0,
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

fn evaluate_dice(context: &ExprContext, value: &ExprValue) -> Result<DiceExpr> {
    context.evaluate_dice(&value.as_text())
}

fn roll_d20_quiet(
    state: &EncounterState,
    owner: usize,
    base: i32,
    mode: RollMode,
    advantage_sources: Vec<String>,
    disadvantage_sources: Vec<String>,
    rng: &mut dyn DiceRng,
) -> D20Result {
    let base = base + exhaustion_d20_penalty(state, owner);
    let roll = D20Roll::roll_total(base, mode, rng);
    D20Result {
        dice: Vec::new(),
        kept: roll.kept,
        base,
        bonus_terms: Vec::new(),
        total: roll.total,
        nat1: roll.nat1,
        nat20: roll.nat20,
        mode,
        advantage_sources,
        disadvantage_sources,
    }
}

fn roll_d20(
    state: &EncounterState,
    owner: usize,
    base: i32,
    mode: RollMode,
    mut bonuses: Vec<BonusTerm>,
    advantage_sources: Vec<String>,
    disadvantage_sources: Vec<String>,
    rng: &mut dyn DiceRng,
) -> D20Result {
    let exhaustion = exhaustion_d20_penalty(state, owner);
    if exhaustion != 0 {
        bonuses.push(flat_bonus_term(exhaustion, "exhaustion"));
    }
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

fn cover_label(cover: Cover) -> &'static str {
    match cover {
        Cover::None => "no cover",
        Cover::Half => "half cover",
        Cover::ThreeQuarters => "three-quarters cover",
        Cover::Total => "total cover",
    }
}

fn cover_bonus(cover: Cover, rules: &crate::content::schema::Rules) -> Option<i32> {
    match cover {
        Cover::None => Some(0),
        Cover::Half => Some(
            rules
                .cover
                .as_ref()
                .and_then(|cover| cover.half)
                .unwrap_or(2),
        ),
        Cover::ThreeQuarters => Some(
            rules
                .cover
                .as_ref()
                .and_then(|cover| cover.three_quarters)
                .unwrap_or(5),
        ),
        Cover::Total => None,
    }
}

fn cover_between(state: &EncounterState, attacker: usize, target: usize) -> Cover {
    if spatial_query_enabled(state) {
        if let Some(result) = spatial_cover_result(state, attacker, target) {
            return result.degree;
        }
    }
    let Some(space) = state.space() else {
        return Cover::None;
    };
    let from = state.combatants[attacker].position_ref();
    let to = state.combatants[target].position_ref();
    space.cover(from.as_ref(), to.as_ref())
}

/// Rejects a single-creature target behind total cover.
fn ensure_cover_allows(state: &EncounterState, attacker: usize, target: usize) -> Result<()> {
    if attacker == target {
        return Ok(());
    }
    if spatial_query_enabled(state) {
        ensure_spatial_line_of_sight(state, attacker, target)?;
    }
    if cover_between(state, attacker, target) == Cover::Total {
        bail!(
            "`{}` has total cover from `{}`",
            state.combatants[target].name,
            state.combatants[attacker].name
        );
    }
    Ok(())
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
    if !has_modifier_sources(state, target) {
        if let Some(attacker) = attacker {
            if attacker != target {
                let cover = cover_between(state, attacker, target);
                if cover != Cover::None {
                    match cover_bonus(cover, &state.content.rules) {
                        Some(bonus) => {
                            ac += bonus;
                            terms.push((bonus, cover_label(cover).to_string()));
                        }
                        None => bail!(
                            "`{}` has total cover from `{}`",
                            combatant.name,
                            state.combatants[attacker].name
                        ),
                    }
                }
            }
        }
        return Ok((ac, terms));
    }
    let context = PredicateContext {
        self_idx: target,
        actor: target,
        target: Some(target),
        attacker,
        linked: None,
        source: None,
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
        if attacker != target {
            let cover = cover_between(state, attacker, target);
            if cover != Cover::None {
                match cover_bonus(cover, &state.content.rules) {
                    Some(bonus) => {
                        ac += bonus;
                        terms.push((bonus, cover_label(cover).to_string()));
                    }
                    None => bail!(
                        "`{}` has total cover from `{}`",
                        combatant.name,
                        state.combatants[attacker].name
                    ),
                }
            }
        }
    }
    Ok((ac, terms))
}

/// Accuracy facts an AI planner can use without rolling dice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttackForecast {
    pub to_hit: i32,
    pub ac: i32,
    pub advantage: bool,
    pub disadvantage: bool,
    pub crit_chance: f64,
    pub in_range: bool,
}

fn forecast_profile(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    profile: &AttackProfile,
    source: Option<&EffectSource>,
) -> AttackForecast {
    let facts = profile.facts();
    let fast_path = source.is_none()
        && !has_modifier_sources(state, attacker)
        && !has_modifier_sources(state, target);
    let attack_ability = profile
        .ability
        .or_else(|| ability_name_from_attack_kind(profile.kind, &state.combatants[attacker]));
    let attacker_context = PredicateContext {
        self_idx: attacker,
        actor: attacker,
        target: Some(target),
        attacker: Some(attacker),
        linked: None,
        source: None,
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
        source: None,
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
    let mut has_advantage = false;
    let mut has_disadvantage = false;
    let mut auto_crit = false;
    if !fast_path {
        for collected in collect_modifiers(
            state,
            attacker,
            ModifierApplies::AttackRoll,
            &attacker_context,
            source,
        ) {
            match collected.modifier.effect {
                ModifierEffect::Advantage => has_advantage = true,
                ModifierEffect::Disadvantage => has_disadvantage = true,
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
                ModifierEffect::Advantage => has_advantage = true,
                ModifierEffect::Disadvantage => has_disadvantage = true,
                ModifierEffect::AutoCrit => auto_crit = true,
                _ => {}
            }
        }
    }
    let (ac, ac_ok) = effective_ac(state, target, Some(attacker), &facts)
        .map(|(ac, _)| (ac, true))
        .unwrap_or((state.combatants[target].creature.ac(), false));
    let mut in_range = ac_ok;
    if let Some(space) = state.space() {
        let attacker_position = state.combatants[attacker].position_ref();
        let target_position = state.combatants[target].position_ref();
        let (attacker_position, target_position) =
            (attacker_position.as_ref(), target_position.as_ref());
        let distance = space.distance(attacker_position, target_position);
        if let Some(distance) = distance {
            let mut kind = profile.kind;
            if kind == AttackKind::MeleeOrRanged {
                kind = if space.can_reach(attacker_position, target_position, profile.reach) {
                    AttackKind::Melee
                } else {
                    AttackKind::Ranged
                };
            }
            in_range = match kind {
                AttackKind::Melee => {
                    space.can_reach(attacker_position, target_position, profile.reach)
                }
                AttackKind::Ranged | AttackKind::MeleeOrRanged => {
                    profile.range.is_some_and(|[_, long]| distance <= long)
                }
            };
            if kind != AttackKind::Melee
                && profile.range.is_some_and(|[normal, _]| distance > normal)
            {
                has_disadvantage = true;
            }
            if kind != AttackKind::Melee && hostile_within_5(state, attacker) {
                has_disadvantage = true;
            }
            if space.light_at(target_position) == Light::Dark
                && !has_dark_sense(&state.combatants[attacker])
            {
                has_disadvantage = true;
            }
            if space.light_at(attacker_position) == Light::Dark
                && !has_dark_sense(&state.combatants[target])
            {
                has_advantage = true;
            }
        } else {
            in_range = false;
        }
    }
    let mode = RollMode::from_flags(has_advantage, has_disadvantage);
    let advantage = mode == RollMode::Advantage;
    let disadvantage = mode == RollMode::Disadvantage;
    let crit_chance = if auto_crit {
        1.0
    } else {
        match mode {
            RollMode::Normal => 0.05,
            RollMode::Advantage => 0.0975,
            RollMode::Disadvantage => 0.0025,
        }
    };
    let context = source_context(
        state,
        attacker,
        source,
        Some(profile.attack_modifier(&state.combatants[attacker])),
        source.and_then(|source| source.slot_level),
    );
    let to_hit = evaluate_value(&context, &profile.to_hit)
        .map(|(_, expression)| expression.modifier)
        .unwrap_or(0);
    AttackForecast {
        to_hit,
        ac,
        advantage,
        disadvantage,
        crit_chance,
        in_range,
    }
}

/// Forecasts a creature attack against one target without consuming or rolling.
pub fn forecast_attack(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    attack: &Attack,
) -> AttackForecast {
    let profile = AttackProfile::from_attack(attack);
    forecast_profile(state, attacker, target, &profile, None)
}

/// Cheap forecast for an attack when neither combatant has modifier sources.
pub fn simple_attack_forecast(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    to_hit: i32,
    kind: AttackKind,
    reach: i32,
    range: Option<[i32; 2]>,
) -> AttackForecast {
    let space = state.space();
    let mut ac = state.combatants[target].creature.ac();
    let mut in_range = true;
    let mut total_cover = false;
    let mut has_advantage = false;
    let mut has_disadvantage = false;
    let attacker_position = state.combatants[attacker].position_ref();
    let target_position = state.combatants[target].position_ref();
    if let Some(space) = space.as_ref() {
        let attacker_position = attacker_position.as_ref();
        let target_position = target_position.as_ref();
        if attacker != target {
            match cover_bonus(
                space.cover(attacker_position, target_position),
                &state.content.rules,
            ) {
                Some(bonus) => ac += bonus,
                None => total_cover = true,
            }
        }
        if let Some(distance) = space.distance(attacker_position, target_position) {
            let mut effective_kind = kind;
            if effective_kind == AttackKind::MeleeOrRanged {
                effective_kind = if space.can_reach(attacker_position, target_position, reach) {
                    AttackKind::Melee
                } else {
                    AttackKind::Ranged
                };
            }
            in_range = match effective_kind {
                AttackKind::Melee => space.can_reach(attacker_position, target_position, reach),
                AttackKind::Ranged | AttackKind::MeleeOrRanged => {
                    range.is_some_and(|[_, long]| distance <= long)
                }
            };
            if effective_kind != AttackKind::Melee
                && range.is_some_and(|[normal, _]| distance > normal)
            {
                has_disadvantage = true;
            }
            if effective_kind != AttackKind::Melee && hostile_within_5(state, attacker) {
                has_disadvantage = true;
            }
            if space.light_at(target_position) == Light::Dark
                && !has_dark_sense(&state.combatants[attacker])
            {
                has_disadvantage = true;
            }
            if space.light_at(attacker_position) == Light::Dark
                && !has_dark_sense(&state.combatants[target])
            {
                has_advantage = true;
            }
        } else {
            in_range = false;
        }
    }
    if total_cover {
        in_range = false;
    }
    let mode = RollMode::from_flags(has_advantage, has_disadvantage);
    let advantage = mode == RollMode::Advantage;
    let disadvantage = mode == RollMode::Disadvantage;
    let crit_chance = match mode {
        RollMode::Normal => 0.05,
        RollMode::Advantage => 0.0975,
        RollMode::Disadvantage => 0.0025,
    };
    AttackForecast {
        to_hit,
        ac,
        advantage,
        disadvantage,
        crit_chance,
        in_range,
    }
}

/// Forecasts an ability `attack` effect against one target without rolling.
pub fn forecast_attack_effect(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    effect: &AttackEffect,
    tags: &[String],
    params: &BTreeMap<String, ExprValue>,
    slot_level: Option<i32>,
    range: Option<i32>,
) -> AttackForecast {
    let source = EffectSource {
        id: "forecast".to_string(),
        name: "forecast".to_string(),
        tags: tags.to_vec(),
        slot_level,
        concentration: false,
        params: params
            .iter()
            .map(|(key, value)| (key.clone(), value.as_text()))
            .collect(),
        duration: None,
        range,
        modifiers: Vec::new(),
    };
    let profile = AttackProfile::from_effect(effect, &source, range);
    forecast_profile(state, attacker, target, &profile, Some(&source))
}

/// Evaluates an expression for one actor, returning the parsed dice.
pub fn evaluate_for_actor(
    state: &EncounterState,
    actor: usize,
    value: &ExprValue,
    params: &BTreeMap<String, ExprValue>,
    attack_mod: Option<i32>,
    slot_level: Option<i32>,
) -> Result<DiceExpr> {
    let mut context = score_context(&state.combatants[actor]);
    if !params.is_empty() {
        context = context.with_params(
            params
                .iter()
                .map(|(key, value)| (key.clone(), value.as_text()))
                .collect(),
        );
    }
    if let Some(slot_level) = slot_level {
        context = context.with_slot_level(slot_level);
    }
    if let Some(attack_mod) = attack_mod {
        context = context.with_attack_mod(attack_mod);
    }
    Ok(evaluate_value(&context, value)?.1)
}

/// Evaluates an expression to its average value for one actor.
pub fn average_expression(
    state: &EncounterState,
    actor: usize,
    value: &ExprValue,
    params: &BTreeMap<String, ExprValue>,
    attack_mod: Option<i32>,
    slot_level: Option<i32>,
) -> Result<f64> {
    Ok(evaluate_for_actor(state, actor, value, params, attack_mod, slot_level)?.average())
}

/// Chance that a target fails a saving throw, including condition modifiers
/// and (for Dex saves) cover.
pub fn save_fail_chance(
    state: &EncounterState,
    target: usize,
    ability: AbilityName,
    dc: i32,
    ignore_cover: bool,
    attacker: Option<usize>,
) -> f64 {
    let combatant = &state.combatants[target];
    let mut has_advantage = false;
    let mut has_disadvantage = false;
    let mut auto_fail = false;
    let mut bonus = 0.0;
    if has_modifier_sources(state, target) {
        let context = PredicateContext {
            self_idx: target,
            actor: target,
            target: Some(target),
            attacker,
            linked: None,
            source: None,
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
        for collected in collect_modifiers(state, target, ModifierApplies::Save, &context, None) {
            match collected.modifier.effect {
                ModifierEffect::Advantage => has_advantage = true,
                ModifierEffect::Disadvantage => has_disadvantage = true,
                ModifierEffect::AutoFail => auto_fail = true,
                ModifierEffect::Bonus => {
                    if let Some(value) = &collected.modifier.value {
                        if let Ok(average) =
                            average_expression(state, target, value, &BTreeMap::new(), None, None)
                        {
                            bonus += average;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if auto_fail {
        return 1.0;
    }
    bonus += f64::from(exhaustion_d20_penalty(state, target));
    if ability == AbilityName::Dex && !ignore_cover {
        if let Some(attacker) = attacker {
            if attacker != target {
                if let Some(cover_bonus) =
                    cover_bonus(cover_between(state, attacker, target), &state.content.rules)
                {
                    bonus += f64::from(cover_bonus);
                }
            }
        }
    }
    let need = f64::from(dc) - f64::from(combatant.save_bonus(ability)) - bonus;
    let mode = RollMode::from_flags(has_advantage, has_disadvantage);
    let success = d20_at_least(need, mode);
    1.0 - success
}

fn d20_at_least(need: f64, mode: RollMode) -> f64 {
    let single = ((21.0 - need).clamp(0.0, 20.0)) / 20.0;
    match mode {
        RollMode::Normal => single,
        RollMode::Advantage => 1.0 - (1.0 - single) * (1.0 - single),
        RollMode::Disadvantage => single * single,
    }
}

/// Damage multiplier from resistances, immunities and damage-taken modifiers.
pub fn damage_multiplier(
    state: &EncounterState,
    target: usize,
    source: Option<usize>,
    damage_type: &str,
    source_tags: &[String],
) -> f64 {
    let combatant = &state.combatants[target];
    let static_immune = combatant
        .creature
        .immunities
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(damage_type));
    if static_immune {
        return 0.0;
    }
    let has_static_lists = combatant
        .creature
        .resistances
        .as_deref()
        .is_some_and(|list| !list.is_empty())
        || combatant
            .creature
            .vulnerabilities
            .as_deref()
            .is_some_and(|list| !list.is_empty());
    if !has_static_lists && !has_modifier_sources(state, target) {
        return 1.0;
    }
    let mut multiplier = 1.0f64;
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
        attacker: source,
        linked: None,
        source: None,
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
            ModifierEffect::Immunity => return 0.0,
            ModifierEffect::Resistance => multiplier *= 0.5,
            ModifierEffect::Vulnerability => multiplier *= 2.0,
            _ => {}
        }
    }
    multiplier
}

/// Whether a combatant currently has a generic engine condition flag.
pub fn has_condition_flag(state: &EncounterState, index: usize, flag: ConditionFlag) -> bool {
    has_flag(state, index, flag)
}

/// Resolves an already-resolved ability directly, mainly for AI turns.
#[allow(clippy::too_many_arguments)]
pub fn use_planned_ability(
    state: &mut EncounterState,
    actor: usize,
    resolved: &ResolvedAbility,
    targets: &[usize],
    zone_center: Option<&Position>,
    option: Option<usize>,
    slot_level: Option<i32>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let source = EffectSource::from_ability(&resolved.ability, &resolved.params);
    let options = AttackOptions::default();
    // The AI passes a position, not a creature identity; treat it as a cell
    // centre so a flyer cannot lift a ground-targeted area.
    let center = zone_center.map(|position| AreaCenter {
        position: position.clone(),
        creature: None,
    });
    use_resolved_ability(
        state,
        actor,
        resolved,
        &source,
        targets,
        center.as_ref(),
        &options,
        option,
        slot_level,
        AbilitySettlement {
            action_spent: false,
            ..AbilitySettlement::default()
        },
        rng,
        log,
    )?;
    log_outcome(state, log);
    Ok(())
}

fn ac_text(total: i32, terms: &[(i32, String)]) -> String {
    let bonus: i32 = terms.iter().map(|(value, _)| *value).sum();
    let base = total - bonus;
    let mut text = format!("AC {base}");
    for (value, label) in terms {
        if label.ends_with("cover") {
            text.push_str(&format!("{}({label})", signed(*value)));
        } else {
            text.push_str(&format!("{}[{label}]", signed(*value)));
        }
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
    log: &mut dyn LogSink,
) -> Result<()> {
    let attacker_index = state
        .find_index(attacker)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target_index = state
        .find_index(target)
        .map_err(|error| anyhow!(error.to_string()))?;
    attack_at(
        state,
        attacker_index,
        target_index,
        attack_name,
        options,
        rng,
        log,
    )
}

/// [`attack`] for callers that already resolved combatant indices. The AI uses
/// this to skip name formatting and lookup on every action.
pub fn attack_at(
    state: &mut EncounterState,
    attacker_index: usize,
    target_index: usize,
    attack_name: &str,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    attack_at_with_limit(
        state,
        attacker_index,
        target_index,
        attack_name,
        options,
        None,
        rng,
        log,
    )
}

/// One attack from the Attack action. It consumes exactly one slot and leaves
/// the remaining slots open for a later `attack`, `grapple` or `shove`.
pub fn attack_one(
    state: &mut EncounterState,
    attacker_name: &str,
    target_name: &str,
    attack_name: &str,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let attacker_index = state
        .find_index(attacker_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target_index = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    attack_at_with_limit(
        state,
        attacker_index,
        target_index,
        attack_name,
        options,
        Some(1),
        rng,
        log,
    )
}

fn attack_at_with_limit(
    state: &mut EncounterState,
    attacker_index: usize,
    target_index: usize,
    attack_name: &str,
    options: &AttackOptions,
    max_attacks: Option<u32>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    ensure_can_act(state, attacker_index)?;
    ensure_targetable(state, target_index)?;
    ensure_not_charmed_target(state, attacker_index, target_index, true)?;
    if attacker_index == target_index {
        bail!("a creature cannot attack itself");
    }
    if !hostile_to(state, attacker_index, target_index) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target_index].name,
            state.combatants[attacker_index].name
        );
    }
    if options.multi {
        if max_attacks.is_some() {
            bail!("a single attack cannot be combined with multiattack");
        }
        let entries = state.combatants[attacker_index]
            .creature
            .multiattack
            .clone()
            .ok_or_else(|| {
                anyhow!(
                    "`{}` has no multiattack",
                    state.combatants[attacker_index].name
                )
            })?;
        spend_action(state, attacker_index, Activation::Action, log)?;
        log.log(format_args!(
            "{} uses multiattack",
            state.combatants[attacker_index].name
        ));
        for entry in entries {
            if state.combatants[target_index].is_dead() {
                log.log(format_args!(
                    "{} is already down; remaining multiattack attacks are skipped",
                    state.combatants[target_index].name
                ));
                break;
            }
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

    let profile = state.combatants[attacker_index]
        .find_attack(attack_name)
        .ok()
        .map(AttackProfile::from_attack);
    if let Some(profile) = profile {
        let remaining = begin_attack_slots(state, attacker_index)?;
        let attacks = max_attacks.map_or(remaining, |limit| remaining.min(limit));
        consume_attack_slots(state, attacker_index, attacks);
        if max_attacks.is_some() && remaining > 1 {
            log.log(format_args!(
                "{} uses one attack; {} Attack action attack(s) remain",
                state.combatants[attacker_index].name,
                remaining - 1
            ));
        }
        let used_in_melee = profile.kind == AttackKind::Melee
            || (profile.kind == AttackKind::MeleeOrRanged
                && ensure_melee_reachable(state, attacker_index, target_index, profile.reach)
                    .is_ok());
        if is_light_melee(&profile) && used_in_melee {
            state.combatants[attacker_index].once_used.insert(
                format!("turn:combat-actions:light-main:{}", profile.id),
                state.round,
            );
        }
        if attacks > 1 {
            log.log(format_args!(
                "{}'s Extra Attack: {attacks} attacks with {}",
                state.combatants[attacker_index].name, profile.name
            ));
        }
        for attack_number in 1..=attacks {
            if state.combatants[target_index].is_dead() {
                log.log(format_args!(
                    "{} is already down; remaining attacks are skipped",
                    state.combatants[target_index].name
                ));
                break;
            }
            if attacks > 1 {
                log.log(format_args!("attack {attack_number}/{attacks}"));
            }
            resolve_attack(
                state,
                attacker_index,
                target_index,
                &profile,
                None,
                options,
                rng,
                log,
                true,
                false,
            )?;
        }
    } else {
        if max_attacks.is_some() {
            bail!(
                "a single attack requires a weapon or unarmed attack, not ability `{attack_name}`"
            );
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
    }
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
    log: &mut dyn LogSink,
) -> Result<()> {
    let combatant = &state.combatants[attacker];
    if let Ok(attack) = combatant.find_attack(name) {
        let profile = AttackProfile::from_attack(attack);
        return resolve_attack(
            state, attacker, target, &profile, None, options, rng, log, false, false,
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
        None,
        options,
        None,
        None,
        AbilitySettlement {
            action_spent: auto,
            ..AbilitySettlement::default()
        },
        rng,
        log,
    )
}

fn spend_action(
    state: &mut EncounterState,
    index: usize,
    activation: Activation,
    log: &mut dyn LogSink,
) -> Result<()> {
    match activation {
        Activation::Action => {
            if state.combatants[index].action_used {
                bail!(
                    "`{}` has already used an action this turn",
                    state.combatants[index].name
                );
            }
            // A whole-action path cannot inherit stale Attack action slots.
            state.combatants[index]
                .once_used
                .retain(|key, _| !key.starts_with(ATTACK_SLOT_PREFIX));
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

fn spend_reaction(state: &mut EncounterState, index: usize, _log: &mut dyn LogSink) -> Result<()> {
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

// ---------------------------------------------------------------------------
// Attack action budget
// ---------------------------------------------------------------------------

fn attack_slots_used(state: &EncounterState, index: usize) -> u32 {
    u32::try_from(
        state.combatants[index]
            .once_used
            .iter()
            .filter(|(key, round)| key.starts_with(ATTACK_SLOT_PREFIX) && **round == state.round)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// Starts or continues the Attack action and returns how many attacks remain.
///
/// A creature that has not used an action this turn starts the Attack action
/// and gets `1 + extra_attacks` slots. A creature in the middle of that Attack
/// action (marked with `turn:attack-action:slot:*`) gets the remaining slots.
/// Any other action already used this turn fails closed.
fn begin_attack_slots(state: &mut EncounterState, index: usize) -> Result<u32> {
    let total = 1u32.saturating_add(extra_attack_count(state, index));
    let used = attack_slots_used(state, index);
    let name = state.combatants[index].name.clone();
    if state.combatants[index].action_used {
        if used == 0 {
            bail!("`{name}` has already used an action this turn");
        }
        if used >= total {
            bail!("`{name}` has no Attack action attacks left this turn");
        }
        return Ok(total - used);
    }
    if used > 0 {
        // A manual action reset opened a new turn without the normal start-of-
        // turn cleanup; discard the stale slots from the previous action.
        state.combatants[index]
            .once_used
            .retain(|key, _| !key.starts_with(ATTACK_SLOT_PREFIX));
    }
    state.combatants[index].action_used = true;
    Ok(total)
}

fn consume_attack_slot(state: &mut EncounterState, index: usize) {
    let used = attack_slots_used(state, index);
    state.combatants[index]
        .once_used
        .insert(format!("{ATTACK_SLOT_PREFIX}{}", used + 1), state.round);
}

fn consume_attack_slots(state: &mut EncounterState, index: usize, count: u32) {
    for _ in 0..count {
        consume_attack_slot(state, index);
    }
}

// ---------------------------------------------------------------------------
// Core combat actions: grapple, shove, unarmed strike, two-weapon fighting
// ---------------------------------------------------------------------------

fn unarmed_rule(state: &EncounterState) -> Result<&UnarmedStrikeRule> {
    state
        .content
        .rules
        .combat_actions
        .as_ref()
        .and_then(|actions| actions.unarmed_strike.as_ref())
        .ok_or_else(|| {
            anyhow!(
                "this ruleset has no `combat_actions.unarmed_strike` data; load the edition rules and run `content check`"
            )
        })
}

fn grapple_rule(state: &EncounterState) -> Result<&GrappleRule> {
    state
        .content
        .rules
        .combat_actions
        .as_ref()
        .and_then(|actions| actions.grapple.as_ref())
        .ok_or_else(|| {
            anyhow!(
                "this ruleset has no `combat_actions.grapple` data; load the edition rules and run `content check`"
            )
        })
}

fn shove_rule(state: &EncounterState) -> Result<&ShoveRule> {
    state
        .content
        .rules
        .combat_actions
        .as_ref()
        .and_then(|actions| actions.shove.as_ref())
        .ok_or_else(|| {
            anyhow!(
                "this ruleset has no `combat_actions.shove` data; load the edition rules and run `content check`"
            )
        })
}

fn two_weapon_rule(state: &EncounterState) -> Result<&TwoWeaponRule> {
    state
        .content
        .rules
        .combat_actions
        .as_ref()
        .and_then(|actions| actions.two_weapon_fighting.as_ref())
        .ok_or_else(|| {
            anyhow!(
                "this ruleset has no `combat_actions.two_weapon_fighting` data; load the edition rules and run `content check`"
            )
        })
}

fn extra_attack_count(state: &EncounterState, index: usize) -> u32 {
    let combatant = &state.combatants[index];
    let mut extra = combatant.creature.extra_attacks.unwrap_or(0);
    for entry in combatant.creature.features.iter().flatten() {
        let ability = match entry {
            AbilityEntry::Inline(ability) => ability.as_ref(),
            AbilityEntry::Ref(reference) => {
                let Some(ability) = state.content.abilities.get(&reference.reference) else {
                    continue;
                };
                ability
            }
        };
        if ability.activation == Activation::Passive {
            if let Some(value) = ability.extra_attacks {
                extra = extra.max(value);
            }
        }
    }
    extra
}

fn is_light_melee(profile: &AttackProfile) -> bool {
    matches!(profile.kind, AttackKind::Melee | AttackKind::MeleeOrRanged)
        && profile.properties.contains(&AttackProperty::Light)
}

fn skill_label(skill: &str) -> String {
    let mut chars = skill.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn ensure_action_size(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    max_size_difference: Option<i32>,
) -> Result<()> {
    let maximum = u8::try_from(max_size_difference.unwrap_or(1).max(0)).unwrap_or(u8::MAX);
    let allowed = size_rank(state.combatants[attacker].creature.size()).saturating_add(maximum);
    let actual = size_rank(state.combatants[target].creature.size());
    if actual > allowed {
        bail!(
            "`{}` is too large to affect: target size {:?}, attacker size {:?} (at most {} size larger)",
            state.combatants[target].name,
            state.combatants[target].creature.size(),
            state.combatants[attacker].creature.size(),
            maximum
        );
    }
    Ok(())
}

fn ensure_melee_reachable(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    reach: i32,
) -> Result<()> {
    let space = state
        .space()
        .ok_or_else(|| anyhow!("this state has no scene"))?;
    let from = state.combatants[attacker].position_ref();
    let to = state.combatants[target].position_ref();
    let (from, to) = (from.as_ref(), to.as_ref());
    let spatial = spatial_query_enabled(state);
    let in_reach = if spatial {
        spatial_can_reach(state, attacker, target, reach)
            .unwrap_or_else(|| space.can_reach(from, to, reach))
    } else {
        space.can_reach(from, to, reach)
    };
    if !in_reach {
        let distance = if spatial {
            spatial_body_distance(state, attacker, target)
        } else {
            space.distance(from, to)
        }
        .unwrap_or(0);
        bail!(
            "`{}` is out of reach ({distance} ft; reach {reach})",
            state.combatants[target].name
        );
    }
    if spatial {
        ensure_spatial_line_of_sight(state, attacker, target)?;
    }
    Ok(())
}

fn choose_contest_defense(
    state: &EncounterState,
    target: usize,
    skills: &[String],
    requested: Option<&str>,
    log: &mut dyn LogSink,
) -> Result<String> {
    if skills.is_empty() {
        bail!("grapple/shove contest has no defender skill");
    }
    let chosen = if let Some(requested) = requested {
        let requested = requested.trim();
        skills
            .iter()
            .find(|skill| skill.eq_ignore_ascii_case(requested))
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "`{requested}` is not a legal defense; choose one of {}",
                    skills.join(", ")
                )
            })?
    } else {
        let mut best = skills[0].clone();
        let mut best_bonus = state.combatants[target].skill_bonus(&best);
        for skill in &skills[1..] {
            let bonus = state.combatants[target].skill_bonus(skill);
            if bonus > best_bonus {
                best = skill.clone();
                best_bonus = bonus;
            }
        }
        log.log(format_args!(
            "{} chooses {} ({best_bonus:+}) to defend",
            state.combatants[target].name,
            skill_label(&best)
        ));
        best
    };
    Ok(chosen)
}

fn choose_save_ability(
    state: &EncounterState,
    target: usize,
    abilities: &[AbilityName],
    requested: Option<&str>,
    log: &mut dyn LogSink,
) -> Result<AbilityName> {
    if abilities.is_empty() {
        bail!("grapple/shove save has no legal ability");
    }
    let chosen = if let Some(requested) = requested {
        let requested = ability_from_text(requested).ok_or_else(|| {
            anyhow!(
                "`{requested}` is not a legal save ability; choose one of {}",
                abilities
                    .iter()
                    .map(|ability| ability.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        if !abilities.contains(&requested) {
            bail!(
                "`{}` is not a legal save ability; choose one of {}",
                requested.as_str(),
                abilities
                    .iter()
                    .map(|ability| ability.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        requested
    } else {
        let mut best = abilities[0];
        let mut best_bonus = state.combatants[target].save_bonus(best);
        for ability in &abilities[1..] {
            let bonus = state.combatants[target].save_bonus(*ability);
            if bonus > best_bonus {
                best = *ability;
                best_bonus = bonus;
            }
        }
        log.log(format_args!(
            "{} chooses a {} save ({best_bonus:+}) to defend",
            state.combatants[target].name,
            best.as_str().to_uppercase()
        ));
        best
    };
    Ok(chosen)
}

enum ActionDefense {
    Contest {
        skill: String,
        defense: String,
    },
    Save {
        ability: AbilityName,
        dc: i32,
        formula: String,
    },
}

fn resolve_action_contest(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    attacker_skill: &str,
    defense: &str,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<bool> {
    let attacker_base = state.combatants[attacker].skill_bonus(attacker_skill);
    let (mode, advantages, disadvantages) = check_modifiers(state, attacker, None, attacker_skill);
    let attacker_roll = roll_d20(
        state,
        attacker,
        attacker_base,
        mode,
        Vec::new(),
        advantages,
        disadvantages,
        rng,
    );
    let target_base = state.combatants[target].skill_bonus(defense);
    let (mode, advantages, disadvantages) = check_modifiers(state, target, None, defense);
    let target_roll = roll_d20(
        state,
        target,
        target_base,
        mode,
        Vec::new(),
        advantages,
        disadvantages,
        rng,
    );
    let success = attacker_roll.total > target_roll.total;
    log.log(format_args!(
        "{} {} contest: {} vs {} {} contest: {} — {} (defender wins ties)",
        state.combatants[attacker].name,
        skill_label(attacker_skill),
        attacker_roll.text(),
        state.combatants[target].name,
        skill_label(defense),
        target_roll.text(),
        if success { "PASS" } else { "FAIL" }
    ));
    Ok(success)
}

fn action_dc(
    state: &EncounterState,
    attacker: usize,
    value: Option<&ExprValue>,
) -> Result<(i32, String)> {
    let value =
        value.ok_or_else(|| anyhow!("combat action save is missing `dc`; run `content check`"))?;
    let context = score_context(&state.combatants[attacker]);
    let (formula, expression) = evaluate_value(&context, value)?;
    if !expression.terms.is_empty() {
        bail!("combat action DC `{formula}` must be a flat number");
    }
    Ok((expression.modifier, formula))
}

fn resolve_action_save(
    state: &mut EncounterState,
    target: usize,
    ability: AbilityName,
    dc: i32,
    formula: &str,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<bool> {
    let (mode, bonuses, advantages, disadvantages, auto_fail) = save_modifiers(
        state,
        target,
        ability,
        &score_context(&state.combatants[target]),
        rng,
    )?;
    if auto_fail {
        log.log(format_args!(
            "{} automatically fails the {} save (DC {dc})",
            state.combatants[target].name,
            ability.as_str().to_uppercase()
        ));
        return Ok(false);
    }
    let base = state.combatants[target].save_bonus(ability);
    let result = roll_d20(
        state,
        target,
        base,
        mode,
        bonuses,
        advantages,
        disadvantages,
        rng,
    );
    let success = result.total >= dc;
    log.log(format_args!(
        "{} {} save: {} vs DC {dc} ({formula}) {}",
        state.combatants[target].name,
        ability.as_str().to_uppercase(),
        result.text(),
        if success { "PASS" } else { "FAIL" }
    ));
    Ok(success)
}

fn action_condition_notes(state: &EncounterState, attacker: usize, dc: Option<i32>) -> String {
    match dc {
        Some(dc) => format!("escape DC {dc}: action, Athletics or Acrobatics check"),
        None => {
            let bonus = state.combatants[attacker].skill_bonus("athletics");
            format!("escape: action, Athletics or Acrobatics contest vs Athletics {bonus:+}")
        }
    }
}

fn apply_action_condition(
    state: &mut EncounterState,
    target: usize,
    condition: &str,
    source: usize,
    linked: bool,
    notes: String,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let source_name = state.combatants[source].name.clone();
    let duration = Duration {
        rounds: None,
        until: None,
        minutes: None,
        permanent: Some(true),
        ends_on: Vec::new(),
    };
    apply_condition_internal(
        state,
        target,
        condition,
        Some(source),
        linked.then(|| source_name.clone()),
        Some(duration),
        None,
        None,
        Vec::new(),
        Some("combat-actions".to_string()),
        false,
        rng,
        log,
    )?;
    if let Some(active) = state.combatants[target]
        .conditions
        .iter_mut()
        .find(|active| active.condition.eq_ignore_ascii_case(condition))
    {
        active.notes = Some(notes);
    }
    Ok(())
}

/// The 2014/2024 grapple action.
pub fn grapple(
    state: &mut EncounterState,
    attacker_name: &str,
    target_name: &str,
    options: &GrappleOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let attacker = state
        .find_index(attacker_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, attacker)?;
    ensure_targetable(state, target)?;
    ensure_not_charmed_target(state, attacker, target, true)?;
    if attacker == target {
        bail!("a creature cannot grapple itself");
    }
    if !hostile_to(state, attacker, target) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target].name,
            state.combatants[attacker].name
        );
    }
    let rule = grapple_rule(state)?.clone();
    let condition = rule.condition.clone().ok_or_else(|| {
        anyhow!("`combat_actions.grapple.condition` is missing; run `content check`")
    })?;
    let target_label = state.combatants[target].name.clone();
    if state.combatants[target].has_condition(&state.content, &condition) {
        bail!("`{target_label}` is already {condition}");
    }
    if state.combatants[target].is_immune_to_condition(&condition) {
        bail!("`{target_label}` is immune to `{condition}`");
    }
    ensure_action_size(state, attacker, target, rule.max_size_difference)?;
    let reach = unarmed_rule(state)?.reach.unwrap_or(5);
    ensure_melee_reachable(state, attacker, target, reach)?;

    let resolution = rule.resolution.unwrap_or_default();
    let defense = match resolution {
        ActionResolution::Contest => {
            let skill = rule.attacker_skill.clone().ok_or_else(|| {
                anyhow!("combat action contest is missing `attacker_skill`; run `content check`")
            })?;
            let skills = rule.defender_skills.as_deref().ok_or_else(|| {
                anyhow!("combat action contest is missing `defender_skills`; run `content check`")
            })?;
            let defense =
                choose_contest_defense(state, target, skills, options.defense.as_deref(), log)?;
            ActionDefense::Contest { skill, defense }
        }
        ActionResolution::Save => {
            let abilities = rule.save_abilities.as_deref().ok_or_else(|| {
                anyhow!("combat action save is missing `save_abilities`; run `content check`")
            })?;
            let (dc, formula) = action_dc(state, attacker, rule.dc.as_ref())?;
            let ability =
                choose_save_ability(state, target, abilities, options.defense.as_deref(), log)?;
            ActionDefense::Save {
                ability,
                dc,
                formula,
            }
        }
    };

    begin_attack_slots(state, attacker)?;
    consume_attack_slot(state, attacker);
    let success = match &defense {
        ActionDefense::Contest { skill, defense } => {
            resolve_action_contest(state, attacker, target, skill, defense, rng, log)?
        }
        ActionDefense::Save {
            ability,
            dc,
            formula,
        } => resolve_action_save(state, target, *ability, *dc, formula, rng, log)?,
    };
    let dc = match &defense {
        ActionDefense::Save { dc, .. } => Some(*dc),
        ActionDefense::Contest { .. } => None,
    };

    let attacker_label = state.combatants[attacker].name.clone();
    if success {
        let notes = action_condition_notes(state, attacker, dc);
        apply_action_condition(
            state,
            target,
            &condition,
            attacker,
            true,
            notes.clone(),
            rng,
            log,
        )?;
        log.log(format_args!(
            "{attacker_label} grapples {target_label}; {target_label} is {condition} ({notes})"
        ));
    } else {
        log.log(format_args!(
            "{attacker_label} fails to grapple {target_label}; no condition applied"
        ));
    }
    log_outcome(state, log);
    Ok(())
}

/// The 2014/2024 shove action.
pub fn shove(
    state: &mut EncounterState,
    attacker_name: &str,
    target_name: &str,
    options: &ShoveOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let attacker = state
        .find_index(attacker_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, attacker)?;
    ensure_targetable(state, target)?;
    ensure_not_charmed_target(state, attacker, target, true)?;
    if attacker == target {
        bail!("a creature cannot shove itself");
    }
    if !hostile_to(state, attacker, target) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target].name,
            state.combatants[attacker].name
        );
    }
    let rule = shove_rule(state)?.clone();
    let prone_condition = rule.prone_condition.clone().ok_or_else(|| {
        anyhow!("`combat_actions.shove.prone_condition` is missing; run `content check`")
    })?;
    let target_label = state.combatants[target].name.clone();
    if options.mode == ShoveMode::KnockDown {
        if state.combatants[target].has_condition(&state.content, &prone_condition) {
            bail!("`{target_label}` is already {prone_condition}");
        }
        if state.combatants[target].is_immune_to_condition(&prone_condition) {
            bail!("`{target_label}` is immune to `{prone_condition}`");
        }
    }
    ensure_action_size(state, attacker, target, rule.max_size_difference)?;
    let reach = unarmed_rule(state)?.reach.unwrap_or(5);
    ensure_melee_reachable(state, attacker, target, reach)?;

    let resolution = rule.resolution.unwrap_or_default();
    let defense = match resolution {
        ActionResolution::Contest => {
            let skill = rule.attacker_skill.clone().ok_or_else(|| {
                anyhow!("combat action contest is missing `attacker_skill`; run `content check`")
            })?;
            let skills = rule.defender_skills.as_deref().ok_or_else(|| {
                anyhow!("combat action contest is missing `defender_skills`; run `content check`")
            })?;
            let defense =
                choose_contest_defense(state, target, skills, options.defense.as_deref(), log)?;
            ActionDefense::Contest { skill, defense }
        }
        ActionResolution::Save => {
            let abilities = rule.save_abilities.as_deref().ok_or_else(|| {
                anyhow!("combat action save is missing `save_abilities`; run `content check`")
            })?;
            let (dc, formula) = action_dc(state, attacker, rule.dc.as_ref())?;
            let ability =
                choose_save_ability(state, target, abilities, options.defense.as_deref(), log)?;
            ActionDefense::Save {
                ability,
                dc,
                formula,
            }
        }
    };

    begin_attack_slots(state, attacker)?;
    consume_attack_slot(state, attacker);
    let success = match &defense {
        ActionDefense::Contest { skill, defense } => {
            resolve_action_contest(state, attacker, target, skill, defense, rng, log)?
        }
        ActionDefense::Save {
            ability,
            dc,
            formula,
        } => resolve_action_save(state, target, *ability, *dc, formula, rng, log)?,
    };

    let attacker_label = state.combatants[attacker].name.clone();
    if !success {
        log.log(format_args!(
            "{attacker_label} fails to shove {target_label}; no effect"
        ));
        log_outcome(state, log);
        return Ok(());
    }

    match options.mode {
        ShoveMode::KnockDown => {
            let notes = format!("shoved down by {attacker_label}");
            apply_action_condition(
                state,
                target,
                &prone_condition,
                attacker,
                false,
                notes.clone(),
                rng,
                log,
            )?;
            log.log(format_args!(
                "{attacker_label} shoves {target_label} down: {target_label} is {prone_condition} ({notes})"
            ));
        }
        ShoveMode::Away => {
            let distance = rule.distance.ok_or_else(|| {
                anyhow!("combat action shove is missing `distance`; run `content check`")
            })?;
            let before = state.combatants[target].position();
            let context = EffectContext {
                actor: attacker,
                target: Some(target),
                source: None,
                attack: None,
                option: None,
                crit: false,
                damage_dealt: 0,
                damage_scale: 1.0,
                has_advantage: false,
                has_disadvantage: false,
            };
            apply_forced_move(
                state,
                &context,
                &ForcedMoveEffect {
                    distance,
                    direction: Direction::Away,
                    max_size: None,
                },
                rng,
                log,
            )?;
            let after = state.combatants[target].position();
            if before == after {
                log.log(format_args!(
                    "{target_label} cannot be pushed further (blocked by a wall, occupied space or the scene boundary)"
                ));
            } else {
                log.log(format_args!(
                    "{attacker_label} pushes {target_label} {distance} ft away"
                ));
            }
        }
    }
    log_outcome(state, log);
    Ok(())
}

fn unarmed_profile(state: &EncounterState, attacker: usize) -> Result<AttackProfile> {
    let rule = unarmed_rule(state)?.clone();
    let ability = rule.ability.unwrap_or(AbilityName::Str);
    let proficient = rule.proficient.unwrap_or(true);
    let modifier = state.combatants[attacker]
        .creature
        .ability_modifier(ability);
    let to_hit = modifier
        + if proficient {
            state.combatants[attacker].creature.proficiency_bonus()
        } else {
            0
        };
    let damage = rule.damage.ok_or_else(|| {
        anyhow!("`combat_actions.unarmed_strike.damage` is missing; run `content check`")
    })?;
    Ok(AttackProfile {
        id: "unarmed-strike".to_string(),
        name: "Unarmed Strike".to_string(),
        kind: AttackKind::Melee,
        ability: Some(ability),
        to_hit: ExprValue::Number(i64::from(to_hit)),
        damage: vec![DamagePart {
            dice: damage,
            damage_type: rule
                .damage_type
                .unwrap_or_else(|| DamageType("bludgeoning".to_string())),
        }],
        reach: rule.reach.unwrap_or(5),
        range: None,
        properties: Vec::new(),
        mastery: None,
        reroll_damage_below: None,
        damage_die_min: None,
        damage_modifier: 0,
        on_hit: Vec::new(),
        on_miss: Vec::new(),
        tags: Vec::new(),
    })
}

/// The Unarmed Strike attack action (both editions).
pub fn attack_unarmed(
    state: &mut EncounterState,
    attacker_name: &str,
    target_name: &str,
    options: &UnarmedOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let attacker = state
        .find_index(attacker_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, attacker)?;
    ensure_targetable(state, target)?;
    ensure_not_charmed_target(state, attacker, target, true)?;
    if attacker == target {
        bail!("a creature cannot attack itself");
    }
    if !hostile_to(state, attacker, target) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target].name,
            state.combatants[attacker].name
        );
    }
    let profile = unarmed_profile(state, attacker)?;
    ensure_melee_reachable(state, attacker, target, profile.reach)?;
    let remaining = begin_attack_slots(state, attacker)?;
    let attacks = if options.one {
        1.min(remaining)
    } else {
        remaining
    };
    consume_attack_slots(state, attacker, attacks);
    if options.one && remaining > 1 {
        log.log(format_args!(
            "{} uses one Unarmed Strike; {} Attack action attack(s) remain",
            state.combatants[attacker].name,
            remaining - 1
        ));
    }

    if attacks > 1 {
        log.log(format_args!(
            "{}'s Extra Attack: {attacks} Unarmed Strikes",
            state.combatants[attacker].name
        ));
    }
    let attack_options = AttackOptions {
        advantage: options.advantage,
        disadvantage: options.disadvantage,
        multi: false,
        no_reactions: options.no_reactions,
        nonlethal: options.nonlethal,
    };
    for attack_number in 1..=attacks {
        if state.combatants[target].is_dead() {
            log.log(format_args!(
                "{} is already down; remaining attacks are skipped",
                state.combatants[target].name
            ));
            break;
        }
        if attacks > 1 {
            log.log(format_args!("attack {attack_number}/{attacks}"));
        }
        resolve_attack(
            state,
            attacker,
            target,
            &profile,
            None,
            &attack_options,
            rng,
            log,
            true,
            false,
        )?;
    }
    log_outcome(state, log);
    Ok(())
}

/// The Two-Weapon Fighting bonus-action attack.
pub fn attack_offhand(
    state: &mut EncounterState,
    attacker_name: &str,
    target_name: &str,
    attack_name: &str,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    attack_offhand_with_options(
        state,
        attacker_name,
        target_name,
        attack_name,
        &AttackOptions::default(),
        rng,
        log,
    )
}

/// [`attack_offhand`] with explicit attack options (advantage, non-lethal).
pub fn attack_offhand_with_options(
    state: &mut EncounterState,
    attacker_name: &str,
    target_name: &str,
    attack_name: &str,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let attacker = state
        .find_index(attacker_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let target = state
        .find_index(target_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, attacker)?;
    ensure_targetable(state, target)?;
    ensure_not_charmed_target(state, attacker, target, true)?;
    if attacker == target {
        bail!("a creature cannot attack itself");
    }
    if !hostile_to(state, attacker, target) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target].name,
            state.combatants[attacker].name
        );
    }
    let attack = state.combatants[attacker]
        .find_attack(attack_name)
        .with_context(|| {
            format!(
                "`{}` cannot attack with `{attack_name}`",
                state.combatants[attacker].name
            )
        })?
        .clone();
    let mut profile = AttackProfile::from_attack(&attack);
    if !matches!(profile.kind, AttackKind::Melee | AttackKind::MeleeOrRanged) {
        bail!("off-hand attack `{}` must be a melee weapon", profile.name);
    }
    let rule = two_weapon_rule(state)?.clone();
    if rule.require_light.unwrap_or(true) && !profile.properties.contains(&AttackProperty::Light) {
        bail!("off-hand attack `{}` is not a Light weapon", profile.name);
    }
    if state.combatants[attacker].bonus_action_used {
        bail!(
            "`{}` has already used a bonus action this turn",
            state.combatants[attacker].name
        );
    }
    let marker = "turn:combat-actions:light-main:";
    let main_hand = state.combatants[attacker]
        .once_used
        .iter()
        .find(|(key, round)| key.starts_with(marker) && **round == state.round)
        .map(|(key, _)| key[marker.len()..].to_string());
    let Some(main_hand) = main_hand else {
        bail!(
            "`{}` must first attack with a Light melee weapon using the Attack action before an off-hand attack",
            state.combatants[attacker].name
        );
    };
    if main_hand.eq_ignore_ascii_case(&profile.id) {
        bail!(
            "off-hand attack needs a different Light melee weapon; `{}` was already used in the main hand",
            profile.name
        );
    }
    ensure_melee_reachable(state, attacker, target, profile.reach)?;
    spend_action(state, attacker, Activation::Bonus, log)?;
    log.log(format_args!(
        "{} uses a bonus action for an off-hand attack with {}",
        state.combatants[attacker].name, profile.name
    ));
    let modifier = profile.attack_modifier(&state.combatants[attacker]);
    if modifier > 0 && !rule.add_positive_ability_modifier.unwrap_or(false) {
        profile.damage_modifier = -modifier;
    }
    resolve_attack(
        state, attacker, target, &profile, None, options, rng, log, true, false,
    )?;
    log_outcome(state, log);
    Ok(())
}

fn has_dark_sense(combatant: &CombatantState) -> bool {
    combatant.creature.senses.as_deref().is_some_and(|senses| {
        let senses = senses.to_ascii_lowercase();
        senses.contains("darkvision") || senses.contains("blindsight")
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_attack(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    profile: &AttackProfile,
    source: Option<&EffectSource>,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
    economy_spent: bool,
    ignore_range: bool,
) -> Result<()> {
    let wants_nonlethal = options.nonlethal
        || state.combatants[attacker]
            .creature
            .tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| tag == "nonlethal"));
    let melee = matches!(profile.kind, AttackKind::Melee | AttackKind::MeleeOrRanged)
        && attacker_in_melee_reach(state, attacker, target, profile);
    if options.nonlethal && !melee {
        log.log(format_args!(
            "note: non-lethal ignored for `{}`: only melee attacks can knock out (2024 PHB)",
            profile.name
        ));
    }
    let previous = state.nonlethal_active;
    state.nonlethal_active = wants_nonlethal && melee;
    let result = resolve_attack_inner(
        state,
        attacker,
        target,
        profile,
        source,
        options,
        rng,
        log,
        economy_spent,
        ignore_range,
    );
    state.nonlethal_active = previous;
    result?;
    run_ready_triggers(state, ReadyTrigger::Attacked, attacker, rng, log);
    Ok(())
}

fn attacker_in_melee_reach(
    state: &EncounterState,
    attacker: usize,
    target: usize,
    profile: &AttackProfile,
) -> bool {
    if profile.kind == AttackKind::Melee {
        return true;
    }
    if spatial_query_enabled(state) {
        if let Some(reachable) = spatial_can_reach(state, attacker, target, profile.reach) {
            return reachable;
        }
    }
    let Some(space) = state.space() else {
        return true;
    };
    let from = state.combatants[attacker].position_ref();
    let to = state.combatants[target].position_ref();
    space.can_reach(from.as_ref(), to.as_ref(), profile.reach)
}

#[allow(clippy::too_many_arguments)]
fn resolve_attack_inner(
    state: &mut EncounterState,
    attacker: usize,
    target: usize,
    profile: &AttackProfile,
    source: Option<&EffectSource>,
    options: &AttackOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
    economy_spent: bool,
    ignore_range: bool,
) -> Result<()> {
    if economy_spent {
        // Caller already spent the action (multiattack / ability effect).
    }
    ensure_can_act(state, attacker)?;
    ensure_targetable(state, target)?;

    let attacker_name = state.combatants[attacker].name.clone();
    let target_name = state.combatants[target].name.clone();
    // Collect both flags from every source, then resolve once: any advantage
    // plus any disadvantage cancels to a normal roll.
    let mut has_advantage = options.advantage;
    let mut has_disadvantage = options.disadvantage;
    let mut advantage_sources = Vec::new();
    let mut disadvantage_sources = Vec::new();
    if options.advantage {
        advantage_sources.push("command".to_string());
    }
    if options.disadvantage {
        disadvantage_sources.push("command".to_string());
    }

    let facts = profile.facts();
    let fast_path = source.is_none()
        && !has_modifier_sources(state, attacker)
        && !has_modifier_sources(state, target);
    let attack_ability = profile
        .ability
        .or_else(|| ability_name_from_attack_kind(profile.kind, &state.combatants[attacker]));
    let attacker_context = PredicateContext {
        self_idx: attacker,
        actor: attacker,
        target: Some(target),
        attacker: Some(attacker),
        linked: None,
        source: None,
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
        source: None,
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

    if !fast_path {
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
                    has_advantage = true;
                }
                ModifierEffect::Disadvantage => {
                    disadvantage_sources.push(collected.label.clone());
                    has_disadvantage = true;
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
                    has_advantage = true;
                }
                ModifierEffect::Disadvantage => {
                    disadvantage_sources.push(collected.label.clone());
                    has_disadvantage = true;
                }
                _ => {}
            }
        }
    }

    // Range, reach and light.
    let space = state
        .space()
        .ok_or_else(|| anyhow!("this state has no scene"))?;
    let attacker_position = state.combatants[attacker].position_ref();
    let target_position = state.combatants[target].position_ref();
    let (attacker_position, target_position) =
        (attacker_position.as_ref(), target_position.as_ref());
    let spatial = spatial_query_enabled(state);
    let distance = if spatial {
        match spatial_body_distance(state, attacker, target) {
            Some(distance) => distance,
            None => space
                .distance(attacker_position, target_position)
                .ok_or_else(|| anyhow!("no path between the combatants"))?,
        }
    } else {
        space
            .distance(attacker_position, target_position)
            .ok_or_else(|| anyhow!("no path between the combatants"))?
    };
    let in_reach = if spatial {
        spatial_can_reach(state, attacker, target, profile.reach)
            .unwrap_or_else(|| space.can_reach(attacker_position, target_position, profile.reach))
    } else {
        space.can_reach(attacker_position, target_position, profile.reach)
    };
    if !ignore_range {
        let mut kind = profile.kind;
        if kind == AttackKind::MeleeOrRanged {
            kind = if in_reach {
                AttackKind::Melee
            } else {
                AttackKind::Ranged
            };
        }
        match kind {
            AttackKind::Melee => {
                if !in_reach {
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
                    has_disadvantage = true;
                }
                if hostile_within_5(state, attacker) {
                    disadvantage_sources.push("ranged in melee".to_string());
                    has_disadvantage = true;
                }
            }
        }
        if spatial {
            ensure_spatial_line_of_sight(state, attacker, target)?;
        }
    }
    if space.light_at(target_position) == Light::Dark
        && !has_dark_sense(&state.combatants[attacker])
    {
        disadvantage_sources.push("darkness".to_string());
        has_disadvantage = true;
    }
    if space.light_at(attacker_position) == Light::Dark
        && !has_dark_sense(&state.combatants[target])
    {
        advantage_sources.push("unseen".to_string());
        has_advantage = true;
    }
    drop(space);

    // Attack roll bonuses.
    let mut bonuses = Vec::new();
    if !fast_path {
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
    }

    let (base_ac, ac_terms) = effective_ac(state, target, Some(attacker), &facts)?;
    let attack_mod = profile.attack_modifier(&state.combatants[attacker]);
    let context = source_context(state, attacker, source, Some(attack_mod), None);
    let base = if log.enabled() {
        let (_, to_hit_expr) = evaluate_value(&context, &profile.to_hit)
            .with_context(|| format!("attack `{}` to_hit", profile.name))?;
        to_hit_expr.modifier
    } else {
        evaluate_dice(&context, &profile.to_hit)
            .with_context(|| format!("attack `{}` to_hit", profile.name))?
            .modifier
    };
    let mode = RollMode::from_flags(has_advantage, has_disadvantage);
    let result = if log.enabled() {
        roll_d20(
            state,
            attacker,
            base,
            mode,
            bonuses,
            advantage_sources,
            disadvantage_sources,
            rng,
        )
    } else {
        roll_d20_quiet(
            state,
            attacker,
            base,
            mode,
            advantage_sources,
            disadvantage_sources,
            rng,
        )
    };
    let mut hit = result.nat20 || (!result.nat1 && result.total >= base_ac);
    let mut crit = result.nat20;
    let mut current_ac = base_ac;
    let mut current_terms = if log.enabled() {
        ac_terms.clone()
    } else {
        Vec::new()
    };
    if log.enabled() {
        log.log(format_args!(
            "{attacker_name} attacks {target_name} with {}: {} vs {} {}",
            profile.name,
            result.text(),
            ac_text(base_ac, &current_terms),
            if hit { "HIT" } else { "MISS" }
        ));
    }

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
                log.log(format_args!(
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
    }
    if !fast_path {
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
    }
    Ok(())
}

fn hostile_within_5(state: &EncounterState, index: usize) -> bool {
    let spatial = spatial_query_enabled(state);
    let Some(space) = state.space() else {
        return false;
    };
    let position = state.combatants[index].position_ref();
    state
        .combatants
        .iter()
        .enumerate()
        .any(|(other, combatant)| {
            other != index
                && combatant.is_active()
                && combatant.side != state.combatants[index].side
                && {
                    if spatial {
                        if let Some(distance) = spatial_body_distance(state, index, other) {
                            return distance <= 5;
                        }
                    }
                    let other_position = combatant.position_ref();
                    space
                        .distance(position.as_ref(), other_position.as_ref())
                        .is_some_and(|distance| distance <= 5)
                }
        })
}

#[allow(clippy::too_many_arguments)]
/// Whether a combatant has conditions or passive abilities that can emit
/// modifiers. AI uses this to take a cheap forecast path.
pub fn combatant_has_modifier_sources(state: &EncounterState, owner: usize) -> bool {
    has_modifier_sources(state, owner)
}

fn has_modifier_sources(state: &EncounterState, owner: usize) -> bool {
    if !state.combatants[owner].conditions.is_empty()
        || state.combatants[owner].exhaustion_level > 0
    {
        return true;
    }
    state.has_static_modifier_sources(owner)
}

/// Resolves only the owned abilities that declare a trigger for `when`.
fn trigger_abilities(
    state: &EncounterState,
    owner: usize,
    when: TriggerWhen,
) -> Vec<ResolvedAbility> {
    let combatant = &state.combatants[owner];
    let actions = combatant.creature.actions.as_deref().unwrap_or(&[]);
    let features = combatant.creature.features.as_deref().unwrap_or(&[]);
    let mut resolved = Vec::new();
    for entry in actions.iter().chain(features.iter()) {
        let matches = match entry {
            AbilityEntry::Inline(ability) => {
                ability.triggers.iter().any(|trigger| trigger.on == when)
            }
            AbilityEntry::Ref(reference) => state
                .content
                .abilities
                .get(&reference.reference)
                .is_some_and(|ability| ability.triggers.iter().any(|trigger| trigger.on == when)),
        };
        if matches {
            if let Ok(ability) = state.content.resolve_ability(entry) {
                resolved.push(ability);
            }
        }
    }
    resolved
}

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
    log: &mut dyn LogSink,
) -> Result<Option<(i32, Vec<(i32, String)>)>> {
    let abilities = trigger_abilities(state, target, TriggerWhen::OnHitTaken);
    if abilities.is_empty() {
        return Ok(None);
    }
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
                source: None,
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
            log.log(format_args!(
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

/// Spends one `spell-slot-N` from `owner` (used by `heal --slot`).
pub fn spend_spell_slot(
    state: &mut EncounterState,
    owner_name: &str,
    level: i32,
    log: &mut dyn LogSink,
) -> Result<()> {
    let owner = state
        .find_index(owner_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let cost = AbilityCost {
        resource: format!("spell-slot-{level}"),
        amount: 1,
    };
    if !spend_cost(state, owner, &cost, log)? {
        bail!(
            "`{}` has no level {level} spell slot left",
            state.combatants[owner].name
        );
    }
    Ok(())
}

fn spend_cost(
    state: &mut EncounterState,
    owner: usize,
    cost: &AbilityCost,
    log: &mut dyn LogSink,
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
    log.log(format_args!(
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
    log: &mut dyn LogSink,
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
    let reroll = context.attack.as_ref().and_then(|attack| {
        attack
            .reroll_damage_below
            .map(|n| (n, false))
            .or(attack.damage_die_min.map(|n| (n, true)))
    });
    let mut rolls = Vec::new();
    for part in parts {
        let record = if let Some((n, floor)) = reroll {
            let expression = evaluate_dice(&expr_context, &part.dice)?;
            let mut record = if floor {
                expression.roll_floored(n, context.crit, rng)
            } else {
                expression.roll_rerolling(n, context.crit, rng)
            };
            if !log.enabled() {
                record.text.clear();
            }
            record
        } else if log.enabled() {
            roll_value(&expr_context, &part.dice, context.crit, rng)?.1
        } else {
            let expression = evaluate_dice(&expr_context, &part.dice)?;
            let total = if context.crit {
                expression.roll_doubled_total(rng)
            } else {
                expression.roll_total(rng)
            };
            RollRecord {
                formula: String::new(),
                dice: Vec::new(),
                modifier: expression.modifier,
                total,
                text: String::new(),
            }
        };
        rolls.push((record, part.damage_type.as_str().to_string()));
    }
    if let Some(attack) = context.attack.as_ref() {
        if attack.damage_modifier != 0 {
            if let Some((record, _)) = rolls.first_mut() {
                record.total += attack.damage_modifier;
                if log.enabled() {
                    record
                        .text
                        .push_str(&format!(" off-hand {:+}", attack.damage_modifier));
                }
            }
        }
    }
    if let Some(first) = parts.first() {
        if context.source.is_some() || has_modifier_sources(state, context.actor) {
            if let Some(bonus) = damage_roll_bonus(
                state,
                context.actor,
                context.source.as_ref(),
                Some(first.damage_type.as_str()),
                rng,
            )? {
                if let Some((record, _)) = rolls.first_mut() {
                    record.total += bonus.record.total;
                    if log.enabled() {
                        record.text.push_str(&format!(" +{}", bonus.record.total));
                    }
                }
            }
        }
    }
    let tags = context
        .source
        .as_ref()
        .map(|source| source.tags.clone())
        .unwrap_or_default();
    apply_damage_rolls(
        state,
        target,
        context.actor,
        rolls,
        &tags,
        context.crit,
        rng,
        log,
    )
}

fn run_mastery_triggers(
    state: &mut EncounterState,
    attacker: usize,
    _target: usize,
    profile: &AttackProfile,
    base_context: &EffectContext,
    when: TriggerWhen,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
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
        source: None,
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
    log: &mut dyn LogSink,
) -> Result<()> {
    let abilities = trigger_abilities(state, owner, when);
    if abilities.is_empty() {
        return Ok(());
    }
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
    log: &mut dyn LogSink,
) {
    let abilities = trigger_abilities(state, owner, when);
    if abilities.is_empty() {
        return;
    }
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
                source: None,
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

fn fire_ends_on(state: &mut EncounterState, owner: usize, event: EndsOn, log: &mut dyn LogSink) {
    let name = state.combatants[owner].name.clone();
    let exhaustion_id = exhaustion_condition_id(state).to_string();
    let mut kept = Vec::new();
    let mut concentration_broken = false;
    let mut exhaustion_ended = false;
    for instance in state.combatants[owner].conditions.drain(..) {
        let ends = instance.ends_on.contains(&event)
            || state
                .content
                .conditions
                .get(&instance.condition)
                .is_some_and(|def| def.ends_on.contains(&event));
        if ends {
            log.log(format_args!("{name} is no longer {}", instance.condition));
            if instance.condition.eq_ignore_ascii_case(&exhaustion_id) {
                exhaustion_ended = true;
            }
            if instance.concentration {
                concentration_broken = true;
            }
        } else {
            kept.push(instance);
        }
    }
    state.combatants[owner].conditions = kept;
    if exhaustion_ended {
        let previous = state.combatants[owner].exhaustion_level;
        state.combatants[owner].exhaustion_level = 0;
        log.log(format_args!(
            "{name}'s exhaustion ends (level {previous} -> 0)"
        ));
    }
    if concentration_broken {
        break_concentration(state, owner, log);
    }
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
    log: &mut dyn LogSink,
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
    log: &mut dyn LogSink,
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
                nonlethal: state.nonlethal_request,
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
                false,
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
                None,
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
            remove_condition_from(state, target, &effect.condition, "removed by effect", log);
            Ok(())
        }
        Effect::ForcedMove(effect) => apply_forced_move(state, context, effect, rng, log),
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
                        false,
                    );
                }
            }
            let source = EffectSource::from_ability(&resolved.ability, &resolved.params);
            let targets: Vec<usize> = context.target.into_iter().collect();
            let zone_center = context.target.map(|target| AreaCenter {
                position: state.combatants[target].position(),
                creature: Some(target),
            });
            use_resolved_ability(
                state,
                context.actor,
                &resolved,
                &source,
                &targets,
                zone_center.as_ref(),
                &AttackOptions::default(),
                context.option,
                context.source.as_ref().and_then(|source| source.slot_level),
                AbilitySettlement {
                    action_spent: true,
                    ..AbilitySettlement::default()
                },
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
            log.log(format_args!("MANUAL: {text}"));
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
            extra_attacks: None,
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
    log: &mut dyn LogSink,
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
    let (mode, mut bonuses, adv_sources, dis_sources, auto_fail) =
        save_modifiers(state, target, effect.ability, &expr_context, rng)?;
    if effect.ability == AbilityName::Dex && !effect.ignore_cover && target != context.actor {
        let cover = cover_between(state, context.actor, target);
        if let Some(bonus) = cover_bonus(cover, &state.content.rules) {
            if bonus != 0 {
                bonuses.push(flat_bonus_term(bonus, cover_label(cover)));
            }
        }
    }
    if auto_fail {
        log.log(format_args!(
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
    log.log(format_args!(
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
    state: &mut EncounterState,
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
        source: None,
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
    let mut has_advantage = false;
    let mut has_disadvantage = false;
    let mut advantages = Vec::new();
    let mut disadvantages = Vec::new();
    let mut bonuses = Vec::new();
    let mut auto_fail = false;
    let collected_modifiers =
        collect_modifiers(state, target, ModifierApplies::Save, &context, None);
    for collected in &collected_modifiers {
        match collected.modifier.effect {
            ModifierEffect::Advantage => {
                advantages.push(collected.label.clone());
                has_advantage = true;
            }
            ModifierEffect::Disadvantage => {
                disadvantages.push(collected.label.clone());
                has_disadvantage = true;
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
                    label: collected.label.clone(),
                    record: expression.roll(rng),
                });
            }
            _ => {}
        }
    }
    finish_collected_modifiers(
        state,
        target,
        ModifierApplies::Save,
        &context,
        None,
        &mut Vec::new(),
    );
    let mode = RollMode::from_flags(has_advantage, has_disadvantage);
    Ok((mode, bonuses, advantages, disadvantages, auto_fail))
}

fn apply_damage_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &DamageEffect,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
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
    let crit = context.crit && effect.crit != CritMode::Never;
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
        if log.enabled() {
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
        } else {
            let expression = evaluate_dice(&expr_context, &effect.dice)?;
            let rolled = if crit {
                expression.roll_doubled_total(rng)
            } else {
                expression.roll_total(rng)
            };
            let damage = ((rolled as f64) * context.damage_scale).floor() as i32;
            rolls.push((
                RollRecord {
                    formula: String::new(),
                    dice: Vec::new(),
                    modifier: expression.modifier,
                    total: damage,
                    text: String::new(),
                },
                damage_type.clone(),
            ));
        }
    }
    if context.source.is_some() || has_modifier_sources(state, context.actor) {
        if let Some(bonus) = damage_roll_bonus(
            state,
            context.actor,
            context.source.as_ref(),
            Some(&damage_type),
            rng,
        )? {
            if let Some((record, _)) = rolls.first_mut() {
                record.total += bonus.record.total;
                if log.enabled() {
                    record.text.push_str(&format!(" +{}", bonus.record.total));
                }
            }
        }
    }
    let tags = context
        .source
        .as_ref()
        .map(|source| source.tags.clone())
        .unwrap_or_default();
    let dealt = apply_damage_rolls(state, target, context.actor, rolls, &tags, crit, rng, log)?;
    if dealt > 0 {
        fire_ends_on(state, context.actor, EndsOn::DealsDamage, log);
    }
    Ok(())
}

fn apply_heal_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &crate::content::schema::HealEffect,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
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
        source: None,
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
    let max_hp = effective_max_hp(state, target);
    state.combatants[target].hp = (before + amount).min(max_hp);
    let healed = state.combatants[target].hp - before;
    log.log(format_args!(
        "{name} regains {healed} HP ({} {})",
        record.text,
        terms.join(" ")
    ));
    if before == 0 && state.combatants[target].hp > 0 && !state.combatants[target].is_dead() {
        revive_from_zero(state, target, log);
    }
    Ok(())
}

fn revive_from_zero(state: &mut EncounterState, target: usize, log: &mut dyn LogSink) {
    state.combatants[target].knocked_out = false;
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
            log.log(format_args!("{name} is no longer {condition}"));
        }
    }
    state.combatants[target].death_saves = DeathSaves::default();
}

fn apply_forced_move(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &ForcedMoveEffect,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let Some(target) = context.target else {
        bail!("forced_move needs a target");
    };
    if !check_size_limit(state.combatants[target].creature.size(), effect.max_size) {
        return Ok(());
    }
    if spatial_forced_move(state, target, context.actor, effect, rng, log)? {
        return Ok(());
    }
    let Some(space) = state.space() else {
        return Ok(());
    };
    let source = state.combatants[context.actor].position();
    let start = state.combatants[target].position();
    let max_distance = effect.distance.max(0);
    let steps: Vec<Position> = match space.mode() {
        SceneMode::Zones => {
            let mut candidates = Vec::new();
            for step in space.neighbours(&start) {
                if step.locked || step.edge_distance > max_distance {
                    continue;
                }
                let Some(distance) = space.distance(&source, &step.position) else {
                    continue;
                };
                candidates.push((step.position, distance));
            }
            let chosen = match effect.direction {
                Direction::Away => candidates.into_iter().max_by_key(|(_, distance)| *distance),
                Direction::Toward => candidates.into_iter().min_by_key(|(_, distance)| *distance),
            };
            chosen
                .map(|(position, _)| vec![position])
                .unwrap_or_default()
        }
        SceneMode::Grid => {
            let (Some((tx, ty)), Some((sx, sy))) = (start.cell_coords(), source.cell_coords())
            else {
                return Ok(());
            };
            let (dx, dy) = match effect.direction {
                Direction::Away => ((tx - sx).signum(), (ty - sy).signum()),
                Direction::Toward => ((sx - tx).signum(), (sy - ty).signum()),
            };
            if dx == 0 && dy == 0 {
                return Ok(());
            }
            let occupied: Vec<(i32, i32)> = state
                .combatants
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != target)
                .filter_map(|(_, combatant)| combatant.cell())
                .map(|cell| (cell[0], cell[1]))
                .collect();
            let cell_size = space
                .distance(&start, &Position::cell(tx + 1, ty))
                .unwrap_or(5);
            let max_steps = max_distance / cell_size.max(1);
            let mut positions = Vec::new();
            let (mut x, mut y) = (tx, ty);
            for _ in 0..max_steps {
                let (nx, ny) = (x + dx, y + dy);
                let next = Position::cell(nx, ny);
                if !space.contains(&next) {
                    break;
                }
                let adjacent = space
                    .neighbours(&Position::cell(x, y))
                    .iter()
                    .any(|step| step.position == next);
                if !adjacent || occupied.contains(&(nx, ny)) {
                    break;
                }
                positions.push(next);
                x = nx;
                y = ny;
            }
            positions
        }
    };
    drop(space);
    for position in steps {
        let before = state.combatants[target].position();
        let display = state
            .space()
            .and_then(|space| space.zone_of(&position))
            .unwrap_or_else(|| OPEN_ZONE.to_string());
        match &position {
            Position::Cell { x, y } => state.combatants[target].set_cell(*x, *y, display.clone()),
            Position::Zone { zone } => state.combatants[target].set_zone(zone.clone()),
        }
        let name = state.combatants[target].name.clone();
        log.log(format_args!("{name} is moved to {display}"));
        fire_enter_triggers(state, target, &position, rng, log)?;
        run_enter_reach_triggers(state, target, &before, rng, log);
    }
    Ok(())
}

fn apply_check_effect(
    state: &mut EncounterState,
    context: &EffectContext,
    effect: &CheckEffect,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
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
    log.log(format_args!(
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
    state: &mut EncounterState,
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
        source: None,
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
    let mut has_advantage = false;
    let mut has_disadvantage = false;
    let mut advantages = Vec::new();
    let mut disadvantages = Vec::new();
    for collected in collect_modifiers(state, who, ModifierApplies::Check, &context, None) {
        match collected.modifier.effect {
            ModifierEffect::Advantage => {
                advantages.push(collected.label.clone());
                has_advantage = true;
            }
            ModifierEffect::Disadvantage => {
                disadvantages.push(collected.label.clone());
                has_disadvantage = true;
            }
            _ => {}
        }
    }
    finish_collected_modifiers(
        state,
        who,
        ModifierApplies::Check,
        &context,
        None,
        &mut Vec::new(),
    );
    (
        RollMode::from_flags(has_advantage, has_disadvantage),
        advantages,
        disadvantages,
    )
}

fn apply_basic_action(
    state: &mut EncounterState,
    context: &EffectContext,
    action: BasicAction,
    log: &mut dyn LogSink,
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
    log.log(format_args!("{name} takes the {key} action"));
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
            None,
            Vec::new(),
            None,
            false,
            &mut seeded_rng(0),
            log,
        )?;
    } else {
        log.log(format_args!("MANUAL: resolve the {key} action"));
    }
    Ok(())
}

fn toggle_scene_trigger(
    state: &mut EncounterState,
    effect: &ToggleTriggerEffect,
    log: &mut dyn LogSink,
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
    log.log(format_args!(
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
    until_subject: Option<String>,
    save_ends: Option<(AbilityName, i32, SaveEndsAt)>,
    ends_on: Vec<EndsOn>,
    created_by: Option<String>,
    concentration: bool,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let Some(def) = state.content.conditions.get(condition_id) else {
        bail!("unknown condition `{condition_id}`");
    };
    let target_name = state.combatants[target].name.clone();
    if state.combatants[target].is_immune_to_condition(condition_id) {
        log.log(format_args!("{target_name} is immune to {condition_id}"));
        return Ok(());
    }
    let def = def.clone();
    let source_name = source.map(|index| state.combatants[index].name.clone());
    let duration = duration.or_else(|| def.duration_default.clone());
    let active_duration = duration.as_ref().map(|duration| {
        if let Some(until) = duration.until {
            let subject = until_subject.clone().unwrap_or_else(|| match until {
                DurationUntil::EndOfTargetNextTurn => target_name.clone(),
                DurationUntil::EndOfSourceNextTurn | DurationUntil::StartOfSourceNextTurn => {
                    source_name.clone().unwrap_or_else(|| target_name.clone())
                }
            });
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
    let mut detail = String::new();
    if let Some(source) = &source_name {
        detail.push_str(&format!("; source: {source}"));
    }
    if let Some(linked) = &linked {
        detail.push_str(&format!("; linked: {linked}"));
    }
    if let Some(behavior) = condition_behavior(state, condition_id) {
        if behavior.cannot_target_source {
            detail.push_str("; cannot attack or damage the source");
        }
        if behavior.cannot_move_closer_to_source {
            detail.push_str("; cannot willingly move closer to the source");
        }
        if behavior.ends_when_source_dies {
            detail.push_str("; ends when the source dies");
        }
        if behavior.hidden {
            detail.push_str("; hidden from sight");
        }
    }
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
        log.log(format_args!(
            "{target_name}'s {condition_id} is refreshed{detail}"
        ));
    } else {
        state.combatants[target].conditions.push(active);
        log.log(format_args!("{target_name} is now {condition_id}{detail}"));
    }
    let incapacitated = has_flag(state, target, ConditionFlag::Incapacitated);
    if incapacitated {
        clear_ready_if_any(state, target, "incapacitated", log);
    }
    if incapacitated && state.concentration.contains_key(&target_name) {
        break_concentration(state, target, log);
    }
    maybe_fall_after_condition(state, target, rng, log)?;
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
    reason: &str,
    log: &mut dyn LogSink,
) {
    let name = state.combatants[target].name.clone();
    let before = state.combatants[target].conditions.len();
    let was_exhaustion = condition_id.eq_ignore_ascii_case(exhaustion_condition_id(state));
    state.combatants[target]
        .conditions
        .retain(|condition| !condition.condition.eq_ignore_ascii_case(condition_id));
    if state.combatants[target].conditions.len() != before {
        log.log(format_args!(
            "{name} is no longer {condition_id} ({reason})"
        ));
        if was_exhaustion {
            let previous = state.combatants[target].exhaustion_level;
            state.combatants[target].exhaustion_level = 0;
            log.log(format_args!(
                "{name}'s exhaustion ends (level {previous} -> 0)"
            ));
        }
        let concentration_condition = state.content.rules.concentration_condition.clone();
        if concentration_condition
            .as_deref()
            .is_some_and(|condition| condition.eq_ignore_ascii_case(condition_id))
        {
            break_concentration(state, target, log);
        }
    }
}

fn exhaustion_effect_summary(state: &EncounterState, level: u32) -> String {
    let Some(rules) = state.content.rules.exhaustion.as_ref() else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(from) = rules.check_disadvantage_from {
        if level >= from {
            parts.push("ability-check disadvantage".to_string());
        }
    }
    if let Some(from) = rules.speed_halved_from {
        if level >= from {
            parts.push("Speed halved".to_string());
        }
    }
    if let Some(from) = rules.attack_save_disadvantage_from {
        if level >= from {
            parts.push("attack/save disadvantage".to_string());
        }
    }
    if let Some(from) = rules.hp_max_halved_from {
        if level >= from {
            parts.push("HP maximum halved".to_string());
        }
    }
    if let Some(from) = rules.speed_zero_from {
        if level >= from {
            parts.push("Speed 0".to_string());
        }
    }
    if let Some(penalty) = rules.d20_penalty_per_level {
        parts.push(format!(
            "{} to d20 tests",
            signed(-(penalty.max(0).saturating_mul(level as i32)))
        ));
    }
    if let Some(penalty) = rules.speed_penalty_per_level {
        parts.push(format!(
            "{} ft Speed",
            signed(-(penalty.max(0).saturating_mul(level as i32)))
        ));
    }
    parts.join(", ")
}

/// Set or clear the additive exhaustion level. The marker condition is kept in
/// sync so existing modifier/status paths see it.
fn set_exhaustion_level(
    state: &mut EncounterState,
    index: usize,
    level: u32,
    reason: &str,
    log: &mut dyn LogSink,
) -> Result<()> {
    let rules = state.content.rules.exhaustion.clone().unwrap_or_default();
    let max = rules.max_level();
    if level > max {
        bail!("exhaustion level {level} is outside 0..={max} for this ruleset");
    }
    let condition_id = rules.condition_id().to_string();
    let name = state.combatants[index].name.clone();
    let previous = state.combatants[index].exhaustion_level;
    if level == 0 {
        state.combatants[index].exhaustion_level = 0;
        let before = state.combatants[index].conditions.len();
        state.combatants[index]
            .conditions
            .retain(|condition| !condition.condition.eq_ignore_ascii_case(&condition_id));
        if state.combatants[index].conditions.len() != before {
            log.log(format_args!(
                "{name} is no longer {condition_id} (level {previous} -> 0; {reason})"
            ));
        }
        return Ok(());
    }
    if !state.combatants[index].has_condition(&state.content, &condition_id) {
        let mut rng = seeded_rng(0);
        apply_condition_internal(
            state,
            index,
            &condition_id,
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
            None,
            Vec::new(),
            None,
            false,
            &mut rng,
            log,
        )?;
    }
    let rules = state.content.rules.exhaustion.clone().unwrap_or_default();
    let already_dead = state.combatants[index].is_dead();
    state.combatants[index].exhaustion_level = level;
    if level >= rules.death_level() {
        state.combatants[index].hp = 0;
        state.combatants[index].dead = true;
        if state.combatants[index].makes_death_saves() {
            state.combatants[index].death_saves.dead = true;
        }
        if !already_dead {
            log.log(format_args!("{name} dies from exhaustion (level {level})"));
            on_death(state, index, log);
        }
        return Ok(());
    }
    let max_hp = effective_max_hp(state, index);
    if state.combatants[index].hp > max_hp {
        state.combatants[index].hp = max_hp;
        log.log(format_args!(
            "{name}'s current HP is capped at the effective maximum {max_hp}"
        ));
    }
    let summary = exhaustion_effect_summary(state, level);
    if summary.is_empty() {
        log.log(format_args!("{name}'s exhaustion is now level {level}"));
    } else {
        log.log(format_args!(
            "{name}'s exhaustion is now level {level} ({summary})"
        ));
    }
    Ok(())
}

/// CLI `condition add`.
pub fn add_condition(
    state: &mut EncounterState,
    who: &str,
    condition_id: &str,
    options: &AddConditionOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let target = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let source_sensitive = condition_behavior(state, condition_id).is_some_and(|behavior| {
        behavior.cannot_target_source
            || behavior.cannot_move_closer_to_source
            || behavior.ends_when_source_dies
    });
    if source_sensitive {
        for (label, name) in [
            ("source", options.source.as_deref()),
            ("linked", options.linked.as_deref()),
        ] {
            if let Some(name) = name {
                state.find_index(name).map_err(|_| {
                    anyhow!(
                        "unknown or ambiguous {label} `{name}` for `{condition_id}`; it must name exactly one existing combatant"
                    )
                })?;
            }
        }
    }
    let exhaustion_id = exhaustion_condition_id(state).to_string();
    if condition_id.eq_ignore_ascii_case(&exhaustion_id) {
        if let Some(level) = options.level {
            if level == 0 {
                bail!("exhaustion level must be between 1 and the ruleset maximum");
            }
            set_exhaustion_level(state, target, level, "set by command", log)?;
            return Ok(());
        }
        // No --level keeps the legacy manual-marker behavior: add/refresh the
        // condition without changing the exhaustion level.
    } else if options.level.is_some() {
        bail!("`level` only applies to exhaustion, not `{condition_id}`");
    }
    if options.rounds.is_none()
        && options.minutes.is_none()
        && options.until.is_none()
        && !options.permanent
        && state.content.conditions.get(condition_id).is_none()
    {
        bail!("unknown condition `{condition_id}`");
    }
    let until_subject = if options.until.is_some() {
        options
            .until_subject
            .as_deref()
            .map(|name| {
                state
                    .find_index(name)
                    .map(|index| state.combatants[index].name.clone())
                    .map_err(|error| anyhow!(error.to_string()))
            })
            .transpose()?
    } else {
        None
    };
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
        until_subject,
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
    log: &mut dyn LogSink,
) -> Result<()> {
    let target = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    if condition_id.eq_ignore_ascii_case(exhaustion_condition_id(state))
        && state.combatants[target].exhaustion_level > 0
    {
        return set_exhaustion_level(state, target, 0, "removed by command", log);
    }
    remove_condition_from(state, target, condition_id, "removed by command", log);
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
    zone_center: Option<&AreaCenter>,
    options: &AttackOptions,
    option: Option<usize>,
    slot_level: Option<i32>,
    settlement: AbilitySettlement,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let ability = &resolved.ability;
    ensure_can_act(state, actor)?;
    let target_spec = ability.target.clone().unwrap_or_default();
    let mut effective_targets: Vec<usize> = targets.to_vec();
    match target_spec.target_type {
        TargetType::Zone => {
            let center = zone_center.cloned().unwrap_or_else(|| {
                if let Some(target) = targets.first().copied() {
                    AreaCenter {
                        position: state.combatants[target].position(),
                        creature: Some(target),
                    }
                } else {
                    AreaCenter {
                        position: state.combatants[actor].position(),
                        creature: Some(actor),
                    }
                }
            });
            let center_position = center.position.clone();
            if let (Some(range), Some(space)) = (ability.range, state.space()) {
                if range > 0 {
                    let spatial_distance = if spatial_query_enabled(state) {
                        match center.creature {
                            Some(target) => spatial_body_distance(state, actor, target),
                            None => spatial_distance_to_position(state, actor, &center_position),
                        }
                    } else {
                        None
                    };
                    let distance = spatial_distance.unwrap_or_else(|| {
                        space
                            .distance(&state.combatants[actor].position(), &center_position)
                            .unwrap_or(i32::MAX)
                    });
                    if distance > range {
                        bail!(
                            "area centre is out of range for {} ({distance} ft; range {range})",
                            ability.display_name()
                        );
                    }
                }
            }
            effective_targets = area_targets(
                state,
                &center_position,
                center.creature,
                ability.area.as_ref(),
            )?;
        }
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
                if *target != actor {
                    if let (Some(range), Some(space)) = (ability.range, state.space()) {
                        let spatial_distance = if spatial_query_enabled(state) {
                            spatial_body_distance(state, actor, *target)
                        } else {
                            None
                        };
                        let distance = spatial_distance.unwrap_or_else(|| {
                            space
                                .distance(
                                    &state.combatants[actor].position(),
                                    &state.combatants[*target].position(),
                                )
                                .unwrap_or(i32::MAX)
                        });
                        if distance > range {
                            bail!(
                                "`{}` is out of range for {} ({distance} ft; range {range})",
                                state.combatants[*target].name,
                                ability.display_name()
                            );
                        }
                    }
                }
                if !ability_ignores_cover(ability) {
                    ensure_cover_allows(state, actor, *target)?;
                }
                ensure_targetable(state, *target)?;
            }
        }
    }
    if let Some(cost) = &ability.cost {
        if !settlement.cost_paid && !can_afford(state, actor, cost) {
            bail!(
                "`{}` cannot afford {} {}",
                state.combatants[actor].name,
                cost.amount,
                cost.resource
            );
        }
    }
    if !settlement.action_spent {
        spend_action(state, actor, ability.activation, log)?;
    }
    if let Some(cost) = &ability.cost {
        if !settlement.cost_paid {
            let _ = spend_cost(state, actor, cost, log)?;
        }
    }
    if ability.concentration && !settlement.concentration_started {
        begin_concentration(state, actor, ability, log);
    }
    if let Some(manual) = &ability.manual {
        log.log(format_args!("MANUAL: {manual}"));
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
    log.log(format_args!(
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
            for target in &effective_targets {
                let context = EffectContext {
                    target: Some(*target),
                    ..context_base.clone()
                };
                apply_effects(state, &context, &ability.effects, rng, log)?;
            }
        }
        TargetType::Zone => {
            for target in &effective_targets {
                let context = EffectContext {
                    target: Some(*target),
                    ..context_base.clone()
                };
                apply_effects(state, &context, &ability.effects, rng, log)?;
            }
        }
    }
    if source_is_spell {
        fire_ends_on(state, actor, EndsOn::OwnSpell, log);
    }
    let _ = options;
    run_ready_triggers(state, ReadyTrigger::Ability, actor, rng, log);
    Ok(())
}

fn ability_ignores_cover(ability: &Ability) -> bool {
    ability.effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::Save(save) if save.ignore_cover
        )
    })
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

/// Every non-dormant, living creature inside an area.
fn area_targets(
    state: &EncounterState,
    center: &Position,
    center_creature: Option<usize>,
    area_spec: Option<&crate::content::schema::AreaSpec>,
) -> Result<Vec<usize>> {
    let radius = area_spec.map(|spec| spec.radius).unwrap_or(0);
    if spatial_query_enabled(state)
        && radius > 0
        && state
            .space()
            .is_some_and(|space| space.mode() == SceneMode::Grid)
    {
        if let Some(origin_pose) = spatial_pose_for_position(state, center, center_creature) {
            let profile = state.effective_spatial_profile();
            let spec = AreaSpecV1 {
                shape: area_spec
                    .and_then(|spec| spec.shape)
                    .unwrap_or(AreaShape::Sphere),
                size_ft: radius,
                width_ft: area_spec.and_then(|spec| spec.width_ft),
            };
            let area = crate::space::area_cells(&spec, &origin_pose, &profile)
                .map_err(|error| anyhow!("area shape {:?}: {error}", spec.shape))?;
            let origin = SpatialBody::new("area-origin", origin_pose, Size::Medium);
            let blockers = spatial_blockers(state, &profile);
            return Ok(state
                .combatants
                .iter()
                .enumerate()
                .filter(|(_, combatant)| !combatant.dormant && !combatant.is_dead())
                .filter(|(index, _)| {
                    spatial_body_for(state, *index).is_some_and(|target| {
                        crate::space::area_intersection(&area, &origin, &target, &blockers)
                            .intersects
                    })
                })
                .map(|(index, _)| index)
                .collect());
        }
    }
    let Some(space) = state.space() else {
        return Ok(Vec::new());
    };
    let owned;
    let positions: &[Position] = match space.area_slice(center, radius) {
        Some(positions) => positions,
        None => {
            owned = space.area_positions(center, radius);
            &owned
        }
    };
    Ok(state
        .combatants
        .iter()
        .enumerate()
        .filter(|(_, combatant)| !combatant.dormant && !combatant.is_dead())
        .filter(|(_, combatant)| positions.contains(&combatant.position()))
        .map(|(index, _)| index)
        .collect())
}

/// CLI `cast`/`use` for an owned ability.
pub fn use_ability(
    state: &mut EncounterState,
    actor_name: &str,
    ability_name: &str,
    options: &UseOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let actor = state
        .find_index(actor_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, actor)?;
    let resolved = state.combatants[actor].find_owned_ability(&state.content, ability_name)?;
    let source = EffectSource::from_ability(&resolved.ability, &resolved.params);
    let target_type = resolved
        .ability
        .target
        .clone()
        .unwrap_or_default()
        .target_type;
    let mut targets = Vec::new();
    let mut zone_center: Option<AreaCenter> = None;
    if target_type == TargetType::Zone {
        zone_center = options
            .targets
            .first()
            .map(|name| resolve_area_center(state, name))
            .transpose()?;
    } else {
        for name in &options.targets {
            let index = state
                .find_index(name)
                .map_err(|error| anyhow!(error.to_string()))?;
            targets.push(index);
        }
    }
    let damaging = effects_have_damage(&resolved.ability.effects);
    for target in &targets {
        ensure_not_charmed_target(state, actor, *target, damaging)?;
    }
    let attack_options = AttackOptions {
        advantage: options.advantage,
        disadvantage: options.disadvantage,
        multi: false,
        no_reactions: options.no_reactions,
        nonlethal: options.nonlethal,
    };
    use_resolved_ability(
        state,
        actor,
        &resolved,
        &source,
        &targets,
        zone_center.as_ref(),
        &attack_options,
        options.option,
        options.slot_level,
        AbilitySettlement {
            action_spent: false,
            ..AbilitySettlement::default()
        },
        rng,
        log,
    )?;
    log_outcome(state, log);
    Ok(())
}

// ---------------------------------------------------------------------------
// Ready actions (v1.5c)
// ---------------------------------------------------------------------------

/// Prepares one response. Setting Ready spends the holder's action; the
/// response itself spends the reaction when it triggers. Repeat, no-action,
/// incapacitated and no-reaction cases fail before any state change.
pub fn ready(
    state: &mut EncounterState,
    who: &str,
    options: &ReadyOptions,
    log: &mut dyn LogSink,
) -> Result<()> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, index)?;
    let name = state.combatants[index].name.clone();
    if state.combatants[index].ready.is_some() {
        bail!("`{name}` already has a readied action");
    }
    let max_responses = state
        .content
        .rules
        .ready
        .as_ref()
        .map(|rules| rules.max_responses())
        .unwrap_or(1);
    if max_responses != 1 {
        bail!("rules.ready.max_responses must be 1; got {max_responses}");
    }
    if state.combatants[index].action_used {
        bail!("`{name}` has already used an action this turn");
    }
    if has_flag(state, index, ConditionFlag::NoReactions) {
        bail!("`{name}` cannot use reactions");
    }
    let description = normalize_ready_description(&options.description)?;
    validate_ready_response(state, index, &options.response)?;
    let ready_ability = ready_ability(state, index, &options.response)?;

    // RAW: setting Ready casts/holds the spell. Pay its cost now, then spend
    // the action; a failed payment or action spend leaves no partial state.
    if let Some(resolved) = &ready_ability {
        if let Some(cost) = &resolved.ability.cost {
            if !spend_cost(state, index, cost, log)? {
                bail!(
                    "`{}` cannot afford {} {}",
                    state.combatants[index].name,
                    cost.amount,
                    cost.resource
                );
            }
        }
    }
    if let Err(error) = spend_action(state, index, Activation::Action, log) {
        if let Some(resolved) = &ready_ability {
            if let Some(cost) = &resolved.ability.cost {
                refund_cost(state, index, cost);
            }
        }
        return Err(error);
    }
    if let Some(resolved) = &ready_ability {
        if resolved.ability.concentration {
            begin_concentration(state, index, &resolved.ability, log);
        }
    }

    let expires_round = next_named_turn_round(state, &name);
    let summary = describe_ready_response(&options.response);
    state.combatants[index].ready = Some(ReadyState {
        trigger: options.trigger,
        description: description.clone(),
        response: options.response.clone(),
        set_round: state.round,
        expires_round,
    });
    log.log(format_args!(
        "{name} readies {summary} (trigger {}: {description})",
        options.trigger.as_str()
    ));
    Ok(())
}

fn ready_ability(
    state: &EncounterState,
    holder: usize,
    response: &ReadyResponse,
) -> Result<Option<ResolvedAbility>> {
    let ReadyResponse::Ability { ability, .. } = response else {
        return Ok(None);
    };
    Ok(Some(
        state.combatants[holder].find_owned_ability(&state.content, ability)?,
    ))
}

fn ready_ability_concentration_active(
    state: &EncounterState,
    holder: usize,
    ability: &str,
) -> bool {
    state
        .concentration
        .get(&state.combatants[holder].name)
        .and_then(|concentration| concentration.ability.as_deref())
        .is_some_and(|active| active.eq_ignore_ascii_case(ability))
}

fn refund_cost(state: &mut EncounterState, owner: usize, cost: &AbilityCost) {
    if let Some(resource) = state.combatants[owner].resources.get_mut(&cost.resource) {
        let amount = i32::try_from(cost.amount).unwrap_or(i32::MAX);
        resource.current = resource.current.saturating_add(amount).min(resource.max);
    }
}

/// Fires the holder's prepared response explicitly (DM trigger). A legal
/// refusal keeps the ready and spends no reaction; the reason is logged.
pub fn trigger_ready(
    state: &mut EncounterState,
    who: &str,
    source: Option<&str>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<ReadyOutcome> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let source = match source {
        Some(source) => Some(
            state
                .find_index(source)
                .map_err(|error| anyhow!(error.to_string()))?,
        ),
        None => None,
    };
    trigger_ready_at(state, index, source, None, rng, log)
}

/// Clears a prepared response without triggering it. Returns whether one was
/// present.
pub fn clear_ready(
    state: &mut EncounterState,
    who: &str,
    reason: &str,
    log: &mut dyn LogSink,
) -> Result<bool> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(clear_ready_if_any(state, index, reason, log))
}

fn clear_ready_if_any(
    state: &mut EncounterState,
    index: usize,
    reason: &str,
    log: &mut dyn LogSink,
) -> bool {
    let name = state.combatants[index].name.clone();
    if state.combatants[index].ready.is_some() {
        log.log(format_args!(
            "{name}'s readied action is cleared ({reason})"
        ));
        end_ready_concentration(state, index, reason, log);
        state.combatants[index].ready = None;
        true
    } else {
        false
    }
}

fn expire_ready(state: &mut EncounterState, index: usize, log: &mut dyn LogSink) -> bool {
    let name = state.combatants[index].name.clone();
    if state.combatants[index].ready.is_some() {
        log.log(format_args!(
            "{name}'s readied action expires (start of their next turn)"
        ));
        end_ready_concentration(state, index, "start of their next turn", log);
        state.combatants[index].ready = None;
        true
    } else {
        false
    }
}

/// Ends the concentration that a readied spell started, but only when the
/// holder is still concentrating on that exact ability. A replacement or a
/// different concentration is left untouched.
fn end_ready_concentration(
    state: &mut EncounterState,
    index: usize,
    reason: &str,
    log: &mut dyn LogSink,
) {
    let name = state.combatants[index].name.clone();
    let Some(ability) =
        state.combatants[index]
            .ready
            .as_ref()
            .and_then(|ready| match &ready.response {
                ReadyResponse::Ability { ability, .. } => Some(ability.clone()),
                _ => None,
            })
    else {
        return;
    };
    let Ok(resolved) = state.combatants[index].find_owned_ability(&state.content, &ability) else {
        return;
    };
    if !resolved.ability.concentration {
        return;
    }
    let active = state
        .concentration
        .get(&name)
        .and_then(|concentration| concentration.ability.as_deref())
        .is_some_and(|active| active.eq_ignore_ascii_case(&ability));
    if !active {
        return;
    }
    log.log(format_args!(
        "{name}'s readied spell concentration ends ({reason})"
    ));
    break_concentration(state, index, log);
}

fn normalize_ready_description(text: &str) -> Result<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        bail!("ready needs a non-empty --trigger description");
    }
    let mut bounded: String = collapsed.chars().take(160).collect();
    if collapsed.chars().count() > 160 {
        bounded.push('…');
    }
    Ok(bounded)
}

fn validate_ready_response(
    state: &EncounterState,
    holder: usize,
    response: &ReadyResponse,
) -> Result<()> {
    match response {
        ReadyResponse::Attack { attack, target } => {
            state.combatants[holder].find_attack(attack)?;
            validate_ready_attack_target(state, holder, target.as_deref())?;
        }
        ReadyResponse::Unarmed { target } => {
            unarmed_profile(state, holder)?;
            validate_ready_attack_target(state, holder, target.as_deref())?;
        }
        ReadyResponse::Ability { ability, target } => {
            let resolved = state.combatants[holder].find_owned_ability(&state.content, ability)?;
            if resolved.ability.activation != Activation::Action {
                bail!(
                    "`{ability}` is a {} ability; only an action can be readied",
                    activation_label(resolved.ability.activation)
                );
            }
            if let Some(cost) = &resolved.ability.cost {
                if !can_afford(state, holder, cost) {
                    bail!(
                        "`{}` cannot afford {} {}",
                        state.combatants[holder].name,
                        cost.amount,
                        cost.resource
                    );
                }
            }
            let target_type = resolved
                .ability
                .target
                .clone()
                .unwrap_or_default()
                .target_type;
            match target_type {
                TargetType::SelfTarget => {
                    if target.is_some() {
                        bail!("`{ability}` targets self and does not take --target");
                    }
                }
                TargetType::Zone => {
                    let label = target.as_deref().ok_or_else(|| {
                        anyhow!(
                            "readied area ability `{ability}` needs --target <zone|X,Y|creature>"
                        )
                    })?;
                    resolve_area_center(state, label)?;
                }
                TargetType::Creature => {
                    if let Some(target) = target {
                        state
                            .find_index(target)
                            .map_err(|error| anyhow!(error.to_string()))?;
                    }
                }
            }
        }
        ReadyResponse::Move { zone, cell } => {
            let target = ready_move_target(zone.as_deref(), *cell)?;
            let mut silent = Vec::new();
            if resolve_move_destination(state, holder, &target, true, &mut silent)?.is_none() {
                bail!("readied move destination is already the holder's position");
            }
        }
    }
    Ok(())
}

fn validate_ready_attack_target(
    state: &EncounterState,
    holder: usize,
    target: Option<&str>,
) -> Result<()> {
    let Some(target) = target else {
        return Ok(());
    };
    let target = state
        .find_index(target)
        .map_err(|error| anyhow!(error.to_string()))?;
    if target == holder {
        bail!("a creature cannot ready an attack against itself");
    }
    if !hostile_to(state, holder, target) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target].name,
            state.combatants[holder].name
        );
    }
    Ok(())
}

fn activation_label(activation: Activation) -> &'static str {
    match activation {
        Activation::Action => "action",
        Activation::Bonus => "bonus-action",
        Activation::Reaction => "reaction",
        Activation::Passive => "passive",
        Activation::Free => "free",
    }
}

fn describe_ready_response(response: &ReadyResponse) -> String {
    let target_suffix = |target: &Option<String>| {
        target
            .as_ref()
            .map(|target| format!(" against {target}"))
            .unwrap_or_default()
    };
    match response {
        ReadyResponse::Attack { attack, target } => {
            format!("weapon attack `{attack}`{}", target_suffix(target))
        }
        ReadyResponse::Unarmed { target } => {
            format!("unarmed strike{}", target_suffix(target))
        }
        ReadyResponse::Ability { ability, target } => {
            format!("ability `{ability}`{}", target_suffix(target))
        }
        ReadyResponse::Move {
            zone: Some(zone), ..
        } => format!("a move to zone `{zone}`"),
        ReadyResponse::Move {
            cell: Some([x, y]), ..
        } => format!("a move to cell ({x},{y})"),
        ReadyResponse::Move { .. } => "a move".to_string(),
    }
}

fn ready_move_target(zone: Option<&str>, cell: Option<[i32; 2]>) -> Result<MoveTarget> {
    match (zone, cell) {
        (Some(zone), None) => Ok(MoveTarget::Zone(zone.to_string())),
        (None, Some([x, y])) => Ok(MoveTarget::Cell(x, y)),
        _ => bail!("a readied move needs exactly one of --zone or --to"),
    }
}

fn trigger_ready_at(
    state: &mut EncounterState,
    index: usize,
    source: Option<usize>,
    automatic: Option<ReadyTrigger>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<ReadyOutcome> {
    let name = state.combatants[index].name.clone();
    let Some(ready) = state.combatants[index].ready.clone() else {
        bail!("`{name}` has no readied action");
    };
    if state.combatants[index].is_dead() {
        clear_ready_if_any(state, index, "dead", log);
        return Ok(ReadyOutcome::Expired);
    }
    if state.combatants[index].dormant {
        clear_ready_if_any(state, index, "no longer active", log);
        return Ok(ReadyOutcome::Expired);
    }
    if is_incapacitated(state, index) {
        clear_ready_if_any(state, index, "incapacitated", log);
        return Ok(ReadyOutcome::Expired);
    }
    if state.combatants[index].hp <= 0 {
        clear_ready_if_any(state, index, "0 HP", log);
        return Ok(ReadyOutcome::Expired);
    }
    if ready.expires_round > 0 && state.round > ready.expires_round {
        clear_ready_if_any(state, index, "expired", log);
        return Ok(ReadyOutcome::Expired);
    }
    if state.combatants[index].reaction_round == Some(state.round)
        || has_flag(state, index, ConditionFlag::NoReactions)
    {
        let reason = format!("`{name}` cannot use a reaction this round");
        log.log(format_args!("{name}'s readied action is refused: {reason}"));
        return Ok(ReadyOutcome::Refused(reason));
    }
    let trigger_label = automatic.map(ReadyTrigger::as_str).unwrap_or("explicit");
    log.log(format_args!(
        "{name}'s readied action triggers ({trigger_label}: {})",
        ready.description
    ));
    state.ready_in_progress = true;
    let result = execute_ready_response(state, index, &ready.response, source, rng, log);
    state.ready_in_progress = false;
    match result {
        Ok(()) => {
            state.combatants[index].reaction_round = Some(state.round);
            state.combatants[index].ready = None;
            log.log(format_args!(
                "{name} uses their reaction for the readied response"
            ));
            Ok(ReadyOutcome::Triggered)
        }
        Err(error) => {
            let reason = error.to_string();
            log.log(format_args!("{name}'s readied action is refused: {reason}"));
            Ok(ReadyOutcome::Refused(reason))
        }
    }
}

fn execute_ready_response(
    state: &mut EncounterState,
    holder: usize,
    response: &ReadyResponse,
    source: Option<usize>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    match response {
        ReadyResponse::Attack { attack, target } => {
            let target = resolve_ready_target(state, holder, target.as_deref(), source, "attack")?;
            ensure_ready_attack_target(state, holder, target)?;
            let profile = AttackProfile::from_attack(state.combatants[holder].find_attack(attack)?);
            resolve_attack(
                state,
                holder,
                target,
                &profile,
                None,
                &AttackOptions::default(),
                rng,
                log,
                true,
                false,
            )?;
            log_outcome(state, log);
        }
        ReadyResponse::Unarmed { target } => {
            let target = resolve_ready_target(state, holder, target.as_deref(), source, "attack")?;
            ensure_ready_attack_target(state, holder, target)?;
            let profile = unarmed_profile(state, holder)?;
            ensure_melee_reachable(state, holder, target, profile.reach)?;
            resolve_attack(
                state,
                holder,
                target,
                &profile,
                None,
                &AttackOptions::default(),
                rng,
                log,
                true,
                false,
            )?;
            log_outcome(state, log);
        }
        ReadyResponse::Ability { ability, target } => {
            let resolved = state.combatants[holder].find_owned_ability(&state.content, ability)?;
            if resolved.ability.activation != Activation::Action {
                bail!(
                    "`{ability}` is a {} ability; only an action can be readied",
                    activation_label(resolved.ability.activation)
                );
            }
            let effect_source = EffectSource::from_ability(&resolved.ability, &resolved.params);
            let target_type = resolved
                .ability
                .target
                .clone()
                .unwrap_or_default()
                .target_type;
            let mut targets = Vec::new();
            let mut zone_center = None;
            match target_type {
                TargetType::SelfTarget => {}
                TargetType::Zone => {
                    let label = target.as_deref().ok_or_else(|| {
                        anyhow!("readied area ability `{ability}` has no stored target")
                    })?;
                    zone_center = Some(resolve_area_center(state, label)?);
                }
                TargetType::Creature => {
                    let target =
                        resolve_ready_target(state, holder, target.as_deref(), source, "ability")?;
                    targets.push(target);
                }
            }
            let damaging = effects_have_damage(&resolved.ability.effects);
            for target in &targets {
                ensure_not_charmed_target(state, holder, *target, damaging)?;
            }
            if resolved.ability.concentration
                && !ready_ability_concentration_active(state, holder, ability)
            {
                bail!("the readied spell's concentration is no longer active");
            }
            use_resolved_ability(
                state,
                holder,
                &resolved,
                &effect_source,
                &targets,
                zone_center.as_ref(),
                &AttackOptions::default(),
                None,
                None,
                AbilitySettlement::prepaid(),
                rng,
                log,
            )?;
            log_outcome(state, log);
        }
        ReadyResponse::Move { zone, cell } => {
            let target = ready_move_target(zone.as_deref(), *cell)?;
            move_to_at(state, holder, &target, false, rng, log)?;
        }
    }
    Ok(())
}

fn resolve_ready_target(
    state: &EncounterState,
    holder: usize,
    stored: Option<&str>,
    source: Option<usize>,
    what: &str,
) -> Result<usize> {
    if let Some(target) = stored {
        return state
            .find_index(target)
            .map_err(|error| anyhow!(error.to_string()));
    }
    if let Some(source) = source {
        return Ok(source);
    }
    bail!(
        "readied {what} needs a target: name one at ready time with --target or pass --source to `trigger` (holder `{}`)",
        state.combatants[holder].name
    )
}

fn ensure_ready_attack_target(state: &EncounterState, holder: usize, target: usize) -> Result<()> {
    ensure_targetable(state, target)?;
    ensure_not_charmed_target(state, holder, target, true)?;
    if holder == target {
        bail!("a creature cannot attack itself");
    }
    if !hostile_to(state, holder, target) {
        bail!(
            "`{}` is not hostile to `{}`",
            state.combatants[target].name,
            state.combatants[holder].name
        );
    }
    Ok(())
}

/// Automatic triggers from an enemy attack, ability or turn start.
fn run_ready_triggers(
    state: &mut EncounterState,
    kind: ReadyTrigger,
    actor: usize,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) {
    if state.ready_in_progress {
        return;
    }
    let Some(actor_state) = state.combatants.get(actor) else {
        return;
    };
    if actor_state.is_dead() || actor_state.dormant {
        return;
    }
    let actor_side = actor_state.side.clone();
    let candidates: Vec<usize> = state
        .combatants
        .iter()
        .enumerate()
        .filter(|(index, combatant)| {
            *index != actor
                && combatant.side != actor_side
                && combatant
                    .ready
                    .as_ref()
                    .is_some_and(|ready| ready.trigger == kind)
        })
        .map(|(index, _)| index)
        .collect();
    for index in candidates {
        if state.combatants[index]
            .ready
            .as_ref()
            .is_none_or(|ready| ready.trigger != kind)
        {
            continue;
        }
        let _ = trigger_ready_at(state, index, Some(actor), Some(kind), rng, log);
    }
}

/// Automatic trigger for a mover crossing into a holder's reach/range. The
/// caller passes the position before the step, so a move that starts inside
/// reach does not count as entering it.
fn run_enter_reach_triggers(
    state: &mut EncounterState,
    mover: usize,
    before: &Position,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) {
    if state.ready_in_progress {
        return;
    }
    let Some(mover_state) = state.combatants.get(mover) else {
        return;
    };
    if mover_state.is_dead() || mover_state.dormant {
        return;
    }
    let mover_side = mover_state.side.clone();
    let candidates: Vec<usize> = state
        .combatants
        .iter()
        .enumerate()
        .filter(|(index, combatant)| {
            *index != mover
                && combatant.side != mover_side
                && combatant
                    .ready
                    .as_ref()
                    .is_some_and(|ready| ready.trigger == ReadyTrigger::EnterReach)
        })
        .map(|(index, _)| index)
        .collect();
    for index in candidates {
        let Some(ready) = state.combatants[index].ready.clone() else {
            continue;
        };
        if ready.trigger != ReadyTrigger::EnterReach {
            continue;
        }
        if state.combatants[mover].is_dead() || state.combatants[mover].hp <= 0 {
            break;
        }
        let reach = ready_watch_reach(state, index, &ready.response);
        let after = state.combatants[mover].position();
        let Some(now) = ready_distance_to_position(state, index, &after) else {
            continue;
        };
        let Some(was) = ready_distance_to_position(state, index, before) else {
            continue;
        };
        if was <= reach || now > reach {
            continue;
        }
        let _ = trigger_ready_at(
            state,
            index,
            Some(mover),
            Some(ReadyTrigger::EnterReach),
            rng,
            log,
        );
    }
}

fn ready_watch_reach(state: &EncounterState, holder: usize, response: &ReadyResponse) -> i32 {
    match response {
        ReadyResponse::Attack { attack, .. } => state.combatants[holder]
            .find_attack(attack)
            .ok()
            .map(|attack| {
                let reach = attack.reach.unwrap_or(5);
                match attack.kind {
                    AttackKind::Melee => reach,
                    AttackKind::Ranged => attack.range.map(|range| range[0]).unwrap_or(reach),
                    AttackKind::MeleeOrRanged => {
                        reach.max(attack.range.map(|range| range[0]).unwrap_or(reach))
                    }
                }
            })
            .unwrap_or(5),
        ReadyResponse::Unarmed { .. } => unarmed_profile(state, holder)
            .map(|profile| profile.reach)
            .unwrap_or(5),
        ReadyResponse::Ability { ability, .. } => state.combatants[holder]
            .find_owned_ability(&state.content, ability)
            .ok()
            .and_then(|resolved| resolved.ability.range)
            .filter(|range| *range > 0)
            .unwrap_or(5),
        ReadyResponse::Move { .. } => 5,
    }
}

fn ready_distance_to_position(
    state: &EncounterState,
    holder: usize,
    position: &Position,
) -> Option<i32> {
    if spatial_query_enabled(state) {
        if let Some(distance) = spatial_distance_to_position(state, holder, position) {
            return Some(distance);
        }
    }
    let space = state.space()?;
    let from = state.combatants[holder].position_ref();
    space.distance(from.as_ref(), position)
}

/// A resolved area centre. `creature` is `Some` only when the caller named a
/// creature; an explicit `x,y` or zone centre keeps the cell/zone floor.
#[derive(Debug, Clone)]
struct AreaCenter {
    position: Position,
    creature: Option<usize>,
}

/// Resolves a zone id, `x,y` grid cell, or creature name to an area centre.
fn resolve_area_center(state: &EncounterState, label: &str) -> Result<AreaCenter> {
    let label = label.trim();
    if let Some((x, y)) = label.split_once(',') {
        if let (Ok(x), Ok(y)) = (x.trim().parse::<i32>(), y.trim().parse::<i32>()) {
            return Ok(AreaCenter {
                position: Position::cell(x, y),
                creature: None,
            });
        }
    }
    if let Some(space) = state.space() {
        if space.has_zone(label) {
            return Ok(AreaCenter {
                position: Position::Zone {
                    zone: label.to_string(),
                },
                creature: None,
            });
        }
    }
    if let Ok(index) = state.find_index(label) {
        return Ok(AreaCenter {
            position: state.combatants[index].position(),
            creature: Some(index),
        });
    }
    bail!("unknown zone or area target `{label}`")
}

/// CLI `use <who> <attack-or-ability>`: falls back to a creature attack.
pub fn use_entry(
    state: &mut EncounterState,
    actor_name: &str,
    name: &str,
    targets: &[String],
    options: &UseOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
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
        ensure_not_charmed_target(state, actor, target, true)?;
        spend_action(state, actor, Activation::Action, log)?;
        let attack = state.combatants[actor].find_attack(name)?.clone();
        let profile = AttackProfile::from_attack(&attack);
        let options = AttackOptions {
            advantage: options.advantage,
            disadvantage: options.disadvantage,
            multi: false,
            no_reactions: options.no_reactions,
            nonlethal: options.nonlethal,
        };
        resolve_attack(
            state, actor, target, &profile, None, &options, rng, log, false, false,
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
    log: &mut dyn LogSink,
) {
    let name = state.combatants[actor].name.clone();
    clear_ready_lost_to_concentration(state, actor, ability, log);
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
            None,
            Vec::new(),
            Some(ability.id.clone()),
            true,
            &mut rng,
            log,
        );
    }
    log.log(format_args!(
        "{name} is concentrating on {}",
        ability.display_name()
    ));
}

/// A new concentration replaces a held readied spell's concentration. The
/// readied spell is lost (its slot/resource is not refunded), so the ready is
/// cleared instead of leaving an unusable response behind.
fn clear_ready_lost_to_concentration(
    state: &mut EncounterState,
    actor: usize,
    ability: &Ability,
    log: &mut dyn LogSink,
) {
    let Some(ready_ability) = state.combatants[actor]
        .ready
        .as_ref()
        .and_then(|ready| match &ready.response {
            ReadyResponse::Ability { ability, .. } => Some(ability.clone()),
            _ => None,
        })
    else {
        return;
    };
    let Ok(resolved) = state.combatants[actor].find_owned_ability(&state.content, &ready_ability)
    else {
        return;
    };
    if !resolved.ability.concentration {
        return;
    }
    let reason = format!(
        "a new concentration ({}) replaced the readied spell",
        ability.id
    );
    clear_ready_if_any(state, actor, &reason, log);
}

/// A creature that dies stops concentrating and loses the conditions that came
/// from other creatures' concentration spells (e.g. a bless it was under), so
/// `status` no longer lists them.
fn on_death(state: &mut EncounterState, target: usize, log: &mut dyn LogSink) {
    let name = state.combatants[target].name.clone();
    clear_ready_if_any(state, target, "dead", log);
    // End the dead creature's own concentration (and its conditions on others).
    break_concentration(state, target, log);
    // Drop conditions the dead creature got from other creatures' concentration.
    let mut kept = Vec::new();
    for condition in state.combatants[target].conditions.drain(..) {
        if condition.concentration && condition.source.as_deref() != Some(name.as_str()) {
            log.log(format_args!(
                "{} is no longer {}",
                name, condition.condition
            ));
        } else {
            kept.push(condition);
        }
    }
    state.combatants[target].conditions = kept;

    // Conditions whose data says they end when their recorded source dies.
    for holder in 0..state.combatants.len() {
        let ended: Vec<String> = state.combatants[holder]
            .conditions
            .iter()
            .filter(|condition| {
                condition
                    .source
                    .as_deref()
                    .or(condition.linked.as_deref())
                    .is_some_and(|source| source.eq_ignore_ascii_case(&name))
                    && condition_behavior(state, &condition.condition)
                        .is_some_and(|behavior| behavior.ends_when_source_dies)
            })
            .map(|condition| condition.condition.clone())
            .collect();
        if ended.is_empty() {
            continue;
        }
        let holder_name = state.combatants[holder].name.clone();
        state.combatants[holder].conditions.retain(|condition| {
            !ended
                .iter()
                .any(|id| condition.condition.eq_ignore_ascii_case(id))
        });
        for condition in ended {
            log.log(format_args!(
                "{holder_name} is no longer {condition} (source died)"
            ));
        }
    }
}

fn break_concentration(state: &mut EncounterState, actor: usize, log: &mut dyn LogSink) {
    let name = state.combatants[actor].name.clone();
    if state.concentration.remove(&name).is_none() {
        return;
    }
    log.log(format_args!("{name}'s concentration ends"));
    for combatant in &mut state.combatants {
        let mut kept = Vec::new();
        for condition in combatant.conditions.drain(..) {
            if condition.concentration && condition.source.as_deref() == Some(name.as_str()) {
                log.log(format_args!(
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
    log: &mut dyn LogSink,
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
        log.log(format_args!(
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
    log.log(format_args!(
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
    critical: bool,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<i32> {
    if rolls.is_empty() {
        return Ok(0);
    }
    let name = if log.enabled() {
        state.combatants[target].name.clone()
    } else {
        String::new()
    };
    // Resistance, vulnerability and immunity apply once to the summed raw
    // damage of each type, not to each part separately: two 1-point slashing
    // parts against resistance deal 1, not 0 + 0.
    let logging = log.enabled();
    let mut grouped: BTreeMap<String, (i32, Vec<String>)> = BTreeMap::new();
    for (record, damage_type) in &rolls {
        let entry = grouped
            .entry(damage_type.clone())
            .or_insert_with(|| (0, Vec::new()));
        entry.0 += record.total;
        if logging {
            entry.1.push(record.text.clone());
        }
    }
    let mut total = 0;
    let mut parts = Vec::new();
    for (damage_type, (raw, formulas)) in grouped {
        let amount = mitigate(state, target, source, raw, &damage_type, source_tags)?;
        total += amount;
        if logging {
            parts.push(format!("{amount} {damage_type} ({})", formulas.join(", ")));
        }
    }
    if total <= 0 {
        if log.enabled() {
            log.log(format_args!(
                "{name} takes no damage ({})",
                parts.join("; ")
            ));
        }
        return Ok(0);
    }

    let temp = state.combatants[target].temp_hp;
    let absorbed = temp.min(total);
    if absorbed > 0 {
        state.combatants[target].temp_hp -= absorbed;
    }
    let hp_damage = total - absorbed;
    let before = state.combatants[target].hp;
    let was_at_zero = before <= 0;
    state.combatants[target].hp = (before - hp_damage).max(0);
    if log.enabled() {
        log.log(format_args!(
            "{name} takes {total} damage ({}){}",
            parts.join("; "),
            if absorbed > 0 {
                format!("; {absorbed} absorbed by temporary HP")
            } else {
                String::new()
            }
        ));
    }
    if state.combatants[target].hp <= 0 {
        clear_ready_if_any(state, target, "0 HP", log);
    }

    // Massive damage.
    let overflow = hp_damage.saturating_sub(before.max(0));
    let max_hp = effective_max_hp(state, target);
    if state.combatants[target].hp == 0 && !was_at_zero && state.nonlethal_active {
        // 2024 PHB "Knocking Out a Creature": Unconscious and stable instead.
        knock_out(state, target, source, log);
        concentration_damage_check(state, target, total, rng, log)?;
        run_damage_taken_triggers(state, target, source, total, rng, log);
        fire_ends_on(state, target, EndsOn::DamageTaken, log);
        return Ok(total);
    }
    if state.combatants[target].hp == 0 && overflow >= max_hp {
        if log.enabled() {
            log.log(format_args!("{name} dies from massive damage"));
        }
        state.combatants[target].dead = true;
        if state.combatants[target].makes_death_saves() {
            state.combatants[target].death_saves.dead = true;
        }
        on_death(state, target, log);
    }

    if state.combatants[target].hp == 0 && !state.combatants[target].is_dead() {
        if state.combatants[target].makes_death_saves() {
            if was_at_zero {
                // Damage taken while already at 0 HP is a death-save failure;
                // it never re-applies the dying state or repeats the drop log.
                if hp_damage > 0 {
                    let failures = if critical { 2 } else { 1 };
                    add_death_save_failures(state, target, failures, log);
                }
            } else {
                apply_dying(state, target, log);
            }
        } else {
            state.combatants[target].dead = true;
            on_death(state, target, log);
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
                    None,
                    Vec::new(),
                    None,
                    false,
                    &mut rng,
                    log,
                );
            }
            if log.enabled() {
                log.log(format_args!("{name} is dead"));
            }
        }
    }

    // Damage-taken triggers and concentration.
    concentration_damage_check(state, target, total, rng, log)?;
    run_damage_taken_triggers(state, target, source, total, rng, log);
    fire_ends_on(state, target, EndsOn::DamageTaken, log);
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
    let multiplier = damage_multiplier(state, target, Some(source), damage_type, source_tags);
    Ok(((amount as f64) * multiplier).floor() as i32)
}

fn knock_out(state: &mut EncounterState, target: usize, source: usize, log: &mut dyn LogSink) {
    let name = state.combatants[target].name.clone();
    state.combatants[target].knocked_out = true;
    state.combatants[target].death_saves.stable = true;
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
            None,
            Vec::new(),
            None,
            false,
            &mut rng,
            log,
        );
    }
    if log.enabled() {
        log.log(format_args!(
            "{name} is knocked out (non-lethal): out of the fight (dying conditions applied, stable); wakes at 1 HP after a Short Rest, on any healing, or DC 10 Medicine first aid"
        ));
    }
}

fn apply_dying(state: &mut EncounterState, target: usize, log: &mut dyn LogSink) {
    let name = state.combatants[target].name.clone();
    state.combatants[target].death_saves.stable = false;
    log.log(format_args!(
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
            None,
            Vec::new(),
            None,
            false,
            &mut rng,
            log,
        );
    }
}

/// Damage taken while a PC is already at 0 HP adds death-save failures.
fn add_death_save_failures(
    state: &mut EncounterState,
    target: usize,
    failures: u8,
    log: &mut dyn LogSink,
) {
    let name = state.combatants[target].name.clone();
    state.combatants[target].death_saves.stable = false;
    state.combatants[target].death_saves.failures = state.combatants[target]
        .death_saves
        .failures
        .saturating_add(failures);
    let total = state.combatants[target].death_saves.failures;
    log.log(format_args!(
        "{name} fails {failures} death saving throw{} from damage ({total}/3)",
        if failures == 1 { "" } else { "s" }
    ));
    if total >= 3 {
        state.combatants[target].death_saves.dead = true;
        state.combatants[target].dead = true;
        on_death(state, target, log);
        log.log(format_args!("{name} dies"));
    }
}

fn run_damage_taken_triggers(
    state: &mut EncounterState,
    target: usize,
    source: usize,
    damage: i32,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) {
    let abilities = trigger_abilities(state, target, TriggerWhen::OnDamageTaken);
    if abilities.is_empty() {
        return;
    }
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
                source: None,
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
    log: &mut dyn LogSink,
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
        crit,
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
    log: &mut dyn LogSink,
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
    // Direct healing is a DM correction, but when a source creature is named
    // the spatial target rules still apply: no healing through total cover.
    if let Some(source) = from_index {
        if source != target && spatial_query_enabled(state) {
            ensure_cover_allows(state, source, target)?;
        }
    }
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
            source: None,
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
                    let context = score_context(&state.combatants[source]).with_slot_level(1);
                    let (_, expression) = evaluate_value(&context, value)?;
                    let extra = expression.roll(rng).total;
                    amount += extra;
                    terms.push(format!("+{extra}[{}]", collected.label));
                }
            }
        }
    }
    // Healing received works the same as the spell `heal` effect path:
    // bonuses and penalties from the target's conditions and features.
    let received_context = PredicateContext {
        self_idx: target,
        actor: target,
        target: Some(target),
        attacker: None,
        linked: None,
        source: None,
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
    let received_value_context = score_context(&state.combatants[from_index.unwrap_or(target)]);
    for collected in collect_modifiers(
        state,
        target,
        ModifierApplies::HealingReceived,
        &received_context,
        None,
    ) {
        if collected.modifier.effect == ModifierEffect::Bonus {
            if let Some(value) = &collected.modifier.value {
                let (_, expression) = evaluate_value(&received_value_context, value)?;
                let extra = expression.roll(rng).total;
                amount += extra;
                terms.push(format!("{extra:+}[{}]", collected.label));
            }
        }
    }
    let name = state.combatants[target].name.clone();
    let before = state.combatants[target].hp;
    let max_hp = effective_max_hp(state, target);
    state.combatants[target].hp = (before + amount).min(max_hp);
    let healed = state.combatants[target].hp - before;
    log.log(format_args!(
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
    log: &mut dyn LogSink,
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
        log.log(format_args!(
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
    log.log(format_args!(
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
    log: &mut dyn LogSink,
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
    log.log(format_args!(
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
    crit: bool,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<crate::roll::RollOutcome> {
    let outcome = crate::roll::roll_command(expression, mode, dc, crit, rng)
        .with_context(|| format!("invalid dice expression `{expression}`"))?;
    log.emit(&outcome.text);
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Movement, activation, death saves, rests
// ---------------------------------------------------------------------------

/// CLI `move` destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveTarget {
    Zone(String),
    Cell(i32, i32),
}

/// CLI `move` / forced movement destination helpers.
///
/// Resolves the route step by step so opportunity attacks and on-enter scene
/// triggers fire at every zone/cell boundary. Movement is never blocked for
/// exceeding the remaining speed; the overrun is logged instead.
pub fn move_to(
    state: &mut EncounterState,
    who: &str,
    target: &MoveTarget,
    no_oa: bool,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    move_to_at(state, index, target, no_oa, rng, log)
}

/// Resolve a CLI destination. The legacy callers pass `check_occupancy`
/// because their 2D router treats any shared cell as occupied; the spatial
/// path defers to `can_occupy`, which understands altitude bands.
fn resolve_move_destination(
    state: &EncounterState,
    index: usize,
    target: &MoveTarget,
    check_occupancy: bool,
    log: &mut dyn LogSink,
) -> Result<Option<Position>> {
    let Some(space) = state.space() else {
        bail!("this state has no scene");
    };
    let start = state.combatants[index].position();
    let destination = match (space.mode(), target) {
        (SceneMode::Zones, MoveTarget::Cell(..)) => {
            bail!("`--to` needs a grid scene; this scene uses zones")
        }
        (SceneMode::Zones, MoveTarget::Zone(zone)) => {
            if !space.has_zone(zone) {
                bail!("unknown zone `{zone}`");
            }
            Position::Zone { zone: zone.clone() }
        }
        (SceneMode::Grid, MoveTarget::Cell(x, y)) => {
            let position = Position::cell(*x, *y);
            if !space.contains(&position) {
                bail!("cell ({x},{y}) is a wall or outside the grid");
            }
            if check_occupancy
                && state
                    .combatants
                    .iter()
                    .enumerate()
                    .any(|(other, combatant)| {
                        other != index && !combatant.is_dead() && combatant.position() == position
                    })
            {
                bail!("cell ({x},{y}) is occupied");
            }
            position
        }
        (SceneMode::Grid, MoveTarget::Zone(zone)) => {
            if !space.has_zone(zone) {
                bail!("unknown zone `{zone}`");
            }
            let occupied: Vec<Position> = state
                .combatants
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, combatant)| combatant.position())
                .collect();
            let mut candidates = Vec::new();
            for cell in space.zone_positions(zone) {
                if cell == start || occupied.contains(&cell) {
                    continue;
                }
                if let Some(cost) = space.route_cost(&start, &cell, &occupied) {
                    candidates.push((cost, cell));
                }
            }
            candidates.sort_by_key(|(cost, cell)| {
                (
                    *cost,
                    cell.cell_coords().map(|(_, y)| y).unwrap_or(0),
                    cell.cell_coords().map(|(x, _)| x).unwrap_or(0),
                )
            });
            match candidates.into_iter().next() {
                Some((_, cell)) => cell,
                None => {
                    if space.zone_of(&start).as_deref() == Some(zone) {
                        log.log(format_args!(
                            "{} is already in {zone}",
                            state.combatants[index].name
                        ));
                        return Ok(None);
                    }
                    bail!("no free cell tagged `{zone}`");
                }
            }
        }
    };
    if destination == start {
        log.log(format_args!(
            "{} is already in {}",
            state.combatants[index].name, state.combatants[index].zone
        ));
        return Ok(None);
    }
    Ok(Some(destination))
}

/// [`move_to`] for callers that already hold the mover's index.
pub fn move_to_at(
    state: &mut EncounterState,
    index: usize,
    target: &MoveTarget,
    no_oa: bool,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let combatant = &state.combatants[index];
    if combatant.dormant {
        bail!("`{}` is dormant and cannot move", combatant.name);
    }
    if combatant.is_dead() || combatant.hp <= 0 {
        bail!("`{}` cannot move", combatant.name);
    }
    if spatial_query_enabled(state)
        && state
            .space()
            .is_some_and(|space| space.mode() == SceneMode::Grid)
    {
        return spatial_move_to_at(state, index, target, no_oa, rng, log);
    }
    let Some(destination) = resolve_move_destination(state, index, target, true, log)? else {
        return Ok(());
    };
    let start = state.combatants[index].position();
    let Some(space) = state.space() else {
        bail!("this state has no scene");
    };
    let blocked: Vec<Position> = state
        .combatants
        .iter()
        .enumerate()
        .filter(|(other, combatant)| {
            *other != index
                && !combatant.dormant
                && !combatant.is_dead()
                && combatant.hp > 0
                && combatant.side != state.combatants[index].side
        })
        .map(|(_, combatant)| combatant.position())
        .collect();
    let route = space
        .move_route(&start, &destination, &blocked)
        .ok_or_else(|| anyhow!("no path to the destination"))?;
    drop(space);
    let mut previous = start.clone();
    for step in &route {
        ensure_frightened_move(state, index, &previous, &step.position)?;
        previous = step.position.clone();
    }
    let name = state.combatants[index].name.clone();
    let mut current = start;
    for step in route {
        if state.combatants[index].is_dead() || state.combatants[index].hp <= 0 {
            break;
        }
        let from_position = current.clone();
        if step.locked {
            log.log(format_args!(
                "{name} finds a locked door and stops at {}",
                state.combatants[index].zone
            ));
            break;
        }
        if step.elevation_change > 0 {
            if let Some(climb) = &step.climb {
                if !roll_climb(state, index, climb, rng, log)? {
                    log.log(format_args!(
                        "{name} fails the climb and stops at {}",
                        state.combatants[index].zone
                    ));
                    break;
                }
            }
        }
        if !no_oa {
            opportunity_attacks(state, index, &current, rng, log)?;
            if state.combatants[index].is_dead() || state.combatants[index].hp <= 0 {
                break;
            }
        }
        // 2024 PHB: climbing (up or down) costs 1 extra foot per foot unless the
        // creature has a Climb Speed.
        let climb_extra =
            if step.elevation_change != 0 && state.combatants[index].creature.speed().climb <= 0 {
                step.elevation_change.abs()
            } else {
                0
            };
        if climb_extra > 0 {
            log.log(format_args!(
                "{name} climbs {} ft {}: +{climb_extra} ft movement (no Climb Speed)",
                step.elevation_change.abs(),
                if step.elevation_change > 0 {
                    "up"
                } else {
                    "down"
                }
            ));
        }
        state.combatants[index].speed_left -= step.cost + climb_extra;
        let (from_label, to_label) = {
            let space = state.space();
            let from_label = state.combatants[index].zone.clone();
            let to_label = space
                .as_ref()
                .and_then(|space| space.zone_of(&step.position))
                .unwrap_or_else(|| match &step.position {
                    Position::Zone { zone } => zone.clone(),
                    Position::Cell { .. } => OPEN_ZONE.to_string(),
                });
            (from_label, to_label)
        };
        match &step.position {
            Position::Cell { x, y } => state.combatants[index].set_cell(*x, *y, to_label.clone()),
            Position::Zone { zone } => state.combatants[index].set_zone(zone.clone()),
        }
        log.log(format_args!(
            "{name} moves {from_label} -> {to_label} ({} ft)",
            step.cost
        ));
        fire_enter_triggers(state, index, &step.position, rng, log)?;
        run_enter_reach_triggers(state, index, &from_position, rng, log);
        if state.combatants[index].position() != step.position {
            log.log(format_args!(
                "{name}'s movement stops after a readied response moved them"
            ));
            break;
        }
        current = step.position;
    }
    if state.combatants[index].speed_left < 0 {
        log.log(format_args!(
            "{name} has used {} ft more than the remaining movement",
            -state.combatants[index].speed_left
        ));
    }
    Ok(())
}

/// Movement map for one horizontal plane. Ground movement uses each cell's
/// floor; flight uses the mover's absolute altitude so walls and tall
/// obstacles below the feet no longer block.
fn spatial_movement_map(
    state: &EncounterState,
    grid: &GridMap,
    plane_z: i32,
    flying: bool,
) -> Result<MovementMap> {
    let wall_height = state
        .scene
        .as_deref()
        .and_then(|scene| scene.cell_height_ft)
        .unwrap_or(10)
        .max(1);
    let mut cells = Vec::with_capacity((grid.width * grid.height) as usize);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let Some(cell) = grid.cell_at(x, y) else {
                bail!("grid cell ({x},{y}) is missing");
            };
            let floor = cell.elevation.unwrap_or(0);
            let mut movement = MovementCell::open();
            movement.elevation_ft = if flying { plane_z } else { floor };
            movement.difficult = cell.difficult && !flying;
            if flying && floor > plane_z {
                // A flyer cannot pass through a floor above its feet.
                movement.blocked = true;
            }
            if cell.wall {
                // A wall is solid up to its own height; a flyer above that
                // height passes over it.
                let top = floor.saturating_add(cell.height_ft.unwrap_or(wall_height));
                if plane_z < top {
                    movement.blocked = true;
                }
            } else if let Some(height) = cell.height_ft {
                // The obstacle occupies `floor..floor+height`; feet at or
                // above its top pass over it.
                if plane_z < floor.saturating_add(height) {
                    movement.blocked = true;
                }
            }
            cells.push(movement);
        }
    }
    let scene_terrain = state
        .scene
        .as_deref()
        .map_or(&[][..], |scene| scene.terrain_volumes.as_slice());
    let terrain = state
        .spatial
        .as_ref()
        .filter(|spatial| !spatial.terrain_volumes.is_empty())
        .map_or(scene_terrain, |spatial| spatial.terrain_volumes.as_slice());
    for volume in terrain {
        let top = volume.cell.z.saturating_add(volume.height_ft);
        if plane_z < volume.cell.z || plane_z >= top {
            continue;
        }
        for dy in 0..volume.footprint.depth {
            for dx in 0..volume.footprint.width {
                let x = volume.cell.x.saturating_add(dx as i32);
                let y = volume.cell.y.saturating_add(dy as i32);
                if x < 0 || y < 0 || x >= grid.width || y >= grid.height {
                    continue;
                }
                cells[(y * grid.width + x) as usize].blocked = true;
                cells[(y * grid.width + x) as usize].elevation_ft = if flying {
                    plane_z
                } else {
                    grid.cell_at(x, y)
                        .and_then(|cell| cell.elevation)
                        .unwrap_or(0)
                };
            }
        }
    }
    MovementMap::new(grid.width, grid.height, cells).map_err(|error| anyhow!(error.to_string()))
}

/// P3-backed grid movement. The path is validated before any state change and
/// every step is re-checked before it is applied, so a blocked/reaction stop
/// can never teleport the mover to the goal.
fn spatial_move_to_at(
    state: &mut EncounterState,
    index: usize,
    target: &MoveTarget,
    no_oa: bool,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let Some(destination) = resolve_move_destination(state, index, target, false, log)? else {
        return Ok(());
    };
    let Position::Cell {
        x: dest_x,
        y: dest_y,
    } = destination
    else {
        bail!("spatial movement needs a grid destination");
    };
    let scene = state
        .scene
        .as_deref()
        .ok_or_else(|| anyhow!("this state has no scene"))?
        .clone();
    let grid = GridMap::parse(&scene).ok_or_else(|| anyhow!("scene has no usable grid"))?;
    let profile = state.effective_spatial_profile();
    let start_pose = state.combatants[index]
        .spatial_pose()
        .ok_or_else(|| anyhow!("`{}` has no spatial pose", state.combatants[index].name))?;
    let start_floor = grid
        .cell_at(start_pose.cell.x, start_pose.cell.y)
        .map(|cell| cell.elevation.unwrap_or(0))
        .ok_or_else(|| anyhow!("the mover is outside the grid"))?;
    let vertical = state.combatants[index].vertical_state();
    let flying = matches!(vertical.flight, FlightState::Flying | FlightState::Hovering);
    let start_z = if flying {
        start_pose
            .cell
            .z
            .max(start_floor.saturating_add(vertical.altitude_ft))
    } else {
        start_floor
    };
    let dest_floor = grid
        .cell_at(dest_x, dest_y)
        .map(|cell| cell.elevation.unwrap_or(0))
        .ok_or_else(|| anyhow!("cell ({dest_x},{dest_y}) is outside the grid"))?;
    if !flying && grid.is_wall(dest_x, dest_y) {
        bail!("cell ({dest_x},{dest_y}) is a wall or outside the grid");
    }
    let goal_z = if flying { start_z } else { dest_floor };
    let goal_pose = SpatialPose {
        cell: CellPos {
            x: dest_x,
            y: dest_y,
            z: goal_z,
        },
        footprint: start_pose.footprint,
        facing: start_pose.facing,
    };
    let mut mover = spatial_body_for(state, index)
        .ok_or_else(|| anyhow!("`{}` has no spatial body", state.combatants[index].name))?;
    mover.pose = SpatialPose {
        cell: CellPos {
            x: start_pose.cell.x,
            y: start_pose.cell.y,
            z: start_z,
        },
        footprint: start_pose.footprint,
        facing: start_pose.facing,
    };
    let mover_side = state.combatants[index].side.clone();
    let occupants: Vec<TraversalCreature> = state
        .combatants
        .iter()
        .enumerate()
        .filter(|(other, combatant)| {
            *other != index && !combatant.dormant && !combatant.is_dead() && combatant.hp > 0
        })
        .filter_map(|(other, combatant)| {
            spatial_body_for(state, other).map(|body| {
                TraversalCreature::new(
                    body,
                    combatant.side == mover_side,
                    is_incapacitated(state, other),
                )
            })
        })
        .collect();
    let map = spatial_movement_map(state, &grid, start_z, flying)?;
    let planner = MovementPlanner::new(&map, &profile, &mover).with_occupants(&occupants);
    let budget = state.combatants[index].speed_left.max(0);
    let plan = planner
        .path(&goal_pose, budget)
        .map_err(|error| anyhow!(error.to_string()))?;
    let path = match plan.outcome {
        PlanOutcome::Reached(path) => path,
        PlanOutcome::OverBudget(path) => {
            bail!(
                "cannot move to ({dest_x},{dest_y}): the route needs {} ft but only {budget} ft of movement remains",
                path.total_cost_ft
            );
        }
        PlanOutcome::Unreachable { reason, blockers } => {
            if blockers.is_empty() {
                bail!("cannot move to ({dest_x},{dest_y}): {}", reason.as_str());
            }
            bail!(
                "cannot move to ({dest_x},{dest_y}): {} ({})",
                reason.as_str(),
                blockers.join(", ")
            );
        }
    };
    let mut previous_position = Position::cell(start_pose.cell.x, start_pose.cell.y);
    for step in &path.steps {
        let next = Position::cell(step.to.cell.x, step.to.cell.y);
        ensure_frightened_move(state, index, &previous_position, &next)?;
        previous_position = next;
    }
    let name = state.combatants[index].name.clone();
    let mut current_pose = mover.pose;
    let mut current_position = Position::cell(start_pose.cell.x, start_pose.cell.y);
    let mut previous_zone = state
        .space()
        .and_then(|space| space.zone_of(&current_position))
        .unwrap_or_else(|| OPEN_ZONE.to_string());
    for (applied_steps, step) in path.steps.into_iter().enumerate() {
        if state.combatants[index].is_dead() || state.combatants[index].hp <= 0 {
            break;
        }
        if !no_oa {
            opportunity_attacks(state, index, &current_position, rng, log)?;
            if state.combatants[index].is_dead() || state.combatants[index].hp <= 0 {
                break;
            }
        }
        let check = planner.step(&current_pose, &step.to);
        if let Some(reason) = check.blocked {
            if check.blockers.is_empty() {
                log.log(format_args!(
                    "{name} stops at ({},{}) after step {applied_steps}: {}",
                    current_pose.cell.x,
                    current_pose.cell.y,
                    reason.as_str()
                ));
            } else {
                log.log(format_args!(
                    "{name} stops at ({},{}) after step {applied_steps}: {} ({})",
                    current_pose.cell.x,
                    current_pose.cell.y,
                    reason.as_str(),
                    check.blockers.join(", ")
                ));
            }
            break;
        }
        let x = step.to.cell.x;
        let y = step.to.cell.y;
        let zone = grid.zone_at(x, y).unwrap_or_else(|| OPEN_ZONE.to_string());
        state.combatants[index].set_cell(x, y, zone.clone());
        state.combatants[index].pose = Some(step.to);
        let floor = grid
            .cell_at(x, y)
            .map(|cell| cell.elevation.unwrap_or(0))
            .unwrap_or(0);
        let mut vertical = state.combatants[index].vertical_state();
        vertical.altitude_ft = (step.to.cell.z - floor).max(0);
        if vertical.flight == FlightState::Grounded && vertical.altitude_ft != 0 {
            vertical.flight = FlightState::Flying;
        }
        vertical
            .validate()
            .map_err(|error| anyhow!(error.to_string()))?;
        state.combatants[index].vertical = Some(vertical);
        state.combatants[index].speed_left -= step.cost_ft;
        log.log(format_args!(
            "{name} moves {previous_zone} -> {zone} ({} ft)",
            step.cost_ft
        ));
        if zone != previous_zone {
            fire_enter_triggers(state, index, &Position::cell(x, y), rng, log)?;
            previous_zone = zone;
        }
        run_enter_reach_triggers(state, index, &current_position, rng, log);
        let expected = Position::cell(x, y);
        if state.combatants[index].position() != expected {
            log.log(format_args!(
                "{name}'s movement stops after a readied response moved them"
            ));
            break;
        }
        current_pose = step.to;
        current_position = expected;
    }
    if state.combatants[index].speed_left < 0 {
        log.log(format_args!(
            "{name} has used {} ft more than the remaining movement",
            -state.combatants[index].speed_left
        ));
    }
    Ok(())
}

/// P3-backed forced movement. Forced movement spends no Speed and provokes no
/// opportunity attacks; area-entry triggers still fire on zone boundaries.
/// A drop larger than one step stops the push (`MoveBlock::Drop`); wiring that
/// into a fall is a separate follow-up because the caller must choose the
/// landing cell.
fn spatial_forced_move(
    state: &mut EncounterState,
    target: usize,
    source: usize,
    effect: &ForcedMoveEffect,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<bool> {
    if !spatial_query_enabled(state)
        || !state
            .space()
            .is_some_and(|space| space.mode() == SceneMode::Grid)
    {
        return Ok(false);
    }
    let Some(scene) = state.scene.as_deref().cloned() else {
        return Ok(false);
    };
    let Some(grid) = GridMap::parse(&scene) else {
        return Ok(false);
    };
    let Some(mover_body) = spatial_body_for(state, target) else {
        return Ok(false);
    };
    let (tx, ty) = (mover_body.pose.cell.x, mover_body.pose.cell.y);
    let Some((sx, sy)) = state.combatants[source].position_ref().cell_coords() else {
        return Ok(false);
    };
    let (dx, dy) = match effect.direction {
        Direction::Away => ((tx - sx).signum(), (ty - sy).signum()),
        Direction::Toward => ((sx - tx).signum(), (sy - ty).signum()),
    };
    if dx == 0 && dy == 0 {
        return Ok(true);
    }
    let profile = state.effective_spatial_profile();
    let vertical = state.combatants[target].vertical_state();
    let flying = matches!(vertical.flight, FlightState::Flying | FlightState::Hovering);
    let map = spatial_movement_map(state, &grid, mover_body.pose.cell.z, flying)?;
    let occupants: Vec<TraversalCreature> = state
        .combatants
        .iter()
        .enumerate()
        .filter(|(other, combatant)| {
            *other != target && !combatant.dormant && !combatant.is_dead() && combatant.hp > 0
        })
        .filter_map(|(other, combatant)| {
            spatial_body_for(state, other).map(|body| {
                TraversalCreature::new(
                    body,
                    combatant.side == state.combatants[target].side,
                    is_incapacitated(state, other),
                )
            })
        })
        .collect();
    let planner = MovementPlanner::new(&map, &profile, &mover_body).with_occupants(&occupants);
    let plan = planner
        .forced_move((dx, dy), effect.distance.max(0))
        .map_err(|error| anyhow!(error.to_string()))?;
    let name = state.combatants[target].name.clone();
    let mut previous_zone = state
        .space()
        .and_then(|space| {
            space.zone_of(&Position::cell(
                mover_body.pose.cell.x,
                mover_body.pose.cell.y,
            ))
        })
        .unwrap_or_else(|| OPEN_ZONE.to_string());
    for step in plan.steps {
        let before = state.combatants[target].position();
        let x = step.to.cell.x;
        let y = step.to.cell.y;
        let zone = grid.zone_at(x, y).unwrap_or_else(|| OPEN_ZONE.to_string());
        state.combatants[target].set_cell(x, y, zone.clone());
        state.combatants[target].pose = Some(step.to);
        let floor = grid
            .cell_at(x, y)
            .map(|cell| cell.elevation.unwrap_or(0))
            .unwrap_or(0);
        let mut vertical = state.combatants[target].vertical_state();
        vertical.altitude_ft = (step.to.cell.z - floor).max(0);
        if vertical.flight == FlightState::Grounded && vertical.altitude_ft != 0 {
            vertical.flight = FlightState::Flying;
        }
        vertical
            .validate()
            .map_err(|error| anyhow!(error.to_string()))?;
        state.combatants[target].vertical = Some(vertical);
        log.log(format_args!("{name} is moved to {zone}"));
        if zone != previous_zone {
            fire_enter_triggers(state, target, &Position::cell(x, y), rng, log)?;
            previous_zone = zone;
        }
        run_enter_reach_triggers(state, target, &before, rng, log);
    }
    if let Some(reason) = plan.stopped_by {
        log.log(format_args!(
            "{name}'s forced movement stops: {}",
            reason.as_str()
        ));
    }
    Ok(true)
}

fn roll_climb(
    state: &mut EncounterState,
    index: usize,
    climb: &crate::content::schema::ClimbSpec,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<bool> {
    let skill = climb.skill.clone();
    let base = state.combatants[index].skill_bonus(&skill);
    let (mode, advantages, disadvantages) =
        check_modifiers(state, index, ability_from_text(&skill), &skill);
    let result = roll_d20(
        state,
        index,
        base,
        mode,
        Vec::new(),
        advantages,
        disadvantages,
        rng,
    );
    let name = state.combatants[index].name.clone();
    let success = result.total >= climb.dc;
    log.log(format_args!(
        "{name} {skill} check: {} vs DC {} {}",
        result.text(),
        climb.dc,
        if success { "PASS" } else { "FAIL" }
    ));
    Ok(success)
}

fn is_disengaged(state: &EncounterState, index: usize) -> bool {
    state
        .content
        .rules
        .basic_actions
        .as_ref()
        .and_then(|actions| actions.get("disengage"))
        .is_some_and(|condition| state.combatants[index].has_condition(&state.content, condition))
}

fn first_melee_attack(state: &EncounterState, index: usize) -> Option<Attack> {
    state.combatants[index]
        .creature
        .attacks
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .find(|attack| matches!(attack.kind, AttackKind::Melee | AttackKind::MeleeOrRanged))
        .cloned()
}

fn opportunity_attacks(
    state: &mut EncounterState,
    mover: usize,
    departure: &Position,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    if is_disengaged(state, mover)
        || has_flag(state, mover, ConditionFlag::NoOpportunityAttacksAgainst)
    {
        return Ok(());
    }
    let Some(space) = state.space() else {
        return Ok(());
    };
    let candidates: Vec<usize> = state
        .combatants
        .iter()
        .enumerate()
        .filter(|(index, combatant)| {
            *index != mover
                && combatant.is_active()
                && combatant.side != state.combatants[mover].side
                && combatant.has_reaction(state.round)
                && !has_flag(state, *index, ConditionFlag::NoReactions)
                && !has_flag(state, *index, ConditionFlag::NoOpportunityAttacks)
                && !is_incapacitated(state, *index)
                && {
                    if spatial_query_enabled(state) {
                        let observer = spatial_body_for(state, *index);
                        let mover_body = spatial_body_for(state, mover);
                        match (observer, mover_body) {
                            (Some(observer), Some(mover_body)) => {
                                let reach = first_melee_attack(state, *index)
                                    .and_then(|attack| attack.reach)
                                    .unwrap_or(5);
                                let profile = state.effective_spatial_profile();
                                let reachable = crate::space::can_reach_bodies(
                                    &observer,
                                    &mover_body,
                                    reach,
                                    &profile,
                                )
                                .reachable;
                                let blockers = spatial_blockers(state, &profile);
                                let sight = crate::space::line_of_sight(
                                    &observer,
                                    &mover_body,
                                    &blockers,
                                    &profile,
                                );
                                reachable && sight.clear
                            }
                            _ => {
                                let position = combatant.position_ref();
                                space.opportunity_threat(position.as_ref(), departure)
                            }
                        }
                    } else {
                        let position = combatant.position_ref();
                        space.opportunity_threat(position.as_ref(), departure)
                    }
                }
        })
        .map(|(index, _)| index)
        .collect();
    drop(space);
    for hostile in candidates {
        if state.combatants[mover].is_dead() || state.combatants[mover].hp <= 0 {
            break;
        }
        let Some(attack) = first_melee_attack(state, hostile) else {
            continue;
        };
        let profile = AttackProfile::from_attack(&attack);
        state.combatants[hostile].reaction_round = Some(state.round);
        log.log(format_args!(
            "{} makes an opportunity attack against {}",
            state.combatants[hostile].name, state.combatants[mover].name
        ));
        resolve_attack(
            state,
            hostile,
            mover,
            &profile,
            None,
            &AttackOptions::default(),
            rng,
            log,
            true,
            true,
        )?;
    }
    Ok(())
}

fn fire_enter_triggers(
    state: &mut EncounterState,
    mover: usize,
    position: &Position,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let trigger_count = state
        .scene
        .as_ref()
        .map(|scene| scene.triggers.len())
        .unwrap_or(0);
    if trigger_count == 0
        || !state.scene.as_ref().is_some_and(|scene| {
            scene
                .triggers
                .iter()
                .any(|trigger| trigger.on == TriggerWhen::OnEnterArea)
        })
    {
        return Ok(());
    }
    let Some(entered) = state.space().and_then(|space| space.zone_of(position)) else {
        return Ok(());
    };
    for index in 0..trigger_count {
        if state.scene.as_ref().is_none_or(|scene| {
            scene
                .triggers
                .get(index)
                .is_none_or(|trigger| trigger.on != TriggerWhen::OnEnterArea)
        }) {
            continue;
        }
        // Clone only the matching trigger; the scene itself is large.
        let Some(trigger) = state
            .scene
            .as_ref()
            .and_then(|scene| scene.triggers.get(index))
            .cloned()
        else {
            continue;
        };
        let trigger = &trigger;
        let key = EncounterState::trigger_key(index, trigger);
        if !state.trigger_enabled(&key) {
            continue;
        }
        let fired = state
            .triggers
            .get(&key)
            .map(|runtime| runtime.fired)
            .unwrap_or(false);
        if trigger.once && fired {
            continue;
        }
        let mut zones = Vec::new();
        if let Some(zone) = &trigger.zone {
            zones.push(zone.as_str());
        }
        zones.extend(trigger.zones.iter().map(String::as_str));
        if !zones.iter().any(|zone| zone.eq_ignore_ascii_case(&entered)) {
            continue;
        }
        if let Some(side) = &trigger.side {
            if !state.combatants[mover]
                .side
                .as_str()
                .eq_ignore_ascii_case(side.as_str())
            {
                continue;
            }
        }
        if let Some(predicate) = &trigger.when {
            let context = PredicateContext {
                target: Some(mover),
                ..PredicateContext::for_owner(mover)
            };
            if !eval_predicate(state, predicate, &context) {
                continue;
            }
        }
        if let Some(manual) = &trigger.manual {
            log.log(format_args!("MANUAL: {manual}"));
        }
        let occupants: Vec<usize> = match trigger.affects.unwrap_or_default() {
            crate::content::schema::TriggerAffects::Entering => vec![mover],
            crate::content::schema::TriggerAffects::ZoneOccupants => {
                let space = state.space();
                state
                    .combatants
                    .iter()
                    .enumerate()
                    .filter(|(_, combatant)| !combatant.dormant)
                    .filter(|(_, combatant)| {
                        space.as_ref().is_some_and(|space| {
                            space.zone_of(&combatant.position()).is_some_and(|zone| {
                                zones
                                    .iter()
                                    .any(|candidate| candidate.eq_ignore_ascii_case(&zone))
                            })
                        })
                    })
                    .map(|(index, _)| index)
                    .collect()
            }
        };
        let source = EffectContext {
            actor: mover,
            target: Some(mover),
            source: None,
            attack: None,
            option: None,
            crit: false,
            damage_dealt: 0,
            damage_scale: 1.0,
            has_advantage: false,
            has_disadvantage: false,
        };
        for occupant in occupants {
            let context = EffectContext {
                target: Some(occupant),
                ..source.clone()
            };
            apply_effects(state, &context, &trigger.effects, rng, log)?;
        }
        state.set_trigger_fired(&key);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// P4 flight and fall interface (TASK-0089)
// ---------------------------------------------------------------------------
//
// This is a seam, not a turn-loop integration. P5 (#174) owns CLI/AI wiring
// and trigger order. A legacy 2D save has no pose, vertical state, spatial
// state or 3D scene data, so `spatial_flight_enabled` stays false and the
// fall entry points return `None` without changing any existing behaviour.

/// Whether this encounter has explicitly opted into the P1 spatial fields.
pub fn spatial_flight_enabled(state: &EncounterState) -> bool {
    state.spatial.is_some()
        || state
            .combatants
            .iter()
            .any(|combatant| combatant.pose.is_some() || combatant.vertical.is_some())
        || state.scene.as_deref().is_some_and(|scene| {
            scene.cell_height_ft.is_some() || !scene.terrain_volumes.is_empty()
        })
}

/// Build the vertical map for a grid encounter. Zone scenes have no metric
/// vertical map; a fall there stays an explicit error rather than an invented
/// approximation.
fn build_fall_map(state: &EncounterState) -> Result<FallMap> {
    let scene = state
        .scene
        .as_deref()
        .ok_or_else(|| anyhow!("spatial flight is enabled without a scene"))?;
    let mut map =
        FallMap::from_scene(scene).ok_or_else(|| anyhow!("scene has no usable grid fall map"))?;
    if let Some(spatial) = &state.spatial {
        if spatial.cell_height_ft.is_some() {
            map.set_default_ceiling(spatial.cell_height_ft)
                .map_err(|error| anyhow!(error.to_string()))?;
        }
        if !spatial.terrain_volumes.is_empty() {
            map.clear_terrain();
            for terrain in &spatial.terrain_volumes {
                map.add_terrain(terrain)
                    .map_err(|error| anyhow!(error.to_string()))?;
            }
        }
    }
    Ok(map)
}

pub fn fall_map_for_state(state: &EncounterState) -> Result<Option<FallMap>> {
    if !spatial_flight_enabled(state) {
        return Ok(None);
    }
    build_fall_map(state).map(Some)
}

/// Edition-specific "loses flight support" check for one combatant. A creature
/// that is not Flying/Hovering never falls from a flight trigger.
pub fn flight_check(state: &EncounterState, who: &str) -> Result<FlightCheck> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let rules = FlightRules::try_for_ruleset(Some(&state.content.ruleset))
        .map_err(|error| anyhow!(error.to_string()))?;
    let combatant = &state.combatants[index];
    let vertical = combatant.vertical_state();
    if !matches!(vertical.flight, FlightState::Flying | FlightState::Hovering) {
        return Ok(FlightCheck {
            edition: rules.edition,
            falls: false,
            reason: None,
            rule_kind: FallRuleKind::Raw,
            explain: vec![Explain::new(
                "flight-not-airborne",
                format!("{:?} is not airborne", vertical.flight),
            )],
        });
    }
    let capability = FlightCapability::from_speed(&combatant.creature.speed());
    let fly_speed_ft = effective_fly_speed(state, index);
    let conditions = FlightConditions {
        knocked_down: has_flag(state, index, ConditionFlag::StandUpHalfSpeed),
        incapacitated: has_flag(state, index, ConditionFlag::Incapacitated),
        speed_zero: has_flag(state, index, ConditionFlag::SpeedZero),
        fly_speed_zero: fly_speed_ft <= 0,
        // No engine flag expresses "otherwise deprived of movement"; a DM
        // ruling or P5 trigger can set it through the lower-level seam.
        deprived_of_movement: false,
    };
    let subject = FlightSubject {
        fly_speed_ft,
        hover: capability.can_hover(),
        held_aloft_by_magic: false,
        conditions,
    };
    Ok(crate::space::flight_fall_trigger(&subject, rules))
}

fn fall_start(state: &EncounterState, index: usize, map: &FallMap) -> Result<((i32, i32), i32)> {
    let combatant = &state.combatants[index];
    if let Some(pose) = combatant.pose {
        return Ok(((pose.cell.x, pose.cell.y), pose.cell.z));
    }
    let (x, y) = combatant
        .position_ref()
        .cell_coords()
        .ok_or_else(|| anyhow!("combatant `{}` has no grid position", combatant.name))?;
    let floor = map
        .floor_at(x, y)
        .ok_or_else(|| anyhow!("combatant `{}` is not on a fall column", combatant.name))?;
    let altitude = combatant.vertical_state().altitude_ft;
    Ok(((x, y), floor.saturating_add(altitude)))
}

/// Resolve a fall from the combatant's current pose/elevation without applying
/// damage. This is the read-only half of the P4 interface.
pub fn resolve_fall(state: &EncounterState, who: &str) -> Result<Option<FallResolution>> {
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let Some(map) = fall_map_for_state(state)? else {
        return Ok(None);
    };
    let rules = FlightRules::try_for_ruleset(Some(&state.content.ruleset))
        .map_err(|error| anyhow!(error.to_string()))?;
    let combatant = &state.combatants[index];
    let ((x, y), start_elevation_ft) = fall_start(state, index, &map)?;
    let height = HeightProfile::default()
        .resolve(combatant.creature.size(), None)
        .map_err(|error| anyhow!(error.to_string()))?;
    let start = SpatialPose {
        cell: CellPos {
            x,
            y,
            z: start_elevation_ft,
        },
        footprint: combatant.spatial_footprint(),
        facing: combatant.pose.and_then(|pose| pose.facing),
    };
    let resolution = FallResolver::new(&map, rules)
        .resolve_from(&start, height, FallCause::Unspecified)
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(Some(resolution))
}

fn landing_condition_id(state: &EncounterState) -> Option<String> {
    state
        .content
        .conditions
        .iter()
        .find(|(_, definition)| definition.flags.contains(&ConditionFlag::StandUpHalfSpeed))
        .map(|(id, _)| id.to_string())
}

/// Check totals supplied by the caller/CLI. P5 rolls the actual checks; P4
/// only applies them so the RAW/house-rule split stays explicit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FallOptions {
    pub landing_check_total: Option<i32>,
    pub liquid_landing_total: Option<i32>,
}

/// What the engine did with one fall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallReport {
    pub resolution: FallResolution,
    pub damage_rolled: i32,
    pub damage_taken: i32,
    pub landing: LandingOutcome,
    pub damage_halved_by_liquid: bool,
    pub landing_applied: bool,
}

/// Resolve and apply a fall: damage, landing condition, and vertical state.
/// The horizontal cell is unchanged for a vertical drop; forced movement that
/// changes cells must be wired by P5.
pub fn apply_fall(
    state: &mut EncounterState,
    who: &str,
    options: &FallOptions,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<Option<FallReport>> {
    let Some(resolution) = resolve_fall(state, who)? else {
        return Ok(None);
    };
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    // Mark the fall before damage/landing conditions are applied. A nested
    // condition (Prone) must not re-trigger the flight check.
    let mut falling = state.combatants[index].vertical_state();
    falling.flight = FlightState::Falling;
    falling
        .validate()
        .map_err(|error| anyhow!(error.to_string()))?;
    state.combatants[index].vertical = Some(falling);
    let name = state.combatants[index].name.clone();
    log.log(format_args!(
        "{name} falls {} ft ({})",
        resolution.fall_distance_ft,
        resolution.damage.expression()
    ));
    // Resolve the content id before touching HP so a missing landing condition
    // is an early error, never a partially applied fall.
    let flat_condition_id = if resolution.raw_flat_on_damage {
        Some(
            landing_condition_id(state)
                .ok_or_else(|| anyhow!("content has no condition carrying the stand-up flag"))?,
        )
    } else {
        None
    };
    let mut damage_rolled = 0;
    let mut damage_taken = 0;
    let mut damage_halved_by_liquid = false;
    if !resolution.damage.is_zero() {
        let expression = resolution.damage.expression();
        let parsed = expression
            .parse::<DiceExpr>()
            .with_context(|| format!("invalid fall damage expression `{expression}`"))?;
        let mut record = parsed.roll(rng);
        if resolution.liquid_halves_damage(options.liquid_landing_total) {
            record.total /= 2;
            record.text = format!("{} halved by liquid landing", record.text);
            damage_halved_by_liquid = true;
        }
        damage_rolled = record.total;
        damage_taken = apply_damage_rolls(
            state,
            index,
            index,
            vec![(record, resolution.damage.damage_type.to_string())],
            &[],
            false,
            rng,
            log,
        )?;
    }
    let landing = resolution.landing_outcome(damage_taken, options.landing_check_total);
    let landing_applied = if landing.condition == LandingCondition::LandedFlat {
        let condition_id = flat_condition_id
            .as_deref()
            .ok_or_else(|| anyhow!("content has no condition carrying the stand-up flag"))?;
        add_condition(
            state,
            who,
            condition_id,
            &AddConditionOptions::default(),
            rng,
            log,
        )?;
        state.combatants[index]
            .conditions
            .iter()
            .any(|active| active.condition.eq_ignore_ascii_case(condition_id))
    } else {
        false
    };
    let mut vertical = state.combatants[index].vertical_state();
    vertical.flight = FlightState::Grounded;
    vertical.altitude_ft = 0;
    vertical.fall_distance_ft = resolution.fall_distance_ft;
    vertical
        .validate()
        .map_err(|error| anyhow!(error.to_string()))?;
    state.combatants[index].vertical = Some(vertical);
    let footprint = state.combatants[index].spatial_footprint();
    let facing = state.combatants[index].pose.and_then(|pose| pose.facing);
    match state.combatants[index].pose.as_mut() {
        Some(pose) => pose.cell = resolution.landing.cell,
        None => {
            state.combatants[index].pose = Some(SpatialPose {
                cell: resolution.landing.cell,
                footprint,
                facing,
            });
        }
    }
    if let Some(Position::Cell { x, y }) = state.combatants[index].pos.as_mut() {
        *x = resolution.landing.cell.x;
        *y = resolution.landing.cell.y;
    }
    Ok(Some(FallReport {
        resolution,
        damage_rolled,
        damage_taken,
        landing,
        damage_halved_by_liquid,
        landing_applied,
    }))
}

fn roll_fall_check(
    state: &mut EncounterState,
    index: usize,
    ability: FallAbility,
    skill: &str,
    dc: i32,
    label: &str,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> i32 {
    let ability_name = match ability {
        FallAbility::Strength => AbilityName::Str,
        FallAbility::Dexterity => AbilityName::Dex,
    };
    let base = state.combatants[index].skill_bonus(skill);
    let (mode, advantages, disadvantages) =
        check_modifiers(state, index, Some(ability_name), skill);
    let result = roll_d20(
        state,
        index,
        base,
        mode,
        Vec::new(),
        advantages,
        disadvantages,
        rng,
    );
    let name = state.combatants[index].name.clone();
    log.log(format_args!(
        "{name} {skill} check: {} vs DC {dc} {} ({label})",
        result.text(),
        if result.total >= dc { "PASS" } else { "FAIL" }
    ));
    result.total
}

/// P5: the CLI rolls the R-09 landing check and the 2024 liquid-landing
/// reaction, then hands the totals to the P4 application path.
pub fn fall_with_checks(
    state: &mut EncounterState,
    who: &str,
    options: FallOptions,
    liquid_ability: FallAbility,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<Option<FallReport>> {
    let Some(resolution) = resolve_fall(state, who)? else {
        return Ok(None);
    };
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    let landing_check_total = match options.landing_check_total {
        Some(total) => Some(total),
        None => resolution.landing_check.as_ref().map(|check| {
            roll_fall_check(
                state,
                index,
                check.check.ability,
                check.check.skill.as_str(),
                check.check.dc,
                "R-09 house rule",
                rng,
                log,
            )
        }),
    };
    let liquid_landing_total = match options.liquid_landing_total {
        Some(total) => Some(total),
        None => resolution.liquid_landing.as_ref().map(|liquid| {
            let (ability, skill) = match liquid_ability {
                FallAbility::Strength => (FallAbility::Strength, "athletics"),
                FallAbility::Dexterity => (FallAbility::Dexterity, "acrobatics"),
            };
            let dc = liquid
                .checks
                .iter()
                .map(|check| check.dc)
                .max()
                .unwrap_or(15);
            roll_fall_check(
                state,
                index,
                ability,
                skill,
                dc,
                "2024 liquid landing reaction",
                rng,
                log,
            )
        }),
    };
    apply_fall(
        state,
        who,
        &FallOptions {
            landing_check_total,
            liquid_landing_total,
        },
        rng,
        log,
    )
}

/// P5: explicit take-off / altitude change. Horizontal movement stays in
/// `move_to_at`; this command owns the vertical axis and ceiling stops.
pub fn fly(
    state: &mut EncounterState,
    who: &str,
    altitude_ft: i32,
    log: &mut dyn LogSink,
) -> Result<()> {
    if altitude_ft < 0 {
        bail!("altitude must not be negative");
    }
    let index = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, index)?;
    let scene = state
        .scene
        .as_deref()
        .ok_or_else(|| anyhow!("this state has no scene"))?
        .clone();
    if scene.mode != SceneMode::Grid {
        bail!("fly needs a grid scene");
    }
    let map = build_fall_map(state)?;
    let pose = state.combatants[index]
        .spatial_pose()
        .ok_or_else(|| anyhow!("`{}` has no spatial pose", state.combatants[index].name))?;
    let floor = map
        .floor_at(pose.cell.x, pose.cell.y)
        .ok_or_else(|| anyhow!("`{}` is not on a fall column", state.combatants[index].name))?;
    let current = state.combatants[index].vertical_state();
    let capability = FlightCapability::from_speed(&state.combatants[index].creature.speed());
    let name = state.combatants[index].name.clone();
    let height = HeightProfile::default()
        .resolve(state.combatants[index].creature.size(), None)
        .map_err(|error| anyhow!(error.to_string()))?;
    let rules = FlightRules::try_for_ruleset(Some(&state.content.ruleset))
        .map_err(|error| anyhow!(error.to_string()))?;
    let airborne = matches!(current.flight, FlightState::Flying | FlightState::Hovering);
    // A grounded creature may already stand on a platform or terrain volume.
    let current_surface = pose.cell.z.max(floor);
    let start = SpatialPose {
        cell: CellPos {
            x: pose.cell.x,
            y: pose.cell.y,
            z: current_surface,
        },
        ..pose
    };
    if altitude_ft == 0 {
        let landing_z = if airborne {
            FallResolver::new(&map, rules)
                .vertical_move(&start, floor, height, &capability, current.flight)
                .map_err(|error| anyhow!(error.to_string()))?
                .allowed_elevation_ft
        } else {
            current_surface
        };
        let mut vertical = current;
        vertical.flight = FlightState::Grounded;
        vertical.altitude_ft = 0;
        vertical
            .validate()
            .map_err(|error| anyhow!(error.to_string()))?;
        state.combatants[index].vertical = Some(vertical);
        state.combatants[index].pose = Some(SpatialPose {
            cell: CellPos {
                x: pose.cell.x,
                y: pose.cell.y,
                z: landing_z,
            },
            ..pose
        });
        log.log(format_args!("{name} lands at {landing_z} ft"));
        return Ok(());
    }
    let action = if airborne {
        crate::space::FlightAction::Fly
    } else {
        crate::space::FlightAction::TakeOff
    };
    let next_flight =
        crate::space::transition_flight(current.flight, action, &capability, current.altitude_ft)
            .map_err(|error| anyhow!(error.to_string()))?;
    // An airborne creature's command altitude is above the map floor; a
    // ground take-off starts from the surface it stands on.
    let base = if airborne { floor } else { current_surface };
    let target = base.saturating_add(altitude_ft);
    let movement = FallResolver::new(&map, rules)
        .vertical_move(&start, target, height, &capability, next_flight)
        .map_err(|error| anyhow!(error.to_string()))?;
    if movement.blocked {
        if let Some(ceiling) = &movement.ceiling {
            log.log(format_args!(
                "{name} stops below ceiling `{}` at {} ft",
                ceiling.surface_id, movement.allowed_elevation_ft
            ));
        } else {
            log.log(format_args!(
                "{name} is stopped at {} ft",
                movement.allowed_elevation_ft
            ));
        }
    }
    let reference = if airborne { floor } else { current_surface };
    let mut vertical = current;
    vertical.flight = next_flight;
    vertical.altitude_ft = (movement.allowed_elevation_ft - reference).max(0);
    vertical
        .validate()
        .map_err(|error| anyhow!(error.to_string()))?;
    state.combatants[index].vertical = Some(vertical);
    state.combatants[index].pose = Some(SpatialPose {
        cell: CellPos {
            x: pose.cell.x,
            y: pose.cell.y,
            z: movement.allowed_elevation_ft,
        },
        ..pose
    });
    log.log(format_args!(
        "{name} flies to {} ft above the floor ({} ft absolute)",
        vertical.altitude_ft, movement.allowed_elevation_ft
    ));
    Ok(())
}

/// P5: a condition that removes flight support drops the creature immediately.
/// The return value is `Ok(())` even when the fall cannot be resolved (for
/// example a zone scene with no metric column); the log explains the limit so
/// gameplay is never silently changed.
fn maybe_fall_after_condition(
    state: &mut EncounterState,
    index: usize,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    if !spatial_flight_enabled(state) {
        return Ok(());
    }
    let vertical = state.combatants[index].vertical_state();
    if !matches!(vertical.flight, FlightState::Flying | FlightState::Hovering) {
        return Ok(());
    }
    let name = state.combatants[index].name.clone();
    let check = flight_check(state, &name)?;
    if !check.falls {
        return Ok(());
    }
    log.log(format_args!(
        "{name} loses flight support ({}, {})",
        check
            .reason
            .map(|reason| reason.as_str().to_string())
            .unwrap_or_else(|| "unstated".to_string()),
        check.edition
    ));
    // Resolve before mutating flight state. A zone scene (or a state with no
    // fall map) must fail explicitly and leave the creature Flying, never in a
    // half-applied Falling state. `apply_fall` marks Falling itself before it
    // applies the landing condition, so nested Prone cannot re-trigger here.
    fall_with_checks(
        state,
        &name,
        FallOptions::default(),
        FallAbility::Dexterity,
        rng,
        log,
    )
    .map_err(|error| {
        anyhow!("`{name}` lost flight support but the fall could not be resolved: {error}")
    })?;
    Ok(())
}

/// `interact <who> <object> [--option N]`.
pub fn interact(
    state: &mut EncounterState,
    who: &str,
    object_query: &str,
    option: Option<usize>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let actor = state
        .find_index(who)
        .map_err(|error| anyhow!(error.to_string()))?;
    ensure_can_act(state, actor)?;
    let scene = state
        .scene
        .clone()
        .ok_or_else(|| anyhow!("this state has no scene"))?;
    let needle = object_query.trim().to_ascii_lowercase();
    let matches: Vec<&crate::content::schema::SceneObject> = scene
        .objects
        .iter()
        .filter(|object| {
            object.id.eq_ignore_ascii_case(&needle)
                || object
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&needle))
        })
        .collect();
    let objects = if matches.is_empty() {
        scene
            .objects
            .iter()
            .filter(|object| {
                object.id.to_ascii_lowercase().starts_with(&needle)
                    || object
                        .name
                        .as_deref()
                        .is_some_and(|name| name.to_ascii_lowercase().starts_with(&needle))
            })
            .collect::<Vec<_>>()
    } else {
        matches
    };
    let object = match objects.as_slice() {
        [object] => (*object).clone(),
        [] => bail!(
            "no scene object matches `{object_query}` (objects: {})",
            scene
                .objects
                .iter()
                .map(|object| object.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => bail!(
            "scene object `{object_query}` is ambiguous; matches: {}",
            objects
                .iter()
                .map(|object| object.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    if spatial_query_enabled(state)
        && state
            .space()
            .is_some_and(|space| space.mode() == SceneMode::Grid)
    {
        let actor_zone = state
            .space()
            .and_then(|space| space.zone_of(&state.combatants[actor].position()));
        if actor_zone.as_deref() != Some(object.zone.as_str()) {
            bail!(
                "`{}` is not in reach of scene object `{}` (object zone `{}`)",
                state.combatants[actor].name,
                object.id,
                object.zone
            );
        }
    }
    let actor_name = state.combatants[actor].name.clone();
    log.log(format_args!(
        "{actor_name} interacts with {}",
        object.name.as_deref().unwrap_or(&object.id)
    ));
    if let Some(manual) = &object.manual {
        log.log(format_args!("MANUAL: {manual}"));
    }
    let Some(interact) = object.interact.clone() else {
        if object.manual.is_some() {
            return Ok(());
        }
        bail!("scene object `{}` has no interaction", object.id);
    };
    let context = EffectContext {
        actor,
        target: Some(actor),
        source: None,
        attack: None,
        option,
        crit: false,
        damage_dealt: 0,
        damage_scale: 1.0,
        has_advantage: false,
        has_disadvantage: false,
    };
    run_interaction(state, &context, &interact, option, rng, log)
}

fn run_interaction(
    state: &mut EncounterState,
    context: &EffectContext,
    interact: &crate::content::schema::Interact,
    option: Option<usize>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    if let Some(options) = &interact.options {
        if options.is_empty() {
            bail!("interaction has an empty options list");
        }
        let index = match option {
            Some(option) if option >= 1 && option <= options.len() => option - 1,
            Some(option) => bail!(
                "option {option} is out of range; choose 1-{}: {}",
                options.len(),
                describe_interact_options(options)
            ),
            None if options.len() == 1 => 0,
            None => bail!(
                "this interaction requires --option N (1-{}): {}",
                options.len(),
                describe_interact_options(options)
            ),
        };
        let option = &options[index];
        if let Some(effects) = &option.effects {
            apply_effects(state, context, effects, rng, log)?;
        }
        if let Some(check) = &option.check {
            let success = roll_check_spec(state, context.actor, check, rng, log)?;
            let effects = if success {
                &option.on_success
            } else {
                &option.on_fail
            };
            if let Some(effects) = effects {
                apply_effects(state, context, effects, rng, log)?;
            }
        } else {
            let effects = option.on_success.as_ref().or(option.on_fail.as_ref());
            if let Some(effects) = effects {
                apply_effects(state, context, effects, rng, log)?;
            }
        }
        return Ok(());
    }
    if let Some(effects) = &interact.effects {
        apply_effects(state, context, effects, rng, log)?;
    }
    if let Some(check) = &interact.check {
        let success = roll_check_spec(state, context.actor, check, rng, log)?;
        let effects = if success {
            &interact.on_success
        } else {
            &interact.on_fail
        };
        if let Some(effects) = effects {
            apply_effects(state, context, effects, rng, log)?;
        }
    }
    Ok(())
}

fn describe_interact_options(options: &[crate::content::schema::InteractOption]) -> String {
    options
        .iter()
        .enumerate()
        .map(|(index, option)| format!("{}: {}", index + 1, option.name))
        .collect::<Vec<_>>()
        .join("; ")
}

fn roll_check_spec(
    state: &mut EncounterState,
    who: usize,
    check: &crate::content::schema::CheckSpec,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<bool> {
    let (label, base, ability) = if let Some(skill) = &check.skill {
        (
            skill.clone(),
            state.combatants[who].skill_bonus(skill),
            check.ability.or_else(|| ability_from_text(skill)),
        )
    } else if let Some(ability) = check.ability {
        (
            ability.as_str().to_string(),
            state.combatants[who].creature.ability_modifier(ability),
            Some(ability),
        )
    } else {
        bail!("interaction check needs `skill` or `ability`");
    };
    let (mode, advantages, disadvantages) = check_modifiers(state, who, ability, &label);
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
    let name = state.combatants[who].name.clone();
    let success = result.total >= check.dc;
    log.log(format_args!(
        "{name} {label} check: {} vs DC {} {}",
        result.text(),
        check.dc,
        if success { "PASS" } else { "FAIL" }
    ));
    Ok(success)
}

/// Compact text description of the scene (no map drawing).
pub fn scene_text(state: &EncounterState) -> Vec<String> {
    let Some(scene) = state.scene.as_ref() else {
        return vec!["(this state has no scene)".to_string()];
    };
    let mut lines = Vec::new();
    let light = scene
        .light
        .map(|light| format!("{light:?}").to_ascii_lowercase())
        .unwrap_or_else(|| "bright".to_string());
    lines.push(format!(
        "Scene {} ({:?}) light {light}",
        scene.display_name(),
        scene.mode
    ));
    if state.space().is_some() {
        if scene.grid.is_some() {
            let map = crate::space::GridMap::parse(scene);
            if let Some(map) = map {
                lines.push(format!(
                    "Grid {} ft per cell, {}x{} cells",
                    map.cell, map.width, map.height
                ));
                for zone in map.zone_ids() {
                    let cells = map
                        .cells_in_zone(&zone)
                        .into_iter()
                        .map(|(x, y)| format!("({x},{y})"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    lines.push(format!("  grid zone {zone}: {cells}"));
                }
                if let Some(grid) = scene.grid.as_ref() {
                    if !grid.zones.is_empty() {
                        let mapped = grid
                            .zones
                            .iter()
                            .map(|(zone, character)| format!("{zone}={character}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        lines.push(format!("  grid zone chars: {mapped}"));
                    }
                }
            }
        } else {
            for zone in &scene.zones {
                let cover = match zone.cover {
                    Cover::None => "none",
                    Cover::Half => "half",
                    Cover::ThreeQuarters => "three-quarters",
                    Cover::Total => "total",
                };
                let zone_light = zone
                    .light
                    .map(|light| format!("{light:?}").to_ascii_lowercase())
                    .unwrap_or_else(|| "scene".to_string());
                lines.push(format!(
                    "  zone {}: cover {cover}, elevation {}, light {zone_light}",
                    zone.id,
                    zone.elevation.unwrap_or(0)
                ));
            }
        }
        if !scene.edges.is_empty() {
            for edge in &scene.edges {
                let mut flags = Vec::new();
                if edge.difficult {
                    flags.push("difficult".to_string());
                }
                if edge.one_way {
                    flags.push("one-way".to_string());
                }
                if let Some(climb) = &edge.climb {
                    flags.push(format!("climb {} DC {}", climb.skill, climb.dc));
                }
                if let Some(door) = &edge.door {
                    flags.push(if door.locked {
                        "locked door".to_string()
                    } else {
                        "door".to_string()
                    });
                }
                let suffix = if flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", flags.join(", "))
                };
                lines.push(format!(
                    "  edge {} <-> {} {} ft{suffix}",
                    edge.a, edge.b, edge.distance
                ));
            }
        }
        for object in &scene.objects {
            let interaction = match &object.interact {
                None => "manual".to_string(),
                Some(interact) => match &interact.options {
                    Some(options) => format!("{} option(s)", options.len()),
                    None => "direct".to_string(),
                },
            };
            lines.push(format!(
                "  object {} in {} ({}), interact: {interaction}",
                object.id,
                object.zone,
                object.name.as_deref().unwrap_or(&object.id)
            ));
        }
        for (index, trigger) in scene.triggers.iter().enumerate() {
            let key = EncounterState::trigger_key(index, trigger);
            let mut zones = Vec::new();
            if let Some(zone) = &trigger.zone {
                zones.push(zone.clone());
            }
            zones.extend(trigger.zones.clone());
            let fired = state
                .triggers
                .get(&key)
                .map(|runtime| runtime.fired)
                .unwrap_or(false);
            let side = trigger
                .side
                .as_ref()
                .map(|side| format!(" side {}", side.as_str()))
                .unwrap_or_default();
            lines.push(format!(
                "  trigger {} on {:?} zone {}{} once={} {}{}",
                key,
                trigger.on,
                zones.join(","),
                side,
                trigger.once,
                if state.trigger_enabled(&key) {
                    "enabled"
                } else {
                    "disabled"
                },
                if fired { " fired" } else { "" }
            ));
        }
        lines.push("Positions:".to_string());
        for index in status_order(state) {
            let combatant = &state.combatants[index];
            let where_text = match combatant.cell() {
                Some([x, y]) => format!("({x},{y}) [{}]", combatant.zone),
                None => combatant.zone.clone(),
            };
            lines.push(format!(
                "  {} {} {} HP at {where_text}",
                combatant.name,
                combatant.side.as_str(),
                combatant.hp
            ));
        }
    } else {
        lines.push("  (no usable space for this scene)".to_string());
    }
    lines
}

pub fn activate(
    state: &mut EncounterState,
    query: &str,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let matches = state.find_activatable(query);
    if matches.is_empty() {
        bail!("no dormant creature matches `{query}`");
    }
    for index in matches {
        if !state.combatants[index].dormant {
            log.log(format_args!(
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
        log.log(format_args!(
            "{} activates: {}",
            state.combatants[index].name,
            roll.text()
        ));
        insert_into_initiative(state, index);
    }
    // Activation can reorder initiative but never adds combatants today; keep
    // the name lookup in sync if that changes.
    state.refresh_name_index();
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
    log: &mut dyn LogSink,
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
    log: &mut dyn LogSink,
) -> Result<()> {
    let name = state.combatants[index].name.clone();
    if !state.combatants[index].makes_death_saves() {
        bail!("`{name}` is not a player character or party companion");
    }
    if state.combatants[index].is_dead() {
        bail!("`{name}` is already dead");
    }
    if state.combatants[index].hp > 0 {
        bail!("`{name}` is not dying");
    }
    if state.combatants[index].death_saves.stable {
        log.log(format_args!("{name} is stable"));
        return Ok(());
    }
    let mode = if exhaustion_attack_save_disadvantage(state, index) {
        RollMode::Disadvantage
    } else {
        RollMode::Normal
    };
    let roll = roll_d20(
        state,
        index,
        0,
        mode,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        rng,
    );
    log.log(format_args!("{name} death save: {}", roll.text()));
    if roll.nat20 {
        state.combatants[index].hp = 1;
        state.combatants[index].death_saves = DeathSaves::default();
        revive_from_zero(state, index, log);
        log.log(format_args!("{name} regains 1 HP"));
        return Ok(());
    }
    if roll.nat1 {
        state.combatants[index].death_saves.failures = state.combatants[index]
            .death_saves
            .failures
            .saturating_add(2);
        log.log(format_args!(
            "{name} critical failure (2 death save failures)"
        ));
    } else if roll.total >= 10 {
        state.combatants[index].death_saves.successes = state.combatants[index]
            .death_saves
            .successes
            .saturating_add(1);
        log.log(format_args!("{name} succeeds on the death save"));
    } else {
        state.combatants[index].death_saves.failures = state.combatants[index]
            .death_saves
            .failures
            .saturating_add(1);
        log.log(format_args!("{name} fails the death save"));
    }
    if state.combatants[index].death_saves.failures >= 3 {
        state.combatants[index].death_saves.dead = true;
        state.combatants[index].dead = true;
        on_death(state, index, log);
        log.log(format_args!("{name} dies"));
    } else if state.combatants[index].death_saves.successes >= 3 {
        state.combatants[index].death_saves.stable = true;
        state.combatants[index].death_saves.successes = 0;
        state.combatants[index].death_saves.failures = 0;
        log.log(format_args!("{name} is stable"));
    }
    Ok(())
}

/// CLI `rest short|long`.
///
/// `all` means every living party-side combatant; enemies and the dead never
/// rest. A short rest can spend `hit_dice` Hit Dice (die + Con each, min 0).
/// A long rest restores HP and resources, regains half the Hit Dice (min 1)
/// and ends the creature's concentration.
pub fn rest(
    state: &mut EncounterState,
    kind: RestKind,
    who: Option<&str>,
    hit_dice: Option<u32>,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let all = match who {
        Some(name) => name.eq_ignore_ascii_case("all"),
        None => true,
    };
    let targets: Vec<usize> = if all {
        (0..state.combatants.len())
            .filter(|&i| state.combatants[i].side.as_str() == "party")
            .collect()
    } else {
        vec![state
            .find_index(who.unwrap_or_default())
            .map_err(|error| anyhow!(error.to_string()))?]
    };
    if hit_dice.is_some() && kind == RestKind::Long {
        bail!("--hd is only for short rests");
    }
    for index in targets {
        let name = state.combatants[index].name.clone();
        if state.combatants[index].is_dead() {
            log.log(format_args!("{name} is dead and cannot rest"));
            continue;
        }
        match kind {
            RestKind::Short => {
                for (id, resource) in state.combatants[index].resources.iter_mut() {
                    if id == "hit-dice" {
                        continue;
                    }
                    if resource.recharge == Recharge::Short {
                        resource.current = resource.max;
                    } else if let Some(regain) = resource.short_rest_regain {
                        resource.restore(regain);
                    }
                }
                log.log(format_args!("{name} takes a short rest"));
                if let Some(n) = hit_dice.filter(|n| *n > 0) {
                    spend_hit_dice(state, index, n, rng, log)?;
                }
            }
            RestKind::Long => {
                if state.concentration.contains_key(&name) {
                    break_concentration(state, index, log);
                }
                let level = state.combatants[index].exhaustion_level;
                if level > 0 {
                    let reduction = state
                        .content
                        .rules
                        .exhaustion
                        .as_ref()
                        .map(|rules| rules.long_rest_reduction())
                        .unwrap_or(1);
                    let next = level.saturating_sub(reduction);
                    set_exhaustion_level(state, index, next, "long rest", log)?;
                }
                let max_hp = effective_max_hp(state, index);
                let combatant = &mut state.combatants[index];
                for (id, resource) in combatant.resources.iter_mut() {
                    if id == "hit-dice" {
                        let regain = (resource.max / 2).max(1);
                        resource.current = (resource.current + regain).min(resource.max);
                    } else {
                        resource.current = resource.max;
                    }
                }
                combatant.hp = max_hp;
                combatant.temp_hp = 0;
                combatant.death_saves = DeathSaves::default();
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
                log.log(format_args!("{name} takes a long rest"));
            }
        }
    }
    log_outcome(state, log);
    Ok(())
}

/// Hit-die size of a combatant's `hit-dice` pool, if it has one.
pub fn hit_die_size(combatant: &CombatantState) -> Option<u32> {
    combatant.creature.resources.as_ref().and_then(|defs| {
        let def = defs.get("hit-dice")?;
        def.die.filter(|d| *d >= 2).or_else(|| {
            let notes = def.notes.as_deref()?;
            let at = notes.find('d')?;
            let digits: String = notes[at + 1..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            digits.parse().ok().filter(|d| *d >= 2)
        })
    })
}

fn spend_hit_dice(
    state: &mut EncounterState,
    index: usize,
    count: u32,
    rng: &mut dyn DiceRng,
    log: &mut dyn LogSink,
) -> Result<()> {
    let name = state.combatants[index].name.clone();
    let die = hit_die_size(&state.combatants[index])
        .ok_or_else(|| anyhow!("`{name}` has no hit-dice resource with a die size"))?;
    let available = state.combatants[index]
        .resources
        .get("hit-dice")
        .map_or(0, |r| r.current.max(0) as u32);
    if count > available {
        bail!("`{name}` has only {available} Hit Dice left");
    }
    let con = state.combatants[index].ability_modifier(AbilityName::Con);
    let mut total = 0;
    for _ in 0..count {
        let roll = rng.roll_die(die) as i32;
        let gain = (roll + con).max(0);
        log.log(format_args!(
            "{name} spends a Hit Die: d{die} {roll} {con:+} = {gain}"
        ));
        total += gain;
    }
    let max_hp = effective_max_hp(state, index);
    let combatant = &mut state.combatants[index];
    if let Some(r) = combatant.resources.get_mut("hit-dice") {
        r.current -= count as i32;
    }
    let before = combatant.hp;
    combatant.hp = (combatant.hp + total).min(max_hp);
    if before <= 0 && combatant.hp > 0 {
        combatant.death_saves = DeathSaves::default();
    }
    log.log(format_args!(
        "{name} regains {} HP ({}/{})",
        combatant.hp - before,
        combatant.hp,
        combatant.max_hp
    ));
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
    let max_hp = effective_max_hp(state, index);
    let hp = if combatant.temp_hp > 0 {
        format!("{}/{} (+{})", combatant.hp, max_hp, combatant.temp_hp)
    } else {
        format!("{}/{}", combatant.hp, max_hp)
    };
    let exhaustion_id = exhaustion_condition_id(state);
    let conditions = if combatant.conditions.is_empty() {
        "none".to_string()
    } else {
        combatant
            .conditions
            .iter()
            .map(|condition| {
                if condition.condition.eq_ignore_ascii_case(exhaustion_id) {
                    return format!(
                        "{}(level {})",
                        condition.condition, combatant.exhaustion_level
                    );
                }
                match &condition.duration {
                    Some(duration) => format!("{}({})", condition.condition, duration.describe()),
                    None => condition.condition.clone(),
                }
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
    let saves = if !combatant.makes_death_saves() {
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
    let vertical = combatant.vertical_state();
    let location = match combatant.cell() {
        Some([x, y]) if vertical.flight != FlightState::Grounded || vertical.altitude_ft != 0 => {
            format!(
                "{} ({x},{y}) @ {} ft {:?}",
                combatant.zone, vertical.altitude_ft, vertical.flight
            )
        }
        Some([x, y]) => format!("{} ({x},{y})", combatant.zone),
        None => combatant.zone.clone(),
    };
    let ready = combatant
        .ready
        .as_ref()
        .map(|ready| {
            format!(
                " | ready: {} ({} trigger)",
                ready.description,
                ready.trigger.as_str()
            )
        })
        .unwrap_or_default();
    format!(
        "{marker}{initiative} {} {} HP {hp} AC {} {}{dormant}{concentration} | conditions: {conditions} | resources: {resources} | saves: {saves}{ready}",
        combatant.name,
        combatant.side.as_str(),
        armor_class(state, index),
        location
    )
}
