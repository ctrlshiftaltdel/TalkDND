//! Download release `combat-sim` and `talkdnd` from the latest successful `build-artifacts`
//! run on `main` and install it at `LetsPlay/bin/`.
//!
//! The command shells out to `gh` (`run list`, then `run download`). `--dry-run`
//! prints those commands and does not use the network.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

pub const USAGE: &str = "usage: talkdnd-dev fetch-binary [--dry-run] [--target <triple>]";

/// Triples published by `.github/workflows/build-artifacts.yml`.
pub const PUBLISHED_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
];

const WORKFLOW_FILE: &str = "build-artifacts.yml";
const RUN_ID_PLACEHOLDER: &str = "<run-id>";
const DOWNLOAD_DIR_PLACEHOLDER: &str = "<download-dir>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchArgs {
    pub target: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    pub target: String,
    pub artifact: String,
    pub binary_name: String,
    pub install_rel: String,
    pub list_args: Vec<String>,
    pub download_args: Vec<String>,
    /// Set the executable bit after install. True on unix hosts.
    pub chmod: bool,
}

pub fn run(args: &[String]) -> Result<u8, String> {
    let parsed = match parse_fetch_args(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{USAGE}");
            eprintln!("{err}");
            return Ok(2);
        }
    };
    let target = match resolve_target(
        parsed.target.as_deref(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ) {
        Ok(target) => target,
        Err(err) => {
            eprintln!("{USAGE}");
            eprintln!("{err}");
            return Ok(2);
        }
    };
    let mut plans = Vec::new();
    for tool in TOOLS {
        plans.push(fetch_plan_for(tool, &target, cfg!(unix))?);
    }
    if parsed.dry_run {
        for plan in &plans {
            print!("{}", dry_run_text(plan));
        }
        return Ok(0);
    }
    for plan in &plans {
        fetch_one(plan)?;
    }
    Ok(0)
}

fn fetch_one(plan: &FetchPlan) -> Result<(), String> {
    let root = crate::repo_root()?;
    let listed = run_gh(&root, &plan.list_args)?;
    if !listed.status.success() {
        return Err(command_failure("gh run list", &listed));
    }
    let listed_stdout = String::from_utf8(listed.stdout)
        .map_err(|_| "gh run list returned non-utf8 output".to_string())?;
    let run_id = parse_run_id(&listed_stdout)?;

    let temp = TempDownload::new()?;
    let download_args = concrete_download_args(plan, run_id, &temp.path);
    let downloaded = run_gh(&root, &download_args)?;
    if !downloaded.status.success() {
        return Err(command_failure("gh run download", &downloaded));
    }

    let src = find_downloaded_binary(&temp.path, &plan.binary_name, &plan.artifact)?;
    let dest = join_rel(&root, &plan.install_rel);
    install_binary(&src, &dest, plan.chmod)?;
    println!(
        "installed {} from build-artifacts run {run_id} ({})",
        plan.install_rel, plan.target
    );
    println!("local file only; do not commit {}", plan.install_rel);
    Ok(())
}

pub fn parse_fetch_args(args: &[String]) -> Result<FetchArgs, String> {
    let mut target = None;
    let mut dry_run = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            return Err("unexpected argument `--`".to_string());
        }
        if arg == "--dry-run" {
            if dry_run {
                return Err("duplicate --dry-run".to_string());
            }
            dry_run = true;
            continue;
        }
        let value = if arg == "--target" {
            iter.next()
                .map(String::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "missing value for --target".to_string())?
        } else if let Some(value) = arg.strip_prefix("--target=") {
            if value.is_empty() {
                return Err("missing value for --target".to_string());
            }
            value
        } else if arg.starts_with('-') {
            return Err(format!("unknown argument `{arg}`"));
        } else {
            return Err(format!("unexpected argument `{arg}`"));
        };
        if target.is_some() {
            return Err("duplicate --target".to_string());
        }
        target = Some(value.to_string());
    }
    Ok(FetchArgs { target, dry_run })
}

/// Map an OS/architecture pair onto a published triple.
pub fn host_target(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        _ => Err(format!(
            "no prebuilt combat-sim for {os}/{arch}; pass --target or build from source"
        )),
    }
}

pub fn resolve_target(explicit: Option<&str>, os: &str, arch: &str) -> Result<String, String> {
    let target = match explicit {
        Some(target) => target.to_string(),
        None => host_target(os, arch)?.to_string(),
    };
    validate_target(&target)?;
    Ok(target)
}

pub fn validate_target(target: &str) -> Result<(), String> {
    if PUBLISHED_TARGETS.contains(&target) {
        Ok(())
    } else {
        Err(format!(
            "unsupported target `{target}`; expected one of {}",
            PUBLISHED_TARGETS.join(", ")
        ))
    }
}

/// Programs installed into `LetsPlay/bin/`. Each has artifact `<tool>-<target>`.
pub const TOOLS: &[&str] = &["combat-sim", "talkdnd"];

pub fn binary_name(target: &str) -> &'static str {
    if target.contains("windows") {
        "combat-sim.exe"
    } else {
        "combat-sim"
    }
}

pub fn fetch_plan(target: &str, host_unix: bool) -> Result<FetchPlan, String> {
    fetch_plan_for("combat-sim", target, host_unix)
}

pub fn fetch_plan_for(tool: &str, target: &str, host_unix: bool) -> Result<FetchPlan, String> {
    validate_target(target)?;
    let binary_name = if target.contains("windows") {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    };
    let artifact = format!("{tool}-{target}");
    let install_rel = format!("LetsPlay/bin/{binary_name}");
    let list_args = vec![
        "gh".to_string(),
        "run".to_string(),
        "list".to_string(),
        "--workflow".to_string(),
        WORKFLOW_FILE.to_string(),
        "--branch".to_string(),
        "main".to_string(),
        "--status".to_string(),
        "success".to_string(),
        "--limit".to_string(),
        "1".to_string(),
        "--json".to_string(),
        "databaseId".to_string(),
    ];
    let download_args = vec![
        "gh".to_string(),
        "run".to_string(),
        "download".to_string(),
        RUN_ID_PLACEHOLDER.to_string(),
        "--name".to_string(),
        artifact.clone(),
        "--dir".to_string(),
        DOWNLOAD_DIR_PLACEHOLDER.to_string(),
    ];
    Ok(FetchPlan {
        target: target.to_string(),
        artifact,
        binary_name,
        install_rel,
        list_args,
        download_args,
        chmod: host_unix,
    })
}

pub fn dry_run_text(plan: &FetchPlan) -> String {
    let mut text = String::new();
    push_command(&mut text, &plan.list_args);
    push_command(&mut text, &plan.download_args);
    text.push_str("cp ");
    text.push_str(DOWNLOAD_DIR_PLACEHOLDER);
    text.push('/');
    text.push_str(&plan.binary_name);
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

/// Read the newest successful run id from `gh run list --json databaseId`.
pub fn parse_run_id(json: &str) -> Result<u64, String> {
    let compact: String = json.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() || compact == "[]" {
        return Err("no successful build-artifacts run on main".to_string());
    }
    const KEY: &str = "\"databaseId\":";
    let Some(pos) = compact.find(KEY) else {
        return Err("gh run list JSON has no databaseId".to_string());
    };
    let rest = &compact[pos + KEY.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err("gh run list databaseId was not a number".to_string());
    }
    let id: u64 = digits
        .parse()
        .map_err(|_| "databaseId does not fit in u64".to_string())?;
    if id == 0 {
        return Err("gh run list databaseId was 0".to_string());
    }
    Ok(id)
}

pub fn find_downloaded_binary(
    dir: &Path,
    binary_name: &str,
    artifact: &str,
) -> Result<PathBuf, String> {
    let direct = dir.join(binary_name);
    if direct.is_file() {
        return Ok(direct);
    }
    let nested = dir.join(artifact).join(binary_name);
    if nested.is_file() {
        return Ok(nested);
    }
    let entries =
        fs::read_dir(dir).map_err(|err| format!("read download dir {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read download dir {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_file() && file_name_is(&path, binary_name) {
            return Ok(path);
        }
        if path.is_dir() {
            let child = path.join(binary_name);
            if child.is_file() {
                return Ok(child);
            }
        }
    }
    Err(format!("downloaded artifact did not contain {binary_name}"))
}

pub fn install_binary(src: &Path, dest: &Path, chmod: bool) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    match fs::symlink_metadata(dest) {
        Ok(_) => {
            fs::remove_file(dest).map_err(|err| format!("remove {}: {err}", dest.display()))?;
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(format!("stat {}: {err}", dest.display())),
    }
    fs::copy(src, dest).map_err(|err| format!("copy to {}: {err}", dest.display()))?;
    if chmod {
        set_executable(dest)?;
    }
    Ok(())
}

pub fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}

fn concrete_download_args(plan: &FetchPlan, run_id: u64, dir: &Path) -> Vec<String> {
    plan.download_args
        .iter()
        .map(|arg| {
            if arg == RUN_ID_PLACEHOLDER {
                run_id.to_string()
            } else if arg == DOWNLOAD_DIR_PLACEHOLDER {
                dir.display().to_string()
            } else {
                arg.clone()
            }
        })
        .collect()
}

fn push_command(text: &mut String, args: &[String]) {
    text.push_str(&args.join(" "));
    text.push('\n');
}

fn file_name_is(path: &Path, name: &str) -> bool {
    path.file_name().and_then(|file_name| file_name.to_str()) == Some(name)
}

fn run_gh(root: &Path, args: &[String]) -> Result<Output, String> {
    let (program, gh_args) = args
        .split_first()
        .ok_or_else(|| "empty gh command".to_string())?;
    Command::new(program)
        .args(gh_args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .map_err(|err| format!("failed to run gh: {err}"))
}

fn command_failure(what: &str, output: &Output) -> String {
    let err = String::from_utf8_lossy(&output.stderr);
    let err = err.trim();
    let code = output.status.code().unwrap_or(-1);
    if err.is_empty() {
        format!("{what} failed (exit {code})")
    } else {
        let mut msg = err.to_string();
        const LIMIT: usize = 400;
        if msg.len() > LIMIT {
            msg.truncate(LIMIT);
        }
        format!("{what} failed: {msg}")
    }
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|err| format!("stat {}: {err}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|err| format!("chmod {}: {err}", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("executable bit is only set on unix".to_string())
    }
}

struct TempDownload {
    path: PathBuf,
}

impl TempDownload {
    fn new() -> Result<Self, String> {
        let tick = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before the Unix epoch".to_string())?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("talkdnd-fetch-{tick}-{}", std::process::id()));
        fs::create_dir_all(&path).map_err(|err| format!("create {}: {err}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDownload {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_flags_in_either_order_and_rejects_bad_input() {
        let parsed =
            parse_fetch_args(&args(&["--dry-run", "--target", "aarch64-apple-darwin"])).unwrap();
        assert!(parsed.dry_run);
        assert_eq!(parsed.target.as_deref(), Some("aarch64-apple-darwin"));

        let parsed =
            parse_fetch_args(&args(&["--target=x86_64-pc-windows-msvc", "--dry-run"])).unwrap();
        assert!(parsed.dry_run);
        assert_eq!(parsed.target.as_deref(), Some("x86_64-pc-windows-msvc"));

        let parsed = parse_fetch_args(&[]).unwrap();
        assert!(!parsed.dry_run);
        assert_eq!(parsed.target, None);

        for bad in [
            vec!["--dry-run", "--dry-run"],
            vec!["--target"],
            vec!["--target="],
            vec!["--target", "linux", "--target", "linux"],
            vec!["extra"],
            vec!["--nope"],
            vec!["--"],
        ] {
            assert!(parse_fetch_args(&args(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn host_target_covers_published_platforms_only() {
        assert_eq!(
            host_target("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            host_target("windows", "x86_64").unwrap(),
            "x86_64-pc-windows-msvc"
        );
        assert_eq!(
            host_target("macos", "aarch64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert!(host_target("linux", "aarch64").is_err());
        assert!(host_target("macos", "x86_64").is_err());
        assert!(host_target("windows", "aarch64").is_err());
    }

    #[test]
    fn explicit_target_wins_and_unknown_triples_are_rejected() {
        assert_eq!(
            resolve_target(Some("aarch64-apple-darwin"), "linux", "x86_64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            resolve_target(None, "windows", "x86_64").unwrap(),
            "x86_64-pc-windows-msvc"
        );
        assert!(resolve_target(Some("wasm32-unknown-unknown"), "linux", "x86_64").is_err());
        assert!(resolve_target(None, "linux", "aarch64").is_err());
    }

    #[test]
    fn plan_selects_binary_name_artifact_and_chmod_from_host() {
        let linux = fetch_plan("x86_64-unknown-linux-gnu", true).unwrap();
        assert_eq!(linux.binary_name, "combat-sim");
        assert_eq!(linux.artifact, "combat-sim-x86_64-unknown-linux-gnu");
        assert_eq!(linux.install_rel, "LetsPlay/bin/combat-sim");
        assert!(linux.chmod);
        assert_eq!(
            linux.list_args,
            args(&[
                "gh",
                "run",
                "list",
                "--workflow",
                "build-artifacts.yml",
                "--branch",
                "main",
                "--status",
                "success",
                "--limit",
                "1",
                "--json",
                "databaseId",
            ])
        );
        assert_eq!(
            linux.download_args,
            args(&[
                "gh",
                "run",
                "download",
                "<run-id>",
                "--name",
                "combat-sim-x86_64-unknown-linux-gnu",
                "--dir",
                "<download-dir>",
            ])
        );

        let windows = fetch_plan("x86_64-pc-windows-msvc", false).unwrap();
        assert_eq!(windows.binary_name, "combat-sim.exe");
        assert_eq!(windows.install_rel, "LetsPlay/bin/combat-sim.exe");
        assert!(!windows.chmod);
        assert!(!windows.install_rel.contains('\\'));

        let macos = fetch_plan("aarch64-apple-darwin", true).unwrap();
        assert_eq!(macos.binary_name, "combat-sim");
        assert!(macos.chmod);

        let text = dry_run_text(&linux);
        assert!(!text.contains('\r'));
        assert_eq!(
            text,
            "\
gh run list --workflow build-artifacts.yml --branch main --status success --limit 1 --json databaseId
gh run download <run-id> --name combat-sim-x86_64-unknown-linux-gnu --dir <download-dir>
cp <download-dir>/combat-sim LetsPlay/bin/combat-sim
chmod u+x LetsPlay/bin/combat-sim
"
        );
        let windows_text = dry_run_text(&windows);
        assert!(
            windows_text.contains("cp <download-dir>/combat-sim.exe LetsPlay/bin/combat-sim.exe\n")
        );
        assert!(!windows_text.contains("chmod"));
    }

    #[test]
    fn parse_run_id_reads_the_first_database_id() {
        assert_eq!(parse_run_id(r#"[{"databaseId":42}]"#).unwrap(), 42);
        assert_eq!(
            parse_run_id("[\n  {\n    \"databaseId\": 99\n  }\n]\n").unwrap(),
            99
        );
        assert!(parse_run_id("[]").is_err());
        assert!(parse_run_id("[\n]\n").is_err());
        assert!(parse_run_id("").is_err());
        assert!(parse_run_id(r#"[{"conclusion":"success"}]"#).is_err());
        assert!(parse_run_id(r#"[{"databaseId":0}]"#).is_err());
        assert!(parse_run_id(r#"[{"databaseId":"nope"}]"#).is_err());
    }

    #[test]
    fn find_binary_accepts_flat_and_nested_layouts() {
        let root = std::env::temp_dir().join(format!(
            "talkdnd-fetch-find-{}-{}",
            std::process::id(),
            unique_tick()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("combat-sim-x86_64-unknown-linux-gnu")).unwrap();
        fs::write(root.join("combat-sim"), b"flat").unwrap();
        let found =
            find_downloaded_binary(&root, "combat-sim", "combat-sim-x86_64-unknown-linux-gnu")
                .unwrap();
        assert_eq!(fs::read(found).unwrap(), b"flat");

        fs::remove_file(root.join("combat-sim")).unwrap();
        let nested = root.join("combat-sim-x86_64-unknown-linux-gnu");
        fs::write(nested.join("combat-sim"), b"nested").unwrap();
        let found =
            find_downloaded_binary(&root, "combat-sim", "combat-sim-x86_64-unknown-linux-gnu")
                .unwrap();
        assert_eq!(fs::read(found).unwrap(), b"nested");

        fs::remove_file(nested.join("combat-sim")).unwrap();
        assert!(find_downloaded_binary(&root, "combat-sim", "missing").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_copies_over_a_regular_file() {
        let root = std::env::temp_dir().join(format!(
            "talkdnd-fetch-copy-{}-{}",
            std::process::id(),
            unique_tick()
        ));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        let dest = root.join("tools/bin/combat-sim");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(&src, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();
        install_binary(&src, &dest, false).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert!(!dest.symlink_metadata().unwrap().file_type().is_symlink());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn install_replaces_symlink_without_following_it_and_sets_mode() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir().join(format!(
            "talkdnd-fetch-link-{}-{}",
            std::process::id(),
            unique_tick()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("tools/bin")).unwrap();
        let real = root.join("release-bin");
        fs::write(&real, b"release").unwrap();
        let dest = root.join("tools/bin/combat-sim");
        symlink(&real, &dest).unwrap();
        let src = root.join("downloaded");
        fs::write(&src, b"downloaded").unwrap();
        install_binary(&src, &dest, true).unwrap();
        assert_eq!(fs::read(&real).unwrap(), b"release");
        assert_eq!(fs::read(&dest).unwrap(), b"downloaded");
        assert!(!dest.symlink_metadata().unwrap().file_type().is_symlink());
        let mode = fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
        let _ = fs::remove_dir_all(&root);
    }

    fn unique_tick() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}

#[cfg(test)]
mod talkdnd_tests {
    use super::*;

    #[test]
    fn talkdnd_artifact() {
        let p = fetch_plan_for("talkdnd", "x86_64-pc-windows-msvc", false).unwrap();
        assert_eq!(p.artifact, "talkdnd-x86_64-pc-windows-msvc");
        assert_eq!(p.install_rel, "LetsPlay/bin/talkdnd.exe");
        assert!(TOOLS.contains(&"talkdnd"));
    }
}
