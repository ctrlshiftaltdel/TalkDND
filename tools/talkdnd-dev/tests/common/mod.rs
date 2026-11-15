//! Shared by the integration tests. Each file under `tests/` is its own crate,
//! so a helper used by only one of them is dead code in the others.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct TempRepo {
    pub path: PathBuf,
}

impl TempRepo {
    pub fn new() -> Self {
        let repo = Self::dir_only();
        git(&repo.path, &["init", "-q"]);
        // Windows runners otherwise rewrite text to CRLF or materialize
        // symlinks as plain files. These temp repos must keep the bytes and
        // links the tests wrote.
        git(&repo.path, &["config", "core.autocrlf", "false"]);
        git(&repo.path, &["config", "core.symlinks", "true"]);
        repo
    }

    pub fn dir_only() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("talkdnd-dev-{tick}-{n}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn write(&self, rel: &str, body: &str) {
        self.write_bytes(rel, body.as_bytes());
    }

    pub fn write_bytes(&self, rel: &str, body: &[u8]) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    pub fn symlink(&self, rel: &str, target: &str) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        symlink_to(target, &path);
    }

    // Leak tests stage paths; migrate tests do not.
    pub fn add(&self, rels: &[&str]) {
        let mut args = vec!["add", "-f", "--"];
        args.extend_from_slice(rels);
        git(&self.path, &args);
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn program() -> PathBuf {
    bin()
}

/// Cargo names this variable after the binary target verbatim, so the hyphen
/// in `talkdnd-dev` stays a hyphen: `CARGO_BIN_EXE_TALKDND_DEV` never exists.
const CARGO_BIN_EXE: &str = "CARGO_BIN_EXE_talkdnd-dev";

/// How far above the test executable the fallback looks. Cargo 1.100 puts the
/// executable in `target/<profile>/build/<crate>/<hash>/out/`, older Cargos in
/// `target/<profile>/deps/`; the binary itself stays at `target/<profile>/`.
/// Anything further up is not ours.
const MAX_ANCESTORS: usize = 8;

fn bin() -> PathBuf {
    // Integration tests get the absolute path, including `.exe` on Windows.
    if let Some(path) = option_env!("CARGO_BIN_EXE_talkdnd-dev") {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "{CARGO_BIN_EXE} = {} is not a file",
            path.display()
        );
        return path;
    }
    let exe = std::env::current_exe().unwrap_or_else(|err| panic!("current_exe: {err}"));
    find_program_near(&exe).unwrap_or_else(|| panic!("{}", missing_program_message(&exe)))
}

/// Locate the `talkdnd-dev` binary for a test executable at `exe`. The depth
/// of Cargo's test layout is not assumed: every ancestor up to the `target`
/// directory is checked, so both the `deps/` and the `build/<crate>/<hash>/out/`
/// layouts resolve. A candidate must be an executable file, so a directory or
/// a stray file with the same name is never run as the program under test.
pub fn find_program_near(exe: &Path) -> Option<PathBuf> {
    let name = program_file_name();
    for dir in exe.ancestors().skip(1).take(MAX_ANCESTORS) {
        let candidate = dir.join(&name);
        if is_program(&candidate) {
            return Some(candidate);
        }
        // Do not leave the build tree: a `talkdnd-dev` above `target` belongs
        // to someone else, not to the crate under test.
        if dir.file_name() == Some(std::ffi::OsStr::new("target")) {
            break;
        }
    }
    None
}

/// The failure text used when neither Cargo's variable nor the layout scan
/// finds the binary. Tests assert on it so "not found" stays a loud error.
pub fn missing_program_message(exe: &Path) -> String {
    format!(
        "cannot locate the talkdnd-dev binary for the test executable {}: \
         Cargo did not provide {CARGO_BIN_EXE} and no executable named {} was \
         found up to {MAX_ANCESTORS} levels above it (searched the Cargo 1.100 \
         build/<crate>/<hash>/out layout and the older deps layout)",
        exe.display(),
        program_file_name().to_string_lossy()
    )
}

fn program_file_name() -> std::ffi::OsString {
    let mut name = std::ffi::OsString::from("talkdnd-dev");
    if cfg!(windows) {
        name.push(".exe");
    }
    name
}

fn is_program(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

fn symlink_to(target: &str, path: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(target, path)
        .unwrap_or_else(|err| panic!("symlink {target} -> {}: {err}", path.display()));
}

pub fn run(dir: &Path, args: &[&str]) -> Output {
    let program = bin();
    let output = Command::new(&program)
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|err| panic!("failed to run {}: {err}", program.display()));
    output
}

pub fn text(bytes: &[u8]) -> String {
    // Process output is compared against `\n` fixtures. A Windows pipe can
    // surface `\r\n`; the program itself writes `\n`.
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("\r\n", "\n")
}
