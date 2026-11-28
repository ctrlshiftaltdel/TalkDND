//! Explicit advancement commit and recovery entries (ADV-P1-10).
//!
//! [`submit`] wires one player-explicit selection to the D protocol: it
//! re-runs the read-only chain (preview → selection → derivation) on the
//! live save, requires the identity binding to be strictly matched, requires
//! the `--preview-id` attestation produced by the read-only `--derive`
//! chain, and only then hands the reviewed candidate bytes to
//! [`advancement_commit::run_to_complete`]. The D protocol takes the SAVE
//! lock itself, re-checks the encounter gate and the protected-file binding
//! under the lock, and publishes registered targets exactly once; this
//! module never writes a target on its own.
//!
//! [`recover`] wires the explicit recovery: without a confirmed choice it
//! only inspects the leftover operation; with `--choice rollback|continue
//! --confirm` the library protects the current tree, then applies the human
//! decision file by file. Nothing is ever auto-replayed.
//!
//! Fail-closed boundaries kept from the approved decisions: writes are
//! refused outside the admitted local Linux ext4 environment (#344
//! admission), on saves without the protocol control files or below schema
//! 11, on characters without a strictly matched binding, and on legacy
//! Markdown-only cards (structure the card first; the submit applies the
//! validated derivation to a structured carrier). `progress::level_up`
//! keeps its existing behaviour and is untouched.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde_norway::Value;

use crate::advancement_commit::{self, Request};
use crate::advancement_derivation::{self, DeriveReport};
use crate::advancement_inner_lock::InnerLockOptions;
use crate::advancement_lock;
use crate::advancement_preview;
use crate::advancement_recovery::{self, RecoveryChoice, RecoveryRequest};
use crate::advancement_selection::{self, SelectionArgs};
use crate::charsheet;
use crate::classdata;
use crate::identity_diag::{self, Status};
use crate::migrate;
use crate::progress::{Progress, XpEntry, XP_TABLE};

/// D-protocol targets beyond the registered base set: every party
/// character's schema-11 sidecar plus `save-meta.yaml` — the same set the
/// identity migration registers, so the binding digest covers them and the
/// publish/recovery file-by-file comparison never skips one.
pub(crate) fn registered_extra_targets(save: &Path) -> Result<Vec<String>> {
    let diagnostic = identity_diag::diagnose(save);
    ensure!(
        diagnostic.status < Status::Ambiguous,
        "身份发现不完整或相互冲突（{:?}），无法确定受保护目标集合",
        diagnostic.status
    );
    let mut out: Vec<String> = diagnostic
        .characters
        .iter()
        .filter(|character| character.markdown_path().starts_with("Characters"))
        .map(|character| format!("advancement/{}.yaml", character.id()))
        .collect();
    out.push("save-meta.yaml".to_owned());
    Ok(out)
}

/// Everything both entries refuse on before any directory or control file is
/// touched: protocol files, schema 11, and the #344 admission observation.
fn preflight(root: &Path, save: &Path) -> Result<()> {
    ensure!(
        advancement_lock::lock_path(save).is_file() && advancement_lock::state_path(save).is_file(),
        "存档没有成长协议控制文件；请先完成 schema 10→11 迁移（`talkdnd migrate --dry-run` 核对前像，再带 --confirm-binding 迁移）；本次未写入任何文件"
    );
    let schema = migrate::detect(save)?;
    ensure!(
        schema == migrate::CURRENT_SCHEMA,
        "成长提交需要 schema {} 存档（当前 schema {schema}）；请先完成身份迁移；本次未写入任何文件",
        migrate::CURRENT_SCHEMA
    );
    let (admitted, reason) = migrate::admission_verdict(save, root);
    ensure!(
        admitted,
        "写入前复查 #344 准入失败（{reason}）；未准入环境拒绝成长写入；本次未写入任何文件"
    );
    Ok(())
}

fn parse_preview_id(raw: &str) -> Result<String> {
    let text = raw.trim();
    let bare = text.strip_prefix("sha256:").unwrap_or(text);
    ensure!(
        bare.len() == 64
            && bare
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "--preview-id 必须是只读 --derive 报告输出的 64 位十六进制提交绑定摘要"
    );
    Ok(format!("sha256:{bare}"))
}

/// One explicit advancement submission (ADV-P1-10).
pub(crate) struct SubmitArgs<'a> {
    pub(crate) who: &'a str,
    pub(crate) opportunity: &'a str,
    pub(crate) feat: Option<&'a str>,
    pub(crate) abilities: Option<&'a str>,
    pub(crate) declared_source: Option<&'a str>,
    pub(crate) dm_confirm: Option<&'a str>,
    pub(crate) preview_id: &'a str,
    pub(crate) milestone: Option<&'a str>,
}

pub(crate) fn submit(root: &Path, save: &Path, args: SubmitArgs<'_>) -> Result<()> {
    if let Some(reason) = args.milestone {
        ensure!(
            !reason.trim().is_empty(),
            "--milestone 需要 --reason \"<剧情节点>\"；本次未写入任何文件"
        );
    }
    let preview_id = parse_preview_id(args.preview_id)?;

    // ---- Phase 0: preflight. Nothing has been read or written yet. ----
    preflight(root, save)?;
    // Fail fast on the encounter gate before any evidence directory is
    // created; the D protocol re-checks it with the SAVE lock held.
    combat_sim::encounter_lifecycle::require_between_encounters(save)
        .context("当前只允许预览：遭遇未明确结束或归属无法确认；未写入任何文件")?;

    // ---- Phase 1: recompute the read-only chain on the live save. ----
    let inspection = advancement_preview::inspect(root, save, args.who)?;
    ensure!(
        inspection.preview.identity == "matched",
        "无绑定：角色的身份绑定不是严格匹配（当前 {}）；请先完成身份迁移或人工核对成长记录；本次未写入任何文件",
        inspection.preview.identity
    );
    let sheet = inspection
        .facts
        .sheet
        .as_ref()
        .context("旧式角色卡（无结构化 sheets/<id>.yaml）暂不支持成长提交：请先由 DM 完成结构化迁移；本次未写入任何文件")?;
    let character_id = sheet.id.clone();
    check_xp_or_milestone(
        save,
        &character_id,
        inspection.preview.current_level,
        args.milestone,
    )?;

    let selection_args = SelectionArgs {
        opportunity: args.opportunity,
        feat: args.feat,
        abilities: args.abilities,
        declared_source: args.declared_source,
        dm_confirm: args.dm_confirm,
    };
    let outcome = advancement_selection::evaluate(
        root,
        &inspection.preview,
        &inspection.facts,
        selection_args,
    )?;
    if outcome.decision == "rejected" {
        let detail = match &outcome.rejection {
            Some(rejection) => format!(
                "不满足：{}；未知：{}",
                joined_or_none(&rejection.unmet_ids),
                joined_or_none(&rejection.unknown_ids)
            ),
            None => "条件校验拒绝".to_owned(),
        };
        bail!("提交拒绝：选择校验未通过（{detail}）；请重新预览并修正选择；未写入任何文件");
    }
    let derived =
        advancement_derivation::derive(root, &inspection.preview, &inspection.facts, &outcome)?;

    // ---- Phase 2: binding attestation. The digest covers the reviewed
    // report (rules, authorization, conditions, derivation, unknowns) plus
    // the D protocol's protected-file binding at this instant. ----
    let extra_targets = registered_extra_targets(save)?;
    let d_binding = advancement_commit::binding_preview_id(save, &extra_targets)?;
    let report = DeriveReport {
        preview: inspection.preview,
        selection: outcome,
        derived: Some(derived),
    };
    let digest = advancement_derivation::submission_digest(&report, &d_binding)?;
    ensure!(
        digest == preview_id,
        "绑定变化，原因未知：--preview-id 与当前绑定摘要不一致（预览过期、选择参数不一致或存档/规则数据已变化）。请重新运行 `talkdnd advancement identity-preview --derive …` 并用新的 --preview-id 提交；未写入任何文件"
    );

    // ---- Phase 3: build the reviewed candidate bytes in memory. ----
    let overlays = build_overlays(save, args.who, &report, args.milestone)?;

    // ---- Phase 4: hand the candidate to the D protocol. SAVE is taken by
    // the protocol itself; the wrapper never holds it (see `run_game`). ----
    let slot = save
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "save".to_owned());
    let backups_root = root.join("Saves").join(migrate::BACKUP_DIR);
    let evidence_root = backups_root.join(".operations");
    let evidence_dir = evidence_root.join(&slot);
    fs::create_dir_all(&evidence_dir).with_context(|| {
        format!(
            "创建操作证据目录 {} 失败；未写入任何目标",
            evidence_dir.display()
        )
    })?;
    let operation_id = format!(
        "adv-{}-{}",
        character_id,
        chrono::Local::now().format("%Y%m%d-%H%M%S-%6f")
    );
    let request = Request {
        root,
        save_dir: save,
        operation_id: &operation_id,
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        backup_parent: &backups_root,
        candidate_parent: &backups_root,
        overlays,
        fault: None,
        extra_targets,
        preview_id: Some(d_binding),
        instance_tag: "",
        hooks: Default::default(),
        inner_lock: InnerLockOptions::default(),
        // The preflight admission observation established the ext4
        // environment; the publish must preserve and re-verify ACL/xattr.
        ext4_metadata: true,
    };
    match advancement_commit::run_to_complete(&request) {
        Ok(done) => print_success(&report, &done.published_paths, &operation_id),
        Err(failed) => {
            let recovery = &failed.report.recovery;
            bail!(
                "成长提交停止：{failed}\n\
                 已写阶段：{:?}；已发布目标：{}；未决 intent：{}。\n\
                 备份 {}（已验证：{}）；候选 {}（已验证：{}）；操作前快照 {}；操作证据目录 {}。\n\
                 不自动续跑或重放。人工恢复：`talkdnd advancement recover`（先只读检查，再由人显式选择回滚到前像或继续到目标）。",
                recovery.interrupt_stage,
                joined_or_none(&failed.report.published_paths),
                joined_or_none(&failed.report.pending_paths),
                recovery
                    .backup_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "（无）".into()),
                recovery.backup_verified,
                recovery
                    .candidate_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "（无）".into()),
                recovery.candidate_verified,
                recovery
                    .prestate
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "（无）".into()),
                evidence_dir.display(),
            )
        }
    }
    Ok(())
}

fn joined_or_none(paths: &[String]) -> String {
    if paths.is_empty() {
        "（无）".to_owned()
    } else {
        paths.join("、")
    }
}

/// The XP eligibility rule of the existing level-up path, re-checked at
/// submit time: explicit XP at or above the current level's threshold, or a
/// milestone with a fresh reason (the old path raises XP to the floor).
fn check_xp_or_milestone(
    save: &Path,
    character_id: &str,
    current_level: i32,
    milestone: Option<&str>,
) -> Result<()> {
    let progress = Progress::load(save)?;
    let xp = progress.xp.get(character_id).copied().unwrap_or(0);
    let threshold = XP_TABLE
        .get(current_level as usize)
        .copied()
        .context("等级越界，无法核对 XP 门槛")?;
    match milestone {
        None => {
            ensure!(
                xp >= threshold,
                "XP 不足：当前 {xp}，{current_level} 级升级需要 {threshold}（重大剧情节点可用 --milestone --reason \"…\"）；本次未写入任何文件"
            );
        }
        Some(reason) => {
            let normalized = norm_reason(reason);
            if let Some(entry) = progress
                .xp_log
                .iter()
                .find(|entry| entry.level.is_some() && norm_reason(&entry.reason) == normalized)
            {
                bail!(
                    "里程碑 \"{}\" 已在 {} 用于一次升级；请换一个剧情节点或改用 XP 升级；本次未写入任何文件",
                    entry.reason,
                    entry.date
                );
            }
        }
    }
    Ok(())
}

fn norm_reason(reason: &str) -> String {
    reason
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The candidate bytes the reviewed derivation produced, in memory only.
/// Every `auto` entry must still match its observed `before` value or the
/// build fails closed; `keep` and `dm_review` entries never touch bytes.
fn build_overlays(
    save: &Path,
    who: &str,
    report: &DeriveReport,
    milestone_reason: Option<&str>,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let derived = report
        .derived
        .as_ref()
        .context("内部错误：候选缺失，拒绝构建覆盖")?;
    let edition = report.preview.ruleset.edition();
    let book = classdata::Book::srd(edition);
    let (mut cf, migrated) = charsheet::load(save, who, &book)?;
    ensure!(
        !migrated,
        "旧式角色卡（无结构化 sheets/<id>.yaml）暂不支持成长提交：请先由 DM 完成结构化迁移；未写入任何文件"
    );

    // Apply exactly the reviewed `auto` entries, verifying each observed
    // `before` value on the live merged block.
    for entry in &derived.entries {
        if entry.disposition != "auto" {
            continue;
        }
        let target = entry
            .target
            .as_deref()
            .context("auto 条目缺少目标路径，拒绝应用")?;
        let path = target
            .strip_prefix("combat.")
            .context("auto 条目目标不在 combat 块内，拒绝应用")?;
        let after_json = entry
            .after
            .clone()
            .context("auto 条目缺少目标值，拒绝应用")?;
        let after = json_to_yaml(&after_json)?;
        let before = entry.before.as_ref().map(json_to_yaml).transpose()?;
        apply_combat_path(&mut cf.combat, path, before.as_ref(), &after)?;
    }

    // The recorded choice: a selected feat id joins the sheet's feat list,
    // where later duplicate checks read it.
    if report.selection.candidate.kind == "feat" {
        let feat_id = report.selection.candidate.id.clone();
        ensure!(
            !cf.feats.contains(&feat_id),
            "专长 {feat_id} 已在角色卡上；重复选择被拒绝；未写入任何文件"
        );
        cf.feats.push(feat_id);
    }

    // The level record the HP provenance chain reads on the next
    // advancement. The full HP form is written only when the derivation
    // proved and applied the HP gain; otherwise the reduced line documents
    // the level step and future HP derivations fail closed to DM review.
    let new_level = report.preview.target_level;
    let prof = crate::progress::prof_for_level(new_level);
    let stamp = chrono::Local::now().format("%Y-%m-%d");
    let hp_entry = derived
        .entries
        .iter()
        .find(|entry| entry.id == "combat.hp" && entry.disposition == "auto");
    let note = match hp_entry {
        Some(entry) => {
            let before = entry
                .before
                .as_ref()
                .and_then(serde_json::Value::as_i64)
                .context("HP 条目缺少前值")?;
            let after = entry
                .after
                .as_ref()
                .and_then(serde_json::Value::as_i64)
                .context("HP 条目缺少目标值")?;
            let die = cf
                .combat
                .get("resources")
                .and_then(|resources| resources.get("hit-dice"))
                .and_then(|hit_dice| hit_dice.get("die"))
                .and_then(Value::as_i64)
                .context("hit-dice 的 die 字段缺失，无法写升级记录")?;
            let average = die / 2 + 1;
            let con_mod = cf
                .combat
                .get("abilities")
                .and_then(|abilities| abilities.get("con"))
                .and_then(Value::as_i64)
                .map(|con| (con - 10).div_euclid(2))
                .context("体质分数缺失，无法写升级记录")?;
            format!(
                "- {stamp}：升到 {new_level} 级；HP +{after}（d{die} average {average}，体质 {con_mod:+}）→ {after_total}；熟练 +{prof}",
                after = after - before,
                after_total = after,
            )
        }
        None => {
            format!("- {stamp}：升到 {new_level} 级（HP 未自动更新，由 DM 核对）；熟练 +{prof}")
        }
    };
    cf.tail_md = append_level_record(&cf.tail_md, &note);

    let mut overlays = BTreeMap::new();
    // sheets/<id>.yaml — the same byte format `charsheet::store` writes.
    let mut stored = cf.clone();
    stored.version = 1;
    let sheet_rel = format!("sheets/{}.yaml", cf.id);
    overlays.insert(
        sheet_rel,
        format!(
            "# TalkDND character data (source of truth). Change it with `talkdnd char {} …`; never by hand.\n{}",
            cf.id,
            serde_norway::to_string(&stored)?
        )
        .into_bytes(),
    );
    // Characters/<name>.md — the same render `charsheet::commit` writes.
    let ps = charsheet::party(save, &cf)?;
    let ledger = crate::identify::load(save).ok();
    let markdown = charsheet::render_with(&cf, ps.members.get(&cf.id), ledger.as_ref());
    let markdown_rel = cf.file.clone();
    ensure!(
        !markdown_rel.is_empty(),
        "角色卡缺少 Markdown 路径，拒绝构建覆盖"
    );
    overlays.insert(markdown_rel, markdown.into_bytes());
    // combat/party-state.yaml — only when the save already tracks one, so a
    // published target never appears from nowhere.
    if save.join("combat").join("party-state.yaml").is_file() {
        let mut current = ps.clone();
        current.version = if current.completed_casts.is_empty() {
            1
        } else {
            2
        };
        overlays.insert(
            "combat/party-state.yaml".to_owned(),
            format!(
                "# TalkDND party state (written by combat-sim; edit with care)\n{}",
                serde_norway::to_string(&current)?
            )
            .into_bytes(),
        );
    }
    // progress.yaml — milestone only (XP raised to the new level's floor and
    // logged), mirroring the existing milestone path.
    if let Some(reason) = milestone_reason {
        ensure!(
            save.join("progress.yaml").is_file(),
            "里程碑升级需要已有 progress.yaml；未写入任何文件"
        );
        let mut progress = Progress::load(save)?;
        let floor = XP_TABLE[(new_level - 1) as usize];
        let xp = progress.xp.get(&cf.id).copied().unwrap_or(0);
        let after = xp.max(floor);
        progress.xp.insert(cf.id.clone(), after);
        progress.xp_log.push(XpEntry {
            date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            category: "milestone-major".into(),
            reason: reason.trim().to_string(),
            each: after - xp,
            to: vec![cf.id.clone()],
            difficulty: None,
            level: Some(new_level),
            capped: BTreeMap::new(),
        });
        overlays.insert(
            "progress.yaml".to_owned(),
            format!(
                "# TalkDND progress ledger: XP per character (hybrid advancement, see xp_log), coins (as cp) and items per character and the shared `party` pool.\n# Edit with `talkdnd xp` / `talkdnd loot`; hand edits are flagged by `talkdnd check` (accept with `talkdnd integrity accept --reason`).\n{}",
                serde_norway::to_string(&progress)?
            )
            .into_bytes(),
        );
    }
    Ok(overlays)
}

/// Same shape as `progress::append_level_record`.
fn append_level_record(tail: &str, note: &str) -> String {
    let mut text = tail.trim_end().to_owned();
    if text.is_empty() {
        return format!("## 升级记录\n\n{note}\n");
    }
    if text.contains("## 升级记录") {
        text.push('\n');
        text.push_str(note);
        text.push('\n');
    } else {
        text.push_str("\n\n## 升级记录\n\n");
        text.push_str(note);
        text.push('\n');
    }
    text
}

/// The derivation reports values as JSON; the combat block is YAML data.
fn json_to_yaml(value: &serde_json::Value) -> Result<Value> {
    serde_norway::from_str(&serde_json::to_string(value)?)
        .with_context(|| format!("无法把派生值 {value} 转换为角色卡数据"))
}

fn show_yaml(value: &Value) -> String {
    serde_norway::to_string(value).unwrap_or_else(|_| "（不可序列化）".to_owned())
}

/// Set one combat-block path (for example `saves.wis` or
/// `attacks.longsword.to_hit`) to `after`, refusing unless the current value
/// equals `before`. Object keys step mappings; a segment whose next step
/// names a mapping-array entry steps into the element with that `id`.
fn apply_combat_path(
    block: &mut Value,
    path: &str,
    before: Option<&Value>,
    after: &Value,
) -> Result<()> {
    let segments: Vec<&str> = path.split('.').collect();
    let last = segments.last().copied().context("派生目标路径为空")?;
    let mut current = block;
    for (index, segment) in segments.iter().enumerate() {
        if segment == &last {
            let observed = current.get(segment).cloned();
            match (before, observed) {
                (Some(expected), Some(actual)) => {
                    ensure!(
                        expected == &actual,
                        "派生目标 {path} 的当前值 {} 与预览观察值 {} 不一致；拒绝应用",
                        show_yaml(&actual),
                        show_yaml(expected),
                    );
                }
                (None, Some(actual)) => {
                    bail!(
                        "派生目标 {path} 当前值 {} 没有可核对的前值；拒绝应用",
                        show_yaml(&actual)
                    );
                }
                (Some(expected), None) => {
                    bail!(
                        "派生目标 {path} 缺失，但预览观察到 {}；拒绝应用",
                        show_yaml(expected)
                    );
                }
                (None, None) => {}
            }
            let mapping = current
                .as_mapping_mut()
                .context(format!("派生目标 {path} 的父节点不是映射，拒绝应用"))?;
            mapping.insert(Value::String((*segment).to_owned()), after.clone());
            return Ok(());
        }
        let next = current
            .get_mut(*segment)
            .context(format!("派生目标 {path} 在 {segment} 处缺失；拒绝应用"))?;
        if next.is_mapping() {
            current = next;
            continue;
        }
        if let Some(entries) = next.as_sequence_mut() {
            // A mapping array keyed by `id` (attacks): the following segment
            // names one entry; recurse into it with the remaining path.
            let owner = segments[index + 1];
            let entry = entries
                .iter_mut()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(owner))
                .context(format!(
                    "派生目标 {path} 在 {segment} 中找不到 id {owner}；拒绝应用"
                ))?;
            ensure!(
                index + 2 < segments.len(),
                "派生目标 {path} 以数组条目 id 结尾，不是可写路径；拒绝应用"
            );
            let remainder = segments[index + 2..].join(".");
            return apply_combat_path(entry, &remainder, before, after);
        }
        bail!("派生目标 {path} 在 {segment} 处类型不支持；拒绝应用");
    }
    unreachable!("the last segment always returns")
}

fn print_success(report: &DeriveReport, published: &[String], operation_id: &str) {
    println!(
        "成长提交完成（操作 {operation_id}）。已发布：{}",
        joined_or_none(published)
    );
    println!("前后差异与来源（仅可靠派生项；其余保持原值）：");
    if let Some(derived) = &report.derived {
        for entry in &derived.entries {
            if entry.disposition == "auto" {
                println!(
                    "  {}：{} → {}（{}；{}）",
                    entry.id,
                    entry
                        .before
                        .as_ref()
                        .map(serde_json::Value::to_string)
                        .unwrap_or_else(|| "（缺失）".into()),
                    entry
                        .after
                        .as_ref()
                        .map(serde_json::Value::to_string)
                        .unwrap_or_else(|| "（缺失）".into()),
                    entry.reason,
                    entry.source,
                );
            }
        }
        for item in &derived.dm_items {
            println!("  人工项：{item}");
        }
    }
    if report.selection.dm_confirmation.provided {
        if let Some(basis) = &report.selection.dm_confirmation.basis {
            println!("DM 确认依据：{basis}（未知条件未经程序验证）");
        }
    }
    if let Some(derived) = &report.derived {
        for entry in &derived.entries {
            if entry.disposition == "dm_review" {
                println!("  保持原值（DM 核对）：{} — {}", entry.id, entry.reason);
            }
        }
    }
    if report.selection.not_program_verified {
        println!("注意：部分条件仅经 DM 确认，未经程序验证。");
    }
    if report.selection.unrecognized_marking == Some("unverified_effects_manual") {
        println!("注意：所选专长不在已授权目录中，标记为未验证、效果手动；请 DM 落实效果。");
    }
}

// ---------------------------------------------------------------- recovery --

pub(crate) struct RecoverArgs<'a> {
    /// `rollback` / `continue`; absent or with `--confirm` unset → inspect.
    pub(crate) choice: Option<&'a str>,
    pub(crate) confirm: bool,
}

pub(crate) fn recover(root: &Path, save: &Path, args: RecoverArgs<'_>) -> Result<()> {
    let choice = match args.choice {
        None => None,
        Some("rollback") => Some(RecoveryChoice::RollbackToPreimage),
        Some("continue") => Some(RecoveryChoice::ContinueToTarget),
        Some(other) => bail!(
            "--choice 只接受 rollback（回滚到前像）或 continue（继续到目标）；得到 {other:?}；未写入任何文件"
        ),
    };
    let confirm = args.confirm && choice.is_some();
    // A confirmed recovery writes targets; it only runs in the admitted
    // environment. Inspection never writes and stays available everywhere.
    if confirm {
        preflight(root, save)?;
    }
    let slot = save
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "save".to_owned());
    let backups_root = root.join("Saves").join(migrate::BACKUP_DIR);
    let evidence_root = backups_root.join(".operations");
    let evidence_dir = evidence_root.join(&slot);
    let recovery_id = format!(
        "recover-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S-%6f")
    );
    let request = RecoveryRequest {
        root,
        save_dir: save,
        recovery_id: &recovery_id,
        evidence_root: &evidence_root,
        evidence_dir: &evidence_dir,
        choice,
        confirm_choice: confirm,
        extra_targets: registered_extra_targets(save)?,
        fault: None,
        hooks: Default::default(),
        inner_lock: InnerLockOptions::default(),
    };
    let report = if confirm {
        match advancement_recovery::recover(&request) {
            Ok(report) => report,
            Err(failed) => {
                let display = failed.to_string();
                let inner = failed.report;
                bail!(
                    "显式恢复停止：{display}\n\
                     原操作 {}；阶段 {:?}；已写目标：{}；证据 {}（快照 {}）。\n\
                     备份 {}；候选 {}。不自动续跑或重放；请人工核对后重新显式恢复。",
                    inner.original_operation_id.as_deref().unwrap_or("（未知）"),
                    inner.stage,
                    joined_or_none(&inner.written_paths),
                    inner
                        .work_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "（无）".into()),
                    inner
                        .snapshot_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "（无）".into()),
                    inner
                        .backup_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "（无）".into()),
                    inner
                        .candidate_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "（无）".into()),
                )
            }
        }
    } else {
        advancement_recovery::inspect_recovery(&request)
    };
    print_recovery(&report, confirm);
    if !confirm {
        println!(
            "本次为只读检查。要执行恢复，请再次运行并给出 --choice rollback|continue --confirm（先保护当前树，再由人显式选择）。"
        );
    }
    // An inspection reporting an in_doubt leftover, human-merge conflicts,
    // or a confirmed recovery that wrote nothing and still carries an
    // unresolved note is a failure; a completed recovery may carry an
    // informational note and is not one.
    let problem = if report.inspect_only {
        report.in_doubt || report.needs_human_merge
    } else {
        report.in_doubt
            || report.needs_human_merge
            || (report.error.is_some()
                && report.written_paths.is_empty()
                && report.already_done_paths.is_empty())
    };
    if problem {
        std::process::exit(1);
    }
    Ok(())
}

fn print_recovery(report: &advancement_recovery::RecoveryReport, confirmed: bool) {
    println!(
        "恢复{}：原操作 {}；协议状态 {:?}；阶段 {:?}。",
        if confirmed { "已执行" } else { "检查" },
        report
            .original_operation_id
            .as_deref()
            .unwrap_or("（未知）"),
        report.protocol_state,
        report.stage,
    );
    if report.inspect_only {
        println!("模式：只读检查（未建立新证据、未写任何目标）。");
    }
    if let Some(choice) = report.choice {
        println!("人工选择：{}。", choice.as_label());
    }
    println!(
        "工作记录 {}；受保护快照 {}。",
        report
            .work_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "（无）".into()),
        report
            .snapshot_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "（无）".into()),
    );
    if !report.written_paths.is_empty() {
        println!("本次写入：{}", joined_or_none(&report.written_paths));
    }
    if !report.already_done_paths.is_empty() {
        println!(
            "此前已应用（未重复写）：{}",
            joined_or_none(&report.already_done_paths)
        );
    }
    if !report.pending_paths.is_empty() {
        println!("未决：{}", joined_or_none(&report.pending_paths));
    }
    if !report.scope_limited_paths.is_empty() {
        println!(
            "范围外（未覆盖、未忽略）：{}",
            joined_or_none(&report.scope_limited_paths)
        );
    }
    if report.needs_human_merge {
        println!("需要人工合并：保留了两份证据，未删除后续日志。");
    }
    if report.in_doubt {
        println!("结论：无法判定（in_doubt），只允许人工处理；未自动续跑或重放。");
    }
    if let Some(error) = &report.error {
        println!("说明：{error}");
    }
}
