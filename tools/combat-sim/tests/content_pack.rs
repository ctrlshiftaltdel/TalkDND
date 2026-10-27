//! ContentPack v1 loading, validation and CLI tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use combat_sim::content::pack::{
    content_hash, ContentPack, Distribution, PackEntry, PackIndex, PackMeta, SCHEMA_VERSION,
};
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
            "combat-sim-pack-{label}-{}-{nanos}-{count}",
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
        "conditions": [{"id": "packed", "name": "Packed"}]
    }))
    .expect("condition content")
}

fn meta(id: &str) -> PackMeta {
    PackMeta {
        id: id.to_string(),
        name: Some("Test Pack".to_string()),
        edition: "2024".to_string(),
        source: Some("synthetic".to_string()),
        license: Some("CC-BY-4.0".to_string()),
        distribution: Distribution::Srd,
        schema_version: SCHEMA_VERSION,
        content_hash: None,
        created_at: None,
        description: Some("synthetic pack".to_string()),
    }
}

fn pack(id: &str) -> ContentPack {
    ContentPack::new(meta(id), content()).expect("pack")
}

fn write_json(dir: &Path, name: &str, value: &Value) {
    let text = serde_json::to_string_pretty(value).expect("json");
    fs::write(dir.join(name), text).expect("write pack");
}

fn write_pack(dir: &Path, name: &str, pack: &ContentPack) {
    write_json(dir, name, &serde_json::to_value(pack).expect("pack json"));
}

fn load(dir: &Path) -> anyhow::Result<combat_sim::content::ContentRegistry> {
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

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn loads_pack_and_records_metadata() {
    let dir = TempDir::new("load");
    let pack = pack("test-pack");
    write_pack(dir.path(), "test.pack.json", &pack);

    let registry = load(dir.path()).expect("pack loads");
    assert_eq!(registry.packs.len(), 1);
    let loaded = registry.packs.get("test-pack").expect("metadata");
    assert_eq!(loaded.edition, "2024");
    assert_eq!(loaded.distribution, Distribution::Srd);
    assert_eq!(loaded.content_hash, pack.pack.content_hash);
    assert!(registry.conditions.get("packed").is_some());
    let source = registry
        .source_of(ItemKind::Condition, "packed")
        .expect("source");
    assert!(source.path.ends_with("test.pack.json"));
}

#[test]
fn loads_yaml_pack() {
    let dir = TempDir::new("yaml-pack");
    let pack = pack("test-pack");
    let text = serde_norway::to_string(&pack).expect("yaml pack");
    fs::write(dir.path().join("test.pack.yaml"), text).expect("write yaml pack");

    let registry = load(dir.path()).expect("yaml pack loads");
    assert_eq!(registry.packs.len(), 1);
    assert!(registry.conditions.get("packed").is_some());
}

#[test]
fn old_registry_without_packs_loads() {
    let registry: ContentRegistry =
        serde_json::from_value(json!({"ruleset": "2024"})).expect("old registry");
    assert!(registry.packs.is_empty());
}

#[test]
fn hash_mismatch_fails_closed() {
    let dir = TempDir::new("hash");
    let mut value = serde_json::to_value(pack("test-pack")).expect("json");
    value["pack"]["content_hash"] = json!("fnv1a64:0000000000000000:0");
    write_json(dir.path(), "bad.pack.json", &value);

    let error = load(dir.path()).expect_err("hash mismatch must fail");
    let text = format!("{error:#}");
    assert!(text.contains("content hash mismatch"), "{text}");
    assert!(text.contains("test-pack"), "{text}");
}

#[test]
fn unknown_schema_fails_closed() {
    let dir = TempDir::new("schema");
    let mut value = serde_json::to_value(pack("test-pack")).expect("json");
    value["pack"]["schema_version"] = json!(99);
    write_json(dir.path(), "future.pack.json", &value);

    let error = load(dir.path()).expect_err("unknown schema must fail");
    let text = format!("{error:#}");
    assert!(text.contains("schema_version"), "{text}");
    assert!(text.contains("test-pack"), "{text}");
}

#[test]
fn duplicate_pack_id_fails_closed() {
    let dir = TempDir::new("duplicate-pack");
    write_pack(dir.path(), "a.pack.json", &pack("test-pack"));
    write_pack(dir.path(), "b.pack.json", &pack("test-pack"));

    let error = load(dir.path()).expect_err("duplicate pack id must fail");
    assert!(
        format!("{error:#}").contains("duplicate content pack id"),
        "{error:#}"
    );
}

#[test]
fn duplicate_entity_id_fails_closed() {
    let content: ContentFile = serde_json::from_value(json!({
        "conditions": [
            {"id": "same", "name": "One"},
            {"id": "same", "name": "Two"}
        ]
    }))
    .expect("content");
    let error = ContentPack::new(meta("dup-entity"), content).expect_err("duplicate entity");
    assert!(
        format!("{error:#}").contains("duplicate entity"),
        "{error:#}"
    );
}

#[test]
fn dangling_index_entry_fails_closed() {
    let dir = TempDir::new("index");
    let mut value = serde_json::to_value(pack("test-pack")).expect("json");
    value["index"] = serde_json::to_value(PackIndex {
        entries: vec![PackEntry {
            id: "missing".to_string(),
            kind: "condition".to_string(),
            ..PackEntry::default()
        }],
        images: vec![],
    })
    .expect("index");
    write_json(dir.path(), "bad-index.pack.json", &value);

    let error = load(dir.path()).expect_err("dangling index must fail");
    assert!(
        format!("{error:#}").contains("references missing"),
        "{error:#}"
    );
}

#[test]
fn pack_hash_round_trips() {
    let first = pack("test-pack");
    let second = pack("test-pack");
    assert_eq!(first.pack.content_hash, second.pack.content_hash);

    let value = serde_json::to_value(&first).expect("json");
    let parsed = ContentPack::from_value(&value).expect("round trip");
    assert_eq!(parsed.pack.content_hash, first.pack.content_hash);
    assert_eq!(
        content_hash(&parsed.content).unwrap(),
        parsed.pack.content_hash.unwrap()
    );
}

#[test]
fn existing_yaml_content_still_loads() {
    let dir = TempDir::new("yaml");
    fs::write(
        dir.path().join("core.yaml"),
        "conditions:\n  - id: yaml-condition\n    name: YAML\n",
    )
    .expect("write yaml");

    let registry = load(dir.path()).expect("yaml loads");
    assert!(registry.conditions.get("yaml-condition").is_some());
    assert!(registry.packs.is_empty());
}

#[test]
fn pack_cli_list_show_verify() {
    let dir = TempDir::new("cli");
    write_pack(dir.path(), "test.pack.json", &pack("test-pack"));
    let lib = dir.path().to_str().expect("utf8");

    let list = run(&["content", "pack", "list", "--lib", lib]);
    assert!(list.status.success(), "{}", stderr(&list));
    let text = stdout(&list);
    assert!(text.contains("test-pack"), "{text}");
    assert!(text.contains("srd"), "{text}");

    let show = run(&["content", "pack", "show", "test-pack", "--lib", lib]);
    assert!(show.status.success(), "{}", stderr(&show));
    let shown: Value = serde_json::from_slice(&show.stdout).expect("json");
    assert_eq!(shown["id"], "test-pack");
    assert_eq!(shown["edition"], "2024");

    let verify = run(&["content", "pack", "verify", "--lib", lib]);
    assert!(verify.status.success(), "{}", stderr(&verify));
    let text = stdout(&verify);
    assert!(text.contains("test-pack: verified"), "{text}");
    assert!(text.contains("content pack verify: 0 error(s)"), "{text}");
}
