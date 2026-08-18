from spec_check.finding import Finding, Severity


def test_render_includes_the_line_number_when_there_is_one():
    # Rust: `format!("[{:?}] {} — {} ({})", ...)` — the severity is the *Debug*
    # form (`Low`, capitalised), and the separator is an em-dash, not a hyphen.
    f = Finding("P1/propagation-missing", Severity.MEDIUM, "DECISIONS.md", 485, "D-49: something")
    assert f.render() == "[Medium] DECISIONS.md:485 — D-49: something (P1/propagation-missing)"


def test_render_omits_the_colon_when_there_is_no_line():
    f = Finding("P3/code-unreferenced", Severity.LOW, "design/01-foundation.md", None, "msg")
    assert f.render() == "[Low] design/01-foundation.md — msg (P3/code-unreferenced)"


def test_json_uses_the_lowercase_severity_and_serde_field_order():
    # serde's `rename_all = "lowercase"` on the enum, and the struct's own field
    # order — which `json.dumps` preserves from the dict, and which the frozen
    # JSON oracle depends on.
    f = Finding("P1/propagation-missing", Severity.HIGH, "DECISIONS.md", 1, "msg")
    assert list(f.to_json().items()) == [
        ("invariant", "P1/propagation-missing"),
        ("severity", "high"),
        ("file", "DECISIONS.md"),
        ("line", 1),
        ("message", "msg"),
    ]


def test_a_missing_line_serialises_as_null_not_as_an_absent_key():
    assert Finding("i", Severity.LOW, "f", None, "m").to_json()["line"] is None


def test_rank_orders_low_below_medium_below_high():
    # `main::rank` maps Severity onto clap's `Gate`, whose derived `Ord` follows
    # declaration order Low < Medium < High. The `--max-severity` gate is a `>=`
    # against it.
    assert Severity.rank(Severity.LOW) < Severity.rank(Severity.MEDIUM)
    assert Severity.rank(Severity.MEDIUM) < Severity.rank(Severity.HIGH)
