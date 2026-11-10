use talkdnd::advancement_validation::{evaluate, ConditionCheck, ConditionStatus::*, ValidationDecision::*};

fn check(id: &str, status: talkdnd::advancement_validation::ConditionStatus) -> ConditionCheck {
    ConditionCheck { id: id.into(), status }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn met_conditions_verify_without_asserting_eligibility_or_authorization() {
    // Arbitrary synthetic labels are not interpreted as ruleset or content identities.
    for label in ["2014-srd", "2024-srd", "2014-full", "2024-full", "unlicensed"] {
        assert_eq!(evaluate(&[check(label, Met)], None).unwrap(), Verified);
    }
}

#[test]
fn unmet_conditions_always_reject_and_sort_even_with_confirmation() {
    let checks = [check("z", NotMet), check("b", Unknown), check("a", NotMet), check("c", Unknown)];
    let expected = Rejected { unmet_ids: ids(&["a", "z"]), unknown_ids: ids(&["b", "c"]) };
    assert_eq!(evaluate(&checks, Some("approved")).unwrap(), expected);
    assert_eq!(evaluate(&[check("only", NotMet)], None).unwrap(), Rejected { unmet_ids: ids(&["only"]), unknown_ids: vec![] });
    let reversed: Vec<_> = checks.into_iter().rev().collect();
    assert_eq!(evaluate(&reversed, Some("approved")).unwrap(), expected);
}

#[test]
fn unknown_requires_a_nonblank_reason_and_never_becomes_verified() {
    let checks = [check("z", Unknown), check("a", Unknown), check("ok", Met)];
    let rejected = Rejected { unmet_ids: vec![], unknown_ids: ids(&["a", "z"]) };
    for reason in [None, Some(""), Some(" \t\n")] {
        assert_eq!(evaluate(&checks, reason).unwrap(), rejected);
    }
    assert_eq!(evaluate(&checks, Some("  reviewed  ")).unwrap(), ConfirmedUnknown { unknown_ids: ids(&["a", "z"]), dm_reason: "reviewed".into() });
    assert_eq!(evaluate(&[check("single", Unknown)], Some("yes")).unwrap(), ConfirmedUnknown { unknown_ids: ids(&["single"]), dm_reason: "yes".into() });
}

#[test]
fn invalid_or_duplicate_ids_and_empty_checks_are_errors() {
    assert!(evaluate(&[], None).is_err());
    for id in ["", "  ", " x", "x ", "\tx", "x\n"] {
        assert!(evaluate(&[check(id, Met)], None).is_err(), "id {id:?}");
    }
    assert!(evaluate(&[check("same", Met), check("same", Unknown)], Some("ok")).is_err());
}

#[test]
fn case_distinct_ids_are_independent_and_sorted_without_normalization() {
    let checks = [check("a", Unknown), check("A", Unknown)];
    assert_eq!(evaluate(&checks, Some("ok")).unwrap(), ConfirmedUnknown { unknown_ids: ids(&["A", "a"]), dm_reason: "ok".into() });
}
