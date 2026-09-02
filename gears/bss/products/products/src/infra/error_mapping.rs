//! The SINGLE authoritative `DomainError` → AIP-193 `CanonicalError` ladder.
//!
//! Modelled on `gears/bss/pricing`'s own `infra::error_mapping`, checked
//! against it code by code (`design/01-foundation.md` §3.3's own note): a
//! domain variant is assigned a canonical category — and an HTTP status — in
//! exactly one place, so the wire contract cannot drift per handler. The
//! design set's own ladder line, quoted verbatim (`design/01-foundation.md`
//! §3.3, normative):
//!
//! > **Problem responses (RFC 9457):** `APPROVAL_REQUIRED` (403);
//! > `DUPLICATE_NAME`, `DUPLICATE_CODE`, `IDEMPOTENCY_CONFLICT`,
//! > `IDEMPOTENCY_KEY_IN_FLIGHT`, `PARENT_TERMINAL`, `PARENT_NOT_PUBLISHED`,
//! > `RETIREMENT_PENDING`, `STALE_REVISION`, `ENTITY_TERMINAL`,
//! > `ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION` (409); `AUDIT_UNAVAILABLE`
//! > (503); `SCOPE_NOT_CONTAINED`, `INCOMPLETE_ENTITY`, `VALIDATION`,
//! > `CONTENT_PII_BLOCKED` (422).
//!
//! # The 422s here are architectural, not wire
//!
//! `design/01-foundation.md` §3.3's own section, headed **"Status rendering
//! — the 422s in this set are architectural, not wire (normative)"**, is
//! the rule this module implements for `VALIDATION`, `SCOPE_NOT_CONTAINED`
//! and `INCOMPLETE_ENTITY`: the `422` annotation says *unprocessable
//! content*, not a wire status, and the platform's `CanonicalError` model
//! has no 422 category at all — `FailedPrecondition` (and
//! `InvalidArgument`/`OutOfRange`) render **400**
//! (`toolkit_canonical_errors::CanonicalError::status_code`) — so each of
//! the three reaches the wire as a 400 carrying its code, and **the code is
//! the discriminator, not the status**. An endpoint **MUST NOT** declare a
//! 422 response for an error carrying a registry code in its `OpenAPI`
//! registration.
//!
//! **This is a choice, not an impossibility, and the design set says so
//! explicitly** — read the whole section before reaching for a way around
//! it. `toolkit_canonical_errors::Http::status_code` builds a
//! `TransportOverride` that `ResourceErrorBuilder::with_override` can attach
//! to move a single occurrence's wire status within the same status class
//! (`CanonicalError::is_same_status_class` gates it to that), and 422 is in
//! `FailedPrecondition`'s 4xx class — the mechanism genuinely exists and
//! genuinely reaches 422. §3.3 considered exactly this and rejected it as an
//! **owner's call, 2026-08-27**: the section used to read "because no path
//! can produce one," found that false, and kept 400 anyway, because *"this
//! gear declares no transport override anywhere, and neither does
//! pricing — so every registry code has exactly one wire shape, which is
//! the property the rule is protecting."* One code, one wire shape, no
//! per-call-site exceptions — that is what a transport override on any of
//! these three would spend, for a status the design set weighed and
//! declined. **If you are here because you rediscovered
//! `Http::status_code` and are about to reach for it: don't — the answer is
//! this paragraph, and re-adding the override is what
//! `the_products_owned_422_codes_stay_wire_400_by_design` in
//! `error_mapping_tests.rs` exists to catch.**
//!
//! **One code the ladder line names is not this gear's to raise, and is
//! not mapped here.** `CONTENT_PII_BLOCKED` is slice `02`'s content
//! write-block. `DomainError` has no variant for it — mapping a code this
//! gear cannot raise would be a dead `match` arm. `PARENT_NOT_PUBLISHED`
//! and `RETIREMENT_PENDING` are 04's owned slots (P-D-96) and are mapped
//! below, with the rest of that slice's seven.
//!
//! # Two resource markers, not one
//!
//! The design set grants `product|sku × write` as two distinct authz labels
//! (`design/01-foundation.md` §2; `crate::authz::labels::{PRODUCT, SKU}`), and
//! a `Problem`'s `resource_type`/`resource_name` and a caller's authorization
//! both key on which of the two actually refused. Collapsing both into one
//! marker would misreport that: a `SKU` write refused by its parent
//! `Product`'s terminal state would read on the wire as a `Product` refusal,
//! which is not what happened and not what the caller's `sku × write` grant
//! was checked against. So this ladder declares **two**:
//!
//! - [`ProductResource`] — the default. Ten of the fourteen `DomainError`
//!   variants are Foundation-generic: the pipeline runs identically over a
//!   `Product` door and a `SKU` door (`design/01-foundation.md` §3.1's seven
//!   phases apply to both create/save/publish/discard doors alike), and
//!   `DomainError` itself carries no field distinguishing which door raised
//!   it — a gap this slice does not close (it would need a field on the enum,
//!   which is `domain/error.rs`, out of this slice's scope; wiring the
//!   correct marker per call site is Phase 4's, which owns the routes that
//!   know which door they are). `ProductResource` is the default for exactly
//!   the variants no other reasoning claims.
//! - [`SkuResource`] — the **three** variants that **cannot** arise on a
//!   `Product` door at all: `ParentTerminal` and `ScopeNotContained` are both
//!   raised only by "Define a SKU"'s containment guard, and
//!   `ParentNotPublished` — `04-lifecycle`'s `inst-pc-ordering`, admitted as
//!   an arm by **P-D-96** — is raised only by the SKU publish path's parent
//!   guard, which is the same shape: a `Product` has no parent whose
//!   publication could be missing
//!   (`design/01-foundation.md` §2, `inst-fd-containment-parent-state` and
//!   `inst-fd-containment-scope`) — a `Product` has no parent to check and no
//!   containment to prove, so these two are unambiguously `SKU`-resource
//!   regardless of the gap above.
//!
//! `AUDIT_UNAVAILABLE` takes **neither** marker, deliberately: it is the
//! audit plane failing to write its own row (`design/01-foundation.md` §4.4),
//! not a `Product` or a `SKU` refusing a write, so tagging it with either
//! marker would name a resource that did not refuse anything. It renders
//! through the bare `CanonicalError::service_unavailable()` builder, which
//! (like the sibling pricing gear's own 503s) carries no `resource_type` at
//! all.
//!
//! # Codes ride `DomainError::code()`, not a second literal
//!
//! Every arm below spells its code as the same `SCREAMING_SNAKE` literal
//! `DomainError::code()` returns for that variant — `error_mapping_tests.rs`
//! asserts every one of the fourteen against it, so the two cannot drift
//! apart silently even though this module cannot call the private-to-neither
//! but per-variant `code()` inline: doing so per arm through a single
//! precomputed `let code = err.code();` would make several arms'
//! bodies byte-for-byte identical (`aborted(detail, code)` for six different
//! variants), which `clippy::match_same_arms` (workspace-denied) refuses —
//! the literal keeps every arm's body distinguishable by the thing that
//! actually differs, its code, and the test file is what keeps the literal
//! honest against the enum.

use toolkit::api::canonical_prelude::{CanonicalError, resource_error};

use crate::domain::error::DomainError;

/// The `Product` entity's resource marker — the default; see the module doc's
/// "Two resource markers, not one" for which fourteen minus two land here.
#[resource_error(gts_id!("cf.bss.products.product.v1~"))]
struct ProductResource;

/// The `SKU` entity's resource marker — used only by the two containment
/// refusals that structurally cannot arise on a `Product` door; see the
/// module doc.
#[resource_error(gts_id!("cf.bss.products.sku.v1~"))]
struct SkuResource;

/// One architectural 422 (rendered 400, the code the discriminator), on the
/// default `ProductResource` marker. No transport override: see the module
/// doc's "The 422s here are architectural, not wire" — the override
/// mechanism exists and reaches 422, and §3.3 rejected using it here as an
/// owner's call, to keep one wire shape per registry code.
fn precondition(field: &'static str, detail: &str, code: &'static str) -> CanonicalError {
    ProductResource::failed_precondition()
        .with_precondition_violation(field, detail, code)
        .create()
}

/// The conflict class's plain shape: a retriable 409 carrying its detail and
/// a bare reason code, on the default `ProductResource` marker.
fn aborted(detail: String, code: &'static str) -> CanonicalError {
    ProductResource::aborted(detail).with_reason(code).create()
}

/// The 403 `APPROVAL_REQUIRED` refusal. The detail is dropped rather than
/// folded into `reason`: `permission_denied()` has no detail slot of its own
/// (a fixed platform sentence fills it), and `reason` is the only free field,
/// so a consumer matching `reason` to `APPROVAL_REQUIRED` exactly — the same
/// exactness the sibling pricing gear's identical 403s rely on — would never
/// match if the detail rode along as `"CODE: detail"`.
fn denied(code: &'static str) -> CanonicalError {
    ProductResource::permission_denied()
        .with_reason(code)
        .create()
}

impl From<DomainError> for CanonicalError {
    fn from(err: DomainError) -> Self {
        use DomainError as D;
        match err {
            // -- The aggregate validation envelope (architectural 422,
            // rendered 400 with no transport override — see the module
            // doc's "The 422s here are architectural, not wire") -- Every
            // blocking violation the `shape` phase collected rides as its
            // own precondition violation, so the caller sees the whole
            // per-field remediation list in one response
            // (`design/01-foundation.md` §3.1's fail-closed collection
            // rule).
            D::Validation(report) => {
                let mut violations = report.violations().iter();
                let Some(first) = violations.next() else {
                    // A rejection with nothing to remediate is a bug in the
                    // pipeline, not a client error: reporting it as a
                    // precondition failure would tell the author to fix
                    // something the response does not name.
                    return CanonicalError::internal(
                        "products: validation failed with an empty report",
                    )
                    .create();
                };
                let mut builder = ProductResource::failed_precondition()
                    .with_precondition_violation(
                        first.subject.clone(),
                        first.detail.clone(),
                        first.code,
                    );
                for violation in violations {
                    builder = builder.with_precondition_violation(
                        violation.subject.clone(),
                        violation.detail.clone(),
                        violation.code,
                    );
                }
                builder.create()
            }

            // -- Aborted (409) -- conflicts the caller can resolve and
            // retry, or a refusal by the row's own current state
            // (`design/01-foundation.md` §3.3's `state`-phase 409 rule,
            // P-D-32).
            D::DuplicateName(detail) => aborted(detail, "DUPLICATE_NAME"),
            D::DuplicateCode(detail) => aborted(detail, "DUPLICATE_CODE"),
            D::StaleRevision { expected, found } => aborted(
                format!("expected {expected}, found {found}"),
                "STALE_REVISION",
            ),
            // The live-entity analogue of `STALE_REVISION`, and the same 409
            // class: `design/02` §3.5 lists it among that slice's conflict
            // codes, and a precondition that no longer holds is an aborted
            // act rather than a malformed request.
            D::StaleLiveOp(detail) => aborted(detail, "STALE_LIVE_OP"),
            D::MeterDeclarationIncomplete(detail) => {
                precondition("meter", &detail, "METER_DECLARATION_INCOMPLETE")
            }
            D::UnrecognizedUnit(detail) => precondition("meter", &detail, "UNRECOGNIZED_UNIT"),
            D::UnitDeprecated(detail) => precondition("meter", &detail, "UNIT_DEPRECATED"),
            D::UnitDelistBlocked(detail) => aborted(detail, "UNIT_DELIST_BLOCKED"),
            D::PlanTierRetireBlocked(detail) => aborted(detail, "PLAN_TIER_RETIRE_BLOCKED"),
            D::AccountingCodeDelistBlocked(detail) => {
                aborted(detail, "ACCOUNTING_CODE_DELIST_BLOCKED")
            }
            D::IdempotencyConflict(detail) => aborted(detail, "IDEMPOTENCY_CONFLICT"),
            D::IdempotencyKeyInFlight(detail) => aborted(detail, "IDEMPOTENCY_KEY_IN_FLIGHT"),
            D::EntityTerminal(detail) => aborted(detail, "ENTITY_TERMINAL"),
            // The clone door's state refusal (P-D-75): the same 409 class —
            // the source's state refuses the act — with its own code, because
            // ENTITY_TERMINAL's meaning is a head write and the clone writes
            // nothing to the source.
            D::CloneSourceDiscarded(detail) => aborted(detail, "CLONE_SOURCE_DISCARDED"),

            // -- FailedPrecondition (400) under the consumer's discriminator.
            // The violation TYPE is `CATALOG_VERSION_REJECTED` — the string
            // pricing's `Rejected` arm matches on (P-D-52) — while the audit
            // row records the domain code `REQUEST_SOURCE_UNKNOWN`; a 403
            // here would land on the consumer projection's `Other` arm and
            // leave `Rejected` unreachable.
            D::RequestSourceUnknown(detail) => precondition(
                "source",
                &detail,
                bss_products_sdk::increments::CATALOG_VERSION_REJECTED,
            ),

            // -- The catalog-version seven (`dod-cv-error-taxonomy`), minus
            // the one above: statuses per the FEATURE's own table, each code
            // through `DomainError::code`'s spelling.
            D::IntentRequired(detail) => precondition("intent", &detail, "INTENT_REQUIRED"),
            D::FreezeIncomplete(detail) => aborted(detail, "FREEZE_INCOMPLETE"),
            D::VersionForcedIncomplete(detail) => aborted(detail, "VERSION_FORCED_INCOMPLETE"),
            D::StagedEntityChanged(detail) => aborted(detail, "STAGED_ENTITY_CHANGED"),
            // 404: the path segment names a resource this tenant has none
            // of. The 404 shape carries no code channel; the code rides
            // `DomainError::code()` into the audit row, which is the
            // channel the taxonomy DoD binds.
            D::CatalogVersionUnknown(detail) => ProductResource::not_found(&detail)
                .with_resource("catalog-version".to_owned())
                .create(),
            // 403 rather than 404 (`dod-cv-error-taxonomy`): the identity is
            // the refusal's subject and a 404 would leak version existence.
            D::ParticipantUnknown(_detail) => denied("PARTICIPANT_UNKNOWN"),

            // -- The bulk five (`dod-bulk-errors`). Four are per-row ledger
            // outcomes whose status applies where the ledger reader reports
            // one row's disposition; `BulkLimit` is the import door's own
            // refusal. Bulk introduces no parallel taxonomy — a row's other
            // failures carry the owning feature's code verbatim.
            D::BulkDependencyFailed(detail) => {
                precondition("dependency", &detail, "BULK_DEPENDENCY_FAILED")
            }
            D::PromotionIdentityConflict(detail) => aborted(detail, "PROMOTION_IDENTITY_CONFLICT"),
            D::PromotionDirtyHead(detail) => aborted(detail, "PROMOTION_DIRTY_HEAD"),
            D::BulkOverrideUnacknowledged(detail) => {
                precondition("override", &detail, "BULK_OVERRIDE_UNACKNOWLEDGED")
            }
            D::BulkLimit(detail) => aborted(detail, "BULK_LIMIT"),

            // -- The watermark door's four (`design/07` §3.2). The future
            // bound is the architectural 422; the other three follow the
            // ladder's ordinary reading of identity and state.
            D::ProducerUnregistered(_detail) => denied("PRODUCER_UNREGISTERED"),
            D::WatermarkRegression(detail) => aborted(detail, "WATERMARK_REGRESSION"),
            D::WatermarkConflict(detail) => aborted(detail, "WATERMARK_CONFLICT"),
            D::WatermarkFuture(detail) => precondition("watermark_at", &detail, "WATERMARK_FUTURE"),
            D::IllegalTransition { from, to } => {
                aborted(format!("from {from} to {to}"), "ILLEGAL_TRANSITION")
            }
            D::IllegalFieldMutation(detail) => aborted(detail, "ILLEGAL_FIELD_MUTATION"),
            // The parent's own state, `SkuResource`'s for the module doc's
            // reason: only "Define a SKU"'s containment guard raises it.
            D::ParentTerminal(detail) => SkuResource::aborted(detail)
                .with_reason("PARENT_TERMINAL")
                .create(),
            // `SkuResource` for the same reason as its sibling above, and
            // **not** the generic `aborted` helper: the refusal is raised only
            // by the SKU publish path's parent guard, so like `ParentTerminal`
            // it cannot arise on a `Product` door at all. A `Product` has no
            // parent whose publication could be missing.
            D::ParentNotPublished(detail) => SkuResource::aborted(detail)
                .with_reason("PARENT_NOT_PUBLISHED")
                .create(),
            D::RetirementPending(detail) => aborted(detail, "RETIREMENT_PENDING"),
            D::ScheduleStaleApproval(detail) => aborted(detail, "SCHEDULE_STALE_APPROVAL"),
            D::ReplacedByNotPublished(detail) => {
                precondition("replacedBy", &detail, "REPLACED_BY_NOT_PUBLISHED")
            }
            D::RetirementLeadTime(detail) => {
                precondition("effectiveAt", &detail, "RETIREMENT_LEAD_TIME")
            }
            D::CascadeConfirmationRequired(detail) => {
                precondition("cascade", &detail, "CASCADE_CONFIRMATION_REQUIRED")
            }
            D::EolDisabled(detail) => precondition("mustMigrateBy", &detail, "EOL_DISABLED"),

            // -- PermissionDenied (403) -- the governance gate's own
            // refusal, raised at every gated act (`design/01-foundation.md`
            // §3.1's `GovernanceGate` phase).
            D::ApprovalRequired(_detail) => denied("APPROVAL_REQUIRED"),
            // The ceremony's own 403, on the same channel and for the same
            // reason: the caller may not take this act whatever the record's
            // state, so it is denied rather than aborted. `design/05` §3.3
            // puts `SELF_APPROVAL_FORBIDDEN` in its 403 list beside
            // `APPROVAL_REQUIRED`.
            D::SelfApprovalForbidden(_detail) => denied("SELF_APPROVAL_FORBIDDEN"),
            // 409, not 403: `design/05` §3.3's convention puts **409** where
            // the current state refuses the act and **403** where the caller
            // may not take it. A superseded record is the first.
            D::ApprovalSuperseded(detail) => aborted(detail, "APPROVAL_SUPERSEDED"),
            // 409: the tree's current state refuses the name, and the index
            // is what decided it (`02` §3.3).
            D::DuplicateCategoryName(detail) => aborted(detail, "DUPLICATE_CATEGORY_NAME"),

            // -- Architectural 422s (rendered 400, no transport override —
            // see the module doc's "The 422s here are architectural, not
            // wire") beyond `Validation` -- `ScopeNotContained` is
            // `SkuResource`'s for the module doc's reason: a `Product` has
            // no parent to prove containment against.
            D::ScopeNotContained(detail) => SkuResource::failed_precondition()
                .with_precondition_violation("scope", &detail, "SCOPE_NOT_CONTAINED")
                .create(),
            D::IncompleteEntity(detail) => precondition("entity", &detail, "INCOMPLETE_ENTITY"),
            D::PrimaryCategoryRequired(detail) => {
                precondition("categories", &detail, "PRIMARY_CATEGORY_REQUIRED")
            }
            // The erasure door's unknown-principal refusal is the same
            // architectural-422 class: the request's content cannot be
            // processed, and the wire renders it 400 with no transport
            // override (@cpt-dod:cpt-cf-bss-products-dod-retention-error-taxonomy:p1).
            D::ErasureUnknownActor(detail) => {
                precondition("actor", &detail, "ERASURE_UNKNOWN_ACTOR")
            }
            // The same architectural-422 class: a re-parent that would close
            // a cycle is content the door cannot process, and the wire
            // renders it 400 with the code as the discriminator.
            D::TaxonomyCycle(detail) => precondition("parentId", &detail, "TAXONOMY_CYCLE"),
            // Slice `02`'s content-PII block, the same architectural-422
            // class: operator free text the door may not store is content it
            // cannot process, and the wire renders it 400 with the code as
            // the discriminator.
            //
            // **The only one of `02`'s sixteen codes with an arm here**, and
            // the reason is where each is raised. The seven content rules
            // raise into a `ValidationReport`, so the `Validation` arm above
            // already renders each violation's own code and a second arm
            // would be unreachable; the remaining codes have no raiser at all
            // until their doors' routes are declared, and an arm for a code
            // this gear cannot raise is the dead `match` arm this module's
            // own paragraph on `PARENT_NOT_PUBLISHED` refuses.
            D::ContentPiiBlocked(detail) => precondition("content", &detail, "CONTENT_PII_BLOCKED"),

            // -- Unavailable (503) -- fail closed, retry later. See the
            // module doc for why this carries neither resource marker.
            //
            // Unlike the sibling pricing gear's 503s, this one has no
            // upstream seam already reporting it: an audit-row write failure
            // is per-request, not a boot-time or deployment fact standing
            // until somebody redeploys, so it reports itself here rather
            // than staying silent on the assumption that something else
            // logs it.
            D::AuditUnavailable(detail) => {
                tracing::error!(
                    dependency = "audit_log",
                    detail,
                    "bss-products: dependency unavailable"
                );
                CanonicalError::service_unavailable().create()
            }
        }
    }
}

#[cfg(test)]
#[path = "error_mapping_tests.rs"]
mod error_mapping_tests;
