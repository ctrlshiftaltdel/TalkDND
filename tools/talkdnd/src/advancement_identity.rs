//! Read-only previews and unpublished first-enrollment candidates for party cards.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use advancement_lock::save_lock::digest;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use crate::identity_diag::{self, Status};

const PREFIX: &str = "<!-- talkdnd-identity: ";
const SUFFIX: &str = " -->";
const PREVIEW_TOKEN: &str = "00000000000000000000000000000000";
pub(crate) const SHEET_FIELD: &str = "talkdnd_identity";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub(crate) version: u32,
    pub(crate) card: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) carrier: Option<String>,
    pub(crate) token: String,
    pub(crate) history: History,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum History {
    Unknown,
}

/// The files and exact preimages that the DM reviewed for first enrollment.
/// Each entry pairs the save-relative locator with the digest of the old bytes.
pub struct EnrollmentApproval<'a> {
    pub markdown: (&'a str, &'a str),
    pub sheet: Option<(&'a str, &'a str)>,
}

/// Inputs from the caller's discovery and preimage validation. The caller must
/// check the complete save under SAVE, verify the backup, and supply current
/// bytes for this discovered character. A bound carrier and sidecar count as
/// one occupied token. Include other tokens reserved by the current operation.
pub struct EnrollmentRequest<'a> {
    pub schema: u32,
    pub character: &'a identity_diag::Character,
    pub markdown: &'a [u8],
    pub sheet: Option<&'a [u8]>,
    pub sidecar_exists: bool,
    pub occupied_tokens: &'a [&'a str],
    pub approval: Option<EnrollmentApproval<'a>>,
}

/// Save-relative target paths and complete proposed bytes for D to stage.
/// These bytes contain save data and must stay out of logs.
pub struct EnrollmentCandidate {
    pub carrier_path: PathBuf,
    pub carrier_bytes: Vec<u8>,
    pub sidecar_path: PathBuf,
    pub sidecar_bytes: Vec<u8>,
}

/// Build both sides of a strict v1 binding using operating-system entropy.
/// This operation only builds bytes in memory. The caller owns filesystem
/// admission, complete enumeration, preimage rechecks, backup and publication.
pub fn prepare_first_enrollment(request: EnrollmentRequest<'_>) -> Result<EnrollmentCandidate> {
    prepare_with_entropy(request, |bytes| {
        combat_sim::dice::try_fill_os_random(bytes).context("operating-system entropy unavailable")
    })
}

fn prepare_with_entropy(
    request: EnrollmentRequest<'_>,
    mut fill: impl FnMut(&mut [u8; 16]) -> Result<()>,
) -> Result<EnrollmentCandidate> {
    let character = request.character;
    let markdown_path = character.markdown_path();
    let sheet_path = character.sheet_path();
    let id = character.id();
    ensure!(
        matches!(request.schema, 9 | 10),
        "first enrollment requires schema 9 or 10"
    );
    ensure!(
        markdown_path.starts_with("Characters") && crate::chargen::valid_id(id),
        "first enrollment requires a valid party character"
    );
    ensure!(
        request.sheet.is_some() == sheet_path.is_some()
            && sheet_path.is_none_or(|path| path == Path::new("sheets").join(format!("{id}.yaml"))),
        "structured carrier does not match discovery"
    );
    ensure!(
        !request.sidecar_exists,
        "advancement sidecar already exists"
    );

    let approval = request.approval.context("DM confirmation is required")?;
    let markdown_digest = digest(request.markdown);
    let sheet_digest = request.sheet.map(digest);
    ensure!(
        approval.markdown == (character.markdown(), markdown_digest.as_str())
            && approval.sheet == character.sheet().zip(sheet_digest.as_deref()),
        "DM confirmation no longer matches the reviewed character files"
    );

    ensure!(
        card_token(request.markdown)?.is_none(),
        "card already carries an identity"
    );
    ensure!(
        !std::str::from_utf8(request.markdown)?.lines().any(|line| {
            line.contains("<!--") && line.to_ascii_lowercase().contains("talkdnd-identity")
        }),
        "invalid identity marker"
    );
    if let Some(sheet) = request.sheet {
        ensure!(
            sheet_identity(sheet)?.1.is_none(),
            "sheet already carries an identity"
        );
    } else {
        ensure_standalone_marker(request.markdown)?;
    }

    let mut occupied = BTreeSet::new();
    for &token in request.occupied_tokens {
        ensure!(
            valid_token(token) && token != PREVIEW_TOKEN,
            "invalid existing identity token"
        );
        ensure!(occupied.insert(token), "duplicate identity token in save");
    }
    let token = generate_token(&occupied, &mut fill)?;
    let carrier_bytes = match request.sheet {
        Some(original) => appended_sheet(original, &token)?,
        None => appended_card(request.markdown, &token)?,
    };
    let record = Record {
        version: 1,
        card: character.markdown().to_owned(),
        carrier: character.sheet().map(str::to_owned),
        token,
        history: History::Unknown,
    };
    Ok(EnrollmentCandidate {
        carrier_path: sheet_path.unwrap_or(markdown_path).to_path_buf(),
        carrier_bytes,
        sidecar_path: Path::new("advancement").join(format!("{id}.yaml")),
        sidecar_bytes: serde_norway::to_string(&record)?.into_bytes(),
    })
}

fn generate_token(
    occupied: &BTreeSet<&str>,
    fill: &mut impl FnMut(&mut [u8; 16]) -> Result<()>,
) -> Result<String> {
    for _ in 0..8 {
        let mut entropy = [0; 16];
        fill(&mut entropy)?;
        let mut token = String::with_capacity(32);
        for byte in entropy {
            write!(token, "{byte:02x}").expect("writing to a String cannot fail");
        }
        if token != PREVIEW_TOKEN && !occupied.contains(token.as_str()) {
            return Ok(token);
        }
    }
    bail!("identity token generation exhausted eight attempts");
}

// The appended blank line ends paragraphs and blank-line-terminated HTML
// blocks. Fenced code and raw HTML blocks with explicit terminators can run
// through that blank line, so check their state before adding the comment.
fn ensure_standalone_marker(original: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(original)?.trim_start_matches('\u{feff}');
    let mut fence: Option<(u8, usize)> = None;
    let mut html_end: Option<&str> = None;
    for line in text.lines() {
        if let Some(end) = html_end {
            if line.to_ascii_lowercase().contains(end) {
                html_end = None;
            }
            continue;
        }
        let body = line.trim_start_matches(' ');
        if line.len() - body.len() > 3 {
            continue;
        }
        let fence_byte = body.as_bytes().first().copied().unwrap_or_default();
        let run = body.bytes().take_while(|&byte| byte == fence_byte).count();
        if let Some((opener, length)) = fence {
            if fence_byte == opener && run >= length && body[run..].trim().is_empty() {
                fence = None;
            }
        } else if matches!(fence_byte, b'`' | b'~')
            && run >= 3
            && (fence_byte != b'`' || !body[run..].contains('`'))
        {
            fence = Some((fence_byte, run));
        } else if let Some(end) = html_block_terminator(body) {
            if !body.to_ascii_lowercase().contains(end) {
                html_end = Some(end);
            }
        }
    }
    ensure!(
        fence.is_none() && html_end.is_none(),
        "identity marker would be inside an unfinished Markdown block"
    );
    Ok(())
}

fn html_block_terminator(body: &str) -> Option<&'static str> {
    let rest = body.strip_prefix('<')?;
    if rest.starts_with("!--") {
        return Some("-->");
    }
    if rest.starts_with("![CDATA[") {
        return Some("]]>");
    }
    if rest.starts_with('?') {
        return Some("?>");
    }
    if rest.starts_with('!') && rest.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) {
        return Some(">");
    }
    let rest = rest.to_ascii_lowercase();
    [
        ("pre", "</pre>"),
        ("script", "</script>"),
        ("style", "</style>"),
        ("textarea", "</textarea>"),
    ]
    .into_iter()
    .find_map(|(tag, end)| {
        rest.strip_prefix(tag)
            .filter(|suffix| suffix.starts_with([' ', '\t', '>']) || suffix.is_empty())
            .map(|_| end)
    })
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

pub(crate) fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn card_token(original: &[u8]) -> Result<Option<&str>> {
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

pub(crate) fn sheet_identity(original: &[u8]) -> Result<(serde_norway::Mapping, Option<String>)> {
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

pub(crate) fn read_regular(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_file(),
        "expected a regular file"
    );
    Ok(fs::read(path)?)
}

// Crate-internal view for the schema-11 migration planner: sidecars by
// filename under `advancement/`. Existence (including an empty file) is the
// fail-closed signal; the caller never treats an existing file as absent.
pub(crate) fn collect_records(save: &Path) -> Result<BTreeMap<String, PathBuf>> {
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

pub(crate) fn load_record(path: &Path) -> Result<Record> {
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
            row.before_digest = Some(digest(&original));
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
    fn enrollment_entropy_retries_zero_and_collision_then_encodes_all_128_bits() {
        let occupied = BTreeSet::from(["11111111111111111111111111111111"]);
        let expected = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let mut entropy = [[0; 16], [0x11; 16], expected].into_iter();
        let token = generate_token(&occupied, &mut |output| {
            *output = entropy.next().unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(token, "00112233445566778899aabbccddeeff");
        assert!(entropy.next().is_none());
    }

    #[test]
    fn enrollment_entropy_failure_stops_and_repeated_collisions_are_bounded() {
        let occupied = BTreeSet::from(["11111111111111111111111111111111"]);
        let mut attempts = 0;
        let unavailable = generate_token(&occupied, &mut |_| {
            attempts += 1;
            bail!("synthetic entropy failure");
        });
        assert_eq!(attempts, 1);
        assert_eq!(
            unavailable.unwrap_err().to_string(),
            "synthetic entropy failure"
        );

        let mut attempts = 0;
        let exhausted = generate_token(&occupied, &mut |output| {
            attempts += 1;
            *output = [0x11; 16];
            Ok(())
        });
        assert_eq!(attempts, 8);
        assert!(exhausted
            .unwrap_err()
            .to_string()
            .contains("eight attempts"));
    }

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
