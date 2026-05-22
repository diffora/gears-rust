use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LINT_NAME: &str = "de1201_docs_rs_all_features";

#[test]
fn cargo_lint_fixtures_cover_manifest_cases() {
    let missing_docs_rs = run_fixture("missing_docs_rs");
    assert_success("missing_docs_rs", &missing_docs_rs);
    assert_contains(
        &missing_docs_rs,
        "publishable crate `de1201_missing_docs_rs` must set `package.metadata.docs.rs.all-features = true` (DE1201)",
    );
    assert_contains(&missing_docs_rs, "`package.metadata.docs.rs` is missing");

    for fixture in ["all_features_true", "publish_false", "excluded_crate"] {
        let output = run_fixture(fixture);
        assert_success(fixture, &output);
        assert_not_contains(&output, "DE1201");
    }
}

fn run_fixture(name: &str) -> Output {
    let fixture = fixtures_dir().join(name);
    let manifest_path = fixture.join("Cargo.toml");
    let lint_parent_dir = lint_parent_dir();
    let target_dir = target_dir().join("de1201_cargo_fixtures").join(name);

    let output = Command::new("cargo")
        .arg("dylint")
        .arg("--path")
        .arg(&lint_parent_dir)
        .arg("--pattern")
        .arg(LINT_NAME)
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--no-deps")
        .arg("--quiet")
        .env("CARGO_TARGET_DIR", target_dir)
        .env_remove("DYLINT_RUSTFLAGS")
        .env_remove("DYLINT_TOML")
        .env_remove("RUSTFLAGS")
        .current_dir(&fixture)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo dylint for fixture `{name}`: {error}"));

    remove_fixture_lockfile(&fixture);
    output
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn lint_parent_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("lint crate should have a parent directory")
        .to_path_buf()
}

fn target_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("lint crate should be inside the Dylint workspace")
        .join("target")
}

fn remove_fixture_lockfile(fixture: &Path) {
    match fs::remove_file(fixture.join("Cargo.lock")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!(
            "failed to remove fixture Cargo.lock in `{}`: {error}",
            fixture.display()
        ),
    }
}

fn assert_success(name: &str, output: &Output) {
    assert!(
        output.status.success(),
        "fixture `{name}` failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_contains(output: &Output, needle: &str) {
    let combined = combined_output(output);
    assert!(
        combined.contains(needle),
        "expected output to contain `{needle}`\noutput:\n{combined}"
    );
}

fn assert_not_contains(output: &Output, needle: &str) {
    let combined = combined_output(output);
    assert!(
        !combined.contains(needle),
        "expected output not to contain `{needle}`\noutput:\n{combined}"
    );
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
