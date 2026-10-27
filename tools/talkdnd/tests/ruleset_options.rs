//! TASK-0107: save-visible ruleset values, migration and fail-closed FULL.
//!
//! Fixtures are synthetic; no save contents leave the temporary folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("talkdnd-ruleset-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "x").unwrap();
    p
}

fn run(root: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .output()
        .unwrap();
    let s =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.success(), s)
}

fn ok(root: &Path, args: &[&str]) -> String {
    let (good, s) = run(root, args);
    assert!(good, "{args:?} failed: {s}");
    s
}

fn new_save(root: &Path, ruleset: Option<&str>, date: &str) -> PathBuf {
    match ruleset {
        Some(rs) => ok(
            root,
            &["new", "--ruleset", rs, "--scope", "0", "--date", date],
        ),
        None => ok(root, &["new", "--scope", "0", "--date", date]),
    };
    root.join("Saves").join(format!("{date}-01"))
}

fn manifest(save: &Path) -> String {
    fs::read_to_string(save.join("combat/campaign.yaml")).unwrap()
}

fn set_old_ruleset(save: &Path, value: &str) {
    let path = save.join("combat/campaign.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let text = text
        .lines()
        .map(|line| {
            if line.starts_with("ruleset:") {
                format!("ruleset: \"{value}\"   # 2014 | 2024")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(path, text).unwrap();
}

#[test]
fn four_values_normalize_and_illegal_values_are_actionable() {
    let r = root("values");
    for (i, (input, want)) in [
        ("5e-srd", "5e-srd"),
        ("  5E-FULL  ", "5e-full"),
        ("5.5e-srd", "5.5e-srd"),
        ("5.5E-FULL", "5.5e-full"),
        ("2014", "5e-srd"),
        ("2024", "5.5e-srd"),
    ]
    .into_iter()
    .enumerate()
    {
        let date = format!("2030-01-{:02}", i + 1);
        let save = new_save(&r, Some(input), &date);
        assert!(
            manifest(&save).contains(&format!("ruleset: \"{want}\"")),
            "{input} -> {want}: {}",
            manifest(&save)
        );
    }

    let (good, out) = run(&r, &["new", "--ruleset", "5.5", "--date", "2030-02-01"]);
    assert!(!good, "{out}");
    for needle in ["5.5e FULL", "5.5e SRD", "5e FULL", "5e SRD", "2014", "2024"] {
        assert!(out.contains(needle), "missing {needle} in {out}");
    }
}

#[test]
fn new_save_and_digest_use_display_names_not_bare_edition_values() {
    let r = root("display");
    let save = new_save(&r, None, "2030-03-01");
    assert!(manifest(&save).contains("ruleset: \"5.5e-srd\""));
    let d = ok(&r, &["start", "--save", "2030-03-01-01"]);
    assert!(d.contains("规则集：**5.5e SRD**"), "{d}");
    assert!(!d.contains("规则：**2024**"), "{d}");
    assert!(d.contains("完整书内容包"), "{d}");

    let save14 = new_save(&r, Some("5e-srd"), "2030-03-02");
    let d = ok(&r, &["start", "--save", "2030-03-02-01"]);
    assert!(d.contains("规则集：**5e SRD**"), "{d}");
    assert_eq!(manifest(&save14).matches("ruleset: \"5e-srd\"").count(), 1);

    let save24 = new_save(&r, Some("5.5e-srd"), "2030-03-03");
    let d = ok(&r, &["start", "--save", "2030-03-03-01"]);
    assert!(d.contains("规则集：**5.5e SRD**"), "{d}");
    assert_eq!(
        manifest(&save24).matches("ruleset: \"5.5e-srd\"").count(),
        1
    );
}

#[test]
fn old_manifest_values_load_and_migrate_on_the_next_write() {
    let r = root("migrate");
    let save = new_save(&r, None, "2030-04-01");
    let path = save.join("combat/campaign.yaml");
    set_old_ruleset(&save, "2024");
    let original = manifest(&save).replace(
        "party: []",
        "# keep this comment\nparty: []   # keep the party field",
    );
    fs::write(&path, &original).unwrap();

    // The old value must still load.
    assert!(manifest(&save).contains("ruleset: \"2024\""));
    let d = ok(&r, &["start", "--save", "2030-04-01-01"]);
    assert!(d.contains("规则集：**5.5e SRD**"), "{d}");
    let migrated = manifest(&save);
    assert!(migrated.contains("ruleset: \"5.5e-srd\""), "{migrated}");
    assert!(migrated.contains("# keep this comment"), "{migrated}");
    assert!(
        migrated.contains("party: []   # keep the party field"),
        "{migrated}"
    );
    assert!(
        migrated.contains("ruleset migrated from 2024 to 5.5e-srd"),
        "{migrated}"
    );
    assert_eq!(migrated.matches("ruleset:").count(), 1, "{migrated}");

    // Idempotent: the next start leaves the migrated manifest byte-identical.
    ok(&r, &["start", "--save", "2030-04-01-01"]);
    assert_eq!(manifest(&save), migrated);

    // The other legacy value maps to the 5e SRD value.
    let save14 = new_save(&r, None, "2030-04-02");
    set_old_ruleset(&save14, "2014");
    ok(&r, &["start", "--save", "2030-04-02-01"]);
    let migrated = manifest(&save14);
    assert!(migrated.contains("ruleset: \"5e-srd\""), "{migrated}");
    assert!(
        migrated.contains("ruleset migrated from 2014 to 5e-srd"),
        "{migrated}"
    );
}

#[test]
fn chargen_set_ruleset_writes_the_canonical_value() {
    let r = root("chargen");
    let save = new_save(&r, None, "2030-05-01");
    ok(
        &r,
        &[
            "chargen",
            "set",
            "ruleset",
            "  5.5E-SRD  ",
            "--save",
            "2030-05-01-01",
        ],
    );
    assert_eq!(manifest(&save).matches("ruleset: \"5.5e-srd\"").count(), 1);
    let state = fs::read_to_string(save.join("chargen.json")).unwrap();
    assert!(state.contains("\"ruleset\": \"5.5e-srd\""), "{state}");
}

#[test]
fn an_invalid_manifest_ruleset_fails_start_with_actionable_error() {
    let r = root("invalid-manifest");
    let save = new_save(&r, None, "2030-01-04");
    set_old_ruleset(&save, "5e");
    let (good, out) = run(&r, &["start", "--save", "2030-01-04-01"]);
    assert!(!good, "an invalid ruleset must fail closed: {out}");
    for needle in [
        "unsupported ruleset",
        "5.5e FULL",
        "5e FULL",
        "5.5e SRD",
        "5e SRD",
        "2014",
        "2024",
    ] {
        assert!(out.contains(needle), "missing {needle} in {out}");
    }
}

#[test]
fn full_without_an_unlocked_pack_fails_start_instead_of_falling_back() {
    let r = root("full-missing");
    let save = new_save(&r, Some("5e-full"), "2030-06-01");
    let before = manifest(&save);
    let (good, out) = run(&r, &["start", "--save", "2030-06-01-01"]);
    assert!(!good, "5e FULL without a pack must fail: {out}");
    for needle in [
        "5e FULL",
        "Player's Handbook",
        "Dungeon Master's Guide",
        "Monster Manual",
        "Rules/2014",
        "talkdnd library scan",
        "5e SRD",
    ] {
        assert!(out.contains(needle), "missing {needle} in {out}");
    }
    assert_eq!(
        manifest(&save),
        before,
        "a failed start must not rewrite the manifest"
    );
    assert!(
        !out.contains("完整书内容包：仅 SRD"),
        "must not fall back: {out}"
    );
}
