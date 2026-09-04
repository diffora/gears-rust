//! Crate-wide source hygiene: the one defect class no `cargo` gate can see.

/// Every `.rs` file at or under `src`, recursively — the whole crate.
///
/// Copied from the sibling pricing gear's `crate_sources` in
/// `tests/module_test.rs`, and for its reason: a question about what the
/// *crate* contains is answered by the crate, and a scan narrower than it
/// would report a hole it never looked into as absent. Here that matters
/// because the class below has appeared in `api/rest`, in `infra/storage` and
/// in `domain` alike.
pub fn crate_sources() -> Vec<std::path::PathBuf> {
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

/// **P-D-105's safety is a count, so the count is measured — and P-D-114 row 31 adds a second.**
///
/// That entry drops `PreAuthorized`'s subject/revision equality for a scheduled
/// flip and puts the authorization on the flipped row's stored `approval_ref`
/// instead. Its whole argument that this is not *"an unbounded bearer token for
/// any subject in the tenant"* — the phrase both `features/lifecycle.md` §7 row
/// 22 and `features/governance.md` §7 row 27 use — is that **a caller cannot
/// write such a row**: every writer of `products_scheduled_transition` runs the
/// governance gate first.
///
/// Nothing in either schema enforces that. It is a code invariant, and the day a
/// fourth, ungated writer appears the predicate silently *becomes* the bearer
/// token, with no test failing and no reviewer prompted. So the invariant is
/// counted here, and this test is the prompt.
///
/// # What it does **not** cover
///
/// One table. `products_bulk_batch.approval_ref` has the same shape and the same
/// role, and its writer — `repo::insert_bulk_batch` — is counted by
/// [`every_writer_of_a_bulk_batch_is_counted_for_p_d_127`] since **P-D-127**
/// row 10 extended P-D-105's arm to that table. **Two tables, two guards**,
/// rather than one test counting both: they rest on different decisions and a
/// merged assertion would report one number for two invariants. Do not read a
/// green run of this test alone as *"every pinned approval in the gear is
/// written behind a gate"*. It says three sites, on one table.
///
/// # If this fails
///
/// A call site was added or removed. Do not just move the number. Establish that
/// the new writer runs `GovernanceGate` before it writes — and if it cannot,
/// P-D-105's argument 3 names the fallback: take the `cascade_parent_id` column
/// it rejected, which makes membership structural instead of invariant-based.
#[test]
fn every_writer_of_a_scheduled_transition_is_counted_for_p_d_105() {
    let mut sites = Vec::new();
    for path in crate_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable crate source");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // The declaration and doc mentions are not call sites.
            if trimmed.starts_with("//") || trimmed.starts_with("pub async fn") {
                continue;
            }
            if line.contains("insert_scheduled_transition(") {
                let name = path
                    .strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or(&path);
                sites.push(format!("{}:{}", name.display(), index + 1));
            }
        }
    }
    sites.retain(|s| !s.contains("_tests.rs"));
    // P-D-114 row 31 rests on the same shape one level up: the cascade's
    // auto-discard arm is not a second ceremony *because* `apply_cascade_plan`
    // runs only inside the gated `run_retire`. Count its callers here too.
    let mut cascade_callers = Vec::new();
    for path in crate_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable crate source");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("async fn apply_cascade_plan") {
                continue;
            }
            if line.contains("apply_cascade_plan(") && !path.to_string_lossy().contains("_tests.rs")
            {
                cascade_callers.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    assert_eq!(
        cascade_callers.len(),
        1,
        "P-D-114 row 31: `apply_cascade_plan` performs the auto-discard under the gate its one \
         caller passed, so it must have exactly one caller and that caller must be gated. Found:\n  {}",
        cascade_callers.join("\n  ")
    );
    assert_eq!(
        sites.len(),
        3,
        "P-D-105 rests on there being exactly three gated writers of \
         `products_scheduled_transition` — two in `run_retire` (Product and SKU) and one in \
         `apply_cascade_plan`, whose only caller is that same gated function. Found:\n  {}\n\
         Read this test's doc before changing the number.",
        sites.join("\n  ")
    );
}

/// **P-D-127 row 10's extension is a count too, and this is it.**
///
/// Row 10 puts every row of a bulk batch under the batch record in
/// `PreAuthorized` mode *"under P-D-105's own predicate — the row's stored
/// `approval_ref` names the consumed record"*, and it grants that extension
/// **"with its own writer-count guard"**. P-D-105 had scoped itself to one
/// table precisely because `products_bulk_batch.approval_ref` *"has different
/// writers"*, so the extension is conditional on that sentence staying false.
///
/// The same argument as the sibling guard above, one table over: the
/// predicate is not a bearer token **because a caller cannot write such a
/// row**. Nothing in either schema enforces that; it is a code invariant, and
/// the day a second, ungated writer of `products_bulk_batch` appears the
/// predicate silently *becomes* the bearer token, with no test failing and no
/// reviewer prompted.
///
/// # If this fails
///
/// A call site was added or removed. Do not just move the number. Establish
/// that the new writer runs `GovernanceGate` before it writes — the import
/// door's does, inside its own transaction — and if it cannot, say so to the
/// decision's owner rather than widening the count.
#[test]
fn every_writer_of_a_bulk_batch_is_counted_for_p_d_127() {
    let mut sites = Vec::new();
    for path in crate_sources() {
        if path.to_string_lossy().contains("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a readable crate source");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // The declaration and doc mentions are not call sites.
            if trimmed.starts_with("//") || trimmed.starts_with("pub async fn") {
                continue;
            }
            if line.contains("insert_bulk_batch(") {
                let name = path
                    .strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or(&path);
                sites.push(format!("{}:{}", name.display(), index + 1));
            }
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "P-D-127 row 10 extends P-D-105 to `products_bulk_batch` on the condition that its \
         writers are counted. There is exactly one — the import door's, inside its own \
         transaction. Found:\n  {}\nRead this test's doc before changing the number.",
        sites.join("\n  ")
    );
}

/// **The system principal is the same value every time** (P-D-113 arm 2).
///
/// The property the v4 lacked, and the whole reason for the change: two
/// runtimes, two hosts, two restarts must write their audit rows under one
/// actor. Asserted by calling the derivation twice rather than against a
/// literal, so a deliberate change of name or namespace is a one-line edit
/// here and not a silent drift.
#[test]
fn the_system_principal_is_stable_across_calls() {
    let first = crate::gear::system_actor_ref();
    let second = crate::gear::system_actor_ref();
    assert_eq!(first, second, "a v5 derivation is a function of its inputs");
    assert!(!first.is_nil());
    assert_eq!(
        first.get_version(),
        Some(uuid::Version::Sha1),
        "v5 - derived, not random; a v4 here would mean the per-boot principal is back"
    );
}

/// **Every writer of `products_catalog_version.retention_released_at` is
/// counted** (**P-D-137**, on P-D-105's pattern).
///
/// # Why an invariant and not a schema rule
///
/// The stamp is what `m20260901_000010`'s `DELETE` arm reads, and the `UPDATE`
/// whitelist admits it moving `NULL` → a value once. So **any** caller who may
/// update that table can make a version deletable — the arm buys a deliberate
/// two-step recorded in the row, not an authorisation. P-D-31 is why it cannot
/// buy more: the session variable that would carry the deleter's identity
/// exists on Postgres and not on `SQLite`, so neither trigger reads one.
///
/// What keeps the stamp to the retention sweep is therefore a **code**
/// invariant, and the day a second writer appears the arm silently becomes a
/// bearer token with no test failing and no reviewer prompted. So the
/// invariant is counted here, and this test is the prompt.
///
/// # What it does not cover
///
/// One column on one table. It says *one call site*, and it says nothing
/// about whether that site ran a clock first — `dod-retention-clock`'s own
/// probes are what assert the candidate had passed its window and the freeze
/// gate.
///
/// # If this fails
///
/// A call site was added or removed. Do not just move the number. Establish
/// that the new writer is the GC's release path — and if a second legitimate
/// writer exists, the alternative P-D-137 did not take is a column the
/// application cannot write at all, with the release expressed as its own
/// governed act.
#[test]
fn every_writer_of_a_release_stamp_is_counted() {
    let mut sites = Vec::new();
    for path in crate_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable crate source");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // The declaration and doc mentions are not call sites.
            if trimmed.starts_with("//") || trimmed.starts_with("pub async fn") {
                continue;
            }
            if line.contains("stamp_retention_release(") {
                let name = path
                    .strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
                    .unwrap_or(&path);
                sites.push(format!("{}:{}", name.display(), index + 1));
            }
        }
    }
    sites.retain(|s| !s.contains("_tests.rs"));
    assert_eq!(
        sites.len(),
        1,
        "exactly one production writer of the release stamp — the retention sweep's own \
         collect_catalog_version, inside the transaction that then deletes the chain. \
         Found: {sites:?}"
    );
    assert!(
        sites[0].contains("infra/retention.rs"),
        "and it is the sweep's, not a door's: {sites:?}"
    );
}
