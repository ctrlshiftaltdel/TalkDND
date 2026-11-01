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
    /// Who runs this party member: `player` (a player's character) or `dm`
    /// (a companion the DM plays). Both live in `Characters/`; the folder no
    /// longer tells them apart. Missing on older files: see [`infer_controller`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<String>,
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
    /// Structured spell lists (schema 10; docs/SPELLCASTING.md §3.5). The
    /// sheet's `spells` block is the only source of the character's lists;
    /// the Markdown `## 法术` section is a rendering, never the source.
    /// `None` on sheets written before schema 10; see
    /// [`crate::migrate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spells: Option<SpellLists>,
}

/// The `spells` block of a character sheet: mode, ability and the id lists
/// (schema 10). `version` is the block format (currently 1); ids match the
/// combat content ids. `CharFile.version` stays 1 — old binaries refuse the
/// whole save at `save-meta` (schema 10) before they could drop this block.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SpellLists {
    /// Block format version (currently 1).
    #[serde(default)]
    pub version: u32,
    /// How the class casts: `known`, `prepared` or `spellbook`.
    #[serde(default)]
    pub mode: String,
    /// Spellcasting ability (`wis`, ...). Audit copy of the class data;
    /// commands fail closed when it disagrees with `classes-<edition>.yaml`.
    #[serde(default)]
    pub ability: String,
    #[serde(default)]
    pub cantrips: Vec<String>,
    /// Prepared spells (prepared/spellbook modes).
    #[serde(default)]
    pub prepared: Vec<String>,
    /// Known spells (known mode).
    #[serde(default)]
    pub known: Vec<String>,
    /// Spellbook contents (spellbook mode; prepare from this).
    #[serde(default)]
    pub book: Vec<String>,
    /// Subclass grants: always prepared, not counted, not unpreparable.
    #[serde(default)]
    pub always_prepared: Vec<String>,
    /// Mystic Arcanum (warlock): one spell per unlocked ring 6..=9.
    #[serde(default)]
    pub arcanum: Vec<String>,
    /// 2014/2024 Land: the chosen terrain for the subclass's terrain-grouped
    /// always-prepared grants (serde default; additive within schema 10).
    /// Set with `spells terrain <地形>`; seeding gates on it (§3.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<String>,
    /// Old free-text entries that could not be mapped to ids (kept verbatim;
    /// the DM maps them with `spells adopt --map`, T3).
    #[serde(default)]
    pub legacy: Vec<String>,
}

impl SpellLists {
    /// The list that `mode` reads from: `known` for known mode, `book` for
    /// spellbook mode, else `prepared`.
    pub fn mode_list(&self) -> &Vec<String> {
        match self.mode.as_str() {
            "known" => &self.known,
            "spellbook" => &self.book,
            _ => &self.prepared,
        }
    }

    /// Fill the format version and keep every list present. Used by the
    /// schema-10 migration for sheets that already carry a block: the
    /// player's lists are never touched, only missing defaults are added.
    pub fn fill_defaults(&mut self) {
        if self.version == 0 {
            self.version = 1;
        }
    }
}

/// `controller` value for a player's character.
pub const PLAYER: &str = "player";
/// `controller` value for a companion run by the DM.
pub const DM: &str = "dm";

/// Controller of a party member whose structured file has no `controller`
/// yet: a sheet still under `NPCs/` or with `kind: npc` in its combat block
/// is a DM-run companion, everything else is a player's character.
pub fn infer_controller(file: &str, combat: &Value) -> &'static str {
    let file = file.replace('\\', "/");
    if file.starts_with("NPCs/") || combat.get("kind").and_then(|v| v.as_str()) == Some("npc") {
        DM
    } else {
        PLAYER
    }
}

/// Short Chinese label for a controller, used on sheets and in summaries.
pub fn controller_label(c: &str) -> &'static str {
    if c == DM {
        "DM 操控的同伴（companion）"
    } else {
        "玩家角色（player）"
    }
}

/// Controller of a sheet found on disk: `sheets/<id>.yaml` `controller`
/// when present, else inferred from its path and combat block.
pub fn controller_for(save_dir: &Path, s: &check::Sheet) -> &'static str {
    let from_file = fs::read_to_string(path(save_dir, &s.id))
        .ok()
        .and_then(|t| serde_norway::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v.get("controller")
                .and_then(|c| c.as_str())
                .map(str::to_string)
        });
    match from_file.as_deref() {
        Some(DM) => DM,
        Some(PLAYER) => PLAYER,
        _ => {
            let rel = s
                .file
                .strip_prefix(save_dir)
                .unwrap_or(&s.file)
                .to_string_lossy()
                .replace('\\', "/");
            infer_controller(&rel, &s.block)
        }
    }
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
    /// `player` or `dm`; see [`infer_controller`] for files without the field.
    pub fn controller(&self) -> &'static str {
        match self.controller.as_deref() {
            Some(DM) => DM,
            Some(PLAYER) => PLAYER,
            _ => infer_controller(&self.file, &self.combat),
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
        controller: Some(infer_controller(&rel, &block).to_string()),
        file: rel,
        ruleset: save::edition(save_dir)?,
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
    if let Some(c) = cf.controller.as_deref() {
        if c != PLAYER && c != DM {
            bail!(
                "{}: controller must be `player` or `dm` (got `{c}`)",
                p.display()
            );
        }
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
    render_with(cf, m, None)
}

/// Render the player-facing sheet. `ledger` supplies unidentified item
/// appearances: the structured `Item.name` and `Item.original` stay unchanged,
/// only the Markdown inventory table is masked. DM-facing commands still show
/// the real inventory.
pub fn render_with(
    cf: &CharFile,
    m: Option<&Member>,
    ledger: Option<&crate::identify::Ledger>,
) -> String {
    let mut md = cf.body_md.trim_end().to_string();
    md += "\n\n";
    md += BEGIN;
    md += "\n\n## 角色选项\n\n";
    md += &controller_line(cf.controller());
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
                item_display_name(it, ledger),
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

/// Player-facing inventory name. An unidentified item with a registered
/// original id shows its appearance (or a generic fallback); everything else
/// keeps its real name.
fn item_display_name(it: &Item, ledger: Option<&crate::identify::Ledger>) -> String {
    let Some(ledger) = ledger else {
        return if it.original.is_some() {
            "一件未鉴定的物品".into()
        } else {
            it.name.clone()
        };
    };
    let original = it.original.as_deref().unwrap_or("");
    match crate::identify::lookup(ledger, &[it.name.as_str(), original]) {
        Some(e) if e.identified => it.name.clone(),
        Some(e) => e
            .appearance
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if it.original.is_some() {
                    "一件未鉴定的物品".into()
                } else {
                    it.name.clone()
                }
            }),
        None => {
            if it.original.is_some() {
                "一件未鉴定的物品".into()
            } else {
                it.name.clone()
            }
        }
    }
}

/// The first line of the generated part: who runs this party member.
pub fn controller_line(controller: &str) -> String {
    format!("- **操控**：{}\n", controller_label(controller))
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
    let ledger = crate::identify::load(save_dir).ok();
    let md = render_with(cf, ps.members.get(&cf.id), ledger.as_ref());
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
    if cf.controller.is_none() {
        cf.controller = fresh.controller.clone();
    }
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
    put(&mut cf, "subclass", &c.subclass);
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
    // Structured spell lists follow the creation state (schema 10 block);
    // re-running the sheet without structured lists keeps the existing block.
    if let Some(lists) = &c.spell_lists {
        cf.spells = Some(lists.clone());
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

// ---------- v1.6 T3: spell list commands (docs/SPELLCASTING.md §3.3/§3.4/§3.9/§4/§5.3) ----------

/// Class context for the spell commands: the class data, the spell universe
/// (§3.9) and the class-derived generation (slots, pact magic, arcanum) at
/// the sheet's level.
struct SpellClass<'a> {
    class: String,
    cd: &'a classdata::ClassDef,
    universe: BTreeMap<String, u8>,
    gen: classdata::Generated,
    level: i32,
    subclass: Option<String>,
    /// Distinct terrains of the selected subclass's grants (empty without
    /// terrain-grouped grants).
    terrains: Vec<String>,
    /// The character's chosen terrain (`lists.terrain`).
    terrain: Option<String>,
}

impl<'a> SpellClass<'a> {
    /// Resolve the sheet's class. Fails closed for unknown classes,
    /// non-casting classes and source-data gaps (§3.3/§3.9).
    fn open(cf: &CharFile, book: &'a Book) -> Result<Self> {
        let class = cf.class().ok_or_else(|| {
            anyhow!(
                "{} 没有职业（先 `talkdnd char {} set class <职业>`）",
                cf.name,
                cf.id
            )
        })?;
        let cd = book.class(&class).ok_or_else(|| {
            anyhow!(
                "职业 `{class}` 在 {} 数据里没有定义：本地数据缺失时先 `talkdnd data import`",
                cf.ruleset
            )
        })?;
        if classdata::spell_mode(cd).is_none() {
            bail!("职业 `{class}` 不是施法职业：没有法术列表命令");
        }
        // The block's ability is an audit copy (§3.5): fail closed on drift.
        if let Some(lists) = &cf.spells {
            if !lists.ability.is_empty() {
                let eff = effective_ability(cd, &class);
                if lists.ability != eff {
                    bail!(
                        "spells 块 ability `{}` 与职业数据 `{eff}` 不一致：fail closed（先修复角色卡或重新迁移）",
                        lists.ability
                    );
                }
            }
        }
        let subclass = subclass_of_cf(cf, cd);
        let terrain = cf.spells.as_ref().and_then(|l| l.terrain.clone());
        // Subclass-aware universe (§3.9): only the selected subclass's
        // grants enter the sources; class-level features_grants always do,
        // both only once their `from_level` is reached. Terrain-grouped
        // grants (Land) additionally require the chosen terrain.
        let universe = classdata::spell_universe(
            book,
            cd,
            subclass.as_deref(),
            cf.level(),
            terrain.as_deref(),
        );
        let terrains = classdata::subclass_terrains(cd, subclass.as_deref());
        let mut scores = BTreeMap::new();
        if let Some(abilities) = cf.combat.get("abilities") {
            for a in data::ABILITIES {
                scores.insert(
                    a.to_string(),
                    abilities.get(a).and_then(Value::as_i64).unwrap_or(10) as i32,
                );
            }
        }
        let creation = crate::chargen::Creation {
            ruleset: Some(cf.ruleset.clone()),
            class: Some(class.clone()),
            subclass: subclass.clone(),
            level: cf.level(),
            scores,
            ..Default::default()
        };
        let gen = classdata::generate(&creation, book, None);
        Ok(Self {
            class,
            cd,
            universe,
            gen,
            level: cf.level(),
            subclass,
            terrains,
            terrain,
        })
    }

    /// Highest spell ring the character can currently cast: the pact slot
    /// level for pact casters, else the highest non-empty slot ring.
    fn max_castable(&self) -> i32 {
        if let Some((_, l)) = self.gen.pact {
            return l;
        }
        self.gen
            .slots
            .iter()
            .rposition(|n| *n > 0)
            .map(|i| i as i32 + 1)
            .unwrap_or(0)
    }

    /// Prepared/known limit at this level (`g.spells`).
    fn list_cap(&self) -> Option<usize> {
        self.gen.spells.map(|(_, n)| n as usize)
    }

    fn cantrip_cap(&self) -> Option<usize> {
        self.gen.cantrips.map(|n| n as usize)
    }

    /// Precise id resolution against the spell universe (D-9: exact id or
    /// slug; no fuzzy matching). Fails closed with the §3.9 message.
    fn source_resolve(&self, spell: &str) -> Result<String> {
        let spell = spell.trim();
        if self.universe.is_empty() {
            bail!(
                "职业 `{}` 没有法术来源数据（职业表与授予声明都缺失）：拒绝校验（docs/SPELLCASTING.md §3.9）",
                self.class
            );
        }
        if let Some(id) = crate::chargen::map_spell_text(spell, &self.universe) {
            return Ok(id);
        }
        // Failed: give the terrain-specific errors before the generic one
        // (§3.9 Land: terrain spells only enter the universe once a terrain
        // is chosen).
        let wanted = crate::chargen::slug(spell);
        if let Some(g) = self
            .subclass
            .as_deref()
            .and_then(|s| self.cd.subclasses.get(s))
            .into_iter()
            .flat_map(|s| s.spells.iter())
            .find(|g| {
                g.terrain.is_some()
                    && g.id
                        .as_deref()
                        .is_some_and(|id| crate::chargen::slug(id) == wanted)
            })
        {
            let gterrain = g.terrain.as_deref().unwrap_or_default();
            if self.terrain.is_none() {
                bail!(
                    "「{spell}」是 {} 子职的地形法术（{gterrain}）：先 `spells terrain <地形>` 选定地形（可选：{}）",
                    self.subclass.as_deref().unwrap_or_default(),
                    self.terrains.join("、")
                );
            }
            if gterrain != self.terrain.as_deref().unwrap_or_default() {
                bail!(
                    "「{spell}」属于 {} 子职的 {gterrain} 地形授予，与当前地形 {} 不符：`spells terrain <地形>` 可切换",
                    self.subclass.as_deref().unwrap_or_default(),
                    self.terrain.as_deref().unwrap_or_default()
                );
            }
        }
        bail!(
            "「{spell}」不在 {} 的法术来源内（基础职业表/子职授予/Magical Secrets；本地数据缺失时先 `talkdnd data import`）",
            self.class
        )
    }

    /// Level of a universe id (0 = cantrip).
    fn level_of(&self, id: &str) -> u8 {
        self.universe.get(id).copied().unwrap_or(0)
    }

    fn cap_text(&self, n: usize, cap: Option<usize>) -> String {
        match cap {
            Some(max) => format!("{n}/{max}"),
            None => format!("{n}"),
        }
    }
}

/// Effective spellcasting ability: the class data, else the built-in SRD
/// class table (the classes-*.yaml files do not repeat it).
fn effective_ability(cd: &classdata::ClassDef, class: &str) -> String {
    cd.spell_ability
        .clone()
        .or_else(|| {
            data::class(class)
                .and_then(|c| c.spell_ability)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// Subclass id of a sheet: `info["subclass"]` when present, else the
/// `（<subclass>）` marker in the sheet header (same fallback as
/// `progress::subclass_of`).
pub(crate) fn subclass_of_cf(cf: &CharFile, cd: &classdata::ClassDef) -> Option<String> {
    if let Some(s) = cf.info.get("subclass") {
        let slug = crate::chargen::slug(s);
        if !slug.is_empty() {
            return Some(slug);
        }
    }
    let header = cf
        .body_md
        .lines()
        .find(|l| l.contains("**等级**") || l.to_lowercase().contains("**level**"))?;
    let lower = header.to_lowercase();
    cd.subclasses
        .keys()
        .find(|id| {
            let id = id.to_lowercase();
            lower.contains(&format!("({id})")) || lower.contains(&format!("（{id}）"))
        })
        .cloned()
}

/// The sheet's structured lists; every list command except `adopt` requires
/// the schema-10 block and refuses to create it silently.
fn spells_block_mut(cf: &mut CharFile) -> Result<&mut SpellLists> {
    cf.spells.as_mut().ok_or_else(|| {
        anyhow!(
            "{} 还没有 spells 法术列表块（schema 10）：先运行 `talkdnd migrate`（整档迁移），或用 `spells adopt` 从正文采纳",
            cf.name
        )
    })
}

/// §3.9 seed candidates: the selected subclass's unlocked always_prepared
/// grants that are not yet in any list (add-only; a spell the player
/// prepared by hand keeps its list and stays unpreparable). Read-only —
/// callers decide whether to apply, so sync --dry-run can report without
/// writing.
fn pending_always_prepared(
    sc: &SpellClass,
    terrain: Option<&str>,
    lists: &SpellLists,
) -> Vec<String> {
    classdata::always_prepared_grants(sc.cd, sc.subclass.as_deref(), sc.level, terrain)
        .into_iter()
        .filter(|id| {
            !lists.always_prepared.contains(id)
                && !lists.cantrips.contains(id)
                && !lists.known.contains(id)
                && !lists.prepared.contains(id)
                && !lists.book.contains(id)
                && !lists.arcanum.contains(id)
        })
        .collect()
}

/// A desired spell ref: target ability id plus the optional ref-level cost
/// override (pact slots and Mystic Arcanum, §3.10).
struct RefPlan {
    id: String,
    level: u8,
    cost: Option<(String, i32)>,
}

/// The castable set per mode (§4.2): known mode casts from `known`,
/// prepared/spellbook from `prepared` (the book itself is not castable),
/// plus cantrips, subclass grants and arcanum in every mode. Sorted and
/// de-duplicated.
pub fn castable_ids(lists: &SpellLists) -> Vec<String> {
    let mut ids: Vec<String> = lists.cantrips.clone();
    ids.extend(match lists.mode.as_str() {
        "known" => lists.known.clone(),
        _ => lists.prepared.clone(),
    });
    ids.extend(lists.always_prepared.iter().cloned());
    ids.extend(lists.arcanum.iter().cloned());
    ids.sort();
    ids.dedup();
    ids
}

/// The ref-level cost override for a castable id (§3.10): arcanum spells
/// cost their `arcanum-N` resource; other leveled spells of a pact caster
/// cost one pact slot of level L. Cantrips never cost a slot.
pub fn spell_ref_cost(
    _id: &str,
    level: u8,
    is_arcanum: bool,
    pact: Option<(i32, i32)>,
) -> Option<(String, i32)> {
    if is_arcanum && level >= 6 {
        Some((format!("arcanum-{level}"), 1))
    } else if level >= 1 {
        pact.map(|(_, l)| (format!("spell-slot-{l}"), 1))
    } else {
        None
    }
}

fn desired_refs(lists: &SpellLists, sc: Option<&SpellClass>) -> Vec<RefPlan> {
    let pact = sc.and_then(|s| s.gen.pact);
    let mut plan: Vec<RefPlan> = castable_ids(lists)
        .into_iter()
        .map(|id| {
            let (level, is_arcanum) = match sc {
                Some(sc) => (sc.level_of(&id), lists.arcanum.contains(&id)),
                None => (0, lists.arcanum.contains(&id)),
            };
            let cost = spell_ref_cost(&id, level, is_arcanum, pact);
            RefPlan { id, level, cost }
        })
        .collect();
    plan.sort_by_key(|r| (r.level, r.id.clone()));
    plan
}

/// The ref as a YAML mapping value (no list dashes) for direct insertion
/// into a combat block sequence.
fn ref_value(id: &str, cost: &Option<(String, i32)>) -> Value {
    let line = match cost {
        Some((res, amount)) => {
            format!("{{ref: {id}, cost: {{resource: {res}, amount: {amount}}}}}")
        }
        None => format!("{{ref: {id}}}"),
    };
    serde_norway::from_str(&line).expect("ref yaml")
}

pub fn cost_ref_line(id: &str, cost: &Option<(String, i32)>) -> String {
    match cost {
        Some((res, amount)) => {
            format!("  - {{ref: {id}, cost: {{resource: {res}, amount: {amount}}}}}")
        }
        None => format!("  - {{ref: {id}}}"),
    }
}

/// Read-only text/JSON view for `spells` / `spells --json` (§4.3): fixed
/// keys, lists sorted by (level, id), counts against the class limits.
/// Never writes anything.
pub fn spells_view(cf: &CharFile, ps: &PartyState, book: &Book, json: bool) -> Result<Vec<String>> {
    let sc = SpellClass::open(cf, book)?;
    let lists = cf.spells.as_ref().ok_or_else(|| {
        anyhow!(
            "{} 还没有 spells 法术列表块（schema 10）：先运行 `talkdnd migrate`（整档迁移），或用 `spells adopt` 从正文采纳",
            cf.name
        )
    })?;
    let sort_list = |v: &[String]| {
        let mut v: Vec<String> = v.to_vec();
        v.sort_by_key(|id| (sc.level_of(id), id.clone()));
        v
    };
    let cantrips = sort_list(&lists.cantrips);
    let known = sort_list(&lists.known);
    let prepared = sort_list(&lists.prepared);
    let book_list = sort_list(&lists.book);
    let always = sort_list(&lists.always_prepared);
    let mut arcanum: Vec<(u8, String)> = lists
        .arcanum
        .iter()
        .map(|id| (sc.level_of(id), id.clone()))
        .collect();
    arcanum.sort();
    let (mode_label, mode_list) = match lists.mode.as_str() {
        "known" => ("已知", sort_list(&lists.known)),
        "spellbook" => ("已准备", sort_list(&lists.prepared)),
        _ => ("可准备", sort_list(&lists.prepared)),
    };
    let member = ps.members.get(&cf.id);
    let slots: Vec<(String, i32, i32, String)> = member
        .map(|m| {
            m.resources
                .iter()
                .filter(|(k, _)| k.starts_with("spell-slot-") || k.starts_with("arcanum-"))
                .map(|(k, r)| (k.clone(), r.current, r.max, r.recharge.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if json {
        let j = serde_json::json!({
            "mode": lists.mode,
            "ability": lists.ability,
            "cantrips": cantrips,
            "known": known,
            "prepared": prepared,
            "book": book_list,
            "always_prepared": always,
            "terrain": lists.terrain,
            "arcanum": arcanum.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>(),
            "legacy": lists.legacy,
            "slots": slots
                .iter()
                .map(|(id, cur, max, rc)| {
                    serde_json::json!({ "id": id, "current": cur, "max": max, "recharge": rc })
                })
                .collect::<Vec<_>>(),
            "counts": {
                "cantrips": counts_obj(cantrips.len(), sc.cantrip_cap()),
                "known": counts_obj(
                    known.len(),
                    sc.list_cap().filter(|_| lists.mode == "known"),
                ),
                "prepared": counts_obj(
                    prepared.len(),
                    sc.list_cap().filter(|_| lists.mode != "known"),
                ),
                "book": counts_obj(book_list.len(), None),
                "always_prepared": counts_obj(always.len(), None),
                "arcanum": counts_obj(arcanum.len(), None),
                "legacy": counts_obj(lists.legacy.len(), None),
            },
        });
        return Ok(vec![serde_json::to_string(&j).expect("json serializes")]);
    }
    let join = |v: &[String]| {
        if v.is_empty() {
            "—".to_string()
        } else {
            v.join("、")
        }
    };
    let mut out = Vec::new();
    out.push(format!(
        "{}（{} {} 级）法术：mode={}，属性={}，可施法至 {} 环",
        cf.name,
        sc.class,
        sc.level,
        lists.mode,
        lists.ability,
        sc.max_castable()
    ));
    if !sc.terrains.is_empty() {
        match &lists.terrain {
            Some(t) => out.push(format!(
                "地形：{t}（`spells terrain <地形>` 可切换；可选：{}）",
                sc.terrains.join("、")
            )),
            None => out.push(format!(
                "地形：未选（{} 子职的法术按地形分组，先 `spells terrain <地形>`；可选：{}）",
                sc.subclass.as_deref().unwrap_or_default(),
                sc.terrains.join("、")
            )),
        }
    }
    out.push(format!(
        "戏法（{}）：{}",
        sc.cap_text(cantrips.len(), sc.cantrip_cap()),
        join(&cantrips)
    ));
    out.push(format!(
        "{mode_label}（{}）：{}",
        sc.cap_text(mode_list.len(), sc.list_cap()),
        join(&mode_list)
    ));
    out.push(format!(
        "法术书（{}）：{}",
        book_list.len(),
        join(&book_list)
    ));
    out.push(format!(
        "始终准备（{}，子职授予，不计数量）：{}",
        always.len(),
        join(&always)
    ));
    let arc = if arcanum.is_empty() {
        "—".to_string()
    } else {
        arcanum
            .iter()
            .map(|(l, id)| format!("{l}环:{id}"))
            .collect::<Vec<_>>()
            .join("　")
    };
    out.push(format!("奥术（Mystic Arcanum，每环一个）：{arc}"));
    if lists.legacy.is_empty() {
        out.push("旧文本（legacy）：—".to_string());
    } else {
        out.push(format!(
            "旧文本（legacy，`spells adopt --map 原名=id` 采纳）：{}",
            lists.legacy.join("、")
        ));
    }
    if slots.is_empty() {
        out.push("法术位：—".to_string());
    } else {
        let pact = sc.gen.pact;
        let mut parts: Vec<String> = Vec::new();
        for (id, cur, max, _) in &slots {
            if let Some(level) = id.strip_prefix("spell-slot-") {
                if pact.is_some_and(|(_, l)| l.to_string() == level) {
                    parts.push(format!("契约法术位（{level} 环，短休）：{cur}/{max}"));
                    continue;
                }
                parts.push(format!("{level}环 {cur}/{max}"));
            } else {
                parts.push(format!("{id} {cur}/{max}（长休）"));
            }
        }
        out.push(format!("法术位：{}", parts.join("　")));
    }
    Ok(out)
}

fn counts_obj(n: usize, max: Option<usize>) -> serde_json::Value {
    match max {
        Some(max) => serde_json::json!({ "n": n, "max": max }),
        None => serde_json::json!({ "n": n }),
    }
}

/// `spells adopt` (D-9): precise matching only. Seeds the block from the
/// sheet body's `## 法术` when it is missing (the sheet itself is never
/// rewritten), applies `--map 原名=id` to legacy entries and re-maps the
/// rest against the universe. Unmatched entries stay in `legacy`.
fn spells_adopt(cf: &mut CharFile, book: &Book, maps: &[String]) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let newly = cf.spells.is_none();
    if newly {
        let mode = classdata::spell_mode(sc.cd).unwrap_or_default();
        let texts = crate::chargen::markdown_spells(&cf.body_md);
        cf.spells = Some(SpellLists {
            version: 1,
            mode: mode.clone(),
            ability: effective_ability(sc.cd, &sc.class),
            ..Default::default()
        });
        if texts.is_empty() {
            return Ok(format!(
                "已创建 spells 块（mode {mode}）；正文没有 `## 法术` 列表，无条目可采纳"
            ));
        }
        // Seed legacy with the precise-mapping leftovers so the code below
        // treats new and existing blocks the same way.
        let (mapped, leftover) = crate::chargen::map_spell_texts(&texts, &sc.universe);
        route_mapped(cf, &mapped, &sc)?;
        cf.spells.as_mut().expect("just set").legacy = leftover;
    }
    let lists = cf.spells.as_mut().expect("block exists");
    let mut legacy = std::mem::take(&mut lists.legacy);
    let mut adopted = 0usize;
    for m in maps {
        let (orig, id) = m
            .split_once('=')
            .map(|(a, b)| (a.trim(), b.trim()))
            .ok_or_else(|| anyhow!("--map 需要 `原名=spell-id`（得到 `{m}`）"))?;
        if id.is_empty() || orig.is_empty() {
            bail!("--map 需要 `原名=spell-id`（得到 `{m}`）");
        }
        let id = sc.source_resolve(id)?;
        let pos = legacy
            .iter()
            .position(|t| t == orig || crate::chargen::slug(t) == crate::chargen::slug(orig))
            .ok_or_else(|| {
                anyhow!(
                    "legacy 里没有「{orig}」（当前：{}）",
                    if legacy.is_empty() {
                        "—".to_string()
                    } else {
                        legacy.join("、")
                    }
                )
            })?;
        legacy.remove(pos);
        adopted += 1;
        route_one(lists, &sc, &id);
    }
    let (mapped, leftover) = crate::chargen::map_spell_texts(&legacy, &sc.universe);
    adopted += mapped.len();
    route_mapped(cf, &mapped, &sc)?;
    let lists = cf.spells.as_mut().expect("block exists");
    lists.legacy = leftover;
    dedupe_lists(lists);
    let lists = cf.spells.as_ref().expect("block exists");
    let mode_list = match lists.mode.as_str() {
        "known" => &lists.known,
        "spellbook" => &lists.book,
        _ => &lists.prepared,
    };
    Ok(format!(
        "采纳 {adopted} 个（新建块：{}；cantrips {}，列表 {}，书 {}）；legacy 剩 {}：{}",
        if newly { "是" } else { "否" },
        lists.cantrips.len(),
        mode_list.len(),
        lists.book.len(),
        lists.legacy.len(),
        if lists.legacy.is_empty() {
            "—".to_string()
        } else {
            lists.legacy.join("、")
        }
    ))
}

/// Route one universe id into the right list by level and mode (same
/// routing as the schema-10 migration). Returns false when the id is
/// already listed (the caller keeps the legacy entry clean anyway).
fn route_one(lists: &mut SpellLists, sc: &SpellClass, id: &str) -> bool {
    let level = sc.level_of(id);
    let there = |v: &Vec<String>| v.iter().any(|s| s == id);
    if there(&lists.cantrips)
        || there(&lists.known)
        || there(&lists.prepared)
        || there(&lists.book)
        || there(&lists.always_prepared)
    {
        return false;
    }
    let target = if level == 0 {
        &mut lists.cantrips
    } else {
        match lists.mode.as_str() {
            "known" => &mut lists.known,
            "spellbook" => &mut lists.book,
            _ => &mut lists.prepared,
        }
    };
    target.push(id.to_string());
    true
}

/// Route a batch of mapped ids, tolerating ones already present (they may
/// have been adopted before).
fn route_mapped(cf: &mut CharFile, mapped: &[String], sc: &SpellClass) -> Result<()> {
    for id in mapped {
        let lists = cf.spells.as_mut().expect("block exists");
        let level = sc.level_of(id);
        let there = |v: &Vec<String>| v.iter().any(|s| s == id);
        if there(&lists.cantrips)
            || there(&lists.known)
            || there(&lists.prepared)
            || there(&lists.book)
            || there(&lists.always_prepared)
        {
            continue;
        }
        let target = if level == 0 {
            &mut lists.cantrips
        } else {
            match lists.mode.as_str() {
                "known" => &mut lists.known,
                "spellbook" => &mut lists.book,
                _ => &mut lists.prepared,
            }
        };
        target.push(id.clone());
    }
    Ok(())
}

fn dedupe_lists(lists: &mut SpellLists) {
    fn d(v: &mut Vec<String>) {
        v.sort();
        v.dedup();
    }
    d(&mut lists.cantrips);
    d(&mut lists.known);
    d(&mut lists.prepared);
    d(&mut lists.book);
    d(&mut lists.always_prepared);
}

/// `spells sync`: rebuild the combat block's spell refs from the lists
/// (§4.2). The managed set is the union of the lists (cantrips ∪ known ∪
/// prepared ∪ always_prepared ∪ arcanum): a ref inside it that is no
/// longer castable is removed, one that is castable is added, refreshed or
/// de-duplicated. Refs outside the union (hand-written) and inline
/// definitions are kept and logged. Dry-run prints the plan without
/// writing anything.
fn spells_sync(
    cf: &mut CharFile,
    book: &Book,
    save_dir: &Path,
    dry_run: bool,
    out: &mut Vec<String>,
) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    spells_block_mut(cf)?;
    let tag = if dry_run { "[dry-run] " } else { "" };
    // §3.9 backfill: seed the selected subclass's unlocked always_prepared
    // grants (T2 blocks migrate with the list empty). Dry-run includes the
    // seeds in its plan but writes nothing.
    let pending = {
        let lists = cf.spells.as_ref().expect("block exists");
        pending_always_prepared(&sc, sc.terrain.as_deref(), lists)
    };
    let plan = if dry_run {
        let mut probe = cf.spells.clone().expect("block exists");
        probe.always_prepared.extend(pending.iter().cloned());
        desired_refs(&probe, Some(&sc))
    } else {
        if !pending.is_empty() {
            cf.spells
                .as_mut()
                .expect("block exists")
                .always_prepared
                .extend(pending.iter().cloned());
        }
        desired_refs(cf.spells.as_ref().expect("block exists"), Some(&sc))
    };
    for id in &pending {
        out.push(format!(
            "{tag}播种子职 always_prepared：{id}（不占准备数量，不可 unprepare）"
        ));
    }
    let mut refreshed = 0usize;
    let mut deduped = 0usize;
    let mut removed = 0usize;
    let mut kept = 0usize;
    // Best-effort registry: added refs are warned about when the content
    // does not define them (U-7; content check stays the authority).
    let registry = match load_save_registry(save_dir) {
        Ok(r) => Some(r),
        Err(e) => {
            out.push(format!(
                "{tag}warning: 内容注册表不可用（{e}）；未校验 ref 目标是否存在"
            ));
            None
        }
    };
    let known_ability = |id: &str| {
        registry
            .as_ref()
            .is_some_and(|r| r.abilities.get(id).is_some())
    };
    // §4.2: managed = cantrips ∪ known ∪ prepared ∪ always_prepared ∪
    // arcanum — wider than the castable set, so a stale list entry (e.g. a
    // leftover known-mode spell under prepared mode) still identifies its
    // ref and gets removed instead of being kept as "unmanaged".
    let lists_ref = cf.spells.as_ref().expect("block exists");
    let mut managed: std::collections::BTreeSet<String> = Default::default();
    for id in lists_ref
        .cantrips
        .iter()
        .chain(lists_ref.known.iter())
        .chain(lists_ref.prepared.iter())
        .chain(lists_ref.always_prepared.iter())
        .chain(lists_ref.arcanum.iter())
    {
        managed.insert(id.clone());
    }
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for key in ["actions", "features"] {
        let Some(Value::Sequence(entries)) = cf.combat.get_mut(key) else {
            continue;
        };
        let mut remove: Vec<usize> = Vec::new();
        for (i, entry) in entries.iter_mut().enumerate() {
            let target = entry.get("ref").and_then(Value::as_str);
            let inline = entry.get("id").and_then(Value::as_str);
            if let Some(target) = target {
                if !plan.iter().any(|r| r.id == target) && managed.contains(target) {
                    // In the list union but no longer castable: stale.
                    remove.push(i);
                    removed += 1;
                    out.push(format!(
                        "{tag}移除受管但不可施放的 stale ref：{target}（{key}[{i}]）"
                    ));
                } else if managed.contains(target) {
                    if seen.contains(target) {
                        remove.push(i);
                        deduped += 1;
                        out.push(format!("{tag}移除重复法术 ref：{target}（{key}[{i}]）"));
                    } else {
                        seen.insert(target.to_string());
                        let desired = plan
                            .iter()
                            .find(|r| r.id == target)
                            .and_then(|r| r.cost.clone());
                        let actual = entry.get("cost").map(|c| {
                            (
                                c.get("resource")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                c.get("amount").and_then(Value::as_i64).unwrap_or(1) as i32,
                            )
                        });
                        if actual != desired {
                            refreshed += 1;
                            out.push(format!(
                                "{tag}刷新法术 ref：{target}（{}）",
                                match &desired {
                                    Some((res, amount)) => format!("成本覆盖 {res} ×{amount}"),
                                    None => "恢复能力自带成本".to_string(),
                                }
                            ));
                            if !dry_run {
                                if let Some(cost) = &desired {
                                    entry["cost"] = serde_norway::from_str::<Value>(&format!(
                                        "{{resource: {res}, amount: {amount}}}",
                                        res = cost.0,
                                        amount = cost.1
                                    ))
                                    .expect("cost yaml");
                                } else {
                                    entry.as_mapping_mut().map(|o| o.remove("cost"));
                                }
                            }
                        }
                    }
                } else {
                    kept += 1;
                    out.push(format!("{tag}未受管 ref，未修改：{target}"));
                }
            } else if let Some(inline) = inline {
                if managed.contains(inline) {
                    seen.insert(inline.to_string());
                    out.push(format!(
                        "{tag}inline 法术定义，未修改：{inline}（不再重复添加 ref）"
                    ));
                }
            }
        }
        if !remove.is_empty() && !dry_run {
            for i in remove.into_iter().rev() {
                entries.remove(i);
            }
        }
    }
    let missing: Vec<&RefPlan> = plan.iter().filter(|r| !seen.contains(&r.id)).collect();
    for r in &missing {
        if !known_ability(&r.id) {
            out.push(format!(
                "{tag}warning: {} 不在内容注册表（施放前先 content check / 补数据）",
                r.id
            ));
        }
    }
    let added = missing.len();
    if !dry_run && !missing.is_empty() {
        let new_entries: Vec<Value> = missing.iter().map(|r| ref_value(&r.id, &r.cost)).collect();
        match cf.combat.get_mut("actions") {
            Some(Value::Sequence(entries)) => {
                entries.extend(new_entries);
            }
            _ => {
                if let Value::Mapping(map) = &mut cf.combat {
                    map.insert(
                        Value::String("actions".into()),
                        Value::Sequence(new_entries),
                    );
                }
            }
        }
    }
    Ok(format!(
        "{tag}sync 完成：新增 {added}、刷新 {refreshed}、移除重复 {deduped}、移除 stale {removed}；未受管/inline 保留 {kept} 条{}",
        if dry_run { "（未写入任何文件）" } else { "" }
    ))
}

/// Remove every combat-block ref to `id` (actions and features). Returns
/// how many entries were removed. Inline definitions are left alone.
fn drop_spell_refs(cf: &mut CharFile, id: &str) -> usize {
    let mut removed = 0;
    for key in ["actions", "features"] {
        let Some(Value::Sequence(entries)) = cf.combat.get_mut(key) else {
            continue;
        };
        let before = entries.len();
        entries.retain(|e| e.get("ref").and_then(Value::as_str) != Some(id));
        removed += before - entries.len();
    }
    removed
}

/// The save's content registry via its campaign manifest (U-7). Fails when
/// the manifest or the content is missing; callers degrade with a warning
/// or fail closed, never silently.
fn load_save_registry(save_dir: &Path) -> Result<combat_sim::content::ContentRegistry> {
    combat_sim::content::ContentSources {
        campaign: Some(save_dir.join("combat").join("campaign.yaml")),
        ..Default::default()
    }
    .load()
}

/// `prepare`: add to the prepared list (§3.4/§3.9/§4.3; D-8 hints only).
fn spells_prepare(
    cf: &mut CharFile,
    book: &Book,
    spell: &str,
    out: &mut Vec<String>,
) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let lists = spells_block_mut(cf)?;
    match lists.mode.as_str() {
        "prepared" | "spellbook" => {}
        "known" => bail!("{} 是已知法术（known）模式：用 `learn`", sc.class),
        other => bail!("spells 块 mode `{other}` 无法识别（known|prepared|spellbook）"),
    }
    let id = sc.source_resolve(spell)?;
    let level = sc.level_of(&id) as i32;
    if level == 0 {
        bail!("「{id}」是戏法，无需准备（戏法在建卡时选择）");
    }
    let max = sc.max_castable();
    if level > max {
        bail!("「{id}」是 {level} 环法术，高于当前可施法环级 {max}");
    }
    if lists.always_prepared.contains(&id) {
        bail!("「{id}」是子职授予的 always_prepared，无需准备也不可 unprepare");
    }
    let cap = sc.list_cap();
    if lists.prepared.contains(&id) {
        return Ok(format!(
            "已准备 {id}（prepared {}）",
            sc.cap_text(lists.prepared.len(), cap)
        ));
    }
    if lists.mode == "spellbook" && !lists.book.contains(&id) {
        bail!(
            "「{id}」不在法术书：先 `talkdnd char {} scribe {id}`",
            cf.id
        );
    }
    if let Some(cap) = cap {
        if lists.prepared.len() >= cap {
            bail!(
                "超过可准备数量：{} 个（上限 {cap}）；先 `unprepare` 一个再准备",
                lists.prepared.len()
            );
        }
    }
    lists.prepared.push(id.clone());
    out.push(
        "提示：按 RAW 准备/替换法术在长休时进行（工具不强制刚长休，D-8）；新法术施放前记得 `spells sync`。"
            .into(),
    );
    Ok(format!(
        "准备 {id}（prepared {}）",
        sc.cap_text(lists.prepared.len(), cap)
    ))
}

/// `unprepare`: remove from the prepared list. `always_prepared` refuses;
/// removing an unprepared spell is an idempotent no-op (§4.3).
fn spells_unprepare(cf: &mut CharFile, book: &Book, spell: &str) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let lists = spells_block_mut(cf)?;
    match lists.mode.as_str() {
        "prepared" | "spellbook" => {}
        "known" => bail!("{} 是已知法术（known）模式：没有准备列表可取消", sc.class),
        other => bail!("spells 块 mode `{other}` 无法识别（known|prepared|spellbook）"),
    }
    let wanted = spell.trim();
    let slug = crate::chargen::slug(wanted);
    if let Some(id) = lists
        .prepared
        .iter()
        .find(|s| s.as_str() == wanted || crate::chargen::slug(s) == slug)
        .cloned()
    {
        if lists.always_prepared.contains(&id) {
            bail!("「{id}」是子职授予的 always_prepared，不可 unprepare");
        }
        lists.prepared.retain(|s| *s != id);
        let count = sc.cap_text(lists.prepared.len(), sc.list_cap());
        // §4.2-3: an unprepared spell must not stay castable through a stale
        // ref — drop the spell's refs from the combat block in the same
        // atomic sheet write. Hand-written refs for other spells are not
        // touched here (spells sync is the ref builder).
        let removed = drop_spell_refs(cf, &id);
        let note = if removed > 0 {
            format!("；同步移除 ref ×{removed}（未准备即不可施放）")
        } else {
            String::new()
        };
        return Ok(format!("取消准备 {id}（prepared {count}）{note}"));
    }
    if lists
        .always_prepared
        .iter()
        .any(|s| s == wanted || crate::chargen::slug(s) == slug)
    {
        bail!("「{wanted}」是子职授予的 always_prepared，不可 unprepare");
    }
    Ok(format!(
        "未准备 {wanted}（prepared {}）",
        sc.cap_text(lists.prepared.len(), sc.list_cap())
    ))
}

/// `spells terrain <地形>` (§3.9 Land): choose the terrain for the
/// subclass's terrain-grouped always-prepared grants. The valid terrains
/// come from the grant data (nothing hardcoded); the choice is stored in
/// the spells block and the newly unlocked grants are seeded in the same
/// atomic write.
fn spells_terrain(cf: &mut CharFile, book: &Book, terrain: &str) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let lists = spells_block_mut(cf)?;
    let wanted = crate::chargen::slug(terrain);
    if sc.terrains.is_empty() {
        let anywhere = sc
            .cd
            .subclasses
            .values()
            .any(|s| s.spells.iter().any(|g| g.terrain.is_some()));
        if anywhere {
            bail!(
                "职业 `{}` 的地形法术按子职分组：当前子职没有地形授予；先选有地形授予的子职（如 land），再 `spells terrain <地形>`",
                sc.class
            );
        }
        bail!("职业 `{}` 没有按地形分组的授予法术：没有地形可选", sc.class);
    }
    if !sc.terrains.contains(&wanted) {
        bail!(
            "「{terrain}」不是 {} 的可选地形（可选：{}）",
            sc.subclass.as_deref().unwrap_or_default(),
            sc.terrains.join("、")
        );
    }
    if lists.terrain.as_deref() == Some(wanted.as_str()) {
        return Ok(format!(
            "地形已是 {wanted}（无变化；可选：{}）",
            sc.terrains.join("、")
        ));
    }
    lists.terrain = Some(wanted.clone());
    // Switching terrain retires the previous terrain's seeded grants: drop
    // every terrain-grouped grant id of the selected subclass from
    // always_prepared (that list is grant-managed only). The same id the
    // player prepared/learned/scribed by hand keeps its other list.
    let terrain_granted = classdata::terrain_granted_ids(sc.cd, sc.subclass.as_deref(), sc.level);
    let removed: Vec<String> = lists
        .always_prepared
        .iter()
        .filter(|id| terrain_granted.contains(*id))
        .cloned()
        .collect();
    lists
        .always_prepared
        .retain(|id| !terrain_granted.contains(id));
    let seeded = pending_always_prepared(&sc, Some(&wanted), lists);
    let mut notes: Vec<String> = Vec::new();
    if !removed.is_empty() {
        notes.push(format!("移除旧地形授予：{}", removed.join("、")));
    }
    if !seeded.is_empty() {
        lists.always_prepared.extend(seeded.iter().cloned());
        notes.push(format!(
            "播种 always_prepared：{}（不占准备数量，施放前记得 `spells sync`）",
            seeded.join("、")
        ));
    }
    let note = if notes.is_empty() {
        String::new()
    } else {
        format!("；{}", notes.join("；"))
    };
    Ok(format!(
        "地形设为 {wanted}（可选：{}）{note}",
        sc.terrains.join("、")
    ))
}

/// `learn`: add to the known list (known classes); Mystic Arcanum picks for
/// warlock route to `arcanum` (one per unlocked ring, §3.10).
fn spells_learn(cf: &mut CharFile, book: &Book, spell: &str) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let lists = spells_block_mut(cf)?;
    match lists.mode.as_str() {
        "known" | "prepared" | "spellbook" => {}
        other => bail!("spells 块 mode `{other}` 无法识别（known|prepared|spellbook）"),
    }
    let id = sc.source_resolve(spell)?;
    let level = sc.level_of(&id) as i32;
    if sc.cd.arcanum && level >= 6 {
        let unlock = [(6, 11), (7, 13), (8, 15), (9, 17)]
            .iter()
            .find(|(ring, _)| *ring == level)
            .map(|(_, lvl)| *lvl)
            .unwrap_or(0);
        if sc.level < unlock {
            bail!(
                "Mystic Arcanum（{level} 环）{unlock} 级解锁（当前 {} 级）",
                sc.level
            );
        }
        if lists.arcanum.contains(&id) {
            bail!("「{id}」已在奥术列表：每环至多一个（arcanum 重复，拒绝）");
        }
        if let Some(existing) = lists
            .arcanum
            .iter()
            .find(|s| sc.level_of(s) as i32 == level)
        {
            bail!(
                "arcanum-{level} 已有「{existing}」：每环至多一个（替换由 DM 处理，docs/SPELLCASTING.md §3.10）"
            );
        }
        lists.arcanum.push(id.clone());
        return Ok(format!(
            "奥术 {id} → arcanum-{level}（每环至多一个，长休恢复；记得 `spells sync` 刷新 ref 成本）"
        ));
    }
    match lists.mode.as_str() {
        "known" => {}
        m => bail!(
            "{} 是 {} 模式：没有已知法术列表（用 `prepare`{}）",
            sc.class,
            m,
            if m == "spellbook" {
                "，抄录用 `scribe`"
            } else {
                ""
            }
        ),
    }
    if level == 0 {
        bail!("戏法在建卡时选择（`chargen set spells`）；v1.6 不提供戏法替换命令");
    }
    let max = sc.max_castable();
    if level > max {
        bail!("「{id}」是 {level} 环法术，高于当前可施法环级 {max}");
    }
    let cap = sc.list_cap();
    if lists.known.contains(&id) {
        return Ok(format!(
            "已学会 {id}（known {}）",
            sc.cap_text(lists.known.len(), cap)
        ));
    }
    if let Some(cap) = cap {
        if lists.known.len() >= cap {
            bail!("超过已知法术数量：{} 个（上限 {cap}）", lists.known.len());
        }
    }
    lists.known.push(id.clone());
    Ok(format!(
        "学会 {id}（known {}）；记得 `spells sync` 刷新角色卡 refs",
        sc.cap_text(lists.known.len(), cap)
    ))
}

/// `scribe`: copy a spell into the wizard spellbook (§3.4): validates the
/// source and level, pre-checks the purse, then spends gold. Never prepares.
fn spells_scribe(
    cf: &mut CharFile,
    book: &Book,
    save_dir: &Path,
    spell: &str,
    hours: Option<i32>,
    cost_gp: Option<i64>,
    out: &mut Vec<String>,
) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let scribe = sc.cd.scribe.ok_or_else(|| {
        anyhow!(
            "只有 wizard 能抄录法术进书（{} 没有 scribe 数据）",
            sc.class
        )
    })?;
    {
        let lists = spells_block_mut(cf)?;
        if lists.mode != "spellbook" {
            bail!("只有法术书模式（spellbook，wizard）能抄录法术");
        }
    }
    let id = sc.source_resolve(spell)?;
    let level = sc.level_of(&id) as i32;
    if level == 0 {
        bail!("「{id}」是戏法，不进法术书");
    }
    let max = sc.max_castable();
    if level > max {
        bail!("「{id}」是 {level} 环法术，高于当前可施法环级 {max}");
    }
    {
        let lists = spells_block_mut(cf)?;
        if lists.book.contains(&id) {
            bail!("「{id}」已在法术书");
        }
    }
    let hours = hours.unwrap_or(scribe.hours_per_level * level);
    let gp = cost_gp.unwrap_or(i64::from(scribe.cost_gp_per_level) * i64::from(level));
    if gp > 0 {
        let cp = crate::progress::to_cp(0, gp, 0, 0, 0);
        let p = crate::progress::Progress::load(save_dir)?;
        let bal = p.purses.get(&cf.id).map(|x| x.cp).unwrap_or(0);
        if bal < cp {
            bail!(
                "金币不足：抄录 {id}（{level} 环）需要 {gp} gp，钱包只有 {}",
                crate::progress::coins(bal)
            );
        }
        out.extend(
            crate::progress::loot(
                save_dir,
                crate::progress::LootOp::Spend {
                    from: &cf.id,
                    cp,
                    items: &[],
                },
                Some(&format!("抄录 {id} 进法术书")),
            )?
            .into_iter()
            .take(1),
        );
    }
    let lists = spells_block_mut(cf)?;
    lists.book.push(id.clone());
    Ok(format!(
        "抄录 {id} → 法术书（book {}）；花费 {gp} gp、{hours} 小时；未准备",
        lists.book.len()
    ))
}

/// `ritual`: validate ritual casting per edition (§5.3) and log it. No slot
/// is spent, no effect is resolved, the world time does not move; a 2014
/// warlock (invocation-granted rituals, D-12) fails closed for the DM.
fn spells_ritual(cf: &mut CharFile, book: &Book, save_dir: &Path, spell: &str) -> Result<String> {
    let sc = SpellClass::open(cf, book)?;
    let wanted = spell.trim();
    let slug = crate::chargen::slug(wanted);
    let edition = cf.ruleset.clone();
    let (id, list_name) = {
        let lists = spells_block_mut(cf)?;
        let matches = |v: &str| v == wanted || crate::chargen::slug(v) == slug;
        if edition == "2014" {
            let (list, list_name): (&Vec<String>, &str) = match sc.cd.ritual.as_deref() {
                Some("known") => {
                    if lists.mode != "known" {
                        bail!(
                            "2014 {} 仪式资格是 known：与 spells 块 mode {} 不一致",
                            sc.class,
                            lists.mode
                        );
                    }
                    (&lists.known, "已知法术")
                }
                Some("prepared") => {
                    if lists.mode != "prepared" && lists.mode != "spellbook" {
                        bail!(
                            "2014 {} 仪式资格是 prepared：与 spells 块 mode {} 不一致",
                            sc.class,
                            lists.mode
                        );
                    }
                    (&lists.prepared, "已准备法术")
                }
                Some("book") => {
                    if lists.mode != "spellbook" {
                        bail!(
                            "2014 {} 仪式资格是 book：与 spells 块 mode {} 不一致",
                            sc.class,
                            lists.mode
                        );
                    }
                    (&lists.book, "法术书")
                }
                other => bail!(
                    "2014 {} 没有职业级仪式施法特性（ritual = {}；魔祈授予的仪式为非目标，D-12）：DM 手动裁定并记日志",
                    sc.class,
                    other.unwrap_or("none")
                ),
            };
            let ritual_value = sc.cd.ritual.as_deref().unwrap_or("none");
            let found = list.iter().find(|s| matches(s)).cloned().ok_or_else(|| {
                anyhow!(
                    "「{wanted}」不在{list_name}里：2014 {} 仪式要求法术在{list_name}（ritual = {ritual_value}）",
                    sc.class
                )
            })?;
            (found, list_name.to_string())
        } else if let Some(found) = lists.prepared.iter().find(|s| matches(s)) {
            (found.clone(), "已准备法术".to_string())
        } else if sc.cd.ritual_adept {
            let found = lists
                .book
                .iter()
                .find(|s| matches(s))
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "「{wanted}」不在已准备或法术书里：2024 仪式要求法术已准备（Ritual Adept 另允许书内未准备）"
                    )
                })?;
            (found, "法术书（Ritual Adept）".to_string())
        } else {
            bail!("「{wanted}」不在已准备法术里：2024 仪式要求法术已准备（Ritual 标签 + 已准备）");
        }
    };
    let registry = load_save_registry(save_dir).map_err(|e| {
        anyhow!(
            "无法校验「{id}」是否为仪式法术（内容注册表不可用：{e}）：失败关闭；DM 可手动裁定并记日志"
        )
    })?;
    let ability = registry.abilities.get(&id).ok_or_else(|| {
        anyhow!(
            "内容注册表没有法术「{id}」（先 `talkdnd start` 的 content check / `talkdnd data import`）"
        )
    })?;
    let is_ritual = ability.spell.as_ref().map(|s| s.ritual).unwrap_or(false)
        || ability.tags.iter().any(|t| t == "ritual");
    if !is_ritual {
        bail!(
            "「{id}」没有仪式（Ritual）标签：仪式施放要求法术带 Ritual 标签（来源：{list_name}）"
        );
    }
    if ability.spell.as_ref().map(|s| s.level).unwrap_or(0) == 0
        && ability.tags.iter().any(|t| t == "cantrip")
    {
        bail!("「{id}」是戏法：戏法没有仪式");
    }
    Ok(format!(
        "仪式 {id}（正常施法时间 + 10 分钟，不消耗法术位；效果由 DM 叙述）"
    ))
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
    /// Spell lists (v1.6 T3, docs/SPELLCASTING.md §4): `spells` shows them,
    /// `spells --json` is the stable machine view, `spells adopt` maps old
    /// free-text names precisely, `spells sync` rebuilds the combat block's
    /// spell refs from the lists
    Spells {
        /// Stable JSON view (fixed keys, sorted lists); same as `spells show --json`
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        cmd: Option<SpellsCmd>,
    },
    /// Add a prepared spell: `prepare bless` (prepared/spellbook classes)
    Prepare { spell: String },
    /// Remove a prepared spell: `unprepare bless` (`always_prepared` refuses)
    Unprepare { spell: String },
    /// Add a known spell (known classes) or a Mystic Arcanum pick (warlock,
    /// rings 6-9): `learn magic-missile`
    Learn { spell: String },
    /// Copy a spell into the wizard spellbook (costs gold/time; not prepared):
    /// `scribe magic-missile` (override with `--hours N --cost-gp N`)
    Scribe {
        spell: String,
        #[arg(long)]
        hours: Option<i32>,
        #[arg(long, allow_hyphen_values = true)]
        cost_gp: Option<i64>,
    },
    /// Ritual-cast a spell (validated and logged; no slot spent, no effects):
    /// `ritual detect-magic`
    Ritual { spell: String },
}

/// Subcommands of `talkdnd char <name> spells`.
#[derive(clap::Subcommand, Clone, Debug)]
pub enum SpellsCmd {
    /// Show the lists (default; read-only)
    Show {
        /// Stable JSON view (fixed keys, sorted lists)
        #[arg(long)]
        json: bool,
    },
    /// Adopt free-text names into the lists (D-9: exact id/slug/name match,
    /// no fuzzy matching; unmatched entries stay in `legacy`):
    /// `spells adopt --map "旧名字=magic-missile"`
    Adopt {
        /// Map a legacy entry to a spell id: `--map 原名=spell-id` (repeatable)
        #[arg(long = "map")]
        maps: Vec<String>,
    },
    /// Rebuild the combat block's spell refs from the lists (managed set
    /// only; unmanaged refs are kept and logged): `spells sync --dry-run`
    Sync {
        /// Print the planned ref changes without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Choose the terrain for a Land subclass's terrain-grouped
    /// always-prepared spells (2014: arctic/coast/…; 2024: arid/polar/…):
    /// `spells terrain arctic`
    Terrain { terrain: String },
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
        Op::Spells {
            json: _,
            cmd: Some(SpellsCmd::Adopt { maps }),
        } => spells_adopt(cf, book, &maps)?,
        Op::Spells {
            json: _,
            cmd: Some(SpellsCmd::Sync { dry_run }),
        } => spells_sync(cf, book, save_dir, dry_run, out)?,
        Op::Spells {
            json: _,
            cmd: Some(SpellsCmd::Terrain { terrain }),
        } => spells_terrain(cf, book, &terrain)?,
        Op::Spells { json, cmd } => {
            let json = json || matches!(cmd, Some(SpellsCmd::Show { json: true }));
            let lines = spells_view(cf, ps, book, json)?;
            out.extend(lines);
            String::new()
        }
        Op::Prepare { spell } => spells_prepare(cf, book, &spell, out)?,
        Op::Unprepare { spell } => spells_unprepare(cf, book, &spell)?,
        Op::Learn { spell } => spells_learn(cf, book, &spell)?,
        Op::Scribe {
            spell,
            hours,
            cost_gp,
        } => spells_scribe(cf, book, save_dir, &spell, hours, cost_gp, out)?,
        Op::Ritual { spell } => spells_ritual(cf, book, save_dir, &spell)?,
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
    let rs = save::edition(save_dir)?;
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
    // `spells` / `spells --json` is a read-only view (§4.3): it never
    // writes the sheet, the party state or the rendered Markdown.
    if let [Op::Spells { json, cmd }] = &ops[..] {
        if !matches!(
            cmd,
            Some(SpellsCmd::Adopt { .. } | SpellsCmd::Sync { .. } | SpellsCmd::Terrain { .. })
        ) {
            let json = *json || matches!(cmd, Some(SpellsCmd::Show { json: true }));
            return spells_view(&cf, &ps, &book, json);
        }
    }
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
    fn unidentified_original_item_uses_appearance_in_the_player_sheet() {
        let d = std::env::temp_dir().join(format!("talkdnd-itemmask-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for sub in ["Characters", "combat"] {
            fs::create_dir_all(d.join(sub)).unwrap();
        }
        fs::write(
            d.join("identified.yaml"),
            "version: 1\nentries:\n  - {name: Moonblade, identified: false, appearance: 一把没有光泽的弯刀}\n",
        )
        .unwrap();
        let mut c = cf("2014");
        c.inventory.push(Item {
            name: "Moonblade".into(),
            original: Some("O-001".into()),
            ..Default::default()
        });
        c.inventory.push(Item {
            name: "Silver Key".into(),
            original: Some("O-002".into()),
            ..Default::default()
        });
        let ps = PartyState::default();
        commit(&d, &c, &ps).unwrap();
        let md = fs::read_to_string(d.join("Characters/Tess.md")).unwrap();
        assert!(md.contains("一把没有光泽的弯刀"), "{md}");
        assert!(!md.contains("Moonblade"), "{md}");
        assert!(md.contains("一件未鉴定的物品"), "{md}");
        assert!(!md.contains("Silver Key"), "{md}");
        let stored = fs::read_to_string(d.join("sheets/tess.yaml")).unwrap();
        assert!(stored.contains("name: Moonblade"), "{stored}");
        assert!(stored.contains("name: Silver Key"), "{stored}");

        crate::identify::identify(&d, "Moonblade", "check", None).unwrap();
        commit(&d, &c, &ps).unwrap();
        let md = fs::read_to_string(d.join("Characters/Tess.md")).unwrap();
        assert!(md.contains("Moonblade"), "{md}");
        assert!(!md.contains("一把没有光泽的弯刀"), "{md}");
        assert!(md.contains("一件未鉴定的物品"), "{md}");
        assert!(!md.contains("Silver Key"), "{md}");
        let _ = fs::remove_dir_all(&d);
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
