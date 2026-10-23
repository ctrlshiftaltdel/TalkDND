//! `talkdnd start`: one command to get a DM ready, plus a compact digest.

use crate::chargen;
use crate::save;
use anyhow::Result;
use combat_sim::fsutil::atomic_write;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

pub fn combat_sim_path(root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("TALKDND_COMBAT_SIM") {
        return PathBuf::from(p);
    }
    let name = if cfg!(windows) {
        "combat-sim.exe"
    } else {
        "combat-sim"
    };
    root.join("bin").join(name)
}

/// Runs `combat-sim content check`; returns a one-line summary.
///
/// The result is cached in `Saves/<slot>/.cache/content-check.json` and reused
/// while the binary and every file the check reads are unchanged. That removes
/// a full process spawn from the common `talkdnd start` path (P3-1).
pub fn content_check(root: &Path, save: &Path) -> (bool, String) {
    let bin = combat_sim_path(root);
    let manifest = save.join("combat/campaign.yaml");
    if !manifest.is_file() {
        return (false, "no combat/campaign.yaml".into());
    }
    if !bin.is_file() {
        return (
            false,
            format!("{} missing — see bin/README.md", bin.display()),
        );
    }
    let key = content_key(root, save, &bin, &manifest);
    if let Some(cache) = load_cache(save) {
        if cache.key == key {
            return (cache.ok, cache.summary);
        }
    }
    let result = run_content_check(&bin, &manifest);
    store_cache(
        save,
        &ContentCache {
            key,
            ok: result.0,
            summary: result.1.clone(),
        },
    );
    result
}

fn run_content_check(bin: &Path, manifest: &Path) -> (bool, String) {
    match Command::new(bin)
        .arg("--campaign")
        .arg(manifest)
        .args(["content", "check"])
        .output()
    {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr);
            let summary = text
                .lines()
                .rev()
                .find(|l| l.contains("error(s)"))
                .or_else(|| text.lines().find(|l| l.starts_with("Error")))
                .unwrap_or_else(|| text.lines().last().unwrap_or(""))
                .trim()
                .trim_start_matches("content check:")
                .trim()
                .to_string();
            (out.status.success(), summary)
        }
        Err(e) => (false, format!("cannot run combat-sim: {e}")),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ContentCache {
    key: String,
    ok: bool,
    summary: String,
}

fn cache_path(save: &Path) -> PathBuf {
    save.join(".cache/content-check.json")
}

fn load_cache(save: &Path) -> Option<ContentCache> {
    let text = fs::read_to_string(cache_path(save)).ok()?;
    serde_json::from_str(&text).ok()
}

fn store_cache(save: &Path, cache: &ContentCache) {
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = atomic_write(cache_path(save), text.as_bytes());
    }
}

/// `mtime(ns):len`, or `missing`, so the key changes when a file changes.
fn stamp(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(m) => {
            let nanos = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{nanos}:{}", m.len())
        }
        Err(_) => "missing".to_string(),
    }
}

fn rel_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_files(dir: &Path, md_only: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, md_only, out);
        } else if path.is_file() && (!md_only || path.extension().is_some_and(|x| x == "md")) {
            out.push(path);
        }
    }
}

/// `combat/` directories from the campaign manifest's inline `adventures:` list.
fn adventure_dirs(save: &Path) -> Vec<PathBuf> {
    let dir = save.join("combat");
    let text = fs::read_to_string(dir.join("campaign.yaml")).unwrap_or_default();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("adventures:") else {
            continue;
        };
        let rest = rest
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['[', ']']);
        return rest
            .split(',')
            .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let p = PathBuf::from(&s);
                if p.is_absolute() {
                    p
                } else {
                    dir.join(p)
                }
            })
            .collect();
    }
    Vec::new()
}

/// Stable hash over the combat-sim binary and every input of `content check`.
fn content_key(root: &Path, save: &Path, bin: &Path, manifest: &Path) -> String {
    let mut rows = vec![
        format!("bin|{}|{}", rel_key(root, bin), stamp(bin)),
        format!("manifest|{}|{}", rel_key(root, manifest), stamp(manifest)),
    ];
    let mut files = Vec::new();
    collect_files(&root.join("Rules/combat"), false, &mut files);
    collect_files(&save.join("Characters"), true, &mut files);
    collect_files(&save.join("NPCs"), true, &mut files);
    for dir in adventure_dirs(save) {
        collect_files(&dir, false, &mut files);
    }
    let mut entries: Vec<String> = files
        .iter()
        .map(|p| format!("{}|{}", rel_key(root, p), stamp(p)))
        .collect();
    entries.sort();
    rows.extend(entries);
    let mut hash = Sha256::new();
    for row in rows {
        hash.update(row.as_bytes());
        hash.update(b"\n");
    }
    format!("{:x}", hash.finalize())
}

/// Starts pdf2md in the background. Never waits: a missing `uv` shows up as a
/// spawn error in the digest instead of a blocking `uv --version` probe.
pub fn kick_pdf2md(root: &Path) -> String {
    if std::env::var_os("TALKDND_NO_PDF2MD").is_some() {
        return "skipped (TALKDND_NO_PDF2MD)".into();
    }
    if !root.join("tools/pdf2md/pyproject.toml").is_file() {
        return "skipped (tools/pdf2md missing)".into();
    }
    let log = root.join("tools/pdf2md/.run.log");
    let out = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .ok();
    let mut cmd = Command::new("uv");
    cmd.current_dir(root)
        .args([
            "run",
            "--project",
            "tools/pdf2md",
            "python",
            "-m",
            "pdf2md",
            "run",
        ])
        .stdin(Stdio::null());
    match out.and_then(|f| f.try_clone().ok().map(|g| (f, g))) {
        Some((a, b)) => {
            cmd.stdout(a).stderr(b);
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    match cmd.spawn() {
        Ok(_) => "started in background (single instance; log tools/pdf2md/.run.log)".into(),
        Err(e) => format!("not started: {e}"),
    }
}

struct Member {
    id: String,
    name: String,
    hp: String,
    ac: String,
    file: String,
    /// `player` or `dm` (see `charsheet::controller_for`).
    controller: &'static str,
}

pub(crate) fn combat_block(text: &str) -> Option<serde_norway::Value> {
    let start = text.find("```combat yaml")?;
    let rest = &text[start..];
    let body_start = rest.find('\n')? + 1;
    let end = rest[body_start..].find("```")?;
    serde_norway::from_str(&rest[body_start..body_start + end]).ok()
}

fn members(save: &Path) -> Vec<Member> {
    let mut v = Vec::new();
    for dir in ["Characters", "NPCs"] {
        let Ok(rd) = fs::read_dir(save.join(dir)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for f in files {
            let Ok(text) = fs::read_to_string(&f) else {
                continue;
            };
            let Some(y) = combat_block(&text) else {
                continue;
            };
            let get = |k: &str| {
                y.get(k).map(|v| match v {
                    serde_norway::Value::String(s) => s.clone(),
                    other => serde_norway::to_string(other)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                })
            };
            let id = get("id").unwrap_or_default();
            let sheet = crate::check::Sheet {
                id: id.clone(),
                name: String::new(),
                file: f.clone(),
                block: y.clone(),
            };
            let controller = crate::charsheet::controller_for(save, &sheet);
            v.push(Member {
                controller,
                id,
                name: get("name").unwrap_or_default(),
                hp: get("hp").unwrap_or("?".into()),
                ac: get("ac").unwrap_or("?".into()),
                file: format!(
                    "{dir}/{}",
                    f.file_name().unwrap_or_default().to_string_lossy()
                ),
            });
        }
    }
    v
}

/// Rules source for the digest; imports book data when it is missing or stale.
fn rules_source_line(root: &Path, rs: &str) -> String {
    let books = crate::classdata::rulebooks(&root.join("Rules").join(rs));
    if books.is_empty() {
        return format!("- 规则来源：仅 SRD（Rules/{rs}/ 下没有转换好的完整规则书；见 docs/chargen.md「本地规则书数据」）\n");
    }
    let mut s = String::new();
    if crate::classdata::import_stale(root, rs) {
        match crate::classdata::run_import(root, rs) {
            Ok((_, subs, bgs)) => s += &format!("- 本地规则书数据：已自动导入（{subs} 个子职、{bgs} 个背景，Rules/chargen/local/，不提交）\n"),
            Err(e) => s += &format!("- 本地规则书数据：自动导入失败（{e}）；手动运行 `talkdnd data import --ruleset {rs}`\n"),
        }
    }
    s += &format!(
        "- 规则来源：**完整规则书优先**（SRD 只在书里找不到时兜底）。页码版（`<a id=\"pN\"></a>` = PDF 第 N 页）：{}\n",
        books.iter().map(|p| rel_root(root, p)).collect::<Vec<_>>().join("；"),
    );
    s
}

pub fn party_ids(save: &Path) -> Vec<String> {
    let text = fs::read_to_string(save.join("combat/campaign.yaml")).unwrap_or_default();
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("party:") {
            let rest = rest.split('#').next().unwrap_or("");
            return rest
                .trim()
                .trim_matches(['[', ']'])
                .split(',')
                .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    Vec::new()
}

fn campaign_name(save: &Path) -> String {
    let text = fs::read_to_string(save.join("combat/campaign.yaml")).unwrap_or_default();
    text.lines()
        .find_map(|l| l.strip_prefix("name:"))
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…（截断，全文见日志）", &s[..end])
}

pub struct Options {
    pub pdf2md: bool,
}

/// Per-phase wall-clock timings for `TALKDND_TIMING=1` (stderr only).
struct Timing {
    on: bool,
    start: Instant,
    last: Instant,
}

impl Timing {
    fn new() -> Self {
        let on = std::env::var("TALKDND_TIMING")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        let now = Instant::now();
        Self {
            on,
            start: now,
            last: now,
        }
    }

    fn mark(&mut self, phase: &str) {
        let now = Instant::now();
        if self.on {
            eprintln!(
                "timing: {phase} {:.1}ms",
                (now - self.last).as_secs_f64() * 1000.0
            );
        }
        self.last = now;
    }

    fn total(&self) {
        if self.on {
            eprintln!(
                "timing: total {:.1}ms",
                self.start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}

/// Adventure slug from the save's chargen, else from the campaign manifest.
pub(crate) fn adventure_slug(save: &Path) -> Option<String> {
    if let Ok(c) = chargen::load(save) {
        if let Some(s) = c.adventure_slug() {
            return Some(s);
        }
    }
    let text = fs::read_to_string(save.join("combat/campaign.yaml")).unwrap_or_default();
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("adventures:") {
            let rest = rest
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['[', ']']);
            if let Some(first) = rest.split(',').next() {
                let p = first.trim().trim_matches(['"', '\'']);
                // .../Adventures/<slug>/combat
                if let Some(slug) = p.rsplit('/').nth(1) {
                    return Some(slug.to_string());
                }
            }
        }
    }
    None
}

fn rel_root(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The digest's adventure-book lines: the book folder(s), PDFs and the
/// paged Markdown index when it exists.
fn adventure_lines(root: &Path, save: &Path) -> String {
    let Some(slug) = adventure_slug(save) else {
        return "- 冒险书：还没设置（`talkdnd chargen set adventure \"<书名>\"`）\n".into();
    };
    let folders = crate::library::adventure_folders(root, &slug);
    let mut out = if folders.is_empty() {
        format!("- 冒险书：`Adventures/{slug}/`\n")
    } else {
        format!(
            "- 冒险书：{}\n",
            folders
                .iter()
                .map(|d| format!("`{}/`", rel_root(root, d)))
                .collect::<Vec<_>>()
                .join("、")
        )
    };
    let pdfs = crate::library::adventure_pdfs(root, &slug);
    let mds = book_mds(&crate::library::adventure_indexes(root, &slug));
    for p in &mds {
        out += &format!("  - Markdown：`{}`\n", rel_root(root, p));
    }
    for p in &pdfs {
        out += &format!("  - PDF：`{}`\n", rel_root(root, p));
    }
    if mds.is_empty() {
        if pdfs.is_empty() {
            out += &format!("  - 这里还没有 PDF；把冒险书 PDF 放进 `Adventures/`（`talkdnd start` 会自动整理到 `Adventures/{slug}/`）\n");
        } else {
            out += "  - Markdown 还没转换：先查上面的 PDF（转换后索引在同一文件夹的 `index/<书名>.pages.md`）\n";
        }
    }
    out
}

/// Each converted book's paged Markdown (`index/<book>.pages.md`, PDF page
/// anchors) and its `index.md` (headings, maps, stat blocks). Page chunks,
/// fixes and review files are working files and stay out of the digest.
fn book_mds(indexes: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in indexes {
        if let Some(p) = crate::library::paged_markdown(d) {
            out.push(p);
        }
        let idx = d.join("index.md");
        if idx.is_file() {
            out.push(idx);
        }
    }
    out
}

pub fn digest(root: &Path, save: &Path, opts: &Options) -> Result<String> {
    let mut timing = Timing::new();
    let slot = save
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let rs = save::ruleset(save);
    // Tidy the book library before pdf2md starts (it skips itself while a
    // worker is converting) and before anything reads a book path.
    let tidy = match crate::library::tidy(root, false) {
        Ok(r) => r.lines(),
        Err(e) => vec![format!(
            "书库整理失败：{e:#}（书本文件没有删除；可运行 `talkdnd library tidy --dry-run` 查看）"
        )],
    };
    timing.mark("library_tidy");
    let (ok, check) = content_check(root, save);
    timing.mark("content_check");
    let pdf = if opts.pdf2md {
        kick_pdf2md(root)
    } else {
        "skipped (--no-pdf2md)".into()
    };
    timing.mark("kick_pdf2md");
    let migration = crate::migrate::ensure(save, root, false)?;
    let repoint = crate::migrate::repoint_books(save, root, false)?;
    timing.mark("migrate");
    let default_notes = crate::update::sync_defaults(root)?;
    timing.mark("sync_defaults");
    let update_notes = crate::update::sync_installed_files(root)?;
    timing.mark("sync_installed_files");
    let log = save::latest_log(save);
    let mut warnings = Vec::new();
    let mut d = String::new();
    d += &format!("# 接档摘要：{slot}\n\n");
    d += &format!(
        "- 规则：**{rs}**（只用 `Rules/{rs}/` 与 `Rules/rulings.md`、`Rules/rulings-{rs}.md`；出厂默认在 `Rules/defaults/`，`Rules/` 下的是本机副本）\n"
    );
    let name = campaign_name(save);
    if !name.is_empty() {
        d += &format!("- 战役：{name}\n");
    }
    d += &format!("- content check：{check}\n- pdf2md：{pdf}\n");
    d += &format!(
        "- 骰子显示：{}（{}；掷骰始终由程序完成并记入 `rolls.log`）\n",
        if crate::settings::dice_display(save) {
            "开"
        } else {
            "关"
        },
        if crate::settings::dice_display(save) {
            "展示完整算式"
        } else {
            "只叙述结果，不报骰值/DC/修正；`talkdnd log rolls` 可查"
        }
    );
    d += &rules_source_line(root, &rs);
    timing.mark("rules_source_line");
    if !ok {
        warnings.push("content check 未通过：战斗前修好，或战斗手动结算".to_string());
    }
    match &log {
        Some(p) => {
            d += &format!(
                "- 最新日志：`Sessions/{}`\n",
                p.file_name().unwrap_or_default().to_string_lossy()
            )
        }
        None => warnings.push("没有会话日志".into()),
    }
    if let Ok(c) = chargen::load(save) {
        if let Some((step, label)) = c.next_step() {
            d += &format!("\n## 建卡未完成\n\n下一步：`{step}` — {label}。按 AGENTS.md「开新局」流程继续，`talkdnd chargen status` 看详情。\n");
        }
    }
    d += "\n## 存档结构\n\n";
    for line in migration.lines() {
        d += &format!("- {line}\n");
    }
    for line in &repoint {
        d += &format!("- {line}\n");
    }
    for line in &tidy {
        d += &format!("- {line}\n");
    }
    for line in &default_notes {
        d += &format!("- {line}\n");
    }
    for line in &update_notes {
        d += &format!("- {line}\n");
    }
    d += "\n## 最后存档点\n\n";
    let cp = log
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| save::last_checkpoint(&t));
    match cp {
        Some(c) => d += &truncate(&c, 1400),
        None => {
            d += "（日志里还没有 `## 存档点`，从日志末尾接着读）";
        }
    }
    d += "\n\n## 队伍（角色卡数据块，maxHP / AC）\n\n";
    let party = party_ids(save);
    let mut ms = members(save);
    // Player characters first, then companions (stable: file order inside).
    ms.sort_by_key(|m| m.controller == crate::charsheet::DM);
    for m in &ms {
        let mark = if party.contains(&m.id) {
            ""
        } else {
            "（不在 party）"
        };
        let role = if m.controller == crate::charsheet::DM {
            "同伴（DM 操控）"
        } else {
            "玩家角色"
        };
        d += &format!(
            "- {} `{}`［{role}］：maxHP {}，AC {} — `{}`{mark}\n",
            m.name, m.id, m.hp, m.ac, m.file
        );
        if m.file.starts_with("NPCs/") && party.contains(&m.id) {
            warnings.push(format!(
                "队友 `{}` 的角色卡还在 `{}`；队友应在 `Characters/`（`talkdnd migrate` 会移动）",
                m.id, m.file
            ));
        }
    }
    for p in &party {
        if !ms.iter().any(|m| &m.id == p) {
            warnings.push(format!("party 里的 `{p}` 没有角色卡数据块"));
        }
    }
    let ps_path = save.join("combat").join(combat_sim::party::FILE_NAME);
    d += "\n## 当前状态（combat/party-state.yaml，战斗命令自动更新）\n\n";
    match combat_sim::party::PartyState::load(&ps_path) {
        Ok(ps) if !ps.members.is_empty() => {
            for line in ps.summary_lines() {
                d += &format!("- {line}\n");
            }
        }
        Ok(_) => d += "- 还没有记录：第一场战斗或 `combat-sim party show` 后生成；之前满血满资源\n",
        Err(e) => warnings.push(format!("party-state.yaml 读不了：{e}")),
    }
    timing.mark("members/party");
    d += "\n## 开场前再读（按需）\n\n";
    d += "- `dm-secrets.md`（只给 DM）、`canon.md`、`Characters/_party.md`（同伴声音与关系）；`NPCs/` 只放剧情 NPC\n";
    d += &adventure_lines(root, save);
    if save.join("house-rules.md").is_file() {
        d += "- `house-rules.md`：本存档开局前商定的房规（局中不改）\n";
    }
    let recent = crate::reveal::recent(save, 8);
    d += "\n## 玩家已知（`talkdnd known` 的压缩版；描述与选项只能从这里出发）\n\n";
    for line in crate::known::digest_lines(save) {
        // Sub-items already start with "- "; indent them under their heading.
        if line.starts_with("- ") {
            d += &format!("  {line}\n");
        } else {
            d += &format!("- {line}\n");
        }
    }
    if !recent.is_empty() {
        d += "\n### 最近揭示（提到专名前先 `talkdnd reveal check`）\n\n";
        for r in recent {
            d += &format!("- {}\n", truncate(&r, 200));
        }
    }
    match crate::audit::audit(save) {
        Ok(r) => {
            d += "\n## 存档校验\n\n";
            if r.created {
                d += "- 旧存档：已按当前文件建立校验基准（integrity.yaml）\n";
            }
            if r.clean() {
                d += "- OK：文件与上次程序写入一致，数值变化都有日志\n";
            }
            for l in r.lines() {
                warnings.push(l);
            }
        }
        Err(e) => warnings.push(format!("存档校验失败：{e}")),
    }
    d += "\n## 剧本偏离度（只看自己）\n\n";
    match crate::original::status_lines(save) {
        Ok(lines) => {
            for line in lines {
                d += &format!("- {line}\n");
            }
        }
        Err(e) => warnings.push(format!("剧本偏离度读不了：{e}")),
    }
    timing.mark("audit");
    if !warnings.is_empty() {
        d += "\n## 警告\n\n";
        for w in &warnings {
            d += &format!("- {w}\n");
        }
    }
    atomic_write(save.join("digest.md"), d.as_bytes())?;
    timing.mark("write");
    timing.total();
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("talkdnd-start-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        let save = p.join("Saves/2030-03-04-01");
        fs::create_dir_all(save.join("Sessions")).unwrap();
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::write(
            save.join("combat/campaign.yaml"),
            "name: X\nruleset: \"2024\"\n",
        )
        .unwrap();
        fs::write(
            save.join("Sessions/2030-03-04-session-00-setup.md"),
            "# log\n",
        )
        .unwrap();
        fs::write(p.join("AGENTS.md"), "x").unwrap();
        p
    }

    #[test]
    fn digest_lists_adventure_pdf_and_markdown() {
        let root = temp_root("adv");
        let save = root.join("Saves/2030-03-04-01");
        fs::write(
            save.join("chargen.json"),
            serde_json::json!({ "adventure": "Curse of Strahd" }).to_string(),
        )
        .unwrap();
        // Old layout (pdf2md 0.5 `.md/<Book>/` with a page-less reader):
        // `start` tidies it into `index/` and drops the redundant reader.
        let adv = root.join("Adventures/curse-of-strahd");
        fs::create_dir_all(adv.join(".md/Curse of Strahd")).unwrap();
        fs::write(adv.join("Curse of Strahd.pdf"), "pdf").unwrap();
        fs::write(adv.join(".md/Curse of Strahd/index.md"), "# index").unwrap();
        fs::write(adv.join(".md/Curse of Strahd/Curse of Strahd.md"), "# book").unwrap();
        fs::write(
            adv.join(".md/Curse of Strahd/Curse of Strahd.pages.md"),
            "# book\n\n<a id=\"p1\"></a>\n<!-- PDF p.1 -->\ntext\n",
        )
        .unwrap();
        for f in [
            "fixes/p0001.md",
            "review/p0002.md",
            "pages/p0001-0010.md",
            "proofread-report.md",
        ] {
            let p = adv.join(".md/Curse of Strahd").join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "x").unwrap();
        }
        let d = digest(&root, &save, &Options { pdf2md: false }).unwrap();
        assert!(d.contains("冒险书：`Adventures/curse-of-strahd/`"), "{d}");
        assert!(
            d.contains("PDF：`Adventures/curse-of-strahd/Curse of Strahd.pdf`"),
            "{d}"
        );
        assert!(
            d.contains("书库整理：已移动 `Adventures/curse-of-strahd/.md/Curse of Strahd/` → `Adventures/curse-of-strahd/index/`"),
            "{d}"
        );
        assert!(
            d.contains("Markdown：`Adventures/curse-of-strahd/index/index.md`"),
            "{d}"
        );
        assert!(
            d.contains("Markdown：`Adventures/curse-of-strahd/index/Curse of Strahd.pages.md`"),
            "{d}"
        );
        assert!(!adv.join("index/Curse of Strahd.md").exists());
        assert!(!adv.join(".md").exists());
        for bad in ["fixes/", "review/", "pages/", "proofread-report"] {
            assert!(!d.contains(bad), "{bad} listed:\n{d}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn digest_says_markdown_not_converted_yet() {
        let root = temp_root("noadv");
        let save = root.join("Saves/2030-03-04-01");
        fs::write(
            save.join("chargen.json"),
            serde_json::json!({ "adventure": "Storm King's Thunder" }).to_string(),
        )
        .unwrap();
        let adv = root.join("Adventures/storm-kings-thunder");
        fs::create_dir_all(&adv).unwrap();
        fs::write(adv.join("Storm King's Thunder.pdf"), "pdf").unwrap();
        let d = digest(&root, &save, &Options { pdf2md: false }).unwrap();
        assert!(
            d.contains("冒险书：`Adventures/storm-kings-thunder/`"),
            "{d}"
        );
        assert!(d.contains("Markdown 还没转换"), "{d}");
        assert!(
            d.contains("PDF：`Adventures/storm-kings-thunder/Storm King's Thunder.pdf`"),
            "{d}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn cache_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("talkdnd-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let save = root.join("Saves/2030-03-04-01");
        fs::create_dir_all(save.join("combat")).unwrap();
        fs::create_dir_all(save.join("Characters")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("Rules/combat/common")).unwrap();
        fs::write(
            save.join("combat/campaign.yaml"),
            "name: X\nruleset: \"2024\"\nadventures: []\n",
        )
        .unwrap();
        fs::write(root.join("bin/combat-sim"), "stub").unwrap();
        fs::write(root.join("Rules/combat/common/conditions.yaml"), "x: 1\n").unwrap();
        fs::write(save.join("Characters/ann.md"), "# Ann\n").unwrap();
        root
    }

    #[test]
    fn content_key_changes_with_every_input() {
        let root = cache_root("key");
        let save = root.join("Saves/2030-03-04-01");
        let bin = root.join("bin/combat-sim");
        let manifest = save.join("combat/campaign.yaml");
        let first = content_key(&root, &save, &bin, &manifest);
        assert_eq!(first, content_key(&root, &save, &bin, &manifest));

        fs::write(save.join("Characters/ann.md"), "# Ann\nmore text\n").unwrap();
        let second = content_key(&root, &save, &bin, &manifest);
        assert_ne!(first, second);

        fs::write(
            root.join("Rules/combat/common/conditions.yaml"),
            "x: 2\nchanged\n",
        )
        .unwrap();
        let third = content_key(&root, &save, &bin, &manifest);
        assert_ne!(second, third);

        fs::write(&bin, "stub-v2").unwrap();
        assert_ne!(third, content_key(&root, &save, &bin, &manifest));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn content_check_is_cached_until_an_input_changes() {
        use std::os::unix::fs::PermissionsExt;

        let root = cache_root("check");
        let save = root.join("Saves/2030-03-04-01");
        let counter = root.join("bin/count.txt");
        let script = format!(
            "#!/bin/sh\necho run >> \"{}\"\necho \"content check: 0 error(s), 0 warning(s)\"\n",
            counter.display()
        );
        let bin = root.join("bin/combat-sim");
        fs::write(&bin, script).unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        let (ok, summary) = content_check(&root, &save);
        assert!(ok, "{summary}");
        assert!(summary.contains("0 error(s)"), "{summary}");
        assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);
        assert!(save.join(".cache/content-check.json").is_file());

        // Second call: unchanged inputs -> cache hit, no process spawn.
        let (ok, summary) = content_check(&root, &save);
        assert!(ok);
        assert!(summary.contains("0 error(s)"));
        assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);

        // A changed content file invalidates the key.
        fs::write(
            root.join("Rules/combat/common/conditions.yaml"),
            "x: 2\nchanged\n",
        )
        .unwrap();
        let _ = content_check(&root, &save);
        assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 2);
        let _ = fs::remove_dir_all(&root);
    }
}
