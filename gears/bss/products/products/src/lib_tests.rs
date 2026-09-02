//! Crate-wide source hygiene: the one defect class no `cargo` gate can see.

/// Every `.rs` file at or under `src`, recursively — the whole crate.
///
/// Copied from the sibling pricing gear's `crate_sources` in
/// `tests/module_test.rs`, and for its reason: a question about what the
/// *crate* contains is answered by the crate, and a scan narrower than it
/// would report a hole it never looked into as absent. Here that matters
/// because the class below has appeared in `api/rest`, in `infra/storage` and
/// in `domain` alike.
fn crate_sources() -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// Runs of six-or-more spaces sitting **mid-prose** on one source line.
///
/// Mid-prose means a word character on both sides, which is what separates the
/// defect from the legitimate uses: a migration aligning SQL column types and
/// a test aligning a table both pad *between* columns, never between the last
/// letter of one word and the first of the next.
fn baked_runs(line: &str) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let prose_before = |c: char| c.is_alphanumeric() || ",;.:`)".contains(c);
    let prose_after = |c: char| c.is_alphanumeric() || "`(".contains(c);
    let mut runs = 0;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != ' ' {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        if i - start >= 6
            && start > 0
            && i < chars.len()
            && prose_before(chars[start - 1])
            && prose_after(chars[i])
        {
            runs += 1;
        }
    }
    runs
}

/// **A `\` line-continuation dropped in a join bakes its indentation into the
/// value**, and nothing else in the toolchain will tell you.
///
/// # The class
///
/// A long string literal is written as wrapped source lines joined by `\`,
/// which rustc strips along with the next line's indentation. Rejoin those
/// lines by hand — or paste them together — and drop the `\`, and the
/// indentation stays inside the string. What ships is prose with a six-,
/// ten- or eighteen-space gap in the middle of a sentence.
///
/// This crate carried **twenty-six** such runs across ten lines and three
/// files before 2026-09-02, the worst of them thirteen runs inside the
/// `OpenAPI` `.description` of the Product save door — text published to
/// consumers — and others inside `RepoError::Db` details an operator reads.
/// They came from five different commits and more than one author, so this is
/// a class and not a slip.
///
/// # Why it must be an assertion
///
/// `cargo fmt` does not reformat string literals and `clippy` does not read
/// their contents, so **every one of the twenty-six passed `fmt --check`,
/// `clippy --all-targets -D warnings`, `doc` and the whole suite**, for weeks.
/// `cfs check-language` reads prose in the docs, not in the code. There is no
/// gate above this one.
///
/// # What it looks at, and what it deliberately does not
///
/// Only lines longer than 100 characters, `rustfmt`'s `max_width`: a line that
/// long carrying prose is a joined literal `rustfmt` could not break, whereas
/// a short line with a wide gap is alignment. Measured against this crate that
/// pair of conditions is exactly discriminating — ten true positives, no false
/// ones. A baked run on a short line would slip past, and that is the accepted
/// cost of not writing a Rust parser here.
#[test]
fn no_source_line_bakes_a_continuations_indentation_into_a_literal() {
    let mut offenders = Vec::new();
    for path in crate_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable crate source");
        for (index, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") || line.chars().count() <= 100 {
                continue;
            }
            let runs = baked_runs(line);
            if runs > 0 {
                let name = path
                    .strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or(&path);
                offenders.push(format!("{}:{} — {runs} run(s)", name.display(), index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these lines carry a line-continuation's indentation inside a string literal, which an \
         operator or an API consumer reads as a gap mid-sentence. Re-wrap the literal with `\\` \
         at each line end, as `domain::governance::NO_POLICY_REASON` does:\n  {}",
        offenders.join("\n  ")
    );
}

/// The guard above is only worth having if it fails on what actually shipped.
///
/// The three inputs are the real shapes: the value this crate carried, a
/// legitimately aligned SQL line from a migration, and a correctly
/// `\`-continued literal.
#[test]
fn the_guard_fires_on_what_shipped_and_not_on_alignment() {
    let shipped = format!(
        "        {}",
        "\"no PII detector or allow-list is registered at this commit, so this host inspects \
         nothing      and admits every string; this is a deviation owed to slice \
         10-retention-erasure\""
    );
    assert_eq!(baked_runs(&shipped), 1, "the value that shipped is caught");

    assert_eq!(
        baked_runs(
            "            .col(ColumnDef::new(Column::TenantId)      .uuid()      .not_null())"
        ),
        0,
        "alignment between a `)` and a `.` is not prose and is left alone"
    );
    assert_eq!(
        baked_runs("    \"a peer submission superseded the open record first, so it could not \\"),
        0,
        "a correctly continued literal carries no run at all"
    );
}
