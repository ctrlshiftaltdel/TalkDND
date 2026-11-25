//! Campaign clock shared by exploration and encounter commands.

use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const FILE: &str = "world-time.yaml";
pub const SECONDS_PER_MINUTE: u64 = 60;
pub const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
pub const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;
pub const SECONDS_PER_ROUND: u64 = 6;

const LEGACY_VERSION: u32 = 1;
pub const VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub due: String,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub dm_only: bool,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldTime {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub calendar: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub elapsed_days: f64,
    #[serde(default)]
    pub events: Vec<Event>,
    /// The authoritative counter after the save opts into second-level timing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<CampaignClock>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CampaignClock {
    pub elapsed_seconds: u64,
    #[serde(
        default,
        skip_serializing_if = "crate::long_cast::CastingLedger::is_empty"
    )]
    pub casting: crate::long_cast::CastingLedger,
}

impl WorldTime {
    pub fn load(save: &Path) -> Result<Self> {
        let path = save.join(FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let world: Self =
            serde_norway::from_str(&text).with_context(|| format!("invalid {}", path.display()))?;
        match (world.version, world.clock.as_ref()) {
            (0 | LEGACY_VERSION, None) => {}
            (VERSION, Some(clock)) if world.elapsed_days == as_days(clock.elapsed_seconds) => {}
            _ => bail!(
                "{}: unsupported or inconsistent world-time schema version {} (this talkdnd supports version {LEGACY_VERSION} and version {VERSION} with a clock)",
                path.display(),
                world.version,
            ),
        }
        if let Some(clock) = &world.clock {
            clock
                .casting
                .verify()
                .with_context(|| format!("invalid {}", path.display()))?;
        }
        Ok(world)
    }

    pub fn store(&self, save: &Path) -> Result<()> {
        let mut current = self.clone();
        if let Some(clock) = &current.clock {
            current.version = VERSION;
            current.elapsed_days = as_days(clock.elapsed_seconds);
        } else {
            current.version = LEGACY_VERSION;
        }
        let description = if current.clock.is_some() {
            "# The exact clock is recorded in seconds. Hand edits are reported by `talkdnd check`."
        } else {
            "# Off by default; only `talkdnd time ...` advances it; hand edits are reported by `talkdnd check`."
        };
        let text = format!(
            "# TalkDND world time: an optional in-world calendar with due-dated events.\n{description}\n{}",
            serde_norway::to_string(&current)?
        );
        crate::fsutil::atomic_write(save.join(FILE), text.as_bytes())
    }

    /// Import the legacy day counter once. The v1 file remains untouched
    /// until the new clock and the caller's update are written together.
    pub fn enable_second_clock(&mut self) -> Result<()> {
        if self.clock.is_some() {
            return Ok(());
        }
        let seconds = (self.elapsed_days * SECONDS_PER_DAY as f64).round();
        if !seconds.is_finite() || seconds < 0.0 || seconds >= u64::MAX as f64 {
            bail!("the legacy world-time counter cannot be converted to campaign seconds");
        }
        let elapsed_seconds = seconds as u64;
        self.clock = Some(CampaignClock {
            elapsed_seconds,
            ..CampaignClock::default()
        });
        self.elapsed_days = as_days(elapsed_seconds);
        self.version = VERSION;
        Ok(())
    }

    pub fn advance_seconds(&mut self, amount: u64) -> Result<()> {
        self.enable_second_clock()?;
        let clock = self.clock.as_mut().expect("second clock is enabled");
        let next = clock
            .elapsed_seconds
            .checked_add(amount)
            .context("campaign seconds overflow")?;
        clock.casting.advance_exploration(amount)?;
        clock.elapsed_seconds = next;
        self.elapsed_days = as_days(clock.elapsed_seconds);
        Ok(())
    }

    pub fn reconcile_combat(
        &mut self,
        battle: &crate::long_cast::EncounterCasting,
        current_round: u32,
    ) -> Result<u64> {
        let clock = self.clock.as_mut().context("campaign has no exact clock")?;
        let mut casting = clock.casting.clone();
        let amount = casting.leave(battle, current_round)?;
        let next = clock
            .elapsed_seconds
            .checked_add(amount)
            .context("campaign seconds overflow")?;
        clock.casting = casting;
        clock.elapsed_seconds = next;
        self.elapsed_days = as_days(next);
        Ok(amount)
    }
}

pub fn as_days(seconds: u64) -> f64 {
    seconds as f64 / SECONDS_PER_DAY as f64
}

/// Taken after SAVE and before encounter/integrity locks. Saves without
/// world time have no casts to protect and keep their existing write paths.
pub fn lock_if_present(save: &Path) -> Result<Option<File>> {
    match fs::metadata(save.join(FILE)) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => bail!("world-time path is not a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("inspect world-time file"),
    }
    let directory = save.join(".cache");
    fs::create_dir_all(&directory)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(".campaign-time.lock"))?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(fs::TryLockError::WouldBlock) if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(fs::TryLockError::WouldBlock) => {
                bail!("campaign clock is busy; no changes were made")
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(error).context("lock campaign clock")
            }
        }
    }
}
