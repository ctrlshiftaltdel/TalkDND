//! Flight, vertical movement and fall resolution tests for TASK-0089 (#173).
//!
//! Every fixture here is synthetic: no monster, spell, campaign or book text is
//! used. The one real-data test reads the tracked SRD spatial overlay from
//! TASK-0094 (#178) without copying its prose.
//!
//! Contract: `tools/combat-sim/docs/SPATIAL-CONTRACT-v1.md`.
//! Rules matrix: `tools/combat-sim/docs/SPATIAL-RULES-MATRIX.md`.
//! Table ruling: `LetsPlay/Rules/rulings.md` R-09.
//!
//! The resolver is the P4 seam only; P5 (#174) wires it into the turn loop,
//! CLI and AI. Legacy 2D saves must never reach it unless the P1 spatial
//! fields are explicitly present.

use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use combat_sim::content::schema::{ConditionDef, ConditionFlag, Creature, Scene, Size, Speed};
use combat_sim::content::{ContentRegistry, ContentSource, ContentSources};
use combat_sim::dice::DiceRng;
use combat_sim::engine::{self, FallOptions};
use combat_sim::space::{
    flight_fall_trigger, transition_flight, CellPos, FallCause, FallColumn, FallDamage, FallMap,
    FallReason, FallResolver, FallRuleKind, FallSurfaceKind, FlightAction, FlightCapability,
    FlightCheck, FlightConditions, FlightEdition, FlightError, FlightRules, FlightState,
    FlightSubject, Footprint, HeightProfile, HeightSource, LandingCondition, LandingRule,
    MoveBlock, MovementCell, MovementMap, MovementPlanner, SpatialBody, SpatialPose,
    SpatialProfile, VerticalSpan,
};
use combat_sim::state::{CombatantState, EncounterState, SpatialState, STATE_SCHEMA_VERSION};
use serde_json::json;

// ---------------------------------------------------------------------------
// Synthetic fixtures
// ---------------------------------------------------------------------------

fn creature(id: &str, size: Size, fly: i32, hover: bool, burrow: i32) -> Creature {
    serde_json::from_value(json!({
        "id": id,
        "name": id,
        "size": size,
        "hp": 30,
        "speed": {"fly": fly, "hover": hover, "burrow": burrow}
    }))
    .expect("synthetic creature")
}

fn condition(id: &str, flags: Vec<ConditionFlag>) -> ConditionDef {
    ConditionDef {
        id: id.to_string(),
        name: None,
        flags,
        implies: Vec::new(),
        modifiers: Vec::new(),
        duration_default: None,
        ends_on: Vec::new(),
        manual: None,
        notes: None,
        todo: Vec::new(),
        source: None,
        description: None,
    }
}

/// Minimal registry with the generic condition flags the engine consults.
/// The ids are synthetic test data; production code never names them.
fn registry(ruleset: &str) -> ContentRegistry {
    let mut registry = ContentRegistry {
        ruleset: ruleset.to_string(),
        ..ContentRegistry::default()
    };
    for (id, flags) in [
        ("prone", vec![ConditionFlag::StandUpHalfSpeed]),
        ("incapacitated", vec![ConditionFlag::Incapacitated]),
        ("held", vec![ConditionFlag::SpeedZero]),
    ] {
        registry.conditions.insert(
            id.to_string(),
            condition(id, flags),
            ContentSource::new("synthetic", None),
        );
    }
    registry
}

fn combatant(
    name: &str,
    creature: Creature,
    altitude: i32,
    flight: FlightState,
    conditions: &[&str],
) -> CombatantState {
    let conditions: Vec<_> = conditions
        .iter()
        .map(|id| json!({"condition": id}))
        .collect();
    serde_json::from_value(json!({
        "creature": creature,
        "name": name,
        "side": "party",
        "max_hp": 30,
        "hp": 30,
        "zone": "open",
        "pos": {"kind": "cell", "x": 0, "y": 0},
        "pose": {
            "cell": {"x": 0, "y": 0, "z": altitude},
            "footprint": {"width": 1, "depth": 1}
        },
        "vertical": {"altitude_ft": altitude, "flight": flight, "fall_distance_ft": 0},
        "conditions": conditions
    }))
    .expect("synthetic combatant")
}

/// A truly legacy combatant: no pose, no vertical state, no P1 spatial data.
fn legacy_combatant(name: &str) -> CombatantState {
    serde_json::from_value(json!({
        "creature": creature("synthetic-legacy", Size::Medium, 30, false, 0),
        "name": name,
        "side": "party",
        "max_hp": 30,
        "hp": 30,
        "zone": "open",
        "pos": {"kind": "cell", "x": 0, "y": 0}
    }))
    .expect("legacy combatant")
}

fn state_with(
    registry: ContentRegistry,
    combatant: CombatantState,
    scene: Option<serde_json::Value>,
    spatial: Option<SpatialState>,
) -> EncounterState {
    let mut state: EncounterState = serde_json::from_value(json!({
        "schema_version": STATE_SCHEMA_VERSION,
        "content": registry,
        "scene": scene,
        "spatial": spatial,
        "combatants": [combatant],
        "initiative": [0],
        "round": 1
    }))
    .expect("synthetic state");
    state.rebuild_caches();
    state
}

fn grid_scene(
    rows: &[&str],
    legend: serde_json::Value,
    cell_height_ft: Option<i32>,
) -> serde_json::Value {
    json!({
        "id": "synthetic-flight-grid",
        "mode": "grid",
        "cell_height_ft": cell_height_ft,
        "grid": {"cell": 5, "rows": rows, "legend": legend}
    })
}

fn flat_scene() -> serde_json::Value {
    grid_scene(&["."], json!({".": {"elevation": 0}}), None)
}

fn pose(x: i32, y: i32, z: i32) -> SpatialPose {
    SpatialPose::new(CellPos::new(x, y, z).expect("cell"), Footprint::ONE, None).expect("pose")
}

fn medium_height() -> combat_sim::space::ResolvedHeight {
    HeightProfile::default()
        .resolve(Size::Medium, None)
        .expect("medium height")
}

fn rules_2014() -> FlightRules {
    FlightRules::for_edition(FlightEdition::Edition2014)
}

fn rules_2024() -> FlightRules {
    FlightRules::for_edition(FlightEdition::Edition2024)
}

fn check(
    edition: FlightEdition,
    fly: i32,
    hover: bool,
    mutate: impl FnOnce(&mut FlightConditions),
) -> FlightCheck {
    let mut subject = FlightSubject {
        fly_speed_ft: fly,
        hover,
        held_aloft_by_magic: false,
        conditions: FlightConditions::default(),
    };
    mutate(&mut subject.conditions);
    flight_fall_trigger(&subject, FlightRules::for_edition(edition))
}

#[derive(Debug)]
struct ScriptedRng {
    values: VecDeque<u32>,
    sides: Vec<u32>,
}

impl ScriptedRng {
    fn new(values: &[u32]) -> Self {
        Self {
            values: values.iter().copied().collect(),
            sides: Vec::new(),
        }
    }
}

impl DiceRng for ScriptedRng {
    fn roll_die(&mut self, sides: u32) -> u32 {
        self.sides.push(sides);
        self.values
            .pop_front()
            .expect("scripted RNG ran out while rolling")
    }
}

fn fall_state(
    ruleset: &str,
    altitude: i32,
    flight: FlightState,
    hover: bool,
    resistance: bool,
) -> EncounterState {
    let mut creature = creature("synthetic-faller", Size::Medium, 30, hover, 0);
    if resistance {
        creature.resistances = Some(vec!["bludgeoning".to_string()]);
    }
    let combatant = combatant("Faller", creature, altitude, flight, &[]);
    state_with(
        registry(ruleset),
        combatant,
        Some(flat_scene()),
        Some(SpatialState::default()),
    )
}

// ---------------------------------------------------------------------------
// AC-1: explicit state, hover branch, no creature names
// ---------------------------------------------------------------------------

#[test]
fn flight_state_and_hover_branch_are_explicit() {
    let capability = FlightCapability {
        fly_speed_ft: 30,
        hover: true,
        burrow_speed_ft: 0,
    };
    assert_eq!(
        transition_flight(FlightState::Grounded, FlightAction::TakeOff, &capability, 0).unwrap(),
        FlightState::Flying
    );
    assert_eq!(
        transition_flight(FlightState::Flying, FlightAction::Hover, &capability, 10).unwrap(),
        FlightState::Hovering
    );
    assert_eq!(
        transition_flight(
            FlightState::Hovering,
            FlightAction::StopHovering,
            &capability,
            10
        )
        .unwrap(),
        FlightState::Flying
    );
    assert_eq!(
        transition_flight(FlightState::Flying, FlightAction::Fall, &capability, 10).unwrap(),
        FlightState::Falling
    );
    assert_eq!(
        transition_flight(FlightState::Falling, FlightAction::Land, &capability, 0).unwrap(),
        FlightState::Grounded
    );

    let no_hover = FlightCapability {
        hover: false,
        ..capability
    };
    assert_eq!(
        transition_flight(FlightState::Flying, FlightAction::Hover, &no_hover, 10),
        Err(FlightError::HoverUnavailable)
    );
    let no_fly = FlightCapability {
        fly_speed_ft: 0,
        ..capability
    };
    assert_eq!(
        transition_flight(FlightState::Grounded, FlightAction::TakeOff, &no_fly, 0),
        Err(FlightError::NoFlySpeed)
    );
    assert!(matches!(
        transition_flight(FlightState::Grounded, FlightAction::Fall, &capability, 10),
        Err(FlightError::InvalidTransition { .. })
    ));
    assert!(matches!(
        transition_flight(FlightState::Flying, FlightAction::Land, &capability, 10),
        Err(FlightError::InvalidTransition { .. })
    ));
}

#[test]
fn flight_capability_reads_data_fields_not_creature_names() {
    let hovering = FlightCapability::from_speed(&Speed {
        fly: 50,
        hover: true,
        burrow: 20,
        ..Speed::default()
    });
    assert!(hovering.can_fly());
    assert!(hovering.can_hover());
    assert!(hovering.can_burrow());
    assert_eq!(hovering.fly_speed_ft, 50);
    assert_eq!(hovering.burrow_speed_ft, 20);

    let plain = FlightCapability::from_speed(&Speed {
        fly: 50,
        ..Speed::default()
    });
    assert!(plain.can_fly());
    assert!(!plain.can_hover());
    assert!(!plain.can_burrow());
}

// ---------------------------------------------------------------------------
// AC-2: 2014 / 2024 fall triggers stay separate
// ---------------------------------------------------------------------------

#[test]
fn fall_triggers_are_edition_specific() {
    // 2014 names knocked-down, speed 0 and "otherwise deprived of movement".
    let incap_2014 = check(FlightEdition::Edition2014, 30, false, |c| {
        c.incapacitated = true
    });
    assert!(!incap_2014.falls);
    assert_eq!(incap_2014.reason, None);

    let speed_zero_2014 = check(FlightEdition::Edition2014, 30, false, |c| {
        c.speed_zero = true
    });
    assert!(speed_zero_2014.falls);
    assert_eq!(speed_zero_2014.reason, Some(FallReason::SpeedZero));

    let deprived_2014 = check(FlightEdition::Edition2014, 30, false, |c| {
        c.deprived_of_movement = true
    });
    assert!(deprived_2014.falls);
    assert_eq!(deprived_2014.reason, Some(FallReason::DeprivedOfMovement));

    // 2024 names Incapacitated, knocked-down, and Fly Speed 0. It does not use
    // 2014's "deprived of movement" catch-all, and a general Speed 0 alone is
    // not the 2024 trigger (Fly Speed 0 is).
    let incap_2024 = check(FlightEdition::Edition2024, 30, false, |c| {
        c.incapacitated = true
    });
    assert!(incap_2024.falls);
    assert_eq!(incap_2024.reason, Some(FallReason::Incapacitated));

    let speed_zero_2024 = check(FlightEdition::Edition2024, 30, false, |c| {
        c.speed_zero = true
    });
    assert!(!speed_zero_2024.falls);

    let deprived_2024 = check(FlightEdition::Edition2024, 30, false, |c| {
        c.deprived_of_movement = true
    });
    assert!(!deprived_2024.falls);

    let fly_zero_2024 = check(FlightEdition::Edition2024, 30, false, |c| {
        c.fly_speed_zero = true
    });
    assert!(fly_zero_2024.falls);
    assert_eq!(fly_zero_2024.reason, Some(FallReason::FlySpeedZero));

    // Both editions fall when knocked down.
    for edition in [FlightEdition::Edition2014, FlightEdition::Edition2024] {
        let knocked = check(edition, 30, false, |c| c.knocked_down = true);
        assert!(knocked.falls, "{edition:?}");
        assert_eq!(knocked.reason, Some(FallReason::KnockedDown));
    }

    // Hover and "held aloft by magic" suppress every trigger in both editions.
    for edition in [FlightEdition::Edition2014, FlightEdition::Edition2024] {
        let hovering = check(edition, 30, true, |c| {
            c.incapacitated = true;
            c.knocked_down = true;
        });
        assert!(!hovering.falls, "{edition:?}");

        let mut subject = FlightSubject {
            fly_speed_ft: 30,
            hover: false,
            held_aloft_by_magic: true,
            conditions: FlightConditions {
                knocked_down: true,
                ..FlightConditions::default()
            },
        };
        assert!(!flight_fall_trigger(&subject, FlightRules::for_edition(edition)).falls);
        subject.held_aloft_by_magic = false;
        assert!(flight_fall_trigger(&subject, FlightRules::for_edition(edition)).falls);
    }

    // A flying creature with no Fly Speed cannot stay aloft in either edition.
    for edition in [FlightEdition::Edition2014, FlightEdition::Edition2024] {
        let no_fly = check(edition, 0, false, |_| {});
        assert!(no_fly.falls, "{edition:?}");
        assert_eq!(no_fly.reason, Some(FallReason::FlySpeedZero));
    }

    // Unknown or missing rulesets are an explicit error, never a silent
    // fallback to the other edition.
    assert_eq!(
        FlightRules::try_for_ruleset(Some("3.5")),
        Err(FlightError::UnknownRuleset {
            ruleset: "3.5".to_string()
        })
    );
    assert_eq!(
        FlightRules::try_for_ruleset(None),
        Err(FlightError::MissingRuleset)
    );
}

#[test]
fn engine_flight_check_maps_condition_flags_per_edition() {
    let incap_2014 = state_with(
        registry("2014"),
        combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, 0),
            20,
            FlightState::Flying,
            &["incapacitated"],
        ),
        None,
        None,
    );
    let check_2014 = engine::flight_check(&incap_2014, "Flyer").unwrap();
    assert!(!check_2014.falls);

    let incap_2024 = state_with(
        registry("2024"),
        combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, 0),
            20,
            FlightState::Flying,
            &["incapacitated"],
        ),
        None,
        None,
    );
    let check_2024 = engine::flight_check(&incap_2024, "Flyer").unwrap();
    assert!(check_2024.falls);
    assert_eq!(check_2024.reason, Some(FallReason::Incapacitated));

    let held_2014 = state_with(
        registry("2014"),
        combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, 0),
            20,
            FlightState::Flying,
            &["held"],
        ),
        None,
        None,
    );
    let held_2014 = engine::flight_check(&held_2014, "Flyer").unwrap();
    assert!(held_2014.falls);
    assert_eq!(held_2014.reason, Some(FallReason::SpeedZero));

    // The engine maps Speed 0 to a 0 Fly Speed for the 2024 rule, so the
    // reason is Fly Speed 0 rather than 2014's generic Speed 0.
    let held_2024 = state_with(
        registry("2024"),
        combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, 0),
            20,
            FlightState::Flying,
            &["held"],
        ),
        None,
        None,
    );
    let held_2024 = engine::flight_check(&held_2024, "Flyer").unwrap();
    assert!(held_2024.falls);
    assert_eq!(held_2024.reason, Some(FallReason::FlySpeedZero));

    let hover_2024 = state_with(
        registry("2024"),
        combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, true, 0),
            20,
            FlightState::Hovering,
            &["incapacitated"],
        ),
        None,
        None,
    );
    assert!(!engine::flight_check(&hover_2024, "Flyer").unwrap().falls);

    let grounded = state_with(
        registry("2024"),
        combatant(
            "Walker",
            creature("synthetic-walker", Size::Medium, 0, false, 0),
            0,
            FlightState::Grounded,
            &[],
        ),
        None,
        None,
    );
    assert!(!engine::flight_check(&grounded, "Walker").unwrap().falls);

    let unknown = state_with(
        registry("3.5"),
        combatant(
            "Flyer",
            creature("synthetic-flyer", Size::Medium, 30, false, 0),
            20,
            FlightState::Flying,
            &[],
        ),
        None,
        None,
    );
    let error = engine::flight_check(&unknown, "Flyer").unwrap_err();
    assert!(error.to_string().contains("3.5"), "{error}");
}

// ---------------------------------------------------------------------------
// AC-5: explicit height defaults and provenance
// ---------------------------------------------------------------------------

#[test]
fn height_defaults_are_explicit_and_auditable() {
    let profile = HeightProfile::default();
    let medium = profile.resolve(Size::Medium, None).unwrap();
    assert_eq!(medium.height_ft, 5);
    assert_eq!(medium.source, HeightSource::SizeDefault(Size::Medium));

    let tiny = profile.resolve(Size::Tiny, None).unwrap();
    assert_eq!(tiny.height_ft, 3);
    assert_eq!(tiny.source, HeightSource::SizeDefault(Size::Tiny));

    let unknown = profile.resolve(Size::Unknown, None).unwrap();
    assert_eq!(unknown.height_ft, 5);
    assert_eq!(unknown.source, HeightSource::UnknownSizeDefault);

    let explicit = profile.resolve(Size::Medium, Some(7)).unwrap();
    assert_eq!(explicit.height_ft, 7);
    assert_eq!(explicit.source, HeightSource::Explicit);

    let custom = HeightProfile {
        medium_ft: 6,
        ..HeightProfile::default()
    };
    let custom_medium = custom.resolve(Size::Medium, None).unwrap();
    assert_eq!(custom_medium.height_ft, 6);
    assert_eq!(
        custom_medium.source,
        HeightSource::SizeDefault(Size::Medium)
    );

    assert!(profile.resolve(Size::Medium, Some(0)).is_err());
}

// ---------------------------------------------------------------------------
// AC-3 / AC-9: fall damage, landing, R-09 RAW vs house rule
// ---------------------------------------------------------------------------

#[test]
fn fall_damage_is_one_die_per_ten_feet_capped_at_twenty() {
    let five = FallDamage::for_distance(5);
    assert_eq!(five.dice, 0);
    assert_eq!(five.expression(), "0");
    assert_eq!(five.rule_kind, FallRuleKind::Raw);

    let ten = FallDamage::for_distance(10);
    assert_eq!(ten.dice, 1);
    assert_eq!(ten.expression(), "1d6");
    assert_eq!(ten.damage_type, "bludgeoning");

    let fifteen = FallDamage::for_distance(15);
    assert_eq!(fifteen.dice, 1);

    let twenty = FallDamage::for_distance(20);
    assert_eq!(twenty.dice, 2);

    let two_hundred = FallDamage::for_distance(200);
    assert_eq!(two_hundred.dice, 20);
    assert!(!two_hundred.capped);

    let two_hundred_ten = FallDamage::for_distance(210);
    assert_eq!(two_hundred_ten.dice, 20);
    assert!(two_hundred_ten.capped);
}

#[test]
fn fall_landing_separates_raw_from_the_r09_house_rule() {
    let map = FallMap::flat(3, 3, 0);
    let resolver = FallResolver::new(&map, rules_2024());

    let deep = resolver
        .resolve_from(&pose(0, 0, 30), medium_height(), FallCause::Jump)
        .expect("30 ft fall");
    assert_eq!(deep.fall_distance_ft, 30);
    assert_eq!(deep.damage.expression(), "3d6");
    assert!(deep.raw_flat_on_damage);
    assert!(deep.landing_check.is_none());
    let deep_landing = deep.landing_outcome(15, None);
    assert_eq!(deep_landing.condition, LandingCondition::LandedFlat);
    assert_eq!(deep_landing.rule, LandingRule::RawFall);

    let boundary = resolver
        .resolve_from(&pose(0, 0, 10), medium_height(), FallCause::Jump)
        .expect("10 ft fall");
    assert_eq!(boundary.fall_distance_ft, 10);
    assert_eq!(boundary.damage.expression(), "1d6");
    let landing_check = boundary.landing_check.as_ref().expect("R-09 check");
    assert_eq!(landing_check.rule_kind, FallRuleKind::HouseRule);
    assert_eq!(landing_check.source, "rulings.md R-09");
    assert_eq!(landing_check.check.dc, 10);
    assert_eq!(landing_check.max_fall_distance_ft, 10);

    let avoided = boundary.landing_outcome(6, Some(10));
    assert_eq!(avoided.condition, LandingCondition::Standing);
    assert_eq!(avoided.rule, LandingRule::HouseRuleR09Avoided);
    let failed = boundary.landing_outcome(6, Some(9));
    assert_eq!(failed.condition, LandingCondition::LandedFlat);
    assert_eq!(failed.rule, LandingRule::RawFall);
    let untried = boundary.landing_outcome(6, None);
    assert_eq!(untried.condition, LandingCondition::LandedFlat);

    let no_damage = resolver
        .resolve_from(&pose(0, 0, 5), medium_height(), FallCause::Jump)
        .expect("5 ft fall");
    assert_eq!(no_damage.damage.expression(), "0");
    assert!(!no_damage.raw_flat_on_damage);
    assert!(no_damage.landing_check.is_none());
    let standing = no_damage.landing_outcome(0, None);
    assert_eq!(standing.condition, LandingCondition::Standing);
    assert_eq!(standing.rule, LandingRule::NoDamage);
}

#[test]
fn liquid_landing_follows_the_2024_rule_only() {
    let map = FallMap::flat(3, 3, 0).with_column(0, 0, FallColumn::liquid(0));
    let start = pose(0, 0, 20);

    let fall_2024 = FallResolver::new(&map, rules_2024())
        .resolve_from(&start, medium_height(), FallCause::Jump)
        .expect("liquid fall");
    assert_eq!(fall_2024.landing.surface_kind, FallSurfaceKind::Liquid);
    let liquid = fall_2024.liquid_landing.as_ref().expect("2024 liquid rule");
    assert_eq!(liquid.rule_kind, FallRuleKind::Raw);
    assert_eq!(liquid.checks.len(), 2);
    assert!(liquid.checks.iter().all(|check| check.dc == 15));
    assert!(liquid.success_halves_damage);
    assert!(fall_2024.liquid_halves_damage(Some(15)));
    assert!(!fall_2024.liquid_halves_damage(Some(14)));
    assert!(!fall_2024.liquid_halves_damage(None));

    let fall_2014 = FallResolver::new(&map, rules_2014())
        .resolve_from(&start, medium_height(), FallCause::Jump)
        .expect("liquid fall");
    assert!(fall_2014.liquid_landing.is_none());
    assert!(!fall_2014.liquid_halves_damage(Some(15)));
}

// ---------------------------------------------------------------------------
// AC-4: ceilings, cliffs, platforms, vertical movement, forced movement
// ---------------------------------------------------------------------------

#[test]
fn fall_resolver_handles_platforms_ceilings_and_multi_floor_columns() {
    let platforms = FallColumn::new(0)
        .with_platform("p1", 10)
        .with_platform("p2", 20);
    let roofed = FallColumn::new(0).with_ceiling("roof", 20);
    let pillar = FallColumn::new(0).with_span(VerticalSpan {
        id: "pillar".to_string(),
        bottom_ft: 0,
        top_ft: 20,
        kind: FallSurfaceKind::Terrain,
    });
    let map = FallMap::flat(3, 3, 0)
        .with_column(0, 0, platforms)
        .with_column(1, 0, roofed)
        .with_column(2, 0, pillar);
    let resolver = FallResolver::new(&map, rules_2024());

    let multi = resolver
        .resolve_from(&pose(0, 0, 45), medium_height(), FallCause::Jump)
        .expect("multi-floor fall");
    assert_eq!(multi.fall_distance_ft, 25);
    assert_eq!(multi.damage.expression(), "2d6");
    assert_eq!(multi.landing.elevation_ft, 20);
    assert_eq!(multi.landing.surface_id, "p2");
    assert_eq!(multi.landing.surface_kind, FallSurfaceKind::Platform);
    assert_eq!(multi.landing.height_above_floor_ft, 20);
    assert_eq!(multi.collisions.len(), 1);
    assert_eq!(multi.collisions[0].surface_id, "p2");

    let short = resolver
        .resolve_from(&pose(0, 0, 25), medium_height(), FallCause::Jump)
        .expect("short fall");
    assert_eq!(short.fall_distance_ft, 5);
    assert_eq!(short.damage.expression(), "0");

    let standing = resolver
        .resolve_from(&pose(0, 0, 20), medium_height(), FallCause::Jump)
        .expect("standing on the platform");
    assert_eq!(standing.fall_distance_ft, 0);
    assert_eq!(standing.landing.surface_id, "p2");

    let ceiling = resolver
        .resolve_from(&pose(1, 0, 40), medium_height(), FallCause::Jump)
        .expect("ceiling fall");
    assert_eq!(ceiling.fall_distance_ft, 19);
    assert_eq!(ceiling.landing.elevation_ft, 21);
    assert_eq!(ceiling.landing.surface_kind, FallSurfaceKind::Ceiling);
    assert_eq!(ceiling.collisions.len(), 1);
    assert_eq!(ceiling.collisions[0].surface_id, "roof");

    assert_eq!(
        resolver.resolve_from(&pose(1, 0, 20), medium_height(), FallCause::Jump),
        Err(FlightError::EmbeddedInSolid {
            surface_id: "roof".to_string(),
            start_ft: 20,
        })
    );

    let terrain = resolver
        .resolve_from(&pose(2, 0, 30), medium_height(), FallCause::Jump)
        .expect("terrain fall");
    assert_eq!(terrain.fall_distance_ft, 10);
    assert_eq!(terrain.landing.surface_kind, FallSurfaceKind::Terrain);
    assert_eq!(terrain.landing.surface_id, "pillar");
}

#[test]
fn fall_resolver_handles_cliffs_and_structured_errors() {
    let map = FallMap::flat(3, 3, 0)
        .with_column(0, 0, FallColumn::new(30))
        .with_column(1, 0, FallColumn::new(0))
        .with_column(2, 0, FallColumn::new(40));
    let resolver = FallResolver::new(&map, rules_2024());

    let cliff = resolver
        .resolve_into(&pose(0, 0, 30), (1, 0), medium_height(), FallCause::Forced)
        .expect("cliff fall");
    assert_eq!(cliff.fall_distance_ft, 30);
    assert_eq!(cliff.start_elevation_ft, 30);
    assert_eq!(cliff.landing.elevation_ft, 0);
    assert_eq!(cliff.landing.cell.x, 1);
    assert_eq!(cliff.damage.expression(), "3d6");
    assert_eq!(cliff.cause, FallCause::Forced);

    let high = resolver
        .resolve_into(&pose(0, 0, 40), (1, 0), medium_height(), FallCause::Jump)
        .expect("high cliff fall");
    assert_eq!(high.fall_distance_ft, 40);
    assert_eq!(high.damage.expression(), "4d6");

    assert_eq!(
        resolver.resolve_into(&pose(0, 0, 30), (2, 0), medium_height(), FallCause::Forced),
        Err(FlightError::LandingAboveStart {
            start_ft: 30,
            surface_ft: 40
        })
    );
    assert_eq!(
        resolver.resolve_into(&pose(0, 0, 30), (9, 9), medium_height(), FallCause::Forced),
        Err(FlightError::UnknownColumn { x: 9, y: 9 })
    );
    assert_eq!(
        resolver.resolve_from(&pose(0, 0, 20), medium_height(), FallCause::Jump),
        Err(FlightError::StartBelowFloor {
            start_ft: 20,
            floor_ft: 30
        })
    );
}

#[test]
fn vertical_move_respects_ceilings_floors_and_solids() {
    let map = FallMap::flat(3, 3, 0).with_column(0, 0, FallColumn::new(0).with_ceiling("roof", 20));
    let resolver = FallResolver::new(&map, rules_2024());
    let capability = FlightCapability {
        fly_speed_ft: 30,
        hover: false,
        burrow_speed_ft: 0,
    };

    let blocked = resolver
        .vertical_move(
            &pose(0, 0, 10),
            25,
            medium_height(),
            &capability,
            FlightState::Flying,
        )
        .expect("vertical move");
    assert!(blocked.blocked);
    assert_eq!(blocked.allowed_elevation_ft, 15);
    assert_eq!(blocked.moved_ft, 5);
    assert_eq!(blocked.cost_ft, 5);
    assert_eq!(blocked.ceiling.expect("ceiling").surface_id, "roof");

    let clear = resolver
        .vertical_move(
            &pose(0, 0, 10),
            15,
            medium_height(),
            &capability,
            FlightState::Flying,
        )
        .expect("clear move");
    assert!(!clear.blocked);
    assert_eq!(clear.allowed_elevation_ft, 15);
    assert!(clear.ceiling.is_none());

    let floor = resolver
        .vertical_move(
            &pose(0, 0, 10),
            -5,
            medium_height(),
            &capability,
            FlightState::Flying,
        )
        .expect("floor move");
    assert!(floor.blocked);
    assert_eq!(floor.allowed_elevation_ft, 0);
    assert_eq!(floor.moved_ft, -10);

    // From above a ceiling, descending stops on its top surface.
    let drop = resolver
        .vertical_move(
            &pose(0, 0, 30),
            10,
            medium_height(),
            &capability,
            FlightState::Flying,
        )
        .expect("drop onto ceiling");
    assert!(drop.blocked);
    assert_eq!(drop.allowed_elevation_ft, 21);
    assert_eq!(drop.moved_ft, -9);

    assert_eq!(
        resolver.vertical_move(
            &pose(0, 0, 0),
            5,
            medium_height(),
            &capability,
            FlightState::Grounded
        ),
        Err(FlightError::InvalidTransition {
            from: FlightState::Grounded,
            action: FlightAction::Fly,
        })
    );
    assert_eq!(
        resolver.vertical_move(
            &pose(0, 0, 10),
            5,
            medium_height(),
            &capability,
            FlightState::Falling
        ),
        Err(FlightError::InvalidTransition {
            from: FlightState::Falling,
            action: FlightAction::Fly,
        })
    );
    let no_fly = FlightCapability {
        fly_speed_ft: 0,
        ..capability
    };
    assert_eq!(
        resolver.vertical_move(
            &pose(0, 0, 10),
            15,
            medium_height(),
            &no_fly,
            FlightState::Flying
        ),
        Err(FlightError::NoFlySpeed)
    );
}

#[test]
fn forced_movement_drop_is_resolved_from_elevation_not_path_cost() {
    let profile = SpatialProfile::for_ruleset(Some("2024"));
    let cliff_cells = vec![MovementCell::elevated(30), MovementCell::elevated(0)];
    let cliff_map = MovementMap::new(2, 1, cliff_cells).expect("cliff movement map");
    let mover = SpatialBody::new("synthetic-mover", pose(0, 0, 30), Size::Medium);
    let plan = MovementPlanner::new(&cliff_map, &profile, &mover)
        .forced_move((1, 0), 30)
        .expect("forced move plan");
    assert_eq!(plan.stopped_by, Some(MoveBlock::Drop));
    assert!(plan.steps.is_empty());
    assert_eq!(plan.moved_ft, 0);

    let fall_map = FallMap::flat(2, 1, 0)
        .with_column(0, 0, FallColumn::new(30))
        .with_column(1, 0, FallColumn::new(0));
    let fall = FallResolver::new(&fall_map, rules_2024())
        .resolve_into(&pose(0, 0, 30), (1, 0), medium_height(), FallCause::Forced)
        .expect("forced fall");
    assert_eq!(fall.fall_distance_ft, 30);
    assert_eq!(fall.cause, FallCause::Forced);

    // On flat ground a 15 ft forced move produces no vertical drop at all.
    let flat_map = MovementMap::open(4, 1);
    let flat_mover = SpatialBody::new("synthetic-mover", pose(0, 0, 0), Size::Medium);
    let flat_plan = MovementPlanner::new(&flat_map, &profile, &flat_mover)
        .forced_move((1, 0), 15)
        .expect("flat forced move");
    assert_eq!(flat_plan.moved_ft, 15);
    let flat_fall = FallResolver::new(&FallMap::flat(4, 1, 0), rules_2024())
        .resolve_into(&pose(0, 0, 0), (3, 0), medium_height(), FallCause::Forced)
        .expect("flat fall");
    assert_eq!(flat_fall.fall_distance_ft, 0);
}

#[test]
fn fall_map_reads_scene_floors_terrain_and_ceilings() {
    let mut scene_json = grid_scene(
        &["^..#"],
        json!({
            "^": {"elevation": 30},
            ".": {"elevation": 0},
            "#": {"wall": true}
        }),
        Some(100),
    );
    scene_json["terrain_volumes"] = json!([{
        "id": "platform",
        "cell": {"x": 2, "y": 0, "z": 10},
        "footprint": {"width": 1, "depth": 1},
        "height_ft": 1
    }]);
    let scene: Scene = serde_json::from_value(scene_json).expect("synthetic scene");
    let map = FallMap::from_scene(&scene).expect("grid fall map");
    assert_eq!(map.floor_at(0, 0), Some(30));
    assert_eq!(map.floor_at(1, 0), Some(0));
    assert_eq!(map.floor_at(3, 0), None);
    let resolver = FallResolver::new(&map, rules_2024());

    let cliff = resolver
        .resolve_into(&pose(0, 0, 30), (1, 0), medium_height(), FallCause::Forced)
        .expect("scene cliff fall");
    assert_eq!(cliff.fall_distance_ft, 30);

    let platform = resolver
        .resolve_into(&pose(0, 0, 30), (2, 0), medium_height(), FallCause::Forced)
        .expect("scene platform fall");
    assert_eq!(platform.fall_distance_ft, 19);
    assert_eq!(platform.landing.elevation_ft, 11);
    assert_eq!(platform.landing.surface_id, "platform");
    assert_eq!(platform.landing.surface_kind, FallSurfaceKind::Terrain);

    let ceiling = resolver
        .resolve_from(&pose(1, 0, 110), medium_height(), FallCause::Jump)
        .expect("scene ceiling fall");
    assert_eq!(ceiling.fall_distance_ft, 9);
    assert_eq!(ceiling.landing.elevation_ft, 101);
    assert_eq!(ceiling.landing.surface_kind, FallSurfaceKind::Ceiling);

    assert_eq!(
        resolver.resolve_into(&pose(0, 0, 30), (3, 0), medium_height(), FallCause::Forced),
        Err(FlightError::UnknownColumn { x: 3, y: 0 })
    );
}

// ---------------------------------------------------------------------------
// AC-3 / AC-9: engine application of damage and landing condition
// ---------------------------------------------------------------------------

#[test]
fn engine_apply_fall_applies_damage_and_landing_condition() {
    let mut state = fall_state("2024", 30, FlightState::Falling, false, true);
    let mut rng = ScriptedRng::new(&[6, 6, 6]);
    let mut log = Vec::new();
    let report = engine::apply_fall(
        &mut state,
        "Faller",
        &FallOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("spatial fall is enabled");

    assert_eq!(report.resolution.fall_distance_ft, 30);
    assert_eq!(report.resolution.damage.expression(), "3d6");
    assert_eq!(
        report.resolution.body_height.source,
        HeightSource::SizeDefault(Size::Medium)
    );
    assert_eq!(report.resolution.body_height.height_ft, 5);
    assert_eq!(report.damage_rolled, 18);
    assert_eq!(report.damage_taken, 9);
    assert_eq!(state.combatants[0].hp, 21);
    assert_eq!(report.landing.condition, LandingCondition::LandedFlat);
    assert_eq!(report.landing.rule, LandingRule::RawFall);
    assert!(report.landing_applied);
    assert_eq!(state.combatants[0].conditions[0].condition, "prone");

    let vertical = state.combatants[0].vertical.expect("vertical state");
    assert_eq!(vertical.flight, FlightState::Grounded);
    assert_eq!(vertical.altitude_ft, 0);
    assert_eq!(vertical.fall_distance_ft, 30);
    let pose = state.combatants[0].pose.expect("pose");
    assert_eq!(pose.cell.z, 0);
    assert!(
        log.iter().any(|line| line.contains("falls 30 ft")),
        "{log:?}"
    );
}

#[test]
fn engine_apply_fall_separates_r09_house_rule_from_raw() {
    let mut avoided_state = fall_state("2024", 10, FlightState::Falling, false, false);
    let mut rng = ScriptedRng::new(&[6]);
    let mut log = Vec::new();
    let avoided = engine::apply_fall(
        &mut avoided_state,
        "Faller",
        &FallOptions {
            landing_check_total: Some(10),
            liquid_landing_total: None,
        },
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("fall");
    assert_eq!(avoided.landing.condition, LandingCondition::Standing);
    assert_eq!(avoided.landing.rule, LandingRule::HouseRuleR09Avoided);
    assert!(!avoided.landing_applied);
    assert!(avoided_state.combatants[0].conditions.is_empty());

    let mut failed_state = fall_state("2024", 10, FlightState::Falling, false, false);
    let mut rng = ScriptedRng::new(&[6]);
    let mut log = Vec::new();
    let failed = engine::apply_fall(
        &mut failed_state,
        "Faller",
        &FallOptions {
            landing_check_total: Some(9),
            liquid_landing_total: None,
        },
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("fall");
    assert_eq!(failed.landing.condition, LandingCondition::LandedFlat);
    assert_eq!(failed.landing.rule, LandingRule::RawFall);
    assert_eq!(failed_state.combatants[0].conditions[0].condition, "prone");

    let mut no_damage = fall_state("2024", 5, FlightState::Falling, false, false);
    let mut rng = ScriptedRng::new(&[]);
    let mut log = Vec::new();
    let no_damage = engine::apply_fall(
        &mut no_damage,
        "Faller",
        &FallOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("fall");
    assert_eq!(no_damage.damage_rolled, 0);
    assert_eq!(no_damage.landing.condition, LandingCondition::Standing);
    assert_eq!(no_damage.landing.rule, LandingRule::NoDamage);
    assert!(!no_damage.landing_applied);

    let mut deep = fall_state("2024", 20, FlightState::Falling, false, false);
    let mut rng = ScriptedRng::new(&[6, 6]);
    let mut log = Vec::new();
    let deep = engine::apply_fall(
        &mut deep,
        "Faller",
        &FallOptions {
            landing_check_total: Some(20),
            liquid_landing_total: None,
        },
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("fall");
    assert!(deep.resolution.landing_check.is_none());
    assert_eq!(deep.landing.condition, LandingCondition::LandedFlat);
}

#[test]
fn engine_fall_uses_state_spatial_overrides() {
    let ceiling_override: SpatialState =
        serde_json::from_value(json!({"cell_height_ft": 20})).expect("ceiling override");
    let state = state_with(
        registry("2024"),
        combatant(
            "Faller",
            creature("synthetic-faller", Size::Medium, 30, false, 0),
            40,
            FlightState::Falling,
            &[],
        ),
        Some(flat_scene()),
        Some(ceiling_override),
    );
    let ceiling = engine::resolve_fall(&state, "Faller")
        .unwrap()
        .expect("ceiling override is enabled");
    assert_eq!(ceiling.fall_distance_ft, 19);
    assert_eq!(ceiling.landing.elevation_ft, 21);
    assert_eq!(ceiling.landing.surface_kind, FallSurfaceKind::Ceiling);

    let terrain_override: SpatialState = serde_json::from_value(json!({
        "terrain_volumes": [{
            "id": "state-platform",
            "cell": {"x": 0, "y": 0, "z": 10},
            "footprint": {"width": 1, "depth": 1},
            "height_ft": 1
        }]
    }))
    .expect("terrain override");
    let mut state = state_with(
        registry("2024"),
        combatant(
            "Faller",
            creature("synthetic-faller", Size::Medium, 30, false, 0),
            40,
            FlightState::Falling,
            &[],
        ),
        Some(flat_scene()),
        Some(terrain_override),
    );
    let terrain = engine::resolve_fall(&state, "Faller")
        .unwrap()
        .expect("terrain override is enabled");
    assert_eq!(terrain.fall_distance_ft, 29);
    assert_eq!(terrain.landing.elevation_ft, 11);
    assert_eq!(terrain.landing.surface_kind, FallSurfaceKind::Terrain);
    assert_eq!(terrain.landing.surface_id, "state-platform");

    let mut rng = ScriptedRng::new(&[6, 6]);
    let mut log = Vec::new();
    let report = engine::apply_fall(
        &mut state,
        "Faller",
        &FallOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("fall");
    assert_eq!(report.damage_rolled, 12);
    let pose = state.combatants[0].pose.expect("pose");
    assert_eq!(pose.cell.z, 11);
    let vertical = state.combatants[0].vertical.expect("vertical");
    assert_eq!(vertical.altitude_ft, 0);
    assert_eq!(vertical.flight, FlightState::Grounded);
}

#[test]
fn engine_fall_start_uses_floor_plus_vertical_without_a_pose() {
    let airborne: CombatantState = serde_json::from_value(json!({
        "creature": creature("synthetic-airborne", Size::Medium, 30, false, 0),
        "name": "Airborne",
        "side": "party",
        "max_hp": 30,
        "hp": 30,
        "zone": "open",
        "pos": {"kind": "cell", "x": 0, "y": 0},
        "vertical": {"altitude_ft": 10, "flight": "falling", "fall_distance_ft": 0}
    }))
    .expect("airborne combatant without a pose");
    let state = state_with(
        registry("2024"),
        airborne,
        Some(grid_scene(&["^"], json!({"^": {"elevation": 30}}), None)),
        Some(SpatialState::default()),
    );
    let fall = engine::resolve_fall(&state, "Airborne")
        .unwrap()
        .expect("fall");
    assert_eq!(fall.start_elevation_ft, 40);
    assert_eq!(fall.start_floor_ft, 30);
    assert_eq!(fall.fall_distance_ft, 10);
    assert_eq!(fall.landing.elevation_ft, 30);
    assert_eq!(fall.damage.expression(), "1d6");
}

// ---------------------------------------------------------------------------
// AC-6: legacy 2D behaviour is unchanged unless explicitly enabled
// ---------------------------------------------------------------------------

#[test]
fn legacy_state_never_reaches_the_fall_resolver() {
    let mut state = state_with(registry("2014"), legacy_combatant("Legacy"), None, None);
    assert!(!engine::spatial_flight_enabled(&state));
    assert!(engine::resolve_fall(&state, "Legacy").unwrap().is_none());
    let mut rng = ScriptedRng::new(&[]);
    let mut log = Vec::new();
    let report = engine::apply_fall(
        &mut state,
        "Legacy",
        &FallOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap();
    assert!(report.is_none());
    assert_eq!(state.combatants[0].hp, 30);
    assert!(state.combatants[0].conditions.is_empty());
    assert!(state.combatants[0].vertical.is_none());
    assert!(!engine::flight_check(&state, "Legacy").unwrap().falls);
}

#[test]
fn legacy_grid_scene_stays_legacy_until_spatial_state_is_present() {
    let legacy = state_with(
        registry("2014"),
        legacy_combatant("Legacy"),
        Some(flat_scene()),
        None,
    );
    assert!(!engine::spatial_flight_enabled(&legacy));
    assert!(engine::resolve_fall(&legacy, "Legacy").unwrap().is_none());

    let enabled = state_with(
        registry("2014"),
        legacy_combatant("Legacy"),
        Some(flat_scene()),
        Some(SpatialState::default()),
    );
    assert!(engine::spatial_flight_enabled(&enabled));
    let resolution = engine::resolve_fall(&enabled, "Legacy")
        .unwrap()
        .expect("explicit spatial state enables the resolver");
    assert_eq!(resolution.fall_distance_ft, 0);

    let mut enabled = enabled;
    let mut rng = ScriptedRng::new(&[]);
    let mut log = Vec::new();
    let report = engine::apply_fall(
        &mut enabled,
        "Legacy",
        &FallOptions::default(),
        &mut rng,
        &mut log,
    )
    .unwrap()
    .expect("explicit spatial state enables application");
    assert_eq!(report.damage_rolled, 0);
    assert_eq!(enabled.combatants[0].hp, 30);
    assert!(enabled.combatants[0].conditions.is_empty());
    assert_eq!(
        enabled.combatants[0].vertical.expect("vertical").flight,
        FlightState::Grounded
    );
}

// ---------------------------------------------------------------------------
// AC-8: real TASK-0094 hover/burrow data reaches the capability seam
// ---------------------------------------------------------------------------
//
// The 2014 real data lives in the tracked `srd-monsters.yaml`; the generated
// `.pack.json` is a separate artifact and is not used as the source of truth
// here. Loading the real 2014 fixture registry matches the #178 test and keeps
// this test independent of pack regeneration.

#[derive(serde::Deserialize)]
struct SpatialOverlay {
    creatures: Vec<Creature>,
}

fn real_2014_registry() -> ContentRegistry {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rulesets/real-2014.yaml");
    ContentSources {
        campaign: Some(manifest),
        ..ContentSources::default()
    }
    .load()
    .expect("real 2014 SRD registry should load")
}

#[test]
fn real_hover_and_burrow_fields_reach_flight_capability() {
    let registry = real_2014_registry();
    let hovering = registry
        .creatures
        .iter()
        .find(|(_, creature)| creature.speed().fly > 0 && creature.speed().hover)
        .expect("at least one 2014 hover creature in the real registry");
    let hovering = FlightCapability::from_speed(&hovering.1.speed());
    assert!(hovering.can_fly());
    assert!(hovering.can_hover());
    assert!(hovering.fly_speed_ft > 0);

    let burrowing = registry
        .creatures
        .iter()
        .find(|(_, creature)| creature.speed().burrow > 0)
        .expect("at least one 2014 burrow creature in the real registry");
    let burrowing = FlightCapability::from_speed(&burrowing.1.speed());
    assert!(burrowing.can_burrow());
    assert!(burrowing.burrow_speed_ft > 0);

    // Keep the 2024 overlay assertions: the overlay is the real 2024 source.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let overlay_path = root.join("LetsPlay/Rules/combat/2024/monsters/srd-spatial.yaml");
    let text = fs::read_to_string(&overlay_path).expect("tracked 2024 SRD spatial overlay");
    let overlay: SpatialOverlay =
        serde_norway::from_str(&text).expect("parse 2024 SRD spatial overlay");
    assert!(!overlay.creatures.is_empty());

    let hovering = overlay
        .creatures
        .iter()
        .find(|creature| creature.speed().fly > 0 && creature.speed().hover)
        .expect("at least one hover creature in the overlay");
    let hovering = FlightCapability::from_speed(&hovering.speed());
    assert!(hovering.can_fly());
    assert!(hovering.can_hover());

    let burrowing = overlay
        .creatures
        .iter()
        .find(|creature| creature.speed().burrow > 0)
        .expect("at least one burrow creature in the overlay");
    let burrowing = FlightCapability::from_speed(&burrowing.speed());
    assert!(burrowing.can_burrow());
    assert!(burrowing.burrow_speed_ft > 0);
}
