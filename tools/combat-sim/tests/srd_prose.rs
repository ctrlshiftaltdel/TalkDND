//! SRD prose import: fixtures, merging, determinism and committed pack.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use combat_sim::content::pack::ContentPack;
use combat_sim::srd_prose;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "combat-sim-prose-{label}-{}-{nanos}-{count}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

const PAGES: &str = r#"<a id="p1"></a>
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
"#;

fn fixtures(dir: &Path) {
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_string(&json!({
            "tool": "0.6.0",
            "pdf": "SRD_Test.pdf",
            "sha256": "00",
            "page_count": 4
        }))
        .expect("json"),
    )
    .expect("manifest");
    let headings = json!({
        "headings": [
            {"level": 2, "page": 1, "text": "Playing the Game"},
            {"level": 3, "page": 1, "text": "Rhythm of Play"},
            {"level": 3, "page": 2, "text": "Actions"},
            {"level": 2, "page": 3, "text": "Character Creation"},
            {"level": 2, "page": 4, "text": "Classes"},
            {"level": 2, "page": 5, "text": "Monsters"}
        ]
    });
    fs::write(
        dir.join("index.json"),
        serde_json::to_string(&headings).expect("json"),
    )
    .expect("index");
    fs::write(dir.join("SRD_Test.pages.md"), PAGES).expect("pages");
}

fn read_pack(path: &Path) -> ContentPack {
    let text = fs::read_to_string(path).expect("read pack");
    let value: Value = serde_json::from_str(&text).expect("pack json");
    ContentPack::from_value(&value).expect("valid pack")
}

#[test]
fn extracts_sections_pages_and_merges_repeats() {
    let index = TempDir::new("src");
    let out = TempDir::new("out");
    fixtures(index.path());

    let report = srd_prose::import("2024", index.path(), out.path()).expect("import");
    assert_eq!(report.entities, 5, "{:?}", report.chapters);
    assert_eq!(report.chapters.len(), 2, "{:?}", report.chapters);
    assert_eq!(report.chapters[0].name, "Playing the Game");
    assert_eq!(report.chapters[0].start_page, 1);
    assert_eq!(report.chapters[0].end_page, 2);
    assert_eq!(report.chapters[1].name, "Character Creation");
    assert_eq!(report.chapters[1].start_page, 3);
    assert_eq!(report.skipped, 1, "the short heading must be skipped");
    assert!(
        report.duplicates.iter().any(|id| id.ends_with("(merged)")),
        "{:?}",
        report.duplicates
    );

    let pack = read_pack(&out.path().join("srd-5.2.1-rules-prose.pack.json"));
    assert_eq!(pack.pack.id, "srd-5.2.1-rules-prose");
    assert_eq!(pack.pack.edition, "2024");
    let sections = pack.content.rule_sections.as_ref().expect("sections");
    let by_id = |id: &str| {
        sections
            .iter()
            .find(|section| section.id == id)
            .unwrap_or_else(|| panic!("missing {id}"))
    };

    let intro = by_id("rule-2024-playing-the-game");
    assert_eq!(intro.page, Some(1));
    assert!(intro
        .text
        .as_deref()
        .unwrap_or("")
        .contains("three pillars"));

    let rhythm = by_id("rule-2024-playing-the-game-rhythm-of-play");
    let text = rhythm.text.as_deref().unwrap_or("");
    assert!(text.contains("First version"), "{text}");
    assert!(text.contains("Second version"), "{text}");

    let actions = by_id("rule-2024-playing-the-game-actions");
    assert_eq!(actions.page, Some(2));

    let step = by_id("rule-2024-character-creation-step-1-choose-class");
    assert_eq!(step.section.as_deref(), Some("Character Creation"));
    assert!(by_id("rule-2024-character-creation").text.is_some());
    assert!(sections
        .iter()
        .all(|section| section.id != "rule-2024-playing-the-game-short"));
}

#[test]
fn generation_is_deterministic() {
    let index = TempDir::new("det-src");
    fixtures(index.path());
    let first = TempDir::new("det-a");
    let second = TempDir::new("det-b");
    srd_prose::import("2024", index.path(), first.path()).expect("first");
    srd_prose::import("2024", index.path(), second.path()).expect("second");
    let name = "srd-5.2.1-rules-prose.pack.json";
    assert_eq!(
        fs::read(first.path().join(name)).expect("first pack"),
        fs::read(second.path().join(name)).expect("second pack")
    );
}

#[test]
fn rejects_unknown_edition() {
    let index = TempDir::new("bad-edition");
    let out = TempDir::new("bad-out");
    let error =
        srd_prose::import("2014", index.path(), out.path()).expect_err("2014 has no prose gap");
    assert!(
        format!("{error:#}").contains("unsupported prose edition"),
        "{error:#}"
    );
}

#[test]
fn committed_2024_prose_pack_is_complete() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../LetsPlay/Rules/combat/2024/srd/srd-5.2.1-rules-prose.pack.json");
    let pack = read_pack(&path);
    assert_eq!(pack.pack.id, "srd-5.2.1-rules-prose");
    assert_eq!(pack.entity_count(), 213);
    let sections = pack.content.rule_sections.as_ref().expect("sections");
    for chapter in [
        "Playing the Game",
        "Character Creation",
        "Rules Glossary",
        "Gameplay Toolbox",
    ] {
        assert!(
            sections
                .iter()
                .any(|section| section.section.as_deref() == Some(chapter)),
            "chapter {chapter} missing"
        );
    }
    for section in sections {
        assert!(
            section
                .text
                .as_deref()
                .is_some_and(|text| text.chars().count() >= 20),
            "{} has no body",
            section.id
        );
        assert!(section.page.is_some(), "{} has no page", section.id);
    }
}
