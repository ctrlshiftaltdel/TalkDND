//! Character creation state, rolls and rule validation.

use crate::data::{self, ABILITIES};
use anyhow::{bail, Context, Result};
use combat_sim::dice::DiceRng;
use combat_sim::dice::RollMode;
use combat_sim::roll::roll_command;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Steps in the order the DM must walk the player through.
pub const STEPS: &[(&str, &str)] = &[
    ("ruleset", "规则版本 2014 / 2024"),
    ("adventure", "冒险"),
    ("species", "种族/物种"),
    ("class", "职业"),
    ("background", "背景"),
    (
        "method",
        "属性生成方式：roll（4d6 去最低）/ array（标准数组）/ pointbuy（购点）",
    ),
    ("scores", "玩家把数值分配到六项属性"),
    ("asi", "属性加值（2014 种族 / 2024 背景）"),
    ("skills", "技能"),
    ("equipment", "装备（含护甲、盾牌）"),
    ("spells", "法术（非施法职业写 none）"),
    ("name", "名字与性格"),
    ("confirm", "确认角色卡"),
];

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Creation {
    pub ruleset: Option<String>,
    pub adventure: Option<String>,
    pub species: Option<String>,
    pub class: Option<String>,
    pub background: Option<String>,
    pub method: Option<String>,
    /// Numbers available to assign (rolled or the array).
    #[serde(default)]
    pub pool: Vec<i32>,
    /// Base scores chosen by the player, before increases.
    #[serde(default)]
    pub scores: BTreeMap<String, i32>,
    /// Player-chosen increases (2024 background; 2014 half-elf free picks).
    #[serde(default)]
    pub asi: BTreeMap<String, i32>,
    #[serde(default)]
    pub asi_done: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    /// Skills from a non-SRD background, given by the player.
    #[serde(default)]
    pub background_skills: Vec<String>,
    pub armor: Option<String>,
    #[serde(default)]
    pub shield: bool,
    #[serde(default)]
    pub equipment: Vec<String>,
    #[serde(default)]
    pub spells: Vec<String>,
    #[serde(default)]
    pub spells_done: bool,
    pub name: Option<String>,
    pub personality: Option<String>,
    #[serde(default = "one")]
    pub level: i32,
    #[serde(default)]
    pub confirmed: bool,
}

fn one() -> i32 {
    1
}

pub fn state_path(save: &Path) -> PathBuf {
    save.join("chargen.json")
}

pub fn load(save: &Path) -> Result<Creation> {
    let p = state_path(save);
    if !p.is_file() {
        bail!(
            "no character creation in progress here ({}); run `talkdnd new` first",
            p.display()
        );
    }
    serde_json::from_str(&fs::read_to_string(&p)?).context("chargen.json is not valid")
}

pub fn store(save: &Path, c: &Creation) -> Result<()> {
    fs::write(state_path(save), serde_json::to_string_pretty(c)? + "\n")?;
    Ok(())
}

impl Creation {
    pub fn rs(&self) -> &str {
        self.ruleset.as_deref().unwrap_or("2024")
    }

    pub fn done(&self, step: &str) -> bool {
        match step {
            "ruleset" => self.ruleset.is_some(),
            "adventure" => self.adventure.is_some(),
            "species" => self.species.is_some(),
            "class" => self.class.is_some(),
            "background" => self.background.is_some(),
            "method" => {
                self.method.is_some()
                    && (self.method.as_deref() == Some("pointbuy") || !self.pool.is_empty())
            }
            "scores" => self.scores.len() == 6,
            "asi" => self.asi_done,
            "skills" => !self.skills.is_empty(),
            "equipment" => self.armor.is_some() || !self.equipment.is_empty(),
            "spells" => self.spells_done,
            "name" => self.name.is_some(),
            "confirm" => self.confirmed,
            _ => false,
        }
    }

    pub fn next_step(&self) -> Option<(&'static str, &'static str)> {
        STEPS.iter().copied().find(|(s, _)| !self.done(s))
    }

    /// Increases that apply automatically (2014 species).
    pub fn fixed_asi(&self) -> BTreeMap<String, i32> {
        let mut m = BTreeMap::new();
        if self.rs() == "2014" {
            if let Some(sp) = self
                .species
                .as_deref()
                .and_then(|s| data::species("2014", s))
            {
                for (a, v) in sp.asi {
                    *m.entry(a.to_string()).or_insert(0) += v;
                }
            }
        }
        m
    }

    pub fn final_scores(&self) -> BTreeMap<String, i32> {
        let fixed = self.fixed_asi();
        ABILITIES
            .iter()
            .map(|a| {
                let base = self.scores.get(*a).copied().unwrap_or(10);
                let bonus =
                    fixed.get(*a).copied().unwrap_or(0) + self.asi.get(*a).copied().unwrap_or(0);
                (a.to_string(), base + bonus)
            })
            .collect()
    }

    pub fn mods(&self) -> BTreeMap<String, i32> {
        self.final_scores()
            .into_iter()
            .map(|(k, v)| (k, data::modifier(v)))
            .collect()
    }

    pub fn hp(&self) -> Option<i32> {
        let cls = data::class(self.class.as_deref()?)?;
        let con = self.mods()["con"];
        let sp_bonus = self
            .species
            .as_deref()
            .and_then(|s| data::species(self.rs(), s))
            .map(|s| s.hp_per_level)
            .unwrap_or(0);
        let avg = cls.hit_die / 2 + 1;
        let lvl = self.level.max(1);
        Some((cls.hit_die + con + sp_bonus) + (lvl - 1) * (avg + con + sp_bonus))
    }

    pub fn ac(&self) -> i32 {
        let m = self.mods();
        let dex = m["dex"];
        let shield = if self.shield { 2 } else { 0 };
        match self.armor.as_deref().filter(|a| *a != "none") {
            Some(a) => match data::ARMOR.iter().find(|(id, _, _)| *id == a) {
                Some((_, base, cap)) => base + cap.map_or(dex, |c| dex.min(c)) + shield,
                None => 10 + dex + shield,
            },
            None => match self.class.as_deref() {
                Some("barbarian") => 10 + dex + m["con"] + shield,
                Some("monk") if !self.shield => 10 + dex + m["wis"],
                _ => 10 + dex + shield,
            },
        }
    }

    /// Skills granted without choice (background, species).
    pub fn granted_skills(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(b) = self.background.as_deref() {
            match data::background(self.rs(), b) {
                Some(bg) => v.extend(bg.skills.iter().map(|s| s.to_string())),
                None => v.extend(self.background_skills.iter().cloned()),
            }
        }
        if let Some(sp) = self
            .species
            .as_deref()
            .and_then(|s| data::species(self.rs(), s))
        {
            v.extend(sp.skills.iter().map(|s| s.to_string()));
        }
        v
    }
}

pub struct Report {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn validate(c: &Creation) -> Report {
    let mut e = Vec::new();
    let mut w = Vec::new();
    let rs = c.rs().to_string();
    if rs != "2014" && rs != "2024" {
        e.push(format!("ruleset must be 2014 or 2024, got {rs}"));
    }
    let sp = c.species.as_deref().and_then(|s| data::species(&rs, s));
    if let Some(s) = c.species.as_deref() {
        if sp.is_none() {
            w.push(format!(
                "species `{s}` is not in SRD ({rs}); check it against the player's book"
            ));
        }
    }
    let cls = c.class.as_deref().and_then(data::class);
    if let Some(s) = c.class.as_deref() {
        if cls.is_none() {
            w.push(format!(
                "class `{s}` is not an SRD class; check it against the player's book"
            ));
        }
    }
    let bg = c
        .background
        .as_deref()
        .and_then(|b| data::background(&rs, b));
    if let Some(b) = c.background.as_deref() {
        if bg.is_none() {
            w.push(format!(
                "background `{b}` is not in SRD ({rs}); check it against the player's book"
            ));
        }
    }

    // Scores vs method.
    if c.scores.len() == 6 {
        let mut base: Vec<i32> = ABILITIES
            .iter()
            .map(|a| c.scores.get(*a).copied().unwrap_or(0))
            .collect();
        match c.method.as_deref() {
            Some("pointbuy") => {
                let mut total = 0;
                for (a, v) in ABILITIES.iter().zip(&base) {
                    match data::point_cost(*v) {
                        Some(x) => total += x,
                        None => e.push(format!("point buy: {a} {v} must be 8..15")),
                    }
                }
                if total > 27 {
                    e.push(format!("point buy costs {total} > 27"));
                } else if total < 27 {
                    w.push(format!("point buy spends {total} of 27 points"));
                }
            }
            Some("roll") | Some("array") => {
                let mut pool = c.pool.clone();
                pool.sort();
                base.sort();
                if pool != base {
                    e.push(format!(
                        "assigned scores {base:?} are not the {} {pool:?}",
                        c.method.as_deref().unwrap_or("")
                    ));
                }
            }
            _ => e.push("method must be roll, array or pointbuy".into()),
        }
    } else if !c.scores.is_empty() {
        e.push("all six abilities need a score".into());
    }

    // Increases.
    let asi_sum: i32 = c.asi.values().sum();
    if c.asi.values().any(|v| *v < 0) {
        e.push("increases cannot be negative".into());
    }
    if rs == "2024" {
        if c.asi_done {
            let vals: Vec<i32> = c.asi.values().copied().filter(|v| *v != 0).collect();
            let ok_shape = matches!(vals.len(), 2 if asi_sum == 3 && vals.contains(&2))
                || (vals.len() == 3 && vals.iter().all(|v| *v == 1));
            if !ok_shape {
                e.push("2024 background increases must be +2/+1 or +1/+1/+1".into());
            }
            if let Some(bg) = bg {
                for a in c.asi.keys().filter(|a| c.asi[*a] != 0) {
                    if !bg.abilities.contains(&a.as_str()) {
                        e.push(format!(
                            "background {} cannot raise {a} (allowed: {})",
                            bg.id,
                            bg.abilities.join(", ")
                        ));
                    }
                }
            }
        }
    } else if let Some(sp) = sp {
        if c.asi_done {
            if asi_sum != sp.free_asi as i32 || c.asi.values().any(|v| *v > 1) {
                if sp.free_asi == 0 {
                    if asi_sum != 0 {
                        e.push(format!(
                            "{} has fixed increases only; remove extra asi",
                            sp.id
                        ));
                    }
                } else {
                    e.push(format!(
                        "{} picks {} different +1 increases",
                        sp.id, sp.free_asi
                    ));
                }
            }
            for a in c.asi.keys() {
                if sp.free_asi_exclude.contains(&a.as_str()) && c.asi[a] > 0 {
                    e.push(format!("{} cannot put a free increase into {a}", sp.id));
                }
            }
        }
    }
    let cap = 20;
    for (a, v) in c.final_scores() {
        if c.scores.len() == 6 && (v > cap || v < 3) {
            e.push(format!("{a} {v} outside 3..{cap}"));
        }
    }

    // Skills.
    if !c.skills.is_empty() {
        for s in c.skills.iter().chain(c.background_skills.iter()) {
            if data::skill_ability(s).is_none() {
                e.push(format!("unknown skill `{s}`"));
            }
        }
        let granted = c.granted_skills();
        let chosen: Vec<&String> = c.skills.iter().filter(|s| !granted.contains(s)).collect();
        let mut seen = std::collections::BTreeSet::new();
        for s in &c.skills {
            if !seen.insert(s) {
                e.push(format!("skill `{s}` listed twice"));
            }
        }
        if let Some(cls) = cls {
            let extra = sp.map(|s| s.free_skills).unwrap_or(0);
            let allowed = cls.skill_count + extra;
            if chosen.len() > allowed {
                e.push(format!(
                    "{} chosen skills, allowed {} ({} class + {} species)",
                    chosen.len(),
                    allowed,
                    cls.skill_count,
                    extra
                ));
            } else if chosen.len() < allowed {
                w.push(format!(
                    "{} chosen skills, may pick {allowed}",
                    chosen.len()
                ));
            }
            if !cls.skills.is_empty() {
                let off: Vec<&&String> = chosen
                    .iter()
                    .filter(|s| !cls.skills.contains(&s.as_str()))
                    .collect();
                if off.len() > extra {
                    e.push(format!(
                        "skills not on the {} list: {}",
                        cls.id,
                        off.iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
    }

    // Equipment.
    if let Some(a) = c.armor.as_deref().filter(|a| *a != "none") {
        if !data::ARMOR.iter().any(|(id, _, _)| *id == a) {
            e.push(format!(
                "unknown armor `{a}` (use none or: {})",
                data::ARMOR
                    .iter()
                    .map(|x| x.0)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if c.level < 1 || c.level > 20 {
        e.push("level must be 1..20".into());
    }
    Report {
        errors: e,
        warnings: w,
    }
}

/// Six 4d6dl1 rolls through the shared combat-sim dice engine.
pub fn roll_pool(rng: &mut dyn DiceRng) -> Result<(Vec<i32>, Vec<String>)> {
    let mut v = Vec::new();
    let mut t = Vec::new();
    for _ in 0..6 {
        let o = roll_command("4d6dl1", RollMode::Normal, None, false, rng)?;
        v.push(o.total);
        t.push(o.text);
    }
    Ok((v, t))
}

/// `str=15,dex=14` or `str=15 dex=14`.
pub fn parse_abilities(args: &[String]) -> Result<BTreeMap<String, i32>> {
    let mut m = BTreeMap::new();
    for part in args
        .iter()
        .flat_map(|a| a.split(','))
        .filter(|s| !s.trim().is_empty())
    {
        let (k, v) = part
            .split_once('=')
            .with_context(|| format!("`{part}`: use ability=value"))?;
        let k = k.trim().to_lowercase();
        if !ABILITIES.contains(&k.as_str()) {
            bail!("unknown ability `{k}` (str dex con int wis cha)");
        }
        let v: i32 = v
            .trim()
            .trim_start_matches('+')
            .parse()
            .with_context(|| format!("`{part}`: not a number"))?;
        if m.insert(k.clone(), v).is_some() {
            bail!("{k} given twice");
        }
    }
    Ok(m)
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Character sheet Markdown with a combat yaml block built from the same numbers.
pub fn sheet(c: &Creation, player: &str) -> Result<(String, String)> {
    let name = c.name.clone().context("name not set")?;
    let cls = c.class.clone().context("class not set")?;
    let scores = c.final_scores();
    let mods = c.mods();
    let lvl = c.level.max(1);
    let prof = data::prof_bonus(lvl);
    let hp = c.hp().unwrap_or(1);
    let ac = c.ac();
    let sp = c.species.as_deref().and_then(|s| data::species(c.rs(), s));
    let speed = sp.map(|s| s.speed).unwrap_or(30);
    let size = sp.map(|s| s.size).unwrap_or("medium");
    let cdat = data::class(&cls);
    let saves: Vec<&str> = cdat.map(|x| x.saves.to_vec()).unwrap_or_default();
    let mut skills: Vec<String> = c.granted_skills();
    for s in &c.skills {
        if !skills.contains(s) {
            skills.push(s.clone());
        }
    }
    let id = slug(&name);
    let mut md = String::new();
    md += &format!("# {name}——角色卡\n\n");
    md += &format!(
        "- **玩家**：{player}\n- **规则**：{}\n- **冒险**：{}\n",
        c.rs(),
        c.adventure.as_deref().unwrap_or("-")
    );
    md += &format!(
        "- **等级**：{cls} {lvl}　**种族**：{}　**背景**：{}\n",
        c.species.as_deref().unwrap_or("-"),
        c.background.as_deref().unwrap_or("-")
    );
    if let Some(p) = &c.personality {
        md += &format!("- **性格**：{p}\n");
    }
    md += "\n## 属性\n\n| 属性 | 分数 | 调整值 |\n|------|------|--------|\n";
    for a in data::ABILITIES {
        md += &format!("| {a} | {} | {:+} |\n", scores[a], mods[a]);
    }
    md += &format!(
        "\n（生成方式：{}；基础 {}；加值 {}）\n",
        c.method.as_deref().unwrap_or("-"),
        data::ABILITIES
            .iter()
            .map(|a| c.scores.get(*a).copied().unwrap_or(0).to_string())
            .collect::<Vec<_>>()
            .join("/"),
        {
            let mut all = c.fixed_asi();
            for (k, v) in &c.asi {
                *all.entry(k.clone()).or_insert(0) += v;
            }
            let s: Vec<String> = all
                .iter()
                .filter(|(_, v)| **v != 0)
                .map(|(k, v)| format!("{k}{v:+}"))
                .collect();
            if s.is_empty() {
                "无".to_string()
            } else {
                s.join(" ")
            }
        }
    );
    md += &format!("\n## 战斗数据\n\n- **熟练加值**：+{prof}\n- **生命值**：{hp}\n- **护甲等级**：{ac}\n- **速度**：{speed} 尺\n- **先攻**：{:+}\n", mods["dex"]);
    md += &format!("- **豁免熟练**：{}\n", saves.join(", "));
    md += "\n## 技能（已含熟练加值）\n\n";
    for s in &skills {
        let a = data::skill_ability(s).unwrap_or("int");
        md += &format!("- {s} {:+}\n", mods[a] + prof);
    }
    md += "\n## 装备\n\n";
    md += &format!(
        "- 护甲：{}{}\n",
        c.armor.as_deref().unwrap_or("none"),
        if c.shield { " + 盾牌" } else { "" }
    );
    for e in &c.equipment {
        md += &format!("- {e}\n");
    }
    md += "\n## 法术\n\n";
    if c.spells.is_empty() {
        md += "- 无\n";
    }
    for s in &c.spells {
        md += &format!("- {s}\n");
    }

    let mut y = String::new();
    y += &format!("id: {id}\nname: {}\nkind: pc\nside: party\nsize: {size}\ncreature_type: humanoid\nlevel: {lvl}\n", yaml_str(&name));
    y += &format!(
        "ac: {ac}\nhp: {hp}\nspeed: {{walk: {speed}}}\ninit_bonus: {}\nprof: {prof}\n",
        mods["dex"]
    );
    y += &format!(
        "abilities: {{{}}}\n",
        data::ABILITIES
            .iter()
            .map(|a| format!("{a}: {}", scores[*a]))
            .collect::<Vec<_>>()
            .join(", ")
    );
    y += &format!(
        "saves: {{{}}}\n",
        saves
            .iter()
            .map(|a| format!("{a}: {}", mods[*a] + prof))
            .collect::<Vec<_>>()
            .join(", ")
    );
    y += &format!(
        "skills: {{{}}}\n",
        skills
            .iter()
            .map(|s| format!(
                "{s}: {}",
                mods[data::skill_ability(s).unwrap_or("int")] + prof
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(a) = cdat.and_then(|x| x.spell_ability) {
        y += &format!(
            "spellcasting: {{ability: {a}, attack: {}, dc: {}}}\n",
            mods[a] + prof,
            8 + mods[a] + prof
        );
    }
    y += &format!(
        "notes: {}\n",
        yaml_str(&format!(
            "{cls} {lvl}. Attacks/actions/resources: DM adds from the rulebook."
        ))
    );
    md += "\n## 战斗数据块（combat-sim）\n\n> 由 `talkdnd chargen sheet` 生成，与上文数值一致。攻击、动作、资源由 DM 按规则书补全；改角色卡时两处同步。\n\n```combat yaml\n";
    md += &y;
    md += "```\n";
    Ok((id, md))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(rs: &str) -> Creation {
        let mut c = Creation {
            ruleset: Some(rs.into()),
            species: Some(if rs == "2014" { "hill-dwarf" } else { "dwarf" }.into()),
            class: Some("cleric".into()),
            background: Some("acolyte".into()),
            method: Some("array".into()),
            pool: data::STANDARD_ARRAY.to_vec(),
            level: 1,
            asi_done: true,
            ..Default::default()
        };
        for (a, v) in ["wis", "con", "str", "cha", "dex", "int"]
            .iter()
            .zip(data::STANDARD_ARRAY)
        {
            c.scores.insert(a.to_string(), v);
        }
        c
    }

    #[test]
    fn steps_follow_order() {
        let c = Creation::default();
        assert_eq!(c.next_step().unwrap().0, "ruleset");
        let c = base("2014");
        assert_eq!(c.next_step().unwrap().0, "adventure");
    }

    #[test]
    fn dwarf_2014_gets_fixed_asi_and_hp() {
        let c = base("2014");
        let s = c.final_scores();
        assert_eq!(s["con"], 16);
        assert_eq!(s["wis"], 16);
        // d8 + con 3 + hill dwarf 1
        assert_eq!(c.hp(), Some(12));
        assert!(validate(&c).errors.is_empty());
    }

    #[test]
    fn assigned_scores_must_match_pool() {
        let mut c = base("2014");
        c.scores.insert("str".into(), 18);
        assert!(validate(&c).errors.iter().any(|e| e.contains("not the")));
    }

    #[test]
    fn point_buy_limits() {
        let mut c = base("2024");
        c.method = Some("pointbuy".into());
        c.asi = [("wis".to_string(), 2), ("int".to_string(), 1)].into();
        for (a, v) in [
            ("str", 15),
            ("dex", 15),
            ("con", 15),
            ("int", 8),
            ("wis", 8),
            ("cha", 8),
        ] {
            c.scores.insert(a.into(), v);
        }
        assert!(validate(&c).errors.is_empty());
        c.scores.insert("int".into(), 10);
        assert!(validate(&c).errors.iter().any(|e| e.contains("> 27")));
        c.scores.insert("int".into(), 16);
        assert!(validate(&c).errors.iter().any(|e| e.contains("8..15")));
    }

    #[test]
    fn background_asi_2024() {
        let mut c = base("2024");
        c.asi = [("wis".to_string(), 2), ("cha".to_string(), 1)].into();
        assert!(validate(&c).errors.is_empty());
        c.asi = [("str".to_string(), 2), ("cha".to_string(), 1)].into();
        assert!(validate(&c)
            .errors
            .iter()
            .any(|e| e.contains("cannot raise str")));
        c.asi = [("wis".to_string(), 3)].into();
        assert!(!validate(&c).errors.is_empty());
        c.asi = [
            ("int".to_string(), 1),
            ("wis".to_string(), 1),
            ("cha".to_string(), 1),
        ]
        .into();
        assert!(validate(&c).errors.is_empty());
    }

    #[test]
    fn half_elf_free_picks() {
        let mut c = base("2014");
        c.species = Some("half-elf".into());
        c.asi = [("cha".to_string(), 1), ("con".to_string(), 1)].into();
        assert!(validate(&c).errors.iter().any(|e| e.contains("cannot put")));
        c.asi = [("wis".to_string(), 1), ("con".to_string(), 1)].into();
        assert!(validate(&c).errors.is_empty());
    }

    #[test]
    fn skills_counted_against_class() {
        let mut c = base("2014");
        c.skills = vec![
            "insight".into(),
            "religion".into(),
            "medicine".into(),
            "history".into(),
        ];
        assert!(validate(&c).errors.is_empty());
        c.skills.push("persuasion".into());
        assert!(validate(&c).errors.iter().any(|e| e.contains("allowed")));
        c.skills = vec!["medicine".into(), "stealth".into()];
        assert!(validate(&c)
            .errors
            .iter()
            .any(|e| e.contains("not on the cleric list")));
    }

    #[test]
    fn non_srd_is_warning() {
        let mut c = base("2014");
        c.species = Some("aasimar".into());
        let r = validate(&c);
        assert!(r.errors.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("not in SRD")));
    }

    #[test]
    fn armor_class() {
        let mut c = base("2014");
        c.armor = Some("chain-mail".into());
        c.shield = true;
        assert_eq!(c.ac(), 18);
        c.armor = Some("scale-mail".into());
        c.scores.insert("dex".into(), 18);
        c.pool = vec![15, 18, 13, 12, 10, 8];
        assert_eq!(c.ac(), 14 + 2 + 2);
        c.class = Some("monk".into());
        c.armor = None;
        c.shield = false;
        assert_eq!(c.ac(), 10 + 4 + 3);
    }

    #[test]
    fn sheet_numbers_match() {
        let mut c = base("2014");
        c.name = Some("Bruni".into());
        c.skills = vec!["medicine".into(), "history".into()];
        let (id, md) = sheet(&c, "p").unwrap();
        assert_eq!(id, "bruni");
        assert!(md.contains("hp: 12\n"));
        assert!(md.contains("| con | 16 | +3 |"));
        assert!(md.contains("spellcasting: {ability: wis, attack: 5, dc: 13}"));
    }

    #[test]
    fn pool_is_six_4d6dl1() {
        let mut rng = combat_sim::dice::seeded_rng(7);
        let (v, t) = roll_pool(&mut rng).unwrap();
        assert_eq!(v.len(), 6);
        assert!(v.iter().all(|x| (3..=18).contains(x)));
        assert!(t.iter().all(|x| x.starts_with("4d6dl1(")));
    }
}
