//! `talkdnd sheet export`: one printable A4 page per character, built from the
//! sheet Markdown (header, sections) and its combat yaml block (numbers).
//!
//! PDF: drawn with krilla (CJK font subset-embedded from a system font), then
//! fillable AcroForm fields (text boxes, tick boxes) are added with lopdf.
//! HTML: the same page with `<input>` fields, for browsers / "Print to PDF".

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde_norway::Value;

use crate::check::Sheet;
use crate::progress::Progress;

pub const ABILITIES: [(&str, &str); 6] = [
    ("str", "力量"),
    ("dex", "敏捷"),
    ("con", "体质"),
    ("int", "智力"),
    ("wis", "感知"),
    ("cha", "魅力"),
];

pub const SKILLS: [(&str, &str, &str); 18] = [
    ("acrobatics", "特技", "dex"),
    ("animal_handling", "驯兽", "wis"),
    ("arcana", "奥秘", "int"),
    ("athletics", "运动", "str"),
    ("deception", "欺瞒", "cha"),
    ("history", "历史", "int"),
    ("insight", "洞悉", "wis"),
    ("intimidation", "威吓", "cha"),
    ("investigation", "调查", "int"),
    ("medicine", "医药", "wis"),
    ("nature", "自然", "int"),
    ("perception", "察觉", "wis"),
    ("performance", "表演", "cha"),
    ("persuasion", "游说", "cha"),
    ("religion", "宗教", "int"),
    ("sleight_of_hand", "巧手", "dex"),
    ("stealth", "隐匿", "dex"),
    ("survival", "求生", "wis"),
];

#[derive(Debug, Default, Clone)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub file: PathBuf,
    /// Labelled header values (玩家, 等级, 种族, 背景, 阵营, 规则, …).
    pub header: Vec<(String, String)>,
    /// Header bullets without a label (NPC one-liners).
    pub info: Vec<String>,
    pub level: i64,
    pub ac: i64,
    pub hp: i64,
    pub speed: String,
    pub init: i64,
    pub prof: i64,
    pub scores: BTreeMap<String, i64>,
    pub saves: BTreeMap<String, i64>,
    pub skills: BTreeMap<String, i64>,
    pub hit_die: Option<i64>,
    pub hit_dice: i64,
    /// (ability, attack, dc)
    pub casting: Option<(String, i64, i64)>,
    pub slots: Vec<(i64, i64)>,
    /// (name, max, recharge)
    pub resources: Vec<(String, i64, String)>,
    pub attacks: Vec<String>,
    pub features: Vec<String>,
    pub equipment: Vec<String>,
    pub spells: Vec<String>,
    pub personality: Vec<String>,
    pub proficiencies: Vec<String>,
    pub xp: Option<i64>,
    /// pp gp ep sp cp
    pub coins: [i64; 5],
}

fn int(v: Option<&Value>) -> Option<i64> {
    v.and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

fn map_i64(v: Option<&Value>) -> BTreeMap<String, i64> {
    let mut m = BTreeMap::new();
    if let Some(Value::Mapping(map)) = v {
        for (k, v) in map {
            if let (Some(k), Some(n)) = (k.as_str(), int(Some(v))) {
                m.insert(k.to_string(), n);
            }
        }
    }
    m
}

pub fn modifier(score: i64) -> i64 {
    (score - 10).div_euclid(2)
}

pub fn signed(n: i64) -> String {
    if n >= 0 {
        format!("+{n}")
    } else {
        format!("−{}", -n)
    }
}

/// Strip Markdown emphasis/links for plain printing.
pub fn plain(s: &str) -> String {
    let mut t = s.replace("**", "").replace("__", "").replace('`', "");
    t = t.replace("_", "");
    // [text](url) -> text
    while let (Some(a), Some(m)) = (t.find('['), t.find("](")) {
        if m < a {
            break;
        }
        let Some(e) = t[m..].find(')') else { break };
        let label = t[a + 1..m].to_string();
        t.replace_range(a..m + e + 1, &label);
    }
    t.trim().to_string()
}

fn section_kind(h: &str) -> Option<&'static str> {
    let l = h.to_lowercase();
    if l.contains("战斗数据块") || l.contains("combat-sim") {
        return None;
    }
    if l.contains("法术") || l.contains("spell") {
        Some("spells")
    } else if l.contains("装备") || l.contains("金钱") || l.contains("equipment") {
        Some("equipment")
    } else if l.contains("个性") || l.contains("性格") || l.contains("personality") {
        Some("personality")
    } else if l.contains("熟练") || l.contains("proficienc") || l.contains("语言") {
        Some("proficiencies")
    } else if l.contains("特性")
        || l.contains("feature")
        || l.contains("专长")
        || l.contains("待补")
    {
        Some("features")
    } else {
        None
    }
}

const HEADER_KEYS: [&str; 12] = [
    "玩家", "规则", "冒险", "剧本", "等级", "种族", "物种", "背景", "阵营", "形象", "性格", "职业",
];

fn parse_header_bullet(b: &str, header: &mut Vec<(String, String)>, info: &mut Vec<String>) {
    // `**k**：v　**k2**：v2`
    let mut found = false;
    for part in b.split('　') {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix("**") {
            if let Some(end) = rest.find("**") {
                let k = rest[..end].trim().to_string();
                let v = rest[end + 2..]
                    .trim_start_matches(['：', ':', ' '])
                    .to_string();
                if HEADER_KEYS.iter().any(|h| k.contains(h)) {
                    header.push((k, plain(&v)));
                    found = true;
                    continue;
                }
            }
        }
        if !p.is_empty() && found {
            // trailing unlabelled piece of a labelled bullet
            if let Some(last) = header.last_mut() {
                last.1 = format!("{} {}", last.1, plain(p));
            }
        }
    }
    if !found {
        info.push(plain(b));
    }
}

/// Parse a sheet file into a printable card.
pub fn card(sheet: &Sheet, progress: &Progress) -> Result<Card> {
    let text = fs::read_to_string(&sheet.file)
        .with_context(|| format!("read {}", sheet.file.display()))?;
    let b = &sheet.block;
    let mut c = Card {
        id: sheet.id.clone(),
        name: sheet.name.clone(),
        file: sheet.file.clone(),
        level: int(b.get("level")).unwrap_or(1),
        ac: int(b.get("ac")).unwrap_or(10),
        hp: int(b.get("hp")).unwrap_or(0),
        init: int(b.get("init_bonus")).unwrap_or(0),
        prof: int(b.get("prof")).unwrap_or(2),
        scores: map_i64(b.get("abilities")),
        saves: map_i64(b.get("saves")),
        skills: map_i64(b.get("skills")),
        ..Default::default()
    };
    if c.init == 0 {
        if let Some(d) = c.scores.get("dex") {
            c.init = modifier(*d);
        }
    }
    c.speed = match b.get("speed") {
        Some(Value::Mapping(m)) => m
            .iter()
            .filter_map(|(k, v)| Some(format!("{} {}", k.as_str()?, int(Some(v))?)))
            .map(|s| s.replace("walk ", ""))
            .collect::<Vec<_>>()
            .join(" / "),
        Some(v) => int(Some(v)).map(|n| n.to_string()).unwrap_or_default(),
        None => "30".into(),
    };
    if let Some(sc) = b.get("spellcasting") {
        c.casting = Some((
            sc.get("ability")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            int(sc.get("attack")).unwrap_or(0),
            int(sc.get("dc")).unwrap_or(0),
        ));
    }
    c.hit_dice = c.level;
    if let Some(Value::Mapping(res)) = b.get("resources") {
        for (k, v) in res {
            let Some(k) = k.as_str() else { continue };
            let max = int(v.get("max")).unwrap_or(1);
            let rech = v
                .get("recharge")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(l) = k.strip_prefix("spell-slot-") {
                if let Ok(l) = l.parse() {
                    c.slots.push((l, max));
                }
            } else if k == "hit-dice" {
                c.hit_dice = max;
                c.hit_die = int(v.get("die"));
            } else {
                c.resources.push((k.to_string(), max, rech));
            }
        }
        c.slots.sort();
    }
    if let Some(Value::Sequence(atks)) = b.get("attacks") {
        for a in atks {
            let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let hit = int(a.get("to_hit")).map(signed).unwrap_or_default();
            let dmg = match a.get("damage") {
                Some(Value::Sequence(ds)) => ds
                    .iter()
                    .map(|d| {
                        format!(
                            "{} {}",
                            d.get("dice").and_then(|v| v.as_str()).unwrap_or(""),
                            d.get("type").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" + "),
                _ => String::new(),
            };
            let range = match a.get("range") {
                Some(Value::Sequence(r)) if r.len() == 2 => format!(
                    " ({}/{})",
                    int(r.first()).unwrap_or(0),
                    int(r.get(1)).unwrap_or(0)
                ),
                _ => String::new(),
            };
            c.attacks.push(format!("{name}  {hit}  {dmg}{range}"));
        }
    }

    // Markdown: header bullets before the first `##`, then sections.
    let mut kind: Option<&str> = Some("header");
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.is_empty() || t.starts_with('>') {
            continue;
        }
        if let Some(h) = t.strip_prefix("## ") {
            kind = section_kind(h);
            continue;
        }
        if let Some(h) = t.strip_prefix("# ") {
            // `# 一二（Yier）——角色卡` -> `一二（Yier）`
            let n = h.split("——").next().unwrap_or(h).trim();
            if !n.is_empty() {
                c.name = plain(n);
            }
            continue;
        }
        let Some(k) = kind else { continue };
        let item = if let Some(h) = t.strip_prefix("### ") {
            format!("【{}】", plain(h))
        } else if t.starts_with('|') {
            if t.contains("---") {
                continue;
            }
            let cells: Vec<String> = t
                .trim_matches('|')
                .split('|')
                .map(plain)
                .filter(|c| !c.is_empty() && c != "-")
                .collect();
            if cells.iter().any(|c| c == "攻击" || c == "属性") {
                continue; // table header row
            }
            cells.join(" · ")
        } else {
            let s = t
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .to_string();
            if k == "header" {
                parse_header_bullet(&s, &mut c.header, &mut c.info);
                continue;
            }
            plain(&s)
        };
        let item = item.trim().to_string();
        if item.is_empty() || item == "无" {
            continue;
        }
        match k {
            "spells" => c.spells.push(item),
            "equipment" => c.equipment.push(item),
            "personality" => c.personality.push(item),
            "proficiencies" => c.proficiencies.push(item),
            "features" => c.features.push(item),
            _ => {}
        }
    }
    // Personality written in the header (`**性格**`).
    if let Some((_, v)) = c.header.iter().find(|(k, _)| k.contains("性格")) {
        if c.personality.is_empty() {
            c.personality.push(v.clone());
        }
    }
    // Spells from the combat block when the sheet has no spell list.
    if c.spells.is_empty() && c.casting.is_some() {
        if let Some(Value::Sequence(acts)) = b.get("actions") {
            let refs: Vec<String> = acts
                .iter()
                .filter_map(|a| a.get("ref").and_then(|v| v.as_str()).map(String::from))
                .collect();
            if !refs.is_empty() {
                c.spells.push(format!("（combat 引用）{}", refs.join("、")));
            }
        }
    }
    if c.hit_die.is_none() {
        let book = crate::classdata::Book::srd("2024");
        let class = crate::progress::class_of(sheet, &text, &book);
        c.hit_die = crate::progress::hit_die(sheet, class.as_deref(), &book).map(i64::from);
    }
    c.xp = progress.xp.get(&c.id).copied();
    if let Some(p) = progress.purses.get(&c.id) {
        c.coins = [0, p.cp / 100, 0, (p.cp % 100) / 10, p.cp % 10];
    } else {
        c.coins = coins_in(&c.equipment);
    }
    Ok(c)
}

/// Find "10 GP" / "5 sp" style amounts in the equipment lines that mention coins.
fn coins_in(lines: &[String]) -> [i64; 5] {
    let mut out = [0; 5];
    for l in lines {
        let ll = l.to_lowercase();
        if !(ll.contains("金钱") || ll.contains("coin") || ll.contains("钱币")) {
            continue;
        }
        let toks: Vec<&str> = ll
            .split(|ch: char| ch.is_whitespace() || "，,、；;：:（）()".contains(ch))
            .filter(|s| !s.is_empty())
            .collect();
        for w in toks.windows(2) {
            let (num, unit) = (w[0], w[1]);
            let (num, unit) = match num.parse::<i64>() {
                Ok(n) => (n, unit.to_string()),
                Err(_) => continue,
            };
            let idx = match unit.trim_end_matches('.') {
                "pp" => 0,
                "gp" | "金币" => 1,
                "ep" => 2,
                "sp" | "银币" => 3,
                "cp" | "铜币" => 4,
                _ => continue,
            };
            out[idx] += num;
        }
        for t in &toks {
            // "10gp"
            let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() || digits.len() == t.len() {
                continue;
            }
            let idx = match &t[digits.len()..] {
                "pp" => 0,
                "gp" => 1,
                "ep" => 2,
                "sp" => 3,
                "cp" => 4,
                _ => continue,
            };
            out[idx] += digits.parse::<i64>().unwrap_or(0);
        }
    }
    out
}

impl Card {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.header
            .iter()
            .find(|(k, _)| k.contains(key))
            .map(|(_, v)| v.as_str())
    }
    pub fn mod_of(&self, ab: &str) -> i64 {
        self.scores.get(ab).map(|s| modifier(*s)).unwrap_or(0)
    }
    pub fn save(&self, ab: &str) -> (i64, bool) {
        match self.saves.get(ab) {
            Some(v) => (*v, true),
            None => (self.mod_of(ab), false),
        }
    }
    pub fn skill(&self, id: &str, ab: &str) -> (i64, bool) {
        match self.skills.get(id) {
            Some(v) => (*v, true),
            None => (self.mod_of(ab), false),
        }
    }
    pub fn passive_perception(&self) -> i64 {
        10 + self.skill("perception", "wis").0
    }
    pub fn subtitle(&self) -> String {
        let mut parts = Vec::new();
        for (k, label) in [
            ("等级", "等级"),
            ("种族", "种族"),
            ("物种", "物种"),
            ("背景", "背景"),
            ("阵营", "阵营"),
            ("玩家", "玩家"),
            ("规则", "规则"),
        ] {
            if let Some(v) = self.get(k) {
                parts.push(format!("{label}：{v}"));
            }
        }
        if parts.is_empty() {
            parts.push(format!("等级 {}", self.level));
        }
        parts.join("　")
    }
    pub fn feature_lines(&self) -> Vec<String> {
        let mut v = self.info.clone();
        v.extend(self.features.iter().cloned());
        v
    }
    pub fn hit_die_label(&self) -> String {
        match self.hit_die {
            Some(d) => format!("{}d{d}", self.hit_dice),
            None => format!("{} 枚", self.hit_dice),
        }
    }
}

// ---------------------------------------------------------------- fonts

pub struct CjkFont {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    pub path: PathBuf,
}

const FONT_CANDIDATES: [&str; 16] = [
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
    "/usr/share/fonts/truetype/arphic/uming.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/Library/Fonts/Arial Unicode.ttf",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\simhei.ttf",
    "C:\\Windows\\Fonts\\simsun.ttc",
];

fn has_cjk(data: &[u8], index: u32) -> Option<bool> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    Some(face.glyph_index('角').is_some() && face.glyph_index('A').is_some())
}

/// Pick the face of a collection whose name says Simplified Chinese, else the
/// first face that has CJK glyphs.
fn pick_index(data: &[u8], want: Option<u32>) -> Option<u32> {
    if let Some(i) = want {
        return has_cjk(data, i).unwrap_or(false).then_some(i);
    }
    let n = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    let mut first = None;
    for i in 0..n {
        let Ok(face) = ttf_parser::Face::parse(data, i) else {
            continue;
        };
        if face.glyph_index('角').is_none() {
            continue;
        }
        first.get_or_insert(i);
        let sc = face.names().into_iter().any(|nm| {
            nm.to_string()
                .map(|s| s.contains(" SC") || s.contains("GB") || s.contains("YaHei"))
                .unwrap_or(false)
        });
        if sc {
            return Some(i);
        }
    }
    first
}

/// Font lookup order: `TALKDND_CJK_FONT=path[#index]`, known system paths,
/// then `fc-match` (Linux/macOS with fontconfig).
pub fn find_font() -> Result<CjkFont> {
    let mut tries: Vec<(PathBuf, Option<u32>)> = Vec::new();
    if let Ok(v) = std::env::var("TALKDND_CJK_FONT") {
        let (p, i) = match v.rsplit_once('#') {
            Some((p, i)) => (p.to_string(), i.parse().ok()),
            None => (v.clone(), None),
        };
        tries.push((PathBuf::from(p), i));
    }
    if let Ok(windir) = std::env::var("WINDIR") {
        for f in ["msyh.ttc", "simhei.ttf", "simsun.ttc", "Deng.ttf"] {
            tries.push((Path::new(&windir).join("Fonts").join(f), None));
        }
    }
    for c in FONT_CANDIDATES {
        tries.push((PathBuf::from(c), None));
    }
    if let Ok(out) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}#%{index}", "sans-serif:lang=zh-cn"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some((p, i)) = s.rsplit_once('#') {
            tries.push((PathBuf::from(p), i.parse().ok()));
        }
    }
    let explicit = std::env::var("TALKDND_CJK_FONT").is_ok();
    for (k, (p, want)) in tries.iter().enumerate() {
        let Ok(data) = fs::read(p) else {
            if k == 0 && explicit {
                bail!("TALKDND_CJK_FONT={} cannot be read", p.display());
            }
            continue;
        };
        if let Some(index) = pick_index(&data, *want) {
            return Ok(CjkFont {
                data: Arc::new(data),
                index,
                path: p.clone(),
            });
        }
    }
    bail!(
        "no Chinese font found. Install one (e.g. Noto Sans CJK SC: `apt install fonts-noto-cjk`) \
         or set TALKDND_CJK_FONT=/path/to/font.ttf[#index]; or use `--format html`"
    )
}

struct Metrics {
    data: Arc<Vec<u8>>,
    index: u32,
    cache: std::cell::RefCell<HashMap<char, f32>>,
}

impl Metrics {
    fn width(&self, s: &str, size: f32) -> f32 {
        let mut cache = self.cache.borrow_mut();
        let face = ttf_parser::Face::parse(&self.data, self.index).ok();
        let upem = face
            .as_ref()
            .map(|f| f.units_per_em() as f32)
            .unwrap_or(1000.0);
        let mut w = 0.0;
        for ch in s.chars() {
            let cw = *cache.entry(ch).or_insert_with(|| {
                face.as_ref()
                    .and_then(|f| f.glyph_index(ch).and_then(|g| f.glyph_hor_advance(g)))
                    .map(|a| a as f32 / upem)
                    .unwrap_or(if ch.is_ascii() { 0.55 } else { 1.0 })
            });
            w += cw;
        }
        w * size
    }

    /// Greedy wrap (per char for CJK, per word for Latin).
    fn wrap(&self, text: &str, size: f32, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut cur = String::new();
        let mut tokens: Vec<String> = Vec::new();
        let mut word = String::new();
        for ch in text.chars() {
            if ch.is_ascii_alphanumeric() || "+-'/.()".contains(ch) {
                word.push(ch);
            } else {
                if !word.is_empty() {
                    tokens.push(std::mem::take(&mut word));
                }
                tokens.push(ch.to_string());
            }
        }
        if !word.is_empty() {
            tokens.push(word);
        }
        for t in tokens {
            let cand = format!("{cur}{t}");
            if self.width(&cand, size) > width && !cur.is_empty() {
                lines.push(cur.trim_end().to_string());
                cur = t.trim_start().to_string();
                // a single overlong token: hard split
                while self.width(&cur, size) > width && cur.chars().count() > 1 {
                    let mut head = String::new();
                    let mut rest = String::new();
                    for ch in cur.chars() {
                        if rest.is_empty() && self.width(&format!("{head}{ch}"), size) <= width {
                            head.push(ch);
                        } else {
                            rest.push(ch);
                        }
                    }
                    if head.is_empty() {
                        break;
                    }
                    lines.push(head);
                    cur = rest;
                }
            } else {
                cur = cand;
            }
        }
        if !cur.trim().is_empty() {
            lines.push(cur.trim_end().to_string());
        }
        lines
    }
}

// ---------------------------------------------------------------- layout

pub const PAGE_W: f32 = 595.0;
pub const PAGE_H: f32 = 842.0;

#[derive(Debug, Clone)]
pub enum FieldKind {
    Text { value: String, multiline: bool },
    Check,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    /// top-left origin, points
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub kind: FieldKind,
}

/// Drawing primitives, top-left origin.
#[derive(Debug, Clone)]
pub enum Op {
    Text {
        x: f32,
        y: f32,
        size: f32,
        text: String,
        gray: f32,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke: f32,
        fill: Option<f32>,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
}

struct Lay<'a> {
    m: &'a Metrics,
    ops: Vec<Op>,
    fields: Vec<Field>,
}

#[allow(clippy::too_many_arguments)]
impl Lay<'_> {
    fn text(&mut self, x: f32, y: f32, size: f32, t: &str) {
        self.ops.push(Op::Text {
            x,
            y,
            size,
            text: t.to_string(),
            gray: 0.0,
        });
    }
    fn label(&mut self, x: f32, y: f32, t: &str) {
        self.ops.push(Op::Text {
            x,
            y,
            size: 6.5,
            text: t.to_string(),
            gray: 0.35,
        });
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.ops.push(Op::Rect {
            x,
            y,
            w,
            h,
            stroke: 0.6,
            fill: None,
        });
    }
    fn title(&mut self, x: f32, y: f32, w: f32, t: &str) {
        self.ops.push(Op::Rect {
            x,
            y,
            w,
            h: 11.0,
            stroke: 0.0,
            fill: Some(0.88),
        });
        self.text(x + 3.0, y + 8.5, 7.5, t);
    }
    fn centered(&mut self, x: f32, w: f32, y: f32, size: f32, t: &str) {
        let tw = self.m.width(t, size);
        self.text(x + (w - tw) / 2.0, y, size, t);
    }
    /// Labelled stat box with a printed value.
    fn stat(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, value: &str) {
        self.rect(x, y, w, h);
        self.centered(x, w, y + 8.0, 6.5, label);
        let size = if self.m.width(value, 13.0) > w - 4.0 {
            8.0
        } else {
            13.0
        };
        self.centered(x, w, y + h - 6.0, size, value);
    }
    fn field(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, value: &str, multi: bool) {
        self.rect(x, y, w, h);
        self.fields.push(Field {
            name: name.to_string(),
            x,
            y,
            w,
            h,
            kind: FieldKind::Text {
                value: value.to_string(),
                multiline: multi,
            },
        });
    }
    fn check(&mut self, name: &str, x: f32, y: f32, s: f32) {
        self.rect(x, y, s, s);
        self.fields.push(Field {
            name: name.to_string(),
            x,
            y,
            w: s,
            h: s,
            kind: FieldKind::Check,
        });
    }
    /// Row of tick boxes; wraps after `per_row`. Returns the height used.
    fn ticks(&mut self, name: &str, x: f32, y: f32, n: i64, per_row: i64) -> f32 {
        let s = 8.0;
        let gap = 2.5;
        for i in 0..n {
            let (r, c) = (i / per_row, i % per_row);
            self.check(
                &format!("{name}_{}", i + 1),
                x + c as f32 * (s + gap),
                y + r as f32 * (s + gap),
                s,
            );
        }
        let rows = ((n.max(1) - 1) / per_row + 1) as f32;
        rows * (s + gap)
    }
    /// Printed list in a box, or a writable field when there is nothing to print.
    fn list(&mut self, name: &str, x: f32, y: f32, w: f32, h: f32, items: &[String], size: f32) {
        if items.is_empty() {
            self.field(name, x, y, w, h, "", true);
        } else {
            self.rect(x, y, w, h);
            self.block(x, y, w, h, items, size);
        }
    }
    /// Wrapped text inside a box; shrinks the font to fit, truncates last resort.
    fn block(&mut self, x: f32, y: f32, w: f32, h: f32, items: &[String], start: f32) {
        let mut size = start;
        let lines = loop {
            let mut lines = Vec::new();
            for it in items {
                let wrapped = self.m.wrap(&format!("• {it}"), size, w - 6.0);
                lines.extend(wrapped);
            }
            let need = lines.len() as f32 * size * 1.3;
            if need <= h - 4.0 || size <= 5.0 {
                break lines;
            }
            size -= 0.25;
        };
        let lh = size * 1.3;
        let max = ((h - 4.0) / lh).floor() as usize;
        for (i, l) in lines.iter().enumerate().take(max) {
            let t = if i + 1 == max && lines.len() > max {
                format!("{l} …")
            } else {
                l.clone()
            };
            self.text(
                x + 3.0,
                y + 2.0 + lh * (i as f32 + 1.0) - size * 0.25,
                size,
                &t,
            );
        }
    }
}

pub struct Page {
    pub ops: Vec<Op>,
    pub fields: Vec<Field>,
}

fn layout(c: &Card, m: &Metrics) -> Page {
    let mut l = Lay {
        m,
        ops: Vec::new(),
        fields: Vec::new(),
    };
    let (ax, aw) = (22.0, 150.0);
    let (bx, bw) = (180.0, 200.0);
    let (cx, cw) = (388.0, 185.0);

    // Header
    l.text(22.0, 42.0, 18.0, &c.name);
    let sub = c.subtitle();
    let mut size = 8.0;
    while m.width(&sub, size) > 360.0 && size > 5.5 {
        size -= 0.25;
    }
    l.text(22.0, 58.0, size, &sub);
    l.label(cx, 30.0, "经验值 XP");
    l.field(
        "xp",
        cx,
        33.0,
        120.0,
        18.0,
        &c.xp.map(|x| x.to_string()).unwrap_or_default(),
        false,
    );
    l.label(cx + 128.0, 30.0, "激励 Inspiration");
    l.check("inspiration", cx + 140.0, 35.0, 13.0);
    l.ops.push(Op::Line {
        x1: 22.0,
        y1: 68.0,
        x2: 573.0,
        y2: 68.0,
    });

    // Column A: abilities, saves, skills, proficiencies
    let mut y = 76.0;
    for (ab, cn) in ABILITIES {
        let score = c.scores.get(ab).copied().unwrap_or(10);
        l.rect(ax, y, aw, 30.0);
        l.text(ax + 5.0, y + 12.0, 8.0, cn);
        l.label(ax + 5.0, y + 23.0, &ab.to_uppercase());
        l.centered(ax + 60.0, 45.0, y + 21.0, 15.0, &signed(modifier(score)));
        l.rect(ax + 112.0, y + 6.0, 32.0, 18.0);
        l.centered(ax + 112.0, 32.0, y + 19.0, 10.0, &score.to_string());
        y += 33.0;
    }
    y += 2.0;
    l.title(ax, y, aw, "豁免 Saving Throws（● 熟练）");
    y += 13.0;
    for (i, (ab, cn)) in ABILITIES.iter().enumerate() {
        let (v, p) = c.save(ab);
        let xx = ax + 3.0 + (i % 2) as f32 * 75.0;
        let yy = y + 9.0 + (i / 2) as f32 * 11.0;
        l.text(
            xx,
            yy,
            7.5,
            &format!("{} {} {}", if p { "●" } else { "○" }, cn, signed(v)),
        );
    }
    y += 38.0;
    l.title(ax, y, aw, "技能 Skills（● 熟练/专精）");
    y += 13.0;
    for (i, (id, cn, ab)) in SKILLS.iter().enumerate() {
        let (v, p) = c.skill(id, ab);
        let xx = ax + 3.0 + (i % 2) as f32 * 75.0;
        let yy = y + 9.0 + (i / 2) as f32 * 11.0;
        l.text(
            xx,
            yy,
            7.5,
            &format!("{} {} {}", if p { "●" } else { "○" }, cn, signed(v)),
        );
    }
    y += 104.0;
    l.title(ax, y, aw, "熟练与语言 Proficiencies");
    y += 12.0;
    l.list("proficiencies", ax, y, aw, 820.0 - y, &c.proficiencies, 7.0);

    // Column B: combat, HP, attacks, resources, spells
    let mut y = 76.0;
    let w4 = (bw - 3.0 * 4.0) / 4.0;
    for (i, (lab, val)) in [
        ("护甲 AC", c.ac.to_string()),
        ("先攻 Init", signed(c.init)),
        ("速度 Speed", c.speed.clone()),
        ("熟练 Prof", signed(c.prof)),
    ]
    .iter()
    .enumerate()
    {
        l.stat(bx + i as f32 * (w4 + 4.0), y, w4, 34.0, lab, val);
    }
    y += 38.0;
    for (i, (lab, val)) in [
        ("最大生命 Max HP", c.hp.to_string()),
        ("被动察觉 Passive", c.passive_perception().to_string()),
        ("生命骰 Hit Dice", c.hit_die_label()),
    ]
    .iter()
    .enumerate()
    {
        let w3 = (bw - 8.0) / 3.0;
        l.stat(bx + i as f32 * (w3 + 4.0), y, w3, 34.0, lab, val);
    }
    y += 42.0;
    l.label(bx, y, "当前生命 Current HP");
    l.label(bx + 128.0, y, "临时生命 Temp HP");
    l.field("hp_current", bx, y + 3.0, 122.0, 34.0, "", false);
    l.field("hp_temp", bx + 128.0, y + 3.0, bw - 128.0, 34.0, "", false);
    y += 44.0;
    l.label(bx, y + 7.0, "已用生命骰");
    let used = l.ticks("hit_dice_used", bx + 42.0, y, c.hit_dice.clamp(1, 20), 10);
    y += used.max(11.0) + 2.0;
    l.label(bx, y + 7.0, "死亡豁免 成功");
    l.ticks("death_success", bx + 50.0, y, 3, 3);
    l.label(bx + 95.0, y + 7.0, "失败");
    l.ticks("death_fail", bx + 115.0, y, 3, 3);
    y += 14.0;
    l.label(bx, y + 6.0, "状态 / 力竭 Conditions / Exhaustion");
    l.field("conditions", bx, y + 9.0, bw, 26.0, "", true);
    y += 40.0;
    l.title(bx, y, bw, "攻击 Attacks（命中 · 伤害）");
    y += 12.0;
    let ah = 14.0 + 10.0 * c.attacks.len().clamp(2, 7) as f32;
    l.rect(bx, y, bw, ah);
    l.block(bx, y, bw, ah, &c.attacks, 7.5);
    y += ah + 5.0;
    l.title(bx, y, bw, "资源 Resources（用掉打勾）");
    y += 14.0;
    for (i, (name, max, rech)) in c.resources.iter().enumerate().take(6) {
        let lab = format!(
            "{name}{}",
            match rech.as_str() {
                "short" => "（短休）",
                "long" => "（长休）",
                "" => "",
                _ => "",
            }
        );
        l.text(bx + 2.0, y + 7.0, 7.0, &lab);
        let h = l.ticks(
            &format!("res{}", i + 1),
            bx + 110.0,
            y,
            (*max).clamp(1, 16),
            8,
        );
        y += h.max(11.0);
    }
    for i in c.resources.len()..2 {
        l.field(&format!("res_extra{}", i + 1), bx, y, bw, 11.0, "", false);
        y += 13.0;
    }
    y += 4.0;
    let head = match &c.casting {
        Some((ab, atk, dc)) => format!(
            "法术 Spells　{} · DC {dc} · 攻击 {}",
            ab.to_uppercase(),
            signed(*atk)
        ),
        None => "法术 Spells".to_string(),
    };
    l.title(bx, y, bw, &head);
    y += 14.0;
    for (lvl, max) in &c.slots {
        l.text(bx + 2.0, y + 7.0, 7.0, &format!("{lvl} 环法术位"));
        let h = l.ticks(&format!("slot{lvl}"), bx + 50.0, y, (*max).clamp(1, 9), 9);
        y += h.max(11.0);
    }
    let sh = 820.0 - y;
    l.list("spells", bx, y, bw, sh, &c.spells, 7.0);

    // Column C: features, equipment, coins, personality, notes
    let mut y = 76.0;
    l.title(cx, y, cw, "特性 Features & Traits");
    y += 12.0;
    l.list("features", cx, y, cw, 300.0, &c.feature_lines(), 7.0);
    y += 305.0;
    l.title(cx, y, cw, "装备 Equipment");
    y += 12.0;
    l.list("equipment", cx, y, cw, 170.0, &c.equipment, 6.8);
    y += 175.0;
    let wc = (cw - 4.0 * 3.0) / 5.0;
    for (i, lab) in ["PP 铂", "GP 金", "EP 银金", "SP 银", "CP 铜"]
        .iter()
        .enumerate()
    {
        let xx = cx + i as f32 * (wc + 3.0);
        l.label(xx, y + 6.0, lab);
        let v = if c.coins[i] == 0 {
            String::new()
        } else {
            c.coins[i].to_string()
        };
        l.field(
            &format!("coin_{}", ["pp", "gp", "ep", "sp", "cp"][i]),
            xx,
            y + 9.0,
            wc,
            18.0,
            &v,
            false,
        );
    }
    y += 32.0;
    l.title(cx, y, cw, "个性 Personality");
    y += 12.0;
    l.list("personality", cx, y, cw, 62.0, &c.personality, 6.8);
    y += 66.0;
    l.title(cx, y, cw, "笔记 Notes");
    y += 12.0;
    l.field("notes", cx, y, cw, 820.0 - y, "", true);

    l.label(
        22.0,
        832.0,
        &format!(
            "TalkDND · {} · 由 talkdnd sheet export 生成；以角色卡 Markdown 为准",
            c.file
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        ),
    );
    Page {
        ops: l.ops,
        fields: l.fields,
    }
}

// ---------------------------------------------------------------- PDF

pub fn render_pdf(c: &Card, font: &CjkFont) -> Result<Vec<u8>> {
    use krilla::color::luma;
    use krilla::geom::{PathBuilder, Point, Rect};
    use krilla::num::NormalizedF32;
    use krilla::page::PageSettings;
    use krilla::paint::{Fill, Stroke};
    use krilla::text::{Font, TextDirection};

    let m = Metrics {
        data: font.data.clone(),
        index: font.index,
        cache: Default::default(),
    };
    let page = layout(c, &m);
    let kfont = Font::new(font.data.clone().into(), font.index)
        .ok_or_else(|| anyhow!("unusable font {}", font.path.display()))?;
    let gray = |g: f32| luma::Color::new((g * 255.0) as u8);
    let mut doc = krilla::Document::new();
    let mut p = doc.start_page_with(
        PageSettings::from_wh(PAGE_W, PAGE_H).ok_or_else(|| anyhow!("page size"))?,
    );
    let mut s = p.surface();
    for op in &page.ops {
        match op {
            Op::Text {
                x,
                y,
                size,
                text,
                gray: g,
            } => {
                s.set_stroke(None);
                s.set_fill(Some(Fill {
                    paint: gray(*g).into(),
                    opacity: NormalizedF32::ONE,
                    rule: Default::default(),
                }));
                s.draw_text(
                    Point::from_xy(*x, *y),
                    kfont.clone(),
                    *size,
                    text,
                    false,
                    TextDirection::Auto,
                );
            }
            Op::Rect {
                x,
                y,
                w,
                h,
                stroke,
                fill,
            } => {
                let mut pb = PathBuilder::new();
                if let Some(r) = Rect::from_xywh(*x, *y, *w, *h) {
                    pb.push_rect(r);
                }
                let Some(path) = pb.finish() else { continue };
                s.set_fill(fill.map(|g| Fill {
                    paint: gray(g).into(),
                    opacity: NormalizedF32::ONE,
                    rule: Default::default(),
                }));
                s.set_stroke((*stroke > 0.0).then(|| Stroke {
                    paint: gray(0.25).into(),
                    width: *stroke,
                    ..Default::default()
                }));
                s.draw_path(&path);
            }
            Op::Line { x1, y1, x2, y2 } => {
                let mut pb = PathBuilder::new();
                pb.move_to(*x1, *y1);
                pb.line_to(*x2, *y2);
                let Some(path) = pb.finish() else { continue };
                s.set_fill(None);
                s.set_stroke(Some(Stroke {
                    paint: gray(0.2).into(),
                    width: 1.0,
                    ..Default::default()
                }));
                s.draw_path(&path);
            }
        }
    }
    s.finish();
    p.finish();
    let bytes = doc
        .finish()
        .map_err(|e| anyhow!("PDF generation failed: {e:?}"))?;
    add_form(&bytes, &page.fields)
}

/// Add AcroForm widgets (text fields, check boxes) to page 1.
fn add_form(pdf: &[u8], fields: &[Field]) -> Result<Vec<u8>> {
    use lopdf::{dictionary, Object, Stream};
    let mut doc = lopdf::Document::load_mem(pdf).context("reload PDF for form fields")?;
    let page_id = *doc
        .get_pages()
        .values()
        .next()
        .ok_or_else(|| anyhow!("no page"))?;
    let helv = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let zadb = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "ZapfDingbats",
    });
    let mut annots = Vec::new();
    for f in fields {
        let rect = vec![
            f.x.into(),
            (PAGE_H - f.y - f.h).into(),
            (f.x + f.w).into(),
            (PAGE_H - f.y).into(),
        ];
        let id = match &f.kind {
            FieldKind::Text { value, multiline } => {
                let mut d = dictionary! {
                    "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
                    "T" => Object::string_literal(f.name.as_str()),
                    "Rect" => rect, "F" => 4, "P" => page_id,
                    "DA" => Object::string_literal(if *multiline { "/Helv 8 Tf 0 g" } else { "/Helv 10 Tf 0 g" }),
                    "MK" => dictionary! {},
                };
                if *multiline {
                    d.set("Ff", 4096);
                }
                if !value.is_empty() {
                    d.set("V", Object::string_literal(value.as_str()));
                }
                doc.add_object(d)
            }
            FieldKind::Check => {
                let (w, h) = (f.w, f.h);
                let size = h * 0.8;
                let on = format!(
                    "q BT 0 g /ZaDb {size:.2} Tf {:.2} {:.2} Td (4) Tj ET Q",
                    w * 0.15,
                    h * 0.2
                );
                let res = dictionary! { "Font" => dictionary! { "ZaDb" => zadb } };
                let yes = doc.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject", "Subtype" => "Form",
                        "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
                        "Resources" => res.clone(),
                    },
                    on.into_bytes(),
                ));
                let off = doc.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject", "Subtype" => "Form",
                        "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
                    },
                    Vec::new(),
                ));
                doc.add_object(dictionary! {
                    "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn",
                    "T" => Object::string_literal(f.name.as_str()),
                    "Rect" => rect, "F" => 4, "P" => page_id,
                    "V" => "Off", "AS" => "Off",
                    "DA" => Object::string_literal("/ZaDb 0 Tf 0 g"),
                    "MK" => dictionary! { "CA" => Object::string_literal("4") },
                    "AP" => dictionary! { "N" => dictionary! { "Yes" => yes, "Off" => off } },
                })
            }
        };
        annots.push(id);
    }
    let refs: Vec<Object> = annots.iter().map(|i| Object::Reference(*i)).collect();
    doc.get_dictionary_mut(page_id)?.set("Annots", refs.clone());
    let form = dictionary! {
        "Fields" => refs,
        "NeedAppearances" => true,
        "DA" => Object::string_literal("/Helv 0 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv, "ZaDb" => zadb } },
    };
    let form_id = doc.add_object(form);
    doc.catalog_mut()?.set("AcroForm", form_id);
    let mut out = Vec::new();
    doc.save_to(&mut out)?;
    Ok(out)
}

// ---------------------------------------------------------------- HTML

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn ul(items: &[String]) -> String {
    if items.is_empty() {
        return "<p class=muted>—</p>".into();
    }
    let mut s = String::from("<ul>");
    for i in items {
        s += &format!("<li>{}</li>", esc(i));
    }
    s + "</ul>"
}

fn boxes(name: &str, n: i64) -> String {
    (1..=n.clamp(1, 20))
        .map(|i| format!("<input type=checkbox name=\"{name}_{i}\">"))
        .collect::<Vec<_>>()
        .join("")
}

pub fn render_html(c: &Card) -> String {
    let mut abil = String::new();
    for (ab, cn) in ABILITIES {
        let sc = c.scores.get(ab).copied().unwrap_or(10);
        abil += &format!(
            "<div class=ab><b>{cn}</b> <small>{}</small><span class=mod>{}</span><span class=sc>{sc}</span></div>",
            ab.to_uppercase(),
            signed(modifier(sc))
        );
    }
    let mut saves = String::new();
    for (ab, cn) in ABILITIES {
        let (v, p) = c.save(ab);
        saves += &format!("<li>{} {cn} {}</li>", if p { "●" } else { "○" }, signed(v));
    }
    let mut skills = String::new();
    for (id, cn, ab) in SKILLS {
        let (v, p) = c.skill(id, ab);
        skills += &format!("<li>{} {cn} {}</li>", if p { "●" } else { "○" }, signed(v));
    }
    let mut res = String::new();
    for (i, (n, max, r)) in c.resources.iter().enumerate() {
        res += &format!(
            "<div>{}{} {}</div>",
            esc(n),
            match r.as_str() {
                "short" => "（短休）",
                "long" => "（长休）",
                _ => "",
            },
            boxes(&format!("res{}", i + 1), *max)
        );
    }
    let mut slots = String::new();
    for (l, max) in &c.slots {
        slots += &format!(
            "<div>{l} 环法术位 {}</div>",
            boxes(&format!("slot{l}"), *max)
        );
    }
    let cast = c
        .casting
        .as_ref()
        .map(|(a, atk, dc)| format!(" {} · DC {dc} · 攻击 {}", a.to_uppercase(), signed(*atk)))
        .unwrap_or_default();
    let coin = |i: usize, lab: &str, k: &str| {
        let v = if c.coins[i] == 0 {
            String::new()
        } else {
            c.coins[i].to_string()
        };
        format!("<label>{lab}<input name=coin_{k} value=\"{v}\"></label>")
    };
    format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>{name}</title>
<style>
@page {{ size: A4; margin: 8mm; }}
* {{ box-sizing: border-box; }}
body {{ font-family: "Noto Sans CJK SC","Noto Sans SC","Source Han Sans SC","PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif; font-size: 8.5pt; margin: 0; }}
.page {{ width: 194mm; height: 279mm; margin: auto; display: grid; grid-template-columns: 27% 37% 36%; grid-template-rows: auto 1fr; gap: 3mm; overflow: hidden; }}
header {{ grid-column: 1 / 4; display: flex; justify-content: space-between; border-bottom: 1.5px solid #333; padding-bottom: 2mm; }}
h1 {{ font-size: 17pt; margin: 0; }} h2 {{ font-size: 8pt; background: #e2e2e2; margin: 2mm 0 1mm; padding: 1px 3px; }}
.ab {{ border: 1px solid #555; padding: 2px 4px; margin-bottom: 2px; display: flex; gap: 4px; align-items: center; }}
.mod {{ font-size: 14pt; margin-left: auto; }} .sc {{ border: 1px solid #777; padding: 0 5px; }}
ul {{ margin: 0; padding-left: 12px; }} li {{ margin: 0; }}
.cols li {{ list-style: none; display: inline-block; width: 48%; }} .cols ul {{ padding: 0; }}
.stats {{ display: grid; grid-template-columns: repeat(4,1fr); gap: 3px; }} .stats3 {{ grid-template-columns: repeat(3,1fr); margin-top: 3px; }}
.stat {{ border: 1px solid #555; text-align: center; padding: 2px; }} .stat b {{ display: block; font-size: 12pt; }}
input:not([type]) , textarea {{ width: 100%; border: 1px solid #555; font: inherit; }}
input[type=checkbox] {{ width: 10px; height: 10px; margin: 1px; }}
.big {{ height: 11mm; font-size: 14pt; }} .box {{ border: 1px solid #555; padding: 2px; }}
.muted {{ color: #777; }} label {{ font-size: 6.5pt; color: #555; }}
.coins {{ display: grid; grid-template-columns: repeat(5,1fr); gap: 2px; }}
@media print {{ input, textarea {{ border: 1px solid #555; }} }}
</style></head><body><div class=page>
<header><div><h1>{name}</h1><div>{sub}</div></div>
<div><label>经验值 XP<input name=xp value="{xp}"></label> <label>激励 <input type=checkbox name=inspiration></label></div></header>
<section>{abil}<h2>豁免（● 熟练）</h2><div class=cols><ul>{saves}</ul></div><h2>技能（● 熟练/专精）</h2><div class=cols><ul>{skills}</ul></div><h2>熟练与语言</h2>{profs}</section>
<section><div class=stats><div class=stat>护甲 AC<b>{ac}</b></div><div class=stat>先攻<b>{init}</b></div><div class=stat>速度<b>{speed}</b></div><div class=stat>熟练<b>{prof}</b></div></div>
<div class="stats stats3"><div class=stat>最大生命<b>{hp}</b></div><div class=stat>被动察觉<b>{pp}</b></div><div class=stat>生命骰<b>{hd}</b></div></div>
<div class=stats style="grid-template-columns:2fr 1fr"><label>当前生命<input class=big name=hp_current></label><label>临时生命<input class=big name=hp_temp></label></div>
<div>已用生命骰 {hdu}</div><div>死亡豁免 成功 {ds} 失败 {df}</div>
<label>状态 / 力竭<textarea name=conditions rows=2></textarea></label>
<h2>攻击（命中 · 伤害）</h2>{atk}<h2>资源（用掉打勾）</h2>{res}<input name=res_extra1>
<h2>法术{cast}</h2>{slots}{spells}</section>
<section><h2>特性</h2><div class=box>{feat}</div><h2>装备</h2><div class=box>{equip}</div>
<div class=coins>{c0}{c1}{c2}{c3}{c4}</div><h2>个性</h2>{pers}<h2>笔记</h2><textarea name=notes rows=6></textarea></section>
</div></body></html>
"#,
        name = esc(&c.name),
        sub = esc(&c.subtitle()),
        xp = c.xp.map(|x| x.to_string()).unwrap_or_default(),
        profs = ul(&c.proficiencies),
        ac = c.ac,
        init = signed(c.init),
        speed = esc(&c.speed),
        prof = signed(c.prof),
        hp = c.hp,
        pp = c.passive_perception(),
        hd = esc(&c.hit_die_label()),
        hdu = boxes("hit_dice_used", c.hit_dice),
        ds = boxes("death_success", 3),
        df = boxes("death_fail", 3),
        atk = ul(&c.attacks),
        spells = ul(&c.spells),
        feat = ul(&c.feature_lines()),
        equip = ul(&c.equipment),
        pers = ul(&c.personality),
        c0 = coin(0, "PP 铂", "pp"),
        c1 = coin(1, "GP 金", "gp"),
        c2 = coin(2, "EP 银金", "ep"),
        c3 = coin(3, "SP 银", "sp"),
        c4 = coin(4, "CP 铜", "cp"),
    )
}

// ---------------------------------------------------------------- command

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Html,
}

/// Export `who` (`all`, an id or a name). Writes next to each sheet:
/// `Characters/<name>.pdf` (or `.html`). Returns the written paths.
pub fn export(save_dir: &Path, who: &str, format: Format) -> Result<Vec<PathBuf>> {
    let sheets = crate::check::sheets(save_dir);
    if sheets.is_empty() {
        bail!(
            "no character sheets with a combat block in {}",
            save_dir.display()
        );
    }
    let chosen: Vec<&Sheet> = if who == "all" {
        sheets.iter().collect()
    } else {
        who.split(',')
            .map(|w| crate::check::find(&sheets, w.trim()))
            .collect::<Result<_>>()?
    };
    let progress = Progress::load(save_dir)?;
    let font = match format {
        Format::Pdf => Some(find_font()?),
        Format::Html => None,
    };
    let mut out = Vec::new();
    for s in chosen {
        let c = card(s, &progress)?;
        let ext = match format {
            Format::Pdf => "pdf",
            Format::Html => "html",
        };
        let path = s.file.with_extension(ext);
        match &font {
            Some(f) => fs::write(&path, render_pdf(&c, f)?)?,
            None => fs::write(&path, render_html(&c))?,
        }
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_save(tag: &str) -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LetsPlay/example/save");
        let dir = std::env::temp_dir().join(format!("talkdnd-sheet-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for sub in ["Characters", "NPCs"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
            for e in fs::read_dir(src.join(sub)).unwrap() {
                let p = e.unwrap().path();
                fs::copy(&p, dir.join(sub).join(p.file_name().unwrap())).unwrap();
            }
        }
        dir
    }

    #[test]
    fn card_from_example_sheet() {
        let dir = example_save("card");
        let sheets = crate::check::sheets(&dir);
        let s = crate::check::find(&sheets, "example-fighter").unwrap();
        let c = card(s, &Progress::default()).unwrap();
        assert_eq!(c.name, "Example Fighter");
        assert_eq!((c.level, c.ac, c.hp, c.prof, c.init), (1, 18, 12, 2, 2));
        assert_eq!(c.hit_die_label(), "1d10");
        assert_eq!(c.save("str"), (5, true));
        assert_eq!(c.save("dex"), (2, false));
        assert_eq!(c.skill("perception", "wis"), (3, true));
        assert_eq!(c.passive_perception(), 13);
        assert_eq!(c.attacks.len(), 3);
        assert!(c.attacks[0].starts_with("Longsword  +5  1d8+3 slashing"));
        assert!(c
            .resources
            .iter()
            .any(|(n, m, r)| n == "second-wind" && *m == 1 && r == "short"));
        assert!(c.features.iter().any(|f| f.contains("Second Wind")));
        assert!(c.equipment.iter().any(|f| f.contains("explorer's pack")));
        assert!(c.spells.is_empty(), "`无` is not a spell");
        assert!(c.subtitle().contains("fighter 1") && c.subtitle().contains("soldier"));
        assert_eq!(
            c.personality,
            vec!["Synthetic example character.".to_string()]
        );
        assert!(!c.features.iter().any(|f| f.contains("combat yaml")));
    }

    #[test]
    fn coins_from_progress_or_sheet_text() {
        assert_eq!(
            coins_in(&["金钱：10 GP（起点）、5 sp".into()]),
            [0, 10, 0, 5, 0]
        );
        assert_eq!(coins_in(&["Coins: 3gp 2cp".into()]), [0, 3, 0, 0, 2]);
        assert_eq!(coins_in(&["10 GP 的剑".into()]), [0; 5], "only coin lines");
        let dir = example_save("coins");
        let sheets = crate::check::sheets(&dir);
        let mut p = Progress::default();
        p.xp.insert("example-fighter".into(), 450);
        p.purses.insert(
            "example-fighter".into(),
            crate::progress::Purse {
                cp: 1234,
                ..Default::default()
            },
        );
        let c = card(&sheets[0], &p).unwrap();
        assert_eq!(c.xp, Some(450));
        assert_eq!(c.coins, [0, 12, 0, 3, 4]);
    }

    #[test]
    fn html_has_play_fields() {
        let dir = example_save("html");
        let out = export(&dir, "all", Format::Html).unwrap();
        assert!(out[0].ends_with("Characters/Example Fighter.html"));
        let h = fs::read_to_string(&out[0]).unwrap();
        for f in [
            "name=hp_current",
            "name=hp_temp",
            "name=\"death_fail_3\"",
            "name=\"hit_dice_used_1\"",
            "name=\"res1_1\"",
            "name=notes",
            "name=xp",
            "name=coin_gp",
            "@page { size: A4",
        ] {
            assert!(h.contains(f), "missing {f}");
        }
    }

    #[test]
    fn pdf_is_one_a4_page_with_form_fields() {
        let font = match find_font() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let dir = example_save("pdf");
        let sheets = crate::check::sheets(&dir);
        let mut c = card(&sheets[0], &Progress::default()).unwrap();
        c.features.push("中文特性：黑暗视觉 60 尺".into());
        let bytes = render_pdf(&c, &font).unwrap();
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.get_pages().len(), 1);
        let form = doc.catalog().unwrap().get(b"AcroForm").unwrap();
        let form = doc.dereference(form).unwrap().1.as_dict().unwrap();
        let fields = form.get(b"Fields").unwrap().as_array().unwrap();
        let names: Vec<String> = fields
            .iter()
            .map(|r| {
                let d = doc.get_dictionary(r.as_reference().unwrap()).unwrap();
                String::from_utf8_lossy(d.get(b"T").unwrap().as_str().unwrap()).to_string()
            })
            .collect();
        for n in [
            "hp_current",
            "hp_temp",
            "xp",
            "inspiration",
            "death_success_3",
            "death_fail_1",
            "hit_dice_used_1",
            "res1_1",
            "coin_gp",
            "conditions",
            "notes",
            "spells",
        ] {
            assert!(names.iter().any(|x| x == n), "missing field {n}");
        }
        let page = doc
            .get_dictionary(*doc.get_pages().get(&1).unwrap())
            .unwrap();
        let mb = page.get(b"MediaBox").unwrap().as_array().unwrap();
        assert_eq!(mb[2].as_float().unwrap().round(), PAGE_W);
        assert_eq!(mb[3].as_float().unwrap().round(), PAGE_H);
        let out = export(&dir, "Example Fighter", Format::Pdf).unwrap();
        assert!(out[0].ends_with("Characters/Example Fighter.pdf"));
    }
}
