//! `talkdnd check`: ability checks, skill checks and saving throws outside
//! combat, with the modifier read from the character sheet's combat block.

use crate::data::{self, ABILITIES};
use crate::save;
use anyhow::{anyhow, bail, Result};
use combat_sim::dice::{DiceRng, RollMode};
use combat_sim::roll::roll_command;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Sheet {
    pub id: String,
    pub name: String,
    pub file: PathBuf,
    pub block: serde_norway::Value,
}

pub fn sheets(save: &Path) -> Vec<Sheet> {
    let mut out = Vec::new();
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
            let Some(block) = crate::start::combat_block(&text) else {
                continue;
            };
            let s = |k: &str| {
                block
                    .get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            out.push(Sheet {
                id: s("id"),
                name: s("name"),
                file: f,
                block,
            });
        }
    }
    out
}

pub fn find<'a>(sheets: &'a [Sheet], who: &str) -> Result<&'a Sheet> {
    let w = who.to_lowercase();
    sheets
        .iter()
        .find(|s| s.id.to_lowercase() == w || s.name.to_lowercase() == w)
        .or_else(|| {
            let hits: Vec<&Sheet> = sheets
                .iter()
                .filter(|s| s.name.to_lowercase().starts_with(&w))
                .collect();
            (hits.len() == 1).then(|| hits[0])
        })
        .ok_or_else(|| {
            anyhow!(
                "no character `{who}`; known: {}",
                sheets
                    .iter()
                    .map(|s| format!("{} ({})", s.name, s.id))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn ability_id(s: &str) -> Option<&'static str> {
    let s = s.to_lowercase();
    let full = [
        ("strength", "str"),
        ("dexterity", "dex"),
        ("constitution", "con"),
        ("intelligence", "int"),
        ("wisdom", "wis"),
        ("charisma", "cha"),
    ];
    ABILITIES
        .iter()
        .find(|a| **a == s)
        .copied()
        .or_else(|| full.iter().find(|(f, _)| *f == s).map(|(_, a)| *a))
}

fn int_at(v: &serde_norway::Value, map: &str, key: &str) -> Option<i32> {
    v.get(map)?.get(key)?.as_i64().map(|x| x as i32)
}

/// Modifier and label for `what`: a skill id, an ability (`str`, `wisdom`),
/// `save:<ability>` / `<ability>-save`, or `init`.
pub fn modifier(sheet: &Sheet, what: &str) -> Result<(i32, String)> {
    let b = &sheet.block;
    let w = what.to_lowercase().replace([' ', '_'], "-");
    let ab_mod = |a: &str| -> i32 {
        let score = int_at(b, "abilities", a).unwrap_or(10);
        (score - 10).div_euclid(2)
    };
    let save_of = w
        .strip_prefix("save:")
        .or_else(|| w.strip_suffix("-save"))
        .or_else(|| w.strip_prefix("save-"));
    if let Some(a) = save_of {
        let a = ability_id(a).ok_or_else(|| anyhow!("unknown ability `{a}`"))?;
        let m = int_at(b, "saves", a).unwrap_or_else(|| ab_mod(a));
        return Ok((m, format!("{a} saving throw")));
    }
    if w == "init" || w == "initiative" {
        let m = b
            .get("init_bonus")
            .and_then(|v| v.as_i64())
            .map(|x| x as i32)
            .unwrap_or_else(|| ab_mod("dex"));
        return Ok((m, "initiative".into()));
    }
    if let Some(a) = ability_id(&w) {
        return Ok((ab_mod(a), format!("{a} check")));
    }
    if let Some(a) = data::skill_ability(&w) {
        let m = int_at(b, "skills", &w).unwrap_or_else(|| ab_mod(a));
        return Ok((m, format!("{w} ({a}) check")));
    }
    bail!(
        "unknown check `{what}`: use a skill ({}), an ability (str..cha), save:<ability> or init",
        data::SKILLS
            .iter()
            .map(|s| s.0)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    save_dir: &Path,
    who: &str,
    what: &str,
    dc: Option<i32>,
    adv: bool,
    dis: bool,
    extra: Option<&str>,
    rng: &mut dyn DiceRng,
    seeded: bool,
) -> Result<String> {
    let all = sheets(save_dir);
    let sheet = find(&all, who)?;
    let (m, label) = modifier(sheet, what)?;
    let mut expr = if m >= 0 {
        format!("1d20+{m}")
    } else {
        format!("1d20{m}")
    };
    let mode = RollMode::from_flags(adv, dis);
    let extra = extra.map(str::trim).filter(|x| !x.is_empty());
    let text = match extra {
        Some(x) if x.contains(['d', 'D']) => {
            // Extra dice (guidance, bless): roll the d20 test and the extra
            // dice separately so advantage still applies to the d20.
            let base = roll_command(&expr, mode, None, false, rng)?;
            let x = x.trim_start_matches('+');
            let more = roll_command(x, RollMode::Normal, None, false, rng)?;
            let total = base.total + more.total;
            let verdict = dc.map_or(String::new(), |dc| {
                format!(" vs DC {dc} {}", if total >= dc { "PASS" } else { "FAIL" })
            });
            format!(
                "{} ; extra {} → total {total}{verdict}",
                base.text, more.text
            )
        }
        Some(x) => {
            if x.starts_with('+') || x.starts_with('-') {
                expr += x;
            } else {
                expr = format!("{expr}+{x}");
            }
            roll_command(&expr, mode, dc, false, rng)?.text
        }
        None => roll_command(&expr, mode, dc, false, rng)?.text,
    };
    let line = format!("{} {label}: {text}", sheet.name);
    if let Some(log) = save::latest_log(save_dir) {
        let tag = if seeded { " (seeded test roll)" } else { "" };
        save::append(
            &log,
            &format!(
                "- [{}] 检定{tag}：{line}",
                chrono::Local::now().format("%H:%M")
            ),
        )?;
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> Sheet {
        let block: serde_norway::Value = serde_norway::from_str(
            "id: bruni\nname: Bruni\ninit_bonus: 0\nabilities: {str: 14, dex: 10, con: 16, int: 8, wis: 17, cha: 9}\nsaves: {wis: 5, cha: 1}\nskills: {insight: 5, medicine: 5}\n",
        )
        .unwrap();
        Sheet {
            id: "bruni".into(),
            name: "Bruni".into(),
            file: PathBuf::new(),
            block,
        }
    }

    #[test]
    fn modifiers_come_from_the_sheet() {
        let s = sheet();
        assert_eq!(modifier(&s, "insight").unwrap().0, 5);
        assert_eq!(
            modifier(&s, "perception").unwrap().0,
            3,
            "untrained = wis mod"
        );
        assert_eq!(modifier(&s, "athletics").unwrap().0, 2);
        assert_eq!(modifier(&s, "Wisdom").unwrap().0, 3);
        assert_eq!(modifier(&s, "save:wis").unwrap().0, 5);
        assert_eq!(modifier(&s, "con-save").unwrap().0, 3);
        assert_eq!(modifier(&s, "int").unwrap().0, -1);
        assert_eq!(modifier(&s, "init").unwrap().0, 0);
        assert!(modifier(&s, "juggling").is_err());
    }

    #[test]
    fn find_by_id_name_or_unique_prefix() {
        let all = vec![sheet()];
        assert_eq!(find(&all, "BRUNI").unwrap().id, "bruni");
        assert_eq!(find(&all, "bru").unwrap().id, "bruni");
        assert!(find(&all, "ilsa").is_err());
    }
}
