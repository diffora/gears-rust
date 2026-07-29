use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use spec_check::finding::Severity;
use spec_check::invariants::closure::DeclaredInstructions;
use spec_check::report::{self, KNOWN_DEBT_TICKET};
use spec_check::targets::SeamIndex;
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
fn is_failing(findings: &[Finding], max_severity: Gate) -> bool {
    findings
        .iter()
        .filter(|f| !report::is_known_debt(f))
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

    let mut findings: Vec<Finding> = Vec::new();
    for corpus in &corpora {
        findings.extend(invariants::propagation::check(corpus, &seams));
        findings.extend(invariants::fr_coverage::check(corpus));
        findings.extend(invariants::closure::check(corpus, &declared));
    }

    let failing = is_failing(&findings, args.max_severity);
    let (live, known_debt) = report::partition_known_debt(findings);

    match args.format {
        Format::Json => {
            #[derive(serde::Serialize)]
            struct Report<'a> {
                findings: &'a [Finding],
                known_debt_suppressed: usize,
                known_debt_tracked_as: &'static str,
                #[serde(skip_serializing_if = "Option::is_none")]
                known_debt: Option<&'a [Finding]>,
            }
            let out = Report {
                findings: &live,
                known_debt_suppressed: known_debt.len(),
                known_debt_tracked_as: KNOWN_DEBT_TICKET,
                known_debt: args.show_known_debt.then_some(known_debt.as_slice()),
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Format::Text => {
            for f in &live {
                println!("{}", f.render());
            }
            if args.show_known_debt && !known_debt.is_empty() {
                println!(
                    "\nKnown debt — accepted, tracked as {KNOWN_DEBT_TICKET}, not new drift ({} finding(s)):",
                    known_debt.len()
                );
                for f in &known_debt {
                    println!("{}", f.render());
                }
            }
            println!("\n{} finding(s)", live.len());
            if !known_debt.is_empty() {
                if args.show_known_debt {
                    println!(
                        "{} known-debt finding(s) shown above, tracked as {KNOWN_DEBT_TICKET} \
                         (accepted, not new drift)",
                        known_debt.len()
                    );
                } else {
                    println!(
                        "{} known-debt finding(s) suppressed, tracked as {KNOWN_DEBT_TICKET} \
                         — pass --show-known-debt to see them",
                        known_debt.len()
                    );
                }
            }
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
        let (id, path) = PINNED_PROPAGATION_GAPS_2026_07_29[0];
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
        assert!(!is_failing(&findings, Gate::Medium));
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
        assert!(is_failing(&findings, Gate::Medium));
    }

    #[test]
    fn a_pinned_finding_above_the_gate_still_does_not_fail() {
        // The whole point of pinning: severity alone never re-triggers a baseline
        // entry, even against the lowest gate.
        let findings = vec![pinned_propagation_finding()];
        assert!(!is_failing(&findings, Gate::Low));
    }
}
