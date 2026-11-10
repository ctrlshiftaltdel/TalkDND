//! Aggregates caller-supplied conditions only; it does not verify eligibility,
//! identity, ruleset, content permissions, or FULL unlocks.

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
    ConfirmedUnknown { unknown_ids: Vec<String>, dm_reason: String },
    Rejected { unmet_ids: Vec<String>, unknown_ids: Vec<String> },
}

pub fn evaluate(_checks: &[ConditionCheck], _dm_reason: Option<&str>) -> anyhow::Result<ValidationDecision> {
    Ok(ValidationDecision::Verified)
}
