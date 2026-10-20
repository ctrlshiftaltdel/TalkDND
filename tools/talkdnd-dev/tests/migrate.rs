mod common;

use std::fs;
use std::path::Path;

use common::{text, TempRepo};

fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&path).unwrap();
            out.push(format!("link:{rel}->{}", target.to_string_lossy()));
        } else if meta.is_dir() {
            out.push(format!("dir:{rel}"));
            walk(root, &path, out);
        } else {
            let body = fs::read(&path).unwrap();
            out.push(format!("file:{rel}:{}", String::from_utf8_lossy(&body)));
        }
    }
}

fn sample_tree(repo: &TempRepo) {
    repo.write("Rules/2014/ch1.md", "ch1-2014");
    repo.write("Rules/2024/ch1.md", "ch1-2024");
    repo.write("Rules/notes.md", "loose-notes");
    repo.write("Rules/.secret", "dot-secret");
    repo.write("Rules/..cache", "dotdot-cache");
    repo.write("Rules/combat/guard.yaml", "guard-stays");
    repo.write("Rules/combat/common/ac.yaml", "ac-stays");
    repo.write("Rules/combat/local/overlay.yaml", "overlay-body");
    repo.write("Rules/combat/2024/monsters/scarecrow.yaml", "crow-body");
    repo.write("Rules/combat/2024/monsters/goblin.yaml", "goblin-stays");
    repo.write("Adventures/quest.md", "quest-body");
    repo.write("Adventures/.notes", "adv-hidden");
    repo.write("Saves/s1/log.md", "save-log");
    repo.write("LetsPlay/Rules/2014/README.md", "tracked-readme");
    repo.write("LetsPlay/Adventures/README.md", "tracked-adv");
}

const DRY_RUN: &str = "\
move: Rules/2014/ch1.md -> LetsPlay/Rules/2014/ch1.md
move: Rules/2024/ch1.md -> LetsPlay/Rules/2024/ch1.md
move: Rules/notes.md -> LetsPlay/Rules/notes.md
move: Rules/.secret -> LetsPlay/Rules/.secret
move: Rules/..cache -> LetsPlay/Rules/..cache
move: Rules/combat/local -> LetsPlay/Rules/combat/local
move: Rules/combat/2024/monsters/scarecrow.yaml -> LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml
move: Adventures/quest.md -> LetsPlay/Adventures/quest.md
move: Adventures/.notes -> LetsPlay/Adventures/.notes
move: Saves/s1 -> LetsPlay/Saves/s1
migrate_local_layout: dry-run, 10 move(s), 0 skip(s) (pass --apply to move)
";

#[test]
fn dry_run_lists_moves_and_does_not_touch_the_tree() {
    let repo = TempRepo::new();
    sample_tree(&repo);
    let before = tree(&repo.path);
    let output = common::run(&repo.path, &["migrate-layout"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), "");
    assert_eq!(text(&output.stdout), DRY_RUN);
    assert!(!text(&output.stdout).contains("overlay-body"));
    assert!(!text(&output.stdout).contains("guard-stays"));
    assert!(!text(&output.stdout).contains("goblin-stays"));
    assert_eq!(tree(&repo.path), before);
}

#[test]
fn apply_moves_once_and_second_apply_is_idle() {
    let repo = TempRepo::new();
    sample_tree(&repo);
    let output = common::run(&repo.path, &["migrate-layout", "--apply"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.starts_with(DRY_RUN.trim_end().rsplit_once('\n').unwrap().0));
    assert!(stdout.ends_with("migrate_local_layout: applied, 10 move(s), 0 skip(s)\n"));

    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/2014/ch1.md")).unwrap(),
        "ch1-2014"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/2014/README.md")).unwrap(),
        "tracked-readme"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/.secret")).unwrap(),
        "dot-secret"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/..cache")).unwrap(),
        "dotdot-cache"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/combat/local/overlay.yaml")).unwrap(),
        "overlay-body"
    );
    assert_eq!(
        fs::read_to_string(
            repo.path
                .join("LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml")
        )
        .unwrap(),
        "crow-body"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Saves/s1/log.md")).unwrap(),
        "save-log"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/combat/guard.yaml")).unwrap(),
        "guard-stays"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/combat/common/ac.yaml")).unwrap(),
        "ac-stays"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/combat/2024/monsters/goblin.yaml")).unwrap(),
        "goblin-stays"
    );
    assert!(!repo.path.join("Rules/2014").exists());
    assert!(!repo.path.join("Rules/2024").exists());
    assert!(!repo.path.join("Rules/notes.md").exists());
    assert!(!repo.path.join("Rules/combat/local").exists());
    assert!(!repo
        .path
        .join("Rules/combat/2024/monsters/scarecrow.yaml")
        .exists());
    assert!(!repo.path.join("Adventures").exists());
    assert!(!repo.path.join("Saves").exists());
    assert!(repo.path.join("Rules/combat").is_dir());

    let again = common::run(&repo.path, &["migrate-layout", "--apply"]);
    assert_eq!(again.status.code(), Some(0), "{}", text(&again.stderr));
    assert_eq!(
        text(&again.stdout),
        "migrate_local_layout: applied, 0 move(s), 0 skip(s)\n"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/combat/guard.yaml")).unwrap(),
        "guard-stays"
    );
}

#[test]
fn existing_destination_is_skipped_and_not_overwritten() {
    let repo = TempRepo::new();
    repo.write("Rules/2014/README.md", "SOURCE_README");
    repo.write("LetsPlay/Rules/2014/README.md", "DEST_README");
    repo.write("Rules/notes.md", "SOURCE_NOTES");
    repo.write("LetsPlay/Rules/notes.md", "DEST_NOTES");
    repo.write("Rules/box/inside.md", "SOURCE_BOX");
    repo.write("LetsPlay/Rules/box", "DEST_BOX");
    repo.write("Rules/linkfile.md", "SOURCE_LINK");
    repo.symlink("LetsPlay/Rules/linkfile.md", "missing-target");
    repo.write("Adventures/README.md", "SOURCE_ADV");
    repo.write("LetsPlay/Adventures/README.md", "DEST_ADV");

    let expected = "\
skip (exists): Rules/2014/README.md -> LetsPlay/Rules/2014/README.md
skip (exists): Rules/box -> LetsPlay/Rules/box
skip (exists): Rules/linkfile.md -> LetsPlay/Rules/linkfile.md
skip (exists): Rules/notes.md -> LetsPlay/Rules/notes.md
skip (exists): Adventures/README.md -> LetsPlay/Adventures/README.md
migrate_local_layout: applied, 0 move(s), 5 skip(s)
";
    let first = common::run(&repo.path, &["migrate-layout", "--apply"]);
    assert_eq!(first.status.code(), Some(0), "{}", text(&first.stderr));
    assert_eq!(text(&first.stdout), expected);
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/2014/README.md")).unwrap(),
        "DEST_README"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/2014/README.md")).unwrap(),
        "SOURCE_README"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/notes.md")).unwrap(),
        "DEST_NOTES"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/box")).unwrap(),
        "DEST_BOX"
    );
    assert!(repo.path.join("Rules/box/inside.md").is_file());
    assert_eq!(
        fs::read_link(repo.path.join("LetsPlay/Rules/linkfile.md")).unwrap(),
        Path::new("missing-target")
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/linkfile.md")).unwrap(),
        "SOURCE_LINK"
    );

    let second = common::run(&repo.path, &["migrate-layout", "--apply"]);
    assert_eq!(text(&second.stdout), expected);
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/notes.md")).unwrap(),
        "SOURCE_NOTES"
    );
}

#[test]
fn directory_merge_keeps_dest_files_and_empty_source_subdir() {
    let repo = TempRepo::new();
    repo.write("Rules/2014/a.md", "file-a");
    repo.write("Rules/2014/sub/b.md", "file-b");
    repo.write("LetsPlay/Rules/2014/README.md", "keep-readme");
    repo.write("LetsPlay/Rules/2014/sub/c.md", "keep-c");

    let output = common::run(&repo.path, &["migrate-layout", "--apply"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "\
move: Rules/2014/a.md -> LetsPlay/Rules/2014/a.md
move: Rules/2014/sub/b.md -> LetsPlay/Rules/2014/sub/b.md
migrate_local_layout: applied, 2 move(s), 0 skip(s)
"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/2014/README.md")).unwrap(),
        "keep-readme"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/2014/sub/c.md")).unwrap(),
        "keep-c"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("LetsPlay/Rules/2014/sub/b.md")).unwrap(),
        "file-b"
    );
    assert!(repo.path.join("Rules/2014/sub").is_dir());
    assert!(!repo.path.join("Rules/2014/sub/b.md").exists());
    assert!(repo.path.join("Rules").is_dir());
}

#[test]
fn broken_symlink_moves_as_one_entry() {
    let repo = TempRepo::new();
    repo.symlink("Rules/2014/broken", "missing-target");
    let dry = common::run(&repo.path, &["migrate-layout"]);
    assert_eq!(
        text(&dry.stdout),
        "\
move: Rules/2014/broken -> LetsPlay/Rules/2014/broken
migrate_local_layout: dry-run, 1 move(s), 0 skip(s) (pass --apply to move)
"
    );
    assert!(fs::symlink_metadata(repo.path.join("Rules/2014/broken")).is_ok());
    assert!(!repo.path.join("LetsPlay/Rules/2014/broken").exists());

    let apply = common::run(&repo.path, &["migrate-layout", "--apply"]);
    assert!(apply.status.success(), "{}", text(&apply.stderr));
    assert_eq!(
        fs::read_link(repo.path.join("LetsPlay/Rules/2014/broken")).unwrap(),
        Path::new("missing-target")
    );
    assert!(!repo.path.join("Rules/2014").exists());
}

#[test]
fn empty_repo_and_subdirectory_and_usage() {
    let empty = TempRepo::new();
    let output = common::run(&empty.path, &["migrate-layout"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        text(&output.stdout),
        "migrate_local_layout: dry-run, 0 move(s), 0 skip(s) (pass --apply to move)\n"
    );
    let applied = common::run(&empty.path, &["migrate-layout", "--apply"]);
    assert_eq!(
        text(&applied.stdout),
        "migrate_local_layout: applied, 0 move(s), 0 skip(s)\n"
    );

    let repo = TempRepo::new();
    repo.write("Rules/notes.md", "from-sub");
    fs::create_dir(repo.path.join("nested")).unwrap();
    let output = common::run(&repo.path.join("nested"), &["migrate-layout"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "\
move: Rules/notes.md -> LetsPlay/Rules/notes.md
migrate_local_layout: dry-run, 1 move(s), 0 skip(s) (pass --apply to move)
"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/notes.md")).unwrap(),
        "from-sub"
    );
    assert!(!repo.path.join("LetsPlay/Rules/notes.md").exists());

    let outside = TempRepo::dir_only();
    let output = common::run(&outside.path, &["migrate-layout"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");

    for args in [
        &["migrate-layout", "--apply", "extra"][..],
        &["migrate-layout", "--dry-run"][..],
        &["nope"][..],
    ] {
        let output = common::run(&repo.path, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(text(&output.stdout), "");
        assert!(text(&output.stderr).contains("usage:"));
    }
    assert_eq!(
        fs::read_to_string(repo.path.join("Rules/notes.md")).unwrap(),
        "from-sub"
    );
}
