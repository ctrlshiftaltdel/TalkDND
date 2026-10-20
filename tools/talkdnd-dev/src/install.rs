//! Copy a locally built release `combat-sim` into `LetsPlay/bin/`.
//!
//! `--dry-run` prints the cargo and copy commands and does not build.

use std::process::Command;

use crate::fetch::{install_binary, join_rel};

pub const USAGE: &str = "usage: talkdnd-dev install-local [--dry-run]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub build_args: Vec<String>,
    pub source_rel: String,
    pub install_rel: String,
    pub chmod: bool,
}

pub fn run(args: &[String]) -> Result<u8, String> {
    let dry_run = match parse_install_args(args) {
        Ok(dry_run) => dry_run,
        Err(err) => {
            eprintln!("{USAGE}");
            eprintln!("{err}");
            return Ok(2);
        }
    };
    let plan = install_plan(cfg!(windows), cfg!(unix));
    if dry_run {
        print!("{}", dry_run_text(&plan));
        return Ok(0);
    }

    let root = crate::repo_root()?;
    let mut cargo = Command::new("cargo");
    cargo
        .args(plan.build_args.iter().skip(1))
        .current_dir(&root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    let status = cargo
        .status()
        .map_err(|err| format!("failed to run cargo: {err}"))?;
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(format!("cargo build --release failed (exit {code})"));
    }

    let src = join_rel(&root, &plan.source_rel);
    if !src.is_file() {
        return Err(format!("release binary not found at {}", plan.source_rel));
    }
    let dest = join_rel(&root, &plan.install_rel);
    install_binary(&src, &dest, plan.chmod)?;
    println!("installed {}", plan.install_rel);
    println!("local file only; do not commit {}", plan.install_rel);
    Ok(0)
}

pub fn parse_install_args(args: &[String]) -> Result<bool, String> {
    let mut dry_run = false;
    for arg in args {
        if arg == "--dry-run" {
            if dry_run {
                return Err("duplicate --dry-run".to_string());
            }
            dry_run = true;
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("unknown argument `{arg}`"));
        }
        return Err(format!("unexpected argument `{arg}`"));
    }
    Ok(dry_run)
}

/// `host_windows` selects the binary name. `chmod` is the current host, not the name.
pub fn install_plan(host_windows: bool, chmod: bool) -> InstallPlan {
    let binary_name = if host_windows {
        "combat-sim.exe"
    } else {
        "combat-sim"
    };
    InstallPlan {
        build_args: vec![
            "cargo".to_string(),
            "build".to_string(),
            "--release".to_string(),
            "--manifest-path".to_string(),
            "tools/combat-sim/Cargo.toml".to_string(),
        ],
        source_rel: format!("tools/combat-sim/target/release/{binary_name}"),
        install_rel: format!("LetsPlay/bin/{binary_name}"),
        chmod,
    }
}

pub fn dry_run_text(plan: &InstallPlan) -> String {
    let mut text = plan.build_args.join(" ");
    text.push('\n');
    text.push_str("cp ");
    text.push_str(&plan.source_rel);
    text.push(' ');
    text.push_str(&plan.install_rel);
    text.push('\n');
    if plan.chmod {
        text.push_str("chmod u+x ");
        text.push_str(&plan.install_rel);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_dry_run_and_rejects_other_args() {
        assert!(!parse_install_args(&[]).unwrap());
        assert!(parse_install_args(&args(&["--dry-run"])).unwrap());
        assert!(parse_install_args(&args(&["--dry-run", "--dry-run"])).is_err());
        assert!(parse_install_args(&args(&["--nope"])).is_err());
        assert!(parse_install_args(&args(&["extra"])).is_err());
    }

    #[test]
    fn plan_copies_the_host_release_binary_into_letsplay() {
        let unix = install_plan(false, true);
        assert_eq!(
            unix.source_rel,
            "tools/combat-sim/target/release/combat-sim"
        );
        assert_eq!(unix.install_rel, "LetsPlay/bin/combat-sim");
        assert!(unix.chmod);
        assert_eq!(
            dry_run_text(&unix),
            "\
cargo build --release --manifest-path tools/combat-sim/Cargo.toml
cp tools/combat-sim/target/release/combat-sim LetsPlay/bin/combat-sim
chmod u+x LetsPlay/bin/combat-sim
"
        );

        let windows = install_plan(true, false);
        assert_eq!(windows.install_rel, "LetsPlay/bin/combat-sim.exe");
        assert_eq!(
            windows.source_rel,
            "tools/combat-sim/target/release/combat-sim.exe"
        );
        assert!(!windows.chmod);
        let text = dry_run_text(&windows);
        assert!(text.contains(
            "cp tools/combat-sim/target/release/combat-sim.exe LetsPlay/bin/combat-sim.exe\n"
        ));
        assert!(!text.contains("chmod"));
        assert!(!text.contains('\r'));
    }
}
