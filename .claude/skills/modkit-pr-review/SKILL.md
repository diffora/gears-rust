---
name: modkit-pr-review
description: "Review Rust PRs against idiomatic Rust guidelines and ModKit framework rules, post inline comments on GitHub"
user-invocable: true
allowed-tools: Bash, Read, Glob, Grep, Write
---

# Rust PR Review

Review a GitHub pull request for Rust code quality and ModKit framework compliance.
Posts findings as inline review comments directly on the PR.

**Usage**: `/modkit-pr-review <PR_NUMBER>`

---

## Inputs

- `<PR_NUMBER>` — required, the GitHub PR number (e.g. `123`)
- `--repo <owner/repo>` — optional, the GitHub repository (e.g. `cyberfabric/cyberware-rust`)

## Resolving the target repository

Before fetching PR data, determine which repository to use:

1. If `--repo` was provided in the arguments, use it.
2. Otherwise, check if an `upstream` remote exists: `git remote get-url upstream 2>/dev/null`. If it returns a URL, extract `owner/repo` from it.
3. Otherwise, fall back to the current repo via `gh repo view --json nameWithOwner -q .nameWithOwner`.

Store the result as `REPO` and pass `--repo $REPO` to all `gh pr` commands, and use it in API paths as `repos/$REPO/pulls/...`.

## Review guidelines

The review has two tiers:

**Tier 1 — Architecture (PR-level, Step 0)**: Apply the `# ARCHITECTURE REVIEW` section of `docs/pr-review/modkit-rust-review.md` once across the whole PR before reading individual files. This catches structural and design-level problems that are invisible inside a single diff hunk.

**Tier 2 — Code (per-file, Steps 2–4)**: Apply the remaining sections of `docs/pr-review/modkit-rust-review.md` to every `.rs` file in the diff.

Apply **ModKit framework compliance** (`docs/pr-review/modkit-framework-compliance-review.md`) **only** to `.rs` files that belong to ModKit-owned code. A file is ModKit-owned when **any** of these signals is present:

1. **Cargo.toml signals** — the nearest `Cargo.toml` (same crate or workspace member) declares a `modkit` dependency/feature, or the crate name starts with `modkit`.
2. **Path heuristics** — the file lives under a path that matches ModKit module conventions (e.g. `modules/*/src/`, `crates/modkit-*/`, or similar namespace).
3. **Source-level symbols** — the file imports from ModKit crates (`use modkit_*`, `use crate::` inside a modkit crate) or references ModKit-specific types/traits such as `OperationBuilder`, `SecureConn`, `SecureORM`, `ClientHub`, or `ModuleLifecycle`.

Apply **Rust unit test quality review** (`docs/pr-review/modkit-tests-quality-review.md`) to every changed Rust test you can identify in the diff, including:
- `#[test]` functions
- async tests such as `#[tokio::test]`
- test modules such as `#[cfg(test)] mod tests`
- assertions added or modified inside production files or dedicated test files
- integration tests under `tests/`
- test-only helper code when it materially affects test validity

If none of the ModKit signals are detected, skip the framework compliance checklist for that file and apply only the general Rust idioms checklist.

For non-Rust files in the diff (TOML, YAML, migrations, etc.) — apply only general correctness checks, do not force Rust-specific rules.

## Coding guidelines reference

When reviewing, also consult:
- `guidelines/DNA/languages/RUST.md` — project Rust conventions
- `guidelines/SECURITY.md` — security requirements

---

## Steps

### Step 1: Fetch PR metadata and diff

```bash
gh pr view <PR_NUMBER> --repo $REPO --json number,title,body,headRefOid,baseRefName,headRefName
gh pr diff <PR_NUMBER> --repo $REPO
```

Save the diff output for analysis. Extract the HEAD commit SHA — you need it for posting comments.

### Step 2: PR-level architecture analysis

With the PR title, body, and diff in hand, assess the PR as a whole before doing a full per-file review. Apply every item in the `# ARCHITECTURE REVIEW` section of `docs/pr-review/modkit-rust-review.md`. Some items can be answered from the file list and PR description alone; others require reading relevant code sections — do that now rather than deferring to the per-file pass.

Record architecture findings. Post each as a PR-level issue comment (not an inline review comment) using `gh api`:

```bash
gh api repos/$REPO/issues/<PR_NUMBER>/comments \
  --method POST \
  -f body="**HIGH**\n\n<issue description>\n\n<fix>"
```

Format: `**<SEVERITY>**` on the first line, blank line, issue, blank line, fix. The PR-scope note (last bullet in the checklist) goes to the terminal summary only — do not post it as a comment.

### Step 3: Identify Rust files in diff

Parse the diff to find all `.rs` files that were added or modified.
For each file, note the changed line ranges (added lines only — you can only comment on lines present in the diff).

### Step 4: Read review guidelines and classify files

Read `docs/pr-review/modkit-rust-review.md` (always needed).

For each `.rs` file from Step 3, determine whether it is ModKit-owned code:
- Check the nearest `Cargo.toml` for modkit dependencies/features or a `modkit-` crate name.
- Check whether the file path matches ModKit module conventions (`modules/*/src/`, `crates/modkit-*/`).
- Scan the file for ModKit imports (`use modkit_*`) or ModKit types (`OperationBuilder`, `SecureConn`, `SecureORM`, `ClientHub`, `ModuleLifecycle`).

If **any** file is classified as ModKit-owned, also read `docs/pr-review/modkit-framework-compliance-review.md`.

### Step 5: Review each changed file

For each `.rs` file in the diff:

a. Read the full current file from the repo (not just the diff hunk) to understand context.
b. Apply **modkit-rust-review.md** checklist items — idiomatic Rust, error handling, async safety, ownership, testing, etc.
c. Apply **modkit-tests-quality-review.md** checklist items - to every changed Rust test you can identify in the diff
d. **Only if the file was classified as ModKit-owned in Step 3**, also apply **modkit-framework-compliance-review.md** checklist items — SDK pattern, OperationBuilder, SecureConn, module layout, error types, etc.
e. Record each finding with: checklist ID, severity, file path, line number, issue description, fix.

### Step 6: Filter and deduplicate

- Drop findings that are not evidenced in the diff
- Drop style issues that rustfmt/clippy should catch
- Drop speculative or hypothetical issues
- Merge overlapping findings on the same line
- Keep only findings where you have concrete evidence

### Step 7: Post inline review comments on GitHub

Use `gh api` to create a pull request review with inline comments.

Build the review payload:

IMPORTANT: The `gh api` `-f` array syntax is limited. For multiple comments, build a JSON file and POST it.

The review `body` MUST be empty string — no summary in the review itself. The summary goes to the terminal only (Step 7).

```bash
cat > /tmp/review-payload.json << 'REVIEW_EOF'
{
  "commit_id": "<HEAD_SHA>",
  "event": "COMMENT",
  "body": "",
  "comments": [
    {
      "path": "modules/foo/src/domain/service.rs",
      "line": 42,
      "side": "RIGHT",
      "body": "**HIGH**\n\nError context discarded by `map_err(|_| ...)`.\n\nPreserve the source error — wrap with `.context()` or map to a domain error that keeps the cause."
    }
  ]
}
REVIEW_EOF

gh api repos/$REPO/pulls/<PR_NUMBER>/reviews \
  --method POST \
  --input /tmp/review-payload.json
```

### Step 8: Print summary

After posting, print a compact summary table to the terminal. Architecture findings appear first, followed by code-level findings.

```
## Rust PR Review: #<PR_NUMBER>

### Architecture
| # | Sev | Issue | Fix |
|---|-----|-------|-----|
| 1 | HIGH | Long-running saga in startup blocks shutdown | Move to background task with cancellation |
| 2 | HIGH | Safety gap documented only in comment | Add startup warning or config guard |

### Code
| # | ID | Sev | Location | Issue | Fix |
|---|----|-----|----------|-------|-----|
| 3 | RUST-ERR-001 | HIGH | service.rs:42 | Error context lost | Preserve source error |
| 4 | MODKIT-SEC-001 | CRIT | handler.rs:18 | Raw DB connection | Use SecureConn |

Posted <N> inline comments and <M> PR-level comments on PR #<PR_NUMBER>.
```

---

## Comment formatting rules

Each inline comment MUST follow this format:

```
**<SEVERITY>**

<One-sentence issue description.>

<One-sentence why it matters.>

<Concrete fix — what to change, not a vague suggestion.>
```

Where `<SEVERITY>` is one of: `CRITICAL`, `HIGH`, `MEDIUM`, `LOW`.

Do NOT include checklist IDs (e.g. RUST-ERR-001, MODKIT-SEC-001) in inline comments. IDs appear only in the terminal summary table (Step 7).

Rules:
- Engineering English. No filler, no praise, no hedging.
- No "consider", "you might want to", "it would be nice if". State what is wrong and what to do.
- One issue per comment. If a line has two problems, post two comments.
- Line number must point to an added/modified line that exists in the diff. Do not comment on unchanged lines.
- If you cannot determine the exact line, do not guess — skip that finding.

---

## What NOT to do

- Do not approve or request changes — use `event: "COMMENT"` only
- Do not post comments on lines outside the diff
- Do not post generic praise or "LGTM" if there are no issues
- Do not invent issues without evidence in the code
- Do not complain about formatting that rustfmt handles
- Do not suggest speculative abstractions or premature generalization
- Do not post more than 30 comments per review (prioritize by severity)
- If there are zero findings, post a single review comment: "No issues found."
