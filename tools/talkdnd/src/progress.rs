//! XP, level-up and loot/gold bookkeeping for a save.
//!
//! The ledger is `Saves/<slot>/progress.yaml` (XP per character, coins and
//! items per character plus a shared `party` pool). Level-up edits the
//! character sheet's `combat yaml` block (level, HP, proficiency, hit dice and
//! proficiency-based bonuses) and lists the class features gained; choices
//! (ASI, subclass, spells) stay with the player and the DM.

use crate::check::{self, Sheet};
use crate::{classdata, data, save};
use anyhow::{anyhow, bail, Context, Result};
use combat_sim::dice::{DiceRng, RollMode};
use combat_sim::roll::roll_command;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE: &str = "progress.yaml";
pub const PARTY: &str = "party";

/// XP needed for each level (index 0 = level 1); same in 2014 and 2024.
pub const XP_TABLE: [i64; 20] = [
    0, 300, 900, 2700, 6500, 14000, 23000, 34000, 48000, 64000, 85000, 100000, 120000, 140000,
    165000, 195000, 225000, 265000, 305000, 355000,
];

pub fn level_for_xp(xp: i64) -> i32 {
    XP_TABLE.iter().rposition(|t| xp >= *t).unwrap_or(0) as i32 + 1
}

pub fn prof_for_level(level: i32) -> i32 {
    2 + (level.clamp(1, 20) - 1) / 4
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Purse {
    /// Total value in copper pieces (coins are kept as one value; `show`
    /// prints it as gp/sp/cp).
    #[serde(default)]
    pub cp: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Progress {
    #[serde(default)]
    pub xp: BTreeMap<String, i64>,
    #[serde(default)]
    pub purses: BTreeMap<String, Purse>,
}

impl Progress {
    pub fn path(save: &Path) -> PathBuf {
        save.join(FILE)
    }
    pub fn load(save: &Path) -> Result<Self> {
        let p = Self::path(save);
        if !p.is_file() {
            return Ok(Self::default());
        }
        serde_norway::from_str(&fs::read_to_string(&p)?)
            .with_context(|| format!("{} is not valid", p.display()))
    }
    pub fn store(&self, save: &Path) -> Result<()> {
        let head = "# TalkDND progress ledger: XP per character, coins (as cp) and items per character and the shared `party` pool.\n# Edit with `talkdnd xp` / `talkdnd loot`; hand edits are fine.\n";
        fs::write(
            Self::path(save),
            head.to_string() + &serde_norway::to_string(self)?,
        )?;
        Ok(())
    }
}

/// `12gp 5sp 3cp` style.
pub fn coins(cp: i64) -> String {
    let sign = if cp < 0 { "-" } else { "" };
    let cp = cp.abs();
    let (gp, sp, c) = (cp / 100, (cp % 100) / 10, cp % 10);
    let mut parts = Vec::new();
    if gp > 0 {
        parts.push(format!("{gp} gp"));
    }
    if sp > 0 {
        parts.push(format!("{sp} sp"));
    }
    if c > 0 || parts.is_empty() {
        parts.push(format!("{c} cp"));
    }
    format!("{sign}{}", parts.join(" "))
}

/// Coin flags to copper: pp 1000, gp 100, ep 50, sp 10, cp 1.
pub fn to_cp(pp: i64, gp: i64, ep: i64, sp: i64, cp: i64) -> i64 {
    pp * 1000 + gp * 100 + ep * 50 + sp * 10 + cp
}

fn log(save_dir: &Path, line: &str) -> Result<()> {
    if let Some(l) = save::latest_log(save_dir) {
        save::append(
            &l,
            &format!("- [{}] {line}", chrono::Local::now().format("%H:%M")),
        )?;
    }
    Ok(())
}

/// Recipients: `all`/`party` = campaign party list, else comma-separated
/// names or ids resolved against the sheets.
fn recipients(save_dir: &Path, to: &str, sheets: &[Sheet]) -> Result<Vec<String>> {
    if to == "all" || to == PARTY {
        let ids = crate::start::party_ids(save_dir);
        if ids.is_empty() {
            bail!("the campaign has no party list; name the characters (--to a,b)");
        }
        return Ok(ids);
    }
    to.split(',')
        .map(|w| check::find(sheets, w.trim()).map(|s| s.id.clone()))
        .collect()
}

fn sheet_level(s: &Sheet) -> i32 {
    s.block.get("level").and_then(|v| v.as_i64()).unwrap_or(1) as i32
}

pub fn xp_add(save_dir: &Path, amount: i64, to: &str, reason: Option<&str>) -> Result<Vec<String>> {
    let sheets = check::sheets(save_dir);
    let ids = recipients(save_dir, to, &sheets)?;
    let mut p = Progress::load(save_dir)?;
    let each = amount / ids.len() as i64;
    let mut out = Vec::new();
    for id in &ids {
        let e = p.xp.entry(id.clone()).or_insert(0);
        *e += each;
        let lv = sheets
            .iter()
            .find(|s| &s.id == id)
            .map(sheet_level)
            .unwrap_or(1);
        let mut line = format!("{id}: {} XP (level {lv})", *e);
        if level_for_xp(*e) > lv {
            line += &format!(
                " — can reach level {}: `talkdnd level-up {id}`",
                level_for_xp(*e)
            );
        }
        out.push(line);
    }
    if amount % ids.len() as i64 != 0 {
        out.push(format!(
            "{} XP left over after the even split",
            amount % ids.len() as i64
        ));
    }
    p.store(save_dir)?;
    let why = reason.map(|r| format!("（{r}）")).unwrap_or_default();
    log(
        save_dir,
        &format!("经验{why}：{amount} XP，{} 人各 {each}", ids.len()),
    )?;
    Ok(out)
}

pub fn xp_show(save_dir: &Path) -> Result<Vec<String>> {
    let sheets = check::sheets(save_dir);
    let p = Progress::load(save_dir)?;
    let mut out = Vec::new();
    for s in &sheets {
        let xp = p.xp.get(&s.id).copied().unwrap_or(0);
        let lv = sheet_level(s);
        let next = XP_TABLE
            .get(lv as usize)
            .map_or("max".to_string(), |t| t.to_string());
        let ready = if level_for_xp(xp) > lv {
            "  ← level-up ready"
        } else {
            ""
        };
        out.push(format!(
            "{} ({}): level {lv}, {xp} XP, next {next}{ready}",
            s.name, s.id
        ));
    }
    Ok(out)
}

/// What the level-up changed, for printing.
#[derive(Debug, Default)]
pub struct LevelUp {
    pub lines: Vec<String>,
}

fn block_bounds(text: &str) -> Option<(usize, usize)> {
    let start = text.find("```combat yaml")?;
    let body = start + text[start..].find('\n')? + 1;
    let end = body + text[body..].find("\n```")?;
    Some((body, end))
}

pub(crate) fn class_of(s: &Sheet, text: &str, book: &classdata::Book) -> Option<String> {
    let notes = s
        .block
        .get("notes")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    // The `等级`/`Level` line of generated sheets, else the top of a hand-made one.
    let header = text
        .lines()
        .find(|l| l.contains("**等级**") || l.to_lowercase().contains("**level**"))
        .map(str::to_string)
        .unwrap_or_else(|| text.lines().take(8).collect::<Vec<_>>().join(" "))
        .to_lowercase();
    const ZH: &[(&str, &str)] = &[
        ("野蛮人", "barbarian"),
        ("吟游诗人", "bard"),
        ("牧师", "cleric"),
        ("德鲁伊", "druid"),
        ("战士", "fighter"),
        ("武僧", "monk"),
        ("圣武士", "paladin"),
        ("游侠", "ranger"),
        ("游荡者", "rogue"),
        ("术士", "sorcerer"),
        ("邪术师", "warlock"),
        ("魔契师", "warlock"),
        ("法师", "wizard"),
    ];
    book.classes
        .keys()
        .find(|c| {
            notes.split_whitespace().next() == Some(c.as_str()) || header.contains(c.as_str())
        })
        .cloned()
        .or_else(|| {
            // Longest match first ("术士" is inside other words less often than "士").
            let mut zh: Vec<_> = ZH.iter().filter(|(z, _)| header.contains(z)).collect();
            zh.sort_by_key(|(z, _)| std::cmp::Reverse(z.chars().count()));
            zh.first().map(|(_, c)| c.to_string())
        })
}

pub(crate) fn hit_die(s: &Sheet, class: Option<&str>, book: &classdata::Book) -> Option<i32> {
    s.block
        .get("resources")
        .and_then(|r| r.get("hit-dice"))
        .and_then(|h| h.get("die"))
        .and_then(|d| d.as_i64())
        .map(|d| d as i32)
        .or_else(|| {
            class
                .and_then(|c| book.classes.get(c))
                .and_then(|c| c.hit_die)
        })
        .or_else(|| class.and_then(data::class).map(|x| x.hit_die))
}

/// Rewrites `key: N` at the start of a block line.
fn set_scalar(lines: &mut [String], key: &str, val: i64) -> bool {
    for l in lines.iter_mut() {
        if l.starts_with(&format!("{key}:")) {
            let comment = l.find(" #").map(|i| l[i..].to_string()).unwrap_or_default();
            *l = format!("{key}: {val}{comment}");
            return true;
        }
    }
    false
}

/// `saves: {dex: 6, int: 1}` with each value passed through `f`.
fn map_flow(lines: &mut [String], key: &str, f: &dyn Fn(&str, i64) -> i64) {
    for l in lines.iter_mut() {
        let Some(rest) = l.strip_prefix(&format!("{key}:")) else {
            continue;
        };
        let inner = rest.trim().trim_start_matches('{').trim_end_matches('}');
        let parts: Vec<String> = inner
            .split(',')
            .filter_map(|kv| {
                let (k, v) = kv.split_once(':')?;
                let (k, v) = (k.trim(), v.trim());
                Some(match v.parse::<i64>() {
                    Ok(n) => format!("{k}: {}", f(k, n)),
                    Err(_) => format!("{k}: {v}"),
                })
            })
            .collect();
        *l = format!("{key}: {{{}}}", parts.join(", "));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn level_up(
    save_dir: &Path,
    root: &Path,
    who: &str,
    hp_mode: &str,
    class_arg: Option<&str>,
    milestone: bool,
    rng: &mut dyn DiceRng,
    seeded: bool,
) -> Result<LevelUp> {
    let sheets = check::sheets(save_dir);
    let s = check::find(&sheets, who)?;
    let text = fs::read_to_string(&s.file)?;
    let (b0, b1) = block_bounds(&text)
        .ok_or_else(|| anyhow!("{} has no combat yaml block", s.file.display()))?;
    let old = sheet_level(s);
    if old >= 20 {
        bail!("{} is already level 20", s.name);
    }
    let new = old + 1;
    let p = Progress::load(save_dir)?;
    let xp = p.xp.get(&s.id).copied().unwrap_or(0);
    if !milestone && xp < XP_TABLE[old as usize] {
        bail!(
            "{} has {xp} XP; level {new} needs {} (pass --milestone for milestone levelling)",
            s.name,
            XP_TABLE[old as usize]
        );
    }
    let rs = save::ruleset(save_dir);
    let book = classdata::Book::load(&rs, Some(root))?;
    let class = class_arg
        .map(str::to_string)
        .or_else(|| class_of(s, &text, &book));
    let die = hit_die(s, class.as_deref(), &book).ok_or_else(|| {
        anyhow!(
            "cannot tell {}'s class or hit die; pass --class <id>",
            s.name
        )
    })?;
    let con = s
        .block
        .get("abilities")
        .and_then(|a| a.get("con"))
        .and_then(|v| v.as_i64())
        .unwrap_or(10);
    let con_mod = (con - 10).div_euclid(2);
    let (gain_die, how) = match hp_mode {
        "avg" | "average" => (i64::from(die / 2 + 1), format!("average {}", die / 2 + 1)),
        "roll" => {
            let r = roll_command(&format!("1d{die}"), RollMode::Normal, None, false, rng)?;
            (r.total as i64, r.text)
        }
        n => {
            let v: i64 = n
                .parse()
                .map_err(|_| anyhow!("--hp is avg, roll or a number"))?;
            if v < 1 || v > i64::from(die) {
                bail!("--hp {v} is outside 1..={die}");
            }
            (v, format!("chosen {v}"))
        }
    };
    let gain = (gain_die + con_mod).max(1);
    let old_prof = prof_for_level(old) as i64;
    let new_prof = prof_for_level(new) as i64;
    let d = new_prof - old_prof;
    let abilities: BTreeMap<String, i64> = ["str", "dex", "con", "int", "wis", "cha"]
        .iter()
        .map(|a| {
            let v = s
                .block
                .get("abilities")
                .and_then(|x| x.get(*a))
                .and_then(|v| v.as_i64())
                .unwrap_or(10);
            (a.to_string(), (v - 10).div_euclid(2))
        })
        .collect();
    let mut lines: Vec<String> = text[b0..b1].lines().map(str::to_string).collect();
    let hp = s.block.get("hp").and_then(|v| v.as_i64()).unwrap_or(0);
    set_scalar(&mut lines, "level", new as i64);
    set_scalar(&mut lines, "hp", hp + gain);
    set_scalar(&mut lines, "prof", new_prof);
    if d != 0 {
        // Saves listed in the block are proficient ones.
        map_flow(&mut lines, "saves", &|_, v| v + d);
        // Skills: proficient (mod + prof) gain d, expertise (mod + 2 prof) 2d.
        map_flow(&mut lines, "skills", &|k, v| {
            let m = data::skill_ability(k)
                .and_then(|a| abilities.get(a))
                .copied()
                .unwrap_or(0);
            match v - m {
                x if x == 2 * old_prof => v + 2 * d,
                x if x == old_prof => v + d,
                _ => v,
            }
        });
        // Weapon/spell attacks with a to_hit number.
        for l in lines.iter_mut() {
            if let Some(i) = l.find("to_hit: ") {
                let tail = &l[i + 8..];
                let n: String = tail
                    .chars()
                    .take_while(|c| *c == '-' || c.is_ascii_digit())
                    .collect();
                if let Ok(v) = n.parse::<i64>() {
                    *l = format!("{}to_hit: {}{}", &l[..i], v + d, &tail[n.len()..]);
                }
            }
        }
    }
    // Hit dice pool.
    for l in lines.iter_mut() {
        if l.trim_start().starts_with("hit-dice:") {
            if let Some(i) = l.find("max: ") {
                let tail = &l[i + 5..];
                let n: String = tail.chars().take_while(char::is_ascii_digit).collect();
                *l = format!("{}max: {new}{}", &l[..i], &tail[n.len()..]);
            }
        }
    }
    // Notes "fighter 1" -> "fighter 2".
    if let Some(c) = &class {
        for l in lines.iter_mut() {
            if l.starts_with("notes:") {
                *l = l.replacen(&format!("{c} {old}"), &format!("{c} {new}"), 1);
            }
        }
    }
    let mut out = text[..b0].to_string() + &lines.join("\n") + &text[b1..];
    let stamp = chrono::Local::now().format("%Y-%m-%d");
    let note = format!(
        "- {stamp}：升到 {new} 级；HP +{gain}（d{die} {how}，体质 {con_mod:+}）→ {}；熟练 +{new_prof}",
        hp + gain
    );
    if out.contains("\n## 升级记录\n") {
        out = out.trim_end().to_string() + "\n" + &note + "\n";
    } else {
        out = out.trim_end().to_string() + "\n\n## 升级记录\n\n" + &note + "\n";
    }
    fs::write(&s.file, out)?;
    let mut r = LevelUp::default();
    r.lines.push(format!(
        "{} → level {new}: HP {hp} → {} (+{gain}: d{die} {how}, CON {con_mod:+}); proficiency +{new_prof}{}",
        s.name,
        hp + gain,
        if d != 0 { " (saves, proficient skills and to-hit adjusted)" } else { "" }
    ));
    if let Some(c) = class.as_deref().and_then(|c| book.classes.get(c)) {
        for f in c.features.iter().filter(|f| f.0 == new) {
            r.lines.push(format!("new feature: {} — {}", f.1, f.2));
        }
        if c.subclass_level == Some(new) {
            r.lines
                .push("choose a subclass now (`talkdnd options` / local data)".into());
        }
        if c.asi.as_ref().is_some_and(|a| a.contains(&new)) {
            r.lines.push(
                "Ability Score Improvement (or a feat): the player chooses; edit the sheet".into(),
            );
        }
    }
    r.lines.push("the sheet prose (header, features, spells) is not rewritten: update it with the player; then `combat-sim party sync` to refresh party HP".into());
    let tag = if seeded { "（测试种子）" } else { "" };
    log(
        save_dir,
        &format!("升级{tag}：{} {old}→{new}，HP +{gain}（{how}）", s.name),
    )?;
    Ok(r)
}

pub enum LootOp<'a> {
    Add {
        to: &'a str,
        cp: i64,
        items: &'a [String],
    },
    Spend {
        from: &'a str,
        cp: i64,
        items: &'a [String],
    },
    Give {
        from: &'a str,
        to: &'a str,
        cp: i64,
        items: &'a [String],
    },
    Split,
}

fn purse_key(save_dir: &Path, who: &str, sheets: &[Sheet]) -> Result<String> {
    if who == PARTY || who == "all" {
        return Ok(PARTY.into());
    }
    let _ = save_dir;
    Ok(check::find(sheets, who)?.id.clone())
}

fn take(p: &mut Purse, who: &str, cp: i64, items: &[String]) -> Result<()> {
    if cp > p.cp {
        bail!("{who} has only {}", coins(p.cp));
    }
    for it in items {
        let i = p
            .items
            .iter()
            .position(|x| x.eq_ignore_ascii_case(it))
            .ok_or_else(|| anyhow!("{who} has no `{it}`"))?;
        p.items.remove(i);
    }
    p.cp -= cp;
    Ok(())
}

pub fn loot(save_dir: &Path, op: LootOp, reason: Option<&str>) -> Result<Vec<String>> {
    let sheets = check::sheets(save_dir);
    let mut p = Progress::load(save_dir)?;
    let why = reason.map(|r| format!("（{r}）")).unwrap_or_default();
    let what = |cp: i64, items: &[String]| {
        let mut v = Vec::new();
        if cp != 0 {
            v.push(coins(cp));
        }
        v.extend(items.iter().cloned());
        v.join("、")
    };
    let line = match op {
        LootOp::Add { to, cp, items } => {
            let k = purse_key(save_dir, to, &sheets)?;
            let e = p.purses.entry(k.clone()).or_default();
            e.cp += cp;
            e.items.extend(items.iter().cloned());
            format!("战利品{why}：{k} 获得 {}", what(cp, items))
        }
        LootOp::Spend { from, cp, items } => {
            let k = purse_key(save_dir, from, &sheets)?;
            take(p.purses.entry(k.clone()).or_default(), &k, cp, items)?;
            format!("花费{why}：{k} 支出 {}", what(cp, items))
        }
        LootOp::Give {
            from,
            to,
            cp,
            items,
        } => {
            let a = purse_key(save_dir, from, &sheets)?;
            let b = purse_key(save_dir, to, &sheets)?;
            take(p.purses.entry(a.clone()).or_default(), &a, cp, items)?;
            let e = p.purses.entry(b.clone()).or_default();
            e.cp += cp;
            e.items.extend(items.iter().cloned());
            format!("转交{why}：{a} → {b} {}", what(cp, items))
        }
        LootOp::Split => {
            let ids = crate::start::party_ids(save_dir);
            if ids.is_empty() {
                bail!("the campaign has no party list");
            }
            let pool = p.purses.entry(PARTY.into()).or_default();
            let each = pool.cp / ids.len() as i64;
            pool.cp -= each * ids.len() as i64;
            for id in &ids {
                p.purses.entry(id.clone()).or_default().cp += each;
            }
            format!(
                "分钱{why}：公共钱袋每人 {}，余 {}",
                coins(each),
                coins(p.purses[PARTY].cp)
            )
        }
    };
    p.store(save_dir)?;
    log(save_dir, &line)?;
    let mut out = vec![line];
    out.extend(loot_show_with(&p));
    Ok(out)
}

fn loot_show_with(p: &Progress) -> Vec<String> {
    p.purses
        .iter()
        .map(|(k, v)| {
            let items = if v.items.is_empty() {
                String::new()
            } else {
                format!("; {}", v.items.join(", "))
            };
            format!("  {k}: {}{items}", coins(v.cp))
        })
        .collect()
}

pub fn loot_show(save_dir: &Path) -> Result<Vec<String>> {
    Ok(loot_show_with(&Progress::load(save_dir)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("talkdnd-prog-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Characters")).unwrap();
        fs::create_dir_all(d.join("combat")).unwrap();
        fs::write(
            d.join("combat/campaign.yaml"),
            "party: [tess, quill]\nruleset: \"2014\"\n",
        )
        .unwrap();
        let sheet = |id: &str, name: &str, lvl: i32, extra: &str| {
            format!("# {name}\n\n- **等级**：fighter {lvl}\n\n```combat yaml\nid: {id}\nname: {name}\nkind: pc\nlevel: {lvl}\nhp: 30\nprof: 2\nabilities: {{str: 16, dex: 14, con: 14, int: 10, wis: 12, cha: 8}}\nsaves: {{str: 5, con: 4}}\nskills: {{athletics: 5, stealth: 6, insight: 1}}\nresources:\n  hit-dice: {{max: {lvl}, recharge: long, die: 10}}\nattacks:\n  - {{id: blade, name: Blade, kind: melee, ability: str, to_hit: 5, damage: [{{dice: \"1d8+3\", type: slashing}}], reach: 5}}\nnotes: \"fighter {lvl}. synthetic\"\n{extra}```\n")
        };
        fs::write(d.join("Characters/Tess.md"), sheet("tess", "Tess", 4, "")).unwrap();
        fs::write(
            d.join("Characters/Quill.md"),
            sheet("quill", "Quill", 1, ""),
        )
        .unwrap();
        d
    }

    #[test]
    fn xp_table_and_coins() {
        assert_eq!(level_for_xp(0), 1);
        assert_eq!(level_for_xp(299), 1);
        assert_eq!(level_for_xp(300), 2);
        assert_eq!(level_for_xp(400000), 20);
        assert_eq!(prof_for_level(4), 2);
        assert_eq!(prof_for_level(5), 3);
        assert_eq!(prof_for_level(17), 6);
        assert_eq!(coins(12345), "123 gp 4 sp 5 cp");
        assert_eq!(coins(0), "0 cp");
        assert_eq!(to_cp(1, 2, 1, 3, 4), 1000 + 200 + 50 + 30 + 4);
    }

    #[test]
    fn xp_split_and_level_up_with_proficiency_bump() {
        let d = save_dir("lv");
        let lines = xp_add(&d, 13001, "all", Some("test")).unwrap();
        assert!(lines[0].starts_with("tess: 6500 XP"), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("1 XP left over")));
        let mut rng = combat_sim::dice::seeded_rng(1);
        let r = level_up(&d, &d, "Tess", "avg", None, false, &mut rng, true).unwrap();
        assert!(r.lines[0].contains("HP 30 → 38"), "{:?}", r.lines);
        let t = fs::read_to_string(d.join("Characters/Tess.md")).unwrap();
        for want in [
            "level: 5",
            "hp: 38",
            "prof: 3",
            "saves: {str: 6, con: 5}",
            "skills: {athletics: 6, stealth: 8, insight: 1}",
            "to_hit: 6",
            "max: 5,",
            "fighter 5.",
            "## 升级记录",
        ] {
            assert!(t.contains(want), "missing {want}:\n{t}");
        }
        // XP gate: Quill has 6500 too, Tess now needs 14000 for level 6.
        assert!(level_up(&d, &d, "tess", "avg", None, false, &mut rng, true).is_err());
        assert!(level_up(&d, &d, "tess", "11", None, true, &mut rng, true).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn loot_add_give_split_spend() {
        let d = save_dir("loot");
        loot(
            &d,
            LootOp::Add {
                to: "party",
                cp: to_cp(0, 21, 0, 0, 1),
                items: &["rope".into()],
            },
            None,
        )
        .unwrap();
        loot(
            &d,
            LootOp::Give {
                from: "party",
                to: "quill",
                cp: 100,
                items: &["rope".into()],
            },
            None,
        )
        .unwrap();
        loot(&d, LootOp::Split, None).unwrap();
        let p = Progress::load(&d).unwrap();
        assert_eq!(p.purses["party"].cp, 1);
        assert_eq!(p.purses["tess"].cp, 1000);
        assert_eq!(p.purses["quill"].cp, 1100);
        assert_eq!(p.purses["quill"].items, vec!["rope".to_string()]);
        assert!(loot(
            &d,
            LootOp::Spend {
                from: "tess",
                cp: 1001,
                items: &[]
            },
            None
        )
        .is_err());
        assert!(loot(
            &d,
            LootOp::Spend {
                from: "tess",
                cp: 0,
                items: &["rope".into()]
            },
            None
        )
        .is_err());
        loot(
            &d,
            LootOp::Spend {
                from: "quill",
                cp: 100,
                items: &["rope".into()],
            },
            None,
        )
        .unwrap();
        assert!(Progress::load(&d).unwrap().purses["quill"].items.is_empty());
        let _ = fs::remove_dir_all(&d);
    }
}
