//! Identity diagnostic: synthetic saves only. No real save, book, secret or
//! personal data is read, and every scenario asserts the save tree is
//! byte-identical after the read-only diagnostic.

use std::fs;
use std::path::{Path, PathBuf};

use talkdnd::identity_diag::{self, Issue, IssueKind, Report, Status};

struct Fixture {
    root: PathBuf,
    save: PathBuf,
}

impl Fixture {
    /// A directory this test created, never a predictable path that already
    /// existed: `create_dir` is atomic, so an occupied name is skipped instead
    /// of removed. Only the directory created here is cleaned up.
    fn new(name: &str) -> Fixture {
        let base = std::env::temp_dir();
        for attempt in 0..64u32 {
            let root = base.join(format!(
                "talkdnd-identity-{}-{}-{}",
                name,
                std::process::id(),
                attempt
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let save = root.join("Saves/2030-01-01-01");
                    fs::create_dir_all(&save).unwrap();
                    return Fixture { root, save };
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create fixture directory {}: {e}", root.display()),
            }
        }
        panic!("no free fixture directory name for {name}");
    }

    fn dir(&self, rel: &str) {
        fs::create_dir_all(self.save.join(rel)).unwrap();
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let p = self.save.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, text).unwrap();
        p
    }

    fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        let p = self.save.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    /// A file outside the save tree (for link targets); never inside `Saves/`.
    fn outside(&self, rel: &str, text: &str) -> PathBuf {
        let p = self.root.join("outside").join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, text).unwrap();
        p
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

    /// Diagnose without the automatic snapshot check; for the permission and
    /// symlink fixtures, whose tree cannot be snapshotted while locked down.
    fn diagnose(&self) -> Report {
        identity_diag::diagnose(&self.save)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Only ever the directory `new` created (unreadable fixtures restore
        // their modes before cleanup; see the tests).
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn card(id: &str, name: &str) -> String {
    format!("# {name}\n\nA synthetic character card.\n\n```combat yaml\nid: {id}\nname: {name}\nlevel: 3\n```\n")
}

fn preview(fixture: &Fixture) -> serde_json::Value {
    fs::write(fixture.root.join("AGENTS.md"), "Synthetic play root\n").unwrap();
    let before = snapshot(&fixture.save);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(&fixture.root)
        .args(["advancement", "identity-preview", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(snapshot(&fixture.save), before);
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn markdown_identity_preview_is_read_only_and_requests_dm_review() {
    let fixture = Fixture::new("append-only-preview");
    fixture.card("Characters/Aria.md", "aria", "Aria");
    let report = preview(&fixture);
    assert_eq!(report["consistency"], "unverified");
    assert_eq!(report["write_capability"], "unverified");
    let card = &report["cards"][0];
    assert_eq!(card["path"], "Characters/Aria.md");
    assert_eq!(card["status"], "review_required");
    assert!(card["before_digest"].is_string());
    assert!(card["added_bytes"].as_u64().unwrap() > 32);
    assert!(!fixture.save.join("advancement").exists());
    assert!(!report.to_string().contains("A synthetic character card"));
}

#[test]
fn markdown_identity_preview_requires_both_records_and_detects_duplicate_tokens() {
    let fixture = Fixture::new("binding-preview");
    const TOKEN: &str = "00112233445566778899aabbccddeeff";
    let marked_card =
        |id, name| format!("{}\n<!-- talkdnd-identity: {TOKEN} -->\n", card(id, name));
    let receipt = |relative, token| {
        format!("version: 1\ncard: {relative}\ntoken: '{token}'\nhistory: unknown\n")
    };
    fixture.write("Characters/Aria.md", &marked_card("aria", "Aria"));
    assert_eq!(preview(&fixture)["cards"][0]["status"], "record_missing");
    fixture.write(
        "advancement/aria.yaml",
        &receipt("Characters/Aria.md", TOKEN),
    );
    let matched = preview(&fixture);
    assert_eq!(matched["cards"][0]["status"], "matched");
    assert!(!matched.to_string().contains(TOKEN));
    fixture.write(
        "advancement/aria.yaml",
        &receipt("Characters/Other.md", TOKEN),
    );
    assert_eq!(preview(&fixture)["cards"][0]["status"], "binding_mismatch");
    fixture.write(
        "advancement/aria.yaml",
        &receipt("Characters/Aria.md", TOKEN),
    );
    fixture.write("Characters/Brin.md", &marked_card("brin", "Brin"));
    fixture.write(
        "advancement/brin.yaml",
        &receipt("Characters/Brin.md", TOKEN),
    );
    fixture.write(
        "advancement/orphan.yaml",
        &receipt("Characters/Gone.md", TOKEN),
    );
    let report = preview(&fixture);
    assert_eq!(report["cards"][0]["status"], "duplicate_token");
    assert_eq!(report["cards"][1]["status"], "duplicate_token");
    assert_eq!(report["unmatched_records"][0], "orphan.yaml");
}

#[test]
fn structured_identity_preview_uses_yaml_and_validates_the_carrier_reference() {
    let fixture = Fixture::new("structured-identity-preview");
    let card_path = "Characters/Aria.md";
    let sheet_path = "sheets/aria.yaml";
    fixture.card(card_path, "aria", "Aria");
    let original = sheet_yaml("aria", "Aria", card_path, 1);
    fixture.write(sheet_path, &original);
    let report = preview(&fixture);
    assert_eq!(report["cards"][0]["carrier"], sheet_path);
    assert_eq!(report["cards"][0]["status"], "review_required");
    let closed_document = format!("{original}...\n");
    fixture.write(sheet_path, &closed_document);
    assert_eq!(
        preview(&fixture)["cards"][0]["status"],
        "format_review_required"
    );

    let token = "19c36d2fab09bc001c681acaeefa4567";
    fixture.write(
        sheet_path,
        &format!("{original}talkdnd_identity: '{token}'\n"),
    );
    let record = format!("version: 1\ncard: {card_path}\ncarrier: {sheet_path}\ntoken: '{token}'\nhistory: unknown\n");
    fixture.write("advancement/aria.yaml", &record);
    assert_eq!(preview(&fixture)["cards"][0]["status"], "matched");
    fixture.write(
        "advancement/aria.yaml",
        &record.replace(sheet_path, card_path),
    );
    assert_eq!(preview(&fixture)["cards"][0]["status"], "binding_mismatch");
    fixture.write("advancement/aria.yaml", &record);
    fixture.write(
        card_path,
        &format!(
            "{}\n<!-- talkdnd-identity: {token} -->\n",
            card("aria", "Aria")
        ),
    );
    assert_eq!(preview(&fixture)["cards"][0]["status"], "carrier_conflict");
}

fn sheet_yaml(id: &str, name: &str, file: &str, version: u32) -> String {
    format!(
        "version: {version}\nid: {id}\nname: {name}\ncontroller: player\nfile: {file}\nruleset: 5.5e-srd\ncombat: {{}}\n"
    )
}

/// Tree snapshot that never follows a link: the link itself is recorded with
/// its target string, so a dangling or external link is covered without being
/// traversed (and without panicking).
fn snapshot_no_follow(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<PathBuf> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
            let meta = fs::symlink_metadata(&p).unwrap();
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                let target = fs::read_link(&p).unwrap();
                out.push((
                    format!("{rel}@"),
                    target.to_string_lossy().into_owned().into_bytes(),
                ));
            } else if file_type.is_dir() {
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

fn has(report: &Report, kind: IssueKind) -> bool {
    report.issues.iter().any(|i| i.kind == kind)
}

/// The whole report as a user or log could see it (Debug plus every detail).
fn rendered(report: &Report) -> String {
    let mut text = format!("{report:?}");
    for issue in &report.issues {
        text.push_str(&issue.detail);
        text.push_str(&issue.path);
    }
    text
}

/// Whether a path is a link and where it points; used to prove the diagnostic
/// neither follows nor rewrites links.
fn link_state(path: &Path) -> (bool, Option<PathBuf>) {
    match fs::symlink_metadata(path) {
        Ok(meta) => (meta.file_type().is_symlink(), fs::read_link(path).ok()),
        Err(_) => (false, None),
    }
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
    assert_eq!(r.characters.len(), 1);
    let c = &r.characters[0];
    assert_eq!(c.id(), "bruni");
    assert_eq!(c.name(), "Bruni");
    assert_eq!(c.markdown(), "Characters/bruni.md");
    assert_eq!(c.sheet(), Some("sheets/bruni.yaml"));
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
    assert_eq!(r.characters[0].sheet(), None, "history not enrolled");
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
    // Affected relative paths are allowed (field values are not).
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
    // The capitalised id is also not a valid character id; the case-insensitive
    // duplicate check stays independent of that.
    assert!(has(&r, IssueKind::DuplicateId), "{:?}", r.issues);
    assert!(has(&r, IssueKind::InvalidId), "{:?}", r.issues);
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
    assert!(
        !issue.detail.contains("tess"),
        "field values must not be echoed: {issue:?}"
    );
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
    assert!(
        !issue.detail.contains("Characters/tess.md"),
        "the `file` value must not be echoed: {issue:?}"
    );
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
    f.write(
        "sheets/bruni.yaml",
        &sheet_yaml("bruni", "Bruni", "Characters/bruni.md", 2),
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::UnsupportedSheetVersion);
    assert_eq!(issue.path, "sheets/bruni.yaml");
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
    // The field name is this diagnostic's own vocabulary, not save content.
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
fn an_absent_or_null_controller_is_accepted() {
    // Older sheets predate `controller`; YAML `null` must read the same way.
    let absent = Fixture::new("controller-absent");
    absent.card("Characters/bruni.md", "bruni", "Bruni");
    absent.write(
        "sheets/bruni.yaml",
        "version: 1\nid: bruni\nname: Bruni\nfile: Characters/bruni.md\n",
    );
    let r = absent.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);

    let null = Fixture::new("controller-null");
    null.card("Characters/bruni.md", "bruni", "Bruni");
    null.write(
        "sheets/bruni.yaml",
        "version: 1\nid: bruni\nname: Bruni\ncontroller: null\nfile: Characters/bruni.md\n",
    );
    let r = null.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
}

#[test]
fn a_non_string_controller_is_incomplete() {
    let f = Fixture::new("controller-type");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write(
        "sheets/bruni.yaml",
        "version: 1\nid: bruni\nname: Bruni\ncontroller: 5\nfile: Characters/bruni.md\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    assert!(has(&r, IssueKind::SheetInvalidField), "{:?}", r.issues);
}

#[test]
fn unknown_sheet_fields_are_reported_by_count_without_names_or_values() {
    let f = Fixture::new("unknown-field");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write(
        "sheets/bruni.yaml",
        &format!(
            "{}s3cr3t_key_abc: s3cr3t_value_xyz\n",
            sheet_yaml("bruni", "Bruni", "Characters/bruni.md", 1)
        ),
    );

    let r = f.report();
    assert_eq!(r.status, Status::Unknown);
    let issue = find(&r, IssueKind::UnknownSheetField);
    assert_eq!(issue.path, "sheets/bruni.yaml");
    assert!(issue.detail.contains('1'), "{issue:?}");
    let text = rendered(&r);
    assert!(!text.contains("s3cr3t_key_abc"), "key name leaked: {text}");
    assert!(!text.contains("s3cr3t_value_xyz"), "value leaked: {text}");
    assert!(
        r.characters[0].sheet().is_some(),
        "the sheet is still bound"
    );
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
    let text = rendered(&r);
    assert!(!text.contains("outside.md"), "the value leaked: {text}");
    assert!(!text.contains("/etc/passwd"), "the value leaked: {text}");
}

#[test]
fn a_sheet_file_value_is_never_echoed() {
    let f = Fixture::new("escape-secret");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "../s3cr3t-token-abc.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    assert!(has(&r, IssueKind::SheetFileEscape), "{:?}", r.issues);
    let text = rendered(&r);
    assert!(
        !text.contains("s3cr3t-token-abc"),
        "the value leaked: {text}"
    );
}

#[test]
fn a_card_id_is_validated_as_a_character_id() {
    for bad in ["Bad_Id", "Bruni", "bad.id", "bad id", "../evil", "-leading"] {
        let f = Fixture::new("bad-id");
        f.card("Characters/x.md", bad, "X");
        let r = f.report();
        // An unusable card id is a conflict on that card, not a mere warning.
        assert_eq!(r.status, Status::Ambiguous, "id {bad:?}: {:?}", r.issues);
        let issue = find(&r, IssueKind::InvalidId);
        assert_eq!(issue.path, "Characters/x.md", "id {bad:?}");
        assert_eq!(
            r.issues
                .iter()
                .filter(|i| i.kind == IssueKind::InvalidId)
                .count(),
            1,
            "id {bad:?}: exactly one finding for the card: {:?}",
            r.issues
        );
    }
    // 81 bytes is one over the 80-byte limit.
    let long = "a".repeat(81);
    let f = Fixture::new("bad-id-long");
    f.card("Characters/x.md", &long, "X");
    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous, "{:?}", r.issues);
    let issue = find(&r, IssueKind::InvalidId);
    assert_eq!(issue.path, "Characters/x.md");
}

#[test]
fn an_invalid_id_is_never_echoed() {
    let f = Fixture::new("bad-id-secret");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.write(
        "sheets/bruni.yaml",
        "version: 1\nid: ../s3cr3t-id\nname: Bruni\nfile: Characters/bruni.md\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    assert!(has(&r, IssueKind::InvalidId), "{:?}", r.issues);
    let text = rendered(&r);
    assert!(!text.contains("s3cr3t-id"), "the id leaked: {text}");
}

#[test]
fn a_sheet_internal_id_must_be_a_valid_character_id() {
    let f = Fixture::new("sheet-bad-id");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "Bad_Id", "Bruni", "Characters/bruni.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let issue = find(&r, IssueKind::InvalidId);
    assert_eq!(issue.path, "sheets/bruni.yaml");
}

#[test]
fn a_sheet_file_name_must_be_a_valid_character_id() {
    let f = Fixture::new("sheet-bad-stem");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("Bruni", "bruni", "Bruni", "Characters/bruni.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    assert!(has(&r, IssueKind::InvalidId), "{:?}", r.issues);
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
fn the_party_overview_and_os_metadata_are_not_cards() {
    let f = Fixture::new("ignored-files");
    f.write("Characters/_party.md", "# Party overview\n\nNot a card.\n");
    f.write("Characters/.DS_Store", "not utf8 \u{fffd}");
    f.dir("sheets");

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
    assert_eq!(r.issues, Vec::new());
}

#[test]
fn an_underscore_prefixed_duplicate_is_not_skipped() {
    let f = Fixture::new("underscore-dup");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.card("Characters/_duplicate.md", "bruni", "Bruni Again");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous, "{:?}", r.issues);
    let issue = find(&r, IssueKind::DuplicateId);
    assert!(
        issue.detail.contains("Characters/_duplicate.md"),
        "{issue:?}"
    );
}

#[test]
fn an_underscore_prefixed_broken_card_is_not_skipped() {
    let f = Fixture::new("underscore-broken");
    f.write("Characters/_broken.md", "# Broken\n\nNo block.\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![(
            IssueKind::MissingCombatBlock,
            "Characters/_broken.md".into()
        )]
    );
}

#[test]
fn an_underscore_prefixed_orphan_sheet_is_not_skipped() {
    let f = Fixture::new("underscore-orphan");
    f.sheet("_ghost", "ghost", "Ghost", "Characters/ghost.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    // `_ghost` is not a valid character id and the sheet belongs to no card;
    // both are reported, and the file is not treated as absent.
    assert!(has(&r, IssueKind::InvalidId), "{:?}", r.issues);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::OrphanSheet && i.path == "sheets/_ghost.yaml"),
        "{:?}",
        r.issues
    );
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
    assert!(
        !issue.detail.contains("ghost"),
        "the id must not be echoed: {issue:?}"
    );
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
    assert!(
        !issue.detail.contains("bruni"),
        "the id must not be echoed: {issue:?}"
    );
}

#[test]
fn a_party_id_is_never_echoed() {
    let f = Fixture::new("party-secret");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.campaign(&["s3cr3t-party-id"]);

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous);
    let text = rendered(&r);
    assert!(!text.contains("s3cr3t-party-id"), "the id leaked: {text}");
}

#[test]
fn quoted_party_ids_and_comments_match_the_tools() {
    let f = Fixture::new("party-quoted");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.card("Characters/tess.md", "tess", "Tess");
    f.sheet("tess", "tess", "Tess", "Characters/tess.md");
    f.write(
        "combat/campaign.yaml",
        "name: \"fixture\"\nparty: [\"bruni\", 'tess']  # keep both\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
}

#[test]
fn a_block_list_party_is_incomplete() {
    let f = Fixture::new("party-block");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.write(
        "combat/campaign.yaml",
        "name: \"fixture\"\nparty:\n  - bruni\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    let issue = find(&r, IssueKind::UnparsableCampaignManifest);
    assert_eq!(issue.path, "combat/campaign.yaml");
    assert!(
        !issue.detail.contains("bruni"),
        "the id must not be echoed: {issue:?}"
    );
}

#[test]
fn party_forms_the_tools_cannot_read_are_incomplete() {
    // A top-level sequence instead of a mapping.
    let sequence = Fixture::new("party-sequence");
    sequence.write("combat/campaign.yaml", "- bruni\n- tess\n");
    let r = sequence.report();
    assert_eq!(r.status, Status::Incomplete);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );

    // `party` as a scalar, not a list.
    let scalar = Fixture::new("party-scalar");
    scalar.write("combat/campaign.yaml", "name: \"fixture\"\nparty: bruni\n");
    let r = scalar.report();
    assert_eq!(r.status, Status::Incomplete);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );

    // A space before the colon: YAML reads the list, the tools' line parser
    // does not, so the two disagree.
    let spaced = Fixture::new("party-spaced-key");
    spaced.card("Characters/bruni.md", "bruni", "Bruni");
    spaced.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    spaced.write(
        "combat/campaign.yaml",
        "name: \"fixture\"\nparty : [bruni]\n",
    );
    let r = spaced.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
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
fn a_manifest_that_is_a_directory_is_incomplete() {
    let f = Fixture::new("manifest-dir");
    f.dir("combat/campaign.yaml");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        r.issues.iter().all(|i| i.path == "combat/campaign.yaml"),
        "{:?}",
        r.issues
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_manifest_is_incomplete() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("manifest-symlink");
    let target = f.write("combat/real.yaml", "party: []\n");
    symlink(&target, f.save.join("combat/campaign.yaml")).unwrap();

    let before = snapshot_no_follow(&f.save);
    let link_before = link_state(&f.save.join("combat/campaign.yaml"));
    let r = f.diagnose();
    assert_eq!(
        link_before,
        link_state(&f.save.join("combat/campaign.yaml")),
        "the diagnostic must not replace the link"
    );
    assert_eq!(
        before,
        snapshot_no_follow(&f.save),
        "the link or its target was touched"
    );

    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        r.issues.iter().any(|i| i.path == "combat/campaign.yaml"),
        "{:?}",
        r.issues
    );
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
    let before = snapshot_no_follow(&f.save);
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
        snapshot_no_follow(&f.save),
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
    let before = snapshot_no_follow(&f.save);
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
        snapshot_no_follow(&f.save),
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

    let ids: Vec<&str> = first.characters.iter().map(|c| c.id()).collect();
    assert_eq!(ids, vec!["bruni", "tess", "tess"]);
    let paths: Vec<&str> = first.characters.iter().map(|c| c.markdown()).collect();
    assert_eq!(
        paths,
        vec!["Characters/bruni.md", "Characters/tess.md", "NPCs/tess.md"]
    );
}

#[test]
fn fixture_directories_are_unique_per_fixture() {
    let a = Fixture::new("same-name");
    let b = Fixture::new("same-name");
    assert_ne!(
        a.root, b.root,
        "a fixture must not reuse another fixture's dir"
    );
    assert!(a.root.is_dir() && b.root.is_dir());
    let a_root = a.root.clone();
    let b_root = b.root.clone();
    drop(a);
    drop(b);
    assert!(!a_root.exists(), "a fixture cleans up its own dir");
    assert!(!b_root.exists(), "a fixture cleans up its own dir");
}

#[test]
fn the_report_debug_does_not_contain_ids_or_names() {
    // The values a card carries are untrusted: the `Debug` view of the whole
    // report is what a log line or an error message shows, so it must redact
    // them. The raw values stay reachable through the documented accessors.
    let f = Fixture::new("debug-redaction");
    f.card("Characters/bruni.md", "s3cr3t-id-abc", "s3cr3t-name-xyz");
    f.sheet(
        "s3cr3t-id-abc",
        "s3cr3t-id-abc",
        "s3cr3t-name-xyz",
        "Characters/bruni.md",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
    let debug = format!("{r:?}");
    assert!(!debug.contains("s3cr3t-id-abc"), "id leaked: {debug}");
    assert!(!debug.contains("s3cr3t-name-xyz"), "name leaked: {debug}");
    // Each character's `Debug` is redacted too, not just the report's.
    let card = format!("{:?}", r.characters[0]);
    assert!(!card.contains("s3cr3t-id-abc"), "id leaked: {card}");
    assert!(!card.contains("s3cr3t-name-xyz"), "name leaked: {card}");
    // The restricted accessors still return the raw value for the caller.
    assert_eq!(r.characters[0].id(), "s3cr3t-id-abc");
    assert_eq!(r.characters[0].name(), "s3cr3t-name-xyz");
}

#[test]
fn a_dotfile_card_is_not_skipped() {
    let f = Fixture::new("dotfile-card");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.card("Characters/.hidden.md", "bruni", "Bruni Again");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous, "{:?}", r.issues);
    let issue = find(&r, IssueKind::DuplicateId);
    assert!(issue.detail.contains("Characters/.hidden.md"), "{issue:?}");
}

#[test]
fn a_dotfile_broken_card_is_not_skipped() {
    let f = Fixture::new("dotfile-broken");
    f.write("Characters/.broken.md", "# Broken\n\nNo block.\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete);
    assert_eq!(
        pairs(&r),
        vec![(
            IssueKind::MissingCombatBlock,
            "Characters/.broken.md".into()
        )]
    );
}

#[test]
fn a_dotfile_sheet_is_not_skipped() {
    let f = Fixture::new("dotfile-sheet");
    f.sheet(".ghost", "ghost", "Ghost", "Characters/ghost.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous, "{:?}", r.issues);
    assert!(has(&r, IssueKind::InvalidId), "{:?}", r.issues);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::OrphanSheet && i.path == "sheets/.ghost.yaml"),
        "{:?}",
        r.issues
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_character_directory_is_incomplete() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("symlink-cards");
    let target = f.root.join("outside/Characters");
    f.outside("Characters/ghost.md", &card("ghost", "Ghost"));
    f.outside(
        "Characters/sheets/ghost.yaml",
        &sheet_yaml("ghost", "Ghost", "Characters/ghost.md", 1),
    );
    symlink(&target, f.save.join("Characters")).unwrap();

    let save_before = snapshot_no_follow(&f.save);
    let target_before = snapshot_no_follow(&target);
    let link_before = link_state(&f.save.join("Characters"));
    let r = f.diagnose();
    assert_eq!(link_before, link_state(&f.save.join("Characters")));
    assert_eq!(
        target_before,
        snapshot_no_follow(&target),
        "the link target was touched"
    );
    assert_eq!(
        save_before,
        snapshot_no_follow(&f.save),
        "the save was touched"
    );

    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::UnreadableDirectory && i.path == "Characters"),
        "{:?}",
        r.issues
    );
    assert!(r.characters.is_empty(), "the target must not be read");
    assert!(
        !rendered(&r).contains("ghost"),
        "target content leaked: {}",
        rendered(&r)
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_sheets_directory_is_incomplete() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("symlink-sheets");
    let target = f.root.join("outside/sheets");
    f.outside(
        "sheets/ghost.yaml",
        &sheet_yaml("ghost", "Ghost", "Characters/ghost.md", 1),
    );
    symlink(&target, f.save.join("sheets")).unwrap();

    let save_before = snapshot_no_follow(&f.save);
    let target_before = snapshot_no_follow(&target);
    let link_before = link_state(&f.save.join("sheets"));
    let r = f.diagnose();
    assert_eq!(link_before, link_state(&f.save.join("sheets")));
    assert_eq!(
        save_before,
        snapshot_no_follow(&f.save),
        "the save was touched"
    );
    assert_eq!(
        target_before,
        snapshot_no_follow(&target),
        "the link target was touched"
    );

    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::UnreadableDirectory && i.path == "sheets"),
        "{:?}",
        r.issues
    );
    assert!(!has(&r, IssueKind::OrphanSheet), "{:?}", r.issues);
    assert!(
        !rendered(&r).contains("ghost"),
        "target content leaked: {}",
        rendered(&r)
    );
}

#[cfg(unix)]
#[test]
fn a_dangling_directory_symlink_is_incomplete() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("symlink-dangling");
    let target = f.root.join("missing-characters");
    symlink(&target, f.save.join("Characters")).unwrap();

    let link_before = link_state(&f.save.join("Characters"));
    assert_eq!(link_before, (true, Some(target.clone())));
    let save_before = snapshot_no_follow(&f.save);
    let r = f.diagnose();
    assert_eq!(
        link_before,
        link_state(&f.save.join("Characters")),
        "the diagnostic must not replace the link"
    );
    assert_eq!(
        save_before,
        snapshot_no_follow(&f.save),
        "the save was touched"
    );
    assert!(
        !target.exists(),
        "the diagnostic must not create the target"
    );

    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::UnreadableDirectory && i.path == "Characters"),
        "{:?}",
        r.issues
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_combat_directory_is_incomplete() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("symlink-combat");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    let target = f.root.join("outside/combat");
    f.outside("combat/campaign.yaml", "name: \"x\"\nparty: [ghost]\n");
    symlink(&target, f.save.join("combat")).unwrap();

    let save_before = snapshot_no_follow(&f.save);
    let target_before = snapshot_no_follow(&target);
    let link_before = link_state(&f.save.join("combat"));
    let r = f.diagnose();
    assert_eq!(link_before, link_state(&f.save.join("combat")));
    assert_eq!(
        save_before,
        snapshot_no_follow(&f.save),
        "the save was touched"
    );
    assert_eq!(
        target_before,
        snapshot_no_follow(&target),
        "the link target was touched"
    );

    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::UnreadableDirectory && i.path == "combat"),
        "{:?}",
        r.issues
    );
    assert!(!has(&r, IssueKind::PartyUnknownId), "{:?}", r.issues);
    assert!(
        !rendered(&r).contains("ghost"),
        "target content leaked: {}",
        rendered(&r)
    );
}

#[test]
fn an_empty_inline_party_list_is_ok() {
    let f = Fixture::new("party-empty-inline");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.write("combat/campaign.yaml", "name: \"fixture\"\nparty: []\n");

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
}

#[test]
fn a_spaced_party_key_is_incomplete_even_when_empty() {
    // Both readers end up with no ids, but the tools' line reader cannot see a
    // `party :` key at all: that form must not pass silently.
    let f = Fixture::new("party-spaced-empty");
    f.write("combat/campaign.yaml", "name: \"fixture\"\nparty : []\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
}

#[test]
fn an_empty_block_list_party_is_incomplete() {
    let f = Fixture::new("party-empty-block");
    f.write("combat/campaign.yaml", "name: \"fixture\"\nparty:\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
}

#[cfg(unix)]
#[test]
fn a_backslash_file_name_is_not_a_subdirectory_path() {
    // On Unix `Characters/a\b.md` is one file name; it must never be treated as
    // the two-component path `Characters/a/b.md` that a sheet could name.
    let f = Fixture::new("backslash");
    f.card(r"Characters/a\b.md", "ab", "Ab");
    f.sheet("ab", "ab", "Ab", "Characters/a/b.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ambiguous, "{:?}", r.issues);
    let issue = find(&r, IssueKind::SheetFileMismatch);
    assert_eq!(issue.path, "sheets/ab.yaml");
    assert_eq!(r.characters.len(), 1);
    assert_eq!(r.characters[0].markdown(), r"Characters/a\b.md");
}

// macOS (APFS) refuses to create a file name that is not valid UTF-8
// (`fs::write` returns EILSEQ), so this fixture cannot exist there. The same
// code path is covered on every Unix by
// `identity_diag::tests::display_path_never_fabricates_a_name`.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_non_utf8_file_name_is_incomplete_without_a_fake_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new("non-utf8-name");
    f.dir("Characters");
    let mut name = OsString::from_vec(vec![b'x', 0xff, 0xfe]);
    name.push(".md");
    fs::write(f.save.join("Characters").join(&name), card("x", "X")).unwrap();

    let before = snapshot_no_follow(&f.save);
    let r = f.diagnose();
    assert_eq!(before, snapshot_no_follow(&f.save), "the save was touched");

    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    let issue = find(&r, IssueKind::InvalidUtf8);
    assert_eq!(issue.path, "Characters", "the report names the directory");
    assert!(r.characters.is_empty(), "no identity from an unnamed file");
    assert!(
        !rendered(&r).contains('\u{fffd}'),
        "a lossy stand-in leaked: {}",
        rendered(&r)
    );
}

#[test]
fn a_nested_party_line_does_not_pass() {
    // The top-level key is `party ` (a space before the colon) and the only
    // `party:` line the tools find is indented under `other`.
    let f = Fixture::new("party-nested-empty");
    f.write("combat/campaign.yaml", "other:\n  party: []\nparty : []\n");

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
}

#[test]
fn a_nested_party_line_with_a_flow_value_does_not_pass() {
    let f = Fixture::new("party-nested-flow");
    f.write(
        "combat/campaign.yaml",
        "other:\n  party: []\nparty:\n  []\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
}

#[test]
fn nested_and_top_level_party_lists_that_agree_do_not_pass() {
    let f = Fixture::new("party-nested-same");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.write(
        "combat/campaign.yaml",
        "other:\n  party: [bruni]\nparty: [bruni]\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
    assert!(!has(&r, IssueKind::PartyUnknownId), "{:?}", r.issues);
}

#[test]
fn a_top_level_party_line_after_other_keys_is_ok() {
    let f = Fixture::new("party-top-level");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.write(
        "combat/campaign.yaml",
        "other:\n  name: x\nparty: [bruni]\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
}

#[cfg(unix)]
#[test]
fn a_sheet_that_names_the_real_backslash_file_binds() {
    // Positive control for the case above: naming the literal file does bind.
    let f = Fixture::new("backslash-binds");
    f.card(r"Characters/a\b.md", "ab", "Ab");
    f.sheet("ab", "ab", "Ab", r"Characters/a\b.md");

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
    assert_eq!(r.characters[0].sheet(), Some("sheets/ab.yaml"));
    assert_eq!(r.characters[0].markdown(), r"Characters/a\b.md");
}

#[test]
fn a_multiline_scalar_party_line_does_not_pass() {
    // The line the tools match is inside a double-quoted scalar, not a mapping
    // key: the manifest's top-level `party` comes from `party : []`, and both
    // readers end up with an empty list.
    let f = Fixture::new("party-multiline-empty");
    f.write(
        "combat/campaign.yaml",
        "note: \"hello\nparty: []\nworld\"\nparty : []\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
}

#[test]
fn a_multiline_scalar_party_line_with_ids_does_not_pass() {
    let f = Fixture::new("party-multiline-ids");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.write(
        "combat/campaign.yaml",
        "note: \"hello\nparty: [bruni]\nworld\"\nparty : [bruni]\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
    assert!(!has(&r, IssueKind::PartyUnknownId), "{:?}", r.issues);
}

#[test]
fn a_multiline_scalar_party_with_a_real_party_line_is_incomplete() {
    // Two column-0 `party:` lines: one inside the scalar, one the real key.
    let f = Fixture::new("party-multiline-twice");
    f.write(
        "combat/campaign.yaml",
        "note: \"hello\nparty: []\nworld\"\nparty: []\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Incomplete, "{:?}", r.issues);
    assert!(
        has(&r, IssueKind::UnparsableCampaignManifest),
        "{:?}",
        r.issues
    );
}

#[test]
fn an_inline_party_list_is_ok() {
    let f = Fixture::new("party-inline-two");
    f.card("Characters/bruni.md", "bruni", "Bruni");
    f.sheet("bruni", "bruni", "Bruni", "Characters/bruni.md");
    f.card("Characters/tess.md", "tess", "Tess");
    f.sheet("tess", "tess", "Tess", "Characters/tess.md");
    f.write(
        "combat/campaign.yaml",
        "name: \"fixture\"\nparty: [bruni, tess]\n",
    );

    let r = f.report();
    assert_eq!(r.status, Status::Ok, "{:?}", r.issues);
}
