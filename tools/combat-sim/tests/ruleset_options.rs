//! TASK-0107: four player-visible rulesets and fail-closed full packs.
//!
//! Every fixture is synthetic: no real book text, no PDF and no save contents.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use combat_sim::content::pack::{ContentPack, Distribution, PackMeta, SCHEMA_VERSION};
use combat_sim::content::schema::ContentFile;
use combat_sim::content::{ContentRegistry, ContentSources};

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
            "combat-sim-ruleset-{label}-{}-{nanos}-{count}",
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

fn creature(id: &str, name: &str, ac: i32) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "ac": ac,
        "hp": 5,
        "abilities": {"str": 10, "dex": 10, "con": 10, "int": 10, "wis": 10, "cha": 10}
    })
}

fn write_base(root: &Path) {
    let lib = root.join("lib");
    fs::create_dir_all(lib.join("common")).unwrap();
    fs::write(
        lib.join("common/base.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("shared", "SRD shared", 10)]
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_pack(root: &Path, id: &str, edition: &str, distribution: Distribution) {
    write_pack_content(
        root,
        id,
        edition,
        distribution,
        json!({"creatures": [creature("shared", "Full shared", 12)]}),
    );
}

fn write_pack_content(
    root: &Path,
    id: &str,
    edition: &str,
    distribution: Distribution,
    content: serde_json::Value,
) {
    let content: ContentFile = serde_json::from_value(content).unwrap();
    let meta = PackMeta {
        id: id.to_string(),
        name: Some("Synthetic pack".to_string()),
        edition: edition.to_string(),
        source: Some("synthetic".to_string()),
        license: Some("test".to_string()),
        distribution,
        schema_version: SCHEMA_VERSION,
        content_hash: None,
        created_at: None,
        description: None,
    };
    let pack = ContentPack::new(meta, content).unwrap();
    let dir = root.join("packs");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{id}.pack.json")),
        serde_json::to_string_pretty(&pack).unwrap(),
    )
    .unwrap();
}

fn write_manifest(root: &Path, ruleset: &str, library: &[&str]) -> PathBuf {
    let save = root.join("save");
    fs::create_dir_all(&save).unwrap();
    let list = library
        .iter()
        .map(|entry| format!("\"{entry}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = save.join("campaign.yaml");
    fs::write(
        &manifest,
        format!("name: synthetic\nruleset: \"{ruleset}\"\nlibrary: [{list}]\n"),
    )
    .unwrap();
    manifest
}

fn load(manifest: &Path) -> anyhow::Result<ContentRegistry> {
    ContentSources {
        campaign: Some(manifest.to_path_buf()),
        ..ContentSources::default()
    }
    .load()
}

#[test]
fn four_values_are_accepted_and_legacy_values_map_to_srd() {
    for (ruleset, want_edition, want_packs) in [
        ("5e-srd", "2014", false),
        ("  5E-SRD  ", "2014", false),
        ("5.5e-srd", "2024", false),
        ("5.5e-SRD", "2024", false),
        ("2014", "2014", false),
        ("2024", "2024", false),
    ] {
        let root = TempDir::new("values");
        write_base(root.path());
        let manifest = write_manifest(root.path(), ruleset, &["../lib"]);
        let registry = load(&manifest).unwrap_or_else(|e| panic!("{ruleset}: {e:#}"));
        assert_eq!(registry.ruleset, want_edition, "{ruleset}");
        assert_eq!(registry.packs.is_empty(), !want_packs, "{ruleset}");
    }
}

#[test]
fn full_values_load_their_edition_pack() {
    for (ruleset, edition, pack_id) in [
        ("5e-full", "2014", "full-2014-phb"),
        ("  5.5E-FULL  ", "2024", "full-2024-phb"),
    ] {
        let root = TempDir::new("full-values");
        write_base(root.path());
        write_pack(root.path(), pack_id, edition, Distribution::Full);
        let manifest = write_manifest(
            root.path(),
            ruleset,
            &["../lib", &format!("../packs/{pack_id}.pack.json")],
        );
        let registry = load(&manifest).unwrap_or_else(|e| panic!("{ruleset}: {e:#}"));
        assert_eq!(registry.ruleset, edition, "{ruleset}");
        assert!(registry.packs.contains_key(pack_id), "{ruleset}");
        assert_eq!(
            registry.creatures.get("shared").unwrap().name.as_deref(),
            Some("Full shared"),
            "{ruleset}"
        );
    }
}

#[test]
fn illegal_values_name_every_accepted_value_and_the_legacy_alias() {
    let root = TempDir::new("illegal");
    write_base(root.path());
    let manifest = write_manifest(root.path(), "5e", &["../lib"]);
    let error = load(&manifest).expect_err("5e is not a ruleset value");
    let text = format!("{error:#}");
    for needle in [
        "unsupported ruleset",
        "5e-srd",
        "5e-full",
        "5.5e-srd",
        "5.5e-full",
        "2014",
        "2024",
    ] {
        assert!(text.contains(needle), "missing {needle} in {text}");
    }
}

#[test]
fn full_without_an_unlocked_pack_fails_and_names_the_book_and_fix() {
    let root = TempDir::new("full-missing");
    write_base(root.path());
    let manifest = write_manifest(root.path(), "5e-full", &["../lib"]);
    let error = load(&manifest).expect_err("5e-full without a full pack must fail");
    let text = format!("{error:#}");
    for needle in [
        "5e FULL",
        "Player's Handbook",
        "Dungeon Master's Guide",
        "Monster Manual",
        "Rules/2014",
        "talkdnd library scan",
        "5e SRD",
    ] {
        assert!(text.contains(needle), "missing {needle} in {text}");
    }
}

#[test]
fn full_layers_srd_then_full_pack_then_adventure_override() {
    let root = TempDir::new("layers");
    write_base(root.path());
    write_pack(root.path(), "full-2014-phb", "2014", Distribution::Full);
    let adventure = root.path().join("adventure");
    fs::create_dir_all(&adventure).unwrap();
    fs::write(
        adventure.join("override.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("shared", "Adventure shared", 14)]
        }))
        .unwrap(),
    )
    .unwrap();

    let srd = write_manifest(
        root.path(),
        "5e-srd",
        &["../lib", "../packs/full-2014-phb.pack.json"],
    );
    let registry = load(&srd).unwrap();
    assert_eq!(
        registry.creatures.get("shared").unwrap().name.as_deref(),
        Some("SRD shared")
    );
    assert!(registry.packs.is_empty(), "*-srd must not load full packs");

    let manifest = {
        let save = root.path().join("save");
        fs::write(
            save.join("campaign.yaml"),
            "name: synthetic\nruleset: \"5e-full\"\nlibrary: [\"../lib\", \"../packs/full-2014-phb.pack.json\"]\nadventures: [\"../adventure\"]\n",
        )
        .unwrap();
        save.join("campaign.yaml")
    };
    let registry = load(&manifest).unwrap();
    assert_eq!(
        registry.creatures.get("shared").unwrap().name.as_deref(),
        Some("Adventure shared"),
        "hand-written adventure content must win over the full pack"
    );
    assert!(registry
        .creatures
        .source("shared")
        .unwrap()
        .path
        .ends_with("adventure/override.yaml"));
}

#[test]
fn full_pack_wins_over_srd_for_the_same_id_when_no_later_override_exists() {
    let root = TempDir::new("full-wins");
    write_base(root.path());
    write_pack(root.path(), "full-2014-phb", "2014", Distribution::Full);
    let manifest = write_manifest(
        root.path(),
        "5e-full",
        &["../lib", "../packs/full-2014-phb.pack.json"],
    );
    let registry = load(&manifest).unwrap();
    assert_eq!(
        registry.creatures.get("shared").unwrap().name.as_deref(),
        Some("Full shared")
    );
    assert!(registry.packs.contains_key("full-2014-phb"));
}

#[test]
fn a_full_save_never_loads_the_other_edition_pack() {
    let root = TempDir::new("mix");
    write_base(root.path());
    write_pack(root.path(), "full-2024-phb", "2024", Distribution::Full);
    let manifest = write_manifest(
        root.path(),
        "5e-full",
        &["../lib", "../packs/full-2024-phb.pack.json"],
    );
    let error = load(&manifest).expect_err("one save must not mix editions");
    let text = format!("{error:#}");
    assert!(text.contains("full-2024-phb"), "{text}");
    assert!(text.contains("2014"), "{text}");
    assert!(
        text.contains("5e-full") || text.contains("5e FULL"),
        "{text}"
    );
}

#[test]
fn real_default_rules_combat_layout_puts_handwritten_after_the_full_pack() {
    let root = TempDir::new("real-layout");
    let combat = root.path().join("Rules/combat");
    for dir in ["common", "2014/srd", "2014/monsters"] {
        fs::create_dir_all(combat.join(dir)).unwrap();
    }
    fs::write(
        combat.join("common/creatures.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [
                creature("shared", "SRD shared", 10),
                creature("srd-id", "SRD srd-id", 10),
                creature("common-only", "Common only", 10)
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        combat.join("2014/srd/srd-base.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("srd-only", "SRD only", 10)]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        combat.join("2014/monsters/hand.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("shared", "Handwritten shared", 13)]
        }))
        .unwrap(),
    )
    .unwrap();
    write_pack_content(
        root.path(),
        "full-2014-phb",
        "2014",
        Distribution::Full,
        json!({
            "creatures": [
                creature("shared", "Full shared", 12),
                creature("srd-id", "Full srd-id", 12)
            ]
        }),
    );
    fs::create_dir_all(root.path().join("Rules/packs")).unwrap();
    fs::rename(
        root.path().join("packs/full-2014-phb.pack.json"),
        root.path().join("Rules/packs/full-2014-phb.pack.json"),
    )
    .unwrap();

    let save = root.path().join("Saves/2030-01-01-01/combat");
    fs::create_dir_all(&save).unwrap();
    let manifest = save.join("campaign.yaml");
    fs::write(
        &manifest,
        "name: synthetic\nruleset: \"5e-full\"\nlibrary: [\"../../../Rules/combat\", \"../../../Rules/packs/full-2014-phb.pack.json\"]\n",
    )
    .unwrap();
    let registry = load(&manifest).unwrap();
    assert_eq!(
        registry.creatures.get("shared").unwrap().name.as_deref(),
        Some("Handwritten shared"),
        "2014/monsters/ must load after the full pack"
    );
    assert!(registry
        .creatures
        .source("shared")
        .unwrap()
        .path
        .ends_with("2014/monsters/hand.yaml"));
    assert_eq!(
        registry.creatures.get("srd-id").unwrap().name.as_deref(),
        Some("Full srd-id"),
        "the full pack must override a pure SRD id"
    );
    assert!(registry.creatures.get("srd-only").is_some());
    assert!(registry.creatures.get("common-only").is_some());

    // `*-srd` keeps the old whole-layer order and never loads the full pack.
    let srd_manifest = save.join("campaign-srd.yaml");
    fs::write(
        &srd_manifest,
        "name: synthetic\nruleset: \"5e-srd\"\nlibrary: [\"../../../Rules/combat\", \"../../../Rules/packs/full-2014-phb.pack.json\"]\n",
    )
    .unwrap();
    let registry = load(&srd_manifest).unwrap();
    assert_eq!(
        registry.creatures.get("shared").unwrap().name.as_deref(),
        Some("Handwritten shared")
    );
    assert_eq!(
        registry.creatures.get("srd-id").unwrap().name.as_deref(),
        Some("SRD srd-id")
    );
    assert!(!registry.packs.contains_key("full-2014-phb"));
}

#[test]
fn srd_overlay_under_monsters_loads_before_the_full_pack() {
    let root = TempDir::new("srd-overlay");
    let combat = root.path().join("Rules/combat");
    for dir in ["common", "2024/srd", "2024/monsters"] {
        fs::create_dir_all(combat.join(dir)).unwrap();
    }
    fs::write(
        combat.join("common/creatures.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("overlay-id", "SRD overlay", 10)]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        combat.join("2024/monsters/srd-spatial.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("overlay-id", "SRD overlay", 10)]
        }))
        .unwrap(),
    )
    .unwrap();
    write_pack_content(
        root.path(),
        "full-2024-phb",
        "2024",
        Distribution::Full,
        json!({
            "creatures": [creature("overlay-id", "Full overlay", 12)]
        }),
    );
    fs::create_dir_all(root.path().join("Rules/packs")).unwrap();
    fs::rename(
        root.path().join("packs/full-2024-phb.pack.json"),
        root.path().join("Rules/packs/full-2024-phb.pack.json"),
    )
    .unwrap();

    let save = root.path().join("Saves/2030-01-01-01/combat");
    fs::create_dir_all(&save).unwrap();
    let manifest = save.join("campaign.yaml");
    fs::write(
        &manifest,
        "name: synthetic\nruleset: \"5.5e-full\"\nlibrary: [\"../../../Rules/combat\", \"../../../Rules/packs/full-2024-phb.pack.json\"]\n",
    )
    .unwrap();
    let registry = load(&manifest).unwrap();
    assert_eq!(
        registry
            .creatures
            .get("overlay-id")
            .unwrap()
            .name
            .as_deref(),
        Some("Full overlay"),
        "2024/monsters/srd-* is SRD base and must load before the full pack"
    );
    assert!(registry
        .creatures
        .source("overlay-id")
        .unwrap()
        .path
        .ends_with("Rules/packs/full-2024-phb.pack.json"));
}

#[test]
fn handwritten_library_overrides_a_full_pack() {
    let root = TempDir::new("handwritten");
    write_base(root.path());
    write_pack(root.path(), "full-2014-phb", "2014", Distribution::Full);
    let hand = root.path().join("hand");
    fs::create_dir_all(&hand).unwrap();
    fs::write(
        hand.join("override.yaml"),
        serde_json::to_string_pretty(&json!({
            "creatures": [creature("shared", "Handwritten shared", 13)]
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest = write_manifest(
        root.path(),
        "5e-full",
        &["../lib", "../packs/full-2014-phb.pack.json", "../hand"],
    );
    let registry = load(&manifest).unwrap();
    assert_eq!(
        registry.creatures.get("shared").unwrap().name.as_deref(),
        Some("Handwritten shared"),
        "hand-written library content must load after the full pack"
    );
}

#[test]
fn an_invalid_full_pack_fails_closed() {
    let root = TempDir::new("invalid-pack");
    write_base(root.path());
    write_pack(root.path(), "full-2014-phb", "2014", Distribution::Full);
    let path = root.path().join("packs/full-2014-phb.pack.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    value["pack"]["content_hash"] = json!("fnv1a64:0000000000000000:0");
    fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
    let manifest = write_manifest(
        root.path(),
        "5e-full",
        &["../lib", "../packs/full-2014-phb.pack.json"],
    );
    let error = load(&manifest).expect_err("a corrupt full pack must fail closed");
    let text = format!("{error:#}");
    assert!(text.contains("content hash mismatch"), "{text}");
    assert!(text.contains("full-2014-phb"), "{text}");
}

#[test]
fn content_check_cli_fails_for_full_without_a_full_pack() {
    let root = TempDir::new("cli-missing");
    write_base(root.path());
    let manifest = write_manifest(root.path(), "5.5e-full", &["../lib"]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_combat-sim"))
        .arg("--campaign")
        .arg(&manifest)
        .args(["content", "check"])
        .output()
        .expect("run combat-sim");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("5.5e FULL"), "{text}");
    assert!(text.contains("Player's Handbook"), "{text}");
    assert!(text.contains("Rules/2024"), "{text}");
    assert!(text.contains("5.5e SRD"), "{text}");
}
