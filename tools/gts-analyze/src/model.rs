//! Data structures: types/instances/references found in a module, plus the aggregate Report.

use serde::Serialize;
use std::collections::BTreeMap;

/// A GTS Type definition found in the module (Rust macro, JSON schema, or `struct_to_gts_schema!`).
#[derive(Serialize, Debug, Clone)]
pub struct TypeDef {
    pub gts_id: String,
    pub file: String,
    pub line: usize,
    pub source_kind: &'static str,
    pub location: String,
    pub struct_name: Option<String>,
    pub base: Option<String>,
    pub dir_path: Option<String>,
    pub properties: Option<String>,
    pub description: Option<String>,
}

/// A GTS Instance declared with `gts_instance!` or `gts_instance_raw!`.
#[derive(Serialize, Debug, Clone)]
pub struct InstanceDef {
    pub gts_id: String,
    pub file: String,
    pub line: usize,
    pub source_kind: &'static str,
    pub location: String,
    pub typed_as: Option<String>,
}

/// Any GTS-shaped string literal found in source / docs.
#[derive(Serialize, Debug, Clone)]
pub struct Reference {
    pub gts_id: String,
    pub file: String,
    pub line: usize,
    pub location: String,
    pub context: String,
}

/// Aggregate scan result. `verbose` controls rendering only; not part of JSON payload.
#[derive(Serialize, Debug, Default)]
pub struct Report {
    pub module_root: String,
    pub include_tests: bool,
    pub skip_docs: bool,
    #[serde(skip_serializing)]
    pub verbose: bool,
    pub file_counts: BTreeMap<String, usize>,
    pub location_counts: BTreeMap<String, usize>,
    pub types: Vec<TypeDef>,
    pub instances: Vec<InstanceDef>,
    pub references: Vec<Reference>,
}
