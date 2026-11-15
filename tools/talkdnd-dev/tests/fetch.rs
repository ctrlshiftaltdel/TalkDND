mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{find_program_near, missing_program_message, program, text, TempRepo};
use talkdnd_dev::fetch::{dry_run_text, fetch_plan_for, host_target, PUBLISHED_TARGETS, TOOLS};

fn tick() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// A `gh` on `PATH` that records every invocation and fails.
struct FakeGh {
    dir: PathBuf,
    marker: PathBuf,
}

impl FakeGh {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("talkdnd-fake-gh-{}-{}", std::process::id(), tick()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("called");
        let script = dir.join(if cfg!(windows) { "gh.cmd" } else { "gh" });
        let marker_text = marker.display().to_string().replace('\'', "");
        if cfg!(windows) {
            fs::write(
                &script,
                format!("@echo called> \"{marker_text}\"\r\nexit /b 99\r\n"),
            )
            .unwrap();
        } else {
            fs::write(
                &script,
                format!("#!/bin/sh\necho called > '{marker_text}'\nexit 99\n"),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&script).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&script, perms).unwrap();
            }
        }
        Self { dir, marker }
    }
}

impl Drop for FakeGh {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn run(args: &[&str], path: &str) -> std::process::Output {
    Command::new(program())
        .args(args)
        .env("PATH", path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run talkdnd-dev")
}

fn all_plans(target: &str) -> String {
    TOOLS
        .iter()
        .map(|t| dry_run_text(&fetch_plan_for(t, target, cfg!(unix)).unwrap()))
        .collect()
}

/// Regression for Cargo 1.100: integration test executables now live under
/// `target/<profile>/build/<crate>/<hash>/out/`, so guessing `deps/` no longer
/// works. `program()` must return the binary Cargo built, not a look-alike.
#[test]
fn program_is_the_cargo_built_binary() {
    let path = program();
    let test_exe = std::env::current_exe().expect("current_exe");
    assert!(
        path.is_file(),
        "program() = {} is not a file (test executable: {})",
        path.display(),
        test_exe.display()
    );
    let name = if cfg!(windows) {
        "talkdnd-dev.exe"
    } else {
        "talkdnd-dev"
    };
    assert_eq!(path.file_name().and_then(|part| part.to_str()), Some(name));
    // Cargo names the variable after the binary target verbatim: the hyphen
    // stays a hyphen. A mangled name silently falls through to layout guessing,
    // so `env!` (not `option_env!`) keeps that contract a compile-time error.
    let cargo = env!("CARGO_BIN_EXE_talkdnd-dev");
    assert_eq!(path, PathBuf::from(cargo));
}

fn bin_name() -> &'static str {
    if cfg!(windows) {
        "talkdnd-dev.exe"
    } else {
        "talkdnd-dev"
    }
}

/// Write a file the fallback accepts as the program: a regular file that, on
/// unix, is executable.
fn write_program(path: &Path) {
    fs::write(path, b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
}

/// Cargo 1.100 moved integration test executables from `target/<profile>/deps/`
/// to `target/<profile>/build/<crate>/<hash>/out/`. The fallback must find the
/// binary from either depth instead of assuming `deps/`.
#[test]
fn fallback_finds_the_binary_from_old_and_new_test_layouts() {
    for layout in ["deps", "build/talkdnd-dev/b7ae5011952c00b5/out"] {
        let repo = TempRepo::dir_only();
        let debug = repo.path.join("target/debug");
        let dir = debug.join(layout);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("fetch-b7ae5011952c00b5");
        fs::write(&exe, b"").unwrap();
        let bin = debug.join(bin_name());
        write_program(&bin);
        assert_eq!(find_program_near(&exe), Some(bin), "layout {layout}");
    }
}

#[test]
fn fallback_never_runs_a_look_alike_directory_or_file() {
    let repo = TempRepo::dir_only();
    let debug = repo.path.join("target/debug");
    let deps = debug.join("deps");
    fs::create_dir_all(&deps).unwrap();
    let exe = deps.join("fetch-b7ae5011952c00b5");
    fs::write(&exe, b"").unwrap();

    // A directory carrying the binary's name must not be picked.
    fs::create_dir_all(debug.join(bin_name())).unwrap();
    assert_eq!(find_program_near(&exe), None);

    // Neither must a non-executable file (Windows has no execute bit).
    fs::remove_dir_all(debug.join(bin_name())).unwrap();
    fs::write(debug.join(bin_name()), b"").unwrap();
    let expected = if cfg!(windows) {
        Some(debug.join(bin_name()))
    } else {
        None
    };
    assert_eq!(find_program_near(&exe), expected);
}

#[test]
fn missing_program_message_names_the_lookup() {
    let exe = PathBuf::from("target/debug/deps/fetch-b7ae5011952c00b5");
    let message = missing_program_message(&exe);
    assert!(message.contains(&exe.display().to_string()), "{message}");
    assert!(message.contains("CARGO_BIN_EXE_talkdnd-dev"), "{message}");
    assert!(message.contains(bin_name()), "{message}");
}

#[test]
fn dry_run_prints_plan_and_does_not_call_gh() {
    let fake = FakeGh::new();
    let path = fake.dir.to_string_lossy().into_owned();
    for target in PUBLISHED_TARGETS {
        let output = run(&["fetch-binary", "--dry-run", "--target", target], &path);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        let expected = all_plans(target);
        assert_eq!(text(&output.stdout), expected);
        assert!(!fake.marker.exists(), "dry-run invoked gh for {target}");
    }
}

#[test]
fn flag_order_and_host_default_select_the_same_plan() {
    let fake = FakeGh::new();
    let path = fake.dir.to_string_lossy().into_owned();
    let target = "x86_64-unknown-linux-gnu";
    let variants = [
        run(&["fetch-binary", "--dry-run", "--target", target], &path),
        run(&["fetch-binary", "--target", target, "--dry-run"], &path),
        run(
            &[
                "fetch-binary",
                "--target=x86_64-unknown-linux-gnu",
                "--dry-run",
            ],
            &path,
        ),
    ];
    let expected = all_plans(target);
    for output in &variants {
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), expected);
    }

    let host = host_target(std::env::consts::OS, std::env::consts::ARCH).unwrap();
    let output = run(&["fetch-binary", "--dry-run"], &path);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), all_plans(host));
    assert!(!fake.marker.exists());
}

#[test]
fn bad_fetch_args_exit_two() {
    let fake = FakeGh::new();
    let path = fake.dir.to_string_lossy().into_owned();
    for args in [
        &["fetch-binary", "--dry-run", "--dry-run"][..],
        &["fetch-binary", "--target"][..],
        &["fetch-binary", "--target="][..],
        &["fetch-binary", "--target", "wasm32-unknown-unknown"][..],
        &["fetch-binary", "extra"][..],
        &["fetch-binary", "--nope"][..],
    ] {
        let output = run(args, &path);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?} {}",
            text(&output.stderr)
        );
        assert_eq!(text(&output.stdout), "");
        assert!(text(&output.stderr).contains("usage: talkdnd-dev fetch-binary"));
        assert!(!fake.marker.exists(), "{args:?} invoked gh");
    }
}

// Windows `Command` also searches the parent process PATH and the application
// and system directories, so a child PATH cannot hide `gh`. Skip this test
// and the git-only PATH helpers it alone uses.
#[cfg(not(windows))]
#[test]
fn missing_gh_fails_before_install() {
    let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../LetsPlay/bin/combat-sim");
    let before_kind = fs::symlink_metadata(&dest)
        .ok()
        .map(|meta| meta.file_type().is_symlink());
    let before_target = fs::read_link(&dest).ok();

    // Runners keep git and gh in the same directory, so a PATH that merely
    // drops gh's directory also drops git. This directory can run git only.
    let git_only = GitWithoutGh::new();
    let output = run(
        &["fetch-binary", "--target", "x86_64-unknown-linux-gnu"],
        &git_only.path(),
    );
    assert_ne!(output.status.code(), Some(0));
    let err = text(&output.stderr);
    assert!(err.contains("failed to run gh"), "{err}");
    assert_eq!(text(&output.stdout), "");

    let after_kind = fs::symlink_metadata(&dest)
        .ok()
        .map(|meta| meta.file_type().is_symlink());
    assert_eq!(before_kind, after_kind);
    assert_eq!(fs::read_link(&dest).ok(), before_target);
}

/// A PATH directory whose only command is a `git` stub. `gh` is absent.
#[cfg(not(windows))]
struct GitWithoutGh {
    dir: PathBuf,
}

#[cfg(not(windows))]
impl GitWithoutGh {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "talkdnd-git-only-{}-{}",
            std::process::id(),
            tick()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        install_git_stub(&dir, &locate_git());
        let path = dir.to_string_lossy().into_owned();
        let git = Command::new("git")
            .arg("--version")
            .env("PATH", &path)
            .output()
            .expect("git on stub PATH");
        assert!(
            git.status.success(),
            "stub PATH lost git: {path} {}",
            text(&git.stderr)
        );
        let gh = Command::new("gh").env("PATH", &path).output();
        assert!(gh.is_err(), "stub PATH can still run gh: {path}");
        Self { dir }
    }

    fn path(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}

#[cfg(not(windows))]
impl Drop for GitWithoutGh {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(not(windows))]
fn locate_git() -> PathBuf {
    let names: &[&str] = if cfg!(windows) {
        &["git.exe", "git.cmd", "git.bat", "git"]
    } else {
        &["git"]
    };
    let raw = std::env::var_os("PATH").expect("PATH");
    for dir in std::env::split_paths(&raw) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("git not found on PATH");
}

#[cfg(not(windows))]
fn install_git_stub(dir: &Path, git: &Path) {
    #[cfg(unix)]
    {
        // A symlink, not a freshly written script: exec of a file another
        // test thread may still hold open for writing fails with ETXTBSY.
        std::os::unix::fs::symlink(git, dir.join("git")).unwrap();
    }
    #[cfg(windows)]
    {
        let script = dir.join("git.cmd");
        let windows = git.to_string_lossy().replace('"', "");
        fs::write(&script, format!("@echo off\r\n\"{windows}\" %*\r\n")).unwrap();
    }
}
