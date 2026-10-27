//! TASK-0103: content-based identification of the player's full rulebooks and
//! the matching full-pack unlock. Every fixture here is synthetic: no real book
//! text, no PDF, and no hash is read. Only the book type/edition/status is
//! asserted, never book prose.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-books-{name}-{}", std::process::id()));
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

/// Synthetic front matter: title page, copyright year, and optional extra
/// lines. This is deliberately not any real book's text.
fn pages(title: &str, year: &str, extra: &str) -> String {
    format!(
        "# {title}\n\n\
         <a id=\"p1\"></a>\n<!-- PDF p.1 -->\nDungeons & Dragons\n\n{title}\n\n\
         <a id=\"p2\"></a>\n<!-- PDF p.2 -->\nCopyright © {year} Synthetic Games LLC\n\n\
         <a id=\"p3\"></a>\n<!-- PDF p.3 -->\n{extra}\n"
    )
}

fn index_json(stem: &str) -> String {
    format!(
        "{{\"pdf\": \"{stem}.pdf\", \"book\": \"{stem}.pages.md\", \"pages\": \"{stem}.pages.md\", \"page_count\": 3, \"done_pages\": 3, \"headings\": [], \"chapters\": []}}"
    )
}

/// One converted book in the standard layout. The file name and folder name
/// are arbitrary on purpose: identification must not use them.
fn book(root: &Path, layer: &str, folder: &str, stem: &str, body: &str) -> PathBuf {
    let dir = root.join("Rules").join(layer).join(folder);
    fs::create_dir_all(dir.join("index")).unwrap();
    fs::write(dir.join(format!("{stem}.pdf")), b"%PDF-1.4 synthetic").unwrap();
    let idx = dir.join("index");
    fs::write(idx.join(format!("{stem}.pages.md")), body).unwrap();
    fs::write(idx.join("index.json"), index_json(stem)).unwrap();
    fs::write(idx.join("index.md"), "# Index\n").unwrap();
    idx
}

#[test]
fn identifies_books_by_content_not_by_file_or_folder_names() {
    let r = root("content");
    book(
        &r,
        "2024",
        "2014-players-handbook",
        "unrelated-name",
        &pages("Player's Handbook", "2014", "Chapter 1: Creating a Character"),
    );
    book(
        &r,
        "2014",
        "players-handbook-2024",
        "also-unrelated",
        &pages(
            "Dungeon Master's Guide",
            "2024",
            "Chapter 1: The Basics",
        ),
    );

    let records = talkdnd::library::scan_books(&r).unwrap();
    let phb = records
        .iter()
        .find(|b| b.status == talkdnd::library::BookStatus::Identified && b.book.as_deref() == Some("phb"))
        .unwrap_or_else(|| panic!("no identified PHB in {records:#?}"));
    assert_eq!(phb.edition.as_deref(), Some("2014"));
    assert!(
        phb.source.contains("2014-players-handbook"),
        "source should be the content path, got {}",
        phb.source
    );

    let dmg = records
        .iter()
        .find(|b| b.status == talkdnd::library::BookStatus::Identified && b.book.as_deref() == Some("dmg"))
        .unwrap_or_else(|| panic!("no identified DMG in {records:#?}"));
    assert_eq!(dmg.edition.as_deref(), Some("2024"));
    assert!(
        dmg.source.contains("players-handbook-2024"),
        "folder name must not decide, got {}",
        dmg.source
    );
}
