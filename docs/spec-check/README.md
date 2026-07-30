# spec-check reports

Output of the `spec-check` skill's semantic layer (`.claude/skills/spec-check`).
**Advisory. Nothing gates on anything here** — no CI job reads these files, and no
exit code depends on them. They are a record of what was measured, on which corpus,
and when someone last looked.

Two kinds of file live here, and the distinction matters:

- **`N1-<gear>[-<run>].md` — generated.** Written by `scripts/judge_report.py` from a
  `neighbourhoods.json` / `verdicts.json` pair. Do not hand-edit: regenerating with
  unchanged inputs must diff clean, so an edit is silently discarded on the next run.
- **`*-findings.md` — hand-written.** The reading of a generated report: which
  hypothesis it answers, which findings survived being checked by eye against the
  real documents, and which turned out to be false positives. Machine output cannot
  say any of that about itself.

Regenerate a report with the four-step runbook in the skill's `SKILL.md` (§ N1):
`neighbourhoods.py` → `judge_batches.py` → one `spec-check-n1-judge` dispatch per
batch → `judge_report.py`. Run artifacts (`neighbourhoods.json`, `verdicts.json`,
batch prompts) stay in the git-ignored `.spec-check/` at the repository root; they
are regenerable and they quote design prose.

## Why not inside `gears/<gear>/docs/`

A spec-check corpus loads **every** `*.md` under the gear's docs root. A report
written there becomes a document the next run parses: its quoted fragments start
matching requirement vocabulary, so the tool begins measuring its own previous
output. `judge_report.py` refuses an `--out` path inside a gear's docs tree for
exactly this reason — the refusal is a guard, not a preference.

## Why references are plain text, not links

Every `path:line` reference here is plain text on purpose. `make lychee` walks this
tree (see `Makefile`), and a link carrying a line anchor has nothing to resolve
against, so link-checking would fail on every reference the reports exist to make.
