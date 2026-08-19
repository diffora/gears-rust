//! One reading of *"is this token in the crate's **code**, or only in its
//! prose"*, shared by the censuses that ask it.
//!
//! # Why this is a module and not a helper in the file that first needed it
//!
//! It was one: `approval_repo_tests` built it so its roster of
//! `AuditSubjectKind`s could tell a writer from a doc comment quoting one, and
//! its own doc names the property — *"prose that quotes a construction is not a
//! writer"*. The trigger registry's census then needed the same sentence and did
//! not have it, so `Trigger::BundleComposition` was **paid by two line comments
//! in `api::rest::bundles`**: deleting the crate's only construction of it, at
//! `infra::bundle`'s `composition_change_set`, would have left the census green
//! over prose alone. That is the state `bulkGroupMove` was in, which is the
//! defect that census exists to find.
//!
//! A second copy would have been two readings of "code" to disagree with each
//! other, on two censuses that exist to keep an attestation honest. So there is
//! one.
//!
//! # It is test-only, and the filename says so
//!
//! `_tests.rs` because both census walks skip that suffix when they enumerate the
//! crate's sources — the instrument must not be read as one of the sources it
//! measures. The module is named for what it does rather than for the file.

/// One file's text with every comment and every string, char and raw-string
/// literal replaced by spaces — **byte positions and newlines preserved**, so a
/// line number taken from the result is a line number in the input.
///
/// This is what makes a census syntactic rather than textual. Two concrete
/// properties depend on it: prose that quotes a construction is not a writer, and
/// a brace inside a literal cannot move an initializer's boundary.
///
/// A lifetime (`&'a str`) is deliberately not read as a char literal: a `'` opens
/// one only when the next character is an escape or is followed by a closing
/// quote, which is what distinguishes `'x'` from `'a` in the one place it matters.
pub fn blank_comments_and_literals(text: &str) -> String {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len());
    let mut state = Lexed::Code;
    let mut index = 0;
    while index < chars.len() {
        let step = match state {
            Lexed::Code => step_code(&chars, index),
            Lexed::LineComment => step_line_comment(&chars, index),
            Lexed::BlockComment(depth) => step_block_comment(&chars, index, depth),
            Lexed::Str => step_quoted(&chars, index, '"'),
            Lexed::Char => step_quoted(&chars, index, '\''),
            Lexed::RawStr(hashes) => step_raw_str(&chars, index, hashes),
        };
        push_span(&mut out, &chars[index..index + step.consumed], step.keep);
        state = step.next;
        index += step.consumed;
    }
    out
}

/// The index of the `}` closing the `{` at `open`, counting depth.
///
/// Exact over [`blank_comments_and_literals`]'s output and **only** there: with
/// every comment and every literal already spaces, a brace is a brace. Over raw
/// text it is not a scanner at all — `"{"` in a string would open a block that
/// never closes.
///
/// It lives beside the blanking rather than in either caller for the reason the
/// module doc gives about the blanking itself: `approval_repo_tests` bounds a
/// `NewApproval` initializer with it so a `..template()` site cannot borrow the
/// next site's literal, and `triggers_tests` bounds a **function body** with it so
/// a producing site can be attributed to the function that contains it. Two copies
/// of a depth count would be two answers to *"where does this item end"* on two
/// censuses that exist to keep one attestation honest.
pub fn matching_brace(code: &str, open: usize) -> Option<usize> {
    matching_delim(code, open, b'{', b'}')
}

/// [`matching_brace`] over any delimiter pair — the same depth count, not a
/// second one.
///
/// The brace reading is the common case and keeps its own name; `authz_tests`
/// bounds an **argument list** with this one, to read the `owner_tenant_id` a
/// gate passes. Generalizing rather than copying is the module doc's own
/// argument: two depth counts would be two answers to *"where does this item
/// end"*.
pub fn matching_delim(code: &str, open: usize, open_ch: u8, close_ch: u8) -> Option<usize> {
    let mut depth = 0_usize;
    for (offset, byte) in code.as_bytes()[open..].iter().enumerate() {
        if *byte == open_ch {
            depth += 1;
        } else if *byte == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some(open + offset);
            }
        }
    }
    None
}

/// What the scanner is inside of.
#[derive(Clone, Copy)]
enum Lexed {
    Code,
    LineComment,
    /// Rust block comments nest, so the depth is carried.
    BlockComment(usize),
    Str,
    Char,
    /// `r#"…"#`, carrying how many `#` close it.
    RawStr(usize),
}

/// One move: what the scanner is in next, how many chars it consumed, and whether
/// they are code (kept) or not (blanked).
struct Step {
    next: Lexed,
    consumed: usize,
    keep: bool,
}

/// Write a span, either as itself or as spaces — `\n` always as itself, which is
/// what preserves line numbers through a multi-line comment or string.
fn push_span(out: &mut String, span: &[(usize, char)], keep: bool) {
    for (_, c) in span {
        if keep {
            out.push(*c);
        } else if *c == '\n' {
            out.push('\n');
        } else {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        }
    }
}

fn char_at(chars: &[(usize, char)], index: usize) -> Option<char> {
    chars.get(index).map(|(_, c)| *c)
}

/// The only arm that keeps anything, and the only one that can open a literal.
fn step_code(chars: &[(usize, char)], index: usize) -> Step {
    let c = chars[index].1;
    let next = char_at(chars, index + 1);
    let blank = |state: Lexed, consumed: usize| Step {
        next: state,
        consumed,
        keep: false,
    };
    if c == '/' && next == Some('/') {
        return blank(Lexed::LineComment, 2);
    }
    if c == '/' && next == Some('*') {
        return blank(Lexed::BlockComment(1), 2);
    }
    if c == '"' {
        return blank(Lexed::Str, 1);
    }
    if let Some(step) = step_raw_str_prefix(chars, index) {
        return step;
    }
    // A `'` opens a char literal only when what follows is an escape or a single
    // character before the closing quote; otherwise it is a lifetime (`&'a str`),
    // and reading that as a literal would blank real code.
    if c == '\'' && (next == Some('\\') || char_at(chars, index + 2) == Some('\'')) {
        return blank(Lexed::Char, 1);
    }
    Step {
        next: Lexed::Code,
        consumed: 1,
        keep: true,
    }
}

/// `r"…"`, `r#"…"#`, `br"…"` and `br#"…"#`, if that is what starts here.
fn step_raw_str_prefix(chars: &[(usize, char)], index: usize) -> Option<Step> {
    let c = chars[index].1;
    if c != 'r' && c != 'b' {
        return None;
    }
    // Not a raw string if the `r` is the tail of an identifier (`for`, `iter`).
    if index > 0 && {
        let previous = chars[index - 1].1;
        previous.is_alphanumeric() || previous == '_'
    } {
        return None;
    }
    let mut look = index + 1;
    if c == 'b' && char_at(chars, look) == Some('r') {
        look += 1;
    } else if c == 'b' {
        // `b"…"` is an ordinary quote and `Lexed::Str` handles it from the `"`.
        return None;
    }
    let mut hashes = 0;
    while char_at(chars, look) == Some('#') {
        hashes += 1;
        look += 1;
    }
    if char_at(chars, look) != Some('"') {
        return None;
    }
    Some(Step {
        next: Lexed::RawStr(hashes),
        consumed: look - index + 1,
        keep: false,
    })
}

fn step_line_comment(chars: &[(usize, char)], index: usize) -> Step {
    Step {
        next: if chars[index].1 == '\n' {
            Lexed::Code
        } else {
            Lexed::LineComment
        },
        consumed: 1,
        keep: false,
    }
}

fn step_block_comment(chars: &[(usize, char)], index: usize, depth: usize) -> Step {
    let c = chars[index].1;
    let next = char_at(chars, index + 1);
    if c == '/' && next == Some('*') {
        return Step {
            next: Lexed::BlockComment(depth + 1),
            consumed: 2,
            keep: false,
        };
    }
    if c == '*' && next == Some('/') {
        return Step {
            next: if depth == 1 {
                Lexed::Code
            } else {
                Lexed::BlockComment(depth - 1)
            },
            consumed: 2,
            keep: false,
        };
    }
    Step {
        next: Lexed::BlockComment(depth),
        consumed: 1,
        keep: false,
    }
}

/// A `"…"` or `'…'` literal, whose escapes consume the escaped character too — so
/// `"\""` does not end at its middle quote.
fn step_quoted(chars: &[(usize, char)], index: usize, terminator: char) -> Step {
    let c = chars[index].1;
    let inside = if terminator == '"' {
        Lexed::Str
    } else {
        Lexed::Char
    };
    if c == '\\' && index + 1 < chars.len() {
        return Step {
            next: inside,
            consumed: 2,
            keep: false,
        };
    }
    Step {
        next: if c == terminator { Lexed::Code } else { inside },
        consumed: 1,
        keep: false,
    }
}

/// A raw string has no escapes; only `"` followed by its own `#` count ends it.
fn step_raw_str(chars: &[(usize, char)], index: usize, hashes: usize) -> Step {
    if chars[index].1 == '"' && (1..=hashes).all(|i| char_at(chars, index + i) == Some('#')) {
        return Step {
            next: Lexed::Code,
            consumed: hashes + 1,
            keep: false,
        };
    }
    Step {
        next: Lexed::RawStr(hashes),
        consumed: 1,
        keep: false,
    }
}
