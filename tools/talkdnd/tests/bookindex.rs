//! TASK-0060 (P07): `talkdnd adventure index build|show|search` on a
//! synthetic converted book. The book is made up: no real book text, and the
//! assertions check that the built index never carries body text.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-bookindex-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "x").unwrap();
    p
}

fn run(root: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .output()
        .unwrap();
    let s =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), s)
}

fn ok(root: &Path, args: &[&str]) -> String {
    let (good, s) = run(root, args);
    assert!(good, "{args:?} failed: {s}");
    s
}

/// pdf2md's paged Markdown of the synthetic book. Body sentences are
/// deliberately distinctive so a leak into structured.json is visible.
const PAGES: &str = "\
# Test Book

<a id=\"p4\"></a>
<!-- PDF p.4 -->
## Chapter 1: First Job

the butler hides a secret in the cellar

### The Inn

inn body text

### The Cellar

cellar body text

<a id=\"p12\"></a>
<!-- PDF p.12 -->
## Chapter 2: Second Job

second body text
";

/// pdf2md's index.json of the same book.
const INDEX: &str = r#"{
 "pdf": "Test Book.pdf",
 "book": "Test Book.pages.md",
 "page_count": 12,
 "done_pages": 12,
 "headings": [
  {"level": 2, "text": "Chapter 1: First Job", "page": 4},
  {"level": 3, "text": "The Inn", "page": 6},
  {"level": 3, "text": "The Cellar", "page": 8},
  {"level": 2, "text": "Chapter 2: Second Job", "page": 12}
 ],
 "chapters": [
  {"title": "Chapter 1: First Job", "page": 4},
  {"title": "Chapter 2: Second Job", "page": 12}
 ],
 "maps": [{"caption": "MAP 1.1: The Inn Grounds", "page": 6, "img": "img/p0006.webp"}],
 "figures": [{"page": 12, "img": "img/p0012.webp"}],
 "statblocks": [8],
 "statblock_checks": [
  {"name": "Cellar Creeper", "page": 8, "status": "ok", "issues": []}
 ],
 "low_quality_pages": [3]
}"#;

/// A converted book in the standard layout: the PDF in the book folder,
/// pdf2md's output in `index/`. Returns the index folder.
fn book(root: &Path) -> PathBuf {
    let dir = root.join("Adventures/test-book/index");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        root.join("Adventures/test-book/Test Book.pdf"),
        b"%PDF-1.4 synthetic",
    )
    .unwrap();
    fs::write(dir.join("Test Book.pages.md"), PAGES).unwrap();
    fs::write(dir.join("index.json"), INDEX).unwrap();
    dir
}

/// A save whose adventure is the synthetic book.
fn save(root: &Path, tag: &str) {
    ok(
        root,
        &[
            "new",
            "--date",
            "2030-01-02",
            "--scope",
            "0",
            "--adventure",
            "Test Book",
        ],
    );
    assert!(root.join("Saves/2030-01-02-01").is_dir(), "{tag}: no save");
}

fn structured(dir: &Path) -> serde_json::Value {
    let text = fs::read_to_string(dir.join("structured.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn build_is_idempotent_and_records_the_source() {
    let r = root("build");
    save(&r, "build");
    let dir = book(&r);
    let out = ok(&r, &["adventure", "index", "build"]);
    assert!(
        out.contains("已重建") && out.contains("structured.json"),
        "{out}"
    );
    let text = fs::read_to_string(dir.join("structured.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["book"], "test-book");
    assert_eq!(v["source"]["path"], "Test Book.pdf");
    let sha = v["source"]["sha256"].as_str().unwrap();
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "{sha}");
    assert_eq!(v["low_quality_pages"], serde_json::json!([3]));
    // Unchanged sources: skipped, and the file is not rewritten.
    let out = ok(&r, &["adventure", "index", "build"]);
    assert!(out.contains("已跳过"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("structured.json")).unwrap(),
        text
    );
}

#[test]
fn a_changed_pdf_rebuilds() {
    let r = root("source");
    save(&r, "source");
    let dir = book(&r);
    ok(&r, &["adventure", "index", "build"]);
    let before = structured(&dir);
    fs::write(
        r.join("Adventures/test-book/Test Book.pdf"),
        b"%PDF-1.4 synthetic, second edition",
    )
    .unwrap();
    let out = ok(&r, &["adventure", "index", "build"]);
    assert!(out.contains("已重建"), "{out}");
    let after = structured(&dir);
    assert_ne!(before["source"]["sha256"], after["source"]["sha256"]);
    assert_eq!(after["headings"], before["headings"]);
}

#[test]
fn missing_index_json_or_pages_md_is_named_in_the_error() {
    let r = root("missing");
    save(&r, "missing");
    let dir = book(&r);
    fs::remove_file(dir.join("index.json")).unwrap();
    let (good, out) = run(&r, &["adventure", "index", "build"]);
    assert!(!good && out.contains("index.json"), "{out}");
    fs::write(dir.join("index.json"), INDEX).unwrap();
    fs::remove_file(dir.join("Test Book.pages.md")).unwrap();
    let (good, out) = run(&r, &["adventure", "index", "build"]);
    assert!(!good && out.contains(".pages.md"), "{out}");
}

#[test]
fn a_book_without_headings_is_refused() {
    let r = root("nohead");
    save(&r, "nohead");
    let dir = book(&r);
    let mut v: serde_json::Value = serde_json::from_str(INDEX).unwrap();
    v["headings"] = serde_json::json!([]);
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();
    let (good, out) = run(&r, &["adventure", "index", "build"]);
    assert!(!good && out.contains("标题"), "{out}");
}

#[test]
fn structured_json_holds_no_body_text() {
    let r = root("nobody");
    save(&r, "nobody");
    let dir = book(&r);
    ok(&r, &["adventure", "index", "build"]);
    let text = fs::read_to_string(dir.join("structured.json")).unwrap();
    for body in ["butler", "inn body", "cellar body", "second body"] {
        assert!(!text.contains(body), "structured.json leaks {body}");
    }
    for label in ["The Cellar", "MAP 1.1", "Cellar Creeper", "Chapter 2"] {
        assert!(text.contains(label), "structured.json misses {label}");
    }
}

#[test]
fn order_is_document_order_and_rewriting_sources_skips() {
    let r = root("order");
    save(&r, "order");
    let dir = book(&r);
    ok(&r, &["adventure", "index", "build"]);
    let first = fs::read_to_string(dir.join("structured.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&first).unwrap();
    let titles: Vec<&str> = v["headings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "Chapter 1: First Job",
            "The Inn",
            "The Cellar",
            "Chapter 2: Second Job",
        ]
    );
    let chapters: Vec<&str> = v["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();
    assert_eq!(chapters, ["Chapter 1: First Job", "Chapter 2: Second Job"]);
    // Same bytes, new mtimes: the derived index is unchanged, so skipped.
    fs::write(dir.join("index.json"), INDEX).unwrap();
    let out = ok(&r, &["adventure", "index", "build"]);
    assert!(out.contains("已跳过"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("structured.json")).unwrap(),
        first
    );
}

#[test]
fn chapters_fall_back_to_heading_detection() {
    let r = root("chapters");
    save(&r, "chapters");
    let dir = book(&r);
    let mut v: serde_json::Value = serde_json::from_str(INDEX).unwrap();
    v["chapters"] = serde_json::json!([]);
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();
    ok(&r, &["adventure", "index", "build"]);
    let v = structured(&dir);
    let chapters: Vec<&str> = v["chapters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();
    assert_eq!(chapters, ["Chapter 1: First Job", "Chapter 2: Second Job"]);
}

#[test]
fn search_matches_labels_only_never_body() {
    let r = root("search");
    save(&r, "search");
    book(&r);
    ok(&r, &["adventure", "index", "build"]);
    let out = ok(&r, &["adventure", "index", "search", "test-book", "cellar"]);
    assert!(
        out.contains("The Cellar") && out.contains("p.8") && out.contains("#p8"),
        "{out}"
    );
    assert!(out.contains("Cellar Creeper"), "{out}");
    let out = ok(
        &r,
        &["adventure", "index", "search", "test-book", "inn grounds"],
    );
    assert!(out.contains("MAP 1.1: The Inn Grounds"), "{out}");
    let out = ok(
        &r,
        &["adventure", "index", "search", "test-book", "second job"],
    );
    assert!(out.contains("Chapter 2: Second Job"), "{out}");
    // Body-only words are never matched.
    let out = ok(&r, &["adventure", "index", "search", "test-book", "butler"]);
    assert!(out.contains("没有找到"), "{out}");
}

#[test]
fn show_prints_titles_pages_anchors_and_no_body() {
    let r = root("show");
    save(&r, "show");
    let dir = book(&r);
    let (good, out) = run(&r, &["adventure", "index", "show", "test-book"]);
    assert!(!good && out.contains("build"), "{out}");
    ok(&r, &["adventure", "index", "build"]);
    let out = ok(&r, &["adventure", "index", "show", "test-book"]);
    assert!(
        out.contains("Chapter 1: First Job") && out.contains("p.4") && out.contains("#p4"),
        "{out}"
    );
    assert!(
        out.contains("The Cellar") && out.contains("Cellar Creeper"),
        "{out}"
    );
    assert!(out.contains("MAP 1.1: The Inn Grounds"), "{out}");
    for body in ["butler", "inn body", "cellar body", "second body"] {
        assert!(!out.contains(body), "show leaks {body}");
    }
    let _ = dir;
}

#[test]
fn build_writes_only_structured_json() {
    let r = root("writes");
    save(&r, "writes");
    let dir = book(&r);
    ok(&r, &["adventure", "index", "build"]);
    let tree = || tree_of(&r);
    let before = tree();
    fs::remove_file(dir.join("structured.json")).unwrap();
    ok(&r, &["adventure", "index", "build"]);
    assert_eq!(tree(), before, "build touched files outside index/");
}

/// Every file under `base`, relative and sorted.
fn tree_of(base: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(base, base, &mut out);
    out
}

#[test]
fn a_newer_structured_version_is_refused() {
    let r = root("newer");
    save(&r, "newer");
    let dir = book(&r);
    ok(&r, &["adventure", "index", "build"]);
    let mut v = structured(&dir);
    v["version"] = serde_json::json!(2);
    fs::write(
        dir.join("structured.json"),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();
    let (good, out) = run(&r, &["adventure", "index", "build"]);
    assert!(!good && out.contains("版本"), "{out}");
}

#[test]
fn a_corrupt_structured_json_says_what_to_do() {
    let r = root("corrupt");
    save(&r, "corrupt");
    let dir = book(&r);
    ok(&r, &["adventure", "index", "build"]);
    fs::write(dir.join("structured.json"), "{ not json").unwrap();
    let (good, out) = run(&r, &["adventure", "index", "build"]);
    assert!(
        !good && out.contains("structured.json") && out.contains("删除"),
        "{out}"
    );
}

#[test]
fn explicit_book_and_unknown_book() {
    let r = root("explicit");
    save(&r, "explicit");
    book(&r);
    let out = ok(&r, &["adventure", "index", "build", "--book", "test-book"]);
    assert!(out.contains("已重建"), "{out}");
    let (good, out) = run(
        &r,
        &["adventure", "index", "build", "--book", "no-such-book"],
    );
    assert!(!good && out.contains("no-such-book"), "{out}");
    let (good, out) = run(&r, &["adventure", "index", "search", "no-such-book", "x"]);
    assert!(!good && out.contains("no-such-book"), "{out}");
}
