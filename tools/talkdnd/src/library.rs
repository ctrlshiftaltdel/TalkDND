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

use anyhow::{Context, Result};
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
    /// Player-selectable candidates when the scan is ambiguous (`2014-phb`).
    #[serde(default)]
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookStatus {
    Identified,
    Ambiguous,
    Unknown,
    SrdOnly,
    MissingIndex,
}

/// Scan `Rules/2014/` and `Rules/2024/` and identify each converted book from
/// its first pages. The folder and file names are never used as evidence.
pub fn scan_books(_root: &Path) -> Result<Vec<BookRecord>> {
    Ok(Vec::new())
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

/// Is this line a pdf2md page marker (`<a id="p12"></a>` / `<!-- PDF p.12 -->`)?
pub fn is_page_marker(line: &str) -> bool {
    let t = line.trim();
    let num = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
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
    false
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
