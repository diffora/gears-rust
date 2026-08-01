use super::*;
use std::fs;

/// Builds a throwaway corpus tree so the loader is tested independently of the
/// real corpus content, which later tasks add.
fn write_tree(root: &std::path::Path) {
    let fam = root.join("tier-boundary");
    fs::create_dir_all(&fam).unwrap();

    fs::write(
        fam.join("_family.toml"),
        r#"
family     = "tier-boundary"
role       = "publish"
gates      = ["graduated", "volume"]
provenance = ["AC#60"]
"#,
    )
    .unwrap();

    fs::write(
        fam.join("a-case.toml"),
        r#"
family     = "tier-boundary"
id         = "a-case"
kind       = "evaluation"
provenance = ["AC#60"]

[snapshot]
model_kind = "graduated"
currency   = "USD"

  [[snapshot.bands]]
  from_qty = 0
  to_qty   = "open"
  unit_amount_minor = 5

[[assert]]
given  = { q = 3 }
expect = { charge_minor = 15 }
"#,
    )
    .unwrap();
}

#[test]
fn loads_families_and_cases() {
    let tmp = std::env::temp_dir().join("bss-fixtures-loader-ok");
    if tmp.exists() {
        fs::remove_dir_all(&tmp).expect("clean the temp tree");
    }
    write_tree(&tmp);

    let corpus = Corpus::load(&tmp).expect("corpus must load");

    assert_eq!(corpus.families.len(), 1);
    assert_eq!(
        corpus.families[0].gates,
        vec![ModelKind::Graduated, ModelKind::Volume]
    );
    assert_eq!(corpus.cases.len(), 1);
    assert_eq!(corpus.cases_for(Family::TierBoundary).count(), 1);
}

#[test]
fn a_parse_error_names_the_file() {
    let tmp = std::env::temp_dir().join("bss-fixtures-loader-bad");
    if tmp.exists() {
        fs::remove_dir_all(&tmp).expect("clean the temp tree");
    }
    write_tree(&tmp);
    fs::write(
        tmp.join("tier-boundary").join("broken.toml"),
        "family = \"nope\"",
    )
    .unwrap();

    let err = Corpus::load(&tmp).expect_err("a broken case must fail the load");

    assert!(
        err.to_string().contains("broken.toml"),
        "error must name the file, got: {err}"
    );
}

#[test]
fn a_case_filed_under_the_wrong_family_directory_is_rejected() {
    let tmp = std::env::temp_dir().join("bss-fixtures-loader-misfiled");
    if tmp.exists() {
        fs::remove_dir_all(&tmp).expect("clean the temp tree");
    }
    write_tree(&tmp);
    let misfiled = fs::read_to_string(tmp.join("tier-boundary").join("a-case.toml"))
        .unwrap()
        .replace(
            r#"family     = "tier-boundary""#,
            r#"family     = "package""#,
        );
    fs::write(tmp.join("tier-boundary").join("misfiled.toml"), misfiled).unwrap();

    let err = Corpus::load(&tmp).expect_err("a misfiled case must fail the load");

    assert!(
        err.to_string().contains("misfiled.toml"),
        "error must name the file, got: {err}"
    );
}

#[test]
fn the_real_corpus_loads() {
    Corpus::load(&Corpus::corpus_root()).expect("the committed corpus must load");
}
