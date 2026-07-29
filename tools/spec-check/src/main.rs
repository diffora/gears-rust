use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use spec_check::finding::Severity;
use spec_check::invariants::closure::DeclaredInstructions;
use spec_check::report;
use spec_check::targets::{self, SeamIndex};
use spec_check::{Corpus, Finding, invariants};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
enum Gate {
    Low,
    Medium,
    High,
}

#[derive(Parser, Debug)]
#[command(
    name = "spec-check",
    about = "Cross-document invariants over BSS gear specs"
)]
struct Args {
    /// One or more gear `docs/` directories.
    #[arg(long = "gear", required = true, num_args = 1..)]
    gears: Vec<PathBuf>,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Lowest severity that fails the run. Applies only to findings that are not pinned,
    /// accepted debt (see `--show-known-debt`) — a pinned finding never fails the run,
    /// regardless of its severity.
    #[arg(long, value_enum, default_value_t = Gate::Medium)]
    max_severity: Gate,

    /// Also print the pinned, accepted-debt findings (tracked as D-69) that the default
    /// output only summarizes the count of. Never changes the exit code.
    #[arg(long)]
    show_known_debt: bool,
}

fn rank(s: Severity) -> Gate {
    match s {
        Severity::Low => Gate::Low,
        Severity::Medium => Gate::Medium,
        Severity::High => Gate::High,
    }
}

/// The run fails when at least one finding that is *not* pinned, accepted debt is at or
/// above `max_severity`. A pinned baseline finding never fails the run on its own, no
/// matter its severity or how high the gate is set — that is the entire point of pinning
/// it: known, worked-off (or at least tracked) debt should not block every run forever.
/// Pulled out as its own pure function (rather than inlined in `main`) so it has
/// something to be unit-tested against directly, without spawning the binary.
///
/// `findings` must all be from the same corpus, named by `gear` — both pinned baselines
/// are gear-qualified (task-review Ruling 3 fix, 2026-07-29, fix round 3), so the
/// known-debt decision needs to know which corpus produced each finding. `main` calls
/// this once per loaded corpus rather than once over every corpus's findings flattened
/// together, so a same-keyed finding from a different gear is never mistaken for
/// pricing's pinned debt.
fn is_failing(findings: &[Finding], gear: &str, max_severity: Gate) -> bool {
    findings
        .iter()
        .filter(|f| !report::is_known_debt(f, gear))
        .any(|f| rank(f.severity) >= max_severity)
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();

    // Load every `--gear` corpus up front, before checking any of them: P1's SEAMS
    // ownership check (see `targets::SeamIndex`) and P3's cross-gear instruction-id
    // closure (see `invariants::closure::DeclaredInstructions`) both need to know what
    // every loaded gear declares, not just the one currently being checked.
    let corpora: Vec<Corpus> = args
        .gears
        .iter()
        .map(|gear| Corpus::load(gear))
        .collect::<Result<_>>()?;
    let seams = SeamIndex::build(&corpora);
    let declared = DeclaredInstructions::build(&corpora);

    // Findings are partitioned into known-debt vs. live *per corpus*, before being
    // accumulated across the whole run — not by flattening every corpus's findings
    // first and partitioning once. Both pinned baselines are gear-qualified (task-review
    // Ruling 3 fix): the known-debt decision needs the gear each finding actually came
    // from, and that context only still exists here, one corpus at a time.
    let mut live: Vec<Finding> = Vec::new();
    let mut known_debt: Vec<Finding> = Vec::new();
    let mut failing = false;
    for corpus in &corpora {
        let mut corpus_findings = Vec::new();
        corpus_findings.extend(invariants::propagation::check(corpus, &seams, &corpora));
        corpus_findings.extend(invariants::fr_coverage::check(corpus));
        corpus_findings.extend(invariants::closure::check(corpus, &declared));

        let gear = targets::gear_name(corpus).unwrap_or_default();
        failing |= is_failing(&corpus_findings, &gear, args.max_severity);
        let (corpus_live, corpus_known_debt) = report::partition_known_debt(corpus_findings, &gear);
        live.extend(corpus_live);
        known_debt.extend(corpus_known_debt);
    }

    // Both renderings live in `report` (which owns the suppression policy they disclose) so
    // they are reachable from a test — see its `render_text` / `JsonReport` doc comments.
    match args.format {
        Format::Json => {
            let out = report::JsonReport::new(&live, &known_debt, args.show_known_debt);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Format::Text => {
            println!(
                "{}",
                report::render_text(&live, &known_debt, args.show_known_debt)
            );
        }
    }

    Ok(if failing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec_check::invariants::propagation::PINNED_PROPAGATION_GAPS_2026_07_29;

    fn pinned_propagation_finding() -> Finding {
        let (_, id, path) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
        Finding {
            invariant: "P1/propagation-missing".to_string(),
            severity: Severity::Medium,
            file: "DECISIONS.md".to_string(),
            line: Some(1),
            message: format!(
                "{id} claims propagation into {path}, but that document never cites {id}"
            ),
        }
    }

    #[test]
    fn a_run_whose_only_findings_are_pinned_baseline_entries_does_not_fail() {
        let findings = vec![pinned_propagation_finding()];
        assert!(!is_failing(&findings, "pricing", Gate::Medium));
    }

    #[test]
    fn a_run_with_one_extra_non_baseline_medium_finding_fails() {
        let findings = vec![
            pinned_propagation_finding(),
            Finding {
                invariant: "P1/propagation-missing".to_string(),
                severity: Severity::Medium,
                file: "DECISIONS.md".to_string(),
                line: Some(2),
                message: "D-999 claims propagation into PRD.md, but that document never cites \
                          D-999"
                    .to_string(),
            },
        ];
        assert!(is_failing(&findings, "pricing", Gate::Medium));
    }

    #[test]
    fn a_pinned_finding_above_the_gate_still_does_not_fail() {
        // The whole point of pinning: severity alone never re-triggers a baseline
        // entry, even against the lowest gate.
        let findings = vec![pinned_propagation_finding()];
        assert!(!is_failing(&findings, "pricing", Gate::Low));
    }

    #[test]
    fn a_same_keyed_finding_from_a_different_gear_fails_the_run() {
        // Cross-corpus safety (task-review Ruling 3, CRITICAL): a finding that is
        // byte-identical in its (id, path)-parseable message to a pinned pricing
        // baseline entry, but attributed to a different gear, must still fail the run —
        // it is new drift in that gear, not the pricing debt the baseline pins.
        let (gear, _, _) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
        assert_eq!(gear, "pricing", "test assumes entry 0 is pricing's");
        let findings = vec![pinned_propagation_finding()];
        assert!(is_failing(&findings, "rating", Gate::Medium));
    }
}
