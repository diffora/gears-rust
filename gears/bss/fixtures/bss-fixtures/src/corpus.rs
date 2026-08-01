//! Loads the on-disk corpus.
//!
//! One directory per family; `_family.toml` declares what the family gates and
//! every other `.toml` is a case. A case whose `family` disagrees with its
//! directory is an error — the directory is the index, so the two must not
//! drift.

use crate::model::{Case, CaseHeader, CaseKind, Family, ModelKind};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path}: declares family {declared:?} but sits in the {directory:?} directory")]
    Misfiled {
        path: String,
        declared: Family,
        directory: Family,
    },
    #[error("{path}: directory name is not a known family")]
    UnknownFamilyDirectory { path: String },
}

/// What a family is for.
///
/// Not every joint fixture is a publish gate. `proration` is AC #61 — a
/// field-consumption contract shared with Subscriptions and Tariffs — and
/// blocks no publish at all. Stating the role explicitly is what separates
/// "gates nothing deliberately" from "someone forgot the gates list".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateRole {
    /// Blocks publish of the listed `modelKind`s through `FixtureGate`, **in
    /// this family's [`Variant`](crate::Variant)**.
    ///
    /// The variant is the second half of the sentence and it is not authored
    /// here: it is read off the family ([`crate::model::Family::variant`]). A
    /// `Publish` family therefore says "these kinds may not publish *this
    /// scenario* without me", which is what lets `supersession-continuity` gate
    /// the tiered kinds (D-22) without claiming to be their `modelKind` fixture.
    Publish,
    /// A joint conformance regression that blocks no publish.
    Conformance,
}

/// A family's `_family.toml` — what this family gates, and why it exists.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyMeta {
    pub family: Family,
    pub role: GateRole,
    /// The `modelKind`s whose publish this family gates. Empty exactly when the
    /// role is `Conformance`.
    #[serde(default)]
    pub gates: Vec<ModelKind>,
    pub provenance: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Corpus {
    pub cases: Vec<Case>,
    pub families: Vec<FamilyMeta>,
}

impl Corpus {
    /// The committed corpus, resolved relative to this crate's manifest.
    #[must_use]
    pub fn corpus_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus")
    }

    /// Loads every family declaration and case under `root`.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] if a directory cannot be read, a file does not
    /// parse, a directory name is not a known family, or a file declares a
    /// different family than the directory it sits in.
    pub fn load(root: &Path) -> Result<Self, CorpusError> {
        let mut cases = Vec::new();
        let mut families = Vec::new();

        let mut dirs: Vec<PathBuf> = read_dir(root)?.into_iter().filter(|p| p.is_dir()).collect();
        dirs.sort();

        for dir in dirs {
            let dir_family = family_from_dir_name(&dir)?;

            let mut entries: Vec<PathBuf> = read_dir(&dir)?
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            entries.sort();

            for path in entries {
                let text = fs::read_to_string(&path).map_err(|source| CorpusError::Io {
                    path: display(&path),
                    source,
                })?;

                if path.file_name().is_some_and(|n| n == "_family.toml") {
                    let meta: FamilyMeta =
                        toml::from_str(&text).map_err(|source| CorpusError::Parse {
                            path: display(&path),
                            source,
                        })?;
                    check_filed(&path, meta.family, dir_family)?;
                    families.push(meta);
                } else {
                    let case = parse_case(&path, &text)?;
                    check_filed(&path, case.family(), dir_family)?;
                    cases.push(case);
                }
            }
        }

        Ok(Corpus { cases, families })
    }

    pub fn cases_for(&self, family: Family) -> impl Iterator<Item = &Case> {
        self.cases.iter().filter(move |c| c.family() == family)
    }
}

/// Reads `kind` first, then parses the whole file into the matching type.
///
/// Two passes on purpose: an internally tagged enum cannot carry
/// `deny_unknown_fields`, and rejecting stray keys is the property that keeps
/// the snapshot/runtime ownership boundary honest.
fn parse_case(path: &Path, text: &str) -> Result<Case, CorpusError> {
    let parse_err = |source: toml::de::Error| CorpusError::Parse {
        path: display(path),
        source,
    };

    let header: CaseHeader = toml::from_str(text).map_err(parse_err)?;

    Ok(match header.kind {
        CaseKind::Evaluation => {
            Case::Evaluation(Box::new(toml::from_str(text).map_err(parse_err)?))
        }
        CaseKind::Publish => Case::Publish(Box::new(toml::from_str(text).map_err(parse_err)?)),
    })
}

fn check_filed(path: &Path, declared: Family, directory: Family) -> Result<(), CorpusError> {
    if declared == directory {
        Ok(())
    } else {
        Err(CorpusError::Misfiled {
            path: display(path),
            declared,
            directory,
        })
    }
}

fn family_from_dir_name(dir: &Path) -> Result<Family, CorpusError> {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    // Reuse the serde mapping so directory names and the `family` field cannot
    // drift apart.
    toml::from_str::<FamilyName>(&format!("family = {name:?}"))
        .map(|f| f.family)
        .map_err(|_| CorpusError::UnknownFamilyDirectory { path: display(dir) })
}

#[derive(Deserialize)]
struct FamilyName {
    family: Family,
}

fn read_dir(path: &Path) -> Result<Vec<PathBuf>, CorpusError> {
    let iter = fs::read_dir(path).map_err(|source| CorpusError::Io {
        path: display(path),
        source,
    })?;

    let mut out = Vec::new();
    for entry in iter {
        let entry = entry.map_err(|source| CorpusError::Io {
            path: display(path),
            source,
        })?;
        out.push(entry.path());
    }
    Ok(out)
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
#[path = "corpus_tests.rs"]
mod corpus_tests;
