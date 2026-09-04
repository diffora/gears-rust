//! Tests for the retention gate (`dod-retention-gate`). Each case is one of
//! the `DoD`'s named failures, armed as the failure rather than as its fix.

use super::{RetentionHold, RetentionVerdict, evaluate};
use crate::domain::states::FreezeAckState;
use crate::infra::storage::repo::FreezeRegistration;

fn reg(participant: &str, state: FreezeAckState, stamped: bool) -> FreezeRegistration {
    FreezeRegistration {
        participant: participant.to_owned(),
        state,
        released_at_stamped: stamped,
    }
}

fn snap(members: &[&str]) -> Vec<String> {
    members.iter().map(|m| (*m).to_owned()).collect()
}

/// **The vacuity the `DoD` names first.** An empty ledger against a non-empty
/// snapshot must HOLD — quantifying over registrations instead of the
/// snapshot is what let a version nobody had frozen be collected.
#[test]
fn an_empty_ledger_holds_a_non_empty_snapshot() {
    let verdict = evaluate(&snap(&["pricing", "contracts"]), &[]);
    let RetentionVerdict::Held(holds) = verdict else {
        panic!("an empty ledger must not satisfy the gate vacuously");
    };
    assert_eq!(holds.len(), 2, "every member holds, not just the first");
    for hold in &holds {
        assert!(matches!(hold, RetentionHold::NoRegistration { .. }));
        assert_eq!(RetentionHold::REASON, "retention_orphan_blocked");
    }
}

/// **The other vacuity, and it is admitted.** An empty snapshot is
/// collectable: nobody ever owed an ack. The two cases above and here differ
/// in which store is empty, and only one of them is a defect.
#[test]
fn an_empty_snapshot_is_collectable() {
    assert_eq!(evaluate(&[], &[]), RetentionVerdict::Collectable);
    assert_eq!(
        evaluate(&[], &[reg("pricing", FreezeAckState::Pending, false)]),
        RetentionVerdict::Collectable,
        "a registration outside the snapshot is not the gate's business"
    );
}

/// A door-released row carries `state = released` with the stamp **NULL**, and
/// that satisfies the first arm — so a gate reading the timestamp would refuse
/// every ordinary release.
#[test]
fn a_door_released_row_satisfies_the_gate_without_a_stamp() {
    assert_eq!(
        evaluate(
            &snap(&["pricing"]),
            &[reg("pricing", FreezeAckState::Released, false)]
        ),
        RetentionVerdict::Collectable
    );
}

/// **The failure the second arm exists for.** A forced participant that later
/// recovered and acked leaves `state = acked` beside a live `released_at` — so
/// reading the timestamp alone collected a version holding live grandfathered
/// references.
#[test]
fn a_stamp_beside_a_live_state_does_not_satisfy_the_gate() {
    let verdict = evaluate(
        &snap(&["pricing"]),
        &[reg("pricing", FreezeAckState::Acked, true)],
    );
    let RetentionVerdict::Held(holds) = verdict else {
        panic!("the timestamp alone must not satisfy the gate");
    };
    assert_eq!(
        holds,
        vec![RetentionHold::LiveRegistration {
            participant: "pricing".to_owned(),
            state: FreezeAckState::Acked,
        }]
    );
}

/// The forced arm needs both halves, and the shape `CHECK` refuses the
/// half-written row on both engines — so this arm reports a row that reached
/// the table past its guard rather than an ordinary state.
#[test]
fn the_forced_arm_needs_its_stamp() {
    let verdict = evaluate(
        &snap(&["pricing"]),
        &[reg("pricing", FreezeAckState::NotFrozenForced, false)],
    );
    assert_eq!(
        verdict,
        RetentionVerdict::Held(vec![RetentionHold::ForcedWithoutStamp {
            participant: "pricing".to_owned()
        }])
    );
    assert_eq!(
        evaluate(
            &snap(&["pricing"]),
            &[reg("pricing", FreezeAckState::NotFrozenForced, true)]
        ),
        RetentionVerdict::Collectable,
        "with the stamp the forced arm is satisfied"
    );
}

/// `pending` holds, which is the ordinary in-flight case, and the verdict
/// carries the state so the skip reason can name it.
#[test]
fn a_pending_registration_holds() {
    let verdict = evaluate(
        &snap(&["pricing"]),
        &[reg("pricing", FreezeAckState::Pending, false)],
    );
    assert_eq!(
        verdict,
        RetentionVerdict::Held(vec![RetentionHold::LiveRegistration {
            participant: "pricing".to_owned(),
            state: FreezeAckState::Pending,
        }])
    );
}

/// **Every** hold is reported rather than the first: an operator repairing one
/// and re-running would otherwise meet the rest one pass at a time.
#[test]
fn every_hold_is_reported_and_a_mixed_set_still_holds() {
    let verdict = evaluate(
        &snap(&["pricing", "contracts", "billing", "rating"]),
        &[
            reg("pricing", FreezeAckState::Released, false),
            reg("contracts", FreezeAckState::Pending, false),
            reg("billing", FreezeAckState::NotFrozenForced, true),
            // `rating` has no row at all.
        ],
    );
    let RetentionVerdict::Held(holds) = verdict else {
        panic!("two members hold");
    };
    let named: Vec<&str> = holds.iter().map(RetentionHold::participant).collect();
    assert_eq!(
        named,
        vec!["contracts", "rating"],
        "the two satisfied members are absent from the holds and the two unsatisfied are present, \
         in snapshot order"
    );
}

// -- The PII detector policy (`dod-pii-detector`) and its normalization --

use crate::domain::retention::{RegistryPiiDetector, normalize_allowlist_value};
use crate::domain::taxonomy::{PiiDetector, PiiVerdict, content_pii_block};

/// A detector over the given allow-list values.
fn detector(values: &[&str]) -> RegistryPiiDetector {
    RegistryPiiDetector::new(values.iter().map(|v| (*v).to_owned()))
}

/// **The four arms, each with a positive control.**
///
/// §6's own words: *"each with a positive control, so no arm passes because
/// the fixture could not reach the permissive branch"*. So every blocking arm
/// below is paired with a string that reaches the same code path and is
/// admitted — an `allow` case beside `block`, an on-list case beside the
/// uncertain one.
#[test]
fn the_four_arms_each_have_a_positive_control() {
    let empty = detector(&[]);
    let listed = detector(&["Ann Fritz"]);

    // allow — the control for every arm below: this text reaches the
    // inspection and comes out clean, so a detector that refused everything
    // would fail here.
    assert_eq!(
        empty.inspect("description", "a 40 GiB monthly storage add-on"),
        PiiVerdict::Clean
    );

    // block — an email address. Its control is the allow case above and the
    // near-miss below, which shares the `@` and is not an address.
    assert!(matches!(
        empty.inspect("reason", "escalated by ops@example.com"),
        PiiVerdict::Blocked(_)
    ));
    assert_eq!(
        empty.inspect("reason", "escalated by ops@ the desk"),
        PiiVerdict::Clean,
        "the near-miss control: an `@` with no dotted domain is not an address, and a rule that \
         blocked it would refuse ordinary prose"
    );

    // block — a phone number, with the SKU-code control the nine-digit floor
    // exists for.
    assert!(matches!(
        empty.inspect("reason", "call +44 20 7946 0958"),
        PiiVerdict::Blocked(_)
    ));
    assert_eq!(
        empty.inspect("reason", "see SKU-1234567 for the tier"),
        PiiVerdict::Clean,
        "the control the floor was chosen for: a catalog identifier is not a phone number, and \
         an operator refused for quoting their own SKU has no lane out"
    );

    // uncertainty — an unlisted person-shaped run. Its control is the listed
    // case immediately below: the same string, admitted.
    assert!(matches!(
        empty.inspect("justification", "named for Ann Fritz"),
        PiiVerdict::Uncertain(_)
    ));

    // allow-by-list — the same run, on the list.
    assert_eq!(
        listed.inspect("justification", "named for Ann Fritz"),
        PiiVerdict::Clean,
        "the allow-by-list arm IS the uncertain arm with an entry present; if this failed the \
         uncertain assertion above would be proving only that the detector refuses names"
    );
}

/// **An unlisted person-shaped run is `Uncertain` and never `Blocked`, and
/// the difference is not cosmetic.**
///
/// `Blocked` asserts a finding — *this is personal data* — that the detector
/// did not make, and that false assertion would reach the operator's refusal
/// and the audit row. Both arms still refuse the write, because the hook
/// holds the fail-closed rule; what differs is what the record says happened.
#[test]
fn an_unlisted_name_is_uncertain_and_an_address_is_blocked() {
    let empty = detector(&[]);
    assert!(matches!(
        empty.inspect("reason", "named for Ann Fritz"),
        PiiVerdict::Uncertain(_)
    ));
    assert!(matches!(
        empty.inspect("reason", "reach ann@fritz.example"),
        PiiVerdict::Blocked(_)
    ));
    // And both still refuse at the hook, which is where C2 lives.
    assert!(content_pii_block(&empty, "reason", "named for Ann Fritz").is_err());
    assert!(content_pii_block(&empty, "reason", "reach ann@fritz.example").is_err());
}

/// **No reason a verdict carries ever quotes what was matched.**
///
/// Swept over every blocking arm rather than asserted on one, because the
/// clause is about the detector's whole output surface: one arm added later
/// with the match interpolated in would be exactly the leak this forbids, and
/// a single-case probe would not see it.
#[test]
fn no_verdict_reason_carries_the_matched_text() {
    let empty = detector(&[]);
    let secrets = [
        ("reason", "escalated by ops@example.com", "ops@example.com"),
        ("reason", "call +44 20 7946 0958", "7946"),
        ("justification", "named for Ann Fritz", "Ann Fritz"),
    ];
    for (field, text, secret) in secrets {
        let rendered = match empty.inspect(field, text) {
            PiiVerdict::Clean => panic!("{text} must not be admitted"),
            PiiVerdict::Blocked(reason) | PiiVerdict::Uncertain(reason) => reason,
        };
        assert!(
            !rendered.contains(secret),
            "the verdict for {text} quoted `{secret}`: {rendered}"
        );
        // The hook's rendering is the one that reaches a door, so it is
        // checked too rather than trusted to inherit the property.
        let blocked = content_pii_block(&empty, field, text)
            .expect_err("this text is refused")
            .into_detail();
        assert!(
            !blocked.contains(secret),
            "the hook's detail for {text} quoted `{secret}`: {blocked}"
        );
        assert!(
            blocked.contains(field),
            "the hook's detail must name the field: {blocked}"
        );
    }
}

/// **The normalization is the match rule, and each of its steps is asserted
/// as the sign-off it saves.**
///
/// Every case below is one spelling Legal would otherwise have to sign off
/// twice.
#[test]
fn the_normalization_folds_case_whitespace_and_compatibility_forms() {
    assert_eq!(normalize_allowlist_value("Ann Fritz"), "ann fritz");
    assert_eq!(normalize_allowlist_value("  Ann   Fritz  "), "ann fritz");
    assert_eq!(normalize_allowlist_value("ANN FRITZ"), "ann fritz");

    // NFKC: the full-width forms decompose to the plain ones. Written as
    // escapes rather than as the characters themselves, and the guard below
    // is why the escapes matter: the first draft of this line pasted what
    // looked like full-width letters and was plain ASCII, so the case
    // asserted the same thing as the first line above and proved nothing
    // about NFKC at all.
    let full_width = "\u{ff21}nn \u{ff26}ritz";
    assert_ne!(
        full_width, "Ann Fritz",
        "the input must differ from the ASCII spelling, or this case is the first one again"
    );
    assert_eq!(normalize_allowlist_value(full_width), "ann fritz");

    // A non-breaking space is whitespace once NFKC has made it one.
    let nbsp = "Ann\u{a0}Fritz";
    assert_ne!(
        nbsp, "Ann Fritz",
        "same guard: a plain space here would make this the first case too"
    );
    assert_eq!(normalize_allowlist_value(nbsp), "ann fritz");
}

/// **The normalization deliberately does NOT strip punctuation or
/// diacritics.**
///
/// Each would widen the match beyond what the sign-off covered. Asserted as
/// inequalities, because a rule's *limits* are what a later "improvement"
/// silently removes.
#[test]
fn the_normalization_does_not_widen_past_the_sign_off() {
    assert_ne!(
        normalize_allowlist_value("O'Neill"),
        normalize_allowlist_value("ONeill"),
        "Legal signed off on one of these spellings, not both"
    );
    assert_ne!(
        normalize_allowlist_value("Ren\u{e9}e"),
        normalize_allowlist_value("Renee"),
        "dropping diacritics would admit a name nobody reviewed"
    );
    assert_ne!(
        normalize_allowlist_value("Fritz Ann"),
        normalize_allowlist_value("Ann Fritz"),
        "word order is part of the name, not noise"
    );
}

/// **A candidate matches an entry however either side was spelled.**
///
/// Both sides run through one function, which is the property that keeps the
/// stored column and the inspected text from drifting into two definitions of
/// "normalized". Driven from the raw spellings a caller would actually send.
#[test]
fn a_listed_name_matches_whatever_spelling_reaches_the_detector() {
    let listed = detector(&["  ANN   Fritz "]);
    for spelling in ["named for Ann Fritz", "named for ANN FRITZ"] {
        assert_eq!(
            listed.inspect("justification", spelling),
            PiiVerdict::Clean,
            "{spelling} must reach the entry it was signed off as"
        );
    }
}

/// **The longest capitalized run is one candidate, not several.**
///
/// `Maria Del Carmen Ruiz` is one name. Asking Legal to sign off three
/// fragments of it is a rule nobody can satisfy, so the candidate is the run.
#[test]
fn the_candidate_is_the_whole_run_and_a_listed_run_admits_it() {
    let empty = detector(&[]);
    assert!(matches!(
        empty.inspect("justification", "the Maria Del Carmen Ruiz line"),
        PiiVerdict::Uncertain(_)
    ));
    let listed = detector(&["Maria Del Carmen Ruiz"]);
    assert_eq!(
        listed.inspect("justification", "the Maria Del Carmen Ruiz line"),
        PiiVerdict::Clean
    );
    let fragment = detector(&["Maria Del"]);
    assert!(
        matches!(
            fragment.inspect("justification", "the Maria Del Carmen Ruiz line"),
            PiiVerdict::Uncertain(_)
        ),
        "a sign-off on part of a name does not admit the whole of it"
    );
}

/// **A single capitalized word is not a candidate.**
///
/// Sentences start with one, and product names are full of them; a rule that
/// made every capitalized word undecidable would refuse nearly every
/// description and the allow-list could never catch up.
#[test]
fn one_capitalized_word_is_not_a_person_shape() {
    let empty = detector(&[]);
    assert_eq!(
        empty.inspect("description", "Storage add-on for the Enterprise tier"),
        PiiVerdict::Clean
    );
}

/// **An empty allow-list is a resolved state, not a failed read.**
///
/// A tenant Legal has signed nothing off for admits ordinary text and finds
/// person-shaped runs undecidable — which is the correct answer, and the one
/// a caller can act on by getting a sign-off.
#[test]
fn an_empty_allow_list_still_admits_ordinary_text() {
    let empty = detector(&[]);
    assert_eq!(
        empty.inspect("description", "a 40 GiB monthly storage add-on"),
        PiiVerdict::Clean
    );
}
