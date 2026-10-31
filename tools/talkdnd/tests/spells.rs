//! v1.6 T2 (TASK-0123): structured spell lists, the schema 9→10 migration
//! and the class spell data behind them. All fixtures are synthetic; no
//! real save is ever read (docs/SPELLCASTING.md §3.5/§3.6, Issue #242).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use combat_sim::dice::DiceRng;
use talkdnd::chargen::{self, Creation};
use talkdnd::charsheet::{self, CharFile, Op, SpellLists};
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
    // Manifest baseline: the migration only rewrites sheets clean against
    // it (AC-2 fail-safe).
    combat_sim::integrity::ensure(&save).unwrap();

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
    // Manifest baseline: the migration only rewrites sheets clean against
    // it (AC-2 fail-safe).
    combat_sim::integrity::ensure(&save).unwrap();
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
    // Manifest baseline: the migration only rewrites sheets clean against
    // it (AC-2 fail-safe).
    combat_sim::integrity::ensure(&save).unwrap();
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

/// AC-2/AC-6 (fail-safe): with no integrity manifest at all the migration
/// cannot tell hand edits from tool writes, so every sheet is treated as
/// dirty — reported, never serde-rewritten, bytes stay identical.
#[test]
fn migration_without_integrity_manifest_keeps_every_sheet_byte_identical() {
    let root = root("mig-nobaseline");
    let save = save_at(&root, "2030-07-07-01");
    write_sheet(&save, "hero", "cleric", "2014", "## 法术\n\n- Bless\n");
    write_chargen(
        &save,
        r#"{"ruleset":"2014","class":"cleric","level":1,"spells_done":true,
            "spells":["bless"]}"#,
    );
    // A hand edit no manifest knows about: a comment a serde round-trip
    // would drop. Deliberately NO combat_sim::integrity::ensure(&save).
    let hero_path = charsheet::path(&save, "hero");
    let hero_text = fs::read_to_string(&hero_path).unwrap();
    fs::write(&hero_path, format!("{hero_text}\n# DM note\n")).unwrap();

    // Dry-run already reports the sheet as dirty, with the warning.
    let dry = migrate::ensure(&save, &root, true).unwrap();
    assert_eq!(dry.spell_dirty, vec!["hero".to_string()], "{dry:?}");
    assert!(dry.spell_sheets.is_empty(), "{dry:?}");
    assert!(
        dry.warnings.iter().any(|w| w.contains("integrity 基线")),
        "{dry:?}"
    );

    // The real migration must not rewrite the sheet either.
    let report = migrate::ensure(&save, &root, false).unwrap();
    assert_eq!(report.spell_dirty, vec!["hero".to_string()], "{report:?}");
    assert_eq!(report.spell_sheets, Vec::<String>::new(), "{report:?}");
    assert_eq!(
        fs::read_to_string(&hero_path).unwrap(),
        format!("{hero_text}\n# DM note\n"),
        "sheet must stay byte-identical without a baseline"
    );
    // The rest of the migration still ran; the save loads at schema 10.
    assert_eq!(migrate::load(&save).unwrap().unwrap().schema, 10);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn migration_covers_noncasters_and_reports_unknown_classes() {
    let root = root("mig-misc");
    let save = save_at(&root, "2030-05-05-01");
    write_sheet(&save, "brute", "barbarian", "2014", "## 法术\n\n- 无\n");
    write_sheet(&save, "odd", "sword-sage", "2014", "");
    // Manifest baseline: the migration only rewrites sheets clean against
    // it (AC-2 fail-safe).
    combat_sim::integrity::ensure(&save).unwrap();
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

/// AC-3 (design §3.3): spellbook mode counts `prepared` against the
/// prepare limit — the `book` may hold far more than the limit allows.
#[test]
fn chargen_validate_spellbook_counts_prepared_not_book() {
    let mut c = base_creation();
    c.class = Some("wizard".into());
    c.scores = BTreeMap::from([("int".into(), 14)]);
    // int 14 (+2) + level 1 allows 3 prepared (2014 formula).
    c.spell_lists = Some(SpellLists {
        version: 1,
        mode: "spellbook".into(),
        ability: "int".into(),
        cantrips: vec!["fire-bolt".into(), "mage-hand".into(), "light".into()],
        book: vec![
            "magic-missile".into(),
            "shield".into(),
            "mage-armor".into(),
            "detect-magic".into(),
            "feather-fall".into(),
        ],
        prepared: vec!["magic-missile".into(), "shield".into(), "mage-armor".into()],
        ..Default::default()
    });
    let r = chargen::validate(&c);
    assert!(
        !r.errors.iter().any(|e| e.contains("too many")),
        "a full book with a legal prepared list must not error: {:?}",
        r.errors
    );
    assert!(
        !r.warnings.iter().any(|w| w.contains("可准备法术")),
        "prepared at the limit must not warn: {:?}",
        r.warnings
    );

    // One prepared spell over the limit is an error.
    c.spell_lists
        .as_mut()
        .unwrap()
        .prepared
        .push("detect-magic".into());
    let r = chargen::validate(&c);
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("too many 可准备法术: 4 (allowed 3)")),
        "{:?}",
        r.errors
    );
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

    // The universe for adopt/validation: base + class-level grants +
    // any_class (all class tables in the book).
    let universe = classdata::spell_universe(&book, bard, None, 20, None);
    assert_eq!(universe.get("magic-missile"), Some(&1));
    assert!(
        universe.contains_key("spirit-guardians"),
        "any_class opens every table (cleric-only spell)"
    );
    let universe = classdata::spell_universe(&book, warlock, Some("fiend"), 20, None);
    assert!(
        universe.contains_key("fireball"),
        "Fiend class_list grants fireball"
    );
    assert!(
        !classdata::spell_universe(&book, warlock, None, 20, None).contains_key("fireball"),
        "a warlock without the Fiend subclass has no expanded list"
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

    // The universe honours multi_list sources (class-level Magical Secrets,
    // valid without a subclass).
    let universe = classdata::spell_universe(&book, bard, None, 20, None);
    assert!(
        universe.contains_key("mass-healing-word"),
        "cleric table reachable through bard Magical Secrets"
    );
}

// ---------------------------------------------------------------------------
// R1 修正（PR #245 退回 1）：法术来源只含角色实际子职的授予
//（docs/SPELLCASTING.md §3.9）。
// ---------------------------------------------------------------------------

#[test]
fn spell_universe_is_subclass_aware() {
    let book = Book::srd("2014");
    // 非 Fiend Warlock：Fiend 扩展表（class_list）不进入来源。
    let warlock = book.class("warlock").unwrap();
    let no_fiend = classdata::spell_universe(&book, warlock, None, 20, None);
    assert!(
        !no_fiend.contains_key("fireball"),
        "unselected Fiend expanded list leaked: fireball present"
    );
    assert!(
        classdata::spell_universe(&book, warlock, Some("fiend"), 20, None).contains_key("fireball"),
        "a Fiend warlock keeps the expanded list"
    );
    // 非 Land Druid：Land 地形法术（always_prepared）不进入来源；选中
    // Land 未选地形时同样不进入；选定地形后只拿该地形（R3）。
    let druid = book.class("druid").unwrap();
    let land = &druid.subclasses["land"];
    let terrain_of = |id: &str| {
        land.spells
            .iter()
            .find(|g| g.id.as_deref() == Some(id))
            .and_then(|g| g.terrain.clone())
            .unwrap_or_default()
    };
    for id in ["slow", "cone-of-cold", "mirror-image"] {
        let t = terrain_of(id);
        assert!(
            !classdata::spell_universe(&book, druid, None, 20, None).contains_key(id),
            "unselected Land grant leaked: {id} present"
        );
        assert!(
            !classdata::spell_universe(&book, druid, Some("land"), 20, None).contains_key(id),
            "no terrain chosen yet: {id} must stay out of the universe"
        );
        assert!(
            classdata::spell_universe(&book, druid, Some("land"), 20, Some(&t)).contains_key(id),
            "the chosen terrain keeps its grant: {id} ({t})"
        );
    }
    // 其他地形仍被拒（slow 是 arctic 授予）。
    assert!(
        !classdata::spell_universe(&book, druid, Some("land"), 20, Some("forest"))
            .contains_key("slow"),
        "another terrain's grant leaked: slow present under forest"
    );
    // 未选 Lore：class 级 Magical Secrets（features_grants）仍全部有效。
    let bard = book.class("bard").unwrap();
    let no_lore = classdata::spell_universe(&book, bard, None, 20, None);
    assert!(
        no_lore.contains_key("spirit-guardians"),
        "class-level Magical Secrets must survive the subclass filter"
    );
    // 2014 Lore 的第 6 级 any_class 与 class 级 Magical Secrets 都开全部
    // 职业表，集合上重合；区分性由 warlock/druid 用例承担，
    // 这里断言选中 lore 不会减少来源。
    let with_lore = classdata::spell_universe(&book, bard, Some("lore"), 20, None);
    assert_eq!(no_lore, with_lore);
    // 2024：Land 的 fire-bolt 戏法只经子职授予进入；class 级
    // multi_list（bard/cleric/druid/wizard）不受子职过滤影响。
    let book24 = Book::srd("2024");
    let bard24 = book24.class("bard").unwrap();
    assert!(
        classdata::spell_universe(&book24, bard24, None, 20, None)
            .contains_key("mass-healing-word"),
        "2024 class-level Magical Secrets survive the filter"
    );
    let druid24 = book24.class("druid").unwrap();
    assert!(
        !classdata::spell_universe(&book24, druid24, None, 20, None).contains_key("fire-bolt"),
        "unselected 2024 Land grant leaked: fire-bolt present"
    );
    assert!(
        classdata::spell_universe(&book24, druid24, Some("land"), 20, Some("arid"))
            .contains_key("fire-bolt"),
        "a 2024 Land druid keeps fire-bolt (arid terrain)"
    );
}

// ---------------------------------------------------------------------------
// R2 修正（PR #245 退回 2）：授予按 from_level 进入来源（docs/SPELLCASTING.md
// §3.9）——低等级不得提前获得高等级特性授予的法术。
// ---------------------------------------------------------------------------

#[test]
fn spell_universe_filters_grants_by_level() {
    // 1 级 Bard：Magical Secrets（10 级 any_class）未到，cleric 的 bless
    // 不在来源；10 级起经 Magical Secrets 进入。
    let book = Book::srd("2014");
    let bard = book.class("bard").unwrap();
    assert!(
        !classdata::spell_universe(&book, bard, None, 1, None).contains_key("bless"),
        "a level-1 bard got a level-10 Magical Secrets spell early"
    );
    assert!(
        classdata::spell_universe(&book, bard, None, 10, None).contains_key("bless"),
        "a level-10 bard reaches bless through Magical Secrets"
    );

    // 2024 Land Druid：子职授予（fire-bolt 戏法）3 级才到位。
    let book24 = Book::srd("2024");
    let druid24 = book24.class("druid").unwrap();
    assert!(
        !classdata::spell_universe(&book24, druid24, Some("land"), 1, None)
            .contains_key("fire-bolt"),
        "a level-1 Land druid got the level-3 subclass cantrip early"
    );
    assert!(
        classdata::spell_universe(&book24, druid24, Some("land"), 3, Some("arid"))
            .contains_key("fire-bolt"),
        "a level-3 Land druid has fire-bolt (arid terrain)"
    );

    // 2014 Fiend Warlock：1 级即有 1 环扩表；3/5 环扩表等对应等级。
    let warlock = book.class("warlock").unwrap();
    let l1 = classdata::spell_universe(&book, warlock, Some("fiend"), 1, None);
    assert!(
        l1.contains_key("burning-hands") && l1.contains_key("command"),
        "a level-1 Fiend keeps the ring-1 expanded list"
    );
    assert!(
        !l1.contains_key("fireball"),
        "the ring-3 Fiend grant must wait for level 5"
    );
    assert!(
        classdata::spell_universe(&book, warlock, Some("fiend"), 5, None).contains_key("fireball"),
        "a level-5 Fiend warlock gains fireball"
    );

    // 基础职业表不按等级过滤：1 级术士的 3 环表内法术仍在来源里，
    // 高环由 max_castable 继续把关（见下一个测试）。
    assert!(
        classdata::spell_universe(&book, warlock, None, 1, None).contains_key("counterspell"),
        "the base table is not level-filtered"
    );
}

#[test]
fn grant_levels_gate_sources_but_rings_stay_with_max_castable() {
    let root = t3_root("grant-level");
    let save = t3_save(&root, "s");
    // 1 级 Fiend Warlock（known 模式）：pact slot 1 环。
    write_caster(
        &save,
        "war",
        "warlock",
        Some("fiend"),
        "2014",
        1,
        &[("cha", 16)],
        empty_lists("known", "cha"),
    );
    // 授予等级：2 环扩表 blindness-deafness 3 级才授予 → 来源拒绝。
    let err = run_op(
        &save,
        "war",
        Op::Learn {
            spell: "blindness-deafness".into(),
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("不在 warlock 的法术来源内"),
        "{err}"
    );
    // 环级：基础表 3 环 counterspell 在来源里，但 1 级 pact slot 只有 1 环。
    let err = run_op(
        &save,
        "war",
        Op::Learn {
            spell: "counterspell".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("高于当前可施法环级"), "{err}");
    let _ = fs::remove_dir_all(&root);
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
    // Manifest baseline: the migration only rewrites sheets clean against
    // it (AC-2 fail-safe).
    combat_sim::integrity::ensure(&save).unwrap();
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
// ---------------------------------------------------------------------------
// v1.6 T3 (TASK-0124): the spell list commands, refs and creation wiring
// (docs/SPELLCASTING.md §3.4/§3.9/§4/§5.3, Issue #244). All fixtures are
// synthetic; no real save is ever read.
// ---------------------------------------------------------------------------

struct T3Rng(u32);
impl DiceRng for T3Rng {
    fn roll_die(&mut self, _sides: u32) -> u32 {
        self.0
    }
}

fn t3rng() -> T3Rng {
    T3Rng(10)
}

fn t3_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("talkdnd-t3-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Saves")).unwrap();
    dir
}
fn t3_save(root: &Path, tag: &str) -> PathBuf {
    let save = root.join("Saves").join(tag);
    for dir in ["Sessions", "sheets", "combat"] {
        fs::create_dir_all(save.join(dir)).unwrap();
    }
    fs::write(save.join("save-meta.yaml"), "schema: 10\ntool: 1.5.0\n").unwrap();
    fs::write(save.join("Sessions/log.md"), "# log\n").unwrap();
    save
}

fn scores_line(scores: &[(&str, i32)]) -> String {
    let all: Vec<String> = ["str", "dex", "con", "int", "wis", "cha"]
        .iter()
        .map(|a| {
            let v = scores
                .iter()
                .find(|(k, _)| k == a)
                .map(|(_, v)| *v)
                .unwrap_or(10);
            format!("{a}: {v}")
        })
        .collect();
    format!("abilities: {{{}}}\n", all.join(", "))
}

/// A structured caster sheet with a real-ish combat block (abilities and a
/// spellcasting entry drive the class-derived limits).
#[allow(clippy::too_many_arguments)]
fn write_caster(
    save: &Path,
    id: &str,
    class: &str,
    subclass: Option<&str>,
    ruleset: &str,
    level: i32,
    scores: &[(&str, i32)],
    lists: SpellLists,
) -> CharFile {
    let mut info = BTreeMap::from([("class".to_string(), class.to_string())]);
    if let Some(sub) = subclass {
        info.insert("subclass".to_string(), sub.to_string());
    }
    let header = match subclass {
        Some(sub) => format!("- **等级**：{class}（{sub}） {level}\n"),
        None => format!("- **等级**：{class} {level}\n"),
    };
    let cf = CharFile {
        version: 1,
        id: id.into(),
        name: id.into(),
        file: format!("Characters/{id}.md"),
        controller: Some(charsheet::PLAYER.into()),
        ruleset: ruleset.into(),
        info,
        body_md: format!("# {id}\n\n{header}"),
        combat: serde_norway::from_str(&format!(
            "{{id: {id}, name: {id}, kind: pc, side: party, level: {level}, ac: 10, hp: 8, speed: {{walk: 30}}, prof: 2, {scores}, resources: {{\"spell-slot-1\": {{max: 2, recharge: long}}}}}}",
            scores = scores_line(scores)
        ))
        .unwrap(),
        spells: Some(lists),
        ..Default::default()
    };
    let dir = save.join("combat");
    fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("campaign.yaml");
    if !manifest.is_file() {
        let lib = save
            .parent()
            .and_then(|r| r.parent())
            .map(|r| r.join("content"))
            .unwrap_or_else(|| save.join("../content"));
        fs::write(
            &manifest,
            format!(
                "name: t3\nlibrary: [{}]\nruleset: \"{}\"\n",
                lib.display(),
                ruleset
            ),
        )
        .unwrap();
    }
    write_sheet_files(save, &cf);
    cf
}

/// The on-disk pair check::sheets and charsheet::load read: the Markdown
/// sheet (body first, then the combat block) plus the stored yaml.
fn write_sheet_files(save: &Path, cf: &CharFile) {
    charsheet::store(save, cf).unwrap();
    let chars = save.join("Characters");
    fs::create_dir_all(&chars).unwrap();
    let md = format!(
        "{}\n\n## 战斗数据块（combat-sim）\n\n```combat yaml\n{}\n```\n",
        cf.body_md,
        charsheet::emit_block(&cf.combat)
    );
    fs::write(chars.join(format!("{}.md", cf.id)), md).unwrap();
}

fn empty_lists(mode: &str, ability: &str) -> SpellLists {
    SpellLists {
        version: 1,
        mode: mode.into(),
        ability: ability.into(),
        ..Default::default()
    }
}

fn lists_of(save: &Path, id: &str) -> SpellLists {
    let text = fs::read_to_string(charsheet::path(save, id)).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).expect("sheet parses");
    cf.spells.expect("spells block")
}

fn sheet_text(save: &Path, id: &str) -> String {
    fs::read_to_string(charsheet::path(save, id)).unwrap()
}

/// The (ref, cost) pairs of a stored sheet's combat block, in file order.
fn ref_entries(save: &Path, id: &str) -> Vec<(String, Option<(String, i64)>)> {
    let text = fs::read_to_string(charsheet::path(save, id)).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).unwrap();
    let mut out = Vec::new();
    for key in ["actions", "features"] {
        if let Some(seq) = cf.combat.get(key).and_then(|v| v.as_sequence()) {
            for e in seq {
                if let Some(id) = e.get("ref").and_then(|v| v.as_str()) {
                    let cost = e.get("cost").map(|c| {
                        (
                            c.get("resource")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            c.get("amount").and_then(|v| v.as_i64()).unwrap_or(1),
                        )
                    });
                    out.push((id.to_string(), cost));
                }
            }
        }
    }
    out
}

/// First leveled spell (lowest ring, then id) on the class's base table.
fn first_leveled(cd: &talkdnd::classdata::ClassDef, ring: u8) -> Option<String> {
    cd.spells
        .iter()
        .filter(|(_, lvl)| **lvl == ring)
        .map(|(id, _)| id.clone())
        .next()
}

fn run_op(save: &Path, who: &str, op: Op) -> Result<Vec<String>, anyhow::Error> {
    charsheet::run(save, who, op, &mut t3rng())
}
// ---------------------------------------------------------------------------
// AC-7: prepare/unprepare across all eight SRD casting classes (both editions)
// ---------------------------------------------------------------------------

#[test]
fn prepare_and_unprepare_cover_all_eight_casting_classes() {
    for rs in ["2014", "2024"] {
        let book = Book::srd(rs);
        for (class, mode, ability) in [
            ("bard", "known", "cha"),
            ("cleric", "prepared", "wis"),
            ("druid", "prepared", "wis"),
            ("paladin", "prepared", "cha"),
            ("ranger", "prepared", "wis"),
            ("sorcerer", "known", "cha"),
            ("warlock", "known", "cha"),
            ("wizard", "spellbook", "int"),
        ] {
            let tag = format!("eight-{rs}-{class}");
            let root = t3_root(&tag);
            let save = t3_save(&root, "s");
            // Half casters (2014) get their first slots at level 2.
            let level = if rs == "2014" && matches!(class, "paladin" | "ranger") {
                2
            } else {
                1
            };
            let cd = book.class(class).expect("class data");
            let spell = first_leveled(cd, 1).expect("class table has a 1st-ring spell");
            let mut lists = empty_lists(mode, ability);
            if mode == "spellbook" {
                lists.book.push(spell.clone());
            }
            write_caster(
                &save,
                "hero",
                class,
                None,
                rs,
                level,
                &[("wis", 14), ("cha", 14), ("int", 14)],
                lists,
            );
            let added = if mode == "known" {
                run_op(
                    &save,
                    "hero",
                    Op::Learn {
                        spell: spell.clone(),
                    },
                )
                .unwrap()
            } else {
                run_op(
                    &save,
                    "hero",
                    Op::Prepare {
                        spell: spell.clone(),
                    },
                )
                .unwrap()
            };
            let joined = added.join("；");
            assert!(
                joined.contains(&spell) && !joined.contains("error"),
                "{rs} {class}: {joined}"
            );
            let lists = lists_of(&save, "hero");
            let listed = match mode {
                "known" => &lists.known,
                "spellbook" => &lists.prepared,
                _ => &lists.prepared,
            };
            assert!(listed.contains(&spell), "{rs} {class}: {spell} not listed");

            if mode == "known" {
                // Known lists have no unlearn verb (docs/SPELLCASTING.md
                // §4.1): unprepare refuses with the mode message.
                let err = run_op(
                    &save,
                    "hero",
                    Op::Unprepare {
                        spell: spell.clone(),
                    },
                )
                .unwrap_err();
                assert!(
                    err.to_string().contains("没有准备列表"),
                    "{rs} {class}: {err}"
                );
            } else {
                let removed = run_op(
                    &save,
                    "hero",
                    Op::Unprepare {
                        spell: spell.clone(),
                    },
                )
                .unwrap();
                assert!(removed.join("；").contains(&spell), "{rs} {class}");
            }
            let lists = lists_of(&save, "hero");
            assert!(
                !lists.prepared.contains(&spell)
                    && (mode == "known" || !lists.known.contains(&spell)),
                "{rs} {class}: {spell} still listed"
            );
            let _ = fs::remove_dir_all(&root);
        }
    }
}
#[test]
fn source_and_book_checks_refuse_and_write_nothing() {
    let root = t3_root("source");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        Some("life"),
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    let before = sheet_text(&save, "hero");

    // Not on the class list (fire-bolt is a wizard cantrip).
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "fire-bolt".into(),
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("不在 cleric 的法术来源内"),
        "{err}"
    );
    // Unknown id.
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "no-such-spell".into(),
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("不在 cleric 的法术来源内"),
        "{err}"
    );
    // Cantrips are not prepared.
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "guidance".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("戏法"), "{err}");
    // A spell above the castable ring.
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "spiritual-weapon".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("高于当前可施法环级"), "{err}");
    // Duplicate prepare is an idempotent no-op (exit 0, no change).
    let out = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "bless".into(),
        },
    )
    .unwrap();
    assert!(out.join("；").contains("已准备"), "{}", out.join("；"));
    assert_eq!(
        before,
        sheet_text(&save, "hero"),
        "failed commands must not write"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn over_capacity_prepare_is_refused() {
    let root = t3_root("capacity");
    let save = t3_save(&root, "s");
    // wis 16 (+3) + level 1 = 4 prepared.
    let mut lists = empty_lists("prepared", "wis");
    for s in ["bless", "cure-wounds", "shield-of-faith", "sanctuary"] {
        lists.prepared.push(s.into());
    }
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    let before = sheet_text(&save, "hero");
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "guiding-bolt".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("超过可准备数量"), "{err}");
    assert_eq!(
        before,
        sheet_text(&save, "hero"),
        "over-capacity must not write"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn spellbook_prepare_needs_the_book_and_scribe_costs_gold() {
    let root = t3_root("scribe");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("spellbook", "int");
    lists.cantrips.push("fire-bolt".into());
    lists.book.push("magic-missile".into());
    write_caster(
        &save,
        "hero",
        "wizard",
        None,
        "2014",
        1,
        &[("int", 16)],
        lists,
    );

    // Not in the book.
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "shield".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("不在法术书"), "{err}");
    // Scribe without gold fails and writes nothing.
    let before = sheet_text(&save, "hero");
    let err = run_op(
        &save,
        "hero",
        Op::Scribe {
            spell: "shield".into(),
            hours: None,
            cost_gp: None,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("金币不足"), "{err}");
    assert_eq!(before, sheet_text(&save, "hero"));
    // With gold: book grows, purse shrinks, nothing is prepared.
    charsheet::run(
        &save,
        "hero",
        Op::Batch {
            ops: vec!["gold add --gp 200".into()],
        },
        &mut t3rng(),
    )
    .unwrap();
    let out = run_op(
        &save,
        "hero",
        Op::Scribe {
            spell: "shield".into(),
            hours: None,
            cost_gp: None,
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("法术书（book 2）"), "{joined}");
    assert!(joined.contains("50 gp"), "{joined}");
    assert!(joined.contains("未准备"), "{joined}");
    let lists = lists_of(&save, "hero");
    assert!(lists.book.contains(&"shield".to_string()));
    assert!(lists.prepared.is_empty());
    // Already in the book.
    let err = run_op(
        &save,
        "hero",
        Op::Scribe {
            spell: "shield".into(),
            hours: None,
            cost_gp: None,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("已在法术书"), "{err}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn always_prepared_refuses_unprepare_and_prepare() {
    let root = t3_root("always");
    let save = t3_save(&root, "s");
    // 2014 Life domain: cure-wounds is always prepared (subclass grant).
    let mut lists = empty_lists("prepared", "wis");
    lists.always_prepared.push("cure-wounds".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        Some("life"),
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    let before = sheet_text(&save, "hero");
    let err = run_op(
        &save,
        "hero",
        Op::Unprepare {
            spell: "cure-wounds".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("always_prepared"), "{err}");
    let err = run_op(
        &save,
        "hero",
        Op::Prepare {
            spell: "cure-wounds".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("always_prepared"), "{err}");
    assert_eq!(before, sheet_text(&save, "hero"));
    let _ = fs::remove_dir_all(&root);
}
#[test]
fn warlock_arcanum_routes_learn_and_limits_rings() {
    let root = t3_root("arcanum");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("known", "cha");
    lists.cantrips.push("eldritch-blast".into());
    write_caster(
        &save,
        "hero",
        "warlock",
        Some("fiend"),
        "2014",
        11,
        &[("cha", 16)],
        lists,
    );
    // 6th ring at level 11 → arcanum.
    let out = run_op(
        &save,
        "hero",
        Op::Learn {
            spell: "eyebite".into(),
        },
    )
    .unwrap();
    assert!(out.join("；").contains("arcanum-6"), "{}", out.join("；"));
    let lists = lists_of(&save, "hero");
    assert_eq!(lists.arcanum, vec!["eyebite".to_string()]);
    // Duplicate arcanum pick refuses.
    let err = run_op(
        &save,
        "hero",
        Op::Learn {
            spell: "eyebite".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("已在奥术列表"), "{err}");
    // One per ring.
    let err = run_op(
        &save,
        "hero",
        Op::Learn {
            spell: "circle-of-death".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("每环至多一个"), "{err}");
    let _ = fs::remove_dir_all(&root);

    // Level 5 warlock: arcanum locked; the known list takes leveled spells.
    let root = t3_root("arcanum-low");
    let save = t3_save(&root, "s");
    let lists = empty_lists("known", "cha");
    write_caster(
        &save,
        "hero",
        "warlock",
        Some("fiend"),
        "2014",
        5,
        &[("cha", 16)],
        lists,
    );
    let err = run_op(
        &save,
        "hero",
        Op::Learn {
            spell: "eyebite".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("11 级解锁"), "{err}");
    let out = run_op(
        &save,
        "hero",
        Op::Learn {
            spell: "hellish-rebuke".into(),
        },
    )
    .unwrap();
    assert!(out.join("；").contains("known"), "{}", out.join("；"));
    // Duplicate learn is idempotent.
    let out = run_op(
        &save,
        "hero",
        Op::Learn {
            spell: "hellish-rebuke".into(),
        },
    )
    .unwrap();
    assert!(out.join("；").contains("已学会"), "{}", out.join("；"));
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// AC-4: sync — managed set, cost overrides, unmanaged protection, dry-run
// ---------------------------------------------------------------------------

#[test]
fn sync_adds_managed_refs_keeps_unmanaged_and_dry_run_writes_nothing() {
    let root = t3_root("sync");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.cantrips.push("guidance".into());
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    // A hand-written ref (unmanaged) plus an inline entry the lists also name.
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let mut cf: CharFile = serde_norway::from_str(&text).unwrap();
    // Hand-written extras: an unmanaged ref and an inline definition.
    let entries = serde_norway::from_str::<serde_norway::Value>(
        "- {ref: identify}\n- {id: bless, name: Bless inline, activation: action, effects: [{note: inline}]}\n",
    )
    .unwrap();
    if let Some(actions) = cf
        .combat
        .get_mut("actions")
        .and_then(|v| v.as_sequence_mut())
    {
        actions.extend(entries.as_sequence().unwrap().iter().cloned());
    } else if let Some(map) = cf.combat.as_mapping_mut() {
        map.insert(serde_norway::Value::String("actions".into()), entries);
    }
    charsheet::store(&save, &cf).unwrap();
    let before = sheet_text(&save, "hero");

    // Dry-run: prints the plan, writes nothing.
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: true }),
        },
    )
    .unwrap();
    let joined = out.join("\n");
    assert!(joined.contains("[dry-run]"), "{joined}");
    assert!(joined.contains("未受管 ref，未修改：identify"), "{joined}");
    assert_eq!(before, sheet_text(&save, "hero"), "dry-run must not write");

    // Real sync: the unmanaged ref and the inline definition survive;
    // guidance is added; bless already has its ref.
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let joined = out.join("\n");
    assert!(joined.contains("未受管 ref，未修改：identify"), "{joined}");
    assert!(
        joined.contains("inline 法术定义，未修改：bless"),
        "{joined}"
    );
    let refs = ref_entries(&save, "hero");
    assert!(refs.iter().any(|(id, _)| id == "identify"), "{refs:?}");
    assert!(refs.iter().any(|(id, _)| id == "guidance"), "{refs:?}");
    // The inline definition covers bless; sync must not add a ref next to it.
    assert!(!refs.iter().any(|(id, _)| id == "bless"), "{refs:?}");
    // The inline definition survives next to the ref.
    let text = sheet_text(&save, "hero");
    assert!(text.contains("Bless inline"), "{text}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sync_removes_stale_union_refs_and_keeps_unmanaged() {
    let root = t3_root("sync-stale");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.cantrips.push("guidance".into());
    lists.prepared.push("cure-wounds".into());
    // Stale leftover: a known-mode entry under prepared mode. It sits inside
    // the list union (managed) but is not castable, so its ref is removed
    // instead of being kept as unmanaged (docs/SPELLCASTING.md §4.2).
    lists.known.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    // A hand-written ref outside the lists (unmanaged) next to the stale one.
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let mut cf: CharFile = serde_norway::from_str(&text).unwrap();
    let entries = serde_norway::from_str::<serde_norway::Value>(
        "- {ref: identify}
- {ref: bless}
",
    )
    .unwrap();
    if let Some(actions) = cf
        .combat
        .get_mut("actions")
        .and_then(|v| v.as_sequence_mut())
    {
        actions.extend(entries.as_sequence().unwrap().iter().cloned());
    } else if let Some(map) = cf.combat.as_mapping_mut() {
        map.insert(serde_norway::Value::String("actions".into()), entries);
    }
    charsheet::store(&save, &cf).unwrap();
    let before = sheet_text(&save, "hero");

    // Dry-run: logs the stale removal and the unmanaged ref, writes nothing.
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: true }),
        },
    )
    .unwrap();
    let joined = out.join(
        "
",
    );
    assert!(joined.contains("[dry-run]"), "{joined}");
    assert!(joined.contains("stale ref：bless"), "{joined}");
    assert!(joined.contains("未受管 ref，未修改：identify"), "{joined}");
    assert_eq!(before, sheet_text(&save, "hero"), "dry-run must not write");

    // Real sync: the stale bless ref is removed, identify survives, and the
    // castable entries (guidance, cure-wounds) gain their refs.
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let joined = out.join(
        "
",
    );
    assert!(
        joined.contains("移除受管但不可施放的 stale ref：bless"),
        "{joined}"
    );
    assert!(joined.contains("未受管 ref，未修改：identify"), "{joined}");
    let refs = ref_entries(&save, "hero");
    assert!(!refs.iter().any(|(id, _)| id == "bless"), "{refs:?}");
    assert!(refs.iter().any(|(id, _)| id == "identify"), "{refs:?}");
    assert!(refs.iter().any(|(id, _)| id == "guidance"), "{refs:?}");
    assert!(refs.iter().any(|(id, _)| id == "cure-wounds"), "{refs:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unselected_subclass_sources_are_refused_but_selected_work() {
    let root = t3_root("subclass-filter");
    let save = t3_save(&root, "s");
    // 非 Fiend Warlock 不能 learn fireball（Fiend class_list）；
    // 选中 Fiend 后可以。
    write_caster(
        &save,
        "wix",
        "warlock",
        None,
        "2014",
        5,
        &[("cha", 16)],
        empty_lists("known", "cha"),
    );
    let err = run_op(
        &save,
        "wix",
        Op::Learn {
            spell: "fireball".into(),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("不在 warlock 的法术来源内"), "{err}");
    assert!(
        !lists_of(&save, "wix")
            .known
            .contains(&"fireball".to_string()),
        "a refused learn must not write the sheet"
    );
    write_caster(
        &save,
        "wix",
        "warlock",
        Some("fiend"),
        "2014",
        5,
        &[("cha", 16)],
        empty_lists("known", "cha"),
    );
    let out = run_op(
        &save,
        "wix",
        Op::Learn {
            spell: "fireball".into(),
        },
    )
    .unwrap();
    let joined = out.join(" ");
    assert!(joined.contains("fireball"), "{joined}");
    assert!(lists_of(&save, "wix")
        .known
        .contains(&"fireball".to_string()));

    // 非 Land Druid 不能 prepare Land 地形法术；选中 Land 后可以。
    write_caster(
        &save,
        "thorn",
        "druid",
        None,
        "2014",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let err = run_op(
        &save,
        "thorn",
        Op::Prepare {
            spell: "slow".into(),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("不在 druid 的法术来源内"), "{err}");
    write_caster(
        &save,
        "thorn",
        "druid",
        Some("land"),
        "2014",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    // 选了 Land 未选地形：slow 被拒并提示先选地形（R3）。
    let err = run_op(
        &save,
        "thorn",
        Op::Prepare {
            spell: "slow".into(),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("先 `spells terrain"), "{err}");
    // 选定 arctic：arctic 授予自动播种（不占准备），slow 直接可用、
    // 不可 unprepare；7 级的 freedom-of-movement 不提前。
    let out = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "arctic".into(),
            }),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(
        joined.contains("arctic") && joined.contains("播种"),
        "{joined}"
    );
    let lists = lists_of(&save, "thorn");
    assert_eq!(lists.terrain.as_deref(), Some("arctic"));
    assert!(lists.always_prepared.contains(&"slow".to_string()));
    assert!(lists.always_prepared.contains(&"hold-person".to_string()));
    assert!(
        !lists
            .always_prepared
            .contains(&"freedom-of-movement".to_string()),
        "from_level 7 must not seed at level 5"
    );
    let err = run_op(
        &save,
        "thorn",
        Op::Unprepare {
            spell: "slow".into(),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("不可 unprepare"), "{err}");

    // 未选 Lore 的第 10 级 Bard 仍有 class 级 Magical Secrets
    // （features_grants 保留）：spirit-guardians 经它可达。
    write_caster(
        &save,
        "mel",
        "bard",
        None,
        "2014",
        10,
        &[("cha", 16)],
        empty_lists("known", "cha"),
    );
    let out = run_op(
        &save,
        "mel",
        Op::Learn {
            spell: "spirit-guardians".into(),
        },
    )
    .unwrap();
    let joined = out.join(" ");
    assert!(joined.contains("spirit-guardians"), "{joined}");
    let _ = fs::remove_dir_all(&root);
}
#[test]
fn sync_overrides_costs_for_pact_and_arcanum_only() {
    let root = t3_root("sync-cost");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("known", "cha");
    lists.cantrips.push("eldritch-blast".into());
    lists.known.push("hellish-rebuke".into());
    lists.arcanum.push("eyebite".into());
    write_caster(
        &save,
        "hero",
        "warlock",
        Some("fiend"),
        "2014",
        11,
        &[("cha", 16)],
        lists,
    );
    run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    // Pact slot level 5 at warlock 11; arcanum costs its own resource.
    let refs = ref_entries(&save, "hero");
    assert_eq!(
        refs.iter()
            .find(|(id, _)| id == "hellish-rebuke")
            .unwrap()
            .1,
        Some(("spell-slot-5".to_string(), 1))
    );
    assert_eq!(
        refs.iter().find(|(id, _)| id == "eyebite").unwrap().1,
        Some(("arcanum-6".to_string(), 1))
    );
    assert_eq!(
        refs.iter()
            .find(|(id, _)| id == "eldritch-blast")
            .unwrap()
            .1,
        None,
        "cantrips never cost a slot"
    );
    let _ = fs::remove_dir_all(&root);

    // A non-pact caster keeps the ability's own cost (no ref-level override).
    let root = t3_root("sync-cost2");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let refs = ref_entries(&save, "hero");
    assert_eq!(
        refs.iter().find(|(id, _)| id == "bless").unwrap().1,
        None,
        "non-pact casters keep the ability's own cost"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sync_dedupes_managed_refs_and_unprepare_drops_the_ref() {
    let root = t3_root("sync-dedupe");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    // Duplicate the bless ref by hand.
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let mut cf: CharFile = serde_norway::from_str(&text).unwrap();
    if let Some(actions) = cf
        .combat
        .get_mut("actions")
        .and_then(|v| v.as_sequence_mut())
    {
        actions.push(serde_norway::from_str::<serde_norway::Value>("{ref: bless}").unwrap());
    }
    charsheet::store(&save, &cf).unwrap();
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    assert!(out.join("；").contains("移除重复"), "{}", out.join("；"));
    assert_eq!(
        ref_entries(&save, "hero")
            .iter()
            .filter(|(id, _)| id == "bless")
            .count(),
        1
    );

    // Unprepare drops the ref in the same write (§4.2-3).
    run_op(
        &save,
        "hero",
        Op::Unprepare {
            spell: "bless".into(),
        },
    )
    .unwrap();
    assert!(!ref_entries(&save, "hero")
        .iter()
        .any(|(id, _)| id == "bless"));
    assert!(!lists_of(&save, "hero")
        .prepared
        .contains(&"bless".to_string()));
    let _ = fs::remove_dir_all(&root);
}

/// AC-7: after sync, the engine's ownership lookup resolves the prepared
/// spell and finds nothing for an unprepared one (the cast entry point).
#[test]
fn prepared_refs_resolve_and_unprepared_do_not() {
    let root = t3_root("cast");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.cantrips.push("guidance".into());
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).unwrap();

    // Minimal test content: the two castable spells plus one that stays
    // unprepared (sanctuary).
    let lib = root.join("content");
    fs::create_dir_all(&lib).unwrap();
    fs::write(
        lib.join("spells.yaml"),
        concat!(
            "abilities:\n",
            "  - id: bless\n",
            "    name: Bless\n",
            "    tags: [spell]\n",
            "    activation: action\n",
            "    cost: {resource: spell-slot-1, amount: 1}\n",
            "    slot_level: 1\n",
            "    target: {type: self}\n",
            "    effects: [{note: Bless}]\n",
            "  - id: guidance\n",
            "    name: Guidance\n",
            "    tags: [spell, cantrip]\n",
            "    activation: action\n",
            "    target: {type: self}\n",
            "    effects: [{note: Guidance}]\n",
            "  - id: sanctuary\n",
            "    name: Sanctuary\n",
            "    tags: [spell]\n",
            "    activation: action\n",
            "    cost: {resource: spell-slot-1, amount: 1}\n",
            "    slot_level: 1\n",
            "    target: {type: self}\n",
            "    effects: [{note: Sanctuary}]\n",
        ),
    )
    .unwrap();
    let registry = combat_sim::content::ContentSources {
        libraries: vec![lib],
        ..Default::default()
    }
    .load()
    .unwrap();

    // The sheet's combat block IS the creature; embed it into a combatant.
    let creature = charsheet::emit_block(&cf.combat);
    let indented = creature
        .lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    let combatant: combat_sim::state::CombatantState = serde_norway::from_str(&format!(
        "creature:\n{indented}\nname: hero\nside: party\nzone: arena\nmax_hp: 8\nhp: 8\n"
    ))
    .unwrap();
    let ok = combatant.find_owned_ability(&registry, "bless").unwrap();
    assert_eq!(ok.ability.id, "bless");
    let ok = combatant.find_owned_ability(&registry, "guidance").unwrap();
    assert_eq!(ok.ability.id, "guidance");
    let err = combatant
        .find_owned_ability(&registry, "sanctuary")
        .unwrap_err();
    assert!(err.to_string().contains("has no ability matching"), "{err}");
    let _ = fs::remove_dir_all(&root);
}
// ---------------------------------------------------------------------------
// AC-3: adopt — precise matching, --map, legacy
// ---------------------------------------------------------------------------

#[test]
fn adopt_maps_precisely_and_keeps_unmatched_legacy() {
    let root = t3_root("adopt");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.legacy = vec!["bless".into(), "治疗真言".into(), "神秘未知术".into()];
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    // "bless" is already an id → routes into prepared; the others stay legacy.
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Adopt { maps: vec![] }),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("采纳 1 个"), "{joined}");
    assert!(joined.contains("legacy 剩 2"), "{joined}");
    let lists = lists_of(&save, "hero");
    assert!(lists.prepared.contains(&"bless".to_string()));
    assert_eq!(
        lists.legacy,
        vec!["治疗真言".to_string(), "神秘未知术".to_string()]
    );

    // --map moves one legacy entry by name; fuzzy targets refuse.
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Adopt {
                maps: vec!["治疗真言=healing-word".into()],
            }),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("legacy 剩 1"), "{joined}");
    let lists = lists_of(&save, "hero");
    assert!(lists.prepared.contains(&"healing-word".to_string()));
    assert_eq!(lists.legacy, vec!["神秘未知术".to_string()]);

    // Mapping to a spell outside the class sources refuses (fail closed).
    let err = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Adopt {
                maps: vec!["神秘未知术=magic-missile".into()],
            }),
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("不在 cleric 的法术来源内"),
        "{err}"
    );
    // A legacy entry that does not exist refuses.
    let err = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Adopt {
                maps: vec!["不存在=healing-word".into()],
            }),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("legacy 里没有"), "{err}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn adopt_creates_the_block_from_the_body_and_keeps_body_intact() {
    let root = t3_root("adopt-body");
    let save = t3_save(&root, "s");
    let body =
        "# Hero\n\n- **等级**：cleric（life） 1\n\n## 法术\n\n- Bless\n- guidance\n- 完全未知术\n";
    let cf = CharFile {
        version: 1,
        id: "hero".into(),
        name: "hero".into(),
        file: "Characters/hero.md".into(),
        controller: Some(charsheet::PLAYER.into()),
        ruleset: "2014".into(),
        info: BTreeMap::from([("class".to_string(), "cleric".to_string())]),
        body_md: body.into(),
        combat: serde_norway::from_str(
            "{id: hero, name: hero, kind: pc, side: party, level: 1, ac: 10, hp: 8, speed: {walk: 30}, prof: 2, abilities: {wis: 16}}",
        )
        .unwrap(),
        ..Default::default()
    };
    write_sheet_files(&save, &cf);
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Adopt { maps: vec![] }),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("新建块：是"), "{joined}");
    let lists = lists_of(&save, "hero");
    assert!(lists.cantrips.contains(&"guidance".to_string()));
    assert!(lists.prepared.contains(&"bless".to_string()));
    assert_eq!(lists.legacy, vec!["完全未知术".to_string()]);
    assert_eq!(lists.ability, "wis");
    // The stored body keeps the 法术 section (the sheet is a rendering).
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).unwrap();
    assert!(cf.body_md.contains("## 法术"), "body must not be rewritten");
    let _ = fs::remove_dir_all(&root);
}
// ---------------------------------------------------------------------------
// AC-6: ritual — the two-edition model and the D-12 refusal
// ---------------------------------------------------------------------------

/// Test content with a ritual spell, a non-ritual spell and a cantrip.
fn ritual_content(root: &Path) -> PathBuf {
    let lib = root.join("content");
    fs::create_dir_all(&lib).unwrap();
    fs::write(
        lib.join("spells.yaml"),
        concat!(
            "abilities:\n",
            "  - id: detect-magic\n",
            "    name: Detect Magic\n",
            "    tags: [spell, ritual]\n",
            "    activation: action\n",
            "    target: {type: self}\n",
            "    effects: [{note: Detect Magic}]\n",
            "  - id: bless\n",
            "    name: Bless\n",
            "    tags: [spell]\n",
            "    activation: action\n",
            "    cost: {resource: spell-slot-1, amount: 1}\n",
            "    slot_level: 1\n",
            "    target: {type: self}\n",
            "    effects: [{note: Bless}]\n",
            "  - id: guidance\n",
            "    name: Guidance\n",
            "    tags: [spell, cantrip]\n",
            "    activation: action\n",
            "    target: {type: self}\n",
            "    effects: [{note: Guidance}]\n",
        ),
    )
    .unwrap();
    lib
}

fn campaign(save: &Path, root: &Path, ruleset: &str) {
    let lib = ritual_content(root);
    let lib = lib.to_string_lossy().to_string();
    let dir = save.join("combat");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("campaign.yaml"),
        format!("name: t\nlibrary: [{lib}]\nruleset: \"{ruleset}\"\n"),
    )
    .unwrap();
}

#[test]
fn ritual_follows_the_two_edition_models() {
    // 2014 cleric (ritual: prepared): the spell must be prepared.
    let root = t3_root("ritual-2014");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.prepared.push("detect-magic".into());
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    campaign(&save, &root, "2014");
    let out = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "detect-magic".into(),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("正常施法时间 + 10 分钟"), "{joined}");
    assert!(joined.contains("不消耗法术位"), "{joined}");
    // A non-ritual spell refuses even when prepared.
    let err = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "bless".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("没有仪式（Ritual）标签"), "{err}");
    // An unprepared spell refuses.
    let err = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "sanctuary".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("不在"), "{err}");
    let _ = fs::remove_dir_all(&root);

    // 2014 wizard (ritual: book): the book alone is enough.
    let root = t3_root("ritual-wizard");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("spellbook", "int");
    lists.book.push("detect-magic".into());
    write_caster(
        &save,
        "hero",
        "wizard",
        None,
        "2014",
        1,
        &[("int", 16)],
        lists,
    );
    campaign(&save, &root, "2014");
    let out = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "detect-magic".into(),
        },
    )
    .unwrap();
    assert!(
        out.join("；").contains("仪式 detect-magic"),
        "{}",
        out.join("；")
    );
    let _ = fs::remove_dir_all(&root);

    // 2014 warlock: D-12 fail-closed.
    let root = t3_root("ritual-warlock");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("known", "cha");
    lists.known.push("detect-magic".into());
    write_caster(
        &save,
        "hero",
        "warlock",
        Some("fiend"),
        "2014",
        11,
        &[("cha", 16)],
        lists,
    );
    campaign(&save, &root, "2014");
    let err = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "detect-magic".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("D-12"), "{err}");
    assert!(err.to_string().contains("DM 手动裁定"), "{err}");
    let _ = fs::remove_dir_all(&root);

    // 2024 wizard: book spells ritual-cast via Ritual Adept (edition-level
    // prepared rule), non-ritual spells refuse.
    let root = t3_root("ritual-2024");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("spellbook", "int");
    lists.book.push("detect-magic".into());
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "wizard",
        None,
        "2024",
        1,
        &[("int", 16)],
        lists,
    );
    campaign(&save, &root, "2024");
    let out = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "detect-magic".into(),
        },
    )
    .unwrap();
    assert!(
        out.join("；").contains("仪式 detect-magic"),
        "{}",
        out.join("；")
    );
    let err = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "bless".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("没有仪式（Ritual）标签"), "{err}");
    let _ = fs::remove_dir_all(&root);

    // 2014 bard (ritual: known) — the known list only.
    let root = t3_root("ritual-bard");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("known", "cha");
    lists.known.push("detect-magic".into());
    write_caster(
        &save,
        "hero",
        "bard",
        None,
        "2014",
        1,
        &[("cha", 16)],
        lists,
    );
    campaign(&save, &root, "2014");
    let out = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "detect-magic".into(),
        },
    )
    .unwrap();
    assert!(
        out.join("；").contains("仪式 detect-magic"),
        "{}",
        out.join("；")
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn ritual_fails_closed_without_content_data() {
    let root = t3_root("ritual-noregistry");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.prepared.push("detect-magic".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    // No campaign manifest → the registry is unavailable.
    let err = run_op(
        &save,
        "hero",
        Op::Ritual {
            spell: "detect-magic".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("失败关闭"), "{err}");
    // Nothing was written.
    assert_eq!(
        fs::read_to_string(save.join("Sessions/log.md")).unwrap(),
        "# log\n"
    );
    let _ = fs::remove_dir_all(&root);
}
// ---------------------------------------------------------------------------
// AC-5: chargen wiring — set spells fills the lists; the sheet writes refs;
// level-up prompts and keeps everything
// ---------------------------------------------------------------------------

#[test]
fn spell_lists_from_text_routes_and_keeps_legacy() {
    let mut c = Creation {
        ruleset: Some("2014".into()),
        class: Some("cleric".into()),
        level: 1,
        level_done: true,
        spells_done: true,
        ..Default::default()
    };
    c.spells = vec![
        "guidance".into(),
        "sacred-flame".into(),
        "bless".into(),
        "cure-wounds".into(),
        "Fire Bolt".into(),
    ];
    let lists = talkdnd::chargen::spell_lists_from_text(&c, None).expect("cleric lists");
    assert_eq!(lists.mode, "prepared");
    assert_eq!(lists.ability, "wis");
    assert_eq!(
        lists.cantrips,
        vec!["guidance".to_string(), "sacred-flame".to_string()]
    );
    assert!(lists.prepared.contains(&"bless".to_string()));
    assert!(lists.prepared.contains(&"cure-wounds".to_string()));
    assert_eq!(
        lists.legacy,
        vec!["Fire Bolt".to_string()],
        "a wizard cantrip is not a cleric spell"
    );
    // Non-casters and unset classes give None (free text still works).
    let mut f = c.clone();
    f.class = Some("fighter".into());
    assert!(talkdnd::chargen::spell_lists_from_text(&f, None).is_none());
    let mut n = c.clone();
    n.class = None;
    assert!(talkdnd::chargen::spell_lists_from_text(&n, None).is_none());
}

#[test]
fn chargen_sheet_writes_spell_refs_into_the_combat_block() {
    let mut c = Creation {
        ruleset: Some("2014".into()),
        class: Some("cleric".into()),
        subclass: Some("life".into()),
        level: 1,
        level_done: true,
        spells_done: true,
        name: Some("Aria".into()),
        ..Default::default()
    };
    c.spells = vec!["guidance".into(), "bless".into(), "Unknown Charm".into()];
    c.spell_lists = talkdnd::chargen::spell_lists_from_text(&c, None);
    let (id, md) = talkdnd::chargen::sheet(&c, "p").unwrap();
    assert_eq!(id, "aria");
    assert!(md.contains("- {ref: bless}"), "{md}");
    assert!(md.contains("- {ref: guidance}"), "{md}");
    // The legacy entry gets no ref (only the lists are the source).
    assert!(!md.contains("unknown-charm"), "{md}");
}

#[test]
fn level_up_prompts_for_new_choices_and_keeps_lists() {
    let root = t3_root("levelup");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.prepared.push("bless".into());
    write_caster(
        &save,
        "hero",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    fs::write(save.join("progress.yaml"), "xp:\n  hero: 0\n").unwrap();
    let out = talkdnd::progress::level_up(
        &save,
        &root,
        "hero",
        "avg",
        None,
        Some("story beat"),
        &mut t3rng(),
        false,
    )
    .unwrap();
    let joined = out.lines.join("\n");
    assert!(joined.contains("spells: 可准备法术 1/"), "{joined}");
    assert!(joined.contains("prepare"), "{joined}");
    // Lists and refs survive untouched.
    let lists = lists_of(&save, "hero");
    assert_eq!(lists.prepared, vec!["bless".to_string()]);
    let _ = fs::remove_dir_all(&root);

    // Warlock level 13 unlocks arcanum-7 and says so.
    let root = t3_root("levelup-arcanum");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("known", "cha");
    lists.cantrips.push("eldritch-blast".into());
    lists.arcanum.push("eyebite".into());
    write_caster(
        &save,
        "hero",
        "warlock",
        Some("fiend"),
        "2014",
        12,
        &[("cha", 16)],
        lists,
    );
    fs::write(save.join("progress.yaml"), "xp:\n  hero: 0\n").unwrap();
    let out = talkdnd::progress::level_up(
        &save,
        &root,
        "hero",
        "avg",
        None,
        Some("story beat 13"),
        &mut t3rng(),
        false,
    )
    .unwrap();
    let joined = out.lines.join("\n");
    assert!(
        joined.contains("Mystic Arcanum (7 环) unlocked"),
        "{joined}"
    );
    let lists = lists_of(&save, "hero");
    assert_eq!(
        lists.arcanum,
        vec!["eyebite".to_string()],
        "arcanum list untouched"
    );
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// AC-1: the view — stable text output and the fixed-key JSON
// ---------------------------------------------------------------------------

#[test]
fn spells_view_is_sorted_with_fixed_json_keys() {
    let root = t3_root("view");
    let save = t3_save(&root, "s");
    let mut lists = empty_lists("prepared", "wis");
    lists.cantrips = vec![
        "thaumaturgy".into(),
        "guidance".into(),
        "sacred-flame".into(),
    ];
    lists.prepared = vec![
        "shield-of-faith".into(),
        "bless".into(),
        "cure-wounds".into(),
    ];
    lists.always_prepared.push("cure-wounds".into());
    lists.legacy = vec!["神秘未知术".into()];
    write_caster(
        &save,
        "hero",
        "cleric",
        Some("life"),
        "2014",
        1,
        &[("wis", 16)],
        lists,
    );
    let text = fs::read_to_string(charsheet::path(&save, "hero")).unwrap();
    let cf: CharFile = serde_norway::from_str(&text).unwrap();
    let ps = charsheet::party(&save, &cf).unwrap();

    let lines = charsheet::spells_view(&cf, &ps, &Book::srd("2014"), false).unwrap();
    let joined = lines.join("\n");
    // Sorted by (level, id).
    assert!(
        joined.contains("戏法（3/3）：guidance、sacred-flame、thaumaturgy"),
        "{joined}"
    );
    assert!(
        joined.contains("可准备（3/4）：bless、cure-wounds、shield-of-faith"),
        "{joined}"
    );
    assert!(joined.contains("始终准备（1"), "{joined}");
    assert!(joined.contains("legacy"), "{joined}");
    assert!(joined.contains("法术位：1环 2/2"), "{joined}");

    let json = charsheet::spells_view(&cf, &ps, &Book::srd("2014"), true)
        .unwrap()
        .join("");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    for key in [
        "mode",
        "ability",
        "cantrips",
        "known",
        "prepared",
        "book",
        "always_prepared",
        "arcanum",
        "legacy",
        "slots",
        "counts",
    ] {
        assert!(v.get(key).is_some(), "missing key {key}: {json}");
    }
    assert_eq!(
        v["cantrips"],
        serde_json::json!(["guidance", "sacred-flame", "thaumaturgy"])
    );
    assert_eq!(v["counts"]["prepared"]["max"], 4);
    assert_eq!(v["slots"][0]["id"], "spell-slot-1");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn non_caster_and_missing_block_fail_with_clear_errors() {
    let root = t3_root("errors");
    let save = t3_save(&root, "s");
    // Fighter: not a caster.
    let cf = write_caster(
        &save,
        "rex",
        "fighter",
        None,
        "2014",
        1,
        &[("str", 16)],
        empty_lists("prepared", "wis"),
    );
    let err = charsheet::spells_view(
        &cf,
        &charsheet::party(&save, &cf).unwrap(),
        &Book::srd("2014"),
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("不是施法职业"), "{err}");
    let err = run_op(
        &save,
        "rex",
        Op::Prepare {
            spell: "bless".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("不是施法职业"), "{err}");

    // A caster without the schema-10 block is told to migrate/adopt.
    let mut cleric_lists = empty_lists("prepared", "wis");
    cleric_lists.prepared.push("bless".into());
    write_caster(
        &save,
        "mina",
        "cleric",
        None,
        "2014",
        1,
        &[("wis", 16)],
        cleric_lists,
    );
    let text = fs::read_to_string(charsheet::path(&save, "mina")).unwrap();
    let mut cf: CharFile = serde_norway::from_str(&text).unwrap();
    cf.spells = None;
    charsheet::store(&save, &cf).unwrap();
    let err = run_op(
        &save,
        "mina",
        Op::Spells {
            json: false,
            cmd: None,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("spells 法术列表块"), "{err}");
    let err = run_op(
        &save,
        "mina",
        Op::Prepare {
            spell: "bless".into(),
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("talkdnd migrate"), "{err}");
    let _ = fs::remove_dir_all(&root);
}
// ---------------------------------------------------------------------------
// R3 修正（PR #245 退回 3）：子职 always_prepared 自动播种与 Land 地形过滤
//（docs/SPELLCASTING.md §3.9）。
// ---------------------------------------------------------------------------

#[test]
fn always_prepared_grants_seed_at_chargen_and_sync_backfills() {
    let book = Book::srd("2014");
    let cd = book.class("cleric").unwrap();

    // 建卡 set spells：1 级 Life 只播种 from_level 1 的 bless/cure-wounds。
    let mut c = Creation {
        ruleset: Some("2014".into()),
        class: Some("cleric".into()),
        subclass: Some("life".into()),
        level: 1,
        level_done: true,
        spells_done: true,
        spells: vec!["guidance".into()],
        ..Default::default()
    };
    c.spell_lists = chargen::spell_lists_from_text(&c, Some(&book));
    let lists = c.spell_lists.clone().unwrap();
    assert!(lists.cantrips.contains(&"guidance".to_string()));
    assert_eq!(
        lists.always_prepared,
        vec!["bless".to_string(), "cure-wounds".to_string()],
        "level-1 Life seeds exactly the ring-1 grants in data order"
    );
    // 播种不占准备数量：基础表法术仍在来源，可准备满上限。
    let universe = classdata::spell_universe(&book, cd, Some("life"), 1, None);
    for s in ["shield-of-faith", "sanctuary", "command", "healing-word"] {
        assert!(universe.contains_key(s), "base-table spell {s} expected");
    }

    // 5 级播种 6 个（beacon-of-hope/revivify 等 from_level 5 到位）。
    c.level = 5;
    c.spell_lists = chargen::spell_lists_from_text(&c, Some(&book));
    let lists5 = c.spell_lists.clone().unwrap();
    for id in [
        "bless",
        "cure-wounds",
        "lesser-restoration",
        "spiritual-weapon",
        "beacon-of-hope",
        "revivify",
    ] {
        assert!(lists5.always_prepared.contains(&id.to_string()), "{id}");
    }
    assert!(
        !lists5.always_prepared.contains(&"death-ward".to_string()),
        "from_level 7 must not seed at level 5"
    );

    // 未选子职：不播种。
    let mut none = c.clone();
    none.subclass = None;
    none.spell_lists = chargen::spell_lists_from_text(&none, Some(&book));
    assert!(
        none.spell_lists.unwrap().always_prepared.is_empty(),
        "no subclass, no always_prepared seeds"
    );

    // T2 旧块（always_prepared 为空）sync 后补齐并生成 refs。
    let root = t3_root("seed-sync");
    let save = t3_save(&root, "s");
    write_caster(
        &save,
        "hero",
        "cleric",
        Some("life"),
        "2014",
        1,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let out = run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("播种"), "{joined}");
    let lists = lists_of(&save, "hero");
    assert_eq!(
        lists.always_prepared,
        vec!["bless".to_string(), "cure-wounds".to_string()]
    );
    let refs: Vec<String> = ref_entries(&save, "hero")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(refs.contains(&"bless".to_string()));
    assert!(refs.contains(&"cure-wounds".to_string()));
    assert!(
        !lists.prepared.contains(&"bless".to_string()),
        "seeding must not duplicate into prepared"
    );

    // dry-run：播种只进报告，sheet 逐字节不变。
    let root2 = t3_root("seed-dry");
    let save2 = t3_save(&root2, "s");
    write_caster(
        &save2,
        "hero",
        "cleric",
        Some("life"),
        "2014",
        1,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let before = sheet_text(&save2, "hero");
    let out = run_op(
        &save2,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: true }),
        },
    )
    .unwrap();
    assert!(
        out.join("；")
            .contains("[dry-run] 播种子职 always_prepared"),
        "{}",
        out.join("；")
    );
    assert_eq!(before, sheet_text(&save2, "hero"), "dry-run must not write");
    // 真正 sync 后 unprepare 播种条目被拒。
    run_op(
        &save2,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let err = run_op(
        &save2,
        "hero",
        Op::Unprepare {
            spell: "bless".into(),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("不可 unprepare"), "{err}");
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&root2);
}

#[test]
fn always_prepared_does_not_count_against_the_prepared_cap() {
    let root = t3_root("seed-cap");
    let save = t3_save(&root, "s");
    write_caster(
        &save,
        "hero",
        "cleric",
        Some("life"),
        "2014",
        1,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    run_op(
        &save,
        "hero",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    // 1 级 Life（wis 16）上限 4：播种 2 个之后仍能准备满 4 个。
    for s in ["shield-of-faith", "sanctuary", "command", "healing-word"] {
        run_op(&save, "hero", Op::Prepare { spell: s.into() })
            .unwrap_or_else(|e| panic!("prepare {s}: {e}"));
    }
    let lists = lists_of(&save, "hero");
    assert_eq!(lists.prepared.len(), 4, "seeds must not consume the cap");
    assert_eq!(lists.always_prepared.len(), 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn seeded_2024_life_and_fiend_follow_from_level() {
    let save_root = t3_root("seed-2024");
    let save = t3_save(&save_root, "s");
    // 2024 Life：1 级没有播种（from_level 3）。
    write_caster(
        &save,
        "lumen",
        "cleric",
        Some("life"),
        "2024",
        1,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    run_op(
        &save,
        "lumen",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    assert!(
        lists_of(&save, "lumen").always_prepared.is_empty(),
        "2024 Life has no grants before level 3"
    );
    // 2024 Fiend 3 级播种 1/2 环，5 级补 fireball。
    write_caster(
        &save,
        "ash",
        "warlock",
        Some("fiend"),
        "2024",
        3,
        &[("cha", 16)],
        empty_lists("known", "cha"),
    );
    run_op(
        &save,
        "ash",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    let lists = lists_of(&save, "ash");
    assert!(lists.always_prepared.contains(&"burning-hands".to_string()));
    assert!(lists.always_prepared.contains(&"command".to_string()));
    assert!(
        !lists.always_prepared.contains(&"fireball".to_string()),
        "2024 Fiend fireball waits for level 5"
    );
    write_caster(
        &save,
        "ash",
        "warlock",
        Some("fiend"),
        "2024",
        5,
        &[("cha", 16)],
        empty_lists("known", "cha"),
    );
    run_op(
        &save,
        "ash",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Sync { dry_run: false }),
        },
    )
    .unwrap();
    assert!(lists_of(&save, "ash")
        .always_prepared
        .contains(&"fireball".to_string()));
    // 2014 Fiend 是 class_list（扩表选项）：绝不播种 always_prepared。
    let book14 = Book::srd("2014");
    let mut c = Creation {
        ruleset: Some("2014".into()),
        class: Some("warlock".into()),
        subclass: Some("fiend".into()),
        level: 5,
        level_done: true,
        spells_done: true,
        spells: vec![],
        ..Default::default()
    };
    c.spell_lists = chargen::spell_lists_from_text(&c, Some(&book14));
    assert!(c.spell_lists.unwrap().always_prepared.is_empty());
    let _ = fs::remove_dir_all(&save_root);
}

#[test]
fn terrain_command_validates_filters_and_displays() {
    let root = t3_root("terrain");
    let save = t3_save(&root, "s");

    // 非 Land 子职（cleric life）：职业没有地形授予，拒绝。
    write_caster(
        &save,
        "lifen",
        "cleric",
        Some("life"),
        "2014",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let err = run_op(
        &save,
        "lifen",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "arctic".into(),
            }),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("没有按地形分组"), "{err}");

    // 未选子职的 druid：提示先选有地形授予的子职。
    write_caster(
        &save,
        "oak",
        "druid",
        None,
        "2014",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let err = run_op(
        &save,
        "oak",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "arctic".into(),
            }),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("先选"), "{err}");

    // Land + 非法地形：拒绝并列出可选值。
    write_caster(
        &save,
        "thorn",
        "druid",
        Some("land"),
        "2014",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let err = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "volcano".into(),
            }),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("可选"), "{err}");

    // 合法地形：写入 + 播种 arctic；视图文本与 JSON 显示地形。
    let out = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "arctic".into(),
            }),
        },
    )
    .unwrap();
    let joined = out.join("；");
    assert!(joined.contains("地形设为 arctic"), "{joined}");
    assert!(joined.contains("hold-person"), "{joined}");
    let lists = lists_of(&save, "thorn");
    assert_eq!(lists.terrain.as_deref(), Some("arctic"));
    for id in ["hold-person", "spike-growth", "sleet-storm", "slow"] {
        assert!(lists.always_prepared.contains(&id.to_string()), "{id}");
    }
    assert!(
        !lists.always_prepared.contains(&"fireball".to_string()),
        "other terrain grants must not seed"
    );
    let view = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: None,
        },
    )
    .unwrap();
    assert!(
        view.iter().any(|l| l.contains("地形：arctic")),
        "{}",
        view.join(
            "
"
        )
    );
    let json = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: true,
            cmd: None,
        },
    )
    .unwrap()
    .join("");
    assert!(json.contains("\"terrain\":\"arctic\""), "{json}");

    // 重复设置同一地形是幂等 no-op。
    let out = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "arctic".into(),
            }),
        },
    )
    .unwrap();
    assert!(out.join("；").contains("无变化"), "{}", out.join("；"));

    // 2024 Land（arid/polar/temperate/tropical）同样工作（独立 save，
    // save 级 edition 取首张表）。
    let root24 = t3_root("terrain-2024");
    let save24 = t3_save(&root24, "s");
    write_caster(
        &save24,
        "moss",
        "druid",
        Some("land"),
        "2024",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    run_op(
        &save24,
        "moss",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "arid".into(),
            }),
        },
    )
    .unwrap();
    let lists = lists_of(&save24, "moss");
    assert_eq!(lists.terrain.as_deref(), Some("arid"));
    assert!(lists.always_prepared.contains(&"fire-bolt".to_string()));
    assert!(lists.always_prepared.contains(&"blur".to_string()));
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&root24);
}

#[test]
fn terrain_failures_and_views_write_nothing() {
    let root = t3_root("terrain-readonly");
    let save = t3_save(&root, "s");
    write_caster(
        &save,
        "thorn",
        "druid",
        Some("land"),
        "2014",
        5,
        &[("wis", 16)],
        empty_lists("prepared", "wis"),
    );
    let before = sheet_text(&save, "thorn");
    let err = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: Some(charsheet::SpellsCmd::Terrain {
                terrain: "volcano".into(),
            }),
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("可选"), "{err}");
    let _ = run_op(
        &save,
        "thorn",
        Op::Spells {
            json: false,
            cmd: None,
        },
    )
    .unwrap();
    assert_eq!(
        before,
        sheet_text(&save, "thorn"),
        "failed/readonly ops must not write"
    );
    let _ = fs::remove_dir_all(&root);
}
