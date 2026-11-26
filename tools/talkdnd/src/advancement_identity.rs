//! Read-only preview of lossless identity binding for existing party cards.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};

use crate::identity_diag::{self, Status};

const PREFIX: &str = "<!-- talkdnd-identity: ";
const SUFFIX: &str = " -->";
const PREVIEW_TOKEN: &str = "00000000000000000000000000000000";
pub(crate) const SHEET_FIELD: &str = "talkdnd_identity";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    card: String,
    #[serde(default)]
    carrier: Option<String>,
    token: String,
    history: History,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum History {
    Unknown,
}

#[derive(Serialize)]
struct Preview {
    consistency: &'static str,
    write_capability: &'static str,
    discovery: &'static str,
    issues: Vec<DiscoveryIssue>,
    cards: Vec<Card>,
    unmatched_records: Vec<String>,
}

#[derive(Serialize)]
struct DiscoveryIssue {
    path: String,
    kind: String,
}

#[derive(Serialize)]
struct Card {
    path: String,
    carrier: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    added_bytes: Option<usize>,
}

fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn card_token(original: &[u8]) -> Result<Option<&str>> {
    let text = std::str::from_utf8(original)?;
    let mut markers = text
        .lines()
        .filter(|line| line.contains("talkdnd-identity:"));
    let Some(line) = markers.next() else {
        return Ok(None);
    };
    ensure!(markers.next().is_none(), "duplicate identity markers");
    let Some(token) = line
        .strip_prefix(PREFIX)
        .and_then(|line| line.strip_suffix(SUFFIX))
    else {
        bail!("invalid identity marker");
    };
    ensure!(valid_token(token), "invalid identity token format");
    Ok(Some(token))
}

fn appended_card(original: &[u8], token: &str) -> Result<Vec<u8>> {
    ensure!(valid_token(token), "invalid identity token format");
    ensure!(
        card_token(original)?.is_none(),
        "card already carries an identity"
    );
    Ok(append_line(
        original,
        &format!("{PREFIX}{token}{SUFFIX}"),
        true,
    ))
}

fn append_line(original: &[u8], line: &str, separate_paragraph: bool) -> Vec<u8> {
    let last_newline = original.iter().rposition(|byte| *byte == b'\n');
    let ending = if last_newline
        .and_then(|i| i.checked_sub(1))
        .is_some_and(|i| original[i] == b'\r')
    {
        "\r\n"
    } else {
        "\n"
    };
    let mut candidate = original.to_vec();
    if !candidate.ends_with(b"\n") {
        candidate.extend_from_slice(ending.as_bytes());
    }
    if separate_paragraph {
        candidate.extend_from_slice(ending.as_bytes());
    }
    candidate.extend_from_slice(line.as_bytes());
    candidate.extend_from_slice(ending.as_bytes());
    candidate
}

fn sheet_identity(original: &[u8]) -> Result<(serde_norway::Mapping, Option<String>)> {
    let mut sheet: serde_norway::Mapping = serde_norway::from_slice(original)
        .map_err(|_| anyhow::anyhow!("invalid structured character card"))?;
    let Some(token) = sheet.remove(SHEET_FIELD) else {
        return Ok((sheet, None));
    };
    let serde_norway::Value::String(token) = token else {
        bail!("invalid structured identity field");
    };
    ensure!(valid_token(&token), "invalid identity token format");
    Ok((sheet, Some(token)))
}

fn appended_sheet(original: &[u8], token: &str) -> Result<Vec<u8>> {
    ensure!(valid_token(token), "invalid identity token format");
    let (before, existing) = sheet_identity(original)?;
    ensure!(existing.is_none(), "sheet already carries an identity");
    let candidate = append_line(original, &format!("{SHEET_FIELD}: \"{token}\""), false);
    let (after, inserted) = sheet_identity(&candidate)?;
    ensure!(
        before == after && inserted.as_deref() == Some(token),
        "cannot attach an identity without changing existing YAML values"
    );
    Ok(candidate)
}

fn read_regular(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "expected a regular file"
    );
    Ok(fs::read(path)?)
}

fn collect_records(save: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let directory = save.join("advancement");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => bail!("advancement must be a regular directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    }
    fs::read_dir(&directory)?
        .map(|entry| {
            let entry = entry?;
            let filename = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid advancement record filename"))?;
            Ok((filename, entry.path()))
        })
        .collect()
}

fn load_record(path: &Path) -> Result<Record> {
    let data = read_regular(path)?;
    let record: Record = serde_norway::from_slice(&data)
        .map_err(|_| anyhow::anyhow!("invalid advancement record"))?;
    ensure!(
        record.version == 1 && valid_token(&record.token),
        "invalid advancement record"
    );
    Ok(record)
}

fn inspect_discovered(save: &Path, diagnostic: &identity_diag::Report) -> Result<Preview> {
    let mut preview = Preview {
        consistency: "unverified",
        write_capability: "unverified",
        discovery: match diagnostic.status {
            Status::Ok => "ok",
            Status::Unknown => "unknown",
            Status::Ambiguous => "ambiguous",
            Status::Incomplete => "incomplete",
        },
        issues: diagnostic
            .issues
            .iter()
            .map(|issue| DiscoveryIssue {
                path: issue.path.clone(),
                kind: format!("{:?}", issue.kind),
            })
            .collect(),
        cards: Vec::new(),
        unmatched_records: Vec::new(),
    };
    if diagnostic.status >= Status::Ambiguous {
        return Ok(preview);
    }
    let mut records = collect_records(save)?;
    let mut tokens: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for character in diagnostic
        .characters
        .iter()
        .filter(|character| character.markdown_path().starts_with("Characters"))
    {
        let mut row = Card {
            path: character.markdown().to_string(),
            carrier: character
                .sheet()
                .unwrap_or_else(|| character.markdown())
                .to_string(),
            status: "unverified",
            before_digest: None,
            added_bytes: None,
        };
        let record_path = records.remove(&format!("{}.yaml", character.id()));
        let (status, binding) = inspect_carrier(save, character, &mut row, record_path.as_deref())?;
        row.status = status;
        for token in binding {
            tokens.entry(token).or_default().push(preview.cards.len());
        }
        preview.cards.push(row);
    }
    for indexes in tokens.values().filter(|indexes| indexes.len() > 1) {
        for index in indexes {
            preview.cards[*index].status = "duplicate_token";
        }
    }
    preview.unmatched_records = records.into_keys().collect();
    Ok(preview)
}

pub(crate) fn binding_status(
    save: &Path,
    diagnostic: &identity_diag::Report,
    character: &identity_diag::Character,
) -> &'static str {
    inspect_discovered(save, diagnostic)
        .ok()
        .and_then(|preview| {
            preview
                .cards
                .into_iter()
                .find(|card| card.path == character.markdown())
        })
        .map_or("unknown", |card| card.status)
}

fn inspect_carrier(
    save: &Path,
    character: &identity_diag::Character,
    row: &mut Card,
    record_path: Option<&Path>,
) -> Result<(&'static str, BTreeSet<String>)> {
    let markdown = read_regular(&save.join(character.markdown_path()))?;
    let mut tokens = BTreeSet::new();
    let markdown_token = match card_token(&markdown) {
        Ok(token) => token.map(str::to_owned),
        Err(_) => return Ok(("invalid_marker", tokens)),
    };
    let structured = character.sheet_path();
    let (original, token) = if let Some(path) = structured {
        if let Some(token) = markdown_token {
            tokens.insert(token);
            return Ok(("carrier_conflict", tokens));
        }
        let sheet = read_regular(&save.join(path))?;
        match sheet_identity(&sheet) {
            Ok((_, token)) => (sheet, token),
            Err(_) => return Ok(("invalid_marker", tokens)),
        }
    } else {
        (markdown, markdown_token)
    };
    if let Some(token) = &token {
        tokens.insert(token.clone());
    }
    let record = match record_path.map(load_record).transpose() {
        Ok(record) => record,
        Err(_) => return Ok(("invalid_record", tokens)),
    };
    if let Some(record) = &record {
        tokens.insert(record.token.clone());
    }
    let status = match (token.as_deref(), record) {
        (None, None) => {
            let proposed = if structured.is_some() {
                appended_sheet(&original, PREVIEW_TOKEN)
            } else {
                appended_card(&original, PREVIEW_TOKEN)
            };
            let Ok(proposed) = proposed else {
                return Ok(("format_review_required", tokens));
            };
            row.before_digest = Some(advancement_lock::save_lock::digest(&original));
            row.added_bytes = Some(proposed.len() - original.len());
            "review_required"
        }
        (Some(_), None) => "record_missing",
        (None, Some(_)) => "marker_missing",
        (Some(token), Some(record)) => match record.history {
            History::Unknown
                if record.card == row.path
                    && record.token == token
                    && record.carrier.as_deref().unwrap_or(&record.card) == row.carrier =>
            {
                "matched"
            }
            History::Unknown => "binding_mismatch",
        },
    };
    Ok((status, tokens))
}

pub fn run(save: &Path, json: bool) -> Result<()> {
    let preview = inspect_discovered(save, &identity_diag::diagnose(save))?;
    if json {
        println!("{}", serde_json::to_string(&preview)?);
        return Ok(());
    }
    println!(
        "身份预览 · 发现状态 {} · 一致性未验证 · 可写性未验证",
        preview.discovery
    );
    for issue in preview.issues {
        println!("{} {}", issue.path, issue.kind);
    }
    for card in preview.cards {
        println!("{} {} {}", card.path, card.carrier, card.status);
    }
    for record in preview.unmatched_records {
        println!("未匹配的成长记录 {record}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_preserves_bom_crlf_and_unterminated_original_bytes() {
        let original = b"\xef\xbb\xbf# Card\r\n\r\nhand edited";
        let proposed = appended_card(original, PREVIEW_TOKEN).unwrap();
        assert!(proposed.starts_with(original));
        let added = std::str::from_utf8(&proposed[original.len()..]).unwrap();
        assert_eq!(
            added,
            format!("\r\n\r\n{PREFIX}{PREVIEW_TOKEN}{SUFFIX}\r\n")
        );
        assert_eq!(card_token(&proposed).unwrap(), Some(PREVIEW_TOKEN));
        assert!(appended_card(&proposed, PREVIEW_TOKEN).is_err());
    }

    #[test]
    fn structured_append_preserves_comments_unknown_values_and_original_bytes() {
        let original =
            b"\xef\xbb\xbf# user comment\r\nid: fixture\r\ncustom: {name: \"old\", enabled: true}";
        let proposed = appended_sheet(original, PREVIEW_TOKEN).unwrap();
        assert!(proposed.starts_with(original));
        let (before, _) = sheet_identity(original).unwrap();
        let (after, identity) = sheet_identity(&proposed).unwrap();
        assert_eq!(before, after);
        assert_eq!(identity.as_deref(), Some(PREVIEW_TOKEN));
        assert!(appended_sheet(&proposed, PREVIEW_TOKEN).is_err());
    }

    #[test]
    fn structured_append_refuses_a_second_document_or_changed_scalar() {
        assert!(appended_sheet(b"id: fixture\n...\n", PREVIEW_TOKEN).is_err());
        assert!(appended_sheet(b"id: fixture\ntext: |+\n  prose", PREVIEW_TOKEN).is_err());
        assert!(appended_sheet(b"id: fixture\ntext: |+\n  prose\n", PREVIEW_TOKEN).is_ok());
    }
}
