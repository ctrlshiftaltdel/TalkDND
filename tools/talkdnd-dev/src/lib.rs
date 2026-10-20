#![forbid(unsafe_code)]

pub mod fetch;
pub mod install;
pub mod leaks;
pub mod letsplay;
pub mod migrate;

use std::path::PathBuf;
use std::process::Command;

/// Repository root from the process working directory.
/// `GIT_DIR` is ignored so a caller inside this repo cannot be pointed elsewhere.
pub fn repo_root() -> Result<PathBuf, String> {
    let output = git()
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|err| format!("git rev-parse: {err}"))?;
    if !output.status.success() {
        return Err(git_failure(
            &output.stderr,
            "git rev-parse --show-toplevel failed",
        ));
    }
    let raw = String::from_utf8(output.stdout)
        .map_err(|_| "git rev-parse returned a non-utf8 path".to_string())?;
    let path = raw.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        return Err("git rev-parse returned an empty path".to_string());
    }
    Ok(PathBuf::from(path))
}

pub(crate) fn git() -> Command {
    let mut cmd = Command::new("git");
    cmd.env_remove("GIT_DIR");
    cmd.env_remove("GIT_WORK_TREE");
    cmd.env_remove("GIT_INDEX_FILE");
    cmd
}

pub(crate) fn git_failure(stderr: &[u8], fallback: &str) -> String {
    let err = String::from_utf8_lossy(stderr);
    let err = err.trim();
    if err.is_empty() {
        fallback.to_string()
    } else {
        err.to_string()
    }
}
