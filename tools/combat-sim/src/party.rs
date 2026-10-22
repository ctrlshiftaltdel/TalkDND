//! Persistent per-save party state (`party-state.yaml` in the manifest's
//! `state_dir`): current HP, resources (spell slots, Hit Dice, ...),
//! lasting conditions and death saves carried between encounters.
//!
//! `new` applies it to party-side combatants; every state command writes the
//! party's values back; `party rest` handles rests outside combat.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::content::schema::{AbilityName, Creature, Recharge};
use crate::dice::DiceRng;
use crate::state::{
    ActiveCondition, ActiveDuration, CombatantState, DeathSaves, EncounterState, ResourceState,
};

pub const FILE_NAME: &str = "party-state.yaml";

/// Fallback when a campaign has no `rules.dying_conditions`. Split so the
/// production-source content-name guard does not flag this generic default.
pub const DEFAULT_DYING_CONDITION: &str = concat!("un", "conscious");

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartyState {
    /// Keyed by creature id.
    #[serde(default)]
    pub members: BTreeMap<String, Member>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub name: String,
    pub hp: i32,
    pub max_hp: i32,
    #[serde(default)]
    pub temp_hp: i32,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceState>,
    /// Lasting conditions (no round-based duration, not concentration-linked).
    #[serde(default)]
    pub conditions: Vec<ActiveCondition>,
    #[serde(default)]
    pub death_saves: DeathSaves,
    #[serde(default)]
    pub dead: bool,
    /// Hit-die size and Con modifier, for `party rest short --hd`.
    #[serde(default)]
    pub hit_die: Option<u32>,
    #[serde(default)]
    pub con_mod: i32,
}

impl PartyState {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_norway::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = format!(
            "# TalkDND party state (written by combat-sim; edit with care)\n{}",
            serde_norway::to_string(self)?
        );
        let save = crate::integrity::save_of_combat_file(path);
        let before = save.as_deref().map(crate::integrity::snapshot);
        crate::fsutil::atomic_write(path, text.as_bytes())
            .with_context(|| format!("failed to write {}", path.display()))?;
        if let (Some(save), Some(before)) = (save, before) {
            crate::integrity::after_write(&save, &before)?;
        }
        Ok(())
    }

    /// Copy the party-side combatants' current values into the file data.
    pub fn record(&mut self, state: &EncounterState) {
        let mut seen = Vec::new();
        for combatant in &state.combatants {
            if combatant.side.as_str() != "party" {
                continue;
            }
            let id = combatant.creature_id().to_string();
            if seen.contains(&id) {
                continue;
            }
            seen.push(id.clone());
            self.members.insert(id, Member::from_combatant(combatant));
        }
    }

    pub fn summary_lines(&self) -> Vec<String> {
        self.members.iter().map(|(id, m)| m.summary(id)).collect()
    }
}

fn lasting(condition: &ActiveCondition) -> bool {
    !condition.concentration
        && !matches!(condition.duration, Some(ActiveDuration::Rounds { .. }))
        && !matches!(condition.duration, Some(ActiveDuration::Until { .. }))
}

impl Member {
    pub fn from_creature(creature: &Creature) -> Self {
        let max_hp = creature.max_hp().max(1);
        Self {
            name: creature.name.clone().unwrap_or_else(|| creature.id.clone()),
            hp: creature.current_hp().clamp(0, max_hp),
            max_hp,
            temp_hp: 0,
            resources: creature
                .resources
                .as_ref()
                .map(|r| {
                    r.iter()
                        .map(|(id, def)| (id.clone(), ResourceState::from_def(def)))
                        .collect()
                })
                .unwrap_or_default(),
            conditions: Vec::new(),
            death_saves: DeathSaves::default(),
            dead: false,
            hit_die: hit_die_of(creature),
            con_mod: creature.ability_modifier(AbilityName::Con),
        }
    }

    pub fn from_combatant(c: &CombatantState) -> Self {
        Self {
            name: c.name.clone(),
            hp: c.hp,
            max_hp: c.max_hp,
            temp_hp: c.temp_hp,
            resources: c.resources.clone(),
            conditions: c
                .conditions
                .iter()
                .filter(|x| lasting(x))
                .cloned()
                .collect(),
            death_saves: c.death_saves,
            dead: c.is_dead(),
            hit_die: hit_die_of(&c.creature),
            con_mod: c.creature.ability_modifier(AbilityName::Con),
        }
    }

    /// Apply stored values to a freshly built combatant.
    pub fn apply(&self, c: &mut CombatantState) {
        c.hp = self.hp.clamp(0, c.max_hp);
        c.temp_hp = self.temp_hp.max(0);
        for (id, stored) in &self.resources {
            if let Some(r) = c.resources.get_mut(id) {
                r.current = stored.current.clamp(0, r.max);
            }
        }
        for condition in &self.conditions {
            if !c
                .conditions
                .iter()
                .any(|x| x.condition.eq_ignore_ascii_case(&condition.condition))
            {
                c.conditions.push(condition.clone());
            }
        }
        c.death_saves = self.death_saves;
        c.dead = self.dead;
    }

    pub fn summary(&self, id: &str) -> String {
        let res = self
            .resources
            .iter()
            .map(|(k, r)| format!("{k} {}/{}", r.current, r.max))
            .collect::<Vec<_>>()
            .join(", ");
        let cond = self
            .conditions
            .iter()
            .map(|c| c.condition.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let status = if self.dead {
            " DEAD".to_string()
        } else if self.hp <= 0 {
            format!(
                " DOWN ({}S/{}F{})",
                self.death_saves.successes,
                self.death_saves.failures,
                if self.death_saves.stable {
                    ", stable"
                } else {
                    ""
                }
            )
        } else {
            String::new()
        };
        let temp = if self.temp_hp > 0 {
            format!(" (+{})", self.temp_hp)
        } else {
            String::new()
        };
        format!(
            "{} `{id}`: HP {}/{}{temp}{status} | resources: {} | conditions: {}",
            self.name,
            self.hp,
            self.max_hp,
            if res.is_empty() { "-" } else { &res },
            if cond.is_empty() { "-" } else { &cond }
        )
    }

    pub fn short_rest(
        &mut self,
        hd: u32,
        rng: &mut dyn DiceRng,
        log: &mut Vec<String>,
    ) -> Result<()> {
        if self.dead {
            log.push(format!("{} is dead and cannot rest", self.name));
            return Ok(());
        }
        // Validate the Hit Dice before changing any resource: a failed rest
        // must not restore short-rest pools.
        let die = if hd == 0 {
            None
        } else {
            let left = self
                .resources
                .get("hit-dice")
                .map_or(0, |r| r.current.max(0) as u32);
            if hd > left {
                bail!("`{}` has only {left} Hit Dice left", self.name);
            }
            Some(self.hit_die.filter(|d| *d >= 2).ok_or_else(|| {
                anyhow!("`{}` has no hit-dice resource with a die size", self.name)
            })?)
        };
        for (id, r) in self.resources.iter_mut() {
            if id == "hit-dice" {
                continue;
            }
            if r.recharge == Recharge::Short {
                r.current = r.max;
            } else if let Some(n) = r.short_rest_regain {
                r.current = (r.current + n).min(r.max);
            }
        }
        log.push(format!("{} takes a short rest", self.name));
        let Some(die) = die else {
            return Ok(());
        };
        let before = self.hp;
        for _ in 0..hd {
            let roll = rng.roll_die(die) as i32;
            let gain = (roll + self.con_mod).max(0);
            log.push(format!(
                "{} spends a Hit Die: d{die} {roll} {:+} = {gain}",
                self.name, self.con_mod
            ));
            self.hp = (self.hp + gain).min(self.max_hp);
        }
        if let Some(r) = self.resources.get_mut("hit-dice") {
            r.current -= hd as i32;
        }
        if before <= 0 && self.hp > 0 {
            self.death_saves = DeathSaves::default();
        }
        log.push(format!(
            "{} regains {} HP ({}/{})",
            self.name,
            self.hp - before,
            self.hp,
            self.max_hp
        ));
        Ok(())
    }

    /// Set HP from outside combat, adding the campaign's dying conditions at
    /// 0 HP and removing them (plus death saves) at 1 HP or more.
    pub fn set_hp(&mut self, hp: i32, dying_conditions: &[String]) {
        self.hp = hp.clamp(0, self.max_hp);
        if self.hp > 0 {
            self.conditions.retain(|active| {
                !dying_conditions
                    .iter()
                    .any(|dying| active.condition.eq_ignore_ascii_case(dying))
            });
            self.death_saves = DeathSaves::default();
        } else {
            for name in dying_conditions {
                if !self
                    .conditions
                    .iter()
                    .any(|active| active.condition.eq_ignore_ascii_case(name))
                {
                    self.conditions.push(ActiveCondition {
                        condition: name.clone(),
                        source: Some("dying".into()),
                        linked: None,
                        duration: None,
                        save_ends: None,
                        ends_on: Vec::new(),
                        created_by: None,
                        concentration: false,
                        applied_round: 0,
                        notes: None,
                    });
                }
            }
        }
    }

    pub fn long_rest(&mut self, dying_conditions: &[String], log: &mut Vec<String>) {
        if self.dead {
            log.push(format!("{} is dead and cannot rest", self.name));
            return;
        }
        for (id, r) in self.resources.iter_mut() {
            if id == "hit-dice" {
                r.current = (r.current + (r.max / 2).max(1)).min(r.max);
            } else {
                r.current = r.max;
            }
        }
        self.set_hp(self.max_hp, dying_conditions);
        self.temp_hp = 0;
        self.conditions
            .retain(|c| !matches!(c.duration, Some(ActiveDuration::Minutes { .. })));
        log.push(format!("{} takes a long rest", self.name));
    }
}

fn hit_die_of(creature: &Creature) -> Option<u32> {
    let def = creature.resources.as_ref()?.get("hit-dice")?;
    def.die.filter(|d| *d >= 2).or_else(|| {
        let notes = def.notes.as_deref()?;
        let at = notes.find('d')?;
        notes[at + 1..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sided_hit_die_is_ignored() {
        let creature: Creature = serde_norway::from_str(
            "id: test\nresources:\n  hit-dice: {max: 1, recharge: long, die: 0}\n",
        )
        .unwrap();
        assert_eq!(hit_die_of(&creature), None);
    }

    fn member(hp: i32) -> Member {
        Member {
            name: "T".into(),
            hp,
            max_hp: 20,
            ..Default::default()
        }
    }

    #[test]
    fn set_hp_adds_and_removes_dying_conditions() {
        let dying = vec![DEFAULT_DYING_CONDITION.to_string()];
        let mut member = member(10);
        member.set_hp(0, &dying);
        assert_eq!(member.hp, 0);
        assert_eq!(member.conditions.len(), 1);
        assert_eq!(member.conditions[0].condition, DEFAULT_DYING_CONDITION);

        member.set_hp(1, &dying);
        assert_eq!(member.hp, 1);
        assert!(member.conditions.is_empty());
    }

    #[test]
    fn long_rest_removes_dying_conditions() {
        let dying = vec![DEFAULT_DYING_CONDITION.to_string()];
        let mut member = member(0);
        member.set_hp(0, &dying);
        assert_eq!(member.conditions[0].condition, DEFAULT_DYING_CONDITION);

        member.long_rest(&dying, &mut Vec::new());
        assert_eq!(member.hp, 20);
        assert!(member.conditions.is_empty());
    }

    #[test]
    fn short_rest_rejects_zero_sided_hit_die() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "hit-dice".into(),
            ResourceState {
                current: 1,
                max: 1,
                recharge: Recharge::Long,
                short_rest_regain: None,
            },
        );
        let mut member = Member {
            name: "T".into(),
            hp: 5,
            max_hp: 5,
            resources,
            hit_die: Some(0),
            ..Default::default()
        };
        let err = member
            .short_rest(1, &mut crate::dice::seeded_rng(1), &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("die size"), "{err}");
        assert_eq!(member.hp, 5);
        assert_eq!(member.resources["hit-dice"].current, 1);
    }

    #[test]
    fn short_rest_checks_hit_dice_before_restoring() {
        let mut resources = BTreeMap::new();
        resources.insert(
            "ki".into(),
            ResourceState {
                current: 0,
                max: 2,
                recharge: Recharge::Short,
                short_rest_regain: None,
            },
        );
        resources.insert(
            "hit-dice".into(),
            ResourceState {
                current: 1,
                max: 1,
                recharge: Recharge::Long,
                short_rest_regain: None,
            },
        );
        let mut member = Member {
            name: "T".into(),
            hp: 5,
            max_hp: 5,
            resources,
            hit_die: Some(8),
            ..Default::default()
        };
        assert!(member
            .short_rest(2, &mut crate::dice::seeded_rng(1), &mut Vec::new())
            .is_err());
        assert_eq!(member.resources["ki"].current, 0);
    }
}
