//! Read-only derivation of one validated advancement selection (ADV-P1-9).
//!
//! Runs on the registered read-only `identity-preview` entry, after the same
//! selection validation used by the plain read-only chain: the validated
//! player selection is consumed, and the combat block's derived values get a
//! reviewable candidate diff. Nothing is written: no locks, no files, no
//! application, no persistence; submission, active-encounter writes and full
//! feat effects belong to later advancement tasks.
//!
//! Reliability rule: a value is proposed for update (`auto`) only when two
//! things are provable from the save and rule data — the current value
//! matches the program's own derivation at the character's current level,
//! **and** the governing source (class proficiency table, recorded
//! background/species grants, weapon proficiency, the card's complete
//! level-up record chain) supports the shape that matched. A numeric match
//! alone proves nothing: a hand or item bonus that happens to equal
//! mod + proficiency would otherwise be promoted on the next proficiency
//! step. Values whose shape, source or history cannot be proven are
//! preserved verbatim as `dm_review` or `keep` entries with the reason.
//! Each `auto` entry carries absolute `before`/`after` values, so a repeated
//! computation is byte-identical and a future application that checks
//! `before` against the current value cannot stack.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_norway::Value;

use crate::advancement_preview::{self, CharacterFacts, Preview};
use crate::advancement_selection::{self, SelectionArgs, SelectionOutcome};
use crate::{chargen, classdata, data, progress};

pub(crate) fn run(
    root: &Path,
    save: &Path,
    who: &str,
    json: bool,
    args: SelectionArgs<'_>,
) -> Result<()> {
    let inspection = advancement_preview::inspect(root, save, who)?;
    let outcome =
        advancement_selection::evaluate(root, &inspection.preview, &inspection.facts, args)?;
    let rejected = outcome.decision == "rejected";
    let derived = if rejected {
        None
    } else {
        Some(derive(
            root,
            &inspection.preview,
            &inspection.facts,
            &outcome,
        )?)
    };
    let report = DeriveReport {
        preview: inspection.preview,
        selection: outcome,
        derived,
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

#[derive(Serialize)]
struct DeriveReport {
    #[serde(flatten)]
    preview: Preview,
    selection: SelectionOutcome,
    /// Absent when the selection was rejected: a rejected selection has no
    /// candidate to derive from.
    derived: Option<DerivedCandidate>,
}

#[derive(Serialize)]
struct DerivedCandidate {
    /// This step never applies effects; application stays outside it.
    application: &'static str,
    /// Echo of the preview's encounter gate; the candidate never touches
    /// encounter state either way.
    encounter_gate: String,
    entries: Vec<Entry>,
    dm_items: Vec<String>,
}

#[derive(Serialize)]
struct Entry {
    /// Stable id inside the candidate (for example `combat.saves.wis`).
    id: String,
    category: &'static str,
    /// `auto` = proposed update with provenance proof; `keep` = intentionally
    /// untouched; `dm_review` = preserved verbatim, DM must decide.
    disposition: &'static str,
    /// Combat-block path an application would write (`auto` entries only).
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<serde_json::Value>,
    reason: String,
    source: String,
}

impl Entry {
    fn auto(
        id: impl Into<String>,
        category: &'static str,
        target: impl Into<String>,
        before: serde_json::Value,
        after: serde_json::Value,
        reason: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category,
            disposition: "auto",
            target: Some(target.into()),
            before: Some(before),
            after: Some(after),
            reason: reason.into(),
            source: source.into(),
        }
    }

    fn keep(id: impl Into<String>, category: &'static str, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            category,
            disposition: "keep",
            target: None,
            before: None,
            after: None,
            reason: reason.into(),
            source: "not derived (candidate boundary)".to_owned(),
        }
    }

    fn dm_review(
        id: impl Into<String>,
        category: &'static str,
        before: Option<serde_json::Value>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category,
            disposition: "dm_review",
            target: None,
            before,
            after: None,
            reason: reason.into(),
            source: "not provable from the save and rule data".to_owned(),
        }
    }
}

// ------------------------------------------------------------- derive core --

/// One parsed `## 升级记录` line: the provenance record a program level-up
/// leaves behind (`- <date>：升到 <level> 级；HP +<gain>（d<die> <how>，体质
/// <con>）→ <total>；熟练 +<prof>`). `gain` and `con` are needed to verify
/// the whole accumulation chain, not just the last line.
struct LevelRecord {
    level: i32,
    gain: i64,
    die: i32,
    con: i64,
    mode: RecordMode,
    total: i64,
}

enum RecordMode {
    Average,
    Chosen,
    Rolled,
}

fn parse_record(line: &str) -> Option<LevelRecord> {
    let level_text: String = after_marker(line, "升到 ")?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let level: i32 = level_text.parse().ok()?;
    let gain_start = line.find("HP +")? + "HP +".len();
    let gain_text: String = line[gain_start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let gain: i64 = gain_text.parse().ok()?;
    let die_start = line.find("（d")? + "（d".len();
    let die_text: String = line[die_start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let die: i32 = die_text.parse().ok()?;
    let how = &line[die_start..];
    let how = how
        .split_once(['，', ','])?
        .0
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == 'd')
        .trim_start();
    let mode = if how.starts_with("average") {
        RecordMode::Average
    } else if how.starts_with("chosen") {
        RecordMode::Chosen
    } else {
        RecordMode::Rolled
    };
    let con_start = line.find("体质 ")? + "体质 ".len();
    let con_text: String = line[con_start..]
        .chars()
        .take_while(|c| *c == '+' || *c == '-' || c.is_ascii_digit())
        .collect();
    let con: i64 = con_text.parse().ok()?;
    let total_start = line.find("→ ")? + "→ ".len();
    let total_text: String = line[total_start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let total: i64 = total_text.parse().ok()?;
    Some(LevelRecord {
        level,
        gain,
        die,
        con,
        mode,
        total,
    })
}

fn after_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let start = line.find(marker)? + marker.len();
    Some(&line[start..])
}

fn derive(
    root: &Path,
    preview: &Preview,
    facts: &CharacterFacts,
    outcome: &SelectionOutcome,
) -> Result<DerivedCandidate> {
    let edition = preview.ruleset.edition();
    // The chargen generation takes the edition string ("2014"/"2024"),
    // the same value the legacy level-up path passes.
    let rs = edition;
    let book = classdata::Book::srd(edition);
    let class = preview.class.clone();
    let definition = book.classes.get(&class);
    let subclass = definition.and_then(|cd| {
        facts
            .sheet
            .as_ref()
            .and_then(|sheet| crate::charsheet::subclass_of_cf(sheet, cd))
    });
    let block = facts
        .structured_combat
        .clone()
        .unwrap_or_else(|| facts.markdown_combat.clone());
    let scores = block_scores(&block);
    // Fields both carriers record but disagree on: neither side is trusted,
    // so those entries stay at their current value for the DM.
    let conflicts = carrier_conflicts(facts.structured_combat.as_ref(), &facts.markdown_combat);
    let increases = validated_increases(outcome);
    let prof_old = i64::from(progress::prof_for_level(preview.current_level));
    let prof_new = i64::from(progress::prof_for_level(preview.target_level));
    let prof_delta = prof_new - prof_old;
    // Modifier of one ability, before (`false`) and after (`true`) applying
    // the validated increases. Conflicting or missing sources make the
    // modifier unprovable, so every dependent value falls to review.
    let mod_of = |ability: &str, bump: bool| -> Option<i64> {
        if preview
            .abilities_with_conflicting_sources
            .contains(&ability)
        {
            return None;
        }
        let score = *scores.get(ability)?;
        // Scores outside the readable 1..=30 range make every dependent
        // value unprovable (hand-modified or corrupt data).
        if !(1..=30).contains(&score) {
            return None;
        }
        let score = score
            + if bump {
                i64::from(*increases.get(ability).unwrap_or(&0))
            } else {
                0
            };
        Some(i64::from(data::modifier(score as i32)))
    };

    // One regeneration per level: the same pure generator the chargen and
    // level-up paths use, gated by comparison against the current block.
    let generation_of = |level: i32| -> classdata::Generated {
        let mut map = BTreeMap::new();
        for ability in data::ABILITIES {
            let score = scores.get(ability).copied().unwrap_or(10);
            map.insert((*ability).to_string(), score as i32);
        }
        let creation = chargen::Creation {
            ruleset: Some(rs.to_string()),
            class: Some(class.clone()),
            subclass: subclass.clone(),
            level,
            scores: map,
            ..Default::default()
        };
        classdata::generate(&creation, &book, Some(root))
    };
    let generation_old = generation_of(preview.current_level);
    let generation_new = generation_of(preview.target_level);

    // Provable proficiency sources, per category:
    // - saving throws: the built-in class proficiency table;
    // - skills: only recorded facts from the save — the chargen state's
    //   chosen/granted skills when its name matches this character, plus
    //   background/species grants resolved from the sheet's exact option
    //   ids. The class choosable list says what a character *could* pick, not
    //   what it did, so it is not evidence by itself;
    // - attacks: the class weapon-proficiency data through
    //   `classdata::proficient` (unarmed is always proficient in the
    //   generator). A numeric match alone never proves proficiency.
    let class_data = data::class(&class);
    let mut provable_skills: BTreeSet<&str> = BTreeSet::new();
    if let Some(recorded) = facts.chargen_skills.as_ref() {
        provable_skills.extend(recorded.iter().map(|skill| skill.as_str()));
    }
    if let Some(sheet) = facts.sheet.as_ref() {
        // Structured options: exact ids resolve through the built-in tables;
        // unresolvable values contribute nothing (their grants stay unproven).
        if let Some(background) = sheet.info.get("background").map(|value| value.trim()) {
            if let Some(granted) = data::background(edition, background) {
                provable_skills.extend(granted.skills.iter().copied());
            }
        }
        if let Some(species) = sheet.info.get("species").map(|value| value.trim()) {
            if let Some(granted) = data::species(edition, species) {
                provable_skills.extend(granted.skills.iter().copied());
            }
        }
    }

    let mut entries: Vec<Entry> = Vec::new();

    // Level: the preview already verified the card's level is the registered
    // current level, so the step to the target level is the one provable
    // change everything else builds on.
    entries.push(Entry::auto(
        "combat.level",
        "level",
        "combat.level",
        serde_json::json!(preview.current_level),
        serde_json::json!(preview.target_level),
        format!(
            "角色卡当前等级 {} 经预览核对；本次机会推进到 {} 级",
            preview.current_level, preview.target_level
        ),
        preview.opportunity_source,
    ));

    // Proficiency bonus.
    if conflicts.contains("combat.prof") {
        entries.push(Entry::dm_review(
            "combat.prof",
            "proficiency",
            block
                .get("prof")
                .and_then(Value::as_i64)
                .map(|value| serde_json::json!(value)),
            "两个载体的 prof 不一致；来源冲突，保持原值并交 DM 核对",
        ));
    } else {
        match block.get("prof").and_then(Value::as_i64) {
            Some(current) if current == prof_old => {
                if prof_delta != 0 {
                    entries.push(Entry::auto(
                        "combat.prof",
                        "proficiency",
                        "combat.prof",
                        serde_json::json!(prof_old),
                        serde_json::json!(prof_new),
                        format!(
                            "当前值与 {} 级标准熟练推导一致；{}→{} 级熟练 {prof_old}→{prof_new}",
                            preview.current_level, preview.current_level, preview.target_level
                        ),
                        "SRD class table proficiency progression",
                    ));
                } else {
                    entries.push(Entry::keep(
                        "combat.prof",
                        "proficiency",
                        format!(
                            "当前值与标准熟练推导一致，且 {}→{} 级熟练加值不变",
                            preview.current_level, preview.target_level
                        ),
                    ));
                }
            }
            Some(current) => entries.push(Entry::dm_review(
                "combat.prof",
                "proficiency",
                Some(serde_json::json!(current)),
                format!(
                    "当前熟练加值 {current} 与 {} 级标准推导 {prof_old} 不一致；可能为手改或未知来源，保持原值",
                    preview.current_level
                ),
            )),
            None => entries.push(Entry::dm_review(
                "combat.prof",
                "proficiency",
                None,
                "战斗块没有 prof 字段；熟练加值来源无法证实，保持原样",
            )),
        }
    }

    // Ability scores named by the validated selection. Scores with
    // conflicting markdown/sheet sources stay untouched (hand edits).
    for (ability, amount) in &increases {
        let id = format!("combat.abilities.{ability}");
        let conflicted = preview
            .abilities_with_conflicting_sources
            .contains(&ability.as_str());
        let current = scores.get(ability.as_str()).copied();
        match (conflicted, current) {
            (true, _) => entries.push(Entry::dm_review(
                id,
                "ability",
                None,
                format!(
                    "{ability} 的 Markdown 卡与结构化角色卡分数冲突（手改或来源未知）；保持原值，不应用 +{amount}"
                ),
            )),
            (false, Some(current)) if current + i64::from(*amount) <= 20 => entries.push(Entry::auto(
                id,
                "ability",
                format!("combat.abilities.{ability}"),
                serde_json::json!(current),
                serde_json::json!(current + i64::from(*amount)),
                format!("已验证选择 {ability}+{amount}；当前分数与 20 上限核对一致"),
                "player selection validated on this entry",
            )),
            (false, Some(current)) => entries.push(Entry::dm_review(
                id,
                "ability",
                Some(serde_json::json!(current)),
                format!(
                    "{ability} {current}→{} 超过 20 上限；保持原值",
                    current + i64::from(*amount)
                ),
            )),
            (false, None) => entries.push(Entry::dm_review(
                id,
                "ability",
                None,
                format!("{ability} 分数缺失；保持原值，不应用 +{amount}"),
            )),
        }
    }
    if !increases.is_empty() {
        let untouched: Vec<String> = data::ABILITIES
            .iter()
            .filter(|ability| !increases.contains_key(**ability))
            .map(|ability| (*ability).to_string())
            .collect();
        entries.push(Entry::keep(
            "combat.abilities.other",
            "ability",
            format!(
                "未出现在已验证选择中的属性（{}）保持原值",
                untouched.join(", ")
            ),
        ));
    }

    // Maximum HP: only a fully verifiable accumulation chain allows an
    // update. See `derive_hp`.
    entries.extend(derive_hp(preview, facts, &book, &class, &block, &mod_of));

    // Saving throws listed in the block. The class proficiency table is the
    // only accepted source: a listed save outside it (hand-added entry) keeps
    // its value, even when the number happens to equal mod + proficiency.
    if let Some(saves) = block.get("saves").and_then(Value::as_mapping) {
        for (key, value) in saves {
            let Some(ability) = key.as_str() else {
                continue;
            };
            let Some(actual) = value.as_i64() else {
                entries.push(Entry::dm_review(
                    format!("combat.saves.{ability}"),
                    "save",
                    None,
                    "存档值不是数字；保持原值",
                ));
                continue;
            };
            let id = format!("combat.saves.{ability}");
            if conflicts.contains(&id) {
                entries.push(Entry::dm_review(
                    id,
                    "save",
                    Some(serde_json::json!(actual)),
                    format!("两个载体的 {ability} 豁免不一致；来源冲突，保持原值并交 DM 核对"),
                ));
                continue;
            }
            let provable = class_data.is_some_and(|c| c.saves.contains(&ability));
            if !provable {
                entries.push(Entry::dm_review(
                    id,
                    "save",
                    Some(serde_json::json!(actual)),
                    format!(
                        "职业熟练豁免表不含 {ability}；该条目来源无法证实（手改或自定义记录），数值 {actual} 保持原值"
                    ),
                ));
                continue;
            }
            match (mod_of(ability, false), mod_of(ability, true)) {
                (Some(m_old), Some(m_new)) => {
                    let expected_old = m_old + prof_old;
                    if actual == expected_old {
                        let after = m_new + prof_new;
                        let shape = format!("属性调整值 {m_old} + 熟练 {prof_old}（职业熟练豁免）");
                        if after == actual {
                            entries.push(Entry::keep(
                                id,
                                "save",
                                format!("存档值等于{shape}，且本等级 {ability} 豁免不变"),
                            ));
                        } else {
                            entries.push(Entry::auto(
                                id,
                                "save",
                                format!("combat.saves.{ability}"),
                                serde_json::json!(actual),
                                serde_json::json!(after),
                                format!("当前值等于{shape}；属性与熟练变化推导为 {actual}→{after}"),
                                "class save proficiency table + validated ability change",
                            ));
                        }
                    } else {
                        entries.push(Entry::dm_review(
                            id,
                            "save",
                            Some(serde_json::json!(actual)),
                            format!(
                                "存档值 {actual} 与 {ability} 熟练豁免的推导（{expected_old}）不一致；可能为手改或魔法物品加值，保持原值"
                            ),
                        ));
                    }
                }
                _ => entries.push(Entry::dm_review(
                    id,
                    "save",
                    Some(serde_json::json!(actual)),
                    format!("{ability} 分数缺失或来源冲突；豁免值保持原值"),
                )),
            }
        }
    }

    // Skills. The choosable class list plus recorded background/species
    // grants are the only accepted proficiency sources; the expertise shape
    // has no persisted record and therefore never auto-updates.
    if let Some(skills) = block.get("skills").and_then(Value::as_mapping) {
        for (key, value) in skills {
            let Some(skill) = key.as_str() else {
                continue;
            };
            let Some(actual) = value.as_i64() else {
                entries.push(Entry::dm_review(
                    format!("combat.skills.{skill}"),
                    "skill",
                    None,
                    "技能值不是数字；保持原值",
                ));
                continue;
            };
            let id = format!("combat.skills.{skill}");
            if conflicts.contains(&id) {
                entries.push(Entry::dm_review(
                    id,
                    "skill",
                    Some(serde_json::json!(actual)),
                    format!("两个载体的 {skill} 技能值不一致；来源冲突，保持原值并交 DM 核对"),
                ));
                continue;
            }
            let Some(ability) = data::skill_ability(skill) else {
                entries.push(Entry::dm_review(
                    id,
                    "skill",
                    Some(serde_json::json!(actual)),
                    format!("技能 {skill} 不在可核对清单内；保持原值"),
                ));
                continue;
            };
            if !provable_skills.contains(skill) {
                entries.push(Entry::dm_review(
                    id,
                    "skill",
                    Some(serde_json::json!(actual)),
                    format!(
                        "技能 {skill} 没有可核对的实际记录（职业可选清单只说明能选什么，不说明选过什么；只有与角色匹配的 chargen 已选记录或背景/种族授予才算来源）；该条目来源无法证实（手改或自定义记录），数值 {actual} 保持原值"
                    ),
                ));
                continue;
            }
            match (mod_of(ability, false), mod_of(ability, true)) {
                (Some(m_old), Some(m_new)) => {
                    let expected_old = m_old + prof_old;
                    if actual == expected_old {
                        let after = m_new + prof_new;
                        let shape =
                            format!("属性调整值 {m_old} + 熟练 {prof_old}（职业/背景/种族熟练）");
                        if after == actual {
                            entries.push(Entry::keep(
                                id,
                                "skill",
                                format!("技能值等于{shape}，且本等级 {skill} 不变"),
                            ));
                        } else {
                            entries.push(Entry::auto(
                                id,
                                "skill",
                                format!("combat.skills.{skill}"),
                                serde_json::json!(actual),
                                serde_json::json!(after),
                                format!(
                                    "当前值等于{shape}；属性与熟练变化推导为 {actual}→{after}"
                                ),
                                "class/background/species skill proficiency + validated ability change",
                            ));
                        }
                    } else if actual == m_old + 2 * prof_old {
                        entries.push(Entry::dm_review(
                            id,
                            "skill",
                            Some(serde_json::json!(actual)),
                            format!(
                                "技能值 {actual} 符合专精形态，但专精选择没有存档记录；无法与手改加值区分，保持原值并交 DM 核对"
                            ),
                        ));
                    } else {
                        entries.push(Entry::dm_review(
                            id,
                            "skill",
                            Some(serde_json::json!(actual)),
                            format!(
                                "技能值 {actual} 与 {skill} 熟练的推导（{expected_old}）不一致；可能为手改或魔法物品加值，保持原值"
                            ),
                        ));
                    }
                }
                _ => entries.push(Entry::dm_review(
                    id,
                    "skill",
                    Some(serde_json::json!(actual)),
                    format!("{ability} 分数缺失或来源冲突；{skill} 保持原值"),
                )),
            }
        }
    }

    // Attacks: to_hit follows the weapon's provable proficiency from the
    // class weapon data (`classdata::proficient`); unarmed is always
    // proficient in the generator. Unknown ids and everything else keep
    // their value.
    if let Some(attacks) = block.get("attacks").and_then(Value::as_sequence) {
        for attack in attacks {
            let Some(to_hit) = attack.get("to_hit").and_then(Value::as_i64) else {
                continue;
            };
            let name = attack
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| attack.get("name").and_then(Value::as_str))
                .unwrap_or("unknown");
            let id = format!("combat.attacks.{name}.to_hit");
            if conflicts.contains(&id) {
                entries.push(Entry::dm_review(
                    id,
                    "attack",
                    Some(serde_json::json!(to_hit)),
                    format!("两个载体的 {name} to_hit 不一致；来源冲突，保持原值并交 DM 核对"),
                ));
                continue;
            }
            let Some(ability) = attack.get("ability").and_then(Value::as_str) else {
                entries.push(Entry::dm_review(
                    id,
                    "attack",
                    Some(serde_json::json!(to_hit)),
                    "攻击没有 ability 字段；to_hit 来源无法证实，保持原值",
                ));
                continue;
            };
            let weapon = attack_proficiency(&book, definition, name, rs);
            let Some(proficient) = weapon else {
                entries.push(Entry::dm_review(
                    id,
                    "attack",
                    Some(serde_json::json!(to_hit)),
                    format!(
                        "攻击 {name} 不在职业武器熟练可核对的范围内（非已知武器且非徒手）；to_hit 来源无法证实，保持原值"
                    ),
                ));
                continue;
            };
            match (mod_of(ability, false), mod_of(ability, true)) {
                (Some(m_old), Some(m_new)) => {
                    let expected_old = m_old + if proficient { prof_old } else { 0 };
                    if to_hit == expected_old {
                        let after = m_new + if proficient { prof_new } else { 0 };
                        let shape = if proficient {
                            format!("{ability} 调整值 {m_old} + 熟练 {prof_old}（武器熟练可核对）")
                        } else {
                            format!("{ability} 调整值 {m_old}（该武器不熟练，可核对）")
                        };
                        if after == to_hit {
                            entries.push(Entry::keep(
                                id,
                                "attack",
                                format!("{name} 的 to_hit 等于{shape}，且本等级不变"),
                            ));
                        } else {
                            entries.push(Entry::auto(
                                id,
                                "attack",
                                format!("combat.attacks.{name}.to_hit"),
                                serde_json::json!(to_hit),
                                serde_json::json!(after),
                                format!("to_hit 等于{shape}；推导为 {to_hit}→{after}"),
                                "class weapon proficiency + validated ability change",
                            ));
                        }
                    } else {
                        entries.push(Entry::dm_review(
                            id,
                            "attack",
                            Some(serde_json::json!(to_hit)),
                            format!(
                                "to_hit {to_hit} 与 {name} 的可核对推导（{expected_old}）不一致；可能为魔法武器加值或手改，保持原值"
                            ),
                        ));
                    }
                }
                _ => entries.push(Entry::dm_review(
                    id,
                    "attack",
                    Some(serde_json::json!(to_hit)),
                    format!("{ability} 分数缺失或来源冲突；{name} 的 to_hit 保持原值"),
                )),
            }
        }
    }

    // Spellcasting attack/DC: recognized only when the block's ability
    // matches the class data and both numbers match the level derivation.
    entries.extend(derive_spellcasting(
        &book, &class, &block, &conflicts, &mod_of, prof_old, prof_new,
    ));

    // Resources: only the `max` of a resource whose current max matches the
    // class table at the current level is proposed; current values (spent
    // uses), die fields and notes are never part of the candidate.
    entries.extend(derive_resources(
        preview,
        &block,
        &conflicts,
        &generation_old,
        &generation_new,
    ));

    // Candidate boundaries the ACs require to be visible in every report.
    entries.push(Entry::keep(
        "spell_lists",
        "preserved",
        "法术列表（spells 块）不在派生候选内；新增法术位容量由 DM 与玩家另行选择",
    ));
    entries.push(Entry::keep(
        "combat_refs",
        "preserved",
        "战斗引用（actions/features）与手动效果保持原样；完整专长效果不在本任务内",
    ));
    entries.push(Entry::keep(
        "resource_currents",
        "preserved",
        "资源当前值（含已花次数）与 hit-dice 的 die 字段保持原样",
    ));
    entries.push(Entry::keep(
        "encounter_state",
        "preserved",
        "当前遭遇与战斗状态保持原样；本候选不做任何写入",
    ));

    Ok(DerivedCandidate {
        application: "not_applied_read_only_candidate",
        encounter_gate: preview.encounter.to_owned(),
        entries,
        dm_items: dm_items(preview, facts, outcome, &book, &class, &generation_new),
    })
}

// ------------------------------------------------- provable proficiency --

/// Provable to-hit proficiency of one attack entry: `Some(true)` /
/// `Some(false)` when the weapon's proficiency can be verified from the
/// class weapon data, `None` when the source cannot be verified. Unarmed is
/// always proficient in the generator; `-2h` variants share the base
/// weapon's proficiency.
fn attack_proficiency(
    book: &classdata::Book,
    definition: Option<&classdata::ClassDef>,
    attack_id: &str,
    rs: &str,
) -> Option<bool> {
    if attack_id == "unarmed" {
        return Some(true);
    }
    let base = attack_id.strip_suffix("-2h").unwrap_or(attack_id);
    let weapon = book.weapons.iter().find(|w| w.id == base)?.for_ruleset(rs);
    Some(classdata::proficient(definition, &weapon))
}

// -------------------------------------------------------------------- HP --

/// Why the current max HP is or is not provably the program's own
/// average-mode accumulation. Level 1 must match the creation formula
/// (the complete one-level chain); later levels must have exactly one
/// average-mode record per level 2..=N whose gains, Constitution values and
/// cumulative totals reconcile all the way back to that formula, whose last
/// total equals the current value, and whose last Constitution still matches
/// the block's current Constitution.
fn hp_chain_status(
    facts: &CharacterFacts,
    die: i32,
    hp: i64,
    current_level: i32,
    con_now: i64,
) -> Result<(), String> {
    let mut records: Vec<LevelRecord> = Vec::new();
    for line in &facts.level_records {
        match parse_record(line) {
            Some(record) => records.push(record),
            None => return Err("存在无法解析的升级记录".to_owned()),
        }
    }
    let die_average = i64::from(die) / 2 + 1;
    // The chain starts from the level-1 creation formula: die + the
    // Constitution modifier the first record shows (a hand-changed pre-L2
    // Constitution breaks the base and fails the chain).
    let mut expected: Option<i64> = None;
    for level in 2..=current_level {
        let rows: Vec<&LevelRecord> = records
            .iter()
            .filter(|record| record.level == level)
            .collect();
        let Some(record) = rows.first().copied() else {
            return Err(format!("升级记录缺少 {level} 级，平均链不完整"));
        };
        if rows.len() > 1 {
            return Err(format!(
                "升级记录中 {level} 级有 {} 条，相互重复或矛盾",
                rows.len()
            ));
        }
        if !matches!(record.mode, RecordMode::Average) {
            return Err(format!(
                "{level} 级记录不是平均模式（掷骰或自选历史无法程序推算）"
            ));
        }
        if record.die != die {
            return Err(format!(
                "{level} 级记录的生命骰 d{} 与职业数据 d{die} 不一致",
                record.die
            ));
        }
        if record.gain != die_average + record.con {
            return Err(format!(
                "{level} 级记录的增量 {} 与平均推导 {}（d{die} 平均 + 体质）不一致",
                record.gain,
                die_average + record.con
            ));
        }
        let base = expected.unwrap_or_else(|| i64::from(die) + record.con);
        let reconciled = base + record.gain;
        if reconciled != record.total {
            return Err(format!(
                "{level} 级记录的累计值 {} 与前序链推导 {reconciled} 不一致",
                record.total
            ));
        }
        expected = Some(record.total);
    }
    if records
        .iter()
        .any(|record| record.level < 2 || record.level > current_level)
    {
        return Err("升级记录含 2 级至当前等级之外的条目，链不连续".to_owned());
    }
    let Some(last) = records.iter().find(|record| record.level == current_level) else {
        return Err("升级记录缺少当前等级".to_owned());
    };
    if last.total != hp {
        return Err(format!(
            "当前最大 HP {hp} 与最近一次升级记录的累计值 {} 不一致",
            last.total
        ));
    }
    if last.con != con_now {
        return Err(format!(
            "升级记录的体质调整值 {:+} 与当前体质调整值 {con_now:+} 不一致；最后一次升级后体质被改动",
            last.con
        ));
    }
    Ok(())
}

fn derive_hp(
    preview: &Preview,
    facts: &CharacterFacts,
    book: &classdata::Book,
    class: &str,
    block: &Value,
    mod_of: &dyn Fn(&str, bool) -> Option<i64>,
) -> Vec<Entry> {
    let id = "combat.hp";
    // A structured card and its rendered Markdown must agree on HP; a
    // mismatch means one carrier was hand-edited and neither side can be
    // trusted as the source.
    if let Some(sheet_combat) = facts.structured_combat.as_ref() {
        let markdown_hp = facts.markdown_combat.get("hp").and_then(Value::as_i64);
        let sheet_hp = sheet_combat.get("hp").and_then(Value::as_i64);
        if let (Some(markdown_hp), Some(sheet_hp)) = (markdown_hp, sheet_hp) {
            if markdown_hp != sheet_hp {
                return vec![Entry::dm_review(
                    id,
                    "hp",
                    Some(serde_json::json!(sheet_hp)),
                    format!(
                        "两个载体的 HP 不一致（Markdown {markdown_hp} / 结构化角色卡 {sheet_hp}）；来源冲突，保持原值并交 DM 核对"
                    ),
                )];
            }
        }
    }
    let Some(hp) = block.get("hp").and_then(Value::as_i64) else {
        return vec![Entry::dm_review(
            id,
            "hp",
            None,
            "战斗块没有 hp 字段；最大 HP 来源无法证实，保持原样",
        )];
    };
    // Hit die follows the generator's own fallback chain: the class book
    // first, then the built-in SRD class table. A class with neither stays
    // unprovable instead of silently defaulting.
    let die = book
        .classes
        .get(class)
        .and_then(|definition| definition.hit_die)
        .or_else(|| data::class(class).map(|class_data| class_data.hit_die));
    let Some(die) = die else {
        return vec![Entry::dm_review(
            id,
            "hp",
            Some(serde_json::json!(hp)),
            format!("职业 {class} 没有生命骰数据；最大 HP 来源无法证实，保持原值"),
        )];
    };
    let (Some(con_old), Some(con_new)) = (mod_of("con", false), mod_of("con", true)) else {
        return vec![Entry::dm_review(
            id,
            "hp",
            Some(serde_json::json!(hp)),
            "体质分数缺失或来源冲突；最大 HP 无法核对，保持原值",
        )];
    };
    // Provenance: the complete average chain, not just the last record.
    let proven = if preview.current_level == 1 {
        if hp == i64::from(die) + con_old {
            Ok(())
        } else {
            Err(format!(
                "1 级最大 HP 应为 d{die}+体质调整值（{con_old:+}）；当前值 {hp} 与其不符"
            ))
        }
    } else {
        hp_chain_status(facts, die, hp, preview.current_level, con_old)
    };
    let chain_note = match &proven {
        Ok(()) => {
            if preview.current_level == 1 {
                "1 级建卡公式逐值吻合（完整链）".to_owned()
            } else {
                format!(
                    "升级记录 {}→{} 级完整平均链逐级吻合",
                    2, preview.current_level
                )
            }
        }
        Err(reason) => reason.clone(),
    };
    if proven.is_err() {
        return vec![Entry::dm_review(
            id,
            "hp",
            Some(serde_json::json!(hp)),
            format!("{chain_note}；最大 HP 保持原值，增量交 DM 核对"),
        )];
    }
    let die_average = i64::from(die) / 2 + 1;
    let gain = (die_average + con_new).max(1);
    let retro = con_new - con_old;
    let after = hp + gain + retro * i64::from(preview.current_level);
    let reason = if retro == 0 {
        format!(
            "平均 HP 链可证实（{chain_note}）；增量 = d{die} 平均 {die_average} + 新体质 {con_new:+} = +{gain}"
        )
    } else {
        format!(
            "平均 HP 链可证实（{chain_note}）；增量 = d{die} 平均 {die_average} + 新体质 {con_new:+} = +{gain}，另有体质 {con_old:+}→{con_new:+} 的追溯 {retro}×{} 级 = +{}",
            preview.current_level,
            retro * i64::from(preview.current_level)
        )
    };
    vec![Entry::auto(
        id,
        "hp",
        "combat.hp",
        serde_json::json!(hp),
        serde_json::json!(after),
        reason,
        "SRD hit die / class data + validated con change（体质调整值变化的追溯规则见 LetsPlay/docs/character-data.md）",
    )]
}

// ---------------------------------------------------------- spellcasting --

fn derive_spellcasting(
    book: &classdata::Book,
    class: &str,
    block: &Value,
    conflicts: &BTreeSet<String>,
    mod_of: &dyn Fn(&str, bool) -> Option<i64>,
    prof_old: i64,
    prof_new: i64,
) -> Vec<Entry> {
    let conflicting: Vec<&str> = ["ability", "attack", "dc"]
        .iter()
        .filter(|field| conflicts.contains(&format!("combat.spellcasting.{field}")))
        .copied()
        .collect();
    if !conflicting.is_empty() {
        return vec![Entry::dm_review(
            "combat.spellcasting",
            "spellcasting",
            None,
            format!(
                "两个载体的施法字段不一致（{}）；来源冲突，保持原值并交 DM 核对",
                conflicting.join(", ")
            ),
        )];
    }
    let Some(ability) = progress::spell_ability(book, class) else {
        return match block.get("spellcasting") {
            Some(_) => vec![Entry::keep(
                "combat.spellcasting",
                "spellcasting",
                "职业数据不含施法属性；既有 spellcasting 块保持原样",
            )],
            None => Vec::new(),
        };
    };
    let Some(sc) = block.get("spellcasting").and_then(Value::as_mapping) else {
        return vec![Entry::dm_review(
            "combat.spellcasting",
            "spellcasting",
            None,
            format!("职业施法属性为 {ability}，但战斗块没有 spellcasting 块；施法数值无法核对"),
        )];
    };
    // The block records the casting ability under `spellcasting.ability`.
    let recorded = sc.get("ability").and_then(Value::as_str);
    let attack = sc.get("attack").and_then(Value::as_i64);
    let dc = sc.get("dc").and_then(Value::as_i64);
    let (Some(m_old), Some(m_new)) = (mod_of(&ability, false), mod_of(&ability, true)) else {
        return vec![Entry::dm_review(
            "combat.spellcasting",
            "spellcasting",
            attack.map(|value| serde_json::json!(value)),
            format!("{ability} 分数缺失或来源冲突；施法 attack/DC 保持原值"),
        )];
    };
    let (Some(attack), Some(dc)) = (attack, dc) else {
        return vec![Entry::dm_review(
            "combat.spellcasting",
            "spellcasting",
            None,
            "spellcasting 块缺少数字的 attack/dc 字段；保持原值",
        )];
    };
    if recorded == Some(ability.as_str())
        && attack == m_old + prof_old
        && dc == 8 + m_old + prof_old
    {
        let after_attack = m_new + prof_new;
        let after_dc = 8 + m_new + prof_new;
        if after_attack == attack && after_dc == dc {
            return vec![Entry::keep(
                "combat.spellcasting",
                "spellcasting",
                "施法 attack/DC 与当前等级推导一致，且本等级不变",
            )];
        }
        return vec![
            Entry::auto(
                "combat.spellcasting.attack",
                "spellcasting",
                "combat.spellcasting.attack",
                serde_json::json!(attack),
                serde_json::json!(after_attack),
                format!(
                    "当前 attack 等于 {ability} 调整值 {m_old} + 熟练 {prof_old}；推导为 {attack}→{after_attack}"
                ),
                "SRD spellcasting progression + validated ability change",
            ),
            Entry::auto(
                "combat.spellcasting.dc",
                "spellcasting",
                "combat.spellcasting.dc",
                serde_json::json!(dc),
                serde_json::json!(after_dc),
                format!(
                    "当前 DC 等于 8 + {ability} 调整值 {m_old} + 熟练 {prof_old}；推导为 {dc}→{after_dc}"
                ),
                "SRD spell save DC + validated ability change",
            ),
        ];
    }
    vec![Entry::dm_review(
        "combat.spellcasting",
        "spellcasting",
        Some(serde_json::json!(attack)),
        format!(
            "施法 attack/DC 与 {ability} 调整值+熟练的当前等级推导不一致（预期 attack {}、DC {}）；可能为手改或多来源施法，保持原值",
            m_old + prof_old,
            8 + m_old + prof_old
        ),
    )]
}

// ------------------------------------------------------------- resources --

fn derive_resources(
    preview: &Preview,
    block: &Value,
    conflicts: &BTreeSet<String>,
    generation_old: &classdata::Generated,
    generation_new: &classdata::Generated,
) -> Vec<Entry> {
    let Some(resources) = block.get("resources").and_then(Value::as_mapping) else {
        return vec![Entry::keep(
            "combat.resources",
            "resource",
            "战斗块没有 resources 块；保持原样",
        )];
    };
    let resource_max = |generated: &classdata::Generated, id: &str| -> Option<i32> {
        generated
            .resources
            .iter()
            .find(|resource| resource.id == id)
            .map(|resource| resource.max)
    };
    let slot_max = |generated: &classdata::Generated, ring: usize| -> Option<i64> {
        generated.slots.get(ring).map(|count| i64::from(*count))
    };
    let pact_of = |generated: &classdata::Generated| -> Option<(i32, i32)> { generated.pact };
    let mut entries = Vec::new();
    for (key, value) in resources {
        let Value::String(key) = key else {
            continue;
        };
        let Some(max) = value.get("max").and_then(Value::as_i64) else {
            entries.push(Entry::dm_review(
                format!("combat.resources.{key}.max"),
                "resource",
                None,
                format!("资源 {key} 没有数字 max 字段；保持原值"),
            ));
            continue;
        };
        let id = format!("combat.resources.{key}.max");
        if conflicts.contains(&id) {
            entries.push(Entry::dm_review(
                id,
                "resource",
                Some(serde_json::json!(max)),
                format!("两个载体的资源 {key} 最大值不一致；来源冲突，保持原值并交 DM 核对"),
            ));
            continue;
        }
        if key == "hit-dice" {
            if max == i64::from(preview.current_level) {
                entries.push(Entry::auto(
                    id,
                    "resource",
                    format!("combat.resources.{key}.max"),
                    serde_json::json!(max),
                    serde_json::json!(preview.target_level),
                    format!(
                        "生命骰数量当前等于等级数；{}→{} 级推导 {max}→{}",
                        preview.current_level, preview.target_level, preview.target_level
                    ),
                    "SRD hit dice progression",
                ));
            } else {
                entries.push(Entry::dm_review(
                    id,
                    "resource",
                    Some(serde_json::json!(max)),
                    format!(
                        "生命骰数量 {max} 与等级 {} 不一致；可能为手改，保持原值",
                        preview.current_level
                    ),
                ));
            }
            continue;
        }
        if key.starts_with("spell-slot-") {
            let ring: usize = key
                .trim_start_matches("spell-slot-")
                .parse::<i32>()
                .ok()
                .and_then(|slot_level| {
                    (1..=9)
                        .contains(&slot_level)
                        .then_some((slot_level - 1) as usize)
                })
                .unwrap_or(usize::MAX);
            // Pact slots replace the regular table for their ring.
            let pact_ring = |generated: &classdata::Generated| -> Option<i64> {
                pact_of(generated)
                    .filter(|(_, level)| ring != usize::MAX && *level as usize == ring + 1)
                    .map(|(count, _)| i64::from(count))
            };
            let expected_old = pact_ring(generation_old).or_else(|| {
                if ring == usize::MAX {
                    None
                } else {
                    slot_max(generation_old, ring)
                }
            });
            let expected_new = pact_ring(generation_new).or_else(|| {
                if ring == usize::MAX {
                    None
                } else {
                    slot_max(generation_new, ring)
                }
            });
            match (expected_old, expected_new) {
                (Some(expected), Some(expected_new)) if max == expected => {
                    if expected_new != max {
                        entries.push(Entry::auto(
                            id,
                            "resource",
                            format!("combat.resources.{key}.max"),
                            serde_json::json!(max),
                            serde_json::json!(expected_new),
                            format!(
                                "法术位 {key} 当前最大值与职业表 {} 级推导一致；{}→{} 级推导 {max}→{expected_new}",
                                preview.current_level, preview.current_level, preview.target_level
                            ),
                            "SRD class spell slot table",
                        ));
                    } else {
                        entries.push(Entry::keep(
                            id,
                            "resource",
                            format!("法术位 {key} 最大值与职业表推导一致，且升级后不变"),
                        ));
                    }
                }
                (Some(expected), None) if max == expected => entries.push(Entry::dm_review(
                    id,
                    "resource",
                    Some(serde_json::json!(max)),
                    format!(
                        "法术位 {key} 在 {} 级与职业表一致，但 {} 级职业表不再生成该环；是否移除交 DM，保持原值",
                        preview.current_level, preview.target_level
                    ),
                )),
                (Some(expected), _) => entries.push(Entry::dm_review(
                    id,
                    "resource",
                    Some(serde_json::json!(max)),
                    format!(
                        "法术位 {key} 当前最大值 {max} 与 {} 级职业表推导 {expected} 不一致；可能为手改，保持原值",
                        preview.current_level
                    ),
                )),
                (None, _) => entries.push(Entry::dm_review(
                    id,
                    "resource",
                    Some(serde_json::json!(max)),
                    format!(
                        "法术位 {key} 不在职业表的 {} 级推导内；来源未知，保持原值",
                        preview.current_level
                    ),
                )),
            }
            continue;
        }
        match (resource_max(generation_old, key), resource_max(generation_new, key)) {
            (Some(expected), Some(expected_new)) if max == i64::from(expected) => {
                if expected_new == expected {
                    entries.push(Entry::keep(
                        id,
                        "resource",
                        format!("资源 {key} 最大值与职业表推导一致，且升级后不变"),
                    ));
                } else {
                    entries.push(Entry::auto(
                        id,
                        "resource",
                        format!("combat.resources.{key}.max"),
                        serde_json::json!(max),
                        serde_json::json!(expected_new),
                        format!(
                            "资源 {key} 当前最大值与职业表 {} 级推导一致；{}→{} 级推导 {max}→{expected_new}",
                            preview.current_level, preview.current_level, preview.target_level
                        ),
                        "SRD class resource table",
                    ));
                }
            }
            (Some(expected), _) => entries.push(Entry::dm_review(
                id,
                "resource",
                Some(serde_json::json!(max)),
                format!(
                    "资源 {key} 当前最大值 {max} 与 {} 级职业表推导 {expected} 不一致；可能为手改，保持原值",
                    preview.current_level
                ),
            )),
            (None, _) => entries.push(Entry::dm_review(
                id,
                "resource",
                Some(serde_json::json!(max)),
                format!("资源 {key} 不在本职业表推导内；自定义或未知来源，保持原值"),
            )),
        }
    }
    entries.push(Entry::keep(
        "combat.resources.current",
        "resource",
        "全部资源的 current（含已花次数）不在候选内；派生只提议 max 变化",
    ));
    entries
}

// -------------------------------------------------------------- dm items --

fn dm_items(
    preview: &Preview,
    facts: &CharacterFacts,
    outcome: &SelectionOutcome,
    book: &classdata::Book,
    class: &str,
    generation_new: &classdata::Generated,
) -> Vec<String> {
    let mut items = Vec::new();
    if outcome.candidate.kind == "feat" {
        items.push(
            "专长效果未自动应用：完整专长效果不在本任务内，由 DM 与玩家按规则书处理".to_owned(),
        );
        if !outcome.candidate.recognized {
            items.push(
                "该专长未经目录识别：效果与前提保持未验证（unverified_effects_manual 语义）"
                    .to_owned(),
            );
        }
    }
    if let Some(definition) = book.classes.get(class) {
        for (level, name, _) in &definition.features {
            if *level == preview.target_level {
                items.push(format!("新职业特性 {name}（{level} 级）需要手动登记"));
            }
        }
        if definition.subclass_level == Some(preview.target_level) {
            items.push(format!(
                "{} 级需要选择子职（talkdnd options）；派生未按子职特性调整资源",
                preview.target_level
            ));
        }
        if definition.arcanum {
            for (ring, unlock) in [(6, 11), (7, 13), (8, 15), (9, 17)] {
                if preview.target_level == unlock {
                    items.push(format!("神秘奥义 {ring} 环解锁：法术选择由玩家与 DM 处理"));
                }
            }
        }
        if let Some(sheet) = facts.sheet.as_ref() {
            if let Some(lists) = sheet.spells.as_ref() {
                let count = match lists.mode.as_str() {
                    "known" => lists.known.len(),
                    _ => lists.prepared.len(),
                };
                if let Some((_, cap)) = generation_new.spells {
                    if count < cap as usize {
                        items.push(format!(
                            "法术列表容量 {} 级为 {cap}，当前 {count} 条：新法术由玩家选择，列表不在候选内",
                            preview.target_level
                        ));
                    }
                }
            }
        }
    }
    for unknown in &preview.unknowns {
        items.push(format!("预览未知项 {unknown}：按既有边界交 DM 核对"));
    }
    if outcome.not_program_verified {
        items.push(
            "存在经 DM 确认继续的未知条件：结果未由程序验证（not_program_verified）".to_owned(),
        );
    }
    if preview.encounter == "active_or_unknown" {
        items.push("活动或无法核实的遭遇：候选仅供审阅，提交会被既有生命周期门阻止".to_owned());
    }
    items
}

// ---------------------------------------------------------------- helpers --

/// Field paths recorded by both carriers with disagreeing values. Operating
/// on one carrier's value and calling it reliable would silently discard the
/// other side, so every derived entry whose field is listed here keeps its
/// current value and goes to the DM.
fn carrier_conflicts(structured: Option<&Value>, markdown: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(structured) = structured else {
        return out;
    };
    // Scalars the candidate derives.
    for key in ["prof", "hp"] {
        let left = structured.get(key);
        let right = markdown.get(key);
        if let (Some(left), Some(right)) = (left, right) {
            if left != right {
                out.insert(format!("combat.{key}"));
            }
        }
    }
    // Per-ability mappings.
    for (section, prefix) in [("saves", "combat.saves"), ("skills", "combat.skills")] {
        let (Some(left), Some(right)) = (
            structured.get(section).and_then(Value::as_mapping),
            markdown.get(section).and_then(Value::as_mapping),
        ) else {
            continue;
        };
        for (key, left_value) in left {
            let Some(key) = key.as_str() else {
                continue;
            };
            if let Some((_, right_value)) =
                right.iter().find(|(other, _)| other.as_str() == Some(key))
            {
                if left_value != right_value {
                    out.insert(format!("{prefix}.{key}"));
                }
            }
        }
    }
    // Resources: the candidate only proposes `max`, so only a `max` mismatch
    // is a conflict for this step.
    if let (Some(left), Some(right)) = (
        structured.get("resources").and_then(Value::as_mapping),
        markdown.get("resources").and_then(Value::as_mapping),
    ) {
        for (key, left_value) in left {
            let Some(key) = key.as_str() else {
                continue;
            };
            if let Some((_, right_value)) =
                right.iter().find(|(other, _)| other.as_str() == Some(key))
            {
                if left_value.get("max") != right_value.get("max") {
                    out.insert(format!("combat.resources.{key}.max"));
                }
            }
        }
    }
    // Attacks: match by id (falling back to name) and compare to_hit.
    if let (Some(left), Some(right)) = (
        structured.get("attacks").and_then(Value::as_sequence),
        markdown.get("attacks").and_then(Value::as_sequence),
    ) {
        for left_attack in left {
            let Some(name) = attack_name(left_attack) else {
                continue;
            };
            if let Some(right_attack) = right.iter().find(|other| attack_name(other) == Some(name))
            {
                if left_attack.get("to_hit") != right_attack.get("to_hit") {
                    out.insert(format!("combat.attacks.{name}.to_hit"));
                }
            }
        }
    }
    // Spellcasting fields.
    if let (Some(left), Some(right)) = (
        structured.get("spellcasting").and_then(Value::as_mapping),
        markdown.get("spellcasting").and_then(Value::as_mapping),
    ) {
        for field in ["ability", "attack", "dc"] {
            let left_value = left
                .iter()
                .find(|(key, _)| key.as_str() == Some(field))
                .map(|(_, value)| value);
            let right_value = right
                .iter()
                .find(|(key, _)| key.as_str() == Some(field))
                .map(|(_, value)| value);
            if left_value != right_value {
                out.insert(format!("combat.spellcasting.{field}"));
            }
        }
    }
    out
}

fn attack_name(attack: &Value) -> Option<&str> {
    attack
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| attack.get("name").and_then(Value::as_str))
}

fn block_scores(block: &Value) -> BTreeMap<&str, i64> {
    let mut scores = BTreeMap::new();
    if let Some(abilities) = block.get("abilities").and_then(Value::as_mapping) {
        for (key, value) in abilities {
            if let (Some(ability), Some(score)) = (key.as_str(), value.as_i64()) {
                scores.insert(ability, score);
            }
        }
    }
    scores
}

/// The validated selection's ability increases (ability selections only).
/// Feat candidates carry no automatic ability application here: full feat
/// effects stay outside this task.
fn validated_increases(outcome: &SelectionOutcome) -> BTreeMap<String, i32> {
    let mut map = BTreeMap::new();
    if let Some(increases) = &outcome.candidate.increases {
        for increase in increases {
            map.insert(increase.ability.clone(), increase.amount);
        }
    }
    map
}
