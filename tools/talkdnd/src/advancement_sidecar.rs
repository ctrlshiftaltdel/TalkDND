//! Schema 11 planning: read-only discovery of unbound party characters, DM
//! confirmation matching, and strict v1 first-enrollment preparation.
//!
//! The planner never writes. It reuses the #345 candidate builder and the
//! #345/#344 strictness rules: discovery must not be ambiguous or incomplete,
//! a sidecar path counts as existing even when the file is empty (an empty or
//! invalid sidecar is a fail-closed signal for humans or explicit recovery,
//! never a missing-file case), and the occupied token set covers every
//! carrier marker and record in the save so new tokens stay unique.
//!
//! Everything the planner returns is a locator (a save-relative path) or a
//! digest; ids and names are untrusted save content and are never echoed.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use advancement_lock::save_lock::digest;
use anyhow::{ensure, Context, Result};

use crate::advancement_identity::{
    card_token, collect_records, load_record, prepare_first_enrollment, read_regular,
    sheet_identity, EnrollmentApproval, EnrollmentCandidate, EnrollmentRequest,
};
use crate::identity_diag::{self, Character, Status};

/// One DM confirmation from the command line: a save-relative POSIX path and
/// the digest of the exact bytes the DM reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingConfirm {
    pub path: String,
    pub digest: String,
}

impl BindingConfirm {
    /// Parse `PATH=DIGEST`. The path is a locator, never character content.
    /// The digest is the project's `save_lock::digest` string; a bare 64-hex
    /// value is accepted and normalized to the `sha256:` form every plan
    /// compares against.
    pub fn parse(spec: &str) -> Result<Self> {
        let (path, digest_text) = spec
            .split_once('=')
            .context("确认参数必须是 PATH=DIGEST 形式")?;
        ensure!(
            !path.is_empty() && !path.contains('\\') && !path.contains(".."),
            "确认路径必须是存档内的 POSIX 相对路径"
        );
        let digest_text = digest_text.trim();
        let bare = digest_text
            .strip_prefix("sha256:")
            .unwrap_or(digest_text);
        ensure!(
            bare.len() == 64
                && bare
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "确认摘要必须是 64 位十六进制 SHA-256"
        );
        Ok(Self {
            path: path.to_string(),
            digest: format!("sha256:{bare}"),
        })
    }
}

/// An unbound character the DM has not confirmed yet: the preimage digests
/// for review and the exact confirm argument to supply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingBinding {
    pub markdown: String,
    pub sheet: Option<String>,
    pub markdown_digest: String,
    pub sheet_digest: Option<String>,
}

impl PendingBinding {
    /// The confirm arguments a DM would pass after reviewing these bytes.
    pub fn confirm_args(&self) -> Vec<String> {
        let mut args = vec![format!(
            "--confirm-binding {}={}",
            self.markdown, self.markdown_digest
        )];
        if let (Some(sheet), Some(digest_text)) = (&self.sheet, &self.sheet_digest) {
            args.push(format!("--confirm-binding {sheet}={digest_text}"));
        }
        args
    }
}

/// The complete schema-11 plan for one save. `refused` is fail-closed: when
/// it is non-empty no binding happens for the whole save.
#[derive(Debug)]
pub struct BindingPlan {
    /// Indices into the diagnostic's party characters that carry a valid
    /// reviewed approval for every carrier byte.
    confirmed: Vec<usize>,
    pub pending: Vec<PendingBinding>,
    pub refused: Vec<String>,
    /// Every token already bound in the save (carriers and records), so
    /// generated tokens stay unique across the save.
    occupied: Vec<String>,
    /// The save has no party characters and no records: schema 11 would add
    /// nothing, so the migration stays at 10 instead of churning the save.
    trivial: bool,
    report: identity_diag::Report,
    schema: u32,
}

impl BindingPlan {
    /// True when every party character is either already strictly matched or
    /// has a complete reviewed confirmation, and nothing was refused. A
    /// trivially empty save is never "ready": it simply has nothing to do.
    pub fn ready_for_schema_11(&self) -> bool {
        !self.trivial && self.refused.is_empty() && self.pending.is_empty()
    }

    /// D-protocol publish targets this plan adds beyond the existing
    /// registered set: every party character's sidecar (bound ones included,
    /// so the preview binding digest covers them) plus `save-meta.yaml`.
    pub fn extra_targets(&self) -> Vec<String> {
        self.report
            .characters
            .iter()
            .filter(|character| character.markdown_path().starts_with("Characters"))
            .map(sidecar_rel)
            .chain(std::iter::once("save-meta.yaml".to_string()))
            .collect()
    }

    fn party(&self, index: usize) -> Option<&Character> {
        self.report
            .characters
            .get(index)
            .filter(|character| character.markdown_path().starts_with("Characters"))
    }
}

fn sidecar_rel(character: &Character) -> String {
    format!("advancement/{}.yaml", character.id())
}

/// Read-only pass over the save. `schema` is the save schema the migration
/// observed (only 9 and 10 can enroll). Confirms may cover already-bound
/// characters; a mismatched confirm is refused, not ignored.
pub fn plan(save: &Path, schema: u32, confirms: &[BindingConfirm]) -> Result<BindingPlan> {
    ensure!(
        matches!(schema, 9 | 10),
        "schema {schema} 不支持身份绑定规划"
    );
    let report = identity_diag::diagnose(save);
    let mut plan = BindingPlan {
        confirmed: Vec::new(),
        pending: Vec::new(),
        refused: Vec::new(),
        occupied: Vec::new(),
        trivial: false,
        report,
        schema,
    };
    if plan.report.status >= Status::Ambiguous {
        plan.refused.push(format!(
            "身份发现状态 {:?}，未完成整档枚举前拒绝自动绑定",
            plan.report.status
        ));
        return Ok(plan);
    }

    // Multiple confirms for one path are allowed only when identical.
    let mut grouped: BTreeMap<&str, Vec<&BindingConfirm>> = BTreeMap::new();
    for confirm in confirms {
        grouped
            .entry(confirm.path.as_str())
            .or_default()
            .push(confirm);
    }
    for (path, group) in &grouped {
        ensure!(
            group
                .iter()
                .all(|confirm| confirm.digest == group[0].digest),
            "同一路径给出多个不同摘要：{path}"
        );
    }

    // Existing records: an unreadable or invalid record is fail-closed, and
    // its token (if parsable) still counts as occupied.
    let mut records = collect_records(save)?;
    let party_count = plan
        .report
        .characters
        .iter()
        .filter(|character| character.markdown_path().starts_with("Characters"))
        .count();
    if party_count == 0 && records.is_empty() {
        plan.trivial = true;
        return Ok(plan);
    }
    for (filename, path) in &records {
        match load_record(path) {
            Ok(record) => plan.occupied.push(record.token),
            Err(err) => {
                plan.refused.push(format!(
                    "成长记录 advancement/{filename} 无效（{err}）；先人工核对或走显式恢复"
                ));
            }
        }
    }

    for (index, character) in plan.report.characters.iter().enumerate() {
        if !character.markdown_path().starts_with("Characters") {
            continue;
        }
        let markdown_rel = character.markdown().to_owned();
        let markdown = match read_regular(&save.join(character.markdown_path())) {
            Ok(bytes) => bytes,
            Err(err) => {
                plan.refused
                    .push(format!("无法读取 {markdown_rel}（{err}）"));
                continue;
            }
        };
        let markdown_token = match card_token(&markdown) {
            Ok(token) => token,
            Err(_) => {
                plan.refused.push(format!("{markdown_rel} 的身份标记无效"));
                continue;
            }
        };
        let sheet_rel = character.sheet().map(str::to_owned);
        let mut sheet_bytes = None;
        let sheet_token = if let Some(sheet_rel) = &sheet_rel {
            let bytes = match read_regular(&save.join(character.sheet_path().expect("discovery"))) {
                Ok(bytes) => bytes,
                Err(err) => {
                    plan.refused.push(format!("无法读取 {sheet_rel}（{err}）"));
                    continue;
                }
            };
            let token = match sheet_identity(&bytes) {
                Ok((_, token)) => token,
                Err(_) => {
                    plan.refused.push(format!("{sheet_rel} 的身份字段无效"));
                    continue;
                }
            };
            sheet_bytes = Some(bytes);
            token
        } else {
            None
        };

        // Every observed token counts as occupied even when its record side
        // is broken: uniqueness must hold over everything observed.
        for token in [markdown_token, sheet_token.as_deref()]
            .into_iter()
            .flatten()
        {
            plan.occupied.push(token.to_owned());
        }

        let markdown_digest = digest(&markdown).as_str().to_owned();
        let sheet_digest = sheet_bytes
            .as_ref()
            .map(|bytes| digest(bytes).as_str().to_owned());

        // A confirm naming this carrier without matching its current bytes is
        // a refusal in every state — bound or pending: the DM reviewed
        // something else, and "already bound" must not swallow that.
        for rel in [Some(markdown_rel.as_str()), sheet_rel.as_deref()]
            .into_iter()
            .flatten()
        {
            let Some(group) = grouped.get(rel) else {
                continue;
            };
            let expected = if rel == markdown_rel {
                markdown_digest.as_str()
            } else {
                // A sheet locator can only be confirmed here when the sheet
                // was read above, so its digest exists.
                sheet_digest
                    .as_deref()
                    .expect("confirmed sheet has a digest")
            };
            if group[0].digest != expected {
                plan.refused
                    .push(format!("{rel} 的确认摘要与当前字节不符；请先重新只读核对"));
            }
        }

        let carrier_token = match sheet_rel.as_deref().zip(sheet_token.as_deref()) {
            Some((_sheet, token)) => Some(token.to_owned()),
            None => markdown_token.map(str::to_owned),
        };
        let record = match records.remove(&format!("{}.yaml", character.id())) {
            Some(path) => match load_record(&path) {
                Ok(record) => Some(record),
                // `collect_records` already refused invalid records above.
                Err(_) => continue,
            },
            None => None,
        };

        if let Some(token) = carrier_token {
            // Bound carrier: the record must strictly match, or this is a
            // human case (record missing/mismatched), never a rebind.
            let expected_carrier = sheet_rel.as_deref().unwrap_or(&markdown_rel);
            let strictly_matched = record.is_some_and(|rec| {
                rec.token == *token
                    && matches!(rec.history, crate::advancement_identity::History::Unknown)
                    && rec.card == markdown_rel
                    && rec.carrier.as_deref().unwrap_or(&rec.card) == expected_carrier
            });
            if !strictly_matched {
                plan.refused.push(format!(
                    "{markdown_rel} 已有身份但成长记录缺失或不匹配；先人工核对或走显式恢复"
                ));
            }
            continue;
        }

        // Unbound character. Any existing sidecar path — including an empty
        // file — blocks first enrollment: "exists" and "missing" differ.
        let sidecar_rel = sidecar_rel(character);
        let sidecar_path =
            save.join(Path::new("advancement").join(format!("{}.yaml", character.id())));
        match fs::symlink_metadata(&sidecar_path) {
            Ok(_) => {
                plan.refused.push(format!(
                    "{sidecar_rel} 已存在（空文件同样视为存在，不算缺失）；先人工核对或走显式恢复"
                ));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                plan.refused
                    .push(format!("无法检查 {sidecar_rel}（{error}）"));
                continue;
            }
        }

        let confirmed = grouped
            .get(markdown_rel.as_str())
            .is_some_and(|group| group[0].digest == markdown_digest)
            && sheet_digest.as_deref().is_none_or(|expected| {
                sheet_rel.as_ref().is_some_and(|rel| {
                    grouped
                        .get(rel.as_str())
                        .is_some_and(|group| group[0].digest == expected)
                })
            });
        if confirmed {
            plan.confirmed.push(index);
        } else {
            plan.pending.push(PendingBinding {
                markdown: markdown_rel,
                sheet: sheet_rel,
                markdown_digest,
                sheet_digest,
            });
        }
    }

    // Records left after every party character consumed theirs are orphans
    // (AC-3): refuse rather than republish or drop them silently.
    for filename in records.keys() {
        plan.refused.push(format!(
            "孤儿成长记录 advancement/{filename}（没有对应的队伍角色）；先人工核对或走显式恢复"
        ));
    }

    // Confirms that name no discovered carrier are stale, not harmless.
    let mut known: Vec<String> = Vec::new();
    for character in plan
        .report
        .characters
        .iter()
        .filter(|character| character.markdown_path().starts_with("Characters"))
    {
        known.push(character.markdown().to_owned());
        if let Some(sheet) = character.sheet() {
            known.push(sheet.to_owned());
        }
    }
    for path in grouped.keys() {
        if !known.iter().any(|known_path| known_path == path) {
            plan.refused.push(format!("确认了不存在的载体 {path}"));
        }
    }
    Ok(plan)
}

/// Build the strict v1 first-enrollment candidates for every confirmed
/// character. Bytes are re-read here; `prepare_first_enrollment` rechecks the
/// reviewed digests, so a save that changed after planning is refused.
pub fn prepare(plan: &BindingPlan, save: &Path) -> Result<Vec<EnrollmentCandidate>> {
    ensure!(
        matches!(plan.schema, 9 | 10),
        "首次入籍只接受 schema 9 或 10，当前 {}",
        plan.schema
    );
    let mut candidates = Vec::new();
    for &index in &plan.confirmed {
        let character = plan
            .party(index)
            .context("规划与发现结果不一致，拒绝继续")?;
        let markdown_rel = character.markdown();
        let markdown = read_regular(&save.join(character.markdown_path()))?;
        let sheet = character
            .sheet_path()
            .map(|path| read_regular(&save.join(path)))
            .transpose()?;
        let sheet_rel = character.sheet();
        let markdown_digest = digest(&markdown).as_str().to_owned();
        let sheet_digest = sheet
            .as_deref()
            .map(|bytes| digest(bytes).as_str().to_owned());
        let sidecar_path =
            save.join(Path::new("advancement").join(format!("{}.yaml", character.id())));
        let sidecar_exists = match fs::symlink_metadata(&sidecar_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        ensure!(
            !sidecar_exists,
            "{} 已存在，拒绝首次入籍（空文件同样视为存在）",
            sidecar_rel(character)
        );
        let occupied: Vec<&str> = plan.occupied.iter().map(String::as_str).collect();
        let request = EnrollmentRequest {
            schema: plan.schema,
            character,
            markdown: &markdown,
            sheet: sheet.as_deref(),
            sidecar_exists,
            occupied_tokens: &occupied,
            approval: Some(EnrollmentApproval {
                markdown: (markdown_rel, &markdown_digest),
                sheet: sheet_rel.zip(sheet_digest.as_deref()),
            }),
        };
        candidates.push(prepare_first_enrollment(request)?);
    }
    Ok(candidates)
}

/// Publish overlays for the whole 10→11 step: the confirmed carriers and
/// sidecars. `save-meta.yaml` is added by the migration caller.
pub fn overlays(candidates: &[EnrollmentCandidate]) -> BTreeMap<String, Vec<u8>> {
    let mut overlays = BTreeMap::new();
    for candidate in candidates {
        overlays.insert(
            candidate.carrier_path.to_string_lossy().replace('\\', "/"),
            candidate.carrier_bytes.clone(),
        );
        overlays.insert(
            candidate.sidecar_path.to_string_lossy().replace('\\', "/"),
            candidate.sidecar_bytes.clone(),
        );
    }
    overlays
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_parse_rejects_bad_paths_and_digests() {
        assert!(BindingConfirm::parse("Characters/A.md=").is_err());
        assert!(BindingConfirm::parse("noequalsign").is_err());
        assert!(BindingConfirm::parse("a\\b.md=0").is_err());
        assert!(BindingConfirm::parse("../x.md=0").is_err());
        assert!(BindingConfirm::parse("Characters/A.md=XYZ").is_err());
        assert!(BindingConfirm::parse("Characters/A.md=0123").is_err());
        let bare = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let ok = BindingConfirm::parse(&format!("Characters/A.md={bare}")).unwrap();
        assert_eq!(ok.path, "Characters/A.md");
        assert_eq!(ok.digest, format!("sha256:{bare}"));
        let prefixed = BindingConfirm::parse(&format!("Characters/A.md=sha256:{bare}")).unwrap();
        assert_eq!(prefixed.digest, ok.digest);
    }

    #[test]
    fn pending_binding_lists_exact_confirm_args() {
        let pending = PendingBinding {
            markdown: "Characters/A.md".into(),
            sheet: Some("sheets/a.yaml".into()),
            markdown_digest: "d1".into(),
            sheet_digest: Some("d2".into()),
        };
        assert_eq!(
            pending.confirm_args(),
            vec![
                "--confirm-binding Characters/A.md=d1",
                "--confirm-binding sheets/a.yaml=d2"
            ]
        );
    }
}
