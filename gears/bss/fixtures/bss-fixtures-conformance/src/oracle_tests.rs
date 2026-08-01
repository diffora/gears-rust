use super::*;
use crate::traits::EvalInput;
use crate::traits::Evaluated;
use bss_fixtures::{Band, BandTop, Corpus, Expect, Family};

/// Every assertion in a family must be reproduced by the oracle. This is what
/// "green" means for the `oracle` flag.
fn assert_family_reproduced(family: Family) {
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");
    let oracle = ReferenceOracle;

    let mut checked = 0;
    for case in corpus.cases_for(family) {
        // Publish cases are answered by a `PublishValidator`, not the oracle.
        let bss_fixtures::Case::Evaluation(case) = case else {
            continue;
        };
        for (i, a) in case.assert.iter().enumerate() {
            let input = EvalInput {
                snapshot: &case.snapshot,
                runtime: &case.runtime,
                given: &a.given,
            };
            let got = oracle
                .evaluate(&input)
                .unwrap_or_else(|e| panic!("{}[{i}]: oracle errored: {e}", case.id));
            let want = match a.expect {
                Expect::Charge(c) => Evaluated::Charge(c.charge_minor),
                Expect::Units(u) => Evaluated::Units {
                    charged: u.units_charged,
                    in_basis: u.units_in_basis,
                },
                Expect::Fold(f) => Evaluated::Fold { q: f.folded_q },
            };
            assert_eq!(
                got, want,
                "{}[{i}]: oracle says {got:?}, corpus says {want:?}",
                case.id
            );
            checked += 1;
        }
    }

    assert!(
        checked > 0,
        "{family:?} has no assertions -- an empty family is not green"
    );
}

fn two_bands() -> Vec<Band> {
    vec![
        Band {
            from_qty: 0,
            to_qty: BandTop::Closed(1000),
            unit_amount_minor: 5,
        },
        Band {
            from_qty: 1000,
            to_qty: BandTop::Open,
            unit_amount_minor: 3,
        },
    ]
}

#[test]
fn oracle_reproduces_tier_boundary() {
    assert_family_reproduced(Family::TierBoundary);
}

#[test]
fn graduated_is_marginal_at_the_band_edge() {
    // Guards the single most expensive misreading: treating q == to_qty as
    // already inside the next band for graduated.
    let bands = two_bands();
    assert_eq!(graduated(&bands, 1000).unwrap(), 5000);
    assert_eq!(graduated(&bands, 1001).unwrap(), 5003);
}

#[test]
fn volume_selects_one_band_for_the_whole_quantity() {
    let bands = two_bands();
    assert_eq!(volume(&bands, 999).unwrap(), 4995);
    assert_eq!(volume(&bands, 1000).unwrap(), 3000);
}

#[test]
fn oracle_reproduces_package() {
    assert_family_reproduced(Family::Package);
}

#[test]
fn package_rounds_blocks_up_never_down() {
    assert_eq!(package(1000, 2500, 0).unwrap(), 0);
    assert_eq!(package(1000, 2500, 1).unwrap(), 2500);
    assert_eq!(package(1000, 2500, 1000).unwrap(), 2500);
    assert_eq!(package(1000, 2500, 1001).unwrap(), 5000);
}

#[test]
fn package_rejects_a_zero_size() {
    // `div_ceil` would panic; fail closed instead.
    assert_eq!(package(0, 2500, 10), Err(EvalError::ZeroPackageSize));
}

#[test]
fn oracle_reproduces_per_unit() {
    assert_family_reproduced(Family::PerUnit);
}

#[test]
fn oracle_reproduces_flat() {
    assert_family_reproduced(Family::Flat);
}

#[test]
fn flat_ignores_the_quantity_where_per_unit_scales_by_it() {
    // The pair that gets confused: PRD 6.2 bans `flat` on a usage row precisely
    // because the plain untiered metered rate is `per_unit`. An evaluator that
    // multiplies a flat row has silently swapped one for the other.
    let flat = bss_fixtures::Snapshot {
        model_kind: bss_fixtures::ModelKind::Flat,
        // `flat` is legal on non-usage rows only, and this pair is the whole
        // point of `inst-mk-chargekind`: the two kinds it separates are these
        // two, on the same charge component.
        charge_kind: bss_fixtures::ChargeKind::Recurring,
        currency: "USD".into(),
        bands: Vec::new(),
        amount_minor: Some(9900),
        package_size: None,
        package_price_minor: None,
        quantity_source: None,
        tier_aggregation_window: None,
        billing_granularity: None,
        proration_basis: None,
        meter: None,
        dimension_key: None,
        aggregation_function: None,
        aggregation_granularity: None,
        max_hold_granules: None,
        tier_qualification_window: None,
        included_allowance: None,
        reserved_rate_minor: None,
        reservation_flavor: None,
    };
    let per_unit = bss_fixtures::Snapshot {
        model_kind: bss_fixtures::ModelKind::PerUnit,
        ..flat.clone()
    };
    let rt = bss_fixtures::Runtime::default();
    let oracle = ReferenceOracle;

    for q in [0, 1, 1000] {
        let flat_charge = oracle
            .evaluate(&EvalInput {
                snapshot: &flat,
                runtime: &rt,
                given: &bss_fixtures::Given {
                    q,
                    ..bss_fixtures::Given::default()
                },
            })
            .expect("flat prices");
        assert_eq!(
            flat_charge,
            Evaluated::Charge(9900),
            "flat must not move with q={q}"
        );
    }

    let per_unit_charge = oracle
        .evaluate(&EvalInput {
            snapshot: &per_unit,
            runtime: &rt,
            given: &bss_fixtures::Given {
                q: 3,
                ..bss_fixtures::Given::default()
            },
        })
        .expect("per_unit prices");
    assert_eq!(
        per_unit_charge,
        Evaluated::Charge(29700),
        "per_unit must scale by q"
    );
}

#[test]
fn oracle_reproduces_proration() {
    assert_family_reproduced(Family::Proration);
}

#[test]
fn a_split_period_sums_to_the_basis_exactly() {
    // Rating T-D-26: the slice fractions of one period must sum to exactly 1.
    // In integer units that is an exact equality, not a float comparison — which
    // is the whole reason this family reports a ratio instead of money.
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");
    let case = corpus
        .cases_for(Family::Proration)
        .find_map(|c| match c {
            bss_fixtures::Case::Evaluation(e) if e.id == "pricewindow-split-sums-to-one" => Some(e),
            _ => None,
        })
        .expect("the split case must exist");
    let oracle = ReferenceOracle;

    let mut slices = Vec::new();
    let mut basis = None;
    for a in &case.assert {
        // The uncut whole-period assertion is not a slice; skip it.
        if a.given.from.is_none() && a.given.to.is_none() {
            continue;
        }
        let got = oracle
            .evaluate(&EvalInput {
                snapshot: &case.snapshot,
                runtime: &case.runtime,
                given: &a.given,
            })
            .expect("slice prorates");
        if let Evaluated::Units { charged, in_basis } = got {
            slices.push(charged);
            basis = Some(in_basis);
        }
    }

    assert_eq!(slices.len(), 2, "the case must carry two slices");
    assert_eq!(
        slices.iter().sum::<u64>(),
        basis.expect("a basis"),
        "no unit may be billed twice or lost at the cut"
    );
}

#[test]
fn oracle_reproduces_supersession_continuity() {
    assert_family_reproduced(Family::SupersessionContinuity);
}

#[test]
fn a_reset_counter_would_price_differently() {
    // What `inst-tb-window-continuity` is worth in money. The successor's bands
    // applied to the continued window total (1500) against the same bands
    // applied to post-supersession usage alone (900): a 2700 difference on one
    // row. The corpus asserts only the correct number; this pins the size of
    // the mistake it prevents.
    let bands = vec![
        Band {
            from_qty: 0,
            to_qty: BandTop::Closed(1000),
            unit_amount_minor: 7,
        },
        Band {
            from_qty: 1000,
            to_qty: BandTop::Open,
            unit_amount_minor: 4,
        },
    ];

    let continued = graduated(&bands, 1500).expect("prices");
    let had_it_reset = graduated(&bands, 900).expect("prices");

    assert_eq!(continued, 9000);
    assert_eq!(had_it_reset, 6300);
    assert_eq!(continued - had_it_reset, 2700);
}

#[test]
fn the_supersession_guard_pins_both_verdicts() {
    // A guard that only ever rejected would make supersession unusable, so the
    // family has to carry the accepted case too: supersession *is* a price
    // change on one key.
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");

    let mut rejections = 0;
    let mut acceptances = 0;
    for case in corpus.cases_for(Family::SupersessionContinuity) {
        let bss_fixtures::Case::Publish(p) = case else {
            continue;
        };
        for a in &p.assert {
            match &a.expect {
                bss_fixtures::PublishVerdict::Rejected { error_code } => {
                    assert_eq!(
                        error_code, "SUPERSESSION_UNIT_MISMATCH",
                        "{}: the guard has one code",
                        p.id
                    );
                    rejections += 1;
                }
                bss_fixtures::PublishVerdict::Accepted => acceptances += 1,
            }
        }
    }

    assert!(rejections >= 4, "one case per protected field family");
    assert!(
        acceptances >= 1,
        "the legitimate price change must be pinned too"
    );
}

#[test]
fn oracle_reproduces_level_aggregation() {
    assert_family_reproduced(Family::LevelAggregation);
}

#[test]
fn a_late_sample_moves_only_its_own_granule() {
    // The base fold and the backfilled one differ by exactly the lift of hour
    // 1's peak: 15 -> 25. Hours 0 and 2 are untouched, which is what makes the
    // correction a standard delta rather than a window-wide recompute.
    let corpus = Corpus::load(&Corpus::corpus_root()).expect("corpus loads");
    let q_of = |id: &str| {
        let case = corpus
            .cases_for(Family::LevelAggregation)
            .find_map(|c| match c {
                bss_fixtures::Case::Evaluation(e) if e.id == id => Some(e),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{id} must exist"));
        match ReferenceOracle
            .evaluate(&EvalInput {
                snapshot: &case.snapshot,
                runtime: &case.runtime,
                given: &case.assert[0].given,
            })
            .expect("folds")
        {
            Evaluated::Fold { q } => q,
            other => panic!("{id} folded to {other:?}"),
        }
    };

    assert_eq!(q_of("late-sample-refold") - q_of("peak-granule-fold"), 5);
}

#[test]
fn oracle_reproduces_reserved() {
    assert_family_reproduced(Family::Reserved);
}

#[test]
fn volume_without_a_covering_band_fails_rather_than_guessing() {
    // A closed top band is publish-blocked (D-17), but the evaluator must not
    // invent a rate if one ever reaches it.
    let closed_top = vec![Band {
        from_qty: 0,
        to_qty: BandTop::Closed(10),
        unit_amount_minor: 5,
    }];

    assert_eq!(
        volume(&closed_top, 10),
        Err(EvalError::NoBandCoversQuantity(10))
    );
}
