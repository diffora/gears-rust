//! D-519: the actor ids every read shows, and their `*_name` siblings.
//!
//! A read builds its answer in its transaction, then names the actors of the whole answer with
//! one `ActorNames::fill` after the transaction has ended: no statement is added, and no
//! transaction waits on Account Management. A POST answer is stored as its key's receipt, and a
//! name is never stored, so no write answer names anyone: its `*_name` fields stay null.
use super::dto::{
    PriceBookExport, PricingApprovalUnitDto, PricingApprovalUnitList, PricingDecisionDto,
    PricingEntryPriceList, PricingExportEntry, PricingPlanCurrent, PricingPlanDto,
    PricingPlanEntrySummary, PricingPlanItemDto, PricingPlanItemReadDto, PricingPlanList,
    PricingPlanRevisionDto, PricingPlanRevisionHeader, PricingPlanRevisionReadDto,
    PricingPriceBookEntryList, PricingPriceBookEntryReadDto, PricingPriceDto, PricingProposedPrice,
    PricingPublishChanges, PricingSettingsDto, PricingSkuEntryDto, PricingSkuEntryList,
};
use super::{AuthoringState, support};
use axum::{http::StatusCode, response::Response};
use bss_rest::actor_names::{ActorFields, ActorName, label};
use std::collections::BTreeMap;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

type Names = BTreeMap<Uuid, ActorName>;

/// A read's 200: `body` with its actors named in one lookup, and its `ETag` when `version` is
/// given.
/// # Errors
/// An `ETag` that is not a header value.
pub(super) async fn named<T: ActorFields + serde::Serialize>(
    state: &AuthoringState,
    ctx: &SecurityContext,
    mut body: T,
    version: Option<u64>,
) -> Result<Response, CanonicalError> {
    state.actor_names.fill(ctx, &mut body).await;
    support::response(StatusCode::OK, &body, version)
}

/// Implements [`ActorFields`] for a response type: each actor id field with its `*_name`
/// sibling, then the nested values that carry their own.
macro_rules! actor_fields {
    ($type:ty { $($id:ident => $name:ident),* } [$($nested:ident),*]) => {
        impl ActorFields for $type {
            fn actor_ids(&self, ids: &mut Vec<Uuid>) {
                $(ids.push(self.$id);)*
                $(self.$nested.actor_ids(ids);)*
            }
            fn fill_names(&mut self, names: &Names) {
                $(self.$name = label(names, self.$id);)*
                $(self.$nested.fill_names(names);)*
            }
        }
    };
}

/// The settings' writer is null before the first write.
impl ActorFields for PricingSettingsDto {
    fn actor_ids(&self, ids: &mut Vec<Uuid>) {
        ids.extend(self.updated_by);
    }
    fn fill_names(&mut self, names: &Names) {
        self.updated_by_name = self.updated_by.and_then(|id| label(names, id));
    }
}

actor_fields!(PricingPriceDto { created_by => created_by_name } []);
actor_fields!(PricingPlanItemDto { created_by => created_by_name } []);
actor_fields!(PricingPlanItemReadDto {}[item]);
actor_fields!(PricingPlanRevisionHeader { created_by => created_by_name } []);
actor_fields!(PricingPlanCurrent { created_by => created_by_name } []);
actor_fields!(PricingPlanDto { created_by => created_by_name } [revisions, current]);
actor_fields!(PricingPlanList {}[items]);
actor_fields!(PricingPlanRevisionDto { created_by => created_by_name } [items]);
actor_fields!(PricingPlanEntrySummary {}[price_on_sale_date]);
actor_fields!(PricingPlanRevisionReadDto {} [revision, entries]);
actor_fields!(PricingDecisionDto { actor => actor_name } []);
actor_fields!(PricingApprovalUnitDto { submitted_by => submitted_by_name } [decisions]);
actor_fields!(PricingApprovalUnitList {}[items]);
actor_fields!(PricingPriceBookEntryReadDto {} [current_price, next_price]);
actor_fields!(PricingPriceBookEntryList {}[items]);
actor_fields!(PricingSkuEntryDto {} [current_price, next_price]);
actor_fields!(PricingSkuEntryList {}[items]);
actor_fields!(PricingEntryPriceList {}[items]);
actor_fields!(PricingExportEntry {}[prices]);
actor_fields!(PriceBookExport {}[entries]);
actor_fields!(PricingProposedPrice {} [price, before]);
actor_fields!(PricingPublishChanges {}[prices]);
