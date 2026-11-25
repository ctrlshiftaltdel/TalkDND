//! Bounded inner-lock acquire for the new advancement path (ADV-P1-3 / OD-9).
//!
//! The new path acquires the integrity lock with an explicit deadline
//! (default 5 seconds, overridable). A timeout fails closed: no target
//! writes, no SAVE or advancement-state changes, and no half-held inner
//! lock.
//!
//! This module does **not** make the whole commit path bounded. The existing
//! `combat_sim::integrity::lock` API still blocks indefinitely, existing
//! call sites still use it, and publish is not wired. `D_save` is recorded
//! for diagnostics and accounting (total budget = `D_save` + sum of
//! per-stage `D_inner`); it is not a bound on a full commit.

use combat_sim::integrity::{self, ManifestLock};
use std::path::Path;
use std::time::{Duration, Instant};

/// Default inner-lock deadline, and the default recorded `D_save`. The
/// SAVE lock's own acquire still uses `advancement_lock::DEFAULT_ACQUIRE_TIMEOUT`.
pub const DEFAULT_INNER_TIMEOUT: Duration = Duration::from_secs(5);

/// Stage name for the integrity inner lock.
pub const INTEGRITY_STAGE: &str = "integrity";

/// Options for one new-path inner-lock attempt. Every public acquire
/// recomputes a fresh budget from these options; leftover time from a
/// previous attempt is never reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerLockOptions {
    /// Recorded `D_save` for diagnostics. Not consumed by the inner acquire.
    pub d_save: Duration,
    /// Per-stage inner deadlines. The integrity stage is required to acquire.
    pub stages: Vec<InnerStageBudget>,
}

/// One stage's inner-lock budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerStageBudget {
    pub name: String,
    pub d_inner: Duration,
}

impl Default for InnerLockOptions {
    fn default() -> Self {
        Self {
            d_save: DEFAULT_INNER_TIMEOUT,
            stages: vec![InnerStageBudget {
                name: INTEGRITY_STAGE.to_string(),
                d_inner: DEFAULT_INNER_TIMEOUT,
            }],
        }
    }
}

impl InnerLockOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override only the integrity inner deadline, keeping the default
    /// recorded `D_save`.
    pub fn with_integrity_timeout(timeout: Duration) -> Self {
        Self {
            stages: vec![InnerStageBudget {
                name: INTEGRITY_STAGE.to_string(),
                d_inner: timeout,
            }],
            ..Self::default()
        }
    }

    /// A fresh budget taken from these options. Call this at the start of
    /// every attempt, including a retry after a timeout.
    pub fn budget(&self) -> WaitBudget {
        WaitBudget {
            d_save: self.d_save,
            stages: self.stages.clone(),
        }
    }
}

/// Fresh wait budget for a single attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitBudget {
    pub d_save: Duration,
    pub stages: Vec<InnerStageBudget>,
}

impl WaitBudget {
    pub fn total(&self) -> Duration {
        self.d_save
            + self
                .stages
                .iter()
                .map(|stage| stage.d_inner)
                .fold(Duration::ZERO, |acc, d| acc.saturating_add(d))
    }

    pub fn d_inner(&self, name: &str) -> Option<Duration> {
        self.stages
            .iter()
            .find(|stage| stage.name == name)
            .map(|stage| stage.d_inner)
    }
}

/// Diagnostic record of the budgets used for one attempt. This is not a
/// claim that a whole commit path waited only this long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireDiagnostics {
    pub d_save: Duration,
    pub d_inner: Vec<(String, Duration)>,
    pub waited: Duration,
    pub timed_out: bool,
}

/// Fail-closed error from a new-path inner-lock attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerLockError {
    pub message: String,
    pub diagnostics: AcquireDiagnostics,
}

impl std::fmt::Display for InnerLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InnerLockError {}

/// A held inner integrity lock for the new path. Released on drop.
pub struct HeldInnerLock {
    _lock: ManifestLock,
    pub diagnostics: AcquireDiagnostics,
}

impl std::fmt::Debug for HeldInnerLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldInnerLock")
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

fn validate_save(save: &Path) -> Result<(), String> {
    if !save.is_dir() {
        return Err(format!("存档目录不存在或不是目录：{}", save.display()));
    }
    Ok(())
}

fn empty_ok(_: &Path) -> Result<(), String> {
    Ok(())
}

/// Acquire the integrity inner lock for the new path.
///
/// Always recomputes the budget from `opts` (AC-5), always validates the
/// save directory first, then waits at most that attempt's integrity
/// `D_inner`. A previous timeout does not shrink the next budget and does
/// not skip validation.
pub fn acquire_integrity(
    save: &Path,
    opts: &InnerLockOptions,
) -> Result<HeldInnerLock, InnerLockError> {
    acquire_integrity_with(save, opts, empty_ok)
}

/// Like [`acquire_integrity`], with an extra validation callback that runs
/// on **every** attempt after the built-in save-directory check. Tests use
/// this to prove a retry does not skip validation.
pub fn acquire_integrity_with<F>(
    save: &Path,
    opts: &InnerLockOptions,
    validate: F,
) -> Result<HeldInnerLock, InnerLockError>
where
    F: Fn(&Path) -> Result<(), String>,
{
    // Fresh budget from the caller's options — never leftover time.
    let budget = opts.budget();
    let diagnostics = AcquireDiagnostics {
        d_save: budget.d_save,
        d_inner: budget
            .stages
            .iter()
            .map(|stage| (stage.name.clone(), stage.d_inner))
            .collect(),
        waited: Duration::ZERO,
        timed_out: false,
    };

    if let Err(reason) = validate_save(save) {
        return Err(InnerLockError {
            message: format!("内层锁获取前校验失败：{reason}"),
            diagnostics,
        });
    }
    if let Err(reason) = validate(save) {
        return Err(InnerLockError {
            message: format!("内层锁获取前校验失败：{reason}"),
            diagnostics,
        });
    }

    let timeout = budget
        .d_inner(INTEGRITY_STAGE)
        .ok_or_else(|| InnerLockError {
            message: "内层锁获取失败：预算中缺少 integrity 阶段的 D_inner".to_string(),
            diagnostics: diagnostics.clone(),
        })?;

    let start = Instant::now();
    match integrity::lock_with_timeout(save, timeout) {
        Ok(lock) => Ok(HeldInnerLock {
            _lock: lock,
            diagnostics: AcquireDiagnostics {
                waited: start.elapsed(),
                timed_out: false,
                ..diagnostics
            },
        }),
        Err(err) => {
            let waited = start.elapsed();
            let mut message = format!("{err:#}");
            if !message.contains("内层锁") {
                message = format!("内层锁获取失败：{message}");
            }
            let timed_out = message.contains("超时");
            Err(InnerLockError {
                message,
                diagnostics: AcquireDiagnostics {
                    waited,
                    timed_out,
                    ..diagnostics
                },
            })
        }
    }
}
