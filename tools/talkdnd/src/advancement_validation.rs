//! Aggregates caller-supplied conditions only; it does not verify eligibility,
//! identity, ruleset, content permissions, or FULL unlocks.

use std::collections::BTreeSet;

use anyhow::bail;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionStatus {
    Met,
    NotMet,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionCheck {
    pub id: String,
    pub status: ConditionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDecision {
    Verified,
    ConfirmedUnknown {
        unknown_ids: Vec<String>,
        dm_reason: String,
    },
    Rejected {
        unmet_ids: Vec<String>,
        unknown_ids: Vec<String>,
    },
}

/// Summarize only the provided checks. Even `Verified` is not an overall
/// eligibility or authorization decision; callers must establish those separately.
pub fn evaluate(
    checks: &[ConditionCheck],
    dm_reason: Option<&str>,
) -> anyhow::Result<ValidationDecision> {
    if checks.is_empty() {
        bail!("at least one condition is required");
    }
    let mut seen = BTreeSet::new();
    let mut unmet_ids = Vec::new();
    let mut unknown_ids = Vec::new();
    for check in checks {
        if check.id.is_empty() || check.id.trim() != check.id {
            bail!("condition ID must be nonblank and have no surrounding whitespace");
        }
        if !seen.insert(&check.id) {
            bail!("duplicate condition ID: {:?}", check.id);
        }
        match check.status {
            ConditionStatus::Met => {}
            ConditionStatus::NotMet => unmet_ids.push(check.id.clone()),
            ConditionStatus::Unknown => unknown_ids.push(check.id.clone()),
        }
    }
    unmet_ids.sort();
    unknown_ids.sort();
    if !unmet_ids.is_empty() {
        return Ok(ValidationDecision::Rejected {
            unmet_ids,
            unknown_ids,
        });
    }
    if unknown_ids.is_empty() {
        return Ok(ValidationDecision::Verified);
    }
    match dm_reason.map(str::trim).filter(|reason| !reason.is_empty()) {
        Some(reason) => Ok(ValidationDecision::ConfirmedUnknown {
            unknown_ids,
            dm_reason: reason.to_owned(),
        }),
        None => Ok(ValidationDecision::Rejected {
            unmet_ids,
            unknown_ids,
        }),
    }
}
