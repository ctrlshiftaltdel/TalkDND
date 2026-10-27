//! The book library: one folder per book under `Adventures/` and
//! `Rules/<edition>/`.
//!
//! ```text
//! Adventures/<slug>/<Original Name>.pdf   the player's own book (name kept)
//! Adventures/<slug>/index/                 pdf2md lookup index (one paged Markdown)
//! Adventures/<slug>/combat/                the adventure's combat data
//! Rules/<edition>/<slug>/<Original>.pdf
//! Rules/<edition>/<slug>/index/
//! ```
//!
//! `<slug>` is [`slug`] of the PDF file name without `.pdf`. Program data
//! (`Rules/chargen`, `Rules/combat`, `Rules/defaults`, `Rules/rulings*.md`) is
//! not a book and stays outside the book folders.
//!
//! The index folder holds exactly one Markdown edition, `<Stem>.pages.md`
//! (page anchors `<a id="pN"></a>`), plus `index.md`/`index.json`,
//! `manifest.json`, `headings.json`, `pages/` and `img/`. Older pdf2md
//! versions wrote the index to a hidden `.md/<Stem>/` folder next to the PDF
//! and also a page-less `<Stem>.md`; readers here still find the old layout
//! until [`tidy`] has moved it.
//!
//! [`tidy`] (run by `talkdnd start` and `talkdnd library tidy`) moves loose
//! PDFs and their existing index into the right folder. It never deletes a
//! book file: on a collision both copies stay and a warning is printed. The
//! only files it removes are the redundant page-less reader editions that
//! pdf2md derived from the paged Markdown.

use anyhow::{bail, Context, Result};
use combat_sim::content::loader::Ruleset;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const INDEX_DIR: &str = "index";
pub const LEGACY_INDEX_DIR: &str = ".md";
pub const EDITIONS: [&str; 2] = ["2014", "2024"];
/// The three full rulebooks the v1.2 full packs cover.
pub const BOOK_KINDS: [&str; 3] = ["phb", "dmg", "mm"];
/// Full-book pack files the player dropped here (`full-<edition>-<book>`).
pub const FULL_PACK_DIR: &str = "Rules/packs";

/// One book found under `Rules/<edition>/`, identified from its converted
/// front matter. The record never contains book text or an absolute path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookRecord {
    #[serde(default)]
    pub edition: Option<String>,
    #[serde(default)]
    pub book: Option<String>,
    pub status: BookStatus,
    /// Index folder or book folder, relative to the play root, `/`-separated.
    pub source: String,
    /// `content` or `manual` (the player chose after an ambiguous scan).
    pub method: String,
    /// Short reason with counts/features only; never book prose.
    pub evidence: String,
    /// mtime+length of the converted Markdown, used only to avoid re-reading
    /// unchanged front matter. Never a PDF hash.
    #[serde(default)]
    pub stamp: String,
    /// Player-selectable candidates when the scan is ambiguous (`full-2014-phb`).
    #[serde(default)]
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookStatus {
    Identified,
    Ambiguous,
    #[default]
    Unknown,
    SrdOnly,
    MissingIndex,
}

/// How many PDF pages are read for the content signature. Front matter holds
/// the title page, copyright year and table of contents in the core books.
const FRONT_PAGES: u32 = 16;
/// Hard cap on bytes read from a converted book (front matter only).
const FRONT_BYTES: usize = 2 * 1024 * 1024;
const BOOKS_FILE: &str = ".talkdnd/books.yaml";
const BOOKS_VERSION: u32 = 1;

/// A short, player-facing name. Never a book's text.
pub fn book_label(kind: &str) -> &'static str {
    match kind {
        "phb" => "Player's Handbook",
        "dmg" => "Dungeon Master's Guide",
        "mm" => "Monster Manual",
        _ => "未知书型",
    }
}

pub fn book_kind(value: &str) -> Option<&'static str> {
    let mut v = String::new();
    for c in value.trim().to_lowercase().chars() {
        if matches!(c, '\'' | '\u{2019}') {
            continue;
        }
        if c.is_ascii_alphanumeric() {
            v.push(c);
        } else if !v.ends_with('-') {
            v.push('-');
        }
    }
    let v = v.trim_matches('-');
    if v == "phb" || v == "players-handbook" || v == "player-handbook" {
        Some("phb")
    } else if v == "dmg" || v == "dungeon-masters-guide" || v == "dungeon-master-guide" {
        Some("dmg")
    } else if v == "mm" || v == "monster-manual" {
        Some("mm")
    } else {
        None
    }
}

/// A pdf2md page marker's page number (`<a id="p12"></a>`,
/// `<!-- PDF p.12 -->`).
fn page_marker_number(line: &str) -> Option<u32> {
    let t = line.trim();
    let num = |s: &str| {
        (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    if let Some(rest) = t
        .strip_prefix("<a id=\"p")
        .and_then(|r| r.strip_suffix("\"></a>"))
    {
        return num(rest);
    }
    if let Some(rest) = t
        .strip_prefix("<!-- PDF p.")
        .and_then(|r| r.strip_suffix(" -->"))
    {
        return num(rest);
    }
    None
}

/// Is this line a pdf2md page marker (`<a id="p12"></a>` / `<!-- PDF p.12 -->`)?
pub fn is_page_marker(line: &str) -> bool {
    page_marker_number(line).is_some()
}

/// Front matter read for identification: page-weighted text and title-level
/// headings. It is dropped after classification and never written anywhere.
#[derive(Debug, Default)]
struct FrontMatter {
    pages: Vec<(u32, String)>,
    headings: Vec<String>,
}

fn front_headings(dir: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(dir.join("index.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in v["headings"].as_array().into_iter().flatten() {
        if let Some(t) = item["text"].as_str() {
            if !t.trim().is_empty() {
                out.push(t.to_string());
            }
        }
    }
    for item in v["chapters"].as_array().into_iter().flatten() {
        if let Some(t) = item["title"].as_str() {
            if !t.trim().is_empty() {
                out.push(t.to_string());
            }
        }
    }
    for item in v["intake"]["toc"].as_array().into_iter().flatten() {
        if let Some(t) = item["title"].as_str() {
            if !t.trim().is_empty() {
                out.push(t.to_string());
            }
        }
    }
    out
}

/// Read only the first [`FRONT_PAGES`] pages (or [`FRONT_BYTES`]) of a
/// converted book. No PDF is opened and no hash is computed.
fn front_matter(dir: &Path) -> Result<Option<FrontMatter>> {
    let Some(path) = paged_markdown(dir) else {
        return Ok(None);
    };
    let file =
        fs::File::open(&path).with_context(|| format!("read converted book {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut pages: Vec<(u32, String)> = Vec::new();
    let mut current = 1u32;
    let mut page_text = String::new();
    let mut bytes = 0usize;
    let mut line = String::new();
    loop {
        line.clear();
        let n = std::io::BufRead::read_line(&mut reader, &mut line)?;
        if n == 0 {
            break;
        }
        bytes += n;
        if let Some(page) = page_marker_number(&line) {
            if !page_text.trim().is_empty() {
                pages.push((current, std::mem::take(&mut page_text)));
            }
            current = page;
            if page > FRONT_PAGES {
                break;
            }
            continue;
        }
        if current <= FRONT_PAGES {
            page_text.push_str(&line);
        }
        if bytes >= FRONT_BYTES {
            break;
        }
    }
    if !page_text.trim().is_empty() {
        pages.push((current, page_text));
    }
    pages.retain(|(page, _)| *page <= FRONT_PAGES);
    Ok(Some(FrontMatter {
        pages,
        headings: front_headings(dir),
    }))
}

/// lowercase, apostrophes removed, every other run of non-alphanumerics
/// collapsed to one space. `Player's Handbook` -> `players handbook`.
fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = true;
    for ch in text.chars() {
        if matches!(ch, '\'' | '\u{2019}' | '`' | '\u{00b4}') {
            continue;
        }
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() {
                out.push(c);
            }
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim().to_string()
}

fn page_weight(page: u32) -> i32 {
    match page {
        1 => 20,
        2 => 16,
        3 => 12,
        4 => 10,
        5..=8 => 5,
        _ => 2,
    }
}

fn title_hits(text: &str, pattern: &str) -> bool {
    let compact = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    text.contains(pattern) || compact(text).contains(&compact(pattern))
}

/// Content features only: title phrases, copyright/publication year and
/// title-level headings. Folder and file names are never passed in.
#[derive(Debug, Default)]
struct Signals {
    books: [i32; 3],
    editions: [i32; 2],
    srd: i32,
}

fn signals(front: &FrontMatter) -> Signals {
    let mut s = Signals::default();
    for (page, text) in &front.pages {
        let norm = normalize_text(text);
        let weight = page_weight(*page);
        for (i, patterns) in [
            &["players handbook", "player handbook"][..],
            &["dungeon masters guide", "dungeon master guide"][..],
            &["monster manual"][..],
        ]
        .iter()
        .enumerate()
        {
            if patterns.iter().any(|p| title_hits(&norm, p)) {
                s.books[i] += weight;
            }
        }
        if *page <= 4 && title_hits(&norm, "system reference document") {
            s.srd += weight;
        }
        for token in norm.split_whitespace() {
            if token == "2014" {
                s.editions[0] += weight;
            } else if token == "2024" || token == "2025" {
                s.editions[1] += weight;
            }
        }
    }
    for heading in &front.headings {
        let norm = normalize_text(heading);
        for (i, patterns) in [
            &["players handbook", "player handbook"][..],
            &["dungeon masters guide", "dungeon master guide"][..],
            &["monster manual"][..],
        ]
        .iter()
        .enumerate()
        {
            if patterns.iter().any(|p| title_hits(&norm, p)) {
                s.books[i] += 1;
            }
        }
    }
    s
}

fn book_ids() -> [&'static str; 3] {
    BOOK_KINDS
}

fn candidates(editions: &[&str], books: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for e in editions {
        for b in books {
            out.push(format!("full-{e}-{b}"));
        }
    }
    out
}

/// Classify a book from its content signature. The rule is deliberately
/// conservative: a missing/conflicting signal yields `Ambiguous`/`Unknown`,
/// never a guess.
fn classify(
    front: &FrontMatter,
) -> (
    BookStatus,
    Option<String>,
    Option<String>,
    Vec<String>,
    String,
) {
    let s = signals(front);
    let top = s
        .books
        .iter()
        .enumerate()
        .max_by_key(|(_, score)| **score)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let top_score = s.books[top];
    let second_score = s
        .books
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != top)
        .map(|(_, score)| *score)
        .max()
        .unwrap_or(0);
    let book_choice = (top_score >= 18 && top_score >= second_score + 8).then_some(book_ids()[top]);

    let (e0, e1) = (s.editions[0], s.editions[1]);
    let edition_choice = if e0 >= 12 && e0 >= e1 + 8 {
        Some(EDITIONS[0])
    } else if e1 >= 12 && e1 >= e0 + 8 {
        Some(EDITIONS[1])
    } else {
        None
    };

    let matched_books: Vec<&str> = s
        .books
        .iter()
        .enumerate()
        .filter(|(_, score)| **score > 0)
        .map(|(i, _)| book_ids()[i])
        .collect();
    let matched_editions: Vec<&str> = s
        .editions
        .iter()
        .enumerate()
        .filter(|(_, score)| **score > 0)
        .map(|(i, _)| EDITIONS[i])
        .collect();

    if s.srd >= 18 && s.srd > top_score {
        return (
            BookStatus::SrdOnly,
            None,
            None,
            Vec::new(),
            format!(
                "pages={}, title=system-reference-document",
                front.pages.len()
            ),
        );
    }

    let page_count = front
        .pages
        .iter()
        .map(|(page, _)| *page)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let evidence = format!(
        "pages={page_count}, title={}, year={}",
        if matched_books.is_empty() {
            "none".to_string()
        } else {
            matched_books.join(",")
        },
        if matched_editions.is_empty() {
            "none".to_string()
        } else {
            matched_editions.join(",")
        }
    );

    match (book_choice, edition_choice) {
        (Some(book), Some(edition)) => (
            BookStatus::Identified,
            Some(edition.to_string()),
            Some(book.to_string()),
            Vec::new(),
            evidence,
        ),
        (Some(book), None) => {
            let editions = if matched_editions.is_empty() {
                EDITIONS.to_vec()
            } else {
                matched_editions
            };
            (
                BookStatus::Ambiguous,
                None,
                Some(book.to_string()),
                candidates(&editions, &[book]),
                evidence,
            )
        }
        (None, Some(edition)) => {
            let books = if matched_books.is_empty() {
                book_ids().to_vec()
            } else {
                matched_books
            };
            (
                BookStatus::Ambiguous,
                Some(edition.to_string()),
                None,
                candidates(&[edition], &books),
                evidence,
            )
        }
        (None, None) => {
            let editions = if matched_editions.is_empty() {
                EDITIONS.to_vec()
            } else {
                matched_editions
            };
            let books = if matched_books.is_empty() {
                book_ids().to_vec()
            } else {
                matched_books
            };
            (
                BookStatus::Unknown,
                None,
                None,
                candidates(&editions, &books),
                evidence,
            )
        }
    }
}

fn index_stamp(dir: &Path) -> String {
    let Some(p) = paged_markdown(dir) else {
        return String::new();
    };
    let meta = fs::metadata(&p);
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{mtime}:{len}")
}

fn has_pdf(folder: &Path) -> bool {
    sorted_entries(folder).into_iter().any(|p| is_pdf(&p))
}

/// Scan `Rules/2014/` and `Rules/2024/` and identify each converted book from
/// its first pages. The folder and file names are never used as evidence.
pub fn scan_books(root: &Path) -> Result<Vec<BookRecord>> {
    let mut out = Vec::new();
    for layer in EDITIONS {
        let category = root.join("Rules").join(layer);
        for folder in book_folders(&category) {
            let indexes = book_indexes(&folder);
            let pdf = has_pdf(&folder);
            if indexes.is_empty() {
                if pdf {
                    out.push(BookRecord {
                        edition: None,
                        book: None,
                        status: BookStatus::MissingIndex,
                        source: rel_dir(root, &folder),
                        method: "content".into(),
                        evidence: "pdf_without_index".into(),
                        stamp: String::new(),
                        candidates: Vec::new(),
                    });
                }
                continue;
            }
            if !pdf {
                for dir in indexes {
                    out.push(BookRecord {
                        edition: None,
                        book: None,
                        status: BookStatus::MissingIndex,
                        source: rel_dir(root, &dir),
                        method: "content".into(),
                        evidence: "index_without_pdf".into(),
                        stamp: index_stamp(&dir),
                        candidates: Vec::new(),
                    });
                }
                continue;
            }
            for dir in indexes {
                let source = rel_dir(root, &dir);
                let stamp = index_stamp(&dir);
                let front = match front_matter(&dir) {
                    Ok(Some(f)) if !f.pages.is_empty() => f,
                    Ok(_) => {
                        out.push(BookRecord {
                            edition: None,
                            book: None,
                            status: BookStatus::MissingIndex,
                            source,
                            method: "content".into(),
                            evidence: "no_paged_markdown".into(),
                            stamp,
                            candidates: Vec::new(),
                        });
                        continue;
                    }
                    Err(_) => {
                        out.push(BookRecord {
                            edition: None,
                            book: None,
                            status: BookStatus::Unknown,
                            source,
                            method: "content".into(),
                            evidence: "unreadable_front_matter".into(),
                            stamp,
                            candidates: Vec::new(),
                        });
                        continue;
                    }
                };
                let (status, edition, book, candidates, evidence) = classify(&front);
                out.push(BookRecord {
                    edition,
                    book,
                    status,
                    source,
                    method: "content".into(),
                    evidence,
                    stamp,
                    candidates,
                });
            }
        }
    }
    out.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(out)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BookRegistry {
    #[serde(default = "books_version")]
    version: u32,
    #[serde(default)]
    books: Vec<BookRecord>,
}

fn books_version() -> u32 {
    BOOKS_VERSION
}

/// The last scan's records (audit: source, method, evidence, pack state).
pub fn load_books(root: &Path) -> Vec<BookRecord> {
    fs::read_to_string(root.join(BOOKS_FILE))
        .ok()
        .and_then(|t| serde_norway::from_str::<BookRegistry>(&t).ok())
        .map(|r| r.books)
        .unwrap_or_default()
}

fn save_books(root: &Path, books: &[BookRecord]) -> Result<()> {
    let text = format!(
        "# TalkDND book identification (local, never commit). Contents are book\n# type/edition/status only; never book text or absolute paths.\n{}",
        serde_norway::to_string(&BookRegistry {
            version: BOOKS_VERSION,
            books: books.to_vec(),
        })?
    );
    combat_sim::fsutil::atomic_write(root.join(BOOKS_FILE), text.as_bytes())
}

/// Scan and register, keeping a player's manual choice while the converted
/// source is unchanged. Idempotent: an unchanged scan writes nothing.
pub fn scan_and_register(root: &Path) -> Result<Vec<BookRecord>> {
    let old = load_books(root);
    let mut fresh = scan_books(root)?;
    for record in &mut fresh {
        if matches!(record.status, BookStatus::Ambiguous | BookStatus::Unknown) {
            if let Some(manual) = old.iter().find(|o| {
                o.method == "manual"
                    && o.source == record.source
                    && !o.stamp.is_empty()
                    && o.stamp == record.stamp
            }) {
                *record = manual.clone();
            }
        }
    }
    if old != fresh {
        save_books(root, &fresh)?;
    }
    Ok(fresh)
}

/// Record the player's explicit choice for an ambiguous/unknown converted
/// book. The source stamp is kept, so a later reconversion asks again.
pub fn choose_book(root: &Path, query: &str, edition: &str, kind: &str) -> Result<BookRecord> {
    let edition = Ruleset::parse(edition)
        .map(|ruleset| ruleset.edition())
        .map_err(|_| {
            anyhow::anyhow!(
                "无效的规则版本「{edition}」；可选 {} 或 2014 / 2024",
                Ruleset::VALUES.join(" / ")
            )
        })?;
    let kind = book_kind(kind)
        .ok_or_else(|| anyhow::anyhow!("无效的书型「{kind}」；可选 {}", BOOK_KINDS.join(" / ")))?;
    let mut records = scan_and_register(root)?;
    let needle = query.trim().trim_end_matches('/');
    if needle.is_empty() {
        bail!("请指定要识别的书：书库文件夹或 library scan 显示的 source");
    }
    let hits: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            let source = r.source.trim_end_matches('/');
            source == needle
                || source.ends_with(&format!("/{needle}"))
                || source.contains(&format!("/{needle}/"))
        })
        .map(|(i, _)| i)
        .collect();
    if hits.is_empty() {
        bail!(
            "没有找到「{query}」的转换索引；先运行 `talkdnd library scan`（目标：{edition} {}）",
            book_label(kind)
        );
    }
    if hits.len() > 1 {
        let sources: Vec<String> = hits.iter().map(|i| records[*i].source.clone()).collect();
        bail!(
            "「{query}」对应多份索引：{}；请用 library scan 显示的完整 source（目标：{edition} {}）",
            sources.join("、"),
            book_label(kind)
        );
    }
    let i = hits[0];
    if records[i].status == BookStatus::SrdOnly {
        bail!("{} 是 SRD 索引，不能当作完整书解锁", records[i].source);
    }
    if records[i].status == BookStatus::MissingIndex {
        bail!(
            "{} 缺少 PDF 或转换索引，不能手动解锁；先放入 PDF 并按 pdf2md 转换",
            records[i].source
        );
    }
    records[i].edition = Some(edition.to_string());
    records[i].book = Some(kind.to_string());
    records[i].status = BookStatus::Identified;
    records[i].method = "manual".into();
    records[i].evidence = "manual_choice".into();
    records[i].candidates.clear();
    let chosen = records[i].clone();
    save_books(root, &records)?;
    Ok(chosen)
}

// ------------------------------------------------------------ full packs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackState {
    Unlocked,
    MissingBook,
    MissingPack,
    WrongEdition,
    Ambiguous,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackStatus {
    /// `full-<edition>-<book>`.
    pub id: String,
    pub edition: String,
    pub book: String,
    pub state: PackState,
    /// Player-facing reason. Names the missing book/edition; never book text.
    pub detail: String,
    /// Pack file relative to the play root, when one was found.
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PackHeaderFile {
    pack: PackHeaderMeta,
}

#[derive(Debug, Deserialize)]
struct PackHeaderMeta {
    id: String,
    #[serde(default)]
    edition: String,
    #[serde(default)]
    distribution: combat_sim::content::Distribution,
    #[serde(default)]
    schema_version: u32,
}

fn pack_suffix(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".pack.json") || n.ends_with(".pack.yaml") || n.ends_with(".pack.yml")
}

fn pack_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for suffix in [".pack.json", ".pack.yaml", ".pack.yml"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name.to_string()
}

fn walk_pack_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for p in sorted_entries(dir) {
        if p.is_dir() {
            walk_pack_files(&p, out);
        } else if p.is_file() && pack_suffix(&name_of(&p)) {
            out.push(p);
        }
    }
}

fn read_pack_header(path: &Path) -> Result<PackHeaderMeta> {
    let text = fs::read_to_string(path).with_context(|| format!("read pack {}", path.display()))?;
    let header = if name_of(path).to_ascii_lowercase().ends_with(".json") {
        serde_json::from_str::<PackHeaderFile>(&text)
            .with_context(|| format!("parse pack {}", path.display()))?
    } else {
        serde_norway::from_str::<PackHeaderFile>(&text)
            .with_context(|| format!("parse pack {}", path.display()))?
    };
    Ok(header.pack)
}

fn pack_value(path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(path).with_context(|| format!("read pack {}", path.display()))?;
    if name_of(path).to_ascii_lowercase().ends_with(".json") {
        serde_json::from_str(&text).with_context(|| format!("parse pack {}", path.display()))
    } else {
        let value: serde_norway::Value = serde_norway::from_str(&text)
            .with_context(|| format!("parse pack {}", path.display()))?;
        serde_json::to_value(value).context("convert pack YAML to JSON")
    }
}

fn pack_statuses(root: &Path) -> Vec<PackStatus> {
    let records = load_books(root);
    let mut files = Vec::new();
    walk_pack_files(&root.join(FULL_PACK_DIR), &mut files);
    files.sort();
    let mut by_id: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    for path in files {
        if let Ok(header) = read_pack_header(&path) {
            by_id.entry(header.id).or_default().push(path);
        } else {
            // Keep an unreadable pack visible under its file name so the
            // player can fix it; it can never unlock by name.
            by_id
                .entry(pack_stem(&name_of(&path)))
                .or_default()
                .push(path);
        }
    }

    let mut out = Vec::new();
    for edition in EDITIONS {
        for book in BOOK_KINDS {
            let id = format!("full-{edition}-{book}");
            let label = book_label(book);
            let found = by_id.get(&id);
            let mut status = PackStatus {
                id: id.clone(),
                edition: edition.to_string(),
                book: book.to_string(),
                state: PackState::MissingBook,
                detail: String::new(),
                file: None,
            };
            if let Some(paths) = found {
                status.file = paths.first().map(|p| rel(root, p));
                if paths.len() > 1 {
                    status.state = PackState::Invalid;
                    status.detail =
                        format!("重复的内容包 id（{} 个文件）；只保留一个", paths.len());
                    out.push(status);
                    continue;
                }
                let path = &paths[0];
                match read_pack_header(path) {
                    Ok(header) => {
                        if header.id != id
                            || header.edition != edition
                            || header.distribution != combat_sim::content::Distribution::Full
                            || header.schema_version != combat_sim::content::pack::SCHEMA_VERSION
                        {
                            status.state = PackState::Invalid;
                            status.detail = format!(
                                "内容包不是有效的 {edition} 完整书（id={}，edition={}，distribution={}，schema={}）",
                                header.id,
                                if header.edition.is_empty() {
                                    "missing"
                                } else {
                                    &header.edition
                                },
                                header.distribution.as_str(),
                                header.schema_version
                            );
                            out.push(status);
                            continue;
                        }
                        let valid = pack_value(path)
                            .and_then(|v| combat_sim::content::ContentPack::from_value(&v))
                            .map(|_| ());
                        if let Err(e) = valid {
                            status.state = PackState::Invalid;
                            status.detail = format!("内容包校验失败：{e}");
                            out.push(status);
                            continue;
                        }
                    }
                    Err(_) => {
                        status.state = PackState::Invalid;
                        status.detail = format!("内容包读不了：{}（应为 {id}）", rel(root, path));
                        out.push(status);
                        continue;
                    }
                }
            }

            let identified = records.iter().any(|r| {
                r.status == BookStatus::Identified
                    && r.edition.as_deref() == Some(edition)
                    && r.book.as_deref() == Some(book)
            });
            if identified {
                if status.file.is_some() {
                    status.state = PackState::Unlocked;
                    status.detail = format!("{edition} {label}");
                } else {
                    status.state = PackState::MissingPack;
                    status.detail = format!(
                        "已识别 {edition} {label}，但缺少完整书内容包（放到 {FULL_PACK_DIR}/{id}.pack.json）"
                    );
                }
                out.push(status);
                continue;
            }

            let other_editions: Vec<&str> = records
                .iter()
                .filter(|r| {
                    r.status == BookStatus::Identified
                        && r.book.as_deref() == Some(book)
                        && r.edition.as_deref() != Some(edition)
                })
                .filter_map(|r| r.edition.as_deref())
                .collect();
            let ambiguous: Vec<&BookRecord> = records
                .iter()
                .filter(|r| {
                    r.status != BookStatus::Identified && r.candidates.iter().any(|c| c == &id)
                })
                .collect();
            if !other_editions.is_empty() {
                status.state = PackState::WrongEdition;
                status.detail = format!(
                    "只有 {} 的 {label}；{id} 需要 {edition} 版（运行 `talkdnd library scan` 查看）",
                    other_editions.join("、")
                );
            } else if !ambiguous.is_empty() {
                status.state = PackState::Ambiguous;
                let sources: Vec<&str> = ambiguous.iter().map(|r| r.source.as_str()).collect();
                status.detail = format!(
                    "识别不确定（{}）；运行 `talkdnd library choose <书> --edition {edition} --kind {book}`",
                    sources.join("、")
                );
            } else {
                status.state = PackState::MissingBook;
                let missing_index = records.iter().any(|r| r.status == BookStatus::MissingIndex);
                status.detail = if missing_index {
                    format!(
                        "缺少 {edition} {label} 的已识别 PDF（有的 PDF 还没有转换索引；运行 `talkdnd library scan`）"
                    )
                } else {
                    format!(
                        "缺少 {edition} {label} 的已识别 PDF（Rules/{edition}/；运行 `talkdnd library scan`）"
                    )
                };
            }
            out.push(status);
        }
    }
    out
}

/// Status of the six expected full packs: unlocked, missing book/edition,
/// missing pack, ambiguous, or invalid. SRD packs are always available and
/// are not listed here.
pub fn pack_status(root: &Path) -> Vec<PackStatus> {
    pack_statuses(root)
}

/// Rewrite the save's `library:` list so a FULL ruleset contains exactly the
/// full packs unlocked for its edition, plus every unmanaged entry it already
/// had. An SRD ruleset removes the managed full packs. Unmatched packs are
/// never added, and the next write also migrates an old ruleset value.
pub fn sync_campaign_packs(root: &Path, save: &Path) -> Result<Vec<String>> {
    crate::save::migrate_ruleset(save)?;
    let ruleset = crate::save::ruleset(save)?;
    let rs = ruleset.edition();
    let statuses = pack_status(root);
    let mut loaded = Vec::new();
    let mut files = Vec::new();
    if ruleset.is_full() {
        for status in &statuses {
            if status.edition == rs && status.state == PackState::Unlocked {
                if let Some(file) = &status.file {
                    loaded.push(format!("{} {}", status.edition, book_label(&status.book)));
                    files.push(root.join(file));
                }
            }
        }
    }
    let manifest = save.join("combat/campaign.yaml");
    let text = fs::read_to_string(&manifest).unwrap_or_default();
    if text.is_empty() {
        return Ok(loaded);
    }
    let base = manifest.parent().unwrap_or(save);
    let mut entries = Vec::new();
    let mut replaced = false;
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with("library:") {
            replaced = true;
            let indent = &line[..line.len() - trimmed.len()];
            let rest = trimmed["library:".len()..].to_string();
            let (value, comment) = match rest.find('#') {
                Some(i) => (
                    rest[..i].to_string(),
                    Some(rest[i..].trim_end().to_string()),
                ),
                None => (rest, None),
            };
            for entry in parse_yaml_list(&value) {
                if !managed_pack_entry(&entry) {
                    entries.push(entry);
                }
            }
            for file in &files {
                entries.push(relative_from(base, file));
            }
            let mut new_line = format!("{indent}library: [{}]", entries.join(", "));
            if let Some(c) = comment {
                new_line.push(' ');
                new_line.push_str(&c);
            }
            out.push_str(&new_line);
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    if replaced && out != text {
        combat_sim::fsutil::atomic_write(&manifest, out.as_bytes())?;
    }
    Ok(loaded)
}

fn parse_yaml_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_matches(['[', ']'])
        .split(',')
        .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Entries this command manages: a `full-*` pack under `Rules/packs/`.
/// Other pack entries (SRD/local) are the player's and must survive a sync.
fn managed_pack_entry(entry: &str) -> bool {
    let norm = entry.replace('\\', "/");
    if !norm.contains("Rules/packs/") {
        return false;
    }
    let base = norm.rsplit('/').next().unwrap_or("");
    pack_stem(base).starts_with("full-")
}

/// `base` to `target`, both under the play root, with `/` separators.
fn relative_from(base: &Path, target: &Path) -> String {
    let from: Vec<std::path::Component> = base.components().collect();
    let to: Vec<std::path::Component> = target.components().collect();
    let common = from
        .iter()
        .zip(to.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = Vec::new();
    for _ in common..from.len() {
        parts.push("..".into());
    }
    for c in &to[common..] {
        parts.push(c.as_os_str().to_string_lossy().to_string());
    }
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// A one-line digest: which full packs the save loads, or exactly which book
/// or pack is missing. Never book prose.
pub fn pack_digest_line(root: &Path, save: &Path) -> Result<String> {
    let ruleset = crate::save::ruleset(save)?;
    let rs = ruleset.edition();
    let statuses = pack_status(root);
    if ruleset.is_srd() {
        return Ok(format!(
            "仅 SRD（{} 只使用 SRD 层，不加载完整书内容包）",
            ruleset.display()
        ));
    }
    let loaded: Vec<&PackStatus> = statuses
        .iter()
        .filter(|s| s.edition == rs && s.state == PackState::Unlocked)
        .collect();
    if !loaded.is_empty() {
        return Ok(format!(
            "已解锁 {}（SRD 仍是基础层）",
            loaded
                .iter()
                .map(|s| format!("{} {}", s.edition, book_label(&s.book)))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    let ambiguous = statuses
        .iter()
        .filter(|s| s.edition == rs && s.state == PackState::Ambiguous)
        .count();
    let missing: Vec<&PackStatus> = statuses
        .iter()
        .filter(|s| s.edition == rs && s.state == PackState::MissingPack)
        .collect();
    let missing_book: Vec<&PackStatus> = statuses
        .iter()
        .filter(|s| s.edition == rs && s.state == PackState::MissingBook)
        .collect();
    let invalid: Vec<&PackStatus> = statuses
        .iter()
        .filter(|s| s.edition == rs && s.state == PackState::Invalid)
        .collect();
    Ok(if ambiguous > 0 {
        format!(
            "未解锁（{} 需要至少一本完整书；{ambiguous} 本书无法确定，运行 `talkdnd library scan` 选择）",
            ruleset.display()
        )
    } else if !missing.is_empty() {
        format!(
            "未解锁（{} 已识别 {}，但缺少 {}；运行 `talkdnd library packs`）",
            ruleset.display(),
            missing
                .iter()
                .map(|s| format!("{} {}", s.edition, book_label(&s.book)))
                .collect::<Vec<_>>()
                .join("、"),
            missing
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join("、")
        )
    } else if !invalid.is_empty() {
        format!(
            "未解锁（{} 的内容包无效：{}；运行 `talkdnd library packs`）",
            ruleset.display(),
            invalid
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join("、")
        )
    } else if !missing_book.is_empty() {
        format!(
            "未解锁（{} 未识别到 {} 的完整书 PDF；运行 `talkdnd library scan`）",
            ruleset.display(),
            missing_book
                .iter()
                .map(|s| book_label(&s.book))
                .collect::<Vec<_>>()
                .join("、")
        )
    } else {
        format!(
            "未解锁（{} 未识别到匹配的完整书 PDF；运行 `talkdnd library scan`）",
            ruleset.display()
        )
    })
}

/// FULL rulesets fail closed: at least one full pack for the save's edition
/// must be unlocked. The error names every missing book and how to add it.
pub fn require_full_packs(root: &Path, ruleset: Ruleset) -> Result<()> {
    if !ruleset.is_full() {
        return Ok(());
    }
    let edition = ruleset.edition();
    let statuses = pack_status(root);
    let unlocked = statuses
        .iter()
        .filter(|status| status.edition == edition && status.state == PackState::Unlocked)
        .count();
    if unlocked > 0 {
        return Ok(());
    }
    let mut details = String::new();
    for status in statuses.iter().filter(|status| status.edition == edition) {
        details.push_str(&format!(
            "\n  - {}：{}",
            book_label(&status.book),
            status.detail
        ));
    }
    let alternative = if edition == "2014" {
        "5e SRD"
    } else {
        "5.5e SRD"
    };
    bail!(
        "{} 需要至少一本已解锁的 {edition} 完整规则书（Player's Handbook / Dungeon Master's Guide / Monster Manual），当前一本都没有解锁。{details}\n把对应 PDF 放进 Rules/{edition}/<书名>/ 并运行 `talkdnd library scan`；完整书内容包放到 Rules/packs/full-{edition}-<phb|dmg|mm>.pack.json；`talkdnd library packs` 查看每本书的状态。也可以改用 {alternative}。",
        ruleset.display()
    )
}

/// Program data under `Rules/` that is not a book.
pub const RULES_PROGRAM_DIRS: [&str; 3] = ["chargen", "combat", "defaults"];
/// Book-folder content that is not a book file.
const BOOK_SUBDIRS: [&str; 2] = [INDEX_DIR, "combat"];
/// Every move tidy made, so saves can be repointed (local state, never committed).
pub const MOVES_FILE: &str = ".talkdnd/library-moves.yaml";
const PDF2MD_LOCK: &str = ".pdf2md/worker.lock";
const PDF2MD_LOCK_STALE_SECS: u64 = 600;

/// Folder name for a book: the PDF file name without `.pdf`, lowercased ASCII
/// letters and digits, apostrophes dropped (`King's` -> `kings`), every other
/// run of characters turned into one hyphen, no leading/trailing hyphen.
/// A name with no ASCII letters or digits becomes `book-<8 hex>` (FNV-1a of
/// the name) so different books still get different folders.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().chars() {
        if matches!(ch, '\'' | '\u{2019}') {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        let mut h: u32 = 0x811c9dc5;
        for b in name.trim().bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x01000193);
        }
        format!("book-{h:08x}")
    } else {
        out
    }
}

fn is_ext(p: &Path, ext: &str) -> bool {
    p.extension()
        .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case(ext))
}

pub fn is_pdf(p: &Path) -> bool {
    p.is_file() && is_ext(p, "pdf")
}

fn stem_of(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

/// A pdf2md output folder (new `index/` or legacy `.md/<Stem>/`).
pub fn is_index_dir(d: &Path) -> bool {
    d.is_dir()
        && (d.join("manifest.json").is_file()
            || d.join("index.json").is_file()
            || paged_files(d).next().is_some()
            // pdf2md < 0.4 wrote only `.md/<Stem>/<Stem>.md`.
            || d.join(format!("{}.md", name_of(d))).is_file())
}

fn paged_files(d: &Path) -> impl Iterator<Item = PathBuf> {
    sorted_entries(d)
        .into_iter()
        .filter(|p| p.is_file() && name_of(p).ends_with(".pages.md"))
}

/// Book stem of an index folder: the manifest's PDF name, else the paged
/// Markdown's name, else the legacy folder name.
pub fn index_stem(d: &Path) -> String {
    if let Ok(text) = fs::read_to_string(d.join("manifest.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(pdf) = v.get("pdf").and_then(|x| x.as_str()) {
                return stem_of(Path::new(pdf));
            }
        }
    }
    if let Some(p) = paged_files(d).next() {
        return name_of(&p).trim_end_matches(".pages.md").to_string();
    }
    let n = name_of(d);
    if n == INDEX_DIR {
        d.parent().map(name_of).unwrap_or(n)
    } else {
        n
    }
}

/// The one Markdown edition of a converted book: `<Stem>.pages.md` (PDF page
/// anchors). Falls back to a legacy page-less `<Stem>.md` only when no paged
/// edition exists (an index from a pdf2md older than 0.4).
pub fn paged_markdown(d: &Path) -> Option<PathBuf> {
    let stem = index_stem(d);
    let named = d.join(format!("{stem}.pages.md"));
    if named.is_file() {
        return Some(named);
    }
    if let Some(p) = paged_files(d).next() {
        return Some(p);
    }
    let reader = d.join(format!("{stem}.md"));
    reader.is_file().then_some(reader)
}

/// Paged Markdown without the page markers (what the reader edition used to
/// contain), for parsers that do not care about page numbers.
pub fn strip_page_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.split_inclusive('\n') {
        if is_page_marker(line) {
            continue;
        }
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
    }
    out
}

/// Read a book's Markdown for parsing (page markers removed).
pub fn read_text(p: &Path) -> std::io::Result<String> {
    fs::read_to_string(p).map(|t| strip_page_markers(&t))
}

/// Index folders of one book folder: `index/`, then legacy `.md/*/`.
pub fn book_indexes(book: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let idx = book.join(INDEX_DIR);
    if is_index_dir(&idx) {
        v.push(idx);
    }
    for d in sorted_entries(&book.join(LEGACY_INDEX_DIR)) {
        if is_index_dir(&d) {
            v.push(d);
        }
    }
    v
}

/// Book folders directly under a category folder (`Adventures/`,
/// `Rules/<edition>/`): every visible sub-folder.
pub fn book_folders(category: &Path) -> Vec<PathBuf> {
    sorted_entries(category)
        .into_iter()
        .filter(|p| p.is_dir() && !name_of(p).starts_with('.'))
        .collect()
}

/// (book key, index folder) for a whole category, new and legacy layouts.
/// The key is the book folder name, or for a legacy index next to a loose
/// PDF, the slug of its stem.
pub fn category_indexes(category: &Path) -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    for book in book_folders(category) {
        let key = name_of(&book);
        for d in book_indexes(&book) {
            v.push((key.clone(), d));
        }
    }
    for d in sorted_entries(&category.join(LEGACY_INDEX_DIR)) {
        if is_index_dir(&d) {
            v.push((slug(&name_of(&d)), d));
        }
    }
    v
}

/// Index folders of one ruleset's books (`Rules/<rs>/<slug>/index/`).
pub fn rule_indexes(root: &Path, rs: &str) -> Vec<PathBuf> {
    category_indexes(&root.join("Rules").join(rs))
        .into_iter()
        .map(|(_, d)| d)
        .collect()
}

/// Paged Markdown of every converted rulebook of a ruleset.
pub fn rule_books(root: &Path, rs: &str) -> Vec<PathBuf> {
    rule_indexes(root, rs)
        .iter()
        .filter_map(|d| paged_markdown(d))
        .collect()
}

/// How well a book key matches the adventure the save asked for:
/// 0 exact, 1 equal without a leading `the-`, 2 a part (`<want>-a`,
/// `<want>-2`, `<want>-part-2`, `<want>-vol-1`).
pub fn key_rank(key: &str, want: &str) -> Option<u8> {
    let key = slug(key);
    let want = slug(want);
    if key == want {
        return Some(0);
    }
    let bare = |s: &str| s.strip_prefix("the-").unwrap_or(s).to_string();
    let (k, w) = (bare(&key), bare(&want));
    if k == w {
        return Some(1);
    }
    let rest = k.strip_prefix(&format!("{w}-"))?;
    let part = rest
        .strip_prefix("part-")
        .or_else(|| rest.strip_prefix("vol-"))
        .or_else(|| rest.strip_prefix("book-"))
        .unwrap_or(rest);
    let single_letter = part.len() == 1 && part.chars().all(|c| c.is_ascii_lowercase());
    let digits = !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    (single_letter || digits).then_some(2)
}

/// Index folders of the adventure `want` (a save's adventure slug or a title),
/// best match first; a book split into several PDFs yields every part.
pub fn adventure_indexes(root: &Path, want: &str) -> Vec<PathBuf> {
    let mut v: Vec<(u8, String, PathBuf)> = category_indexes(&root.join("Adventures"))
        .into_iter()
        .filter_map(|(k, d)| key_rank(&k, want).map(|r| (r, k, d)))
        .collect();
    v.sort();
    v.into_iter().map(|(_, _, d)| d).collect()
}

/// Book folders of the adventure `want`, best match first.
pub fn adventure_folders(root: &Path, want: &str) -> Vec<PathBuf> {
    let mut v: Vec<(u8, PathBuf)> = book_folders(&root.join("Adventures"))
        .into_iter()
        .filter_map(|d| key_rank(&name_of(&d), want).map(|r| (r, d)))
        .collect();
    v.sort();
    v.into_iter().map(|(_, d)| d).collect()
}

/// Folder name for a save's adventure: an existing book folder that is the
/// same book (exact slug, or differing only by a leading `the-`), else the
/// slug of the title. Keeps a save's `combat/` next to the tidied PDF.
pub fn adventure_folder_name(root: &Path, title: &str) -> String {
    let want = crate::chargen::slug(title);
    book_folders(&root.join("Adventures"))
        .into_iter()
        .map(|d| name_of(&d))
        .filter_map(|n| key_rank(&n, &want).filter(|r| *r <= 1).map(|r| (r, n)))
        .min()
        .map(|(_, n)| n)
        .unwrap_or(want)
}

/// PDFs that belong to the adventure `want` (in its folders, or still loose).
pub fn adventure_pdfs(root: &Path, want: &str) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for d in adventure_folders(root, want) {
        v.extend(sorted_entries(&d).into_iter().filter(|p| is_pdf(p)));
    }
    for p in sorted_entries(&root.join("Adventures")) {
        if is_pdf(&p) && key_rank(&stem_of(&p), want).is_some() {
            v.push(p);
        }
    }
    v
}

// ---------------------------------------------------------------- tidy

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Move {
    /// Paths relative to the play folder, `/`-separated. A folder ends in `/`.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MovesLog {
    #[serde(default)]
    moves: Vec<Move>,
}

#[derive(Debug, Default)]
pub struct TidyReport {
    pub dry_run: bool,
    pub moves: Vec<Move>,
    /// Redundant page-less reader editions removed (path, bytes).
    pub readers: Vec<(String, u64)>,
    pub warnings: Vec<String>,
    /// Skipped entirely (pdf2md busy).
    pub skipped: Option<String>,
}

impl TidyReport {
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
            && self.readers.is_empty()
            && self.warnings.is_empty()
            && self.skipped.is_none()
    }

    pub fn reader_bytes(&self) -> u64 {
        self.readers.iter().map(|(_, b)| b).sum()
    }

    pub fn lines(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(s) = &self.skipped {
            v.push(format!("书库整理：跳过（{s}）"));
            return v;
        }
        let verb = if self.dry_run {
            "会移动"
        } else {
            "已移动"
        };
        for m in &self.moves {
            v.push(format!("书库整理：{verb} `{}` → `{}`", m.from, m.to));
        }
        if !self.readers.is_empty() {
            v.push(format!(
                "书库整理：{} {} 个多余的无页码阅读版（已有页码版 .pages.md），共 {} 字节",
                if self.dry_run {
                    "会删除"
                } else {
                    "已删除"
                },
                self.readers.len(),
                self.reader_bytes()
            ));
            for (p, b) in &self.readers {
                v.push(format!("  - `{p}`（{b} 字节）"));
            }
        }
        for w in &self.warnings {
            v.push(format!("书库整理：注意：{w}"));
        }
        if v.is_empty() {
            v.push("书库整理：Adventures/ 与 Rules/ 已符合一书一文件夹，无需改动".into());
        }
        v
    }
}

/// Result of folding split-volume folders into the full-book folder.
#[derive(Debug, Default)]
pub struct MergeReport {
    pub dry_run: bool,
    pub canonical: Option<String>,
    pub archived: Vec<Move>,
    pub combat: Vec<Move>,
    pub warnings: Vec<String>,
    pub skipped: Option<String>,
}

impl MergeReport {
    pub fn lines(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(s) = &self.skipped {
            v.push(format!("书库合并：跳过（{s}）"));
            return v;
        }
        let Some(canonical) = &self.canonical else {
            v.push("书库合并：没有找到整本文件夹；先把整合版 PDF 放进去再运行".into());
            return v;
        };
        v.push(format!("书库合并：整本文件夹 `{canonical}`"));
        if self.archived.is_empty() && self.combat.is_empty() && self.warnings.is_empty() {
            v.push("书库合并：没有分卷，无需合并".into());
            return v;
        }
        let verb = if self.dry_run {
            "会归档"
        } else {
            "已归档"
        };
        for m in &self.archived {
            v.push(format!("书库合并：{verb} `{}` → `{}`", m.from, m.to));
        }
        let verb = if self.dry_run {
            "会移动"
        } else {
            "已移动"
        };
        for m in &self.combat {
            v.push(format!("书库合并：{verb} `{}` → `{}`", m.from, m.to));
        }
        for w in &self.warnings {
            v.push(format!("书库合并：注意：{w}"));
        }
        v
    }
}

/// Fold `<slug>-a`, `<slug>-b`, `<slug>-2` ... into the full `<slug>`
/// folder. Part folders are moved, never deleted, into
/// `.talkdnd/book-backups/merge-<slug>/`; a part's `combat/` is moved into
/// the full folder only when that folder has none.
pub fn merge_parts(root: &Path, book: &str, dry_run: bool) -> Result<MergeReport> {
    let mut report = MergeReport {
        dry_run,
        ..Default::default()
    };
    if pdf2md_busy(root) {
        report.skipped =
            Some("pdf2md 正在后台转换；等它结束后再运行 `talkdnd library merge`".into());
        return Ok(report);
    }
    let want = slug(book);
    let adventure_root = root.join("Adventures");
    let mut full: Vec<(u8, PathBuf)> = Vec::new();
    let mut parts: Vec<PathBuf> = Vec::new();
    for d in book_folders(&adventure_root) {
        match key_rank(&name_of(&d), &want) {
            Some(rank @ (0 | 1)) => full.push((rank, d)),
            Some(2) => parts.push(d),
            _ => {}
        }
    }
    full.sort();
    let Some((_, canonical)) = full.into_iter().next() else {
        anyhow::bail!(
            "没有找到 `{want}` 的整本文件夹；先把整合版 PDF 放进 `Adventures/{want}/` 再运行"
        )
    };
    report.canonical = Some(rel_dir(root, &canonical));
    if parts.is_empty() {
        return Ok(report);
    }
    if !sorted_entries(&canonical).into_iter().any(|p| is_pdf(&p)) {
        anyhow::bail!(
            "`{}` 里没有 PDF；先放入整合版 PDF",
            rel_dir(root, &canonical)
        );
    }
    let backup = next_merge_backup(root, &want);
    let mut logical = Vec::new();
    for part in &parts {
        let part_combat = part.join("combat");
        let canonical_combat = canonical.join("combat");
        if part_combat.is_dir() {
            if canonical_combat.exists() {
                report.warnings.push(format!(
                    "`{}` 与整本都有 `combat/`；分卷留在备份，未合并战斗数据",
                    rel_dir(root, part)
                ));
            } else {
                let m = Move {
                    from: rel_dir(root, &part_combat),
                    to: rel_dir(root, &canonical_combat),
                };
                if !dry_run {
                    fs::rename(&part_combat, &canonical_combat)
                        .with_context(|| format!("move {} -> {}", m.from, m.to))?;
                }
                report.combat.push(m);
            }
        }
        let dst = backup.join(name_of(part));
        let m = Move {
            from: rel_dir(root, part),
            to: rel_dir(root, &dst),
        };
        if !dry_run {
            fs::create_dir_all(&backup).with_context(|| format!("create {}", backup.display()))?;
            fs::rename(part, &dst).with_context(|| format!("move {} -> {}", m.from, m.to))?;
        }
        report.archived.push(m);
        // A save that pointed at the part must now point at the surviving full
        // book, not at the backup.
        logical.push(Move {
            from: rel_dir(root, part),
            to: rel_dir(root, &canonical),
        });
    }
    if !dry_run {
        record_moves(root, &logical)?;
    }
    Ok(report)
}

fn next_merge_backup(root: &Path, want: &str) -> PathBuf {
    let base = root.join(".talkdnd/book-backups");
    let first = base.join(format!("merge-{want}"));
    if !first.exists() {
        return first;
    }
    let mut n = 2u32;
    loop {
        let p = base.join(format!("merge-{want}-{n}"));
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

/// Warn when a full-book folder and its split volumes both exist. A book that
/// only has parts is a legitimate multi-volume set, so it is not reported.
fn merge_warnings(adventure_root: &Path) -> Vec<String> {
    let folders = book_folders(adventure_root);
    let mut out = Vec::new();
    for d in &folders {
        let name = name_of(d);
        let mut parts: Vec<String> = folders
            .iter()
            .filter(|o| key_rank(&name_of(o), &name) == Some(2))
            .map(|o| format!("`Adventures/{}`", name_of(o)))
            .collect();
        if parts.is_empty() {
            continue;
        }
        parts.sort();
        out.push(format!(
            "同一本书既有整本也有分卷：`Adventures/{name}` 与 {}；确认整本完整后运行 `talkdnd library merge {name}`",
            parts.join("、")
        ));
    }
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn rel_dir(root: &Path, p: &Path) -> String {
    format!("{}/", rel(root, p).trim_end_matches('/'))
}

/// Is a pdf2md worker running right now (lock held and fresh)?
pub fn pdf2md_busy(root: &Path) -> bool {
    let lock = root.join(PDF2MD_LOCK);
    let Ok(meta) = fs::metadata(&lock) else {
        return false;
    };
    let text = fs::read_to_string(&lock).unwrap_or_default();
    let owner: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if owner.get("released").and_then(|v| v.as_bool()) == Some(true) {
        return false;
    }
    let fresh = meta
        .modified()
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < PDF2MD_LOCK_STALE_SECS);
    if !fresh {
        return false;
    }
    if cfg!(target_os = "linux") {
        if let Some(pid) = owner.get("pid").and_then(|v| v.as_u64()) {
            return Path::new(&format!("/proc/{pid}")).exists();
        }
    }
    true
}

/// All PDFs under a category, skipping hidden folders and each book folder's
/// `index/` and `combat/`.
fn find_pdfs(dir: &Path, top: bool, out: &mut Vec<PathBuf>) {
    for p in sorted_entries(dir) {
        let name = name_of(&p);
        if p.is_dir() {
            if name.starts_with('.') || (!top && BOOK_SUBDIRS.contains(&name.as_str())) {
                continue;
            }
            if top && BOOK_SUBDIRS.contains(&name.as_str()) && is_index_dir(&p) {
                continue;
            }
            find_pdfs(&p, false, out);
        } else if is_pdf(&p) {
            out.push(p);
        }
    }
}

struct Tidy<'a> {
    root: &'a Path,
    dry_run: bool,
    report: TidyReport,
    /// Destinations already claimed in this run (dry-run has no disk effect).
    claimed: Vec<PathBuf>,
    /// Sources already moved in this run.
    gone: Vec<PathBuf>,
}

impl Tidy<'_> {
    fn exists(&self, p: &Path) -> bool {
        (p.exists() && !self.gone.iter().any(|g| p.starts_with(g)))
            || self.claimed.iter().any(|c| c == p)
    }

    fn mv(&mut self, from: &Path, to: &Path, dir: bool) -> Result<()> {
        let (f, t) = if dir {
            (rel_dir(self.root, from), rel_dir(self.root, to))
        } else {
            (rel(self.root, from), rel(self.root, to))
        };
        if !self.dry_run {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            fs::rename(from, to)
                .with_context(|| format!("move {} -> {}", from.display(), to.display()))?;
        }
        self.claimed.push(to.to_path_buf());
        self.gone.push(from.to_path_buf());
        self.report.moves.push(Move { from: f, to: t });
        Ok(())
    }

    fn warn(&mut self, w: String) {
        if !self.report.warnings.contains(&w) {
            self.report.warnings.push(w);
        }
    }

    /// Tidy one category folder (`Adventures/` or `Rules/<edition>/`).
    fn category(&mut self, cat: &Path) -> Result<()> {
        if !cat.is_dir() {
            return Ok(());
        }
        let mut pdfs = Vec::new();
        find_pdfs(cat, true, &mut pdfs);
        for pdf in pdfs {
            self.pdf(cat, &pdf)?;
        }
        // Leftover legacy indexes (PDF already in place, or missing).
        for book in book_folders(cat) {
            if self.gone.iter().any(|g| book.starts_with(g)) {
                continue;
            }
            let key = name_of(&book);
            for d in sorted_entries(&book.join(LEGACY_INDEX_DIR)) {
                if !is_index_dir(&d) {
                    continue;
                }
                let stem = index_stem(&d);
                let dest = if slug(&stem) == key {
                    book.join(INDEX_DIR)
                } else {
                    cat.join(slug(&stem)).join(INDEX_DIR)
                };
                self.index(&d, &dest)?;
            }
            self.cleanup_legacy(&book.join(LEGACY_INDEX_DIR));
        }
        for d in sorted_entries(&cat.join(LEGACY_INDEX_DIR)) {
            if !is_index_dir(&d) || self.gone.contains(&d) {
                continue;
            }
            let stem = index_stem(&d);
            let dest = cat.join(slug(&stem)).join(INDEX_DIR);
            if !self.exists(&cat.join(slug(&stem))) {
                self.warn(format!(
                    "`{}` 没有对应的 PDF；索引仍移到 `{}`，请把 `{stem}.pdf` 放进同一文件夹",
                    rel_dir(self.root, &d),
                    rel_dir(self.root, &dest)
                ));
            }
            self.index(&d, &dest)?;
        }
        self.cleanup_legacy(&cat.join(LEGACY_INDEX_DIR));
        // Loose Markdown is not part of the standard (PDF + generated index).
        for p in sorted_entries(cat) {
            let n = name_of(&p).to_lowercase();
            if p.is_file() && is_ext(&p, "md") && n != "readme.md" {
                self.warn(format!(
                    "`{}` 是手工放的 Markdown，不是标准格式（标准：PDF + pdf2md 生成的 `<slug>/index/`），程序不读取它；不需要可以删除，需要的话请放对应的 PDF",
                    rel(self.root, &p)
                ));
            }
        }
        Ok(())
    }

    fn pdf(&mut self, cat: &Path, pdf: &Path) -> Result<()> {
        let stem = stem_of(pdf);
        let key = slug(&stem);
        let dest_dir = cat.join(&key);
        let parent = pdf.parent().unwrap_or(cat).to_path_buf();
        if parent == dest_dir {
            return Ok(());
        }
        // A whole mis-named book folder (one PDF, maybe combat/): rename it.
        if parent != cat && parent.parent() == Some(cat) && !self.exists(&dest_dir) {
            let mut others = Vec::new();
            find_pdfs(&parent, true, &mut others);
            if others.len() == 1 {
                let had_index = is_index_dir(&parent.join(LEGACY_INDEX_DIR).join(&stem));
                self.mv(&parent, &dest_dir, true)?;
                if had_index {
                    let legacy = dest_dir.join(LEGACY_INDEX_DIR).join(&stem);
                    self.index(&legacy, &dest_dir.join(INDEX_DIR))?;
                    self.cleanup_legacy(&dest_dir.join(LEGACY_INDEX_DIR));
                }
                return Ok(());
            }
        }
        let target = dest_dir.join(name_of(pdf));
        if self.exists(&target) {
            self.warn(format!(
                "`{}` 已经有同名文件 `{}`；两份都保留、这份没有移动，请确认后手动处理",
                rel(self.root, pdf),
                rel(self.root, &target)
            ));
            return Ok(());
        }
        if dest_dir.is_dir() || self.exists(&dest_dir) {
            let mut have = Vec::new();
            if dest_dir.is_dir() {
                find_pdfs(&dest_dir, true, &mut have);
            }
            if let Some(other) = have.first() {
                self.warn(format!(
                    "`{}` 和 `{}` 得到同一个文件夹名 `{key}`；这份没有移动，请改名后再整理",
                    rel(self.root, pdf),
                    rel(self.root, other)
                ));
                return Ok(());
            }
        }
        self.mv(pdf, &target, false)?;
        let legacy = parent.join(LEGACY_INDEX_DIR).join(&stem);
        if is_index_dir(&legacy) {
            self.index(&legacy, &dest_dir.join(INDEX_DIR))?;
            self.cleanup_legacy(&parent.join(LEGACY_INDEX_DIR));
        } else if parent != cat {
            let idx = parent.join(INDEX_DIR);
            let mut left = Vec::new();
            find_pdfs(&parent, true, &mut left);
            left.retain(|p| !self.gone.contains(p));
            if is_index_dir(&idx) && index_stem(&idx) == stem && left.is_empty() {
                self.index(&idx, &dest_dir.join(INDEX_DIR))?;
            }
        }
        Ok(())
    }

    fn index(&mut self, from: &Path, to: &Path) -> Result<()> {
        if from == to {
            return Ok(());
        }
        if self.exists(to) {
            self.warn(format!(
                "`{}` 已有索引；`{}` 两份都保留、没有移动，请确认后手动处理",
                rel_dir(self.root, to),
                rel_dir(self.root, from)
            ));
            return Ok(());
        }
        self.mv(from, to, true)
    }

    /// Remove an empty legacy `.md/` folder (never one with files in it).
    fn cleanup_legacy(&mut self, d: &Path) {
        if !self.dry_run && d.is_dir() && fs::read_dir(d).is_ok_and(|mut r| r.next().is_none()) {
            let _ = fs::remove_dir(d);
        }
    }

    /// Delete page-less reader editions where the paged edition exists.
    fn readers(&mut self, cat: &Path) -> Result<()> {
        for (_, d) in category_indexes(cat) {
            let stem = index_stem(&d);
            let reader = d.join(format!("{stem}.md"));
            let paged = d.join(format!("{stem}.pages.md"));
            if !reader.is_file() || !paged.is_file() {
                continue;
            }
            let text = fs::read_to_string(&paged).unwrap_or_default();
            if !text.lines().any(is_page_marker) {
                continue;
            }
            let bytes = fs::metadata(&reader).map(|m| m.len()).unwrap_or(0);
            if !self.dry_run {
                fs::remove_file(&reader).with_context(|| format!("remove {}", reader.display()))?;
                point_index_at_pages(&d, &stem)?;
            }
            self.report.readers.push((rel(self.root, &reader), bytes));
        }
        Ok(())
    }
}

/// After the reader edition is gone, point `index.md`'s "Read:" line and
/// `index.json`'s `book` at the paged edition (what pdf2md 0.6.0 writes).
fn point_index_at_pages(dir: &Path, stem: &str) -> Result<()> {
    let pages = format!("{stem}.pages.md");
    let index_md = dir.join("index.md");
    if let Ok(text) = fs::read_to_string(&index_md) {
        let old = format!(
            "Read: [{stem}.md](<{stem}.md>) (no page anchors). Page lookup: [{pages}](<{pages}>). "
        );
        if text.contains(&old) {
            let new = format!("Read: [{pages}](<{pages}>) (`<a id=\"pN\"></a>` = PDF page N). ");
            combat_sim::fsutil::atomic_write(&index_md, text.replace(&old, &new).as_bytes())?;
        }
    }
    let index_json = dir.join("index.json");
    if let Ok(text) = fs::read_to_string(&index_json) {
        if let Ok(serde_json::Value::Object(mut idx)) = serde_json::from_str(&text) {
            if idx.get("book").and_then(|b| b.as_str()) == Some(&format!("{stem}.md")) {
                idx.insert("book".into(), serde_json::Value::String(pages));
                let out = serde_json::to_string_pretty(&idx)?;
                combat_sim::fsutil::atomic_write(&index_json, out.as_bytes())?;
            }
        }
    }
    Ok(())
}

/// Bring `Adventures/` and `Rules/<edition>/` to one folder per book.
/// Idempotent; `dry_run` reports without touching any file.
pub fn tidy(root: &Path, dry_run: bool) -> Result<TidyReport> {
    let mut t = Tidy {
        root,
        dry_run,
        report: TidyReport {
            dry_run,
            ..Default::default()
        },
        claimed: Vec::new(),
        gone: Vec::new(),
    };
    if pdf2md_busy(root) {
        t.report.skipped =
            Some("pdf2md 正在后台转换；等它结束后再运行 `talkdnd library tidy`".into());
        return Ok(t.report);
    }
    t.category(&root.join("Adventures"))?;
    for w in merge_warnings(&root.join("Adventures")) {
        t.warn(w);
    }
    for ed in EDITIONS {
        t.category(&root.join("Rules").join(ed))?;
    }
    // Rulebooks whose edition is unknown are never moved.
    let rules = root.join("Rules");
    for p in sorted_entries(&rules) {
        let n = name_of(&p);
        if p.is_dir() {
            if n.starts_with('.')
                || EDITIONS.contains(&n.as_str())
                || RULES_PROGRAM_DIRS.contains(&n.as_str())
            {
                continue;
            }
            let mut pdfs = Vec::new();
            find_pdfs(&p, false, &mut pdfs);
            for pdf in pdfs {
                t.warn(unknown_edition(root, &pdf));
            }
        } else if is_pdf(&p) {
            t.warn(unknown_edition(root, &p));
        }
    }
    t.readers(&root.join("Adventures"))?;
    for ed in EDITIONS {
        t.readers(&root.join("Rules").join(ed))?;
    }
    if !dry_run && !t.report.moves.is_empty() {
        record_moves(root, &t.report.moves)?;
    }
    Ok(t.report)
}

fn unknown_edition(root: &Path, pdf: &Path) -> String {
    format!(
        "`{}` 看不出是哪一版规则书，没有移动；请手动放进 `Rules/2014/` 或 `Rules/2024/`，再运行 `talkdnd library tidy`",
        rel(root, pdf)
    )
}

fn record_moves(root: &Path, moves: &[Move]) -> Result<()> {
    let mut log = load_moves(root);
    for m in moves {
        if !log.contains(m) {
            log.push(m.clone());
        }
    }
    let text = format!(
        "# TalkDND library moves (written by `talkdnd library tidy`; local, never commit).\n# Saves that still point at an old path are repointed on `talkdnd start`.\n{}",
        serde_norway::to_string(&MovesLog { moves: log })?
    );
    combat_sim::fsutil::atomic_write(root.join(MOVES_FILE), text.as_bytes())
}

pub fn load_moves(root: &Path) -> Vec<Move> {
    fs::read_to_string(root.join(MOVES_FILE))
        .ok()
        .and_then(|t| serde_norway::from_str::<MovesLog>(&t).ok())
        .map(|l| l.moves)
        .unwrap_or_default()
}

// ---------------------------------------------------------------- saves

/// Rewrite one text: every recorded move (`from` -> `to`), then any
/// remaining legacy index path `<book>/.md/<Stem>/` -> `<book>/index/`.
pub fn repoint_text(text: &str, moves: &[Move]) -> String {
    let mut out = text.to_string();
    // Longest source first so a folder move does not pre-empt a file move.
    let mut ms: Vec<&Move> = moves.iter().collect();
    ms.sort_by_key(|m| std::cmp::Reverse(m.from.len()));
    for m in ms {
        if m.from.ends_with('/') {
            out = out.replace(&m.from, &m.to);
            let bare_from = m.from.trim_end_matches('/');
            let bare_to = m.to.trim_end_matches('/');
            out = replace_token(&out, bare_from, bare_to);
        } else {
            out = replace_token(&out, &m.from, &m.to);
        }
    }
    legacy_index_paths(&out)
}

/// Replace `from` only where it is not followed by a name character.
fn replace_token(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() || !text.contains(from) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(from) {
        let after = &rest[i + from.len()..];
        let next = after.chars().next();
        out.push_str(&rest[..i]);
        if next.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `Adventures/<slug>/.md/<Stem>/x` -> `Adventures/<slug>/index/x` (and the
/// same under `Rules/<edition>/`), for paths written before the move log.
fn legacy_index_paths(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let needle = "/.md/";
    while let Some(i) = rest.find(needle) {
        let head = &rest[..i];
        let after = &rest[i + needle.len()..];
        // The segment before must be a book folder: <cat>/<slug>.
        let seg_start = head.rfind('/').map(|j| j + 1).unwrap_or(0);
        let book = &head[seg_start..];
        let cat_head = &head[..seg_start.saturating_sub(1)];
        let in_category = cat_head.ends_with("Adventures")
            || EDITIONS
                .iter()
                .any(|e| cat_head.ends_with(&format!("Rules/{e}")));
        let book_ok = !book.is_empty()
            && book
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if let (true, true, Some(end)) = (in_category, book_ok, after.find('/')) {
            out.push_str(head);
            out.push('/');
            out.push_str(INDEX_DIR);
            out.push('/');
            rest = &after[end + 1..];
        } else {
            out.push_str(&rest[..i + needle.len()]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Save files that may hold book paths: YAML/JSON/Markdown, except the
/// session logs (history), the regenerated digest and machine caches.
pub fn save_text_files(save: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, top: bool, out: &mut Vec<PathBuf>) {
        for p in sorted_entries(dir) {
            let n = name_of(&p);
            if p.is_dir() {
                if n.starts_with('.') || (top && (n == "Sessions" || n == "_backups")) {
                    continue;
                }
                walk(&p, false, out);
            } else if p.is_file()
                && (is_ext(&p, "yaml")
                    || is_ext(&p, "yml")
                    || is_ext(&p, "json")
                    || is_ext(&p, "md"))
                && !(top && n == "digest.md")
            {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(save, true, &mut v);
    v
}

/// Files of `save` whose book paths would change: (path, new text).
pub fn plan_repoint(root: &Path, save: &Path) -> Vec<(PathBuf, String)> {
    let moves = load_moves(root);
    let mut v = Vec::new();
    for f in save_text_files(save) {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        if !text.contains("Adventures") && !text.contains("Rules") {
            continue;
        }
        let new = repoint_text(&text, &moves);
        if new != text {
            v.push((f, new));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("talkdnd-library-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    fn index(d: &Path, stem: &str) {
        write(
            &d.join("manifest.json"),
            &format!("{{\"pdf\": \"{stem}.pdf\"}}"),
        );
        write(
            &d.join(format!("{stem}.pages.md")),
            "# Book\n\n<a id=\"p1\"></a>\n<!-- PDF p.1 -->\n## Heading\n\nText.\n",
        );
        write(
            &d.join(format!("{stem}.md")),
            "# Book\n\n## Heading\n\nText.\n",
        );
        write(
            &d.join("index.json"),
            &format!("{{\"book\": \"{stem}.md\", \"pages\": \"{stem}.pages.md\"}}"),
        );
        write(
            &d.join("index.md"),
            &format!(
                "# Index\n\nRead: [{stem}.md](<{stem}.md>) (no page anchors). \
                 Page lookup: [{stem}.pages.md](<{stem}.pages.md>). PDF: `{stem}.pdf`.\n"
            ),
        );
        write(&d.join("pages/p0001-0001.md"), "x");
    }

    #[test]
    fn book_kind_accepts_ids_and_titles() {
        assert_eq!(book_kind("phb"), Some("phb"));
        assert_eq!(book_kind("Player's Handbook"), Some("phb"));
        assert_eq!(book_kind("Dungeon Master's Guide"), Some("dmg"));
        assert_eq!(book_kind("Monster Manual"), Some("mm"));
        assert_eq!(book_kind("campaign guide"), None);
    }

    #[test]
    fn slug_rule() {
        assert_eq!(slug("Storm King's Thunder"), "storm-kings-thunder");
        assert_eq!(
            slug("Keys from the Golden Vault"),
            "keys-from-the-golden-vault"
        );
        assert_eq!(slug("2024 Players Handbook v2"), "2024-players-handbook-v2");
        assert_eq!(slug("SRD_CC_v5.2.1"), "srd-cc-v5-2-1");
        assert_eq!(slug("  Café — Book!  "), "caf-book");
        assert!(slug("龙与地下城").starts_with("book-"));
        assert_ne!(slug("龙与地下城"), slug("冒险"));
    }

    #[test]
    fn key_rank_parts_and_article() {
        assert_eq!(key_rank("curse-of-strahd", "curse-of-strahd"), Some(0));
        assert_eq!(
            key_rank(
                "the-wild-beyond-the-witchlight",
                "wild-beyond-the-witchlight"
            ),
            Some(1)
        );
        assert_eq!(
            key_rank(
                "journeys-through-the-radiant-citadel-a",
                "journeys-through-the-radiant-citadel"
            ),
            Some(2)
        );
        assert_eq!(key_rank("foo-part-2", "foo"), Some(2));
        assert_eq!(key_rank("foo-bar", "foo"), None);
    }

    #[test]
    fn page_markers() {
        assert!(is_page_marker("<a id=\"p12\"></a>"));
        assert!(is_page_marker("<!-- PDF p.3 -->\n"));
        assert!(!is_page_marker("<a id=\"x\"></a>"));
        let s = strip_page_markers("A\n\n<a id=\"p1\"></a>\n<!-- PDF p.1 -->\n\nB\n");
        assert_eq!(s, "A\n\nB\n");
    }

    #[test]
    fn tidy_moves_loose_books_and_indexes() {
        let r = tmp("tidy");
        let adv = r.join("Adventures");
        write(&adv.join("README.md"), "readme");
        write(&adv.join("Storm King's Thunder.pdf"), "pdf1");
        index(
            &adv.join(".md/Storm King's Thunder"),
            "Storm King's Thunder",
        );
        write(
            &adv.join("keys-from-the-golden-vault/Keys from the Golden Vault.pdf"),
            "pdf2",
        );
        index(
            &adv.join("keys-from-the-golden-vault/.md/Keys from the Golden Vault"),
            "Keys from the Golden Vault",
        );
        write(
            &adv.join("keys-from-the-golden-vault/combat/creatures.yaml"),
            "c: 1\n",
        );
        write(&adv.join("deep/nested/Other Book.pdf"), "pdf3");
        write(&adv.join("Hand-Made.md"), "manual");
        write(&r.join("Rules/2024/2024 Players Handbook v2.pdf"), "pdf4");
        index(
            &r.join("Rules/2024/.md/2024 Players Handbook v2"),
            "2024 Players Handbook v2",
        );
        write(&r.join("Rules/Mystery.pdf"), "pdf5");
        write(&r.join("Rules/rulings.md"), "rulings");

        let dry = tidy(&r, true).unwrap();
        assert!(dry.moves.len() >= 6, "{:?}", dry.moves);
        assert!(
            adv.join("Storm King's Thunder.pdf").is_file(),
            "dry-run moved a file"
        );

        let rep = tidy(&r, false).unwrap();
        assert_eq!(
            dry.moves, rep.moves,
            "dry-run and real run must plan the same moves"
        );
        let skt = adv.join("storm-kings-thunder");
        assert!(skt.join("Storm King's Thunder.pdf").is_file());
        assert!(skt.join("index/Storm King's Thunder.pages.md").is_file());
        assert!(
            !skt.join("index/Storm King's Thunder.md").exists(),
            "reader removed"
        );
        assert!(!adv.join(".md").exists());
        let idx_md = fs::read_to_string(skt.join("index/index.md")).unwrap();
        assert!(
            idx_md.contains("Read: [Storm King's Thunder.pages.md]") && idx_md.contains("PDF: `"),
            "{idx_md}"
        );
        let idx_json = fs::read_to_string(skt.join("index/index.json")).unwrap();
        assert!(
            idx_json.contains("\"book\": \"Storm King's Thunder.pages.md\""),
            "{idx_json}"
        );
        let kgv = adv.join("keys-from-the-golden-vault");
        assert!(kgv.join("index/manifest.json").is_file());
        assert!(kgv.join("combat/creatures.yaml").is_file());
        assert!(adv.join("other-book/Other Book.pdf").is_file());
        let phb = r.join("Rules/2024/2024-players-handbook-v2");
        assert!(phb.join("2024 Players Handbook v2.pdf").is_file());
        assert!(phb
            .join("index/2024 Players Handbook v2.pages.md")
            .is_file());
        assert!(
            r.join("Rules/Mystery.pdf").is_file(),
            "unknown edition stays"
        );
        assert!(rep.warnings.iter().any(|w| w.contains("Mystery.pdf")));
        assert!(rep.warnings.iter().any(|w| w.contains("Hand-Made.md")));
        assert!(adv.join("Hand-Made.md").is_file(), "never deleted");
        assert_eq!(rep.readers.len(), 3);
        assert!(load_moves(&r).len() >= 6);

        // Lookups use the new layout.
        let idx = adventure_indexes(&r, "storm-kings-thunder");
        assert_eq!(idx, vec![skt.join("index")]);
        assert_eq!(
            paged_markdown(&idx[0]).unwrap(),
            skt.join("index/Storm King's Thunder.pages.md")
        );
        assert_eq!(rule_books(&r, "2024").len(), 1);

        // Idempotent.
        let again = tidy(&r, false).unwrap();
        assert!(
            again.moves.is_empty() && again.readers.is_empty(),
            "{:?}",
            again.lines()
        );
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn merge_archives_parts_and_keeps_combat() {
        let r = tmp("merge");
        let adv = r.join("Adventures");
        write(&adv.join("foo/Foo.pdf"), "pdf");
        write(&adv.join("foo/index/manifest.json"), "{}");
        write(&adv.join("foo-a/Foo A.pdf"), "pdf");
        write(&adv.join("foo-a/combat/a.yaml"), "a: 1\n");
        write(&adv.join("foo-b/Foo B.pdf"), "pdf");

        let rep = merge_parts(&r, "foo", false).unwrap();
        assert!(
            rep.lines().iter().any(|l| l.contains("已归档")),
            "{:?}",
            rep.lines()
        );
        assert!(!adv.join("foo-a").exists());
        assert!(!adv.join("foo-b").exists());
        let backup = r.join(".talkdnd/book-backups/merge-foo");
        assert!(backup.join("foo-a/Foo A.pdf").is_file());
        assert!(backup.join("foo-b/Foo B.pdf").is_file());
        assert!(adv.join("foo/combat/a.yaml").is_file());
        assert!(!backup.join("foo-a/combat").exists());
        let moves = load_moves(&r);
        assert!(moves
            .iter()
            .any(|m| m.from == "Adventures/foo-a/" && m.to == "Adventures/foo/"));
        assert!(moves
            .iter()
            .any(|m| m.from == "Adventures/foo-b/" && m.to == "Adventures/foo/"));

        let again = merge_parts(&r, "foo", false).unwrap();
        assert!(again.lines().iter().any(|l| l.contains("无需合并")));
    }

    #[test]
    fn merge_dry_run_changes_nothing() {
        let r = tmp("merge-dry");
        let adv = r.join("Adventures");
        write(&adv.join("foo/Foo.pdf"), "pdf");
        write(&adv.join("foo-a/Foo A.pdf"), "pdf");

        let rep = merge_parts(&r, "foo", true).unwrap();
        assert_eq!(rep.archived.len(), 1);
        assert!(adv.join("foo-a/Foo A.pdf").is_file());
        assert!(!r.join(".talkdnd/book-backups").exists());
        assert!(load_moves(&r).is_empty());
    }

    #[test]
    fn merge_refuses_without_a_full_book() {
        let r = tmp("merge-refuse");
        let adv = r.join("Adventures");
        write(&adv.join("foo-a/Foo A.pdf"), "pdf");
        write(&adv.join("foo-b/Foo B.pdf"), "pdf");
        assert!(merge_parts(&r, "foo", false).is_err());
        assert!(adv.join("foo-a/Foo A.pdf").is_file());
    }

    #[test]
    fn merge_keeps_a_parts_combat_when_both_sides_have_one() {
        let r = tmp("merge-collide");
        let adv = r.join("Adventures");
        write(&adv.join("foo/Foo.pdf"), "pdf");
        write(&adv.join("foo/combat/keep.yaml"), "keep: 1\n");
        write(&adv.join("foo-a/Foo A.pdf"), "pdf");
        write(&adv.join("foo-a/combat/a.yaml"), "a: 1\n");

        let rep = merge_parts(&r, "foo", false).unwrap();
        assert!(
            rep.warnings.iter().any(|w| w.contains("未合并战斗数据")),
            "{:?}",
            rep.warnings
        );
        assert!(!adv.join("foo/combat/a.yaml").exists());
        assert!(r
            .join(".talkdnd/book-backups/merge-foo/foo-a/combat/a.yaml")
            .is_file());
    }

    #[test]
    fn tidy_warns_about_a_full_book_with_parts() {
        let r = tmp("merge-warn");
        let adv = r.join("Adventures");
        write(&adv.join("foo/Foo.pdf"), "pdf");
        write(&adv.join("foo-a/Foo A.pdf"), "pdf");
        let rep = tidy(&r, true).unwrap();
        assert!(
            rep.warnings.iter().any(|w| w.contains("library merge foo")),
            "{:?}",
            rep.warnings
        );

        let r2 = tmp("merge-parts-only");
        write(&r2.join("Adventures/foo-a/Foo A.pdf"), "pdf");
        write(&r2.join("Adventures/foo-b/Foo B.pdf"), "pdf");
        let rep2 = tidy(&r2, true).unwrap();
        assert!(
            !rep2
                .warnings
                .iter()
                .any(|w| w.contains("同一本书既有整本也有分卷")),
            "{:?}",
            rep2.warnings
        );
    }

    #[test]
    fn tidy_keeps_both_on_collision() {
        let r = tmp("collide");
        let adv = r.join("Adventures");
        write(&adv.join("Curse of Strahd.pdf"), "loose copy");
        write(
            &adv.join("curse-of-strahd/Curse of Strahd.pdf"),
            "folder copy",
        );
        let rep = tidy(&r, false).unwrap();
        assert!(rep.moves.is_empty());
        assert!(rep.warnings.iter().any(|w| w.contains("两份都保留")));
        assert_eq!(
            fs::read_to_string(adv.join("Curse of Strahd.pdf")).unwrap(),
            "loose copy"
        );
        assert_eq!(
            fs::read_to_string(adv.join("curse-of-strahd/Curse of Strahd.pdf")).unwrap(),
            "folder copy"
        );
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn tidy_renames_misnamed_book_folder_with_combat() {
        let r = tmp("rename");
        let adv = r.join("Adventures");
        write(&adv.join("Strahd Stuff/Curse of Strahd.pdf"), "pdf");
        write(&adv.join("Strahd Stuff/combat/scenes/a.yaml"), "x: 1\n");
        index(
            &adv.join("Strahd Stuff/.md/Curse of Strahd"),
            "Curse of Strahd",
        );
        let rep = tidy(&r, false).unwrap();
        let cos = adv.join("curse-of-strahd");
        assert!(
            cos.join("Curse of Strahd.pdf").is_file(),
            "{:?}",
            rep.lines()
        );
        assert!(cos.join("combat/scenes/a.yaml").is_file());
        assert!(cos.join("index/manifest.json").is_file());
        let moves = load_moves(&r);
        let t = repoint_text(
            "adventures: [../../../Adventures/Strahd Stuff/combat]\n",
            &moves,
        );
        assert_eq!(
            t,
            "adventures: [../../../Adventures/curse-of-strahd/combat]\n"
        );
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn repoint_legacy_index_paths() {
        let t = repoint_text(
            "img: Adventures/keys-from-the-golden-vault/.md/Keys from the Golden Vault/img/p0001.webp\nother: notes/.md/x/y\n",
            &[],
        );
        assert_eq!(
            t,
            "img: Adventures/keys-from-the-golden-vault/index/img/p0001.webp\nother: notes/.md/x/y\n"
        );
        let m = vec![Move {
            from: "Adventures/Foo.pdf".into(),
            to: "Adventures/foo/Foo.pdf".into(),
        }];
        assert_eq!(
            repoint_text("pdf: Adventures/Foo.pdf\nx: Adventures/Foo.pdfx\n", &m),
            "pdf: Adventures/foo/Foo.pdf\nx: Adventures/Foo.pdfx\n"
        );
    }

    #[test]
    fn busy_lock_skips_tidy() {
        let r = tmp("busy");
        write(&r.join("Adventures/Loose.pdf"), "pdf");
        write(
            &r.join(PDF2MD_LOCK),
            &format!(
                "{{\"pid\": {}, \"started\": 0, \"updated\": 0}}",
                std::process::id()
            ),
        );
        let rep = tidy(&r, false).unwrap();
        assert!(rep.skipped.is_some());
        assert!(r.join("Adventures/Loose.pdf").is_file());
        write(&r.join(PDF2MD_LOCK), "{\"pid\": 1, \"released\": true}");
        let rep = tidy(&r, false).unwrap();
        assert!(rep.skipped.is_none());
        assert!(r.join("Adventures/loose/Loose.pdf").is_file());
        let _ = fs::remove_dir_all(&r);
    }
}
