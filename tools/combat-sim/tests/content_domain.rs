//! Domain entity coverage for ContentPack and the content registry.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use combat_sim::content::pack::{ContentPack, Distribution, PackIndex, PackMeta, SCHEMA_VERSION};
use combat_sim::content::schema::ContentFile;
use combat_sim::content::{ContentRegistry, ContentSources, ItemKind};

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
            "combat-sim-domain-{label}-{}-{nanos}-{count}",
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

fn content() -> ContentFile {
    serde_json::from_value(json!({
        "items": [{"id": "item-1", "name": "Item One", "text": "item text"}],
        "classes": [{"id": "class-1", "name": "Class One", "text": "class text"}],
        "subclasses": [{"id": "subclass-1", "name": "Subclass One"}],
        "species": [{"id": "species-1", "name": "Species One"}],
        "backgrounds": [{"id": "background-1", "name": "Background One"}],
        "feats": [{"id": "feat-1", "name": "Feat One"}],
        "rule_sections": [{"id": "rule-1", "name": "Rule One", "text": "rule text"}],
        "tables": [{"id": "table-1", "name": "Table One"}]
    }))
    .expect("domain content")
}

fn meta() -> PackMeta {
    PackMeta {
        id: "domain-pack".to_string(),
        name: Some("Domain Pack".to_string()),
        edition: "2024".to_string(),
        source: Some("synthetic".to_string()),
        license: Some("CC-BY-4.0".to_string()),
        distribution: Distribution::Srd,
        schema_version: SCHEMA_VERSION,
        content_hash: None,
        created_at: None,
        description: None,
    }
}

fn domain_index() -> PackIndex {
    PackIndex {
        entries: vec![
            ("item-1", "item"),
            ("class-1", "class"),
            ("subclass-1", "subclass"),
            ("species-1", "species"),
            ("background-1", "background"),
            ("feat-1", "feat"),
            ("rule-1", "rule-section"),
            ("table-1", "table"),
        ]
        .into_iter()
        .map(|(id, kind)| combat_sim::content::pack::PackEntry {
            id: id.to_string(),
            kind: kind.to_string(),
            ..Default::default()
        })
        .collect(),
        images: vec![],
    }
}

fn pack() -> ContentPack {
    let mut pack = ContentPack::new(meta(), content()).expect("pack");
    pack.index = domain_index();
    pack.validate().expect("domain index");
    pack
}

fn load(dir: &Path) -> anyhow::Result<ContentRegistry> {
    ContentSources {
        libraries: vec![dir.to_path_buf()],
        ..ContentSources::default()
    }
    .load()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .args(args)
        .output()
        .expect("run combat-sim")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn loads_all_domain_entity_kinds() {
    let dir = TempDir::new("load");
    let text = serde_json::to_string_pretty(&pack()).expect("json");
    fs::write(dir.path().join("domain.pack.json"), text).expect("write pack");

    let registry = load(dir.path()).expect("domain pack loads");
    assert_eq!(registry.items.len(), 1);
    assert_eq!(registry.classes.len(), 1);
    assert_eq!(registry.subclasses.len(), 1);
    assert_eq!(registry.species.len(), 1);
    assert_eq!(registry.backgrounds.len(), 1);
    assert_eq!(registry.feats.len(), 1);
    assert_eq!(registry.rule_sections.len(), 1);
    assert_eq!(registry.tables.len(), 1);
    assert!(registry.find_any("item-1").is_some());
    assert!(registry.find_any("rule-1").is_some());
    assert!(registry
        .source_of(ItemKind::Class, "class-1")
        .unwrap()
        .path
        .ends_with("domain.pack.json"));
}

#[test]
fn domain_entity_duplicate_id_fails() {
    let value = json!({
        "items": [{"id": "same"}, {"id": "same"}]
    });
    let content: ContentFile = serde_json::from_value(value).expect("content");
    let error = ContentPack::new(meta(), content).expect_err("duplicate");
    assert!(
        format!("{error:#}").contains("duplicate entity"),
        "{error:#}"
    );
}

#[test]
fn invalid_domain_index_kind_fails() {
    let mut pack = ContentPack::new(meta(), content()).expect("pack");
    pack.index = PackIndex {
        entries: vec![combat_sim::content::pack::PackEntry {
            id: "item-1".to_string(),
            kind: "spellbook".to_string(),
            ..Default::default()
        }],
        images: vec![],
    };
    let error = pack.validate().expect_err("invalid kind");
    assert!(format!("{error:#}").contains("unknown kind"), "{error:#}");
}

#[test]
fn old_registry_without_domain_stores_loads() {
    let registry: ContentRegistry =
        serde_json::from_value(json!({"ruleset": "2024"})).expect("old registry");
    assert_eq!(registry.items.len(), 0);
    assert_eq!(registry.classes.len(), 0);
    assert_eq!(registry.tables.len(), 0);
}

#[test]
fn content_list_shows_domain_kinds() {
    let dir = TempDir::new("cli");
    let text = serde_json::to_string_pretty(&pack()).expect("json");
    fs::write(dir.path().join("domain.pack.json"), text).expect("write pack");
    let lib = dir.path().to_str().expect("utf8");

    let output = run(&["content", "list", "--kind", "items", "--lib", lib]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = stdout(&output);
    assert!(text.contains("item\titem-1"), "{text}");

    let output = run(&["content", "show", "rule-1", "--lib", lib]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let shown: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(shown["kind"], "rule-section");
    assert_eq!(shown["item"]["id"], "rule-1");
}
