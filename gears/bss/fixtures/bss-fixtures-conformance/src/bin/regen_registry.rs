//! Regenerates `corpus/registry.toml` from the corpus.
//!
//! Run: `cargo run -p bss-fixtures-conformance --bin regen_registry`
//! The freshness test asserts the committed file matches. Commit a regeneration
//! on its own.

use bss_fixtures::Corpus;
use bss_fixtures_conformance::registry_gen;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Corpus::corpus_root();
    let corpus = Corpus::load(&root)?;

    std::fs::write(
        root.join("registry.toml"),
        registry_gen::render_for(&corpus)?,
    )?;

    Ok(())
}
