use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

/// One gear's `docs/` tree, read once. Keys are paths relative to the tree root,
/// always with `/` separators so tests and findings read the same on every platform.
pub struct Corpus {
    root: PathBuf,
    files: BTreeMap<String, String>,
}

impl Corpus {
    pub fn load(root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
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
}
