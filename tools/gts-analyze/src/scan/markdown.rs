//! Generic regex-based ref scanner for non-Rust text files.

use std::path::Path;

use regex::Regex;

use crate::classify::classify_location;
use crate::model::Reference;
use crate::scan::{gts_bare_re, gts_in_string_re, line_at, shorten_line};

/// Scan a `.md` / `.toml` / `.yaml` / etc. file for GTS-id references.
/// `.md` uses bare-id matching (no surrounding quotes required);
/// everything else uses quoted-string matching.
pub fn scan_file(rel: &Path, text: &str, ext: &str, out: &mut Vec<Reference>) {
    let regex: &Regex = if ext == "md" {
        gts_bare_re()
    } else {
        gts_in_string_re()
    };
    let location = classify_location(rel).to_string();
    let rel_str = rel.to_string_lossy().into_owned();

    for cap in regex.captures_iter(text) {
        let m = cap.get(1).expect("group 1 always present");
        let gts_id = m.as_str().to_string();
        let (line_no, line_text) = line_at(text, m.start());
        out.push(Reference {
            gts_id,
            file: rel_str.clone(),
            line: line_no,
            location: location.clone(),
            context: shorten_line(line_text),
        });
    }
}
