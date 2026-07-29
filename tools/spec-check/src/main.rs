use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use spec_check::finding::Severity;
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

    /// Lowest severity that fails the run.
    #[arg(long, value_enum, default_value_t = Gate::Medium)]
    max_severity: Gate,
}

fn rank(s: Severity) -> Gate {
    match s {
        Severity::Low => Gate::Low,
        Severity::Medium => Gate::Medium,
        Severity::High => Gate::High,
    }
}

fn main() -> Result<ExitCode> {
    let args = Args::parse();

    // Load every `--gear` corpus up front, before checking any of them: P1's SEAMS
    // ownership check (see `targets::SeamIndex`) needs to know what every loaded gear's
    // SEAMS.md defines, not just the one currently being checked.
    let corpora: Vec<Corpus> = args
        .gears
        .iter()
        .map(|gear| Corpus::load(gear))
        .collect::<Result<_>>()?;
    let seams = SeamIndex::build(&corpora);

    let mut findings: Vec<Finding> = Vec::new();
    for corpus in &corpora {
        findings.extend(invariants::propagation::check(corpus, &seams));
        findings.extend(invariants::fr_coverage::check(corpus));
        findings.extend(invariants::closure::check(corpus));
    }

    match args.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&findings)?),
        Format::Text => {
            for f in &findings {
                println!("{}", f.render());
            }
            println!("\n{} finding(s)", findings.len());
        }
    }

    let failing = findings
        .iter()
        .any(|f| rank(f.severity) >= args.max_severity);
    Ok(if failing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
