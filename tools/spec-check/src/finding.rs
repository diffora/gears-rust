use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Stable id, e.g. `P1/propagation-missing`. Grep-able and safe to pin in tests.
    pub invariant: String,
    pub severity: Severity,
    /// Corpus-relative path of the document that must change.
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
}

impl Finding {
    pub fn render(&self) -> String {
        let loc = match self.line {
            Some(l) => format!("{}:{}", self.file, l),
            None => self.file.clone(),
        };
        format!(
            "[{:?}] {} — {} ({})",
            self.severity, loc, self.message, self.invariant
        )
    }
}
