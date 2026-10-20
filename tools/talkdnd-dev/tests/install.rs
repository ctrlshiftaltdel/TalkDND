mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{program, text};
use talkdnd_dev::install::{dry_run_text, install_plan};

#[test]
fn dry_run_prints_plan_and_does_not_call_cargo() {
    let fake = FakeCargo::new();
    let path = fake.dir.to_string_lossy().into_owned();
    let output = Command::new(program())
        .args(["install-local", "--dry-run"])
        .env("PATH", &path)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run talkdnd-dev");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let expected = dry_run_text(&install_plan(cfg!(windows), cfg!(unix)));
    assert_eq!(text(&output.stdout), expected);
    assert!(!fake.marker.exists(), "dry-run invoked cargo");
}

#[test]
fn bad_args_exit_two_without_cargo() {
    let fake = FakeCargo::new();
    let path = fake.dir.to_string_lossy().into_owned();
    for args in [
        &["install-local", "--dry-run", "--dry-run"][..],
        &["install-local", "--nope"][..],
        &["install-local", "extra"][..],
    ] {
        let output = Command::new(program())
            .args(args)
            .env("PATH", &path)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run talkdnd-dev");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?} {}",
            text(&output.stderr)
        );
        assert_eq!(text(&output.stdout), "");
        assert!(text(&output.stderr).contains("usage: talkdnd-dev install-local"));
        assert!(!fake.marker.exists(), "{args:?} invoked cargo");
    }
}

struct FakeCargo {
    dir: PathBuf,
    marker: PathBuf,
}

impl FakeCargo {
    fn new() -> Self {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "talkdnd-fake-cargo-{}-{}",
            std::process::id(),
            tick
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("called");
        let script = dir.join(if cfg!(windows) { "cargo.cmd" } else { "cargo" });
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

impl Drop for FakeCargo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
