//! AM base **envelope** Type Schemas (`tenant_type.v1~`,
//! `tenant_metadata.v1~`) — the trait-carrier bases that derived vendor
//! tenant-type / metadata schemas chain from (via `$schema`) to inherit
//! behavioural traits.
//!
//! These are pure trait carriers marked `x-gts-abstract: true`: they
//! have no direct instances and only define the `x-gts-traits-schema`
//! (the trait shape, with per-trait `default`s). Concrete derived vendor
//! schemas resolve / inherit the trait values. Being abstract exempts
//! them from the OP#13 completeness check, so — unlike the previous
//! non-abstract workaround — they need NOT declare their own
//! `x-gts-traits` values (defaults flow from the trait-schema's
//! `default` keyword via `GtsTypeSchema::effective_traits`).
//!
//! Declared with `#[modkit_gts::gts_type_schema(..)]` using the
//! gts-rust 0.10.0 traits/modifiers support: `traits_schema = inline(T)`
//! emits `x-gts-traits-schema` from the Rust trait struct `T`, and
//! `gts_abstract = true` emits `x-gts-abstract: true`. The Rust trait
//! structs are the single source of truth; the
//! `docs/schemas/<name>.v1.schema.json` files document the same trait
//! contract and the drift guard (below) keeps them in agreement.
//!
//! The macro additionally emits a data-type top-level shape (a required
//! `id` field, `additionalProperties: false`). That shape is inert for
//! these envelopes: derived schemas chain via `$schema` (not
//! `allOf`/`$ref`), so the base body is never merged into payload
//! validation (`GtsTypeSchema::effective_schema`), trait resolution
//! reads only `x-gts-traits[-schema]`, and the tenant-type checker keys
//! off the chain prefix — none consult the envelope's top-level data
//! shape. See `account-management` `metadata_schema_registry` /
//! `types_registry::checker`.

use gts_macros::GtsTraitsSchema;
use modkit_gts::gts_type_schema;
use schemars::JsonSchema;

// ---------------------------------------------------------------------------
// tenant_type.v1~ envelope
// ---------------------------------------------------------------------------

// Behavioural traits for `gts.cf.core.am.tenant_type.v1~` — emitted as
// the envelope's `x-gts-traits-schema`. NOTE: a `///` doc comment here
// would surface as a `description` on the emitted `x-gts-traits-schema`
// object (which the documented contract omits), so this stays a plain
// `//` comment. Field `///` docs ARE wanted — they become the per-trait
// `description`; `#[serde(default)]` yields the `default` keyword and
// drops the trait from `required`.
// Field docs are verbatim copies of the documented `docs/schemas/` trait
// contract (the drift guard asserts byte-equality), so they are prose,
// not rustdoc — suppress the markdown-backtick lint.
// A single `allowed_parent_types` entry: the GTS type identifier of a
// tenant type permitted as parent. The `x-gts-ref` extension makes the
// gts store validate each value is a `tenant_type.v1~`-derived type id
// when a derived tenant-type schema is registered. The value is a bare
// `gts.`-prefixed literal, which per GTS spec §9.6 is enforced as a
// `startsWith` prefix check (no glob `*` — that is non-canonical; only
// the literal `gts.*` is a wildcard). The prefix also permits same-type
// nesting (a type listing its own id); `/$id` self-reference is a
// different, exact-match construct and is not what we want here.
// `transparent` so the schema is just the inner string + the extension
// (no wrapper title) and the value serialises as a bare string.
#[allow(dead_code)]
#[derive(serde::Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent, extend("x-gts-ref" = "gts.cf.core.am.tenant_type.v1~"))]
pub struct TenantTypeRef(pub String);

#[allow(clippy::doc_markdown)]
#[derive(JsonSchema, serde::Serialize, GtsTraitsSchema)]
#[serde(deny_unknown_fields)]
pub struct TenantTypeTraits {
    /// GTS instance identifiers of tenant types allowed as parent. Empty array means the type is root-only or leaf-only. The root tenant type has allowed_parent_types: [] by convention.
    #[serde(default)]
    pub allowed_parent_types: Vec<TenantTypeRef>,
    /// Whether tenants of this type typically require dedicated IdP-side resources. Account Management still calls IdpPluginClient::provision_tenant and deprovision_tenant; provider implementations may use this trait to decide whether to create resources or complete as a no-op.
    #[serde(default)]
    pub idp_provisioning: bool,
}

/// Base tenant-type envelope (abstract). Carries no instance data — the
/// `id` field exists only to satisfy the `gts-macros` base-struct
/// contract (a base must declare an `id: GtsInstanceId` or `gts_type:
/// GtsTypeId`) and is inert (see module docs).
#[gts_type_schema(
    dir_path = "schemas",
    base = true,
    type_id = "gts.cf.core.am.tenant_type.v1~",
    description = "Base tenant type schema for Account Management. Derived tenant type schemas resolve behavioral traits via x-gts-traits. Traits configure system behavior for processing tenants of each type - they are not part of the tenant instance data model.",
    properties = "id",
    traits_schema = inline(TenantTypeTraits),
    gts_abstract = true
)]
pub struct TenantTypeEnvelopeV1 {
    /// Required by the `gts-macros` base-struct contract; envelopes carry
    /// no instance data.
    pub id: gts::GtsInstanceId,
}

// ---------------------------------------------------------------------------
// tenant_metadata.v1~ envelope
// ---------------------------------------------------------------------------

// How `MetadataService` resolves a metadata schema's values across the
// tenant hierarchy. Serialises snake_case to match the wire tokens read
// by `metadata_schema_registry`. NOTE: variant `///` docs would make
// schemars emit a `oneOf` of `const` subschemas instead of the flat
// `enum: ["override_only", "inherit"]` the documented contract uses, so
// the variants are left undocumented here.
// Variants exist for schema generation (`enum` values); only `OverrideOnly`
// is constructed in Rust (via `Default`).
#[allow(dead_code)]
#[derive(JsonSchema, serde::Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetadataInheritancePolicy {
    #[default]
    OverrideOnly,
    Inherit,
}

// Behavioural traits for `gts.cf.core.am.tenant_metadata.v1~` — emitted
// as the envelope's `x-gts-traits-schema` (plain `//`; see
// `TenantTypeTraits`).
#[allow(clippy::doc_markdown)]
#[derive(JsonSchema, serde::Serialize, GtsTraitsSchema)]
#[serde(deny_unknown_fields)]
pub struct TenantMetadataTraits {
    /// How MetadataService resolves values across the tenant hierarchy for this schema. 'override_only' returns the tenant's own entry or empty; 'inherit' walks ancestors via parent_id, stopping at self-managed barriers.
    #[serde(default)]
    pub inheritance_policy: MetadataInheritancePolicy,
}

/// Base tenant-metadata envelope (abstract). See [`TenantTypeEnvelopeV1`]
/// for the inert-`id` rationale.
#[gts_type_schema(
    dir_path = "schemas",
    base = true,
    type_id = "gts.cf.core.am.tenant_metadata.v1~",
    description = "Base tenant metadata schema for Account Management. Derived metadata schemas resolve behavioral traits via x-gts-traits. Traits configure how MetadataService treats entries of each schema - they are not part of the metadata payload.",
    properties = "id",
    traits_schema = inline(TenantMetadataTraits),
    gts_abstract = true
)]
pub struct TenantMetadataEnvelopeV1 {
    /// Required by the `gts-macros` base-struct contract; envelopes carry
    /// no instance data.
    pub id: gts::GtsInstanceId,
}

#[cfg(test)]
mod sync_tests {
    //! Drift guard: the macro-generated envelope's **trait contract**
    //! (`x-gts-traits-schema` shape + the envelope's `x-gts-traits`
    //! defaults) MUST agree with the documented
    //! `docs/schemas/<name>.v1.schema.json`. The macro additionally emits
    //! a data-type top-level shape (`id`/`properties`/`required`/
    //! `additionalProperties`) that the docs omit by design and that is
    //! inert at runtime (see module docs), so only the trait subtrees are
    //! compared.

    use std::fs;
    use std::path::PathBuf;

    use gts::GtsSchema;
    use serde_json::Value;

    use super::{TenantMetadataEnvelopeV1, TenantTypeEnvelopeV1};

    fn read_disk_schema(file_name: &str) -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("docs")
            .join("schemas")
            .join(file_name);
        let body = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_json::from_str(&body)
            .unwrap_or_else(|err| panic!("parse {} as JSON: {err}", path.display()))
    }

    /// Assert the macro-generated `x-gts-traits-schema` equals the
    /// documented one, and that the envelope is marked abstract (so it is
    /// exempt from OP#13 completeness and needs no declared `x-gts-traits`).
    fn assert_trait_contract(generated: &Value, disk: &Value, type_id: &str) {
        assert_eq!(
            generated.get("x-gts-traits-schema"),
            disk.get("x-gts-traits-schema"),
            "macro x-gts-traits-schema for {type_id} drifted from docs/schemas/ -- \
             re-sync the trait struct with the documented trait contract",
        );
        assert_eq!(
            generated.get("x-gts-abstract"),
            Some(&Value::Bool(true)),
            "envelope {type_id} must be x-gts-abstract: true (pure trait carrier)",
        );
        assert_eq!(
            generated.get("x-gts-abstract"),
            disk.get("x-gts-abstract"),
            "envelope {type_id} x-gts-abstract drifted from docs/schemas/",
        );
    }

    #[test]
    fn tenant_type_envelope_trait_contract_matches_docs() {
        let generated = TenantTypeEnvelopeV1::gts_schema_with_refs();
        assert_trait_contract(
            &generated,
            &read_disk_schema("tenant_type.v1.schema.json"),
            TenantTypeEnvelopeV1::TYPE_ID,
        );
        // `type_id` and the `allowed_parent_types` `x-gts-ref` are separate
        // attribute literals (Rust attributes can't reference a const), so
        // guard that they agree: the ref must equal this envelope's TYPE_ID
        // so derived chains validate against the correct base prefix.
        let xref = generated
            .pointer("/x-gts-traits-schema/properties/allowed_parent_types/items/x-gts-ref");
        assert_eq!(
            xref.and_then(Value::as_str),
            Some(TenantTypeEnvelopeV1::TYPE_ID),
            "allowed_parent_types x-gts-ref literal must match the envelope's type_id literal",
        );
    }

    #[test]
    fn tenant_metadata_envelope_trait_contract_matches_docs() {
        assert_trait_contract(
            &TenantMetadataEnvelopeV1::gts_schema_with_refs(),
            &read_disk_schema("tenant_metadata.v1.schema.json"),
            TenantMetadataEnvelopeV1::TYPE_ID,
        );
    }
}
