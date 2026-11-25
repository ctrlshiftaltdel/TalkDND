//! Read-only preview for binding old Markdown-only party cards to advancement.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};

use crate::identity_diag::{self, Status};

const PREFIX: &str = "<!-- talkdnd-identity: ";
const SUFFIX: &str = " -->";
const PREVIEW_TOKEN: &str = "00000000000000000000000000000000";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    card: String,
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
    candidate.extend_from_slice(format!("{ending}{PREFIX}{token}{SUFFIX}{ending}").as_bytes());
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

fn inspect(save: &Path) -> Result<Preview> {
    let diagnostic = identity_diag::diagnose(save);
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
            .into_iter()
            .map(|issue| DiscoveryIssue {
                path: issue.path,
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
            status: "structured_card",
            before_digest: None,
            added_bytes: None,
        };
        let record_path = records.remove(&format!("{}.yaml", character.id()));
        if character.sheet().is_some() {
            if record_path.is_some() {
                row.status = "structured_record_unverified";
            }
            preview.cards.push(row);
            continue;
        }
        let (status, binding) = inspect_markdown(
            &save.join(character.markdown_path()),
            &mut row,
            record_path.as_deref(),
        )?;
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

fn inspect_markdown(
    path: &Path,
    row: &mut Card,
    record_path: Option<&Path>,
) -> Result<(&'static str, BTreeSet<String>)> {
    let original = read_regular(path)?;
    let mut tokens = BTreeSet::new();
    let token = match card_token(&original) {
        Ok(token) => token,
        Err(_) => return Ok(("invalid_marker", tokens)),
    };
    if let Some(token) = token {
        tokens.insert(token.to_owned());
    }
    let record = match record_path.map(load_record).transpose() {
        Ok(record) => record,
        Err(_) => return Ok(("invalid_record", tokens)),
    };
    if let Some(record) = &record {
        tokens.insert(record.token.clone());
    }
    let status = match (token, record) {
        (None, None) => {
            let proposed = appended_card(&original, PREVIEW_TOKEN)?;
            row.before_digest = Some(advancement_lock::save_lock::digest(&original));
            row.added_bytes = Some(proposed.len() - original.len());
            "review_required"
        }
        (Some(_), None) => "record_missing",
        (None, Some(_)) => "marker_missing",
        (Some(token), Some(record)) => match record.history {
            History::Unknown if record.card == row.path && record.token == token => "matched",
            History::Unknown => "binding_mismatch",
        },
    };
    Ok((status, tokens))
}

pub fn run(save: &Path, json: bool) -> Result<()> {
    let preview = inspect(save)?;
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
        println!("{} {}", card.path, card.status);
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
}
