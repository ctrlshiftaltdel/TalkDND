//! The play folder may be extracted anywhere and renamed to anything: the
//! tools find it from `bin/..`, independent of the working directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_letsplay() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LetsPlay")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            fs::copy(&p, &t).unwrap();
        }
    }
}

/// A minimal package: only shipped (non-book) files plus the binaries.
fn package(base: &Path) -> PathBuf {
    let src = repo_letsplay();
    let play = base.join("My Game 冒险 v2");
    for f in [".talkdnd-root", "AGENTS.md"] {
        fs::create_dir_all(&play).unwrap();
        fs::copy(src.join(f), play.join(f)).unwrap();
    }
    for d in ["Rules/combat", "Rules/defaults", "docs"] {
        copy_dir(&src.join(d), &play.join(d));
    }
    fs::create_dir_all(play.join("Saves")).unwrap();
    fs::create_dir_all(play.join("Adventures")).unwrap();
    fs::create_dir_all(play.join("bin")).unwrap();
    let exe = if cfg!(windows) { ".exe" } else { "" };
    fs::copy(
        env!("CARGO_BIN_EXE_talkdnd"),
        play.join(format!("bin/talkdnd{exe}")),
    )
    .unwrap();
    play
}

fn combat_sim_build() -> Option<PathBuf> {
    let exe = if cfg!(windows) { ".exe" } else { "" };
    ["debug", "release"]
        .iter()
        .map(|p| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../combat-sim/target/{p}/combat-sim{exe}"))
        })
        .find(|p| p.is_file())
}

fn run(play: &Path, cwd: &Path, tool: &str, args: &[&str]) -> String {
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let out = Command::new(play.join(format!("bin/{tool}{exe}")))
        .args(args)
        .current_dir(cwd)
        .env_remove("TALKDND_ROOT")
        .env_remove("COMBAT_SIM_CAMPAIGN")
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{tool} {args:?} failed:\n{text}");
    text
}

#[test]
fn renamed_package_works_from_any_cwd() {
    let base = std::env::temp_dir().join(format!("talkdnd-portable-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let play = package(&base);
    let elsewhere = base.join("somewhere else");
    fs::create_dir_all(&elsewhere).unwrap();
    let combat = combat_sim_build();
    if let Some(cs) = &combat {
        let exe = if cfg!(windows) { ".exe" } else { "" };
        fs::copy(cs, play.join(format!("bin/combat-sim{exe}"))).unwrap();
    }

    run(
        &play,
        &elsewhere,
        "talkdnd",
        &["new", "--ruleset", "2014", "--date", "2030-01-01"],
    );
    let slot = play.join("Saves/2030-01-01-01");
    assert!(slot.join("combat/campaign.yaml").is_file());
    assert!(
        !elsewhere.join("Saves").exists(),
        "nothing may be written to the cwd"
    );

    let status = run(&play, &elsewhere, "talkdnd", &["chargen", "status"]);
    assert!(status.contains("ruleset"), "{status}");
    run(&play, &elsewhere, "talkdnd", &["chargen", "array"]);

    let digest = run(&play, &elsewhere, "talkdnd", &["start", "--no-pdf2md"]);
    assert!(digest.contains("2030-01-01-01"), "{digest}");
    assert!(slot.join("digest.md").is_file());

    if combat.is_some() {
        let check = run(
            &play,
            &elsewhere,
            "combat-sim",
            &[
                "--campaign",
                "Saves/2030-01-01-01/combat/campaign.yaml",
                "content",
                "check",
            ],
        );
        assert!(check.contains("0 error(s)"), "{check}");
        assert!(digest.contains("content check：0 error(s)"), "{digest}");
        run(&play, &elsewhere, "combat-sim", &["roll", "1d20+5"]);
    }

    // `--root` and TALKDND_ROOT still win over everything.
    let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .args(["chargen", "status"])
        .current_dir(&elsewhere)
        .env("TALKDND_ROOT", &play)
        .output()
        .unwrap();
    assert!(out.status.success());
    let _ = fs::remove_dir_all(&base);
}
