## Releasing (automated)

This repository uses **release-plz** to automate:
- version bumps
- changelog updates
- crates.io publishing
- GitHub releases

### How the flow works

**Every push to `main`** runs both release-plz commands, in this order:

1. `release-plz release` publishes crates to crates.io and creates GitHub Releases. It
   only publishes versions that are in the manifests but **not yet on crates.io**, so on
   an ordinary push it finds nothing to do and exits. This is also what makes the pipeline
   self-healing: if a publish is interrupted, the next push to `main` finishes it — no
   manual intervention.
2. `release-plz release-pr` opens or updates a **Release PR** with:
   - crate versions (per-crate, based on each crate's `Cargo.toml`)
   - the root [`CHANGELOG.md`](../CHANGELOG.md)

   It is ordered after `release` so it never derives the next versions from crates.io
   while a publish is mid-flight.

The Release PR is labelled **`release-plz`** automatically. Merging it is what puts the new
versions on `main`; the workflow then attempts to publish on that merge's push, like any
other. If that attempt fails, the versions stay on `main` unpublished until a later push
retries them — see the self-healing note above.

Nothing here keys off the label or off the Release PR being merged — `release` decides for
itself by comparing manifests against crates.io. That is the shape release-plz documents,
and it is why a release cannot be lost by a workflow being skipped or cancelled.

Workflows:
- Root workspace: [`.github/workflows/release-plz.yml`](../.github/workflows/release-plz.yml)

### What gets published

Publishing is controlled by Cargo manifests:
- crates with `publish = false` are **never published**
- crates without `publish = false` are **publishable** (subject to crates.io rules)

This repo is configured so that:
- `apps/**` and `examples/**` are **not** publishable (we set `publish = false`)
- `libs/**` and `gears/**` are publishable as intended

### Versioning policy (as implemented)

- **Framework (`libs/toolkit-*`)**: share a single version via `version.workspace = true` and the root workspace version (`Cargo.toml` → `[workspace.package] version`).
- **System SDKs (`libs/system-sdks/**`)**: each crate has its own explicit version.
- **Gears (`gears/**`)**: each gear and each `*-sdk` has its own explicit version.

### Dependency ordering

release-plz publishes crates in the correct order for intra-workspace dependencies.

### Safety checks

The release workflow does **not** run tests and does **not** block on CI. Merging the
Release PR is the maintainer's release decision; if you merge it, the workflow attempts
to publish the crates.

What does verify a release:

- [`ci.yml`](../.github/workflows/ci.yml) runs on every push to `main`, so the tip of
  `main` gets tested — including the integrated result of merging, which PR CI never
  sees. Pushes touching only markdown or `docs/**` are filtered out by the workflow's
  `paths-ignore`, so no run is created for them: such a commit has no CI result of its
  own and inherits none. That is acceptable only because its code is identical to the
  previous commit, which does have one. If `CI` is ever made a required status check,
  this case needs an always-succeeding placeholder job — a requirement whose workflow
  never runs is never satisfied.
- The publish job runs on the pushed commit itself, so each crate is built from the source
  that landed on `main`. Not a byte-for-byte copy of it: cargo rewrites every manifest on
  the way out, resolving workspace inheritance such as `version.workspace = true` into
  concrete values.
- `cargo publish` compiles every crate, so a crate that does not build cannot be
  published. It builds lib and bin targets only — it runs no tests.

What this does **not** give you: a green light at publish time. A CI run takes 34-57
minutes while the publish starts within a few minutes of the push, so CI for the published
commit is still running while the crates are going out. The CI result lands on the same
commit afterwards and is visible on it in GitHub.

If you want a blocking gate, that belongs in branch protection on `main` (required
status checks) or a merge queue, not in the release workflow.

### Emergency / manual release

If you need a hotfix / manual release, prefer triggering the GitHub Actions workflow instead of publishing locally:

1. Ensure versions are bumped (edit the relevant `Cargo.toml` version fields) and the change is on the target branch.
2. Go to GitHub → **Actions** → **Release (release-plz)** → **Run workflow**.
3. Select `mode = release` (publishes crates + creates GitHub Releases).

Note: `mode = release` publishes whatever is on the target branch and does not block on
CI, so confirm the branch is green first. Running the workspace tests locally gives
faster feedback than waiting for CI:

```bash
cargo test --workspace --no-fail-fast --exclude cf-gears-toolkit-macros-tests --exclude cf-gears-toolkit-db-macros
```

Fallback if CI is unavailable: publish locally from a clean checkout (you must have `CARGO_REGISTRY_TOKEN` set):

```bash
export CARGO_REGISTRY_TOKEN=***   # your crates.io token
cargo publish -p <crate_name>
```

### Notes for the very first publish (bootstrap)

- **crates.io rate limiting (HTTP 429)** can happen when publishing many crates for the first time.
  If the publish job fails with 429, just re-run the same workflow after the timestamp shown in the error.
  The process is idempotent: already-published crates will be skipped on retry.

