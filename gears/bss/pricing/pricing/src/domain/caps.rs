//! The explicit length caps of the text a request writes (D-457), counted in characters (Unicode
//! scalar values), as a book's description always was (D-444). Only a write is judged, and only
//! on the fields it carries: a stored row over a cap stays readable.

/// A code: a book's and a plan's, a dimension key and a dimension value.
pub const CODE_MAX_CHARS: usize = 64;
/// A book's and a plan's name.
pub const NAME_MAX_CHARS: usize = 200;
/// A note or a description: a price's note, a vote's (the approval engine's cap) and a book's
/// description.
pub const NOTE_MAX_CHARS: usize = 2000;
/// A GL code and a tax category, the settings' defaults.
pub const LABEL_MAX_CHARS: usize = 64;
/// An invoice line template: an entry's override and the settings' template of a SKU type.
pub const TEMPLATE_MAX_CHARS: usize = 2000;

// The gear's note cap is the one the approval engine judges a vote's note by.
const _: () = assert!(NOTE_MAX_CHARS == bss_approval::NOTE_MAX_CHARS);

/// Whether `text` is longer than `max` characters.
#[must_use]
pub fn over(text: &str, max: usize) -> bool {
    text.chars().count() > max
}
