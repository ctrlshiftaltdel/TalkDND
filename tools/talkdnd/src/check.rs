//! `talkdnd check`: ability checks, skill checks, tool checks and saving
//! throws outside combat, with the modifier read from the character sheet's
//! combat block.

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

fn ability_mod(b: &serde_norway::Value, a: &str) -> i32 {
    let score = int_at(b, "abilities", a).unwrap_or(10);
    (score - 10).div_euclid(2)
}

/// Proficiency bonus from the combat block: `prof` when present, else derived
/// from `level` (2 + (level-1)/4, level 1 → +2).
fn prof_bonus_of(b: &serde_norway::Value) -> i32 {
    if let Some(p) = b.get("prof").and_then(|v| v.as_i64()) {
        return p as i32;
    }
    let level = b.get("level").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
    2 + (level.clamp(1, 20) - 1) / 4
}

/// Tools the character is proficient in, read from the structured sheet file
/// (`sheets/<id>.yaml`, `tools` list). A missing or unparsable file means no
/// tool proficiencies; the Markdown prose is never consulted.
fn structured_tools(save_dir: &Path, sheet: &Sheet) -> Vec<String> {
    let Ok(text) = fs::read_to_string(crate::charsheet::path(save_dir, &sheet.id)) else {
        return Vec::new();
    };
    let Ok(v) = serde_norway::from_str::<serde_norway::Value>(&text) else {
        return Vec::new();
    };
    v.get("tools")
        .and_then(|t| t.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Tool-name matching key: case-, whitespace- and punctuation-insensitive
/// (`Thieves' Tools`, `thieves tools` and `THIEVES-TOOLS` all match).
fn norm_tool(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        match c {
            '\'' | '’' | '‘' | '"' | '“' | '”' => {}
            '-' | '_' => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Well-known alternate spellings of the canonical (SRD) tool names.
fn tool_alias(n: &str) -> String {
    match n {
        "thiefs tools" | "thief tools" => "thieves tools",
        "tinker tools" => "tinkers tools",
        "alchemist supplies" => "alchemists supplies",
        "smith tools" => "smiths tools",
        "mason tools" => "masons tools",
        "jeweler tools" => "jewelers tools",
        "navigator tools" | "navigators kit" => "navigators tools",
        "disguise kits" => "disguise kit",
        "forgery kits" => "forgery kit",
        "herbalism kits" => "herbalism kit",
        "poisoners kits" => "poisoners kit",
        "dice sets" => "dice set",
        "gaming sets" => "gaming set",
        "musical instruments" => "musical instrument",
        _ => n,
    }
    .to_string()
}

/// Modifier and label for `tool:<name>` with the explicitly chosen ability.
/// Proficiency comes only from the structured sheet's `tools` list: a
/// proficient tool adds the combat block's proficiency bonus on top of the
/// ability modifier; anything else rolls the bare ability modifier and says so.
fn tool_modifier(
    sheet: &Sheet,
    save_dir: &Path,
    tool: &str,
    ability: Option<&str>,
) -> Result<(i32, String)> {
    let tool = tool.trim();
    if tool.is_empty() {
        bail!("empty tool name: use `tool:<name> --ability <str|dex|con|int|wis|cha>`");
    }
    let raw = ability
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| anyhow!("tool check `{tool}` needs --ability <str|dex|con|int|wis|cha>"))?;
    let a = ability_id(raw)
        .ok_or_else(|| anyhow!("unknown ability `{raw}`: use str, dex, con, int, wis or cha"))?;
    let b = &sheet.block;
    let ab = ability_mod(b, a);
    let canon = tool_alias(&norm_tool(tool));
    let proficient = structured_tools(save_dir, sheet)
        .iter()
        .any(|t| tool_alias(&norm_tool(t)) == canon);
    let m = if proficient {
        ab + prof_bonus_of(b)
    } else {
        ab
    };
    let label = if proficient {
        format!("{tool} ({a}, proficient) check")
    } else {
        format!("{tool} ({a}) check, not proficient (no proficiency bonus added)")
    };
    Ok((m, label))
}

/// Modifier and label for `what`: a skill id, an ability (`str`, `wisdom`),
/// `save:<ability>` / `<ability>-save`, or `init`. Tool checks go through
/// [`tool_modifier`] (see [`run`]).
pub fn modifier(sheet: &Sheet, what: &str) -> Result<(i32, String)> {
    let b = &sheet.block;
    let w = what.to_lowercase().replace([' ', '_'], "-");
    let ab_mod = |a: &str| ability_mod(b, a);
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
        "unknown check `{what}`: use a skill ({}), an ability (str..cha), save:<ability>, init or tool:<name> --ability <ability>",
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
    ability: Option<&str>,
    dc: Option<i32>,
    adv: bool,
    dis: bool,
    extra: Option<&str>,
    rng: &mut dyn DiceRng,
    seeded: bool,
) -> Result<String> {
    let all = sheets(save_dir);
    let sheet = find(&all, who)?;
    let (m, label) = if what.to_lowercase().starts_with("tool:") {
        tool_modifier(sheet, save_dir, &what[5..], ability)?
    } else {
        if ability.is_some() {
            bail!("--ability only applies to tool:<name> checks, not `{what}`");
        }
        modifier(sheet, what)?
    };
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
    crate::rolls::append(save_dir, &sheet.name, &label, &text, seeded)?;
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

    #[test]
    fn tool_names_normalize_case_space_and_punctuation() {
        assert_eq!(norm_tool("  Thieves'  Tools "), "thieves tools");
        assert_eq!(norm_tool("THIEVES-TOOLS"), "thieves tools");
        assert_eq!(norm_tool("dice_set"), "dice set");
        assert_eq!(norm_tool("\u{201c}Dice\u{201d} Set"), "dice set");
        assert_eq!(tool_alias(&norm_tool("Thief's Tools")), "thieves tools");
        assert_eq!(tool_alias(&norm_tool("tinker tools")), "tinkers tools");
        assert_eq!(tool_alias(&norm_tool("Disguise Kits")), "disguise kit");
        assert_eq!(tool_alias(&norm_tool("herbalism kit")), "herbalism kit");
    }

    #[test]
    fn tool_modifier_reads_structured_tools_and_pb() {
        let dir = std::env::temp_dir().join(format!("talkdnd-tool-mod-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sheets")).unwrap();
        fs::write(
            dir.join("sheets/bruni.yaml"),
            "version: 1\ntools: [\"thieves' tools\", dice set]\n",
        )
        .unwrap();
        let block: serde_norway::Value = serde_norway::from_str(
            "id: bruni\nname: Bruni\nlevel: 5\nprof: 3\nabilities: {dex: 14, wis: 17}\n",
        )
        .unwrap();
        let s = Sheet {
            id: "bruni".into(),
            name: "Bruni".into(),
            file: PathBuf::new(),
            block,
        };
        // Proficient: dex +2 plus prof +3.
        let (m, label) = tool_modifier(&s, &dir, "thieves' tools", Some("dex")).unwrap();
        assert_eq!(m, 5);
        assert_eq!(label, "thieves' tools (dex, proficient) check");
        // Case and spaces do not change the outcome; full ability names work.
        let (m, _) = tool_modifier(&s, &dir, "  DICE  SET ", Some("Dexterity")).unwrap();
        assert_eq!(m, 5, "normalized to a listed tool");
        // Not in the structured list: bare ability modifier, explicit note.
        let (m, label) = tool_modifier(&s, &dir, "smith tools", Some("wis")).unwrap();
        assert_eq!(m, 3, "no proficiency bonus");
        assert_eq!(
            label,
            "smith tools (wis) check, not proficient (no proficiency bonus added)"
        );
        // Errors: missing ability, unknown ability, empty tool name.
        assert!(tool_modifier(&s, &dir, "x", None).is_err());
        assert!(tool_modifier(&s, &dir, "x", Some("ego")).is_err());
        assert!(tool_modifier(&s, &dir, "  ", Some("dex")).is_err());
        // No structured file at all: nothing is proficient.
        let empty = std::env::temp_dir().join(format!("talkdnd-tool-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&empty);
        fs::create_dir_all(&empty).unwrap();
        let (m, label) = tool_modifier(&s, &empty, "thieves' tools", Some("dex")).unwrap();
        assert_eq!(m, 2);
        assert!(label.contains("not proficient"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty);
    }

    #[test]
    fn prof_bonus_comes_from_prof_or_level() {
        let v: serde_norway::Value = serde_norway::from_str("prof: 3\n").unwrap();
        assert_eq!(prof_bonus_of(&v), 3);
        let v: serde_norway::Value = serde_norway::from_str("level: 5\n").unwrap();
        assert_eq!(prof_bonus_of(&v), 3);
        let v: serde_norway::Value = serde_norway::from_str("{}\n").unwrap();
        assert_eq!(prof_bonus_of(&v), 2, "default level 1");
    }
}
