//! The length caps of D-457 on the request bodies, judged by each door right after it parses its
//! body, before it reads or writes anything. A field the body does not carry is not judged, and
//! only new text is: a text that must name a stored row (a dimension key or value an entry, a
//! price or a PATCH of the registry names, a value it removes) is never capped, so a row stored
//! before the caps never locks its registry (the second review of W1a, L1). The PUT of the
//! registry caps only the keys and values the stored registry does not hold, against the stored
//! rows ([`new_dimension_text`]).
use super::dto::{
    PriceBookCreate, PriceBookPatch, PricingDimensionKeyPatch, PricingPlanClone, PricingPlanCreate,
    PricingPlanPatch, PricingPriceBookEntryCreate, PricingPriceBookEntryPatch, PricingPriceCreate,
    PricingPricePatch, PricingSettingsPut,
};
use super::support::invalid_because;
use crate::domain::caps::{
    CODE_MAX_CHARS, LABEL_MAX_CHARS, NAME_MAX_CHARS, NOTE_MAX_CHARS, TEMPLATE_MAX_CHARS, over,
};
use toolkit_canonical_errors::CanonicalError;

/// 400 `FIELD_TOO_LONG` on `field` for a text longer than `max` characters.
pub(super) fn field(name: &str, text: &str, max: usize) -> Result<(), CanonicalError> {
    if over(text, max) {
        Err(invalid_because(
            name,
            "FIELD_TOO_LONG",
            &format!("{name} is at most {max} characters"),
        ))
    } else {
        Ok(())
    }
}
/// [`field`] for each text of a list.
fn each(name: &str, texts: &[String], max: usize) -> Result<(), CanonicalError> {
    texts.iter().try_for_each(|text| field(name, text, max))
}
/// A note keeps its own code: 400 `NOTE_TOO_LONG` on `note`, as a vote's note (the approval
/// engine) and products' submit notes answer it.
fn note(text: Option<&str>) -> Result<(), CanonicalError> {
    if text.is_some_and(|t| over(t, NOTE_MAX_CHARS)) {
        Err(invalid_because(
            "note",
            "NOTE_TOO_LONG",
            &format!("a note is at most {NOTE_MAX_CHARS} characters"),
        ))
    } else {
        Ok(())
    }
}
/// A request body whose text fields have their caps.
pub(super) trait Capped {
    /// # Errors
    /// 400 `FIELD_TOO_LONG` (or `NOTE_TOO_LONG`) on the first field over its cap.
    fn caps(&self) -> Result<(), CanonicalError>;
}
impl Capped for PriceBookCreate {
    // The description keeps its own code, `BOOK_DESCRIPTION_TOO_LONG`, judged with the book.
    fn caps(&self) -> Result<(), CanonicalError> {
        field("code", &self.code, CODE_MAX_CHARS)?;
        field("name", &self.name, NAME_MAX_CHARS)
    }
}
impl Capped for PriceBookPatch {
    fn caps(&self) -> Result<(), CanonicalError> {
        self.name
            .as_deref()
            .map_or(Ok(()), |name| field("name", name, NAME_MAX_CHARS))
    }
}
impl Capped for PricingPlanCreate {
    fn caps(&self) -> Result<(), CanonicalError> {
        field("code", &self.code, CODE_MAX_CHARS)?;
        field("name", &self.name, NAME_MAX_CHARS)
    }
}
impl Capped for PricingPlanClone {
    fn caps(&self) -> Result<(), CanonicalError> {
        field("code", &self.code, CODE_MAX_CHARS)?;
        field("name", &self.name, NAME_MAX_CHARS)
    }
}
impl Capped for PricingPlanPatch {
    fn caps(&self) -> Result<(), CanonicalError> {
        field("name", &self.name, NAME_MAX_CHARS)
    }
}
// A price's `dim_value` names a value of its entry's key (400 `DIM_VALUE_UNKNOWN` otherwise), so
// it is not new text and has no cap of its own.
impl Capped for PricingPriceCreate {
    fn caps(&self) -> Result<(), CanonicalError> {
        note(self.note.as_deref())
    }
}
impl Capped for PricingPricePatch {
    fn caps(&self) -> Result<(), CanonicalError> {
        note(self.note.as_ref().and_then(Option::as_deref))
    }
}
// An entry's `dimension_key` names a declared key (400 `DIM_NOT_DECLARED` otherwise), so it is not
// new text and has no cap of its own.
impl Capped for PricingPriceBookEntryCreate {
    fn caps(&self) -> Result<(), CanonicalError> {
        self.invoice_line_override
            .as_deref()
            .map_or(Ok(()), |line| {
                field("invoice_line_override", line, TEMPLATE_MAX_CHARS)
            })
    }
}
impl Capped for PricingPriceBookEntryPatch {
    fn caps(&self) -> Result<(), CanonicalError> {
        match &self.invoice_line_override {
            Some(Some(line)) => field("invoice_line_override", line, TEMPLATE_MAX_CHARS),
            _ => Ok(()),
        }
    }
}
impl Capped for PricingSettingsPut {
    fn caps(&self) -> Result<(), CanonicalError> {
        if let Some(gl) = &self.default_gl {
            field("default_gl", gl, LABEL_MAX_CHARS)?;
        }
        if let Some(tax) = &self.default_tax_category {
            field("default_tax_category", tax, LABEL_MAX_CHARS)?;
        }
        self.invoice_line_templates
            .values()
            .try_for_each(|line| field("invoice_line_templates", line, TEMPLATE_MAX_CHARS))
    }
}
/// The PUT of the registry (`PricingDimensions`, judged in `configuration::put_dimensions` against
/// the stored registry): 400 `FIELD_TOO_LONG` on `key` for a key the registry does not hold, and on
/// `values` for a value its key does not hold, longer than a code's cap. A key or value already
/// stored passes whatever its length.
pub(super) fn new_dimension_text(
    key: &str,
    values: &[String],
    stored: Option<&[String]>,
) -> Result<(), CanonicalError> {
    let Some(held) = stored else {
        field("key", key, CODE_MAX_CHARS)?;
        return each("values", values, CODE_MAX_CHARS);
    };
    values
        .iter()
        .filter(|value| !held.contains(value))
        .try_for_each(|value| field("values", value, CODE_MAX_CHARS))
}
// The PATCH names a stored key (400 `DIM_NOT_DECLARED` otherwise) and removes stored values (400
// `DIM_VALUE_UNKNOWN` otherwise): only the values it adds are new text.
impl Capped for PricingDimensionKeyPatch {
    fn caps(&self) -> Result<(), CanonicalError> {
        each("add", &self.add, CODE_MAX_CHARS)
    }
}
