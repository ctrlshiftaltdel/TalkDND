//! Character-sheet option data: fighting styles, backgrounds (SRD plus local
//! book imports), class starting equipment. Item lists and names only.

use anyhow::{bail, Result};
use combat_sim::dice::DiceRng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const YAML: &str = include_str!("../data/sheet-options.yaml");

#[derive(Debug, Clone, Deserialize)]
pub struct Style {
    pub name: String,
    pub zh: String,
    pub effect: String,
    pub rulesets: Vec<String>,
    #[serde(default)]
    pub srd: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StyleClass {
    pub level: i32,
    #[serde(default)]
    pub allowed: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Pack {
    #[serde(default)]
    pub items: Vec<String>,
    /// Gold pieces.
    #[serde(default)]
    pub gold: i64,
}

/// A background: SRD (embedded) or imported from the player's own book.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BackgroundDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 2024 origin feat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feat: Option<String>,
    /// 2014 background feature name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// Number of languages of the player's choice.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub languages: i32,
    /// 2014 fixed equipment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipment: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero64")]
    pub gold: i64,
    /// 2024 equipment options A/B.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, Pack>,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}
fn is_zero64(v: &i64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClassEquip {
    #[serde(default)]
    pub groups: Vec<Vec<String>>,
    #[serde(default)]
    pub fixed: Vec<String>,
    #[serde(default)]
    pub gold_dice: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, Pack>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Data {
    pub fighting_styles: BTreeMap<String, Style>,
    pub fighting_style_classes: BTreeMap<String, BTreeMap<String, StyleClass>>,
    pub backgrounds: BTreeMap<String, BTreeMap<String, BackgroundDef>>,
    pub class_equipment: BTreeMap<String, BTreeMap<String, ClassEquip>>,
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| serde_norway::from_str(YAML).expect("embedded sheet-options.yaml"))
}

pub fn style(id: &str) -> Option<&'static Style> {
    data().fighting_styles.get(id)
}

/// (level, allowed ids) when the class gains a Fighting Style.
pub fn style_for_class(rs: &str, class: &str) -> Option<(i32, Vec<String>)> {
    let sc = data().fighting_style_classes.get(rs)?.get(class)?;
    let allowed = if sc.allowed.is_empty() {
        data()
            .fighting_styles
            .iter()
            .filter(|(id, s)| {
                s.rulesets.iter().any(|r| r == rs)
                    && match id.as_str() {
                        "blessed-warrior" => class == "paladin",
                        "druidic-warrior" => class == "ranger",
                        _ => true,
                    }
            })
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        sc.allowed.clone()
    };
    Some((sc.level, allowed))
}

pub fn srd_background(rs: &str, id: &str) -> Option<&'static BackgroundDef> {
    data().backgrounds.get(rs)?.get(id)
}

/// Split option text like "light crossbow, 20 bolts" into items.
fn split_items(v: &[String]) -> Vec<String> {
    v.iter()
        .flat_map(|s| s.split(", ").map(|x| x.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Resolved starting equipment: items, gold in gp, and any roll text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub items: Vec<String>,
    pub gold: i64,
    pub roll: Option<String>,
}

/// Class starting equipment. 2014: `a,b,a` (one letter per choice group) or
/// `gold` (rolls the class's starting-gold dice); 2024: `A`, `B` or `C`.
pub fn class_equipment(
    rs: &str,
    class: &str,
    choice: &str,
    rng: &mut dyn DiceRng,
) -> Result<Resolved> {
    let Some(ce) = data().class_equipment.get(rs).and_then(|m| m.get(class)) else {
        bail!(
            "no starting-equipment data for {rs} `{class}`: list the items with `chargen set equipment` and add gold with `talkdnd loot add --gp N --to party`"
        );
    };
    let choice = choice.trim();
    if !ce.options.is_empty() {
        let k = choice.to_uppercase();
        let Some(p) = ce.options.get(&k) else {
            bail!(
                "{rs} {class}: choose one of {}",
                ce.options.keys().cloned().collect::<Vec<_>>().join(" / ")
            );
        };
        return Ok(Resolved {
            items: split_items(&p.items),
            gold: p.gold,
            roll: None,
        });
    }
    if choice.eq_ignore_ascii_case("gold") {
        let dice = ce.gold_dice.clone().unwrap_or_else(|| "4d4x10".into());
        let (expr, mult) = match dice.split_once('x') {
            Some((e, m)) => (e.to_string(), m.parse::<i64>().unwrap_or(1)),
            None => (dice.clone(), 1),
        };
        let parsed: combat_sim::dice::DiceExpr = expr
            .parse()
            .map_err(|e| anyhow::anyhow!("bad gold dice {expr}: {e:?}"))?;
        let r = parsed.roll(rng);
        let gold = r.total as i64 * mult;
        return Ok(Resolved {
            items: Vec::new(),
            gold,
            roll: Some(format!(
                "{}{} = {gold} gp",
                r.text,
                if mult > 1 {
                    format!(" ×{mult}")
                } else {
                    String::new()
                }
            )),
        });
    }
    let picks: Vec<String> = choice
        .split([',', ' '])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if picks.len() != ce.groups.len() {
        let desc: Vec<String> = ce
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let opts: Vec<String> = g
                    .iter()
                    .enumerate()
                    .map(|(j, o)| format!("({}) {o}", (b'a' + j as u8) as char))
                    .collect();
                format!("{}. {}", i + 1, opts.join(" "))
            })
            .collect();
        bail!(
            "{rs} {class}: give {} letter(s), e.g. `{}`, or `gold`. Groups: {}",
            ce.groups.len(),
            vec!["a"; ce.groups.len()].join(","),
            desc.join("; ")
        );
    }
    let mut chosen = Vec::new();
    for (i, (g, p)) in ce.groups.iter().zip(&picks).enumerate() {
        let idx = p.bytes().next().map(|b| b.wrapping_sub(b'a') as usize);
        match idx.and_then(|i| g.get(i)) {
            Some(o) => chosen.push(o.clone()),
            None => bail!("group {} has {} option(s); got `{p}`", i + 1, g.len()),
        }
    }
    chosen.extend(ce.fixed.iter().cloned());
    Ok(Resolved {
        items: split_items(&chosen),
        gold: 0,
        roll: None,
    })
}

/// Background equipment: 2014 fixed list; 2024 option `A`/`B`.
pub fn background_equipment(bg: &BackgroundDef, choice: Option<&str>) -> Result<Resolved> {
    if !bg.options.is_empty() {
        let Some(c) = choice else {
            bail!(
                "this background has equipment options {}: `chargen set background-equipment A`",
                bg.options.keys().cloned().collect::<Vec<_>>().join("/")
            );
        };
        let Some(p) = bg.options.get(&c.trim().to_uppercase()) else {
            bail!("unknown option `{c}`");
        };
        return Ok(Resolved {
            items: split_items(&p.items),
            gold: p.gold,
            roll: None,
        });
    }
    Ok(Resolved {
        items: split_items(&bg.equipment),
        gold: bg.gold,
        roll: None,
    })
}

/// A placeholder item (`{martial weapon}`) the player still has to name.
pub fn is_placeholder(item: &str) -> bool {
    item.contains('{')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_loads_and_styles_by_class() {
        assert!(style("great-weapon-fighting").is_some());
        let (l, a) = style_for_class("2014", "paladin").unwrap();
        assert_eq!(l, 2);
        assert!(a.contains(&"great-weapon-fighting".to_string()));
        assert!(!a.contains(&"archery".to_string()));
        let (l, a) = style_for_class("2024", "fighter").unwrap();
        assert_eq!(l, 1);
        assert!(a.contains(&"dueling".to_string()));
        assert!(!a.contains(&"blessed-warrior".to_string()));
        assert!(style_for_class("2014", "rogue").is_none());
    }

    #[test]
    fn class_equipment_hint_names_real_commands() {
        let mut rng = combat_sim::dice::seeded_rng(1);
        let err = class_equipment("2014", "no-such-class", "a", &mut rng)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("chargen set gold"), "{err}");
        assert!(err.contains("chargen set equipment"), "{err}");
        assert!(err.contains("talkdnd loot add --gp"), "{err}");
    }

    #[test]
    fn class_equipment_2014_groups_and_gold() {
        let mut rng = combat_sim::dice::seeded_rng(1);
        let r = class_equipment("2014", "fighter", "a,b,a,b", &mut rng).unwrap();
        assert!(r.items.contains(&"chain mail".to_string()));
        assert!(r.items.contains(&"light crossbow".to_string()));
        assert!(r.items.contains(&"explorer's pack".to_string()));
        assert!(class_equipment("2014", "fighter", "a,b", &mut rng).is_err());
        let g = class_equipment("2014", "fighter", "gold", &mut rng).unwrap();
        assert!(g.gold >= 50 && g.gold <= 200 && g.gold % 10 == 0);
        assert!(g.roll.is_some());
    }

    #[test]
    fn class_equipment_2024_options() {
        let mut rng = combat_sim::dice::seeded_rng(1);
        let r = class_equipment("2024", "fighter", "a", &mut rng).unwrap();
        assert_eq!(r.gold, 4);
        assert!(r.items.contains(&"greatsword".to_string()));
        assert_eq!(
            class_equipment("2024", "fighter", "C", &mut rng)
                .unwrap()
                .gold,
            155
        );
    }

    #[test]
    fn background_equipment_srd() {
        let a = srd_background("2014", "acolyte").unwrap();
        let r = background_equipment(a, None).unwrap();
        assert_eq!(r.gold, 15);
        let s = srd_background("2024", "soldier").unwrap();
        assert!(background_equipment(s, None).is_err());
        assert_eq!(background_equipment(s, Some("b")).unwrap().gold, 50);
    }
}
