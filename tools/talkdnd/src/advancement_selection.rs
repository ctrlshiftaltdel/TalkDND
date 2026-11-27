//! Read-only validation of one player-explicit advancement selection.
//!
//! Runs on the registered read-only `identity-preview` entry: the player
//! explicitly states the opportunity and one candidate, every decidable
//! condition gets 满足/不满足/未知 with source and edition, known-illegal
//! conditions reject first, and unknown conditions continue only with a
//! separate non-empty DM confirmation that stays marked 未由程序验证.
//! Nothing is written: no locks, no files, no application, no persistence.
//! Unrecognized feats are marked 未验证、效果手动 only after the opportunity,
//! declared authorization and duplicate conditions independently pass.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use combat_sim::content::loader::Ruleset;
use combat_sim::content::schema::DomainEntity;
use combat_sim::content::ContentPack;
use serde::Serialize;

use crate::advancement_preview::{self, CharacterFacts, Preview};
use crate::advancement_validation::{self, ConditionCheck, ConditionStatus};
use crate::{charsheet, classdata, data, library};

use ConditionStatus::{Met, NotMet, Unknown};

/// The opportunity ids the preview emits; selection compares exact strings.
const ASI_2014: &str = "ability_score_improvement_or_optional_feat";
const ASI_2024: &str = "ability_score_improvement_feat_or_other_qualified_feat";
const EPIC_BOON: &str = "epic_boon_or_other_qualified_feat";

pub(crate) struct SelectionArgs<'a> {
    pub opportunity: &'a str,
    pub feat: Option<&'a str>,
    pub abilities: Option<&'a str>,
    pub declared_source: Option<&'a str>,
    pub dm_confirm: Option<&'a str>,
}

pub(crate) fn run(
    root: &Path,
    save: &Path,
    who: &str,
    json: bool,
    args: SelectionArgs<'_>,
) -> Result<()> {
    let selection = parse_selection(&args)?;
    let inspection = advancement_preview::inspect(root, save, who)?;
    let outcome = validate(root, &inspection.preview, &inspection.facts, selection)?;
    let rejected = outcome.decision == "rejected";
    let report = SelectionReport {
        preview: inspection.preview,
        selection: outcome,
    };
    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print!("{}", serde_norway::to_string(&report)?);
    }
    if rejected {
        std::process::exit(1);
    }
    Ok(())
}

// ------------------------------------------------------------- input parsing --

struct ParsedSelection {
    opportunity: String,
    feat: Option<String>,
    abilities: Option<Vec<(String, i32)>>,
    declared_source: Option<String>,
    dm_confirm: Option<String>,
}

fn parse_selection(args: &SelectionArgs<'_>) -> Result<ParsedSelection> {
    let opportunity = args.opportunity.trim();
    ensure!(
        !opportunity.is_empty() && opportunity == args.opportunity,
        "机会 ID 必须为非空且不带首尾空白的精确值；别名和显示名不接受"
    );
    let feat = match args.feat {
        Some(raw) => {
            ensure!(
                !raw.is_empty() && raw.trim() == raw,
                "专长 ID 必须为非空且不带首尾空白的精确值；别名和显示名不接受"
            );
            ensure!(
                raw.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
                "专长 ID 格式错误：只接受小写字母、数字、连字符或下划线，不接受别名或显示名"
            );
            Some(raw.to_owned())
        }
        None => None,
    };
    let abilities = match args.abilities {
        Some(raw) => Some(parse_ability_spec(raw)?),
        None => None,
    };
    ensure!(
        feat.is_some() ^ abilities.is_some(),
        "缺少候选：请用 --select-feat 或 --select-abilities 给出唯一候选；两者不能同时给出"
    );
    let declared_source = match args.declared_source {
        Some(raw) => {
            ensure!(
                !raw.is_empty() && raw.trim() == raw,
                "来源声明必须为非空且不带首尾空白的精确值"
            );
            ensure!(
                raw.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
                "来源声明格式错误：只接受小写字母、数字、连字符或下划线"
            );
            Some(raw.to_owned())
        }
        None => None,
    };
    let dm_confirm = match args.dm_confirm {
        Some(raw) => {
            let basis = raw.trim();
            ensure!(
                !basis.is_empty(),
                "DM 确认依据不能为空；未知条件需要非空且可审计的确认依据"
            );
            Some(basis.to_owned())
        }
        None => None,
    };
    Ok(ParsedSelection {
        opportunity: opportunity.to_owned(),
        feat,
        abilities,
        declared_source,
        dm_confirm,
    })
}

/// Strict grammar: `ability+amount` pairs separated by commas. Aliases, extra
/// whitespace, unknown abilities, repeated abilities and out-of-range amounts
/// are input errors, not conditions.
fn parse_ability_spec(raw: &str) -> Result<Vec<(String, i32)>> {
    ensure!(
        !raw.is_empty() && raw.trim() == raw,
        "属性提升格式错误：请用 str+2 或 str+1,dex+1 这样的精确写法"
    );
    let mut seen = BTreeSet::new();
    let mut pairs = Vec::new();
    for part in raw.split(',') {
        let Some((ability, amount)) = part.rsplit_once('+') else {
            bail!("属性提升格式错误：{part:?} 不是 <属性>+<数值>");
        };
        ensure!(
            !ability.is_empty() && !amount.is_empty(),
            "属性提升格式错误：{part:?} 缺少属性或数值"
        );
        ensure!(
            data::ABILITIES.contains(&ability),
            "未知属性 {ability:?}：只接受 str、dex、con、int、wis、cha 的精确拼写"
        );
        ensure!(seen.insert(ability), "同一属性不能出现两次：{ability:?}");
        let amount: i32 = amount
            .parse()
            .map_err(|_| anyhow::anyhow!("提升量 {amount:?} 不是整数"))?;
        ensure!(
            (1..=2).contains(&amount),
            "提升量越界：{amount}（单次每项只能是 1 或 2）"
        );
        pairs.push((ability.to_owned(), amount));
    }
    Ok(pairs)
}

// ------------------------------------------------------------ condition rows --

struct ConditionRow {
    id: String,
    status: ConditionStatus,
    source: String,
    edition: String,
    detail: String,
}

impl ConditionRow {
    fn new(
        id: impl Into<String>,
        status: ConditionStatus,
        source: impl Into<String>,
        edition: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status,
            source: source.into(),
            edition: edition.to_owned(),
            detail: detail.into(),
        }
    }
}

#[derive(Serialize)]
struct ConditionRowOut {
    id: String,
    status: &'static str,
    source: String,
    edition: String,
    detail: String,
}

fn status_str(status: ConditionStatus) -> &'static str {
    match status {
        Met => "met",
        NotMet => "not_met",
        Unknown => "unknown",
    }
}

fn srd_pack_id(edition: &str) -> &'static str {
    if edition == "2014" {
        "srd-5.1-feats"
    } else {
        "srd-5.2.1-feats"
    }
}

/// The SRD rule granting the ability-score increase: 2014 carries it in the
/// class progression, 2024 in the tracked Ability Score Improvement feat.
fn asi_rule_source(edition: &str) -> &'static str {
    if edition == "2014" {
        "SRD 5.1 class progression (Ability Score Improvement)"
    } else {
        "SRD 5.2.1 Ability Score Improvement feat"
    }
}

// --------------------------------------------------------------- report out --

#[derive(Serialize)]
struct SelectionReport {
    #[serde(flatten)]
    preview: Preview,
    selection: SelectionOutcome,
}

#[derive(Serialize)]
struct SelectionOutcome {
    player_selection: PlayerSelectionEcho,
    candidate: CandidateEcho,
    conditions: Vec<ConditionRowOut>,
    decision: &'static str,
    dm_confirmation: DmConfirmation,
    /// True only when unknown conditions were continued via DM confirmation.
    not_program_verified: bool,
    /// This entry never applies effects; application stays outside it.
    application: &'static str,
    /// AC-3 marking for unrecognized feats; absent when it does not apply.
    unrecognized_marking: Option<&'static str>,
    rejection: Option<Rejection>,
    unknowns: Vec<String>,
}

#[derive(Serialize)]
struct PlayerSelectionEcho {
    opportunity: String,
    opportunity_source: &'static str,
    declared_source: Option<String>,
}

#[derive(Serialize)]
struct CandidateEcho {
    kind: &'static str,
    id: String,
    name: Option<String>,
    recognized: bool,
    source: Option<String>,
    category: Option<String>,
    increases: Option<Vec<IncreaseEcho>>,
}

#[derive(Serialize)]
struct IncreaseEcho {
    ability: String,
    amount: i32,
    current_score: Option<i32>,
    score_after: Option<i32>,
}

#[derive(Serialize)]
struct DmConfirmation {
    provided: bool,
    basis: Option<String>,
}

#[derive(Serialize)]
struct Rejection {
    unmet_ids: Vec<String>,
    unknown_ids: Vec<String>,
}

fn validate(
    root: &Path,
    preview: &Preview,
    facts: &CharacterFacts,
    selection: ParsedSelection,
) -> Result<SelectionOutcome> {
    let edition = preview.ruleset.edition();
    let mut rows: Vec<ConditionRow> = Vec::new();

    let opportunity_status = match preview.opportunity {
        Some(opportunity) if opportunity == selection.opportunity => Met,
        Some(_) => NotMet,
        None => NotMet,
    };
    let detail = match preview.opportunity {
        Some(opportunity) if opportunity == selection.opportunity => {
            format!("该等级的登记机会即 {opportunity}")
        }
        Some(opportunity) => format!(
            "该等级的登记机会是 {opportunity}，不是 {}",
            selection.opportunity
        ),
        None => "该等级没有登记的成长机会".to_owned(),
    };
    rows.push(ConditionRow::new(
        "opportunity_matches",
        opportunity_status,
        preview.opportunity_source,
        edition,
        detail,
    ));
    let no_opportunity = preview.opportunity.is_none();

    let CandidateOutcome {
        rows: candidate_rows,
        echo: candidate,
        unrecognized,
    } = match (&selection.abilities, &selection.feat) {
        (Some(pairs), _) => ability_outcome(preview, edition, pairs, no_opportunity),
        (None, Some(feat)) => feat_outcome(
            root,
            preview,
            facts,
            edition,
            feat,
            selection.declared_source.as_deref(),
            no_opportunity,
        )?,
        (None, None) => unreachable!("parse_selection guarantees exactly one candidate"),
    };

    rows.extend(candidate_rows);
    ensure!(!rows.is_empty(), "没有可判定的条件");

    let checks: Vec<ConditionCheck> = rows
        .iter()
        .map(|row| ConditionCheck {
            id: row.id.clone(),
            status: row.status,
        })
        .collect();
    let decision = advancement_validation::evaluate(&checks, selection.dm_confirm.as_deref())?;

    let (decision_str, not_program_verified, rejection, dm_provided) = match &decision {
        advancement_validation::ValidationDecision::Verified => ("verified", false, None, false),
        advancement_validation::ValidationDecision::ConfirmedUnknown { .. } => {
            ("confirmed_unknown", true, None, true)
        }
        advancement_validation::ValidationDecision::Rejected {
            unmet_ids,
            unknown_ids,
        } => (
            "rejected",
            false,
            Some(Rejection {
                unmet_ids: unmet_ids.clone(),
                unknown_ids: unknown_ids.clone(),
            }),
            selection.dm_confirm.is_some(),
        ),
    };

    // AC-3: an unrecognized feat is marked 未验证、效果手动 only when the
    // opportunity, authorization and duplicate conditions each independently
    // passed; DM confirmations of unknowns cannot substitute for them.
    let marking_required = [
        "opportunity_matches",
        "candidate_authorized",
        "known_duplicate",
    ];
    let unrecognized_marking = if unrecognized
        && !matches!(
            decision,
            advancement_validation::ValidationDecision::Rejected { .. }
        )
        && rows
            .iter()
            .all(|row| !marking_required.contains(&row.id.as_str()) || row.status == Met)
    {
        Some("unverified_effects_manual")
    } else {
        None
    };

    let mut unknowns: Vec<String> = preview
        .unknowns
        .iter()
        .map(|entry| (*entry).to_owned())
        .collect();
    if unrecognized && unrecognized_marking.is_none() {
        unknowns.push("unrecognized_marking_requirements".to_owned());
    }

    Ok(SelectionOutcome {
        player_selection: PlayerSelectionEcho {
            opportunity: selection.opportunity,
            opportunity_source: preview.opportunity_source,
            declared_source: selection.declared_source,
        },
        candidate,
        conditions: rows
            .into_iter()
            .map(|row| ConditionRowOut {
                id: row.id,
                status: status_str(row.status),
                source: row.source,
                edition: row.edition,
                detail: row.detail,
            })
            .collect(),
        decision: decision_str,
        dm_confirmation: DmConfirmation {
            provided: dm_provided,
            basis: selection.dm_confirm,
        },
        not_program_verified,
        application: "not_applied_read_only_validation",
        unrecognized_marking,
        rejection,
        unknowns,
    })
}

// ------------------------------------------------------------- feat conditions --

struct CandidateOutcome {
    rows: Vec<ConditionRow>,
    echo: CandidateEcho,
    unrecognized: bool,
}

fn feat_outcome(
    root: &Path,
    preview: &Preview,
    facts: &CharacterFacts,
    edition: &str,
    feat_id: &str,
    declared_source: Option<&str>,
    no_opportunity: bool,
) -> Result<CandidateOutcome> {
    if no_opportunity {
        // Nothing else is decidable without a registered opportunity.
        return Ok(CandidateOutcome {
            rows: Vec::new(),
            echo: CandidateEcho {
                kind: "feat",
                id: feat_id.to_owned(),
                name: None,
                recognized: false,
                source: declared_source.map(str::to_owned),
                category: None,
                increases: None,
            },
            unrecognized: true,
        });
    }
    let statuses = library::pack_status(root);
    let recognized = preview
        .candidates
        .iter()
        .find(|candidate| candidate.id == feat_id)
        .cloned();
    let mut rows = Vec::new();
    let (authorized_status, authorized_detail, echo_source) = match &recognized {
        Some(candidate) => (
            Met,
            format!("专长来自已授权目录 {}", candidate.source),
            Some(candidate.source.clone()),
        ),
        None => {
            unrecognized_authorization(root, preview.ruleset, feat_id, declared_source, &statuses)?
        }
    };
    rows.push(ConditionRow::new(
        "candidate_authorized",
        authorized_status,
        echo_source.unwrap_or_else(|| "declared_source".to_owned()),
        edition,
        authorized_detail,
    ));

    let (echo, unrecognized) = match &recognized {
        Some(candidate) => {
            let entity = load_feat(root, preview.ruleset, &candidate.source, feat_id, &statuses)?;
            append_prereq_rows(
                &mut rows,
                preview,
                facts,
                edition,
                &candidate.source,
                &entity,
            );
            append_category_row(&mut rows, preview, edition, entity.data.as_ref());
            let (duplicate, duplicate_detail) =
                duplicate_status(facts, entity.id.as_str(), entity.name.as_deref());
            rows.push(ConditionRow::new(
                "known_duplicate",
                duplicate,
                "structured character sheet",
                edition,
                duplicate_detail,
            ));
            (
                CandidateEcho {
                    kind: "feat",
                    id: feat_id.to_owned(),
                    name: Some(candidate.name.clone()),
                    recognized: true,
                    source: Some(candidate.source.clone()),
                    category: candidate.category.clone(),
                    increases: None,
                },
                false,
            )
        }
        None => {
            rows.push(ConditionRow::new(
                "prerequisites_verifiable",
                Unknown,
                "no catalog entry",
                edition,
                "未识别专长没有可核对的前提数据；效果与前提保持未验证",
            ));
            let (duplicate, duplicate_detail) = duplicate_status(facts, feat_id, None);
            rows.push(ConditionRow::new(
                "known_duplicate",
                duplicate,
                "structured character sheet",
                edition,
                duplicate_detail,
            ));
            (
                CandidateEcho {
                    kind: "feat",
                    id: feat_id.to_owned(),
                    name: None,
                    recognized: false,
                    source: declared_source.map(str::to_owned),
                    category: None,
                    increases: None,
                },
                true,
            )
        }
    };
    Ok(CandidateOutcome {
        rows,
        echo,
        unrecognized,
    })
}

/// Authorization verdict for a feat the authorized catalog does not contain.
/// A known-but-unauthorized source (other edition, locked FULL pack) rejects;
/// a declared authorized source passes; anything else stays unknown.
fn unrecognized_authorization(
    root: &Path,
    ruleset: Ruleset,
    feat_id: &str,
    declared_source: Option<&str>,
    statuses: &[library::PackStatus],
) -> Result<(ConditionStatus, String, Option<String>)> {
    let hits = scan_unauthorized_sources(root, ruleset, feat_id, statuses)?;
    if !hits.is_empty() {
        return Ok((
            NotMet,
            format!(
                "专长只出现在未授权来源 {}；当前规则集不授权这些来源",
                hits.join(", ")
            ),
            Some(hits.join(", ")),
        ));
    }
    let authorized = authorized_source_ids(ruleset, statuses);
    let known = known_source_ids(ruleset, &authorized, statuses);
    match declared_source {
        Some(declared) if authorized.iter().any(|id| id == declared) => Ok((
            Met,
            format!("声明来源 {declared} 属于本规则集的授权目录；专长本身仍未经目录核对"),
            Some(declared.to_owned()),
        )),
        Some(declared) if known.iter().any(|id| id == declared) => Ok((
            NotMet,
            format!("声明来源 {declared} 不在当前规则集的授权范围内"),
            Some(declared.to_owned()),
        )),
        Some(declared) => Ok((
            Unknown,
            format!("声明来源 {declared} 无法在本机核对为已授权内容"),
            Some(declared.to_owned()),
        )),
        None => Ok((
            Unknown,
            "未识别专长缺少 --select-source 来源声明；未授权内容不能由程序接受".to_owned(),
            None,
        )),
    }
}

fn authorized_source_ids(ruleset: Ruleset, statuses: &[library::PackStatus]) -> Vec<String> {
    let edition = ruleset.edition();
    let mut ids = vec![srd_pack_id(edition).to_owned()];
    if ruleset.is_full() {
        ids.extend(
            statuses
                .iter()
                .filter(|status| {
                    status.edition == edition && status.state == library::PackState::Unlocked
                })
                .map(|status| status.id.clone()),
        );
    }
    ids
}

fn known_source_ids(
    ruleset: Ruleset,
    authorized: &[String],
    statuses: &[library::PackStatus],
) -> Vec<String> {
    let edition = ruleset.edition();
    let other = if edition == "2014" { "2024" } else { "2014" };
    let mut ids = vec![srd_pack_id(other).to_owned()];
    ids.extend(
        statuses
            .iter()
            .filter(|status| !authorized.contains(&status.id))
            .map(|status| status.id.clone()),
    );
    ids
}

/// Search the packs this ruleset does not authorize for the feat id: the other
/// edition's tracked SRD pack and every present FULL pack file.
fn scan_unauthorized_sources(
    root: &Path,
    ruleset: Ruleset,
    feat_id: &str,
    statuses: &[library::PackStatus],
) -> Result<Vec<String>> {
    let edition = ruleset.edition();
    let other = if edition == "2014" { "2024" } else { "2014" };
    let other_id = srd_pack_id(other);
    let mut hits = Vec::new();
    let other_path = root
        .join("Rules/combat")
        .join(other)
        .join("srd")
        .join(format!("{other_id}.pack.json"));
    if regular_file(&other_path) && find_in_pack(&other_path, feat_id)?.is_some() {
        hits.push(other_id.to_owned());
    }
    let authorized = authorized_source_ids(ruleset, statuses);
    for status in statuses {
        if authorized.contains(&status.id) {
            continue;
        }
        let Some(file) = &status.file else { continue };
        let path = root.join(file);
        if regular_file(&path) && find_in_pack(&path, feat_id)?.is_some() {
            hits.push(status.id.clone());
        }
    }
    Ok(hits)
}

fn find_in_pack(path: &Path, feat_id: &str) -> Result<Option<DomainEntity>> {
    let Ok(value) = library::pack_value(path) else {
        return Ok(None);
    };
    let Ok(pack) = ContentPack::from_value(&value) else {
        return Ok(None);
    };
    Ok(pack
        .content
        .feats
        .unwrap_or_default()
        .into_iter()
        .find(|feat| feat.id == feat_id))
}

fn load_feat(
    root: &Path,
    ruleset: Ruleset,
    source: &str,
    feat_id: &str,
    statuses: &[library::PackStatus],
) -> Result<DomainEntity> {
    let edition = ruleset.edition();
    let path = if source == srd_pack_id(edition) {
        root.join("Rules/combat")
            .join(edition)
            .join("srd")
            .join(format!("{}.pack.json", srd_pack_id(edition)))
    } else {
        let status = statuses
            .iter()
            .find(|status| status.id == source)
            .context("已授权内容包缺少来源记录")?;
        let file = status.file.as_ref().context("已授权内容包没有来源文件")?;
        root.join(file)
    };
    ensure!(regular_file(&path), "专长目录来源不是普通文件");
    find_in_pack(&path, feat_id)?.context("已授权目录中没有该专长条目")
}

enum ParsedPrereq {
    MinimumLevel(i64),
    FeatureNamed(String),
    AbilityScore { ability: String, minimum: i64 },
    Unparsed(Vec<String>),
}

/// Parses only the shapes the tracked SRD packs use: the 2024 dict form
/// (`minimum_level`, `feature_named`) and the 2014 array form
/// (`ability_score.index` + `minimum_score`). Anything else stays unparsed.
fn parse_prereqs(data: Option<&serde_json::Value>) -> Vec<ParsedPrereq> {
    let mut out: Vec<ParsedPrereq> = Vec::new();
    let mut unparsed: Vec<String> = Vec::new();
    if let Some(prereqs) = data.and_then(|data| data.get("prerequisites")) {
        match prereqs {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    match (key.as_str(), value) {
                        ("minimum_level", serde_json::Value::Number(number)) => {
                            if let Some(level) = number.as_i64() {
                                out.push(ParsedPrereq::MinimumLevel(level));
                                continue;
                            }
                        }
                        ("feature_named", serde_json::Value::String(name)) => {
                            out.push(ParsedPrereq::FeatureNamed(name.clone()));
                            continue;
                        }
                        _ => {}
                    }
                    unparsed.push(key.clone());
                }
            }
            serde_json::Value::Array(items) => {
                let mut seen_abilities = BTreeSet::new();
                for item in items {
                    let ability = item
                        .get("ability_score")
                        .and_then(|score| score.get("index"))
                        .and_then(serde_json::Value::as_str);
                    let minimum = item
                        .get("minimum_score")
                        .and_then(serde_json::Value::as_i64);
                    match (ability, minimum) {
                        (Some(ability), Some(minimum))
                            if data::ABILITIES.contains(&ability)
                                && seen_abilities.insert(ability) =>
                        {
                            out.push(ParsedPrereq::AbilityScore {
                                ability: ability.to_owned(),
                                minimum,
                            });
                        }
                        _ => unparsed.push("array item".to_owned()),
                    }
                }
            }
            _ => unparsed.push("prerequisites".to_owned()),
        }
    }
    if !unparsed.is_empty() {
        out.push(ParsedPrereq::Unparsed(unparsed));
    }
    out
}

fn append_prereq_rows(
    rows: &mut Vec<ConditionRow>,
    preview: &Preview,
    facts: &CharacterFacts,
    edition: &str,
    source: &str,
    entity: &DomainEntity,
) {
    for prereq in parse_prereqs(entity.data.as_ref()) {
        match prereq {
            ParsedPrereq::MinimumLevel(minimum) => {
                let status = if i64::from(preview.target_level) >= minimum {
                    Met
                } else {
                    NotMet
                };
                rows.push(ConditionRow::new(
                    "prerequisite_minimum_level",
                    status,
                    source,
                    edition,
                    format!("前提等级 ≥ {minimum}；目标等级 {}", preview.target_level),
                ));
            }
            ParsedPrereq::FeatureNamed(name) => {
                let status = feature_named_status(preview, facts, &name);
                rows.push(ConditionRow::new(
                    "prerequisite_feature_named",
                    status,
                    source,
                    edition,
                    match status {
                        Met => format!("职业特性表包含 {name}（目标等级前）"),
                        _ => format!("基础职业特性表没有 {name}；子职或本地资料可能提供，保持未知"),
                    },
                ));
            }
            ParsedPrereq::AbilityScore { ability, minimum } => {
                let score = score_of(preview, &ability);
                let status = match score {
                    Some(score) if i64::from(score) >= minimum => Met,
                    Some(_) => NotMet,
                    None => Unknown,
                };
                rows.push(ConditionRow::new(
                    format!("prerequisite_ability_{ability}"),
                    status,
                    source,
                    edition,
                    match score {
                        Some(score) => format!("前提 {ability} ≥ {minimum}；当前 {score}"),
                        None => format!("前提 {ability} ≥ {minimum}；当前分数未知或来源冲突"),
                    },
                ));
            }
            ParsedPrereq::Unparsed(keys) => {
                rows.push(ConditionRow::new(
                    "prerequisite_unparsed",
                    Unknown,
                    source,
                    edition,
                    format!("前提数据包含未支持的字段 {}；保持未知", keys.join(", ")),
                ));
            }
        }
    }
}

/// Met only from the base class feature table (or the sheet's own subclass);
/// otherwise unknown — subclass-granted features must not become false
/// known-illegal rejections.
fn feature_named_status(preview: &Preview, facts: &CharacterFacts, name: &str) -> ConditionStatus {
    let edition = preview.ruleset.edition();
    let Some(definition) = classdata::Book::srd(edition).classes.get(&preview.class) else {
        return Unknown;
    };
    let has = |features: &[(i32, String, String)]| {
        features.iter().any(|(level, feature, _)| {
            *level <= preview.target_level && feature.eq_ignore_ascii_case(name)
        })
    };
    if has(&definition.features) {
        return Met;
    }
    let Some(sheet) = facts.sheet.as_ref() else {
        return Unknown;
    };
    let Some(slug) = charsheet::subclass_of_cf(sheet, definition) else {
        return Unknown;
    };
    let Some(sub) = definition.subclasses.get(&slug) else {
        return Unknown;
    };
    if has(&sub.features) {
        Met
    } else {
        Unknown
    }
}

fn append_category_row(
    rows: &mut Vec<ConditionRow>,
    preview: &Preview,
    edition: &str,
    data: Option<&serde_json::Value>,
) {
    if preview.opportunity != Some(EPIC_BOON) {
        return;
    }
    let category = data
        .and_then(|data| data.get("type"))
        .and_then(serde_json::Value::as_str);
    let status = if category == Some("epic-boon") {
        Met
    } else {
        Unknown
    };
    rows.push(ConditionRow::new(
        "candidate_category_vs_opportunity",
        status,
        "SRD 5.2.1 class progression (Epic Boon)",
        edition,
        match status {
            Met => "专长类别为 epic-boon，与史诗恩惠机会匹配".to_owned(),
            _ => "专长类别不是 epic-boon；该机会下是否允许其他合格专长未经跟踪数据裁定".to_owned(),
        },
    ));
}

/// Known duplicates come from the structured sheet's feat list; legacy
/// Markdown-only cards keep their selection history unknown.
fn duplicate_status(
    facts: &CharacterFacts,
    id: &str,
    name: Option<&str>,
) -> (ConditionStatus, String) {
    let Some(sheet) = facts.sheet.as_ref() else {
        return (
            Unknown,
            "没有结构化角色卡；既有选择历史按未知处理".to_owned(),
        );
    };
    let id = id.trim().to_lowercase();
    let name = name.unwrap_or_default().trim().to_lowercase();
    if sheet.feats.iter().any(|entry| {
        let entry = entry.trim().to_lowercase();
        !entry.is_empty() && (entry == id || (!name.is_empty() && entry == name))
    }) {
        (NotMet, format!("结构化角色卡已记录 {id}，已知重复"))
    } else {
        (Met, "结构化角色卡未记录该专长".to_owned())
    }
}

// --------------------------------------------------------- ability conditions --

fn ability_outcome(
    preview: &Preview,
    edition: &str,
    pairs: &[(String, i32)],
    no_opportunity: bool,
) -> CandidateOutcome {
    let mut rows = Vec::new();
    if !no_opportunity {
        let grants = matches!(
            preview.opportunity,
            Some(opportunity) if opportunity == ASI_2014 || opportunity == ASI_2024
        );
        rows.push(ConditionRow::new(
            "opportunity_grants_ability_increase",
            if grants { Met } else { NotMet },
            preview.opportunity_source,
            edition,
            if grants {
                "该机会授予属性提升（+2 一项或 +1 两项）".to_owned()
            } else {
                "该机会授予专长而非属性提升；裸属性提升不是本机会的候选形式".to_owned()
            },
        ));
        let total: i32 = pairs.iter().map(|(_, amount)| amount).sum();
        rows.push(ConditionRow::new(
            "increase_total_matches_grant",
            if total == 2 { Met } else { NotMet },
            asi_rule_source(edition),
            edition,
            format!("属性提升总量必须恰好 +2（+2 一项或 +1 两项）；当前 +{total}"),
        ));
        for (ability, amount) in pairs {
            let score = score_of(preview, ability);
            let status = match score {
                Some(score) if score + amount <= 20 => Met,
                Some(_) => NotMet,
                None => Unknown,
            };
            rows.push(ConditionRow::new(
                format!("{ability}_increase_within_cap"),
                status,
                asi_rule_source(edition),
                edition,
                match score {
                    Some(score) => format!(
                        "{ability} {score} → {}；属性提升不能把分数提高到 20 以上",
                        score + amount
                    ),
                    None => format!("{ability} 当前分数未知或来源冲突；无法核对 20 上限"),
                },
            ));
        }
    }
    CandidateOutcome {
        rows,
        echo: CandidateEcho {
            kind: "ability_increases",
            id: canonical_spec(pairs),
            name: None,
            recognized: true,
            source: Some(asi_rule_source(edition).to_owned()),
            category: None,
            increases: Some(
                pairs
                    .iter()
                    .map(|(ability, amount)| {
                        let score = score_of(preview, ability);
                        IncreaseEcho {
                            ability: ability.clone(),
                            amount: *amount,
                            current_score: score,
                            score_after: score.map(|score| score + amount),
                        }
                    })
                    .collect(),
            ),
        },
        unrecognized: false,
    }
}

fn canonical_spec(pairs: &[(String, i32)]) -> String {
    pairs
        .iter()
        .map(|(ability, amount)| format!("{ability}+{amount}"))
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------- helpers --

fn score_of(preview: &Preview, ability: &str) -> Option<i32> {
    if preview
        .abilities_with_conflicting_sources
        .iter()
        .any(|conflict| *conflict == ability)
    {
        return None;
    }
    preview
        .abilities
        .iter()
        .find(|row| row.ability == ability)
        .map(|row| row.current_score)
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}
