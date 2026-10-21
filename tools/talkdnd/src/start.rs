//! `talkdnd start`: one command to get a DM ready, plus a compact digest.

use crate::chargen;
use crate::save;
use anyhow::Result;
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

fn party_ids(save: &Path) -> Vec<String> {
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
    d += "- 冒险书：从存档点定位章节，查 `Adventures/.md/<书名>/index.md`，地图看 PDF\n";
    if !warnings.is_empty() {
        d += "\n## 警告\n\n";
        for w in &warnings {
            d += &format!("- {w}\n");
        }
    }
    fs::write(save.join("digest.md"), &d)?;
    Ok(d)
}
