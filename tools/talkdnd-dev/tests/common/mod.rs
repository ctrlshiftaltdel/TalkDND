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

/// Binary target (and directory under `build/`) that Cargo compiles.
const CRATE_NAME: &str = "talkdnd-dev";

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
    program_for(&exe)
}

/// Resolve the program for a test executable, failing loudly when the layout
/// is unknown or the binary is missing. Split out so tests exercise the real
/// not-found branch instead of only the message text.
pub fn program_for(exe: &Path) -> PathBuf {
    find_program_near(exe).unwrap_or_else(|| panic!("{}", missing_program_message(exe)))
}

/// Locate the `talkdnd-dev` binary for a test executable at `exe`.
///
/// Only the two layouts Cargo uses for integration tests are understood, and
/// only `profile/<bin>` inside the profile root that the layout implies is ever
/// considered:
///
/// - `target/<profile>/deps/<test>` (older Cargo): the profile root is the
///   parent of `deps/`.
/// - `target/<profile>/build/talkdnd-dev/<hash>/out/<test>` (Cargo 1.100): the
///   profile root is the parent of that `build/` directory.
///
/// Any other location yields `None`. Scanning every ancestor for a file with
/// the binary's name would run a look-alike Cargo never built, such as
/// `<profile>/build/talkdnd-dev/<hash>/out/talkdnd-dev`, or a binary sitting
/// above a custom target directory.
pub fn find_program_near(exe: &Path) -> Option<PathBuf> {
    let candidate = profile_root(exe)?.join(program_file_name());
    is_program(&candidate).then_some(candidate)
}

/// The Cargo profile directory (`target/<profile>`) implied by a test
/// executable's location, or `None` for a layout this fallback does not know.
fn profile_root(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if dir_named(dir, "deps") {
        return dir.parent().map(Path::to_path_buf);
    }
    if !dir_named(dir, "out") {
        return None;
    }
    let crate_dir = dir.parent()?.parent()?;
    if !dir_named(crate_dir, CRATE_NAME) {
        return None;
    }
    let build = crate_dir.parent()?;
    if !dir_named(build, "build") {
        return None;
    }
    build.parent().map(Path::to_path_buf)
}

fn dir_named(dir: &Path, name: &str) -> bool {
    dir.file_name() == Some(std::ffi::OsStr::new(name))
}

/// The failure text used when neither Cargo's variable nor the layout lookup
/// finds the binary. Tests assert on it so "not found" stays a loud error.
pub fn missing_program_message(exe: &Path) -> String {
    match profile_root(exe) {
        Some(profile) => format!(
            "cannot locate the talkdnd-dev binary for the test executable {}: \
             Cargo did not provide {CARGO_BIN_EXE} and {} is missing or not \
             executable",
            exe.display(),
            profile.join(program_file_name()).display()
        ),
        None => format!(
            "cannot locate the talkdnd-dev binary for the test executable {}: \
             Cargo did not provide {CARGO_BIN_EXE} and its location is not a \
             test layout this fallback understands (target/<profile>/deps/ or \
             target/<profile>/build/talkdnd-dev/<hash>/out/)",
            exe.display()
        ),
    }
}

fn program_file_name() -> std::ffi::OsString {
    let mut name = std::ffi::OsString::from(CRATE_NAME);
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
