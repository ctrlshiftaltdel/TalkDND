//! Extract the SRD rules prose that the 5e-database JSON does not carry.
//!
//! The 2024 SRD is shipped as structured data for monsters, spells, classes and
//! items, but the 5e-database 2024 dataset has no rules chapters. The local
//! pdf2md index has the full SRD text, so this importer walks the page markdown
//! between known chapter headings and writes a prose pack.
//!
//! Only SRD text is read. The pdf2md output itself is local and gitignored; the
//! generated pack is committed under CC-BY-4.0.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::content::pack::{
    ContentPack, Distribution, PackEntry, PackIndex, PackMeta, SCHEMA_VERSION,
};
use crate::content::schema::ContentFile;
use crate::srd::slug;

/// Chapters extracted by this importer, in book order. Other chapters are
/// already covered by the structured packs (monsters, spells, items, classes).
const TARGET_CHAPTERS: [&str; 4] = [
    "Playing the Game",
    "Character Creation",
    "Rules Glossary",
    "Gameplay Toolbox",
];

/// All top-level chapters, used to find where a target chapter ends.
const ALL_CHAPTERS: [&str; 12] = [
    "Playing the Game",
    "Character Creation",
    "Classes",
    "Character Origins",
    "Feats",
    "Equipment",
    "Spells",
    "Rules Glossary",
    "Gameplay Toolbox",
    "Magic Items",
    "Monsters",
    "Animals",
];

/// Sections whose body is shorter than this are heading noise and are skipped.
const MIN_BODY_CHARS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterReport {
    pub name: String,
    pub start_page: u32,
    pub end_page: u32,
    pub sections: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProseReport {
    pub edition: String,
    pub pack_id: String,
    pub entities: usize,
    pub chapters: Vec<ChapterReport>,
    pub skipped: usize,
    pub duplicates: Vec<String>,
    pub gaps: Vec<String>,
}

struct Section {
    chapter: String,
    heading: String,
    page: u32,
    body: String,
}

/// Imports one edition's rules prose from a pdf2md `index/` directory.
pub fn import(edition: &str, index_dir: &Path, out: &Path) -> Result<ProseReport> {
    let edition = match edition {
        "2024" => "2024",
        other => bail!("unsupported prose edition `{other}`; only 2024 has prose gaps"),
    };
    let manifest: Value = read_json(&index_dir.join("manifest.json"))?;
    let index: Value = read_json(&index_dir.join("index.json"))?;

    let pdf = manifest
        .get("pdf")
        .and_then(Value::as_str)
        .context("pdf2md manifest has no `pdf` field")?;
    let stem = pdf.strip_suffix(".pdf").unwrap_or(pdf);
    let pages_path = index_dir.join(format!("{stem}.pages.md"));
    let pages_text = std::fs::read_to_string(&pages_path)
        .with_context(|| format!("read pdf2md pages {}", pages_path.display()))?;
    let pages = split_pages(&pages_text);
    if pages.is_empty() {
        bail!("no page anchors in {}", pages_path.display());
    }
    let page_count = manifest
        .get("page_count")
        .and_then(Value::as_u64)
        .unwrap_or(pages.len() as u64) as u32;

    let starts = chapter_starts(&index, page_count);
    let boundaries = chapter_boundaries(&index, page_count);
    let mut report = ProseReport {
        edition: edition.to_string(),
        ..ProseReport::default()
    };

    let mut values: Vec<Value> = Vec::new();
    let mut entries = Vec::new();
    let mut index_by_id: BTreeMap<String, usize> = BTreeMap::new();
    for chapter in TARGET_CHAPTERS {
        let Some(&start) = starts.get(chapter) else {
            report.gaps.push(format!(
                "chapter `{chapter}` not found in the pdf2md index; skipped"
            ));
            continue;
        };
        // Chapter headings are not always level 2 ("Magic Items" is a
        // level-4 heading in the 2024 SRD), so the end boundary considers the
        // next known chapter title at any heading level.
        let end = boundaries
            .values()
            .flatten()
            .filter(|page| **page > start)
            .min()
            .copied()
            .unwrap_or(page_count + 1);
        let sections = chapter_sections(&pages, chapter, start, end);
        report.chapters.push(ChapterReport {
            name: chapter.to_string(),
            start_page: start,
            end_page: end.saturating_sub(1),
            sections: sections.len(),
        });
        for section in sections {
            if section.body.trim().chars().count() < MIN_BODY_CHARS {
                report.skipped += 1;
                continue;
            }
            let id = section_id(edition, &section);
            if let Some(&index) = index_by_id.get(&id) {
                // The same heading can appear twice when a section continues on
                // the next page or when a running header repeats. Merge real
                // extra text; ignore a literal repeat.
                let extra = tidy(&section.body);
                let existing = values[index]["text"].as_str().unwrap_or("").to_string();
                if extra.chars().count() >= MIN_BODY_CHARS && !existing.contains(&extra) {
                    values[index]["text"] = json!(format!("{existing}\n\n{extra}"));
                    report.duplicates.push(format!("{id} (merged)"));
                } else {
                    report.duplicates.push(format!("{id} (repeated)"));
                }
                continue;
            }
            let text = tidy(&section.body);
            values.push(json!({
                "id": id,
                "name": section.heading,
                "source": format!("SRD 5.2.1 (pdf2md {})", manifest.get("tool").and_then(Value::as_str).unwrap_or("?")),
                "section": section.chapter,
                "page": section.page,
                "tags": ["rule", "srd", "2024"],
                "text": text,
                "data": {"chapter": section.chapter},
            }));
            index_by_id.insert(id.clone(), values.len() - 1);
            entries.push(PackEntry {
                id,
                kind: "rule-section".to_string(),
                name: Some(section.heading),
                aliases: Vec::new(),
                page: Some(section.page as i32),
                section: Some(section.chapter),
                refs: Vec::new(),
            });
        }
    }

    if values.is_empty() {
        bail!("no prose sections extracted from {}", index_dir.display());
    }
    values.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("id").and_then(Value::as_str).unwrap_or(""))
    });
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    let content: ContentFile = serde_json::from_value(json!({"rule_sections": values}))
        .context("generated prose does not match the content schema")?;
    let pack_id = "srd-5.2.1-rules-prose".to_string();
    let meta = PackMeta {
        id: pack_id.clone(),
        name: Some("SRD 5.2.1 Rules Prose".to_string()),
        edition: edition.to_string(),
        source: Some("SRD 5.2.1 (pdf2md)".to_string()),
        license: Some("CC-BY-4.0".to_string()),
        distribution: Distribution::Srd,
        schema_version: SCHEMA_VERSION,
        content_hash: None,
        created_at: None,
        description: Some(format!(
            "{} rules sections from the SRD 5.2.1 (Playing the Game, Character Creation, Rules Glossary, Gameplay Toolbox).",
            values.len()
        )),
    };
    let mut pack = ContentPack::new(meta, content)?;
    pack.index = PackIndex {
        entries,
        images: Vec::new(),
    };
    pack.validate()?;

    std::fs::create_dir_all(out)
        .with_context(|| format!("create pack output directory {}", out.display()))?;
    let file = format!("{pack_id}.pack.json");
    let text = serde_json::to_string_pretty(&pack).context("serialize prose pack")?;
    std::fs::write(out.join(&file), format!("{text}\n"))
        .with_context(|| format!("write prose pack {}", out.join(&file).display()))?;

    report.pack_id = pack_id;
    report.entities = pack.entity_count();
    Ok(report)
}

fn read_json(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

/// First page of each known chapter, by exact level-2 heading text.
fn chapter_starts(index: &Value, page_count: u32) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    let Some(headings) = index.get("headings").and_then(Value::as_array) else {
        return out;
    };
    for heading in headings {
        let level = heading.get("level").and_then(Value::as_u64).unwrap_or(0);
        let text = heading.get("text").and_then(Value::as_str).unwrap_or("");
        let page = heading.get("page").and_then(Value::as_u64).unwrap_or(0) as u32;
        if level != 2 || page == 0 || page > page_count {
            continue;
        }
        if ALL_CHAPTERS.contains(&text) {
            out.entry(text.to_string()).or_insert(page);
        }
    }
    out
}

/// Pages where each known chapter title appears, at any heading level. Used
/// only to find where a chapter ends.
fn chapter_boundaries(index: &Value, page_count: u32) -> BTreeMap<String, Vec<u32>> {
    let mut out: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    let Some(headings) = index.get("headings").and_then(Value::as_array) else {
        return out;
    };
    for heading in headings {
        let text = heading.get("text").and_then(Value::as_str).unwrap_or("");
        let page = heading.get("page").and_then(Value::as_u64).unwrap_or(0) as u32;
        if page == 0 || page > page_count || !ALL_CHAPTERS.contains(&text) {
            continue;
        }
        out.entry(text.to_string()).or_default().push(page);
    }
    for pages in out.values_mut() {
        pages.sort_unstable();
        pages.dedup();
    }
    out
}

/// Splits the chapter page range into level-2/3 sections.
fn chapter_sections(
    pages: &BTreeMap<u32, String>,
    chapter: &str,
    start: u32,
    end: u32,
) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current: Option<Section> = None;
    let mut chapter_title: Option<String> = None;
    for page in start..end {
        let Some(text) = pages.get(&page) else {
            continue;
        };
        for line in text.lines() {
            if let Some((level, heading)) = heading_line(line) {
                if level <= 3 {
                    if let Some(section) = current.take() {
                        sections.push(section);
                    }
                    if level == 2 && chapter_title.is_none() {
                        chapter_title = Some(heading.clone());
                    }
                    current = Some(Section {
                        chapter: chapter_title.clone().unwrap_or_else(|| chapter.to_string()),
                        heading,
                        page,
                        body: String::new(),
                    });
                    continue;
                }
            }
            if let Some(section) = current.as_mut() {
                section.body.push_str(line);
                section.body.push('\n');
            }
        }
    }
    if let Some(section) = current.take() {
        sections.push(section);
    }
    sections
}

fn section_id(edition: &str, section: &Section) -> String {
    let chapter = slug(&section.chapter);
    let heading = slug(&section.heading);
    if chapter == heading || chapter.is_empty() {
        format!("rule-{edition}-{heading}")
    } else {
        format!("rule-{edition}-{chapter}-{heading}")
    }
}

/// `<a id="p41"></a>` starts a page.
fn split_pages(text: &str) -> BTreeMap<u32, String> {
    let mut pages = BTreeMap::new();
    let mut current: Option<u32> = None;
    let mut buffer = String::new();
    for line in text.lines() {
        if let Some(page) = page_anchor(line) {
            if let Some(previous) = current.replace(page) {
                pages.insert(previous, std::mem::take(&mut buffer));
            }
            continue;
        }
        if current.is_none() {
            continue;
        }
        if line.starts_with("<!-- PDF p.") {
            continue;
        }
        buffer.push_str(line);
        buffer.push('\n');
    }
    if let Some(previous) = current {
        pages.insert(previous, buffer);
    }
    pages
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
