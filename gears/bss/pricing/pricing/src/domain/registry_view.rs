//! The registry read model, as the rule plane sees it (D-372).
//!
//! One registry read, indexed. The row-SKU rules are synchronous
//! [`ValidationRule`](crate::domain::validation::ValidationRule)s over a
//! [`PriceRow`](crate::domain::price_row::PriceRow) — the pipeline hands a rule
//! a subject and a report and nothing else — so the asynchronous
//! [`get_skus`](crate::domain::ports::ProductCatalogClientV1) of the ids the
//! write names happens **once at the door** and its result rides in beside
//! the subject.
//!
//! That is a deliberate shape rather than a convenience. A rule that could read
//! the registry itself would read it per row, so a plan of forty rows would make
//! forty calls whose answers may disagree with each other; and a publish judged
//! against a registry that moved mid-run would be judged against no single state
//! of the world at all. One read, one index, one verdict.
//!
//! The index is **not** a cache: it is built per request and dropped with it.
//! Holding one across requests would answer a publish from whatever the registry
//! said when the process started, which is exactly the drift D-372's I6 exists to
//! refuse.

use std::collections::HashMap;

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::ports::CatalogSku;
use crate::domain::scope_key::SkuId;

/// One registry listing, keyed by the id a price row's `sku_id` binds to.
///
/// Absence is the honest answer: a SKU the listing does not carry is one this
/// gear cannot judge, and [`Self::get`] says so rather than inventing a default.
/// `inst-pr-sku-published` turns that absence into `SKU_NOT_PUBLISHED`, and it is
/// the only rule that reads absence as a fault — see
/// [`row_sku_rules`](crate::domain::row_sku_rules) for why the other three
/// decline instead.
#[domain_model]
#[derive(Debug, Default)]
pub struct SkuIndex {
    by_id: HashMap<Uuid, CatalogSku>,
}

impl SkuIndex {
    /// Index a registry listing.
    ///
    /// A duplicate id keeps the **last** listing, which is `HashMap`'s own rule
    /// and not a decision this gear makes: the registry is the sole author of
    /// these ids and a listing carrying one twice is a registry fault, not a
    /// case with a correct resolution here.
    #[must_use]
    pub fn from_listing(listing: Vec<CatalogSku>) -> Self {
        Self {
            by_id: listing.into_iter().map(|sku| (sku.sku_id, sku)).collect(),
        }
    }

    /// The SKU `id` names, or `None` when this listing does not carry it.
    ///
    /// An **empty** index therefore refuses every row it judges, which is the
    /// property the door that builds one has to answer for: a failed or
    /// unconfigured registry read must not reach here as an empty listing, or a
    /// registry outage renders as a publish refusal naming the author's SKU.
    /// Task 7 owns that decision at the door.
    #[must_use]
    pub fn get(&self, id: SkuId) -> Option<&CatalogSku> {
        self.by_id.get(&id.as_uuid())
    }
}
