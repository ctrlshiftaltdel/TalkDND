//! Structural book index (P07; LetsPlay/docs/campaign-continuity.md §4.6/§5.5).
//!
//! `talkdnd adventure index ...` builds and searches a title-level index of
//! a converted book (chapters, sections, maps, figures, statblocks) so the DM
//! never reads a whole book to find one section. The built index lives in the
//! book's own `index/structured.json` and holds titles, page numbers and
//! anchors only — never body text. It is derived from pdf2md's existing
//! output (`index.json` plus the paged Markdown) and is rebuilt only when
//! that source changes.

use crate::{adventure, library, start, update};
use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The built index, inside the book's `index/`.
const STRUCTURED: &str = "structured.json";
/// The only `structured.json` version this program writes and reads.
const VERSION: u32 = 1;
/// How far above the index folder the book's PDF is looked for.
const PDF_SEARCH_UP: usize = 2;

#[derive(Subcommand, Clone)]
pub enum Cmd {
    /// Build or refresh the index (unchanged sources are skipped)
    Build {
        /// Book slug or title (default: this save's adventure)
        #[arg(long)]
        book: Option<String>,
    },
    /// Show the index: titles, pages, anchors (never body text)
    Show {
        /// Book slug or title
        book: String,
    },
    /// Search titles/section/figure/statblock labels only
    Search {
        /// Book slug or title
        book: String,
        /// Words or phrase to find
        query: Vec<String>,
    },
}

/// `adventure index build|show|search` (P07, TASK-0060).
pub fn run(root: &Path, save: &Path, cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Build { book } => {
            for d in index_dirs(root, save, book.as_deref())? {
                println!("{}", build_one(root, &d)?);
            }
        }
        Cmd::Show { book } => show(root, save, &book)?,
        Cmd::Search { book, query } => search(root, save, &book, &query.join(" "))?,
    }
    Ok(())
}

// ---------------------------------------------------------------- structured

/// `index/structured.json`, version 1. Titles, pages and anchors only: no
/// body text, page images or extracted prose is ever stored here.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    version: u32,
    /// Book folder slug (`Adventures/<slug>` or `Rules/<edition>/<slug>`).
    #[serde(default)]
    book: String,
    #[serde(default)]
    source: Source,
    /// Every Markdown heading, in document order.
    #[serde(default)]
    headings: Vec<Heading>,
    #[serde(default)]
    chapters: Vec<Entry>,
    #[serde(default)]
    maps: Vec<Entry>,
    #[serde(default)]
    figures: Vec<Entry>,
    #[serde(default)]
    statblocks: Vec<Entry>,
    #[serde(default)]
    low_quality_pages: Vec<u32>,
}

/// Where the index came from: the book's PDF relative to its folder, with
/// its sha256 — or, when the PDF cannot be read, its mtime in seconds.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Source {
    #[serde(default)]
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mtime: Option<u64>,
}

/// A Markdown heading with its PDF page.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Heading {
    #[serde(default)]
    level: u32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    anchor: String,
}

/// One labelled position in the book: a chapter, map, figure or statblock.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    anchor: String,
}

fn entry(title: String, page: u32) -> Entry {
    Entry {
        title,
        page,
        anchor: format!("p{page}"),
    }
}

// ------------------------------------------------------------- locating books

/// Index folders of `--book` (a path or a book name), else of the save's
/// adventure. A book split into several PDFs yields every part, best match
/// first; an explicit name may also name a rulebook.
fn index_dirs(root: &Path, save: &Path, book: Option<&str>) -> Result<Vec<PathBuf>> {
    let want = match book {
        Some(b) => b.to_string(),
        None => start::adventure_slug(save).ok_or_else(|| {
            anyhow!(
                "本存档没有设置冒险；先用 `talkdnd chargen set adventure …`，或加 --book <书名>"
            )
        })?,
    };
    // A path to an index folder or to a book folder.
    for c in [PathBuf::from(&want), root.join(&want)] {
        if c.join("index.json").is_file() {
            return Ok(vec![c]);
        }
        let dirs = library::book_indexes(&c);
        if !dirs.is_empty() {
            return Ok(dirs);
        }
    }
    // A book name: this save's Adventures first, then both rulesets.
    let mut out = library::adventure_indexes(root, &want);
    if book.is_some() {
        let mut rules: Vec<(u8, usize, PathBuf)> = Vec::new();
        for (n, rs) in library::EDITIONS.iter().enumerate() {
            for (key, d) in library::category_indexes(&root.join("Rules").join(rs)) {
                if let Some(rank) = library::key_rank(&key, &want) {
                    rules.push((rank, n, d));
                }
            }
        }
        rules.sort();
        out.extend(rules.into_iter().map(|(_, _, d)| d));
    }
    if out.is_empty() {
        bail!(
            "没有找到「{want}」的已转换书籍；把 PDF 放进 Adventures/ 或 Rules/<版本>/，再按 LetsPlay/docs/pdf2md.md 用 pdf2md 转换"
        );
    }
    Ok(out)
}

// ------------------------------------------------------------------- building

/// Build one book's `structured.json`, or report that it is up to date.
fn build_one(root: &Path, dir: &Path) -> Result<String> {
    let fresh = derive(dir)?;
    let out = dir.join(STRUCTURED);
    if out.is_file() {
        let text = fs::read_to_string(&out).with_context(|| format!("read {}", out.display()))?;
        let old: Index = serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "{} 损坏（{e}）；请删除该文件后重新运行 `talkdnd adventure index build`",
                out.display()
            )
        })?;
        if old.version > VERSION {
            bail!(
                "{} 的版本是 {}，本程序只支持版本 {}；请升级 talkdnd",
                out.display(),
                old.version,
                VERSION
            );
        }
        if old == fresh {
            return Ok(format!("索引已跳过（来源未变）：{}", rel(root, &out)));
        }
    }
    let mut text = serde_json::to_string_pretty(&fresh)?;
    text.push('\n');
    atomic_write(&out, text.as_bytes())?;
    Ok(format!(
        "索引已重建：{}（标题 {}、章节 {}、地图 {}、图 {}、数据卡 {}、低质量页 {}）",
        rel(root, &out),
        fresh.headings.len(),
        fresh.chapters.len(),
        fresh.maps.len(),
        fresh.figures.len(),
        fresh.statblocks.len(),
        fresh.low_quality_pages.len(),
    ))
}

/// The structured index of one book folder, derived from its pdf2md output.
/// Deterministic: identical inputs give an identical value.
fn derive(dir: &Path) -> Result<Index> {
    let idx_path = dir.join("index.json");
    if !idx_path.is_file() {
        bail!(
            "缺少 {}；这本书还没有 pdf2md 索引，先按 LetsPlay/docs/pdf2md.md 转换",
            idx_path.display()
        );
    }
    let text =
        fs::read_to_string(&idx_path).with_context(|| format!("read {}", idx_path.display()))?;
    let idx: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        anyhow!(
            "{} 损坏（{e}）；请重新运行 pdf2md 转换，或删掉该书的 index/ 后重试",
            idx_path.display()
        )
    })?;
    if !idx.is_object() {
        bail!("{} 不是预期的 index.json 结构", idx_path.display());
    }
    let stem = library::index_stem(dir);
    let pages = pages_markdown(dir).ok_or_else(|| {
        anyhow!(
            "缺少 {}；这本书还没有 pdf2md 页码版，先按 LetsPlay/docs/pdf2md.md 转换",
            dir.join(format!("{stem}.pages.md")).display()
        )
    })?;

    let mut headings: Vec<Heading> = Vec::new();
    for h in idx["headings"].as_array().into_iter().flatten() {
        let level = page_field(&h["level"]);
        let title = str_field(&h["text"]);
        let page = page_field(&h["page"]);
        if level == 0 || title.is_empty() || page == 0 {
            continue;
        }
        headings.push(Heading {
            level,
            title,
            page,
            anchor: format!("p{page}"),
        });
    }
    if headings.is_empty() {
        bail!(
            "{} 里还没有标题；pdf2md 可能没转完，或整本需要 OCR（见 LetsPlay/docs/pdf2md.md）",
            idx_path.display()
        );
    }

    // Chapters: pdf2md's own list, else the manifest's intake audit, else
    // the headings that read like chapter openers.
    let chapter_of = |v: &serde_json::Value| {
        let title = str_field(&v["title"]);
        let page = page_field(&v["page"]);
        (!title.is_empty() && page > 0).then_some(entry(title, page))
    };
    let mut chapters: Vec<Entry> = Vec::new();
    for c in idx["chapters"].as_array().into_iter().flatten() {
        if let Some(e) = chapter_of(c) {
            chapters.push(e);
        }
    }
    if chapters.is_empty() {
        if let Some(m) = manifest_json(dir) {
            for c in m["intake"]["chapters"].as_array().into_iter().flatten() {
                if let Some(e) = chapter_of(c) {
                    chapters.push(e);
                }
            }
        }
    }
    if chapters.is_empty() {
        for h in &headings {
            if looks_like_chapter(&h.title) {
                chapters.push(entry(h.title.clone(), h.page));
            }
        }
    }

    let mut maps: Vec<Entry> = Vec::new();
    for m in idx["maps"].as_array().into_iter().flatten() {
        let page = page_field(&m["page"]);
        if page == 0 {
            continue;
        }
        let mut title = str_field(&m["caption"]);
        if title.is_empty() {
            title = format!("Map p.{page}");
        }
        maps.push(entry(title, page));
    }

    let mut figures: Vec<Entry> = Vec::new();
    for f in idx["figures"].as_array().into_iter().flatten() {
        let page = page_field(&f["page"]);
        if page == 0 {
            continue;
        }
        figures.push(entry(format!("Figure p.{page}"), page));
    }

    let checks: Vec<(u32, String)> = idx["statblock_checks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let page = page_field(&c["page"]);
            let name = str_field(&c["name"]);
            (page > 0 && !name.is_empty()).then_some((page, name))
        })
        .collect();
    let mut statblocks: Vec<Entry> = Vec::new();
    for s in idx["statblocks"].as_array().into_iter().flatten() {
        let page = page_field(s);
        if page == 0 {
            continue;
        }
        let title = checks
            .iter()
            .find(|(p, _)| *p == page)
            .map(|(_, n)| n.clone())
            .or_else(|| {
                headings
                    .iter()
                    .rev()
                    .find(|h| h.page <= page)
                    .map(|h| h.title.clone())
            })
            .unwrap_or_else(|| format!("Stat block p.{page}"));
        statblocks.push(entry(title, page));
    }

    let mut low: Vec<u32> = idx["low_quality_pages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_u64())
        .map(|p| p as u32)
        .filter(|p| *p > 0)
        .collect();
    low.sort_unstable();
    low.dedup();

    chapters.sort_by_key(|e| e.page);
    maps.sort_by_key(|e| e.page);
    figures.sort_by_key(|e| e.page);
    statblocks.sort_by_key(|e| e.page);

    let name = pdf_name(dir, &idx);
    let source = match find_pdf(dir, &name) {
        Some(pdf) => match fs::read(&pdf) {
            Ok(bytes) => Source {
                path: name,
                sha256: Some(update::sha256_hex(&bytes)),
                mtime: None,
            },
            Err(_) => Source {
                path: name,
                sha256: None,
                mtime: Some(mtime_secs(&pdf)),
            },
        },
        None => Source {
            path: name,
            sha256: None,
            mtime: Some(mtime_secs(&pages)),
        },
    };

    Ok(Index {
        version: VERSION,
        book: book_key(dir),
        source,
        headings,
        chapters,
        maps,
        figures,
        statblocks,
        low_quality_pages: low,
    })
}

// --------------------------------------------------------------- show / search

/// Every resolved book's stored index, best match first.
fn load_all(root: &Path, save: &Path, book: &str) -> Result<Vec<(PathBuf, Index)>> {
    let dirs = index_dirs(root, save, Some(book))?;
    let mut out = Vec::new();
    for d in dirs {
        let p = d.join(STRUCTURED);
        if !p.is_file() {
            bail!(
                "{} 还没有结构索引；先运行 `talkdnd adventure index build --book {book}`",
                rel(root, &d)
            );
        }
        let text = fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        let idx: Index = serde_json::from_str(&text).map_err(|e| {
            anyhow!(
                "{} 损坏（{e}）；请删除该文件后重新运行 `talkdnd adventure index build`",
                p.display()
            )
        })?;
        if idx.version > VERSION {
            bail!(
                "{} 的版本是 {}，本程序只支持版本 {}；请升级 talkdnd",
                p.display(),
                idx.version,
                VERSION
            );
        }
        out.push((d, idx));
    }
    Ok(out)
}

/// `adventure index show <book>`: titles, pages and anchors — never body.
fn show(root: &Path, save: &Path, book: &str) -> Result<()> {
    for (dir, idx) in load_all(root, save, book)? {
        println!(
            "索引：{}（只含标题、页码与锚点，不含正文）",
            rel(root, &dir.join(STRUCTURED))
        );
        println!("来源：{}（{}）", idx.source.path, source_desc(&idx.source));
        println!("章节（{}）：", idx.chapters.len());
        print_entries(&idx.chapters);
        println!("节（{}）：", idx.headings.len());
        for h in &idx.headings {
            println!(
                "  {}{} — p.{} #{}",
                "  ".repeat(h.level.saturating_sub(1) as usize),
                h.title,
                h.page,
                h.anchor
            );
        }
        println!("地图（{}）：", idx.maps.len());
        print_entries(&idx.maps);
        println!("图（{}）：", idx.figures.len());
        print_entries(&idx.figures);
        println!("数据卡（{}）：", idx.statblocks.len());
        print_entries(&idx.statblocks);
        let low: Vec<String> = idx
            .low_quality_pages
            .iter()
            .map(|p| p.to_string())
            .collect();
        println!(
            "低质量页（{}）：{}",
            low.len(),
            if low.is_empty() {
                "（无）".to_string()
            } else {
                low.join(", ")
            }
        );
    }
    Ok(())
}

fn print_entries(entries: &[Entry]) {
    if entries.is_empty() {
        println!("  （无）");
        return;
    }
    for e in entries {
        println!("  {} — p.{} #{}", e.title, e.page, e.anchor);
    }
}

/// `adventure index search <book> <query>`: titles, chapter, figure and
/// statblock labels only — never body text.
fn search(root: &Path, save: &Path, book: &str, query: &str) -> Result<()> {
    let q = adventure::normalize_for_search(query);
    if q.is_empty() {
        bail!("搜索词去掉标点后为空");
    }
    let books = load_all(root, save, book)?;
    let multi = books.len() > 1;
    let mut hits = 0usize;
    for (dir, idx) in &books {
        let part = if multi {
            format!("{}，", library::index_stem(dir))
        } else {
            String::new()
        };
        let sections: Vec<Entry> = idx
            .headings
            .iter()
            .map(|h| Entry {
                title: h.title.clone(),
                page: h.page,
                anchor: h.anchor.clone(),
            })
            .collect();
        let kinds: [(&str, &Vec<Entry>); 5] = [
            ("章", &idx.chapters),
            ("节", &sections),
            ("地图", &idx.maps),
            ("图", &idx.figures),
            ("数据卡", &idx.statblocks),
        ];
        for (kind, entries) in kinds {
            for e in entries {
                if adventure::normalize_for_search(&e.title).contains(&q) {
                    println!("  [{kind}]{part}{} — p.{} #{}", e.title, e.page, e.anchor);
                    hits += 1;
                }
            }
        }
    }
    if hits == 0 {
        println!(
            "没有找到标题含「{}」的条目；索引只匹配标题/章节/图示/数据卡标签，不搜正文",
            query.trim()
        );
    }
    Ok(())
}

// -------------------------------------------------------------------- helpers

/// A path as the player sees it: relative to the play root, `/`-separated.
fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn name_of_path(p: &str) -> String {
    Path::new(p)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn str_field(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn page_field(v: &serde_json::Value) -> u32 {
    v.as_u64().unwrap_or(0) as u32
}

/// The book's folder slug: the folder holding `index/`, or for a legacy
/// `.md/<Stem>/` index the slug of its stem.
fn book_key(dir: &Path) -> String {
    let parent = dir.parent().unwrap_or(dir);
    if name_of(parent) == library::LEGACY_INDEX_DIR {
        library::slug(&library::index_stem(dir))
    } else {
        name_of(parent)
    }
}

/// The book's PDF file name: index.json's, else the manifest's, else the
/// index stem.
fn pdf_name(dir: &Path, idx: &serde_json::Value) -> String {
    let named = |v: &serde_json::Value| {
        v.get("pdf")
            .and_then(|x| x.as_str())
            .map(name_of_path)
            .filter(|n| !n.is_empty())
    };
    if let Some(n) = named(idx) {
        return n;
    }
    if let Some(m) = manifest_json(dir) {
        if let Some(n) = named(&m) {
            return n;
        }
    }
    format!("{}.pdf", library::index_stem(dir))
}

/// The PDF, looked for in the index folder and a couple of folders up (the
/// book folder, then the category folder for a not-yet-tidied book).
fn find_pdf(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut cur = dir.to_path_buf();
    for _ in 0..=PDF_SEARCH_UP {
        let p = cur.join(name);
        if p.is_file() {
            return Some(p);
        }
        let Some(parent) = cur.parent().filter(|p| !p.as_os_str().is_empty()) else {
            break;
        };
        cur = parent.to_path_buf();
    }
    None
}

fn manifest_json(dir: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(dir.join("manifest.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// The book's paged Markdown: `<Stem>.pages.md`, else the only other
/// `*.pages.md` in the folder. A page-less legacy `<Stem>.md` is not enough
/// for a page index.
fn pages_markdown(dir: &Path) -> Option<PathBuf> {
    let named = dir.join(format!("{}.pages.md", library::index_stem(dir)));
    if named.is_file() {
        return Some(named);
    }
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && name_of(p).ends_with(".pages.md"))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Modification time in whole seconds since the epoch (0 when unknown).
fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// The recorded source fingerprint, short enough for one line.
fn source_desc(s: &Source) -> String {
    match (&s.sha256, s.mtime) {
        (Some(h), _) => format!("sha256 {}…", h.get(..12).unwrap_or(h)),
        (None, Some(m)) => format!("mtime {m}"),
        (None, None) => "无来源记录".to_string(),
    }
}

/// Conservative chapter test for books whose index.json has no chapter
/// list: leading chapter/part/appendix numbering, as `adventure section`
/// understands it.
fn looks_like_chapter(title: &str) -> bool {
    let t = title.trim().to_lowercase();
    let word = |w: &str| {
        t.strip_prefix(w).is_some_and(|r| {
            r.starts_with(|c: char| c.is_whitespace() || c.is_ascii_digit() || c == ':')
        })
    };
    word("chapter")
        || word("part")
        || word("appendix")
        || word("prologue")
        || word("epilogue")
        || (t.starts_with('第') && t.contains('章'))
}
