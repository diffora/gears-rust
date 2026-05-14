---
name: gts-analyze
description: Analyze how the GTS (Global Type System) is used in a Cyber Ware module — defined types, declared instances, and references across sdk / main / plugin / docs. Thin wrapper around the `gts-analyze` CLI under tools/.
user-invocable: true
allowed-tools: Bash
---

# gts-analyze

Thin wrapper around the workspace binary at `tools/gts-analyze/`. Source code, full docs, and human-readable usage live there: `tools/gts-analyze/README.md`.

## How to invoke

1. Parse the user's invocation for a module path (absolute or repo-relative). If absent, ask once.
2. Run from the workspace root:
   ```bash
   cargo run --release -p gts-analyze -- <MODULE_PATH> [flags]
   ```
   First invocation in a fresh checkout compiles the crate (~30s with `syn`); subsequent runs are instant.
3. Show the markdown report verbatim — don't summarise away the per-type detail.

## Flags

- `--format json` — machine-readable JSON instead of markdown.
- `--out FILE` — write report to file instead of stdout.
- `--include-tests` — include Rust test files (default skips integration tests, benches, `*_test(s).rs`, and items behind `#[test]` / `#[cfg(test)]`).
- `--skip-docs` — drop every `*.md` and any file under `docs/`. Note: type definitions sourced from `docs/schemas/*.schema.json` disappear too — use only for code-only audits.
- `-v` / `--verbose` — expand reference lists to per-line `file:line — context`. Default is compact (file + count).

## Report sections

1. **Summary** — file counts and totals.
2. **GTS Types Defined Here** — per defined type: schema source (`#[gts_type_schema]` / `*.schema.json` / `struct_to_gts_schema!`), location, struct name when applicable, and usage breakdown across `sdk` / `main` / `plugin` / `doc`.
3. **GTS Instances Defined Here** — per declared instance: macro form, inferred typed struct, base type, location, and usage breakdown.
4. **Other GTS IDs Referenced** — table of IDs not defined here, plus a collapsible per-ID file breakdown.

## Tips

- A GTS ID ending with `~` is a **Type**, otherwise an **Instance** (gts-spec §8.1).
- For doc-heavy modules with hundreds of `.md` references, suggest `--skip-docs` if the user only cares about code usage.
- `cargo run` builds on first invocation; if the user is running this often, `cargo build --release -p gts-analyze` once up front avoids latency.
