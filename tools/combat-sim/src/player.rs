//! Player-facing name masking for combat output.
//!
//! The DM's view (`combat-sim status/log/simulate`) shows real names. With
//! `--player`, a monster or item that the party has not identified is printed
//! as its appearance description instead: an `appearance` override from
//! `Saves/<slot>/identified.yaml` (written by `talkdnd identify --appearance`),
//! or a generic size + type phrase such as `一只大型野兽`.
//!
//! The identified ledger is read from `--save`, `TALKDND_SAVE`, or the save
//! that owns `--campaign` (`<save>/combat/campaign.yaml`).

use crate::content::schema::{Creature, CreatureKind, Size};
use crate::state::{CombatantState, EncounterState};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
struct Ledger {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Entry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    identified: bool,
    #[serde(default)]
    appearance: Option<String>,
}

/// Resolved identified ledger for one output command.
#[derive(Debug, Clone, Default)]
pub struct PlayerMask {
    entries: Vec<Entry>,
}

impl PlayerMask {
    pub fn load(save: Option<&Path>) -> Self {
        let Some(save) = save else {
            return Self::default();
        };
        let text = fs::read_to_string(save.join("identified.yaml")).unwrap_or_default();
        let ledger: Ledger = serde_norway::from_str(&text).unwrap_or_default();
        Self {
            entries: ledger.entries,
        }
    }

    fn entry(&self, keys: &[&str]) -> Option<&Entry> {
        self.entries.iter().find(|e| {
            keys.iter()
                .any(|k| !k.trim().is_empty() && e.name.eq_ignore_ascii_case(k.trim()))
        })
    }

    /// Name to print for one combatant in the player view. `None` means the
    /// real name is fine (identified, or a PC/NPC the party already knows).
    pub fn name_for(&self, combatant: &CombatantState) -> Option<String> {
        if matches!(
            combatant.creature.kind(),
            CreatureKind::Pc | CreatureKind::Npc
        ) {
            return None;
        }
        let masked = self.name_for_parts(&combatant.name, &combatant.creature)?;
        // "Wolf 1" / "Wolf 2" must stay distinguishable: carry the instance
        // number over to the description ("一只中型野兽 #1").
        let suffix = combatant
            .name
            .rsplit_once(' ')
            .map(|(_, n)| n)
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .filter(|n| !combatant.creature.display_name().ends_with(*n));
        Some(match suffix {
            Some(n) => format!("{masked} #{n}"),
            None => masked,
        })
    }

    /// Same as [`Self::name_for`] without a full combatant (tests, previews).
    pub fn name_for_parts(&self, name: &str, creature: &Creature) -> Option<String> {
        if matches!(creature.kind(), CreatureKind::Pc | CreatureKind::Npc) {
            return None;
        }
        let keys = [name, creature.display_name(), creature.id.as_str()];
        if let Some(e) = self.entry(&keys) {
            if e.identified {
                return None;
            }
            if let Some(a) = e
                .appearance
                .as_deref()
                .map(str::trim)
                .filter(|a| !a.is_empty())
            {
                return Some(a.to_string());
            }
        }
        if let Some(a) = creature
            .appearance
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
        {
            return Some(a.to_string());
        }
        Some(generic_appearance(creature))
    }

    /// `(real, masked)` replacements, longest real name first so a name that
    /// is a prefix of another does not corrupt the longer one.
    pub fn replacements(&self, state: &EncounterState) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for c in &state.combatants {
            if let Some(masked) = self.name_for(c) {
                if masked != c.name {
                    out.push((c.name.clone(), masked));
                }
            }
        }
        out.sort_by_key(|(real, _)| std::cmp::Reverse(real.chars().count()));
        out
    }

    pub fn line(&self, line: &str, replacements: &[(String, String)]) -> String {
        let mut out = line.to_string();
        for (real, masked) in replacements {
            out = out.replace(real, masked);
        }
        out
    }
}

/// `一只大型野兽` style fallback from the creature's size and type.
pub fn generic_appearance(creature: &Creature) -> String {
    let size = match creature.size() {
        Size::Tiny => "微型",
        Size::Small => "小型",
        Size::Medium => "中型",
        Size::Large => "大型",
        Size::Huge => "巨型",
        Size::Gargantuan => "超巨型",
        Size::Unknown => "",
    };
    let kind = creature
        .creature_type
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(type_zh)
        .unwrap_or("生物");
    format!("一只{size}{kind}")
}

fn type_zh(raw: &str) -> &'static str {
    // Types may carry qualifiers ("humanoid (goblinoid)"); match on the head.
    let head = raw
        .split(['(', '（', ',', '/', ' '])
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase();
    match head.as_str() {
        "beast" | "野兽" => "野兽",
        "humanoid" | "类人生物" => "类人生物",
        "undead" | "亡灵" => "亡灵",
        "dragon" | "龙" => "龙",
        "fiend" | "邪魔" => "邪魔",
        "celestial" | "天界生物" => "天界生物",
        "fey" | "精类" => "精类",
        "giant" | "巨人" => "巨人",
        "monstrosity" | "怪物" => "怪物",
        "ooze" | "泥怪" => "泥怪",
        "plant" | "植物" => "植物",
        "construct" | "构装体" => "构装体",
        "elemental" | "元素" => "元素",
        "aberration" | "异怪" => "异怪",
        _ => "生物",
    }
}

/// The save folder for identified.yaml / rolls.log.
pub fn save_dir(explicit: Option<&Path>, campaign: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os("TALKDND_SAVE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    // <save>/combat/campaign.yaml -> <save>. Only a campaign inside a
    // `combat/` directory belongs to a save (dev fixtures must not grow a
    // stray rolls.log).
    campaign
        .and_then(|p| {
            p.parent()
                .filter(|dir| dir.file_name().is_some_and(|n| n == "combat"))
        })
        .and_then(|combat| combat.parent())
        .map(Path::to_path_buf)
}

/// Append roll-bearing state-command lines to `<save>/rolls.log`. Only lines
/// with dice notation are kept, so condition/status narration does not flood
/// the roll log; the state log itself is unchanged.
pub fn append_combat_lines(save: Option<&Path>, lines: &[String]) {
    for line in lines {
        if looks_like_dice(line) {
            append_roll(save, "combat", line);
        }
    }
}

fn looks_like_dice(line: &str) -> bool {
    const DICE: &[&str] = &[
        "1d2(", "1d3(", "1d4(", "1d6(", "1d8(", "1d10(", "1d12(", "1d20(", "1d100(", "d4(", "d6(",
        "d8(", "d10(", "d12(", "d20(", "d100(",
    ];
    DICE.iter().any(|d| line.contains(d))
}

/// Append a roll to `<save>/rolls.log` (same line format as talkdnd).
pub fn append_roll(save: Option<&Path>, what: &str, text: &str) {
    let Some(save) = save else {
        return;
    };
    let line = format!(
        "- [{}] combat-sim {what}：{}\n",
        chrono_like_now(),
        text.trim()
    );
    let path = save.join("rolls.log");
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

fn chrono_like_now() -> String {
    // combat-sim has no chrono dependency; local time via `date` would spawn a
    // process. UTC is fine for an audit log, but keep the familiar shape.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant's civil-from-days (UTC).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Mask names in a status JSON document (text mode is masked line by line).
pub fn mask_status_json(value: &mut serde_json::Value, state: &EncounterState, mask: &PlayerMask) {
    let replacements = mask.replacements(state);
    if let Some(active) = value
        .get_mut("active")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    {
        let masked = mask.line(&active, &replacements);
        value["active"] = serde_json::Value::String(masked);
    }
    if let Some(combatants) = value.get_mut("combatants").and_then(|v| v.as_array_mut()) {
        for c in combatants {
            if let Some(name) = c.get("name").and_then(|v| v.as_str()).map(str::to_string) {
                c["name"] = serde_json::Value::String(mask.line(&name, &replacements));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::schema::Size;

    fn creature(id: &str, name: &str, size: Size, kind: &str) -> Creature {
        let json = format!(
            r#"{{"id":"{id}","name":"{name}","size":"{}","creature_type":"{kind}","hp":7}}"#,
            match size {
                Size::Tiny => "tiny",
                Size::Small => "small",
                Size::Medium => "medium",
                Size::Large => "large",
                Size::Huge => "huge",
                Size::Gargantuan => "gargantuan",
                Size::Unknown => "unknown",
            }
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn generic_appearance_uses_size_and_type() {
        assert_eq!(
            generic_appearance(&creature("wolf", "Wolf", Size::Large, "beast")),
            "一只大型野兽"
        );
        assert_eq!(
            generic_appearance(&creature("goblin", "Goblin", Size::Small, "humanoid")),
            "一只小型类人生物"
        );
    }

    #[test]
    fn appearance_and_identification_switch_the_player_name() {
        let d = std::env::temp_dir().join(format!("combat-sim-mask-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("identified.yaml"),
            "version: 1\nentries:\n  - {name: Wolf, identified: false, appearance: 一只灰白色的巨狼}\n  - {name: Goblin, identified: true}\n",
        )
        .unwrap();
        let wolf = creature("wolf", "Wolf", Size::Large, "beast");
        let goblin = creature("goblin", "Goblin", Size::Small, "humanoid");
        let mask = PlayerMask::load(Some(&d));
        assert_eq!(mask.entries.len(), 2);
        assert_eq!(
            mask.entry(&["wolf"]).unwrap().appearance.as_deref(),
            Some("一只灰白色的巨狼")
        );
        assert!(mask.entry(&["GOBLIN"]).unwrap().identified);
        assert_eq!(
            mask.name_for_parts("Wolf", &wolf).as_deref(),
            Some("一只灰白色的巨狼")
        );
        assert_eq!(mask.name_for_parts("Goblin", &goblin), None);
        assert_eq!(
            mask.name_for_parts("Bear", &creature("bear", "Bear", Size::Large, "beast"))
                .as_deref(),
            Some("一只大型野兽")
        );
        // Once the ledger says identified, the real name comes back.
        fs::write(
            d.join("identified.yaml"),
            "version: 1\nentries:\n  - {name: Wolf, identified: true}\n",
        )
        .unwrap();
        let mask = PlayerMask::load(Some(&d));
        assert_eq!(mask.name_for_parts("Wolf", &wolf), None);
        // PCs keep their names even without a ledger entry.
        let mut pc = creature("hero", "Hero", Size::Medium, "humanoid");
        pc.kind = Some(CreatureKind::Pc);
        assert_eq!(mask.name_for_parts("Hero", &pc), None);
        let _ = fs::remove_dir_all(&d);
    }
}
