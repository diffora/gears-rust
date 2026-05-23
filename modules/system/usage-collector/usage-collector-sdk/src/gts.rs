//! GTS schema for usage-collector storage plugin instances.

use gts_macros::struct_to_gts_schema;
use modkit::gts::PluginV1;

pub const USAGE_RECORD_GTS: &str = "gts.cf.core.usage.record.v1~";

/// GTS type for storage backend plugin instances registered with types-registry.
#[derive(Default)]
#[struct_to_gts_schema(
    dir_path = "schemas",
    base = PluginV1,
    schema_id = "gts.cf.modkit.plugins.plugin.v1~cf.core.usage.plugin.v1~",
    description = "Usage Collector plugin specification",
    properties = "",
)]
pub struct UsageCollectorPluginSpecV1;
