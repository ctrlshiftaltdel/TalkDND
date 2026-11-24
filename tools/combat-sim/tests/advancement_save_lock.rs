//! ADV-P1-1b: adopted combat-sim writers take the SAVE lock, exercised on the
//! **real** `combat-sim` binary.
//!
//! A save without protocol files keeps its existing behaviour and is never
//! auto-initialised. On a protocol-enabled save, group B writers acquire the
//! SAVE lock before writing; if the lock or state is missing, unwritable,
//! `active` or `in_doubt`, they fail closed and leave the tree unchanged.
//! Missing SAVE state on a protocol-enabled save is the plan's conservative
//! temporary handling, documented in the refusal text. Leftover 1c–1e and
//! `handout build|mark|list --dm` writers still hit the transition gate in
//! `talkdnd`. `combat-sim` does not depend on `talkdnd`.

use advancement_lock::{save_lock as adv, Class, Cli, WRITERS};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SLOT: &str = "2030-01-02-01";

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "combat-sim-p1b-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("Saves")).unwrap();
    fs::write(root.join("AGENTS.md"), "x").unwrap();
    root
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct Play {
    root: PathBuf,
    save: PathBuf,
    campaign: PathBuf,
    state: PathBuf,
}

/// YAML double quotes treat `\` as an escape. Forward slashes work on Windows
/// (`C:\\Users\\...` would be parsed as a Unicode escape and fail).
fn yaml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn play(tag: &str) -> Play {
    let root = temp_root(tag);
    let library = root.join("library");
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/engine/library"),
        &library,
    );
    let save = root.join("Saves").join(SLOT);
    let combat = save.join("combat");
    fs::create_dir_all(&combat).unwrap();
    let campaign = combat.join("campaign.yaml");
    fs::write(
        &campaign,
        format!(
            "name: P1b Fixtures\nlibrary:\n  - \"{}\"\nstate_dir: \"{}\"\nparty:\n  - striker-a\n  - warden-b\n",
            yaml_path(&library),
            yaml_path(&combat)
        ),
    )
    .unwrap();
    let state = combat.join("e.state.json");
    Play {
        root,
        save,
        campaign,
        state,
    }
}

fn run_bin(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("failed to run combat-sim")
}

fn run(play: &Play, args: &[&str]) -> Output {
    let mut full = vec![
        "--state".to_string(),
        play.state.to_string_lossy().into_owned(),
        "--save".to_string(),
        play.save.to_string_lossy().into_owned(),
        "--campaign".to_string(),
        play.campaign.to_string_lossy().into_owned(),
    ];
    full.extend(args.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    run_bin(&refs)
}

fn output_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
}

fn ok(play: &Play, args: &[&str]) -> String {
    let out = run(play, args);
    assert!(
        out.status.success(),
        "`{}` failed: {}",
        args.join(" "),
        output_text(&out)
    );
    output_text(&out)
}

fn init_protocol(play: &Play) {
    match adv::init(&play.root, &play.save, &adv::InitOptions::new()) {
        Ok(_) => {}
        Err(err) if err.kind() == adv::ErrorKind::StatePublishedNotDurable => {}
        Err(err) => panic!("init protocol: {err}"),
    }
    assert!(
        play.save.join(adv::LOCK_FILE).is_file() && play.save.join(adv::STATE_FILE).is_file(),
        "init must create both protocol files"
    );
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|entry| entry.unwrap())
            .collect();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(base).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                out.insert(rel, None);
                walk(base, &path, out);
            } else {
                out.insert(rel, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn argv(row: &advancement_lock::Writer, save: &Path) -> Vec<String> {
    let save = save.to_string_lossy();
    row.argv
        .iter()
        .map(|arg| arg.replace("{save}", &save))
        .collect()
}

fn argv_refs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

fn adopted_rows() -> Vec<&'static advancement_lock::Writer> {
    WRITERS
        .iter()
        .filter(|row| row.cli == Cli::CombatSim && row.class == Class::AdoptedWrite)
        .collect()
}

fn plant_state(save: &Path, mutate: impl FnOnce(&mut adv::StateRecord)) {
    let path = adv::state_path(save);
    let mut record: adv::StateRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    mutate(&mut record);
    adv::reseal(&mut record).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
}

fn assert_refused_unwritten(
    out: &Output,
    save: &Path,
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    entry: &str,
    expect: &[&str],
) {
    let text = output_text(out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{entry} must exit 1, got {:?}\n{text}",
        out.status.code()
    );
    assert!(
        text.contains("已启用成长协议"),
        "{entry}: refusal must name the protocol\n{text}"
    );
    assert!(
        text.contains("未写入任何目标文件"),
        "{entry}: refusal must say nothing was written (not maybe-written)\n{text}"
    );
    assert!(
        !text.contains("可能已被写入"),
        "{entry}: lock-unavailable refusal must not use the maybe-written stage\n{text}"
    );
    for needle in expect {
        assert!(
            text.contains(needle),
            "{entry}: expected `{needle}` in\n{text}"
        );
    }
    assert_eq!(
        snapshot(save),
        *before,
        "{entry} wrote the save while refusing"
    );
}

fn new_encounter(play: &Play) {
    let out = run(
        play,
        &[
            "new",
            "--encounter",
            "test-fight",
            "--party",
            "striker-a,warden-b",
        ],
    );
    assert!(
        out.status.success(),
        "new encounter failed: {}",
        output_text(&out)
    );
}

fn write_srd_prose_index(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string(&json!({
            "tool": "0.6.0",
            "pdf": "SRD_Test.pdf",
            "sha256": "00",
            "page_count": 4
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("index.json"),
        serde_json::to_string(&json!({
            "headings": [
                {"level": 2, "page": 1, "text": "Playing the Game"},
                {"level": 3, "page": 1, "text": "Rhythm of Play"},
                {"level": 3, "page": 2, "text": "Actions"},
                {"level": 2, "page": 3, "text": "Character Creation"},
                {"level": 2, "page": 4, "text": "Classes"},
                {"level": 2, "page": 5, "text": "Monsters"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("SRD_Test.pages.md"),
        r#"<a id="p1"></a>
<!-- PDF p.1 -->
## Playing the Game

The three pillars of play are social interaction, exploration, and combat.

### Rhythm of Play

First version of the rhythm text.

### Short

x

<a id="p2"></a>
<!-- PDF p.2 -->
### Actions

When you do something other than moving, you take an action.

### Rhythm of Play

Second version adds the continuation text that must be merged.

<a id="p3"></a>
<!-- PDF p.3 -->
## Character Creation

You record the main details of your character on a sheet.

### Step 1: Choose Class

Choose a class and write it on your character sheet.

<a id="p4"></a>
<!-- PDF p.4 -->
## Classes

Class chapter start.
"#,
    )
    .unwrap();
}

fn write_srd_pack_src(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let monster = json!([{
        "index": "cog-wolf", "name": "Cog Wolf", "size": "Medium", "type": "construct",
        "armor_class": [{"type": "natural", "value": 13}], "hit_points": 11, "hit_points_roll": "2d8+2",
        "speed": {"walk": "40 ft."}, "strength": 12, "dexterity": 15, "constitution": 12,
        "intelligence": 3, "wisdom": 12, "charisma": 6, "proficiency_bonus": 2,
        "challenge_rating": 0.25, "xp": 50,
        "actions": [{"name": "Bite", "desc": "Melee Weapon Attack: +4 to hit.", "attack_bonus": 4,
            "damage": [{"damage_type": {"index": "piercing"}, "damage_dice": "1d6+2"}]}]
    }]);
    let spell = json!([{
        "index": "cog-bolt", "name": "Cog Bolt", "level": 0, "desc": ["Zap."],
        "range": "60 feet", "casting_time": "1 action", "attack_type": "ranged",
        "damage": {"damage_type": {"index": "lightning"}, "damage_at_character_level": {"1": "1d10"}}
    }]);
    for (name, value) in [
        ("Monsters", monster),
        ("Spells", spell),
        (
            "Classes",
            json!([{"index": "test-class", "name": "Test Class", "hit_die": 8,
                "saving_throws": [{"index": "dex", "name": "DEX"}],
                "proficiencies": [{"index": "light-armor", "name": "Light armor"}]}]),
        ),
        (
            "Subclasses",
            json!([{"index": "test-subclass", "name": "Test Subclass",
                "class": {"index": "test-class", "name": "Test Class"},
                "desc": ["A test subclass."]}]),
        ),
        (
            "Levels",
            json!([{"index": "test-class-1", "level": 1, "prof_bonus": 2,
                "features": [{"index": "test-feature", "name": "Test Feature"}],
                "class": {"index": "test-class", "name": "Test Class"}}]),
        ),
        (
            "Features",
            json!([{"index": "test-feature", "name": "Test Feature", "level": 1,
                "desc": ["A test feature."],
                "class": {"index": "test-class", "name": "Test Class"}}]),
        ),
    ] {
        fs::write(
            dir.join(format!("5e-SRD-{name}.json")),
            serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
    }
}

fn write_import_srd_monsters(path: &Path) {
    fs::write(
        path,
        serde_json::to_string(&json!([{
            "index": "cog-wolf", "name": "Cog Wolf", "size": "Medium", "type": "construct",
            "armor_class": [{"type": "natural", "value": 13}], "hit_points": 11, "hit_points_roll": "2d8+2",
            "speed": {"walk": "40 ft."}, "strength": 12, "dexterity": 15, "constitution": 12,
            "intelligence": 3, "wisdom": 12, "charisma": 6, "proficiency_bonus": 2,
            "challenge_rating": 0.25, "xp": 50
        }]))
        .unwrap(),
    )
    .unwrap();
}

fn write_full_pack_index(dir: &Path) -> PathBuf {
    let index = dir.join("index");
    fs::create_dir_all(&index).unwrap();
    fs::write(
        index.join("manifest.json"),
        serde_json::to_string_pretty(&json!({
            "tool": "0.6.0",
            "pdf": "Test Book.pdf",
            "sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "page_count": 4,
            "created": "2026-01-02T03:04:05",
            "intake": {"chapters": []}
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        index.join("index.json"),
        serde_json::to_string_pretty(&json!({
            "page_count": 4,
            "done_pages": 3,
            "headings": [
                {"level": 2, "page": 1, "text": "Chapter 1: The First Job"},
                {"level": 3, "page": 1, "text": "The Locked Door"},
                {"level": 3, "page": 2, "text": "The Guard Post"},
                {"level": 2, "page": 3, "text": "Chapter 2: The Second Job"}
            ],
            "chapters": [
                {"title": "Chapter 1: The First Job", "page": 1},
                {"title": "Chapter 2: The Second Job", "page": 3}
            ],
            "figures": [],
            "art": [],
            "low_quality_pages": [],
            "statblocks": []
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        index.join("Test Book.pages.md"),
        r#"<a id="p1"></a>
<!-- PDF p.1 -->
## Chapter 1: The First Job

Opening text for the first chapter.

### The Locked Door

The door is locked.

<a id="p2"></a>
<!-- PDF p.2 -->
### The Guard Post

The guard keeps a key.

<a id="p3"></a>
<!-- PDF p.3 -->
## Chapter 2: The Second Job

Second chapter text.

<a id="p4"></a>
<!-- PDF p.4 -->
Body without a heading stays out of the section index.
"#,
    )
    .unwrap();
    index
}

fn write_probe_pack(dir: &Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    let content: combat_sim::content::schema::ContentFile = serde_json::from_value(json!({
        "conditions": [{"id": "packed", "name": "Packed"}]
    }))
    .unwrap();
    let pack = combat_sim::content::pack::ContentPack::new(
        combat_sim::content::pack::PackMeta {
            id: "probe".into(),
            name: Some("Probe".into()),
            edition: "2024".into(),
            source: None,
            license: None,
            distribution: combat_sim::content::pack::Distribution::Full,
            schema_version: combat_sim::content::pack::SCHEMA_VERSION,
            content_hash: None,
            created_at: None,
            description: None,
        },
        content,
    )
    .unwrap();
    fs::write(dir.join(name), serde_json::to_string(&pack).unwrap()).unwrap();
}

fn prepare_with_encounter(tag: &str) -> Play {
    let play = play(tag);
    new_encounter(&play);
    init_protocol(&play);
    play
}

#[test]
fn adopted_state_party_and_roll_writers_succeed_under_an_initialized_protocol() {
    let play = prepare_with_encounter("success");
    let before_init = fs::metadata(&play.state).unwrap().modified().unwrap();
    ok(&play, &["init", "--seed", "7"]);
    let after_init = fs::metadata(&play.state).unwrap().modified().unwrap();
    assert!(
        after_init >= before_init,
        "init must rewrite the encounter state"
    );

    ok(&play, &["party", "set-hp", "striker-a", "3"]);
    let party = play.save.join("combat").join("party-state.yaml");
    assert!(party.is_file(), "party set-hp must write party-state.yaml");
    let party_text = fs::read_to_string(&party).unwrap();
    assert!(
        party_text.contains("striker-a") || party_text.contains("hp: 3"),
        "party-state should record the HP change:\n{party_text}"
    );

    let rolls = play.save.join("rolls.log");
    let before_rolls = if rolls.is_file() {
        fs::read(&rolls).unwrap()
    } else {
        Vec::new()
    };
    ok(&play, &["roll", "1d20", "--seed", "3"]);
    let after_rolls = fs::read(&rolls).unwrap();
    assert!(
        after_rolls != before_rolls,
        "roll without --state must append rolls.log"
    );
}

#[test]
fn roll_without_state_only_appends_rolls_log() {
    let play = prepare_with_encounter("roll-log");
    let before_state = fs::read(&play.state).unwrap();
    let before_party = snapshot(&play.save.join("combat"));
    let out = run_bin(&[
        "--save",
        play.save.to_str().unwrap(),
        "--campaign",
        play.campaign.to_str().unwrap(),
        "roll",
        "1d20",
        "--seed",
        "9",
    ]);
    assert!(
        out.status.success(),
        "roll without --state failed: {}",
        output_text(&out)
    );
    assert_eq!(
        fs::read(&play.state).unwrap(),
        before_state,
        "roll without --state must not rewrite the encounter state"
    );
    // party-state may be absent; the combat/ tree should stay the same.
    assert_eq!(
        snapshot(&play.save.join("combat")),
        before_party,
        "roll without --state must not rewrite party or state under combat/"
    );
    let log = fs::read_to_string(play.save.join("rolls.log")).unwrap();
    assert!(log.contains("1d20"), "rolls.log:\n{log}");
}

#[test]
fn roll_with_existing_state_rewrites_state() {
    let play = prepare_with_encounter("roll-state");
    let before_state = fs::read(&play.state).unwrap();
    ok(&play, &["roll", "1d6", "--seed", "5"]);
    let after_state = fs::read(&play.state).unwrap();
    assert_ne!(
        after_state, before_state,
        "roll with --state pointing at an existing file must save the encounter"
    );
}

#[test]
fn without_the_protocol_adopted_writers_keep_working_and_do_not_init() {
    let play = play("noproto");
    new_encounter(&play);
    assert!(!play.save.join(adv::LOCK_FILE).exists());
    assert!(!play.save.join(adv::STATE_FILE).exists());
    ok(&play, &["init", "--seed", "2"]);
    ok(&play, &["party", "set-hp", "striker-a", "4"]);
    ok(&play, &["roll", "1d20", "--seed", "2"]);
    assert!(!play.save.join(adv::LOCK_FILE).exists());
    assert!(!play.save.join(adv::STATE_FILE).exists());
}

#[test]
fn missing_or_unusable_save_state_refuses_adopted_writers() {
    let play = play("missing");
    new_encounter(&play);
    init_protocol(&play);
    let rows = adopted_rows();
    assert!(rows.len() >= 8, "combat-sim adopted set looks too small");

    let lock_bytes = fs::read(play.save.join(adv::LOCK_FILE)).unwrap();
    let state_bytes = fs::read(play.save.join(adv::STATE_FILE)).unwrap();

    fs::remove_file(play.save.join(adv::STATE_FILE)).unwrap();
    let before = snapshot(&play.save);
    for row in &rows {
        let args = argv(row, &play.save);
        let out = run(&play, &argv_refs(&args));
        assert_refused_unwritten(
            &out,
            &play.save,
            &before,
            row.entry,
            &["SAVE state 缺失", "不自动初始化"],
        );
    }

    fs::write(play.save.join(adv::LOCK_FILE), &lock_bytes).unwrap();
    fs::write(play.save.join(adv::STATE_FILE), &state_bytes).unwrap();
}

#[test]
fn lock_file_missing_on_a_protocol_save_refuses_adopted_writers() {
    let play = play("nolock");
    new_encounter(&play);
    init_protocol(&play);
    let state_bytes = fs::read(play.save.join(adv::STATE_FILE)).unwrap();
    fs::remove_file(play.save.join(adv::LOCK_FILE)).unwrap();
    // The remaining state file still counts as the protocol being present.
    let before = snapshot(&play.save);
    for row in adopted_rows() {
        let args = argv(row, &play.save);
        let out = run(&play, &argv_refs(&args));
        assert_refused_unwritten(
            &out,
            &play.save,
            &before,
            row.entry,
            &["SAVE 锁文件缺失", "不自动初始化"],
        );
    }
    let _ = state_bytes;
}

#[test]
fn active_and_in_doubt_state_refuse_adopted_writers() {
    let play = play("states");
    new_encounter(&play);
    init_protocol(&play);
    let state_bytes = fs::read(play.save.join(adv::STATE_FILE)).unwrap();
    let rows = adopted_rows();

    plant_state(&play.save, |record| {
        record.state = adv::State::Active;
        record.stage = adv::Stage::Admit;
        record.operation_id = "p1b-plant-active".into();
    });
    let before = snapshot(&play.save);
    for row in &rows {
        let args = argv(row, &play.save);
        let out = run(&play, &argv_refs(&args));
        assert_refused_unwritten(&out, &play.save, &before, row.entry, &["active"]);
    }

    fs::write(play.save.join(adv::STATE_FILE), &state_bytes).unwrap();
    plant_state(&play.save, |record| {
        record.state = adv::State::InDoubt;
        record.stage = adv::Stage::Publish;
        record.operation_id = "p1b-plant-in-doubt".into();
        record.last_error = Some("planted".into());
    });
    let before = snapshot(&play.save);
    for row in &rows {
        let args = argv(row, &play.save);
        let out = run(&play, &argv_refs(&args));
        assert_refused_unwritten(&out, &play.save, &before, row.entry, &["in_doubt"]);
    }
}

#[test]
fn read_only_combat_commands_leave_the_protocol_save_byte_identical() {
    let play = prepare_with_encounter("readonly");
    ok(&play, &["init", "--seed", "7"]);
    let before = snapshot(&play.save);
    for args in [
        ["status"].as_slice(),
        ["scene"].as_slice(),
        ["log"].as_slice(),
        ["party", "show"].as_slice(),
        ["content", "list"].as_slice(),
        ["content", "pack", "list"].as_slice(),
    ] {
        let out = run(&play, args);
        let text = output_text(&out);
        assert!(
            out.status.success(),
            "{} failed under the protocol\n{text}",
            args.join(" ")
        );
        assert!(
            !text.contains("尚未接入成长协议"),
            "{} must not be refused\n{text}",
            args.join(" ")
        );
        assert_eq!(
            snapshot(&play.save),
            before,
            "{} wrote the save while declared read-only",
            args.join(" ")
        );
    }
}

#[test]
fn content_out_inside_a_protocol_save_takes_the_save_lock_and_writes() {
    let play = prepare_with_encounter("out-in");
    let out_dir = play.save.join("content");
    fs::create_dir_all(&out_dir).unwrap();

    let monsters = play.root.join("monsters.json");
    write_import_srd_monsters(&monsters);
    let out = run_bin(&[
        "content",
        "import-srd",
        "--monsters",
        monsters.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "import-srd in-save failed: {}",
        output_text(&out)
    );
    let written: Vec<_> = fs::read_dir(&out_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        written
            .iter()
            .any(|n| n.contains("monster") || n.ends_with(".yaml")),
        "import-srd should write content under the save, got {written:?}"
    );

    let index = play.root.join("prose-index");
    write_srd_prose_index(&index);
    let out = run_bin(&[
        "content",
        "import-srd-prose",
        "--edition",
        "2024",
        "--index",
        index.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "import-srd-prose in-save failed: {}",
        output_text(&out)
    );

    let src = play.root.join("srd-src");
    write_srd_pack_src(&src);
    let out = run_bin(&[
        "content",
        "import-srd-pack",
        "--edition",
        "2024",
        "--src",
        src.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "import-srd-pack in-save failed: {}",
        output_text(&out)
    );

    let book = play.root.join("book");
    let index = write_full_pack_index(&book);
    let out = run_bin(&[
        "content",
        "pack",
        "build",
        "--edition",
        "2014",
        "--book",
        "phb",
        "--index",
        index.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "pack build in-save failed: {}",
        output_text(&out)
    );

    let packs = play.root.join("packs");
    write_probe_pack(&packs, "probe.pack.json");
    let release_out = play.save.join("release");
    let out = run_bin(&[
        "content",
        "pack",
        "release",
        "--packs",
        packs.to_str().unwrap(),
        "--out",
        release_out.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "pack release in-save failed: {}",
        output_text(&out)
    );
    assert!(
        release_out.join("full-packs-manifest.json").is_file()
            || fs::read_dir(&release_out).unwrap().next().is_some(),
        "pack release should write under the save"
    );
}

#[test]
fn content_out_outside_any_save_is_allowed_even_if_a_protocol_save_exists() {
    let play = prepare_with_encounter("out-out");
    let outside = play.root.join("outside-out");
    fs::create_dir_all(&outside).unwrap();
    let before = snapshot(&play.save);

    let monsters = play.root.join("monsters.json");
    write_import_srd_monsters(&monsters);
    let out = run_bin(&[
        "--save",
        play.save.to_str().unwrap(),
        "content",
        "import-srd",
        "--monsters",
        monsters.to_str().unwrap(),
        "--out",
        outside.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "import-srd out-of-save failed: {}",
        output_text(&out)
    );
    assert_eq!(
        snapshot(&play.save),
        before,
        "out-of-save --out must not write the protocol save"
    );
    let written: Vec<_> = fs::read_dir(&outside)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !written.is_empty(),
        "import-srd should write outside the save, got {written:?}"
    );

    let index = play.root.join("prose-index");
    write_srd_prose_index(&index);
    let out = run_bin(&[
        "--save",
        play.save.to_str().unwrap(),
        "content",
        "import-srd-prose",
        "--edition",
        "2024",
        "--index",
        index.to_str().unwrap(),
        "--out",
        outside.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "import-srd-prose out-of-save failed: {}",
        output_text(&out)
    );

    let src = play.root.join("srd-src");
    write_srd_pack_src(&src);
    let out = run_bin(&[
        "--save",
        play.save.to_str().unwrap(),
        "content",
        "import-srd-pack",
        "--edition",
        "2014",
        "--src",
        src.to_str().unwrap(),
        "--out",
        outside.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "import-srd-pack out-of-save failed: {}",
        output_text(&out)
    );

    let book = play.root.join("book");
    let index = write_full_pack_index(&book);
    let out = run_bin(&[
        "--save",
        play.save.to_str().unwrap(),
        "content",
        "pack",
        "build",
        "--edition",
        "2014",
        "--book",
        "phb",
        "--index",
        index.to_str().unwrap(),
        "--out",
        outside.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "pack build out-of-save failed: {}",
        output_text(&out)
    );

    let packs = play.root.join("packs");
    write_probe_pack(&packs, "probe.pack.json");
    let release_out = play.root.join("release-out");
    let out = run_bin(&[
        "--save",
        play.save.to_str().unwrap(),
        "content",
        "pack",
        "release",
        "--packs",
        packs.to_str().unwrap(),
        "--out",
        release_out.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "pack release out-of-save failed: {}",
        output_text(&out)
    );
    assert_eq!(snapshot(&play.save), before);
}

#[test]
fn pack_dry_run_is_read_only_even_when_out_is_inside_a_protocol_save() {
    let play = prepare_with_encounter("dry-run");
    let out_dir = play.save.join("content");
    fs::create_dir_all(&out_dir).unwrap();
    let before = snapshot(&play.save);
    let book = play.root.join("book");
    let index = write_full_pack_index(&book);
    let out = run_bin(&[
        "content",
        "pack",
        "build",
        "--edition",
        "2014",
        "--book",
        "phb",
        "--index",
        index.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--dry-run",
    ]);
    let text = output_text(&out);
    assert!(out.status.success(), "pack build --dry-run failed: {text}");
    assert_eq!(
        snapshot(&play.save),
        before,
        "pack build --dry-run wrote the save"
    );

    let packs = play.root.join("packs");
    write_probe_pack(&packs, "probe.pack.json");
    let out = run_bin(&[
        "content",
        "pack",
        "release",
        "--packs",
        packs.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--dry-run",
    ]);
    let text = output_text(&out);
    assert!(
        out.status.success(),
        "pack release --dry-run failed: {text}"
    );
    assert_eq!(
        snapshot(&play.save),
        before,
        "pack release --dry-run wrote the save"
    );
}

#[test]
fn content_out_inside_a_protocol_save_is_refused_when_save_state_is_missing() {
    let play = prepare_with_encounter("out-refuse");
    fs::remove_file(play.save.join(adv::STATE_FILE)).unwrap();
    let out_dir = play.save.join("content");
    fs::create_dir_all(&out_dir).unwrap();
    let before = snapshot(&play.save);
    let monsters = play.root.join("monsters.json");
    write_import_srd_monsters(&monsters);
    let out = run_bin(&[
        "content",
        "import-srd",
        "--monsters",
        monsters.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_refused_unwritten(
        &out,
        &play.save,
        &before,
        "combat-sim:content-import-srd",
        &["SAVE state 缺失", "不自动初始化"],
    );
}

#[cfg(unix)]
#[test]
fn a_failed_party_write_reports_the_state_stage_already_written() {
    let play = prepare_with_encounter("stage");
    ok(&play, &["init", "--seed", "1"]);
    // Point the encounter at a party-state path that cannot be written, then
    // run a mutating command: state.save should succeed, party.save should
    // stop and name the stages.
    let blocked = play.root.join("blocked-party");
    fs::create_dir_all(&blocked).unwrap();
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&play.state).unwrap()).unwrap();
    state["party_state"] = json!(blocked.to_string_lossy().into_owned());
    fs::write(&play.state, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    let out = run(&play, &["next"]);
    let text = output_text(&out);
    assert_eq!(out.status.code(), Some(1), "next should fail\n{text}");
    assert!(
        text.contains("已写入遭遇状态"),
        "failure must report the written stage\n{text}"
    );
    assert!(
        text.contains("尚未写入 party") || text.contains("尚未写入 party 或追加日志"),
        "failure must report the unwritten stage\n{text}"
    );
    assert!(
        text.contains("不是事务") || text.contains("可能已被写入"),
        "failure must not claim multi-file atomicity\n{text}"
    );
}

#[test]
fn leftover_talkdnd_writers_are_out_of_scope_and_combat_sim_has_none() {
    assert!(
        WRITERS
            .iter()
            .filter(|row| row.cli == Cli::CombatSim && row.class == Class::UnadoptedWrite)
            .count()
            == 0
    );
    assert!(WRITERS
        .iter()
        .any(|row| row.cli == Cli::Talkdnd && row.class == Class::UnadoptedWrite));
}
