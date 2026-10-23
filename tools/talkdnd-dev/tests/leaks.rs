mod common;

use std::fs;
use std::path::Path;

use common::{text, TempRepo};

const MARKER: &str = "BODY_MARKER_9f3a_DO_NOT_PRINT";
const LIMIT: usize = 307200;

fn tracked(repo: &TempRepo, rels: &[&str]) {
    for rel in rels {
        if !repo.path.join(rel).exists() && fs::symlink_metadata(repo.path.join(rel)).is_err() {
            repo.write(rel, MARKER);
        }
    }
    repo.add(rels);
}

#[test]
fn clean_index_exits_zero_and_prints_nothing() {
    let repo = TempRepo::new();
    let allowed = [
        "README.md",
        "LetsPlay/Rules/rulings.md",
        "LetsPlay/Rules/rulings-2014.md",
        "LetsPlay/Rules/rulings-2024.md",
        "LetsPlay/Rules/defaults/rulings.md",
        "LetsPlay/Rules/defaults/rulings-2014.md",
        "LetsPlay/Rules/defaults/rulings-2024.md",
        "LetsPlay/Rules/combat/rulings-house.md",
        "LetsPlay/Rules/2014/README.md",
        "LetsPlay/Rules/2024/README.md",
        "LetsPlay/Rules/combat/README.md",
        "LetsPlay/Rules/combat/common/ac.yaml",
        "LetsPlay/Rules/UPPER.MD",
        "LetsPlay/Rules/map.Png",
        "LetsPlay/Adventures/README.md",
        "LetsPlay/Saves/README.md",
        "Rules/2014/README.md",
        "Rules/2024/README.md",
        "Adventures/README.md",
        "Saves/README.md",
        "tools/main.rs",
        "tools/lower/icon.png",
        "tools/upper/icon.PNG",
        "books/manual.Pdf",
        "cover.jpg",
        "other/DM-secrets.md",
        "other/Scarecrow.yaml",
    ];
    tracked(&repo, &allowed);
    repo.write_bytes("prose/exact.md", &vec![b'x'; LIMIT]);
    repo.write_bytes("prose/exact.TXT", &vec![b'x'; LIMIT]);
    repo.add(&["prose/exact.md", "prose/exact.TXT"]);
    repo.write("untracked/secret.pdf", MARKER);

    let output = common::run(&repo.path, &["check-leaks"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
    assert!(!text(&output.stderr).contains(MARKER));

    let nested = repo.path.join("tools");
    let output = common::run(&nested, &["check-leaks"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "");
}

#[test]
fn leaks_print_paths_only_and_exit_one() {
    let repo = TempRepo::new();
    // Each case pair lives in its own directory. APFS and NTFS keep one
    // directory entry when names differ only by case, so `manual.pdf` and
    // `manual.PDF` cannot both be tracked from the same folder.
    let hits = [
        "books/lower/manual.pdf",
        "books/upper/manual.PDF",
        "books/lower/manual.epub",
        "books/upper/manual.EPUB",
        "books/lower/manual.docx",
        "books/upper/manual.DOCX",
        "LetsPlay/Rules/2014/players.md",
        "LetsPlay/Rules/2014/rulings.md",
        "LetsPlay/Rules/2024/dmg.txt",
        "Rules/2014/old.md",
        "Rules/2024/old.md",
        "LetsPlay/Adventures/quest.md",
        "LetsPlay/Adventures/nested/README.md",
        "Adventures/quest.md",
        "LetsPlay/Saves/session.md",
        "LetsPlay/Saves/deep/log.txt",
        "Saves/session.md",
        "secrets/dm-secrets.md",
        "LetsPlay/Rules/combat/2024/monsters/scarecrow.yaml",
        "other/scarecrow.yaml",
        "LetsPlay/Rules/notes.md",
        "LetsPlay/Rules/combat/notes.md",
        "Rules/loose.md",
        "Rules/combat/also.md",
        "LetsPlay/Rules/img-lower/map.png",
        "LetsPlay/Rules/img-upper/map.PNG",
        "LetsPlay/Adventures/pic.jpg",
        "LetsPlay/Saves/pic.jpeg",
        "Adventures/pic.gif",
        "Saves/pic.webp",
        "Rules/pic.tif",
        "LetsPlay/Rules/2024/pic.tiff",
        "Rules/2014/pic.bmp",
        "prose/lower/over.md",
        "prose/lower/over.txt",
        "prose/upper/over.MD",
        "prose/upper/over.TXT",
        "tools/link.md",
        "ghost.pdf",
    ];
    let allowed = [
        "LetsPlay/Rules/rulings.md",
        "LetsPlay/Rules/defaults/rulings.md",
        "LetsPlay/Rules/defaults/rulings-2024.md",
        "LetsPlay/Rules/combat/rulings-house.md",
        "LetsPlay/Rules/2014/README.md",
        "LetsPlay/Adventures/README.md",
        "tools/ok.png",
        "tools/keep.rs",
        "books/mixed/manual.Pdf",
        "other/DM-secrets.md",
        "other/Scarecrow.yaml",
        "prose/exact.md",
    ];
    for rel in hits.iter().chain(allowed.iter()) {
        match *rel {
            "prose/lower/over.md"
            | "prose/lower/over.txt"
            | "prose/upper/over.MD"
            | "prose/upper/over.TXT" => {
                repo.write_bytes(rel, &vec![b'Q'; LIMIT + 1]);
            }
            "prose/exact.md" => {
                repo.write_bytes(rel, &vec![b'x'; LIMIT]);
            }
            "tools/link.md" | "ghost.pdf" => {}
            _ => repo.write(rel, MARKER),
        }
    }
    repo.write_bytes("payload.bin", &vec![b'Z'; LIMIT + 1]);
    // Absolute target. Windows does not reliably follow `../payload.bin`.
    let payload = repo.path.join("payload.bin");
    repo.symlink(
        "tools/link.md",
        payload.to_str().expect("utf-8 payload path"),
    );
    let followed = fs::metadata(repo.path.join("tools/link.md")).expect("symlink follows payload");
    assert!(followed.is_file());
    assert!(followed.len() > LIMIT as u64);
    repo.write("ghost.pdf", MARKER);
    repo.write("untracked/secret.pdf", MARKER);

    let mut add: Vec<&str> = hits
        .iter()
        .copied()
        .chain(allowed.iter().copied())
        .filter(|rel| *rel != "ghost.pdf")
        .collect();
    add.push("ghost.pdf");
    repo.add(&add);
    fs::remove_file(repo.path.join("ghost.pdf")).unwrap();
    assert!(!repo.path.join("ghost.pdf").exists());

    let output = common::run(&repo.path, &["check-leaks"]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    let stderr = text(&output.stderr);
    assert!(!stdout.contains(MARKER), "{stdout}");
    assert!(!stderr.contains(MARKER), "{stderr}");
    assert!(!stdout.contains(&"Q".repeat(40)), "{stdout}");
    assert!(!stdout.contains(&"Z".repeat(40)), "{stdout}");
    assert!(!stdout.contains("untracked/secret.pdf"));
    assert!(!stdout.contains("payload.bin"));
    for rel in &allowed {
        assert!(
            !stdout.lines().any(|line| line == *rel),
            "{rel} in {stdout}"
        );
    }
    let mut lines: Vec<&str> = stdout.lines().collect();
    let mut expected: Vec<&str> = hits.to_vec();
    lines.sort_unstable();
    expected.sort_unstable();
    assert_eq!(lines, expected);
}

#[test]
fn usage_and_missing_git_exit_two() {
    let outside = TempRepo::dir_only();
    let output = common::run(&outside.path, &["check-leaks"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
    assert!(!text(&output.stderr).is_empty());

    let output = common::run(Path::new("."), &["check-leaks", "extra"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(
        text(&output.stderr).trim(),
        "usage: talkdnd-dev check-leaks"
    );

    let output = common::run(Path::new("."), &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("usage:"));
}

#[test]
fn pdf2md_output_and_environments_are_leaks() {
    use talkdnd_dev::leaks::generated_tree;
    assert!(generated_tree("LetsPlay/Rules/2024/.md/Book/Book.md"));
    assert!(generated_tree("LetsPlay/Adventures/x/.md/X/img/p0001.webp"));
    assert!(generated_tree("LetsPlay/.pdf2md/status.json"));
    assert!(generated_tree("LetsPlay/tools/pdf2md/.venv/pyvenv.cfg"));
    assert!(generated_tree("LetsPlay/Adventures/x/index/X.pages.md"));
    assert!(generated_tree("LetsPlay/Rules/2024/x/index/img/p0001.webp"));
    assert!(generated_tree("Rules/2014/x/index/manifest.json"));
    assert!(!generated_tree("docs/index/README.md"));
    assert!(!generated_tree("LetsPlay/tools/pdf2md/pdf2md/convert.py"));
    assert!(!generated_tree("LetsPlay/README.md"));
}
