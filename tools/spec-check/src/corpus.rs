use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

/// One gear's `docs/` tree, read once. Keys are paths relative to the tree root,
/// always with `/` separators so tests and findings read the same on every platform.
pub struct Corpus {
    root: PathBuf,
    files: BTreeMap<String, String>,
}

impl Corpus {
    /// Reads every `*.md` under `root`.
    ///
    /// Errors — rather than returning an empty corpus — when `root` is not an existing
    /// directory, and propagates every `WalkDir` error instead of dropping it. A typo in
    /// the Makefile or a renamed `docs/` directory used to yield `Ok` with zero files, so
    /// every invariant found nothing and the CLI exited 0 having checked nothing at all:
    /// the exact `enum_drift` failure mode this tool exists to catch, in the tool itself.
    /// A gate must never claim coverage it does not have, and a silent run must never be
    /// indistinguishable from a clean one.
    pub fn load(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            bail!(
                "{} is not an existing directory — a gear docs tree must exist to be \
                 checked (an empty corpus would silently pass every invariant)",
                root.display()
            );
        }
        let mut files = BTreeMap::new();
        for entry in WalkDir::new(root) {
            let entry =
                entry.with_context(|| format!("walking the docs tree under {}", root.display()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .with_context(|| format!("{} is outside {}", path.display(), root.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            files.insert(rel, text);
        }
        Ok(Self {
            root: root.to_path_buf(),
            files,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn text(&self, rel: &str) -> Option<&str> {
        self.files.get(rel).map(String::as_str)
    }

    pub fn has(&self, rel: &str) -> bool {
        self.files.contains_key(rel)
    }

    /// Builds an in-memory corpus. Test-only in practice, but not `#[cfg(test)]`
    /// so integration tests in `tests/` can use it too.
    pub fn from_parts<'a>(root: &str, parts: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            root: PathBuf::from(root),
            files: parts
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn pricing_docs() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gears/bss/pricing/docs")
    }

    #[test]
    fn loads_every_markdown_file_under_the_gear_docs_tree() {
        let corpus = Corpus::load(&pricing_docs()).expect("pricing corpus loads");
        // PRD, DESIGN, DECISIONS, STRIPE-GAP-ANALYSIS, 3 ADRs, 13 design/*.md
        assert!(
            corpus.files().count() >= 20,
            "got {}",
            corpus.files().count()
        );
        assert!(corpus.text("PRD.md").is_some());
        assert!(corpus.text("design/03-price-structure.md").is_some());
        assert!(corpus.text("does-not-exist.md").is_none());
    }

    #[test]
    fn a_nonexistent_root_is_an_error_not_an_empty_corpus() {
        // The failure this guards: `WalkDir::new(root).filter_map(Result::ok)` swallowed
        // the `Err` a missing root yields, so a mistyped `--gear` produced an `Ok` corpus
        // with zero files, every invariant found nothing, and the run exited 0 having
        // checked nothing. `load` already returned `Result`; the signature never needed
        // to lie.
        let root = pricing_docs().join("DOES-NOT-EXIST");
        let err = match Corpus::load(&root) {
            Err(e) => e,
            Ok(c) => panic!(
                "a missing gear docs tree must not load; got {} file(s)",
                c.files().count()
            ),
        };
        assert!(
            err.to_string().contains("DOES-NOT-EXIST"),
            "the error must name the path that is missing: {err}"
        );
    }

    #[test]
    fn a_file_passed_where_a_directory_is_expected_is_an_error() {
        // `--gear gears/bss/pricing/docs/PRD.md` walks exactly one file whose
        // `strip_prefix(root)` is empty, which would have loaded a corpus with a single
        // ""-keyed entry and no `PRD.md` — every lookup misses and the run is silently
        // vacuous, same class of defect as a missing root.
        let root = pricing_docs().join("PRD.md");
        assert!(Corpus::load(&root).is_err(), "a file is not a docs tree");
    }
}
