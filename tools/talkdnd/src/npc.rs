//! Random SRD companions: species, class, background, rolled abilities.

use crate::chargen::{self, Creation};
use crate::data;
use anyhow::Result;
use combat_sim::dice::DiceRng;

pub struct Npc {
    pub creation: Creation,
    pub roll_texts: Vec<String>,
}

fn pick<'a, T>(rng: &mut dyn DiceRng, v: &'a [T]) -> &'a T {
    let n = v.len() as u32;
    &v[(rng.roll_die(n) - 1) as usize]
}

/// Picks k distinct items.
fn pick_n(rng: &mut dyn DiceRng, v: &[&str], k: usize, taken: &[String]) -> Vec<String> {
    let mut pool: Vec<&str> = v
        .iter()
        .copied()
        .filter(|s| !taken.iter().any(|t| t == s))
        .collect();
    let mut out = Vec::new();
    while out.len() < k && !pool.is_empty() {
        let i = (rng.roll_die(pool.len() as u32) - 1) as usize;
        out.push(pool.remove(i).to_string());
    }
    out
}

pub fn random(ruleset: &str, level: i32, rng: &mut dyn DiceRng) -> Result<Npc> {
    let sp = pick(rng, data::species_list(ruleset));
    let cls = pick(rng, data::CLASSES);
    let bg = pick(rng, data::background_list(ruleset));
    let (mut pool, roll_texts) = chargen::roll_pool(rng)?;
    pool.sort_by(|a, b| b.cmp(a));
    let mut c = Creation {
        ruleset: Some(ruleset.into()),
        species: Some(sp.id.into()),
        class: Some(cls.id.into()),
        background: Some(bg.id.into()),
        method: Some("roll".into()),
        pool: pool.clone(),
        level,
        asi_done: true,
        spells_done: true,
        ..Default::default()
    };
    for (a, v) in cls.priority.iter().zip(&pool) {
        c.scores.insert(a.to_string(), *v);
    }
    if ruleset == "2024" {
        // +2 to the background ability the class values most, +1 to the next.
        let mut order: Vec<&str> = bg.abilities.to_vec();
        order.sort_by_key(|a| cls.priority.iter().position(|p| p == a));
        c.asi.insert(order[0].into(), 2);
        c.asi.insert(order[1].into(), 1);
    } else if sp.free_asi > 0 {
        let mut n = 0;
        for a in cls.priority {
            if n < sp.free_asi && !sp.free_asi_exclude.contains(&a) {
                c.asi.insert(a.into(), 1);
                n += 1;
            }
        }
    }
    let mut taken = c.granted_skills();
    let all: Vec<&str> = data::SKILLS.iter().map(|s| s.0).collect();
    let list: Vec<&str> = if cls.skills.is_empty() {
        all.clone()
    } else {
        cls.skills.to_vec()
    };
    let mut skills = pick_n(rng, &list, cls.skill_count, &taken);
    taken.extend(skills.iter().cloned());
    let extra_list: Vec<&str> = if sp.free_skill_list.is_empty() {
        all
    } else {
        sp.free_skill_list.to_vec()
    };
    skills.extend(pick_n(rng, &extra_list, sp.free_skills, &taken));
    c.skills = skills;
    c.armor = Some(default_armor(cls.id).into());
    c.shield = matches!(cls.id, "fighter" | "paladin" | "cleric");
    Ok(Npc {
        creation: c,
        roll_texts,
    })
}

fn default_armor(class: &str) -> &'static str {
    match class {
        "fighter" | "paladin" => "chain-mail",
        "cleric" | "ranger" => "scale-mail",
        "barbarian" | "monk" | "sorcerer" | "wizard" => "none",
        "druid" => "leather",
        _ => "leather",
    }
}

pub fn summary(n: &Npc) -> String {
    let c = &n.creation;
    let s = c.final_scores();
    format!(
        "{} {} {} (background {}) | {} | HP {} AC {} | skills: {} | rolls: {}",
        c.species.as_deref().unwrap_or("-"),
        c.class.as_deref().unwrap_or("-"),
        c.level,
        c.background.as_deref().unwrap_or("-"),
        data::ABILITIES
            .iter()
            .map(|a| format!("{a} {}", s[*a]))
            .collect::<Vec<_>>()
            .join(" "),
        c.hp().unwrap_or(0),
        c.ac(),
        c.skills.join(", "),
        n.roll_texts.join(" ")
    )
}
