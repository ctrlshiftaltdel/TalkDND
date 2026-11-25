//! Explicit lifecycle for encounter files belonging to a campaign save.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::{EncounterState, DEFAULT_STATE_FILE, STATE_SUFFIX};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub id: String,
    /// Absent until the DM explicitly closes the encounter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
}

impl Registration {
    pub(crate) fn open() -> Self {
        Self {
            id: format!("{:032x}", rand::random::<u128>()),
            closed_reason: None,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed_reason.is_some()
    }

    pub(crate) fn close(&mut self, reason: &str) -> Result<()> {
        let reason = reason.trim();
        if reason.is_empty() || reason.contains(['\r', '\n']) {
            bail!("give a non-empty, single-line reason for closing the encounter");
        }
        if self
            .closed_reason
            .as_deref()
            .is_some_and(|value| value != reason)
        {
            bail!("the encounter has already been closed with a different reason");
        }
        self.closed_reason = Some(reason.into());
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.id.len() != 32
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("invalid encounter registration ID");
        }
        if let Some(reason) = &self.closed_reason {
            if reason.trim().is_empty() || reason.contains(['\r', '\n']) {
                bail!("invalid encounter closure reason");
            }
        }
        Ok(())
    }
}

pub(crate) fn owner(path: &Path) -> Result<Option<PathBuf>> {
    let Some(directory) = path.parent() else {
        return Ok(None);
    };
    let Some(save) = directory.parent() else {
        return Ok(None);
    };
    let name = path.file_name().and_then(|name| name.to_str());
    if directory.file_name().and_then(|name| name.to_str()) != Some("combat")
        || save
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("Saves")
        || !name.is_some_and(|name| name == DEFAULT_STATE_FILE || name.ends_with(STATE_SUFFIX))
        || advancement_lock::play_root_for_save(save).is_none()
    {
        return Ok(None);
    }
    let owner = fs::canonicalize(save).context("resolve the encounter's save")?;
    if fs::canonicalize(directory)?.parent() != Some(owner.as_path()) {
        bail!("the encounter directory does not belong to this save");
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("encounter registration needs a regular state file")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect the encounter state file"),
    }
    Ok(Some(owner))
}

pub(crate) fn verify_party_link(save: &Path, state: &EncounterState) -> Result<()> {
    let party = state
        .party_state
        .as_deref()
        .context("the encounter has no verified campaign party-state reference")?;
    if party.file_name().and_then(|name| name.to_str()) != Some(crate::party::FILE_NAME)
        || party.parent().map(fs::canonicalize).transpose()?.as_deref()
            != Some(save.join("combat").as_path())
    {
        bail!("the encounter references a different save's party state");
    }
    match fs::symlink_metadata(party) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("the campaign party state is not a regular file");
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("inspect the campaign party state"),
    }
    Ok(())
}

/// Read-only gate for advancement. The publisher must call it with the
/// save lock held so cooperating encounter writers cannot change the answer.
pub fn require_between_encounters(save: &Path) -> Result<()> {
    let world = crate::campaign_time::WorldTime::load(save).context("read campaign clock")?;
    if world
        .clock
        .as_ref()
        .is_some_and(|clock| clock.casting.handoff.is_some())
    {
        bail!("the campaign clock is still attached to an encounter");
    }
    let directory = save.join("combat");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => bail!("the combat state directory has ambiguous ownership"),
        Err(error) => return Err(error).context("inspect the combat state directory"),
    }
    let campaign = directory.join("campaign.yaml");
    match fs::read_to_string(&campaign) {
        Ok(text) => {
            let manifest: crate::content::schema::CampaignManifest = serde_norway::from_str(&text)
                .context("read encounter locations from campaign.yaml")?;
            let state_dir = manifest
                .state_dir
                .context("campaign.yaml has no state_dir")?;
            if fs::canonicalize(directory.join(state_dir))? != fs::canonicalize(&directory)? {
                bail!("the campaign uses an unverified encounter state directory");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("read campaign.yaml"),
    }
    let mut states = fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
    states.sort_by_key(|entry| entry.file_name());
    let mut seen = BTreeSet::new();
    for entry in states {
        let filename = entry.file_name();
        let Some(name) = filename.to_str() else {
            bail!("the combat directory contains a filename that cannot be verified");
        };
        let lowercase = name.to_ascii_lowercase();
        if lowercase != DEFAULT_STATE_FILE && !lowercase.ends_with(STATE_SUFFIX) {
            continue;
        }
        let path = entry.path();
        let owner = owner(&path)?.with_context(|| format!("cannot verify the owner of {name}"))?;
        let state =
            EncounterState::load(&path).with_context(|| format!("read encounter {name}"))?;
        verify_party_link(&owner, &state).with_context(|| format!("check encounter {name}"))?;
        if state.awaiting_reaction() {
            bail!("encounter {name} has a pending reaction");
        }
        let registration = state
            .registration
            .context(format!("review unregistered encounter {name}"))?;
        if !registration.is_closed() {
            bail!("encounter {name} is still open");
        }
        if !seen.insert(registration.id) {
            bail!("duplicate encounter registration at {name}");
        }
    }
    Ok(())
}
