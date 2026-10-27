//! Build full-book content packs from local pdf2md indexes.
//!
//! Full packs are release artifacts, not Git content. The input is a player's
//! (or the owner's) local pdf2md `index/` directory for a 2014/2024 PHB, DMG or
//! MM; the output is a normal [`ContentPack`] with `distribution = full`.
//!
//! The builder never prints or embeds section text in its report. Only counts,
//! chapter metadata, page numbers and the source PDF hash prefix are recorded.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::content::pack::{
    ContentPack, Distribution, PackEntry, PackImage, PackIndex, PackMeta, SCHEMA_VERSION,
};
use crate::content::schema::{ContentFile, DomainEntity};
use crate::fsutil::atomic_write;
use crate::srd::slug;

/// Licence recorded in every full pack. The actual distribution decision is in
/// `docs/PROJECT-CHARTER.md`; this string must not be read as a new decision.
pub const FULL_LICENSE: &str =
    "Owner-authorized full-book distribution (see docs/PROJECT-CHARTER.md)";

/// Sections shorter than this are heading noise and are skipped.
const MIN_BODY_CHARS: usize = 1;

/// Longest slug fragment used in a section id.
const MAX_ID_SLUG: usize = 60;

/// The six books a full pack can be built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullBook {
    PlayerHandbook,
    DungeonMastersGuide,
    MonsterManual,
}

impl FullBook {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "phb" | "players-handbook" | "player's-handbook" => Ok(Self::PlayerHandbook),
            "dmg" | "dungeon-masters-guide" | "dungeon-master's-guide" => {
                Ok(Self::DungeonMastersGuide)
            }
            "mm" | "monster-manual" => Ok(Self::MonsterManual),
            other => bail!("unsupported full book `{other}`; expected phb, dmg or mm"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlayerHandbook => "phb",
            Self::DungeonMastersGuide => "dmg",
            Self::MonsterManual => "mm",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PlayerHandbook => "Player's Handbook",
            Self::DungeonMastersGuide => "Dungeon Master's Guide",
            Self::MonsterManual => "Monster Manual",
        }
    }
}

/// Counts and gaps from one build. Never contains section text or input paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    pub pack_id: String,
    pub edition: String,
    pub book: String,
    pub file: String,
    pub sections: usize,
    pub chapters: usize,
    pub images: usize,
    pub pages: u32,
    pub page_anchors: usize,
    pub empty_pages: usize,
    pub low_quality_pages: usize,
    pub statblock_pages: usize,
    pub skipped_headings: usize,
    pub unsafe_images: usize,
    pub gaps: Vec<String>,
}

/// One full pack in the release manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseEntry {
    pub id: String,
    pub file: String,
    pub edition: String,
    pub book: Option<String>,
    pub bytes: u64,
    pub sections: usize,
    pub chapters: usize,
    pub images: usize,
    pub content_hash: String,
    pub source: Option<String>,
    pub license: Option<String>,
}

/// Validated full packs ready for a release bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseReport {
    pub packs: Vec<ReleaseEntry>,
    pub total_bytes: u64,
}

impl ReleaseReport {
    /// Deterministic manifest value. It contains no local paths or timestamps.
    pub fn to_value(&self) -> Value {
        json!({
            "schema": 1,
            "tool": format!("combat-sim {}", env!("CARGO_PKG_VERSION")),
            "packs": self.packs.iter().map(|pack| json!({
                "id": pack.id,
                "file": pack.file,
                "edition": pack.edition,
                "book": pack.book,
                "bytes": pack.bytes,
                "sections": pack.sections,
                "chapters": pack.chapters,
                "images": pack.images,
                "content_hash": pack.content_hash,
                "source": pack.source,
                "license": pack.license,
            })).collect::<Vec<_>>(),
            "total_bytes": self.total_bytes,
        })
    }
}

/// Builds a full pack in memory. The caller decides whether to write it.
pub fn build_pack(
    edition: &str,
    book: &str,
    index_dir: &Path,
) -> Result<(ContentPack, BuildReport)> {
    let edition = normalize_edition(edition)?;
    let book = FullBook::parse(book)?;
    let input = read_input(index_dir)?;

    let mut raw = parse_sections(&input.pages);
    raw = merge_repeats(raw);
    let mut skipped_headings = 0;
    raw.retain(|section| {
        let keep =
            section.body.chars().count() >= MIN_BODY_CHARS || looks_like_chapter(&section.title);
        if !keep {
            skipped_headings += 1;
        }
        keep
    });
    if raw.is_empty() {
        bail!(
            "no sections extracted from the pdf2md index for {edition} {}; the book may not be converted yet",
            book.label()
        );
    }

    let mut gaps = Vec::new();
    let chapters = if input.chapters.is_empty() {
        let inferred: Vec<Chapter> = raw
            .iter()
            .filter(|section| looks_like_chapter(&section.title))
            .map(|section| Chapter {
                title: section.title.clone(),
                page: section.page,
            })
            .collect();
        if inferred.is_empty() {
            gaps.push(
                "no chapter list or chapter headings found; sections use `Front Matter`"
                    .to_string(),
            );
        }
        inferred
    } else {
        input.chapters.clone()
    };
    if input.chapters.is_empty() && !chapters.is_empty() {
        gaps.push(format!(
            "no chapter metadata in index.json or manifest.json; {} chapter(s) inferred from headings",
            chapters.len()
        ));
    }

    let mut used_ids = BTreeSet::new();
    let mut sections: Vec<Section> = Vec::with_capacity(raw.len());
    let mut page_to_id: BTreeMap<u32, String> = BTreeMap::new();
    for section in raw {
        let chapter = chapter_for(&chapters, section.page, &section.title);
        let id = section_id(edition, book, section.page, &section.title, &mut used_ids);
        page_to_id.entry(section.page).or_insert_with(|| id.clone());
        sections.push(Section {
            id,
            chapter,
            title: section.title,
            level: section.level,
            page: section.page,
            body: tidy(&section.body),
            refs: Vec::new(),
        });
    }

    for section in &mut sections {
        let mut refs = BTreeSet::new();
        for page in page_references(&section.body) {
            let target = page_to_id
                .range(..=page)
                .next_back()
                .map(|(_, id)| id.clone());
            if let Some(target) = target {
                if target != section.id {
                    refs.insert(target);
                }
            }
        }
        section.refs = refs.into_iter().collect();
    }

    let source = source_label(book, &input);
    let mut images = input.images.clone();
    for image in &mut images {
        if let Some(page) = image.page {
            image.section = Some(chapter_for(&chapters, page.max(0) as u32, ""));
        }
    }
    let meta = PackMeta {
        id: pack_id(edition, book),
        name: Some(format!("{} {} (full)", book.label(), edition)),
        edition: edition.to_string(),
        source: Some(source),
        license: Some(FULL_LICENSE.to_string()),
        distribution: Distribution::Full,
        schema_version: SCHEMA_VERSION,
        content_hash: None,
        created_at: input.created.clone(),
        description: Some(format!(
            "{} sections, {} chapters and {} images from the {} {} pdf2md index.",
            sections.len(),
            chapters.len(),
            images.len(),
            edition,
            book.label()
        )),
    };

    let entities: Vec<DomainEntity> = sections
        .iter()
        .map(|section| DomainEntity {
            id: section.id.clone(),
            name: Some(section.title.clone()),
            source: Some(book.label().to_string()),
            page: Some(section.page as i32),
            section: Some(section.chapter.clone()),
            tags: vec![
                "full".to_string(),
                "rule".to_string(),
                edition.to_string(),
                book.as_str().to_string(),
            ],
            aliases: Vec::new(),
            text: (!section.body.is_empty()).then(|| section.body.clone()),
            refs: section.refs.clone(),
            data: Some(json!({
                "book": book.as_str(),
                "edition": edition,
                "chapter": section.chapter,
                "level": section.level,
                "page": section.page,
                "anchor": format!("p{}", section.page),
            })),
            manual: None,
            notes: None,
            todo: Vec::new(),
        })
        .collect();
    let content = ContentFile {
        rule_sections: Some(entities),
        ..ContentFile::default()
    };

    let mut pack = ContentPack::new(meta, content)?;
    let mut entries: Vec<PackEntry> = sections
        .iter()
        .map(|section| PackEntry {
            id: section.id.clone(),
            kind: "rule-section".to_string(),
            name: Some(section.title.clone()),
            aliases: Vec::new(),
            page: Some(section.page as i32),
            section: Some(section.chapter.clone()),
            refs: section.refs.clone(),
        })
        .collect();
    entries.sort_by(|a, b| (a.kind.as_str(), a.id.as_str()).cmp(&(b.kind.as_str(), b.id.as_str())));
    pack.index = PackIndex { entries, images };
    pack.validate()?;

    if input.done_pages < input.page_count {
        gaps.push(format!(
            "{} page(s) not converted (done_pages {} of {})",
            input.page_count - input.done_pages,
            input.done_pages,
            input.page_count
        ));
    }
    if input.page_anchors < input.page_count as usize {
        gaps.push(format!(
            "{} page(s) have no page anchor in the pages markdown",
            input.page_count as usize - input.page_anchors
        ));
    }
    if input.empty_pages > 0 {
        gaps.push(format!(
            "{} empty page(s); consult the PDF for those pages",
            input.empty_pages
        ));
    }
    if !input.low_quality.is_empty() {
        gaps.push(format!(
            "{} low-quality page(s); consult the PDF for those pages",
            input.low_quality.len()
        ));
    }
    if !input.statblocks.is_empty() {
        gaps.push(format!(
            "{} stat-block page(s) are not structured by this builder; consult the PDF",
            input.statblocks.len()
        ));
    }
    if input.unsafe_images > 0 {
        gaps.push(format!(
            "{} image reference(s) with a non-relative path were skipped",
            input.unsafe_images
        ));
    }

    let report = BuildReport {
        pack_id: pack.pack.id.clone(),
        edition: edition.to_string(),
        book: book.as_str().to_string(),
        file: format!("{}.pack.json", pack.pack.id),
        sections: pack.entity_count(),
        chapters: chapters.len(),
        images: pack.image_count(),
        pages: input.page_count,
        page_anchors: input.page_anchors,
        empty_pages: input.empty_pages,
        low_quality_pages: input.low_quality.len(),
        statblock_pages: input.statblocks.len(),
        skipped_headings,
        unsafe_images: input.unsafe_images,
        gaps,
    };
    Ok((pack, report))
}

/// Builds a full pack and, unless `dry_run`, writes `<pack-id>.pack.json`.
pub fn build(
    edition: &str,
    book: &str,
    index_dir: &Path,
    out: &Path,
    dry_run: bool,
) -> Result<BuildReport> {
    let (pack, report) = build_pack(edition, book, index_dir)?;
    if dry_run {
        return Ok(report);
    }
    fs::create_dir_all(out)
        .with_context(|| format!("create full-pack output directory {}", out.display()))?;
    let text = serde_json::to_string_pretty(&pack).context("serialize full pack")?;
    let path = out.join(&report.file);
    atomic_write(&path, format!("{text}\n").as_bytes())
        .with_context(|| format!("write full pack {}", path.display()))?;
    Ok(report)
}

/// Validates a directory of full packs and optionally copies them plus a
/// deterministic `full-packs-manifest.json` into `out`.
///
/// `out = None` is the dry run: validation and the returned manifest only,
/// nothing written.
pub fn release(packs_dir: &Path, out: Option<&Path>) -> Result<ReleaseReport> {
    let mut files: Vec<PathBuf> = Vec::new();
    let entries = fs::read_dir(packs_dir)
        .with_context(|| format!("read full-pack directory {}", packs_dir.display()))?;
    for entry in entries {
        let path = entry
            .with_context(|| format!("read full-pack directory {}", packs_dir.display()))?
            .path();
        if path.is_file() && path.to_string_lossy().ends_with(".pack.json") {
            files.push(path);
        }
    }
    files.sort();
    if files.is_empty() {
        bail!(
            "no full packs (*.pack.json) found in {}",
            packs_dir.display()
        );
    }

    let mut seen = BTreeSet::new();
    let mut packs: Vec<(PathBuf, ContentPack)> = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read full pack {}", path.display()))?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("parse full pack {}", path.display()))?;
        let pack = ContentPack::from_value(&value)
            .with_context(|| format!("invalid full pack {}", path.display()))?;
        if pack.pack.distribution != Distribution::Full {
            bail!(
                "content pack {} is not a full pack (distribution {})",
                pack.pack.id,
                pack.pack.distribution.as_str()
            );
        }
        if !seen.insert(pack.pack.id.clone()) {
            bail!("duplicate full pack id `{}`", pack.pack.id);
        }
        packs.push((path, pack));
    }
    packs.sort_by(|a, b| a.1.pack.id.cmp(&b.1.pack.id));

    let mut report = ReleaseReport {
        packs: Vec::new(),
        total_bytes: 0,
    };
    for (path, pack) in &packs {
        let bytes = fs::metadata(path)
            .with_context(|| format!("stat full pack {}", path.display()))?
            .len();
        report.total_bytes += bytes;
        report.packs.push(ReleaseEntry {
            id: pack.pack.id.clone(),
            file: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("pack.json")
                .to_string(),
            edition: pack.pack.edition.clone(),
            book: book_from_id(&pack.pack.id),
            bytes,
            sections: pack.entity_count(),
            chapters: chapters_from_description(pack),
            images: pack.image_count(),
            content_hash: pack.pack.content_hash.clone().unwrap_or_default(),
            source: pack.pack.source.clone(),
            license: pack.pack.license.clone(),
        });
    }

    if let Some(out) = out {
        fs::create_dir_all(out)
            .with_context(|| format!("create release directory {}", out.display()))?;
        for (path, _) in &packs {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("pack.json");
            let destination = out.join(name);
            if destination != *path {
                fs::copy(path, &destination).with_context(|| {
                    format!(
                        "copy full pack {} to {}",
                        path.display(),
                        destination.display()
                    )
                })?;
            }
        }
        let manifest = serde_json::to_string_pretty(&report.to_value())
            .context("serialize full-pack release manifest")?;
        atomic_write(
            out.join("full-packs-manifest.json"),
            format!("{manifest}\n").as_bytes(),
        )
        .context("write full-packs-manifest.json")?;
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Chapter {
    title: String,
    page: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawSection {
    title: String,
    level: usize,
    page: u32,
    body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    id: String,
    chapter: String,
    title: String,
    level: usize,
    page: u32,
    body: String,
    refs: Vec<String>,
}

struct Input {
    pages: BTreeMap<u32, String>,
    page_anchors: usize,
    chapters: Vec<Chapter>,
    images: Vec<PackImage>,
    unsafe_images: usize,
    page_count: u32,
    done_pages: u32,
    empty_pages: usize,
    low_quality: Vec<u32>,
    statblocks: Vec<u32>,
    created: Option<String>,
    sha256: Option<String>,
}

fn read_input(index_dir: &Path) -> Result<Input> {
    let manifest_path = index_dir.join("manifest.json");
    let manifest: Value = read_json(&manifest_path)?;
    let index_path = index_dir.join("index.json");
    let index: Value = read_json(&index_path)?;
    if !index.is_object() {
        bail!("{} is not a JSON object", index_path.display());
    }

    let pdf = manifest["pdf"]
        .as_str()
        .or_else(|| index["pdf"].as_str())
        .unwrap_or("book.pdf");
    let pages_path = pages_markdown(index_dir, pdf, &index)?;
    let text = fs::read_to_string(&pages_path)
        .with_context(|| format!("read pages markdown {}", pages_path.display()))?;
    let (pages, empty_pages, page_anchors) = split_pages(&text);
    if pages.is_empty() {
        bail!("no page anchors in {}", pages_path.display());
    }

    let page_count = manifest["page_count"]
        .as_u64()
        .or_else(|| index["page_count"].as_u64())
        .unwrap_or(pages.len() as u64) as u32;
    let done_pages = index["done_pages"].as_u64().unwrap_or(page_anchors as u64) as u32;
    let chapters = chapters(&index, &manifest);

    let mut images = Vec::new();
    let mut seen_images = BTreeSet::new();
    let mut unsafe_images = 0;
    for key in ["figures", "art", "maps"] {
        for item in index[key].as_array().into_iter().flatten() {
            let page = item["page"].as_u64().unwrap_or(0) as u32;
            if page == 0 {
                continue;
            }
            let Some(raw) = item["img"]
                .as_str()
                .or_else(|| item["path"].as_str())
                .or_else(|| item["image"].as_str())
            else {
                continue;
            };
            let Some(path) = safe_relative_path(raw) else {
                unsafe_images += 1;
                continue;
            };
            if !seen_images.insert(path.clone()) {
                continue;
            }
            images.push(PackImage {
                id: format!("img-p{page}-{}", images.len() + 1),
                page: Some(page as i32),
                section: None,
                caption: None,
                path: Some(path),
            });
        }
    }

    let low_quality = pages_list(&index["low_quality_pages"]);
    let statblocks = pages_list(&index["statblocks"]);
    let sha256 = manifest["sha256"]
        .as_str()
        .map(str::trim)
        .filter(|hash| hash.len() >= 12 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(str::to_string);
    let created = manifest["created"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Ok(Input {
        pages,
        page_anchors,
        chapters,
        images,
        unsafe_images,
        page_count,
        done_pages,
        empty_pages,
        low_quality,
        statblocks,
        created,
        sha256,
    })
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn pages_markdown(index_dir: &Path, pdf: &str, index: &Value) -> Result<PathBuf> {
    let stem = pdf.strip_suffix(".pdf").unwrap_or(pdf);
    let named = index_dir.join(format!("{stem}.pages.md"));
    if named.is_file() {
        return Ok(named);
    }
    if let Some(book) = index["book"].as_str() {
        let named = index_dir.join(book);
        if named.is_file() {
            return Ok(named);
        }
    }
    let mut found: Vec<PathBuf> = fs::read_dir(index_dir)
        .with_context(|| format!("read pdf2md index directory {}", index_dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".pages.md"))
        })
        .collect();
    found.sort();
    found.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!("no *.pages.md in the pdf2md index directory; convert the book first")
    })
}

fn chapters(index: &Value, manifest: &Value) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = Vec::new();
    for source in [&index["chapters"], &manifest["intake"]["chapters"]] {
        for chapter in source.as_array().into_iter().flatten() {
            let title = chapter["title"].as_str().unwrap_or("").trim();
            let page = chapter["page"].as_u64().unwrap_or(0) as u32;
            if !title.is_empty() && page > 0 {
                out.push(Chapter {
                    title: title.to_string(),
                    page,
                });
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort_by(|a, b| (a.page, a.title.as_str()).cmp(&(b.page, b.title.as_str())));
    out.dedup();
    out
}

fn pages_list(value: &Value) -> Vec<u32> {
    let mut pages: Vec<u32> = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .map(|page| page.min(u32::MAX as u64) as u32)
        .filter(|page| *page > 0)
        .collect();
    pages.sort_unstable();
    pages.dedup();
    pages
}

fn safe_relative_path(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.contains(':') {
        return None;
    }
    let path = Path::new(&normalized);
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(normalized)
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

fn parse_sections(pages: &BTreeMap<u32, String>) -> Vec<RawSection> {
    let mut out = Vec::new();
    let mut current: Option<RawSection> = None;
    for (page, text) in pages {
        for line in text.lines() {
            if let Some((level, title)) = heading_line(line) {
                if let Some(section) = current.take() {
                    out.push(section);
                }
                current = Some(RawSection {
                    title,
                    level,
                    page: *page,
                    body: String::new(),
                });
                continue;
            }
            if let Some(section) = current.as_mut() {
                section.body.push_str(line);
                section.body.push('\n');
            }
        }
    }
    if let Some(section) = current {
        out.push(section);
    }
    out
}

/// A repeated running header on the same page continues the previous section.
fn merge_repeats(sections: Vec<RawSection>) -> Vec<RawSection> {
    let mut out: Vec<RawSection> = Vec::new();
    for section in sections {
        if let Some(previous) = out.last_mut() {
            if previous.page == section.page
                && previous.level == section.level
                && previous.title == section.title
            {
                if !section.body.trim().is_empty() {
                    if !previous.body.trim().is_empty() {
                        previous.body.push('\n');
                    }
                    previous.body.push_str(&section.body);
                }
                continue;
            }
        }
        out.push(section);
    }
    out
}

fn section_id(
    edition: &str,
    book: FullBook,
    page: u32,
    title: &str,
    used: &mut BTreeSet<String>,
) -> String {
    let mut fragment = slug(title);
    fragment.truncate(MAX_ID_SLUG);
    let fragment = fragment.trim_end_matches('-');
    let base = if fragment.is_empty() {
        format!("full-{edition}-{}-p{page}-section", book.as_str())
    } else {
        format!("full-{edition}-{}-p{page}-{fragment}", book.as_str())
    };
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("an infinite suffix loop always finds an unused id")
}

fn chapter_for(chapters: &[Chapter], page: u32, title: &str) -> String {
    if looks_like_chapter(title) {
        return title.to_string();
    }
    chapters
        .iter()
        .rev()
        .find(|chapter| chapter.page <= page)
        .map(|chapter| chapter.title.clone())
        .unwrap_or_else(|| "Front Matter".to_string())
}

fn looks_like_chapter(title: &str) -> bool {
    let text = title.trim().to_ascii_lowercase();
    let word = |prefix: &str| {
        text.strip_prefix(prefix).is_some_and(|rest| {
            rest.starts_with(|ch: char| ch.is_whitespace() || ch.is_ascii_digit() || ch == ':')
        })
    };
    word("chapter")
        || word("part")
        || word("appendix")
        || word("prologue")
        || word("epilogue")
        || (text.starts_with('第') && text.contains('章'))
}

fn page_references(body: &str) -> BTreeSet<u32> {
    let mut pages = BTreeSet::new();
    let bytes = body.as_bytes();
    let mut offset = 0;
    while let Some(position) = body[offset..].find("#p") {
        let start = offset + position + 2;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > start {
            if let Ok(page) = body[start..end].parse::<u32>() {
                pages.insert(page);
            }
        }
        offset = end.max(start);
    }
    pages
}

/// Chapter count recorded by the builder in the pack description. The pack
/// schema has no chapter-count field (changing it is out of this task's path),
/// so the release manifest reuses the same number instead of inventing a
/// second definition. Packs without that description fall back to distinct
/// section labels.
fn chapters_from_description(pack: &ContentPack) -> usize {
    let parsed = pack
        .pack
        .description
        .as_deref()
        .and_then(|description| description.split_once(" chapters"))
        .and_then(|(before, _)| before.rsplit(' ').next())
        .and_then(|count| count.parse::<usize>().ok());
    parsed.unwrap_or_else(|| chapter_count(pack))
}

fn chapter_count(pack: &ContentPack) -> usize {
    pack.content
        .rule_sections
        .as_ref()
        .map(|sections| {
            sections
                .iter()
                .filter_map(|section| section.section.as_deref())
                .collect::<BTreeSet<_>>()
                .len()
        })
        .unwrap_or(0)
}

fn book_from_id(id: &str) -> Option<String> {
    let rest = id.strip_prefix("full-")?;
    let (_, book) = rest.split_once('-')?;
    Some(book.to_string())
}

fn source_label(book: FullBook, input: &Input) -> String {
    match &input.sha256 {
        Some(hash) => format!(
            "{} (pdf2md index; sha256 {})",
            book.label(),
            &hash[..12.min(hash.len())]
        ),
        None => format!("{} (pdf2md index)", book.label()),
    }
}

fn pack_id(edition: &str, book: FullBook) -> String {
    format!("full-{edition}-{}", book.as_str())
}

fn normalize_edition(edition: &str) -> Result<&str> {
    match edition.trim() {
        "2014" => Ok("2014"),
        "2024" => Ok("2024"),
        other => bail!("unsupported full-pack edition `{other}`; expected 2014 or 2024"),
    }
}

/// `<a id="p41"></a>` starts a page.
fn split_pages(text: &str) -> (BTreeMap<u32, String>, usize, usize) {
    let mut pages = BTreeMap::new();
    let mut current: Option<u32> = None;
    let mut buffer = String::new();
    let mut anchors = 0;
    for line in text.lines() {
        if let Some(page) = page_anchor(line) {
            anchors += 1;
            if let Some(previous) = current.replace(page) {
                pages.insert(previous, std::mem::take(&mut buffer));
            }
            continue;
        }
        if current.is_none() || line.starts_with("<!-- PDF p.") {
            continue;
        }
        buffer.push_str(line);
        buffer.push('\n');
    }
    if let Some(previous) = current {
        pages.insert(previous, buffer);
    }
    let empty_pages = pages.values().filter(|text| text.trim().is_empty()).count();
    (pages, empty_pages, anchors)
}

fn page_anchor(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("<a id=\"p")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    rest[digits.len()..]
        .starts_with("\"></a>")
        .then(|| digits.parse().ok())
        .flatten()
}

fn heading_line(line: &str) -> Option<(usize, String)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim();
    if text.is_empty() {
        return None;
    }
    Some((hashes, text.to_string()))
}

/// Trims page noise and caps repeated blank lines while keeping every word.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blanks = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}
