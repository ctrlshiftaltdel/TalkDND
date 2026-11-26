//! Optional, host-local pilot feedback. Only typed outcomes cross the report boundary.

mod github;

use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::content::loader::Ruleset;
use crate::fsutil::atomic_write;

const ENABLE: &str = "TALKDND_PLAYTEST_FEEDBACK";
const PUBLISH_ROOT: &str = "TALKDND_PLAYTEST_PUBLISH_ROOT";
pub const TOKEN_ENV: &str = "TALKDND_PLAYTEST_GH_TOKEN";
const WORKER: &str = "--talkdnd-playtest-feedback-worker";
const FORMAT: u8 = 1;
const THRESHOLD: u64 = 3;
const MAX_STATE_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    NewGame,
    Resume,
    CheckpointSnapshot,
    CheckpointRestore,
    Reaction,
}

impl Stage {
    fn code(self) -> &'static str {
        match self {
            Self::NewGame => "new_game",
            Self::Resume => "resume",
            Self::CheckpointSnapshot => "checkpoint_snapshot",
            Self::CheckpointRestore => "checkpoint_restore",
            Self::Reaction => "reaction",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Waiting,
    Refused,
    Unclassified,
    StorageFailure,
    LockUnavailable,
    RollbackFailed,
}

impl Outcome {
    fn report_code(self, stage: Stage) -> Option<&'static str> {
        match self {
            Self::StorageFailure => Some("storage_failure"),
            Self::LockUnavailable => Some("lock_unavailable"),
            Self::RollbackFailed if stage == Stage::CheckpointRestore => Some("rollback_failed"),
            _ => None,
        }
    }
}

/// Keep the original diagnostic for the player CLI. The recorder reads the
/// classification alone, even when the error is wrapped with ordinary context.
#[derive(Debug)]
struct ClassifiedError {
    outcome: Outcome,
    message: String,
}

impl fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ClassifiedError {}

pub fn classified_error(outcome: Outcome, message: String) -> anyhow::Error {
    ClassifiedError { outcome, message }.into()
}

pub fn classify(result: &anyhow::Result<()>, waiting: bool) -> Outcome {
    match result {
        Ok(()) if waiting => Outcome::Waiting,
        Ok(()) => Outcome::Completed,
        Err(error) => {
            if let Some(classified) = error.downcast_ref::<ClassifiedError>() {
                return classified.outcome;
            }
            match error.downcast_ref::<io::Error>().map(io::Error::kind) {
                Some(
                    io::ErrorKind::PermissionDenied
                    | io::ErrorKind::ReadOnlyFilesystem
                    | io::ErrorKind::StorageFull
                    | io::ErrorKind::QuotaExceeded,
                ) => Outcome::StorageFailure,
                _ => Outcome::Unclassified,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Platform {
    Linux,
    Macos,
    Windows,
    Other,
}

impl Platform {
    fn current() -> Self {
        match std::env::consts::OS {
            "linux" => Self::Linux,
            "macos" => Self::Macos,
            "windows" => Self::Windows,
            _ => Self::Other,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Elapsed {
    UnderSecond,
    UnderTenSeconds,
    TenSecondsOrMore,
}

impl From<Duration> for Elapsed {
    fn from(elapsed: Duration) -> Self {
        match elapsed.as_secs() {
            0 => Self::UnderSecond,
            1..=9 => Self::UnderTenSeconds,
            _ => Self::TenSecondsOrMore,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    stage: Stage,
    outcome: Outcome,
    ruleset: Option<Ruleset>,
    platform: Platform,
    version: [u16; 3],
    elapsed: Elapsed,
}

impl Event {
    fn new(stage: Stage, outcome: Outcome, ruleset: Option<Ruleset>, elapsed: Duration) -> Self {
        Self {
            stage,
            outcome,
            ruleset,
            platform: Platform::current(),
            elapsed: elapsed.into(),
            version: [
                env!("CARGO_PKG_VERSION_MAJOR"),
                env!("CARGO_PKG_VERSION_MINOR"),
                env!("CARGO_PKG_VERSION_PATCH"),
            ]
            .map(|part| part.parse().expect("numeric package version")),
        }
    }

    fn matching(self, other: Self) -> bool {
        self.stage == other.stage && self.outcome == other.outcome
    }

    fn marker(self) -> Option<String> {
        let code = self.outcome.report_code(self.stage)?;
        Some(format!(
            "<!-- talkdnd-playtest:v1:{}:{code} -->",
            self.stage.code()
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Delivery {
    Ready,
    Uncertain,
    Linked(u64),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    event: Event,
    delivery: Delivery,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Count {
    event: Event,
    count: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueState {
    format: u8,
    totals: Vec<Count>,
    recent: [Vec<Count>; 2],
    reports: Vec<Report>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            format: FORMAT,
            totals: Vec::new(),
            recent: [Vec::new(), Vec::new()],
            reports: Vec::new(),
        }
    }
}

impl QueueState {
    fn record(&mut self, event: Event) {
        if event.outcome == Outcome::Completed
            && matches!(event.stage, Stage::NewGame | Stage::Resume)
        {
            self.recent.rotate_right(1);
            self.recent[0].clear();
        }
        for counts in [&mut self.totals, &mut self.recent[0]] {
            if let Some(prior) = counts.iter_mut().find(|count| count.event == event) {
                prior.count = prior.count.saturating_add(1);
            } else {
                counts.push(Count { event, count: 1 });
            }
        }
        if event.outcome.report_code(event.stage).is_none()
            || self
                .reports
                .iter()
                .any(|report| report.event.matching(event))
        {
            return;
        }
        let occurrences = self
            .recent
            .iter()
            .flatten()
            .filter(|count| count.event.matching(event))
            .fold(0u64, |total, count| total.saturating_add(count.count));
        if event.outcome == Outcome::RollbackFailed || occurrences >= THRESHOLD {
            self.reports.push(Report {
                event,
                delivery: Delivery::Ready,
            });
        }
    }

    fn has_work(&self) -> bool {
        self.reports
            .iter()
            .any(|report| !matches!(report.delivery, Delivery::Linked(_)))
    }
}

struct Queue(PathBuf);

impl Queue {
    fn open(root: &Path) -> io::Result<Self> {
        if !advancement_lock::is_play_root(root) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let control = root.join(".talkdnd");
        let queue = control.join("playtest-feedback");
        for path in [&control, &queue] {
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_dir() || is_link(&metadata) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
        }
        Ok(Self(queue))
    }

    fn lock(&self, name: &str, attempts: usize) -> anyhow::Result<File> {
        let path = self.0.join(name);
        safe_existing_file(&path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        for index in 0..attempts {
            match lock.try_lock() {
                Ok(()) => return Ok(lock),
                Err(fs::TryLockError::WouldBlock) if index + 1 < attempts => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => return Err(error.into()),
            }
        }
        unreachable!("at least one lock attempt")
    }

    fn read(&self) -> anyhow::Result<QueueState> {
        let path = self.0.join("state.json");
        if !safe_existing_file(&path)? {
            return Ok(QueueState::default());
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_STATE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_STATE_BYTES,
            "playtest queue exceeds limit"
        );
        let state: QueueState = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(state.format == FORMAT, "unknown playtest queue format");
        Ok(state)
    }

    fn update<T>(&self, apply: impl FnOnce(&mut QueueState) -> T) -> anyhow::Result<T> {
        let _lock = self.lock("record.lock", 5)?;
        let mut state = self.read()?;
        let result = apply(&mut state);
        let bytes = serde_json::to_vec(&state)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_STATE_BYTES,
            "playtest queue exceeds limit"
        );
        atomic_write(self.0.join("state.json"), bytes)?;
        Ok(result)
    }
}

fn safe_existing_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !is_link(&metadata) => Ok(true),
        Ok(_) => Err(io::ErrorKind::InvalidInput.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn enabled() -> bool {
    std::env::var_os(ENABLE).as_deref() == Some(OsStr::new("1"))
}

/// Read only the declared ruleset. An uncertain or unreadable selection is
/// represented by the fixed unknown value, without preserving the input.
pub fn ruleset_for_save(save: &Path) -> Option<Ruleset> {
    #[derive(Deserialize)]
    struct Manifest {
        ruleset: Option<String>,
    }
    let campaign = File::open(save.join("combat/campaign.yaml")).ok()?;
    let manifest: Manifest = serde_norway::from_reader(campaign.take(64 * 1024)).ok()?;
    match manifest.ruleset {
        Some(value) => Ruleset::parse(&value).ok(),
        None => Some(Ruleset::FivePointFiveESrd),
    }
}

/// Run after the gameplay command and its locks have all returned. Feedback
/// failure is local and never replaces the gameplay result or output.
pub fn observe(
    root: &Path,
    stage: Stage,
    outcome: Outcome,
    ruleset: Option<Ruleset>,
    elapsed: Duration,
) {
    if !enabled() {
        return;
    }
    let Ok(queue) = Queue::open(root) else { return };
    let event = Event::new(stage, outcome, ruleset, elapsed);
    if !queue
        .update(|state| {
            state.record(event);
            state.has_work()
        })
        .unwrap_or(false)
    {
        return;
    }
    let Some(configured) = std::env::var_os(PUBLISH_ROOT) else {
        return;
    };
    let Some(bound_root) = publishing_root(root, Path::new(&configured)) else {
        return;
    };
    let (Some(token), Ok(executable)) = (github::dedicated_token(), std::env::current_exe()) else {
        return;
    };
    let mut command = std::process::Command::new(executable);
    github::isolated_process(&mut command);
    command
        .arg(WORKER)
        .current_dir(&bound_root)
        .env(ENABLE, "1")
        .env(PUBLISH_ROOT, &bound_root)
        .env(TOKEN_ENV, token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Ok(mut worker) = command.spawn() {
        // Reap when a host invokes the library in a long-lived process.
        let _ = std::thread::Builder::new().spawn(move || worker.wait());
    }
}

/// The two standalone binaries share the hidden worker entry point.
pub fn worker_if_requested() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(WORKER)) || args.next().is_some() {
        return false;
    }
    if enabled() {
        if let (Ok(root), Some(configured)) =
            (std::env::current_dir(), std::env::var_os(PUBLISH_ROOT))
        {
            run_publisher(&root, Path::new(&configured), |queue| {
                github::dedicated_token().and_then(|token| github::Gh::new(token, &queue.0))
            });
        }
    }
    true
}

/// A pilot has one canonical publishing root. Another installation on the
/// same host can record locally, but never uses this root's publisher identity.
fn publishing_root(root: &Path, configured: &Path) -> Option<PathBuf> {
    if !configured.is_absolute() {
        return None;
    }
    let active = root.canonicalize().ok()?;
    let allowed = configured.canonicalize().ok()?;
    (active == allowed && advancement_lock::is_play_root(&allowed)).then_some(allowed)
}

fn run_publisher<T: github::Transport>(
    root: &Path,
    configured: &Path,
    make_transport: impl FnOnce(&Queue) -> Option<T>,
) {
    let Some(allowed) = publishing_root(root, configured) else {
        return;
    };
    let Ok(queue) = Queue::open(&allowed) else {
        return;
    };
    if let Some(transport) = make_transport(&queue) {
        let _ = deliver(&queue, &transport);
    }
}

fn deliver(queue: &Queue, transport: &impl github::Transport) -> anyhow::Result<()> {
    // Separate from record.lock so gameplay can keep recording during requests.
    let _worker = queue.lock("worker.lock", 1)?;
    let reports = {
        let _record = queue.lock("record.lock", 5)?;
        queue.read()?.reports
    };
    for report in reports {
        if matches!(report.delivery, Delivery::Linked(_)) {
            continue;
        }
        let Some(marker) = report.event.marker() else {
            continue;
        };
        match transport.find(&marker) {
            Ok(Some(issue)) => set_delivery(queue, report.event, Delivery::Linked(issue))?,
            Ok(None) if report.delivery == Delivery::Ready => {
                // Persist before starting a POST. After an ambiguous result or
                // a crash, only a lookup can settle this fingerprint.
                set_delivery(queue, report.event, Delivery::Uncertain)?;
                match transport.create(report.event, &marker) {
                    github::Creation::Created(issue) => {
                        set_delivery(queue, report.event, Delivery::Linked(issue))?
                    }
                    github::Creation::NotStarted => {
                        set_delivery(queue, report.event, Delivery::Ready)?
                    }
                    github::Creation::Uncertain => {}
                }
            }
            Ok(None) => {}
            Err(()) => return Ok(()), // pre-POST failure, retry on a future observation
        }
    }
    Ok(())
}

fn set_delivery(queue: &Queue, event: Event, delivery: Delivery) -> anyhow::Result<()> {
    queue.update(|state| {
        if let Some(report) = state
            .reports
            .iter_mut()
            .find(|report| report.event.matching(event))
        {
            report.delivery = delivery;
        }
    })
}

#[cfg(test)]
mod tests;
