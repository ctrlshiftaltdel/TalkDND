//! TASK-0103: content-based identification of the player's full rulebooks and
//! the matching full-pack unlock. Every fixture here is synthetic: no real book
//! text, no PDF, and no hash is read. Only the book type/edition/status is
//! asserted, never book prose.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use talkdnd::library::{BookStatus, PackState};

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

/// A valid full pack with one synthetic rule section. `distribution` lets a
/// test write the wrong kind of pack without hand-crafting a hash.
fn pack(
    root: &Path,
    id: &str,
    edition: &str,
    distribution: combat_sim::content::Distribution,
) -> PathBuf {
    let meta = combat_sim::content::PackMeta {
        id: id.into(),
        name: Some("Synthetic full pack".into()),
        edition: edition.into(),
        source: Some("synthetic".into()),
        license: Some("test".into()),
        distribution,
        schema_version: combat_sim::content::pack::SCHEMA_VERSION,
        content_hash: None,
        created_at: None,
        description: None,
    };
    let content = combat_sim::content::schema::ContentFile {
        rule_sections: Some(vec![combat_sim::content::schema::DomainEntity {
            id: format!("{id}-section"),
            name: Some("Synthetic section".into()),
            ..Default::default()
        }]),
        ..Default::default()
    };
    let pack = combat_sim::content::ContentPack::new(meta, content).unwrap();
    let dir = root.join("Rules/packs");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.pack.json"));
    fs::write(&path, serde_json::to_string_pretty(&pack).unwrap()).unwrap();
    path
}

fn save(root: &Path, edition: &str) -> PathBuf {
    ok(
        root,
        &[
            "new",
            "--ruleset",
            edition,
            "--scope",
            "0",
            "--date",
            "2030-01-02",
        ],
    );
    root.join("Saves/2030-01-02-01")
}

fn library_line(save: &Path) -> String {
    let text = fs::read_to_string(save.join("combat/campaign.yaml")).unwrap();
    text.lines()
        .find(|l| l.trim_start().starts_with("library:"))
        .unwrap_or("")
        .to_string()
}

#[test]
fn identifies_books_by_content_not_by_file_or_folder_names() {
    let r = root("content");
    book(
        &r,
        "2024",
        "2014-players-handbook",
        "unrelated-name",
        &pages(
            "Player's Handbook",
            "2014",
            "Chapter 1: Creating a Character",
        ),
    );
    book(
        &r,
        "2014",
        "players-handbook-2024",
        "also-unrelated",
        &pages("Dungeon Master's Guide", "2024", "Chapter 1: The Basics"),
    );

    let records = talkdnd::library::scan_books(&r).unwrap();
    let phb = records
        .iter()
        .find(|b| {
            b.status == talkdnd::library::BookStatus::Identified && b.book.as_deref() == Some("phb")
        })
        .unwrap_or_else(|| panic!("no identified PHB in {records:#?}"));
    assert_eq!(phb.edition.as_deref(), Some("2014"));
    assert!(
        phb.source.contains("2014-players-handbook"),
        "source should be the content path, got {}",
        phb.source
    );

    let dmg = records
        .iter()
        .find(|b| {
            b.status == talkdnd::library::BookStatus::Identified && b.book.as_deref() == Some("dmg")
        })
        .unwrap_or_else(|| panic!("no identified DMG in {records:#?}"));
    assert_eq!(dmg.edition.as_deref(), Some("2024"));
    assert!(
        dmg.source.contains("players-handbook-2024"),
        "folder name must not decide, got {}",
        dmg.source
    );
}

#[test]
fn unlocks_only_the_matching_full_pack_for_the_save_edition() {
    let r = root("unlock");
    book(
        &r,
        "2014",
        "whatever",
        "2014 phb",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    book(
        &r,
        "2024",
        "other",
        "2024 phb",
        &pages("Player's Handbook", "2024", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    pack(
        &r,
        "full-2024-phb",
        "2024",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    talkdnd::library::scan_and_register(&r).unwrap();
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert_eq!(loaded, vec!["2014 Player's Handbook".to_string()]);
    let line = library_line(&s);
    assert!(line.contains("full-2014-phb.pack.json"), "{line}");
    assert!(!line.contains("full-2024-phb.pack.json"), "{line}");

    let statuses = talkdnd::library::pack_status(&r);
    let phb = statuses.iter().find(|p| p.id == "full-2014-phb").unwrap();
    assert_eq!(phb.state, PackState::Unlocked, "{phb:?}");
    let p24 = statuses.iter().find(|p| p.id == "full-2024-phb").unwrap();
    assert_eq!(p24.state, PackState::Unlocked, "{p24:?}");

    let s24 = r.join("Saves/2030-01-02-02");
    ok(
        &r,
        &[
            "new",
            "--ruleset",
            "5.5e-full",
            "--scope",
            "0",
            "--date",
            "2030-01-02",
        ],
    );
    talkdnd::library::sync_campaign_packs(&r, &s24).unwrap();
    let line = library_line(&s24);
    assert!(line.contains("full-2024-phb.pack.json"), "{line}");
    assert!(!line.contains("full-2014-phb.pack.json"), "{line}");
}

#[test]
fn a_pack_without_a_matching_pdf_never_unlocks() {
    let r = root("no-pdf");
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let before = library_line(&s);
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert!(records.is_empty(), "{records:#?}");
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert!(loaded.is_empty(), "{loaded:?}");
    assert_eq!(library_line(&s), before);
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::MissingBook, "{status:?}");
    assert!(
        status.detail.contains("2014") && status.detail.contains("Player's Handbook"),
        "missing book/edition must be named: {}",
        status.detail
    );
    assert!(
        talkdnd::library::pack_digest_line(&r, &s)
            .unwrap()
            .contains("未解锁"),
        "{}",
        talkdnd::library::pack_digest_line(&r, &s).unwrap()
    );
}

#[test]
fn a_wrong_edition_does_not_unlock_another_editions_pack() {
    let r = root("wrong-edition");
    // The content says 2024 PHB even though it was placed under Rules/2014.
    book(
        &r,
        "2014",
        "misfiled",
        "book",
        &pages("Player's Handbook", "2024", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].edition.as_deref(), Some("2024"));
    assert_eq!(records[0].book.as_deref(), Some("phb"));
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert!(loaded.is_empty(), "{loaded:?}");
    assert!(!library_line(&s).contains("full-"));
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::WrongEdition, "{status:?}");
    assert!(
        status.detail.contains("2024") && status.detail.contains("2014"),
        "{}",
        status.detail
    );
}

#[test]
fn an_ambiguous_scan_waits_for_the_player_and_then_unlocks() {
    let r = root("ambiguous");
    // Title page only: no copyright/publication year, so the edition is not
    // knowable from content. The scan must not silently pick one.
    let body = "\
<a id=\"p1\"></a>\n<!-- PDF p.1 -->\nDungeons & Dragons\nPlayer's Handbook\n\
<a id=\"p2\"></a>\n<!-- PDF p.2 -->\nChapter 1: Creating a Character\n";
    let dir = book(&r, "2014", "unclear", "book", body);
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    let record = &records[0];
    assert_eq!(record.status, BookStatus::Ambiguous, "{record:#?}");
    assert_eq!(record.book.as_deref(), Some("phb"));
    assert!(record.candidates.contains(&"full-2014-phb".to_string()));
    assert!(record.candidates.contains(&"full-2024-phb".to_string()));
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::Ambiguous, "{status:?}");
    assert!(talkdnd::library::sync_campaign_packs(&r, &s)
        .unwrap()
        .is_empty());
    assert!(!library_line(&s).contains("full-"));

    let source = record.source.clone();
    let chosen = talkdnd::library::choose_book(&r, &source, "2014", "phb").unwrap();
    assert_eq!(chosen.status, BookStatus::Identified);
    assert_eq!(chosen.method, "manual");
    // A later scan keeps the explicit choice while the source is unchanged.
    let again = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(again[0].method, "manual");
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert_eq!(loaded, vec!["2014 Player's Handbook".to_string()]);
    assert!(library_line(&s).contains("full-2014-phb.pack.json"));
    assert!(dir.is_dir());
}

#[test]
fn an_index_without_the_players_pdf_is_not_a_full_book() {
    let r = root("index-only");
    book(
        &r,
        "2014",
        "leftover",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    fs::remove_file(r.join("Rules/2014/leftover/book.pdf")).unwrap();
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].status, BookStatus::MissingIndex, "{records:#?}");
    assert!(records[0].book.is_none());
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::MissingBook, "{status:?}");
}

#[test]
fn duplicate_copies_register_separately_but_unlock_once() {
    let r = root("duplicates");
    book(
        &r,
        "2014",
        "copy-one",
        "one",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    book(
        &r,
        "2014",
        "copy-two",
        "two",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|r| r.status == BookStatus::Identified)
            .count(),
        2,
        "{records:#?}"
    );
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert_eq!(loaded.len(), 1, "{loaded:?}");
    let line = library_line(&s);
    assert_eq!(line.matches("full-2014-phb.pack.json").count(), 1, "{line}");
}

#[test]
fn an_srd_or_masquerading_file_is_not_a_full_book() {
    let r = root("srd");
    // Folder and PDF name claim a full PHB; the front matter is the SRD.
    let body = "\
<a id=\"p1\"></a>\n<!-- PDF p.1 -->\nDungeons & Dragons\nSystem Reference Document 5.2.1\n\
<a id=\"p2\"></a>\n<!-- PDF p.2 -->\nCopyright © 2024 Synthetic Games LLC\n\
<a id=\"p3\"></a>\n<!-- PDF p.3 -->\nPlayer's Handbook\n";
    book(
        &r,
        "2024",
        "players-handbook-2024",
        "Player's Handbook 2024",
        body,
    );
    pack(
        &r,
        "full-2024-phb",
        "2024",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5.5e-srd");
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records[0].status, BookStatus::SrdOnly, "{records:#?}");
    assert!(talkdnd::library::sync_campaign_packs(&r, &s)
        .unwrap()
        .is_empty());
    assert!(!library_line(&s).contains("full-"));
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2024-phb")
        .unwrap();
    assert_eq!(status.state, PackState::MissingBook, "{status:?}");
}

#[test]
fn an_invalid_pack_never_unlocks() {
    let r = root("invalid-pack");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Srd,
    );
    let s = save(&r, "5e-full");
    talkdnd::library::scan_and_register(&r).unwrap();
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::Invalid, "{status:?}");
    assert!(
        status.detail.contains("2014"),
        "invalid pack must name the edition: {}",
        status.detail
    );
    assert!(talkdnd::library::sync_campaign_packs(&r, &s)
        .unwrap()
        .is_empty());
    assert!(!library_line(&s).contains("full-"));
    let digest = talkdnd::library::pack_digest_line(&r, &s).unwrap();
    assert!(
        digest.contains("full-2014-phb"),
        "invalid pack must be named in the digest: {digest}"
    );

    // A corrupt file is reported, not trusted by its name.
    fs::write(r.join("Rules/packs/full-2014-phb.pack.json"), "{not json").unwrap();
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::Invalid, "{status:?}");
}

#[test]
fn spaced_ocr_titles_are_recognized() {
    let r = root("ocr");
    book(
        &r,
        "2014",
        "noisy",
        "book",
        &pages("P l a y e r ' s   H a n d b o o k", "2014", "Chapter 1"),
    );
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records[0].status, BookStatus::Identified, "{records:#?}");
    assert_eq!(records[0].edition.as_deref(), Some("2014"));
    assert_eq!(records[0].book.as_deref(), Some("phb"));
}

#[test]
fn detection_and_commands_never_write_or_print_book_prose() {
    let marker = "SYNTHETIC-PROSE-MARKER-9f3a";
    let r = root("privacy");
    let body = format!(
        "<a id=\"p1\"></a>\n<!-- PDF p.1 -->\nDungeons & Dragons\nPlayer's Handbook\n\
         <a id=\"p2\"></a>\n<!-- PDF p.2 -->\nCopyright © 2014 Synthetic Games LLC\n\
         {marker}\n"
    );
    book(&r, "2014", "secret", "book", &body);
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-srd");
    let scan = ok(&r, &["library", "scan"]);
    assert!(!scan.contains(marker), "scan leaked book text: {scan}");
    let packs = ok(&r, &["library", "packs"]);
    assert!(!packs.contains(marker), "packs leaked book text: {packs}");
    let registry = fs::read_to_string(r.join(".talkdnd/books.yaml")).unwrap();
    assert!(!registry.contains(marker), "registry leaked book text");
    let start = ok(&r, &["start"]);
    assert!(
        !start.contains(marker),
        "start digest leaked book text: {start}"
    );
    assert!(s.join("digest.md").is_file());
}

#[test]
fn cli_scan_and_packs_name_the_book_and_edition() {
    let r = root("cli");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let scan = ok(&r, &["library", "scan"]);
    assert!(scan.contains("2014 Player's Handbook"), "{scan}");
    assert!(scan.contains("Rules/2014/book/index/"), "{scan}");
    let packs = ok(&r, &["library", "packs"]);
    assert!(packs.contains("full-2014-phb"), "{packs}");
    assert!(packs.contains("已解锁"), "{packs}");
    assert!(packs.contains("SRD"), "{packs}");
    assert!(
        packs.contains("2014") && packs.contains("Monster Manual"),
        "missing books must be named: {packs}"
    );
}

#[test]
fn start_keeps_the_old_digest_and_stays_srd_only_without_books() {
    let r = root("start");
    let s = save(&r, "5.5e-srd");
    let before = library_line(&s);
    let out = ok(&r, &["start"]);
    assert!(out.contains("规则集：**5.5e SRD**"), "{out}");
    assert!(out.contains("content check"), "{out}");
    assert!(out.contains("完整书内容包：仅 SRD"), "{out}");
    assert_eq!(library_line(&s), before);
    assert!(s.join("digest.md").is_file());
}

#[test]
fn start_loads_the_matching_pack_and_drops_it_when_the_pdf_is_gone() {
    let r = root("start-unlock");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let out = ok(&r, &["start"]);
    assert!(out.contains("规则集：**5e FULL**"), "{out}");
    assert!(
        out.contains("完整书内容包：已解锁 2014 Player's Handbook"),
        "{out}"
    );
    assert!(
        library_line(&s).contains("full-2014-phb.pack.json"),
        "{}",
        library_line(&s)
    );

    // Taking the pack away must fail closed, not fall back to SRD.
    let pack_path = r.join("Rules/packs/full-2014-phb.pack.json");
    let pack_bytes = fs::read(&pack_path).unwrap();
    fs::remove_file(&pack_path).unwrap();
    let (good, out) = run(&r, &["start"]);
    assert!(!good, "5e FULL without a pack must fail: {out}");
    for needle in [
        "5e FULL",
        "Player's Handbook",
        "Rules/2014",
        "talkdnd library scan",
        "5e SRD",
    ] {
        assert!(out.contains(needle), "missing {needle} in {out}");
    }
    assert!(
        !out.contains("完整书内容包：仅 SRD"),
        "must not fall back: {out}"
    );
    assert!(
        library_line(&s).contains("full-2014-phb.pack.json"),
        "a failed start must not rewrite the library: {}",
        library_line(&s)
    );

    // Taking the book PDF away must also fail closed: no false unlock.
    fs::write(&pack_path, pack_bytes).unwrap();
    fs::remove_dir_all(r.join("Rules/2014/book")).unwrap();
    let (good, out) = run(&r, &["start"]);
    assert!(!good, "5e FULL without the book must fail: {out}");
    assert!(out.contains("Player's Handbook"), "{out}");
}

#[test]
fn an_index_without_pages_is_unconverted_not_identified() {
    let r = root("no-pages");
    let dir = book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    fs::remove_file(dir.join("book.pages.md")).unwrap();
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].status, BookStatus::MissingIndex, "{records:#?}");

    // A player's manual choice cannot bypass missing content.
    let err = talkdnd::library::choose_book(&r, &records[0].source, "2014", "phb")
        .unwrap_err()
        .to_string();
    assert!(err.contains("缺少 PDF 或转换索引"), "{err}");
    assert!(!talkdnd::library::pack_status(&r)
        .iter()
        .any(|p| p.state == PackState::Unlocked));
}

#[test]
fn a_wrong_book_does_not_unlock_the_other_books_pack() {
    let r = root("wrong-book");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Dungeon Master's Guide", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    talkdnd::library::scan_and_register(&r).unwrap();
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::MissingBook, "{status:?}");
    assert!(status.detail.contains("Player's Handbook"), "{status:?}");
    assert!(talkdnd::library::sync_campaign_packs(&r, &s)
        .unwrap()
        .is_empty());
}

#[test]
fn choose_on_the_cli_records_the_player_selection() {
    let r = root("choose-cli");
    let body = "\
<a id=\"p1\"></a>\n<!-- PDF p.1 -->\nDungeons & Dragons\nPlayer's Handbook\n\
<a id=\"p2\"></a>\n<!-- PDF p.2 -->\nChapter 1\n";
    book(&r, "2014", "unclear", "book", body);
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    ok(&r, &["library", "scan"]);
    let out = ok(
        &r,
        &[
            "library",
            "choose",
            "Rules/2014/unclear/index",
            "--edition",
            "2014",
            "--kind",
            "phb",
        ],
    );
    assert!(out.contains("2014 Player's Handbook"), "{out}");
    let packs = ok(&r, &["library", "packs"]);
    assert!(packs.contains("full-2014-phb — 已解锁"), "{packs}");
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert_eq!(loaded, vec!["2014 Player's Handbook".to_string()]);
}

#[test]
fn duplicate_pack_ids_are_invalid() {
    let r = root("duplicate-packs");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let copy = r.join("Rules/packs/copy/copy.pack.json");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(r.join("Rules/packs/full-2014-phb.pack.json"), &copy).unwrap();
    talkdnd::library::scan_and_register(&r).unwrap();
    let status = talkdnd::library::pack_status(&r)
        .into_iter()
        .find(|p| p.id == "full-2014-phb")
        .unwrap();
    assert_eq!(status.state, PackState::Invalid, "{status:?}");
    assert!(
        status.detail.contains("重复") || status.detail.contains("校验失败"),
        "{status:?}"
    );
}

#[test]
fn a_missing_title_page_stays_ambiguous() {
    let r = root("missing-title");
    let body = "\
<a id=\"p2\"></a>\n<!-- PDF p.2 -->\nDungeons & Dragons\nPlayer's Handbook\n\
<a id=\"p3\"></a>\n<!-- PDF p.3 -->\nCopyright © 2014 Synthetic Games LLC\n";
    book(&r, "2014", "book", "book", body);
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records[0].status, BookStatus::Ambiguous, "{records:#?}");
    assert!(
        !talkdnd::library::pack_status(&r)
            .iter()
            .any(|p| p.state == PackState::Unlocked),
        "a missing title page must not unlock"
    );
}

#[test]
fn scan_and_register_is_idempotent() {
    let r = root("idempotent");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    let first = talkdnd::library::scan_and_register(&r).unwrap();
    let text = fs::read_to_string(r.join(".talkdnd/books.yaml")).unwrap();
    let second = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        text,
        fs::read_to_string(r.join(".talkdnd/books.yaml")).unwrap()
    );
}

#[test]
fn sync_keeps_unmanaged_srd_packs_in_the_library_list() {
    let r = root("srd-pack");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let manifest = s.join("combat/campaign.yaml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        text.replace(
            "library: [../../../Rules/combat]",
            "library: [../../../Rules/combat, ../../../Rules/packs/srd-custom.pack.json]",
        ),
    )
    .unwrap();
    talkdnd::library::scan_and_register(&r).unwrap();
    talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    let line = library_line(&s);
    assert!(line.contains("srd-custom.pack.json"), "{line}");
    assert!(line.contains("full-2014-phb.pack.json"), "{line}");
}

#[test]
fn the_save_manifest_loads_only_the_unlocked_full_pack() {
    let r = root("engine-load");
    book(
        &r,
        "2014",
        "book",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    pack(
        &r,
        "full-2024-phb",
        "2024",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    ok(&r, &["start"]);
    fs::create_dir_all(r.join("Rules/combat")).unwrap();
    let sources = combat_sim::content::ContentSources {
        campaign: Some(s.join("combat/campaign.yaml")),
        ..Default::default()
    };
    let registry = sources.load().unwrap();
    assert_eq!(registry.ruleset, "2014");
    assert!(
        registry.packs.contains_key("full-2014-phb"),
        "unlocked pack not loaded: {:?}",
        registry.packs.keys().collect::<Vec<_>>()
    );
    assert!(
        !registry.packs.contains_key("full-2024-phb"),
        "wrong-edition pack loaded"
    );
}

#[test]
fn a_manual_choice_does_not_survive_losing_the_pdf() {
    let r = root("manual-no-pdf");
    let body = "\
<a id=\"p1\"></a>\n<!-- PDF p.1 -->\nDungeons & Dragons\nPlayer's Handbook\n\
<a id=\"p2\"></a>\n<!-- PDF p.2 -->\nChapter 1\n";
    book(&r, "2014", "unclear", "book", body);
    pack(
        &r,
        "full-2014-phb",
        "2014",
        combat_sim::content::Distribution::Full,
    );
    let s = save(&r, "5e-full");
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    talkdnd::library::choose_book(&r, &records[0].source, "2014", "phb").unwrap();
    talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert!(library_line(&s).contains("full-2014-phb.pack.json"));

    fs::remove_file(r.join("Rules/2014/unclear/book.pdf")).unwrap();
    let records = talkdnd::library::scan_and_register(&r).unwrap();
    assert_eq!(records[0].status, BookStatus::MissingIndex, "{records:#?}");
    talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert!(!library_line(&s).contains("full-"), "{}", library_line(&s));
}

#[test]
fn each_matching_book_unlocks_its_own_pack() {
    let r = root("two-books");
    book(
        &r,
        "2014",
        "phb",
        "book",
        &pages("Player's Handbook", "2014", "Chapter 1"),
    );
    book(
        &r,
        "2014",
        "dmg",
        "book",
        &pages("Dungeon Master's Guide", "2014", "Chapter 1"),
    );
    for id in ["full-2014-phb", "full-2014-dmg"] {
        pack(&r, id, "2014", combat_sim::content::Distribution::Full);
    }
    let s = save(&r, "5e-full");
    talkdnd::library::scan_and_register(&r).unwrap();
    let loaded = talkdnd::library::sync_campaign_packs(&r, &s).unwrap();
    assert_eq!(
        loaded,
        vec![
            "2014 Player's Handbook".to_string(),
            "2014 Dungeon Master's Guide".to_string()
        ]
    );
    let line = library_line(&s);
    assert!(line.contains("full-2014-phb.pack.json"), "{line}");
    assert!(line.contains("full-2014-dmg.pack.json"), "{line}");
}
