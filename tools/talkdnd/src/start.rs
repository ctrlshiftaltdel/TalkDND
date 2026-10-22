//! `talkdnd start`: one command to get a DM ready, plus a compact digest.

use crate::chargen;
use crate::save;
use anyhow::Result;
use combat_sim::fsutil::atomic_write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    match Command::new(&bin)
        .arg("--campaign")
        .arg(&manifest)
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

/// Starts pdf2md in the background when uv is available. Never waits.
pub fn kick_pdf2md(root: &Path) -> String {
    if std::env::var_os("TALKDND_NO_PDF2MD").is_some() {
        return "skipped (TALKDND_NO_PDF2MD)".into();
    }
    if !root.join("tools/pdf2md/pyproject.toml").is_file() {
        return "skipped (tools/pdf2md missing)".into();
    }
    let uv_ok = Command::new("uv")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !uv_ok {
        return "skipped (uv not installed; see docs/pdf2md.md)".into();
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
            v.push(Member {
                id: get("id").unwrap_or_default(),
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
    let (reader, pages) = crate::classdata::rulebooks(&root.join("Rules").join(rs));
    if reader.is_empty() {
        return format!("- 规则来源：仅 SRD（Rules/{rs}/ 下没有完整规则书 Markdown；见 docs/chargen.md「本地规则书数据」）\n");
    }
    let mut s = String::new();
    if crate::classdata::import_stale(root, rs) {
        match crate::classdata::run_import(root, rs) {
            Ok((_, subs, bgs)) => s += &format!("- 本地规则书数据：已自动导入（{subs} 个子职、{bgs} 个背景，Rules/chargen/local/，不提交）\n"),
            Err(e) => s += &format!("- 本地规则书数据：自动导入失败（{e}）；手动运行 `talkdnd data import --ruleset {rs}`\n"),
        }
    }
    let rel = |p: &std::path::PathBuf| {
        p.strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .replace('\\', "/")
    };
    s += &format!(
        "- 规则来源：**完整规则书优先**（SRD 只在书里找不到时兜底）。阅读版：{}{}\n",
        reader.iter().map(rel).collect::<Vec<_>>().join("；"),
        if pages.is_empty() {
            String::new()
        } else {
            format!(
                "；分页版（查页码）：{}",
                pages.iter().map(rel).collect::<Vec<_>>().join("；")
            )
        }
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

/// The digest's adventure-book lines: the slug's folder, its PDFs, and its
/// converted Markdown index when it exists.
fn adventure_lines(root: &Path, save: &Path) -> String {
    let Some(slug) = adventure_slug(save) else {
        return "- 冒险书：还没设置（`talkdnd chargen set adventure \"<书名>\"`）\n".into();
    };
    let dir = root.join("Adventures").join(&slug);
    let mut out = format!("- 冒险书：`Adventures/{slug}/`\n");
    let mut pdfs: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            let is_pdf = p
                .extension()
                .map(|x| x.to_string_lossy().eq_ignore_ascii_case("pdf"))
                .unwrap_or(false);
            if is_pdf {
                pdfs.push(p);
            }
        }
    }
    pdfs.sort();
    let mds = book_mds(&dir.join(".md"));
    for p in &mds {
        out += &format!("  - Markdown：`{}`\n", rel_root(root, p));
    }
    for p in &pdfs {
        out += &format!("  - PDF：`{}`\n", rel_root(root, p));
    }
    if mds.is_empty() {
        if pdfs.is_empty() {
            out += "  - 这里还没有 PDF；把冒险书 PDF 放进这个目录\n";
        } else {
            out += "  - Markdown 还没转换：先查上面的 PDF（转换后索引在 `.md/<书名>/index.md`）\n";
        }
    }
    out
}

/// Each converted book's main Markdown (`.md/<book>/<book>.md`) and its
/// `index.md` (headings, maps, stat blocks). Pages, fixes and review files
/// are working files and stay out of the digest.
fn book_mds(md_root: &Path) -> Vec<PathBuf> {
    let mut books: Vec<PathBuf> = fs::read_dir(md_root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    books.sort();
    let mut out = Vec::new();
    for b in books {
        let name = b
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for f in [b.join(format!("{name}.md")), b.join("index.md")] {
            if f.is_file() {
                out.push(f);
            }
        }
    }
    out
}

pub fn digest(root: &Path, save: &Path, opts: &Options) -> Result<String> {
    let slot = save
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let rs = save::ruleset(save);
    let (ok, check) = content_check(root, save);
    let pdf = if opts.pdf2md {
        kick_pdf2md(root)
    } else {
        "skipped (--no-pdf2md)".into()
    };
    let log = save::latest_log(save);
    let mut warnings = Vec::new();
    let mut d = String::new();
    d += &format!("# 接档摘要：{slot}\n\n");
    d += &format!(
        "- 规则：**{rs}**（只用 `Rules/{rs}/` 与 `Rules/rulings.md`、`Rules/rulings-{rs}.md`）\n"
    );
    let name = campaign_name(save);
    if !name.is_empty() {
        d += &format!("- 战役：{name}\n");
    }
    d += &format!("- content check：{check}\n- pdf2md：{pdf}\n");
    d += &rules_source_line(root, &rs);
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
    let ms = members(save);
    for m in &ms {
        let mark = if party.contains(&m.id) {
            ""
        } else {
            "（不在 party）"
        };
        d += &format!(
            "- {} `{}`：maxHP {}，AC {} — `{}`{mark}\n",
            m.name, m.id, m.hp, m.ac, m.file
        );
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
    d += "\n## 开场前再读（按需）\n\n";
    d += "- `dm-secrets.md`（只给 DM）、`canon.md`、`NPCs/_party.md`（同伴声音）\n";
    d += &adventure_lines(root, save);
    if save.join("house-rules.md").is_file() {
        d += "- `house-rules.md`：本存档开局前商定的房规（局中不改）\n";
    }
    let recent = crate::reveal::recent(save, 8);
    d += "\n## 已揭示（reveals.yaml 最近几条；提到专名或剧情事实前先 `talkdnd reveal check`）\n\n";
    if recent.is_empty() {
        d += "- （还没有记录：角色得知新事实时 `talkdnd reveal add`）\n";
    }
    for r in recent {
        d += &format!("- {}\n", truncate(&r, 200));
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
    if !warnings.is_empty() {
        d += "\n## 警告\n\n";
        for w in &warnings {
            d += &format!("- {w}\n");
        }
    }
    atomic_write(save.join("digest.md"), d.as_bytes())?;
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
        let adv = root.join("Adventures/curse-of-strahd");
        fs::create_dir_all(adv.join(".md/Curse of Strahd")).unwrap();
        fs::write(adv.join("Curse of Strahd.pdf"), "pdf").unwrap();
        fs::write(adv.join(".md/Curse of Strahd/index.md"), "# index").unwrap();
        fs::write(adv.join(".md/Curse of Strahd/Curse of Strahd.md"), "# book").unwrap();
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
            d.contains("Markdown：`Adventures/curse-of-strahd/.md/Curse of Strahd/index.md`"),
            "{d}"
        );
        assert!(d.contains("Curse of Strahd/Curse of Strahd.md`"), "{d}");
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
}
