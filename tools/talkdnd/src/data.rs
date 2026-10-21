//! SRD-only character options (SRD 5.1 for 2014, SRD 5.2.1 for 2024).
//! Only mechanical facts (names, numbers) are stored here; no rules text.

pub const ABILITIES: [&str; 6] = ["str", "dex", "con", "int", "wis", "cha"];

pub const SKILLS: &[(&str, &str)] = &[
    ("acrobatics", "dex"),
    ("animal-handling", "wis"),
    ("arcana", "int"),
    ("athletics", "str"),
    ("deception", "cha"),
    ("history", "int"),
    ("insight", "wis"),
    ("intimidation", "cha"),
    ("investigation", "int"),
    ("medicine", "wis"),
    ("nature", "int"),
    ("perception", "wis"),
    ("performance", "cha"),
    ("persuasion", "cha"),
    ("religion", "int"),
    ("sleight-of-hand", "dex"),
    ("stealth", "dex"),
    ("survival", "wis"),
];

pub fn skill_ability(skill: &str) -> Option<&'static str> {
    SKILLS.iter().find(|(s, _)| *s == skill).map(|(_, a)| *a)
}

pub struct Species {
    pub id: &'static str,
    /// Fixed ability increases (2014 only).
    pub asi: &'static [(&'static str, i32)],
    /// Number of free +1 increases the player picks (2014 half-elf).
    pub free_asi: usize,
    /// Abilities excluded from free picks.
    pub free_asi_exclude: &'static [&'static str],
    pub speed: i32,
    pub size: &'static str,
    /// Fixed skill proficiencies.
    pub skills: &'static [&'static str],
    /// Extra skills the player picks, and the list (empty = any).
    pub free_skills: usize,
    pub free_skill_list: &'static [&'static str],
    /// HP bonus per level (2014 hill dwarf; 2024 dwarf).
    pub hp_per_level: i32,
}

const fn sp(id: &'static str, asi: &'static [(&'static str, i32)], speed: i32) -> Species {
    Species {
        id,
        asi,
        free_asi: 0,
        free_asi_exclude: &[],
        speed,
        size: "medium",
        skills: &[],
        free_skills: 0,
        free_skill_list: &[],
        hp_per_level: 0,
    }
}

pub static SPECIES_2014: &[Species] = &[
    Species {
        hp_per_level: 1,
        ..sp("hill-dwarf", &[("con", 2), ("wis", 1)], 25)
    },
    Species {
        skills: &["perception"],
        ..sp("high-elf", &[("dex", 2), ("int", 1)], 30)
    },
    Species {
        size: "small",
        ..sp("lightfoot-halfling", &[("dex", 2), ("cha", 1)], 25)
    },
    sp(
        "human",
        &[
            ("str", 1),
            ("dex", 1),
            ("con", 1),
            ("int", 1),
            ("wis", 1),
            ("cha", 1),
        ],
        30,
    ),
    sp("dragonborn", &[("str", 2), ("cha", 1)], 30),
    Species {
        size: "small",
        ..sp("rock-gnome", &[("int", 2), ("con", 1)], 25)
    },
    Species {
        free_asi: 2,
        free_asi_exclude: &["cha"],
        free_skills: 2,
        ..sp("half-elf", &[("cha", 2)], 30)
    },
    Species {
        skills: &["intimidation"],
        ..sp("half-orc", &[("str", 2), ("con", 1)], 30)
    },
    sp("tiefling", &[("int", 1), ("cha", 2)], 30),
];

pub static SPECIES_2024: &[Species] = &[
    sp("dragonborn", &[], 30),
    Species {
        hp_per_level: 1,
        ..sp("dwarf", &[], 30)
    },
    Species {
        free_skills: 1,
        free_skill_list: &["insight", "perception", "survival"],
        ..sp("elf", &[], 30)
    },
    Species {
        size: "small",
        ..sp("gnome", &[], 30)
    },
    sp("goliath", &[], 35),
    Species {
        size: "small",
        ..sp("halfling", &[], 30)
    },
    Species {
        free_skills: 1,
        ..sp("human", &[], 30)
    },
    sp("orc", &[], 30),
    sp("tiefling", &[], 30),
];

pub struct Class {
    pub id: &'static str,
    pub hit_die: i32,
    pub saves: [&'static str; 2],
    pub skill_count: usize,
    /// Empty = any skill (bard).
    pub skills: &'static [&'static str],
    /// Abilities in priority order for random/auto assignment.
    pub priority: [&'static str; 6],
    pub spell_ability: Option<&'static str>,
}

pub static CLASSES: &[Class] = &[
    Class {
        id: "barbarian",
        hit_die: 12,
        saves: ["str", "con"],
        skill_count: 2,
        skills: &[
            "animal-handling",
            "athletics",
            "intimidation",
            "nature",
            "perception",
            "survival",
        ],
        priority: ["str", "con", "dex", "wis", "cha", "int"],
        spell_ability: None,
    },
    Class {
        id: "bard",
        hit_die: 8,
        saves: ["dex", "cha"],
        skill_count: 3,
        skills: &[],
        priority: ["cha", "dex", "con", "wis", "int", "str"],
        spell_ability: Some("cha"),
    },
    Class {
        id: "cleric",
        hit_die: 8,
        saves: ["wis", "cha"],
        skill_count: 2,
        skills: &["history", "insight", "medicine", "persuasion", "religion"],
        priority: ["wis", "con", "str", "cha", "dex", "int"],
        spell_ability: Some("wis"),
    },
    Class {
        id: "druid",
        hit_die: 8,
        saves: ["int", "wis"],
        skill_count: 2,
        skills: &[
            "arcana",
            "animal-handling",
            "insight",
            "medicine",
            "nature",
            "perception",
            "religion",
            "survival",
        ],
        priority: ["wis", "con", "dex", "int", "cha", "str"],
        spell_ability: Some("wis"),
    },
    Class {
        id: "fighter",
        hit_die: 10,
        saves: ["str", "con"],
        skill_count: 2,
        skills: &[
            "acrobatics",
            "animal-handling",
            "athletics",
            "history",
            "insight",
            "intimidation",
            "perception",
            "survival",
        ],
        priority: ["str", "con", "dex", "wis", "int", "cha"],
        spell_ability: None,
    },
    Class {
        id: "monk",
        hit_die: 8,
        saves: ["str", "dex"],
        skill_count: 2,
        skills: &[
            "acrobatics",
            "athletics",
            "history",
            "insight",
            "religion",
            "stealth",
        ],
        priority: ["dex", "wis", "con", "str", "int", "cha"],
        spell_ability: None,
    },
    Class {
        id: "paladin",
        hit_die: 10,
        saves: ["wis", "cha"],
        skill_count: 2,
        skills: &[
            "athletics",
            "insight",
            "intimidation",
            "medicine",
            "persuasion",
            "religion",
        ],
        priority: ["str", "cha", "con", "wis", "dex", "int"],
        spell_ability: Some("cha"),
    },
    Class {
        id: "ranger",
        hit_die: 10,
        saves: ["str", "dex"],
        skill_count: 3,
        skills: &[
            "animal-handling",
            "athletics",
            "insight",
            "investigation",
            "nature",
            "perception",
            "stealth",
            "survival",
        ],
        priority: ["dex", "wis", "con", "str", "int", "cha"],
        spell_ability: Some("wis"),
    },
    Class {
        id: "rogue",
        hit_die: 8,
        saves: ["dex", "int"],
        skill_count: 4,
        skills: &[
            "acrobatics",
            "athletics",
            "deception",
            "insight",
            "intimidation",
            "investigation",
            "perception",
            "performance",
            "persuasion",
            "sleight-of-hand",
            "stealth",
        ],
        priority: ["dex", "con", "int", "wis", "cha", "str"],
        spell_ability: None,
    },
    Class {
        id: "sorcerer",
        hit_die: 6,
        saves: ["con", "cha"],
        skill_count: 2,
        skills: &[
            "arcana",
            "deception",
            "insight",
            "intimidation",
            "persuasion",
            "religion",
        ],
        priority: ["cha", "con", "dex", "wis", "int", "str"],
        spell_ability: Some("cha"),
    },
    Class {
        id: "warlock",
        hit_die: 8,
        saves: ["wis", "cha"],
        skill_count: 2,
        skills: &[
            "arcana",
            "deception",
            "history",
            "intimidation",
            "investigation",
            "nature",
            "religion",
        ],
        priority: ["cha", "con", "dex", "wis", "int", "str"],
        spell_ability: Some("cha"),
    },
    Class {
        id: "wizard",
        hit_die: 6,
        saves: ["int", "wis"],
        skill_count: 2,
        skills: &[
            "arcana",
            "history",
            "insight",
            "investigation",
            "medicine",
            "religion",
        ],
        priority: ["int", "con", "dex", "wis", "cha", "str"],
        spell_ability: Some("int"),
    },
];

/// 2014 ranger SRD list differs from 2024 only by edition rules we don't model; same list used.
pub fn class(id: &str) -> Option<&'static Class> {
    CLASSES.iter().find(|c| c.id == id)
}

pub struct Background {
    pub id: &'static str,
    pub skills: [&'static str; 2],
    /// 2024: the three abilities the background may raise.
    pub abilities: &'static [&'static str],
}

pub static BACKGROUNDS_2014: &[Background] = &[Background {
    id: "acolyte",
    skills: ["insight", "religion"],
    abilities: &[],
}];

pub static BACKGROUNDS_2024: &[Background] = &[
    Background {
        id: "acolyte",
        skills: ["insight", "religion"],
        abilities: &["int", "wis", "cha"],
    },
    Background {
        id: "criminal",
        skills: ["sleight-of-hand", "stealth"],
        abilities: &["dex", "con", "int"],
    },
    Background {
        id: "sage",
        skills: ["arcana", "history"],
        abilities: &["con", "int", "wis"],
    },
    Background {
        id: "soldier",
        skills: ["athletics", "intimidation"],
        abilities: &["str", "dex", "con"],
    },
];

pub fn species(ruleset: &str, id: &str) -> Option<&'static Species> {
    let list = if ruleset == "2014" {
        SPECIES_2014
    } else {
        SPECIES_2024
    };
    list.iter().find(|s| s.id == id)
}

pub fn background(ruleset: &str, id: &str) -> Option<&'static Background> {
    let list = if ruleset == "2014" {
        BACKGROUNDS_2014
    } else {
        BACKGROUNDS_2024
    };
    list.iter().find(|b| b.id == id)
}

pub fn species_list(ruleset: &str) -> &'static [Species] {
    if ruleset == "2014" {
        SPECIES_2014
    } else {
        SPECIES_2024
    }
}

pub fn background_list(ruleset: &str) -> &'static [Background] {
    if ruleset == "2014" {
        BACKGROUNDS_2014
    } else {
        BACKGROUNDS_2024
    }
}

/// (id, base AC, dex cap: None = full, Some(0) = none)
pub static ARMOR: &[(&str, i32, Option<i32>)] = &[
    ("padded", 11, None),
    ("leather", 11, None),
    ("studded-leather", 12, None),
    ("hide", 12, Some(2)),
    ("chain-shirt", 13, Some(2)),
    ("scale-mail", 14, Some(2)),
    ("breastplate", 14, Some(2)),
    ("half-plate", 15, Some(2)),
    ("ring-mail", 14, Some(0)),
    ("chain-mail", 16, Some(0)),
    ("splint", 17, Some(0)),
    ("plate", 18, Some(0)),
];

pub const STANDARD_ARRAY: [i32; 6] = [15, 14, 13, 12, 10, 8];

pub fn point_cost(score: i32) -> Option<i32> {
    match score {
        8 => Some(0),
        9 => Some(1),
        10 => Some(2),
        11 => Some(3),
        12 => Some(4),
        13 => Some(5),
        14 => Some(7),
        15 => Some(9),
        _ => None,
    }
}

pub fn modifier(score: i32) -> i32 {
    (score - 10).div_euclid(2)
}

pub fn prof_bonus(level: i32) -> i32 {
    2 + (level - 1).max(0) / 4
}
