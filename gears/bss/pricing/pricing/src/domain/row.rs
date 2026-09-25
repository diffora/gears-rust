//! Row validation, per-value chains, temporary pairs and dated metering continuity.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-dimension-fallback:p1
//! @cpt-dod:cpt-cf-bss-pricing-dod-temporary-value-fallback:p1
use super::{
    RuleError,
    money::{self, PriceData},
    price::{ChargeKind, Model, model_allowed},
};
use rust_decimal::Decimal;
use time::Date;
use uuid::Uuid;

string_enum!(Eligibility {All=>"all", New=>"new"});
string_enum!(RowState {Draft=>"draft", Pending=>"pending", Approved=>"approved", Rejected=>"rejected"});

#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: Uuid,
    pub price_id: Uuid,
    pub version_no: i32,
    pub dim_value: Option<String>,
    pub model: Model,
    pub price: Option<PriceData>,
    pub min_fee: Option<Decimal>,
    pub eligibility: Eligibility,
    pub effective_from: Date,
    pub effective_to: Option<Date>,
    pub temporary_until: Option<Date>,
    pub paired_row_id: Option<Uuid>,
    pub return_of_row_id: Option<Uuid>,
    pub closed_explicitly: bool,
    pub state: RowState,
}
/// Parse a start before constructing a typed row.
/// # Errors
/// Returns `WINDOW_START_INVALID` for absent or invalid calendar dates.
pub fn parse_start(text: &str) -> Result<Date, RuleError> {
    Date::parse(text, &time::format_description::well_known::Iso8601::DATE)
        .map_err(|_| RuleError::new("WINDOW_START_INVALID"))
}
/// Validate all independent authoring rules. Currency scale is supplied by the book context.
#[must_use]
pub fn validate(
    row: &Row,
    kind: ChargeKind,
    values: Option<&[String]>,
    siblings: &[Row],
    today: Date,
    minor_digits: u32,
) -> Vec<RuleError> {
    let mut errors = Vec::new();
    if !model_allowed(kind, row.model) {
        errors.push(RuleError::new("MODEL_KIND_CHARGEKIND_MISMATCH"));
    }
    if let Some(data) = &row.price {
        errors.extend(money::validate(row.model, data));
    } else {
        errors.push(RuleError::new("PRICE_MISSING"));
    }
    if row.state != RowState::Approved && row.effective_from < today {
        errors.push(RuleError::new("WINDOW_START_IN_PAST"));
    }
    if siblings.iter().any(|other| {
        other.id != row.id
            && other.price_id == row.price_id
            && other.dim_value == row.dim_value
            && other.state == RowState::Approved
            && other.effective_from == row.effective_from
    }) {
        errors.push(RuleError::new("WINDOW_OVERLAP"));
    }
    if let Some(value) = &row.dim_value {
        match values {
            None => errors.push(RuleError::new("DIM_NOT_DECLARED")),
            Some(allowed) if !allowed.contains(value) => {
                errors.push(RuleError::new("DIM_VALUE_UNKNOWN"));
            }
            Some(_) => {}
        }
    }
    if row
        .min_fee
        .is_some_and(|fee| fee < Decimal::ZERO || fee.scale() > minor_digits)
    {
        errors.push(RuleError::new("MIN_FEE_INVALID"));
    }
    errors
}
/// Display state combines stored approval and the window at the requested date.
#[must_use]
pub fn status(row: &Row, today: Date) -> &'static str {
    window_status(row.state, row.effective_from, row.effective_to, today)
}
/// The same display state from the stored columns alone.
#[must_use]
pub fn window_status(state: RowState, from: Date, to: Option<Date>, today: Date) -> &'static str {
    if state != RowState::Approved {
        return state.as_str();
    }
    if to.is_some_and(|end| end <= today) {
        "superseded"
    } else if from > today {
        "scheduled"
    } else {
        "active"
    }
}
/// Approved rows of exactly one chain, ordered by start and version.
#[must_use]
pub fn approved_rows<'a>(rows: &'a [Row], price_id: Uuid, dim: Option<&str>) -> Vec<&'a Row> {
    let mut chain: Vec<_> = rows
        .iter()
        .filter(|r| {
            r.price_id == price_id && r.dim_value.as_deref() == dim && r.state == RowState::Approved
        })
        .collect();
    chain.sort_by_key(|r| (r.effective_from, r.version_no));
    chain
}
/// Recompute implicit ends; an explicit end is kept unless a successor starts inside it.
pub fn normalize_windows(rows: &mut [Row]) {
    let mut order: Vec<usize> = (0..rows.len())
        .filter(|i| rows[*i].state == RowState::Approved)
        .collect();
    order.sort_by_key(|i| {
        (
            rows[*i].price_id,
            rows[*i].dim_value.clone(),
            rows[*i].effective_from,
            rows[*i].version_no,
        )
    });
    for (pos, index) in order.iter().copied().enumerate() {
        let next_start = order.get(pos + 1).and_then(|next| {
            let r = &rows[*next];
            (r.price_id == rows[index].price_id && r.dim_value == rows[index].dim_value)
                .then_some(r.effective_from)
        });
        rows[index].effective_to = if rows[index].closed_explicitly {
            // An explicit end survives every normalisation, but a successor that
            // starts inside it still closes it: one row in force per chain and date.
            match (rows[index].effective_to, next_start) {
                (Some(end), Some(next)) => Some(end.min(next)),
                (end, next) => end.or(next),
            }
        } else {
            next_start
        };
    }
}
/// The row of exactly one chain (no default fallback) in force on a date.
#[must_use]
pub fn own_version_at<'a>(
    rows: &'a [Row],
    price_id: Uuid,
    date: Date,
    dim: Option<&str>,
) -> Option<&'a Row> {
    approved_rows(rows, price_id, dim)
        .into_iter()
        .rev()
        .find(|r| r.effective_from <= date && r.effective_to.is_none_or(|end| date < end))
}
/// Prefer the value's in-force row, then the default chain.
#[must_use]
pub fn version_at<'a>(
    rows: &'a [Row],
    price_id: Uuid,
    date: Date,
    dim: Option<&str>,
) -> Option<&'a Row> {
    own_version_at(rows, price_id, date, dim)
        .or_else(|| dim.and_then(|_| own_version_at(rows, price_id, date, None)))
}
/// The last approved row of the exact chain, only if unbounded.
#[must_use]
pub fn open_tail<'a>(rows: &'a [Row], price_id: Uuid, dim: Option<&str>) -> Option<&'a Row> {
    approved_rows(rows, price_id, dim)
        .last()
        .copied()
        .filter(|r| r.effective_to.is_none())
}
#[toolkit_macros::domain_model]
#[derive(Debug)]
pub struct Coverage<'a> {
    pub version: Option<&'a Row>,
    pub missing: Vec<String>,
    pub closing: Vec<String>,
}
/// Check present coverage and an open tail for every declared value.
#[must_use]
pub fn coverage_on<'a>(
    rows: &'a [Row],
    price_id: Uuid,
    date: Date,
    values: &[String],
) -> Coverage<'a> {
    let mut result = Coverage {
        version: version_at(rows, price_id, date, None),
        missing: Vec::new(),
        closing: Vec::new(),
    };
    let dims: Vec<Option<&str>> = if values.is_empty() {
        vec![None]
    } else {
        values.iter().map(|s| Some(s.as_str())).collect()
    };
    for dim in dims {
        if let Some(row) = version_at(rows, price_id, date, dim) {
            result.version = result.version.or(Some(row));
            if open_tail(rows, price_id, dim).is_none() && open_tail(rows, price_id, None).is_none()
            {
                result.closing.push(dim.unwrap_or_default().to_owned());
            }
        } else {
            result.missing.push(dim.unwrap_or_default().to_owned());
        }
    }
    result
}
/// Validate a temporary end before constructing a pair.
/// # Errors
/// Returns `WINDOW_END_INVALID` for invalid dates or a nonpositive duration.
pub fn validate_temporary(start: Date, until: &str) -> Result<Date, RuleError> {
    let end = parse_start(until).map_err(|_| RuleError::new("WINDOW_END_INVALID"))?;
    if end <= start {
        Err(RuleError::new("WINDOW_END_INVALID"))
    } else {
        Ok(end)
    }
}
/// Build a pair on an owned chain, or a single explicitly closed row on an empty chain, or
/// the promo alone when the chain's next row starts exactly on the end.
/// # Errors
/// Refuses a nonpositive duration or an exhausted version number.
pub fn temporary(
    rows: &[Row],
    mut promo: Row,
    until: Date,
    return_id: Uuid,
) -> Result<Vec<Row>, RuleError> {
    if until <= promo.effective_from {
        return Err(RuleError::new("WINDOW_END_INVALID"));
    }
    promo.temporary_until = Some(until);
    promo.paired_row_id = None;
    promo.return_of_row_id = None;
    // Return only to the chain's own row in force on the end date. A chain that
    // has ended, or that starts later, is neither revived nor copied backwards:
    // the value then falls back to the default after the temporary row.
    let back = own_version_at(rows, promo.price_id, until, promo.dim_value.as_deref());
    if back.is_some_and(|b| b.effective_from == until) {
        // The next approved row starts exactly on the end and already ends the promo: the
        // promo alone, which normalisation closes at that start; nothing to return to.
        promo.effective_to = Some(until);
        promo.closed_explicitly = false;
        return Ok(vec![promo]);
    }
    if let Some(back) = back {
        let mut returned = promo.clone();
        returned.id = return_id;
        returned.version_no = promo
            .version_no
            .checked_add(1)
            .ok_or_else(|| RuleError::new("VERSION_EXHAUSTED"))?;
        returned.effective_from = until;
        // Back to a row that itself ends explicitly (a closed value row): only until that end,
        // after which the value falls back to the default again.
        returned.effective_to = back.effective_to.filter(|_| back.closed_explicitly);
        returned.temporary_until = None;
        returned.closed_explicitly = back.closed_explicitly;
        returned.model = back.model;
        returned.price.clone_from(&back.price);
        returned.min_fee = back.min_fee;
        returned.return_of_row_id = Some(back.id);
        returned.paired_row_id = Some(promo.id);
        promo.paired_row_id = Some(returned.id);
        promo.closed_explicitly = false;
        promo.effective_to = Some(until);
        Ok(vec![promo, returned])
    } else {
        promo.effective_to = Some(until);
        promo.closed_explicitly = true;
        Ok(vec![promo])
    }
}
/// Whether a temporary row still matches the chain as it now stands (D-391). `rows` are the
/// approved rows outside the unit; `unit` holds the temporary row's partner. A pair's return
/// must restore the row in force on the (shifted) end, with that row's money. A temporary row
/// without a return is right only while nothing of its own chain is in force on its end, or
/// while the next row starts exactly there and so ends it.
#[must_use]
pub fn temporary_is_current(rows: &[Row], temporary: &Row, unit: &[Row]) -> bool {
    let Some(until) = temporary.temporary_until else {
        return true;
    };
    let back = own_version_at(
        rows,
        temporary.price_id,
        until,
        temporary.dim_value.as_deref(),
    );
    match temporary.paired_row_id {
        Some(partner) => {
            let returned = unit.iter().find(|r| r.id == partner);
            match (returned, back) {
                (Some(r), Some(b)) => {
                    r.return_of_row_id == Some(b.id)
                        && r.model == b.model
                        && r.price == b.price
                        && r.min_fee == b.min_fee
                }
                _ => false,
            }
        }
        None => match back {
            Some(b) => b.effective_from == until,
            None => temporary.closed_explicitly,
        },
    }
}
/// Shift a row and its explicit/temporary end by the same duration.
/// Call with the same displacement for the return partner.
/// # Errors
/// Refuses date overflow.
pub fn shift(row: &Row, new_from: Date) -> Result<Row, RuleError> {
    let delta = new_from - row.effective_from;
    let mut shifted = row.clone();
    shifted.effective_from = new_from;
    let move_date = |date: Date| {
        date.checked_add(delta)
            .ok_or_else(|| RuleError::new("WINDOW_END_INVALID"))
    };
    shifted.temporary_until = row.temporary_until.map(move_date).transpose()?;
    shifted.effective_to = row.effective_to.map(move_date).transpose()?;
    Ok(shifted)
}
/// Apply a common effective date to a selection (decision 7): every row starts on it,
/// except a pair's return half, which moves by its promo half's displacement so the
/// pair keeps its length. A return closed at its outer row's end keeps that end: the
/// outer row does not move. `None` moves nothing.
/// # Errors
/// Refuses date overflow.
pub fn shift_selection(rows: &[Row], date: Option<Date>) -> Result<Vec<Row>, RuleError> {
    let Some(date) = date else {
        return Ok(rows.to_vec());
    };
    rows.iter()
        .map(|r| {
            let promo = r
                .return_of_row_id
                .and(r.paired_row_id)
                .and_then(|partner| rows.iter().find(|p| p.id == partner));
            match promo {
                Some(promo) => {
                    let delta = date - promo.effective_from;
                    let start = r
                        .effective_from
                        .checked_add(delta)
                        .ok_or_else(|| RuleError::new("WINDOW_START_INVALID"))?;
                    let mut moved = shift(r, start)?;
                    if r.closed_explicitly {
                        moved.effective_to = r.effective_to;
                    }
                    Ok(moved)
                }
                None => shift(r, date),
            }
        })
        .collect()
}
/// The row of the same chain in force on the day before `row` starts, if any.
#[must_use]
pub fn in_force_before<'a>(chain: &'a [Row], row: &Row) -> Option<&'a Row> {
    let eve = row.effective_from.previous_day()?;
    own_version_at(chain, row.price_id, eve, row.dim_value.as_deref())
}
/// The input field a refusal code names, for the wire problem.
#[must_use]
pub fn field_of(code: &str) -> &'static str {
    match code {
        "MODEL_KIND_CHARGEKIND_MISMATCH" | "MODEL_INVALID" | "CHAIN_MODEL_CHANGED" => "model",
        "WINDOW_START_IN_PAST" | "WINDOW_START_INVALID" | "WINDOW_OVERLAP" => "effective_from",
        "WINDOW_END_INVALID" | "PAIR_RETURN_STALE" => "temporary_until",
        "DIM_NOT_DECLARED" | "DIM_VALUE_UNKNOWN" => "dim_value",
        "MIN_FEE_INVALID" => "min_fee",
        "ELIGIBILITY_INVALID" => "eligibility",
        "PAIR_SPLIT" | "ROW_NOT_IN_BOOK" | "ROW_NOT_DRAFT" => "row_ids",
        _ => "price",
    }
}
/// Draft rows of one book ordered by start, price id and version number.
#[must_use]
pub fn proposed_rows<'a>(
    book_id: Uuid,
    price_books: &[(Uuid, Uuid)],
    rows: &'a [Row],
) -> Vec<&'a Row> {
    let mut proposed: Vec<_> = rows
        .iter()
        .filter(|r| r.state == RowState::Draft && price_books.contains(&(r.price_id, book_id)))
        .collect();
    proposed.sort_by_key(|r| (r.effective_from, r.price_id, r.version_no));
    proposed
}
/// SKU version metering. Callers fetch each side as of that row's effective start (D-402).
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkuMetering {
    pub unit: Option<String>,
    pub usage_type_ref: Option<String>,
}
/// Preserve model, package size and dated metering on usage chains.
/// # Errors
/// Returns `CHAIN_MODEL_CHANGED` if any guarded attribute changes.
pub fn chain_guard(
    kind: ChargeKind,
    predecessor: &Row,
    before: &SkuMetering,
    successor: &Row,
    after: &SkuMetering,
) -> Result<(), RuleError> {
    if kind != ChargeKind::Usage {
        return Ok(());
    }
    let size = |r: &Row| match &r.price {
        Some(PriceData::Package { package_size, .. }) => Some(*package_size),
        _ => None,
    };
    if predecessor.model != successor.model
        || before != after
        || (predecessor.model == Model::Package && size(predecessor) != size(successor))
    {
        return Err(RuleError::new("CHAIN_MODEL_CHANGED"));
    }
    Ok(())
}
#[cfg(test)]
#[path = "row_tests.rs"]
mod tests;
