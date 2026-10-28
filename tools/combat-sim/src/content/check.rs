//! Static validation for loaded content.
//!
//! The checker deliberately collects every problem instead of stopping at the
//! first one. Errors are broken references and malformed ids; TODO notes are
//! warnings so they stay visible without failing a load.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::expr;
use super::loader::{ContentRegistry, ContentSource};
use super::schema::{
    Ability, AbilityCost, AbilityEntry, AbilityName, ActionResolution, Activation, AiAssignment,
    AiParams, Attack, AttackEffect, ConditionDef, Creature, Duration, Effect, ExprValue,
    MasteryDef, Modifier, Predicate, Scene, SceneMode, SceneTrigger, Trigger,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One validation message tied to the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
    pub source: Option<ContentSource>,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
        };
        match &self.source {
            Some(source) => write!(f, "{label} {source}: {}", self.message),
            None => write!(f, "{label}: {}", self.message),
        }
    }
}

/// The complete result of `content check`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    pub issues: Vec<Issue>,
}

impl CheckReport {
    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Issue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == Severity::Warning)
    }

    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    pub fn warning_count(&self) -> usize {
        self.warnings().count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    fn error(&mut self, source: Option<&ContentSource>, message: impl Into<String>) {
        self.issues.push(Issue {
            severity: Severity::Error,
            message: message.into(),
            source: source.cloned(),
        });
    }

    fn warning(&mut self, source: Option<&ContentSource>, message: impl Into<String>) {
        self.issues.push(Issue {
            severity: Severity::Warning,
            message: message.into(),
            source: source.cloned(),
        });
    }
}

/// Runs every static check and returns all errors and warnings.
pub fn check(registry: &ContentRegistry) -> CheckReport {
    let mut report = CheckReport::default();

    check_ids_and_todos(registry, &mut report);
    check_condition_implies(registry, &mut report);
    check_rules(registry, &mut report);
    check_exhaustion(registry, &mut report);
    check_condition_behaviors(registry, &mut report);
    check_combat_actions(registry, &mut report);
    check_ready(registry, &mut report);

    let (resolved, unresolved) = resolve_creatures(registry, &mut report);
    let creature_refs = collect_creature_refs(registry, &resolved);

    check_global_abilities(registry, &creature_refs, &mut report);
    check_conditions(registry, &creature_refs, &mut report);
    check_masteries(registry, &creature_refs, &mut report);
    check_ai_templates(registry, &mut report, &creature_refs);
    check_scenes(registry, &mut report, &creature_refs);
    check_obscurement(registry, &mut report);
    check_encounters(registry, &mut report);
    check_creature_content(registry, &resolved, &creature_refs, &mut report);
    check_unresolved_creature_content(registry, &unresolved, &creature_refs, &mut report);

    report
}

// ---------------------------------------------------------------------------
// Ids and TODO notes
// ---------------------------------------------------------------------------

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        })
}

/// Whether a tag names one of the eight schools of magic (the legacy school
/// tag the SRD import writes).
fn is_school_tag(tag: &str) -> bool {
    [
        "abjuration",
        "conjuration",
        "divination",
        "enchantment",
        "evocation",
        "illusion",
        "necromancy",
        "transmutation",
    ]
    .contains(&tag)
}

fn check_id(report: &mut CheckReport, source: Option<&ContentSource>, kind: &str, id: &str) {
    if !valid_id(id) {
        report.error(
            source,
            format!(
                "{kind} id `{id}` is invalid; ids must be lowercase words separated by hyphens"
            ),
        );
    }
}

fn check_ids_and_todos(registry: &ContentRegistry, report: &mut CheckReport) {
    for (id, sourced) in registry.conditions.iter_sourced() {
        check_id(report, Some(&sourced.source), "condition", id);
        add_todos(
            report,
            Some(&sourced.source),
            &format!("condition `{id}`"),
            &sourced.item.todo,
        );
    }
    for (id, sourced) in registry.masteries.iter_sourced() {
        check_id(report, Some(&sourced.source), "mastery", id);
        add_todos(
            report,
            Some(&sourced.source),
            &format!("mastery `{id}`"),
            &sourced.item.todo,
        );
        for (index, trigger) in sourced.item.triggers.iter().enumerate() {
            add_todos(
                report,
                Some(&sourced.source),
                &format!("mastery `{id}` trigger #{}", index + 1),
                &trigger.todo,
            );
        }
    }
    for (id, sourced) in registry.abilities.iter_sourced() {
        check_id(report, Some(&sourced.source), "ability", id);
        add_todos(
            report,
            Some(&sourced.source),
            &format!("ability `{id}`"),
            &sourced.item.todo,
        );
        for (index, trigger) in sourced.item.triggers.iter().enumerate() {
            add_todos(
                report,
                Some(&sourced.source),
                &format!("ability `{id}` trigger #{}", index + 1),
                &trigger.todo,
            );
        }
    }
    for (id, sourced) in registry.creatures.iter_sourced() {
        check_id(report, Some(&sourced.source), "creature", id);
        if let Some(todos) = &sourced.item.todo {
            add_todos(
                report,
                Some(&sourced.source),
                &format!("creature `{id}`"),
                todos,
            );
        }
        if let Some(attacks) = &sourced.item.attacks {
            for attack in attacks {
                check_id(report, Some(&sourced.source), "attack", &attack.id);
                add_todos(
                    report,
                    Some(&sourced.source),
                    &format!("creature `{id}` attack `{}`", attack.id),
                    &attack.todo,
                );
            }
        }
        for entry in sourced
            .item
            .actions
            .iter()
            .flatten()
            .chain(sourced.item.features.iter().flatten())
        {
            if let AbilityEntry::Inline(ability) = entry {
                check_id(report, Some(&sourced.source), "inline ability", &ability.id);
                add_todos(
                    report,
                    Some(&sourced.source),
                    &format!("creature `{id}` ability `{}`", ability.id),
                    &ability.todo,
                );
                for (index, trigger) in ability.triggers.iter().enumerate() {
                    add_todos(
                        report,
                        Some(&sourced.source),
                        &format!(
                            "creature `{id}` ability `{}` trigger #{}",
                            ability.id,
                            index + 1
                        ),
                        &trigger.todo,
                    );
                }
            }
        }
    }
    for (id, sourced) in registry.ai_templates.iter_sourced() {
        check_id(report, Some(&sourced.source), "ai-template", id);
        add_todos(
            report,
            Some(&sourced.source),
            &format!("ai-template `{id}`"),
            &sourced.item.todo,
        );
    }
    for (id, sourced) in registry.scenes.iter_sourced() {
        check_id(report, Some(&sourced.source), "scene", id);
        add_todos(
            report,
            Some(&sourced.source),
            &format!("scene `{id}`"),
            &sourced.item.todo,
        );
        for zone in &sourced.item.zones {
            check_id(report, Some(&sourced.source), "zone", &zone.id);
            add_todos(
                report,
                Some(&sourced.source),
                &format!("scene `{id}` zone `{}`", zone.id),
                &zone.todo,
            );
        }
        for edge in &sourced.item.edges {
            add_todos(
                report,
                Some(&sourced.source),
                &format!("scene `{id}` edge {} <-> {}", edge.a, edge.b),
                &edge.todo,
            );
        }
        for object in &sourced.item.objects {
            check_id(report, Some(&sourced.source), "object", &object.id);
            add_todos(
                report,
                Some(&sourced.source),
                &format!("scene `{id}` object `{}`", object.id),
                &object.todo,
            );
        }
        for trigger in &sourced.item.triggers {
            if let Some(trigger_id) = &trigger.id {
                check_id(report, Some(&sourced.source), "trigger", trigger_id);
            }
            add_todos(
                report,
                Some(&sourced.source),
                &format!(
                    "scene `{id}` trigger `{}`",
                    trigger.id.as_deref().unwrap_or("?")
                ),
                &trigger.todo,
            );
        }
        if let Some(grid) = &sourced.item.grid {
            for zone_id in grid.zones.keys() {
                check_id(report, Some(&sourced.source), "zone", zone_id);
            }
            for (character, cell) in &grid.legend {
                if let Some(zone_id) = &cell.zone {
                    check_id(
                        report,
                        Some(&sourced.source),
                        &format!("zone (grid legend `{character}`)"),
                        zone_id,
                    );
                }
            }
        }
    }
    for (id, sourced) in registry.encounters.iter_sourced() {
        check_id(report, Some(&sourced.source), "encounter", id);
        add_todos(
            report,
            Some(&sourced.source),
            &format!("encounter `{id}`"),
            &sourced.item.todo,
        );
        for member in &sourced.item.members {
            add_todos(
                report,
                Some(&sourced.source),
                &format!("encounter `{id}` member `{}`", member.creature),
                &member.todo,
            );
        }
    }
}

fn add_todos(
    report: &mut CheckReport,
    source: Option<&ContentSource>,
    context: &str,
    todos: &[String],
) {
    for todo in todos {
        report.warning(source, format!("{context}: TODO: {todo}"));
    }
}

// ---------------------------------------------------------------------------
// Creature resolution and reference sets
// ---------------------------------------------------------------------------

fn resolve_creatures(
    registry: &ContentRegistry,
    report: &mut CheckReport,
) -> (BTreeMap<String, Creature>, BTreeSet<String>) {
    let mut resolved = BTreeMap::new();
    let mut unresolved = BTreeSet::new();
    for (id, _) in registry.creatures.iter() {
        let source = registry.creatures.source(id);
        match registry.resolve_creature(id) {
            Ok(creature) => {
                resolved.insert(id.to_string(), creature);
            }
            Err(error) => {
                report.error(source, format!("creature `{id}`: {error}"));
                unresolved.insert(id.to_string());
            }
        }
    }
    (resolved, unresolved)
}

fn collect_creature_refs(
    registry: &ContentRegistry,
    resolved: &BTreeMap<String, Creature>,
) -> BTreeSet<String> {
    let mut references = BTreeSet::new();
    for id in registry.creatures.ids() {
        references.insert(id.to_string());
        if let Some(raw) = registry.creatures.get(id) {
            if let Some(tags) = &raw.tags {
                references.extend(tags.iter().cloned());
            }
        }
    }
    for creature in resolved.values() {
        if let Some(tags) = &creature.tags {
            references.extend(tags.iter().cloned());
        }
    }
    references
}

fn check_condition_implies(registry: &ContentRegistry, report: &mut CheckReport) {
    for (id, sourced) in registry.conditions.iter_sourced() {
        for implied in &sourced.item.implies {
            if registry.conditions.get(implied).is_none() {
                report.error(
                    Some(&sourced.source),
                    format!("condition `{id}` implies unknown condition `{implied}`"),
                );
            }
        }
    }
}

fn check_rules(registry: &ContentRegistry, report: &mut CheckReport) {
    let source = registry.rules_source.as_ref();
    if let Some(conditions) = &registry.rules.dying_conditions {
        for condition in conditions {
            check_condition(
                report,
                registry,
                source,
                condition,
                "rules.dying_conditions",
            );
        }
    }
    if let Some(condition) = &registry.rules.dead_monster_condition {
        check_condition(
            report,
            registry,
            source,
            condition,
            "rules.dead_monster_condition",
        );
    }
    if let Some(actions) = &registry.rules.basic_actions {
        for (action, condition) in actions {
            check_condition(
                report,
                registry,
                source,
                condition,
                &format!("rules.basic_actions.{action}"),
            );
        }
    }
    if let Some(condition) = &registry.rules.concentration_condition {
        check_condition(
            report,
            registry,
            source,
            condition,
            "rules.concentration_condition",
        );
    }
}

fn check_exhaustion(registry: &ContentRegistry, report: &mut CheckReport) {
    let Some(rules) = &registry.rules.exhaustion else {
        return;
    };
    let source = registry.rules_source.as_ref();
    let condition = rules.condition_id();
    check_condition(
        report,
        registry,
        source,
        condition,
        "rules.exhaustion.condition",
    );
    let max = rules.max_level();
    if max == 0 {
        report.error(source, "rules.exhaustion.max_level must be positive");
    }
    if rules.death_level() == 0 || rules.death_level() > max {
        report.error(
            source,
            format!("rules.exhaustion.death_level must be between 1 and max_level ({max})"),
        );
    }
    if rules.long_rest_reduction() == 0 {
        report.error(
            source,
            "rules.exhaustion.long_rest_reduction must be positive",
        );
    }
    for (name, value) in [
        (
            "rules.exhaustion.check_disadvantage_from",
            rules.check_disadvantage_from,
        ),
        (
            "rules.exhaustion.speed_halved_from",
            rules.speed_halved_from,
        ),
        (
            "rules.exhaustion.attack_save_disadvantage_from",
            rules.attack_save_disadvantage_from,
        ),
        (
            "rules.exhaustion.hp_max_halved_from",
            rules.hp_max_halved_from,
        ),
        ("rules.exhaustion.speed_zero_from", rules.speed_zero_from),
    ] {
        if let Some(value) = value {
            if value == 0 || value > max {
                report.error(
                    source,
                    format!("{name} must be between 1 and max_level ({max})"),
                );
            }
        }
    }
    if rules
        .d20_penalty_per_level
        .is_some_and(|penalty| penalty < 0)
    {
        report.error(
            source,
            "rules.exhaustion.d20_penalty_per_level must not be negative",
        );
    }
    if rules
        .speed_penalty_per_level
        .is_some_and(|penalty| penalty < 0)
    {
        report.error(
            source,
            "rules.exhaustion.speed_penalty_per_level must not be negative",
        );
    }
}

fn check_ready(registry: &ContentRegistry, report: &mut CheckReport) {
    let Some(ready) = &registry.rules.ready else {
        return;
    };
    let source = registry.rules_source.as_ref();
    let max = ready.max_responses();
    if max != 1 {
        report.error(
            source,
            format!(
                "rules.ready.max_responses must be 1 (one prepared response per combatant); got {max}"
            ),
        );
    }
    if !ready.expires_at_start_of_next_turn() {
        report.error(source, "rules.ready.expires must be `start_of_next_turn`");
    }
}

fn check_condition_behaviors(registry: &ContentRegistry, report: &mut CheckReport) {
    let Some(behaviors) = &registry.rules.condition_behaviors else {
        return;
    };
    let source = registry.rules_source.as_ref();
    for (condition, behavior) in behaviors {
        if registry.conditions.get(condition).is_none() {
            report.error(
                source,
                format!("rules.condition_behaviors: unknown condition `{condition}`"),
            );
        }
        if behavior.is_empty() {
            report.warning(
                source,
                format!("rules.condition_behaviors.{condition} has no behaviour flags"),
            );
        }
    }
}

fn check_condition(
    report: &mut CheckReport,
    registry: &ContentRegistry,
    source: Option<&ContentSource>,
    condition: &str,
    context: &str,
) {
    if registry.conditions.get(condition).is_none() {
        report.error(
            source,
            format!("{context}: unknown condition `{condition}`"),
        );
    }
}

fn check_combat_actions(registry: &ContentRegistry, report: &mut CheckReport) {
    let Some(actions) = &registry.rules.combat_actions else {
        return;
    };
    let source = registry.rules_source.as_ref();

    if let Some(unarmed) = &actions.unarmed_strike {
        match &unarmed.damage {
            Some(damage) => {
                check_action_expression(report, source, damage, "unarmed_strike.damage")
            }
            None => report.error(
                source,
                "rules.combat_actions.unarmed_strike.damage is required",
            ),
        }
        if unarmed.reach.is_some_and(|reach| reach <= 0) {
            report.error(
                source,
                "rules.combat_actions.unarmed_strike.reach must be positive",
            );
        }
    }

    if let Some(grapple) = &actions.grapple {
        if let Some(condition) = &grapple.condition {
            check_condition(
                report,
                registry,
                source,
                condition,
                "rules.combat_actions.grapple.condition",
            );
        } else {
            report.error(source, "rules.combat_actions.grapple.condition is required");
        }
        check_resolution(
            report,
            source,
            grapple.resolution,
            grapple.attacker_skill.as_deref(),
            grapple.defender_skills.as_deref(),
            grapple.save_abilities.as_deref(),
            grapple.dc.as_ref(),
            "grapple",
        );
        if grapple
            .max_size_difference
            .is_some_and(|difference| difference < 0)
        {
            report.error(
                source,
                "rules.combat_actions.grapple.max_size_difference must not be negative",
            );
        }
    }

    if let Some(shove) = &actions.shove {
        if let Some(condition) = &shove.prone_condition {
            check_condition(
                report,
                registry,
                source,
                condition,
                "rules.combat_actions.shove.prone_condition",
            );
        } else {
            report.error(
                source,
                "rules.combat_actions.shove.prone_condition is required",
            );
        }
        check_resolution(
            report,
            source,
            shove.resolution,
            shove.attacker_skill.as_deref(),
            shove.defender_skills.as_deref(),
            shove.save_abilities.as_deref(),
            shove.dc.as_ref(),
            "shove",
        );
        if shove
            .max_size_difference
            .is_some_and(|difference| difference < 0)
        {
            report.error(
                source,
                "rules.combat_actions.shove.max_size_difference must not be negative",
            );
        }
        if shove.distance.is_some_and(|distance| distance <= 0) {
            report.error(
                source,
                "rules.combat_actions.shove.distance must be positive",
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_resolution(
    report: &mut CheckReport,
    source: Option<&ContentSource>,
    resolution: Option<ActionResolution>,
    attacker_skill: Option<&str>,
    defender_skills: Option<&[String]>,
    save_abilities: Option<&[AbilityName]>,
    dc: Option<&ExprValue>,
    action: &str,
) {
    match resolution.unwrap_or_default() {
        ActionResolution::Contest => {
            if attacker_skill.is_none() {
                report.error(
                    source,
                    format!(
                        "rules.combat_actions.{action}.attacker_skill is required for a contest"
                    ),
                );
            }
            if defender_skills.is_none_or(|skills| skills.is_empty()) {
                report.error(
                    source,
                    format!(
                        "rules.combat_actions.{action}.defender_skills is required for a contest"
                    ),
                );
            }
        }
        ActionResolution::Save => {
            if save_abilities.is_none_or(|abilities| abilities.is_empty()) {
                report.error(
                    source,
                    format!("rules.combat_actions.{action}.save_abilities is required for a save"),
                );
            }
            match dc {
                Some(dc) => check_action_expression(report, source, dc, &format!("{action}.dc")),
                None => report.error(
                    source,
                    format!("rules.combat_actions.{action}.dc is required for a save"),
                ),
            }
        }
    }
}

fn check_action_expression(
    report: &mut CheckReport,
    source: Option<&ContentSource>,
    value: &ExprValue,
    context: &str,
) {
    let expr_context = expr::ExprContext::new()
        .with_prof(2)
        .with_level(1)
        .with_attack_mod(0);
    if let Err(error) = expr_context.evaluate(&value.as_text()) {
        report.error(source, format!("rules.combat_actions.{context}: {error}"));
    }
}

// ---------------------------------------------------------------------------
// Global ability / condition / mastery checks
// ---------------------------------------------------------------------------

fn check_global_abilities(
    registry: &ContentRegistry,
    creature_refs: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    for (id, sourced) in registry.abilities.iter_sourced() {
        let mut walker = Walker::new(
            registry,
            Some(&sourced.source),
            format!("ability `{id}`"),
            None,
        );
        walker.creature_refs = creature_refs.clone();
        walker.ability(&sourced.item);
        report.issues.extend(walker.finish().issues);
    }
}

fn check_conditions(
    registry: &ContentRegistry,
    creature_refs: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    for (id, sourced) in registry.conditions.iter_sourced() {
        let mut walker = Walker::new(
            registry,
            Some(&sourced.source),
            format!("condition `{id}`"),
            None,
        );
        walker.creature_refs = creature_refs.clone();
        walker.condition(&sourced.item);
        report.issues.extend(walker.finish().issues);
    }
}

fn check_masteries(
    registry: &ContentRegistry,
    creature_refs: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    for (id, sourced) in registry.masteries.iter_sourced() {
        let mut walker = Walker::new(
            registry,
            Some(&sourced.source),
            format!("mastery `{id}`"),
            None,
        );
        walker.creature_refs = creature_refs.clone();
        walker.mastery(&sourced.item);
        report.issues.extend(walker.finish().issues);
    }
}

// ---------------------------------------------------------------------------
// AI templates and assignments
// ---------------------------------------------------------------------------

fn check_ai_templates(
    registry: &ContentRegistry,
    report: &mut CheckReport,
    _creature_refs: &BTreeSet<String>,
) {
    for (id, sourced) in registry.ai_templates.iter_sourced() {
        check_ai_params(
            report,
            registry,
            Some(&sourced.source),
            &sourced.item.params,
            &format!("ai-template `{id}`"),
        );
    }
}

fn check_ai_assignment(
    report: &mut CheckReport,
    registry: &ContentRegistry,
    source: Option<&ContentSource>,
    assignment: &AiAssignment,
    context: &str,
) {
    if registry.ai_templates.get(&assignment.template).is_none() {
        report.error(
            source,
            format!("{context}: unknown ai-template `{}`", assignment.template),
        );
    }
    if let Some(params) = &assignment.params {
        check_ai_params(report, registry, source, params, context);
    }
}

fn check_ai_params(
    report: &mut CheckReport,
    registry: &ContentRegistry,
    source: Option<&ContentSource>,
    params: &AiParams,
    context: &str,
) {
    if params
        .flee_zone
        .as_deref()
        .is_some_and(|zone| zone.trim().is_empty())
    {
        report.error(source, format!("{context}: flee_zone must not be empty"));
    }
    if let Some(conditions) = &params.avoid_targets_with {
        for condition in conditions {
            if registry.conditions.get(condition).is_none() {
                report.error(
                    source,
                    format!(
                        "{context}: avoid_targets_with references unknown condition `{condition}`"
                    ),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scenes and encounters
// ---------------------------------------------------------------------------

fn check_obscurement(registry: &ContentRegistry, report: &mut CheckReport) {
    for (id, sourced) in registry.scenes.iter_sourced() {
        let scene = &sourced.item;
        if scene.mode != SceneMode::Grid {
            continue;
        }
        for zone in &scene.zones {
            if zone.obscured.is_some() {
                report.warning(
                    Some(&sourced.source),
                    format!(
                        "scene `{id}` zone `{}` sets `obscured`, but grid scenes use the scene-level value; move it to the scene",
                        zone.id
                    ),
                );
            }
        }
    }
}

fn scene_zone_ids(scene: &Scene) -> BTreeSet<String> {
    let mut zones: BTreeSet<String> = scene.zones.iter().map(|zone| zone.id.clone()).collect();
    if let Some(grid) = &scene.grid {
        for cell in grid.legend.values() {
            if let Some(zone) = &cell.zone {
                zones.insert(zone.clone());
            }
        }
        zones.extend(grid.zones.keys().cloned());
    }
    zones
}

fn check_scenes(
    registry: &ContentRegistry,
    report: &mut CheckReport,
    creature_refs: &BTreeSet<String>,
) {
    for (id, sourced) in registry.scenes.iter_sourced() {
        let scene = &sourced.item;
        let zones = scene_zone_ids(scene);
        for edge in &scene.edges {
            if !zones.contains(&edge.a) {
                report.error(
                    Some(&sourced.source),
                    format!("scene `{id}` edge references unknown zone `{}`", edge.a),
                );
            }
            if !zones.contains(&edge.b) {
                report.error(
                    Some(&sourced.source),
                    format!("scene `{id}` edge references unknown zone `{}`", edge.b),
                );
            }
        }
        for object in &scene.objects {
            if !zones.contains(&object.zone) {
                report.error(
                    Some(&sourced.source),
                    format!(
                        "scene `{id}` object `{}` is in unknown zone `{}`",
                        object.id, object.zone
                    ),
                );
            }
        }
        for trigger in &scene.triggers {
            if let Some(zone) = &trigger.zone {
                if !zones.contains(zone) {
                    report.error(
                        Some(&sourced.source),
                        format!(
                            "scene `{id}` trigger `{}` references unknown zone `{zone}`",
                            trigger.id.as_deref().unwrap_or("?")
                        ),
                    );
                }
            }
            for zone in &trigger.zones {
                if !zones.contains(zone) {
                    report.error(
                        Some(&sourced.source),
                        format!(
                            "scene `{id}` trigger `{}` references unknown zone `{zone}`",
                            trigger.id.as_deref().unwrap_or("?")
                        ),
                    );
                }
            }
        }

        let trigger_ids: BTreeSet<String> = scene
            .triggers
            .iter()
            .filter_map(|trigger| trigger.id.clone())
            .collect();
        let mut walker = Walker::new(
            registry,
            Some(&sourced.source),
            format!("scene `{id}`"),
            None,
        );
        walker.scene_triggers = Some(trigger_ids);
        walker.creature_refs = creature_refs.clone();
        for object in &scene.objects {
            if let Some(interact) = &object.interact {
                walker.interact(interact, &format!("object `{}`", object.id));
            }
        }
        for trigger in &scene.triggers {
            walker.scene_trigger(trigger);
        }
        report.issues.extend(walker.finish().issues);
    }
}

fn check_encounters(registry: &ContentRegistry, report: &mut CheckReport) {
    for (id, sourced) in registry.encounters.iter_sourced() {
        let encounter = &sourced.item;
        let scene = match registry.scenes.get(&encounter.scene) {
            Some(scene) => Some(scene),
            None => {
                report.error(
                    Some(&sourced.source),
                    format!(
                        "encounter `{id}` references unknown scene `{}`",
                        encounter.scene
                    ),
                );
                None
            }
        };
        let zones = scene.map(scene_zone_ids);
        if let (Some(party_zone), Some(zones)) = (&encounter.party_zone, &zones) {
            if !zones.contains(party_zone) {
                report.error(
                    Some(&sourced.source),
                    format!("encounter `{id}` party_zone references unknown zone `{party_zone}`"),
                );
            }
        }
        for member in &encounter.members {
            if registry.creatures.get(&member.creature).is_none() {
                report.error(
                    Some(&sourced.source),
                    format!(
                        "encounter `{id}` references unknown creature `{}`",
                        member.creature
                    ),
                );
            }
            if member.count() == 0 {
                report.error(
                    Some(&sourced.source),
                    format!("encounter `{id}` member `{}` has count 0", member.creature),
                );
            }
            if let (Some(zone), Some(zones)) = (&member.zone, &zones) {
                if !zones.contains(zone) {
                    report.error(
                        Some(&sourced.source),
                        format!(
                            "encounter `{id}` member `{}` references unknown zone `{zone}`",
                            member.creature
                        ),
                    );
                }
            }
            if let Some(ai) = &member.ai {
                check_ai_assignment(
                    report,
                    registry,
                    Some(&sourced.source),
                    ai,
                    &format!("encounter `{id}` member `{}`", member.creature),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Creature-bound checks
// ---------------------------------------------------------------------------

fn check_creature_content(
    registry: &ContentRegistry,
    resolved: &BTreeMap<String, Creature>,
    creature_refs: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    for (id, creature) in resolved {
        let source = registry.creatures.source(id).cloned();
        let mut use_refs: BTreeSet<String> = registry.abilities.ids().map(str::to_string).collect();
        if let Some(attacks) = &creature.attacks {
            use_refs.extend(attacks.iter().map(|attack| attack.id.clone()));
        }
        for entry in creature
            .actions
            .iter()
            .flatten()
            .chain(creature.features.iter().flatten())
        {
            use_refs.insert(entry.id().to_string());
        }
        let resources = Some(
            creature
                .resources
                .as_ref()
                .map(|resources| resources.keys().cloned().collect())
                .unwrap_or_default(),
        );
        let mut walker = Walker::new(
            registry,
            source.as_ref(),
            format!("creature `{id}`"),
            resources,
        );
        walker.use_refs = use_refs;
        walker.creature_refs = creature_refs.clone();
        walker.creature(creature);
        report.issues.extend(walker.finish().issues);
    }
}

/// Creatures whose base chain failed are still checked for the parts they
/// declare locally, so one bad `base:` does not hide unrelated errors.
fn check_unresolved_creature_content(
    registry: &ContentRegistry,
    unresolved: &BTreeSet<String>,
    creature_refs: &BTreeSet<String>,
    report: &mut CheckReport,
) {
    for id in unresolved {
        let Some(creature) = registry.creatures.get(id) else {
            continue;
        };
        let source = registry.creatures.source(id).cloned();
        let resources = Some(
            creature
                .resources
                .as_ref()
                .map(|resources| resources.keys().cloned().collect())
                .unwrap_or_default(),
        );
        let mut walker = Walker::new(
            registry,
            source.as_ref(),
            format!("creature `{id}`"),
            resources,
        );
        walker.creature_refs = creature_refs.clone();
        walker.creature(creature);
        report.issues.extend(walker.finish().issues);
    }
}

// ---------------------------------------------------------------------------
// Effect / predicate walker
// ---------------------------------------------------------------------------

struct Walker<'a> {
    registry: &'a ContentRegistry,
    report: CheckReport,
    source: Option<ContentSource>,
    context: String,
    resources: Option<BTreeSet<String>>,
    use_refs: BTreeSet<String>,
    creature_refs: BTreeSet<String>,
    scene_triggers: Option<BTreeSet<String>>,
}

impl<'a> Walker<'a> {
    fn new(
        registry: &'a ContentRegistry,
        source: Option<&ContentSource>,
        context: String,
        resources: Option<BTreeSet<String>>,
    ) -> Self {
        Self {
            registry,
            report: CheckReport::default(),
            source: source.cloned(),
            context,
            resources,
            use_refs: registry.abilities.ids().map(str::to_string).collect(),
            creature_refs: BTreeSet::new(),
            scene_triggers: None,
        }
    }

    fn finish(self) -> CheckReport {
        self.report
    }

    fn error(&mut self, message: impl Into<String>) {
        let message = format!("{}: {}", self.context, message.into());
        self.report.error(self.source.as_ref(), message);
    }

    fn warning(&mut self, message: impl Into<String>) {
        let message = format!("{}: {}", self.context, message.into());
        self.report.warning(self.source.as_ref(), message);
    }

    fn creature(&mut self, creature: &Creature) {
        if let Some(attacks) = &creature.attacks {
            for attack in attacks {
                self.attack(attack);
            }
        }
        for entry in creature
            .actions
            .iter()
            .flatten()
            .chain(creature.features.iter().flatten())
        {
            self.ability_entry(entry);
        }
        if let Some(conditions) = &creature.conditions {
            for condition in conditions {
                self.condition_ref(condition);
            }
        }
        if let Some(conditions) = &creature.condition_immunities {
            for condition in conditions {
                self.condition_ref(condition);
            }
        }
        if let Some(multiattack) = &creature.multiattack {
            let mut available: BTreeSet<String> = creature
                .attacks
                .iter()
                .flatten()
                .map(|attack| attack.id.clone())
                .collect();
            for entry in creature
                .actions
                .iter()
                .flatten()
                .chain(creature.features.iter().flatten())
            {
                available.insert(entry.id().to_string());
            }
            for reference in multiattack {
                if !available.contains(reference) {
                    self.error(format!(
                        "multiattack references unknown attack/ability `{reference}`"
                    ));
                }
            }
        }
        if creature.extra_attacks.is_some_and(|extra| extra > 10) {
            self.error("extra_attacks must be at most 10");
        }
        if let Some(ai) = &creature.ai {
            check_ai_assignment(
                &mut self.report,
                self.registry,
                self.source.as_ref(),
                ai,
                &self.context,
            );
        }
    }

    fn attack(&mut self, attack: &Attack) {
        if let Some(to_hit) = &attack.to_hit {
            self.expression(to_hit, "attack to_hit");
        }
        for damage in &attack.damage {
            self.expression(&damage.dice, "attack damage");
        }
        if let Some(mastery) = &attack.mastery {
            if self.registry.masteries.get(mastery).is_none() {
                self.error(format!(
                    "attack `{}` references unknown mastery `{mastery}`",
                    attack.id
                ));
            }
        }
        if let Some(cost) = &attack.cost {
            self.cost(cost, &format!("attack `{}`", attack.id));
        }
        self.effects(&attack.on_hit);
    }

    fn ability_entry(&mut self, entry: &AbilityEntry) {
        match entry {
            AbilityEntry::Ref(reference) => {
                let resolved = self.registry.resolve_ability(entry);
                match resolved {
                    Ok(resolved) => {
                        if let Some(cost) = entry.cost().or(resolved.ability.cost.as_ref()) {
                            self.cost(cost, &format!("ability `{}`", reference.reference));
                        }
                        for (index, trigger) in resolved.ability.triggers.iter().enumerate() {
                            if let Some(cost) = &trigger.cost {
                                self.cost(
                                    cost,
                                    &format!(
                                        "ability `{}` trigger #{}",
                                        reference.reference,
                                        index + 1
                                    ),
                                );
                            }
                        }
                    }
                    Err(error) => self.error(error.to_string()),
                }
            }
            AbilityEntry::Inline(ability) => self.ability(ability),
        }
    }

    fn ability(&mut self, ability: &Ability) {
        if let Some(cost) = &ability.cost {
            self.cost(cost, &format!("ability `{}`", ability.id));
        }
        if let Some(extra) = ability.extra_attacks {
            if extra > 10 {
                self.error(format!(
                    "ability `{}` extra_attacks must be at most 10",
                    ability.id
                ));
            }
            if ability.activation != Activation::Passive {
                self.warning(format!(
                    "ability `{}` grants extra attacks but is not passive",
                    ability.id
                ));
            }
        }
        self.spell_meta(ability);
        self.effects(&ability.effects);
        for (index, trigger) in ability.triggers.iter().enumerate() {
            self.trigger(trigger, &format!("trigger #{}", index + 1));
        }
        self.modifiers(&ability.modifiers);
    }

    /// Checks the optional `spell` metadata block against the fields it must
    /// agree with. Abilities without the block are not touched: legacy content
    /// keeps its exact behaviour and produces no new issues.
    fn spell_meta(&mut self, ability: &Ability) {
        let Some(spell) = &ability.spell else {
            return;
        };
        let id = &ability.id;
        let level = spell.level as i32;
        if ability.slot_level.unwrap_or(0) != level {
            self.error(format!(
                "ability `{id}` spell level {level} does not match slot_level {:?}",
                ability.slot_level
            ));
        }
        if level > 0 {
            let expected = format!("spell-slot-{level}");
            match &ability.cost {
                Some(cost) if cost.resource == expected => {}
                Some(cost) => self.error(format!(
                    "ability `{id}` spell cost resource `{}` does not match `{expected}`; per-caster overrides belong on the character sheet AbilityRef, not here",
                    cost.resource
                )),
                None => self.error(format!(
                    "ability `{id}` is a level {level} spell but has no cost; expected `{expected}`"
                )),
            }
        }
        if let Some(upcast) = &spell.upcast {
            for key in upcast.dice_by_level.keys() {
                if (*key as i32) < level || *key > 9 {
                    self.error(format!(
                        "ability `{id}` upcast dice_by_level key {key} is out of range; keys must be slot levels from {level} to 9"
                    ));
                }
            }
        }
        if let Some(material) = &spell.components.material {
            if (material.cost_gp.is_some() || material.consumed) && material.text.is_empty() {
                self.error(format!(
                    "ability `{id}` has a material component with cost or consumption but no text describing it"
                ));
            }
        }
        if let Some(school) = spell.school {
            if ability
                .tags
                .iter()
                .any(|tag| tag != school.as_tag() && is_school_tag(tag))
            {
                self.warning(format!(
                    "ability `{id}` spell school `{}` disagrees with its school tag",
                    school.as_tag()
                ));
            }
        }
        let tagged_ritual = ability.tags.iter().any(|tag| tag == "ritual");
        if tagged_ritual != spell.ritual {
            self.warning(format!(
                "ability `{id}` spell ritual flag ({}) disagrees with its `ritual` tag",
                spell.ritual
            ));
        }
    }

    fn condition(&mut self, condition: &ConditionDef) {
        self.modifiers(&condition.modifiers);
        self.duration(condition.duration_default.as_ref());
    }

    fn mastery(&mut self, mastery: &MasteryDef) {
        for (index, trigger) in mastery.triggers.iter().enumerate() {
            self.trigger(trigger, &format!("trigger #{}", index + 1));
        }
        self.modifiers(&mastery.modifiers);
    }

    fn scene_trigger(&mut self, trigger: &SceneTrigger) {
        if let Some(when) = &trigger.when {
            self.predicate(when);
        }
        self.effects(&trigger.effects);
    }

    fn interact(&mut self, interact: &super::schema::Interact, object: &str) {
        if let Some(effects) = &interact.effects {
            self.effects(effects);
        }
        if let Some(effects) = &interact.on_success {
            self.effects(effects);
        }
        if let Some(effects) = &interact.on_fail {
            self.effects(effects);
        }
        if let Some(options) = &interact.options {
            for (index, option) in options.iter().enumerate() {
                if option.effects.is_none()
                    && option.on_success.is_none()
                    && option.on_fail.is_none()
                    && option.check.is_none()
                {
                    self.warning(format!(
                        "object `{object}` option #{} `{}` does nothing",
                        index + 1,
                        option.name
                    ));
                }
                if let Some(effects) = &option.effects {
                    self.effects(effects);
                }
                if let Some(effects) = &option.on_success {
                    self.effects(effects);
                }
                if let Some(effects) = &option.on_fail {
                    self.effects(effects);
                }
            }
        }
    }

    fn trigger(&mut self, trigger: &Trigger, label: &str) {
        if let Some(when) = &trigger.when {
            self.predicate(when);
        }
        if let Some(cost) = &trigger.cost {
            self.cost(cost, label);
        }
        self.effects(&trigger.effects);
    }

    fn modifiers(&mut self, modifiers: &[Modifier]) {
        for modifier in modifiers {
            if let Some(value) = &modifier.value {
                self.expression(value, "modifier value");
            }
            if let Some(when) = &modifier.when {
                self.predicate(when);
            }
        }
    }

    fn duration(&mut self, duration: Option<&Duration>) {
        if let Some(duration) = duration {
            if let Err(error) = duration.validate() {
                self.error(error);
            }
        }
    }

    fn effects(&mut self, effects: &[Effect]) {
        for effect in effects {
            self.effect(effect);
        }
    }

    fn effect(&mut self, effect: &Effect) {
        match effect {
            Effect::Attack(attack) => self.attack_effect(attack),
            Effect::Save(save) => {
                self.expression(&save.dc, "save dc");
                self.effects(&save.on_fail);
                self.effects(&save.on_success);
            }
            Effect::Damage(damage) => self.expression(&damage.dice, "damage dice"),
            Effect::Heal(heal) => self.expression(&heal.dice, "heal dice"),
            Effect::ApplyCondition(apply) => {
                self.condition_ref(&apply.condition);
                self.duration(apply.duration.as_ref());
                if let Some(save_ends) = &apply.save_ends {
                    self.expression(&save_ends.dc, "save_ends dc");
                }
            }
            Effect::RemoveCondition(remove) => self.condition_ref(&remove.condition),
            Effect::ForcedMove(_) => {}
            Effect::Use(reference) => {
                if !self.use_refs.contains(reference) {
                    self.error(format!(
                        "`use` references unknown ability/attack `{reference}`"
                    ));
                }
            }
            Effect::Activate(activate) => {
                if !self.creature_refs.contains(&activate.creature) {
                    self.warning(format!(
                        "`activate` references `{}`, which is not a known creature id or tag (instance names are allowed)",
                        activate.creature
                    ));
                }
            }
            Effect::Check(check) => {
                self.expression(&check.dc, "check dc");
                self.effects(&check.on_success);
                self.effects(&check.on_fail);
            }
            Effect::BasicAction(_) => {}
            Effect::Choice(options) => {
                for effects in options {
                    self.effects(effects);
                }
            }
            Effect::Note(_) => {}
            Effect::ToggleTrigger(toggle) => {
                if let Some(triggers) = &self.scene_triggers {
                    if !triggers.contains(&toggle.trigger) {
                        self.error(format!(
                            "`toggle_trigger` references unknown scene trigger `{}`",
                            toggle.trigger
                        ));
                    }
                }
            }
        }
    }

    fn attack_effect(&mut self, attack: &AttackEffect) {
        self.expression(&attack.to_hit, "attack to_hit");
        for damage in &attack.damage {
            self.expression(&damage.dice, "attack damage");
        }
        if let Some(mastery) = &attack.mastery {
            if self.registry.masteries.get(mastery).is_none() {
                self.error(format!(
                    "attack effect references unknown mastery `{mastery}`"
                ));
            }
        }
        self.effects(&attack.on_hit);
        self.effects(&attack.on_miss);
    }

    fn predicate(&mut self, predicate: &Predicate) {
        match predicate {
            Predicate::All(children) | Predicate::Any(children) => {
                for child in children {
                    self.predicate(child);
                }
            }
            Predicate::Not(child) => self.predicate(child),
            Predicate::TargetHasCondition(condition) | Predicate::SelfHasCondition(condition) => {
                self.condition_ref(condition)
            }
            _ => {}
        }
    }

    fn condition_ref(&mut self, condition: &str) {
        if self.registry.conditions.get(condition).is_none() {
            self.error(format!("unknown condition `{condition}`"));
        }
    }

    fn cost(&mut self, cost: &AbilityCost, label: &str) {
        if let Some(resources) = &self.resources {
            if !resources.contains(&cost.resource) {
                self.error(format!(
                    "{label} costs unknown resource `{}`",
                    cost.resource
                ));
            }
        }
    }

    fn expression(&mut self, value: &ExprValue, label: &str) {
        let text = value.as_text();
        if let Err(error) = expr::validate_template(&text) {
            self.error(format!("{label}: {error}"));
        }
    }
}
