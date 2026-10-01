//! Every instant a door stores, and so echoes in its answer, is whole microseconds, as Postgres keeps
//! it: a write answers what it wrote (pricing D-453; phase 9 review B1). So a clock read in `src/`
//! is a date (`now_utc().date()`), a doc comment, the one helper that cuts it (`stored_now`), or a
//! named exception below. A door that reads `now_utc()` and stores it fails here, whether or not a
//! door test happens to cover it.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// Clock reads that keep the clock's own precision, each with its reason.
const ALLOWED: &[&str] = &[
    // `stored_now` itself: it cuts.
    "src/infra/storage.rs",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs")
            && !name.ends_with("_tests.rs")
            && name != "test_support.rs"
        {
            out.push(path);
        }
    }
}

#[test]
fn every_clock_read_in_src_is_a_date_or_goes_through_stored_now() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    let mut stray = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if !line.contains("now_utc()")
                || code.starts_with("//")
                || line.contains("now_utc().date()")
                || ALLOWED.contains(&rel.as_str())
            {
                continue;
            }
            stray.push(format!("{rel}:{}: {code}", n + 1));
        }
    }
    assert!(
        stray.is_empty(),
        "a stored instant must be cut to whole microseconds through stored_now(): {stray:#?}"
    );
}
