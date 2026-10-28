//! v1.6 T2 (TASK-0123): structured spell lists, the schema 9→10 migration
//! and the class spell data behind them. All fixtures are synthetic; no
//! real save is ever read (docs/SPELLCASTING.md §3.5/§3.6, Issue #242).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use talkdnd::chargen::{self, Creation};
use talkdnd::charsheet::{self, CharFile, SpellLists};
use talkdnd::classdata::{self, Book};
use talkdnd::migrate;

fn root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("talkdnd-spells-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Saves")).unwrap();
    dir
}

fn save_at(root: &Path, tag: &str) -> PathBuf {
    let save = root.join("Saves").join(tag);
    for dir in ["Sessions", "sheets", "combat"] {
        fs::create_dir_all(save.join(dir)).unwrap();
    }
    fs::write(save.join("save-meta.yaml"), "schema: 9\ntool: 1.5.0\n").unwrap();
    fs::write(save.join("Sessions/log.md"), "# log\n").unwrap();
    save
}

/// A minimal structured sheet (canonical store format, without `spells`).
fn write_sheet(save: &Path, id: &str, class: &str, ruleset: &str, body_md: &str) {
    let cf = CharFile {
        version: 1,
        id: id.into(),
        name: id.into(),
        file: format!("Characters/{id}.md"),
        controller: Some(charsheet::PLAYER.into()),
        ruleset: ruleset.into(),
        info: BTreeMap::from([("class".into(), class.into())]),
        body_md: body_md.into(),
        combat: serde_norway::from_str(&format!("{{id: {id}, kind: pc, side: party}}")).unwrap(),
        ..Default::default()
    };
    charsheet::store(save, &cf).unwrap();
}

fn spells_of(save: &Path, id: &str) -> SpellLists {
    let text = fs::read_to_string(charsheet::path(save, id)).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).expect("sheet parses");
    cf.spells.expect("spells block")
}

fn write_chargen(save: &Path, json: &str) {
    fs::write(chargen::state_path(save), json).unwrap();
}

// ---------------------------------------------------------------------------
// AC-1: the sheet model
// ---------------------------------------------------------------------------

#[test]
fn old_sheet_loads_without_spells_and_store_preserves_the_block() {
    let root = root("model");
    let save = save_at(&root, "s");
    // An old sheet: no spells field at all.
    write_sheet(&save, "hero", "cleric", "2014", "");
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).unwrap();
    assert!(cf.spells.is_none(), "missing field must load as None");
    assert_eq!(cf.version, 1, "CharFile.version stays 1 (D-1=B)");

    // Round-trip without a block: the field stays absent on disk.
    charsheet::store(&save, &cf).unwrap();
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    assert!(!text.contains("spells:"), "{text}");

    // Round-trip with a block: store keeps every list.
    let mut with = cf;
    with.spells = Some(SpellLists {
        version: 1,
        mode: "prepared".into(),
        ability: "wis".into(),
        cantrips: vec!["guidance".into()],
        prepared: vec!["bless".into()],
        legacy: vec!["旧条目".into()],
        ..Default::default()
    });
    charsheet::store(&save, &with).unwrap();
    let lists = spells_of(&save, "hero");
    assert_eq!(lists.mode, "prepared");
    assert_eq!(lists.ability, "wis");
    assert_eq!(lists.cantrips, vec!["guidance".to_string()]);
    assert_eq!(lists.prepared, vec!["bless".to_string()]);
    assert_eq!(lists.legacy, vec!["旧条目".to_string()]);
    // mode_list() follows the mode.
    assert_eq!(lists.mode_list(), &lists.prepared);
    let mut known = lists.clone();
    known.mode = "known".into();
    assert_eq!(known.mode_list(), &known.known);
    known.mode = "spellbook".into();
    assert_eq!(known.mode_list(), &known.book);
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// AC-2/AC-6: the schema 9→10 migration
// ---------------------------------------------------------------------------

#[test]
fn migrates_a_v9_save_from_chargen_json_and_is_idempotent() {
    let root = root("mig-chargen");
    let save = save_at(&root, "2030-01-01-01");
    write_sheet(&save, "hero", "cleric", "2014", "");
    write_chargen(
        &save,
        r#"{"ruleset":"2014","class":"cleric","level":1,"spells_done":true,
            "spells":["Bless","guidance","Cure Wounds","远程治疗术"]}"#,
    );

    // Dry-run: reports the plan, writes nothing.
    let dry = migrate::ensure(&save, &root, true).unwrap();
    assert!((dry.from, dry.to) == (9, migrate::CURRENT_SCHEMA));
    assert_eq!(dry.spell_sheets, vec!["hero".to_string()]);
    assert!(
        dry.spell_legacy.iter().any(|l| l.contains("远程治疗术")),
        "{dry:?}"
    );
    assert!(
        !fs::read_to_string(charsheet::path(&save, "hero"))
            .unwrap()
            .contains("spells:"),
        "dry-run must not write"
    );
    assert!(
        !root.join("Saves/_backups").exists(),
        "dry-run must not back up"
    );

    let report = migrate::ensure(&save, &root, false).unwrap();
    assert_eq!(migrate::load(&save).unwrap().unwrap().schema, 10);
    assert!(report.backup.is_some(), "whole-save backup before writing");
    assert_eq!(report.spell_sheets, vec!["hero".to_string()]);
    assert!(report.spell_dirty.is_empty());

    // Adopted from chargen.json: precise id or slug-of-name, rest → legacy.
    let lists = spells_of(&save, "hero");
    assert_eq!(lists.version, 1);
    assert_eq!(lists.mode, "prepared", "2014 cleric prepares");
    assert_eq!(lists.ability, "wis");
    assert_eq!(lists.cantrips, vec!["guidance".to_string()]);
    assert_eq!(lists.prepared.len(), 2, "{lists:?}");
    assert!(lists.prepared.contains(&"bless".to_string()));
    assert!(
        lists.prepared.contains(&"cure-wounds".to_string()),
        "slug-of-name matches \"Cure Wonds\"-style text: {lists:?}"
    );
    assert_eq!(lists.legacy, vec!["远程治疗术".to_string()]);

    // The backup holds the pre-migration sheet (restorable), the session
    // log records the migration.
    let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves/_backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    let backup_sheet = fs::read_to_string(backups[0].join("sheets/hero.yaml")).unwrap();
    assert!(!backup_sheet.contains("spells:"), "{backup_sheet}");
    let log = fs::read_to_string(save.join("Sessions/log.md")).unwrap();
    assert!(log.contains("schema 9→10"), "{log}");

    // Idempotent: a second run is a no-op.
    let again = migrate::ensure(&save, &root, false).unwrap();
    assert!(again.already_current(), "{again:?}");
    assert!(again.spell_sheets.is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migrates_from_markdown_fallback_with_book_mode_for_wizard() {
    let root = root("mig-md");
    let save = save_at(&root, "2030-02-02-01");
    let body = "# 法师\n\n## 法术\n\n- Fireball\n- Mage Hand\n- 随手写的法术\n\n## 特性\n";
    write_sheet(&save, "mira", "wizard", "2024", body);
    // chargen.json for a different class: must NOT be adopted here.
    write_chargen(
        &save,
        r#"{"ruleset":"2024","class":"cleric","level":1,"spells_done":true,
            "spells":["bless"]}"#,
    );

    migrate::ensure(&save, &root, false).unwrap();
    let lists = spells_of(&save, "mira");
    assert_eq!(lists.mode, "spellbook", "wizard keeps a book");
    assert_eq!(lists.ability, "int");
    assert_eq!(lists.book, vec!["fireball".to_string()]);
    assert_eq!(lists.cantrips, vec!["mage-hand".to_string()]);
    assert_eq!(lists.legacy, vec!["随手写的法术".to_string()]);
    assert!(lists.prepared.is_empty());
    // The Markdown body is never rewritten by the migration.
    let text = fs::read_to_string(charsheet::path(&save, "mira")).unwrap();
    assert!(
        text.contains("随手写的法术"),
        "body_md keeps the original line"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_adopts_2024_new_cantrips_as_cantrips() {
    let root = root("mig-2024-cantrips");
    let save = save_at(&root, "2030-03-03-01");
    let body = "## 法术\n\n- Elementalism\n- Sorcerous Burst\n- Burning Hands\n";
    write_sheet(&save, "zara", "sorcerer", "2024", body);
    migrate::ensure(&save, &root, false).unwrap();
    let lists = spells_of(&save, "zara");
    // 2024 spells the old static CANTRIPS list does not know must land in
    // cantrips (level 0 from the generated class table), not in prepared.
    assert!(
        lists.cantrips.contains(&"elementalism".to_string()),
        "{lists:?}"
    );
    assert!(
        lists.cantrips.contains(&"sorcerous-burst".to_string()),
        "{lists:?}"
    );
    assert_eq!(lists.prepared, vec!["burning-hands".to_string()]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_skips_dirty_sheets_in_both_shapes() {
    let root = root("mig-dirty");
    // Dirty sheet without a spells block: must stay byte-identical.
    let save = save_at(&root, "2030-04-04-01");
    write_sheet(&save, "hero", "cleric", "2014", "## 法术\n\n- Bless\n");
    write_sheet(&save, "scribe", "wizard", "2024", "");
    // Give the second sheet a hand-written block before flagging it dirty.
    let scribe_path = charsheet::path(&save, "scribe");
    let mut scribe_text = fs::read_to_string(&scribe_path).unwrap();
    scribe_text.push_str("spells:\n  version: 1\n  mode: spellbook\n  ability: int\n  book: [fireball]\n  # hand comment\n");
    fs::write(&scribe_path, &scribe_text).unwrap();

    combat_sim::integrity::ensure(&save).unwrap();
    // Hand-edit both sheets after the baseline so both turn dirty.
    let hero_path = charsheet::path(&save, "hero");
    let hero_text = fs::read_to_string(&hero_path).unwrap();
    fs::write(&hero_path, format!("{hero_text}\n# DM note\n")).unwrap();
    fs::write(&scribe_path, format!("{scribe_text}\n# DM note\n")).unwrap();

    let report = migrate::ensure(&save, &root, false).unwrap();
    assert_eq!(
        report.spell_dirty,
        vec!["hero".to_string(), "scribe".to_string()],
        "{report:?}"
    );
    // Byte-identical, still flagged, no serde round-trip.
    assert_eq!(
        fs::read_to_string(&hero_path).unwrap(),
        format!("{hero_text}\n# DM note\n")
    );
    assert_eq!(
        fs::read_to_string(&scribe_path).unwrap(),
        format!("{scribe_text}\n# DM note\n")
    );
    let changes = combat_sim::integrity::changes(&save).unwrap();
    assert!(
        changes.iter().any(|c| c.path() == "sheets/hero.yaml"),
        "{changes:?}"
    );
    assert!(
        changes.iter().any(|c| c.path() == "sheets/scribe.yaml"),
        "{changes:?}"
    );

    // The dirty sheet still loads: both shapes parse.
    let cf: CharFile = serde_norway::from_str(&fs::read_to_string(&scribe_path).unwrap()).unwrap();
    assert_eq!(
        cf.spells.as_ref().unwrap().book,
        vec!["fireball".to_string()]
    );
    let cf: CharFile = serde_norway::from_str(&fs::read_to_string(&hero_path).unwrap()).unwrap();
    assert!(cf.spells.is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_covers_noncasters_and_reports_unknown_classes() {
    let root = root("mig-misc");
    let save = save_at(&root, "2030-05-05-01");
    write_sheet(&save, "brute", "barbarian", "2014", "## 法术\n\n- 无\n");
    write_sheet(&save, "odd", "sword-sage", "2014", "");
    let report = migrate::ensure(&save, &root, false).unwrap();
    assert_eq!(report.spell_sheets.len(), 2, "{report:?}");
    // Non-caster: an empty block, no fabricated lists.
    let lists = spells_of(&save, "brute");
    assert_eq!(lists.version, 1);
    assert!(lists.mode.is_empty() && lists.ability.is_empty());
    assert!(lists.prepared.is_empty() && lists.cantrips.is_empty());
    // Unknown class: warning, empty block, no adoption.
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("sword-sage") || w.contains("odd")),
        "{report:?}"
    );
    let _ = fs::remove_dir_all(&root);
}
// ---------------------------------------------------------------------------
// AC-3: chargen structured lists with legacy fallback
// ---------------------------------------------------------------------------

fn base_creation() -> Creation {
    Creation {
        ruleset: Some("2014".into()),
        class: Some("cleric".into()),
        level: 1,
        // wis 14 → 2014 prepared = wis + level = 3.
        scores: BTreeMap::from([("wis".into(), 14)]),
        spells_done: true,
        ..Default::default()
    }
}

#[test]
fn chargen_validate_prefers_structured_lists() {
    let mut c = base_creation();
    // 2024 cleric table at level 1 allows 3 cantrips / 4 prepared; the 2014
    // formula allows wis+level prepared and 3 cantrips.
    c.spell_lists = Some(SpellLists {
        version: 1,
        mode: "prepared".into(),
        ability: "wis".into(),
        cantrips: vec![
            "guidance".into(),
            "sacred-flame".into(),
            "resistance".into(),
            "thaumaturgy".into(),
        ],
        prepared: vec!["bless".into(), "cure-wounds".into()],
        ..Default::default()
    });
    let r = chargen::validate(&c);
    assert!(
        r.errors.iter().any(|e| e.contains("too many cantrips: 4")),
        "{:?}",
        r.errors
    );
    // wis 14 + level 1 allows 3 prepared; 2 chosen → a count warning.
    assert!(
        !r.errors.iter().any(|e| e.contains("可准备法术")),
        "{:?}",
        r.errors
    );
    assert!(
        r.warnings.iter().any(|w| w.contains("可准备法术: 2 of")),
        "{:?}",
        r.warnings
    );

    // Over the prepared limit is an error; always_prepared never counts.
    let mut c = base_creation();
    c.level = 1;
    c.spell_lists = Some(SpellLists {
        version: 1,
        mode: "prepared".into(),
        ability: "wis".into(),
        cantrips: vec![
            "guidance".into(),
            "sacred-flame".into(),
            "resistance".into(),
        ],
        prepared: vec![
            "bless".into(),
            "cure-wounds".into(),
            "shield-of-faith".into(),
        ],
        always_prepared: vec!["bless".into()],
        ..Default::default()
    });
    let r = chargen::validate(&c);
    assert!(
        !r.errors.iter().any(|e| e.contains("too many")),
        "{:?}",
        r.errors
    );
    let _ = c;
}

#[test]
fn chargen_validate_still_accepts_legacy_free_text() {
    let mut c = base_creation();
    c.spell_lists = None;
    c.spells = vec![
        "guidance".into(),
        "sacred-flame".into(),
        "resistance".into(),
        "thaumaturgy".into(),
        "bless".into(),
        "cure-wounds".into(),
        "shield-of-faith".into(),
    ];
    let r = chargen::validate(&c);
    // The legacy heuristic counts 3 cantrips (guidance, sacred-flame,
    // resistance); thaumaturgy is missing from the static list — exactly the
    // gap the structured lists (spell.level == 0) close. So leveled = 4 > 3.
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("too many 可准备法术: 4")),
        "legacy free text still counted: {:?}",
        r.errors
    );
}

// ---------------------------------------------------------------------------
// AC-4/AC-7: class data — tables, ritual, grants, arcanum, pact
// ---------------------------------------------------------------------------

#[test]
fn class_data_2014_has_tables_ritual_and_grants() {
    let book = Book::srd("2014");
    let bard = book.class("bard").unwrap();
    assert_eq!(classdata::spell_mode(bard).as_deref(), Some("known"));
    assert_eq!(bard.ritual.as_deref(), Some("known"));
    assert_eq!(bard.spells.get("vicious-mockery"), Some(&0));
    assert_eq!(bard.spells.get("charm-person"), Some(&1));
    assert!(
        !bard.spells.contains_key("magic-missile"),
        "not a 2014 bard spell"
    );
    // Magical Secrets (class feature): 10/14/18, any class, counts.
    assert_eq!(bard.features_grants.len(), 3);
    assert!(bard.features_grants.iter().all(|g| g.grant == "any_class"));
    assert_eq!(bard.features_grants[0].from_level, 10);
    assert_eq!(bard.features_grants[0].count, Some(2));
    assert!(bard.features_grants[0].counts);

    let cleric = book.class("cleric").unwrap();
    assert_eq!(cleric.ritual.as_deref(), Some("prepared"));
    let druid = book.class("druid").unwrap();
    assert_eq!(druid.ritual.as_deref(), Some("prepared"));
    let paladin = book.class("paladin").unwrap();
    assert_eq!(paladin.ritual.as_deref(), Some("none"));
    let ranger = book.class("ranger").unwrap();
    assert_eq!(ranger.ritual.as_deref(), Some("none"));
    let sorcerer = book.class("sorcerer").unwrap();
    assert_eq!(sorcerer.ritual.as_deref(), Some("none"));
    let wizard = book.class("wizard").unwrap();
    assert_eq!(wizard.ritual.as_deref(), Some("book"));
    assert!(wizard.book);
    assert_eq!(classdata::spell_mode(wizard).as_deref(), Some("spellbook"));
    let scribe = wizard.scribe.expect("wizard scribe numbers");
    assert_eq!((scribe.hours_per_level, scribe.cost_gp_per_level), (2, 50));

    let warlock = book.class("warlock").unwrap();
    assert_eq!(warlock.ritual.as_deref(), Some("none"));
    assert!(warlock.arcanum);

    // Subclass grants follow the SRD text, never flattened.
    let life = &cleric.subclasses["life"];
    assert_eq!(life.spells.len(), 9);
    assert!(life.spells.iter().all(|g| g.grant == "always_prepared"));
    let bless = life
        .spells
        .iter()
        .find(|g| g.id.as_deref() == Some("bless"))
        .unwrap();
    assert_eq!((bless.level, bless.from_level), (Some(1), 1));
    let devotion = &book.class("paladin").unwrap().subclasses["devotion"];
    assert_eq!(devotion.spells.len(), 10);
    assert!(devotion.spells.iter().all(|g| g.grant == "always_prepared"));
    let fiend = &warlock.subclasses["fiend"];
    assert_eq!(fiend.spells.len(), 10);
    assert!(
        fiend.spells.iter().all(|g| g.grant == "class_list"),
        "Fiend is an expanded list, not always-prepared"
    );
    let land = &druid.subclasses["land"];
    assert_eq!(land.spells.len(), 56);
    assert!(
        land.spells.iter().all(|g| g.terrain.is_some()),
        "{:?}",
        land.spells.first()
    );
    let lore = &bard.subclasses["lore"];
    assert_eq!(lore.spells.len(), 1);
    let ams = &lore.spells[0];
    assert_eq!(ams.grant, "any_class");
    assert_eq!((ams.from_level, ams.count), (6, Some(2)));
    assert!(!ams.counts, "Lore picks do not count toward known");

    // The universe for adopt/validation: base + fixed grants + any_class
    // (all class tables in the book).
    let universe = classdata::spell_universe(&book, bard);
    assert_eq!(universe.get("magic-missile"), Some(&1));
    assert!(
        universe.contains_key("spirit-guardians"),
        "any_class opens every table (cleric-only spell)"
    );
    let universe = classdata::spell_universe(&book, warlock);
    assert!(
        universe.contains_key("fireball"),
        "Fiend class_list grants fireball"
    );
}

#[test]
fn class_data_2024_has_edition_ritual_and_handwritten_grants() {
    let book = Book::srd("2024");
    for (id, cd) in &book.classes {
        let _ = id;
        // 2024 carries no per-class ritual field (edition-level rule).
        assert!(cd.ritual.is_none(), "{id}: {:?}", cd.ritual);
    }
    let wizard = book.class("wizard").unwrap();
    assert!(wizard.ritual_adept, "2024 wizard Ritual Adept");
    assert!(wizard.book);
    assert_eq!(wizard.cantrip_replace.as_deref(), Some("long_rest"));
    let bard = book.class("bard").unwrap();
    assert_eq!(bard.cantrip_replace.as_deref(), Some("level_up"));
    assert_eq!(classdata::spell_mode(bard).as_deref(), Some("prepared"));
    // 2024 Magical Secrets: multi_list over bard/cleric/druid/wizard.
    assert_eq!(bard.features_grants.len(), 1);
    let ms = &bard.features_grants[0];
    assert_eq!(ms.grant, "multi_list");
    assert_eq!(ms.from, vec!["bard", "cleric", "druid", "wizard"]);
    assert_eq!(ms.from_level, 10);
    assert!(ms.id.is_none(), "choice grant has no fixed id");

    // Handwritten 2024 subclass grants (SRD 5.2.1 text).
    let life = &book.class("cleric").unwrap().subclasses["life"];
    assert_eq!(life.spells.len(), 10);
    assert!(life.spells.iter().all(|g| g.grant == "always_prepared"));
    let fiend = &book.class("warlock").unwrap().subclasses["fiend"];
    assert_eq!(fiend.spells.len(), 10);
    assert!(
        fiend.spells.iter().all(|g| g.grant == "always_prepared"),
        "2024 Fiend Spells are always prepared (SRD text)"
    );
    let land = &book.class("druid").unwrap().subclasses["land"];
    assert_eq!(land.spells.len(), 21);
    let terrains: std::collections::BTreeSet<_> = land
        .spells
        .iter()
        .filter_map(|g| g.terrain.as_deref())
        .collect();
    assert_eq!(terrains, ["arid", "polar", "temperate", "tropical"].into());
    let draconic = &book.class("sorcerer").unwrap().subclasses["draconic"];
    assert_eq!(draconic.spells.len(), 10);
    assert!(draconic.spells.iter().all(|g| g.grant == "always_prepared"));
    let lore = &bard.subclasses["lore"];
    assert_eq!(lore.spells.len(), 1);
    assert_eq!(lore.spells[0].grant, "multi_list");
    assert_eq!(lore.spells[0].from, vec!["cleric", "druid", "wizard"]);

    // The universe honours multi_list sources.
    let universe = classdata::spell_universe(&book, bard);
    assert!(
        universe.contains_key("mass-healing-word"),
        "cleric table reachable through bard Magical Secrets"
    );
}

#[test]
fn pact_slots_and_arcanum_resources_generate_by_level() {
    // pact(lvl): (count, slot level).
    assert_eq!(classdata::pact(1), (1, 1));
    assert_eq!(classdata::pact(5), (2, 3));
    assert_eq!(classdata::pact(11), (3, 5));
    assert_eq!(classdata::pact(17), (4, 5));

    let book = Book::srd("2014");
    let make = |lvl| Creation {
        ruleset: Some("2014".into()),
        class: Some("warlock".into()),
        level: lvl,
        ..Default::default()
    };
    // Level 1: one pact slot, no arcanum.
    let g = classdata::generate(&make(1), &book, None);
    assert_eq!(g.pact, Some((1, 1)));
    assert!(!g.resources.iter().any(|r| r.id.starts_with("arcanum-")));
    // Level 5: two 3rd-level pact slots.
    let g = classdata::generate(&make(5), &book, None);
    assert_eq!(g.pact, Some((2, 3)));
    // Level 10: still no arcanum; level 11 unlocks ring 6.
    let g = classdata::generate(&make(10), &book, None);
    assert!(!g.resources.iter().any(|r| r.id.starts_with("arcanum-")));
    let g = classdata::generate(&make(11), &book, None);
    let ids: Vec<_> = g.resources.iter().map(|r| r.id.clone()).collect();
    assert!(ids.contains(&"arcanum-6".to_string()), "{ids:?}");
    assert!(!ids.contains(&"arcanum-7".to_string()), "{ids:?}");
    let a6 = g.resources.iter().find(|r| r.id == "arcanum-6").unwrap();
    assert_eq!((a6.max, a6.recharge.as_str()), (1, "long"));
    // 13/15/17 unlock 7/8/9.
    let g = classdata::generate(&make(17), &book, None);
    let ids: Vec<_> = g.resources.iter().map(|r| r.id.clone()).collect();
    for ring in 6..=9 {
        assert!(ids.contains(&format!("arcanum-{ring}")), "{ids:?}");
    }
    // Non-warlocks never get arcanum resources.
    let g = classdata::generate(
        &Creation {
            ruleset: Some("2014".into()),
            class: Some("wizard".into()),
            level: 17,
            ..Default::default()
        },
        &book,
        None,
    );
    assert!(!g.resources.iter().any(|r| r.id.starts_with("arcanum-")));
    assert!(g.pact.is_none());
}

#[test]
fn migration_backup_restores_a_schema_9_save_that_migrates_again() {
    let root = root("mig-restore");
    let save = save_at(&root, "2030-06-06-01");
    write_sheet(&save, "hero", "cleric", "2014", "");
    write_chargen(
        &save,
        r#"{"ruleset":"2014","class":"cleric","level":1,"spells_done":true,
            "spells":["bless"]}"#,
    );
    migrate::ensure(&save, &root, false).unwrap();
    assert_eq!(migrate::load(&save).unwrap().unwrap().schema, 10);

    // Roll back: replace the save with its backup (the documented recovery).
    let backups: Vec<PathBuf> = fs::read_dir(root.join("Saves/_backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = backups[0].clone();
    fs::remove_dir_all(&save).unwrap();
    copy_tree(&backup, &save);
    assert_eq!(migrate::load(&save).unwrap().unwrap().schema, 9);
    assert!(
        !fs::read_to_string(charsheet::path(&save, "hero"))
            .unwrap()
            .contains("spells:"),
        "restored sheet is the pre-migration state"
    );

    // The restored save migrates cleanly again (deterministic, no leftovers).
    let report = migrate::ensure(&save, &root, false).unwrap();
    assert_eq!(report.spell_sheets, vec!["hero".to_string()]);
    let lists = spells_of(&save, "hero");
    assert_eq!(lists.prepared, vec!["bless".to_string()]);
    assert_eq!(migrate::load(&save).unwrap().unwrap().schema, 10);
    let _ = fs::remove_dir_all(&root);
}

/// Small recursive copy for the restore step (test-only).
fn copy_tree(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}
