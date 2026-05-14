# gts-analyze

Analyze GTS (Global Type System) usage in a single Cyber Ware module. Reports types defined here, instances declared here, and every GTS identifier reference across `.rs` / `.md` / `.json` / `.toml` / `.yaml` files, broken down by location (`sdk` / `main` / `plugin` / `doc`).

## Usage

```sh
cargo run --release -p gts-analyze -- modules/system/resource-group
cargo run --release -p gts-analyze -- modules/mini-chat --skip-docs --verbose
cargo run --release -p gts-analyze -- modules/system/account-management --format json --out report.json
```

### Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--format md\|json` | `md` | Output format. Markdown is human-readable; JSON is machine-readable with the same data. |
| `--out FILE` | stdout | Write report to file. |
| `--include-tests` | off | Include Rust test files. By default test code is skipped via three layered filters: directory (`tests/`, `benches/`), filename (`*_test.rs`, `*_tests.rs`, `test.rs`, `tests.rs`), and syn attribute (`#[test]`, `#[bench]`, `*::test`, positive `#[cfg(test)]`). |
| `--skip-docs` | off | Exclude every `*.md` file and every file under any `docs/` directory. JSON schemas in `docs/schemas/` are dropped too — type definitions sourced from there will not appear in the report. Use for a code-only audit. |
| `-v`, `--verbose` | off | Expand each location's reference list to per-line `file:line — context`. Default collapses to one row per file with a hit count, producing a ~3–4× smaller report. |

## What the report contains

1. **Summary table** — file counts per extension, plus totals for types / instances / references.
2. **GTS Types Defined Here** — every type the module defines, with how its schema is declared:
   - Rust `#[gts_type_schema]` attribute on a struct,
   - JSON Schema file (`*.schema.json` with a `$id`),
   - or `struct_to_gts_schema!` macro invocation.
   For each, the report shows where the type is referenced, broken down by `sdk` / `main` / `plugin` / `doc`.
3. **GTS Instances Defined Here** — declarations via `gts_instance!` / `gts_instance_raw!`, with the inferred base type and reference breakdown.
4. **Other GTS IDs Referenced** — IDs that appear in code or docs but are not defined in this module. A summary table plus a collapsible per-ID file/location breakdown.

## Implementation notes

- `.rs` files are parsed with `syn` — accurate macro / attribute / token-tree extraction including raw strings and nested macros. Inline `#[cfg(test)] mod tests { … }` blocks are correctly elided when `--include-tests` is not set.
- `.md` files are scanned with a bare-identifier regex; everything else uses a quoted-string regex.
- `.json` files are parsed; documents with a `$id` of `gts://gts.<…>` or `gts.<…>` register as type definitions, and any string-literal GTS IDs inside count as references.
- All location buckets are derived from the path: any component named `docs` → `doc`, any component `plugins` or `*-plugin` → `plugin`, any component `*-sdk` → `sdk`, otherwise → `main`.

## Invocation via Claude

A thin slash command is shipped in `.claude/skills/gts-analyze/SKILL.md` — typing `/gts-analyze <module-path>` from Claude invokes this binary. The skill is just a wrapper; the source of truth is here.
