//! Secret terms and the pre-narration guard (`talkdnd guard`).
//!
//! Terms come from local, DM-only sources:
//! - `Saves/<slot>/dm-secrets.md`: `- term: X` lines and `[[X]]` markers;
//! - `Saves/<slot>/dm-plan.md`: the same syntax;
//! - the locally converted adventure: headings of chapters ahead of progress,
//!   plus headings and DM-only-section proper nouns / `[[X]]` terms;
//! - hidden aspects of registered original content (`original add --hidden`);
//! - manual entries (`talkdnd secret add`).
//!
//! Anything already in `reveals.yaml` is dropped. The merged list is cached in
//! `Saves/<slot>/secret-terms.cache.json`, keyed by the mtimes of every source.
//! Nothing here is ever committed: saves are gitignored.

use crate::adventure;
use crate::{original, reveal};
use anyhow::{bail, Result};
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const FILE: &str = "secret-terms.yaml";
pub const CACHE: &str = "secret-terms.cache.json";
const CACHE_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ManualTerm {
    pub term: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<u32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub added: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manual {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub terms: Vec<ManualTerm>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Term {
    pub term: String,
    pub source: String,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    key: String,
    terms: Vec<Term>,
}

fn manual_path(save_dir: &Path) -> PathBuf {
    save_dir.join(FILE)
}

fn load_manual(save_dir: &Path) -> Result<Manual> {
    let p = manual_path(save_dir);
    if !p.is_file() {
        return Ok(Manual::default());
    }
    let m: Manual = serde_norway::from_str(&fs::read_to_string(&p)?)
        .map_err(|e| anyhow::anyhow!("invalid {}: {e}", p.display()))?;
    if m.version > 1 {
        bail!(
            "{}: unsupported secret terms version {} (this talkdnd supports 1)",
            p.display(),
            m.version
        );
    }
    Ok(m)
}

fn store_manual(save_dir: &Path, m: &Manual) -> Result<()> {
    let mut current = m.clone();
    current.version = 1;
    let text = format!(
        "# TalkDND secret terms (DM only; players must never see this).\n# Add with `talkdnd secret add`, remove with `talkdnd secret remove`.\n{}",
        serde_norway::to_string(&current)?
    );
    atomic_write(manual_path(save_dir), text.as_bytes())
}

fn invalidate(save_dir: &Path) {
    let _ = fs::remove_file(save_dir.join(CACHE));
}

pub fn add(save_dir: &Path, term: &str, chapter: Option<u32>, note: &str) -> Result<String> {
    let term = term.trim();
    if term.is_empty() {
        bail!("秘密词不能为空");
    }
    let mut m = load_manual(save_dir)?;
    if m.terms.iter().any(|t| t.term.eq_ignore_ascii_case(term)) {
        bail!("秘密词「{term}」已经在列表里");
    }
    m.terms.push(ManualTerm {
        term: term.to_string(),
        chapter,
        note: note.trim().to_string(),
        added: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
    });
    store_manual(save_dir, &m)?;
    invalidate(save_dir);
    Ok(format!(
        "已加入秘密词「{term}」{}{}（只存在本存档，已 gitignore）",
        chapter.map(|c| format!("，第 {c} 章")).unwrap_or_default(),
        if note.trim().is_empty() {
            String::new()
        } else {
            format!("，备注：{}", note.trim())
        }
    ))
}

pub fn remove(save_dir: &Path, term: &str) -> Result<String> {
    let term = term.trim();
    let mut m = load_manual(save_dir)?;
    let before = m.terms.len();
    m.terms.retain(|t| !t.term.eq_ignore_ascii_case(term));
    if m.terms.len() == before {
        bail!(
            "手动秘密词里没有「{term}」；自动收集的词（dm-secrets.md、dm-plan.md、冒险书、原创隐藏面）不能单独删除，请改对应来源"
        );
    }
    store_manual(save_dir, &m)?;
    invalidate(save_dir);
    Ok(format!("已移除秘密词「{term}」"))
}

/// Manual plus computed terms, already filtered against `reveals.yaml`.
pub fn list(save_dir: &Path, root: &Path) -> Vec<Term> {
    collect(save_dir, root)
}

/// `talkdnd secret list`: term and where it came from.
pub fn list_lines(save_dir: &Path, root: &Path) -> Vec<String> {
    let terms = list(save_dir, root);
    if terms.is_empty() {
        return vec!["（还没有秘密词；用 `talkdnd secret add <词>` 或写 dm-secrets.md）".into()];
    }
    terms
        .iter()
        .map(|t| format!("{}  （{}）", t.term, t.source))
        .collect()
}

fn stamp(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(m) => {
            let nanos = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{nanos}:{}", m.len())
        }
        Err(_) => "missing".to_string(),
    }
}

fn cache_key(save_dir: &Path, root: &Path) -> String {
    let mut parts: Vec<String> = [
        "dm-secrets.md",
        original::PLAN_FILE,
        FILE,
        original::FILE,
        reveal::FILE,
    ]
    .iter()
    .map(|name| {
        let p = save_dir.join(name);
        format!("{}={}", p.display(), stamp(&p))
    })
    .collect();
    // `adventure::ahead_of_progress` depends on the track and party level.
    // These files are small; include a content hash so a same-size rewrite on
    // a coarse-mtime filesystem cannot leave a stale cache behind.
    let progress = save_dir.join(crate::progress::FILE);
    parts.push(format!("{}={}", progress.display(), deep_stamp(&progress)));
    for p in sheet_files(save_dir) {
        parts.push(format!("{}={}", p.display(), deep_stamp(&p)));
    }
    if let Some(book) = adventure::book_path(root, save_dir) {
        parts.push(format!("{}={}", book.display(), stamp(&book)));
    }
    parts.join("|")
}

fn deep_stamp(path: &Path) -> String {
    let quick = stamp(path);
    match fs::read(path) {
        Ok(bytes) => {
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(&bytes);
            format!("{quick}:{:x}", hash.finalize())
        }
        Err(_) => quick,
    }
}

/// Character/NPC Markdown files that supply the party level for the
/// ahead-of-progress check (path plus mtime/size and a content hash; saves
/// have few small files).
fn sheet_files(save_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["Characters", "NPCs"] {
        let Ok(rd) = fs::read_dir(save_dir.join(dir)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
        out.extend(files);
    }
    out
}

fn load_cache(save_dir: &Path, key: &str) -> Option<Vec<Term>> {
    let text = fs::read_to_string(save_dir.join(CACHE)).ok()?;
    let cache: Cache = serde_json::from_str(&text).ok()?;
    (cache.version == CACHE_VERSION && cache.key == key).then_some(cache.terms)
}

fn store_cache(save_dir: &Path, key: &str, terms: &[Term]) {
    let cache = Cache {
        version: CACHE_VERSION,
        key: key.to_string(),
        terms: terms.to_vec(),
    };
    if let Ok(text) = serde_json::to_string(&cache) {
        let _ = atomic_write(save_dir.join(CACHE), text.as_bytes());
    }
}

/// Extract `- term: X` and `[[X]]` terms from a DM notes file. Lines starting
/// with `#` are comments (the plan template ships commented examples).
pub fn parse_marked_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        for prefix in ["- term:", "- term：", "- 秘密词:", "- 秘密词："] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let term = rest
                    .split(" #")
                    .next()
                    .unwrap_or(rest)
                    .trim()
                    .trim_matches(['"', '\'', '“', '”'])
                    .trim();
                if !term.is_empty() {
                    out.push(term.to_string());
                }
            }
        }
        out.extend(brackets(t));
    }
    out
}

fn brackets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let term = after[..end].trim();
        if !term.is_empty() {
            out.push(term.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

/// A heading without its chapter/part numbering: `Chapter 3: The Vault` →
/// `The Vault`, `第3章：密室` → `密室`.
pub fn clean_heading(heading: &str) -> String {
    let h = heading.trim().trim_start_matches('#').trim();
    let lower = h.to_lowercase();
    for prefix in ["chapter", "part"] {
        if lower.starts_with(prefix) {
            let rest = &h[prefix.len()..];
            let rest = rest.trim_start_matches(|c: char| {
                c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == ':'
            });
            if let Some((_, tail)) = rest.split_once([':', '：']) {
                return tail.trim().to_string();
            }
            return rest.trim().to_string();
        }
    }
    if let Some(i) = h.find('第') {
        if let Some(j) = h[i..].find('章') {
            let tail = h[i + j + '章'.len_utf8()..].trim_start_matches([':', '：', ' ', '、', '.']);
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
    }
    let digits = h.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && h[digits..]
            .chars()
            .next()
            .is_some_and(|c| c == '.' || c == '、' || c == ' ')
    {
        return h[digits..].trim_start_matches(['.', '、', ' ']).to_string();
    }
    h.to_string()
}

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "if", "in", "on", "at", "to", "of", "for", "with",
    "from", "by", "as", "is", "are", "was", "were", "this", "that", "these", "those", "it", "its",
    "he", "she", "they", "you", "your", "their", "his", "her", "chapter", "part", "dm", "player",
    "when", "while", "after", "before", "during", "each", "every", "no", "not", "all", "any",
    "both", "read", "note", "notes", "secret", "secrets", "only", "use", "using", "then", "there",
    "here", "what", "which", "who", "how", "why", "can", "will", "would", "should",
];

/// Conservative proper-noun extraction for DM-only sections: runs of two or
/// more capitalised words (English). Chinese terms should be marked with
/// `[[...]]`; arbitrary 2-8 character CJK runs would flood the guard.
pub fn proper_nouns(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line
            .split(|c: char| !(c.is_alphabetic() || c == '\'' || c == '-'))
            .filter(|w| !w.is_empty())
            .collect();
        let mut run: Vec<&str> = Vec::new();
        let flush = |run: &mut Vec<&str>, out: &mut Vec<String>| {
            if run.len() >= 2 {
                out.push(run.join(" "));
            }
            run.clear();
        };
        for w in words {
            let first = w.chars().next().unwrap_or('a');
            let stop = STOP_WORDS.contains(&w.to_lowercase().as_str());
            if first.is_uppercase() && !stop {
                run.push(w);
                if run.len() >= 5 {
                    flush(&mut run, &mut out);
                }
            } else {
                flush(&mut run, &mut out);
            }
        }
        flush(&mut run, &mut out);
    }
    out
}

fn collect_uncached(save_dir: &Path, root: &Path) -> Vec<Term> {
    let mut map: BTreeMap<String, Term> = BTreeMap::new();
    let mut put = |term: &str, source: &str| {
        // Markdown emphasis left over from conversion (`_Wharf_`, `**X**`).
        let term = term
            .trim()
            .trim_matches(['"', '\'', '“', '”', '_', '*', '`'])
            .trim();
        if term.chars().count() < 2 {
            return;
        }
        map.entry(term.to_lowercase()).or_insert_with(|| Term {
            term: term.to_string(),
            source: source.to_string(),
        });
    };
    for file in ["dm-secrets.md", original::PLAN_FILE] {
        if let Ok(text) = fs::read_to_string(save_dir.join(file)) {
            for term in parse_marked_terms(&text) {
                put(&term, file);
            }
        }
    }
    if let Ok(manual) = load_manual(save_dir) {
        for t in &manual.terms {
            let mut source = "手动".to_string();
            if let Some(ch) = t.chapter {
                source += &format!("，第 {ch} 章");
            }
            if !t.note.is_empty() {
                source += &format!("，{}", t.note);
            }
            put(&t.term, &source);
        }
    }
    if let Ok(ledger) = original::load(save_dir) {
        for e in &ledger.entries {
            let Some(hidden) = e.hidden.as_deref() else {
                continue;
            };
            if original::revealed(save_dir, &e.id) {
                continue;
            }
            let source = format!("原创 {} 隐藏面", e.id);
            // The whole sentence catches a verbatim copy; [[terms]] and
            // proper nouns catch a paraphrase that names the secret.
            put(hidden, &source);
            for term in brackets(hidden) {
                put(&term, &source);
            }
            for term in proper_nouns(hidden) {
                put(&term, &source);
            }
        }
    }
    if let Ok((_, book)) = adventure::read_book(root, save_dir) {
        for (section, dm_only) in adventure::secret_sections(root, save_dir) {
            let source = format!("冒险书：{}", section.heading);
            let title = clean_heading(&section.heading);
            // A one-word English heading ("Wharf", "Tavern") is too generic to
            // be a secret on its own; names in DM-only bodies still count.
            let title_core = title.trim_matches(['_', '*', '`', ' ']);
            let one_ascii_word = title_core.is_ascii() && !title_core.contains(char::is_whitespace);
            if !one_ascii_word {
                put(&title, &source);
            }
            if dm_only {
                let body = section.text(&book);
                for term in brackets(body) {
                    put(&term, &source);
                }
                for term in proper_nouns(body) {
                    put(&term, &source);
                }
            }
        }
    }
    map.into_values()
        .filter(|t| !reveal::is_revealed(save_dir, &t.term))
        .collect()
}

pub fn collect(save_dir: &Path, root: &Path) -> Vec<Term> {
    let key = cache_key(save_dir, root);
    if let Some(terms) = load_cache(save_dir, &key) {
        return terms;
    }
    let terms = collect_uncached(save_dir, root);
    store_cache(save_dir, &key, &terms);
    terms
}

/// Substring match; an English term must also sit on word boundaries
/// ("Inn" does not match "inner"). Chinese terms match anywhere.
fn contains_term(hay: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }
    let word = |c: char| c.is_ascii_alphanumeric();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(term) {
        let start = from + pos;
        let end = start + term.len();
        let left_ok =
            !term.starts_with(word) || !hay[..start].chars().next_back().is_some_and(word);
        let right_ok = !term.ends_with(word) || !hay[end..].chars().next().is_some_and(word);
        if left_ok && right_ok {
            return true;
        }
        from = start + term.chars().next().map_or(1, char::len_utf8);
    }
    false
}

/// Guard output for `text`: one line per unrevealed secret term found.
/// Case-insensitive; English terms need word boundaries, Chinese terms match
/// as substrings.
pub fn guard_terms(save_dir: &Path, root: &Path, text: &str) -> Vec<String> {
    let hay = text.to_lowercase();
    collect(save_dir, root)
        .into_iter()
        .filter(|t| contains_term(&hay, &t.term.to_lowercase()))
        .map(|t| {
            format!(
                "剧透词：{}（建议改写为玩家已知的说法）  [来源：{}]",
                t.term, t.source
            )
        })
        .collect()
}

/// Read-only marker coverage of one DM notes file: how many explicit
/// markers (`- term:`, `- 秘密词:`, `[[词]]`) it has and which content lines
/// carry none. Only counts and 1-based line numbers are recorded — never
/// line text — so a report can be shown or logged without leaking secrets.
#[derive(Debug, Default, PartialEq)]
pub struct ScanReport {
    pub markers: usize,
    pub content_lines: usize,
    pub marked_lines: usize,
    pub unmarked_lines: Vec<usize>,
}

pub fn scan_text(text: &str) -> ScanReport {
    let mut rep = ScanReport::default();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        rep.content_lines += 1;
        let terms = parse_marked_terms(line);
        if terms.is_empty() {
            rep.unmarked_lines.push(i + 1);
        } else {
            rep.markers += terms.len();
            rep.marked_lines += 1;
        }
    }
    rep
}

/// `talkdnd secret scan [--from <file>]`: read-only coverage report for a DM
/// notes file (default: this save's `dm-secrets.md`). The source file is
/// never modified, the report holds counts and line numbers only — no file
/// content — and nothing here is written to the session log.
pub fn scan_lines(save_dir: &Path, from: Option<&str>) -> Result<Vec<String>> {
    let path = match from {
        Some(f) => PathBuf::from(f),
        None => save_dir.join("dm-secrets.md"),
    };
    if !path.is_file() {
        bail!("文件不存在：{}", path.display());
    }
    let rep = scan_text(&fs::read_to_string(&path)?);
    let mut out = vec![format!(
        "扫描 {}（只读；只报告数量与行号，不显示正文）",
        path.display()
    )];
    if rep.content_lines == 0 {
        out.push("- 文件为空或只有注释：没有需要保护的内容".into());
        return Ok(out);
    }
    out.push(format!(
        "- 显式标记：{} 个（`- term:` / `- 秘密词:` / `[[词]]`）",
        rep.markers
    ));
    out.push(format!(
        "- 内容行：{} 行，有标记 {} 行，未标记 {} 行",
        rep.content_lines,
        rep.marked_lines,
        rep.unmarked_lines.len()
    ));
    if rep.unmarked_lines.is_empty() {
        out.push("- 覆盖率：所有内容行都有显式标记".into());
    } else {
        let shown: Vec<String> = rep
            .unmarked_lines
            .iter()
            .take(30)
            .map(|l| l.to_string())
            .collect();
        out.push(format!(
            "- 未标记行：{}{}",
            shown.join(", "),
            if rep.unmarked_lines.len() > 30 {
                format!(" …（其余 {} 行）", rep.unmarked_lines.len() - 30)
            } else {
                String::new()
            }
        ));
        out.push(format!(
            "- 覆盖率：{}/{} 内容行有显式标记；未标记的叙述不会被当作秘密词",
            rep.marked_lines, rep.content_lines
        ));
    }
    if rep.markers == 0 {
        out.push(
            "- 警告：整份文件没有任何可识别标记；要保护的词用 `- term:` 或 `[[词]]` 标记，或用 `talkdnd secret import --from <文件>` 导入".into(),
        );
    }
    Ok(out)
}

/// Terms on one import line: the existing marker forms (`- term: X`,
/// `- 秘密词: X`, `[[X]]`) or one plain term per line. Comment and blank
/// lines yield nothing. `Err(())` marks a content line with no usable term:
/// an empty marker value, or a term shorter than two characters (the
/// collector never keeps shorter terms).
fn parse_import_line(line: &str) -> Result<Vec<String>, ()> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return Ok(Vec::new());
    }
    let mut marker = false;
    let mut out = Vec::new();
    for prefix in ["- term:", "- term：", "- 秘密词:", "- 秘密词："] {
        if let Some(rest) = t.strip_prefix(prefix) {
            marker = true;
            let term = rest
                .split(" #")
                .next()
                .unwrap_or(rest)
                .trim()
                .trim_matches(['"', '\'', '“', '”'])
                .trim();
            if !term.is_empty() {
                out.push(term.to_string());
            }
        }
    }
    let found = brackets(t);
    if !found.is_empty() {
        out.extend(found);
    } else if marker {
        // `- term:` with an empty value: a format error, not a whole-line term.
    } else {
        let term = t.trim_matches(['"', '\'', '“', '”']).trim();
        if term.is_empty() {
            return Err(());
        }
        out.push(term.to_string());
    }
    if out.is_empty() || out.iter().any(|term| term.chars().count() < 2) {
        return Err(());
    }
    Ok(out)
}

/// `talkdnd secret import --from <file> [--dry-run | --apply]`: import
/// DM-reviewed explicit terms, one per line or in the existing marker forms.
/// The default is a dry-run preview that writes nothing; only an explicit
/// apply writes `secret-terms.yaml`. Terms are deduplicated
/// case-insensitively against each other and the existing manual list.
///
/// Output boundaries (LetsPlay/docs/anti-spoiler.md, layer 4b): the dry-run
/// preview is a **DM-only local review** — it lists up to 20 pending terms
/// on stdout so the DM can review them before applying, but it writes
/// nothing, not `secret-terms.yaml` and not the session log, and it must
/// never be pasted into player-visible material. `scan` is the report that
/// never quotes file content (counts and line numbers only); apply counts
/// and errors are likewise content-free beyond the pending terms above.
pub fn import_lines(save_dir: &Path, from: &str, apply: bool) -> Result<Vec<String>> {
    let path = PathBuf::from(from);
    if !path.is_file() {
        bail!("文件不存在：{}", path.display());
    }
    let text = fs::read_to_string(&path)?;
    let mut candidates: Vec<String> = Vec::new();
    let mut error_lines: Vec<usize> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        match parse_import_line(line) {
            Ok(terms) => candidates.extend(terms),
            Err(()) => error_lines.push(i + 1),
        }
    }
    let existing = load_manual(save_dir)?;
    let mut seen: Vec<String> = existing
        .terms
        .iter()
        .map(|t| t.term.to_lowercase())
        .collect();
    let mut fresh: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    for term in candidates {
        let key = term.to_lowercase();
        if seen.contains(&key) {
            skipped += 1;
        } else {
            seen.push(key);
            fresh.push(term);
        }
    }
    let mut out = vec![if apply {
        format!("导入：{from}（显式词条，须为 DM 审核过的内容）")
    } else {
        format!("导入预览（dry-run，未写入存档）：{from}")
    }];
    if fresh.is_empty() {
        out.push("- 将新增 0 条".into());
    } else {
        out.push(format!("- 将新增 {} 条", fresh.len()));
        for term in fresh.iter().take(20) {
            out.push(format!("  - {term}"));
        }
        if fresh.len() > 20 {
            out.push(format!("  - …（其余 {} 条略）", fresh.len() - 20));
        }
    }
    out.push(format!("- 跳过 {skipped} 条（重复）"));
    if error_lines.is_empty() {
        out.push("- 格式错误 0 条".into());
    } else {
        let shown: Vec<String> = error_lines.iter().take(30).map(|l| l.to_string()).collect();
        out.push(format!(
            "- 格式错误 {} 条：第 {} 行（词条为空或不足 2 字；不导入）",
            error_lines.len(),
            shown.join(", ")
        ));
    }
    if !apply {
        out.push("确认无误后加 --apply 写入存档".into());
        return Ok(out);
    }
    if fresh.is_empty() {
        out.push("没有要新增的词条；存档未改动".into());
        return Ok(out);
    }
    let added = fresh.len();
    let mut m = existing;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    for term in fresh {
        m.terms.push(ManualTerm {
            term,
            chapter: None,
            note: String::new(),
            added: now.clone(),
        });
    }
    store_manual(save_dir, &m)?;
    invalidate(save_dir);
    out.push(format!(
        "已写入 {added} 条到 {FILE}（只存在本存档，已 gitignore）；秘密词缓存已失效"
    ));
    Ok(out)
}

/// `talkdnd start` digest warning: `dm-secrets.md` has content but no
/// recognized explicit marker, so nothing in it is protected. The warning
/// carries counts only, never file content. Missing, empty and comment-only
/// files stay quiet.
pub fn zero_marker_warning(save_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(save_dir.join("dm-secrets.md")).ok()?;
    let rep = scan_text(&text);
    if rep.markers > 0 || rep.content_lines == 0 {
        return None;
    }
    Some(format!(
        "dm-secrets.md 有内容但没有任何可识别的秘密标记（`- term:` / `- 秘密词:` / `[[词]]`）；普通段落不会被当作秘密词，{} 行内容未受保护。要保护的词需要显式标记，或用 `talkdnd secret import --from <文件>` 导入",
        rep.content_lines
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate;

    fn root(tag: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("talkdnd-secret-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&r);
        let save_dir = r.join("Saves/2030-01-01-01");
        for d in ["Sessions", "combat", "Characters"] {
            fs::create_dir_all(save_dir.join(d)).unwrap();
        }
        fs::write(
            save_dir.join("combat/campaign.yaml"),
            "party: []\nruleset: \"2024\"\n",
        )
        .unwrap();
        fs::write(save_dir.join("Sessions/s.md"), "# log\n").unwrap();
        migrate::init(&save_dir).unwrap();
        r
    }

    #[test]
    fn english_terms_need_word_boundaries_chinese_do_not() {
        assert!(contains_term("meet the red hand tonight", "red hand"));
        assert!(!contains_term("the inner hall", "inn"));
        assert!(contains_term("to the inn, then", "inn"));
        assert!(contains_term("你们发现了暗门后的密室", "密室"));
        assert!(!contains_term("anything", ""));
    }

    #[test]
    fn parses_marked_terms_and_ignores_comments() {
        let text = "# - term: hidden comment\n- term: Red Mask\nA clue about [[Silver Key]].\n# [[Not This]]\n";
        let terms = parse_marked_terms(text);
        assert_eq!(terms, vec!["Red Mask", "Silver Key"]);
        assert_eq!(clean_heading("Chapter 2: Second Job"), "Second Job");
        assert_eq!(clean_heading("第3章：密室"), "密室");
        assert_eq!(clean_heading("2. The Vault"), "The Vault");
        let nouns = proper_nouns("The butler is Red Mask and Lady Vex.\n");
        assert!(nouns.iter().any(|n| n == "Red Mask"), "{nouns:?}");
        assert!(nouns.iter().any(|n| n == "Lady Vex"), "{nouns:?}");
    }

    #[test]
    fn guard_flags_clean_and_excluded_terms() {
        let r = root("guard");
        let save = r.join("Saves/2030-01-01-01");
        fs::write(
            save.join("dm-secrets.md"),
            "# secrets\n- term: Red Mask\n- term: 密室\n",
        )
        .unwrap();
        add(&save, "Silver Key", Some(2), "prop").unwrap();
        let hits = guard_terms(
            &save,
            &r,
            "The Red Mask waits in the 密室 with the Silver Key.",
        );
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits
            .iter()
            .all(|h| h.starts_with("剧透词：") && h.contains("建议改写")));
        assert!(guard_terms(&save, &r, "A quiet street.").is_empty());
        // Case-insensitive Latin matching.
        assert_eq!(guard_terms(&save, &r, "RED MASK").len(), 1);
        // Revealed terms are dropped, and the cache is invalidated.
        crate::reveal::add(&save, "红面具就是 Red Mask", "test", &[]).unwrap();
        let hits = guard_terms(&save, &r, "Red Mask");
        assert!(hits.is_empty(), "{hits:?}");
        let listed = list_lines(&save, &r).join("\n");
        assert!(
            listed.contains("密室") && !listed.contains("Red Mask"),
            "{listed}"
        );
        assert!(remove(&save, "silver key").unwrap().contains("移除"));
        assert!(remove(&save, "silver key").is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn cache_invalidates_when_progress_or_party_level_changes() {
        let r = root("cache");
        let save = r.join("Saves/2030-01-01-01");
        fs::create_dir_all(save.join("Characters")).unwrap();
        fs::write(
            save.join("combat/campaign.yaml"),
            "party: [hero]\nruleset: \"2014\"\nadventures: [../../../Adventures/test-adventure/combat]\n",
        )
        .unwrap();
        fs::write(
            save.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nlevel: 1\nhp: 10\n```\n",
        )
        .unwrap();
        let track_first = "version: 1\ntrack:\n  - {name: First Job, level: 1}\n  - {name: Second Job, level: 3}\n";
        fs::write(save.join("progress.yaml"), track_first).unwrap();
        let book = r.join("Adventures/test-adventure/index");
        fs::create_dir_all(&book).unwrap();
        fs::write(
            book.join("Test Book.pages.md"),
            "# Test Book\n\n## Chapter 1: First Job\n\nfirst body\n\n## Chapter 2: Second Job\n\nsecond body\n",
        )
        .unwrap();

        let has = |terms: Vec<Term>| terms.iter().any(|t| t.term == "Second Job");
        assert!(
            has(collect(&save, &r)),
            "future chapter heading should be a secret term"
        );

        // A progress-only change (Second Job is now the current chapter) must
        // invalidate the cache even though every DM/book file is untouched.
        fs::write(
            save.join("progress.yaml"),
            "version: 1\ntrack:\n  - {name: First Job, level: 1}\n  - {name: Second Job, level: 1}\n",
        )
        .unwrap();
        assert!(
            !has(collect(&save, &r)),
            "progress change must invalidate the cache"
        );

        // Restore the track, then change only the party level: Second Job is
        // current at level 5, so it is no longer ahead.
        fs::write(save.join("progress.yaml"), track_first).unwrap();
        assert!(has(collect(&save, &r)));
        fs::write(
            save.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nlevel: 5\nhp: 40\n```\n",
        )
        .unwrap();
        assert!(
            !has(collect(&save, &r)),
            "party level change must invalidate the cache"
        );
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn original_hidden_aspects_feed_the_guard_until_revealed() {
        let r = root("orig");
        let save = r.join("Saves/2030-01-01-01");
        fs::write(
            save.join("save-meta.yaml"),
            "schema: 5\ntool: test\ndeviation: {scope: 2, ratio: 25}\nsettings: {dice_display: true}\n",
        )
        .unwrap();
        fs::write(
            save.join("Characters/Hero.md"),
            "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nlevel: 5\nhp: 20\n```\n",
        )
        .unwrap();
        let mut progress = String::from("version: 1\nxp_log:\n");
        for i in 0..20 {
            progress.push_str(&format!(
                "- {{date: '2026-01-01 10:{i:02}', category: quest, reason: unit{i}, each: 1, to: [hero]}}\n"
            ));
        }
        fs::write(save.join("progress.yaml"), progress).unwrap();
        original::add(
            &save,
            &original::New {
                kind: "npc",
                name: "Guide",
                rarity: None,
                note: "",
                hidden: "Guide is [[Red Mask]]",
                player_hook: false,
            },
        )
        .unwrap();
        let hits = guard_terms(&save, &r, "You meet Red Mask");
        assert!(hits.iter().any(|h| h.contains("Red Mask")), "{hits:?}");
        crate::reveal::add_original(&save, "O-001", None, "test").unwrap();
        assert!(guard_terms(&save, &r, "You meet Red Mask").is_empty());
        assert!(original::known_entries(&save)
            .iter()
            .any(|e| e.id == "O-001"));
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn scan_counts_markers_and_unmarked_lines_without_content() {
        let text = "# secrets\n- term: Red Mask\n- 秘密词: 红面具\n\nA clue about [[Silver Key]].\nplain unmarked narrative\n\n# comment [[Not This]]\n";
        let rep = scan_text(text);
        assert_eq!(rep.markers, 3, "{rep:?}");
        assert_eq!(rep.content_lines, 4, "{rep:?}");
        assert_eq!(rep.marked_lines, 3, "{rep:?}");
        assert_eq!(rep.unmarked_lines, vec![6], "{rep:?}");
        assert_eq!(scan_text(""), ScanReport::default());
        assert_eq!(scan_text("# only a comment\n").content_lines, 0);
    }

    #[test]
    fn import_parses_marker_and_plain_lines() {
        assert_eq!(
            parse_import_line("- term: Red Mask").unwrap(),
            vec!["Red Mask"]
        );
        assert_eq!(
            parse_import_line("- 秘密词：红面具").unwrap(),
            vec!["红面具"]
        );
        assert_eq!(
            parse_import_line("A [[Silver Key]] clue").unwrap(),
            vec!["Silver Key"]
        );
        assert_eq!(parse_import_line("红面具").unwrap(), vec!["红面具"]);
        assert_eq!(
            parse_import_line("\"Quoted Name\"").unwrap(),
            vec!["Quoted Name"]
        );
        assert_eq!(
            parse_import_line("# comment").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(parse_import_line("").unwrap(), Vec::<String>::new());
        // Malformed: empty marker value, or a term shorter than two chars.
        assert!(parse_import_line("- term:").is_err());
        assert!(parse_import_line("- 秘密词:  ").is_err());
        assert!(parse_import_line("x").is_err());
        assert!(parse_import_line("\"\"").is_err());
    }

    #[test]
    fn import_dry_run_writes_nothing_apply_dedupes_and_invalidates() {
        let r = root("import");
        let save = r.join("Saves/2030-01-01-01");
        let from = r.join("terms.txt");
        fs::write(
            &from,
            "# DM reviewed\n- term: Red Mask\n红面具\nSilver Key\nRed Mask\n- term:\nx\n",
        )
        .unwrap();
        // Dry-run: preview only, nothing is written to the save. The
        // preview is the DM-only local review: it lists the pending terms
        // on stdout, but neither the save nor the session log changes.
        let log_before = fs::read_to_string(save.join("Sessions/s.md")).unwrap();
        let out = import_lines(&save, from.to_str().unwrap(), false)
            .unwrap()
            .join("\n");
        assert!(out.contains("dry-run"), "{out}");
        assert!(
            out.contains("Red Mask") && out.contains("Silver Key") && out.contains("红面具"),
            "{out}"
        );
        assert!(out.contains("将新增 3 条"), "{out}");
        assert!(out.contains("跳过 1 条"), "{out}");
        assert!(out.contains("格式错误 2 条：第 6, 7 行"), "{out}");
        assert!(!save.join(FILE).exists(), "{out}");
        assert_eq!(
            fs::read_to_string(save.join("Sessions/s.md")).unwrap(),
            log_before,
            "dry-run must not touch the session log"
        );
        // Populate the cache with the pre-import state.
        assert!(guard_terms(&save, &r, "the Red Mask waits").is_empty());
        // Apply writes, and the new term is guarded right away (cache gone).
        let out = import_lines(&save, from.to_str().unwrap(), true)
            .unwrap()
            .join("\n");
        assert!(out.contains("已写入 3 条"), "{out}");
        assert!(list(&save, &r).iter().any(|t| t.term == "红面具"));
        assert!(save.join(FILE).exists(), "only apply writes {FILE}");
        assert_eq!(
            fs::read_to_string(save.join("Sessions/s.md")).unwrap(),
            log_before,
            "apply writes secret-terms.yaml, never the session log"
        );
        assert!(
            !guard_terms(&save, &r, "the Red Mask waits").is_empty(),
            "imported terms must be guarded after the cache is invalidated"
        );
        // A second apply adds nothing: everything is a duplicate now.
        let out = import_lines(&save, from.to_str().unwrap(), true)
            .unwrap()
            .join("\n");
        assert!(
            out.contains("将新增 0 条") && out.contains("存档未改动"),
            "{out}"
        );
        // Revealed terms stay filtered.
        crate::reveal::add(&save, "Red Mask 已揭示", "test", &[]).unwrap();
        assert!(guard_terms(&save, &r, "the Red Mask waits").is_empty());
        assert!(scan_lines(&save, Some("no/such/file.md")).is_err());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn scan_lines_reports_counts_only_and_never_content() {
        let r = root("scanlines");
        let save = r.join("Saves/2030-01-01-01");
        fs::write(
            save.join("dm-secrets.md"),
            "- term: Red Mask\nplain narrative line\n",
        )
        .unwrap();
        let out = scan_lines(&save, None).unwrap().join("\n");
        assert!(out.contains("显式标记：1 个"), "{out}");
        assert!(
            out.contains("内容行：2 行，有标记 1 行，未标记 1 行"),
            "{out}"
        );
        assert!(out.contains("未标记行：2"), "{out}");
        assert!(!out.contains("plain narrative line"), "{out}");
        // Comment-only file: quiet, no coverage warning.
        fs::write(save.join("other.md"), "# DM 秘密（只给 DM）\n\n").unwrap();
        let out = scan_lines(&save, Some(save.join("other.md").to_str().unwrap()))
            .unwrap()
            .join("\n");
        assert!(out.contains("没有需要保护的内容"), "{out}");
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn zero_marker_warning_only_for_nonempty_unmarked_files() {
        let r = root("zerowarn");
        let save = r.join("Saves/2030-01-01-01");
        assert!(zero_marker_warning(&save).is_none(), "missing file");
        fs::write(save.join("dm-secrets.md"), "").unwrap();
        assert!(zero_marker_warning(&save).is_none(), "empty file");
        fs::write(save.join("dm-secrets.md"), "   \n# 只给 DM\n\n").unwrap();
        assert!(zero_marker_warning(&save).is_none(), "comment-only file");
        fs::write(save.join("dm-secrets.md"), "整段叙述没有任何显式标记。\n").unwrap();
        let w = zero_marker_warning(&save).unwrap();
        assert!(
            w.contains("没有任何可识别的秘密标记") && w.contains("1 行内容未受保护"),
            "{w}"
        );
        assert!(!w.contains("整段叙述"), "{w}");
        fs::write(save.join("dm-secrets.md"), "- term: 红面具\n").unwrap();
        assert!(zero_marker_warning(&save).is_none(), "marked file");
        let _ = fs::remove_dir_all(&r);
    }
}
