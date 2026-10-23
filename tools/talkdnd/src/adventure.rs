//! Scene-scoped reading of the locally imported adventure Markdown.
//!
//! The DM must not read the whole book at once: `talkdnd adventure section`
//! prints only the block under the matching heading and refuses (unless
//! `--reason` is given and logged) to print a chapter that is ahead of the
//! party's progress track. `--list` prints headings without any body text.
//! `talkdnd adventure search` finds lines in the page edition and prints the
//! PDF page number of every hit.
//!
//! Matching is fuzzy at query time only: the stored Markdown is never rewritten.
//! Case, curly/straight quotes, ligatures, punctuation, whitespace and
//! hyphenated line breaks are normalised on both sides; a line may also lose or
//! garble its first 1-4 characters (scanned books often have a shadow at the
//! gutter, e.g. `ne` for `One` or `..mown` for `known`).
//!
//! Converted books live in `Adventures/<slug>/.md/<Book>/` and are gitignored;
//! this module only ever reads them at play time.

use crate::progress::{self, Progress};
use crate::{original, save, start};
use anyhow::{anyhow, bail, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub level: usize,
    pub heading: String,
    pub start: usize,
    pub end: usize,
    /// Byte range in the book (robust to CRLF, unlike line indices).
    pub start_byte: usize,
    pub end_byte: usize,
    /// Ancestor headings, outermost first, then this heading.
    pub path: Vec<String>,
}

impl Section {
    pub fn text<'a>(&self, book: &'a str) -> &'a str {
        let start = self.start_byte.min(book.len());
        let end = self.end_byte.min(book.len()).max(start);
        &book[start..end]
    }
}

fn heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    let rest = trimmed[level..].trim();
    if rest.is_empty() {
        return None;
    }
    Some((level, rest.to_string()))
}

/// Split Markdown into heading-delimited sections. A section runs to the next
/// heading of the same or a higher level, so a chapter includes its subsections.
pub fn sections(book: &str) -> Vec<Section> {
    let mut lines: Vec<&str> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    let mut pos = 0;
    for line in book.split_inclusive('\n') {
        offsets.push(pos);
        pos += line.len();
        lines.push(line);
    }
    let mut heads: Vec<(usize, usize, String)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some((level, heading)) = heading(line) {
            heads.push((i, level, heading));
        }
    }
    let mut out = Vec::new();
    for (n, (start, level, heading)) in heads.iter().enumerate() {
        let mut end_line = lines.len();
        for (next, next_level, _) in heads.iter().skip(n + 1) {
            if next_level <= level {
                end_line = *next;
                break;
            }
        }
        let mut path: Vec<String> = heads[..n]
            .iter()
            .filter(|(_, l, _)| *l < *level)
            .map(|(_, _, h)| h.clone())
            .collect();
        path.push(heading.clone());
        out.push(Section {
            level: *level,
            heading: heading.clone(),
            start: *start,
            end: end_line,
            start_byte: offsets.get(*start).copied().unwrap_or(book.len()),
            end_byte: offsets.get(end_line).copied().unwrap_or(book.len()),
            path,
        });
    }
    out
}

/// The save's converted adventure book (reader version, else `index.md`).
pub fn book_path(root: &Path, save_dir: &Path) -> Option<PathBuf> {
    let slug = start::adventure_slug(save_dir)?;
    let md = root.join("Adventures").join(&slug).join(".md");
    let mut books: Vec<PathBuf> = fs::read_dir(md)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    books.sort();
    for b in books {
        let name = b.file_name()?.to_string_lossy().to_string();
        let reader = b.join(format!("{name}.md"));
        if reader.is_file() {
            return Some(reader);
        }
        let index = b.join("index.md");
        if index.is_file() {
            return Some(index);
        }
    }
    None
}

/// The page edition (page anchors, then `index.md`) used by `adventure search`.
/// Search promises a PDF page number for every hit, so the reader edition
/// (which has no anchors) is not used here.
pub fn lookup_path(root: &Path, save_dir: &Path) -> Option<PathBuf> {
    let slug = start::adventure_slug(save_dir)?;
    let md = root.join("Adventures").join(&slug).join(".md");
    let mut books: Vec<PathBuf> = fs::read_dir(md)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    books.sort();
    for b in books {
        let name = b.file_name()?.to_string_lossy().to_string();
        let pages = b.join(format!("{name}.pages.md"));
        if pages.is_file() {
            return Some(pages);
        }
        let index = b.join("index.md");
        if index.is_file() {
            return Some(index);
        }
    }
    None
}

/// Fold one string for query-time matching. Punctuation and whitespace are
/// dropped; case, quotes, ligatures and hyphenated line breaks are normalised.
pub fn normalize_for_search(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, '-' | '\u{2010}' | '\u{2011}') {
            // "exam-\nple" -> "example"; also allow "exam- \r\nple".
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], ' ' | '\t') {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '\n' || chars[j] == '\r') {
                i = j + 1;
                continue;
            }
        }
        match c {
            '\u{00ad}' => {}
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            // Curly quotes, apostrophes, dashes and other punctuation vanish.
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => {}
        }
        i += 1;
    }
    out
}

/// Does `line` contain `query` (both already normalised), allowing the line to
/// have lost or garbled its first 1-4 characters?
fn fuzzy_hit(line: &str, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    if line.contains(query) {
        return true;
    }
    let q: Vec<char> = query.chars().collect();
    for k in 1..=4.min(q.len()) {
        // A two-letter residue is too weak for k > 1; keep it only for a
        // single missing/garbled first character.
        let min_len = if k == 1 { 2 } else { 3 };
        let rest: String = q[k..].iter().collect();
        if rest.chars().count() < min_len {
            continue;
        }
        if let Some(pos) = line.find(&rest) {
            if line[..pos].chars().count() <= 4 {
                return true;
            }
        }
    }
    false
}

fn ends_hyphen(s: &str) -> bool {
    s.chars()
        .last()
        .is_some_and(|c| matches!(c, '-' | '\u{2010}' | '\u{2011}'))
}

fn starts_lower(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_lowercase())
}

/// Group physical lines into logical lines, undoing hyphenated line breaks.
fn logical_lines<'a, I: IntoIterator<Item = &'a str>>(lines: I) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if !cur.is_empty() && ends_hyphen(&cur) && starts_lower(line) {
            cur.pop();
            cur.push_str(line);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push_str(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn anchor_page(line: &str) -> Option<usize> {
    let start = line.find("<a id=\"p")? + "<a id=\"p".len();
    let digits: String = line[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn index_page(line: &str) -> Option<usize> {
    for pat in ["#p", "p."] {
        for (i, _) in line.match_indices(pat) {
            let start = i + pat.len();
            let digits: String = line[start..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(n) = digits.parse() {
                return Some(n);
            }
        }
    }
    None
}

fn is_page_meta(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("<a id=")
        || t.starts_with("<!-- PDF p.")
        || t.starts_with("> Low-quality extraction (")
}

/// `(PDF page, logical line)` pairs from the page edition. Page 0 means an
/// `index.md` line where no page could be parsed (never returned by search).
fn lookup_lines(md: &str, paged: bool) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    if paged {
        let mut page = 0usize;
        let mut block: Vec<&str> = Vec::new();
        let flush = |page: usize, block: &mut Vec<&str>, out: &mut Vec<(usize, String)>| {
            if page > 0 {
                for line in logical_lines(block.drain(..)) {
                    out.push((page, line));
                }
            } else {
                block.clear();
            }
        };
        for raw in md.lines() {
            if let Some(n) = anchor_page(raw) {
                flush(page, &mut block, &mut out);
                page = n;
                continue;
            }
            if is_page_meta(raw) {
                continue;
            }
            block.push(raw);
        }
        flush(page, &mut block, &mut out);
    } else {
        for raw in md.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("Read:") {
                continue;
            }
            if let Some(page) = index_page(line) {
                out.push((page, line.to_string()));
            }
        }
    }
    out
}

/// Short intake-audit line read from `index.json` / `manifest.json`.
fn intake_line(dir: &Path) -> Option<String> {
    for name in ["index.json", "manifest.json"] {
        let Ok(text) = fs::read_to_string(dir.join(name)) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(a) = v.get("intake").or_else(|| v.get("audit")) else {
            continue;
        };
        let Some(verdict) = a.get("verdict").and_then(|x| x.as_str()) else {
            continue;
        };
        let get = |k: &str| a.get(k).and_then(|x| x.as_u64());
        let mut bits = Vec::new();
        if let Some(n) = get("bookmarks") {
            bits.push(format!("书签 {n}"));
        }
        if let (Some(z), Some(p)) = (get("zero_pages"), get("pages")) {
            bits.push(format!("0 字页 {z}/{p}"));
        }
        if let Some(r) = a.get("garbage_rate").and_then(|x| x.as_f64()) {
            bits.push(format!("乱码 {:.2}%", r * 100.0));
        }
        if let Some(ch) = a.get("chapters").and_then(|x| x.as_array()) {
            bits.push(format!("章节起点 {}", ch.len()));
        }
        let detail = if bits.is_empty() {
            String::new()
        } else {
            format!("（{}）", bits.join("，"))
        };
        return Some(format!("文字层：{verdict}{detail}"));
    }
    None
}

/// `talkdnd adventure search <query>`: matching lines with PDF page numbers.
pub fn search(root: &Path, save_dir: &Path, query: &str, limit: usize) -> Result<Vec<String>> {
    let path = lookup_path(root, save_dir).ok_or_else(|| {
        anyhow!(
            "没有找到本存档冒险的页码版 Markdown（Adventures/<slug>/.md/<书名>/<书名>.pages.md 或 index.md）；先按 docs/pdf2md.md 转换 PDF"
        )
    })?;
    let q = normalize_for_search(query);
    if q.is_empty() {
        bail!("搜索词去掉标点后为空");
    }
    let md = fs::read_to_string(&path).unwrap_or_default();
    if md.trim().is_empty() {
        bail!("{} is empty", path.display());
    }
    let paged = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".pages.md"));
    let mut out = Vec::new();
    if let Some(line) = intake_line(path.parent().unwrap_or(Path::new("."))) {
        out.push(line);
    }
    let mut hits = 0;
    for (page, line) in lookup_lines(&md, paged) {
        if fuzzy_hit(&normalize_for_search(&line), &q) {
            out.push(format!("p.{page}: {line}"));
            hits += 1;
            if limit > 0 && hits >= limit {
                break;
            }
        }
    }
    if hits == 0 {
        out.push(format!(
            "没有找到含「{}」的正文行；试试更短的词，或用 `talkdnd adventure section --list` 看标题（不显示正文）",
            query.trim()
        ));
    }
    Ok(out)
}

pub fn read_book(root: &Path, save_dir: &Path) -> Result<(PathBuf, String)> {
    let path = book_path(root, save_dir).ok_or_else(|| {
        anyhow!(
            "没有找到本存档冒险的本地 Markdown（Adventures/<slug>/.md/<书名>/）；先按 docs/pdf2md.md 转换 PDF，或用 `talkdnd chargen set adventure \"<书名>\"` 确认冒险"
        )
    })?;
    let text = fs::read_to_string(&path).unwrap_or_default();
    if text.trim().is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok((path, text))
}

/// Heading index of the current chapter in the progress track.
pub fn current_track_index(save_dir: &Path) -> Option<usize> {
    let p = Progress::load(save_dir).ok()?;
    let track = progress::track(save_dir, &p);
    if track.is_empty() {
        return None;
    }
    let ctx = original::context(save_dir).ok()?;
    track
        .iter()
        .position(|s| s.name.eq_ignore_ascii_case(&ctx.chapter))
        .or_else(|| track.iter().rposition(|s| s.level <= ctx.party_level))
}

/// Leading chapter numbering (`Chapter 3: X`, `第 3 章`, `3. X`), if any.
fn chapter_number(heading: &str) -> Option<usize> {
    let lower = heading.to_lowercase();
    let rest = lower
        .strip_prefix("chapter")
        .or_else(|| lower.strip_prefix("part"))
        .map(str::trim_start);
    if let Some(rest) = rest {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<usize>() {
            return Some(n);
        }
    }
    if let Some(i) = heading.find('第') {
        let rest = &heading[i + '第'.len_utf8()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<usize>() {
            return Some(n);
        }
    }
    let digits: String = heading.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() && heading[digits.len()..].starts_with(['.', '、', ' ']) {
        if let Ok(n) = digits.parse::<usize>() {
            return Some(n);
        }
    }
    None
}

/// If this section belongs to a chapter ahead of the party's current one,
/// return that chapter's label. Conservative: an unknown chapter is allowed.
pub fn ahead_of_progress(save_dir: &Path, section: &Section) -> Option<String> {
    let p = Progress::load(save_dir).ok()?;
    let track = progress::track(save_dir, &p);
    let current = current_track_index(save_dir)?;
    let hay = format!("{} {}", section.path.join(" / "), section.heading).to_lowercase();
    let mut matched = None;
    for (i, step) in track.iter().enumerate() {
        if hay.contains(&step.name.to_lowercase()) {
            matched = Some(i);
        }
    }
    if let Some(i) = matched {
        if i > current {
            return Some(track[i].name.clone());
        }
        return None;
    }
    if let Some(n) = chapter_number(&section.heading)
        .or_else(|| section.path.iter().find_map(|h| chapter_number(h)))
    {
        if n > current + 1 {
            return Some(format!("第 {n} 章"));
        }
    }
    None
}

const DM_ONLY_MARKERS: &[&str] = &[
    "dm only",
    "dm-only",
    "dm 专用",
    "dm专用",
    "城主",
    "dm notes",
    "dm's notes",
    "secret",
    "秘密",
];

pub fn dm_only(section: &Section) -> bool {
    let hay = format!("{} {}", section.heading, section.path.join(" ")).to_lowercase();
    DM_ONLY_MARKERS.iter().any(|m| hay.contains(m))
}

fn find_section<'a>(sections: &'a [Section], query: &str) -> Option<&'a Section> {
    let q = normalize_for_search(query);
    if q.is_empty() {
        return None;
    }
    let heading = |s: &&Section| normalize_for_search(&s.heading);
    sections
        .iter()
        .find(|s| heading(s) == q)
        .or_else(|| sections.iter().find(|s| fuzzy_hit(&heading(s), &q)))
        .or_else(|| {
            sections.iter().find(|s| {
                s.path
                    .iter()
                    .any(|h| fuzzy_hit(&normalize_for_search(h), &q))
            })
        })
}

/// `talkdnd adventure section <query> [--list] [--reason …]`.
pub fn section(
    root: &Path,
    save_dir: &Path,
    query: &str,
    list: bool,
    reason: Option<&str>,
) -> Result<Vec<String>> {
    let (path, book) = read_book(root, save_dir)?;
    let sections = sections(&book);
    if sections.is_empty() {
        bail!("冒险 Markdown 里没有 Markdown 标题，无法按场景定位");
    }
    let audit = intake_line(path.parent().unwrap_or(Path::new(".")));
    if list {
        let mut out = Vec::new();
        if let Some(line) = audit {
            out.push(line);
        }
        for s in &sections {
            let mark = if dm_only(s) { " [DM-only]" } else { "" };
            out.push(format!(
                "{}{} {}{mark}",
                "  ".repeat(s.level.saturating_sub(1)),
                "#".repeat(s.level),
                s.heading
            ));
        }
        return Ok(out);
    }
    let found = find_section(&sections, query).ok_or_else(|| {
        anyhow!(
            "没有找到标题含「{}」的小节；先 `talkdnd adventure section --list` 看标题（不显示正文）",
            query.trim()
        )
    })?;
    let ahead = ahead_of_progress(save_dir, found);
    let dm_only = dm_only(found);
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    if (ahead.is_some() || dm_only) && reason.is_none() {
        let why = if let Some(ch) = &ahead {
            format!("这一节属于后面的章节「{ch}」，超出当前进度")
        } else {
            "这一节标为 DM-only".to_string()
        };
        bail!(
            "{why}。按 docs/anti-spoiler.md 先只读当前场景；确有必要超前阅读时，用 --reason \"<原因>\" 明确记录"
        );
    }
    let mut out = vec![format!("# {}", found.path.join(" / "))];
    if let Some(line) = audit {
        out.push(line);
    }
    if let Some(ch) = &ahead {
        out.push(format!(
            "（超前阅读：{ch}；原因：{}）",
            reason.unwrap_or("")
        ));
    } else if dm_only {
        out.push(format!("（DM-only 小节；原因：{}）", reason.unwrap_or("")));
    }
    out.push(found.text(&book).trim_end().to_string());
    if ahead.is_some() || dm_only {
        let what = if let Some(ch) = &ahead {
            format!("超前阅读：{}（{ch}）", found.heading)
        } else {
            format!("DM-only 阅读：{}", found.heading)
        };
        if let Some(log) = save::latest_log(save_dir) {
            save::append(
                &log,
                &format!(
                    "- [{}] {what}；原因：{}",
                    chrono::Local::now().format("%H:%M"),
                    reason.unwrap_or("")
                ),
            )?;
        }
    }
    Ok(out)
}

/// Sections that feed the secret-term list: everything ahead of progress and
/// every DM-only section. The caller extracts terms from headings (ahead) and
/// bodies (DM-only); bodies are never printed by `talkdnd adventure section`
/// unless the DM explicitly asks for that section with a reason.
pub fn secret_sections(root: &Path, save_dir: &Path) -> Vec<(Section, bool)> {
    let Ok((_, book)) = read_book(root, save_dir) else {
        return Vec::new();
    };
    sections(&book)
        .into_iter()
        .filter_map(|s| {
            let ahead = ahead_of_progress(save_dir, &s).is_some();
            let dm = dm_only(&s);
            (ahead || dm).then_some((s, dm))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("talkdnd-adv-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let save_dir = r.join("Saves/2030-01-01-01");
        for d in ["Sessions", "combat", "Characters"] {
            fs::create_dir_all(save_dir.join(d)).unwrap();
        }
        fs::write(
            save_dir.join("combat/campaign.yaml"),
            "party: [hero]\nruleset: \"2014\"\nadventures: [../../../Adventures/test-adventure/combat]\n",
        )
        .unwrap();
        fs::write(
            save_dir.join("Sessions/2030-01-01-session-00.md"),
            "# log\n",
        )
        .unwrap();
        fs::write(
            save_dir.join("progress.yaml"),
            "version: 1\ntrack:\n  - {name: First Job, level: 1}\n  - {name: Second Job, level: 3}\n",
        )
        .unwrap();
        fs::write(
            save_dir.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nlevel: 1\nhp: 10\n```\n",
        )
        .unwrap();
        let book = r.join("Adventures/test-adventure/.md/Test Book");
        fs::create_dir_all(&book).unwrap();
        fs::write(
            book.join("Test Book.md"),
            "# Test Book\n\nintro\n\n## Chapter 1: First Job\n\n### The Inn\n\nfirst body\n\n### The Cellar\n\ncellar body\n\n## Chapter 2: Second Job\n\nsecond body\n\n## DM Notes\n\n- The butler is [[Red Mask]]\n",
        )
        .unwrap();
        r
    }

    #[test]
    fn sections_are_heading_delimited_and_listed_without_body() {
        let r = root("list");
        let save = r.join("Saves/2030-01-01-01");
        let text = fs::read_to_string(book_path(&r, &save).unwrap()).unwrap();
        let s = sections(&text);
        let inn = s.iter().find(|x| x.heading == "The Inn").unwrap();
        assert!(inn.text(&text).contains("first body"));
        assert!(!inn.text(&text).contains("cellar body"));
        assert_eq!(s[0].heading, "Test Book");
        assert!(
            s[0].text(&text).contains("first body"),
            "chapter includes subsections"
        );
        let list = section(&r, &save, "", true, None).unwrap().join("\n");
        assert!(list.contains("Chapter 2: Second Job"));
        assert!(!list.contains("second body"));
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn ahead_sections_need_a_reason_and_are_logged() {
        let r = root("ahead");
        let save = r.join("Saves/2030-01-01-01");
        // Current chapter = First Job (party level 1).
        let err = section(&r, &save, "Second Job", false, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("后面的章节") && err.contains("--reason"),
            "{err}"
        );
        assert!(
            !fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md"))
                .unwrap()
                .contains("超前阅读")
        );
        let out = section(&r, &save, "Second Job", false, Some("player skipped ahead"))
            .unwrap()
            .join("\n");
        assert!(out.contains("second body"), "{out}");
        assert!(out.contains("超前阅读"));
        let log = fs::read_to_string(save.join("Sessions/2030-01-01-session-00.md")).unwrap();
        assert!(
            log.contains("超前阅读") && log.contains("player skipped ahead"),
            "{log}"
        );
        // Current chapter is allowed without a reason.
        let out = section(&r, &save, "The Inn", false, None)
            .unwrap()
            .join("\n");
        assert!(
            out.contains("first body") && !out.contains("second body"),
            "{out}"
        );
        let _ = fs::remove_dir_all(&r);
    }

    fn write_pages(r: &Path, body: &str) -> PathBuf {
        let book = r.join("Adventures/test-adventure/.md/Test Book");
        fs::create_dir_all(&book).unwrap();
        let pages = book.join("Test Book.pages.md");
        fs::write(&pages, body).unwrap();
        pages
    }

    #[test]
    fn search_is_fuzzy_and_reports_pdf_pages() {
        let r = root("search");
        let save = r.join("Saves/2030-01-01-01");
        write_pages(
            &r,
            "# Test Book\n\n\
             <a id=\"p1\"></a>\n<!-- PDF p.1 -->\nOne morning the exam-\nple was clear.\n\n\
             <a id=\"p2\"></a>\n<!-- PDF p.2 -->\n..mown to all, the “hero’s” ﬁnal ﬂight.\n\n\
             <a id=\"p3\"></a>\n<!-- PDF p.3 -->\nThe vault is below.\n\n\
             <a id=\"p4\"></a>\n<!-- PDF p.4 -->\nne does not wait.\n\n\
             <a id=\"p5\"></a>\n<!-- PDF p.5 -->\nult of the old keep.\n\n\
             <a id=\"p6\"></a>\n<!-- PDF p.6 -->\nsure lies beneath.\n",
        );
        let hits = |q: &str| search(&r, &save, q, 0).unwrap().join("\n");
        // hyphenated line break
        let out = hits("example");
        assert!(
            out.contains("p.1: One morning the example was clear."),
            "{out}"
        );
        // scanned gutter shadow taking the first two letters of "known"
        let out = hits("known");
        assert!(out.contains("p.2:"), "{out}");
        // curly quotes + ligatures + punctuation/whitespace
        let out = hits("hero's final flight");
        assert!(out.contains("p.2:"), "{out}");
        let out = hits("hero’s  ﬁnal-flight");
        assert!(out.contains("p.2:"), "{out}");
        // 1 missing leading character ("One" -> "ne")
        let out = hits("one");
        assert!(out.contains("p.4:"), "{out}");
        // 3 missing leading characters ("vault" -> "ult")
        let out = hits("vault");
        assert!(out.contains("p.5:") && out.contains("p.3:"), "{out}");
        // 4 missing leading characters ("treasure" -> "sure")
        let out = hits("treasure");
        assert!(out.contains("p.6:"), "{out}");
        // every hit carries a PDF page number
        let all = hits("the");
        for line in all.lines() {
            assert!(line.starts_with("p."), "no page number: {line}");
        }
        // limit
        assert_eq!(
            search(&r, &save, "the", 1)
                .unwrap()
                .iter()
                .filter(|l| l.starts_with("p."))
                .count(),
            1
        );
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn search_falls_back_to_index_md_pages() {
        let r = root("search-index");
        let save = r.join("Saves/2030-01-01-01");
        let book = r.join("Adventures/test-adventure/.md/Test Book");
        fs::write(
            book.join("index.md"),
            "# Index: Test Book\n\n## Headings\n\n- [Final Flight](<Test Book.pages.md#p9>) - p.9\n",
        )
        .unwrap();
        let out = search(&r, &save, "final flight", 0).unwrap().join("\n");
        assert!(out.contains("p.9:"), "{out}");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn section_headings_match_fuzzily_without_rewriting_the_book() {
        let r = root("fuzzy-head");
        let save = r.join("Saves/2030-01-01-01");
        let (_, book) = read_book(&r, &save).unwrap();
        let s = sections(&book);
        assert_eq!(find_section(&s, "theinn").unwrap().heading, "The Inn");
        assert_eq!(find_section(&s, "he Cellar").unwrap().heading, "The Cellar");
        // The stored text is untouched: only the query is normalised.
        assert!(book.contains("### The Cellar"));
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn dm_only_sections_need_a_reason() {
        let r = root("dmonly");
        let save = r.join("Saves/2030-01-01-01");
        let err = section(&r, &save, "DM Notes", false, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("DM-only"), "{err}");
        let out = section(&r, &save, "DM Notes", false, Some("prep"))
            .unwrap()
            .join("\n");
        assert!(out.contains("Red Mask"), "{out}");
        let _ = fs::remove_dir_all(&r);
    }
}
