//! Reading the error mapping's log lines out of a test's captured log.

/// The log target of the `DomainError` to `CanonicalError` mapping, which every line it
/// writes carries.
const TARGET: &str = "event_broker::api::rest::error: ";

/// The mapping's lines among one test's captured log lines, each as
/// `<LEVEL> <message and fields>`; the handler tests read a 503's cause with
/// it too. It never panics: `logs_assert` runs it under the capture's global
/// lock, which a panic would poison for every other test of the binary.
pub fn mapping_lines(lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| {
            let (head, rest) = line.split_once(TARGET)?;
            let level = head.split_whitespace().nth(1)?;
            Some(format!("{level} {rest}"))
        })
        .collect()
}
