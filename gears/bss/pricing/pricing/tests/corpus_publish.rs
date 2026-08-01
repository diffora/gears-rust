//! The gear's half of the joint conformance corpus: the publish cases, and the
//! generated `registry.toml` they earn a flag in.
//!
//! This file is the **single authority** on the committed registry's content.
//! `bss-fixtures-conformance` can compute only one of the two halves the file
//! records — it cannot reach the gear, by the corpus's own invariant — so an
//! expectation built there would be an expectation over a registry with the
//! publish half nailed shut. Two crates each asserting a different expected
//! content is how a generated file starts flapping.

#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../examples/regen_registry/validator.rs"]
mod validator;

use bss_fixtures::{Corpus, ModelKind, Registry};
use bss_fixtures_conformance::registry_gen;

fn corpus() -> Corpus {
    Corpus::load(&Corpus::corpus_root()).expect("corpus loads")
}

#[test]
fn the_committed_registry_is_fresh() {
    let corpus = corpus();
    let earned = validator::publish_report(&corpus).earned_kinds();
    let expected = registry_gen::render_for(&corpus, &earned).expect("registry renders");
    let committed = std::fs::read_to_string(Corpus::corpus_root().join("registry.toml"))
        .expect("committed registry exists");

    assert_eq!(
        committed, expected,
        "registry.toml is stale -- run `cargo run -p bss-pricing --example regen_registry` \
         and commit the regeneration on its own"
    );
}

#[test]
fn the_committed_publish_flags_are_exactly_what_a_run_earned() {
    // No flag is set by hand. The committed file must agree, kind for kind, with
    // what running the validator over the corpus produces right now -- otherwise
    // a `true` in the file is a claim nothing backs.
    let corpus = corpus();
    let earned = validator::publish_report(&corpus).earned_kinds();
    let committed = Registry::load(&Corpus::corpus_root().join("registry.toml"))
        .expect("committed registry parses");

    for kind in ModelKind::ALL {
        let variant = committed
            .variants
            .iter()
            .find(|v| v.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} must be registered"));
        assert_eq!(
            variant.publish,
            earned.contains(&kind),
            "the committed publish flag for {kind:?} is not what a validator run earns"
        );
    }
}

/// Every publish case the gear does not answer as the corpus states, pinned.
///
/// **The disagreement is the product.** The corpus and the gear are two
/// independent readings of one design set, so where they differ a human decides
/// which is wrong — the expected value is never adjusted to make a run pass, and
/// neither is the rule. This list therefore exists to be *loud*: it fails the
/// moment either side moves, which is exactly when the decision is due.
///
/// It is empty, and each entry was closed by deciding which side was wrong:
///
/// - `price-change-accepted`, `unit-change-rejected`, `kind-flip-rejected`,
///   `carry-allowance-change-rejected`, `package-size-change-rejected` — the
///   gear answered `EVAL_POLICY_MISSING`. **The gear was right and the corpus
///   was under-authored.** None of those successors carried
///   `tierAggregationWindow`, which `inst-tb-window` requires on every tiered
///   usage row and `inst-pk-window` (D-58) on every `package` usage row. They
///   were authored as *deltas*, but the publish question is asked of a whole
///   row, and a row that would not publish on an empty key does not publish on
///   an occupied one either. Fixed in the corpus: both sides of each pair now
///   carry the window, equal, so the guard's subject is unchanged.
/// - `consumption-on-level-rejected` — the gear cannot assess it at all: the
///   snapshot carries the Slice-10 reservation pair and `PriceRow` is the
///   Slice-3 shape. **The corpus was right and the gear is not there yet**, so
///   neither side moved. The corpus now *says* so, with `declined_until` on the
///   case — the `trailing-tier` treatment at case granularity — and a decline is
///   therefore the anticipated answer rather than a disagreement. The verdict
///   stays authored and stays checked; see the two tests below.
#[test]
fn the_open_corpus_disagreements_are_exactly_these() {
    let report = validator::publish_report(&corpus());

    let mut failing: Vec<&str> = report.failures().map(|o| o.case_id.as_str()).collect();
    failing.sort_unstable();

    assert_eq!(
        failing,
        Vec::<&str>::new(),
        "the corpus/gear disagreement set moved; a human decides which side was wrong \
         before this list is edited"
    );
}

#[test]
fn every_kind_has_earned_its_publish_half() {
    // The consequence of the list above, stated where the gate reads it. The gate
    // is `oracle && publish`, and both halves are now earned for all five kinds
    // by two runs with different owners.
    //
    // `flat` and `per_unit` earn theirs from publish cases that did not exist
    // before: the corpus carried none for either kind, and since the flag is
    // earned per kind by a passing run, a kind with no case could never earn it
    // -- the gate stayed shut forever rather than failing. That is now a build
    // failure (`check_publish_case_coverage`), not a silence.
    let corpus = corpus();
    let earned = validator::publish_report(&corpus).earned_kinds();

    assert_eq!(
        earned,
        ModelKind::ALL.to_vec(),
        "a kind lost its publish half; the disagreement list above is stale"
    );

    let committed = Registry::load(&Corpus::corpus_root().join("registry.toml"))
        .expect("committed registry parses");
    for kind in ModelKind::ALL {
        assert!(
            committed.gate_open_for(kind),
            "{kind:?} earned both halves and must open the fixture gate"
        );
    }
}

#[test]
fn a_declined_case_is_reported_as_undecidable_not_as_a_verdict() {
    // Failing rather than guessing is the sanctioned answer to a snapshot the
    // row shape cannot hold, and it must stay distinguishable from a rejection:
    // an `EvalError` says the gear has no opinion, a `Rejected` says it has one.
    let report = validator::publish_report(&corpus());

    let reserved = report
        .outcomes
        .iter()
        .find(|o| o.case_id == "consumption-on-level-rejected")
        .expect("the reserved publish case is in the corpus");

    assert!(
        validator::describe_answer(&reserved.actual).starts_with("undecidable"),
        "a Slice-10 snapshot must be declined, not judged against a Slice-3 row"
    );
}

#[test]
fn the_undecidable_case_is_the_corpuss_own_declaration_and_earns_nothing() {
    // The decline is anticipated because the **corpus** says the case is
    // authored against a slice nothing has built -- not because the gear was
    // allowed to skip a case it found inconvenient. That distinction is the
    // whole of why the marker lives in the case file.
    //
    // Note what it does not do: `graduated` still earns its publish half, from
    // the three Slice-3 cases it does answer. A Slice-10 rule the gear cannot
    // reach is not evidence against Slice-3 rules it reproduces exactly.
    let report = validator::publish_report(&corpus());

    let declined: Vec<&str> = report.declined().map(|o| o.case_id.as_str()).collect();
    assert_eq!(declined, vec!["consumption-on-level-rejected"]);

    assert_eq!(
        report.stale_declines().count(),
        0,
        "the gear answered a case the corpus calls unanswerable; the declaration is stale \
         and the case owes the registry its evidence again"
    );
    assert!(
        validator::publish_report(&corpus())
            .earned_kinds()
            .contains(&ModelKind::Graduated)
    );
}
