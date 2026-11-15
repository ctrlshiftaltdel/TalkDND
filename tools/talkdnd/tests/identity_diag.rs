//! Identity diagnostic: synthetic saves only. No real save, book, secret or
//! personal data is read, and every scenario asserts the save tree is
//! byte-identical after the read-only diagnostic.

use std::fs;
use std::path::{Path, PathBuf};

use talkdnd::identity_diag::{self, Character, Issue, IssueKind, Report, Status};

struct Fixture {
    root: PathBuf,
    save: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("talkdnd-identity-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let save = root.join("Saves/2030-01-01-01");
        fs::create_dir_all(&save).unwrap();
        Fixture { root, save }
    }

    fn dir(&self, rel: &str) {
        fs::create_dir_all(self.save.join(rel)).unwrap();
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.save.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        let p = self.save.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    /// A character card: prose plus the `combat yaml` block the tools read.
    fn card(&self, rel: &str, id: &str, name: &str) {
        self.write(rel, &card(id, name));
    }

    /// A structured sheet under `sheets/<stem>.yaml` bound to its own card.
    fn sheet(&self, stem: &str, id: &str, name: &str, file: &str) {
        self.write(
            &format!("sheets/{stem}.yaml"),
            &sheet_yaml(id, name, file, 1),
        );
    }

    fn campaign(&self, party: &[&str]) {
        self.write(
            "combat/campaign.yaml",
            &format!(
                "name: \"fixture\"\nruleset: \"5.5e-srd\"\nparty: [{}]\n",
                party.join(", ")
            ),
        );
    }

    /// Diagnose, then prove the read did not touch the save tree.
    fn report(&self) -> Report {
        let before = snapshot(&self.save);
        let report = self.diagnose();
        let after = snapshot(&self.save);
        assert_eq!(
            before, after,
            "the identity diagnostic must not write to the save"
        );
        report
    }

    /// Diagnose without the automatic snapshot check; for the permission
    /// fixtures, whose tree cannot be snapshotted while locked down.
    fn diagnose(&self) -> Report {
        identity_diag::diagnose(&self.save)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Unreadable fixtures restore their modes before cleanup (see tests).
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn card(id: &str, name: &str) -> String {
    format!("# {name}\n\nA synthetic character card.\n\n```combat yaml\nid: {id}\nname: {name}\nlevel: 3\n```\n")
}

fn sheet_yaml(id: &str, name: &str, file: &str, version: u32) -> String {
    format!(
        "version: {version}\nid: {id}\nname: {name}\ncontroller: player\nfile: {file}\nruleset: 5.5e-srd\ncombat: {{}}\n"
    )
}

/// Every path (dirs included) with its bytes, so a create/delete/modify shows up.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                out.push((format!("{rel}/"), Vec::new()));
                walk(base, &p, out);
            } else {
                out.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

fn pairs(report: &Report) -> Vec<(IssueKind, String)> {
    report
        .issues
        .iter()
        .map(|i| (i.kind, i.path.clone()))
        .collect()
}

fn find(report: &Report, kind: IssueKind) -> &Issue {
    report
        .issues
        .iter()
        .find(|i| i.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} in {:?}", report.issues))
}

#[test]
fn a_consistent_save_is_ok() {
    let f = Fixture::new("ok");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.campaign(&["bruni"]);

    let r = f.report();
    assert_eq!(r.status, Status::Ok);
    assert_eq!(r.issues, Vec::new(), "{:?}", r.issues);
    assert_eq!(
        r.characters,
        vec![Character {
            id: "bruni".into(),
            name: "Bruni".into(),
            markdown: "Characters/bruni.md".into(),
            sheet: Some("sheets/bruni.yaml".into()),
        }]
    );
}

#[test]
fn missing_directories_are_normal() {
    let f = Fixture::new("empty");
    let r = f.report();
    assert_eq!(r.status, Status::Ok);
    assert_eq!(r.issues, Vec::new(), "{:?}", r.issues);
    assert!(r.characters.is_empty());

    // An empty campaign manifest (no party key) is normal too.
    f.write("combat/campaign.yaml", "name: \"fixture\"\n");
    let r = f.report();
    assert_eq!(r.status, Status::Ok);
    assert_eq!(r.issues, Vec::new(), "{:?}", r.issues);
}

#[test]
fn a_markdown_without_a_structured_sheet_is_unknown() {
    let f = Fixture::new("md-only");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.campaign(&["bruni"]);

    let r = f.report();
    assert_eq!(r.status, Status::Unknown);
    assert_eq!(
        pairs(&r),
        vec![(
            IssueKind::MarkdownWithoutSheet,
            "Characters/bruni.md".into()
        )]
    );
    assert_eq!(r.characters[0].sheet, None, "history not enrolled");
}

#[test]
fn duplicate_ids_across_characters_and_npcs_are_ambiguous() {
    let f = Fixture::new("dup-id");
    f.card("Characters/tess.md", "tess", "Tess");
    f.card("NPCs/tess.md", "tess", "Tess the NPC");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::DuplicateId);
    assert_eq!(
        issue.path, "Characters/tess.md",
        "first path in sorted order"
    );
    assert!(issue.detail.contains("Characters/tess.md"), "{issue:?}");
    assert!(issue.detail.contains("NPCs/tess.md"), "{issue:?}");
}

#[test]
fn ids_that_differ_only_by_case_are_ambiguous() {
    let f = Fixture::new("case-id");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.card("NPCs/Bruni.md", "Bruni", "Sergeant Bruni");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::DuplicateId);
    assert!(issue.detail.contains("bruni"), "{issue:?}");
}

#[test]
fn duplicate_names_are_ambiguous() {
    let f = Fixture::new("dup-name");
    f.card("Characters/tess.md", "tess", "Tess");
    f.card("NPCs/tess-copy.md", "tess-copy", "TESS");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::DuplicateName);
    assert_eq!(issue.path, "Characters/tess.md", "{issue:?}");
}

#[test]
fn an_id_that_matches_another_characters_name_is_ambiguous() {
    let f = Fixture::new("id-name-cross");
    f.card("Characters/tess.md", "tess", "Tess the Bold");
    f.card("NPCs/innkeeper.md", "innkeeper", "Tess");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let kinds: Vec<IssueKind> = r.issues.iter().map(|i| i.kind).collect();
    assert!(
        kinds.contains(&IssueKind::IdNameCollision),
        "{:?}",
        r.issues
    );
    assert!(
        !kinds.contains(&IssueKind::DuplicateName),
        "the names differ: {:?}",
        r.issues
    );
    assert!(
        !kinds.contains(&IssueKind::DuplicateId),
        "the ids differ: {:?}",
        r.issues
    );
}

#[test]
fn a_sheet_whose_internal_id_disagrees_with_its_file_name_is_ambiguous() {
    let f = Fixture::new("sheet-id");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "tess", "Tess", "Characters/bruni.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::SheetIdMismatch);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains("tess"), "{issue:?}");
}

#[test]
fn a_sheet_bound_to_another_markdown_is_ambiguous() {
    let f = Fixture::new("sheet-file");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.card("Characters/tess.md", "tess", "Tess");
    f.sheet("bruni", "bruni", "Bruni", "Characters/tess.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::SheetFileMismatch);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains("Characters/tess.md"), "{issue:?}");
    // tess genuinely has no sheet of its own; that is reported, not hidden.
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::MarkdownWithoutSheet && i.path == "Characters/tess.md"),
        "{:?}",
        r.issues
    );
}

#[test]
fn an_orphan_sheet_is_ambiguous() {
    let f = Fixture::new("orphan");
    f.sheet("ghost", "ghost", "Ghost", "Characters/ghost.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    assert_eq!(
        pairs(&r),
        vec![(IssueKind::OrphanSheet, "sheets/ghost.yaml".into())]
    );
}

#[test]
fn a_corrupt_sheet_is_incomplete_not_absent() {
    let f = Fixture::new("corrupt-yaml");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write("sheets/bruni.yaml", "version: 1\nid: [unclosed\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::UnparsableSheet);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    // The card is still reported as unbound rather than silently paired.
    assert!(r
        .issues
        .iter()
        .any(|i| i.kind == IssueKind::MarkdownWithoutSheet));
}

#[test]
fn a_future_sheet_version_is_incomplete() {
    let f = Fixture::new("future-version");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.write(
        "sheets/bruni.yaml",
        &sheet_yaml("bruni", "Bruni", "Characters/bruni.md", 2),
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::UnsupportedSheetVersion);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains('2'), "{issue:?}");
}

#[test]
fn a_sheet_missing_an_identity_field_is_incomplete() {
    let f = Fixture::new("sheet-missing");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write(
        "sheets/bruni.yaml",
        "version: 1\nname: Bruni\nfile: Characters/bruni.md\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::SheetMissingField);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains("id"), "{issue:?}");
}

#[test]
fn an_invalid_controller_is_incomplete() {
    let f = Fixture::new("controller");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write(
        "sheets/bruni.yaml",
        "version: 1\nid: bruni\nname: Bruni\ncontroller: shared\nfile: Characters/bruni.md\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::SheetInvalidField);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains("controller"), "{issue:?}");
}

#[test]
fn unknown_sheet_fields_are_unknown_and_values_are_not_echoed() {
    let f = Fixture::new("unknown-field");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write(
        "sheets/bruni.yaml",
        &format!(
            "{}token: secret-abc123\n",
            sheet_yaml("bruni", "Bruni", "Characters/bruni.md", 1)
        ),
    );

    let r = f.report();
    assert_eq!(r.status, Status::Unknown);
    let issue = find(&r, IssueKind::UnknownSheetField);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains("token"), "{issue:?}");
    assert!(
        !issue.detail.contains("secret-abc123"),
        "values must not be echoed: {issue:?}"
    );
    assert!(r.characters[0].sheet.is_some(), "the sheet is still bound");
}

#[test]
fn a_sheet_file_path_that_escapes_the_save_is_ambiguous() {
    let f = Fixture::new("escape");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "../outside.md");
    f.card("Characters/tess.md", "tess", "Tess");
    f.sheet("tess", "tess", "Tess", "/etc/passwd");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    assert_eq!(
        pairs(&r),
        vec![
            (IssueKind::SheetFileEscape, "sheets/bruni.yaml".into()),
            (IssueKind::SheetFileEscape, "sheets/tess.yaml".into()),
        ]
    );
}

#[test]
fn a_markdown_id_that_is_not_a_safe_file_name_is_ambiguous() {
    let f = Fixture::new("bad-id");
    f.card("Characters/bruni.md", "../evil", "Bruni");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::InvalidId);
    assert_eq!(issue.path, "Characters/bruni.md");
    assert!(issue.detail.contains("evil"), "{issue:?}");
}

#[test]
fn party_entries_that_name_unknown_characters_are_ambiguous() {
    let f = Fixture::new("party-unknown");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.campaign(&["bruni", "ghost"]);

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::PartyUnknownId);
    assert_eq!(issue.path, "combat/campaign.yaml");
    assert!(issue.detail.contains("ghost"), "{issue:?}");
}

#[test]
fn duplicate_party_entries_are_ambiguous() {
    let f = Fixture::new("party-dup");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.campaign(&["bruni", "bruni"]);

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::PartyDuplicateId);
    assert_eq!(issue.path, "combat/campaign.yaml");
    assert!(issue.detail.contains("bruni"), "{issue:?}");
}

#[test]
fn a_corrupt_campaign_manifest_is_incomplete() {
    let f = Fixture::new("campaign");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write("combat/campaign.yaml", "party: [unclosed\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::UnparsableCampaignManifest);
    assert_eq!(issue.path, "combat/campaign.yaml");
}

#[test]
fn a_markdown_without_a_combat_block_is_incomplete() {
    let f = Fixture::new("no-block");
    f.write(
        "Characters/broken.md",
        "# Broken\n\nNo combat block here.\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![(IssueKind::MissingCombatBlock, "Characters/broken.md".into())]
    );
}

#[test]
fn a_combat_block_without_an_id_is_incomplete() {
    let f = Fixture::new("no-id");
    f.write(
        "Characters/broken.md",
        "# Broken\n\n```combat yaml\nname: Broken\n```\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::MarkdownMissingField);
    assert_eq!(issue.path, "Characters/broken.md");
    assert!(issue.detail.contains("id"), "{issue:?}");
}

#[test]
fn underscore_and_dot_files_are_not_character_cards() {
    let f = Fixture::new("ignored-files");
    f.write("Characters/_party.md", "# Party overview\n\nNot a card.\n");
    f.write("Characters/.DS_Store", "not utf8 \u{fffd}");
    f.dir("sheets");

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
    assert_eq!(r.issues, Vec::new());
}

#[test]
fn unexpected_entries_are_incomplete() {
    let f = Fixture::new("unexpected");
    f.write("Characters/notes.txt", "free notes\n");
    f.dir("Characters/archive");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![
            (IssueKind::UnexpectedEntry, "Characters/archive".into()),
            (IssueKind::UnexpectedEntry, "Characters/notes.txt".into()),
        ]
    );
}

#[test]
fn invalid_utf8_is_incomplete() {
    let f = Fixture::new("utf8");
    f.write_bytes("Characters/bruni.md", b"# Bruni\n\n\xff\xfe not utf8\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![(IssueKind::InvalidUtf8, "Characters/bruni.md".into())]
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("unreadable-file");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    let before = snapshot(&f.save);
    let path = f.save.join("Characters/bruni.md");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        fs::read_to_string(&path).is_err(),
        "the fixture needs a genuinely unreadable file (run as a non-root user)"
    );

    let r = f.diagnose();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        before,
        snapshot(&f.save),
        "the identity diagnostic must not touch the save"
    );
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![(IssueKind::UnreadableFile, "Characters/bruni.md".into())]
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("unreadable-dir");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    let before = snapshot(&f.save);
    let dir = f.save.join("Characters");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        fs::read_dir(&dir).is_err(),
        "the fixture needs a genuinely unreadable directory (run as a non-root user)"
    );

    let r = f.diagnose();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        before,
        snapshot(&f.save),
        "the identity diagnostic must not touch the save"
    );
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![(IssueKind::UnreadableDirectory, "Characters".into())]
    );
}

#[test]
fn the_worst_finding_wins() {
    // Ok
    let ok = Fixture::new("worst-ok");
    ok.card("Characters/bruni.md", "bruni", "Bruni");
    ok.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    assert_eq!(ok.report().status, Status::Ok);

    // Unknown
    let unknown = Fixture::new("worst-unknown");
    unknown.card("Characters/bruni.md", "bruni", "Bruni");
    assert_eq!(unknown.report().status, Status::Unknown);

    // Ambiguous beats Unknown.
    let ambiguous = Fixture::new("worst-ambiguous");
    ambiguous.card("Characters/tess.md", "tess", "Tess");
    ambiguous.card("NPCs/tess.md", "tess", "Tess II");
    assert_eq!(ambiguous.report().status, Status::Ambiguous);

    // Incomplete beats Ambiguous.
    let incomplete = Fixture::new("worst-incomplete");
    incomplete.card("Characters/tess.md", "tess", "Tess");
    incomplete.card("NPCs/tess.md", "tess", "Tess II");
    incomplete.write("sheets/tess.yaml", "version: 1\nid: [unclosed\n");
    assert_eq!(incomplete.report().status, Status::Incomplete);
}

#[test]
fn issues_and_characters_are_deterministic() {
    let f = Fixture::new("order");
    // Created in an order that is not the output order.
    f.write("sheets/zeta.yaml", "version: 1\nid: [unclosed\n");
    f.card("NPCs/tess.md", "tess", "Tess");
    f.card("Characters/tess.md", "tess", "Tess");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write("sheets/alpha.yaml", "version: 1\nid: [unclosed\n");

    let first = f.report();
    let second = f.report();
    assert_eq!(first, second, "same input, same report");

    let mut sorted = first.issues.clone();
    sorted.sort();
    assert_eq!(first.issues, sorted, "issues follow the declared order");
    assert!(
        first.issues.windows(2).all(|w| w[0] <= w[1]),
        "issues are sorted: {:?}",
        first.issues
    );

    let ids: Vec<&str> = first.characters.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["bruni", "tess", "tess"]);
    let paths: Vec<&str> = first
        .characters
        .iter()
        .map(|c| c.markdown.as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["Characters/bruni.md", "Characters/tess.md", "NPCs/tess.md"]
    );
}
