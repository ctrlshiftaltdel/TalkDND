//! Full-book pack builder and release packaging: synthetic pdf2md fixtures only.
//!
//! CI has no books. Every fixture here is invented text laid out like a pdf2md
//! `index/` directory; no test reads or writes a real book.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use combat_sim::content::full_pack::{self, FULL_LICENSE};
use combat_sim::content::pack::{ContentPack, Distribution, PackMeta, SCHEMA_VERSION};
use combat_sim::content::schema::ContentFile;
use combat_sim::content::{ContentSources, ItemKind};

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
            "combat-sim-full-{label}-{}-{nanos}-{count}",
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
## Chapter 1: The First Job

Opening text for the first chapter.

### The Locked Door

The door is locked. See the [guard post](Test Book.pages.md#p2).

<a id="p2"></a>
<!-- PDF p.2 -->
### The Guard Post

The guard keeps a key. See [chapter two](Test Book.pages.md#p3).

<a id="p3"></a>
<!-- PDF p.3 -->
## Chapter 2: The Second Job

Second chapter text.

<a id="p4"></a>
<!-- PDF p.4 -->
Body without a heading stays out of the section index.
"#;

fn write_fixture(dir: &Path) -> PathBuf {
    let index = dir.join("index");
    fs::create_dir_all(&index).expect("index dir");
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
        .expect("manifest json"),
    )
    .expect("manifest");
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
            "figures": [{"img": "img/p0001-a1.webp", "page": 1}],
            "art": [{"img": "img/p0002.webp", "page": 2}],
            "low_quality_pages": [2],
            "statblocks": [2]
        }))
        .expect("index json"),
    )
    .expect("index.json");
    fs::write(index.join("Test Book.pages.md"), PAGES).expect("pages");
    index
}

fn section<'a>(pack: &'a ContentPack, name: &str) -> &'a combat_sim::content::schema::DomainEntity {
    pack.content
        .rule_sections
        .as_ref()
        .expect("rule sections")
        .iter()
        .find(|s| s.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("section {name} not found"))
}

fn read_pack(path: &Path) -> ContentPack {
    let text = fs::read_to_string(path).expect("read pack");
    let value: Value = serde_json::from_str(&text).expect("pack json");
    ContentPack::from_value(&value).expect("valid pack")
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
fn builds_full_pack_with_metadata_sections_refs_and_images() {
    let dir = TempDir::new("build");
    let index = write_fixture(dir.path());
    let (pack, report) = full_pack::build_pack("2014", "phb", &index).expect("full pack builds");

    assert_eq!(pack.pack.id, "full-2014-phb");
    assert_eq!(pack.pack.edition, "2014");
    assert_eq!(pack.pack.distribution, Distribution::Full);
    assert_eq!(pack.pack.schema_version, SCHEMA_VERSION);
    assert_eq!(pack.pack.created_at.as_deref(), Some("2026-01-02T03:04:05"));
    assert_eq!(pack.pack.license.as_deref(), Some(FULL_LICENSE));
    assert!(
        pack.pack
            .source
            .as_deref()
            .unwrap_or("")
            .contains("sha256 abcdef012345"),
        "{:?}",
        pack.pack.source
    );
    assert!(
        pack.pack
            .name
            .as_deref()
            .unwrap_or("")
            .contains("Player's Handbook"),
        "{:?}",
        pack.pack.name
    );
    assert!(pack.pack.content_hash.is_some());
    assert!(pack
        .pack
        .description
        .as_deref()
        .unwrap_or("")
        .contains("4 sections"));
    pack.validate().expect("pack validates");

    let sections = pack.content.rule_sections.as_ref().expect("sections");
    assert_eq!(sections.len(), 4, "{sections:#?}");
    assert_eq!(report.sections, 4);
    assert_eq!(report.chapters, 2);
    assert_eq!(report.images, 2);
    assert_eq!(report.pages, 4);
    assert_eq!(report.low_quality_pages, 1);
    assert_eq!(report.statblock_pages, 1);
    assert_eq!(report.skipped_headings, 0);

    let locked = section(&pack, "The Locked Door");
    assert_eq!(locked.page, Some(1));
    assert_eq!(locked.section.as_deref(), Some("Chapter 1: The First Job"));
    assert!(locked
        .text
        .as_deref()
        .unwrap_or("")
        .contains("The door is locked."));
    let data = locked.data.as_ref().expect("structured data");
    assert_eq!(data["book"], "phb");
    assert_eq!(data["edition"], "2014");
    assert_eq!(data["chapter"], "Chapter 1: The First Job");
    assert_eq!(data["level"], 3);
    assert_eq!(data["page"], 1);
    assert_eq!(data["anchor"], "p1");

    let guard = section(&pack, "The Guard Post");
    assert!(
        locked.refs.iter().any(|r| r == &guard.id),
        "locked refs {:?}, guard id {}",
        locked.refs,
        guard.id
    );
    let chapter_two = section(&pack, "Chapter 2: The Second Job");
    assert!(
        guard.refs.iter().any(|r| r == &chapter_two.id),
        "guard refs {:?}",
        guard.refs
    );

    assert_eq!(pack.index.entries.len(), 4);
    let entry = pack
        .index
        .entries
        .iter()
        .find(|e| e.id == locked.id)
        .expect("index entry");
    assert_eq!(entry.kind, "rule-section");
    assert_eq!(entry.page, Some(1));
    assert_eq!(entry.section.as_deref(), Some("Chapter 1: The First Job"));
    assert!(entry.refs.contains(&guard.id));

    assert_eq!(pack.index.images.len(), 2);
    for image in &pack.index.images {
        let path = image.path.as_deref().expect("relative image path");
        assert!(!path.starts_with('/') && !path.contains(".."), "{path}");
        assert_eq!(image.section.as_deref(), Some("Chapter 1: The First Job"));
    }

    assert!(
        report
            .gaps
            .iter()
            .any(|g| g.contains("1 page(s) not converted")),
        "{:?}",
        report.gaps
    );
    assert!(
        report.gaps.iter().any(|g| g.contains("1 low-quality page")),
        "{:?}",
        report.gaps
    );
    assert!(
        report.gaps.iter().any(|g| g.contains("1 stat-block page")),
        "{:?}",
        report.gaps
    );
    let debug = format!("{report:?}");
    assert!(!debug.contains("The door is locked."), "{debug}");
    assert!(!debug.contains("Test Book.pages.md"), "{debug}");
}

#[test]
fn build_is_deterministic_and_dry_run_writes_nothing() {
    let dir = TempDir::new("deterministic");
    let index = write_fixture(dir.path());
    let (first, _) = full_pack::build_pack("2014", "phb", &index).expect("first");
    let (second, _) = full_pack::build_pack("2014", "phb", &index).expect("second");
    assert_eq!(first, second);

    let out = dir.path().join("out");
    let report = full_pack::build("2014", "phb", &index, &out, true).expect("dry run");
    assert_eq!(report.sections, 4);
    assert!(!out.exists(), "dry run created {}", out.display());

    let report = full_pack::build("2014", "phb", &index, &out, false).expect("write");
    assert_eq!(report.file, "full-2014-phb.pack.json");
    let written = out.join(&report.file);
    assert!(written.is_file());
    let pack = read_pack(&written);
    assert_eq!(pack, first);
    let raw = fs::read_to_string(&written).expect("pack text");
    assert!(
        !raw.contains(index.to_str().expect("utf8")),
        "pack embedded the local index path"
    );
}

#[test]
fn missing_sources_and_unknown_selections_fail_closed() {
    let dir = TempDir::new("errors");
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).expect("empty dir");
    let out = dir.path().join("out");

    let error = full_pack::build("2014", "phb", &empty, &out, true).expect_err("no manifest");
    let text = format!("{error:#}");
    assert!(text.contains("manifest.json"), "{text}");
    assert!(!out.exists());

    let index = write_fixture(dir.path());
    let error = full_pack::build("2020", "phb", &index, &out, true).expect_err("bad edition");
    assert!(format!("{error:#}").contains("2014 or 2024"), "{error:#}");

    let error = full_pack::build("2014", "phb2", &index, &out, true).expect_err("bad book");
    assert!(format!("{error:#}").contains("phb"), "{error:#}");

    fs::write(index.join("index.json"), "{").expect("corrupt index");
    let error = full_pack::build("2014", "phb", &index, &out, true).expect_err("bad index");
    assert!(format!("{error:#}").contains("index.json"), "{error:#}");
}

#[test]
fn unsafe_image_paths_are_skipped_and_reported() {
    let dir = TempDir::new("images");
    let index = write_fixture(dir.path());
    fs::write(
        index.join("index.json"),
        serde_json::to_string_pretty(&json!({
            "page_count": 4,
            "done_pages": 4,
            "headings": [],
            "chapters": [{"title": "Chapter 1: The First Job", "page": 1}],
            "figures": [
                {"img": "img/p0001-a1.webp", "page": 1},
                {"img": "../../outside.webp", "page": 1},
                {"img": "/tmp/personal/secret.webp", "page": 2}
            ],
            "art": [],
            "low_quality_pages": [],
            "statblocks": []
        }))
        .expect("index json"),
    )
    .expect("index");

    let (pack, report) = full_pack::build_pack("2014", "phb", &index).expect("build");
    assert_eq!(pack.index.images.len(), 1);
    assert_eq!(report.unsafe_images, 2);
    assert!(
        report
            .gaps
            .iter()
            .any(|g| g.contains("2 image") && g.contains("relative")),
        "{:?}",
        report.gaps
    );
    let raw = serde_json::to_string(&pack).expect("pack json");
    assert!(!raw.contains("outside"), "{raw}");
    assert!(!raw.contains("secret.webp"), "{raw}");
}

#[test]
fn release_manifest_is_sorted_hashed_and_never_embeds_local_paths() {
    let dir = TempDir::new("release");
    let index = write_fixture(dir.path());
    let packs = dir.path().join("packs");
    full_pack::build("2014", "phb", &index, &packs, false).expect("phb");
    full_pack::build("2024", "mm", &index, &packs, false).expect("mm");

    let planned = full_pack::release(&packs, None).expect("dry release");
    let ids: Vec<&str> = planned.packs.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["full-2014-phb", "full-2024-mm"]);
    assert!(planned.total_bytes > 0);

    let out = dir.path().join("release-out");
    let written = full_pack::release(&packs, Some(&out)).expect("release");
    assert_eq!(written.packs.len(), 2);
    let manifest_path = out.join("full-packs-manifest.json");
    let text = fs::read_to_string(&manifest_path).expect("manifest");
    let value: Value = serde_json::from_str(&text).expect("manifest json");
    assert_eq!(value["schema"], 1);
    assert_eq!(value["packs"].as_array().expect("packs").len(), 2);
    assert_eq!(value["packs"][0]["id"], "full-2014-phb");
    assert_eq!(value["packs"][1]["id"], "full-2024-mm");
    assert_eq!(value["packs"][0]["sections"], 4);
    assert_eq!(value["packs"][0]["chapters"], 2);
    assert_eq!(value["packs"][0]["images"], 2);
    assert!(value["packs"][0]["content_hash"]
        .as_str()
        .unwrap_or("")
        .starts_with("fnv1a64:"));
    assert_eq!(
        value["total_bytes"].as_u64().unwrap_or(0),
        written.total_bytes
    );
    for pack in value["packs"].as_array().expect("packs") {
        let file = pack["file"].as_str().expect("file");
        assert!(out.join(file).is_file(), "missing copied {file}");
    }
    assert!(
        !text.contains(dir.path().to_str().expect("utf8")),
        "release manifest embedded a local path"
    );
    let copied = read_pack(&out.join("full-2014-phb.pack.json"));
    assert_eq!(copied.pack.distribution, Distribution::Full);
}

#[test]
fn release_rejects_empty_dirs_srd_packs_and_duplicate_ids() {
    let dir = TempDir::new("release-errors");
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).expect("empty");
    let error = full_pack::release(&empty, None).expect_err("empty release");
    assert!(format!("{error:#}").contains("no full packs"), "{error:#}");

    let srd_dir = dir.path().join("srd");
    fs::create_dir_all(&srd_dir).expect("srd dir");
    let content: ContentFile = serde_json::from_value(json!({
        "conditions": [{"id": "packed", "name": "Packed"}]
    }))
    .expect("content");
    let srd = ContentPack::new(
        PackMeta {
            id: "srd-pack".to_string(),
            name: Some("SRD".to_string()),
            edition: "2014".to_string(),
            source: Some("synthetic".to_string()),
            license: Some("CC-BY-4.0".to_string()),
            distribution: Distribution::Srd,
            schema_version: SCHEMA_VERSION,
            content_hash: None,
            created_at: None,
            description: None,
        },
        content,
    )
    .expect("srd pack");
    fs::write(
        srd_dir.join("srd.pack.json"),
        serde_json::to_string(&srd).expect("json"),
    )
    .expect("write srd");
    let error = full_pack::release(&srd_dir, None).expect_err("srd release");
    assert!(
        format!("{error:#}").contains("not a full pack"),
        "{error:#}"
    );

    let duplicate = dir.path().join("duplicate");
    fs::create_dir_all(&duplicate).expect("duplicate dir");
    let index = write_fixture(dir.path());
    full_pack::build("2014", "phb", &index, &duplicate, false).expect("build");
    fs::copy(
        duplicate.join("full-2014-phb.pack.json"),
        duplicate.join("copy.pack.json"),
    )
    .expect("copy");
    let error = full_pack::release(&duplicate, None).expect_err("duplicate release");
    assert!(
        format!("{error:#}").contains("duplicate full pack id"),
        "{error:#}"
    );
}

#[test]
fn cli_builds_a_full_pack_that_list_show_and_verify_accept() {
    let dir = TempDir::new("cli");
    let index = write_fixture(dir.path());
    let out = dir.path().join("out");
    let index_arg = index.to_str().expect("utf8");
    let out_arg = out.to_str().expect("utf8");

    let build = run(&[
        "content",
        "pack",
        "build",
        "--edition",
        "2014",
        "--book",
        "phb",
        "--index",
        index_arg,
        "--out",
        out_arg,
    ]);
    assert!(build.status.success(), "{}", stderr(&build));
    let text = stdout(&build);
    assert!(text.contains("full-2014-phb"), "{text}");
    assert!(text.contains("4 sections"), "{text}");
    assert!(text.contains("2 images"), "{text}");
    assert!(!text.contains("The door is locked."), "{text}");

    let list = run(&["content", "pack", "list", "--lib", out_arg]);
    assert!(list.status.success(), "{}", stderr(&list));
    assert!(stdout(&list).contains("full-2014-phb"), "{}", stdout(&list));

    let show = run(&["content", "pack", "show", "full-2014-phb", "--lib", out_arg]);
    assert!(show.status.success(), "{}", stderr(&show));
    let shown: Value = serde_json::from_slice(&show.stdout).expect("show json");
    assert_eq!(shown["distribution"], "full");

    let verify = run(&["content", "pack", "verify", "--lib", out_arg]);
    assert!(verify.status.success(), "{}", stderr(&verify));
    assert!(
        stdout(&verify).contains("full-2014-phb: verified"),
        "{}",
        stdout(&verify)
    );
    assert!(
        stdout(&verify).contains("content pack verify: 0 error(s)"),
        "{}",
        stdout(&verify)
    );

    let dry_out = dir.path().join("dry-out");
    let dry_out_arg = dry_out.to_str().expect("utf8");
    let dry = run(&[
        "content",
        "pack",
        "build",
        "--edition",
        "2014",
        "--book",
        "phb",
        "--index",
        index_arg,
        "--out",
        dry_out_arg,
        "--dry-run",
    ]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(!dry_out.exists(), "CLI dry run wrote output");
}

#[test]
fn cli_release_dry_run_and_staging() {
    let dir = TempDir::new("cli-release");
    let index = write_fixture(dir.path());
    let packs = dir.path().join("packs");
    full_pack::build("2014", "phb", &index, &packs, false).expect("build");
    let out = dir.path().join("staged");
    let packs_arg = packs.to_str().expect("utf8");
    let out_arg = out.to_str().expect("utf8");

    let dry = run(&[
        "content",
        "pack",
        "release",
        "--packs",
        packs_arg,
        "--out",
        out_arg,
        "--dry-run",
    ]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(!out.exists(), "release dry run wrote output");
    assert!(stdout(&dry).contains("full-2014-phb"), "{}", stdout(&dry));

    let real = run(&[
        "content", "pack", "release", "--packs", packs_arg, "--out", out_arg,
    ]);
    assert!(real.status.success(), "{}", stderr(&real));
    assert!(out.join("full-packs-manifest.json").is_file());
    assert!(out.join("full-2014-phb.pack.json").is_file());
    assert!(stdout(&real).contains("1 full pack"), "{}", stdout(&real));
}

#[test]
fn legacy_yaml_and_srd_packs_still_load_and_override_a_full_pack() {
    let dir = TempDir::new("legacy");
    let index = write_fixture(dir.path());
    let lib = dir.path().join("lib");
    full_pack::build("2014", "phb", &index, &lib, false).expect("full pack");

    let content: ContentFile = serde_json::from_value(json!({
        "conditions": [{"id": "packed", "name": "Old Pack"}]
    }))
    .expect("content");
    let old = ContentPack::new(
        PackMeta {
            id: "old-srd".to_string(),
            name: Some("Old SRD".to_string()),
            edition: "2014".to_string(),
            source: Some("synthetic".to_string()),
            license: Some("CC-BY-4.0".to_string()),
            distribution: Distribution::Srd,
            schema_version: SCHEMA_VERSION,
            content_hash: None,
            created_at: None,
            description: None,
        },
        content,
    )
    .expect("old pack");
    fs::write(
        lib.join("old.pack.json"),
        serde_json::to_string(&old).expect("json"),
    )
    .expect("old pack file");
    fs::write(
        lib.join("core.yaml"),
        "conditions:\n  - id: packed\n    name: YAML Override\n  - id: yaml-only\n    name: YAML Only\n",
    )
    .expect("yaml");

    let registry = ContentSources {
        libraries: vec![lib.clone()],
        ..ContentSources::default()
    }
    .load()
    .expect("library loads");
    assert_eq!(registry.packs.len(), 2);
    assert!(registry.packs.contains_key("full-2014-phb"));
    assert!(registry.packs.contains_key("old-srd"));
    assert_eq!(
        registry
            .conditions
            .get("packed")
            .map(|c| c.name.as_deref().unwrap_or("")),
        Some("YAML Override")
    );
    assert!(registry.conditions.get("yaml-only").is_some());
    let source = registry
        .source_of(ItemKind::Condition, "yaml-only")
        .expect("source");
    assert!(source.path.ends_with("core.yaml"));
}
