//! P-D-262: the actor ids every read shows, and their `*_name` siblings.
//!
//! A read builds its answer from its statements, then names the actors of the whole answer with
//! one `ActorNames::fill`: no statement is added, and no transaction waits on Account Management.
//! A write answer may be stored as its key's receipt, and a name is never stored, so no write
//! answer names anyone: its `*_name` fields stay null.
use super::dto::{
    DecisionDto, ProductsDerivedUsageType, ProductsDerivedUsageTypeItem,
    ProductsDerivedUsageTypeVersion, ProductsDerivedVersionHeader, ProductsSkuHistoryEntry,
    SkuCard, SkuDto, SkuListItem, UnitDto, UnitList,
};
use bss_rest::actor_names::{ActorFields, Names, label};
use serde_json::Value;
use uuid::Uuid;

bss_rest::actor_fields!(SkuDto { created_by => created_by_name } []);
bss_rest::actor_fields!(SkuListItem {}[sku]);
bss_rest::actor_fields!(SkuCard {}[sku]);
bss_rest::actor_fields!(ProductsSkuHistoryEntry { actor => actor_name } []);
bss_rest::actor_fields!(DecisionDto { actor => actor_name } []);
bss_rest::actor_fields!(UnitList {}[items]);
bss_rest::actor_fields!(ProductsDerivedUsageTypeVersion { created_by => created_by_name } []);
bss_rest::actor_fields!(ProductsDerivedVersionHeader { created_by => created_by_name } []);
bss_rest::actor_fields!(ProductsDerivedUsageType { created_by => created_by_name } [versions]);
bss_rest::actor_fields!(ProductsDerivedUsageTypeItem { created_by => created_by_name } [latest]);

/// A unit names its submitter and its voters, and the creator of the live SKU its card carries
/// (`impact_live`, a [`SkuDto`] as JSON).
impl ActorFields for UnitDto {
    fn actor_ids(&self, ids: &mut Vec<Uuid>) {
        ids.push(self.submitted_by);
        self.decisions.actor_ids(ids);
        ids.extend(live_creator(self.impact_live.as_ref()));
    }
    fn fill_names(&mut self, names: &Names) {
        self.submitted_by_name = label(names, self.submitted_by);
        self.decisions.fill_names(names);
        if let Some(id) = live_creator(self.impact_live.as_ref())
            && let Some(Value::Object(live)) = self.impact_live.as_mut()
        {
            live.insert(
                "created_by_name".to_owned(),
                label(names, id).map_or(Value::Null, Value::String),
            );
        }
    }
}

/// The `created_by` of a live SKU a card carries as JSON.
fn live_creator(live: Option<&Value>) -> Option<Uuid> {
    live?.get("created_by")?.as_str()?.parse().ok()
}
