//! Only the generated report and its stable fingerprint are sent to GitHub.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Event, Outcome, TOKEN_ENV};

const REPO: &str = "repos/ctrlshiftaltdel/TalkDND/issues";
const MAX_RESPONSE: u64 = 4096;
const DEADLINE: Duration = Duration::from_secs(10);

pub(super) enum Creation {
    Created(u64),
    NotStarted,
    Uncertain,
}

pub(super) trait Transport {
    fn find(&self, marker: &str) -> Result<Option<u64>, ()>;
    fn create(&self, event: Event, marker: &str) -> Creation;
}

pub(super) fn dedicated_token() -> Option<OsString> {
    let token = std::env::var(TOKEN_ENV).ok()?;
    (!token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then(|| token.into())
}

pub(super) fn isolated_process(command: &mut Command) {
    command.env_clear();
    for key in ["PATH", "SYSTEMROOT", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

pub(super) struct Gh {
    executable: PathBuf,
    token: OsString,
    config_dir: PathBuf,
}

impl Gh {
    pub(super) fn new(token: OsString, config_dir: &Path) -> Option<Self> {
        let name = if cfg!(windows) { "gh.exe" } else { "gh" };
        let executable = std::env::split_paths(&std::env::var_os("PATH")?)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())?;
        Some(Self {
            executable,
            token,
            config_dir: config_dir.to_path_buf(),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        isolated_process(&mut command);
        command
            .args(["api", "--hostname", "github.com"])
            .env("GH_TOKEN", &self.token)
            .env("GH_CONFIG_DIR", &self.config_dir)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }
}

impl Transport for Gh {
    fn find(&self, marker: &str) -> Result<Option<u64>, ()> {
        let endpoint = format!("{REPO}?state=all&per_page=100");
        let query = format!(
            ".[] | select(.pull_request == null) | select(((.body // \"\") | split(\"\\n\")[0]) == \"{marker}\") | .number"
        );
        let mut command = self.command();
        command.args(["--method", "GET", "--paginate", &endpoint, "--jq", &query]);
        let response = run(command, None).map_err(|_| ())?;
        parse_numbers(&response).map(|numbers| numbers.into_iter().min())
    }

    fn create(&self, event: Event, marker: &str) -> Creation {
        let Some(body) = request(event, marker) else {
            return Creation::NotStarted;
        };
        let mut command = self.command();
        command.args(["--method", "POST", REPO, "--input", "-", "--jq", ".number"]);
        match run(command, Some(&body)) {
            Ok(response) => match parse_numbers(&response).as_deref() {
                Ok([issue]) => Creation::Created(*issue),
                _ => Creation::Uncertain,
            },
            Err(RequestError::NotStarted) => Creation::NotStarted,
            Err(RequestError::Uncertain) => Creation::Uncertain,
        }
    }
}

fn parse_numbers(response: &[u8]) -> Result<Vec<u64>, ()> {
    std::str::from_utf8(response)
        .map_err(|_| ())?
        .lines()
        .map(|line| {
            line.trim()
                .parse::<u64>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or(())
        })
        .collect()
}

pub(super) fn request(event: Event, marker: &str) -> Option<Vec<u8>> {
    // Re-derive the marker so no other input can reach the issue body.
    if event.marker().as_deref() != Some(marker) {
        return None;
    }
    let stage = event.stage.code();
    let code = event.outcome.report_code(event.stage)?;
    let ruleset = event.ruleset.map_or("unknown", |rules| rules.value());
    let [major, minor, patch] = event.version;
    let count = if event.outcome == Outcome::RollbackFailed {
        "at least one confirmed occurrence"
    } else {
        "at least three occurrences across the two most recent local sessions"
    };
    let platform = event.platform.name();
    serde_json::to_vec(&serde_json::json!({
        "title": format!("[playtest] {stage}: {code}"),
        "body": format!("{marker}\n\nAutomated playtest report\n\n- Version: {major}.{minor}.{patch}\n- Platform: {platform}\n- Ruleset: {ruleset}\n- Stage: {stage}\n- Event: {code}\n- Count: {count}")
    })).ok()
}

enum RequestError {
    NotStarted,
    Uncertain,
}

fn run(mut command: Command, body: Option<&[u8]>) -> Result<Vec<u8>, RequestError> {
    command.stdin(if body.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let deadline = Instant::now() + DEADLINE;
    let mut child = command.spawn().map_err(|_| RequestError::NotStarted)?;
    let stdout = child.stdout.take().expect("stdout piped");
    let (sender, receiver) = std::sync::mpsc::channel();
    if std::thread::Builder::new()
        .spawn(move || {
            let mut output = Vec::new();
            let result = stdout
                .take(MAX_RESPONSE + 1)
                .read_to_end(&mut output)
                .map(|_| output);
            let _ = sender.send(result);
        })
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(RequestError::Uncertain);
    }
    if let Some(body) = body {
        // The generated request is below the pipe buffer and contains only
        // bounded static fields. Close stdin to finish gh's input.
        if child
            .stdin
            .take()
            .expect("stdin piped")
            .write_all(body)
            .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RequestError::Uncertain);
        }
    }
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let output = receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| RequestError::Uncertain)?
                    .map_err(|_| RequestError::Uncertain)?;
                if output.len() <= MAX_RESPONSE as usize {
                    return Ok(output);
                }
                return Err(RequestError::Uncertain);
            }
            Ok(Some(_)) => return Err(RequestError::Uncertain),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RequestError::Uncertain);
            }
        }
    }
}
