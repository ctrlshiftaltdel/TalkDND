//! Durable progress and the handoff between the campaign clock and an encounter.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::campaign_time::SECONDS_PER_ROUND;
use crate::content::schema::{Ability, AbilityCost, CastingUnit};
use crate::state::{DEFAULT_STATE_FILE, STATE_SUFFIX};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CastRecipe {
    pub caster_id: String,
    pub spell_id: String,
    pub edition: String,
    pub required_seconds: u64,
    pub slot_level: Option<i32>,
    pub cost: Option<AbilityCost>,
    pub material: Option<String>,
}

impl CastRecipe {
    pub fn from_ability(
        caster_id: &str,
        edition: &str,
        ability: &Ability,
        material: Option<String>,
    ) -> Result<Self> {
        let spell = ability
            .spell
            .as_ref()
            .context("this spell has no structured casting time")?;
        let unit = match spell.casting_time.unit {
            CastingUnit::Minute => crate::campaign_time::SECONDS_PER_MINUTE,
            CastingUnit::Hour => crate::campaign_time::SECONDS_PER_HOUR,
            _ => bail!("this spell does not have a minute- or hour-long casting time"),
        };
        let count = spell.casting_time.amount.unwrap_or(1);
        if count == 0 {
            bail!("casting time must be positive");
        }
        if ability.cost.as_ref().is_some_and(|cost| cost.amount == 0) {
            bail!("spell costs must be positive");
        }
        if spell
            .components
            .material
            .as_ref()
            .is_some_and(|item| item.cost_gp.is_some() || item.consumed)
            && material
                .as_deref()
                .is_none_or(|item| item.trim().is_empty())
        {
            bail!("this spell needs a non-empty --material declaration");
        }
        let slot_level = ability
            .cost
            .as_ref()
            .and_then(|cost| cost.resource.strip_prefix("spell-slot-")?.parse().ok())
            .or(ability.slot_level);
        Ok(Self {
            caster_id: caster_id.to_string(),
            spell_id: ability.id.clone(),
            edition: edition.to_string(),
            required_seconds: unit * u64::from(count),
            slot_level,
            cost: ability.cost.clone(),
            material,
        })
    }
}

pub(crate) const CONCENTRATION_PREFIX: &str = "long-cast:";

pub fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CastStatus {
    Casting,
    Interrupted,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CastProgress {
    pub recipe: CastRecipe,
    pub elapsed_seconds: u64,
    pub status: CastStatus,
}

impl CastProgress {
    pub fn ready_to_complete(&self) -> bool {
        self.status == CastStatus::Casting && self.elapsed_seconds == self.recipe.required_seconds
    }

    pub fn phase_name(&self) -> &'static str {
        match self.status {
            CastStatus::Casting if self.ready_to_complete() => "ready",
            CastStatus::Casting => "casting",
            CastStatus::Interrupted => "interrupted",
            CastStatus::Completed => "completed",
        }
    }

    fn advance(&mut self, seconds: u64) {
        if self.status == CastStatus::Casting {
            self.elapsed_seconds = self
                .elapsed_seconds
                .saturating_add(seconds)
                .min(self.recipe.required_seconds);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub id: String,
    /// An encounter filename relative to combat/, never a supplied path.
    pub encounter_file: String,
    pub before_digest: String,
    pub start_round: u32,
}

impl Handoff {
    pub fn verify_filename(name: &str) -> Result<()> {
        let filename = Path::new(name).file_name().and_then(|s| s.to_str());
        if filename != Some(name)
            || name.contains(['/', '\\', ':'])
            || (name != DEFAULT_STATE_FILE && !name.ends_with(STATE_SUFFIX))
        {
            bail!("a casting handoff needs an encounter filename within combat/");
        }
        Ok(())
    }

    pub fn encounter_path(save: &Path, name: &str) -> Result<PathBuf> {
        Self::verify_filename(name)?;
        let save = fs::canonicalize(save).context("resolve the casting save")?;
        let directory = save.join("combat");
        let path = directory.join(name);
        let metadata = fs::symlink_metadata(&path).context("inspect the casting encounter")?;
        if !metadata.file_type().is_file()
            || fs::canonicalize(&path)?.parent() != Some(directory.as_path())
        {
            bail!(
                "the casting encounter must be a regular file inside the save's combat directory"
            );
        }
        Ok(path)
    }

    pub fn fingerprint(path: &Path) -> Result<String> {
        let bytes = fs::read(path).context("read the casting encounter revision")?;
        Ok(advancement_lock::save_lock::digest(&bytes))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EncounterCasting {
    pub handoff_id: String,
    pub start_round: u32,
    pub casts: BTreeMap<String, EncounterCast>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EncounterCast {
    pub progress: CastProgress,
    /// The last caster turn that actually spent an action on this casting.
    pub last_action_round: Option<u32>,
}

impl EncounterCasting {
    /// Complete rounds count once for the campaign clock. A current round
    /// already spent on casting counts when the encounter ends in that round.
    pub fn elapsed_seconds(&self, current_round: u32) -> Result<u64> {
        let completed = current_round
            .checked_sub(self.start_round)
            .context("encounter round is earlier than the casting handoff")?;
        let mut rounds = u64::from(completed);
        for cast in self.casts.values() {
            if let Some(last) = cast.last_action_round {
                if last < self.start_round || last > current_round {
                    bail!("casting action round is outside the handoff");
                }
                rounds = rounds.max(u64::from(last - self.start_round) + 1);
            }
        }
        Ok(rounds * SECONDS_PER_ROUND)
    }
}

impl EncounterCast {
    pub fn spend_turn(&mut self, round: u32) -> Result<()> {
        if self.progress.status != CastStatus::Casting {
            bail!("this casting is no longer active");
        }
        if self.progress.ready_to_complete() {
            bail!("this casting is ready to complete");
        }
        if let Some(last) = self.last_action_round {
            if last.checked_add(1) != Some(round) {
                bail!("the casting action has already been spent this round or a turn was missed");
            }
        }
        self.last_action_round = Some(round);
        self.progress.advance(SECONDS_PER_ROUND);
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CastingLedger {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub casts: BTreeMap<String, CastProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<Handoff>,
}

impl CastingLedger {
    pub fn is_empty(&self) -> bool {
        self.casts.is_empty() && self.handoff.is_none()
    }

    pub fn verify(&self) -> Result<()> {
        if let Some(handoff) = &self.handoff {
            Handoff::verify_filename(&handoff.encounter_file)?;
            if handoff.id.is_empty() || handoff.before_digest.is_empty() || handoff.start_round < 1
            {
                bail!("incomplete long casting handoff");
            }
        }
        let mut active = std::collections::BTreeSet::new();
        for (id, cast) in &self.casts {
            let recipe = &cast.recipe;
            if id.is_empty()
                || recipe.caster_id.is_empty()
                || recipe.spell_id.is_empty()
                || !matches!(recipe.edition.as_str(), "2014" | "2024")
                || recipe.required_seconds < 60
                || cast.elapsed_seconds > recipe.required_seconds
                || (cast.status == CastStatus::Completed
                    && cast.elapsed_seconds != recipe.required_seconds)
            {
                bail!("invalid long casting record");
            }
            if cast.status == CastStatus::Casting && !active.insert(&recipe.caster_id) {
                bail!("a caster cannot maintain two long castings");
            }
        }
        Ok(())
    }

    pub fn begin(&mut self, id: String, recipe: CastRecipe) -> Result<()> {
        let mut after = self.clone();
        after.require_exploration()?;
        if after.casts.contains_key(&id) {
            bail!("long casting ID already exists");
        }
        after.casts.insert(
            id,
            CastProgress {
                recipe,
                elapsed_seconds: 0,
                status: CastStatus::Casting,
            },
        );
        after.verify()?;
        *self = after;
        Ok(())
    }

    pub fn advance_exploration(&mut self, seconds: u64) -> Result<()> {
        self.require_exploration()?;
        for cast in self.casts.values_mut() {
            cast.advance(seconds);
        }
        Ok(())
    }

    pub fn enter(&mut self, handoff: Handoff) -> Result<()> {
        let mut after = self.clone();
        after.require_exploration()?;
        if !after
            .casts
            .values()
            .any(|cast| cast.status == CastStatus::Casting)
        {
            bail!("there are no active long castings to hand off");
        }
        if after.casts.values().any(CastProgress::ready_to_complete) {
            bail!("finish or interrupt ready long castings before entering combat");
        }
        after.handoff = Some(handoff);
        after.verify()?;
        *self = after;
        Ok(())
    }

    pub fn encounter_progress(&self) -> Result<EncounterCasting> {
        let handoff = self.handoff.as_ref().context("no active casting handoff")?;
        Ok(EncounterCasting {
            handoff_id: handoff.id.clone(),
            start_round: handoff.start_round,
            casts: self
                .casts
                .iter()
                .filter(|(_, cast)| cast.status == CastStatus::Casting)
                .map(|(id, progress)| {
                    (
                        id.clone(),
                        EncounterCast {
                            progress: progress.clone(),
                            last_action_round: None,
                        },
                    )
                })
                .collect(),
        })
    }

    pub fn validate_encounter(&self, battle: &EncounterCasting, current_round: u32) -> Result<u64> {
        let expected = self.encounter_progress()?;
        if expected.handoff_id != battle.handoff_id
            || expected.start_round != battle.start_round
            || expected.casts.keys().ne(battle.casts.keys())
        {
            bail!("encounter has a different long casting handoff");
        }
        let seconds = battle.elapsed_seconds(current_round)?;
        for (id, starting) in &expected.casts {
            let resolved = &battle.casts[id];
            let final_cast = &resolved.progress;
            let before = starting.progress.elapsed_seconds;
            let delta = final_cast.elapsed_seconds.checked_sub(before);
            let valid_delta = match resolved.last_action_round {
                None => delta == Some(0),
                Some(last) => delta.is_some_and(|elapsed| {
                    let maximum = u64::from(last - battle.start_round + 1) * SECONDS_PER_ROUND;
                    elapsed > 0
                        && elapsed <= maximum
                        && (elapsed % SECONDS_PER_ROUND == 0
                            || final_cast.elapsed_seconds == final_cast.recipe.required_seconds)
                }),
            };
            let duration = final_cast.recipe.required_seconds;
            if starting.progress.recipe != final_cast.recipe
                || !valid_delta
                || final_cast.elapsed_seconds > duration
                || (final_cast.status == CastStatus::Completed
                    && final_cast.elapsed_seconds != duration)
            {
                bail!("encounter contains inconsistent long casting progress");
            }
        }
        Ok(seconds)
    }

    pub fn leave(&mut self, battle: &EncounterCasting, current_round: u32) -> Result<u64> {
        let seconds = self.validate_encounter(battle, current_round)?;
        self.casts.extend(
            battle
                .casts
                .iter()
                .map(|(id, cast)| (id.clone(), cast.progress.clone())),
        );
        self.handoff = None;
        Ok(seconds)
    }

    pub fn require_exploration(&self) -> Result<()> {
        if self.handoff.is_some() {
            bail!(
                "world time is attached to an encounter; reconcile it before advancing exploration"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign_time::WorldTime;

    fn recipe() -> CastRecipe {
        CastRecipe {
            caster_id: "test-caster".into(),
            spell_id: "test-spell".into(),
            edition: "2024".into(),
            required_seconds: 60,
            slot_level: Some(1),
            cost: Some(AbilityCost {
                resource: "spell-slot-1".into(),
                amount: 1,
            }),
            material: None,
        }
    }

    #[test]
    fn exploration_and_combat_time_reconcile_once_after_reload() {
        let mut campaign = WorldTime {
            enabled: true,
            ..WorldTime::default()
        };
        campaign.enable_second_clock().unwrap();
        let clock = campaign.clock.as_mut().unwrap();
        clock.casting.begin("test-id".into(), recipe()).unwrap();
        campaign.advance_seconds(42).unwrap();
        let clock = campaign.clock.as_mut().unwrap();
        clock
            .casting
            .enter(Handoff {
                id: "test-handoff".into(),
                encounter_file: "fight.state.json".into(),
                before_digest: "synthetic".into(),
                start_round: 3,
            })
            .unwrap();
        assert!(campaign.advance_seconds(6).is_err());
        let mut battle = campaign
            .clock
            .as_ref()
            .unwrap()
            .casting
            .encounter_progress()
            .unwrap();
        let cast = battle.casts.get_mut("test-id").unwrap();
        cast.spend_turn(3).unwrap();
        assert!(cast.spend_turn(3).is_err());
        cast.spend_turn(4).unwrap();
        let battle: EncounterCasting =
            serde_json::from_slice(&serde_json::to_vec(&battle).unwrap()).unwrap();
        assert_eq!(campaign.reconcile_combat(&battle, 4).unwrap(), 12);
        assert!(campaign.reconcile_combat(&battle, 4).is_err());
        campaign.advance_seconds(6).unwrap();
        let clock = campaign.clock.unwrap();
        assert_eq!(clock.elapsed_seconds, 60);
        assert!(clock.casting.casts["test-id"].ready_to_complete());
    }

    #[test]
    fn a_mismatched_encounter_cannot_change_the_campaign() {
        let mut ledger = CastingLedger::default();
        ledger.begin("first".into(), recipe()).unwrap();
        assert!(ledger.begin("second".into(), recipe()).is_err());
        assert!(Handoff::verify_filename("../fight.state.json").is_err());
        ledger
            .enter(Handoff {
                id: "current".into(),
                encounter_file: "fight.state.json".into(),
                before_digest: "synthetic".into(),
                start_round: 1,
            })
            .unwrap();
        let before = ledger.clone();
        let mut battle = ledger.encounter_progress().unwrap();
        battle.handoff_id = "older".into();
        assert!(ledger.leave(&battle, 2).is_err());
        assert_eq!(ledger, before);
    }
}
