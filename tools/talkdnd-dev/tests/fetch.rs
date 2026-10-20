mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{program, text};
use talkdnd_dev::fetch::{dry_run_text, fetch_plan, host_target, PUBLISHED_TARGETS};

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

#[test]
fn dry_run_prints_plan_and_does_not_call_gh() {
    let fake = FakeGh::new();
    let path = fake.dir.to_string_lossy().into_owned();
    for target in PUBLISHED_TARGETS {
        let output = run(&["fetch-binary", "--dry-run", "--target", target], &path);
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        let expected = dry_run_text(&fetch_plan(target, cfg!(unix)).unwrap());
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
    let expected = dry_run_text(&fetch_plan(target, cfg!(unix)).unwrap());
    for output in &variants {
        assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), expected);
    }

    let host = host_target(std::env::consts::OS, std::env::consts::ARCH).unwrap();
    let output = run(&["fetch-binary", "--dry-run"], &path);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        dry_run_text(&fetch_plan(host, cfg!(unix)).unwrap())
    );
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

#[test]
fn missing_gh_fails_before_install() {
    let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../LetsPlay/bin/combat-sim");
    let before_kind = fs::symlink_metadata(&dest)
        .ok()
        .map(|meta| meta.file_type().is_symlink());
    let before_target = fs::read_link(&dest).ok();

    let path = path_with_git_without_gh();
    let output = run(
        &["fetch-binary", "--target", "x86_64-unknown-linux-gnu"],
        &path,
    );
    assert_ne!(output.status.code(), Some(0));
    let err = text(&output.stderr);
    assert!(err.contains("gh"), "{err}");
    assert_eq!(text(&output.stdout), "");

    let after_kind = fs::symlink_metadata(&dest)
        .ok()
        .map(|meta| meta.file_type().is_symlink());
    assert_eq!(before_kind, after_kind);
    assert_eq!(fs::read_link(&dest).ok(), before_target);
}

fn path_with_git_without_gh() -> String {
    let raw = std::env::var_os("PATH").unwrap_or_default();
    let mut kept = Vec::new();
    for dir in std::env::split_paths(&raw) {
        let gh = if cfg!(windows) {
            dir.join("gh.exe")
        } else {
            dir.join("gh")
        };
        let gh_cmd = dir.join("gh.cmd");
        if !gh.exists() && !gh_cmd.exists() {
            kept.push(dir);
        }
    }
    let joined = std::env::join_paths(kept).expect("PATH");
    let path = joined.to_string_lossy().into_owned();
    let git = Command::new("git")
        .arg("--version")
        .env("PATH", &path)
        .output()
        .expect("git on filtered PATH");
    assert!(git.status.success(), "filtered PATH lost git: {path}");
    path
}
