//! Player-facing preview of one level, using the save's existing read-only inputs.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use combat_sim::content::loader::Ruleset;
use combat_sim::content::schema::CampaignManifest;
use combat_sim::content::{ContentPack, Distribution};
use serde::Serialize;
use serde_norway::Value;

use crate::advancement_validation::{self, ConditionCheck, ConditionStatus, ValidationDecision};
use crate::{advancement_identity, charsheet, classdata, data, identity_diag, library, migrate};

#[derive(Serialize)]
pub(crate) struct Preview {
    pub(crate) consistency: &'static str,
    pub(crate) write_capability: &'static str,
    /// `unavailable_in_read_only_preview` until the full chain (preview →
    /// selection → derivation) runs; that chain overwrites this field with
    /// the submit binding digest a later `advancement submit --preview-id`
    /// must present verbatim.
    pub(crate) submission_binding: String,
    pub(crate) identity: &'static str,
    pub(crate) encounter: &'static str,
    pub(crate) ruleset: Ruleset,
    pub(crate) class: String,
    pub(crate) class_source: &'static str,
    pub(crate) current_level: i32,
    pub(crate) target_level: i32,
    pub(crate) proficiency_bonus: i32,
    pub(crate) xp_threshold: i64,
    pub(crate) opportunity: Option<&'static str>,
    pub(crate) opportunity_source: &'static str,
    pub(crate) class_features: Vec<String>,
    pub(crate) abilities: Vec<AbilityPotential>,
    pub(crate) abilities_with_conflicting_sources: Vec<&'static str>,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) unknowns: Vec<&'static str>,
    pub(crate) dm_review: &'static str,
}

#[derive(Serialize)]
pub(crate) struct AbilityPotential {
    pub(crate) ability: &'static str,
    pub(crate) current_score: i32,
    pub(crate) current_modifier: i32,
    pub(crate) modifier_after_plus_one: Option<i32>,
    pub(crate) modifier_after_plus_two: Option<i32>,
}

#[derive(Clone, Serialize)]
pub(crate) struct Candidate {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) category: Option<String>,
    pub(crate) minimum_level: Option<i64>,
    pub(crate) eligibility: &'static str,
}

pub(crate) struct CharacterFacts {
    pub(crate) class: String,
    pub(crate) class_source: &'static str,
    pub(crate) structured_combat: Option<Value>,
    /// The structured sheet when one exists; legacy Markdown-only cards have
    /// none, so recorded feats and the subclass stay unknown to selection.
    pub(crate) sheet: Option<charsheet::CharFile>,
    /// The Markdown card's combat block (the block the preview itself reads).
    pub(crate) markdown_combat: Value,
    /// `## 升级记录` lines from the card: sheet `tail_md` for structured
    /// cards, the rendered Markdown section otherwise. Each program level-up
    /// appends one line, so they are the provenance record for accumulated
    /// values such as max HP.
    pub(crate) level_records: Vec<String>,
}

pub(crate) fn run(root: &Path, save: &Path, who: &str, json: bool) -> Result<()> {
    let inspection = inspect(root, save, who)?;
    let preview = inspection.preview;
    if json {
        println!("{}", serde_json::to_string(&preview)?);
    } else {
        print!("{}", serde_norway::to_string(&preview)?);
    }
    Ok(())
}

/// What one inspection produced: the player-facing preview plus the character
/// facts (including the structured sheet, when one exists) that read-only
/// selection validation reuses.
pub(crate) struct Inspection {
    pub(crate) preview: Preview,
    pub(crate) facts: CharacterFacts,
}

pub(crate) fn inspect(root: &Path, save: &Path, who: &str) -> Result<Inspection> {
    let saves = root.join("Saves");
    ensure!(
        save.parent() == Some(saves.as_path()),
        "成长预览需要游玩目录中的存档"
    );
    for path in [saves, save.to_path_buf(), save.join("combat")] {
        ensure!(
            fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir()),
            "存档或战斗目录不是可验证的普通目录"
        );
    }
    let manifest: CampaignManifest = serde_norway::from_str(&regular_text(
        &save.join("combat/campaign.yaml"),
        "战役规则来源不可读取",
    )?)
    .map_err(|_| anyhow::anyhow!("战役规则来源损坏"))?;
    let ruleset = Ruleset::parse(manifest.ruleset.as_deref().context("战役未指定规则集")?)
        .map_err(|_| anyhow::anyhow!("战役规则集无效，请核对 campaign.yaml"))?;

    ensure!(
        regular_file(&save.join(migrate::FILE)),
        "缺少可验证的存档 schema"
    );
    let schema = migrate::load(save)
        .map_err(|_| anyhow::anyhow!("无法读取当前存档 schema"))?
        .context("缺少存档 schema")?
        .schema;
    ensure!(matches!(schema, 9..=11), "成长预览支持 schema 9、10 和 11");

    let diagnostic = identity_diag::diagnose(save);
    ensure!(
        diagnostic.status < identity_diag::Status::Ambiguous,
        "角色发现不完整或资料相互冲突，请核对身份诊断"
    );
    let who = who.trim().to_lowercase();
    ensure!(!who.is_empty(), "请选择要预览的玩家角色");
    let character = diagnostic
        .characters
        .iter()
        .find(|c| {
            c.markdown_path().starts_with("Characters")
                && (c.id().to_lowercase() == who || c.name().to_lowercase() == who)
        })
        .context("未找到该玩家角色")?;
    let markdown = regular_text(&save.join(character.markdown_path()), "角色卡不可读取")?;
    let combat = crate::start::combat_block(&markdown).context("角色卡没有可信的 combat 数据")?;
    let current_level = level(&combat)?;
    ensure!(current_level < 20, "角色已达 20 级");
    let target_level = current_level + 1;
    let character_facts =
        character_facts(save, character, &combat, current_level, ruleset, &markdown)?;

    let edition = ruleset.edition();
    let book = classdata::Book::srd(edition);
    let definition = book
        .classes
        .get(&character_facts.class)
        .context("缺少该职业经核对的升级表")?;
    let class_features: Vec<String> = definition
        .features
        .iter()
        .filter(|feature| feature.0 == target_level)
        .map(|feature| feature.1.clone())
        .collect();
    // #331 behavior: a class whose book entry carries no explicit `asi`
    // list fails closed here; enabling the generator's standard pattern is
    // a separate direction decision, not part of this task.
    let has_asi = definition
        .asi
        .as_ref()
        .context("职业升级表缺少属性提升安排")?
        .contains(&target_level);
    let opportunity = if edition == "2014" && has_asi {
        Some("ability_score_improvement_or_optional_feat")
    } else if class_features.iter().any(|name| name == "Epic Boon") {
        Some("epic_boon_or_other_qualified_feat")
    } else if has_asi {
        Some("ability_score_improvement_feat_or_other_qualified_feat")
    } else {
        None
    };

    let mut unknowns = vec!["milestone_and_prior_selections", "source_consistency"];
    if edition == "2014" && has_asi {
        unknowns.push("optional_feat_rule_at_this_table");
    }
    if opportunity.is_some() && !ruleset.is_full() {
        unknowns.push("feat_catalog_limited_to_srd");
    }
    let (candidates, missing_catalog) =
        candidates(root, ruleset, target_level, opportunity.is_some())?;
    if opportunity.is_some() && missing_catalog {
        unknowns.push("feat_catalog_incomplete");
    }
    let (abilities, abilities_with_conflicting_sources) = if has_asi {
        let (values, conflicts) =
            ability_potential(&combat, character_facts.structured_combat.as_ref());
        if values.len() + conflicts.len() != data::ABILITIES.len() {
            unknowns.push("missing_ability_scores");
        }
        if !conflicts.is_empty() {
            unknowns.push("conflicting_ability_sources");
        }
        (values, conflicts)
    } else {
        (Vec::new(), Vec::new())
    };
    let world = save.join("world-time.yaml");
    let world_readable = match fs::symlink_metadata(world) {
        Ok(meta) => meta.file_type().is_file(),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    };
    let encounter = if world_readable
        && combat_sim::encounter_lifecycle::require_between_encounters(save).is_ok()
    {
        "clear_at_read"
    } else {
        "active_or_unknown"
    };
    Ok(Inspection {
        preview: Preview {
            consistency: "unverified",
            write_capability: "unverified",
            submission_binding: "unavailable_in_read_only_preview".to_owned(),
            identity: advancement_identity::binding_status(save, &diagnostic, character),
            encounter,
            ruleset,
            class: character_facts.class.clone(),
            class_source: character_facts.class_source,
            current_level,
            target_level,
            proficiency_bonus: crate::progress::prof_for_level(target_level),
            xp_threshold: crate::progress::XP_TABLE[current_level as usize],
            opportunity,
            opportunity_source: if edition == "2014" {
                "SRD 5.1 class progression"
            } else {
                "SRD 5.2.1 class progression"
            },
            class_features,
            abilities,
            abilities_with_conflicting_sources,
            candidates,
            unknowns,
            dm_review: "专长前置、能力值分配、HP 与其他衍生效果需由玩家和 DM 核对",
        },
        facts: character_facts,
    })
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

fn regular_text(path: &Path, reason: &'static str) -> Result<String> {
    ensure!(regular_file(path), "{reason}");
    fs::read_to_string(path).map_err(|_| anyhow::anyhow!(reason))
}

fn level(combat: &Value) -> Result<i32> {
    combat
        .get("level")
        .and_then(Value::as_i64)
        .filter(|level| (1..=20).contains(level))
        .map(|level| level as i32)
        .context("角色卡没有可信的 1–20 等级")
}

fn character_facts(
    save: &Path,
    character: &identity_diag::Character,
    combat: &Value,
    current_level: i32,
    ruleset: Ruleset,
    markdown: &str,
) -> Result<CharacterFacts> {
    let records = level_record_lines(markdown);
    let inferred = charsheet::infer_controller(character.markdown(), combat);
    let Some(path) = character.sheet_path() else {
        ensure!(inferred == charsheet::PLAYER, "未找到该玩家角色");
        let (class, origin_level) = combat
            .get("notes")
            .and_then(Value::as_str)
            .and_then(|notes| notes.split_once(". Generated by talkdnd"))
            .and_then(|(prefix, _)| prefix.rsplit_once(' '))
            .context("旧角色卡没有可核对的职业字段")?;
        ensure!(
            origin_level
                .parse::<i32>()
                .is_ok_and(|value| (1..=current_level).contains(&value)),
            "旧角色卡的职业来源与等级不一致"
        );
        return Ok(CharacterFacts {
            class: class.to_ascii_lowercase(),
            class_source: "generated_card",
            structured_combat: None,
            sheet: None,
            markdown_combat: combat.clone(),
            level_records: records,
        });
    };
    let text = regular_text(&save.join(path), "结构化角色卡不可读取")?;
    let sheet: charsheet::CharFile =
        serde_norway::from_str(&text).map_err(|_| anyhow::anyhow!("结构化角色卡无效"))?;
    ensure!(
        sheet.controller.as_deref().unwrap_or(inferred) == charsheet::PLAYER,
        "未找到该玩家角色"
    );
    ensure!(
        sheet.id == character.id() && sheet.name == character.name(),
        "结构化角色卡与已发现身份不一致"
    );
    let sheet_ruleset =
        Ruleset::parse(&sheet.ruleset).map_err(|_| anyhow::anyhow!("角色卡规则版次无效"))?;
    ensure!(
        sheet_ruleset.edition() == ruleset.edition(),
        "角色卡与战役规则版次不一致"
    );
    ensure!(
        level(&sheet.combat)? == current_level,
        "结构化角色卡与 Markdown 等级不一致"
    );
    let class = sheet.class().context("角色卡没有可信的职业字段")?;
    Ok(CharacterFacts {
        class: class.to_ascii_lowercase(),
        class_source: "structured_sheet",
        structured_combat: Some(sheet.combat.clone()),
        sheet: Some(sheet),
        markdown_combat: combat.clone(),
        level_records: records,
    })
}

/// The `## 升级记录` bullet lines of a card. Structured sheets render their
/// `tail_md` into the Markdown card, so one parse covers both carriers.
fn level_record_lines(markdown: &str) -> Vec<String> {
    let Some(index) = markdown.find("## 升级记录") else {
        return Vec::new();
    };
    markdown[index..]
        .lines()
        .skip(1)
        .skip_while(|line| line.trim().is_empty())
        .take_while(|line| line.trim_start().starts_with('-'))
        .map(str::to_owned)
        .collect()
}

pub(crate) fn ability_potential(
    markdown: &Value,
    structured: Option<&Value>,
) -> (Vec<AbilityPotential>, Vec<&'static str>) {
    let score_in = |combat: &Value, ability: &str| {
        combat
            .get("abilities")?
            .get(ability)?
            .as_i64()
            .filter(|score| (1..=30).contains(score))
            .map(|score| score as i32)
    };
    let mut values = Vec::new();
    let mut conflicts = Vec::new();
    for ability in data::ABILITIES {
        let score = score_in(markdown, ability);
        if structured.is_some_and(|sheet| score_in(sheet, ability) != score) {
            conflicts.push(ability);
            continue;
        }
        let Some(score) = score else { continue };
        let after = |delta| (score + delta <= 20).then_some(data::modifier(score + delta));
        values.push(AbilityPotential {
            ability,
            current_score: score,
            current_modifier: data::modifier(score),
            modifier_after_plus_one: after(1),
            modifier_after_plus_two: after(2),
        });
    }
    (values, conflicts)
}

pub(crate) fn candidates(
    root: &Path,
    ruleset: Ruleset,
    target_level: i32,
    show_choices: bool,
) -> Result<(Vec<Candidate>, bool)> {
    let edition = ruleset.edition();
    let full_packs = if ruleset.is_full() {
        let unlocked: Vec<_> = library::pack_status(root)
            .into_iter()
            .filter(|status| {
                status.edition == edition && status.state == library::PackState::Unlocked
            })
            .collect();
        ensure!(
            !unlocked.is_empty(),
            "此 FULL 规则集缺少同版已授权的内容包，请检查书库的解锁状态"
        );
        unlocked
    } else {
        Vec::new()
    };
    if !show_choices {
        return Ok((Vec::new(), false));
    }

    let srd = if edition == "2014" {
        "srd-5.1-feats"
    } else {
        "srd-5.2.1-feats"
    };
    let srd_path = root
        .join("Rules/combat")
        .join(edition)
        .join("srd")
        .join(format!("{srd}.pack.json"));
    let mut rows = BTreeMap::new();
    let missing_srd = match fs::symlink_metadata(&srd_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        _ => add_candidates(
            &mut rows,
            &srd_path,
            edition,
            Distribution::Srd,
            srd,
            target_level,
        )?,
    };
    for status in full_packs {
        let file = status.file.context("已解锁内容包没有来源")?;
        add_candidates(
            &mut rows,
            &root.join(file),
            edition,
            Distribution::Full,
            &status.id,
            target_level,
        )?;
    }
    let missing_catalog = missing_srd || rows.is_empty();
    Ok((rows.into_values().collect(), missing_catalog))
}

fn add_candidates(
    rows: &mut BTreeMap<String, Candidate>,
    path: &Path,
    edition: &str,
    distribution: Distribution,
    source: &str,
    target_level: i32,
) -> Result<bool> {
    ensure!(regular_file(path), "专长目录来源不是普通文件");
    let pack = library::pack_value(path)
        .and_then(|value| ContentPack::from_value(&value))
        .map_err(|_| anyhow::anyhow!("专长目录无法验证，请核对已安装内容包"))?;
    ensure!(
        pack.pack.id == source
            && pack.pack.edition == edition
            && pack.pack.distribution == distribution,
        "专长目录的规则版次或授权来源不一致"
    );
    let feats = pack.content.feats.unwrap_or_default();
    let empty = feats.is_empty();
    for feat in feats {
        let category = feat.data.as_ref().and_then(|data| data.get("type"));
        let minimum_level = feat
            .data
            .as_ref()
            .and_then(|data| data.pointer("/prerequisites/minimum_level"))
            .and_then(serde_json::Value::as_i64);
        let mut checks = vec![ConditionCheck {
            id: "qualification_and_history".into(),
            status: ConditionStatus::Unknown,
        }];
        if let Some(minimum) = minimum_level {
            checks.push(ConditionCheck {
                id: "minimum_level".into(),
                status: if i64::from(target_level) >= minimum {
                    ConditionStatus::Met
                } else {
                    ConditionStatus::NotMet
                },
            });
        }
        let eligibility = match advancement_validation::evaluate(&checks, None)? {
            ValidationDecision::Rejected { unmet_ids, .. } if !unmet_ids.is_empty() => "not_met",
            _ => "unknown",
        };
        rows.insert(
            feat.id.clone(),
            Candidate {
                name: feat.name.unwrap_or_else(|| feat.id.clone()),
                id: feat.id,
                source: source.into(),
                category: category
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                minimum_level,
                eligibility,
            },
        );
    }
    Ok(empty)
}
