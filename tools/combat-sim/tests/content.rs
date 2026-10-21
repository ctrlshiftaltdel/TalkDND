//! Loading, inheritance, resolution and validation tests for the v2 content
//! layer. All fixtures use generic names.

use std::path::{Path, PathBuf};

use combat_sim::content::schema::{AbilityName, Effect, ExprValue, Predicate, SaveEndsAt};
use combat_sim::content::{check, ContentRegistry, ContentSources, ItemKind};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/content")
        .join(relative)
}

fn campaign_sources() -> ContentSources {
    ContentSources {
        campaign: Some(fixture("manifest.yaml")),
        ..ContentSources::default()
    }
}

fn load_campaign() -> ContentRegistry {
    campaign_sources()
        .load()
        .expect("fixture campaign should load")
}

fn library_sources(path: &str) -> ContentSources {
    ContentSources {
        libraries: vec![fixture(path)],
        ..ContentSources::default()
    }
}

fn messages(registry: &ContentRegistry) -> String {
    check(registry)
        .issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn loads_all_sources_with_counts_and_origins() {
    let registry = load_campaign();

    assert_eq!(registry.conditions.len(), 4);
    assert_eq!(registry.masteries.len(), 1);
    assert_eq!(registry.abilities.len(), 3);
    assert_eq!(registry.creatures.len(), 6);
    assert_eq!(registry.ai_templates.len(), 2);
    assert_eq!(registry.scenes.len(), 2);
    assert_eq!(registry.encounters.len(), 2);

    assert_eq!(registry.manifest_name.as_deref(), Some("Fixture Campaign"));
    assert_eq!(
        registry.manifest_path.as_ref(),
        Some(&fixture("manifest.yaml"))
    );
    assert_eq!(registry.state_dir.as_ref(), Some(&fixture("state")));

    assert_eq!(
        registry.creatures.source("dummy-a").unwrap().path,
        fixture("library/core.yaml")
    );
    assert_eq!(
        registry.abilities.source("zap").unwrap().path,
        fixture("library/core.yaml")
    );
    assert_eq!(
        registry.conditions.source("exposed").unwrap().path,
        fixture("library/more.json")
    );
    assert_eq!(
        registry.conditions.source("steady").unwrap().path,
        fixture("chars/container.md")
    );

    let hero = registry.creatures.source("hero-a").unwrap();
    assert_eq!(hero.path, fixture("chars/party.md"));
    assert!(
        hero.line.is_some_and(|line| line > 0),
        "markdown source should carry the opening fence line"
    );
}

#[test]
fn override_replaces_later_sources_by_id() {
    let registry = load_campaign();
    let twin = registry.creatures.get("twin-a").expect("twin-a is loaded");
    assert_eq!(twin.name.as_deref(), Some("Twin A Revised"));
    assert_eq!(twin.ac, Some(13));
    assert_eq!(
        registry.creatures.source("twin-a").unwrap().path,
        fixture("library/override.yaml")
    );

    let rules = &registry.rules;
    let actions = rules.basic_actions.as_ref().unwrap();
    assert_eq!(actions.get("dodge").map(String::as_str), Some("dazed"));
    assert_eq!(
        actions.get("hide").map(String::as_str),
        Some("dazed"),
        "later rules maps merge entry by entry"
    );
    let cover = rules.cover.as_ref().unwrap();
    assert_eq!(cover.half, Some(3), "later cover value wins");
    assert_eq!(
        cover.three_quarters,
        Some(5),
        "earlier cover value survives"
    );
    assert_eq!(
        rules.dying_conditions.as_deref(),
        Some(["dazed".to_string()].as_slice())
    );
    assert_eq!(rules.concentration_condition.as_deref(), Some("staggered"));
    assert_eq!(
        registry.rules_source.as_ref().map(|source| &source.path),
        Some(&fixture("library/override.yaml"))
    );
}

#[test]
fn base_inheritance_merges_maps_and_replaces_fields() {
    let registry = load_campaign();
    let scout = registry.resolve_creature("scout-b").unwrap();
    assert_eq!(scout.id, "scout-b");
    assert_eq!(scout.name.as_deref(), Some("Scout B"));
    assert_eq!(scout.ac, Some(12));
    assert_eq!(scout.hp, Some(10));
    assert_eq!(scout.attacks.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        scout.attacks.as_ref().unwrap()[0].id,
        "club",
        "attacks are inherited from the base"
    );
    let abilities = scout.abilities.as_ref().unwrap();
    assert_eq!(abilities.get("str"), Some(&14), "base map entry survives");
    assert_eq!(abilities.get("dex"), Some(&16), "child map entry wins");
    assert!(
        scout.resources.as_ref().unwrap().contains_key("focus"),
        "base resources survive"
    );
    assert_eq!(
        scout.ai.as_ref().map(|ai| ai.template.as_str()),
        Some("skirmish-a")
    );
    assert!(scout
        .tags
        .as_ref()
        .unwrap()
        .contains(&"scout-b".to_string()));
}

#[test]
fn resolves_ability_refs_with_params_and_overrides() {
    let registry = load_campaign();
    let dummy = registry.creatures.get("dummy-a").unwrap();
    let actions = dummy.actions.as_ref().unwrap();
    let resolved = registry.resolve_ability(&actions[0]).unwrap();
    assert_eq!(resolved.ability.id, "zap");
    assert_eq!(
        resolved
            .ability
            .cost
            .as_ref()
            .map(|cost| cost.resource.as_str()),
        Some("focus"),
        "entry cost overrides the referenced ability"
    );
    assert_eq!(
        resolved.params.get("dice"),
        Some(&ExprValue::Text("1d8".to_string()))
    );

    let entry: combat_sim::content::schema::AbilityEntry =
        serde_norway::from_str("{ref: zap, name: Zappy, cost: {resource: focus, amount: 1}}")
            .expect("YAML ability ref should parse");
    let resolved = registry.resolve_ability(&entry).unwrap();
    assert_eq!(resolved.ability.name.as_deref(), Some("Zappy"));
    assert_eq!(
        resolved.ability.cost.as_ref().map(|cost| cost.amount),
        Some(1)
    );
}

#[test]
fn markdown_blocks_load_and_other_fences_are_ignored() {
    let registry = load_campaign();
    let hero_a = registry.creatures.get("hero-a").unwrap();
    assert_eq!(hero_a.ac, Some(15));
    assert_eq!(
        hero_a.conditions.as_deref(),
        Some(["dazed".to_string()].as_slice()),
        "a single creature with an initial `conditions` list is not a container"
    );

    let hero_b = registry.creatures.get("hero-b").unwrap();
    assert_eq!(hero_b.ac, Some(13));
    assert!(
        registry.creatures.get("ignored-creature").is_none(),
        "non-combat fences must be ignored"
    );
}

#[test]
fn manifest_paths_are_relative_and_env_falls_back() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let registry = load_campaign();
    assert_eq!(registry.scenes.get("hall-a").unwrap().id, "hall-a");
    assert_eq!(registry.state_dir.as_ref(), Some(&fixture("state")));

    std::env::set_var("COMBAT_SIM_CAMPAIGN", fixture("manifest.yaml"));
    let loaded = ContentSources::default()
        .load()
        .expect("COMBAT_SIM_CAMPAIGN should be used when --campaign is absent");
    std::env::remove_var("COMBAT_SIM_CAMPAIGN");

    assert_eq!(loaded.manifest_name.as_deref(), Some("Fixture Campaign"));
    assert_eq!(loaded.creatures.len(), 6);
}

#[test]
fn check_accepts_the_fixture_campaign_and_reports_todos() {
    let registry = load_campaign();
    let report = check(&registry);
    assert_eq!(
        report.error_count(),
        0,
        "unexpected errors:\n{}",
        messages(&registry)
    );
    let text = messages(&registry);
    assert!(text.contains("TODO: replace with final numbers"), "{text}");
}

#[test]
fn check_reports_every_reference_category() {
    let registry = library_sources("bad/refs").load().unwrap();
    let text = messages(&registry);
    for expected in [
        "unknown mastery `missing-mastery`",
        "unknown ability `missing-ability`",
        "`use` references unknown ability/attack `missing-use`",
        "unknown ai-template `missing-template`",
        "unknown condition `missing-effect-condition`",
        "unknown condition `missing-implies`",
        "unknown condition `missing-predicate`",
        "unknown condition `missing-initial-condition`",
        "unknown condition `missing-immunity-condition`",
        "unknown condition `missing-ai-condition`",
        "unknown condition `missing-creature-avoid`",
        "unknown condition `missing-rule-condition`",
        "unknown condition `missing-basic-condition`",
        "unknown condition `missing-concentration-condition`",
        "unknown creature `missing-creature`",
        "unknown scene `missing-scene`",
        "unknown zone `missing-zone`",
        "unknown zone `missing-object-zone`",
        "unknown zone `missing-trigger-zone`",
        "unknown zone `missing-party-zone`",
        "unknown zone `missing-member-zone`",
        "unknown scene trigger `missing-trigger`",
        "multiattack references unknown attack/ability `missing-multiattack`",
        "costs unknown resource `missing-resource`",
        "id `Bad_Ref` is invalid",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
    }
    assert!(
        text.contains("`activate` references `nobody`"),
        "activate should warn for unmatched instance names:\n{text}"
    );
}

#[test]
fn check_reports_base_errors() {
    let registry = library_sources("bad/base").load().unwrap();
    let text = messages(&registry);
    assert!(
        text.contains("unknown base `missing-base`"),
        "missing base not reported:\n{text}"
    );
    assert!(
        text.contains("base cycle"),
        "base cycle not reported:\n{text}"
    );
}

#[test]
fn unknown_fields_keep_their_detail() {
    let error = library_sources("bad/unknown-field.yaml")
        .load()
        .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("unknown field `armor_class`"), "{text}");
    assert!(text.contains("unknown-field.yaml"), "{text}");
}

#[test]
fn effect_payload_errors_keep_inner_unknown_field_detail() {
    let error = library_sources("bad/effect-field.yaml").load().unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("unknown field `typ`"), "{text}");
    assert!(text.contains("`dice`"), "{text}");
}

#[test]
fn duration_requires_exactly_one_kind() {
    let error = library_sources("bad/duration.yaml").load().unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("exactly one"), "{text}");
}

#[test]
fn check_reports_expression_errors() {
    let registry = library_sources("bad/expression.yaml").load().unwrap();
    let text = messages(&registry);
    assert!(
        text.contains("unknown expression variable `nope`"),
        "{text}"
    );
    assert!(text.contains("unmatched `{`"), "{text}");
}

#[test]
fn apply_condition_save_ends_deserializes_and_validates() {
    let registry = library_sources("save-ends.yaml").load().unwrap();
    let ability = registry.abilities.get("linger").expect("linger is loaded");
    let [Effect::ApplyCondition(apply)] = ability.effects.as_slice() else {
        panic!("expected one apply_condition effect: {:?}", ability.effects);
    };
    let save_ends = apply
        .save_ends
        .as_ref()
        .expect("save_ends should deserialize");
    assert_eq!(save_ends.ability, AbilityName::Wis);
    assert_eq!(save_ends.dc, ExprValue::Text("10+{level}".to_string()));
    assert_eq!(save_ends.at, SaveEndsAt::EndOfTurn);

    let report = check(&registry);
    assert_eq!(
        report.error_count(),
        0,
        "unexpected errors:\n{}",
        messages(&registry)
    );
}

#[test]
fn check_reports_invalid_save_ends_dc() {
    let registry = library_sources("bad/save-ends.yaml").load().unwrap();
    let text = messages(&registry);
    assert!(
        text.contains("save_ends dc: unknown expression variable `nope`"),
        "{text}"
    );
}

#[test]
fn save_ends_rejects_unknown_fields() {
    let error = serde_norway::from_str::<Effect>(
        "apply_condition: {condition: wavering, save_ends: {ability: wis, dc: 10, at: end_of_turn, later: true}}",
    )
    .unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("unknown field `later`"), "{text}");
}

#[test]
fn markdown_errors_include_the_block_line() {
    let error = library_sources("bad/markdown.md").load().unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("markdown.md:3"), "{text}");
    assert!(text.contains("unknown field `mystery`"), "{text}");
}

#[test]
fn plain_map_effects_and_predicates_deserialize() {
    let effects: Vec<Effect> = serde_norway::from_str(
        r#"
        - damage: {dice: "1d6", type: fire}
        - apply_condition: {condition: dazed, to: target}
        "#,
    )
    .expect("plain-map effects should deserialize");
    assert_eq!(effects.len(), 2);
    assert!(matches!(effects[0], Effect::Damage(_)));
    assert!(matches!(effects[1], Effect::ApplyCondition(_)));

    let predicate: Predicate =
        serde_norway::from_str("has_advantage:").expect("null predicate is true");
    assert_eq!(predicate, Predicate::HasAdvantage(Default::default()));

    let predicate: Predicate =
        serde_norway::from_str("{has_disadvantage: true}").expect("boolean predicate");
    assert_eq!(
        predicate,
        Predicate::HasDisadvantage(combat_sim::content::schema::Flag(true))
    );

    let predicate: Predicate = serde_norway::from_str("{not: {linked_is_target: true}}").unwrap();
    assert!(matches!(predicate, Predicate::Not(_)));

    let serialized = serde_json::to_value(&effects).unwrap();
    assert!(
        serialized[0].get("damage").is_some(),
        "serialization keeps the same map shape: {serialized}"
    );
}

#[test]
fn recharge_forms_parse_and_serialize() {
    use combat_sim::content::schema::{Recharge, ResourceDef};

    let resource: ResourceDef = serde_norway::from_str("max: 1\nrecharge: recharge 5-6").unwrap();
    assert_eq!(resource.recharge, Recharge::Range { min: 5, max: 6 });
    assert_eq!(resource.recharge.to_string(), "recharge 5-6");

    let exact: Recharge = serde_norway::from_str("recharge 5").unwrap();
    assert_eq!(exact, Recharge::Single(5));
    assert_eq!(serde_json::to_value(&exact).unwrap(), "recharge 5");

    let keyword: Recharge = serde_norway::from_str("short").unwrap();
    assert_eq!(keyword, Recharge::Short);
    assert_eq!(serde_json::to_value(&keyword).unwrap(), "short");
}

#[test]
fn items_are_available_by_kind() {
    let registry = load_campaign();
    let creatures = registry.entries(ItemKind::Creature);
    assert!(creatures.iter().any(|(id, _)| *id == "dummy-a"));
    assert!(
        registry.find_any("zap").is_some(),
        "find_any searches every kind"
    );
    assert!(registry.find_any("does-not-exist").is_none());
}

#[test]
fn directory_scan_skips_hidden_dirs_non_content_and_nested_layers() {
    // COMBAT_SIM_CAMPAIGN is set by another test; keep it out of this load.
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use combat_sim::content::ContentSources;
    let root = std::env::temp_dir().join(format!("cs-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let adv = root.join("Adventures/book");
    std::fs::create_dir_all(adv.join(".md/Book/fixes")).unwrap();
    std::fs::create_dir_all(adv.join("combat/extra/common")).unwrap();
    std::fs::create_dir_all(adv.join("combat/extra/2014")).unwrap();
    std::fs::write(adv.join(".md/Book/fixes/review.log.jsonl"), "{}\n").unwrap();
    std::fs::write(
        adv.join(".md/Book/Book.md"),
        "```combat yaml\nnot: [valid\n```\n",
    )
    .unwrap();
    std::fs::write(adv.join("combat/tool.py"), "print(1)\n").unwrap();
    std::fs::write(
        adv.join("combat/extra/2014/broken.yaml"),
        "encounters: [{id: x, scene: missing}]\n",
    )
    .unwrap();
    std::fs::write(
        adv.join("combat/creatures.yaml"),
        "creatures:\n  - {id: cog, name: Cog, ac: 10, hp: 5, abilities: {str: 10, dex: 10, con: 10, int: 10, wis: 10, cha: 10}}\n",
    )
    .unwrap();
    for src in [adv.clone(), adv.join("combat")] {
        let sources = ContentSources {
            adventures: vec![src],
            ..Default::default()
        };
        let reg = sources.load().expect("scan should skip non-content files");
        assert!(reg.creatures.ids().any(|i| i == "cog"));
        assert_eq!(reg.encounters.ids().count(), 0);
    }
    // A file named explicitly must still be content.
    let sources = ContentSources {
        adventures: vec![adv.join("combat/tool.py")],
        ..Default::default()
    };
    assert!(sources.load().is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn edition_folders_follow_the_campaign_ruleset() {
    // COMBAT_SIM_CAMPAIGN is set by another test; keep it out of this load.
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use combat_sim::content::ContentSources;
    let root = std::env::temp_dir().join(format!("cs-edition-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let combat = root.join("Adventures/book/combat");
    let cr = |id: &str| {
        format!("creatures:\n  - {{id: {id}, name: {id}, ac: 10, hp: 5, abilities: {{str: 10, dex: 10, con: 10, int: 10, wis: 10, cha: 10}}}}\n")
    };
    for (dir, id) in [
        ("heists/2014", "old"),
        ("heists/2024", "new"),
        ("deep/x/2014", "deep-old"),
    ] {
        std::fs::create_dir_all(combat.join(dir)).unwrap();
        std::fs::write(combat.join(dir).join("c.yaml"), cr(id)).unwrap();
    }
    std::fs::write(combat.join("shared.yaml"), cr("shared")).unwrap();
    // A file (not folder) named 2014 is not an edition folder.
    std::fs::create_dir_all(combat.join("misc")).unwrap();
    std::fs::write(combat.join("misc/2024.yaml"), cr("file-2024")).unwrap();
    std::fs::create_dir_all(root.join("save")).unwrap();
    for (rs, want, not) in [
        ("2014", ["old", "deep-old"], "new"),
        ("2024", ["new", "shared"], "old"),
    ] {
        let manifest = root.join("save").join(format!("campaign-{rs}.yaml"));
        std::fs::write(
            &manifest,
            format!("name: t\nadventures: [../Adventures/book]\nruleset: \"{rs}\"\n"),
        )
        .unwrap();
        let reg = ContentSources {
            campaign: Some(manifest),
            ..Default::default()
        }
        .load()
        .unwrap();
        let ids: Vec<String> = reg.creatures.ids().map(|s| s.to_string()).collect();
        for w in want.iter().chain(["shared", "file-2024"].iter()) {
            assert!(ids.iter().any(|i| i == w), "{rs}: missing {w} in {ids:?}");
        }
        assert!(!ids.iter().any(|i| i == not), "{rs}: loaded {not}");
        if rs == "2024" {
            assert!(!ids.iter().any(|i| i == "deep-old"));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Serialises tests that set or depend on COMBAT_SIM_CAMPAIGN.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
