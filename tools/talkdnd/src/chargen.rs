//! Character creation state, rolls and rule validation.

use crate::data::{self, ABILITIES};
use anyhow::{bail, Context, Result};
use combat_sim::content::loader::Ruleset;
use combat_sim::dice::DiceRng;
use combat_sim::dice::RollMode;
use combat_sim::fsutil::atomic_write;
use combat_sim::roll::roll_command;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Steps in the order the DM must walk the player through.
pub const STEPS: &[(&str, &str)] = &[
    (
        "ruleset",
        "规则集 5.5e FULL / 5e FULL / 5.5e SRD / 5e SRD",
    ),
    ("adventure", "冒险"),
    ("species", "种族/物种"),
    ("class", "职业"),
    ("level", "起始等级（按冒险建议范围）"),
    ("subclass", "子职（达到子职等级时）"),
    (
        "fighting-style",
        "战斗风格（2014 战士 1 级、圣武士/游侠 2 级；2024 为战斗风格专长）",
    ),
    ("background", "背景"),
    (
        "method",
        "属性生成方式：roll（4d6 去最低）/ array（标准数组）/ pointbuy（购点）",
    ),
    ("scores", "玩家把数值分配到六项属性"),
    ("asi", "属性加值（2014 种族 / 2024 背景）"),
    ("skills", "技能"),
    (
        "proficiencies",
        "语言与工具（背景/种族给的自选项：`set languages` / `set tools`）",
    ),
    (
        "equipment",
        "起始装备：`set class-equipment`（2014 每组选 a/b/c 或 gold；2024 选 A/B/C）+ 背景装备（2024 `set background-equipment A|B`）",
    ),
    ("spells", "法术（非施法职业写 none）"),
    ("name", "名字"),
    (
        "traits",
        "性格特点、理想、牵绊、缺点（`set traits/ideals/bonds/flaws`）与阵营",
    ),
    ("confirm", "确认角色卡"),
];

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Creation {
    pub ruleset: Option<String>,
    pub adventure: Option<String>,
    pub species: Option<String>,
    pub class: Option<String>,
    /// Subclass id (SRD or local data); needed once the class level reaches it.
    #[serde(default)]
    pub subclass: Option<String>,
    /// 2024 Weapon Mastery choices (weapon ids).
    #[serde(default)]
    pub masteries: Vec<String>,
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
    /// Personality traits.
    pub personality: Option<String>,
    #[serde(default)]
    pub ideals: Option<String>,
    #[serde(default)]
    pub bonds: Option<String>,
    #[serde(default)]
    pub flaws: Option<String>,
    #[serde(default)]
    pub alignment: Option<String>,
    /// Fighting Style id (2014 class feature / 2024 feat).
    #[serde(default)]
    pub fighting_style: Option<String>,
    #[serde(default)]
    pub feats: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub proficiencies_done: bool,
    /// Class starting-equipment choice (2014 `a,b,a` or `gold`; 2024 `A`).
    #[serde(default)]
    pub class_equipment: Option<String>,
    /// Items from that choice (resolved when chosen).
    #[serde(default)]
    pub class_items: Vec<String>,
    /// Gold from the class choice (gp), e.g. a 2014 starting-gold roll.
    #[serde(default)]
    pub class_gold: i64,
    /// 2024 background equipment option (A/B).
    #[serde(default)]
    pub background_equipment: Option<String>,
    /// Starting gold already put into the purse (`chargen confirm`).
    #[serde(default)]
    pub gold_granted: bool,
    #[serde(default = "one")]
    pub level: i32,
    /// Whether the player has explicitly chosen the starting level.
    #[serde(default)]
    pub level_done: bool,
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
    let mut c: Creation =
        serde_json::from_str(&fs::read_to_string(&p)?).context("chargen.json is not valid")?;
    // Old creation files may still hold `2014` / `2024`; show the canonical
    // ruleset value from now on. The next store writes it back.
    if let Some(value) = &c.ruleset {
        if let Ok(ruleset) = Ruleset::parse(value) {
            c.ruleset = Some(ruleset.value().to_string());
        }
    }
    Ok(c)
}

pub fn store(save: &Path, c: &Creation) -> Result<()> {
    atomic_write(
        state_path(save),
        (serde_json::to_string_pretty(c)? + "\n").as_bytes(),
    )?;
    Ok(())
}

impl Creation {
    /// Engine edition for rules data. Accepts both the canonical ruleset
    /// values and the old `2014` / `2024` values.
    pub fn rs(&self) -> &str {
        match self.ruleset.as_deref().map(str::trim) {
            Some(value)
                if value.eq_ignore_ascii_case("5e-srd")
                    || value.eq_ignore_ascii_case("5e-full")
                    || value.eq_ignore_ascii_case("2014") =>
            {
                "2014"
            }
            _ => "2024",
        }
    }

    pub fn done(&self, step: &str) -> bool {
        match step {
            "ruleset" => self.ruleset.is_some(),
            "adventure" => self.adventure.is_some(),
            "species" => self.species.is_some(),
            "class" => self.class.is_some(),
            "level" => self.level_done,
            "subclass" => self.subclass.is_some() || !self.subclass_needed(),
            "fighting-style" => self.fighting_style.is_some() || self.style_level().is_none(),
            "proficiencies" => self.proficiencies_done,
            "traits" => {
                self.personality.is_some()
                    && self.ideals.is_some()
                    && self.bonds.is_some()
                    && self.flaws.is_some()
            }
            "background" => self.background.is_some(),
            "method" => {
                self.method.is_some()
                    && (self.method.as_deref() == Some("pointbuy") || !self.pool.is_empty())
            }
            "scores" => self.scores.len() == 6,
            "asi" => self.asi_done,
            "skills" => !self.skills.is_empty(),
            "equipment" => {
                (self.class_equipment.is_some()
                    || self.armor.is_some()
                    || !self.equipment.is_empty())
                    && (self.background_equipment.is_some()
                        || self
                            .background_def(None)
                            .is_none_or(|b| b.options.is_empty()))
            }
            "spells" => self.spells_done,
            "name" => self.name.is_some(),
            "confirm" => self.confirmed,
            _ => false,
        }
    }

    /// Next unfinished step and its prompt. The level step shows the chosen
    /// adventure's suggested start range.
    pub fn next_step(&self) -> Option<(&'static str, String)> {
        let (step, label) = STEPS.iter().copied().find(|(s, _)| !self.done(s))?;
        let label = if step == "level" {
            self.level_prompt()
        } else {
            (*label).to_string()
        };
        Some((step, label))
    }

    /// Level at which the class gains a Fighting Style, if it is needed now.
    pub fn style_level(&self) -> Option<i32> {
        let (l, _) = crate::options::style_for_class(self.rs(), self.class.as_deref()?)?;
        (self.level >= l).then_some(l)
    }

    /// Background data: SRD, else the local book import.
    pub fn background_def(
        &self,
        book: Option<&crate::classdata::Book>,
    ) -> Option<crate::options::BackgroundDef> {
        let b = self.background.as_deref()?;
        crate::options::srd_background(self.rs(), b)
            .cloned()
            .or_else(|| book.and_then(|bk| bk.backgrounds.get(b).cloned()))
    }

    /// Background equipment and gold (empty when the background has no data).
    pub fn background_pack(
        &self,
        book: Option<&crate::classdata::Book>,
    ) -> crate::options::Resolved {
        self.background_def(book)
            .and_then(|b| {
                crate::options::background_equipment(&b, self.background_equipment.as_deref()).ok()
            })
            .unwrap_or_default()
    }

    /// Every starting item: class choice, background, then items the player named.
    pub fn all_items(&self, book: Option<&crate::classdata::Book>) -> Vec<String> {
        let mut v = self.class_items.clone();
        v.extend(self.background_pack(book).items);
        v.extend(self.equipment.iter().cloned());
        v
    }

    /// Body armor worn: the `armor` choice, else the first armor item.
    pub fn worn_armor(&self) -> Option<String> {
        match self.armor.as_deref() {
            Some(a) if a != "none" => Some(a.to_string()),
            Some(_) => None,
            None => self
                .all_items(None)
                .iter()
                .find_map(|i| crate::charsheet::armor_of(i).map(|a| a.0.to_string())),
        }
    }

    pub fn has_shield(&self) -> bool {
        self.shield
            || self
                .all_items(None)
                .iter()
                .any(|i| crate::charsheet::is_shield(i))
    }

    /// Slug of the chosen adventure (used for the start-level data lookup).
    pub fn adventure_slug(&self) -> Option<String> {
        self.adventure.as_deref().map(slug)
    }

    /// Level at which the chosen class grants its subclass (from class data).
    pub fn subclass_level(&self) -> Option<i32> {
        let cid = self.class.as_deref()?;
        let book = crate::classdata::Book::srd(self.rs());
        book.class(cid).map(|c| c.subclass_level.unwrap_or(3))
    }

    /// True when the chosen class needs a subclass at the current level.
    pub fn subclass_needed(&self) -> bool {
        self.subclass_level().is_some_and(|sl| self.level >= sl)
    }

    /// Prompt for the level step, including the adventure's suggested range.
    pub fn level_prompt(&self) -> String {
        match self
            .adventure_slug()
            .as_deref()
            .and_then(crate::classdata::adventure)
        {
            Some(a) => format!(
                "起始等级 {}-{}{}",
                a.start_level_min,
                a.start_level_max,
                a.note
                    .as_deref()
                    .map(|n| format!("（{n}）"))
                    .unwrap_or_default()
            ),
            None => "起始等级 1..20".to_string(),
        }
    }

    /// Armor and shield proficiencies from the class plus the chosen subclass.
    pub fn armor_proficiency(&self) -> Vec<String> {
        let book = crate::classdata::Book::srd(self.rs());
        let mut v: Vec<String> = self
            .class
            .as_deref()
            .map(data::armor)
            .unwrap_or(&[])
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let (Some(cid), Some(sid)) = (self.class.as_deref(), self.subclass.as_deref()) {
            if let Some(sub) = book.class(cid).and_then(|c| c.subclasses.get(&slug(sid))) {
                v.extend(sub.armor_proficiency.iter().cloned());
            }
        }
        v
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
        let defense =
            self.fighting_style.as_deref() == Some("defense") && self.worn_armor().is_some();
        self.ac_unstyled() + i32::from(defense)
    }

    /// AC without a worn-armor-free override; `ac_unarmored` is the no-armor value.
    pub fn ac_unarmored(&self) -> i32 {
        let mut c = self.clone();
        c.armor = Some("none".into());
        c.shield = false;
        c.class_items.clear();
        c.equipment.clear();
        c.background = None;
        c.ac_unstyled()
    }

    fn ac_unstyled(&self) -> i32 {
        let m = self.mods();
        let dex = m["dex"];
        let shield = if self.has_shield() { 2 } else { 0 };
        let worn = self.worn_armor();
        match worn.as_deref() {
            Some(a) => match data::ARMOR.iter().find(|(id, _, _)| *id == a) {
                Some((_, base, cap)) => base + cap.map_or(dex, |c| dex.min(c)) + shield,
                None => 10 + dex + shield,
            },
            None => match self.class.as_deref() {
                Some("barbarian") => 10 + dex + m["con"] + shield,
                Some("monk") if !self.has_shield() => 10 + dex + m["wis"],
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
    if let Err(error) = Ruleset::parse(c.ruleset.as_deref().unwrap_or("5.5e-srd")) {
        e.push(error.to_string());
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
        // A chosen skill already granted by the background or species is a
        // duplicate pick: say so explicitly.
        let bg_skills: Vec<String> = c
            .background
            .as_deref()
            .and_then(|b| data::background(&rs, b))
            .map(|b| b.skills.iter().map(|s| s.to_string()).collect())
            .unwrap_or_else(|| c.background_skills.clone());
        let sp_skills: Vec<String> = sp
            .map(|s| s.skills.iter().map(|x| x.to_string()).collect())
            .unwrap_or_default();
        for s in &c.skills {
            if bg_skills.contains(s) {
                w.push(format!(
                    "skill `{s}` is already granted by the background; pick another"
                ));
            } else if sp_skills.contains(s) {
                w.push(format!(
                    "skill `{s}` is already granted by the species; pick another"
                ));
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

    // Level: 1..20 and inside the adventure's suggested start range.
    if c.level < 1 || c.level > 20 {
        e.push("level must be 1..20".into());
    } else if let Some(slug) = c.adventure_slug() {
        match crate::classdata::adventure(&slug) {
            Some(a) => {
                if c.level < a.start_level_min || c.level > a.start_level_max {
                    e.push(format!(
                        "level {} is outside the suggested start range {}-{} for {}",
                        c.level,
                        a.start_level_min,
                        a.start_level_max,
                        c.adventure.as_deref().unwrap_or(&slug)
                    ));
                }
            }
            None => {
                if let Some(adv) = c.adventure.as_deref() {
                    w.push(format!(
                        "adventure `{adv}` has no start-level data; confirm the level with the book"
                    ));
                }
            }
        }
    }

    // Subclass: required once the class level reaches its subclass level.
    if c.subclass.is_none() && c.subclass_needed() {
        let sl = c.subclass_level().unwrap_or(3);
        e.push(format!(
            "level {} {} needs a subclass (gained at level {}): `talkdnd chargen set subclass <id>`",
            c.level,
            c.class.as_deref().unwrap_or(""),
            sl
        ));
    }

    // Armor and shield proficiency (SRD classes; a subclass can add more).
    if data::class(c.class.as_deref().unwrap_or("")).is_some() {
        let prof = c.armor_proficiency();
        if let Some(a) = c.armor.as_deref().filter(|a| *a != "none") {
            if let Some(cat) = data::armor_category(a) {
                if !prof.iter().any(|p| p == cat || p == "all") {
                    e.push(format!(
                        "{} is not proficient with {} armor (`{a}`)",
                        c.class.as_deref().unwrap_or(""),
                        cat
                    ));
                }
            }
        }
        if c.shield && !prof.iter().any(|p| p == "shield" || p == "all") {
            e.push(format!(
                "{} is not proficient with shields",
                c.class.as_deref().unwrap_or("")
            ));
        }
    }

    // Fighting Style.
    if let Some(cl) = c.class.as_deref() {
        match (crate::options::style_for_class(&rs, cl), c.fighting_style.as_deref()) {
            (Some((l, _)), None) if c.level >= l => e.push(format!(
                "{cl} {} needs a Fighting Style (gained at level {l}): `talkdnd chargen set fighting-style <id>`",
                c.level
            )),
            (Some((_, allowed)), Some(fs)) if !allowed.iter().any(|a| a == fs) => w.push(format!(
                "fighting style `{fs}` is not on the {rs} {cl} list ({}); allowed only if the player's book offers it",
                allowed.join(", ")
            )),
            (None, Some(fs)) => w.push(format!("{cl} has no Fighting Style feature; `{fs}` needs a feat or a DM ruling")),
            _ => {}
        }
    }
    if let Some(fs) = c.fighting_style.as_deref() {
        if crate::options::style(fs).is_none() {
            w.push(format!(
                "fighting style `{fs}` is not in the data; the DM rules its effect"
            ));
        }
    }
    for it in c.all_items(None) {
        if crate::options::is_placeholder(&it) {
            w.push(format!(
                "starting equipment `{it}`: the player names the item (`chargen set equipment …`)"
            ));
        }
    }

    // Spell counts against class resources at the chosen level.
    if c.spells_done {
        let g = crate::classdata::generate(c, &crate::classdata::Book::srd(&rs), None);
        let cantrips = c.spells.iter().filter(|s| data::is_cantrip(s)).count() as i32;
        let leveled = c.spells.len() as i32 - cantrips;
        if let Some(exp) = g.cantrips {
            if cantrips > exp {
                e.push(format!("too many cantrips: {cantrips} (allowed {exp})"));
            } else if cantrips < exp {
                w.push(format!("cantrips: {cantrips} of {exp}"));
            }
        }
        if let Some((kind, exp)) = g.spells {
            if leveled > exp {
                e.push(format!("too many {kind}: {leveled} (allowed {exp})"));
            } else if leveled < exp {
                w.push(format!("{kind}: {leveled} of {exp}"));
            }
        }
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
        if matches!(ch, '\'' | '\u{2019}') {
            // "King's" -> "kings", matching the adventure folder names.
            continue;
        }
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// A safe id for file names and references: lowercase ASCII letters, digits
/// and hyphens, starting with a letter or digit.
pub fn valid_id(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    s.len() <= 80
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Character sheet Markdown with a combat yaml block built from the same numbers.
pub fn sheet(c: &Creation, player: &str) -> Result<(String, String)> {
    sheet_with(c, player, &crate::classdata::Book::srd(c.rs()), None)
}

/// Sheet using SRD plus local data; `root` is the LetsPlay folder (for import hints).
pub fn sheet_with(
    c: &Creation,
    player: &str,
    book: &crate::classdata::Book,
    root: Option<&Path>,
) -> Result<(String, String)> {
    let g = crate::classdata::generate(c, book, root);
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
        "- **等级**：{cls}{} {lvl}　**种族**：{}　**背景**：{}\n",
        c.subclass
            .as_deref()
            .map(|s| format!("（{s}）"))
            .unwrap_or_default(),
        c.species.as_deref().unwrap_or("-"),
        c.background.as_deref().unwrap_or("-")
    );
    if let Some(p) = &c.personality {
        md += &format!("- **性格**：{p}\n");
    }
    if let Some(fs) = &c.fighting_style {
        let st = crate::options::style(fs);
        md += &format!(
            "- **战斗风格**：{}{}\n",
            st.map(|s| format!("{}（{}）", s.zh, s.name))
                .unwrap_or(fs.clone()),
            st.map(|s| format!("——{}", s.effect)).unwrap_or_default()
        );
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
    md += &species_traits_md(sp);
    md += &format!("\n## 战斗数据\n\n- **熟练加值**：+{prof}\n- **生命值**：{hp}\n- **护甲等级**：{ac}\n- **速度**：{speed} 尺\n- **先攻**：{:+}\n", mods["dex"]);
    md += &format!("- **豁免熟练**：{}\n", saves.join(", "));
    md += "\n## 技能（已含熟练加值）\n\n";
    for s in &skills {
        let a = data::skill_ability(s).unwrap_or("int");
        md += &format!("- {s} {:+}\n", mods[a] + prof);
    }
    md += &gen_md(&g, prof);
    md += "\n## 装备\n\n";
    md += &format!(
        "- 护甲：{}{}\n",
        c.armor.as_deref().unwrap_or("none"),
        if c.shield { " + 盾牌" } else { "" }
    );
    for e in c.all_items(Some(book)) {
        md += &format!("- {e}\n");
    }
    let gold = c.class_gold + c.background_pack(Some(book)).gold;
    if gold > 0 {
        md += &format!("- 起始金币：{gold} gp（`chargen confirm` 时记入钱袋）\n");
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
    if sp.is_some_and(|s| s.darkvision > 0) {
        y += &format!("senses: \"Darkvision {} 尺\"\n", sp.unwrap().darkvision);
    }
    if let Some(s) = sp.filter(|s| !s.resistances.is_empty()) {
        y += &format!("resistances: [{}]\n", s.resistances.join(", "));
    }
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
    y += &gen_yaml(&g);
    y += &format!(
        "notes: {}\n",
        yaml_str(&format!(
            "{cls} {lvl}. Generated by talkdnd from SRD/local data; features without a combat ref are handled by the DM."
        ))
    );
    md += "\n## 战斗数据块（combat-sim）\n\n> 由 `talkdnd chargen sheet` 生成，与上文数值一致。攻击、资源与职业特性由程序按 SRD/本地数据生成；没有 combat-sim 引用的特性由 DM 裁定。改角色卡时重新运行 `chargen sheet`。\n\n```combat yaml\n";
    md += &y;
    md += "```\n";
    Ok((id, md))
}

/// Species traits section for the sheet (senses, speed, resistances, traits).
fn species_traits_md(sp: Option<&data::Species>) -> String {
    let Some(sp) = sp else {
        return String::new();
    };
    let mut md = String::new();
    md += "\n## 种族特性\n\n";
    md += &format!(
        "- **视觉**：{}\n",
        if sp.darkvision > 0 {
            format!("黑暗视觉 {} 尺", sp.darkvision)
        } else {
            "无黑暗视觉".to_string()
        }
    );
    md += &format!("- **速度**：{} 尺\n", sp.speed);
    md += &format!(
        "- **伤害抗性**：{}\n",
        if sp.resistances.is_empty() {
            "无".to_string()
        } else {
            sp.resistances.join("、")
        }
    );
    for (name, summary) in sp.traits {
        md += &format!("- **{name}**：{summary}\n");
    }
    md
}

fn gen_md(g: &crate::classdata::Generated, _prof: i32) -> String {
    let mut md = String::new();
    md += "\n## 攻击\n\n| 攻击 | 命中 | 伤害 | 射程/触及 | 备注 |\n|------|------|------|-----------|------|\n";
    for a in &g.attacks {
        let reach = match (a.reach, a.range) {
            (Some(r), Some([n, l])) => format!("{r} 尺；投掷 {n}/{l}"),
            (Some(r), None) => format!("{r} 尺"),
            (None, Some([n, l])) => format!("{n}/{l}"),
            _ => "-".into(),
        };
        let mut note = a.properties.join(", ");
        if let Some(m) = &a.mastery {
            note += &format!("{}精通 {m}", if note.is_empty() { "" } else { "；" });
        }
        if !a.note.is_empty() {
            note += &format!("{}{}", if note.is_empty() { "" } else { "；" }, a.note);
        }
        md += &format!(
            "| {} | {:+} | {} {} | {reach} | {} |\n",
            a.name,
            a.to_hit,
            a.dice,
            a.dtype,
            if note.is_empty() { "-".into() } else { note }
        );
    }
    if let Some(n) = g.mastery {
        md += &format!("\n武器精通：可选 {n} 种（`chargen set masteries`）。\n");
    }
    md += "\n## 资源\n\n";
    for r in &g.resources {
        let rc = match r.recharge.as_str() {
            "short" => "短休恢复",
            "long" => "长休恢复",
            o => o,
        };
        let sr = r
            .short_regain
            .map(|n| format!("，短休恢复 {n}"))
            .unwrap_or_default();
        md += &format!(
            "- {}：{}（{rc}{sr}）",
            r.name,
            if r.max >= 99 {
                "无限".to_string()
            } else {
                r.max.to_string()
            }
        );
        if let Some(n) = &r.notes {
            md += &format!("　{n}");
        }
        md += "\n";
    }
    if !g.slots.is_empty() {
        let s: Vec<String> = g
            .slots
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{}环×{n}", i + 1))
            .collect();
        md += &format!("- 法术位：{}（长休恢复）\n", s.join("　"));
    }
    if let Some((n, l)) = g.pact {
        md += &format!("- 契约法术位：{n} 个 {l} 环（短休恢复）\n");
    }
    if let Some(n) = g.cantrips {
        md += &format!("- 戏法：{n}\n");
    }
    if let Some((k, n)) = g.spells {
        md += &format!("- {k}：{n}\n");
    }
    if !g.features.is_empty() {
        md += "\n## 职业特性\n\n";
        for (l, n, s) in &g.features {
            md += &format!("- {l} 级 **{n}**：{s}\n");
        }
    }
    if !g.warnings.is_empty() {
        md += "\n## 待补（DM）\n\n";
        for w in &g.warnings {
            md += &format!("- {w}\n");
        }
    }
    md
}

fn gen_yaml(g: &crate::classdata::Generated) -> String {
    let mut y = String::new();
    let mut res: Vec<String> = Vec::new();
    for r in &g.resources {
        let mut e = format!("{{max: {}, recharge: {}", r.max, r.recharge);
        if let Some(n) = r.short_regain {
            e += &format!(", short_rest_regain: {n}");
        }
        if r.id == "hit-dice" {
            if let Some(d) = r
                .name
                .split("(d")
                .nth(1)
                .and_then(|x| x.trim_end_matches(')').parse::<u32>().ok())
            {
                e += &format!(", die: {d}");
            }
        }
        e += "}";
        res.push(format!("  {}: {e}\n", r.id));
    }
    for (i, n) in g.slots.iter().enumerate() {
        res.push(format!(
            "  spell-slot-{}: {{max: {n}, recharge: long}}\n",
            i + 1
        ));
    }
    if let Some((n, l)) = g.pact {
        res.push(format!(
            "  spell-slot-{l}: {{max: {n}, recharge: short, notes: \"pact magic\"}}\n"
        ));
    }
    if !res.is_empty() {
        y += "resources:\n";
        for r in res {
            y += &r;
        }
    }
    y += "attacks:\n";
    for a in &g.attacks {
        y += &format!(
            "  - {{id: {}, name: {}, kind: {}, ability: {}, to_hit: {}, damage: [{{dice: \"{}\", type: {}}}]",
            a.id,
            yaml_str(&a.name),
            a.kind,
            a.ability,
            a.to_hit,
            a.dice,
            a.dtype
        );
        if let Some(r) = a.reach {
            y += &format!(", reach: {r}");
        }
        if let Some([n, l]) = a.range {
            y += &format!(", range: [{n}, {l}]");
        }
        if !a.properties.is_empty() {
            y += &format!(", properties: [{}]", a.properties.join(", "));
        }
        if let Some(m) = &a.mastery {
            y += &format!(", mastery: {m}");
        }
        y += "}\n";
    }
    let acts: Vec<_> = g.refs.iter().filter(|r| r.2).collect();
    let feats: Vec<_> = g.refs.iter().filter(|r| !r.2).collect();
    for (key, list) in [("actions", acts), ("features", feats)] {
        if list.is_empty() {
            continue;
        }
        y += &format!("{key}:\n");
        for (r, p, _) in list {
            match p {
                Some(p) => y += &format!("  - {{ref: {r}, params: {p}}}\n"),
                None => y += &format!("  - {{ref: {r}}}\n"),
            }
        }
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(rs: &str) -> Creation {
        let mut c = Creation {
            ruleset: Some(rs.into()),
            species: Some(if rs == "2014" { "hill-dwarf" } else { "dwarf" }.into()),
            class: Some("cleric".into()),
            subclass: Some("life".into()),
            background: Some("acolyte".into()),
            method: Some("array".into()),
            pool: data::STANDARD_ARRAY.to_vec(),
            level: 1,
            level_done: true,
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
    fn ids_are_path_safe() {
        assert_eq!(slug("../../evil"), "evil");
        assert_eq!(slug("Aria"), "aria");
        assert!(valid_id("evil"));
        assert!(valid_id("a-1"));
        assert!(!valid_id("../evil"));
        assert!(!valid_id("Aria"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(""));
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
    fn duplicate_granted_skill_is_warning() {
        let mut c = base("2014");
        // Acolyte grants insight + religion.
        c.skills = vec!["insight".into(), "medicine".into()];
        let r = validate(&c);
        assert!(r.errors.is_empty());
        assert!(r
            .warnings
            .iter()
            .any(|w| w.contains("already granted by the background")));
        // High elf grants perception.
        c.species = Some("high-elf".into());
        c.skills = vec!["perception".into(), "medicine".into()];
        let r = validate(&c);
        assert!(r.errors.is_empty());
        assert!(r
            .warnings
            .iter()
            .any(|w| w.contains("already granted by the species")));
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

    #[test]
    fn level_step_uses_adventure_range() {
        let mut c = base("2014");
        c.adventure = Some("Keys from the Golden Vault".into());
        assert!(c.level_prompt().contains("1-11"), "{}", c.level_prompt());
        c.level = 12;
        assert!(validate(&c).errors.iter().any(|e| e.contains("outside")));
        c.level = 1;
        assert!(validate(&c).errors.is_empty());
    }

    #[test]
    fn unknown_adventure_is_warning() {
        let mut c = base("2014");
        c.adventure = Some("Homebrew Adventure".into());
        c.level = 7;
        let r = validate(&c);
        assert!(r.errors.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("no start-level data")));
    }

    #[test]
    fn level_out_of_bounds() {
        let mut c = base("2014");
        c.level = 21;
        assert!(validate(&c).errors.iter().any(|e| e.contains("1..20")));
        c.level = 0;
        assert!(validate(&c).errors.iter().any(|e| e.contains("1..20")));
    }

    #[test]
    fn subclass_step_and_requirement() {
        let mut c = base("2014");
        c.adventure = Some("Storm King's Thunder".into());
        c.subclass = None;
        c.level_done = false;
        assert_eq!(c.next_step().unwrap().0, "level");
        c.level_done = true;
        assert_eq!(c.next_step().unwrap().0, "subclass");
        assert!(validate(&c)
            .errors
            .iter()
            .any(|e| e.contains("needs a subclass")));
        c.subclass = Some("life".into());
        assert!(validate(&c).errors.is_empty());
        // Other classes only need a subclass at level 3.
        let mut f = base("2024");
        f.class = Some("fighter".into());
        f.subclass = None;
        f.level = 1;
        assert!(!f.subclass_needed());
        f.level = 3;
        assert!(f.subclass_needed());
        assert!(validate(&f)
            .errors
            .iter()
            .any(|e| e.contains("needs a subclass")));
    }

    #[test]
    fn armor_proficiency_from_subclass() {
        // 2014 Life domain grants heavy armor.
        let mut c = base("2014");
        c.armor = Some("chain-mail".into());
        assert!(validate(&c).errors.is_empty(), "{:?}", validate(&c).errors);
        // Rogue has no heavy armor.
        let mut r = base("2014");
        r.class = Some("rogue".into());
        r.subclass = None;
        r.armor = Some("chain-mail".into());
        assert!(validate(&r)
            .errors
            .iter()
            .any(|e| e.contains("not proficient with heavy")));
        // Wizard has no shield.
        let mut m = base("2014");
        m.class = Some("wizard".into());
        m.subclass = None;
        m.shield = true;
        assert!(validate(&m)
            .errors
            .iter()
            .any(|e| e.contains("not proficient with shields")));
    }

    #[test]
    fn spell_counts_against_class() {
        // 2014 cleric: 3 cantrips, prepared = wis mod (+3) + level (1) = 4.
        let mut c = base("2014");
        c.spells = vec![
            "sacred-flame".into(),
            "guidance".into(),
            "resistance".into(),
            "cure-wounds".into(),
            "shield-of-faith".into(),
            "spiritual-weapon".into(),
            "bless".into(),
        ];
        c.spells_done = true;
        assert!(validate(&c).errors.is_empty(), "{:?}", validate(&c).errors);
        c.spells.push("mending".into());
        assert!(validate(&c)
            .errors
            .iter()
            .any(|e| e.contains("too many cantrips")));
        c.spells = vec![
            "sacred-flame".into(),
            "guidance".into(),
            "resistance".into(),
            "cure-wounds".into(),
        ];
        assert!(validate(&c)
            .warnings
            .iter()
            .any(|w| w.contains("可准备法术")));
        c.spells = vec![
            "sacred-flame".into(),
            "guidance".into(),
            "resistance".into(),
            "cure-wounds".into(),
            "bless".into(),
            "sanctuary".into(),
            "shield-of-faith".into(),
            "spiritual-weapon".into(),
        ];
        assert!(validate(&c)
            .errors
            .iter()
            .any(|e| e.contains("too many 可准备法术")));
    }

    #[test]
    fn species_traits_in_sheet() {
        let mut c = base("2014"); // hill dwarf
        c.name = Some("Bruni".into());
        let (id, md) = sheet(&c, "p").unwrap();
        assert_eq!(id, "bruni");
        assert!(md.contains("## 种族特性"));
        assert!(md.contains("黑暗视觉 60 尺"));
        assert!(md.contains("**伤害抗性**：poison"));
        assert!(md.contains("senses: \"Darkvision 60 尺\""));
        assert!(md.contains("resistances: [poison]"));
    }
}
