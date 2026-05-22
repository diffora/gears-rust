#![feature(rustc_private)]
#![warn(unused_extern_crates)]

extern crate rustc_span;

use cargo_metadata::{Metadata, MetadataCommand, Package};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_span::DUMMY_SP;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default, serde::Deserialize)]
struct Config {
    #[serde(default)]
    excluded_crates: Vec<String>,
}

struct De1201DocsRsAllFeatures {
    excluded_crates: HashSet<String>,
}

impl De1201DocsRsAllFeatures {
    pub fn new() -> Self {
        let config: Config = dylint_linting::config_or_default(env!("CARGO_PKG_NAME"));
        Self {
            excluded_crates: config.excluded_crates.into_iter().collect(),
        }
    }
}

dylint_linting::impl_late_lint! {
    /// DE1201: Publishable crates must enable docs.rs all-features builds
    ///
    /// ### What it does
    ///
    /// Checks publishable crates for:
    ///
    /// ```toml
    /// [package.metadata.docs.rs]
    /// all-features = true
    /// ```
    ///
    /// ### Why
    ///
    /// docs.rs builds each crate with a constrained feature set unless configured
    /// otherwise. Enabling all features catches documentation failures for optional
    /// feature combinations before publishing and keeps public API docs complete.
    ///
    /// ### Scope
    ///
    /// - Applies to crates where Cargo metadata says publishing is allowed.
    /// - Skips crates with `publish = false`.
    /// - Skips crate names listed in `[de1201_docs_rs_all_features].excluded_crates`.
    pub DE1201_DOCS_RS_ALL_FEATURES,
    Warn,
    "publishable crates must set package.metadata.docs.rs.all-features = true (DE1201)",
    De1201DocsRsAllFeatures::new()
}

impl LateLintPass<'_> for De1201DocsRsAllFeatures {
    fn check_crate(&mut self, cx: &LateContext<'_>) {
        let Ok(manifest_path) = current_manifest_path() else {
            return;
        };

        let metadata = match MetadataCommand::new()
            .manifest_path(&manifest_path)
            .no_deps()
            .exec()
        {
            Ok(metadata) => metadata,
            Err(error) => {
                cx.span_lint(DE1201_DOCS_RS_ALL_FEATURES, DUMMY_SP, |diag| {
                    diag.primary_message(format!(
                        "could not read Cargo metadata for docs.rs configuration check: {error}"
                    ));
                });
                return;
            }
        };

        let Some(package) = find_current_package(&metadata, &manifest_path) else {
            cx.span_lint(DE1201_DOCS_RS_ALL_FEATURES, DUMMY_SP, |diag| {
                diag.primary_message(format!(
                    "could not find current package in Cargo metadata for `{}`",
                    manifest_path.display()
                ));
            });
            return;
        };

        let Some(status) = docs_rs_all_features_violation(
            package.name.as_ref(),
            package.publish.as_deref(),
            &package.metadata,
            &self.excluded_crates,
        ) else {
            return;
        };

        cx.span_lint(DE1201_DOCS_RS_ALL_FEATURES, DUMMY_SP, |diag| {
            diag.primary_message(format!(
                "publishable crate `{}` must set `package.metadata.docs.rs.all-features = true` (DE1201)",
                package.name
            ));
            diag.help(format!(
                "{}; add `[package.metadata.docs.rs] all-features = true` to `{}` or add `{}` to `[de1201_docs_rs_all_features].excluded_crates` in `dylint.toml`",
                status.help_reason(),
                package.manifest_path,
                package.name,
            ));
        });
    }
}

fn current_manifest_path() -> Result<PathBuf, std::env::VarError> {
    std::env::var("CARGO_MANIFEST_DIR").map(|dir| PathBuf::from(dir).join("Cargo.toml"))
}

fn find_current_package<'metadata>(
    metadata: &'metadata Metadata,
    manifest_path: &Path,
) -> Option<&'metadata Package> {
    let expected = normalize_path(&manifest_path.to_string_lossy());
    metadata.packages.iter().find(|package| {
        let actual = normalize_path(package.manifest_path.as_str());
        actual == expected
    })
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn docs_rs_all_features_violation(
    package_name: &str,
    publish: Option<&[String]>,
    metadata: &Value,
    excluded_crates: &HashSet<String>,
) -> Option<DocsRsAllFeaturesStatus> {
    if excluded_crates.contains(package_name) || !is_publishable(publish) {
        return None;
    }

    match docs_rs_all_features_status(metadata) {
        DocsRsAllFeaturesStatus::Enabled => None,
        status => Some(status),
    }
}

fn is_publishable(publish: Option<&[String]>) -> bool {
    publish.is_none_or(|registries| !registries.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocsRsAllFeaturesStatus {
    Enabled,
    MissingDocsRsTable,
    MissingAllFeatures,
    Disabled,
    NonBoolean,
}

impl DocsRsAllFeaturesStatus {
    fn help_reason(self) -> &'static str {
        match self {
            Self::Enabled => "docs.rs all-features is enabled",
            Self::MissingDocsRsTable => "`package.metadata.docs.rs` is missing",
            Self::MissingAllFeatures => "`package.metadata.docs.rs.all-features` is missing",
            Self::Disabled => "`package.metadata.docs.rs.all-features` is false",
            Self::NonBoolean => "`package.metadata.docs.rs.all-features` is not a boolean",
        }
    }
}

fn docs_rs_all_features_status(metadata: &Value) -> DocsRsAllFeaturesStatus {
    let Some(docs_rs) = docs_rs_metadata(metadata) else {
        return DocsRsAllFeaturesStatus::MissingDocsRsTable;
    };

    match docs_rs.get("all-features") {
        Some(Value::Bool(true)) => DocsRsAllFeaturesStatus::Enabled,
        Some(Value::Bool(false)) => DocsRsAllFeaturesStatus::Disabled,
        Some(_) => DocsRsAllFeaturesStatus::NonBoolean,
        None => DocsRsAllFeaturesStatus::MissingAllFeatures,
    }
}

fn docs_rs_metadata(metadata: &Value) -> Option<&Value> {
    metadata
        .get("docs")
        .and_then(|docs| docs.get("rs"))
        .or_else(|| metadata.get("docs.rs"))
}

#[cfg(test)]
mod tests {
    use super::{
        DocsRsAllFeaturesStatus, docs_rs_all_features_status, docs_rs_all_features_violation,
        is_publishable,
    };
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn publish_omitted_is_publishable() {
        assert!(is_publishable(None));
    }

    #[test]
    fn publish_empty_list_is_not_publishable() {
        let publish = Vec::new();
        assert!(!is_publishable(Some(&publish)));
    }

    #[test]
    fn publish_non_empty_list_is_publishable() {
        let publish = vec!["crates-io".to_string()];
        assert!(is_publishable(Some(&publish)));
    }

    #[test]
    fn missing_docs_rs_table_is_violation() {
        assert_eq!(
            docs_rs_all_features_status(&json!({})),
            DocsRsAllFeaturesStatus::MissingDocsRsTable
        );
    }

    #[test]
    fn missing_all_features_is_violation() {
        assert_eq!(
            docs_rs_all_features_status(&json!({
                "docs": {
                    "rs": {}
                }
            })),
            DocsRsAllFeaturesStatus::MissingAllFeatures
        );
    }

    #[test]
    fn false_all_features_is_violation() {
        assert_eq!(
            docs_rs_all_features_status(&json!({
                "docs": {
                    "rs": {
                        "all-features": false
                    }
                }
            })),
            DocsRsAllFeaturesStatus::Disabled
        );
    }

    #[test]
    fn non_boolean_all_features_is_violation() {
        assert_eq!(
            docs_rs_all_features_status(&json!({
                "docs": {
                    "rs": {
                        "all-features": "true"
                    }
                }
            })),
            DocsRsAllFeaturesStatus::NonBoolean
        );
    }

    #[test]
    fn true_all_features_is_allowed() {
        assert_eq!(
            docs_rs_all_features_status(&json!({
                "docs": {
                    "rs": {
                        "all-features": true
                    }
                }
            })),
            DocsRsAllFeaturesStatus::Enabled
        );
    }

    #[test]
    fn quoted_docs_rs_table_is_allowed() {
        assert_eq!(
            docs_rs_all_features_status(&json!({
                "docs.rs": {
                    "all-features": true
                }
            })),
            DocsRsAllFeaturesStatus::Enabled
        );
    }

    #[test]
    fn publish_false_skips_violation() {
        let publish = Vec::new();
        let exclusions = HashSet::new();

        assert_eq!(
            docs_rs_all_features_violation(
                "internal-crate",
                Some(&publish),
                &json!({}),
                &exclusions
            ),
            None
        );
    }

    #[test]
    fn excluded_crate_skips_violation() {
        let exclusions = HashSet::from(["excluded-crate".to_string()]);

        assert_eq!(
            docs_rs_all_features_violation("excluded-crate", None, &json!({}), &exclusions),
            None
        );
    }

    #[test]
    fn publishable_missing_metadata_reports_violation() {
        let exclusions = HashSet::new();

        assert_eq!(
            docs_rs_all_features_violation("publishable-crate", None, &json!({}), &exclusions),
            Some(DocsRsAllFeaturesStatus::MissingDocsRsTable)
        );
    }
}
