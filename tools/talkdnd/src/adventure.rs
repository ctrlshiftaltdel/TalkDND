//! Scene-scoped reading of the locally imported adventure Markdown.
//!
//! The DM must not read the whole book at once: `talkdnd adventure section`
//! prints only the block under the matching heading and refuses (unless
//! `--reason` is given and logged) to print a chapter that is ahead of the
//! party's progress track. `--list` prints headings without any body text.
//!
//! Converted books live in `Adventures/<slug>/.md/<Book>/<Book>.md` and are
//! gitignored; this module only ever reads them at play time.

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
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    sections
        .iter()
        .find(|s| s.heading.to_lowercase() == q)
        .or_else(|| {
            sections
                .iter()
                .find(|s| s.heading.to_lowercase().contains(&q))
        })
        .or_else(|| {
            sections
                .iter()
                .find(|s| s.path.iter().any(|h| h.to_lowercase().contains(&q)))
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
    let (_, book) = read_book(root, save_dir)?;
    let sections = sections(&book);
    if sections.is_empty() {
        bail!("冒险 Markdown 里没有 Markdown 标题，无法按场景定位");
    }
    if list {
        let mut out = Vec::new();
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
