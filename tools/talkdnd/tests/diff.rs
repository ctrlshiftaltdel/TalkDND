//! `snapshot diff` (P01, TASK-0059; campaign-continuity.md §5.1).

use std::fs;
use std::path::{Path, PathBuf};

use talkdnd::checkpoint::{self, Cmd};
use talkdnd::diff;
use talkdnd::migrate;

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("talkdnd-diff-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let save = root.join("Saves/2030-01-02-01");
    for d in ["Sessions", "combat", "Characters", "sheets"] {
        fs::create_dir_all(save.join(d)).unwrap();
    }
    fs::write(root.join("AGENTS.md"), "x\n").unwrap();
    fs::write(
        save.join("Sessions/2030-01-02-session-00-setup.md"),
        "# 会话\n",
    )
    .unwrap();
    fs::write(
        save.join("combat/campaign.yaml"),
        "name: 测试战役\nruleset: \"2024\"\nparty: [tess]\n",
    )
    .unwrap();
    fs::write(
        save.join("progress.yaml"),
        "xp: {tess: 300}\npurses:\n  tess: {cp: 500}\n  party: {cp: 0, items: [rope]}\n",
    )
    .unwrap();
    fs::write(
        save.join("Characters/tess.md"),
        "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nlevel: 3\nhp: 12\nmax_hp: 20\n```\n",
    )
    .unwrap();
    fs::write(save.join("sheets/tess.yaml"), "id: tess\n").unwrap();
    fs::write(
        save.join("combat/party-state.yaml"),
        "version: 1\nmembers:\n  tess:\n    name: Tess\n    hp: 12\n    max_hp: 20\n    temp_hp: 0\n    resources:\n      法术位: {current: 2, max: 2, recharge: long}\n    conditions:\n    - condition: poisoned\n",
    )
    .unwrap();
    migrate::init(&save).unwrap();
    (root, save)
}

fn root_of(save: &Path) -> PathBuf {
    save.parent().unwrap().parent().unwrap().to_path_buf()
}

fn snapshot(save: &Path, reason: Option<&str>) -> String {
    checkpoint::run(
        &root_of(save),
        save,
        Cmd::Snapshot {
            reason: reason.map(str::to_string),
            json: false,
        },
    )
    .unwrap();
    checkpoint::latest(save).unwrap().id
}

#[test]
fn diff_reports_every_contract_field() {
    let (root, save) = fixture("fields");
    let a = snapshot(&save, Some("first"));
    // The party plays on: HP, temp HP, level, a resource, a condition, gold,
    // items, XP and the party list all change.
    fs::write(
        save.join("combat/party-state.yaml"),
        "version: 1\nmembers:\n  tess:\n    name: Tess\n    hp: 8\n    max_hp: 20\n    temp_hp: 5\n    resources:\n      法术位: {current: 1, max: 2, recharge: long}\n    conditions: []\n",
    )
    .unwrap();
    fs::write(
        save.join("Characters/tess.md"),
        "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nlevel: 4\nhp: 8\nmax_hp: 20\n```\n",
    )
    .unwrap();
    fs::write(
        save.join("progress.yaml"),
        "xp: {tess: 900}\npurses:\n  tess: {cp: 300}\n  party: {cp: 0, items: [rope, torch]}\n",
    )
    .unwrap();
    fs::write(
        save.join("combat/campaign.yaml"),
        "name: 测试战役\nruleset: \"2024\"\nparty: [tess, mira]\n",
    )
    .unwrap();
    let b = snapshot(&save, Some("second"));
    let lines = diff::diff_lines(&save, &a, &b).unwrap();
    for want in [
        "HP: tess 12 → 8",
        "临时HP: tess 0 → 5",
        "等级: tess 3 → 4",
        "资源: tess 法术位 2/2 → 1/2",
        "状态: tess -poisoned",
        "金币: tess 500 → 300",
        "物品: party +torch",
        "XP: tess 300 → 900",
        "队伍成员: +mira",
    ] {
        assert!(
            lines.iter().any(|l| l == want),
            "missing {want:?} in {lines:?}"
        );
    }
    assert_eq!(lines.len(), 9, "{lines:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn diff_is_deterministic_for_the_same_input() {
    let (root, save) = fixture("determinism");
    let a = snapshot(&save, Some("first"));
    fs::write(save.join("progress.yaml"), "xp: {tess: 900}\n").unwrap();
    let b = snapshot(&save, Some("second"));
    let first = diff::diff_lines(&save, &a, &b).unwrap();
    for _ in 0..3 {
        assert_eq!(diff::diff_lines(&save, &a, &b).unwrap(), first);
    }
    // Reversed order reports the same values the other way round.
    let back = diff::diff_lines(&save, &b, &a).unwrap();
    assert_eq!(back.len(), first.len());
    assert!(back.iter().any(|l| l == "XP: tess 900 → 300"), "{back:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn diff_labels_missing_and_renamed_fields_unknown() {
    let (root, save) = fixture("unknown");
    let a = snapshot(&save, Some("first"));
    // The live save loses its party-state and renames the member; the sheet
    // loses its level. Nothing may be guessed.
    fs::remove_file(save.join("combat/party-state.yaml")).unwrap();
    fs::remove_file(save.join("Characters/tess.md")).unwrap();
    fs::write(
        save.join("Characters/tess-prime.md"),
        "# Tess Prime\n\n```combat yaml\nid: tess-prime\nname: Tess Prime\nhp: 8\nmax_hp: 20\n```\n",
    )
    .unwrap();
    let lines = diff::diff_lines(&save, &a, "current").unwrap();
    assert!(lines.iter().any(|l| l == "HP: tess 12 → 未知"), "{lines:?}");
    assert!(
        lines.iter().any(|l| l == "等级: tess 3 → 未知"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "资源: tess 法术位 2/2 → 未知"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "状态: tess poisoned → 未知"),
        "{lines:?}"
    );
    // The renamed member is a new key, never a match.
    assert!(
        !lines.iter().any(|l| l == "HP: tess-prime 未知 → 未知"),
        "a key missing on both sides must not be reported: {lines:?}"
    );
    // Gold and XP come from progress.yaml, which still exists.
    assert!(
        !lines.iter().any(|l| l == "XP: tess 300 → 300"),
        "unchanged values must not be reported: {lines:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn diff_refuses_a_missing_or_unusable_checkpoint() {
    let (root, save) = fixture("missing");
    let a = snapshot(&save, None);
    let err = diff::diff_lines(&save, "20990101-000000", "current").unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
    let err = diff::diff_lines(&save, &a, "20990101-000000").unwrap_err();
    assert!(err.to_string().contains("不存在"), "{err}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn diff_after_restore_shows_no_changes() {
    let (root, save) = fixture("after-restore");
    let a = snapshot(&save, None);
    fs::write(save.join("progress.yaml"), "xp: {tess: 900}\n").unwrap();
    fs::write(
        save.join("combat/party-state.yaml"),
        "version: 1\nmembers:\n  tess:\n    name: Tess\n    hp: 3\n    max_hp: 20\n    temp_hp: 0\n    resources: {}\n    conditions: []\n",
    )
    .unwrap();
    checkpoint::run(
        &root_of(&save),
        &save,
        Cmd::Restore {
            id: a.clone(),
            yes: true,
            json: false,
        },
    )
    .unwrap();
    assert!(diff::diff_lines(&save, &a, "current").unwrap().is_empty());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn diff_works_with_spaces_in_the_play_folder_path() {
    // A play folder with spaces in its name (the release may be extracted
    // anywhere; CI runs this on Linux, macOS and Windows). No path component
    // ends with a space: Windows trims those and the path stops resolving.
    let play =
        std::env::temp_dir().join(format!("talkdnd diff 冒险 {}/My Game", std::process::id()));
    let _ = fs::remove_dir_all(&play);
    let save = play.join("Saves/2030-01-02-01");
    for d in ["Sessions", "combat", "Characters", "sheets"] {
        fs::create_dir_all(save.join(d)).unwrap();
    }
    fs::write(play.join("AGENTS.md"), "x\n").unwrap();
    fs::write(
        save.join("Sessions/2030-01-02-session-00-setup.md"),
        "# 会话\n",
    )
    .unwrap();
    fs::write(
        save.join("combat/campaign.yaml"),
        "name: 测试战役\nruleset: \"2024\"\nparty: [tess]\n",
    )
    .unwrap();
    fs::write(
        save.join("progress.yaml"),
        "xp: {tess: 300}\npurses:\n  tess: {cp: 500}\n",
    )
    .unwrap();
    fs::write(
        save.join("Characters/tess.md"),
        "# Tess\n\n```combat yaml\nid: tess\nname: Tess\nlevel: 3\nhp: 12\nmax_hp: 20\n```\n",
    )
    .unwrap();
    migrate::init(&save).unwrap();
    let a = checkpoint::run(
        &play,
        &save,
        Cmd::Snapshot {
            reason: Some("space test".into()),
            json: false,
        },
    );
    assert!(a.is_ok(), "{a:?}");
    fs::write(save.join("progress.yaml"), "xp: {tess: 900}\n").unwrap();
    let id = checkpoint::latest(&save).unwrap().id;
    let lines = diff::diff_lines(&save, &id, "current").unwrap();
    assert!(lines.iter().any(|l| l == "XP: tess 300 → 900"), "{lines:?}");
    let _ = fs::remove_dir_all(&play);
}
