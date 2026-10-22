//! Structured character data: `Saves/<slot>/sheets/<id>.yaml` is the source of
//! truth for build options, inventory, inspiration, exhaustion and notes; live
//! combat values (HP, temp HP, resources, conditions, death saves) live in
//! `combat/party-state.yaml`, shared with combat-sim. The Markdown sheet
//! (`Characters/<Name>.md`) is rendered from both and never edited by hand.

use crate::classdata::{self, Book};
use crate::{check, data, options, save};
use anyhow::{anyhow, bail, Context, Result};
use combat_sim::content::schema::ResourceDef;
use combat_sim::dice::DiceRng;
use combat_sim::fsutil::atomic_write;
use combat_sim::party::{Member, PartyState};
use combat_sim::state::ResourceState;
use serde::{Deserialize, Serialize};
use serde_norway::{Mapping, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const BEGIN: &str =
    "<!-- talkdnd:char 以下由 `talkdnd char` 生成，不要手改；改动一律用 `talkdnd char <名字> …` -->";
pub const END: &str = "<!-- /talkdnd:char -->";

fn one() -> i32 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub name: String,
    #[serde(default = "one")]
    pub qty: i32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub equipped: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub attuned: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ammo: bool,
    /// Magic bonus to attack and damage (weapons).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub bonus: i32,
    /// Extra AC while equipped (shields, rings, magic armor).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ac_bonus: i32,
    /// Original-content id (`O-001`) when this is a registered original item.
    /// DM-only: never rendered onto the player-facing sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CharFile {
    pub version: u32,
    pub id: String,
    pub name: String,
    /// Markdown sheet path relative to the save (e.g. `Characters/Yier.md`).
    pub file: String,
    pub ruleset: String,
    /// Free sheet fields: class, species, background, alignment, traits,
    /// ideals, bonds, flaws, appearance, backstory, allies, player.
    #[serde(default)]
    pub info: BTreeMap<String, String>,
    #[serde(default)]
    pub fighting_style: Option<String>,
    #[serde(default)]
    pub feats: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    /// Extra features & traits lines (background feature, feats, ...).
    #[serde(default)]
    pub features: Vec<String>,
    /// Weapon proficiencies: simple, martial or weapon ids.
    #[serde(default)]
    pub weapon_prof: Vec<String>,
    #[serde(default)]
    pub masteries: Vec<String>,
    #[serde(default)]
    pub inventory: Vec<Item>,
    /// AC without armor items (unarmored formula or a migrated sheet's AC).
    #[serde(default)]
    pub ac_base: i32,
    /// The migrated AC already includes worn armor (Defense applies).
    #[serde(default)]
    pub ac_base_armored: bool,
    /// The migrated AC already includes any fighting-style bonus (Defense
    /// is not added again on top of `ac_base`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ac_base_has_style: bool,
    #[serde(default)]
    pub inspiration: bool,
    #[serde(default)]
    pub exhaustion: i32,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Freeform DM notes for what the programs cannot model.
    #[serde(default)]
    pub dm_notes: String,
    /// Prose part of the sheet (identity, scores, skills, features).
    #[serde(default)]
    pub body_md: String,
    /// Text after the generated part (level-up records).
    #[serde(default)]
    pub tail_md: String,
    /// The combat-sim block (single copy; rendered into the sheet).
    #[serde(default)]
    pub combat: Value,
}

pub fn path(save: &Path, id: &str) -> PathBuf {
    save.join("sheets").join(format!("{id}.yaml"))
}

fn party_path(save: &Path) -> PathBuf {
    save.join("combat").join(combat_sim::party::FILE_NAME)
}

fn get_i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_i64())
}

impl CharFile {
    pub fn score(&self, a: &str) -> i32 {
        self.combat
            .get("abilities")
            .and_then(|x| x.get(a))
            .and_then(|v| v.as_i64())
            .unwrap_or(10) as i32
    }
    pub fn m(&self, a: &str) -> i32 {
        data::modifier(self.score(a))
    }
    pub fn prof(&self) -> i32 {
        get_i(&self.combat, "prof").unwrap_or(2) as i32
    }
    pub fn level(&self) -> i32 {
        get_i(&self.combat, "level").unwrap_or(1) as i32
    }
    pub fn max_hp(&self) -> i32 {
        get_i(&self.combat, "hp").unwrap_or(1) as i32
    }
    fn set(&mut self, k: &str, v: Value) {
        if let Value::Mapping(m) = &mut self.combat {
            m.insert(Value::String(k.into()), v);
        }
    }
    pub fn class(&self) -> Option<String> {
        self.info.get("class").cloned()
    }
    fn find_item(&self, name: &str) -> Result<usize> {
        let n = name.trim().to_lowercase();
        if let Some(i) = self
            .inventory
            .iter()
            .position(|it| it.name.to_lowercase() == n)
        {
            return Ok(i);
        }
        let hits: Vec<usize> = (0..self.inventory.len())
            .filter(|i| self.inventory[*i].name.to_lowercase().contains(&n))
            .collect();
        match hits.as_slice() {
            [i] => Ok(*i),
            [] => bail!("{} has no item `{name}`", self.name),
            _ => bail!(
                "`{name}` matches several items: {}",
                hits.iter()
                    .map(|i| self.inventory[*i].name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Parse "20 arrows" / "2 daggers" into (qty, name).
pub fn parse_item(s: &str) -> Item {
    let s = s.trim();
    let (qty, name) = match s.split_once(' ') {
        Some((n, rest)) if n.parse::<i32>().is_ok() => (n.parse().unwrap_or(1), rest.to_string()),
        _ => (1, s.to_string()),
    };
    let l = name.to_lowercase();
    let ammo = [
        "arrow", "bolt", "bullet", "needle", "dart", "箭", "弩矢", "弹丸",
    ]
    .iter()
    .any(|w| l.contains(w))
        && !l.contains("quiver of");
    Item {
        name,
        qty,
        ammo,
        ..Default::default()
    }
}

const ARMOR_ZH: &[(&str, &str)] = &[
    ("镶钉皮甲", "studded-leather"),
    ("兽皮甲", "hide"),
    ("链甲衫", "chain-shirt"),
    ("半身甲", "half-plate"),
    ("鳞甲", "scale-mail"),
    ("胸甲", "breastplate"),
    ("环甲", "ring-mail"),
    ("链甲", "chain-mail"),
    ("板甲", "plate"),
    ("条板甲", "splint"),
    ("布甲", "padded"),
    ("皮甲", "leather"),
];

/// Armor id of an item name, if it is body armor.
pub fn armor_of(name: &str) -> Option<&'static (&'static str, i32, Option<i32>)> {
    let s = format!("-{}-", crate::chargen::slug(name));
    let mut list: Vec<&(&str, i32, Option<i32>)> = data::ARMOR.iter().collect();
    list.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
    let id = list
        .iter()
        .find(|a| {
            s.contains(&format!("-{}-", a.0)) && !(a.0 == "plate" && s.contains("half-plate"))
        })
        .map(|a| a.0)
        .or_else(|| {
            ARMOR_ZH
                .iter()
                .find(|(z, _)| name.contains(z))
                .map(|(_, id)| *id)
        })?;
    data::ARMOR.iter().find(|a| a.0 == id)
}

pub fn is_shield(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("shield") || name.contains('盾')
}

fn text_mentions_armor(t: &str) -> bool {
    t.lines()
        .filter(|l| !l.contains("熟练") && !l.to_lowercase().contains("proficien"))
        .any(|l| {
            armor_of(l).is_some()
                || ["leather armor", "chain mail", "scale mail", "plate armor"]
                    .iter()
                    .any(|w| l.to_lowercase().contains(w))
        })
}

// ---------- combat block emitter (flow maps, as the other tools expect) ----------

fn plain_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ' | '+'))
        && !s.starts_with([' ', '-', '+'])
        && !s.ends_with(' ')
        && !matches!(
            s.to_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
        )
        && s.parse::<f64>().is_err()
}

fn flow(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if plain_ok(s) && !s.contains('+') {
                s.clone()
            } else {
                serde_json::to_string(s).unwrap_or_default()
            }
        }
        Value::Sequence(q) => format!("[{}]", q.iter().map(flow).collect::<Vec<_>>().join(", ")),
        Value::Mapping(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{}: {}", flow(k), flow(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Tagged(t) => flow(&t.value),
    }
}

pub fn emit_block(v: &Value) -> String {
    let mut out = String::new();
    let Value::Mapping(m) = v else {
        return out;
    };
    for (k, val) in m {
        let key = flow(k);
        match val {
            Value::Sequence(q) if q.iter().any(|x| x.is_mapping()) => {
                out += &format!("{key}:\n");
                for x in q {
                    out += &format!("  - {}\n", flow(x));
                }
            }
            Value::Mapping(inner)
                if key == "resources" && inner.values().all(|x| x.is_mapping()) =>
            {
                out += &format!("{key}:\n");
                for (rk, rv) in inner {
                    out += &format!("  {}: {}\n", flow(rk), flow(rv));
                }
            }
            Value::Sequence(q) if q.is_empty() && key != "todo" => {
                out += &format!("{key}: []\n");
            }
            _ => out += &format!("{key}: {}\n", flow(val)),
        }
    }
    out
}

// ---------- migrate / load / store / render ----------

/// Split a sheet into (body before the generated part, combat block, tail).
fn split_md(text: &str) -> (String, Option<Value>, String) {
    if let (Some(b), Some(e)) = (text.find(BEGIN), text.find(END)) {
        let block = crate::start::combat_block(&text[b..e]);
        return (
            text[..b].trim_end().to_string(),
            block,
            text[e + END.len()..].trim().to_string(),
        );
    }
    // Old hand-written or chargen sheet: drop the combat block section.
    let block = crate::start::combat_block(text);
    let Some(fence) = text.find("```combat yaml") else {
        return (text.trim_end().to_string(), block, String::new());
    };
    let head_start = text[..fence]
        .rfind("\n## ")
        .filter(|i| text[*i..fence].contains("combat") || text[*i..fence].contains("战斗数据块"))
        .unwrap_or(fence);
    let after = fence + 3;
    let close = text[after..]
        .find("\n```")
        .map(|i| after + i + 4)
        .unwrap_or(text.len());
    let close = text[close..]
        .find('\n')
        .map(|i| close + i + 1)
        .unwrap_or(text.len());
    let body = text[..head_start].trim_end().to_string();
    let tail = text[close..].trim().to_string();
    (body, block, tail)
}

fn guess_class(block: &Value, body: &str) -> Option<String> {
    let notes = block.get("notes").and_then(|v| v.as_str()).unwrap_or("");
    let hay = format!("{notes}\n{body}").to_lowercase();
    let zh = [
        ("barbarian", "野蛮人"),
        ("bard", "吟游诗人"),
        ("cleric", "牧师"),
        ("druid", "德鲁伊"),
        ("fighter", "战士"),
        ("monk", "武僧"),
        ("paladin", "圣武士"),
        ("ranger", "游侠"),
        ("rogue", "游荡者"),
        ("sorcerer", "术士"),
        ("warlock", "邪术师"),
        ("wizard", "法师"),
    ];
    for (en, z) in zh {
        if hay.contains(en) || hay.contains(z) {
            return Some(en.to_string());
        }
    }
    None
}

/// Build the structured file from an existing Markdown sheet. Nothing is
/// lost: the prose body is kept and rendered above the generated part.
pub fn migrate(save_dir: &Path, sheet: &check::Sheet, book: &Book) -> Result<CharFile> {
    if !crate::chargen::valid_id(&sheet.id) {
        bail!(
            "invalid character id `{}` in {}; use lowercase letters, digits and hyphens",
            sheet.id,
            sheet.file.display()
        );
    }
    let text = fs::read_to_string(&sheet.file)?;
    let (body, block, tail) = split_md(&text);
    let block = block.context("sheet has no ```combat yaml``` block")?;
    let rel = sheet
        .file
        .strip_prefix(save_dir)
        .unwrap_or(&sheet.file)
        .to_string_lossy()
        .replace('\\', "/");
    let mut cf = CharFile {
        version: 1,
        id: sheet.id.clone(),
        name: sheet.name.clone(),
        file: rel,
        ruleset: save::ruleset(save_dir),
        ac_base: get_i(&block, "ac").unwrap_or(10) as i32,
        ac_base_armored: text_mentions_armor(&body),
        body_md: body.clone(),
        tail_md: tail,
        combat: block.clone(),
        weapon_prof: vec!["simple".into(), "martial".into()],
        ..Default::default()
    };
    if let Some(c) = guess_class(&block, &body) {
        if let Some(cd) = book.class(&c) {
            if !cd.weapons.is_empty() {
                cf.weapon_prof = cd.weapons.clone();
            }
        }
        cf.info.insert("class".into(), c);
    }
    // Weapons that already have attacks are carried, equipped.
    if let Some(Value::Sequence(atks)) = block.get("attacks") {
        for a in atks {
            let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if book.weapons.iter().any(|w| w.id == id) && !cf.inventory.iter().any(|i| i.name == id)
            {
                cf.inventory.push(Item {
                    name: id.to_string(),
                    qty: 1,
                    equipped: true,
                    ..Default::default()
                });
            }
        }
    }
    migrate_armor(&mut cf, &body);
    Ok(cf)
}

/// Lines of the old sheet that name worn gear (not proficiency lists or spells).
fn gear_lines(body: &str) -> impl Iterator<Item = &str> {
    body.lines().filter(|l| {
        let low = l.to_lowercase();
        !l.contains("熟练")
            && !low.contains("proficien")
            && !low.contains("spell")
            && !l.contains("法术")
            && !l.contains("护盾术")
    })
}

/// Turn the armor and shield named on an old sheet into equipped items so AC
/// is computed from items. If the items do not explain the sheet's AC, keep
/// the sheet's AC as the base and mark it as already including the style.
fn migrate_armor(cf: &mut CharFile, body: &str) {
    let old_ac = cf.ac_base;
    let armor = gear_lines(body).find_map(armor_of);
    let shield = gear_lines(body).any(is_shield);
    let dex = cf.m("dex");
    if let Some((id, base, cap)) = armor {
        let mut expect = base + cap.map_or(dex, |c| dex.min(c));
        if shield {
            expect += 2;
        }
        if old_ac == expect || old_ac == expect + 1 {
            cf.inventory.push(Item {
                name: id.replace('-', " "),
                qty: 1,
                equipped: true,
                ..Default::default()
            });
            if shield {
                cf.inventory.push(Item {
                    name: "shield".into(),
                    qty: 1,
                    equipped: true,
                    ..Default::default()
                });
            }
            cf.ac_base = 10 + dex;
            cf.ac_base_armored = false;
            cf.ac_base_has_style = false;
            cf.ac_base_has_style = false;
            if old_ac == expect + 1 && sheet_mentions_defense(body) {
                cf.fighting_style = Some("defense".into());
            }
            return;
        }
    }
    // Could not explain the AC from items: it stays the base, style included.
    cf.ac_base_has_style = true;
}

fn sheet_mentions_defense(body: &str) -> bool {
    body.lines().any(|l| {
        let low = l.to_lowercase();
        (low.contains("defense") || l.contains("防御"))
            && (low.contains("fighting style") || l.contains("战斗风格"))
    })
}

pub fn store(save_dir: &Path, cf: &CharFile) -> Result<()> {
    if !crate::chargen::valid_id(&cf.id) {
        bail!(
            "invalid character id `{}`; use lowercase letters, digits and hyphens",
            cf.id
        );
    }
    let p = path(save_dir, &cf.id);
    fs::create_dir_all(p.parent().unwrap())?;
    let mut current = cf.clone();
    current.version = 1;
    let text = format!(
        "# TalkDND character data (source of truth). Change it with `talkdnd char {} …`; never by hand.\n{}",
        cf.id,
        serde_norway::to_string(&current)?
    );
    atomic_write(&p, text.as_bytes())?;
    Ok(())
}

fn parse_char_file(p: &Path, text: &str) -> Result<CharFile> {
    let cf: CharFile =
        serde_norway::from_str(text).with_context(|| format!("{} is not valid", p.display()))?;
    if cf.version > 1 {
        bail!(
            "{}: unsupported character schema version {} (this talkdnd supports 1); update talkdnd",
            p.display(),
            cf.version
        );
    }
    Ok(cf)
}

/// Load the structured file, migrating the Markdown sheet on first use. The
/// sheet's combat block and tail are absorbed so tool edits made to the
/// Markdown (e.g. `talkdnd level-up`) are kept.
pub fn load(save_dir: &Path, who: &str, book: &Book) -> Result<(CharFile, bool)> {
    let sheets = check::sheets(save_dir);
    let s = check::find(&sheets, who)?;
    if !crate::chargen::valid_id(&s.id) {
        bail!("invalid character id `{}` in {}", s.id, s.file.display());
    }
    let p = path(save_dir, &s.id);
    if !p.is_file() {
        return Ok((migrate(save_dir, s, book)?, true));
    }
    let mut cf: CharFile = parse_char_file(&p, &fs::read_to_string(&p)?)?;
    let text = fs::read_to_string(&s.file).unwrap_or_default();
    if text.contains(BEGIN) {
        let (_, block, tail) = split_md(&text);
        if let Some(b) = block {
            cf.combat = b;
        }
        cf.tail_md = tail;
    }
    Ok((cf, false))
}

fn member_for(ps: &mut PartyState, cf: &CharFile) -> Result<()> {
    let max = cf.max_hp();
    if let Some(m) = ps.members.get_mut(&cf.id) {
        if m.max_hp != max {
            if max > m.max_hp {
                m.hp += max - m.max_hp;
            }
            m.max_hp = max;
            m.hp = m.hp.min(max);
        }
        // Resource maxima and recharge follow the sheet; resources gained on
        // level-up are added without disturbing the current values of old ones.
        if let Some(Value::Mapping(res)) = cf.combat.get("resources") {
            for (k, v) in res {
                let Some(k) = k.as_str() else {
                    continue;
                };
                let Ok(def) = serde_norway::from_value::<ResourceDef>(v.clone()) else {
                    continue;
                };
                match m.resources.get_mut(k) {
                    Some(r) => {
                        r.max = def.max;
                        r.current = r.current.clamp(0, r.max);
                        r.recharge = def.recharge;
                        r.short_rest_regain = def.short_rest_regain;
                    }
                    None => {
                        m.resources
                            .insert(k.to_string(), ResourceState::from_def(&def));
                    }
                }
            }
        }
        return Ok(());
    }
    let creature: combat_sim::content::schema::Creature =
        serde_norway::from_value(cf.combat.clone()).map_err(|e| anyhow!("combat block: {e}"))?;
    ps.members
        .insert(cf.id.clone(), Member::from_creature(&creature));
    Ok(())
}

/// Live state for a character (party-state.yaml), created from the sheet if missing.
pub fn party(save_dir: &Path, cf: &CharFile) -> Result<PartyState> {
    let mut ps = PartyState::load(&party_path(save_dir))?;
    member_for(&mut ps, cf)?;
    Ok(ps)
}

fn md_list(v: &[String]) -> String {
    if v.is_empty() {
        "—".into()
    } else {
        v.join("、")
    }
}

pub fn render(cf: &CharFile, m: Option<&Member>) -> String {
    let mut md = cf.body_md.trim_end().to_string();
    md += "\n\n";
    md += BEGIN;
    md += "\n\n## 角色选项\n\n";
    let info = |k: &str| cf.info.get(k).cloned().unwrap_or_else(|| "—".into());
    if let Some(s) = &cf.fighting_style {
        let (n, e) = options::style(s)
            .map(|x| (format!("{}（{}）", x.zh, x.name), x.effect.clone()))
            .unwrap_or((s.clone(), "由 DM 按书裁定".into()));
        md += &format!("- **战斗风格**：{n}——{e}\n");
    }
    md += &format!("- **背景**：{}\n", info("background"));
    md += &format!("- **阵营**：{}\n", info("alignment"));
    md += &format!("- **专长**：{}\n", md_list(&cf.feats));
    md += &format!("- **语言**：{}\n", md_list(&cf.languages));
    md += &format!("- **工具**：{}\n", md_list(&cf.tools));
    for (k, zh) in [
        ("traits", "性格特点"),
        ("ideals", "理想"),
        ("bonds", "牵绊"),
        ("flaws", "缺点"),
        ("appearance", "外貌"),
        ("allies", "盟友与组织"),
        ("backstory", "背景故事"),
    ] {
        if let Some(v) = cf.info.get(k) {
            md += &format!("- **{zh}**：{v}\n");
        }
    }
    if !cf.features.is_empty() {
        md += "\n### 其他特性与特质\n\n";
        for f in &cf.features {
            md += &format!("- {f}\n");
        }
    }
    md += "\n## 当前状态\n\n";
    md += &format!("- {}\n", status_line(cf, m));
    if let Some(m) = m {
        if !m.resources.is_empty() {
            md += "- **资源**：";
            md += &m
                .resources
                .iter()
                .map(|(k, r)| format!("{k} {}/{}", r.current, r.max))
                .collect::<Vec<_>>()
                .join("　");
            md += "\n";
        }
    }
    md += "\n## 物品栏\n\n";
    if cf.inventory.is_empty() {
        md += "- （空）\n";
    } else {
        md += "| 物品 | 数量 | 状态 |\n|------|------|------|\n";
        for it in &cf.inventory {
            let mut st = Vec::new();
            if it.equipped {
                st.push("已装备".to_string());
            }
            if it.attuned {
                st.push("已同调".into());
            }
            if it.ammo {
                st.push("弹药".into());
            }
            if it.bonus != 0 {
                st.push(format!("{:+}", it.bonus));
            }
            if it.ac_bonus != 0 {
                st.push(format!("AC {:+}", it.ac_bonus));
            }
            md += &format!(
                "| {} | {} | {} |\n",
                it.name,
                it.qty,
                if st.is_empty() {
                    "—".into()
                } else {
                    st.join("、")
                }
            );
        }
    }
    md += "\n金钱见 `talkdnd loot show`（progress.yaml 钱袋）。\n";
    if !cf.notes.is_empty() {
        md += "\n## 笔记\n\n";
        for n in &cf.notes {
            md += &format!("- {n}\n");
        }
    }
    md += "\n## DM 备注\n\n";
    md += if cf.dm_notes.trim().is_empty() {
        "—\n".to_string()
    } else {
        format!("{}\n", cf.dm_notes.trim())
    }
    .as_str();
    md += "\n## 战斗数据块（combat-sim）\n\n> 由 `talkdnd char` 从 sheets/ 生成，与上文一致。\n\n```combat yaml\n";
    md += &emit_block(&cf.combat);
    md += "```\n\n";
    md += END;
    md += "\n";
    if !cf.tail_md.trim().is_empty() {
        md += "\n";
        md += cf.tail_md.trim();
        md += "\n";
    }
    md
}

pub fn status_line(cf: &CharFile, m: Option<&Member>) -> String {
    let ac = get_i(&cf.combat, "ac").unwrap_or(10);
    let mut s = match m {
        Some(m) => {
            let mut s = format!("{} HP {}/{}", cf.name, m.hp, m.max_hp);
            if m.temp_hp > 0 {
                s += &format!(" (+{} 临时)", m.temp_hp);
            }
            if m.dead {
                s += " 死亡";
            } else if m.hp <= 0 {
                s += &format!(
                    " 倒地（死亡豁免 {}成功/{}失败{}）",
                    m.death_saves.successes,
                    m.death_saves.failures,
                    if m.death_saves.stable {
                        "，已稳定"
                    } else {
                        ""
                    }
                );
            }
            s
        }
        None => format!("{} HP {}/{}", cf.name, cf.max_hp(), cf.max_hp()),
    };
    s += &format!(" | AC {ac}");
    if let Some(m) = m {
        let slots: Vec<String> = m
            .resources
            .iter()
            .filter(|(k, _)| k.starts_with("spell-slot-"))
            .map(|(k, r)| format!("{}:{}/{}", &k[11..], r.current, r.max))
            .collect();
        if !slots.is_empty() {
            s += &format!(" | 法术位 {}", slots.join(" "));
        }
        if !m.conditions.is_empty() {
            s += &format!(
                " | 状态 {}",
                m.conditions
                    .iter()
                    .map(|c| c.condition.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    if cf.exhaustion > 0 {
        s += &format!(" | 力竭 {}", cf.exhaustion);
    }
    if cf.inspiration {
        s += " | 激励";
    }
    s
}

/// Write sheets/<id>.yaml, party-state and the rendered Markdown.
pub fn commit(save_dir: &Path, cf: &CharFile, ps: &PartyState) -> Result<()> {
    store(save_dir, cf)?;
    let pp = party_path(save_dir);
    if pp.parent().is_some_and(|d| d.is_dir()) {
        ps.save(&pp)?;
    }
    let md = render(cf, ps.members.get(&cf.id));
    atomic_write(markdown_path(save_dir, &cf.file)?, md.as_bytes())?;
    Ok(())
}

/// The rendered sheet path must stay inside the save directory: a hand-edited
/// `file:` in sheets/<id>.yaml must not write outside of it.
fn markdown_path(save_dir: &Path, file: &str) -> Result<PathBuf> {
    let rel = Path::new(file);
    let safe = !file.is_empty()
        && rel.is_relative()
        && rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !safe {
        bail!("invalid sheet path `{file}`; it must stay inside the save");
    }
    Ok(save_dir.join(rel))
}

// ---------- derived values: AC and weapon attacks ----------

#[cfg_attr(not(test), allow(dead_code))]
fn add_flat(dice: &str, n: i32) -> String {
    if n == 0 {
        return dice.to_string();
    }
    // Find the trailing flat modifier.
    let (head, flat) = match dice.rfind(['+', '-']) {
        Some(i)
            if dice[i + 1..].chars().all(|c| c.is_ascii_digit()) && !dice[i + 1..].is_empty() =>
        {
            (&dice[..i], dice[i..].parse::<i32>().unwrap_or(0))
        }
        _ => (dice, 0),
    };
    classdata::dmg(head, flat + n)
}

/// Recompute AC and weapon attacks from equipped items and the fighting style.
pub fn recompute(cf: &mut CharFile, book: &Book) {
    let rs = cf.ruleset.clone();
    let style = cf.fighting_style.clone().unwrap_or_default();
    let dex = cf.m("dex");
    // AC.
    let armor = cf
        .inventory
        .iter()
        .filter(|i| i.equipped)
        .find_map(|i| armor_of(&i.name));
    let shield = cf
        .inventory
        .iter()
        .any(|i| i.equipped && is_shield(&i.name));
    let mut ac = match armor {
        Some((_, base, cap)) => base + cap.map_or(dex, |c| dex.min(c)),
        None => cf.ac_base,
    };
    if shield {
        ac += 2;
    }
    ac += cf
        .inventory
        .iter()
        .filter(|i| i.equipped)
        .map(|i| i.ac_bonus)
        .sum::<i32>();
    if style == "defense" && (armor.is_some() || (cf.ac_base_armored && !cf.ac_base_has_style)) {
        ac += 1;
    }
    cf.set("ac", Value::Number(ac.into()));

    // Weapon attacks.
    let prof = cf.prof();
    let class_def = classdata::ClassDef {
        weapons: cf.weapon_prof.clone(),
        ..Default::default()
    };
    let old: Vec<Value> = match cf.combat.get("attacks") {
        Some(Value::Sequence(v)) => v.clone(),
        _ => Vec::new(),
    };
    let is_weapon_id = |id: &str| {
        let base = id.strip_suffix("-2h").unwrap_or(id);
        book.weapons.iter().any(|w| w.id == base)
    };
    let old_by_id: BTreeMap<String, Value> = old
        .iter()
        .filter_map(|a| Some((a.get("id")?.as_str()?.to_string(), a.clone())))
        .collect();
    let mut out: Vec<Value> = Vec::new();
    let mut seen = Vec::new();
    for it in cf.inventory.iter().filter(|i| i.equipped) {
        let Some(wid) = classdata::weapons_in(std::slice::from_ref(&it.name), book)
            .into_iter()
            .next()
        else {
            continue;
        };
        if seen.contains(&wid) {
            continue;
        }
        seen.push(wid.clone());
        let Some(w) = book.weapons.iter().find(|w| w.id == wid) else {
            continue;
        };
        let w = w.for_ruleset(&rs);
        let ability = if w.ranged || (w.has("finesse") && cf.m("dex") > cf.m("str")) {
            "dex"
        } else {
            "str"
        };
        let am = cf.m(ability);
        let is_prof = classdata::proficient(Some(&class_def), &w);
        let mut to_hit = am + if is_prof { prof } else { 0 } + it.bonus;
        // Keep a larger die from the old attack (e.g. monk martial arts).
        let mut die = w.dice.clone();
        if let Some(prev) = old_by_id
            .get(&wid)
            .and_then(|a| a.get("damage"))
            .and_then(|d| d.get(0))
            .and_then(|d| d.get("dice"))
            .and_then(|d| d.as_str())
        {
            let pd: String = prev
                .chars()
                .take_while(|c| *c != '+' && *c != '-')
                .collect();
            let size = |d: &str| {
                d.split_once('d')
                    .and_then(|(_, s)| s.parse::<i32>().ok())
                    .unwrap_or(0)
            };
            if pd.starts_with("1d") && die.starts_with("1d") && size(&pd) > size(&die) {
                die = pd;
            }
        }
        let kind = if w.ranged {
            "ranged"
        } else if w.has("thrown") {
            "melee_or_ranged"
        } else {
            "melee"
        };
        if style == "archery" && kind == "ranged" {
            to_hit += 2;
        }
        let two_handed = w.has("two-handed");
        let mut dmg_bonus = am + it.bonus;
        if style == "dueling" && kind != "ranged" && !two_handed {
            dmg_bonus += 2;
        }
        let mk = |id: String, name: String, dice: String, gwf: bool| -> Value {
            let mut m = Mapping::new();
            let s = |x: &str| Value::String(x.to_string());
            m.insert(s("id"), s(&id));
            m.insert(s("name"), s(&name));
            m.insert(s("kind"), s(kind));
            m.insert(s("ability"), s(ability));
            m.insert(s("to_hit"), Value::Number(to_hit.into()));
            let mut d = Mapping::new();
            d.insert(s("dice"), s(&dice));
            d.insert(s("type"), s(&w.dtype));
            m.insert(s("damage"), Value::Sequence(vec![Value::Mapping(d)]));
            if !w.ranged {
                m.insert(
                    s("reach"),
                    Value::Number((if w.has("reach") { 10 } else { 5 }).into()),
                );
            }
            if let Some([a, b]) = w.range {
                m.insert(
                    s("range"),
                    Value::Sequence(vec![Value::Number(a.into()), Value::Number(b.into())]),
                );
            }
            if !w.properties.is_empty() {
                m.insert(
                    s("properties"),
                    Value::Sequence(w.properties.iter().map(|p| s(p)).collect()),
                );
            }
            if let Some(ms) = w.mastery.as_ref().filter(|_| cf.masteries.contains(&w.id)) {
                m.insert(s("mastery"), s(ms));
            }
            if gwf {
                if rs == "2024" {
                    m.insert(s("damage_die_min"), Value::Number(3.into()));
                } else {
                    m.insert(s("reroll_damage_below"), Value::Number(2.into()));
                }
            }
            Value::Mapping(m)
        };
        let gwf = style == "great-weapon-fighting" && kind != "ranged";
        out.push(mk(
            w.id.clone(),
            w.name.clone(),
            classdata::dmg(&die, dmg_bonus),
            gwf && two_handed,
        ));
        if let Some(v) = &w.v {
            if kind != "ranged" {
                let b2 = am + it.bonus; // Dueling does not apply two-handed.
                out.push(mk(
                    format!("{}-2h", w.id),
                    format!("{} (two-handed)", w.name),
                    classdata::dmg(v, b2),
                    gwf,
                ));
            }
        }
    }
    let kept: Vec<Value> = old
        .into_iter()
        .filter(|a| {
            let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("");
            !is_weapon_id(id)
        })
        .collect();
    out.extend(kept);
    cf.set("attacks", Value::Sequence(out));
}

/// After `chargen sheet`: build the structured file from the creation choices
/// and render the sheet from it (replaces any earlier draft for this id).
pub fn from_chargen(
    save_dir: &Path,
    c: &crate::chargen::Creation,
    book: &Book,
    sheet_file: &Path,
) -> Result<CharFile> {
    let sheets = check::sheets(save_dir);
    let s = sheets
        .iter()
        .find(|s| s.file == sheet_file)
        .context("written sheet not found")?;
    let fresh = migrate(save_dir, s, book)?;
    let existing = if path(save_dir, &s.id).is_file() {
        let p = path(save_dir, &s.id);
        Some(parse_char_file(&p, &fs::read_to_string(&p)?)?)
    } else {
        None
    };
    let is_new = existing.is_none();
    let mut cf = existing.unwrap_or_else(|| fresh.clone());
    // The freshly written Markdown is the new build snapshot. Keep the old
    // tail (level-up records) unless the new sheet carries one.
    cf.file = fresh.file.clone();
    cf.ruleset = fresh.ruleset.clone();
    cf.body_md = fresh.body_md.clone();
    cf.combat = fresh.combat.clone();
    if !fresh.tail_md.trim().is_empty() {
        cf.tail_md = fresh.tail_md.clone();
    }
    // The inventory table replaces chargen's duplicated gear section.
    if let Some(i) = cf.body_md.find("\n## 装备\n") {
        let rest = &cf.body_md[i + 1..];
        let end = rest[3..]
            .find("\n## ")
            .map(|j| i + 1 + 3 + j)
            .unwrap_or(cf.body_md.len());
        cf.body_md = format!("{}{}", &cf.body_md[..i], &cf.body_md[end..]);
    }
    // Build choices always follow the creation state.
    let put = |cf: &mut CharFile, k: &str, v: &Option<String>| {
        if let Some(v) = v {
            cf.info.insert(k.into(), v.clone());
        }
    };
    put(&mut cf, "class", &c.class);
    put(&mut cf, "species", &c.species);
    put(&mut cf, "background", &c.background);
    put(&mut cf, "alignment", &c.alignment);
    put(&mut cf, "traits", &c.personality);
    put(&mut cf, "ideals", &c.ideals);
    put(&mut cf, "bonds", &c.bonds);
    put(&mut cf, "flaws", &c.flaws);
    // Play choices are kept; only fill what the structured file does not have.
    if cf.fighting_style.is_none() {
        cf.fighting_style = c.fighting_style.clone();
    }
    if cf.feats.is_empty() {
        cf.feats = c.feats.clone();
    }
    if cf.languages.is_empty() {
        cf.languages = c.languages.clone();
    }
    if cf.tools.is_empty() {
        cf.tools = c.tools.clone();
    }
    if cf.masteries.is_empty() {
        cf.masteries = c.masteries.clone();
    }
    if cf.weapon_prof.is_empty() {
        if let Some(cd) = c.class.as_deref().and_then(|x| book.class(x)) {
            if !cd.weapons.is_empty() {
                cf.weapon_prof = cd.weapons.clone();
            }
        }
    }
    if let Some(bg) = c.background_def(Some(book)) {
        if let Some(f) = &bg.feature {
            let line = format!("背景特性：{f}");
            if !cf.features.contains(&line) {
                cf.features.push(line);
            }
        }
        if let Some(f) = &bg.feat {
            if !cf.feats.contains(f) {
                cf.feats.push(f.clone());
            }
        }
        for t in &bg.tools {
            if !cf.tools.contains(t) {
                cf.tools.push(t.clone());
            }
        }
    }
    if c.rs() == "2024" {
        if let Some(st) = c.fighting_style.as_deref().and_then(options::style) {
            let f = format!("Fighting Style: {}", st.name);
            if !cf.feats.contains(&f) {
                cf.feats.push(f);
            }
        }
    }
    // Starting equipment: add missing items, never remove what play added.
    if is_new {
        cf.inventory.clear();
    }
    let mut items = c.all_items(Some(book));
    if let Some(a) = c.armor.as_deref().filter(|a| *a != "none") {
        if !items.iter().any(|i| armor_of(i).map(|x| x.0) == Some(a)) {
            items.push(a.replace('-', " "));
        }
    }
    if c.shield && !items.iter().any(|i| is_shield(i)) {
        items.push("shield".into());
    }
    let mut armor_on = cf
        .inventory
        .iter()
        .any(|i| i.equipped && armor_of(&i.name).is_some());
    for raw in items {
        if options::is_placeholder(&raw) {
            continue;
        }
        let mut it = parse_item(&raw);
        let weapon = !classdata::weapons_in(std::slice::from_ref(&it.name), book).is_empty();
        let armor = armor_of(&it.name).is_some();
        let equipped = weapon || is_shield(&it.name) || (armor && !armor_on);
        armor_on |= armor;
        it.equipped = equipped;
        match cf
            .inventory
            .iter()
            .position(|x| x.name.eq_ignore_ascii_case(&it.name))
        {
            Some(i) => {
                if cf.inventory[i].qty < it.qty {
                    cf.inventory[i].qty = it.qty;
                }
                if is_new {
                    cf.inventory[i].equipped = equipped;
                }
            }
            None => cf.inventory.push(it),
        }
    }
    if is_new {
        cf.ac_base = c.ac_unarmored();
        cf.ac_base_armored = false;
        cf.ac_base_has_style = false;
    }
    recompute(&mut cf, book);
    let ps = party(save_dir, &cf)?;
    commit(save_dir, &cf, &ps)?;
    Ok(cf)
}

// ---------- commands ----------

#[derive(clap::Args, Clone, Debug, Default)]
pub struct Coins {
    #[arg(long, default_value_t = 0)]
    pub pp: i64,
    #[arg(long, default_value_t = 0)]
    pub gp: i64,
    #[arg(long, default_value_t = 0)]
    pub ep: i64,
    #[arg(long, default_value_t = 0)]
    pub sp: i64,
    #[arg(long, default_value_t = 0)]
    pub cp: i64,
}

#[derive(clap::Subcommand, Clone, Debug)]
pub enum Op {
    /// One-line status (default)
    Show,
    /// Create sheets/<id>.yaml from the Markdown sheet (done automatically on first use)
    Init,
    /// Take damage (temp HP first): `damage 7`
    Damage { amount: i32 },
    /// Regain HP: `heal 5`
    Heal { amount: i32 },
    /// Set temporary HP (keeps the higher value): `temp 8`
    Temp { amount: i32 },
    /// Spend a resource or spell slot: `use spell-slot-1`, `use slot2`, `use rage`
    Use {
        resource: String,
        #[arg(default_value_t = 1)]
        n: i32,
    },
    /// Restore a resource: `restore ki 2`, `restore slot1 all`
    Restore {
        resource: String,
        #[arg(default_value = "all")]
        n: String,
    },
    /// `rest short [--hd N]` (spends Hit Dice, rolls logged) or `rest long`
    Rest {
        kind: String,
        #[arg(long, default_value_t = 0)]
        hd: u32,
    },
    /// `condition add poisoned` / `condition remove poisoned`
    Condition { action: String, name: String },
    /// Exhaustion: `exhaustion +1`, `exhaustion -1`, `exhaustion 0`
    Exhaustion {
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Heroic Inspiration: `inspiration on|off`
    Inspiration { state: String },
    /// Death save at 0 HP: `death-save roll` (d20, logged), `success`, `fail`, `stable`
    DeathSave { result: String },
    /// Coins via the progress.yaml purse: `gold add --gp 5 --sp 3`, `gold spend --gp 2`
    Gold {
        action: String,
        #[command(flatten)]
        coins: Coins,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Inventory: `item add "rope (50 ft)" --qty 1`, `item equip longsword`,
    /// `item remove torch --qty 2`, `item attune "ring of protection"`.
    /// Names outside the weapon/armor/common-gear data need `--canon` or
    /// `--original O-xxx` (a registered original item)
    Item {
        action: String,
        name: Vec<String>,
        #[arg(long)]
        qty: Option<i32>,
        #[arg(long)]
        equip: bool,
        #[arg(long, allow_hyphen_values = true)]
        bonus: Option<i32>,
        #[arg(long, allow_hyphen_values = true)]
        ac_bonus: Option<i32>,
        #[arg(long)]
        ammo: bool,
        /// The DM checked the book: accept a name that is not in the weapon,
        /// armor or common-gear data
        #[arg(long)]
        canon: bool,
        /// Registered original item (`O-001`); validated against the save's
        /// original ledger
        #[arg(long, conflicts_with = "canon")]
        original: Option<String>,
    },
    /// Ammunition: `ammo arrows -3` (spent) or `ammo arrows +2` (recovered)
    Ammo {
        name: String,
        #[arg(allow_hyphen_values = true)]
        delta: i32,
    },
    /// Append a player-visible note
    Note { text: Vec<String> },
    /// Append to the freeform DM notes (only what programs cannot model)
    DmNote { text: Vec<String> },
    /// Sheet choices: fighting-style, background, feats, languages, tools,
    /// masteries, class, alignment, traits, ideals, bonds, flaws, appearance,
    /// backstory, allies, ac-base
    Set {
        field: String,
        value: Vec<String>,
        /// background: equipment option (2024 A/B)
        #[arg(long)]
        option: Option<String>,
        /// background: don't add starting gold (already in the purse)
        #[arg(long)]
        no_gold: bool,
        /// background: don't add the equipment
        #[arg(long)]
        no_items: bool,
    },
    /// Several changes at once: `batch "damage 7; use slot1; ammo arrows -2"`
    Batch { ops: Vec<String> },
}

#[derive(clap::Parser)]
#[command(no_binary_name = true)]
struct OpLine {
    #[command(subcommand)]
    op: Op,
}

/// Split one batch entry into words (double or single quotes group words).
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (q, c) {
            (Some(x), c) if c == x => q = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                q = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() || any {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.is_empty() || any {
        out.push(cur);
    }
    out
}

pub fn parse_batch(text: &str) -> Result<Vec<Op>> {
    let mut v = Vec::new();
    for part in text
        .split([';', '\n'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let w = words(part);
        let line = <OpLine as clap::Parser>::try_parse_from(w).map_err(|e| {
            anyhow!(
                "batch `{part}`: {}",
                e.to_string().lines().next().unwrap_or("")
            )
        })?;
        if matches!(line.op, Op::Batch { .. }) {
            bail!("batch cannot nest");
        }
        v.push(line.op);
    }
    Ok(v)
}

fn resource_id(r: &str) -> String {
    let l = r.trim().to_lowercase().replace(' ', "");
    if let Some(n) = l.strip_prefix("slot") {
        return format!("spell-slot-{n}");
    }
    if let Some(n) = l.strip_prefix("法术位") {
        return format!("spell-slot-{n}");
    }
    if l == "hd" || l == "hitdice" {
        return "hit-dice".into();
    }
    l
}

fn log_line(save_dir: &Path, name: &str, msg: &str) -> Result<()> {
    if let Some(l) = save::latest_log(save_dir) {
        save::append(
            &l,
            &format!(
                "- [{}] 角色 {name}：{msg}",
                chrono::Local::now().format("%H:%M")
            ),
        )?;
    }
    Ok(())
}

fn condition(name: &str) -> Result<combat_sim::state::ActiveCondition> {
    serde_norway::from_str(&format!(
        "condition: {}\nsource: talkdnd char\n",
        serde_json::to_string(&name.trim().to_lowercase())?
    ))
    .map_err(|e| anyhow!("{e}"))
}

/// The campaign's dying conditions from `Rules/combat/common/rules.yaml`.
/// Falls back to `unconscious` when the file or key is absent.
fn dying_conditions(save_dir: &Path) -> Vec<String> {
    let fallback = || vec!["unconscious".to_string()];
    let Some(root) = save_dir.parent().and_then(|p| p.parent()) else {
        return fallback();
    };
    let path = root.join("Rules/combat/common/rules.yaml");
    let Ok(text) = fs::read_to_string(&path) else {
        return fallback();
    };
    let Ok(value) = serde_norway::from_str::<Value>(&text) else {
        return fallback();
    };
    let Some(rules) = value.get("rules") else {
        return fallback();
    };
    match rules.get("dying_conditions") {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(|name| name.trim().to_lowercase())
            .filter(|name| !name.is_empty())
            .collect(),
        _ => fallback(),
    }
}

/// Keep the party-state conditions in step with HP: dying conditions at 0 HP,
/// none (and fresh death saves) above 0.
fn sync_hp_conditions(m: &mut Member, dying: &[String]) -> Result<()> {
    if m.hp > 0 {
        m.conditions.retain(|active| {
            !dying
                .iter()
                .any(|name| active.condition.eq_ignore_ascii_case(name))
        });
        m.death_saves = Default::default();
    } else {
        for name in dying {
            if !m
                .conditions
                .iter()
                .any(|active| active.condition.eq_ignore_ascii_case(name))
            {
                m.conditions.push(condition(name)?);
            }
        }
    }
    Ok(())
}

/// Apply a background from SRD or the local book import: feature/feat,
/// tools, languages, equipment items and gold.
fn apply_background(
    save_dir: &Path,
    cf: &mut CharFile,
    book: &Book,
    id: &str,
    option: Option<&str>,
    no_gold: bool,
    no_items: bool,
) -> Result<Vec<String>> {
    let sid = crate::chargen::slug(id);
    let bg = options::srd_background(&cf.ruleset, &sid)
        .cloned()
        .or_else(|| book.backgrounds.get(&sid).cloned());
    cf.info.insert("background".into(), sid.clone());
    let mut msgs = vec![format!("背景 {sid}")];
    let Some(bg) = bg else {
        msgs.push(format!(
            "无 `{sid}` 的背景数据（{}）：特性、工具、装备与金币用 `char set features/tools`、`char item add`、`char gold add` 补",
            crate::classdata::import_hint(save_dir.parent().and_then(|p| p.parent()), &cf.ruleset)
        ));
        return Ok(msgs);
    };
    if let Some(f) = &bg.feature {
        let line = format!("背景特性：{f}");
        if !cf.features.contains(&line) {
            cf.features.push(line);
        }
    }
    if let Some(f) = &bg.feat {
        if !cf.feats.contains(f) {
            cf.feats.push(f.clone());
        }
    }
    for t in &bg.tools {
        if !cf.tools.contains(t) {
            cf.tools.push(t.clone());
        }
    }
    if bg.languages > 0 && cf.languages.is_empty() {
        msgs.push(format!(
            "背景给 {} 门自选语言：请玩家选，`char set languages …`",
            bg.languages
        ));
    }
    let r = options::background_equipment(&bg, option)?;
    if !no_items {
        for it in &r.items {
            let mut item = parse_item(it);
            if let Some(i) = cf
                .inventory
                .iter()
                .position(|x| x.name.eq_ignore_ascii_case(&item.name))
            {
                cf.inventory[i].qty += item.qty;
            } else {
                item.equipped = false;
                cf.inventory.push(item);
            }
        }
        if !r.items.is_empty() {
            msgs.push(format!("背景装备：{}", r.items.join("、")));
        }
    }
    if !no_gold && r.gold > 0 {
        crate::progress::loot(
            save_dir,
            crate::progress::LootOp::Add {
                to: &cf.id,
                cp: r.gold * 100,
                items: &[],
            },
            Some(&format!("{} 起始金币（背景 {sid}）", cf.name)),
        )?;
        msgs.push(format!("起始金币 {} gp 入钱袋", r.gold));
    }
    Ok(msgs)
}

fn list(v: &str) -> Vec<String> {
    v.split([',', '，', '、', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Run one op on a loaded character; returns a short log message ("" = nothing to log).
#[allow(clippy::too_many_arguments)]
fn apply(
    save_dir: &Path,
    cf: &mut CharFile,
    ps: &mut PartyState,
    book: &Book,
    op: Op,
    rng: &mut dyn DiceRng,
    out: &mut Vec<String>,
) -> Result<String> {
    let id = cf.id.clone();
    let dying = dying_conditions(save_dir);
    let m = ps
        .members
        .get_mut(&id)
        .ok_or_else(|| anyhow!("no party state for {id}"))?;
    let msg = match op {
        Op::Show | Op::Init => String::new(),
        Op::Damage { amount } => {
            if amount < 0 {
                bail!("damage must be positive");
            }
            let from_temp = amount.min(m.temp_hp);
            m.temp_hp -= from_temp;
            let rest = amount - from_temp;
            let before = m.hp;
            if before <= 0 && rest > 0 {
                m.death_saves.failures = (m.death_saves.failures + 1).min(3);
                m.death_saves.stable = false;
            }
            m.hp = (m.hp - rest).max(0);
            let massive = before > 0 && rest - before >= m.max_hp;
            if massive || m.death_saves.failures >= 3 {
                m.dead = true;
                m.death_saves.dead = true;
            }
            sync_hp_conditions(m, &dying)?;
            format!(
                "受到 {amount} 伤害{}，HP {before}→{}{}",
                if from_temp > 0 {
                    format!("（临时 HP 吸收 {from_temp}）")
                } else {
                    String::new()
                },
                m.hp,
                if m.dead {
                    "，死亡"
                } else if m.hp == 0 {
                    "，倒地"
                } else {
                    ""
                }
            )
        }
        Op::Heal { amount } => {
            if m.dead {
                bail!("{} is dead", cf.name);
            }
            let before = m.hp;
            m.hp = (m.hp + amount.max(0)).min(m.max_hp);
            if before <= 0 && m.hp > 0 {
                m.death_saves = Default::default();
            }
            sync_hp_conditions(m, &dying)?;
            format!("治疗 {amount}，HP {before}→{}", m.hp)
        }
        Op::Temp { amount } => {
            let before = m.temp_hp;
            m.temp_hp = m.temp_hp.max(amount);
            format!("临时 HP {before}→{}（不叠加，取高）", m.temp_hp)
        }
        Op::Use { resource, n } => {
            let rid = resource_id(&resource);
            let r = m.resources.get_mut(&rid).ok_or_else(|| {
                anyhow!(
                    "{} has no resource `{rid}` (have: {})",
                    cf.name,
                    m_keys(&cf.combat)
                )
            })?;
            if r.current < n {
                bail!("{rid}: only {}/{} left", r.current, r.max);
            }
            r.current -= n;
            format!("消耗 {rid} ×{n}（剩 {}/{}）", r.current, r.max)
        }
        Op::Restore { resource, n } => {
            let rid = resource_id(&resource);
            let r = m
                .resources
                .get_mut(&rid)
                .ok_or_else(|| anyhow!("{} has no resource `{rid}`", cf.name))?;
            let add = if n == "all" || n == "max" {
                r.max
            } else {
                n.parse::<i32>().context("restore N|all")?
            };
            r.current = (r.current + add).min(r.max);
            format!("恢复 {rid}（{}/{}）", r.current, r.max)
        }
        Op::Rest { kind, hd } => {
            let mut log = Vec::new();
            match kind.as_str() {
                "short" | "短休" => {
                    m.short_rest(hd, rng, &mut log)?;
                }
                "long" | "长休" => {
                    m.long_rest(&dying, &mut log);
                    if cf.exhaustion > 0 {
                        cf.exhaustion -= 1;
                        log.push(format!("exhaustion → {}", cf.exhaustion));
                    }
                }
                _ => bail!("rest short|long"),
            }
            sync_hp_conditions(m, &dying)?;
            format!(
                "{}休息：{}（HP {}/{}）",
                if kind.starts_with('s') || kind == "短休" {
                    "短"
                } else {
                    "长"
                },
                log.join("；"),
                m.hp,
                m.max_hp
            )
        }
        Op::Condition { action, name } => {
            let n = name.trim().to_lowercase();
            match action.as_str() {
                "add" | "+" => {
                    if !m.conditions.iter().any(|c| c.condition == n) {
                        m.conditions.push(condition(&n)?);
                    }
                    format!("获得状态 {n}")
                }
                "remove" | "rm" | "-" => {
                    let before = m.conditions.len();
                    m.conditions.retain(|c| c.condition != n);
                    if before == m.conditions.len() {
                        bail!("{} is not {n}", cf.name);
                    }
                    format!("解除状态 {n}")
                }
                _ => bail!("condition add|remove <name>"),
            }
        }
        Op::Exhaustion { value } => {
            let before = cf.exhaustion;
            let v = value.trim();
            cf.exhaustion = if let Some(d) = v.strip_prefix('+') {
                before + d.parse::<i32>()?
            } else if v.starts_with('-') {
                before + v.parse::<i32>()?
            } else {
                v.parse::<i32>()?
            }
            .clamp(0, 6);
            if cf.exhaustion >= 6 {
                m.dead = true;
            }
            format!("力竭 {before}→{}", cf.exhaustion)
        }
        Op::Inspiration { state } => {
            cf.inspiration = matches!(state.as_str(), "on" | "yes" | "1" | "true" | "gain" | "+");
            format!("激励 {}", if cf.inspiration { "获得" } else { "用掉" })
        }
        Op::DeathSave { result } => {
            if m.hp > 0 {
                bail!("{} is not at 0 HP", cf.name);
            }
            let ds = &mut m.death_saves;
            let text = match result.as_str() {
                "roll" => {
                    let d = rng.roll_die(20);
                    match d {
                        20 => {
                            m.hp = 1;
                            m.death_saves = Default::default();
                            "死亡豁免 d20=20，恢复 1 HP".to_string()
                        }
                        1 => {
                            ds.failures = (ds.failures + 2).min(3);
                            "死亡豁免 d20=1，失败 ×2".to_string()
                        }
                        d if d >= 10 => {
                            ds.successes += 1;
                            format!("死亡豁免 d20={d}，成功")
                        }
                        d => {
                            ds.failures += 1;
                            format!("死亡豁免 d20={d}，失败")
                        }
                    }
                }
                "success" => {
                    ds.successes += 1;
                    "死亡豁免成功".into()
                }
                "fail" => {
                    ds.failures += 1;
                    "死亡豁免失败".into()
                }
                "stable" => {
                    ds.stable = true;
                    "已稳定".into()
                }
                _ => bail!("death-save roll|success|fail|stable"),
            };
            let ds = &mut m.death_saves;
            if ds.successes >= 3 {
                ds.stable = true;
                ds.successes = 0;
                ds.failures = 0;
            }
            if ds.failures >= 3 {
                ds.dead = true;
                m.dead = true;
            }
            sync_hp_conditions(m, &dying)?;
            format!(
                "{text}（HP {}，{}成功/{}失败{}）",
                m.hp,
                m.death_saves.successes,
                m.death_saves.failures,
                if m.dead {
                    "，死亡"
                } else if m.death_saves.stable {
                    "，稳定"
                } else {
                    ""
                }
            )
        }
        Op::Gold {
            action,
            coins,
            reason,
        } => {
            let cp = crate::progress::to_cp(coins.pp, coins.gp, coins.ep, coins.sp, coins.cp);
            if cp <= 0 {
                bail!("give an amount, e.g. --gp 5");
            }
            let why = reason.unwrap_or_else(|| format!("{} 金钱", cf.name));
            let op = match action.as_str() {
                "add" | "+" => crate::progress::LootOp::Add {
                    to: &id,
                    cp,
                    items: &[],
                },
                "spend" | "-" => crate::progress::LootOp::Spend {
                    from: &id,
                    cp,
                    items: &[],
                },
                _ => bail!("gold add|spend --gp N"),
            };
            out.extend(
                crate::progress::loot(save_dir, op, Some(&why))?
                    .into_iter()
                    .take(1),
            );
            String::new()
        }
        Op::Item {
            action,
            name,
            qty,
            equip,
            bonus,
            ac_bonus,
            ammo,
            canon,
            original,
        } => {
            let name = name.join(" ");
            if name.trim().is_empty() {
                bail!("item {action} <name>");
            }
            if action != "add" && (canon || original.is_some()) {
                bail!("--canon/--original 只用于 `item add`");
            }
            let res = match action.as_str() {
                "add" => {
                    let mut it = parse_item(&name);
                    if let Some(q) = qty {
                        it.qty = q;
                    }
                    if let Some(id) = &original {
                        crate::original::validate_item_reference(save_dir, id)?;
                        it.original = Some(id.trim().to_string());
                    } else if !canon && !crate::original::is_canon_item(&it.name, book) {
                        bail!(
                            "物品「{}」不在规则/剧本数据里。确认书里有这件东西就加 --canon；是原创物品就先 `talkdnd original add --type item --name \"{}\" --rarity <稀有度>`，再用 `--original O-xxx` 加入。",
                            it.name,
                            it.name
                        );
                    }
                    it.equipped = equip;
                    it.bonus = bonus.unwrap_or(0);
                    it.ac_bonus = ac_bonus.unwrap_or(0);
                    it.ammo |= ammo;
                    let label = format!("{} ×{}", it.name, it.qty);
                    match cf
                        .inventory
                        .iter()
                        .position(|x| x.name.eq_ignore_ascii_case(&it.name))
                    {
                        Some(i) => {
                            if let Some(new_id) = &it.original {
                                match &cf.inventory[i].original {
                                    Some(old_id) if old_id != new_id => bail!(
                                        "同名物品已标为原创 {old_id}，不能再作为 {new_id} 加入"
                                    ),
                                    None => cf.inventory[i].original = Some(new_id.clone()),
                                    _ => {}
                                }
                            }
                            cf.inventory[i].qty += it.qty;
                        }
                        None => cf.inventory.push(it),
                    }
                    format!(
                        "获得物品 {label}{}{}",
                        if equip { "（已装备）" } else { "" },
                        match &original {
                            Some(id) => format!("（原创 {id}）"),
                            None => String::new(),
                        }
                    )
                }
                "remove" | "rm" | "drop" => {
                    let i = cf.find_item(&name)?;
                    let q = qty.unwrap_or(cf.inventory[i].qty);
                    let n = cf.inventory[i].name.clone();
                    if q >= cf.inventory[i].qty {
                        cf.inventory.remove(i);
                    } else {
                        cf.inventory[i].qty -= q;
                    }
                    format!("失去物品 {n} ×{q}")
                }
                "equip" | "unequip" => {
                    let i = cf.find_item(&name)?;
                    let on = action == "equip";
                    if on && armor_of(&cf.inventory[i].name).is_some() {
                        for it in cf.inventory.iter_mut() {
                            if armor_of(&it.name).is_some() {
                                it.equipped = false;
                            }
                        }
                    }
                    cf.inventory[i].equipped = on;
                    if let Some(b) = bonus {
                        cf.inventory[i].bonus = b;
                    }
                    if let Some(b) = ac_bonus {
                        cf.inventory[i].ac_bonus = b;
                    }
                    format!(
                        "{}{}",
                        if on { "装备 " } else { "卸下 " },
                        cf.inventory[i].name
                    )
                }
                "attune" | "unattune" => {
                    let i = cf.find_item(&name)?;
                    let on = action == "attune";
                    if on
                        && !cf.inventory[i].attuned
                        && cf.inventory.iter().filter(|x| x.attuned).count() >= 3
                    {
                        bail!("already attuned to 3 items");
                    }
                    cf.inventory[i].attuned = on;
                    format!(
                        "{}同调 {}",
                        if on { "" } else { "解除" },
                        cf.inventory[i].name
                    )
                }
                _ => bail!("item add|remove|equip|unequip|attune|unattune <name>"),
            };
            recompute(cf, book);
            res
        }
        Op::Ammo { name, delta } => {
            let i = cf.find_item(&name)?;
            let it = &mut cf.inventory[i];
            if it.qty + delta < 0 {
                bail!("{} has only {} {}", cf.name, it.qty, it.name);
            }
            it.qty += delta;
            format!("弹药 {} {delta:+}（剩 {}）", it.name, it.qty)
        }
        Op::Note { text } => {
            let t = text.join(" ");
            cf.notes.push(t.clone());
            format!("笔记：{t}")
        }
        Op::DmNote { text } => {
            let t = text.join(" ");
            if !cf.dm_notes.is_empty() {
                cf.dm_notes.push('\n');
            }
            cf.dm_notes += &format!("- {t}");
            String::new()
        }
        Op::Set {
            field,
            value,
            option,
            no_gold,
            no_items,
        } => {
            let v = value.join(" ").trim().to_string();
            match field.as_str() {
                "fighting-style" | "style" => {
                    let sid = crate::chargen::slug(&v);
                    match options::style(&sid) {
                        None => out.push(format!("warning: `{sid}` is not a known fighting style; recorded, the DM rules its effect")),
                        Some(st) if !st.rulesets.contains(&cf.ruleset) => out.push(format!("warning: {} is not a {} option", st.name, cf.ruleset)),
                        _ => {}
                    }
                    if let Some(c) = cf.class() {
                        match options::style_for_class(&cf.ruleset, &c) {
                            None => out.push(format!("warning: {c} has no Fighting Style feature")),
                            Some((l, allowed)) => {
                                if cf.level() < l {
                                    out.push(format!(
                                        "warning: {c} gets a Fighting Style at level {l}"
                                    ));
                                }
                                if !allowed.contains(&sid) {
                                    out.push(format!(
                                        "warning: `{sid}` is not on the {c} list ({})",
                                        allowed.join(", ")
                                    ));
                                }
                            }
                        }
                    }
                    cf.fighting_style = Some(sid.clone());
                    if cf.ruleset == "2024" {
                        let feat = options::style(&sid)
                            .map(|s| s.name.clone())
                            .unwrap_or(sid.clone());
                        let feat = format!("Fighting Style: {feat}");
                        cf.feats.retain(|f| !f.starts_with("Fighting Style:"));
                        cf.feats.push(feat);
                    }
                    recompute(cf, book);
                    format!("战斗风格 {sid}（攻击/AC 已按规则重算）")
                }
                "background" => {
                    let msgs = apply_background(
                        save_dir,
                        cf,
                        book,
                        &v,
                        option.as_deref(),
                        no_gold,
                        no_items,
                    )?;
                    out.extend(msgs.iter().skip(1).cloned());
                    msgs.join("；")
                }
                "feats" => {
                    cf.feats = list(&v);
                    format!("专长 {}", cf.feats.join("、"))
                }
                "languages" => {
                    cf.languages = list(&v);
                    format!("语言 {}", cf.languages.join("、"))
                }
                "tools" => {
                    cf.tools = list(&v);
                    format!("工具 {}", cf.tools.join("、"))
                }
                "features" => {
                    cf.features.extend(list(&v));
                    format!("特性 {v}")
                }
                "masteries" => {
                    cf.masteries = list(&v).iter().map(|x| crate::chargen::slug(x)).collect();
                    recompute(cf, book);
                    format!("武器精通 {}", cf.masteries.join("、"))
                }
                "weapon-prof" => {
                    cf.weapon_prof = list(&v);
                    recompute(cf, book);
                    format!("武器熟练 {v}")
                }
                "ac-base" => {
                    cf.ac_base = v.parse().context("ac-base N")?;
                    recompute(cf, book);
                    format!("基础 AC {}", cf.ac_base)
                }
                "class" => {
                    let c = crate::chargen::slug(&v);
                    cf.info.insert("class".into(), c.clone());
                    format!("职业 {c}")
                }
                k @ ("alignment" | "traits" | "personality" | "ideals" | "bonds" | "flaws"
                | "appearance" | "backstory" | "allies" | "player" | "species") => {
                    let k = if k == "personality" { "traits" } else { k };
                    cf.info.insert(k.into(), v.clone());
                    format!("{k}：{v}")
                }
                other => bail!("unknown field `{other}`"),
            }
        }
        Op::Batch { .. } => bail!("batch cannot nest"),
    };
    Ok(msg)
}

fn m_keys(combat: &Value) -> String {
    match combat.get("resources") {
        Some(Value::Mapping(m)) => m
            .keys()
            .filter_map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => "-".into(),
    }
}

/// `talkdnd char <who> <op>`: load, apply (all ops of a batch), log once per
/// op, then write yaml + party state + rendered sheet. Atomic: nothing is
/// written when any op fails.
pub fn run(save_dir: &Path, who: &str, op: Op, rng: &mut dyn DiceRng) -> Result<Vec<String>> {
    let rs = save::ruleset(save_dir);
    let root = save_dir.parent().and_then(|p| p.parent());
    let book = Book::load(&rs, root)?;
    if who == "all" || who == "party" {
        let Op::Rest { .. } = op else {
            bail!("`all` works with `rest` only");
        };
        let mut out = Vec::new();
        for id in crate::start::party_ids(save_dir) {
            if check::find(&check::sheets(save_dir), &id).is_ok() {
                out.extend(run(save_dir, &id, op.clone(), rng)?);
            }
        }
        return Ok(out);
    }
    let (mut cf, migrated) = load(save_dir, who, &book)?;
    let mut ps = party(save_dir, &cf)?;
    let ops = match op {
        Op::Batch { ops } => parse_batch(&ops.join(" "))?,
        o => vec![o],
    };
    let mut out = Vec::new();
    if migrated {
        out.push(format!(
            "migrated {} → sheets/{}.yaml (the Markdown sheet is now rendered from it)",
            cf.file, cf.id
        ));
    }
    let mut msgs = Vec::new();
    for o in ops {
        let msg = apply(save_dir, &mut cf, &mut ps, &book, o, rng, &mut out)?;
        if !msg.is_empty() {
            msgs.push(msg);
        }
    }
    commit(save_dir, &cf, &ps)?;
    if migrated {
        log_line(
            save_dir,
            &cf.name,
            "角色卡迁移为结构化数据（sheets/，内容不变）",
        )?;
    }
    if !msgs.is_empty() {
        log_line(save_dir, &cf.name, &msgs.join("；"))?;
        out.push(msgs.join("；"));
    }
    out.push(status_line(&cf, ps.members.get(&cf.id)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cf(rs: &str) -> CharFile {
        let combat: Value = serde_norway::from_str(
            "id: tess\nname: Tess\nkind: pc\nside: party\nlevel: 1\nac: 10\nhp: 12\nprof: 2\nabilities: {str: 16, dex: 12, con: 14, int: 10, wis: 10, cha: 8}\nsaves: {str: 5, con: 4}\nattacks:\n  - {id: unarmed, name: Unarmed Strike, kind: melee, ability: str, to_hit: 5, damage: [{dice: \"4\", type: bludgeoning}], reach: 5}\n",
        )
        .unwrap();
        CharFile {
            version: 1,
            id: "tess".into(),
            name: "Tess".into(),
            file: "Characters/Tess.md".into(),
            ruleset: rs.into(),
            ac_base: 11,
            weapon_prof: vec!["simple".into(), "martial".into()],
            combat,
            ..Default::default()
        }
    }

    fn atk<'a>(c: &'a CharFile, id: &str) -> &'a Value {
        match c.combat.get("attacks") {
            Some(Value::Sequence(v)) => v
                .iter()
                .find(|a| a.get("id").and_then(|x| x.as_str()) == Some(id))
                .unwrap_or_else(|| panic!("no attack {id}")),
            _ => panic!(),
        }
    }

    fn dice(a: &Value) -> String {
        a["damage"][0]["dice"].as_str().unwrap().to_string()
    }

    #[test]
    fn gwf_2014_and_2024() {
        let book = Book::srd("2014");
        let mut c = cf("2014");
        c.fighting_style = Some("great-weapon-fighting".into());
        c.inventory.push(Item {
            name: "greatsword".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        c.inventory.push(Item {
            name: "longsword".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        recompute(&mut c, &book);
        assert_eq!(
            atk(&c, "greatsword")["reroll_damage_below"].as_i64(),
            Some(2)
        );
        assert!(atk(&c, "longsword").get("reroll_damage_below").is_none());
        assert_eq!(
            atk(&c, "longsword-2h")["reroll_damage_below"].as_i64(),
            Some(2)
        );
        assert_eq!(dice(atk(&c, "longsword-2h")), "1d10+3");
        assert!(atk(&c, "unarmed").is_mapping());
        let mut d = cf("2024");
        d.fighting_style = Some("great-weapon-fighting".into());
        d.inventory.push(Item {
            name: "greatsword".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        recompute(&mut d, &Book::srd("2024"));
        assert_eq!(atk(&d, "greatsword")["damage_die_min"].as_i64(), Some(3));
    }

    #[test]
    fn migrated_ac_not_stacked_with_defense() {
        let book = Book::srd("2014");
        // Armor and shield named on the old sheet become equipped items.
        let mut c = cf("2014");
        c.ac_base = 19;
        let body = "- 战斗风格：防御（Defense）\n- 装备：链甲（chain mail）、盾牌\n- 护甲熟练：所有护甲、盾牌\n";
        migrate_armor(&mut c, body);
        assert!(c
            .inventory
            .iter()
            .any(|i| i.name == "chain mail" && i.equipped));
        assert!(c.inventory.iter().any(|i| i.name == "shield" && i.equipped));
        assert_eq!(c.fighting_style.as_deref(), Some("defense"));
        c.fighting_style = Some("defense".into());
        recompute(&mut c, &book);
        assert_eq!(c.combat["ac"].as_i64(), Some(19));
        // Items do not explain the AC: keep it, style already included.
        let mut d = cf("2014");
        d.ac_base = 19;
        d.ac_base_armored = true;
        migrate_armor(&mut d, "- 护甲：链甲（chain mail）\n");
        assert!(d.inventory.is_empty() && d.ac_base_has_style);
        d.fighting_style = Some("defense".into());
        recompute(&mut d, &book);
        assert_eq!(d.combat["ac"].as_i64(), Some(19), "no second +1");
    }

    #[test]
    fn defense_dueling_archery() {
        let book = Book::srd("2014");
        let mut c = cf("2014");
        c.fighting_style = Some("defense".into());
        recompute(&mut c, &book);
        assert_eq!(c.combat["ac"].as_i64(), Some(11), "no armor, no Defense");
        c.inventory.push(Item {
            name: "chain mail".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        c.inventory.push(Item {
            name: "shield".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        recompute(&mut c, &book);
        assert_eq!(c.combat["ac"].as_i64(), Some(16 + 2 + 1));
        c.fighting_style = Some("dueling".into());
        c.inventory.push(Item {
            name: "longsword".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        recompute(&mut c, &book);
        assert_eq!(c.combat["ac"].as_i64(), Some(18));
        assert_eq!(dice(atk(&c, "longsword")), "1d8+5");
        assert_eq!(dice(atk(&c, "longsword-2h")), "1d10+3");
        c.fighting_style = Some("archery".into());
        c.inventory.push(Item {
            name: "longbow".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        recompute(&mut c, &book);
        assert_eq!(atk(&c, "longbow")["to_hit"].as_i64(), Some(1 + 2 + 2));
        assert_eq!(dice(atk(&c, "longsword")), "1d8+3");
        // Unequip removes the attack.
        c.inventory.retain(|i| i.name != "longbow");
        recompute(&mut c, &book);
        assert!(
            matches!(c.combat.get("attacks"), Some(Value::Sequence(v)) if !v.iter().any(|a| a["id"].as_str() == Some("longbow")))
        );
    }

    #[test]
    fn emitted_block_round_trips_and_keeps_flow_lines() {
        let mut c = cf("2014");
        c.inventory.push(Item {
            name: "greatsword".into(),
            qty: 1,
            equipped: true,
            ..Default::default()
        });
        recompute(&mut c, &Book::srd("2014"));
        let y = emit_block(&c.combat);
        assert!(y.contains("saves: {str: 5, con: 4}"), "{y}");
        assert!(y.contains("  - {id: greatsword"), "{y}");
        let back: Value = serde_norway::from_str(&y).unwrap();
        assert_eq!(back, c.combat);
        let _: combat_sim::content::schema::Creature = serde_norway::from_value(back).unwrap();
    }

    #[test]
    fn hp_transitions_add_and_remove_dying_conditions() {
        struct FixedRng(u32);
        impl DiceRng for FixedRng {
            fn roll_die(&mut self, sides: u32) -> u32 {
                assert_eq!(sides, 20);
                self.0
            }
        }

        let root = std::env::temp_dir().join(format!("talkdnd-dying-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let d = root.join("Saves/2030-01-01-01");
        fs::create_dir_all(&d).unwrap();
        fs::create_dir_all(root.join("Rules/combat/common")).unwrap();
        fs::write(
            root.join("Rules/combat/common/rules.yaml"),
            "rules:\n  dying_conditions: [unconscious]\n",
        )
        .unwrap();
        let book = Book::srd("2014");
        let mut cf = cf("2014");
        let mut ps = PartyState::default();
        ps.members.insert(
            "tess".into(),
            Member {
                name: "Tess".into(),
                hp: 12,
                max_hp: 12,
                ..Default::default()
            },
        );
        let mut out = Vec::new();
        let mut rng = FixedRng(1);

        apply(
            &d,
            &mut cf,
            &mut ps,
            &book,
            Op::Damage { amount: 12 },
            &mut rng,
            &mut out,
        )
        .unwrap();
        assert_eq!(ps.members["tess"].hp, 0);
        assert!(ps.members["tess"]
            .conditions
            .iter()
            .any(|c| c.condition == "unconscious"));

        apply(
            &d,
            &mut cf,
            &mut ps,
            &book,
            Op::Heal { amount: 5 },
            &mut rng,
            &mut out,
        )
        .unwrap();
        assert_eq!(ps.members["tess"].hp, 5);
        assert!(ps.members["tess"].conditions.is_empty());

        apply(
            &d,
            &mut cf,
            &mut ps,
            &book,
            Op::Damage { amount: 5 },
            &mut rng,
            &mut out,
        )
        .unwrap();
        apply(
            &d,
            &mut cf,
            &mut ps,
            &book,
            Op::DeathSave {
                result: "roll".into(),
            },
            &mut FixedRng(20),
            &mut out,
        )
        .unwrap();
        assert_eq!(ps.members["tess"].hp, 1);
        assert!(ps.members["tess"].conditions.is_empty());

        apply(
            &d,
            &mut cf,
            &mut ps,
            &book,
            Op::Damage { amount: 5 },
            &mut rng,
            &mut out,
        )
        .unwrap();
        assert!(ps.members["tess"]
            .conditions
            .iter()
            .any(|c| c.condition == "unconscious"));
        apply(
            &d,
            &mut cf,
            &mut ps,
            &book,
            Op::Rest {
                kind: "long".into(),
                hd: 0,
            },
            &mut rng,
            &mut out,
        )
        .unwrap();
        assert_eq!(ps.members["tess"].hp, 12);
        assert!(ps.members["tess"].conditions.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn split_old_sheet_keeps_body_and_tail() {
        let md = "# A\n\nbody\n\n## 战斗数据块（combat-sim）\n\n> x\n\n```combat yaml\nid: a\nhp: 3\n```\n\n## 升级记录\n\n- up\n";
        let (b, blk, t) = split_md(md);
        assert_eq!(b, "# A\n\nbody");
        assert_eq!(blk.unwrap()["hp"].as_i64(), Some(3));
        assert_eq!(t, "## 升级记录\n\n- up");
    }

    #[test]
    fn rerunning_chargen_sheet_keeps_play_data() {
        let d = std::env::temp_dir().join(format!("talkdnd-fromchargen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Characters")).unwrap();
        fs::create_dir_all(d.join("combat")).unwrap();
        let book = Book::srd("2014");
        let mut c = crate::chargen::Creation {
            ruleset: Some("2014".into()),
            species: Some("human".into()),
            class: Some("fighter".into()),
            background: Some("acolyte".into()),
            method: Some("array".into()),
            pool: data::STANDARD_ARRAY.to_vec(),
            level: 1,
            level_done: true,
            asi_done: true,
            spells_done: true,
            name: Some("Aria".into()),
            armor: Some("chain-mail".into()),
            shield: true,
            ..Default::default()
        };
        for (a, v) in ["str", "dex", "con", "int", "wis", "cha"]
            .iter()
            .zip(data::STANDARD_ARRAY)
        {
            c.scores.insert(a.to_string(), v);
        }
        let (id, md) = crate::chargen::sheet_with(&c, "p", &book, Some(&d)).unwrap();
        assert_eq!(id, "aria");
        let sheet = d.join("Characters/aria.md");
        fs::write(&sheet, &md).unwrap();
        let mut saved = from_chargen(&d, &c, &book, &sheet).unwrap();
        saved.inventory.push(Item {
            name: "potion of healing".into(),
            qty: 2,
            ..Default::default()
        });
        saved.notes.push("met the baron".into());
        saved.dm_notes = "secret".into();
        saved.feats.push("Alert".into());
        saved.languages.push("Elvish".into());
        saved.tools.push("thieves' tools".into());
        saved.fighting_style = Some("defense".into());
        store(&d, &saved).unwrap();

        // Re-running the creation sheet must not wipe play data.
        let again = from_chargen(&d, &c, &book, &sheet).unwrap();
        assert!(again
            .inventory
            .iter()
            .any(|i| i.name == "potion of healing" && i.qty == 2));
        assert!(again.notes.iter().any(|n| n == "met the baron"));
        assert_eq!(again.dm_notes, "secret");
        assert!(again.feats.iter().any(|f| f == "Alert"));
        assert!(again.languages.iter().any(|l| l == "Elvish"));
        assert!(again.tools.iter().any(|t| t == "thieves' tools"));
        assert_eq!(again.fighting_style.as_deref(), Some("defense"));
        let rendered = fs::read_to_string(&sheet).unwrap();
        assert!(rendered.contains("potion of healing"), "{rendered}");
        assert!(rendered.contains("met the baron"), "{rendered}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn store_rejects_path_traversal_ids() {
        let d = std::env::temp_dir().join(format!("talkdnd-badid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let mut c = cf("2014");
        c.id = "../../evil".into();
        assert!(store(&d, &c).is_err());
        assert!(!d.join("sheets").join("..").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn commit_rejects_sheet_paths_outside_the_save() {
        let base = std::env::temp_dir().join(format!("talkdnd-badfile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let d = base.join("save");
        fs::create_dir_all(&d).unwrap();
        let mut c = cf("2014");
        c.file = "../evil.md".into();
        assert!(commit(&d, &c, &PartyState::default()).is_err());
        assert!(!base.join("evil.md").exists());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn items_and_armor() {
        let i = parse_item("20 arrows");
        assert_eq!((i.qty, i.name.as_str(), i.ammo), (20, "arrows", true));
        assert_eq!(
            armor_of("studded leather armor").unwrap().0,
            "studded-leather"
        );
        assert_eq!(armor_of("皮甲").unwrap().0, "leather");
        assert!(armor_of("half plate").unwrap().0 == "half-plate");
        assert!(armor_of("rope").is_none());
        assert_eq!(add_flat("1d8+3", 2), "1d8+5");
    }

    #[test]
    fn item_add_requires_canon_or_a_registered_original() {
        let d = std::env::temp_dir().join(format!("talkdnd-origitem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for sub in ["Characters", "Sessions", "combat"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        fs::write(d.join("Sessions/s1.md"), "# log\n").unwrap();
        crate::migrate::init_with_deviation(
            &d,
            &crate::migrate::Deviation::new(Some(1), Some(25)).unwrap(),
        )
        .unwrap();
        fs::write(
            d.join("original.yaml"),
            "version: 1\nentries:\n  - {id: O-001, type: item, name: Moonblade, rarity: uncommon, chapter: 未分章, logged: \"2026-10-22 10:00\"}\n",
        )
        .unwrap();
        let book = Book::srd("2014");
        let mut c = cf("2014");
        let mut ps = PartyState::default();
        ps.members.insert(
            "tess".into(),
            Member {
                name: "Tess".into(),
                hp: 12,
                max_hp: 12,
                ..Default::default()
            },
        );
        let mut out = Vec::new();
        let mut rng = combat_sim::dice::seeded_rng(1);
        let item = |action: &str, name: &str, canon: bool, original: Option<&str>| Op::Item {
            action: action.into(),
            name: vec![name.into()],
            qty: None,
            equip: false,
            bonus: None,
            ac_bonus: None,
            ammo: false,
            canon,
            original: original.map(str::to_string),
        };
        apply(
            &d,
            &mut c,
            &mut ps,
            &book,
            item("add", "longsword", false, None),
            &mut rng,
            &mut out,
        )
        .unwrap();
        assert!(c.inventory.iter().any(|i| i.name == "longsword"));
        // Not in the data and not registered: refused with both hints.
        let e = apply(
            &d,
            &mut c,
            &mut ps,
            &book,
            item("add", "vorpal sword", false, None),
            &mut rng,
            &mut out,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("--canon") && e.contains("--original"), "{e}");
        assert!(!c.inventory.iter().any(|i| i.name == "vorpal sword"));
        // The DM checked the book: --canon accepts it.
        apply(
            &d,
            &mut c,
            &mut ps,
            &book,
            item("add", "vorpal sword", true, None),
            &mut rng,
            &mut out,
        )
        .unwrap();
        // A registered item original is accepted and keeps its DM-only marker.
        apply(
            &d,
            &mut c,
            &mut ps,
            &book,
            item("add", "Moonblade", false, Some("O-001")),
            &mut rng,
            &mut out,
        )
        .unwrap();
        let moon = c.inventory.iter().find(|i| i.name == "Moonblade").unwrap();
        assert_eq!(moon.original.as_deref(), Some("O-001"));
        let md = render(&c, None);
        assert!(!md.contains("O-001") && !md.contains("original"), "{md}");
        // Bogus ids are refused.
        for id in ["O-999", "O-002"] {
            let e = apply(
                &d,
                &mut c,
                &mut ps,
                &book,
                item("add", "Moonblade", false, Some(id)),
                &mut rng,
                &mut out,
            )
            .unwrap_err()
            .to_string();
            assert!(e.contains(id), "{e}");
        }
        // Common gear keeps working without flags.
        for name in ["rope", "torch", "20 arrows", "potion of healing"] {
            apply(
                &d,
                &mut c,
                &mut ps,
                &book,
                item("add", name, false, None),
                &mut rng,
                &mut out,
            )
            .unwrap();
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_newer_character_schema_is_refused_with_the_file_name() {
        let d = std::env::temp_dir().join(format!("talkdnd-schema-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Characters")).unwrap();
        fs::create_dir_all(d.join("sheets")).unwrap();
        fs::write(
            d.join("Characters/tess.md"),
            "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nkind: pc\n```\n",
        )
        .unwrap();
        fs::write(
            d.join("sheets/tess.yaml"),
            "version: 99\nid: tess\nname: Tess\nfile: Characters/tess.md\nruleset: \"2024\"\n",
        )
        .unwrap();
        let err = load(&d, "tess", &Book::srd("2024"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("tess.yaml") && err.contains("99"), "{err}");
        let _ = fs::remove_dir_all(&d);
    }
}
