//! Class progression, weapons, and the attacks/features/resources they produce.
//!
//! Tracked data (`data/*.yaml`, embedded at build time) covers SRD 5.1 and SRD 5.2 only.
//! Non-SRD content comes from local, gitignored files in `Rules/chargen/local/*.yaml`
//! (written by `talkdnd data import` or by hand) and is merged over the SRD data at runtime.

use crate::chargen::Creation;
use crate::data;
use anyhow::{Context, Result};
use combat_sim::fsutil::atomic_write;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const WEAPONS: &str = include_str!("../data/weapons.yaml");
const CLASSES_2014: &str = include_str!("../data/classes-2014.yaml");
const CLASSES_2024: &str = include_str!("../data/classes-2024.yaml");
const ADVENTURES_YAML: &str = include_str!("../data/adventures.yaml");

/// Local data folder, relative to the LetsPlay root.
pub const LOCAL_DIR: &str = "Rules/chargen/local";
/// Version of the local-book importer. `import_stale` re-runs older imports.
pub const IMPORTER_VERSION: u32 = 2;

/// `[level, name, summary]`
pub type Feature = (i32, String, String);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeaponOverride {
    #[serde(default)]
    pub dice: Option<String>,
    #[serde(default)]
    pub v: Option<String>,
    #[serde(default)]
    pub properties: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Weapon {
    pub id: String,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub ranged: bool,
    pub dice: String,
    #[serde(default)]
    pub v: Option<String>,
    #[serde(rename = "type")]
    pub dtype: String,
    #[serde(default)]
    pub properties: Vec<String>,
    #[serde(default)]
    pub range: Option<[i32; 2]>,
    #[serde(default)]
    pub mastery: Option<String>,
    #[serde(default)]
    pub r2024: Option<WeaponOverride>,
}

impl Weapon {
    pub fn for_ruleset(&self, rs: &str) -> Weapon {
        let mut w = self.clone();
        if rs == "2024" {
            if let Some(o) = &self.r2024 {
                if let Some(d) = &o.dice {
                    w.dice = d.clone();
                }
                if o.v.is_some() {
                    w.v = o.v.clone();
                }
                if let Some(p) = &o.properties {
                    w.properties = p.clone();
                }
            }
        } else {
            w.mastery = None;
        }
        w
    }
    pub fn has(&self, p: &str) -> bool {
        self.properties.iter().any(|x| x == p)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Count {
    Table(Vec<i32>),
    Formula(String),
    #[default]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Res {
    pub id: String,
    pub name: String,
    pub recharge: String,
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub by_level: Option<Vec<i32>>,
    #[serde(default)]
    pub formula: Option<String>,
    #[serde(default)]
    pub short_regain: Option<i32>,
    #[serde(default)]
    pub short_from: Option<i32>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sub {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Armor proficiencies granted by this subclass (e.g. 2014 Life: heavy).
    #[serde(default)]
    pub armor_proficiency: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClassDef {
    /// Needed only for classes not in the built-in SRD list.
    #[serde(default)]
    pub hit_die: Option<i32>,
    #[serde(default)]
    pub caster: Option<String>,
    #[serde(default)]
    pub spell_ability: Option<String>,
    #[serde(default)]
    pub weapons: Vec<String>,
    #[serde(default)]
    pub mastery: Option<Vec<i32>>,
    #[serde(default)]
    pub asi: Option<Vec<i32>>,
    #[serde(default)]
    pub cantrips: Option<Vec<i32>>,
    #[serde(default)]
    pub known: Option<Vec<i32>>,
    #[serde(default)]
    pub prepared: Count,
    #[serde(default)]
    pub martial_arts: Option<Vec<i32>>,
    #[serde(default)]
    pub subclass_level: Option<i32>,
    #[serde(default)]
    pub features: Vec<Feature>,
    #[serde(default)]
    pub resources: Vec<Res>,
    #[serde(default)]
    pub subclasses: BTreeMap<String, Sub>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataFile {
    /// Importer that generated this file (`IMPORTER_VERSION`); 0 = unknown/old.
    #[serde(default)]
    pub importer_version: u32,
    /// "2014", "2024" or "any".
    #[serde(default)]
    pub ruleset: Option<String>,
    #[serde(default)]
    pub classes: BTreeMap<String, ClassDef>,
    #[serde(default)]
    pub weapons: Vec<Weapon>,
    /// Backgrounds from the player's own books (local imports only).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub backgrounds: BTreeMap<String, crate::options::BackgroundDef>,
}

/// Suggested starting level range for an adventure (facts only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Adventure {
    pub start_level_min: i32,
    pub start_level_max: i32,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AdventuresData {
    #[serde(default)]
    adventures: BTreeMap<String, Adventure>,
}

/// Adventure start-level facts for a slug, from embedded `data/adventures.yaml`.
static ADVENTURES_CACHE: std::sync::OnceLock<BTreeMap<String, Adventure>> =
    std::sync::OnceLock::new();

/// Adventure start-level facts for a slug, from embedded `data/adventures.yaml`.
pub fn adventure(id: &str) -> Option<&'static Adventure> {
    let map = ADVENTURES_CACHE.get_or_init(|| {
        let data: AdventuresData =
            serde_norway::from_str(ADVENTURES_YAML).expect("embedded adventure data");
        data.adventures
    });
    map.get(id)
}

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub rs: String,
    pub classes: BTreeMap<String, ClassDef>,
    pub weapons: Vec<Weapon>,
    pub backgrounds: BTreeMap<String, crate::options::BackgroundDef>,
    pub local_files: Vec<PathBuf>,
    /// Ids that came from local files (not SRD).
    pub local_ids: Vec<String>,
}

fn at(v: &[i32], lvl: i32) -> i32 {
    let i = (lvl.clamp(1, 20) - 1) as usize;
    v.get(i).or(v.last()).copied().unwrap_or(0)
}

impl Book {
    /// SRD data only.
    pub fn srd(rs: &str) -> Book {
        let classes: DataFile = serde_norway::from_str(if rs == "2014" {
            CLASSES_2014
        } else {
            CLASSES_2024
        })
        .expect("embedded class data");
        let weapons: DataFile = serde_norway::from_str(WEAPONS).expect("embedded weapon data");
        Book {
            rs: rs.to_string(),
            classes: classes.classes,
            weapons: weapons.weapons,
            ..Default::default()
        }
    }

    /// SRD data plus `<root>/Rules/chargen/local/*.yaml` for this ruleset.
    pub fn load(rs: &str, root: Option<&Path>) -> Result<Book> {
        let mut b = Book::srd(rs);
        let Some(root) = root else { return Ok(b) };
        let dir = root.join(LOCAL_DIR);
        let Ok(rd) = fs::read_dir(&dir) else {
            return Ok(b);
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
            .collect();
        files.sort();
        for f in files {
            let df: DataFile = serde_norway::from_str(&fs::read_to_string(&f)?)
                .with_context(|| format!("{} is not valid local chargen data", f.display()))?;
            let r = df.ruleset.as_deref().unwrap_or("any");
            if r != "any" && r != rs {
                continue;
            }
            b.merge(df);
            b.local_files.push(f);
        }
        Ok(b)
    }

    pub fn merge(&mut self, df: DataFile) {
        for (id, c) in df.classes {
            match self.classes.get_mut(&id) {
                Some(have) => {
                    for (sid, s) in c.subclasses {
                        self.local_ids.push(format!("{id}/{sid}"));
                        let e = have.subclasses.entry(sid).or_default();
                        if s.name.is_some() {
                            e.name = s.name;
                        }
                        if !s.armor_proficiency.is_empty() {
                            e.armor_proficiency = s.armor_proficiency;
                        }
                        e.features.extend(s.features);
                    }
                    have.features.extend(c.features);
                    have.resources.extend(c.resources);
                }
                None => {
                    self.local_ids.push(id.clone());
                    for sid in c.subclasses.keys() {
                        self.local_ids.push(format!("{id}/{sid}"));
                    }
                    self.classes.insert(id, c);
                }
            }
        }
        for (id, b) in df.backgrounds {
            self.local_ids.push(format!("background/{id}"));
            self.backgrounds.insert(id, b);
        }
        for w in df.weapons {
            self.local_ids.push(w.id.clone());
            self.weapons.retain(|x| x.id != w.id);
            self.weapons.push(w);
        }
    }

    pub fn class(&self, id: &str) -> Option<&ClassDef> {
        self.classes.get(id)
    }
}

/// The exact command(s) a DM agent should run when local data is missing.
pub fn import_hint(root: Option<&Path>, rs: &str) -> String {
    let have_md = root
        .map(|r| md_files(&r.join("Rules").join(rs)).is_empty())
        .map(|empty| !empty)
        .unwrap_or(false);
    if have_md {
        format!("run `talkdnd data import --ruleset {rs}`, then `talkdnd chargen sheet` again (see docs/chargen.md「本地规则书数据」)")
    } else {
        format!("no converted Markdown for Rules/{rs} yet (Rules/{rs}/<slug>/index/<书名>.pages.md): run `uv run --project tools/pdf2md python -m pdf2md run --book \"Players Handbook\" --budget 300`, then `talkdnd data import --ruleset {rs}`; until then keep SRD data and fill the gap by hand (see docs/chargen.md「本地规则书数据」)")
    }
}

/// Full-rulebook Markdown for a ruleset: the paged Markdown of every
/// converted book under `Rules/<rs>/<slug>/index/` (legacy `.md/<book>/`
/// still read), without the SRD. `<a id="pN"></a>` anchors = PDF page N.
pub fn rulebooks(rules: &Path) -> Vec<PathBuf> {
    let mut v = md_files(rules);
    v.retain(|p| {
        !p.file_name()
            .is_some_and(|n| n.to_string_lossy().contains("SRD"))
    });
    v
}

/// Whether the local import is missing, made by an older importer, or older
/// than the converted books.
pub fn import_stale(root: &Path, rs: &str) -> bool {
    let md = md_files(&root.join("Rules").join(rs));
    if md.is_empty() {
        return false;
    }
    let out = root.join(LOCAL_DIR).join(format!("imported-{rs}.yaml"));
    let Ok(text) = fs::read_to_string(&out) else {
        return true;
    };
    let Ok(old) = serde_norway::from_str::<DataFile>(&text) else {
        return true;
    };
    if old.importer_version < IMPORTER_VERSION {
        return true;
    }
    let Ok(t_out) = fs::metadata(&out).and_then(|m| m.modified()) else {
        return true;
    };
    md.iter().any(|f| {
        fs::metadata(f)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t > t_out)
    })
}

/// Build `Rules/chargen/local/imported-<rs>.yaml` from the player's books.
/// A worse re-import (fewer subclasses + backgrounds) keeps the old file as
/// `.bak` and warns; the new file is still written atomically.
pub fn run_import(root: &Path, rs: &str) -> Result<(PathBuf, usize, usize)> {
    let md = md_files(&root.join("Rules").join(rs));
    if md.is_empty() {
        anyhow::bail!("{}", import_hint(Some(root), rs));
    }
    let srd = Book::srd(rs);
    let mut df = import_markdown(rs, &srd, &md)?;
    df.importer_version = IMPORTER_VERSION;
    let dir = root.join(LOCAL_DIR);
    fs::create_dir_all(&dir)?;
    let out = dir.join(format!("imported-{rs}.yaml"));
    let subs = df
        .classes
        .values()
        .map(|c| c.subclasses.len())
        .sum::<usize>();
    let bgs = df.backgrounds.len();
    let new_count = subs + bgs;
    if let Ok(text) = fs::read_to_string(&out) {
        if let Ok(old) = serde_norway::from_str::<DataFile>(&text) {
            let old_count = old
                .classes
                .values()
                .map(|c| c.subclasses.len())
                .sum::<usize>()
                + old.backgrounds.len();
            if new_count < old_count {
                let backup = dir.join(format!("imported-{rs}.yaml.bak"));
                fs::copy(&out, &backup).with_context(|| format!("backup {}", out.display()))?;
                eprintln!(
                    "warning: 这次导入只有 {new_count} 项（子职+背景），旧文件有 {old_count} 项；旧文件已保留为 {}",
                    backup.display()
                );
            }
        }
    }
    let head = format!("# Generated by `talkdnd data import --ruleset {rs}` from your own books. LOCAL ONLY: never commit.\n# Names, item lists and levels only; check them against the book. Hand edits belong in another file (e.g. manual-{rs}.yaml), this one is overwritten.\n");
    atomic_write(&out, (head + &serde_norway::to_string(&df)?).as_bytes())?;
    Ok((out, subs, bgs))
}

/// Converted books of one ruleset folder (`Rules/<rs>/`): the paged
/// Markdown in each `<slug>/index/` (or a legacy `.md/<book>/`).
pub fn md_files(rules: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = crate::library::category_indexes(rules)
        .iter()
        .filter_map(|(_, d)| crate::library::paged_markdown(d))
        .collect();
    v.sort();
    v.dedup();
    v
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttackOut {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub ability: String,
    pub to_hit: i32,
    pub dice: String,
    pub dtype: String,
    pub reach: Option<i32>,
    pub range: Option<[i32; 2]>,
    pub properties: Vec<String>,
    pub mastery: Option<String>,
    pub note: String,
    pub proficient: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResOut {
    pub id: String,
    pub name: String,
    pub max: i32,
    pub recharge: String,
    pub short_regain: Option<i32>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Generated {
    pub attacks: Vec<AttackOut>,
    pub features: Vec<Feature>,
    pub resources: Vec<ResOut>,
    /// Slots by spell level 1..=9 (pact slots go into `pact`).
    pub slots: Vec<i32>,
    /// (count, slot level)
    pub pact: Option<(i32, i32)>,
    pub cantrips: Option<i32>,
    pub spells: Option<(&'static str, i32)>,
    pub mastery: Option<i32>,
    /// Combat-library ability refs: (ref, params yaml, is_action)
    pub refs: Vec<(String, Option<String>, bool)>,
    pub warnings: Vec<String>,
}

const FULL: [[i32; 9]; 20] = [
    [2, 0, 0, 0, 0, 0, 0, 0, 0],
    [3, 0, 0, 0, 0, 0, 0, 0, 0],
    [4, 2, 0, 0, 0, 0, 0, 0, 0],
    [4, 3, 0, 0, 0, 0, 0, 0, 0],
    [4, 3, 2, 0, 0, 0, 0, 0, 0],
    [4, 3, 3, 0, 0, 0, 0, 0, 0],
    [4, 3, 3, 1, 0, 0, 0, 0, 0],
    [4, 3, 3, 2, 0, 0, 0, 0, 0],
    [4, 3, 3, 3, 1, 0, 0, 0, 0],
    [4, 3, 3, 3, 2, 0, 0, 0, 0],
    [4, 3, 3, 3, 2, 1, 0, 0, 0],
    [4, 3, 3, 3, 2, 1, 0, 0, 0],
    [4, 3, 3, 3, 2, 1, 1, 0, 0],
    [4, 3, 3, 3, 2, 1, 1, 0, 0],
    [4, 3, 3, 3, 2, 1, 1, 1, 0],
    [4, 3, 3, 3, 2, 1, 1, 1, 0],
    [4, 3, 3, 3, 2, 1, 1, 1, 1],
    [4, 3, 3, 3, 3, 1, 1, 1, 1],
    [4, 3, 3, 3, 3, 2, 1, 1, 1],
    [4, 3, 3, 3, 3, 2, 2, 1, 1],
];

/// Spell slots for a caster type at a class level.
pub fn slots(caster: &str, rs: &str, lvl: i32) -> Vec<i32> {
    let eff = match caster {
        "full" => lvl,
        "half" if rs == "2014" && lvl < 2 => 0,
        "half" => (lvl + 1) / 2,
        _ => 0,
    };
    if eff <= 0 {
        return Vec::new();
    }
    let row = FULL[(eff.min(20) - 1) as usize];
    let n = row.iter().rposition(|x| *x > 0).map(|i| i + 1).unwrap_or(0);
    row[..n].to_vec()
}

pub fn pact(lvl: i32) -> (i32, i32) {
    let count = match lvl {
        ..=1 => 1,
        2..=10 => 2,
        11..=16 => 3,
        _ => 4,
    };
    let level = ((lvl + 1) / 2).min(5);
    (count, level)
}

fn formula(f: &str, lvl: i32, prof: i32, mods: &BTreeMap<String, i32>) -> i32 {
    let f = f.replace(' ', "");
    let m = |a: &str| mods.get(a).copied().unwrap_or(0);
    match f.as_str() {
        "level" => lvl,
        "half" | "level/2" => lvl / 2,
        "5*level" => 5 * lvl,
        "prof" => prof,
        _ => {
            if let Some(a) = f.strip_prefix("1+") {
                (1 + m(a)).max(1)
            } else if let Some((a, b)) = f.split_once('+') {
                let other = match b {
                    "level" => lvl,
                    "half" => lvl / 2,
                    _ => b.parse().unwrap_or(0),
                };
                (m(a) + other).max(1)
            } else {
                m(&f).max(1)
            }
        }
    }
}

const ZH: &[(&str, &str)] = &[
    ("轻弩", "light-crossbow"),
    ("手弩", "hand-crossbow"),
    ("重弩", "heavy-crossbow"),
    ("长剑", "longsword"),
    ("短剑", "shortsword"),
    ("巨剑", "greatsword"),
    ("细剑", "rapier"),
    ("刺剑", "rapier"),
    ("弯刀", "scimitar"),
    ("匕首", "dagger"),
    ("巨斧", "greataxe"),
    ("战斧", "battleaxe"),
    ("手斧", "handaxe"),
    ("标枪", "javelin"),
    ("长棍", "quarterstaff"),
    ("木棍", "quarterstaff"),
    ("硬头锤", "mace"),
    ("钉头锤", "mace"),
    ("短弓", "shortbow"),
    ("长弓", "longbow"),
    ("战锤", "warhammer"),
    ("轻锤", "light-hammer"),
    ("巨锤", "maul"),
    ("巨棒", "greatclub"),
    ("木棒", "club"),
    ("投石索", "sling"),
    ("飞镖", "dart"),
    ("长鞭", "whip"),
    ("镰刀", "sickle"),
    ("晨星", "morningstar"),
    ("链枷", "flail"),
    ("长柄刀", "glaive"),
    ("长戟", "halberd"),
    ("长枪", "pike"),
    ("骑枪", "lance"),
    ("三叉戟", "trident"),
    ("战镐", "war-pick"),
    ("吹箭筒", "blowgun"),
    ("长矛", "spear"),
    ("矛", "spear"),
];

/// Weapon ids found in free-text equipment lines (English slugs or common Chinese names).
pub fn weapons_in(equipment: &[String], book: &Book) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in equipment {
        let s = format!("-{}-", crate::chargen::slug(e));
        let mut ids: Vec<&Weapon> = book.weapons.iter().collect();
        ids.sort_by_key(|w| std::cmp::Reverse(w.id.len()));
        let mut found = None;
        for w in ids {
            if s.contains(&format!("-{}-", w.id)) || s.contains(&format!("-{}s-", w.id)) {
                found = Some(w.id.clone());
                break;
            }
        }
        if found.is_none() {
            found = ZH
                .iter()
                .find(|(zh, _)| e.contains(zh))
                .map(|(_, id)| id.to_string());
        }
        if let Some(id) = found {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

pub fn proficient(class: Option<&ClassDef>, w: &Weapon) -> bool {
    let Some(c) = class else { return false };
    c.weapons.iter().any(|p| match p.as_str() {
        "simple" => w.category == "simple",
        "martial" => w.category == "martial",
        "martial-light" => w.category == "martial" && w.has("light"),
        "martial-finesse-or-light" => {
            w.category == "martial" && (w.has("finesse") || w.has("light"))
        }
        id => id == w.id,
    })
}

fn monk_weapon(rs: &str, w: &Weapon) -> bool {
    if w.ranged {
        return false;
    }
    if rs == "2014" {
        w.id == "shortsword" || (w.category == "simple" && !w.has("heavy") && !w.has("two-handed"))
    } else {
        w.category == "simple" || (w.category == "martial" && w.has("light"))
    }
}

pub fn dmg(dice: &str, m: i32) -> String {
    if dice.contains('d') {
        match m.cmp(&0) {
            std::cmp::Ordering::Greater => format!("{dice}+{m}"),
            std::cmp::Ordering::Less => format!("{dice}{m}"),
            _ => dice.to_string(),
        }
    } else {
        (dice.parse::<i32>().unwrap_or(1) + m).max(1).to_string()
    }
}

/// Everything the sheet needs beyond scores, skills and AC.
pub fn generate(c: &Creation, book: &Book, root: Option<&Path>) -> Generated {
    let rs = c.rs();
    let lvl = c.level.clamp(1, 20);
    let prof = data::prof_bonus(lvl);
    let mods = c.mods();
    let m = |a: &str| mods.get(a).copied().unwrap_or(0);
    let mut g = Generated::default();
    let cid = c.class.clone().unwrap_or_default();
    let cls = book.class(&cid);
    if cls.is_none() && !cid.is_empty() {
        g.warnings.push(format!(
            "class `{cid}` has no progression data ({rs}): attacks use no weapon proficiency, features/resources are missing; {}",
            import_hint(root, rs)
        ));
    }
    let sub = c.subclass.as_deref().map(crate::chargen::slug);
    if let (Some(cl), Some(s)) = (cls, sub.as_deref()) {
        if !cl.subclasses.contains_key(s) {
            g.warnings.push(format!(
                "subclass `{s}` of `{cid}` has no feature data ({rs}); {}",
                import_hint(root, rs)
            ));
        }
    }
    let ma = cls
        .and_then(|x| x.martial_arts.as_ref())
        .map(|t| at(t, lvl));
    let mastery = cls
        .and_then(|x| x.mastery.as_ref())
        .filter(|_| rs == "2024")
        .map(|t| at(t, lvl))
        .filter(|n| *n > 0);
    g.mastery = mastery;

    // Attacks.
    let owned = weapons_in(&c.all_items(Some(book)), book);
    let chosen: Vec<String> = if !c.masteries.is_empty() {
        c.masteries
            .iter()
            .map(|s| crate::chargen::slug(s))
            .collect()
    } else if mastery.is_some_and(|n| owned.len() as i32 <= n) {
        owned.clone()
    } else {
        Vec::new()
    };
    if let Some(n) = mastery {
        if c.masteries.is_empty() && owned.len() as i32 > n {
            g.warnings.push(format!(
                "weapon mastery: choose {n} weapon(s) with `talkdnd chargen set masteries <a>,<b>`; masteries are left off until then"
            ));
        }
    }
    for id in &owned {
        let Some(w) = book.weapons.iter().find(|w| &w.id == id) else {
            continue;
        };
        let w = w.for_ruleset(rs);
        let is_prof = proficient(cls, &w);
        let ability = if w.ranged {
            "dex"
        } else if w.has("finesse") || (ma.is_some() && monk_weapon(rs, &w)) {
            if m("dex") > m("str") {
                "dex"
            } else {
                "str"
            }
        } else {
            "str"
        };
        let am = m(ability);
        let mut dice = w.dice.clone();
        if let Some(d) = ma {
            if monk_weapon(rs, &w) {
                let die: i32 = w
                    .dice
                    .split_once('d')
                    .and_then(|(n, s)| (n == "1").then(|| s.parse().ok()).flatten())
                    .unwrap_or(0);
                if d > die {
                    dice = format!("1d{d}");
                }
            }
        }
        let kind = if w.ranged {
            "ranged"
        } else if w.has("thrown") {
            "melee_or_ranged"
        } else {
            "melee"
        };
        let mut note = String::new();
        if let Some(v) = &w.v {
            note += &format!("双手 {}", dmg(v, am));
        }
        if !is_prof {
            if !note.is_empty() {
                note += "；";
            }
            note += "不熟练（不加熟练加值）";
        }
        g.attacks.push(AttackOut {
            id: w.id.clone(),
            name: w.name.clone(),
            kind,
            ability: ability.into(),
            to_hit: am + if is_prof { prof } else { 0 },
            dice: dmg(&dice, am),
            dtype: w.dtype.clone(),
            reach: (!w.ranged).then_some(if w.has("reach") { 10 } else { 5 }),
            range: w.range,
            properties: w.properties.clone(),
            mastery: w.mastery.clone().filter(|_| chosen.contains(&w.id)),
            note,
            proficient: is_prof,
        });
    }
    // Unarmed strike.
    let (ua_ab, ua_dice) = match ma {
        Some(d) => {
            let a = if m("dex") > m("str") { "dex" } else { "str" };
            (a, dmg(&format!("1d{d}"), m(a)))
        }
        None => ("str", dmg("1", m("str"))),
    };
    g.attacks.push(AttackOut {
        id: "unarmed".into(),
        name: "Unarmed Strike".into(),
        kind: "melee",
        ability: ua_ab.into(),
        to_hit: m(ua_ab) + prof,
        dice: ua_dice,
        dtype: "bludgeoning".into(),
        reach: Some(5),
        range: None,
        properties: Vec::new(),
        mastery: None,
        note: String::new(),
        proficient: true,
    });

    let Some(cl) = cls else { return g };

    // Features.
    let mut feats: Vec<Feature> = cl.features.iter().filter(|f| f.0 <= lvl).cloned().collect();
    let asi = cl.asi.clone().unwrap_or_else(|| vec![4, 8, 12, 16, 19]);
    for l in asi.into_iter().filter(|l| *l <= lvl) {
        let (n, s) = if rs == "2024" {
            (
                "Ability Score Improvement",
                "属性值提升专长或其他符合条件的专长",
            )
        } else {
            (
                "Ability Score Improvement",
                "两项属性各 +1 或一项 +2（或选专长）",
            )
        };
        feats.push((l, n.into(), s.into()));
    }
    if let Some(s) = sub.as_deref().and_then(|s| cl.subclasses.get(s)) {
        feats.extend(s.features.iter().filter(|f| f.0 <= lvl).cloned());
    } else {
        let sl = cl.subclass_level.unwrap_or(3);
        if lvl >= sl && sub.is_none() {
            g.warnings.push(format!(
                "level {lvl} {cid} needs a subclass (gained at level {sl}): `talkdnd chargen set subclass <id>`; SRD: {}",
                cl.subclasses.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    feats.sort_by_key(|f| f.0);
    if cid == "rogue" {
        let n = (lvl + 1) / 2;
        for f in feats.iter_mut().filter(|f| f.1 == "Sneak Attack") {
            f.2 = format!("{}（当前 {n}d6）", f.2);
        }
        g.refs.push((
            "sneak-attack".into(),
            Some(format!("{{dice: \"{n}d6\"}}")),
            false,
        ));
        if lvl >= 2 {
            g.refs.push(("cunning-action".into(), None, true));
        }
    }
    if cid == "fighter" {
        g.refs.push(("second-wind".into(), None, true));
    }
    if cid == "cleric" && sub.as_deref() == Some("life") {
        g.refs.push(("disciple-of-life".into(), None, false));
    }
    g.features = feats;

    // Resources.
    let hd = cl
        .hit_die
        .or_else(|| data::class(&cid).map(|x| x.hit_die))
        .unwrap_or(8);
    g.resources.push(ResOut {
        id: "hit-dice".into(),
        name: format!("Hit Dice (d{hd})"),
        max: lvl,
        recharge: "long".into(),
        short_regain: None,
        notes: Some(format!(
            "短休花费 d{hd} + 体质恢复 HP；长休恢复一半（至少 1）"
        )),
    });
    for r in &cl.resources {
        if r.from.is_some_and(|f| lvl < f) {
            continue;
        }
        let max = match (&r.by_level, &r.formula) {
            (Some(t), _) => at(t, lvl),
            (None, Some(f)) => formula(f, lvl, prof, &mods),
            _ => 0,
        };
        if max <= 0 {
            continue;
        }
        let recharge = if r.short_from.is_some_and(|f| lvl >= f) {
            "short".to_string()
        } else {
            r.recharge.clone()
        };
        g.resources.push(ResOut {
            id: r.id.clone(),
            name: r.name.clone(),
            max,
            recharge,
            short_regain: r.short_regain,
            notes: r.notes.clone(),
        });
    }
    match cl.caster.as_deref() {
        Some("pact") => g.pact = Some(pact(lvl)),
        Some(t) => g.slots = slots(t, rs, lvl),
        None => {}
    }
    g.cantrips = cl.cantrips.as_ref().map(|t| at(t, lvl)).filter(|n| *n > 0);
    let sa = data::class(&cid)
        .and_then(|x| x.spell_ability)
        .map(str::to_string)
        .or_else(|| cl.spell_ability.clone())
        .unwrap_or_else(|| "int".into());
    g.spells = if let Some(k) = &cl.known {
        Some(("已知法术", at(k, lvl))).filter(|x| x.1 > 0)
    } else {
        match &cl.prepared {
            Count::Table(t) => Some(("可准备法术", at(t, lvl))),
            Count::Formula(f) => {
                let n = match f.split_once('+').map(|x| x.1) {
                    Some("half") => (m(&sa) + lvl / 2).max(1),
                    _ => (m(&sa) + lvl).max(1),
                };
                (!g.slots.is_empty()).then_some(("可准备法术", n))
            }
            Count::None => None,
        }
    };
    g
}

fn field_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let lower = text.to_lowercase();
    let i = lower.find(&key.to_lowercase())?;
    Some(
        text[i + key.len()..]
            .trim_start_matches([':', '*', ' '])
            .trim(),
    )
}

fn cut_at<'a>(s: &'a str, stops: &[&str]) -> &'a str {
    let lower = s.to_lowercase();
    let mut end = s.len();
    for st in stops {
        if let Some(i) = lower.find(&st.to_lowercase()) {
            end = end.min(i);
        }
    }
    s[..end].trim().trim_end_matches(['*', ' ', '.'])
}

fn clean(s: &str) -> String {
    s.replace("**", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split "A, B, and a purse containing 25 gp" into items and gold.
fn items_and_gold(s: &str) -> (Vec<String>, i64) {
    let s = clean(s);
    let mut gold = 0;
    let mut items = Vec::new();
    for part in s.split([',', ';']) {
        let p = part.trim().trim_start_matches("and ").trim();
        let p = p
            .trim_start_matches("a set of ")
            .trim_start_matches("A set of ");
        let p = p
            .trim_start_matches("a ")
            .trim_start_matches("A ")
            .trim_start_matches("an ")
            .trim_start_matches("An ")
            .trim();
        if p.is_empty() {
            continue;
        }
        let words: Vec<&str> = p.split_whitespace().collect();
        if let Some(pos) = words.iter().position(|w| {
            let w = w.to_lowercase();
            w == "gp" || w == "gp." || w == "cp" || w == "gp)"
        }) {
            if pos > 0 {
                if let Ok(n) = words[pos - 1].parse::<i64>() {
                    gold += n;
                    let rest = words[..pos - 1].join(" ");
                    let rest = rest.trim_end_matches(" containing").trim();
                    if !rest.is_empty() && rest != "pouch containing" {
                        items.push(rest.to_string());
                    }
                    continue;
                }
            }
        }
        items.push(p.to_string());
    }
    (items, gold)
}

/// Best-effort background import (names, item lists, gold only).
/// 2014: a heading followed by `Feature: …`; fields may sit just before the
/// heading when the two-column page order is mixed up.
/// 2024: a heading followed by `Ability Scores:` … `Equipment: … (A) … (B) N GP`.
pub fn import_backgrounds(rs: &str, text: &str) -> BTreeMap<String, crate::options::BackgroundDef> {
    let lines: Vec<&str> = text.lines().collect();
    let heading = |l: &str| -> Option<String> {
        let t = l.trim();
        if !t.starts_with('#') {
            return None;
        }
        let h = t.trim_start_matches('#').trim().to_string();
        let words = h.split_whitespace().count();
        ((1..=4).contains(&words) && !h.contains(':')).then_some(h)
    };
    let mut out = BTreeMap::new();
    if rs == "2014" {
        const JUNK: &[&str] = &[
            "suggested",
            "specialty",
            "routines",
            "schemes",
            "event",
            "business",
            "origin",
            "other",
            "ideal",
            "bond",
            "flaw",
            "trait",
            "characteristic",
            "variant",
            "seclusion",
            "background",
            "feature",
            "customiz",
        ];
        let name_heading = |l: &str| {
            heading(l).filter(|h| {
                let lo = h.to_lowercase();
                h.split_whitespace().count() <= 3 && !JUNK.iter().any(|j| lo.contains(j))
            })
        };
        let field = |l: &str, key: &str| -> Option<String> {
            let pos = l.to_lowercase().find(&key.to_lowercase())?;
            Some(
                l[pos + key.len()..]
                    .trim_start_matches(['*', ':', ' '])
                    .to_string(),
            )
        };
        let stops = [
            "**Tool Proficiencies",
            "Tool Proficiencies",
            "**Languages",
            "Languages:",
            "**Equipment",
            "Equipment:",
        ];
        let mut i = 0;
        while i < lines.len() {
            let l = lines[i];
            let Some(sk) = field(l, "Skill Proficiencies") else {
                i += 1;
                continue;
            };
            let cluster_end = (i + 9).min(lines.len());
            // Next non-variant Feature heading.
            let Some(feat_at) = (i + 1..(i + 120).min(lines.len())).find(|j| {
                let t = lines[*j].trim();
                t.starts_with('#') && t.trim_start_matches('#').trim().starts_with("Feature:")
            }) else {
                i += 1;
                continue;
            };
            // A name heading shortly before the fields, but not past the
            // previous background's Feature heading.
            let mut before = None;
            for j in (i.saturating_sub(20)..i).rev() {
                let t = lines[j].trim();
                if t.starts_with('#') && t.trim_start_matches('#').trim().starts_with("Feature:") {
                    break;
                }
                if let Some(h) = name_heading(lines[j]) {
                    before = Some(h);
                    break;
                }
            }
            let between = (i + 1..feat_at).rev().find_map(|j| name_heading(lines[j]));
            let Some(name) = before.or(between) else {
                i = cluster_end;
                continue;
            };
            let mut b = crate::options::BackgroundDef {
                name: Some(name.clone()),
                feature: Some(
                    lines[feat_at].trim().trim_start_matches('#').trim()["Feature:".len()..]
                        .trim()
                        .to_string(),
                ),
                ..Default::default()
            };
            let own_stops: Vec<&str> = stops.to_vec();
            b.skills = clean(cut_at(&sk, &own_stops))
                .split(',')
                .map(|x| crate::chargen::slug(x.trim()))
                .filter(|x| !x.is_empty() && x.len() < 20)
                .collect();
            for j in i..cluster_end {
                let l = lines[j];
                if let Some(v) = field(l, "Tool Proficiencies") {
                    let v = clean(cut_at(
                        &v,
                        &["**Languages", "Languages:", "**Equipment", "Equipment:"],
                    ));
                    if b.tools.is_empty() && !v.is_empty() {
                        b.tools = v
                            .split(',')
                            .map(|x| x.trim().to_string())
                            .filter(|x| !x.is_empty())
                            .collect();
                    }
                }
                if let Some(v) = field(l, "Languages") {
                    let lo = v.to_lowercase();
                    if b.languages == 0 {
                        b.languages = if lo.starts_with("two") {
                            2
                        } else if lo.starts_with("one") {
                            1
                        } else {
                            0
                        };
                    }
                }
                if let Some(v) = field(l, "Equipment") {
                    if b.equipment.is_empty() {
                        // Equipment may wrap onto the next non-empty line.
                        let mut v = v.trim().to_string();
                        if !v.to_lowercase().contains("gp") {
                            if let Some(n) = lines[j + 1..cluster_end.min(j + 4)]
                                .iter()
                                .find(|x| !x.trim().is_empty() && !x.trim().starts_with('#'))
                            {
                                v = format!("{v} {}", n.trim());
                            }
                        }
                        let (items, gold) = items_and_gold(&v);
                        b.equipment = items;
                        b.gold = gold;
                    }
                }
            }
            out.entry(crate::chargen::slug(&name)).or_insert(b);
            i = cluster_end;
        }
        return out;
    }
    for (i, l) in lines.iter().enumerate() {
        let Some(h) = heading(l) else { continue };
        let window: Vec<&str> = lines[i + 1..(i + 14).min(lines.len())].to_vec();
        let joined = window.join(" ");
        let lo = h.to_lowercase();
        if !joined.contains("Ability Scores:")
            || !joined.contains("Equipment")
            || [
                "equipment",
                "proficien",
                "description",
                "ability",
                "feat",
                "background",
            ]
            .iter()
            .any(|j| lo.contains(j))
        {
            continue;
        }
        let name = h
            .split_whitespace()
            .map(|w| {
                let mut c = w.to_lowercase();
                if let Some(f) = c.get(..1) {
                    c = f.to_uppercase() + &c[1..];
                }
                c
            })
            .collect::<Vec<_>>()
            .join(" ");
        let mut b = crate::options::BackgroundDef {
            name: Some(name.clone()),
            ..Default::default()
        };
        if let Some(f) = field_after(&joined, "Feat:") {
            b.feat =
                Some(cut_at(f, &[" (see", "Skill Proficiencies", "Tool Proficiency"]).to_string());
        }
        if let Some(f) = field_after(&joined, "Skill Proficiencies:") {
            b.skills = cut_at(f, &["Tool Proficiency", "Equipment"])
                .split(" and ")
                .map(|x| crate::chargen::slug(x.trim()))
                .filter(|x| !x.is_empty())
                .collect();
        }
        if let Some(f) = field_after(&joined, "Tool Proficiency:") {
            let t = cut_at(f, &["Equipment", "(see"]);
            if !t.is_empty() {
                b.tools = vec![t.to_string()];
            }
        }
        if let Some(e) = field_after(&joined, "Equipment") {
            let e = e.replace("(8)", "(B)").replace("A or 8", "A or B");
            if let Some(a) = e.find("(A)") {
                let rest = &e[a + 3..];
                let (a_txt, b_txt) = match rest.find("(B)") {
                    Some(j) => (&rest[..j], Some(&rest[j + 3..])),
                    None => (rest, None),
                };
                let a_txt = a_txt
                    .trim()
                    .trim_end_matches("or")
                    .trim()
                    .trim_end_matches(';');
                let (items, gold) = items_and_gold(a_txt);
                b.options
                    .insert("A".into(), crate::options::Pack { items, gold });
                if let Some(bt) = b_txt {
                    let (_, g) = items_and_gold(
                        cut_at(bt, &["\n"])
                            .split_whitespace()
                            .take(2)
                            .collect::<Vec<_>>()
                            .join(" ")
                            .as_str(),
                    );
                    b.options.insert(
                        "B".into(),
                        crate::options::Pack {
                            items: Vec::new(),
                            gold: g,
                        },
                    );
                }
            }
        }
        if !b.options.is_empty() {
            out.insert(crate::chargen::slug(&name), b);
        }
    }
    out
}

/// Best-effort importer: subclass feature names by level from converted Markdown.
///
/// Looks for `<Class> Subclasses` headings, then subclass headings and
/// `Level N: Feature` headings below them. Writes names only (no rules text) and
/// marks every summary as "check the book".
pub fn import_markdown(rs: &str, srd: &Book, files: &[PathBuf]) -> Result<DataFile> {
    let mut out = DataFile {
        ruleset: Some(rs.to_string()),
        ..Default::default()
    };
    let class_ids: Vec<String> = srd.classes.keys().cloned().collect();
    let mut known: BTreeMap<(String, String), Vec<(i32, String)>> = BTreeMap::new();
    for f in files {
        // Page anchors are layout, not content: the importer parses the
        // paged Markdown exactly like the old page-less reader edition.
        let text = crate::library::read_text(f)?;
        for (id, b) in import_backgrounds(rs, &text) {
            out.backgrounds.entry(id).or_insert(b);
        }
        if rs == "2014" {
            import_2014(&text, srd, &mut out);
            for (c, name, feats) in import_2014_known(&text, srd) {
                let k = (c, name);
                if known
                    .get(&k)
                    .is_none_or(|f: &Vec<(i32, String)>| f.len() < feats.len())
                {
                    known.insert(k, feats);
                }
            }
            continue;
        }
        let mut cur_class: Option<String> = None;
        let mut cur_sub: Option<String> = None;
        for line in text.lines() {
            let t = line.trim();
            if !t.starts_with('#') {
                continue;
            }
            let h = t.trim_start_matches('#').trim();
            let lower = h.to_lowercase();
            if let Some(cname) = lower.strip_suffix(" subclasses") {
                let id = crate::chargen::slug(cname);
                cur_class = class_ids.contains(&id).then_some(id);
                cur_sub = None;
                continue;
            }
            let Some(cls) = cur_class.clone() else {
                continue;
            };
            if let Some((lv, name)) = parse_level_heading(h) {
                if let Some(s) = &cur_sub {
                    let e = out
                        .classes
                        .entry(cls.clone())
                        .or_default()
                        .subclasses
                        .entry(s.clone())
                        .or_default();
                    if !e.features.iter().any(|x| x.1 == name) {
                        e.features.push((lv, name, "见规则书（导入）".into()));
                    }
                }
                continue;
            }
            // A heading that names a class ends that class's subclass section.
            if class_ids.contains(&crate::chargen::slug(&lower))
                || lower.ends_with(" features")
                || lower.starts_with("chapter")
            {
                cur_class = None;
                cur_sub = None;
                continue;
            }
            let words = h.split_whitespace().count();
            let looks_like_name = (1..=6).contains(&words)
                && h.chars().any(|c| c.is_alphabetic())
                && !h.contains(':');
            if looks_like_name {
                let sid = crate::chargen::slug(h);
                let is_srd = srd.classes.get(&cls).is_some_and(|c| {
                    c.subclasses.iter().any(|(k, v)| {
                        *k == sid
                            || v.name.as_deref().map(crate::chargen::slug).as_deref()
                                == Some(sid.as_str())
                    })
                });
                let noise = ["spell", "level", "evel", "trait", "action"]
                    .iter()
                    .any(|n| sid.contains(n));
                if is_srd || noise {
                    cur_sub = None; // SRD subclass: tracked data already covers it
                } else {
                    cur_sub = Some(sid.clone());
                    out.classes
                        .entry(cls.clone())
                        .or_default()
                        .subclasses
                        .entry(sid)
                        .or_default()
                        .name = Some(title(h));
                }
            }
        }
    }
    // A real subclass starts with a feature at the level the class gains it.
    for (cid, c) in out.classes.iter_mut() {
        let first = srd
            .classes
            .get(cid)
            .and_then(|x| x.subclass_level)
            .filter(|_| rs == "2014")
            .unwrap_or(3);
        c.subclasses
            .retain(|_, s| s.features.iter().map(|f| f.0).min() == Some(first));
        for s in c.subclasses.values_mut() {
            s.features.sort_by_key(|f| f.0);
        }
    }
    out.classes.retain(|_, c| !c.subclasses.is_empty());
    // Lore headings the layout pass took for subclasses have no features.
    for c in out.classes.values_mut() {
        c.subclasses.retain(|_, s| !s.features.is_empty());
    }
    out.classes.retain(|_, c| !c.subclasses.is_empty());
    // Known 2014 subclasses replace what the layout pass guessed for them.
    for ((c, name), feats) in known {
        let sid = crate::chargen::slug(&name);
        let entry = out.classes.entry(c).or_default();
        let e = entry.subclasses.entry(sid).or_default();
        e.name = Some(name);
        e.features = feats
            .into_iter()
            .map(|(l, n)| (l, n, "见规则书（导入）".into()))
            .collect();
    }
    Ok(out)
}

/// 2014 PHB group headings that open each class's subclass list.
const GROUPS_2014: &[(&str, &str)] = &[
    ("primal paths", "barbarian"),
    ("bard colleges", "bard"),
    ("divine domains", "cleric"),
    ("druid circles", "druid"),
    ("druid circle", "druid"),
    ("martial archetypes", "fighter"),
    ("monastic traditions", "monk"),
    ("sacred oaths", "paladin"),
    ("ranger archetypes", "ranger"),
    ("roguish archetypes", "rogue"),
    ("sorcerous origins", "sorcerer"),
    ("otherworldly patrons", "warlock"),
    ("arcane traditions", "wizard"),
];

/// First "Nth level" in a feature's text ("At 6th level", "Starting at 14th level").
fn level_in_text(t: &str) -> Option<i32> {
    let low = t.to_lowercase();
    let i = low.find(" level")?;
    let head = &low[..i];
    let word = head
        .rsplit(|c: char| c.is_whitespace() || c == '(')
        .next()?;
    let digits: String = word.chars().take_while(|c| c.is_ascii_digit()).collect();
    let lv: i32 = digits.parse().ok()?;
    let suffix = &word[digits.len()..];
    (matches!(suffix, "st" | "nd" | "rd" | "th") && (1..=20).contains(&lv)).then_some(lv)
}

/// 2014 layout: `### Divine Domains` → `### Life Domain` → `#### Feature` headings
/// whose level is written in the text ("At 6th level ..."). Feature names only;
/// summaries stay "see the rulebook".
fn import_2014(text: &str, srd: &Book, out: &mut DataFile) {
    let lines: Vec<&str> = text.lines().collect();
    let mut cls: Option<String> = None;
    let mut sub: Option<(String, usize)> = None; // id, heading depth
    let mut group_depth = 0usize;
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        i += 1;
        if !t.starts_with('#') {
            continue;
        }
        let depth = t.chars().take_while(|c| *c == '#').count();
        let mut h = t.trim_start_matches('#').trim().to_string();
        let lower = h.to_lowercase();
        if let Some((_, c)) = GROUPS_2014.iter().find(|(g, _)| lower == *g) {
            cls = Some(c.to_string());
            sub = None;
            group_depth = depth;
            continue;
        }
        let Some(c) = cls.clone() else {
            continue;
        };
        // Leaving the class: a shallower heading, a class name or a chapter.
        if depth < group_depth
            || srd.classes.contains_key(&crate::chargen::slug(&lower))
            || lower.starts_with("chapter")
            || lower.starts_with("creating a")
            || lower == "class features"
            || lower == "quick build"
        {
            cls = None;
            sub = None;
            continue;
        }
        // "Channel Divinity:" split over two headings.
        if h.ends_with(':') && i < lines.len() && lines[i].trim().starts_with('#') {
            h = format!("{} {}", h, lines[i].trim().trim_start_matches('#').trim());
            i += 1;
        }
        // Base-class feature names of any class are never subclasses or
        // subclass features (OCR moves them around the page).
        let class_feature = srd.classes.values().any(|cd| {
            cd.features
                .iter()
                .any(|f| crate::chargen::slug(&f.1) == crate::chargen::slug(&h))
        });
        const GENERIC: &[&str] = &[
            "ability-score-improvement",
            "extra-attack",
            "spellcasting",
            "proficiencies",
            "hit-points",
            "equipment",
            "class-features",
            "multiclassing",
            "quick-build",
        ];
        if class_feature || GENERIC.contains(&crate::chargen::slug(&h).as_str()) {
            continue; // OCR put a base-class feature heading here
        }
        if depth <= group_depth {
            // A subclass name.
            let sid = crate::chargen::slug(&h);
            let words = h.split_whitespace().count();
            let is_srd = srd.classes.get(&c).is_some_and(|cd| {
                cd.subclasses.iter().any(|(k, v)| {
                    *k == sid
                        || v.name.as_deref().map(crate::chargen::slug).as_deref()
                            == Some(sid.as_str())
                })
            });
            if is_srd || !(1..=6).contains(&words) || h.contains(':') {
                sub = None;
                continue;
            }
            out.classes
                .entry(c.clone())
                .or_default()
                .subclasses
                .entry(sid.clone())
                .or_default()
                .name = Some(title(&h));
            sub = Some((sid, depth));
            continue;
        }
        let Some((sid, _)) = sub.clone() else {
            continue;
        };
        let low = h.to_lowercase();
        if low.ends_with("level spells") || low.chars().all(|c| !c.is_alphabetic()) {
            continue; // table header
        }
        if srd
            .classes
            .contains_key(&crate::chargen::slug(low.trim_start_matches("the ")))
        {
            continue; // a class's lore sidebar ("The Cleric")
        }
        // Level from the first body paragraph.
        let body: String = lines[i..]
            .iter()
            .take_while(|l| !l.trim().starts_with('#'))
            .take(6)
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let first = srd
            .classes
            .get(&c)
            .and_then(|x| x.subclass_level)
            .unwrap_or(3);
        let e = out
            .classes
            .entry(c.clone())
            .or_default()
            .subclasses
            .entry(sid)
            .or_default();
        let prev = e.features.last().map(|f| f.0).unwrap_or(first);
        let lv = level_in_text(&body).unwrap_or(prev).max(first);
        let name = title(&h);
        if !e.features.iter().any(|x| x.1 == name) {
            e.features.push((lv, name, "见规则书（导入）".into()));
        }
    }
}

/// 2014 Player's Handbook subclasses by name, with the words that mark their
/// own feature headings. OCR'd two-column pages interleave neighbouring
/// subclasses (Nature/Tempest, Trickery/War), so headings are given to the
/// subclass whose words they contain before falling back to page order.
/// SRD subclasses are listed too: they only bound the others' sections.
const KNOWN_2014: &[(&str, &str, &[&str])] = &[
    (
        "barbarian",
        "Path of the Berserker",
        &["berserker", "frenzy"],
    ),
    (
        "barbarian",
        "Path of the Totem Warrior",
        &["totem", "spirit"],
    ),
    ("bard", "College of Lore", &["lore"]),
    (
        "bard",
        "College of Valor",
        &["valor", "battle magic", "combat inspiration"],
    ),
    ("cleric", "Knowledge Domain", &["knowledge"]),
    ("cleric", "Life Domain", &["life"]),
    ("cleric", "Light Domain", &["light"]),
    ("cleric", "Nature Domain", &["nature", "charm animals"]),
    (
        "cleric",
        "Tempest Domain",
        &["tempest", "storm", "thunder", "wrath"],
    ),
    (
        "cleric",
        "Trickery Domain",
        &["trickery", "trickster", "duplicity", "cloak of shadows"],
    ),
    ("cleric", "War Domain", &["war ", "battle", "guided strike"]),
    ("druid", "Circle of the Land", &["land"]),
    (
        "druid",
        "Circle of the Moon",
        &[
            "moon",
            "combat wild shape",
            "elemental wild shape",
            "thousand forms",
        ],
    ),
    ("fighter", "Champion", &["champion", "critical"]),
    (
        "fighter",
        "Battle Master",
        &["battle master", "superiority", "maneuver", "student of war"],
    ),
    (
        "fighter",
        "Eldritch Knight",
        &["eldritch", "weapon bond", "war magic", "arcane charge"],
    ),
    (
        "monk",
        "Way of the Open Hand",
        &["open hand", "wholeness", "tranquility", "quivering"],
    ),
    ("monk", "Way of Shadow", &["shadow"]),
    (
        "monk",
        "Way of the Four Elements",
        &["four elements", "elemental", "disciple of the elements"],
    ),
    ("paladin", "Oath of Devotion", &["devotion"]),
    ("paladin", "Oath of the Ancients", &["ancients"]),
    (
        "paladin",
        "Oath of Vengeance",
        &["vengeance", "vengeful", "avenging"],
    ),
    ("ranger", "Hunter", &["hunter"]),
    (
        "ranger",
        "Beast Master",
        &["beast master", "companion", "bestial", "beast"],
    ),
    (
        "rogue",
        "Thief",
        &["thief's", "thief\u{2019}s", "fast hands", "second-story"],
    ),
    (
        "rogue",
        "Assassin",
        &["assassin", "infiltration", "impostor", "death strike"],
    ),
    (
        "rogue",
        "Arcane Trickster",
        &[
            "arcane trickster",
            "mage hand",
            "magical ambush",
            "versatile trickster",
            "spell thief",
        ],
    ),
    ("sorcerer", "Draconic Bloodline", &["draconic", "dragon"]),
    (
        "sorcerer",
        "Wild Magic",
        &[
            "wild magic",
            "tides of chaos",
            "bend luck",
            "controlled chaos",
            "spell bombardment",
        ],
    ),
    (
        "warlock",
        "The Archfey",
        &[
            "archfey",
            "fey presence",
            "misty escape",
            "beguiling",
            "dark delirium",
        ],
    ),
    ("warlock", "The Fiend", &["fiend", "dark one"]),
    (
        "warlock",
        "The Great Old One",
        &[
            "great old one",
            "awakened mind",
            "entropic ward",
            "thought shield",
            "create thrall",
        ],
    ),
    ("wizard", "School of Abjuration", &["abjur"]),
    ("wizard", "School of Conjuration", &["conjur"]),
    (
        "wizard",
        "School of Divination",
        &[
            "divination",
            "portent",
            "expert divination",
            "the third eye",
        ],
    ),
    (
        "wizard",
        "School of Enchantment",
        &[
            "enchant",
            "hypnotic",
            "split enchantment",
            "alter memories",
            "instinctive charm",
        ],
    ),
    (
        "wizard",
        "School of Evocation",
        &[
            "evocation",
            "sculpt",
            "potent cantrip",
            "empowered evocation",
            "overchannel",
        ],
    ),
    ("wizard", "School of Illusion", &["illusion", "illusory"]),
    (
        "wizard",
        "School of Necromancy",
        &["necromancy", "grim harvest", "undead", "inured"],
    ),
    (
        "wizard",
        "School of Transmutation",
        &["transmut", "minor alchemy", "shapechanger"],
    ),
];

fn squash(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect()
}

/// Known 2014 subclasses: anchored on their own name heading (or, when OCR lost
/// it, on a heading naming one of their words plus "spells"/"tenets"), and fed
/// every following heading that has a feature level, given to the sibling whose
/// words it names. Replaces whatever the layout pass found for them.
/// (class, subclass name, [(level, feature)]).
type KnownSub = (String, String, Vec<(i32, String)>);

fn import_2014_known(text: &str, srd: &Book) -> Vec<KnownSub> {
    let mut res = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let heads: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with('#'))
        .map(|(i, l)| (i, l.trim().trim_start_matches('#').trim().to_string()))
        .collect();
    // The book index at the end repeats every name; stop at it.
    let end = heads
        .iter()
        .position(|(_, h)| h.eq_ignore_ascii_case("index"))
        .unwrap_or(heads.len());
    let heads = &heads[..end];
    for (c, _) in GROUPS_2014
        .iter()
        .map(|(g, c)| (c, g))
        .collect::<std::collections::BTreeMap<_, _>>()
    {
        let Some(gpos) = heads.iter().position(|(_, h)| {
            GROUPS_2014
                .iter()
                .any(|(g, cc)| cc == c && h.eq_ignore_ascii_case(g))
        }) else {
            continue;
        };
        // The class's section: back to its own class heading (subclass
        // descriptions often come before the group heading in the OCR order)
        // and forward to the next class heading.
        let is_class = |h: &str| srd.classes.contains_key(&crate::chargen::slug(h));
        let start = heads[..gpos]
            .iter()
            .rposition(|(_, h)| crate::chargen::slug(h) == *c)
            .unwrap_or(gpos);
        let stop = heads[gpos + 1..]
            .iter()
            .position(|(_, h)| {
                is_class(h)
                    || h.to_lowercase().starts_with("chapter")
                    || GROUPS_2014
                        .iter()
                        .any(|(g, cc)| cc != c && h.eq_ignore_ascii_case(g))
            })
            .map(|k| gpos + 1 + k)
            .unwrap_or(heads.len());
        let region = &heads[start..stop];
        let subs: Vec<&(&str, &str, &[&str])> = KNOWN_2014.iter().filter(|k| k.0 == *c).collect();
        // Anchors.
        let mut anchors: Vec<(usize, usize)> = Vec::new(); // (region index, sub index)
        for (si, (_, name, words)) in subs.iter().enumerate() {
            let n = squash(name);
            let short = squash(
                name.trim_start_matches("The ")
                    .split_whitespace()
                    .last()
                    .unwrap_or(""),
            );
            let pos = region.iter().position(|(_, h)| squash(h) == n).or_else(|| {
                region.iter().position(|(_, h)| {
                    let l = h.to_lowercase();
                    let s = squash(h);
                    (s.starts_with(&n) && l.ends_with("spells"))
                        || (l.starts_with("tenets of") && s.contains(&short))
                        || (s == squash(words[0]))
                })
            });
            if let Some(p) = pos {
                anchors.push((p, si));
            }
        }
        anchors.sort();
        let owner_by_words = |h: &str| -> Option<usize> {
            let l = format!("{} ", h.to_lowercase());
            let hits: Vec<usize> = subs
                .iter()
                .enumerate()
                .filter(|(_, k)| k.2.iter().any(|w| l.contains(w)))
                .map(|(i, _)| i)
                .collect();
            (hits.len() == 1).then(|| hits[0])
        };
        let first = srd
            .classes
            .get(*c)
            .and_then(|x| x.subclass_level)
            .unwrap_or(3);
        let mut feats: Vec<Vec<(i32, String)>> = vec![Vec::new(); subs.len()];
        let mut found = vec![false; subs.len()];
        for &(_, si) in &anchors {
            found[si] = true;
        }
        let anchor_rows: Vec<usize> = anchors.iter().map(|a| a.0).collect();
        for (ri, (li, h)) in region.iter().enumerate() {
            if anchor_rows.contains(&ri) && subs.iter().any(|k| squash(k.1) == squash(h)) {
                continue;
            }
            let Some(&(_, page_owner)) = anchors.iter().rev().find(|a| a.0 <= ri) else {
                continue;
            };
            let mut h = h.clone();
            let mut body_from = li + 1;
            if h.ends_with(':') {
                if let Some((nli, nh)) = region.get(ri + 1) {
                    h = format!("{h} {nh}");
                    body_from = nli + 1;
                }
            }
            let low = h.to_lowercase();
            let sl = crate::chargen::slug(&h);
            let base = srd
                .classes
                .values()
                .any(|cd| cd.features.iter().any(|f| crate::chargen::slug(&f.1) == sl));
            if base
                || low.ends_with("level spells")
                || low.starts_with("tenets of")
                || low.starts_with("cleric level")
                || low == "channel divinity"
                || is_class(&h)
                || subs.iter().any(|k| squash(k.1) == squash(&h))
                || GROUPS_2014.iter().any(|(g, _)| low == *g)
                || [
                    "cantrips",
                    "preparing and casting spells",
                    "domain spells",
                    "spellcasting focus",
                    "ritual casting",
                    "spellcasting ability",
                ]
                .contains(&low.as_str())
                || [
                    "spellcasting",
                    "ability score improvement",
                    "extra attack",
                    "proficiencies",
                    "equipment",
                    "hit points",
                    "quick build",
                    "class features",
                    "multiclassing",
                ]
                .contains(&low.as_str())
                || h.split_whitespace().count() > 7
                || !h.chars().next().is_some_and(|ch| ch.is_uppercase())
            {
                continue;
            }
            let body: String = lines[body_from.min(lines.len())..]
                .iter()
                .take_while(|l| !l.trim().starts_with('#'))
                .take(8)
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            let owner = owner_by_words(&h).unwrap_or(page_owner);
            let lv = level_in_text(&body);
            let spells = low.ends_with(" spells");
            if lv.is_none() && !spells {
                continue; // no level: lore or a table, not a feature
            }
            let lv = lv.unwrap_or(first).max(first.min(lv.unwrap_or(first)));
            if !feats[owner].iter().any(|f| squash(&f.1) == squash(&h)) {
                feats[owner].push((lv, title(&h)));
            }
        }
        let srd_sub = |name: &str| {
            srd.classes.get(*c).is_some_and(|cd| {
                cd.subclasses.iter().any(|(k, v)| {
                    squash(k) == squash(name)
                        || v.name.as_deref().map(squash).as_deref() == Some(squash(name).as_str())
                        || squash(name).contains(&squash(k))
                })
            })
        };
        for (si, (_, name, _)) in subs.iter().enumerate() {
            if srd_sub(name) || feats[si].is_empty() {
                continue;
            }
            let mut fs = std::mem::take(&mut feats[si]);
            fs.sort_by_key(|f| f.0);
            res.push((c.to_string(), name.to_string(), fs));
        }
    }
    res
}

fn title(h: &str) -> String {
    h.split_whitespace()
        .map(|w| {
            let l = w.to_lowercase();
            let mut cs = l.chars();
            match cs.next() {
                Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `Level 3: Primal Companion` (tolerates OCR noise like `EVEL7: NAME`).
fn parse_level_heading(h: &str) -> Option<(i32, String)> {
    let up = h.to_uppercase();
    let i = up.find("EVEL")?;
    if i > 2 {
        return None;
    }
    let rest = &h[i + 4..];
    let rest = rest.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let lv: i32 = digits.parse().ok()?;
    if !(1..=20).contains(&lv) {
        return None;
    }
    let after = rest[digits.len()..]
        .trim_start_matches([' ', ':', '：'])
        .trim();
    let after = after.trim_start_matches(':').trim();
    if after.is_empty() {
        return None;
    }
    Some((lv, title(after)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(rs: &str, class: &str, lvl: i32, scores: [i32; 6], eq: &[&str]) -> Creation {
        let mut c = Creation {
            ruleset: Some(rs.into()),
            class: Some(class.into()),
            level: lvl,
            equipment: eq.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        for (a, v) in data::ABILITIES.iter().zip(scores) {
            c.scores.insert(a.to_string(), v);
        }
        c
    }

    fn atk<'a>(g: &'a Generated, id: &str) -> &'a AttackOut {
        g.attacks.iter().find(|a| a.id == id).unwrap()
    }

    fn res<'a>(g: &'a Generated, id: &str) -> Option<&'a ResOut> {
        g.resources.iter().find(|a| a.id == id)
    }

    #[test]
    fn embedded_data_parses_for_all_classes() {
        for rs in ["2014", "2024"] {
            let b = Book::srd(rs);
            assert_eq!(b.classes.len(), 12, "{rs}");
            for (id, c) in &b.classes {
                assert!(data::class(id).is_some(), "{id}");
                assert_eq!(c.subclasses.len(), 1, "{rs} {id}");
                for r in &c.resources {
                    if let Some(t) = &r.by_level {
                        assert_eq!(t.len(), 20, "{rs} {id} {}", r.id);
                    }
                }
                for t in [&c.cantrips, &c.known, &c.mastery, &c.martial_arts]
                    .into_iter()
                    .flatten()
                {
                    assert_eq!(t.len(), 20, "{rs} {id}");
                }
            }
            assert!(b.weapons.len() >= 36);
        }
    }

    #[test]
    fn adventure_data_parses() {
        let a = adventure("keys-from-the-golden-vault").unwrap();
        assert_eq!((a.start_level_min, a.start_level_max), (1, 11));
        let c = adventure("curse-of-strahd").unwrap();
        assert_eq!((c.start_level_min, c.start_level_max), (1, 3));
        assert!(adventure("nonesuch").is_none());
    }

    #[test]
    fn life_domain_grants_heavy_armor() {
        let b = Book::srd("2014");
        let life = &b.classes["cleric"].subclasses["life"];
        assert_eq!(life.armor_proficiency, vec!["heavy".to_string()]);
    }

    #[test]
    fn fighter_longsword() {
        let c = pc(
            "2024",
            "fighter",
            1,
            [16, 12, 14, 10, 10, 8],
            &["Longsword", "Shield"],
        );
        let g = generate(&c, &Book::srd("2024"), None);
        let a = atk(&g, "longsword");
        assert_eq!(
            (a.ability.as_str(), a.to_hit, a.dice.as_str()),
            ("str", 5, "1d8+3")
        );
        assert_eq!(a.mastery.as_deref(), Some("sap"));
        assert!(a.note.contains("1d10+3"));
        assert_eq!(res(&g, "second-wind").unwrap().max, 2);
        assert!(g.refs.iter().any(|r| r.0 == "second-wind"));
        assert_eq!(atk(&g, "unarmed").dice, "4");
    }

    #[test]
    fn rogue_rapier_finesse_and_ranged() {
        let c = pc(
            "2014",
            "rogue",
            3,
            [8, 17, 12, 10, 12, 14],
            &["rapier", "shortbow and 20 arrows", "2 daggers"],
        );
        let g = generate(&c, &Book::srd("2014"), None);
        let r = atk(&g, "rapier");
        assert_eq!(
            (r.ability.as_str(), r.to_hit, r.dice.as_str()),
            ("dex", 5, "1d8+3")
        );
        let b = atk(&g, "shortbow");
        assert_eq!((b.kind, b.range), ("ranged", Some([80, 320])));
        assert_eq!(atk(&g, "dagger").kind, "melee_or_ranged");
        assert!(r.mastery.is_none(), "2014 has no mastery");
        assert!(g
            .features
            .iter()
            .any(|f| f.1 == "Sneak Attack" && f.2.contains("2d6")));
        assert!(g.warnings.iter().any(|w| w.contains("subclass")));
    }

    #[test]
    fn chinese_names_and_nonproficient() {
        let c = pc(
            "2014",
            "wizard",
            1,
            [10, 14, 12, 16, 12, 8],
            &["长剑", "匕首"],
        );
        let g = generate(&c, &Book::srd("2014"), None);
        assert!(!atk(&g, "longsword").proficient);
        assert_eq!(atk(&g, "longsword").to_hit, 0);
        assert!(atk(&g, "dagger").proficient);
    }

    #[test]
    fn wizard_slots() {
        let b = Book::srd("2014");
        let c = pc("2014", "wizard", 1, [8, 14, 12, 16, 12, 10], &[]);
        let g = generate(&c, &b, None);
        assert_eq!(g.slots, vec![2]);
        assert_eq!(g.cantrips, Some(3));
        assert_eq!(g.spells, Some(("可准备法术", 4)));
        let c = pc("2014", "wizard", 5, [8, 14, 12, 16, 12, 10], &[]);
        let g = generate(&c, &b, None);
        assert_eq!(g.slots, vec![4, 3, 2]);
        assert_eq!(g.spells, Some(("可准备法术", 8)));
        assert_eq!(res(&g, "arcane-recovery").unwrap().max, 1);
    }

    #[test]
    fn half_and_pact_casters() {
        assert!(slots("half", "2014", 1).is_empty());
        assert_eq!(slots("half", "2024", 1), vec![2]);
        assert_eq!(slots("half", "2014", 5), vec![4, 2]);
        assert_eq!(pact(1), (1, 1));
        assert_eq!(pact(5), (2, 3));
        assert_eq!(pact(11), (3, 5));
        let c = pc("2024", "warlock", 3, [8, 14, 12, 10, 12, 16], &[]);
        let g = generate(&c, &Book::srd("2024"), None);
        assert_eq!(g.pact, Some((2, 2)));
    }

    #[test]
    fn barbarian_rage_and_monk() {
        let c = pc(
            "2014",
            "barbarian",
            3,
            [16, 14, 14, 8, 10, 10],
            &["greataxe"],
        );
        let g = generate(&c, &Book::srd("2014"), None);
        assert_eq!(res(&g, "rage").unwrap().max, 3);
        assert_eq!(atk(&g, "greataxe").dice, "1d12+3");
        let c = pc("2024", "barbarian", 1, [16, 14, 14, 8, 10, 10], &[]);
        let g = generate(&c, &Book::srd("2024"), None);
        let r = res(&g, "rage").unwrap();
        assert_eq!((r.max, r.short_regain), (2, Some(1)));
        let c = pc("2024", "monk", 1, [10, 16, 12, 10, 14, 8], &["spear"]);
        let g = generate(&c, &Book::srd("2024"), None);
        assert_eq!(atk(&g, "unarmed").dice, "1d6+3");
        assert_eq!(atk(&g, "spear").ability, "dex");
        assert!(res(&g, "focus").is_none());
    }

    #[test]
    fn bard_inspiration_recharge() {
        let c = pc("2014", "bard", 5, [8, 14, 12, 10, 12, 16], &[]);
        let g = generate(&c, &Book::srd("2014"), None);
        let r = res(&g, "bardic-inspiration").unwrap();
        assert_eq!((r.max, r.recharge.as_str()), (3, "short"));
        assert_eq!(g.spells, Some(("已知法术", 8)));
    }

    #[test]
    fn local_override_merges() {
        let dir = std::env::temp_dir().join(format!("talkdnd-local-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(LOCAL_DIR)).unwrap();
        fs::write(
            dir.join(LOCAL_DIR).join("mine.yaml"),
            "ruleset: \"2024\"\nclasses:\n  fighter:\n    subclasses:\n      test-knight:\n        name: Test Knight\n        features:\n          - [3, Shiny Shield, synthetic]\n  tester:\n    hit_die: 10\n    weapons: [simple]\n    features:\n      - [1, Probe, synthetic]\n    resources:\n      - {id: probe, name: Probe, recharge: short, formula: prof}\nweapons:\n  - {id: test-blade, name: Test Blade, category: martial, dice: 1d10, type: slashing}\n",
        )
        .unwrap();
        let b = Book::load("2024", Some(&dir)).unwrap();
        assert_eq!(b.local_files.len(), 1);
        let mut c = pc(
            "2024",
            "fighter",
            3,
            [16, 12, 14, 10, 10, 8],
            &["test blade"],
        );
        c.subclass = Some("test-knight".into());
        let g = generate(&c, &b, Some(&dir));
        assert!(g.features.iter().any(|f| f.1 == "Shiny Shield"));
        assert!(atk(&g, "test-blade").proficient);
        assert!(g.warnings.iter().all(|w| !w.contains("subclass")));
        let c = pc("2024", "tester", 1, [10; 6], &[]);
        let g = generate(&c, &b, Some(&dir));
        assert_eq!(res(&g, "probe").unwrap().max, 2);
        assert_eq!(res(&g, "hit-dice").unwrap().name, "Hit Dice (d10)");
        // Not loaded for the other ruleset.
        assert!(Book::load("2014", Some(&dir))
            .unwrap()
            .class("tester")
            .is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_data_prints_command() {
        let dir = std::env::temp_dir().join(format!("talkdnd-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Rules/2024")).unwrap();
        let mut c = pc("2024", "fighter", 3, [10; 6], &[]);
        c.subclass = Some("nonesuch".into());
        let g = generate(&c, &Book::srd("2024"), Some(&dir));
        let w = g.warnings.join("\n");
        assert!(w.contains("python -m pdf2md run"), "{w}");
        assert!(w.contains("talkdnd data import --ruleset 2024"), "{w}");
        let md = dir.join("Rules/2024/.md/Book/Book.md");
        fs::create_dir_all(md.parent().unwrap()).unwrap();
        fs::write(&md, "x").unwrap();
        let g = generate(&c, &Book::srd("2024"), Some(&dir));
        let w = g.warnings.join("\n");
        assert!(
            !w.contains("pdf2md run") && w.contains("talkdnd data import --ruleset 2024"),
            "{w}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn importer_reads_2014_layout() {
        let dir = std::env::temp_dir().join(format!("talkdnd-imp14-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.md");
        fs::write(
            &f,
            "## Cleric\n### Divine Domains\nintro\n### Life Domain\n#### Bonus Proficiency\n### Gear Domain\n#### Gear Domain Spells\n#### Cleric Level Spells\n#### Cog Blessing\nAt 1st level, you tinker.\n#### Channel Divinity:\n#### Wind Up\nStarting at 2nd level, you wind.\n#### Spring Strike\nAt 8th level, you strike.\n### Martial Archetypes\n### Champion\n### Second Wind\n#### Improved Critical\nBeginning when you choose this archetype at 3rd level\n### Gizmo Knight\n#### Cog Shield\nWhen you choose this archetype at 3rd level, shield.\n## Monk\n### Way of Nothing\n",
        )
        .unwrap();
        let df = import_markdown("2014", &Book::srd("2014"), &[f]).unwrap();
        let g = &df.classes["cleric"].subclasses["gear-domain"];
        let names: Vec<_> = g.features.iter().map(|f| (f.0, f.1.as_str())).collect();
        assert_eq!(
            names,
            vec![
                (1, "Gear Domain Spells"),
                (1, "Cog Blessing"),
                (2, "Channel Divinity: Wind Up"),
                (8, "Spring Strike")
            ]
        );
        assert!(!df.classes["cleric"].subclasses.contains_key("life-domain"));
        assert!(!df.classes["cleric"].subclasses.contains_key("life"));
        let k = &df.classes["fighter"].subclasses;
        assert!(
            k.contains_key("gizmo-knight") && !k.contains_key("second-wind"),
            "{k:?}"
        );
        assert!(!df.classes.contains_key("monk"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn importer_reads_synthetic_markdown() {
        let dir = std::env::temp_dir().join(format!("talkdnd-imp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.md");
        fs::write(
            &f,
            "## Fighter Subclasses\n### Champion\n### Level 3: Improved Critical\n### Gizmo Knight\nbody\n### Level 3: Cog Shield\n### EVEL7: :SPRING STEP\n## Monk\n### Level 1: Not A Subclass\n",
        )
        .unwrap();
        let df = import_markdown("2024", &Book::srd("2024"), &[f]).unwrap();
        let s = &df.classes["fighter"].subclasses["gizmo-knight"];
        assert_eq!(s.name.as_deref(), Some("Gizmo Knight"));
        let names: Vec<_> = s.features.iter().map(|f| (f.0, f.1.as_str())).collect();
        assert_eq!(names, vec![(3, "Cog Shield"), (7, "Spring Step")]);
        assert!(!df.classes["fighter"].subclasses.contains_key("champion"));
        assert_eq!(df.classes.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reimport_backs_up_a_better_import_and_stale_checks_the_importer_version() {
        let root = std::env::temp_dir().join(format!("talkdnd-reimport-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let md = root.join("Rules/2024/.md/Book/Book.md");
        fs::create_dir_all(md.parent().unwrap()).unwrap();
        fs::write(
            &md,
            "## Fighter Subclasses\n### Gizmo Knight\nbody\n### Level 3: Cog Shield\n",
        )
        .unwrap();
        let (out, subs, _bgs) = run_import(&root, "2024").unwrap();
        assert_eq!(subs, 1);
        assert!(!import_stale(&root, "2024"), "a fresh import is current");
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("importer_version: 2"), "{text}");

        // A worse re-import keeps the old file and still writes the new one.
        fs::write(&md, "## Monk\n### Level 1: Not A Subclass\n").unwrap();
        let (_out, subs, _bgs) = run_import(&root, "2024").unwrap();
        assert_eq!(subs, 0);
        let backup = root.join(LOCAL_DIR).join("imported-2024.yaml.bak");
        let old = fs::read_to_string(&backup).unwrap();
        assert!(old.contains("gizmo-knight"), "{old}");
        let text = fs::read_to_string(root.join(LOCAL_DIR).join("imported-2024.yaml")).unwrap();
        assert!(!text.contains("gizmo-knight"), "{text}");

        // An import from an older importer is stale even when it is newer on disk.
        let p = root.join(LOCAL_DIR).join("imported-2024.yaml");
        fs::write(&p, "ruleset: \"2024\"\nclasses: {}\nbackgrounds: {}\n").unwrap();
        assert!(import_stale(&root, "2024"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn importer_2014_known_subclasses_untangle_columns() {
        let dir = std::env::temp_dir().join(format!("talkdnd-imp14k-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.md");
        // Two OCR columns woven together: Tempest headings inside Nature's run,
        // War features before the War Domain heading, one lost name heading.
        fs::write(
            &f,
            "## Cleric\n### Divine Domains\n### Nature Domain\n#### Nature Domain Spells\nx\n#### Acolyte of Nature\nAt 1st level, you learn.\n#### Tempest Domain Spells\nx\n#### Wrath of the Storm\nAlso at 1st level, you rebuke.\n#### Master of Nature\nAt 17th level, you command.\n### Tempest Domain\n#### Thunderbolt Strike\nAt 6th level, you push.\n#### Trickery Domain Spells\n#### Blessing of the Trickster\nStarting at 1st level, you bless.\n#### War Priest\nFrom 1st level, you attack.\n#### Improved Duplicity\nAt 17th level, you copy.\n#### War Domain\nintro\n#### Avatar of Battle\nAt 17th level, you resist.\n#### Oath Lore\nNot a feature.\n## Druid\n### Druid Circles\n",
        )
        .unwrap();
        let df = import_markdown("2014", &Book::srd("2014"), &[f]).unwrap();
        let names = |s: &str| -> Vec<(i32, String)> {
            df.classes["cleric"].subclasses[s]
                .features
                .iter()
                .map(|f| (f.0, f.1.clone()))
                .collect()
        };
        let n = |v: &[(i32, &str)]| {
            v.iter()
                .map(|(a, b)| (*a, b.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("nature-domain"),
            n(&[
                (1, "Nature Domain Spells"),
                (1, "Acolyte Of Nature"),
                (17, "Master Of Nature")
            ])
        );
        assert_eq!(
            names("tempest-domain"),
            n(&[
                (1, "Tempest Domain Spells"),
                (1, "Wrath Of The Storm"),
                (6, "Thunderbolt Strike")
            ])
        );
        assert_eq!(
            names("trickery-domain"),
            n(&[
                (1, "Trickery Domain Spells"),
                (1, "Blessing Of The Trickster"),
                (17, "Improved Duplicity")
            ])
        );
        assert_eq!(
            names("war-domain"),
            n(&[(1, "War Priest"), (17, "Avatar Of Battle")])
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod background_import_tests {
    use super::*;

    // Synthetic text in the books' layout (no book prose).
    #[test]
    fn imports_2014_backgrounds_even_when_fields_precede_the_heading() {
        let t = "#### Tester\n\nFlavor.\n\n#### Skill Proficiencies: Insight, History\n\n**Languages:** Two of your choice\n\n**Equipment:** A quill, a set of common clothes, and a belt pouch containing 10 gp\n\n#### Feature: Test Feature\n\nText.\n\n#### Suggested Characteristics\n\nx\n\n**Skill Proficiencies:** History, Persuasion\n\n**Tool Proficiencies:** One type of gaming set\n\n**Languages:** One of your choice **Equipment:** A set of fine clothes, a signet ring, and a purse containing 25 gp\n\n#### Other Testers\n\n#### Heir\n\nFlavor.\n\n#### Feature: Heir Privilege\n";
        let b = import_backgrounds("2014", t);
        let a = &b["tester"];
        assert_eq!(a.feature.as_deref(), Some("Test Feature"));
        assert_eq!(a.languages, 2);
        assert_eq!(a.gold, 10);
        assert_eq!(a.skills, vec!["insight", "history"]);
        let h = &b["heir"];
        assert_eq!(h.feature.as_deref(), Some("Heir Privilege"));
        assert_eq!(h.equipment, vec!["fine clothes", "signet ring", "purse"]);
        assert_eq!(h.gold, 25);
        assert_eq!(h.tools, vec!["One type of gaming set"]);
        assert_eq!(h.languages, 1);
    }

    #[test]
    fn imports_2024_background_options() {
        let t = "### TESTER\n\nAbility Scores: Strength, Dexterity, Wisdom Feat: Alert (see chapter 5) Skill Proficiencies: Athletics and Stealth Tool Proficiency: Thieves' Tools\n\nEquipment: Choose A or 8: (A) Dagger, Rope, 12 CP; or (8) 50 GP\n";
        let b = import_backgrounds("2024", t);
        let x = &b["tester"];
        assert_eq!(x.feat.as_deref(), Some("Alert"));
        assert_eq!(x.skills, vec!["athletics", "stealth"]);
        assert_eq!(x.options["A"].items, vec!["Dagger", "Rope"]);
        assert_eq!(x.options["A"].gold, 12);
        assert_eq!(x.options["B"].gold, 50);
    }
}
