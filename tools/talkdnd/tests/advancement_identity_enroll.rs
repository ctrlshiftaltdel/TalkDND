//! First enrollment with synthetic files. Candidate construction is read-only.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use advancement_lock::save_lock::digest;
use serde_norway::{Mapping, Value};
use talkdnd::advancement_commit::{
    prepare_first_enrollment, EnrollmentApproval, EnrollmentCandidate, EnrollmentRequest,
};
use talkdnd::identity_diag::{self, Character};

const CARD_PATH: &str = "Characters/Fixture.md";
const SHEET_PATH: &str = "sheets/fixture.yaml";
const SIDECAR_PATH: &str = "advancement/fixture.yaml";
const CARD: &[u8] = b"\xef\xbb\xbf# Fixture\r\n\r\n```combat yaml\r\nid: fixture\r\nname: Fixture\r\n```\r\n\r\nDM note";
const SHEET: &[u8] = b"\xef\xbb\xbf# DM note\r\nversion: 1\r\nid: fixture\r\nname: Fixture\r\nfile: Characters/Fixture.md\r\ncustom: {old: [4, 8], present: true}";
const TOKEN: &str = "11111111111111111111111111111111";
const ZERO: &str = "00000000000000000000000000000000";

struct Fixture {
    root: PathBuf,
    save: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        for n in 0..64 {
            let root =
                std::env::temp_dir().join(format!("talkdnd-enrollment-{}-{n}", std::process::id()));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let save = root.join("Saves/2030-01-01-01");
                    for dir in ["Characters", "sheets", "advancement", "combat"] {
                        fs::create_dir_all(save.join(dir)).unwrap();
                    }
                    fs::write(root.join("AGENTS.md"), "Synthetic play root\n").unwrap();
                    fs::write(
                        save.join("combat/campaign.yaml"),
                        "name: fixture\nruleset: 5.5e-srd\nparty: [fixture]\n",
                    )
                    .unwrap();
                    return Self { root, save };
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create enrollment fixture: {error}"),
            }
        }
        panic!("no free enrollment fixture directory");
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        fs::write(self.save.join(path), bytes).unwrap();
    }

    fn character(&self) -> Character {
        let mut report = identity_diag::diagnose(&self.save);
        assert_eq!(report.characters.len(), 1);
        report.characters.pop().unwrap()
    }

    fn assert_unchanged(&self, card: &[u8], sheet: Option<&[u8]>, sidecar: Option<&[u8]>) {
        assert_eq!(fs::read(self.save.join(CARD_PATH)).unwrap(), card);
        for (path, bytes) in [(SHEET_PATH, sheet), (SIDECAR_PATH, sidecar)] {
            match bytes {
                Some(original) => assert_eq!(fs::read(self.save.join(path)).unwrap(), original),
                None => assert!(!self.save.join(path).exists()),
            }
        }
    }

    fn assert_candidate_is_read_as_matched(&self, candidate: EnrollmentCandidate, token: &str) {
        fs::write(
            self.save.join(candidate.carrier_path),
            candidate.carrier_bytes,
        )
        .unwrap();
        fs::write(
            self.save.join(candidate.sidecar_path),
            candidate.sidecar_bytes,
        )
        .unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_talkdnd"))
            .arg("--root")
            .arg(&self.root)
            .args(["advancement", "identity-preview", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let preview: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(preview["cards"][0]["status"], "matched");
        assert_eq!(preview["consistency"], "unverified");
        assert_eq!(preview["write_capability"], "unverified");
        assert!(!preview.to_string().contains(token));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_sidecar(candidate: &EnrollmentCandidate, structured: bool) -> String {
    assert_eq!(candidate.sidecar_path, Path::new(SIDECAR_PATH));
    let sidecar: Mapping = serde_norway::from_slice(&candidate.sidecar_bytes).unwrap();
    assert_eq!(sidecar.len(), if structured { 5 } else { 4 });
    assert_eq!(sidecar.get("version").and_then(Value::as_u64), Some(1));
    assert_eq!(sidecar.get("card").and_then(Value::as_str), Some(CARD_PATH));
    assert_eq!(
        sidecar.get("history").and_then(Value::as_str),
        Some("unknown")
    );
    assert_eq!(
        sidecar.get("carrier").and_then(Value::as_str),
        structured.then_some(SHEET_PATH)
    );
    let token = sidecar.get("token").and_then(Value::as_str).unwrap();
    assert_eq!(token.len(), 32);
    assert!(token
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert_ne!(token, ZERO);
    assert_ne!(token, TOKEN);
    token.to_owned()
}

#[test]
fn markdown_v1_candidate_preserves_original_and_matches_existing_preview_reader() {
    let fixture = Fixture::new();
    fixture.write(CARD_PATH, CARD);
    let character = fixture.character();
    let candidate = prepare_first_enrollment(EnrollmentRequest {
        schema: 9,
        character: &character,
        markdown: CARD,
        sheet: None,
        sidecar_exists: false,
        occupied_tokens: &[TOKEN],
        approval: Some(EnrollmentApproval {
            markdown: (CARD_PATH, &digest(CARD)),
            sheet: None,
        }),
    })
    .unwrap();
    assert_eq!(candidate.carrier_path, Path::new(CARD_PATH));
    let token = assert_sidecar(&candidate, false);
    let suffix = candidate.carrier_bytes.strip_prefix(CARD).unwrap();
    assert_eq!(
        suffix,
        format!("\r\n\r\n<!-- talkdnd-identity: {token} -->\r\n").as_bytes()
    );
    fixture.assert_unchanged(CARD, None, None);
    fixture.assert_candidate_is_read_as_matched(candidate, &token);
}

#[test]
fn structured_v1_candidate_retains_yaml_values_bom_comments_and_unknown_keys() {
    let fixture = Fixture::new();
    fixture.write(CARD_PATH, CARD);
    fixture.write(SHEET_PATH, SHEET);
    let character = fixture.character();
    let candidate = prepare_first_enrollment(EnrollmentRequest {
        schema: 10,
        character: &character,
        markdown: CARD,
        sheet: Some(SHEET),
        sidecar_exists: false,
        occupied_tokens: &[TOKEN],
        approval: Some(EnrollmentApproval {
            markdown: (CARD_PATH, &digest(CARD)),
            sheet: Some((SHEET_PATH, &digest(SHEET))),
        }),
    })
    .unwrap();
    assert_eq!(candidate.carrier_path, Path::new(SHEET_PATH));
    let token = assert_sidecar(&candidate, true);
    let suffix = candidate.carrier_bytes.strip_prefix(SHEET).unwrap();
    assert_eq!(
        suffix,
        format!("\r\ntalkdnd_identity: \"{token}\"\r\n").as_bytes()
    );
    let old: Mapping = serde_norway::from_slice(SHEET).unwrap();
    let mut new: Mapping = serde_norway::from_slice(&candidate.carrier_bytes).unwrap();
    assert_eq!(
        new.remove("talkdnd_identity"),
        Some(Value::String(token.clone()))
    );
    assert_eq!(new, old);
    fixture.assert_unchanged(CARD, Some(SHEET), None);
    fixture.assert_candidate_is_read_as_matched(candidate, &token);
}

#[test]
fn enrollment_refuses_missing_or_stale_approval_and_existing_tokens_or_sidecar() {
    let fixture = Fixture::new();
    fixture.write(CARD_PATH, CARD);
    let character = fixture.character();
    let card_digest = digest(CARD);
    let request = || EnrollmentRequest {
        schema: 10,
        character: &character,
        markdown: CARD,
        sheet: None,
        sidecar_exists: false,
        occupied_tokens: &[],
        approval: Some(EnrollmentApproval {
            markdown: (CARD_PATH, &card_digest),
            sheet: None,
        }),
    };

    for (input, message) in [
        (
            EnrollmentRequest {
                approval: None,
                ..request()
            },
            "DM confirmation is required",
        ),
        (
            EnrollmentRequest {
                approval: Some(EnrollmentApproval {
                    markdown: (CARD_PATH, "stale"),
                    sheet: None,
                }),
                ..request()
            },
            "DM confirmation no longer matches",
        ),
        (
            EnrollmentRequest {
                occupied_tokens: &[TOKEN, TOKEN],
                ..request()
            },
            "duplicate identity token",
        ),
        (
            EnrollmentRequest {
                occupied_tokens: &[ZERO],
                ..request()
            },
            "invalid existing identity token",
        ),
        (
            EnrollmentRequest {
                schema: 11,
                ..request()
            },
            "first enrollment requires schema 9 or 10",
        ),
    ] {
        assert!(prepare_first_enrollment(input)
            .err()
            .unwrap()
            .to_string()
            .contains(message));
        fixture.assert_unchanged(CARD, None, None);
    }

    let existing = b"legacy sidecar with no usable binding";
    fixture.write(SIDECAR_PATH, existing);
    let invalid = EnrollmentRequest {
        sidecar_exists: true,
        ..request()
    };
    assert!(prepare_first_enrollment(invalid)
        .err()
        .unwrap()
        .to_string()
        .contains("sidecar already exists"));
    fixture.assert_unchanged(CARD, None, Some(existing));
}

#[test]
fn enrollment_refuses_single_sided_and_ambiguous_markdown_markers() {
    let fixture = Fixture::new();
    fixture.write(CARD_PATH, CARD);
    let character = fixture.character();
    for addition in [
        format!("\r\n\r\n<!-- talkdnd-identity: {TOKEN} -->"),
        format!("\r\n\r\n<!-- TALKDND-IDENTITY : {TOKEN} -->"),
        "\r\n\r\n~~~~example\r\nopen fence".to_string(),
        "\r\n\r\n<script>\r\nunfinished raw HTML".to_string(),
    ] {
        let mut bytes = CARD.to_vec();
        bytes.extend_from_slice(addition.as_bytes());
        fixture.write(CARD_PATH, &bytes);
        assert!(prepare_first_enrollment(EnrollmentRequest {
            schema: 10,
            character: &character,
            markdown: &bytes,
            sheet: None,
            sidecar_exists: false,
            occupied_tokens: &[],
            approval: Some(EnrollmentApproval {
                markdown: (CARD_PATH, &digest(&bytes)),
                sheet: None
            }),
        })
        .is_err());
        fixture.assert_unchanged(&bytes, None, None);
    }
}

#[test]
fn enrollment_refuses_existing_structured_tokens_and_lossy_yaml_appends() {
    let fixture = Fixture::new();
    fixture.write(CARD_PATH, CARD);
    fixture.write(SHEET_PATH, SHEET);
    let character = fixture.character();
    for addition in [
        format!("\r\ntalkdnd_identity: \"{TOKEN}\"\r\n"),
        "\r\n...\r\n".to_string(),
        "\r\n---\r\nother: document\r\n".to_string(),
        "\r\ncustom: repeated\r\n".to_string(),
        "\r\ntext: |+\r\n  unterminated scalar".to_string(),
    ] {
        let mut bytes = SHEET.to_vec();
        bytes.extend_from_slice(addition.as_bytes());
        fixture.write(SHEET_PATH, &bytes);
        assert!(prepare_first_enrollment(EnrollmentRequest {
            schema: 10,
            character: &character,
            markdown: CARD,
            sheet: Some(&bytes),
            sidecar_exists: false,
            occupied_tokens: &[],
            approval: Some(EnrollmentApproval {
                markdown: (CARD_PATH, &digest(CARD)),
                sheet: Some((SHEET_PATH, &digest(&bytes))),
            }),
        })
        .is_err());
        fixture.assert_unchanged(CARD, Some(&bytes), None);
    }
}
