//! `GET /bss-products/v1/products/{id}` — the Product read door
//! (`cpt-cf-bss-products-dod-read-door`, `docs/features/foundation.md`
//! "Authoring head read") — and `POST /bss-products/v1/products`, the
//! Product create door (`cpt-cf-bss-products-dod-create-doors`,
//! `cpt-cf-bss-products-dod-code-reservation`, `docs/features/foundation.md`
//! "Create doors" and "Code reservation, atomic at insert").
//!
//! This was the first door this gear opened with a request body an author
//! actually reads, and the plan put it first in Phase 4 for a reason worth
//! restating here: without it an author who has not just written a Product
//! has no `ETag` to send back as `If-Match`, so every mutating door this
//! phase still owes — save, publish, discard — depends on this one existing
//! first. `POST /bss-products/v1/products`, this file's other door, is the
//! first of those; **`POST /bss-products/v1/skus` is the next slice's, not
//! this one's** — no containment or parent check is built here, and
//! `crate::domain::containment` (already present, unused by this file) is
//! where that logic lands.
//!
//! # Authorization
//!
//! [`get_product`] gates on `product x read` (`crate::authz::resource_types
//! ::PRODUCT`, `crate::authz::actions::READ`) with `owner_tenant_id = None` —
//! the PDP derives the scope from the subject rather than from anything the
//! caller sent — and `require_constraints = true`, so an unconstrained
//! *allow* fail-closes instead of exposing every tenant's catalog.
//! `resource_id` is pinned to the requested id, since this is a single-row
//! read. **The scope [`get_product`] hands to [`repo::find_product`] is the
//! one the PDP returned**, never one built from `ctx.subject_tenant_id()`
//! directly — a door that authorized against the compiled scope and then
//! read under a tenant-shaped one would leave the SQL-level filter
//! unenforced on the read that actually touches the row.
//!
//! [`create_product`] gates on `product x write`, `owner_tenant_id =
//! Some(tenant_id)` — the row is written **to** the caller's own tenant, so
//! `crate::authz::access_scope`'s cross-tenant membership assertion applies —
//! and the same `require_constraints = true`. See [`create_product`]'s own
//! doc for why this gate runs **after** [`repo::resolve_actor_ref`], not
//! before it.
//!
//! # The miss: absent and out-of-scope answer identically
//!
//! [`repo::find_product`] already documents why it answers `Ok(None)` both
//! when no such row exists and when one exists outside the caller's
//! authorized scope: a repository that told the two apart would let a caller
//! learn that a row belongs to another tenant just by asking for it, which
//! is the existence leak `docs/design/01-foundation.md` §3.3 keeps this
//! catalog closed against. [`get_product`] preserves that: `Ok(None)` maps to
//! one `not_found` build with the requested id and nothing else, never a
//! distinguishable code or detail for either cause.
//!
//! The status this module answers a miss with is a plain `404`, not a
//! `403`. `docs/design/01-foundation.md` §3.3 is explicit that **a `404` is
//! bare** (P-D-35): "a path segment is judged before the pipeline opens, so
//! no phase raises it ... no rule raises it at all." A row this door cannot
//! show the caller is exactly that kind of judgement — made before any
//! Foundation rule runs over the row's content — so it renders the same way
//! an unmatched path segment would: `404`, carrying a resource type and the
//! id the caller asked for, but no `DomainError`-style registry code.
//!
//! # The hit: `200` plus the `ETag`
//!
//! A found row answers `200` with [`ProductView`] and an `ETag` header built
//! from `internal_revision` (`preconditions::etag`), never
//! `published_version` — see `domain::concurrency`'s own doc for why the two
//! counters are not interchangeable here. Emitting the tag is not optional:
//! a caller who cannot obtain one cannot satisfy a mutating door's `If-Match`
//! precondition on the very next request.
//!
//! # The create door: what it writes, and what it deliberately does not
//!
//! [`create_product`] persists exactly one row and one outbox message, in one
//! transaction: the entity as `draft`, `published_version = 0`,
//! `internal_revision = 1`, and the `ProductCreated` event
//! (`crate::infra::events`, P-D-22, P-D-27) — **and nothing else**. Content
//! rows — 02's category assignments, 03's attribute values — belong to the
//! save door under P-D-46; slice 01's open item 11 asks whether a create
//! should ever admit them, and until that resolves this door writes none.
//!
//! A caller-supplied id is refused, not silently dropped: [`CreateProductRequest`]
//! carries an explicit, optional `id` field, and a present value is refused
//! `VALIDATION` naming `id` — see that type's own doc for why this reading of
//! `dod-create-doors` was chosen over the field-less one.
//!
//! **`brand_id` claims this door cannot check.** `dod-create-doors` and
//! P-D-33 both call for `brand_id` to be "validated against the caller's
//! brand claims", refusing `VALIDATION` when it names a brand the caller
//! does not hold. Measured against what is actually on the wire:
//! `toolkit_security::SecurityContext` (`libs/toolkit-security/src/
//! context.rs`) exposes exactly five things — `subject_id`, `subject_type`,
//! `subject_tenant_id`, `token_scopes`, `bearer_token` — and none of them is
//! a brand claim or a set of held brands. There is nothing on this door's
//! own `SecurityContext` to validate `brand_id` against, and inventing a
//! lookup (a table this slice's target paths exclude, a side-channel this
//! design set never named) would be answering a question the identity layer
//! has not yet been asked. This door therefore validates only that
//! `brand_id` is present and non-nil (`VALIDATION` otherwise) — the part of
//! `dod-create-doors` this door *can* discharge — and the "does the caller
//! hold this brand" half stays open, owed to whoever adds a brand claim to
//! `SecurityContext` (the token-issuer/identity owner, not this gear).
//!
//! **Telling `DUPLICATE_NAME` from `DUPLICATE_CODE` from an unrelated
//! storage failure.** `infra::storage::repo::insert_product`'s own doc
//! states plainly that Phase 1 left the conflict undifferentiated —
//! `RepoError::Db`, one shape for a scope failure, a `CHECK` violation or
//! either unique-index collision — because no caller existed yet to act on
//! a finer answer. This door is that caller, and `infra::storage::repo` is
//! outside this slice's target paths, so the repository's return type could
//! not be widened into a typed conflict here. [`classify_insert_conflict`]
//! instead reads the driver text the failure already carries, anchored to
//! the two facts the migration fixes and this file can cite without
//! guessing: the unique indexes' own names,
//! `uq_products_product_name`/`uq_products_product_code`
//! (`infra/storage/migrations/m20260829_000002_create_products_product.rs`),
//! and the columns each covers. Postgres's driver text names the constraint
//! by that literal index name; `SQLite`'s (the backend this file's own test
//! suite runs against) instead lists the covered columns —
//! `name_normalized` / `product_code` — so [`classify_insert_conflict`]
//! checks for either form. **The cost, stated plainly**: this is a
//! substring match over a driver's own error text, not a typed database
//! answer — a future driver upgrade that reworded either message, or a
//! third index added to the same table whose name also contains
//! `product_code` or `name_normalized`, could misclassify silently. The
//! correct long-term fix is exactly what this slice cannot build: a typed
//! conflict variant on [`RepoError`] itself, returned by `insert_product`
//! rather than inferred after the fact.
//!
//! # The event
//!
//! `ProductCreated`'s envelope is built by `crate::infra::events`, not here —
//! see that module's doc for the body core and the partition formula
//! (P-D-22), and `crate::infra::broker` for the SDK envelope the producer arm
//! carries instead. Which of the two a given boot uses is
//! [`crate::infra::broker::EventSink`]'s to say; this door only enqueues
//! through it. Two earlier revisions of this sentence each described a state
//! the code had already left — a wiring gap, then an unlanded producer — which
//! is why it now names the type rather than the state.
//!
//! # Idempotency: what this door claims, and where
//!
//! A create carrying an `Idempotency-Key` claims
//! `(tenant_id, "/bss-products/v1/products", key)` for the digest of its own
//! parsed body ([`payload_digest`]), and **the claim `INSERT` runs inside the
//! mutation's transaction** ([`insert_product_with_event`], P-D-42) so a
//! rollback frees the key. A create **without** the header skips the phase
//! and creates normally (P-D-34) — it is a skip, not a refusal, and a later
//! edit must not invert it. `endpoint` is the concrete resource path, never
//! a route template ([`CREATE_ENDPOINT`]).
//!
//! The three outcomes: a fresh claim proceeds; a stored answer whose digest
//! matches is replayed with nothing executed; a stored answer under a
//! different digest is `IDEMPOTENCY_CONFLICT` and a live claim is
//! `IDEMPOTENCY_KEY_IN_FLIGHT`. Both refusals go through the same
//! `crate::api::rest::audit_refusal_and_report` every other refusal here
//! uses — an idempotency refusal is a refusal, and gets no fourth path.
//!
//! A committed create **writes its answer back** before the transaction
//! ends: `state = answered` with the `201` and the rendered
//! [`ProductView`], through `crate::api::rest::record_idempotency_answer`,
//! on the same `tx` as the claim and the row
//! ([`insert_product_with_event`]). That is what makes the client's own
//! in-window retry replay the original response instead of being refused
//! `IDEMPOTENCY_KEY_IN_FLIGHT` for an act that already succeeded.
//!
//! # The publish door
//!
//! `POST /bss-products/v1/products/{id}/publish` (`dod-publish-door`,
//! `docs/design/01-foundation.md` §2 "Publish an entity (the mechanics
//! half)") gates on `product x publish` and runs, in this order:
//!
//! 1. `actor_ref` resolution and authorization — **as at create**, and
//!    through the same helpers ([`open_head_door`]).
//! 2. The idempotency phase, whose key's `endpoint` is the **concrete
//!    resource path** and therefore carries the id ([`publish_endpoint`],
//!    P-D-42). A create's concrete path happens to be a constant because
//!    there is no id yet; this door's is not, and the store's key type was
//!    widened to an owned `String` rather than have this door claim under
//!    the route template.
//! 3. `If-Match` is read and parsed here (`VALIDATION` if absent or
//!    unreadable) but **compared** inside the act, for the ordering reason
//!    below (**P-D-33**).
//! 4. The head read, whose only miss is this module's bare `404`.
//! 5. Then the act itself, on **one transaction** ([`run_publish`]): the
//!    idempotency claim; the head re-read under the write; terminality
//!    (`ENTITY_TERMINAL`); the precondition comparison (`STALE_REVISION`);
//!    the **full validation pipeline re-run** (`inst-fd-publish-revalidate`)
//!    over the entity as it now stands, so one that stopped being
//!    publishable since approval fails closed — **with no containment step**,
//!    because a Product has no parent: `products_product` carries
//!    `region_scope`/`brand_scope` as its own bucket-iii columns and no
//!    reference upwards, so the SKU door's own
//!    `recheck_parent_containment` — a child re-checking itself against its
//!    parent — has no analogue here, and a publish writes neither scope
//!    column, so it can move nothing its children would have to be re-judged
//!    against. §3.3's `SCOPE_NOT_CONTAINED` does reach this entity kind, at
//!    the **save** door, where a narrowing can orphan a live child; see
//!    [`check_children_stay_contained`]. The **governance
//!    gate**, whose
//!    [`GateMode`] is an explicit argument of the in-process entry point
//!    ([`publish_product_under_gate`]) and never a wire parameter
//!    (`inst-fd-gate-mode`) — the `REST` handler passes
//!    [`GateMode::Gate`] and nothing else — refusing `APPROVAL_REQUIRED`;
//!    then the writes —
//!    freeze the **post-act image** at `published_version + 1` first,
//!    because the head-row guard admits the bump only where the matching
//!    frozen row already exists; then **exactly one** head-row `UPDATE`,
//!    because the guard bumps `internal_revision` on every admitted `UPDATE`
//!    without exception and two statements would bump twice for one act;
//!    then the `ProductPublished` enqueue, through the outbox path the
//!    create doors already use.
//! 6. The idempotency key is answered with the success response, as at
//!    create, so the client's own retry replays it.
//! 7. **Every refusal audits**, on its own runner, before it is reported —
//!    and an unwritable audit row answers `AUDIT_UNAVAILABLE` 503 instead of
//!    the domain refusal, exactly as at create. The one exception is the
//!    bare `404`, and [`open_head_door`]'s doc says why it can have no audit
//!    row.
//!
//! ## Why the precondition is compared inside the act and not before it
//!
//! This is the one place the head doors' shape differs from the create
//! doors', and it is forced by two rules that only interact here.
//! `Phase::Idempotency` runs **before** `Phase::Precondition`
//! (`crate::domain::validation::Phase`), and P-D-42 puts the claim `INSERT`
//! inside the guarded mutation's transaction. Together they put every phase
//! after the claim inside that transaction too.
//!
//! Reversing them is not a stylistic choice but a functional defect, and a
//! silent one: a client whose publish committed and whose response was lost
//! retries with the `If-Match` it still holds — the revision *before* the
//! act it never learned about — so its precondition is stale **by
//! construction**. A door that judged the precondition first would refuse
//! every such retry `STALE_REVISION` and would never reach the stored
//! answer, leaving the idempotency store inert at exactly this door.
//! `products_tests::a_replayed_publish_serves_the_stored_answer_and_does_not_publish_twice`
//! is what holds this closed; it fails with a `409` against the other order.
//!
//! [`run_publish`] carries the same argument at the code, and
//! [`publish_revalidation_pipeline`] carries what the re-run does and does
//! **not** currently reach — the Foundation's own registered rule set is one
//! rule at this commit, and the `→ published` validators the design names
//! are 04's and 05's, which do not exist yet. The door re-runs the pipeline
//! it has; it does not claim to run rules nobody has registered.
//!
//! The content the freeze renders is named by [`PRODUCT_CONTENT_ROSTER`],
//! which is where a later slice adding a content column changes one thing
//! rather than three — and `products_tests::
//! the_product_content_roster_is_the_head_table_minus_the_excluded_columns`
//! is what fails if that slice forgets, since it derives the expected roster
//! from the executed schema rather than from a second hand-written list.
//!
//! ## What this door does **not** build, and who owns each
//!
//! Three clauses of `dod-publish-door`/`inst-fd-publish-txn` are not
//! discharged here. None is silently omitted; each has an owner and a
//! measurable reason:
//!
//! - **The retirement re-announcement** (`inst-fd-publish-reannounce`,
//!   **P-D-48**): discharged. [`run_publish`] reads the live retire intent
//!   through [`repo::find_live_retire_intents`] and, when the publish sits
//!   inside the lead window, enqueues `ProductRetired` with the new
//!   `fromVersion` in the same transaction. A publish outside any window
//!   enqueues none.
//!
//!   @cpt-dod:cpt-cf-bss-products-dod-lead-window-reannounce:p1
//! - **The corrected bucket-ii value** (`inst-fd-publish-correction`,
//!   **P-D-41**): §4.2 admits a bucket-ii write only in the same statement as
//!   a `published_version` bump, and the door that supplies one is **slice
//!   07**'s `CorrectionDoor`, which has no caller here. When it lands, the
//!   value must be applied to the record **before** [`freeze_for`] renders it
//!   *and* carried by [`repo::publish_product_head`]'s own statement — not by
//!   a second one.
//! - **`composition_pending`** (§4.2, **P-D-32**): a `products_sku` column,
//!   and **only** that — `bundle` is a value of the SKU-only `type` column, so
//!   `products_product` has no twin and this Product door may never carry the
//!   flag at all. It is not a gap on this side; it is an asymmetry in the
//!   schema. §1.5's **In** list names *"the `PublishDoor`'s
//!   `composition_pending` write"* and that write is built, on the SKU door
//!   alone (`skus::run_publish`, `repo::publish_sku_head`).
//!   [`repo::publish_product_head`]'s own doc states the same from the storage
//!   side.
//!
//! A clause that **was** owed and is now discharged: **`publishedVersion` on
//! the `ProductPublished` body** (§4.5): *"every one of the eight carries the
//! same body core ... `ProductPublished`/`SkuPublished` **additionally carry
//! `publishedVersion`**, which is what 06 reads as content and 08's projector
//! keys on"*. Both this door and the SKU one enqueued the core alone while
//! `infra::events` sat outside either slice's target paths. It now carries the
//! field, on [`events::PublishedEventBody`] rather than as a sixth field of
//! [`events::EventBodyCore`] — "additionally" is the word that decides that,
//! and `events`' own module doc argues it. [`announce_and_answer`] supplies
//! the post-act `N + 1`, read back off the head the act committed.
//!
//! A fourth clause is discharged by a host that does nothing:
//! `inst-fd-publish-consume` requires the gate's `satisfied` record to be
//! flipped `consumed` in this transaction. [`NoMaterialityPolicyGate`]
//! names no record ([`crate::domain::governance::ApprovalDisposition::NoRecord`]), so there is nothing
//! to consume, and `GateAuthorization::approval_to_consume` is the only
//! route to an id for the flip — it answers `None`, which is why the flip is
//! absent rather than forgotten. Slice 05 supplies both the record and the
//! store the flip writes to.
//!
//! # The discard door
//!
//! `POST /bss-products/v1/products/{id}/discard` (`dod-transition-guard`,
//! §2 "Discard a never-published draft") is legal **only** from `draft` with
//! `published_version = 0`; the transition is `draft → discarded`, which is
//! terminal. Anything else is `ILLEGAL_TRANSITION` and a head write on an
//! already-terminal row is `ENTITY_TERMINAL` — two refusals, decided by
//! `transition::guard`, which asks terminality first so the answer names the
//! rule that actually refused.
//!
//! The legality conditions are **also** in [`repo::discard_product_head`]'s
//! own `WHERE` clause, and that is the copy that decides: a read-then-write
//! would let a concurrent publish slip between the guard's answer and the
//! statement. The guard decides the *edge*; the database decides the *row*.
//!
//! **The reservations release by that same write.** `uq_products_product_name`
//! and `uq_products_product_code` are both partial unique indexes excluding
//! `discarded` rows, so the name and the `productCode` leave both indexes the
//! moment the `UPDATE` commits and are free for the next holder. There is no
//! release step, and a later reader should not add one — there would be
//! nothing left for it to release. This is also why a discarded draft frees
//! its name while a `retired` entity keeps it: the predicate names
//! `discarded` alone.
//!
//! **What the transition costs the row** is read off `transition::guard`'s
//! return value rather than decided at the call site
//! ([`transition::invalidation_for`]): a transition bumps `internal_revision`
//! and
//! fires the approval-invalidation hook, **except** one that consumes an
//! approval in the same transaction, which bumps once with no hook (P-D-26,
//! P-D-34). A discard consumes none, so the hook fires; publish's
//! `draft → published` is the gated edge, so it bumps once — through the
//! door's own single `UPDATE` — and fires no hook. Both doors run the same
//! [`fire_invalidation_hook`] call on the same argument, so neither has the
//! distinction hard-coded.
//!
//! **The gate phase runs on the discard door too, and passes trivially.**
//! §3.1's `inst-fd-pipeline-gate-phase` puts the phase at *every* mutating
//! door and has it pass where the act is ungated (**P-D-34**), and §1.1 calls
//! governance *"a registered gate phase inside the pipeline, hosting any
//! gated act ... not a separate path around it"*;
//! [`crate::domain::validation::Phase::GovernanceGate`] carries the same
//! words. So [`run_discard`] asks the host, in [`GateMode::Gate`], exactly
//! as [`run_publish`] does, and the default host authorizes naming no record
//! — a discard consumes no approval and today requires none. What that buys
//! is the case the phase exists for: the moment slice 05 registers a
//! ceremony on a transition, this door already has the seam and needs no
//! reopening. `inst-fd-governance-gate` is **not** the authority for
//! skipping it — that instruction is about the publish door and does not
//! reach the question.
//!
//!
//! # The deprecate door — the one span this module registers that the set
//! # does not declare
//!
//! `POST /bss-products/v1/products/{id}/deprecate` performs `design/04` §2
//! `inst-lc-deprecate`: `published → deprecated` with the `direct` stamp in
//! the same statement, then the cascade onto the children with a stated
//! disposition per state, each moved child written **pinned at the revision
//! the classification read** and announced as `SkuDeprecated`, the whole act
//! one transaction. The set declares no entity-scoped span for it — its one
//! declared carrier is `09`'s batch lane — so the route, and the choice to
//! gate it on `product × write` plus a SKU-scoped `sku × write` for the
//! child half, are the crate's own; `features/lifecycle.md` §7 row 36
//! carries that question, and the two `DoD`s this door reaches stay unticked
//! until it resolves. `dod-deprecation-provenance`, `dod-deprecation-cascade`.
//!
//! # The save door
//!
//! `PATCH /bss-products/v1/products/{id}` (`cpt-cf-bss-products-dod-save-door`,
//! §4.1's bucket assignment, `inst-fd-transition-bump`) is the third head act
//! and shares the whole spine of the other two: [`open_head_door`], the
//! claim, `transaction_with_retry`, the audited refusal, [`answer_head_act`].
//! What is its own is the **bucket routing** and what it declines to write.
//!
//! It runs, in `crate::domain::validation::Phase::ordered()`'s own order:
//! the idempotency claim on the mutation's runner; the `If-Match` comparison
//! **inside** the transaction; the `JSON` shape of every named value; then
//! terminality and bucket routing over the **whole** request before any
//! column is written; then the governance-gate phase in [`GateMode::Gate`],
//! passing trivially; then **exactly one** head-row `UPDATE` carrying the
//! routed columns, `internal_revision += 1` and `updated_at`; then the
//! approval-invalidation hook, which a save **fires**; then the event and
//! the stored answer. [`run_save`] carries the argument for each.
//!
//! **A save writes no version row and takes no edge.** The head is the
//! authoring surface in every non-terminal state (`inst-fd-transition-guard`)
//! and `published_version` does not move, so a published Product is renamed
//! here rather than through a second door — §4.1's own reading of bucket iii.
//!
//! **The buckets, and what each answers.** Bucket i (`product_code`,
//! `brand_id`) is admitted while `published_version = 0` and refused
//! `ILLEGAL_FIELD_MUTATION` after first publish. Bucket iii (`name` with its
//! index operand, `region_scope`, `brand_scope`) is admitted on any
//! non-terminal head. Bucket ii would be refused after first publish
//! **naming** slice 07's correction door and not forwarding to it, and
//! `cloned_from`'s create-only class would be refused in any update at all —
//! **neither class has a column at this commit** (`crate::domain::bucket`'s
//! module doc measures that), so both arms are built and neither is
//! reachable. A field the registry has no row for is P-D-50's fail-closed
//! miss and is refused rather than routed to a default bucket.
//!
//! **What the `DoD` still owes, and to whom.** `dod-save-door` also covers a
//! content-row half this slice cannot build — `products_product_category` and
//! `products_attribute_value` (**slice 02**), whose tables do not exist at
//! this commit, and the metering declaration (**slice 03**), which is not a
//! table at all: its §4 puts `metering_unit` and `usage_type_ref` on
//! `products_sku`, and that table carries neither column yet. The `DoD`
//! therefore reads as **partial**, not met; [`save_product_under_gate`]'s own
//! doc says which slice owns each and where it lands.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-read-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-create-doors:p1
//! @cpt-dod:cpt-cf-bss-products-dod-code-reservation:p1
//! @cpt-dod:cpt-cf-bss-products-dod-idempotency-store:p1
//! @cpt-cf-bss-products-dod-publish-door
//! @cpt-dod:cpt-cf-bss-products-dod-transition-guard:p1
//! @cpt-cf-bss-products-dod-save-door

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::ETAG;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use sea_orm::DbErr;
use serde_json::{Map as JsonMap, Value as JsonValue};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::DbError;
use toolkit_db::secure::{AccessScope, DBRunner, TxConfig};
use toolkit_security::SecurityContext;
use uuid::Uuid;

use bss_products_sdk::models::{EntityKind as CatalogEntityKind, LifecycleState};

use crate::api::rest::preconditions;
use crate::api::rest::skus::{
    CancelRetirementRequest, cancel_payload_digest, classify_sku_insert_conflict,
    confirmation_must_hold, insert_sku_with_event, interim_retirement_lead, no_live_retire_intent,
    parent_scope_pair, scope_input_from_payload, scope_not_contained_domain_err,
};
use crate::api::rest::{
    ApiState, CREATE_RESPONSE_STATUS, ClaimVerdict, CompositeClaimVerdict, CreateOutcome,
    IdempotencyClaimInput, authz_error_to_canonical, claim_composite_idempotency,
    claim_idempotency, contention_db_err, idempotency_key, record_idempotency_answer,
    replay_response, repo_error_to_canonical, require_authenticated,
};
use crate::domain::bucket;
use crate::domain::canonical;
use crate::domain::cascade::{
    CascadePlan, DeferralResolution, PARENT_FLIP_HELD_REASON, arm_for, parent_flip_clears,
    require_cascade_confirmation,
};
use crate::domain::concurrency::InternalRevision;
use crate::domain::containment::ResolvedScope;
use crate::domain::deprecation::{ChildDisposition, Provenance, disposition_for, stamp_for};
use crate::domain::disposition::{
    self, CLONE_SUGGESTION_ATTEMPTS, ProductCloneSource, SkuCloneSource,
};
use crate::domain::error::DomainError;
use crate::domain::governance::{
    ApprovalId, EntityRef, GateAuthorization, GateMode, GateSubject, GovernanceGate,
    NoMaterialityPolicyGate,
};
use crate::domain::idempotency;
use crate::domain::live_op::GovernedLiveOp;
use crate::domain::name;
use crate::domain::retirement::{effective_at, eol_lockout, publish_reannounces_retirement};
use crate::domain::rules::{
    CreateEntityCandidate, NameShapeRule, PrimaryCategoryRequired, PublishedTransitionSubject,
};
use crate::domain::taxonomy::{
    AssignmentCandidate, AssignmentRole, AttributeDefinitionKnownRule, CarriedDefinition,
    CategoryRoleConflictRule, ContentSaveSubject, LocalizedValue, PiiDetector,
    PublishedContentSubject, ResolvedDefinition, ValueCandidate, content_pii_block,
    content_save_pipeline, published_content_pipeline,
};
use crate::domain::transition::{
    self, ApprovalInvalidation, ApprovalInvalidationHook as _, NoApprovalStoreHook,
};
use crate::domain::undeprecation::{
    BlockingIntent, children_the_reversal_touches, refuse_if_live_retire_intents,
};
use crate::domain::validation::{ValidationPipeline, ValidationReport};
use crate::infra::events;
use crate::infra::storage::RepoError;
use crate::infra::storage::repo::{
    self, HeadWrite, NewDeferredRetirement, NewEntityVersion, NewProduct, NewScheduledTransition,
    NewSku, NullableText, ProductRecord, RefusalSubject, SavedName, SkuRecord, VersionedEntityKind,
};

/// `OpenAPI` tag for the Product surface's operations.
const TAG: &str = "BSS Products";

/// The `endpoint` component of every idempotency key this door claims
/// (**P-D-42**: *"`endpoint` MUST be the concrete resource path, not the
/// route template"*).
///
/// A create's concrete path **is** the collection path — there is no id yet
/// to put in it — so this constant and the path [`router`] registers are the
/// same string, and they are two spellings on purpose: the router's is a
/// literal because route registration is what the architecture lint over
/// route shape reads, and this one is what the store is keyed by. They must
/// stay equal; a door claiming a key under a path it does not serve would
/// let one caller's key collide with another door's.
///
/// Three lane names are reserved for callers with no wire surface —
/// `internal:scheduled-activation`, `internal:cascade-leg`,
/// `internal:bulk-row` — and this phase has none, so none is used here. They
/// are named rather than defined as constants because the first non-`HTTP`
/// caller is the one that knows which of the three it is and what it writes
/// in `client_key`.
const CREATE_ENDPOINT: &str = "/bss-products/v1/products";

/// The Product entity's resource marker for this file's own 403/404
/// answers — an authz deny on either door, and the read door's scope-closed
/// miss. Deliberately its own type, distinct from `infra::error_mapping`'s
/// private `ProductResource`: that one renders the `DomainError` ladder
/// [`create_product`]'s conflicts and validation failures raise
/// (`DomainError`'s own `From<DomainError> for CanonicalError`, via
/// `.into()`), carrying its own copy of this same GTS resource type. Both
/// name the same entity from call sites that do not share a `DomainError`
/// ladder to route through, which is `infra::error_mapping`'s own stated
/// reason for not sharing a single marker across every caller either.
#[resource_error(gts_id!("cf.bss.products.product.v1~"))]
struct ProductResource;

/// The read surface of a Product head.
///
/// Carries exactly the fields `docs/features/foundation.md`'s read-door
/// contract names: the ids, the name and its external code, the lifecycle
/// state, both revision counters, both scope columns, the creator and the
/// two timestamps. `name_normalized` is deliberately absent — it is the
/// uniqueness index's own operand, never a field an author reads or writes.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(response)]
pub struct ProductView {
    /// The row's own id.
    pub product_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The brand the Product belongs to.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// The optional external mapping code.
    pub product_code: Option<String>,
    /// Where the entity sits in the lifecycle machine (`LifecycleState`'s
    /// wire spelling — carried as a plain string on the read surface, the
    /// same way `fx_revaluation_mode::FxRevaluationModeView` carries its own
    /// enum field, since neither enum derives `Serialize` on its own).
    pub lifecycle_state: String,
    /// Moves on every admitted write. The operand of this door's `ETag`.
    pub internal_revision: i64,
    /// Moves only on publish.
    pub published_version: i64,
    /// The region value set. Empty means unrestricted.
    pub region_scope: String,
    /// The brand value set. Empty means unrestricted.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant.
    pub created_at: DateTime<Utc>,
    /// The instant of the row's last admitted write.
    pub updated_at: DateTime<Utc>,
}

/// `POST /products/{id}/deprecate`'s `200` body: the head as
/// [`ProductView`] answers it, plus the cascade's three listings.
///
/// A type of its own — never three keys grafted onto a rendered
/// [`ProductView`] — for two reasons that already cost a review finding
/// each: the `OpenAPI` schema must carry the listings a generated client can
/// read (a schema of `ProductView` alone advertises a body without the
/// operator-visible half of `dod-deprecation-cascade`), and the field names
/// must follow the same `snake_case` rule `api_dto` stamps on every other
/// response of this gear, which hand-inserted map keys silently did not.
#[toolkit_macros::api_dto(response)]
pub struct DeprecatedProductView {
    /// The row's own id.
    pub product_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The brand the Product belongs to.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// The optional external mapping code.
    pub product_code: Option<String>,
    /// Where the entity sits in the lifecycle machine — `deprecated`, on
    /// every success this door answers.
    pub lifecycle_state: String,
    /// Moves on every admitted write. The operand of this door's `ETag`.
    pub internal_revision: i64,
    /// Moves only on publish; a deprecation never touches it.
    pub published_version: i64,
    /// The region value set. Empty means unrestricted.
    pub region_scope: String,
    /// The brand value set. Empty means unrestricted.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant.
    pub created_at: DateTime<Utc>,
    /// The instant of the row's last admitted write.
    pub updated_at: DateTime<Utc>,
    /// The children this act moved to `deprecated` with a `cascaded` stamp.
    pub cascaded_skus: Vec<Uuid>,
    /// The children left untouched because they were already `deprecated` —
    /// their provenance is never re-stamped.
    pub already_deprecated_skus: Vec<Uuid>,
    /// The `draft` children skipped and listed, never transitioned. This
    /// listing is the operator-visible half of `dod-deprecation-cascade`.
    pub skipped_draft_skus: Vec<Uuid>,
}

impl DeprecatedProductView {
    fn from_parts(record: ProductRecord, report: CascadeReport) -> Self {
        Self {
            product_id: record.product_id,
            tenant_id: record.tenant_id,
            brand_id: record.brand_id,
            name: record.name,
            product_code: record.product_code,
            lifecycle_state: record.lifecycle_state.as_str().to_owned(),
            internal_revision: record.internal_revision,
            published_version: record.published_version,
            region_scope: record.region_scope,
            brand_scope: record.brand_scope,
            created_by: record.created_by,
            created_at: record.created_at,
            updated_at: record.updated_at,
            cascaded_skus: report.deprecated,
            already_deprecated_skus: report.left_untouched,
            skipped_draft_skus: report.skipped_drafts,
        }
    }
}

impl From<ProductRecord> for ProductView {
    fn from(record: ProductRecord) -> Self {
        Self {
            product_id: record.product_id,
            tenant_id: record.tenant_id,
            brand_id: record.brand_id,
            name: record.name,
            product_code: record.product_code,
            lifecycle_state: record.lifecycle_state.as_str().to_owned(),
            internal_revision: record.internal_revision,
            published_version: record.published_version,
            region_scope: record.region_scope,
            brand_scope: record.brand_scope,
            created_by: record.created_by,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// Build the Axum router for the Product read and create doors and register
/// both with the supplied `OpenAPI` registry.
///
/// Registers its own absolute paths (`/bss-products/v1/products/{id}`,
/// `/bss-products/v1/products`) rather than being nested under the gear's
/// prefix by a caller — the same shape the sibling ledger gear's door
/// modules use — so `register_rest` merges this router directly onto the
/// host router.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/bss-products/v1/products/{id}")
        .operation_id("bss_products.get_product")
        .summary("Read a Product head")
        .description(
            "Returns the Product head named by `id`: its identity, its lifecycle state, both \
             revision counters and both scope columns. Gates on `product x read`; a Product \
             outside the caller's authorized scope reads exactly like an absent one (`404`, no \
             existence leak). The `ETag` header carries `internal_revision` and is what \
             `PATCH`/`POST .../publish`/`POST .../discard` accept back as `If-Match`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to read.")
        .handler(get_product)
        .json_response_with_schema::<ProductView>(openapi, StatusCode::OK, "The Product head.")
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);

    let router = OperationBuilder::post("/bss-products/v1/products")
        .operation_id("bss_products.create_product")
        .summary("Create a Product")
        .description(
            "Mints a new Product head as `draft` (`published_version = 0`, \
             `internal_revision = 1`) and enqueues its `ProductCreated` event in the same \
             transaction, and writes nothing else; content rows are the save door's. Gates on \
             `product x write`. The id is server-minted; a caller-supplied `id` is refused \
             `VALIDATION`. \
             `product_code`'s and the normalized name's uniqueness are reserved by the insert \
             itself: a collision refuses `DUPLICATE_CODE`/`DUPLICATE_NAME`, each with an \
             audited reason. \
             An optional `Idempotency-Key` header claims the key \
             `(tenant, /bss-products/v1/products, key)` in the same transaction as the \
             mutation: a duplicate under a live key is refused \
             `IDEMPOTENCY_KEY_IN_FLIGHT`, and the same key under a different payload is \
             refused `IDEMPOTENCY_CONFLICT`. A request without the header is created \
             normally.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<CreateProductRequest>(openapi, "The Product to create.")
        .handler(create_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::CREATED,
            "The created Product head.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/products/{id}/clone")
        .operation_id("bss_products.clone_product")
        .summary("Clone a Product")
        .description(
            "Clones a Product per design/11's disposition table: new id minted, name and \
             product_code suggested first-free (`{name}-copy-N`, `-revived` for a retired \
             source, P-D-62) unless overridden in the body, brand and scopes copied, \
             lifecycle reset to `draft`, and `cloned_from`/`cloned_from_version` written in \
             the creating statement (P-D-76). A draft source is read at its head; a \
             published, deprecated or retired source is read from its last frozen version, \
             never the head's pending edits. A `discarded` source is refused \
             `CLONE_SOURCE_DISCARDED` (409, P-D-75). Gates on `product x write`; an \
             operator-supplied name or code collision is the ordinary \
             `DUPLICATE_NAME`/`DUPLICATE_CODE`. Accepts an optional `Idempotency-Key`: a \
             keyed retry replays the first clone (P-D-75).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to clone.")
        .json_request::<CloneProductRequest>(openapi, "The optional overrides.")
        .handler(clone_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::CREATED,
            "The created clone's head.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/products/{id}/publish")
        .operation_id("bss_products.publish_product")
        .summary("Publish a Product")
        .description(
            "Freezes the Product's current content as `products_entity_version` version N+1 and \
             moves the head in one transaction: `published_version += 1`, \
             `internal_revision += 1`, and `lifecycle_state = 'published'` where the head was a \
             draft. A re-publish of a `published` or `deprecated` head changes the version and \
             leaves the state alone. Gates on `product x publish` and requires `If-Match` on the \
             head's internal revision: an absent precondition is refused `VALIDATION`, a stale \
             one `STALE_REVISION`. The full validation pipeline is re-run here, so an entity \
             that stopped being publishable since it was approved fails closed; the governance \
             gate runs inside the door and a refusal is `APPROVAL_REQUIRED`. A head that is \
             `retired` or `discarded` is refused `ENTITY_TERMINAL`. Enqueues \
             `ProductPublished` in the same transaction, and accepts an optional \
             `Idempotency-Key`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to publish.")
        .handler(publish_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::OK,
            "The published Product head.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = register_save_door(router, openapi);

    let router = OperationBuilder::post("/bss-products/v1/products/{id}/discard")
        .operation_id("bss_products.discard_product")
        .summary("Discard a never-published draft Product")
        .description(
            "Moves a `draft` Product with `published_version = 0` to the terminal `discarded` \
             state, bumps `internal_revision` and enqueues `ProductDiscarded`, in one \
             transaction. The Product's name and `productCode` reservations release by that \
             same write, both partial unique indexes excluding discarded rows. Any other \
             starting state is refused `ILLEGAL_TRANSITION`, and a head that is already \
             `retired` or `discarded` is refused `ENTITY_TERMINAL`. Gates on \
             `product x write` and requires `If-Match` on the head's internal revision. \
             Accepts an optional `Idempotency-Key`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to discard.")
        .handler(discard_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::OK,
            "The discarded Product head.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    let router = register_deprecate_door(router, openapi);
    let router = register_undeprecate_door(router, openapi);
    let router = register_retire_door(router, openapi);
    let router = register_cancel_retire_door(router, openapi);
    let router = register_resume_retire_door(router, openapi);

    router.layer(Extension(state))
}

/// Register the deprecate door on `router`.
///
/// The Product save door, lifted out of [`router`] for the reason
/// [`register_deprecate_door`] was: that function crossed clippy's
/// `too_many_lines` floor again, this time at 202, when this door's
/// `.description` was re-wrapped with the `\` continuations its nine siblings
/// already used — the literal had been hand-joined onto one 1,186-character
/// line with thirteen runs of baked-in indentation inside it.
///
/// The save door is the right one to lift next: it is the largest block left
/// inline, and its own doc below is where a reader asks what a `PATCH` may
/// name.
fn register_save_door(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::patch("/bss-products/v1/products/{id}")
        .operation_id("bss_products.save_product")
        .summary("Save a Product head")
        .description(
            "Writes the named fields onto the Product head in one guarded UPDATE, bumps \
             `internal_revision` by one and enqueues `ProductHeadSaved`, and writes no \
             version row and moves no `published_version`, the head being the authoring \
             surface in every non-terminal state. Every field the body names is routed by its \
             field-mutability bucket before any of them is written, so a request naming one \
             refused field applies none of the others. Identity fields (`brand_id`, \
             `product_code`) are admitted only before first publish and refused \
             `ILLEGAL_FIELD_MUTATION` after it; `name`, `region_scope` and `brand_scope` are \
             admitted on any non-terminal head, published or not. A field no bucket registry \
             row names is refused `ILLEGAL_FIELD_MUTATION` rather than routed to a default. \
             Gates on `product x write` and requires `If-Match`: absent is `VALIDATION`, \
             stale is `STALE_REVISION`. A `retired` or `discarded` head is refused \
             `ENTITY_TERMINAL`. Accepts an optional `Idempotency-Key`, whose digest is taken \
             over this body.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to save.")
        .json_request::<SaveProductRequest>(openapi, "The fields to write.")
        .handler(save_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::OK,
            "The saved Product head, at its new revision.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Split out of [`router`] because that function crossed clippy's
/// `too_many_lines` floor when this seventh registration landed, and the
/// deprecate door is the right one to lift: it is the only span in this
/// module the design set does not declare (`features/lifecycle.md` §7 row
/// 36), so a reader looking for the crate's own additions finds them in one
/// function rather than by diffing seven blocks.
fn register_deprecate_door(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/products/{id}/deprecate")
        .operation_id("bss_products.deprecate_product")
        .summary("Deprecate a published Product and cascade onto its SKUs")
        .description(
            "Moves a `published` Product to `deprecated`, stamping \
             `deprecation_provenance = 'direct'` in the same statement as the state change, \
             bumping `internal_revision` and enqueuing `ProductDeprecated` with the \
             provenance in its payload. In the same transaction the deprecation cascades \
             onto the Product's SKUs, with a stated disposition per child state: a \
             `published` child is deprecated `cascaded` and announced as `SkuDeprecated`; an \
             already-`deprecated` child is left untouched and its provenance is never \
             re-stamped; a `draft` child is skipped and listed, the transition floor \
             admitting no `draft -> deprecated` edge; `retired` and `discarded` children are \
             outside the population. The response carries the head plus the three listings \
             (`cascaded_skus`, `already_deprecated_skus`, `skipped_draft_skus`). Any other \
             starting state is refused `ILLEGAL_TRANSITION`, and a `retired` or `discarded` \
             head is refused `ENTITY_TERMINAL`. Gates on `product x write` and, for the \
             child reads and writes, `sku x write`. Requires `If-Match` on the head's \
             internal revision. Accepts an optional `Idempotency-Key`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to deprecate.")
        .handler(deprecate_product)
        .json_response_with_schema::<DeprecatedProductView>(
            openapi,
            StatusCode::OK,
            "The deprecated Product head, plus the cascade's three listings.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Register the un-deprecate door on `router`.
fn register_undeprecate_door(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/products/{id}/undeprecate")
        .operation_id("bss_products.undeprecate_product")
        .summary("Un-deprecate a Product and reverse cascaded child deprecations")
        .description(
            "Moves a `deprecated` Product to `published` and, in the same transaction, \
             reverses only those child SKUs whose `deprecation_provenance` is `cascaded`. \
             A child's `direct` deprecation survives. Empty body; the two-person ceremony \
             is the governance gate (`Gate` mode), not a payload. Refused `RETIREMENT_PENDING` \
             while a live retire intent exists on the Product or on any child this reversal \
             would revive. Gates on `product x write` and, for the child writes, `sku x write`. \
             Requires `If-Match`. Accepts an optional `Idempotency-Key`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to un-deprecate.")
        .handler(undeprecate_product)
        .json_response_with_schema::<ProductView>(
            openapi,
            StatusCode::OK,
            "The published Product head.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// `POST …/products/{id}/retire` request — §3.3. Same fields as the SKU
/// door plus `cascadeConfirmed`. `replacedBy` has no Product column and is
/// accepted so the wire shape matches; it is not persisted.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct RetireProductRequest {
    /// Operator reason. 02's PII hook is not in this crate.
    pub reason: String,
    /// Optional successor. No Product column; accepted, not written.
    pub replaced_by: Option<Uuid>,
    /// Optional operator instant. Absent: now + interim lead (30 days).
    pub effective_at: Option<DateTime<Utc>>,
    /// Accepted so a supplied value raises `EOL_DISABLED`.
    pub must_migrate_by: Option<DateTime<Utc>>,
    /// Narrowest confirmation that lets the door exist.
    pub confirmed: bool,
    /// Absent or false over live children is `CASCADE_CONFIRMATION_REQUIRED`.
    pub cascade_confirmed: Option<bool>,
}

fn register_retire_door(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/products/{id}/retire")
        .operation_id("bss_products.retire_product")
        .summary("Initiate Product retirement")
        .description(
            "Forces the Product `deprecated` if it is `published`, records a retire \
             `ScheduledTransition`, and applies the cascade plan in the same transaction. \
             `confirmed` is required. `cascadeConfirmed` is required when live children \
             exist. `mustMigrateBy` is refused `EOL_DISABLED`. Gates on `product x write` \
             and, for child writes, `sku x write`. Requires `If-Match`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product to retire.")
        .json_request::<RetireProductRequest>(openapi, "Retirement initiation.")
        .handler(retire_product)
        .json_response_with_schema::<ProductView>(openapi, StatusCode::OK, "The Product head.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

fn register_cancel_retire_door(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/products/{id}/retire/cancel")
        .operation_id("bss_products.cancel_product_retirement")
        .summary("Cancel a live Product retirement")
        .description(
            "Accepts 02's `GovernedLiveOp` envelope. The write lands at approval. \
             Success is 202 with no body. Supersedes the live retire intent on the \
             Product and every child leg, and clears `replaced_by_sku_id` on those \
             legs. Gates on `product x write`. Requires `If-Match`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product whose retirement to cancel.")
        .json_request::<CancelRetirementRequest>(openapi, "The live-op envelope.")
        .handler(cancel_product_retirement)
        .no_content_response(StatusCode::ACCEPTED, "Accepted; no body.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Operator resume of a deferred cascade (`dod-deferred-intent`, P-D-114
/// row 11). Writes `resolution = children_cleared`.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct ResumeRetirementRequest {
    /// Confirmation that the listed children have cleared.
    pub confirmed: bool,
}

fn register_resume_retire_door(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/bss-products/v1/products/{id}/retire/resume")
        .operation_id("bss_products.resume_product_retirement")
        .summary("Resume a deferred Product retirement")
        .description(
            "Writes `resolution = children_cleared` on the unresolved \
             deferred-retirement row once every child is `retired` or \
             `discarded`. Gates on `product x write`. Requires `If-Match`.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("id", "The Product whose deferred cascade to resume.")
        .json_request::<ResumeRetirementRequest>(openapi, "Resume confirmation.")
        .handler(resume_product_retirement)
        .json_response_with_schema::<ProductView>(openapi, StatusCode::OK, "The Product head.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// `GET /products/{id}`.
///
/// See this module's doc for the authorization scope, the miss/hit split and
/// why the miss carries no registry code.
async fn get_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();

    // The scope handed to the repository below is exactly this one — never
    // one rebuilt from `tenant_id` — so the read stays under the SQL-level
    // filter the PDP actually granted.
    let scope = crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRODUCT,
        crate::authz::actions::READ,
        /* owner_tenant_id */ None,
        /* resource_id */ Some(product_id),
        /* require_constraints */ true,
    )
    .await
    .map_err(|e| {
        authz_error_to_canonical(e, |reason| {
            ProductResource::permission_denied()
                .with_reason(reason)
                .create()
        })
    })?;

    let conn = state
        .db
        .conn()
        .map_err(|e| CanonicalError::internal(format!("bss-products: db conn: {e}")).create())?;

    let record = repo::find_product(&conn, &scope, tenant_id, product_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| product_not_found(product_id))?;

    let tag = preconditions::etag(InternalRevision::new(record.internal_revision));
    Ok(([(ETAG, tag)], axum::Json(ProductView::from(record))).into_response())
}

/// The `404` a miss (absent OR out-of-scope, indistinguishably) answers
/// with. Bare on purpose — see this module's doc's "The miss" section.
fn product_not_found(product_id: Uuid) -> CanonicalError {
    ProductResource::not_found("no Product matches this id in the caller's scope")
        .with_resource(product_id.to_string())
        .create()
}

/// `POST /bss-products/v1/products` request body.
///
/// Carries an **explicit, optional `id` field** (`dod-create-doors`: "minting
/// the entity id server-side and refusing a caller-supplied id as
/// `VALIDATION`"). Two shapes could have satisfied that clause: give the
/// type nowhere to put an `id` at all, so one silently never reaches
/// persistence, or accept an optional `id` and refuse the request with
/// `VALIDATION` when one arrives. An earlier revision of this door took the
/// first, field-less reading; measured against the `DoD`'s own words it fell
/// short, because "refusing a caller-supplied id as `VALIDATION`" names a
/// response the caller receives, not merely a property of what lands in
/// storage — a field-less DTO makes an `id` key **silently ignored**
/// (`#[toolkit_macros::api_dto(request)]`'s expansion adds no `#[serde(
/// deny_unknown_fields)]`, matching every other request DTO on this surface),
/// which answers `201`, never the named refusal the `DoD` calls for. This DTO
/// takes the second, explicit-field reading instead: [`create_product`]
/// refuses a present `id` as `VALIDATION` naming the field, which is the
/// property the `DoD` actually asks for. The cost this reading pays that the
/// field-less one did not is a runtime branch a later edit could forget to
/// keep wired — this door's own test suite is what re-proves it stays
/// refused. Deliberately **not** `#[serde(deny_unknown_fields)]`: that would
/// refuse every unrecognized key on this DTO, not only `id`, changing the
/// wire contract for every other field far beyond this one rule.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CreateProductRequest {
    /// Must be absent. Present only so a caller-supplied value can be
    /// refused `VALIDATION` by name rather than silently dropped — see this
    /// type's own doc for why the explicit-field reading was chosen over the
    /// field-less one. The entity id is always server-minted
    /// (`dod-create-doors`).
    pub id: Option<Uuid>,
    /// Required. Validated for presence only in this slice — see this
    /// module's doc, "`brand_id` claims this door cannot check", for why the
    /// caller's-brand-claims half of `dod-create-doors`/P-D-33 is not built
    /// here.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored. `name_normalized` is derived
    /// from it (`crate::domain::name::normalize`), never accepted from the
    /// caller.
    pub name: String,
    /// The optional external mapping code, reserved by the insert itself
    /// (`dod-code-reservation`). Trimmed, and a blank-after-trim value
    /// collapses to `None` before the insert (F-2 fix): `uq_products_product_code`
    /// is partial `WHERE product_code IS NOT NULL`, so an un-trimmed `""` is a
    /// real, reservable value that two unrelated callers sending an unset
    /// optional field as `""` would otherwise collide on.
    pub product_code: Option<String>,
    /// The region value set, written from the payload when present; empty
    /// (unrestricted) when omitted.
    pub region_scope: Option<String>,
    /// The brand value set, written from the payload when present; empty
    /// (unrestricted) when omitted.
    pub brand_scope: Option<String>,
}

/// The digest of one parsed create request, as the claim is taken against
/// (`crate::domain::idempotency`, **P-D-34**).
///
/// The operand is the **fields this request carries**, each rendered from
/// the parsed value rather than from the received bytes: an omitted optional
/// field is left out of the object entirely rather than rendered `null`, so
/// a client that omits `product_code` and one that clears it are only the
/// same request to the extent this `DTO` can tell them apart — which it
/// cannot, since `Option<String>` collapses an absent key and an explicit
/// `null` into one `None` (`CreateSkuRequest`'s own doc names the same
/// three-state reading from the other side). Telling those two apart is owed
/// to whichever door first needs the distinction on the wire, and it costs a
/// `DTO` change, not a change here.
///
/// The values are rendered as the domain rendering takes them: a `Uuid` as
/// its canonical hyphenated string, a `String` verbatim — deliberately
/// **before** this door's own trimming and `name` normalization, because the
/// hash's subject is the request the caller sent, not the row this door
/// derives from it.
///
/// Nothing about the transport enters the object: no header, no
/// correlation id, no precondition (**P-D-34**; the `If-Match` exclusion is
/// structural here, since this function is never handed the headers).
fn payload_digest(request: &CreateProductRequest) -> Vec<u8> {
    let mut fields = JsonMap::new();
    if let Some(id) = request.id {
        fields.insert("id".to_owned(), JsonValue::String(id.to_string()));
    }
    fields.insert(
        "brand_id".to_owned(),
        JsonValue::String(request.brand_id.to_string()),
    );
    fields.insert("name".to_owned(), JsonValue::String(request.name.clone()));
    if let Some(code) = request.product_code.clone() {
        fields.insert("product_code".to_owned(), JsonValue::String(code));
    }
    if let Some(region) = request.region_scope.clone() {
        fields.insert("region_scope".to_owned(), JsonValue::String(region));
    }
    if let Some(brand) = request.brand_scope.clone() {
        fields.insert("brand_scope".to_owned(), JsonValue::String(brand));
    }
    idempotency::payload_digest(&JsonValue::Object(fields))
}

/// The door-facing face of [`crate::infra::create::insert_product_with_event`]
/// — the shared create transaction lives in infra so the batch worker calls
/// it without importing this layer; this wrapper supplies the door's own
/// state and the [`ProductView`] rendering the stored idempotency answer and
/// the `201` body are both built from.
pub(crate) async fn insert_product_with_event(
    state: &ApiState,
    scope: AccessScope,
    new: NewProduct,
    claim: Option<IdempotencyClaimInput>,
    actor_ref: Uuid,
) -> Result<CreateOutcome, DbError> {
    crate::infra::create::insert_product_with_event(
        &state.db,
        &state.sink,
        scope,
        new,
        crate::infra::create::JoinedRecords { claim, stamp: None },
        actor_ref,
        render_created_product,
    )
    .await
}

/// The created Product as its `201` answers it — the one rendering both the
/// response and the stored idempotency answer are built from.
fn render_created_product(
    record: repo::ProductRecord,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(ProductView::from(record))
}

/// `POST /products`: mint a Product head as a `draft`.
///
/// See this module's doc, "The create door", for what this writes and what
/// it deliberately does not, and "`brand_id` claims this door cannot check"
/// for the one clause of `dod-create-doors` this slice discharges only
/// partially.
///
/// # Every refusal is audited, on its own runner
///
/// `dod-audit-trail` (`design/01-foundation.md` §4.4, "What the table holds")
/// requires an append-only audit row for **every** refusal a door raises, not
/// only the code-reservation conflicts this door happened to audit first.
/// Every branch below that can return `Err` before the mutation commits —
/// the authorization denial, both shape `VALIDATION`s, and the two
/// code-reservation conflicts — goes through `crate::api::rest::
/// audit_refusal_and_report`, which writes the row on its own transaction
/// and only then answers the refusal, honouring `repo::write_refusal_audit`'s
/// own contract: an unwritable row answers `AUDIT_UNAVAILABLE` instead,
/// never the refusal that would otherwise have been reported.
/// `AUDIT_UNAVAILABLE` itself gets no row of its own (P-D-34; it *is* the row
/// that could not be written), so it alone bypasses this helper.
///
/// Every refusal here is raised **before** the entity is minted, so each one
/// names its subject with `RefusalSubject::Attempted`, carrying `name` —
/// never `RefusalSubject::Minted`, which would require a `subject_id` that,
/// this early, identifies nothing (`design/01-foundation.md` §4.4's three
/// notes on the roster).
///
/// # Order of operations, and why
///
/// 1. The request is destructured up front — before anything that can
///    refuse runs — so `trimmed_name` exists for every refusal below to
///    audit against, the authorization denial included.
/// 2. [`repo::resolve_actor_ref`] (via `crate::api::rest::
///    resolve_creator_actor_ref`), on its own transaction — **before** the
///    authorization gate. `repo::resolve_actor_ref`'s own doc states the
///    reason this door does not reorder: a refusal below rolls this door's
///    own mutation transaction back while the refusal's audit row commits
///    independently and requires an `actor_ref` to attribute to, so the ref
///    must already exist before anything that can refuse runs — and an
///    authorization deny is exactly such a refusal.
/// 3. The `product x write` gate (`crate::authz::access_scope`), anchored to
///    the caller's own tenant. A denial is audited under the caller's own
///    tenant-scoped self access (`crate::api::rest::
///    audit_refusal_and_report`'s own doc explains why no write scope exists
///    yet to reuse).
/// 4. The idempotency phase (`dod-idempotency-store`): read
///    `Idempotency-Key` off the headers and digest the parsed body. §2's own
///    step list puts this in step 2, *with* the authorization gate and after
///    the `actor_ref` resolution, which is why it sits here and not ahead of
///    step 2 — see `crate::api::rest`'s module doc, "The idempotency phase",
///    for the reading of `dod-idempotency-store`'s "first pipeline phase"
///    against §2's step list. **A request with no header skips the phase**
///    (P-D-34); a header present but unusable is `VALIDATION`, audited like
///    every other shape refusal. The claim `INSERT` itself is not made here:
///    it joins the mutation's transaction in step 6 (P-D-42).
/// 5. Shape validation: `name` non-blank, `brand_id` non-nil, `id`
///    absent (`dod-create-doors`'s server-minted-id clause, F-6), and both
///    scope columns parsed through
///    [`crate::domain::containment::ResolvedScope::parse`] so an empty token
///    cannot be stored. Every violation is collected into one report rather
///    than answered at the first (P-D-37). Audited under the gate's own
///    scope.
/// 6. The mutation: the claim, [`repo::insert_product`],
///    `crate::infra::events`'s `ProductCreated` enqueue and the answer
///    written back into the claim, in one transaction
///    (`dod-create-doors`, P-D-42, `inst-fd-idem-claim-write`).
/// 7. On a unique-index collision, [`classify_insert_conflict`] and
///    [`refuse_insert_conflict`]; on an idempotency verdict, a replay served
///    from the stored answer or a refusal audited through
///    `crate::api::rest::audit_refusal_and_report` — never a fourth path.
async fn create_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Json(body): Json<CreateProductRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());

    // The digest is taken from the parsed request before it is destructured,
    // so the operand is what the caller sent rather than what the steps
    // below derive from it (`payload_digest`'s own doc).
    let payload_hash = payload_digest(&body);

    // -- 1. Destructure up front: every refusal below, including the
    // authorization denial, audits against `trimmed_name`. --
    let CreateProductRequest {
        id: caller_supplied_id,
        brand_id,
        name: raw_name,
        product_code: raw_product_code,
        region_scope,
        brand_scope,
    } = body;
    let trimmed_name = raw_name.trim().to_owned();

    // -- 2. actor_ref resolution: its own transaction, ahead of the gate. --
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;

    // -- 3. The authorization gate. --
    let scope = match crate::authz::access_scope(
        &enforcer,
        &ctx,
        &crate::authz::resource_types::PRODUCT,
        crate::authz::actions::WRITE,
        /* owner_tenant_id */ Some(tenant_id),
        /* resource_id */ None,
        /* require_constraints */ true,
    )
    .await
    {
        Ok(scope) => scope,
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            return Err(crate::api::rest::audit_refusal_and_report(
                &state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::PRODUCT,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(trimmed_name.clone()),
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await);
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            return Err(authz_error_to_canonical(err, |reason| {
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }));
        }
    };

    // -- 4. The idempotency phase: the key, and the digest taken above. An
    // absent header is the skip (P-D-34), not a refusal; a present but
    // unusable one is `VALIDATION`, audited under the gate's own scope like
    // every other shape refusal. --
    let client_key = match idempotency_key(&headers) {
        Ok(key) => key,
        Err(domain_err) => {
            return Err(crate::api::rest::audit_refusal_and_report(
                &state,
                &scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::PRODUCT,
                    error_code: domain_err.code(),
                },
                RefusalSubject::Attempted(trimmed_name.clone()),
                CanonicalError::from(domain_err),
            )
            .await);
        }
    };
    let claim = client_key.map(|key| {
        IdempotencyClaimInput::new(
            CREATE_ENDPOINT,
            key,
            payload_hash,
            now,
            state.idempotency_retention_hours,
        )
    });

    // -- 5. Shape validation. --
    let mut report = ValidationReport::new();
    if trimmed_name.is_empty() {
        report.violate("VALIDATION", "name", "name must not be blank");
    }
    if brand_id.is_nil() {
        report.violate("VALIDATION", "brand_id", "brand_id is required");
    }
    if caller_supplied_id.is_some() {
        report.violate(
            "VALIDATION",
            "id",
            "id is server-minted and must not be supplied",
        );
    }
    // Both scope columns are **parsed** and not merely carried through, which
    // is `create_sku`'s own rule (`scope_input_from_payload`) reaching this
    // door: a value carrying an empty token -- `","`, `"eu,,us"` -- is
    // refused rather than stored verbatim. The refusal is load-bearing beyond
    // this row, and that is why it belongs at the *create* door and not only
    // at the save door: no `CHECK` constrains either column
    // (`m20260829_000002` declares them `text NOT NULL DEFAULT ''` with no
    // predicate), while `skus::recheck_parent_containment` and
    // `create_sku` both parse the **parent Product's** stored scope and
    // answer a `500` where it does not parse. A create admitted here would
    // therefore let a caller plant a poison value with a `201` and detonate
    // it on a different entity's door as an operator alarm -- the provenance
    // inversion `RepoError::CorruptRow`'s own doc rules out. The violations
    // fold into the report above rather than returning early so a body wrong
    // in two places reports both (P-D-37). The parsed value is discarded and
    // the raw string stored, the column holding the caller's own spelling.
    for (wire, raw) in [
        ("region_scope", region_scope.as_deref()),
        ("brand_scope", brand_scope.as_deref()),
    ] {
        if raw.is_some_and(|value| ResolvedScope::parse(value).is_err()) {
            report.violate(
                "VALIDATION",
                wire,
                format!("{wire} contains an empty value between separators"),
            );
        }
    }
    if !report.is_empty() {
        let domain_err = DomainError::Validation(report);
        return Err(crate::api::rest::audit_refusal_and_report(
            &state,
            &scope,
            crate::api::rest::RefusalAuditContext {
                tenant_id,
                actor_ref,
                subject_kind: crate::authz::labels::PRODUCT,
                error_code: domain_err.code(),
            },
            RefusalSubject::Attempted(trimmed_name.clone()),
            CanonicalError::from(domain_err),
        )
        .await);
    }

    // `product_code` is trimmed and a blank-after-trim value collapses to
    // `None` (F-2): `uq_products_product_code` is partial `WHERE
    // product_code IS NOT NULL`, so an un-trimmed empty string is a real,
    // reservable value, and two unrelated creates that both send `""` (the
    // value many clients emit for an unset optional text field) would
    // otherwise collide on it.
    let product_code = raw_product_code
        .map(|code| code.trim().to_owned())
        .filter(|code| !code.is_empty());

    // Kept for the conflict path below, which needs it back if the insert
    // this door is about to attempt loses the code-reservation race.
    let attempted_code = product_code.clone();
    let new = NewProduct {
        product_id: Uuid::new_v4(),
        tenant_id,
        brand_id,
        name: trimmed_name.clone(),
        name_normalized: name::normalize(&trimmed_name),
        product_code,
        region_scope: region_scope.unwrap_or_default(),
        brand_scope: brand_scope.unwrap_or_default(),
        created_by: actor_ref.to_string(),
        // `now` arrives already truncated to microseconds — the handler
        // stamps it through `canonical::write_instant` (P-D-82), which is
        // what closed the cross-engine digest hazard this comment used to
        // carry as a debt: neither engine now holds a fractional digit the
        // other could round differently.
        created_at: now,
        // An ordinary create has no lineage; the clone door is the pair's
        // only writer (P-D-76).
        cloned_from: None,
        cloned_from_version: None,
    };

    // -- 6. The mutation: the idempotency claim, the entity row, its
    // creation outbox row and the answer written back into the claim, one
    // transaction, nothing else written. --
    let insert_outcome =
        insert_product_with_event(&state, scope.clone(), new, claim, actor_ref).await;

    match insert_outcome {
        Ok(CreateOutcome::Created {
            internal_revision,
            body,
        }) => {
            let tag = preconditions::etag(InternalRevision::new(internal_revision));
            // `body` is the value rendered inside the mutation transaction
            // and, for a keyed create, stored there as the idempotency
            // answer. Answering it rather than re-rendering the view is what
            // makes a later replay reproduce this response and not a
            // lookalike.
            Ok((CREATE_RESPONSE_STATUS, [(ETAG, tag)], Json(body)).into_response())
        }
        // A replay executes nothing and audits nothing: it is not a refusal,
        // and the act it reproduces was audited (or, being a success,
        // deliberately not — P-D-21) when it originally ran.
        Ok(CreateOutcome::Replay { status, body }) => Ok(replay_response(status, body)),
        // An idempotency refusal is a refusal like any other: the audit row
        // on its own runner first, then the answer — or `AUDIT_UNAVAILABLE`
        // over it if that row cannot be written.
        Ok(CreateOutcome::Refused(domain_err)) => Err(crate::api::rest::audit_refusal_and_report(
            &state,
            &scope,
            crate::api::rest::RefusalAuditContext {
                tenant_id,
                actor_ref,
                subject_kind: crate::authz::labels::PRODUCT,
                error_code: domain_err.code(),
            },
            RefusalSubject::Attempted(trimmed_name),
            CanonicalError::from(domain_err),
        )
        .await),
        Err(db_error) => {
            let message = db_error.to_string();
            match classify_insert_conflict(&message) {
                Some(conflict) => Err(refuse_insert_conflict(
                    &state,
                    &scope,
                    tenant_id,
                    actor_ref,
                    conflict,
                    &trimmed_name,
                    attempted_code.as_deref(),
                )
                .await),
                None => Err(repo_error_to_canonical(&RepoError::Db(message))),
            }
        }
    }
}

/// Which unique index [`classify_insert_conflict`] read an insert failure's
/// driver text as having violated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertConflict {
    /// `uq_products_product_name` — `(tenant_id, brand_id,
    /// name_normalized)`.
    DuplicateName,
    /// `uq_products_product_code` — `(tenant_id, product_code)`.
    DuplicateCode,
}

/// Tell a duplicate name from a duplicate code from an unrelated storage
/// failure, off the driver text an insert failure already carries.
///
/// See this module's doc, "Telling `DUPLICATE_NAME` from `DUPLICATE_CODE`
/// from an unrelated storage failure", for why this is a substring match
/// over driver text rather than a typed answer, and what that costs.
/// `product_code` is checked first because Postgres's own constraint name
/// (`uq_products_product_code`) and `SQLite`'s column-list wording both
/// contain it literally, and it cannot appear in a message the name index
/// produced on either backend.
///
/// **Two disjuncts, not three.** A third one testing for
/// `unique constraint failed` — `SQLite`'s exact opening words — stood here
/// beside `unique constraint` and could never add a match: it is a strict
/// superstring of its neighbour, so every message it would have accepted the
/// neighbour had accepted a term earlier. Deleting it changes no answer this
/// function gives on either backend; it removes a disjunct a reader would
/// otherwise take for a case being covered.
fn classify_insert_conflict(message: &str) -> Option<InsertConflict> {
    let lower = message.to_ascii_lowercase();
    let looks_like_a_unique_violation =
        lower.contains("unique constraint") || lower.contains("duplicate key");
    if !looks_like_a_unique_violation {
        return None;
    }
    if lower.contains("product_code") {
        Some(InsertConflict::DuplicateCode)
    } else if lower.contains("name_normalized") || lower.contains("uq_products_product_name") {
        Some(InsertConflict::DuplicateName)
    } else {
        None
    }
}

/// Refuse an insert conflict: write its audit row on a transaction of its
/// own, then answer the domain refusal — or, if the audit row could not be
/// written, `AUDIT_UNAVAILABLE` instead, never the domain refusal
/// (`crate::api::rest::audit_refusal_and_report`'s own contract;
/// `dod-code-reservation`: "refusing the loser of a concurrent race ... with
/// an audited reason").
///
/// `scope` is the caller's own compiled write scope (from the authorization
/// gate this door already ran) — the refusal audit row is written under the
/// same tenant-scoped access the mutation itself was authorized under, not a
/// fresh, broader one.
async fn refuse_insert_conflict(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    conflict: InsertConflict,
    holder_name: &str,
    holder_code: Option<&str>,
) -> CanonicalError {
    let (error_code, refusal_subject_key, domain_err) = match conflict {
        InsertConflict::DuplicateName => (
            "DUPLICATE_NAME",
            holder_name.to_owned(),
            DomainError::DuplicateName(format!(
                "a Product named \"{holder_name}\" already exists for this tenant and brand"
            )),
        ),
        InsertConflict::DuplicateCode => {
            let code = holder_code.unwrap_or_default().to_owned();
            let domain_err = DomainError::DuplicateCode(format!(
                "product_code \"{code}\" is already reserved for this tenant"
            ));
            ("DUPLICATE_CODE", code, domain_err)
        }
    };

    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::PRODUCT,
            error_code,
        },
        RefusalSubject::Attempted(refusal_subject_key),
        CanonicalError::from(domain_err),
    )
    .await
}

/// The status a publish and a discard answer on success, and therefore the
/// status a replay of either reproduces (§2's own door table: both answer
/// **200**, where a create answers **201**).
///
/// One spelling, read by the response each door builds **and** by the answer
/// it stores, for [`CREATE_RESPONSE_STATUS`]'s reason: a stored status that
/// was not the status answered would make every later replay a different
/// response from the original.
const HEAD_ACT_RESPONSE_STATUS: StatusCode = StatusCode::OK;

/// The **complete named field set** a frozen Product version row's content
/// is rendered against (`canonical::Absence::Null`, §4.3, **P-D-24**,
/// **P-D-35**).
///
/// Named here, once, in code, because §4.3 makes the roster load-bearing in
/// two directions. A field the roster names and the value omits is rendered
/// `null` rather than dropped, which is the clause that keeps absence and
/// the empty string apart; and *"adding a column to a frozen row's content
/// is a digest-version bump, not a silent change"*, which is only checkable
/// if there is one place that says what the content was. A later slice
/// adding a content column adds it here and bumps
/// [`canonical::DIGEST_VERSION`] with it — `products_tests::
/// the_product_content_roster_is_the_head_table_minus_the_excluded_columns`
/// is what fails if it forgets.
///
/// # The rule this is derived from
///
/// §4.3 scopes the frozen content as **the publish-time entity**, excluding
/// the metadata map and excluding `lifecycle_state`,
/// `deprecation_provenance`, `replaced_by_sku_id` and `internal_revision`
/// (**P-D-24**, extended by **P-D-35**). So the roster is
/// `products_product`'s own column list minus those, and **not** a hand-made
/// selection: a column of the head that §4.3 does not exclude is content.
///
/// **What is in it**: `brand_id`, `brand_scope`, `created_at`, `created_by`,
/// `name`, `name_normalized`, `product_code`, `product_id`, `region_scope`,
/// `tenant_id`.
///
/// **What is excluded, and why each:**
///
/// - `lifecycle_state` and `internal_revision` — §4.3's own, verbatim
///   (**P-D-24**, **P-D-35**).
/// - `deprecation_provenance` and `replaced_by_sku_id` — §4.3's other two,
///   which have **no column on this table at this commit**
///   (`deprecation_provenance` is slice 04's, `replaced_by_sku_id` is a SKU
///   column), so their exclusion here is structural as well as stated.
/// - The metadata map — slice 02's `products_metadata` (**P-D-06**): it
///   lives beside the entity and is captured only by `CatalogVersion`
///   snapshots. Not a column either.
/// - `updated_at` and `published_version` — **neither is named by §4.3's
///   enumeration, and both are excluded anyway**. §4.3 lists four columns
///   plus the metadata map and neither of these two is on that list, so each
///   is a reading this code states and argues. The two sections below are
///   those arguments, and the section after them is what the pair buys
///   together.
///
/// # `updated_at` is excluded by P-D-35's own criterion
///
/// This is an **application of a stated criterion to a column the
/// enumeration does not list**, not a new rule this code invented. §4.3
/// gives P-D-35's criterion in words: those columns *"move on transitions,
/// which write no version row, so freezing them would need the digest to
/// change on a write that produces no row to digest"* — and it adds
/// `internal_revision` on exactly that ground, noting it *"was left out of
/// the original enumeration"*. `updated_at` is *"a column that moves on a
/// transition writing no version row"* in the criterion's own words: it
/// moves on every transition and every save. It was left out of the
/// enumeration the same way `internal_revision` was, and §5 corroborates
/// from a second direction — it already counts the update timestamp among
/// the mechanical columns that sit outside the bucket comparison.
///
/// Nothing is lost by leaving it out: the instant of the write that produced
/// this version is already on the version row, as `published_at`.
///
/// # `published_version` is excluded because it is the row's own key
///
/// `products_entity_version` is keyed by `(tenant_id, entity_kind,
/// entity_id, published_version)`. Rendering `published_version` into the
/// content therefore writes the key **inside the payload it keys** — the row
/// says which version it is twice, once where a reader looks and once where
/// the digest is taken. That duplication is not merely redundant; it is what
/// makes the digest move on a publish whose content did not change, since
/// the version number moves by construction on every publish.
///
/// The column is also the one place in this door where the pre-act and
/// post-act head images differ on a roster field, and an earlier revision of
/// this file paid for that with a second argument to [`product_content`] and
/// a shared expression in [`freeze_for`] to keep the key and the content
/// agreeing. With the column off the roster the hazard is **structurally
/// absent** rather than handled: the content is a function of the head alone
/// and there is no second number that could disagree with the key.
///
/// # What the two exclusions buy together
///
/// *The same content produces the same digest.* That is the property that
/// lets a reader answer *"did the content change between version N and
/// N+1"* by comparing two rows' digests — the question §2's
/// `inst-fd-publish-reannounce` raises when it contemplates re-announcing
/// unchanged content, the question slice 06's `CatalogVersion` is built on,
/// and the one slice 10's restore drill asks of a pair of rows.
///
/// It is a property of **both** exclusions and of neither alone, which is
/// worth saying plainly because an earlier revision of this doc claimed it
/// for the `updated_at` exclusion by itself. That claim was measurably
/// false while `published_version` was still on the roster: the version
/// number moves on every publish, so the digest moved on every publish
/// whether or not a single content field had changed, and excluding
/// `updated_at` bought nothing on its own.
///
/// **The design set's §4.3 enumeration is owed both additions.** It should
/// name `updated_at` and `published_version` beside `internal_revision` as
/// columns the original enumeration missed; until it does, this doc is where
/// the reading is recorded, and `skus::SKU_VERSION_CONTENT_ROSTER`'s twin
/// says the same.
const PRODUCT_CONTENT_ROSTER: [&str; 12] = [
    "brand_id",
    "brand_scope",
    "cloned_from",
    "cloned_from_version",
    "created_at",
    "created_by",
    "name",
    "name_normalized",
    "product_code",
    "product_id",
    "region_scope",
    "tenant_id",
];

/// The concrete resource path a publish claims its idempotency key under
/// (**P-D-42**: *"`endpoint` MUST be the concrete resource path, not the
/// route template"*).
///
/// A function rather than a constant because the path **carries the id**, and
/// that is exactly the requirement: `/bss-products/v1/products/{that id}/publish`,
/// never the `{id}` template [`router`] registers. Two publishes of two
/// different Products under one client key are then two keys, which is what
/// makes the key safe for a client that reuses one across a batch.
fn publish_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/publish")
}

/// [`publish_endpoint`]'s discard twin, on the same terms.
fn discard_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/discard")
}

/// [`publish_endpoint`]'s deprecate twin, on the same terms.
fn deprecate_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/deprecate")
}

fn undeprecate_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/undeprecate")
}

fn retire_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/retire")
}

fn cancel_retire_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/retire/cancel")
}

/// The idempotency digest of a **bodiless** head act (`crate::domain::
/// idempotency`, **P-D-34**).
///
/// P-D-34 takes the hash over *"the canonical rendering of the parsed
/// request, excluding the precondition header"*. A publish and a discard
/// carry no request body at all, so the parsed request's named field set is
/// **empty** and every such request under this door renders identically —
/// the digest is a constant, and deliberately so. Nothing is lost by that:
/// the two operands that distinguish one act from another are already in the
/// key, the entity id through the concrete endpoint ([`publish_endpoint`])
/// and the caller's own key beside it, and the `If-Match` revision is the one
/// operand P-D-34 explicitly keeps out. A door that folded the precondition
/// in would refuse a client's own retry as `IDEMPOTENCY_CONFLICT` the moment
/// a neighbour's write moved the head, which is the opposite of what the
/// store is for.
fn bodiless_payload_digest() -> Vec<u8> {
    idempotency::payload_digest(&JsonValue::Object(JsonMap::new()))
}

/// The frozen content of one Product head, as the object
/// [`PRODUCT_CONTENT_ROSTER`] is rendered against.
///
/// # The pre-act/post-act hazard is structurally absent here
///
/// `inst-fd-publish-freeze` and **P-D-33** take the freeze over the image
/// the act **leaves behind**, while `record` is the head as it stood
/// **before** the write — so on the face of it this function has to be told
/// which fields the act moved. It does not, and the reason is the roster:
/// every column a publish moves is excluded from it. `published_version`,
/// `internal_revision`, `lifecycle_state` and `updated_at` are the four
/// columns the act writes, and none of them is content
/// ([`PRODUCT_CONTENT_ROSTER`]'s own doc argues each). On the roster's own
/// fields the two images are **equal**, so the pre-act head renders exactly
/// what the post-act head would.
///
/// This function took `published_version` as a second argument until
/// `published_version` left the roster, precisely so the content would carry
/// the post-act `N + 1` that [`freeze_for`] keys the row at. That argument
/// had no reader once the column was excluded, and with it went the last
/// place where a pre-act value could have been frozen under a post-act key.
/// The hazard is now absent by construction rather than handled by a
/// convention two call sites had to keep.
///
/// It comes back the moment a **content** column starts moving in the act.
/// Slice 07's `CorrectionDoor` is that case: it supplies a corrected
/// bucket-ii value, and that value must be applied to `record` **before**
/// this function renders it, never to the head-row `UPDATE` alone, or the
/// freeze would store content the act never produced.
///
/// `product_code` is **omitted** from the map when the head carries none,
/// rather than inserted as `JsonValue::Null`. That is not a shortcut: it is
/// what exercises `canonical::Absence::Null`'s own clause — the roster names
/// the field, so the rendering writes `null` for it — and a door that
/// pre-filled the nulls itself would make the roster a no-op precisely in
/// the case it exists for.
///
/// `pub(crate)` since 2026-09-04: `05`'s submit door stores the head's
/// content as the record's `content_snapshot`, and that snapshot must be the
/// **same rendering** the publish door will freeze — a second renderer would
/// let an approver sign bytes the publish never produces.
pub(crate) fn product_content(record: &ProductRecord) -> JsonValue {
    let mut content = JsonMap::new();
    content.insert(
        "brand_id".to_owned(),
        JsonValue::String(record.brand_id.to_string()),
    );
    content.insert(
        "brand_scope".to_owned(),
        JsonValue::String(record.brand_scope.clone()),
    );
    if let Some(source) = record.cloned_from {
        // Lineage joins the roster by its own membership rule — publish does
        // not move it (P-D-76). Omit-when-absent exercises `Absence::Null`,
        // the `product_code` precedent.
        content.insert(
            "cloned_from".to_owned(),
            JsonValue::String(source.to_string()),
        );
    }
    if let Some(version) = record.cloned_from_version {
        content.insert(
            "cloned_from_version".to_owned(),
            JsonValue::Number(version.into()),
        );
    }
    content.insert(
        "created_at".to_owned(),
        JsonValue::String(canonical::render_instant(record.created_at)),
    );
    content.insert(
        "created_by".to_owned(),
        JsonValue::String(record.created_by.clone()),
    );
    content.insert("name".to_owned(), JsonValue::String(record.name.clone()));
    content.insert(
        "name_normalized".to_owned(),
        JsonValue::String(record.name_normalized.clone()),
    );
    if let Some(code) = record.product_code.clone() {
        content.insert("product_code".to_owned(), JsonValue::String(code));
    }
    content.insert(
        "product_id".to_owned(),
        JsonValue::String(record.product_id.to_string()),
    );
    content.insert(
        "region_scope".to_owned(),
        JsonValue::String(record.region_scope.clone()),
    );
    content.insert(
        "tenant_id".to_owned(),
        JsonValue::String(record.tenant_id.to_string()),
    );
    JsonValue::Object(content)
}

/// Which authorization grant and which audit `action` token a head act runs
/// under, and the concrete endpoint it claims its key at — the three things
/// [`open_head_door`] needs that differ between publish and discard.
///
/// Grouped rather than passed as three loose arguments for
/// `RefusalAuditContext`'s own reason: they always travel together, and two
/// of the three are `&'static str`s that a call site could transpose without
/// the compiler noticing.
struct HeadAct {
    /// `crate::authz::actions::PUBLISH` for the publish door, `::WRITE` for
    /// the discard and save doors — see [`publish_product`] and
    /// [`discard_product`] for why the discard door gates on `write` rather
    /// than on a `discard` action of its own. The two vocabularies are not
    /// one-to-one: three acts share two grants, which is why the audit token
    /// beside this is a field of its own.
    authz_action: &'static str,
    /// The `products_audit_log.action` token every refusal of this act is
    /// recorded under: `publish`, `discard` or `save`
    /// ([`PUBLISH_AUDIT_ACTION`], [`DISCARD_AUDIT_ACTION`],
    /// [`SAVE_AUDIT_ACTION`]).
    audit_action: &'static str,
    /// The concrete resource path this act's idempotency key is claimed at
    /// ([`publish_endpoint`], [`discard_endpoint`], [`save_endpoint`]).
    endpoint: String,
    /// The digest the claim is taken against
    /// (`crate::domain::idempotency`, **P-D-34**): the canonical rendering of
    /// the **parsed request**.
    ///
    /// A field rather than a call [`open_head_door`] makes for itself, and the
    /// save door is why. A publish and a discard carry no request body at all,
    /// so their digest is the constant [`bodiless_payload_digest`]; a save
    /// carries one, and hashing the constant for it would make two different
    /// saves of one head under one client key collide as replays of each
    /// other — the store answering the second with the first's body, for a
    /// change it never made. The digest is therefore the act's, like the
    /// endpoint beside it, and neither is derivable from the other.
    payload_digest: Vec<u8>,
}

/// Everything the phases before the mutation established, handed on to the
/// act itself.
///
/// A value rather than a tuple because five of its seven fields are the
/// operands a refusal below it audits with, and a caller that had to unpack
/// them positionally would eventually pass the wrong `Uuid`.
struct OpenedHeadDoor {
    /// The caller's own tenant.
    tenant_id: Uuid,
    /// The pseudonymous ref every audit row of this act attributes to.
    actor_ref: Uuid,
    /// The scope the authorization gate compiled — the one every read and
    /// every write below runs under, never one rebuilt from `tenant_id`.
    scope: AccessScope,
    /// The head as it stood when the door opened. The mutation re-decides
    /// everything this record was read for, under the write itself; this
    /// copy exists so a refusal can be *named* precisely.
    record: ProductRecord,
    /// The revision the caller pinned with `If-Match` (**P-D-33**).
    expected: InternalRevision,
    /// The claim this act will take inside its own mutation transaction, or
    /// `None` where the request carried no `Idempotency-Key` (the skip,
    /// P-D-34).
    claim: Option<IdempotencyClaimInput>,
    /// The door's own request instant, stamped once.
    now: DateTime<Utc>,
}

impl OpenedHeadDoor {
    /// The refusal subject every audit row below this point names.
    ///
    /// [`RefusalSubject::Minted`] rather than the create doors'
    /// [`RefusalSubject::Attempted`]: the subject of a publish or a discard
    /// is a row that already exists and already has an id, which is exactly
    /// the distinction §4.4's roster draws. The revision travels with it so
    /// an operator reading the trail can see which image of the head was
    /// refused.
    fn refusal_subject(&self) -> RefusalSubject {
        RefusalSubject::Minted {
            subject_id: self.record.product_id,
            subject_revision: Some(self.record.internal_revision),
        }
    }

    /// The owned operand set this act's transaction runs on — see
    /// [`HeadActInputs`] for why the transaction cannot simply borrow this
    /// value.
    fn act_inputs(&self) -> HeadActInputs {
        HeadActInputs {
            scope: self.scope.clone(),
            tenant_id: self.tenant_id,
            product_id: self.record.product_id,
            actor_ref: self.actor_ref,
            expected: self.expected.get(),
            now: self.now,
            claim: self.claim.clone(),
        }
    }
}

/// Write one refusal's audit row on its own runner and then answer it — the
/// head doors' equivalent of the create doors' direct
/// `crate::api::rest::audit_refusal_and_report` call, differing only in that
/// the `action` token recorded is this act's rather than `create`.
///
/// Every refusal either door raises goes through here, so "the row commits
/// before the refusal is reported, and an unwritable row answers
/// `AUDIT_UNAVAILABLE` instead" cannot be forgotten on a branch a later edit
/// adds.
async fn audit_and_refuse(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    audit_action: &str,
    domain_err: DomainError,
) -> CanonicalError {
    let error_code = domain_err.code();
    crate::api::rest::audit_refusal_of_action_and_report(
        state,
        &opened.scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id: opened.tenant_id,
            actor_ref: opened.actor_ref,
            subject_kind: crate::authz::labels::PRODUCT,
            error_code,
        },
        audit_action,
        opened.refusal_subject(),
        CanonicalError::from(domain_err),
    )
    .await
}

/// Who a refusal raised **before** the head has been read is recorded
/// against: the compiled scope it audits under, the caller, the id the
/// caller named, and this act's own audit `action` token.
///
/// A value rather than four loose arguments because three of the four are
/// `Uuid`s or `&str`s a call site could transpose without the compiler
/// noticing — `RefusalAuditContext`'s own reason for existing, one layer up.
struct HeadRefusalTarget<'target> {
    /// The scope the audit row is written under — the caller's own compiled
    /// scope from the authorization gate this door has already passed.
    scope: &'target AccessScope,
    /// Owning tenant.
    tenant_id: Uuid,
    /// The pseudonymous ref this refusal attributes to.
    actor_ref: Uuid,
    /// The head the caller named. Carried as an id with **no revision**: the
    /// row has not been read at this point, and a revision reported here
    /// would be one nothing measured.
    product_id: Uuid,
    /// `publish`, `discard` or `save`.
    audit_action: &'target str,
}

/// Audit one pre-read refusal and answer it — the header phases' equivalent
/// of [`audit_and_refuse`], differing only in that the subject it names
/// carries no revision.
async fn audit_head_refusal(
    state: &ApiState,
    target: &HeadRefusalTarget<'_>,
    domain_err: DomainError,
) -> CanonicalError {
    let error_code = domain_err.code();
    crate::api::rest::audit_refusal_of_action_and_report(
        state,
        target.scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id: target.tenant_id,
            actor_ref: target.actor_ref,
            subject_kind: crate::authz::labels::PRODUCT,
            error_code,
        },
        target.audit_action,
        RefusalSubject::Minted {
            subject_id: target.product_id,
            subject_revision: None,
        },
        CanonicalError::from(domain_err),
    )
    .await
}

/// The phases both head doors run before either one's own act: `actor_ref`
/// resolution, authorization, the idempotency key, the `If-Match`
/// precondition and the head read.
///
/// # The order, and why it is this order
///
/// 1. **`actor_ref` resolution**, on its own transaction, **before** the
///    authorization gate — `repo::resolve_actor_ref`'s own doc gives the
///    reason and [`create_product`] follows it identically: a refusal below
///    audits on a transaction of its own and needs a ref to attribute to, and
///    an authorization denial is such a refusal.
/// 2. **The authorization gate**, anchored to the caller's own tenant with
///    `require_constraints = true` and the resource id pinned. A denial is
///    audited under the caller's tenant-scoped self access, there being no
///    compiled write scope to reuse — the gate is what refused.
/// 3. **The idempotency phase** (`Phase::Idempotency`, the pipeline's first):
///    the key off the header, the digest of the parsed request
///    ([`bodiless_payload_digest`]). An absent header **skips** the phase
///    (P-D-34); a present but unusable one is `VALIDATION`. The claim
///    `INSERT` is not taken here — it joins the mutation's own transaction
///    (P-D-42).
/// 4. **The `If-Match` precondition** (`Phase::Precondition`, the pipeline's
///    second, which is why it follows the key rather than leading): absent is
///    `VALIDATION`, unparseable is `VALIDATION`, and a *stale* one is not
///    judged here at all — the comparison belongs under the write
///    (`preconditions`' own doc).
/// 5. **The head read**, under the compiled scope. A miss — absent, or
///    outside the caller's scope, indistinguishably — is this module's bare
///    `404` ([`product_not_found`]).
///
/// # The `404` is the one refusal that writes no audit row
///
/// Every other refusal below audits. A miss does not, and the reason is the
/// same one this module's doc gives for the read door's `404` being bare: it
/// is judged before the pipeline opens and raises no registry code at all, so
/// there is no `error_code` for `products_audit_log.error_code` to carry —
/// inventing a `NOT_FOUND` token for the column would put a code in the
/// audit trail that the error taxonomy does not define. It is also the one
/// answer that must not distinguish an absent row from another tenant's,
/// and an audit row written for one and not the other would be exactly that
/// distinction, recorded.
async fn open_head_door(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    act: &HeadAct,
) -> Result<OpenedHeadDoor, CanonicalError> {
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());

    // -- 1. actor_ref resolution: its own transaction, ahead of the gate. --
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(state, tenant_id, ctx.subject_id(), now)
            .await?;

    // -- 2. The authorization gate. --
    let scope = match crate::authz::access_scope(
        enforcer,
        ctx,
        &crate::authz::resource_types::PRODUCT,
        act.authz_action,
        /* owner_tenant_id */ Some(tenant_id),
        /* resource_id */ Some(product_id),
        /* require_constraints */ true,
    )
    .await
    {
        Ok(scope) => scope,
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            return Err(crate::api::rest::audit_refusal_of_action_and_report(
                state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: crate::authz::labels::PRODUCT,
                    error_code: "PERMISSION_DENIED",
                },
                act.audit_action,
                // The head has not been read yet — a denied caller may not
                // learn whether it exists — so the subject is named by the
                // id alone, with no revision to report.
                RefusalSubject::Minted {
                    subject_id: product_id,
                    subject_revision: None,
                },
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await);
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            return Err(authz_error_to_canonical(err, |reason| {
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }));
        }
    };

    // Both remaining header phases refuse under the compiled scope and
    // name the same subject; `target` is the shape they share.
    let target = HeadRefusalTarget {
        scope: &scope,
        tenant_id,
        actor_ref,
        product_id,
        audit_action: act.audit_action,
    };

    // -- 3. The idempotency phase: the key, and the bodiless digest. --
    let client_key = match idempotency_key(headers) {
        Ok(key) => key,
        Err(domain_err) => return Err(audit_head_refusal(state, &target, domain_err).await),
    };
    let claim = client_key.map(|key| {
        IdempotencyClaimInput::new(
            act.endpoint.clone(),
            key,
            act.payload_digest.clone(),
            now,
            state.idempotency_retention_hours,
        )
    });

    // -- 4. The `If-Match` precondition (P-D-33). --
    let expected = match preconditions::if_match(headers) {
        Ok(revision) => revision,
        Err(domain_err) => return Err(audit_head_refusal(state, &target, domain_err).await),
    };

    // -- 5. The head read, under the compiled scope. The connection is
    // released before the act's own transaction opens: a pool pinned to one
    // connection — which is what this door's own test harness runs — would
    // otherwise deadlock against itself. --
    let record = {
        let conn = state.db.conn().map_err(|e| {
            CanonicalError::internal(format!("bss-products: db conn: {e}")).create()
        })?;
        repo::find_product(&conn, &scope, tenant_id, product_id)
            .await
            .map_err(|e| repo_error_to_canonical(&e))?
            .ok_or_else(|| product_not_found(product_id))?
    };

    Ok(OpenedHeadDoor {
        tenant_id,
        actor_ref,
        scope,
        record,
        expected,
        claim,
        now,
    })
}

/// The operands one head act's transaction runs on, owned so the
/// `transaction_with_retry` closure can hold them.
///
/// A copy of five fields of [`OpenedHeadDoor`] rather than a reference to it,
/// and the retry helper is the reason: its body is
/// `for<'a> FnMut(&'a DbTx<'a>) -> Pin<Box<dyn Future + Send + 'a>>`, and the
/// higher-ranked `'a` cannot be bounded by any lifetime the caller holds — a
/// borrow of the door's state simply does not typecheck there. `Clone` for
/// the same helper's other reason: the body is `FnMut` and may be re-entered
/// on a retryable contention failure, so every attempt takes its own copy and
/// no attempt can consume what the next one needs. The values *carried here*
/// are attempt-independent by construction — `now` was stamped before the
/// first attempt, and the claim's window with it. The envelope id the act
/// eventually enqueues is not one of them; see
/// [`insert_product_with_event`] for why that is harmless.
#[derive(Clone)]
struct HeadActInputs {
    /// The compiled scope every read and write of the act runs under.
    scope: AccessScope,
    /// Owning tenant.
    tenant_id: Uuid,
    /// The head being acted on.
    product_id: Uuid,
    /// The pseudonymous ref the frozen version row attributes the publish
    /// to.
    actor_ref: Uuid,
    /// The revision the caller pinned, as the head-row filter compares it.
    expected: i64,
    /// The act's instant, stamped once before the first attempt.
    now: DateTime<Utc>,
    /// The claim to take as the transaction's first statement, or `None`
    /// where the request carried no key (P-D-34's skip).
    claim: Option<IdempotencyClaimInput>,
}

/// What one head act's transaction produced.
///
/// [`CreateOutcome`]'s two success shapes without its third: a refusal
/// decided inside the transaction is an `Err` here rather than an `Ok`
/// variant, because a publish has already **written** its frozen version row
/// by the time the head-row `UPDATE` can report `Unmatched`, and only an
/// `Err` rolls that write back. See [`HeadActError`].
enum HeadActOutcome {
    /// The act ran: the revision the `ETag` is minted from, and the response
    /// body rendered inside the transaction and stored there as the
    /// idempotency answer when the request carried a key.
    Applied {
        /// The committed `internal_revision`.
        internal_revision: i64,
        /// The response body, as answered and as stored.
        body: JsonValue,
    },
    /// A stored answer was replayed; nothing was written.
    Replay {
        /// The stored status.
        status: i32,
        /// The stored body.
        body: JsonValue,
    },
}

/// Why a head act's transaction ended without applying.
///
/// # Every variant here rolls the transaction back, and that is the point
///
/// `Db::transaction_with_retry` **commits on `Ok`**. The create doors can
/// therefore return their idempotency refusal as an `Ok` variant: the claim
/// `INSERT` is their first statement, so a refusal there has written
/// nothing and committing an empty transaction is harmless. The publish door
/// cannot: `inst-fd-publish-txn` forces the freeze **before** the head-row
/// `UPDATE`, so by the time [`repo::publish_product_head`] can answer
/// [`repo::HeadWrite::Unmatched`] a `products_entity_version` row is already
/// written on this transaction. Committing that would leave a frozen version
/// for a publish that never happened — and, worse, one the head-row guard
/// would then accept as the missing prerequisite for a later bump nobody
/// authorized. So a refusal discovered inside the transaction travels as an
/// error, and the rollback is what un-writes the freeze.
///
/// The retry classifier ([`head_act_contention_db_err`]) answers `None` for
/// every variant but [`Self::Db`], so a refusal is never mistaken for
/// contention and re-attempted.
enum HeadActError {
    /// A domain refusal decided inside the transaction: the idempotency
    /// phase's, or the head-row write's own `Unmatched` once
    /// [`classify_unmatched_publish`]/[`classify_unmatched_discard`] has read
    /// which of its several meanings applied.
    Refused(DomainError),
    /// The head vanished from the caller's scope between the door's read and
    /// its write. Answered as this module's bare `404`, unaudited, for
    /// [`open_head_door`]'s stated reason.
    Vanished,
    /// A storage failure, including one the contention classifier may decide
    /// to retry.
    Db(DbError),
}

impl From<DbError> for HeadActError {
    fn from(error: DbError) -> Self {
        Self::Db(error)
    }
}

impl HeadActError {
    /// Wrap any error whose text is all this door needs.
    ///
    /// The repository layer returns `RepoError`, the events layer returns
    /// `EventsError`, and neither can be retried by
    /// `transaction_with_retry` unless it arrives as a `DbErr`; each is a
    /// storage or infrastructure failure of this act's own mutation, so each
    /// becomes [`Self::Db`] carrying the text that named it. This mirrors
    /// what [`insert_product_with_event`] does with the same failures.
    fn from_storage(error: &impl core::fmt::Display) -> Self {
        Self::Db(DbError::Sea(DbErr::Custom(error.to_string())))
    }

    /// Wrap a repository failure, preserving the driver error inside it.
    ///
    /// The difference from [`Self::from_storage`] is the whole of a fix this
    /// door's own doc used to claim without holding: `RepoError::Driver`
    /// carries `sea-orm`'s error as the driver raised it, and
    /// [`RepoError::to_db_err`] hands that variant on unchanged, so
    /// `transaction_with_retry`'s classifier can still see an `Exec`/`Query`
    /// contention failure and re-attempt the act. Rendering the same failure
    /// through `from_storage` would flatten it to `DbErr::Custom`, which
    /// `is_retryable_contention` answers `false` for by construction — the
    /// bare 500 this door promised a retry instead of.
    fn from_repo(error: &RepoError) -> Self {
        Self::Db(DbError::Sea(error.to_db_err()))
    }
}

/// The `DbErr` inside a [`HeadActError`], for `transaction_with_retry`'s
/// contention classifier.
///
/// Only [`HeadActError::Db`] can carry one. A refusal and a vanished head
/// answer `None` deliberately: both are decided answers, and retrying either
/// would re-run an act whose outcome the door has already established.
fn head_act_contention_db_err(error: &HeadActError) -> Option<&DbErr> {
    match error {
        HeadActError::Db(db_error) => contention_db_err(db_error),
        HeadActError::Refused(_) | HeadActError::Vanished => None,
    }
}

/// Which refusal a zero-row head write actually was.
///
/// [`repo::HeadWrite::Unmatched`] has several readings — the head no longer
/// carries the pinned revision, it is terminal, it is another tenant's, or
/// (for a discard) it is not legal to discard — and that repository's own
/// doc says a door that needs to tell them apart re-reads the head. This is
/// that re-read. **It decides only which message is returned, never whether
/// the write landed**, so it carries none of the race a read-then-write
/// would: the database has already judged the row image the write would have
/// landed on, and this read judges only what to call the answer.
///
/// Ordered revision-first because a moved revision is the only one of the
/// readings the caller can act on — refetch the head, re-send the `ETag` —
/// and reporting a terminal state for a row a neighbour retired *after*
/// moving it would name the second cause of a refusal whose first cause the
/// caller could have fixed.
async fn classify_unmatched_publish(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
) -> HeadActError {
    match repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id).await {
        Ok(Some(head)) if head.internal_revision != inputs.expected => {
            HeadActError::Refused(DomainError::StaleRevision {
                expected: inputs.expected,
                found: head.internal_revision,
            })
        }
        Ok(Some(head)) if head.lifecycle_state.is_terminal() => {
            HeadActError::Refused(DomainError::EntityTerminal(format!(
                "no head write is admitted on a {} entity",
                head.lifecycle_state.as_str()
            )))
        }
        // The filter admits `draft`, `published` and `deprecated` at the
        // pinned revision, and the three arms above have excluded every
        // other reading, so this row satisfies the filter that did not match
        // it. That is a contradiction in the store rather than a refusal of
        // the caller, and it is reported as one instead of being dressed up
        // as a stale revision the caller could pointlessly retry.
        Ok(Some(head)) => HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "publish matched no row for product {} at revision {}, yet the head is {} at \
             revision {}",
            head.product_id,
            inputs.expected,
            head.lifecycle_state.as_str(),
            head.internal_revision
        )))),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_repo(&error),
    }
}

/// [`classify_unmatched_publish`]'s discard twin, with the one reading a
/// discard adds: the act is legal **only** from `draft` with
/// `published_version = 0` (`inst-fd-discard`), both conditions carried in
/// [`repo::discard_product_head`]'s own filter, so a row that fails either
/// is refused `ILLEGAL_TRANSITION` naming the edge it asked for.
async fn classify_unmatched_discard(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
) -> HeadActError {
    match repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id).await {
        Ok(Some(head)) if head.internal_revision != inputs.expected => {
            HeadActError::Refused(DomainError::StaleRevision {
                expected: inputs.expected,
                found: head.internal_revision,
            })
        }
        Ok(Some(head)) if head.lifecycle_state.is_terminal() => {
            HeadActError::Refused(DomainError::EntityTerminal(format!(
                "no head write is admitted on a {} entity",
                head.lifecycle_state.as_str()
            )))
        }
        Ok(Some(head)) => HeadActError::Refused(DomainError::IllegalTransition {
            from: head.lifecycle_state.as_str().to_owned(),
            to: LifecycleState::Discarded.as_str().to_owned(),
        }),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_repo(&error),
    }
}

/// Which Foundation event a head act announces — and, with it, which body
/// shape §4.5 puts on it.
///
/// An enum rather than the payload-type token alone, because the token and
/// the body shape are **one** decision: §4.5 gives `ProductPublished` a
/// `publishedVersion` beyond the shared core and gives `ProductDiscarded`
/// only the core, so a caller that could pass an arbitrary token would be
/// able to pair either token with either body. Here it cannot: the token is
/// read off the variant and so is the body.
#[derive(Debug, Clone, Copy)]
enum Announcement {
    /// `ProductPublished` — core plus `publishedVersion`
    /// (`events::PublishedEventBody`).
    Published,
    /// `ProductDiscarded` — the bare core. A discard writes no version row
    /// and moves no version counter, so there is no `publishedVersion` this
    /// event could truthfully carry.
    Discarded,
    /// `ProductHeadSaved` — the bare core, for the discard's reason exactly:
    /// a save writes no version row and moves no version counter. §4.5 makes
    /// `lifecycleState` *"the discriminator a consumer of `*HeadSaved` needs,
    /// since a save lands on a `draft`, `published` or `deprecated` head
    /// alike"*, and that field is already in the core.
    HeadSaved,
    /// `ProductUndeprecated` — the bare core. Un-deprecation writes no
    /// version row.
    Undeprecated,
}

impl Announcement {
    /// The `payload_type` token the outbox row carries.
    ///
    /// `ProductDeprecated` is deliberately not an arm: the deprecate act's
    /// body is [`DeprecatedProductView`], not [`ProductView`], so it has its
    /// own tail (`deprecate_announce_and_answer`) rather than a fourth case
    /// in [`announce_and_answer`]'s.
    const fn payload_type(self) -> &'static str {
        match self {
            Self::Published => events::PRODUCT_PUBLISHED_PAYLOAD_TYPE,
            Self::Discarded => events::PRODUCT_DISCARDED_PAYLOAD_TYPE,
            Self::HeadSaved => events::PRODUCT_HEAD_SAVED_PAYLOAD_TYPE,
            Self::Undeprecated => events::PRODUCT_UNDEPRECATED_PAYLOAD_TYPE,
        }
    }
}

/// Render the head as it now stands, enqueue `announcement` for it, and —
/// where the request carried a key — store that rendering as the act's
/// idempotency answer. The tail every head act's transaction shares.
///
/// The head is **re-read** rather than reconstructed from the pre-act record
/// plus what the door believes it wrote. The reconstruction is what a reader
/// expects to find here, and it is exactly the thing this file should not
/// do: the head-row guard, the `CASE` that decides the edge inside
/// [`repo::publish_product_head`] and the two counters are all the
/// **database's** answers, and a door that told the client its own arithmetic
/// instead would report a `200` describing a row that might differ from the
/// one it committed. The event's `internal_revision` is "the value as
/// committed by the act" (**P-D-29**) for the same reason.
///
/// The re-read is also what supplies `publishedVersion`: `head
/// .published_version` **after** the act is `N + 1`, the very number
/// [`freeze_for`] keyed the frozen row at, read back from the row the
/// `UPDATE` committed rather than recomputed here.
async fn announce_and_answer(
    runner: &(impl DBRunner + Sync),
    outbox: &crate::infra::broker::EventSink,
    inputs: &HeadActInputs,
    announcement: Announcement,
) -> Result<HeadActOutcome, HeadActError> {
    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    let core = events::EventBodyCore {
        tenant_id: head.tenant_id,
        entity_kind: events::EntityKind::Product.as_str(),
        entity_id: head.product_id,
        internal_revision: head.internal_revision,
        lifecycle_state: head.lifecycle_state.as_str(),
    };
    let payload_type = announcement.payload_type();
    match announcement {
        Announcement::Published => {
            events::enqueue_published(
                outbox,
                runner,
                head.product_id,
                payload_type,
                &core,
                head.published_version,
                inputs.actor_ref,
            )
            .await
        }
        Announcement::Discarded | Announcement::HeadSaved | Announcement::Undeprecated => {
            events::enqueue(
                outbox,
                runner,
                head.product_id,
                payload_type,
                &core,
                inputs.actor_ref,
            )
            .await
        }
    }
    .map_err(|e| HeadActError::from_storage(&e))?;

    if matches!(announcement, Announcement::Published) {
        reannounce_retirement_if_live(runner, outbox, inputs, &core, head.published_version)
            .await?;
    }

    let internal_revision = head.internal_revision;
    let body = serde_json::to_value(ProductView::from(head)).map_err(|e| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "render the published Product: {e}"
        ))))
    })?;

    if let Some(input) = inputs.claim.as_ref() {
        record_idempotency_answer(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            input,
            HEAD_ACT_RESPONSE_STATUS,
            &body,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }

    Ok(HeadActOutcome::Applied {
        internal_revision,
        body,
    })
}

/// P-D-20 / P-D-48: a publish that moves the version while a live retire
/// intent is in the lead window re-emits `ProductRetired` in this same
/// transaction. The read is the operand the door did not have when the
/// clause was deferred to slice 04.
async fn reannounce_retirement_if_live(
    runner: &(impl DBRunner + Sync),
    outbox: &crate::infra::broker::EventSink,
    inputs: &HeadActInputs,
    core: &events::EventBodyCore,
    from_version: i64,
) -> Result<(), HeadActError> {
    let intents =
        repo::find_live_retire_intents(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
    let Some(intent) = intents.into_iter().next() else {
        return Ok(());
    };
    if !publish_reannounces_retirement(inputs.now, intent.created_at, intent.at) {
        return Ok(());
    }
    let reason = intent.retirement_reason.unwrap_or_default();
    events::enqueue_retired(
        outbox,
        runner,
        inputs.product_id,
        events::PRODUCT_RETIRED_PAYLOAD_TYPE,
        events::RetiredEventBody {
            core,
            from_version,
            reason,
            replaced_by: None,
            effective_at: intent.at.to_rfc3339_opts(SecondsFormat::Secs, true),
            must_migrate_by: None,
        },
        inputs.actor_ref,
    )
    .await
    .map_err(|e| HeadActError::from_storage(&e))
}

/// Take the act's idempotency claim, if it carries one, on the mutation's
/// own runner (**P-D-42**) — the first statement of every head act's
/// transaction, exactly as it is the first statement of
/// [`insert_product_with_event`]'s.
///
/// `Ok(None)` means proceed; `Ok(Some(outcome))` is a replay to serve with
/// nothing executed; an `Err` is a refusal, and it rolls the transaction back
/// rather than committing an empty one — see [`HeadActError`] for why every
/// refusal here is an error and not an outcome.
async fn claim_for_head_act(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    claim: Option<&IdempotencyClaimInput>,
) -> Result<Option<HeadActOutcome>, HeadActError> {
    let Some(input) = claim else {
        return Ok(None);
    };
    match claim_idempotency(runner, scope, tenant_id, input)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
    {
        ClaimVerdict::Proceed => Ok(None),
        ClaimVerdict::Replay { status, body } => Ok(Some(HeadActOutcome::Replay { status, body })),
        ClaimVerdict::Refused(refusal) => Err(HeadActError::Refused(refusal)),
    }
}

/// Freeze the version row, then move the head, then announce — one
/// transaction, in the order `inst-fd-publish-txn` forces
/// (`dod-publish-door`). The act itself is [`run_publish`]; this function is
/// the runner it rides on.
///
/// # The order is not a preference
///
/// The head-row guard admits a `published_version` bump **only where the
/// matching `products_entity_version` row already exists**
/// (`m20260829_000002_create_products_product`'s
/// `trg_products_product_published_version_row`), so the freeze has to be
/// visible to the bump. Freeze first, bump second, and both on this one
/// transaction — a freeze committed on a runner of its own would survive a
/// rolled-back publish and would then stand as the guard's prerequisite for
/// a version no committed act ever produced.
///
/// # Exactly one head-row statement
///
/// [`repo::publish_product_head`] carries the version bump, the revision
/// bump, the `draft -> published` edge and `updated_at` in one `UPDATE`,
/// because the guard bumps `internal_revision` on **every** admitted
/// `UPDATE` without exception (`inst-fd-publish-bump`) and two statements
/// would therefore move it twice for one act. Neither this function nor
/// [`run_publish`] may grow a second head-row write.
///
/// # It runs under `transaction_with_retry`, for the create door's reason
///
/// The claim `INSERT` is this transaction's first statement and is the gate
/// (P-D-42), which makes this the transaction concurrent duplicates
/// deliberately collide on; `DBProvider::transaction` has no contention
/// retry. The body is safe to re-run: the claim rolls back with everything
/// after it, so a retried attempt starts against exactly the state the first
/// one did — no key held, no version row, no head movement. Its inputs are
/// attempt-independent, `now` having been stamped before the first; the
/// envelope's `event_id`, minted per enqueue, is the one value that differs
/// per attempt, and it is harmless for the reason
/// [`insert_product_with_event`] states.
///
/// # The retry needs the driver's error, not its text
///
/// `is_retryable_contention` matches `DbErr::Exec`/`DbErr::Query` only, so
/// this section's promise holds only while the failure keeps that variant
/// all the way from the driver to [`head_act_contention_db_err`]. It is
/// [`RepoError::Driver`] that carries it and
/// [`HeadActError::from_repo`] that preserves it; a wrap through `Display`
/// anywhere on that path — which is what this door originally did, inheriting
/// it from the create door — turns every collision back into a bare `500`.
///
/// # The gate arrives as an `Arc`, and it has to
///
/// `transaction_with_retry`'s body is
/// `for<'a> FnMut(&'a DbTx<'a>) -> Pin<Box<dyn Future + Send + 'a>>`: the
/// higher-ranked `'a` cannot be bounded by any lifetime the caller holds, so
/// a borrowed `&dyn GovernanceGate` cannot be captured by the closure at
/// all — the same constraint [`HeadActInputs`] exists for. An owned,
/// cheaply-cloned handle can be, which is why the port travels as
/// `Arc<dyn GovernanceGate + Send + Sync>` from the handler down.
///
/// `mode` travels beside it, by value: [`GateMode`] is `Copy`, so every retry
/// attempt takes its own copy the way every other input does.
async fn publish_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    mode: GateMode,
) -> Result<HeadActOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let inputs = opened.act_inputs();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                // `FnMut`: every attempt takes its own copies, so a retried
                // attempt never finds an input the previous one consumed.
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                Box::pin(
                    async move { run_publish(tx, &inputs, gate.as_ref(), mode, &outbox).await },
                )
            },
        )
        .await
}

/// Discard a never-published draft: one guarded head-row `UPDATE`, the
/// approval-invalidation hook the transition owes, and the
/// `ProductDiscarded` event — one transaction (`inst-fd-discard`). The act
/// itself is [`run_discard`].
///
/// # No freeze, and no release statement
///
/// A discard writes no `products_entity_version` row: nothing was published,
/// so there is no version to freeze. It also writes no reservation-release
/// statement, and that is a property of the indexes rather than an omission
/// here — `uq_products_product_name` and `uq_products_product_code` are both
/// partial on `lifecycle_state <> 'discarded'`, so the row leaves both the
/// moment this `UPDATE` commits and the name and the `productCode` are free
/// for the next holder. A second statement would have no rows to touch.
///
/// # The legality is in the `WHERE` clause
///
/// `draft` and `published_version = 0` are both carried by
/// [`repo::discard_product_head`]'s own filter, so the **database** judges
/// the row image the write lands on. The [`transition::guard`] call
/// [`run_discard`] makes decides the *edge* and reports what the transition
/// costs; it does not stand in for the filter, because even a read taken
/// inside this transaction is a read, and the filter is the write.
///
/// The gate host travels as an `Arc` for [`publish_in_one_transaction`]'s
/// stated reason: the phase runs here too (`inst-fd-pipeline-gate-phase`),
/// and the closure this transaction takes cannot capture a borrow.
async fn discard_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<HeadActOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let inputs = opened.act_inputs();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                Box::pin(async move { run_discard(tx, &inputs, gate.as_ref(), &outbox).await })
            },
        )
        .await
}

/// Turn one head act's transaction result into the response the caller gets.
///
/// Shared by the head-act doors — publish, discard, deprecate and save: the
/// success, the replay, the audited refusal, the vanished head and the
/// storage failure are answered identically whichever act produced them, and
/// the only thing that differs is the `action` token a refusal's audit row
/// records.
async fn answer_head_act(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    audit_action: &str,
    result: Result<HeadActOutcome, HeadActError>,
) -> Result<Response, CanonicalError> {
    match result {
        Ok(HeadActOutcome::Applied {
            internal_revision,
            body,
        }) => {
            let tag = preconditions::etag(InternalRevision::new(internal_revision));
            // `body` is the value rendered inside the mutation transaction
            // and, for a keyed act, stored there as the idempotency answer.
            // Answering it rather than re-rendering the view is what makes a
            // later replay reproduce this response and not a lookalike.
            Ok((HEAD_ACT_RESPONSE_STATUS, [(ETAG, tag)], axum::Json(body)).into_response())
        }
        // A replay executes nothing and audits nothing: it is not a refusal,
        // and the act it reproduces was audited — or, being a success,
        // deliberately not (P-D-21) — when it originally ran.
        Ok(HeadActOutcome::Replay { status, body }) => Ok(replay_response(status, body)),
        Err(HeadActError::Refused(domain_err)) => {
            Err(audit_and_refuse(state, opened, audit_action, domain_err).await)
        }
        Err(HeadActError::Vanished) => Err(product_not_found(opened.record.product_id)),
        Err(HeadActError::Db(db_error)) => Err(repo_error_to_canonical(&RepoError::Db(
            db_error.to_string(),
        ))),
    }
}

/// Cancel answers **202** with no body. [`answer_head_act`] always
/// answers 200 + JSON for [`HeadActOutcome::Applied`].
async fn answer_cancel_act(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    audit_action: &str,
    result: Result<HeadActOutcome, HeadActError>,
) -> Result<Response, CanonicalError> {
    match result {
        Ok(HeadActOutcome::Applied { .. } | HeadActOutcome::Replay { .. }) => {
            Ok(StatusCode::ACCEPTED.into_response())
        }
        Err(HeadActError::Refused(domain_err)) => {
            Err(audit_and_refuse(state, opened, audit_action, domain_err).await)
        }
        Err(HeadActError::Vanished) => Err(product_not_found(opened.record.product_id)),
        Err(HeadActError::Db(db_error)) => Err(repo_error_to_canonical(&RepoError::Db(
            db_error.to_string(),
        ))),
    }
}

/// The validation pipeline the publish door re-runs
/// (`inst-fd-publish-revalidate`).
///
/// **This is the Foundation's pipeline, and this slice's Foundation
/// registers exactly one rule** — `crate::domain::rules::NameShapeRule`. The
/// design's own wording for this step is "shape, state, identity, **every
/// registered validator for `→ published`**", and the registered validators
/// are 04's and 05's: they do not exist at this commit, so the re-run cannot
/// include them and this door does not pretend it does. What the re-run
/// *does* discharge is the fail-closed property the step exists for — the
/// pipeline is re-executed at publish over the entity as it now stands,
/// rather than the door trusting a judgement made when the entity was
/// authored — so a validator registered later is picked up here with no
/// change to this door.
///
/// Built per call rather than held in a `static`: the pipeline owns boxed
/// trait objects, and a lazily-initialised global would buy nothing at this
/// size while adding an initialisation order to reason about.
fn publish_revalidation_pipeline() -> ValidationPipeline<CreateEntityCandidate> {
    ValidationPipeline::new().with_rule(Box::new(NameShapeRule))
}

/// The `→ published` edge's own pipeline — the registered validators that
/// judge the **transition** rather than the row's shape
/// (`inst-tx-primary-at-publish`; `dod-primary-at-publish`).
///
/// **Separate from the pipeline above, and the separation is the rule.** The
/// PRD makes a primary category *"optional at draft, required at publish"*,
/// and the pipeline above runs on **both** the publish edge and the save
/// door's re-validation — so a rule registered there would refuse a save on
/// a draft that carries no primary, which is the case the design explicitly
/// admits. This pipeline runs only from [`run_publish`].
///
/// The `Phase::RegisteredValidators` note on the SKU door says that phase is
/// *"empty, and that is a real gap"* because the validators it named were
/// `04`'s and `05`'s. This is the first one that is neither: `02` declares
/// it, `02` declares its code, and the phase is no longer vacuous on the
/// Product publish path.
fn published_transition_pipeline() -> ValidationPipeline<PublishedTransitionSubject> {
    ValidationPipeline::new().with_rule(Box::new(PrimaryCategoryRequired))
}

/// Turn a failing `→ published` registered validator into the door's refusal,
/// carrying the **rule's own code** rather than the generic one.
///
/// [`revalidation_refusal`] answers `INCOMPLETE_ENTITY` because the rules it
/// folds are the Foundation's shape rules, which have no codes of their own.
/// `inst-fd-publish-revalidate` names *"`INCOMPLETE_ENTITY`/**rule-named
/// code**"* — two alternatives — and a registered validator that declares one
/// is the second case. A consumer matches on the code to know what to repair,
/// and *"assign a primary category"* is a different repair from every other
/// reason a publish can be incomplete.
///
/// Only the codes this crate declares are surfaced; anything else falls back,
/// so a future rule cannot leak an unmapped code onto the wire by forgetting
/// to add a ladder arm.
fn transition_refusal(report: &ValidationReport) -> DomainError {
    let detail = report
        .violations()
        .iter()
        .map(|violation| format!("{}: {}", violation.subject, violation.detail))
        .collect::<Vec<_>>()
        .join("; ");
    let only = report.violations().first().map(|v| v.code);
    if only == Some(PrimaryCategoryRequired::CODE) && report.violations().len() == 1 {
        return DomainError::PrimaryCategoryRequired(detail);
    }
    revalidation_refusal(report)
}

/// The candidate the re-run judges: the head **as it now stands**, not the
/// payload that created it.
///
/// `CreateEntityCandidate` is named for the door that first presented one,
/// and it is the right shape here for the reason its own doc gives — it
/// carries the payload fields plus the normalization the identity phase keys
/// on. A publish presents the row instead of a payload, which is precisely
/// what `inst-fd-publish-revalidate` asks for: *an entity that stopped being
/// publishable since approval fails closed*.
fn publish_candidate(record: &ProductRecord) -> CreateEntityCandidate {
    CreateEntityCandidate {
        tenant_id: record.tenant_id,
        brand_id: record.brand_id,
        name: record.name.clone(),
        code: record.product_code.clone(),
    }
}

/// Turn a failing publish re-validation into the door's refusal.
///
/// **`INCOMPLETE_ENTITY` rather than `VALIDATION`.**
/// `inst-fd-publish-revalidate` names *"`INCOMPLETE_ENTITY`/rule-named
/// code"* for an entity that stopped being publishable since approval, and
/// the distinction is not cosmetic here: a publish carries **no request
/// body** ([`bodiless_payload_digest`] is built on exactly that fact), so a
/// `VALIDATION` problem would name a field of a request that had no fields.
/// It would tell a caller to fix its payload when the payload was fine and
/// the row was not. This door answered `VALIDATION` until this fix;
/// `skus::revalidation_refusal` is the twin, and it argued the same point
/// first.
///
/// The report's violations are folded into the detail string rather than
/// carried as per-field entries, because the wire shape
/// `DomainError::IncompleteEntity` offers is a message: what a reader needs
/// is *which* rule stopped being satisfied, and `subject: detail` per
/// violation is that, joined.
fn revalidation_refusal(report: &ValidationReport) -> DomainError {
    let detail = report
        .violations()
        .iter()
        .map(|violation| format!("{}: {}", violation.subject, violation.detail))
        .collect::<Vec<_>>()
        .join("; ");
    DomainError::IncompleteEntity(format!("the entity is no longer publishable: {detail}"))
}

/// Fire the approval-invalidation hook where the transition floor says this
/// act's edge fires one, inside the act's own transaction
/// (`dod-supersede`).
///
/// # The store-backed write lives here rather than behind the domain trait
///
/// [`transition::ApprovalInvalidationHook`] is **synchronous and storeless**
/// — `invalidate(&self, subject)` has no runner — which was right while there
/// was no approval store to reach. There is one now, and the trait's shape
/// cannot carry a transactional write. Changing that signature is
/// `01-foundation`'s own act (`dod-approval-hook`) and a different one from
/// this, so the trait stays as the domain seam and the write happens here, on
/// the transaction the act already holds. `NoApprovalStoreHook` therefore
/// remains the trait's only impl, and it is no longer the whole behaviour.
///
/// # The hook does not fire where the act consumes an approval
///
/// That is not a condition this function re-derives: the floor returns
/// [`ApprovalInvalidation::Skip`] for exactly those edges, because *"a hook
/// firing against the record the act is spending has no defined ordering"*
/// (05 C3, and **P-D-30** reproduced the same collision on
/// `deprecated -> published`). This function reads the floor's answer.
///
/// # Nothing is re-submitted
///
/// `inst-gv-supersede` requires re-submission to be an explicit human act,
/// *"never automatic — auto-resubmit would pin content nobody re-read"*. So a
/// fired hook writes one row and creates none, and a subject with no open
/// record is a no-op rather than an error: the frozen-content write is legal
/// whether or not a ceremony was open against it.
async fn fire_invalidation_hook(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
    invalidation: ApprovalInvalidation,
) -> Result<(), HeadActError> {
    if invalidation != ApprovalInvalidation::Fire {
        return Ok(());
    }
    let subject = GateSubject::entity_publish(
        EntityRef {
            tenant_id: inputs.tenant_id,
            entity_kind: CatalogEntityKind::Product,
            entity_id: inputs.product_id,
        },
        InternalRevision::new(inputs.expected),
    );
    // The domain seam still runs: it is the pure part of the act, and a host
    // that ever refuses is a refusal of the transition.
    NoApprovalStoreHook
        .invalidate(EntityRef {
            tenant_id: inputs.tenant_id,
            entity_kind: CatalogEntityKind::Product,
            entity_id: inputs.product_id,
        })
        .map_err(HeadActError::Refused)?;
    repo::supersede_open_approval(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        &subject,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::Db(toolkit_db::DbError::Sea(e.to_db_err())))?;
    Ok(())
}

/// The state a publish leaves the head in, which is also the `to` side of
/// the edge [`transition::guard`] is asked about.
///
/// Decided from the row image, exactly as [`repo::publish_product_head`]'s
/// own `CASE` decides it: a `draft` becomes `published`, and every other
/// admitted head keeps its state. Asking the guard about
/// `deprecated -> published` instead would pull the two-person un-deprecate
/// ceremony onto a content re-publish that changes no state — the reading
/// `inst-fd-publish-revalidate` explicitly rejects.
const fn published_state_after(from: LifecycleState) -> LifecycleState {
    match from {
        LifecycleState::Draft => LifecycleState::Published,
        LifecycleState::Published
        | LifecycleState::Deprecated
        | LifecycleState::Retired
        | LifecycleState::Discarded => from,
    }
}

/// `POST /bss-products/v1/products/{id}/deprecate` — the operator act, and
/// the **cascade** onto its children (`inst-lc-deprecate`,
/// `dod-deprecation-provenance`, `dod-deprecation-cascade`).
///
/// # Errors
///
/// Every refusal the door raises, each audited; the bare `404`; the `500` a
/// storage or gate-host failure raises.
async fn deprecate_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    deprecate_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// The deprecate door with its governance host explicit —
/// [`discard_product_under_gate`]'s twin, and for its reasons.
///
/// # Errors
///
/// As [`deprecate_product`].
async fn deprecate_product_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let act = HeadAct {
        // `write`, not a `deprecate` action of its own: the discard door's
        // argument unchanged — the authorization vocabulary is not
        // one-to-one with the act vocabulary, and the audit token beside
        // this is what tells the trail apart. Whether the act deserves a
        // dedicated grant the way publish has `product × publish` is part of
        // `features/lifecycle.md` §7 row 36.
        authz_action: crate::authz::actions::WRITE,
        audit_action: DEPRECATE_AUDIT_ACTION,
        endpoint: deprecate_endpoint(product_id),
        payload_digest: bodiless_payload_digest(),
    };
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;
    // The cascade reads and writes `products_sku`, and the opened door's
    // scope is compiled for the PRODUCT resource — on the SKU table its
    // `resource_id` constraints would filter `sku_id` by *product* ids, so a
    // constrained grant would silently cascade nothing. The clone door is
    // the precedent: a Product act that touches child rows compiles a second
    // scope for the SKU resource and spends both grants.
    let sku_scope = clone_scope_for(
        state,
        enforcer,
        ctx,
        opened.tenant_id,
        opened.actor_ref,
        product_id,
        &crate::authz::resource_types::SKU,
    )
    .await?;
    let result = deprecate_in_one_transaction(state, &opened, &sku_scope, gate).await;
    answer_head_act(state, &opened, DEPRECATE_AUDIT_ACTION, result).await
}

/// [`discard_in_one_transaction`]'s deprecate twin.
async fn deprecate_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    sku_scope: &AccessScope,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<HeadActOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let inputs = opened.act_inputs();
    let sku_scope = sku_scope.clone();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                let sku_scope = sku_scope.clone();
                Box::pin(async move {
                    run_deprecate(tx, &inputs, &sku_scope, gate.as_ref(), &outbox).await
                })
            },
        )
        .await
}

/// The deprecation act's whole transaction: the parent's `published →
/// deprecated` with its `direct` stamp, then the cascade onto the children,
/// then one announcement per row actually moved — and the idempotency answer
/// stored **after** the listings are merged, so a keyed replay reproduces
/// the same body this call answered.
///
/// # The cascade's population is decided once, and each write is pinned
///
/// The children are read here, inside this transaction and under the SKU
/// scope, and classified by `domain::deprecation::disposition_for` —
/// `published` ones cascade, already-`deprecated` ones are left untouched
/// with their provenance intact, `draft` ones are **skipped and listed**,
/// and terminal ones are outside the population. Each classified child is
/// then written by `repo::cascade_deprecate_child`, **pinned at the revision
/// the classification read**: a child a concurrent writer moved between the
/// read and the write answers `Unmatched`, and the whole mutation is refused
/// `STALE_REVISION` rather than committing a half-cascade or announcing a
/// revision the row never held (`01 inst-fd-fail-closed`; P-D-29's "the
/// value as committed by the act" is then the pin plus one, proven by the
/// pinned write itself). Each moved child also runs the approval-invalidation
/// hook: `published → deprecated` is an ungated edge, so the floor gives it
/// `ApprovalInvalidation::Fire`, for the children exactly as for the parent.
///
/// # Why the drafts are listed rather than refused
///
/// The floor admits no `draft → deprecated` edge, and an earlier revision of
/// the design keyed the cascade on *"non-terminal children"* — which made
/// deprecating a Product with one draft SKU fail `ILLEGAL_TRANSITION` with no
/// remedy an operator could take. The listing is what the operator sees; it
/// rides the response's `skipped_draft_skus`.
///
/// # Errors
///
/// As [`run_discard`], plus the cascade's own fail-closed refusal.
async fn run_deprecate(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    gate: &(dyn GovernanceGate + Send + Sync),
    outbox: &crate::infra::broker::EventSink,
) -> Result<HeadActOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    let decision = transition::guard(head.lifecycle_state, LifecycleState::Deprecated)
        .map_err(HeadActError::Refused)?;

    // The stamp the parent takes. `stamp_for` answers `None` on an
    // already-`deprecated` head, and `transition::guard` does NOT refuse
    // that head — the diagonal is `NotATransition` by the floor's own rule 3
    // — so this `else` arm is the door's live refusal of a second
    // deprecation, not an assertion about an unreachable state.
    let Some(parent_stamp) =
        stamp_for(head.lifecycle_state, Provenance::Direct).map_err(HeadActError::Refused)?
    else {
        return Err(HeadActError::Refused(DomainError::IllegalTransition {
            from: head.lifecycle_state.as_str().to_owned(),
            to: LifecycleState::Deprecated.as_str().to_owned(),
        }));
    };

    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    let write = repo::deprecate_product_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        inputs.expected,
        parent_stamp,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if write == HeadWrite::Unmatched {
        return Err(classify_unmatched_deprecate(runner, inputs).await);
    }

    let report = cascade_onto_children(runner, outbox, inputs, sku_scope).await?;

    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

    deprecate_announce_and_answer(runner, outbox, inputs, report).await
}

/// Render the deprecated head with its three listings, enqueue
/// `ProductDeprecated`, and — where the request carried a key — store **that
/// merged body** as the idempotency answer.
///
/// [`announce_and_answer`]'s deprecate twin rather than a fourth arm on it:
/// this act's answered body is [`DeprecatedProductView`], not
/// [`ProductView`], and the stored answer must be the same value — an
/// earlier revision merged the listings after the store, so a keyed replay
/// reproduced the head alone.
async fn deprecate_announce_and_answer(
    runner: &(impl DBRunner + Sync),
    outbox: &crate::infra::broker::EventSink,
    inputs: &HeadActInputs,
    report: CascadeReport,
) -> Result<HeadActOutcome, HeadActError> {
    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    let core = events::EventBodyCore {
        tenant_id: head.tenant_id,
        entity_kind: events::EntityKind::Product.as_str(),
        entity_id: head.product_id,
        internal_revision: head.internal_revision,
        lifecycle_state: head.lifecycle_state.as_str(),
    };
    let provenance = head
        .deprecation_provenance
        .unwrap_or(Provenance::Direct)
        .as_str();
    events::enqueue_deprecated(
        outbox,
        runner,
        head.product_id,
        events::PRODUCT_DEPRECATED_PAYLOAD_TYPE,
        &core,
        provenance,
        inputs.actor_ref,
    )
    .await
    .map_err(|e| HeadActError::from_storage(&e))?;

    let internal_revision = head.internal_revision;
    let body =
        serde_json::to_value(DeprecatedProductView::from_parts(head, report)).map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "render the deprecated Product: {e}"
            ))))
        })?;

    if let Some(input) = inputs.claim.as_ref() {
        record_idempotency_answer(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            input,
            HEAD_ACT_RESPONSE_STATUS,
            &body,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }

    Ok(HeadActOutcome::Applied {
        internal_revision,
        body,
    })
}

/// What the cascade did, in the three readings the operator is owed.
struct CascadeReport {
    /// The children moved to `deprecated` with a `cascaded` stamp.
    deprecated: Vec<Uuid>,
    /// The children **left untouched** because they were already
    /// `deprecated`. Their provenance is not re-stamped, which is what keeps
    /// a `direct` child from being revived by this parent's later
    /// un-deprecation (`dod-provenance-reversal`).
    left_untouched: Vec<Uuid>,
    /// The `draft` children **skipped and listed**. This list is the
    /// operator-visible half of `dod-deprecation-cascade`.
    skipped_drafts: Vec<Uuid>,
}

/// Classify, write, supersede and announce the cascade — the second half of
/// [`run_deprecate`], on the same runner and therefore in the same
/// transaction.
///
/// Reads and writes `products_sku` under the **SKU** scope, never the opened
/// door's Product one — see `deprecate_product_under_gate` for why the
/// Product scope's constraints would misfilter the child table.
async fn cascade_onto_children(
    runner: &(impl DBRunner + Sync),
    outbox: &crate::infra::broker::EventSink,
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
) -> Result<CascadeReport, HeadActError> {
    let children =
        repo::find_skus_of_product(runner, sku_scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;

    let mut report = CascadeReport {
        deprecated: Vec::new(),
        left_untouched: Vec::new(),
        skipped_drafts: Vec::new(),
    };
    let mut moved: Vec<(Uuid, i64)> = Vec::new();
    for child in &children {
        match disposition_for(child.lifecycle_state) {
            ChildDisposition::Deprecate => {
                report.deprecated.push(child.sku_id);
                moved.push((child.sku_id, child.internal_revision));
            }
            ChildDisposition::LeaveUntouched => report.left_untouched.push(child.sku_id),
            ChildDisposition::SkipAndList => report.skipped_drafts.push(child.sku_id),
            // Read out of the store by `find_skus_of_product`, which excludes
            // `discarded` but not `retired`. Outside the population is not
            // the same as skipped inside it, so it reaches no listing.
            ChildDisposition::OutsidePopulation => {}
        }
    }

    for (sku_id, pinned) in moved {
        let write = repo::cascade_deprecate_child(
            runner,
            sku_scope,
            inputs.tenant_id,
            sku_id,
            pinned,
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
        if write == HeadWrite::Unmatched {
            // The child moved between the classification and its pinned
            // write. Refusing the whole mutation is `inst-fd-fail-closed`;
            // STALE_REVISION is the honest class — the operator re-reads and
            // retries, exactly as for the head's own pin — and the found
            // value is read off the committed row so the refusal names what
            // actually moved.
            let found = repo::find_sku(runner, sku_scope, inputs.tenant_id, sku_id)
                .await
                .map_err(|e| HeadActError::from_repo(&e))?
                .map_or(0, |row| row.internal_revision);
            return Err(HeadActError::Refused(DomainError::StaleRevision {
                expected: pinned,
                found,
            }));
        }

        // Each moved child crossed `published → deprecated`, an ungated edge,
        // so the floor's own answer for it is `Fire` — the same supersede the
        // parent's hook runs, aimed at the child's subject.
        repo::supersede_open_approval(
            runner,
            sku_scope,
            inputs.tenant_id,
            // `SubjectPin::Unpinned` on purpose: the supersede filters on
            // `(tenant, kind, subject_ref)` alone, because a head edit
            // invalidates whatever revision the open record pinned — naming a
            // revision here would suggest the hook is selective and it is not.
            &GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Sku,
                    entity_id: sku_id,
                },
                InternalRevision::new(0),
            ),
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::Db(toolkit_db::DbError::Sea(e.to_db_err())))?;

        // The pinned write is what makes this arithmetic the committed value
        // (P-D-29): the UPDATE matched `internal_revision = pinned`, so the
        // row it wrote is at `pinned + 1` — never a guess over an image a
        // concurrent save may have moved.
        let core = events::EventBodyCore {
            tenant_id: inputs.tenant_id,
            entity_kind: events::EntityKind::Sku.as_str(),
            entity_id: sku_id,
            internal_revision: pinned + 1,
            lifecycle_state: LifecycleState::Deprecated.as_str(),
        };
        events::enqueue_deprecated(
            outbox,
            runner,
            sku_id,
            events::SKU_DEPRECATED_PAYLOAD_TYPE,
            &core,
            Provenance::Cascaded.as_str(),
            inputs.actor_ref,
        )
        .await
        .map_err(|e| HeadActError::from_storage(&e))?;
    }

    Ok(report)
}

/// [`classify_unmatched_discard`]'s deprecate twin: which precondition the
/// `UPDATE` missed on, named from the head as it now stands.
async fn classify_unmatched_deprecate(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
) -> HeadActError {
    match repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id).await {
        Ok(Some(head)) if head.internal_revision != inputs.expected => {
            HeadActError::Refused(DomainError::StaleRevision {
                expected: inputs.expected,
                found: head.internal_revision,
            })
        }
        Ok(Some(head)) if head.lifecycle_state.is_terminal() => {
            HeadActError::Refused(DomainError::EntityTerminal(format!(
                "no head write is admitted on a {} entity",
                head.lifecycle_state.as_str()
            )))
        }
        Ok(Some(head)) => HeadActError::Refused(DomainError::IllegalTransition {
            from: head.lifecycle_state.as_str().to_owned(),
            to: LifecycleState::Deprecated.as_str().to_owned(),
        }),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_repo(&error),
    }
}

/// `POST /bss-products/v1/products/{id}/undeprecate` — empty body,
/// `inst-lc-undeprecate` / `inst-lc-provenance-reversal`.
///
/// Gate call (report this exactly): `gate.evaluate(
/// GateSubject::entity_publish(EntityRef { tenant_id, entity_kind: Product,
/// entity_id }), InternalRevision::new(expected), GateMode::Gate )` after
/// the edge and the live-intent guard, before the write.
/// `NoMaterialityPolicyGate` authorizes under `Gate` and cannot produce
/// the two-person refusal.
///
/// # Errors
///
/// Every refusal the door raises, each audited; the bare `404`; the `500` a
/// storage or gate-host failure raises.
async fn undeprecate_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    undeprecate_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// The un-deprecate door with its governance host explicit.
///
/// # Errors
///
/// As [`undeprecate_product`].
async fn undeprecate_product_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let act = HeadAct {
        authz_action: crate::authz::actions::WRITE,
        audit_action: UNDEPRECATE_AUDIT_ACTION,
        endpoint: undeprecate_endpoint(product_id),
        payload_digest: bodiless_payload_digest(),
    };
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;
    let sku_scope = clone_scope_for(
        state,
        enforcer,
        ctx,
        opened.tenant_id,
        opened.actor_ref,
        product_id,
        &crate::authz::resource_types::SKU,
    )
    .await?;
    let result = undeprecate_in_one_transaction(state, &opened, &sku_scope, gate).await;
    answer_head_act(state, &opened, UNDEPRECATE_AUDIT_ACTION, result).await
}

async fn undeprecate_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    sku_scope: &AccessScope,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<HeadActOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let inputs = opened.act_inputs();
    let sku_scope = sku_scope.clone();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                let sku_scope = sku_scope.clone();
                Box::pin(async move {
                    run_undeprecate(tx, &inputs, &sku_scope, gate.as_ref(), &outbox).await
                })
            },
        )
        .await
}

async fn run_undeprecate(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    gate: &(dyn GovernanceGate + Send + Sync),
    outbox: &crate::infra::broker::EventSink,
) -> Result<HeadActOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    let decision = transition::guard(head.lifecycle_state, LifecycleState::Published)
        .map_err(HeadActError::Refused)?;

    let children =
        repo::find_skus_of_product(runner, sku_scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
    let stored: Vec<(Uuid, Option<Provenance>)> = children
        .iter()
        .map(|child| (child.sku_id, child.deprecation_provenance))
        .collect();
    let revived = children_the_reversal_touches(&stored);

    let mut intents = Vec::new();
    collect_live_retire_intents(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        &mut intents,
    )
    .await?;
    for child_id in &revived {
        collect_live_retire_intents(runner, sku_scope, inputs.tenant_id, *child_id, &mut intents)
            .await?;
    }
    refuse_if_live_retire_intents(inputs.product_id, &revived, &intents)
        .map_err(|refusal| HeadActError::Refused(refusal.into_domain_error()))?;

    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    let write = repo::undeprecate_product_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        inputs.expected,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if write == HeadWrite::Unmatched {
        return Err(classify_unmatched_undeprecate(runner, inputs).await);
    }

    reverse_cascaded_children(runner, outbox, inputs, sku_scope, &children, &revived).await?;

    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

    announce_and_answer(runner, outbox, inputs, Announcement::Undeprecated).await
}

async fn collect_live_retire_intents(
    runner: &(impl DBRunner + Sync),
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_id: Uuid,
    into: &mut Vec<BlockingIntent>,
) -> Result<(), HeadActError> {
    let rows = repo::find_live_retire_intents(runner, scope, tenant_id, entity_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    into.extend(rows.into_iter().map(|row| BlockingIntent {
        entity_id: row.entity_id,
    }));
    Ok(())
}

async fn reverse_cascaded_children(
    runner: &(impl DBRunner + Sync),
    outbox: &crate::infra::broker::EventSink,
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    children: &[SkuRecord],
    revived: &[Uuid],
) -> Result<(), HeadActError> {
    for child in children {
        if !revived.contains(&child.sku_id) {
            continue;
        }
        let write = repo::undeprecate_sku_head(
            runner,
            sku_scope,
            inputs.tenant_id,
            child.sku_id,
            child.internal_revision,
            Some(Provenance::Cascaded),
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
        if write == HeadWrite::Unmatched {
            return Err(HeadActError::Refused(DomainError::StaleRevision {
                expected: child.internal_revision,
                found: child.internal_revision,
            }));
        }
        let after = repo::find_sku(runner, sku_scope, inputs.tenant_id, child.sku_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?
            .ok_or(HeadActError::Vanished)?;
        let core = events::EventBodyCore {
            tenant_id: after.tenant_id,
            entity_kind: events::EntityKind::Sku.as_str(),
            entity_id: after.sku_id,
            internal_revision: after.internal_revision,
            lifecycle_state: after.lifecycle_state.as_str(),
        };
        events::enqueue(
            outbox,
            runner,
            after.sku_id,
            events::SKU_UNDEPRECATED_PAYLOAD_TYPE,
            &core,
            inputs.actor_ref,
        )
        .await
        .map_err(|e| HeadActError::from_storage(&e))?;
    }
    Ok(())
}

async fn classify_unmatched_undeprecate(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
) -> HeadActError {
    match repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id).await {
        Ok(Some(head)) if head.internal_revision != inputs.expected => {
            HeadActError::Refused(DomainError::StaleRevision {
                expected: inputs.expected,
                found: head.internal_revision,
            })
        }
        Ok(Some(head)) if head.lifecycle_state.is_terminal() => {
            HeadActError::Refused(DomainError::EntityTerminal(format!(
                "no head write is admitted on a {} entity",
                head.lifecycle_state.as_str()
            )))
        }
        Ok(Some(head)) => HeadActError::Refused(DomainError::IllegalTransition {
            from: head.lifecycle_state.as_str().to_owned(),
            to: LifecycleState::Published.as_str().to_owned(),
        }),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_repo(&error),
    }
}

fn retire_product_payload_digest(request: &RetireProductRequest) -> Vec<u8> {
    let mut map = JsonMap::new();
    map.insert(
        "reason".to_owned(),
        JsonValue::String(request.reason.clone()),
    );
    map.insert("confirmed".to_owned(), JsonValue::Bool(request.confirmed));
    if let Some(id) = request.replaced_by {
        map.insert("replacedBy".to_owned(), JsonValue::String(id.to_string()));
    }
    if let Some(at) = request.effective_at {
        map.insert("effectiveAt".to_owned(), JsonValue::String(at.to_rfc3339()));
    }
    if let Some(at) = request.must_migrate_by {
        map.insert(
            "mustMigrateBy".to_owned(),
            JsonValue::String(at.to_rfc3339()),
        );
    }
    if let Some(confirmed) = request.cascade_confirmed {
        map.insert("cascadeConfirmed".to_owned(), JsonValue::Bool(confirmed));
    }
    idempotency::payload_digest(&JsonValue::Object(map))
}

/// `POST /bss-products/v1/products/{id}/retire` — §3.3 body.
///
/// Gate call: `gate.evaluate(GateSubject::entity_publish(EntityRef { tenant_id,
/// entity_kind: Product, entity_id }, InternalRevision::new(expected)),
/// GateMode::Gate)` after the edge and domain refusals, before the write.
///
/// 07's reference predicate is not a live operand; children are classified
/// with `referenced = false` until that host exists. The initiation emits
/// `ProductRetired`.
///
/// # Errors
///
/// See [`retire_product_under_gate`].
async fn retire_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<RetireProductRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    retire_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        request,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// # Errors
///
/// As [`retire_product`].
async fn retire_product_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    request: RetireProductRequest,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let act = HeadAct {
        authz_action: crate::authz::actions::WRITE,
        audit_action: RETIRE_AUDIT_ACTION,
        endpoint: retire_endpoint(product_id),
        payload_digest: retire_product_payload_digest(&request),
    };
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;
    let sku_scope = clone_scope_for(
        state,
        enforcer,
        ctx,
        opened.tenant_id,
        opened.actor_ref,
        product_id,
        &crate::authz::resource_types::SKU,
    )
    .await?;
    let detector =
        crate::api::rest::retention::tenant_pii_detector(state, opened.tenant_id).await?;
    let result =
        retire_in_one_transaction(state, &opened, &sku_scope, gate, &detector, request).await;
    answer_head_act(state, &opened, RETIRE_AUDIT_ACTION, result).await
}

async fn retire_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    sku_scope: &AccessScope,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    detector: &Arc<dyn PiiDetector + Send + Sync>,
    request: RetireProductRequest,
) -> Result<HeadActOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let detector = Arc::clone(detector);
    let inputs = opened.act_inputs();
    let sku_scope = sku_scope.clone();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let detector = Arc::clone(&detector);
                let inputs = inputs.clone();
                let sku_scope = sku_scope.clone();
                let request = request.clone();
                Box::pin(async move {
                    run_retire(
                        tx,
                        &inputs,
                        &sku_scope,
                        gate.as_ref(),
                        detector.as_ref(),
                        &request,
                        &outbox,
                    )
                    .await
                })
            },
        )
        .await
}

async fn run_retire(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    gate: &(dyn GovernanceGate + Send + Sync),
    detector: &(dyn PiiDetector + Send + Sync),
    request: &RetireProductRequest,
    outbox: &crate::infra::broker::EventSink,
) -> Result<HeadActOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    confirmation_must_hold(request.confirmed).map_err(HeadActError::Refused)?;

    content_pii_block(detector, "reason", &request.reason).map_err(|blocked| {
        HeadActError::Refused(DomainError::ContentPiiBlocked(blocked.into_detail()))
    })?;

    eol_lockout(false, request.must_migrate_by.is_some())
        .map_err(|refusal| HeadActError::Refused(refusal.into_domain_error()))?;

    let at = effective_at(inputs.now, interim_retirement_lead(), request.effective_at)
        .map_err(|refusal| HeadActError::Refused(refusal.into_domain_error()))?;

    let children =
        repo::find_skus_of_product(runner, sku_scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;

    let live_children = children
        .iter()
        .filter(|child| {
            !matches!(
                child.lifecycle_state,
                LifecycleState::Retired | LifecycleState::Discarded
            )
        })
        .count();
    require_cascade_confirmation(request.cascade_confirmed.unwrap_or(false), live_children)
        .map_err(|refusal| HeadActError::Refused(refusal.into_domain_error()))?;

    // 07's predicate is not a live operand. `referenced = false` classifies
    // published/deprecated children as Retire and drafts as AutoDiscard.
    let operands: Vec<(LifecycleState, bool)> = children
        .iter()
        .map(|child| (child.lifecycle_state, false))
        .collect();
    let plan = CascadePlan::compute(&operands);
    let referenced: Vec<bool> = operands.iter().map(|(_, flag)| *flag).collect();

    let mut intents = Vec::new();
    collect_live_retire_intents(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        &mut intents,
    )
    .await?;
    refuse_if_live_retire_intents(inputs.product_id, &[], &intents)
        .map_err(|refusal| HeadActError::Refused(refusal.into_domain_error()))?;

    let decision = transition::guard(head.lifecycle_state, LifecycleState::Deprecated)
        .map_err(HeadActError::Refused)?;
    let stamp =
        stamp_for(head.lifecycle_state, Provenance::Direct).map_err(HeadActError::Refused)?;

    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    let approval_ref = Uuid::now_v7();
    let parent_transition_id = Uuid::now_v7();
    apply_cascade_plan(
        runner,
        inputs,
        sku_scope,
        &CascadeApply {
            children: &children,
            referenced: &referenced,
            reason: &request.reason,
            at,
            approval_ref,
        },
    )
    .await?;

    if let Some(provenance) = stamp {
        let write = repo::deprecate_product_head(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            inputs.product_id,
            inputs.expected,
            provenance,
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
        if write == HeadWrite::Unmatched {
            return Err(classify_unmatched_deprecate(runner, inputs).await);
        }
    }

    repo::insert_scheduled_transition(
        runner,
        &inputs.scope,
        &NewScheduledTransition {
            transition_id: parent_transition_id,
            tenant_id: inputs.tenant_id,
            entity_kind: "product".to_owned(),
            entity_id: inputs.product_id,
            kind: "retire".to_owned(),
            at,
            approval_ref,
            retirement_reason: Some(request.reason.clone()),
            now: inputs.now,
        },
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    if !plan.leave.is_empty() {
        let listed: Vec<JsonValue> = plan
            .leave
            .iter()
            .map(|&index| {
                serde_json::json!({
                    "sku": children[index].sku_id,
                    "reason": "referenced",
                })
            })
            .collect();
        let children_snapshot = serde_json::to_string(&listed).map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "snapshot leave-and-list children: {e}"
            ))))
        })?;
        repo::insert_deferred_retirement(
            runner,
            &inputs.scope,
            &NewDeferredRetirement {
                tenant_id: inputs.tenant_id,
                product_id: inputs.product_id,
                cascade_ref: parent_transition_id,
                children_snapshot,
                created_by: inputs.actor_ref,
                now: inputs.now,
            },
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }

    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

    announce_retired_and_answer(
        runner,
        outbox,
        inputs,
        &request.reason,
        at,
        head.published_version,
    )
    .await
}

/// The cascade operands that travel together: the classified children and
/// the shared scheduled-transition fields every retire arm writes.
struct CascadeApply<'a> {
    children: &'a [SkuRecord],
    referenced: &'a [bool],
    reason: &'a str,
    at: DateTime<Utc>,
    approval_ref: Uuid,
}

/// @cpt-dod:cpt-cf-bss-products-dod-cascade-plan:p1 — three arms, one txn,
/// supersede publish+retire for every classified child.
async fn apply_cascade_plan(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    apply: &CascadeApply<'_>,
) -> Result<(), HeadActError> {
    for (child, referenced) in apply.children.iter().zip(apply.referenced) {
        let Some(arm) = arm_for(child.lifecycle_state, *referenced) else {
            continue;
        };
        repo::supersede_live_intents(
            runner,
            sku_scope,
            inputs.tenant_id,
            child.sku_id,
            "publish",
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
        repo::supersede_live_intents(
            runner,
            sku_scope,
            inputs.tenant_id,
            child.sku_id,
            "retire",
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;

        match arm {
            crate::domain::cascade::CascadeArm::Retire => {
                if child.lifecycle_state == LifecycleState::Published {
                    let write = repo::cascade_deprecate_child(
                        runner,
                        sku_scope,
                        inputs.tenant_id,
                        child.sku_id,
                        child.internal_revision,
                        inputs.now,
                    )
                    .await
                    .map_err(|e| HeadActError::from_repo(&e))?;
                    if write == HeadWrite::Unmatched {
                        return Err(HeadActError::Refused(DomainError::StaleRevision {
                            expected: child.internal_revision,
                            found: child.internal_revision,
                        }));
                    }
                }
                repo::insert_scheduled_transition(
                    runner,
                    sku_scope,
                    &NewScheduledTransition {
                        transition_id: Uuid::now_v7(),
                        tenant_id: inputs.tenant_id,
                        entity_kind: "sku".to_owned(),
                        entity_id: child.sku_id,
                        kind: "retire".to_owned(),
                        at: apply.at,
                        approval_ref: apply.approval_ref,
                        retirement_reason: Some(apply.reason.to_owned()),
                        now: inputs.now,
                    },
                )
                .await
                .map_err(|e| HeadActError::from_repo(&e))?;
            }
            crate::domain::cascade::CascadeArm::LeaveAndList => {
                if child.lifecycle_state == LifecycleState::Published {
                    let write = repo::cascade_deprecate_child(
                        runner,
                        sku_scope,
                        inputs.tenant_id,
                        child.sku_id,
                        child.internal_revision,
                        inputs.now,
                    )
                    .await
                    .map_err(|e| HeadActError::from_repo(&e))?;
                    if write == HeadWrite::Unmatched {
                        return Err(HeadActError::Refused(DomainError::StaleRevision {
                            expected: child.internal_revision,
                            found: child.internal_revision,
                        }));
                    }
                }
            }
            crate::domain::cascade::CascadeArm::AutoDiscard => {
                let write = repo::discard_sku_head(
                    runner,
                    sku_scope,
                    inputs.tenant_id,
                    child.sku_id,
                    child.internal_revision,
                    inputs.now,
                )
                .await
                .map_err(|e| HeadActError::from_repo(&e))?;
                if write == HeadWrite::Unmatched {
                    return Err(HeadActError::Refused(DomainError::StaleRevision {
                        expected: child.internal_revision,
                        found: child.internal_revision,
                    }));
                }
            }
        }
    }
    Ok(())
}

/// Initiation emits `ProductRetired` (`dod-cascade-parent-path`, P-D-115).
/// `fromVersion` is the head's `published_version` at this instant;
/// `replacedBy` is absent on a Product.
async fn announce_retired_and_answer(
    runner: &(impl DBRunner + Sync),
    outbox: &crate::infra::broker::EventSink,
    inputs: &HeadActInputs,
    reason: &str,
    at: DateTime<Utc>,
    from_version: i64,
) -> Result<HeadActOutcome, HeadActError> {
    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;
    // RetirementScheduled is the door's audit row (action `retire`); it
    // is not a broker event. ProductRetired is the initiation announcement.
    let core = events::EventBodyCore {
        tenant_id: head.tenant_id,
        entity_kind: events::EntityKind::Product.as_str(),
        entity_id: head.product_id,
        internal_revision: head.internal_revision,
        lifecycle_state: head.lifecycle_state.as_str(),
    };
    events::enqueue_retired(
        outbox,
        runner,
        inputs.product_id,
        events::PRODUCT_RETIRED_PAYLOAD_TYPE,
        events::RetiredEventBody {
            core: &core,
            from_version,
            reason: reason.to_owned(),
            replaced_by: None,
            effective_at: at.to_rfc3339_opts(SecondsFormat::Secs, true),
            must_migrate_by: None,
        },
        inputs.actor_ref,
    )
    .await
    .map_err(|e| HeadActError::from_storage(&e))?;

    let internal_revision = head.internal_revision;
    let body = serde_json::to_value(ProductView::from(head)).map_err(|e| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "render the retired Product: {e}"
        ))))
    })?;
    if let Some(input) = inputs.claim.as_ref() {
        record_idempotency_answer(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            input,
            HEAD_ACT_RESPONSE_STATUS,
            &body,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }
    Ok(HeadActOutcome::Applied {
        internal_revision,
        body,
    })
}

/// `POST /bss-products/v1/products/{id}/retire/cancel`.
///
/// Gate call: `gate.evaluate(GateSubject::entity_publish(EntityRef { tenant_id,
/// entity_kind: Product, entity_id }, InternalRevision::new(expected)),
/// GateMode::Gate)` after the live-op pin, before the write.
///
/// # Errors
///
/// See [`cancel_product_retirement_under_gate`].
async fn cancel_product_retirement(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<CancelRetirementRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    cancel_product_retirement_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        request,
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// # Errors
///
/// As [`cancel_product_retirement`].
async fn cancel_product_retirement_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    request: CancelRetirementRequest,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let act = HeadAct {
        authz_action: crate::authz::actions::WRITE,
        audit_action: CANCEL_RETIRE_AUDIT_ACTION,
        endpoint: cancel_retire_endpoint(product_id),
        payload_digest: cancel_payload_digest(&request),
    };
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;
    let sku_scope = clone_scope_for(
        state,
        enforcer,
        ctx,
        opened.tenant_id,
        opened.actor_ref,
        product_id,
        &crate::authz::resource_types::SKU,
    )
    .await?;
    let result = cancel_retire_in_one_transaction(state, &opened, &sku_scope, gate, request).await;
    answer_cancel_act(state, &opened, CANCEL_RETIRE_AUDIT_ACTION, result).await
}

async fn cancel_retire_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    sku_scope: &AccessScope,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    request: CancelRetirementRequest,
) -> Result<HeadActOutcome, HeadActError> {
    let gate = Arc::clone(gate);
    let inputs = opened.act_inputs();
    let sku_scope = sku_scope.clone();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                let sku_scope = sku_scope.clone();
                let request = request.clone();
                Box::pin(async move {
                    run_cancel_retire(tx, &inputs, &sku_scope, gate.as_ref(), &request).await
                })
            },
        )
        .await
}

async fn run_cancel_retire(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    gate: &(dyn GovernanceGate + Send + Sync),
    request: &CancelRetirementRequest,
) -> Result<HeadActOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    let rows =
        repo::find_live_retire_intents(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
    let Some(intent) = rows.into_iter().next() else {
        return Err(HeadActError::Refused(no_live_retire_intent()));
    };

    let op = GovernedLiveOp {
        kind: request.kind.clone(),
        target: request.target.clone(),
        payload: request.payload.clone(),
        expected_state: request.expected_state.clone(),
    };
    op.apply(&intent.state, || Ok(()))
        .map_err(HeadActError::Refused)?;

    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    repo::supersede_live_intents(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        "retire",
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    let children =
        repo::find_skus_of_product(runner, sku_scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
    for child in children {
        repo::supersede_live_intents(
            runner,
            sku_scope,
            inputs.tenant_id,
            child.sku_id,
            "retire",
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
        let _cleared = repo::clear_replaced_by(
            runner,
            sku_scope,
            inputs.tenant_id,
            child.sku_id,
            child.internal_revision,
            inputs.now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }

    repo::write_eventless_act_audit(
        runner,
        &inputs.scope,
        repo::AuditCommon {
            audit_id: Uuid::now_v7(),
            tenant_id: inputs.tenant_id,
            actor_ref: inputs.actor_ref,
            action: CANCEL_RETIRE_AUDIT_ACTION.to_owned(),
            subject_kind: crate::authz::labels::PRODUCT.to_owned(),
            reason: None,
            correlation_id: crate::infra::events::correlation_id(),
            written_at: inputs.now,
        },
        inputs.product_id,
        Some(head.internal_revision),
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    if let Some(input) = inputs.claim.as_ref() {
        record_idempotency_answer(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            input,
            StatusCode::ACCEPTED,
            &JsonValue::Null,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }

    Ok(HeadActOutcome::Applied {
        internal_revision: head.internal_revision,
        body: JsonValue::Null,
    })
}

async fn resume_product_retirement(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<ResumeRetirementRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let act = HeadAct {
        authz_action: crate::authz::actions::WRITE,
        audit_action: RESUME_RETIRE_AUDIT_ACTION,
        endpoint: format!("/bss-products/v1/products/{product_id}/retire/resume"),
        payload_digest: idempotency::payload_digest(&serde_json::json!({
            "confirmed": request.confirmed
        })),
    };
    let opened = open_head_door(&state, &enforcer, &ctx, product_id, &headers, &act).await?;
    let sku_scope = clone_scope_for(
        &state,
        &enforcer,
        &ctx,
        opened.tenant_id,
        opened.actor_ref,
        product_id,
        &crate::authz::resource_types::SKU,
    )
    .await?;
    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::new(NoMaterialityPolicyGate);
    let result = resume_in_one_transaction(&state, &opened, &sku_scope, &gate, request).await;
    answer_head_act(&state, &opened, RESUME_RETIRE_AUDIT_ACTION, result).await
}

async fn resume_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    sku_scope: &AccessScope,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    request: ResumeRetirementRequest,
) -> Result<HeadActOutcome, HeadActError> {
    let gate = Arc::clone(gate);
    let inputs = opened.act_inputs();
    let sku_scope = sku_scope.clone();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let gate = Arc::clone(&gate);
                let inputs = inputs.clone();
                let sku_scope = sku_scope.clone();
                let request = request.clone();
                Box::pin(async move {
                    run_resume(tx, &inputs, &sku_scope, gate.as_ref(), &request).await
                })
            },
        )
        .await
}

/// @cpt-dod:cpt-cf-bss-products-dod-deferred-intent:p1 — operator resume
/// writes `resolution = children_cleared`.
async fn run_resume(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    sku_scope: &AccessScope,
    gate: &(dyn GovernanceGate + Send + Sync),
    request: &ResumeRetirementRequest,
) -> Result<HeadActOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    confirmation_must_hold(request.confirmed).map_err(HeadActError::Refused)?;

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;
    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    let Some(deferral) = repo::find_unresolved_deferred_retirement(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?
    else {
        return Err(HeadActError::Refused(no_live_retire_intent()));
    };

    let children =
        repo::find_skus_of_product(runner, sku_scope, inputs.tenant_id, inputs.product_id)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
    let states: Vec<LifecycleState> = children.iter().map(|c| c.lifecycle_state).collect();
    if !parent_flip_clears(&states) {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "children", PARENT_FLIP_HELD_REASON);
        return Err(HeadActError::Refused(DomainError::Validation(report)));
    }

    let verdict = gate
        .evaluate(
            // The pin rides the subject since P-D-125 row 52 (strand B,
            // merged 2026-09-04; reconstructed at strand C's merge).
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    let written = repo::resolve_deferred_retirement(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        deferral.cascade_ref,
        DeferralResolution::ChildrenCleared.as_str(),
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if !written {
        return Err(HeadActError::Refused(no_live_retire_intent()));
    }

    repo::write_eventless_act_audit(
        runner,
        &inputs.scope,
        repo::AuditCommon {
            audit_id: Uuid::now_v7(),
            tenant_id: inputs.tenant_id,
            actor_ref: inputs.actor_ref,
            action: RESUME_RETIRE_AUDIT_ACTION.to_owned(),
            subject_kind: crate::authz::labels::PRODUCT.to_owned(),
            reason: Some(DeferralResolution::ChildrenCleared.as_str().to_owned()),
            correlation_id: crate::infra::events::correlation_id(),
            written_at: inputs.now,
        },
        inputs.product_id,
        Some(head.internal_revision),
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    let internal_revision = head.internal_revision;
    let body = serde_json::to_value(ProductView::from(head)).map_err(|e| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "render the resumed Product: {e}"
        ))))
    })?;
    if let Some(input) = inputs.claim.as_ref() {
        record_idempotency_answer(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            input,
            HEAD_ACT_RESPONSE_STATUS,
            &body,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    }
    Ok(HeadActOutcome::Applied {
        internal_revision,
        body,
    })
}

/// The `products_audit_log.action` token every publish refusal is recorded
/// under. Named, not spelled at each call site, so the trail is greppable by
/// one string.
const PUBLISH_AUDIT_ACTION: &str = "publish";

/// [`PUBLISH_AUDIT_ACTION`]'s discard twin.
const DISCARD_AUDIT_ACTION: &str = "discard";

/// [`PUBLISH_AUDIT_ACTION`]'s deprecate twin.
const DEPRECATE_AUDIT_ACTION: &str = "deprecate";

/// [`DEPRECATE_AUDIT_ACTION`]'s reversal twin.
const UNDEPRECATE_AUDIT_ACTION: &str = "undeprecate";

const RETIRE_AUDIT_ACTION: &str = "retire";

const CANCEL_RETIRE_AUDIT_ACTION: &str = "retire.cancel";

const RESUME_RETIRE_AUDIT_ACTION: &str = "retire.resume";

/// The version row this publish freezes: the act's own image, rendered
/// canonically and digested (`inst-fd-publish-freeze`, §4.3, **P-D-33**).
///
/// `published_version` is `N + 1` — the version this act produces, not the
/// one the head currently carries — which is what makes the head-row guard's
/// subquery find this row when the `UPDATE` a statement later asks for it.
/// That number is the **only** post-act value this function needs. P-D-33
/// requires the freeze to be the image the act leaves behind, and it is: the
/// four columns the act moves are all off [`PRODUCT_CONTENT_ROSTER`], so
/// [`product_content`] renders the same bytes from the pre-act head as it
/// would from the post-act one. [`product_content`]'s own doc carries that
/// argument and the day it stops holding.
///
/// `approval_ref` is whatever the gate's verdict named, under **either**
/// mode: `GateAuthorization::approval_ref`, not `approval_to_consume`. The
/// column records which approval stands behind the frozen version; the
/// consume flip is a different question, asked of a different accessor, and
/// there is nothing to spend under the default host.
fn freeze_for(
    inputs: &HeadActInputs,
    head: &ProductRecord,
    authorization: &GateAuthorization,
) -> NewEntityVersion {
    // `N + 1`, the version this act produces. It is read by the row's **key**
    // and by nothing else: `published_version` is not on
    // `PRODUCT_CONTENT_ROSTER`, so the content does not restate it and there
    // is no second copy of this number for the key to disagree with.
    let published_version = head.published_version + 1;
    let rendering = canonical::canonical_rendering(
        &product_content(head),
        canonical::Absence::Null {
            roster: &PRODUCT_CONTENT_ROSTER,
        },
    );
    let content_digest = canonical::content_digest(&rendering);
    NewEntityVersion {
        tenant_id: inputs.tenant_id,
        entity_kind: VersionedEntityKind::Product,
        entity_id: inputs.product_id,
        published_version,
        content: rendering,
        content_digest,
        digest_version: canonical::DIGEST_VERSION,
        approval_ref: authorization.approval_ref().map(ApprovalId::get),
        actor_ref: inputs.actor_ref,
        published_at: inputs.now,
    }
}

/// The publish act itself, **every phase of it on the mutation's own
/// transaction** and in the pipeline's own phase order
/// (`crate::domain::validation::Phase`): the idempotency claim, the
/// precondition, the re-validation, the edge, the governance gate, then the
/// writes.
///
/// The edge sits **before** the gate because `Phase::ordered()` puts `State`
/// ahead of `GovernanceGate`. This door asked the gate first until this fix,
/// which contradicted the order this very sentence claims; the call site
/// carries the argument and the measurement of what the swap changes today.
///
/// # Why every phase is in here, and not half of them outside
///
/// `Phase::Idempotency` runs **before** `Phase::Precondition`, and that
/// ordering is not decorative: a client whose first publish committed but
/// whose response was lost retries with the `If-Match` it still holds — the
/// revision *before* the publish it never learned about. That precondition
/// is stale by construction. A door that judged the precondition first would
/// refuse every such retry `STALE_REVISION` and would never reach the stored
/// answer, which is precisely the case the idempotency store exists for; the
/// store would be inert at this door.
///
/// The claim `INSERT` must join this transaction (**P-D-42**), so "the
/// idempotency phase first" and "the claim inside the mutation" together
/// force every later phase in here too. That is why the precondition
/// comparison, the pipeline re-run and the gate all run on `runner` rather
/// than ahead of it in the handler. It costs nothing — none of them writes —
/// and it buys three properties: the phases run in the order §3.1 states,
/// every refusal rolls back whatever the transaction had already written,
/// and the head each phase judges is the one read **under the write** rather
/// than one read a moment earlier.
///
/// `crate::domain::governance`'s own doc anticipates this: a store-backed
/// host will want "an operand the door already loaded inside its
/// transaction", which is exactly where slice 05's host will find itself.
///
/// # The mode is an argument, not a literal
///
/// `dod-publish-door` (**P-D-30**): *"the door MUST take a gate mode as an
/// explicit argument"*. `mode` is that argument. Under [`GateMode::Gate`]
/// the host looks for a `satisfied` record and consumes it; under
/// [`GateMode::PreAuthorized`] it verifies the named record and consumes
/// nothing, which is what lets `04-lifecycle`'s scheduled-publish runner
/// drive **this** door instead of a second one — a runner forced through
/// `Gate` would meet an already-`consumed` record and fail the run
/// terminally.
///
/// §2's `inst-fd-gate-mode` calls the mode *"an internal door argument,
/// never a wire-visible parameter"*. Those are two clauses, not one: the
/// second constrains where the argument may come **from**, and an earlier
/// revision of this door read it as forbidding the first. It is structurally
/// wire-invisible here rather than conventionally so — [`GateMode`] is
/// reachable from no request DTO, no header reader and no query extractor in
/// this crate, and the only `axum` handler that reaches this function,
/// [`publish_product`], passes the [`GateMode::Gate`] literal. The single
/// in-process entry point that can pass anything else is
/// [`publish_product_under_gate`], which is not routed.
async fn run_publish(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    gate: &(dyn GovernanceGate + Send + Sync),
    mode: GateMode,
    outbox: &crate::infra::broker::EventSink,
) -> Result<HeadActOutcome, HeadActError> {
    // -- Phase 1, idempotency: the claim, and the replay that ends the act
    // before any precondition is judged. --
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    // The head as it stands **under the write**. A miss here is the head
    // vanishing from the caller's scope between the door's own read and this
    // one, and it answers the same bare `404`.
    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    // -- Terminality, which reaches every head write and not only a
    // transition (`inst-fd-terminal`, P-D-25 widened by P-D-32). Asked
    // directly rather than left to `transition::guard` below, because a
    // re-publish takes no edge at all and an edge-keyed check would let
    // exactly this write through. --
    transition::check_head_write(head.lifecycle_state).map_err(HeadActError::Refused)?;

    // -- Phase 2, the precondition (P-D-33, `inst-fd-publish-pin`). The
    // head-row `UPDATE` carries the same comparison in its own filter, and
    // that copy is what decides whether the write lands; this one decides
    // whether the gate is asked at all, since an approval is only usable
    // against the exact revision it pinned. --
    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    // -- Phases 3 to 5, the pipeline re-run (`inst-fd-publish-revalidate`).
    // The phase the run stopped at is not carried onto the wire: the report
    // carries the codes and the per-field detail, and the audit row records
    // one code (P-D-37). --
    if let Some((_phase, report)) = publish_revalidation_pipeline().run(&publish_candidate(&head)) {
        return Err(HeadActError::Refused(revalidation_refusal(&report)));
    }

    // -- The `→ published` edge's registered validators. Read outside the
    // pipeline because `ValidationRule::evaluate` is synchronous: the
    // assignment lives in `products_product_category`, which is 02's table
    // and not this row. --

    // -- Phases 3 to 5 continued, the state phase: the edge, and what the
    // floor says it costs. `published_state_after` decides the `to` side from
    // the row image, the same way the head-row `UPDATE`'s own `CASE` does.
    //
    // **It runs before the gate, and the order is the pipeline's rather than
    // this door's.** `Phase::ordered()` puts `State` ahead of
    // `GovernanceGate`, so an act that is not legal at all must be refused as
    // illegal rather than answered with an approval question: a caller told
    // to seek an approval for an edge the machine does not admit has been
    // sent to obtain something that would not help. This door asked the gate
    // first until this fix, contradicting the phase order its own module doc
    // claims to follow, while `skus::run_publish` and both discard doors had
    // the compliant order from the start.
    //
    // At this commit the reordering changes no answer, and that is measured
    // rather than assumed: `check_head_write` above has already refused every
    // terminal head, and on the three states that survive it
    // (`draft`, `published`, `deprecated`) `published_state_after` yields
    // either the admitted `draft -> published` edge or the same-value
    // diagonal, both of which `transition::guard` admits. So the guard cannot
    // refuse a publish today and the two orders are observationally equal.
    // The order is fixed anyway, because the thing that makes it observable
    // is a *later* slice widening `published_state_after` or the edge list,
    // and a defect that only appears then is one nobody will be looking for.
    // --
    let decision = transition::guard(
        head.lifecycle_state,
        published_state_after(head.lifecycle_state),
    )
    .map_err(HeadActError::Refused)?;

    // -- Phase 6, the registered validators — AFTER the state phase, which is
    // `Phase::ordered()`'s own sequence (shape, state, identity, registered
    // validators, gate). This door made the same argument in the other
    // direction when it put the gate after the state check: an act that is
    // not legal at all must be refused as illegal rather than answered with a
    // question about categories. An earlier revision ran this block BEFORE
    // `transition::guard`, inverting the order for the first registered
    // validator this gear ever had. --
    let has_primary =
        repo::has_primary_category(runner, &inputs.scope, head.tenant_id, head.product_id)
            .await
            .map_err(|e| HeadActError::Db(toolkit_db::DbError::Sea(e.to_db_err())))?;
    if let Some((_phase, report)) =
        published_transition_pipeline().run(&PublishedTransitionSubject {
            has_primary_category: has_primary,
        })
    {
        return Err(HeadActError::Refused(transition_refusal(&report)));
    }

    // -- The same phase, `02`'s other publish-time rule: every localized
    // definition this entity carries values for must carry one at the global
    // coordinate, or the fallback chain runs out for at least one brand
    // (`inst-av-default-locale`). Its own pipeline rather than a rule on the
    // one above, because its subject is the entity's **stored** values and
    // `PublishedTransitionSubject` has no field for them -- the same reason
    // that type exists beside `CreateEntityCandidate`.
    //
    // At publish and never at draft save: a partially-authored draft is
    // legal, which is why this is here and not in `content_save_pipeline`. --
    let carried = carried_definitions(runner, inputs, head.product_id).await?;
    if !carried.carried.is_empty()
        && let Some((_phase, report)) = published_content_pipeline().run(&carried)
    {
        return Err(HeadActError::Refused(DomainError::Validation(report)));
    }

    // @cpt-dod:cpt-cf-bss-products-dod-scope-narrowing:p1 — at publish,
    // naming the falling-out children (P-D-115). The save-door check stays.
    refuse_narrowing_at_publish(runner, inputs, &head).await?;

    // -- Phase 7, the governance gate, inside the door, in the mode this
    // act was entered under (`inst-fd-gate-mode`). `Gate` from every wire
    // surface; `PreAuthorized` only from an in-process caller, which is the
    // seam `04-lifecycle`'s scheduled-publish runner arrives through. The
    // two `Err`s are two different kinds of thing and take two different
    // routes.
    //
    // `evaluate`'s is the host failing to **reach** an answer — a
    // record-store read that failed, say. `crate::domain::governance`'s own
    // contract forbids reporting that as a refusal: it "must not be reported
    // as `APPROVAL_REQUIRED`, which would tell an operator an approval was
    // missing when none was ever looked at", and a refusal would also audit
    // an infrastructure fault as a domain decision and answer it 4xx. So it
    // becomes a `Db` error, this door's internal-failure channel, which
    // rolls the transaction back and answers 5xx. `into_authorization`'s
    // `Err` is the ceremony's own no — `APPROVAL_REQUIRED` — and that one is
    // a refusal.
    //
    // The host branch is **unreachable at this commit**:
    // [`NoMaterialityPolicyGate`] is infallible, so its `evaluate` never
    // answers `Err`. It goes live the moment slice 05 registers a
    // store-backed host, which is why it is routed now rather than when the
    // first operator reads `APPROVAL_REQUIRED` off a failed read.
    // `skus::run_publish` has carried this shape from the start; this door
    // mapped both arms to `Refused` until this fix.
    //
    // The phase is **last**, after the state phase above, which is where
    // `Phase::ordered()` puts it and where `skus::run_publish` and both
    // discard doors already asked it. --
    let subject = EntityRef {
        tenant_id: inputs.tenant_id,
        entity_kind: CatalogEntityKind::Product,
        entity_id: inputs.product_id,
    };
    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(subject, InternalRevision::new(inputs.expected)),
            mode,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    let authorization = verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    // -- a. Freeze the post-act image, at `published_version + 1`. --
    repo::insert_entity_version(
        runner,
        &inputs.scope,
        freeze_for(inputs, &head, &authorization),
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    // -- b. Then exactly one head-row `UPDATE`. --
    let write = repo::publish_product_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        inputs.expected,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if write == HeadWrite::Unmatched {
        // An error rather than an outcome, and the whole reason
        // [`HeadActError`] exists: this rolls the freeze back. An `Ok` here
        // would commit a frozen version for a publish that never landed.
        return Err(classify_unmatched_publish(runner, inputs).await);
    }

    // -- c. The approval-invalidation hook, where the floor says this edge
    // fires one. On `draft -> published` it does not — that is
    // `ApprovalInvalidation::Skip`, this transaction being the one that
    // consumes the approval — so this call is a no-op today. It is here, and
    // it reads `transition::invalidation_for`'s answer, so that the decision
    // stays `crate::domain::transition`'s rather than becoming a fact
    // hard-coded here. That function is the single home the two doors' own
    // copies of this fold were owed: `ADMITTED_EDGES` and `GATED_EDGES`
    // already live there, and the `NotATransition` arm is a case of the same
    // rule rather than a case outside it. --
    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

    // -- d. Then the event, and the stored answer. --
    announce_and_answer(runner, outbox, inputs, Announcement::Published).await
}

/// The discard act itself, on [`run_publish`]'s terms exactly: every phase on
/// the mutation's own transaction, the idempotency claim first, and the head
/// read under the write.
///
/// The one phase a discard does **not** have is as deliberate as the ones it
/// does: there is **no pipeline re-run**, because nothing is being published
/// and `inst-fd-publish-revalidate` is the publish act's clause.
///
/// # The governance-gate phase runs here, and passes trivially
///
/// §3.1's `inst-fd-pipeline-gate-phase` says the phase *"runs at every
/// mutating door and passes trivially where the act is ungated
/// (**P-D-34**)"*, and §1.1 makes governance *"a registered gate phase
/// inside the pipeline, hosting any gated act — publish or transition alike
/// (**P-D-30**) — not a separate path around it"*.
/// [`crate::domain::validation::Phase::GovernanceGate`]'s own doc carries
/// the same rule. So the phase is asked here, in [`GateMode::Gate`], and the
/// gear's default host authorizes naming no record: a discard of a
/// never-published draft consumes no approval and today requires none.
///
/// Behaviourally that is the same answer as not asking. It stops being the
/// same answer the moment slice 05 registers a ceremony on a transition,
/// which is the case the phase exists to make reachable **without reopening
/// every door** — and the door that had to be reopened would be exactly this
/// one. An earlier revision cited `inst-fd-governance-gate` as authority for
/// skipping the phase; that instruction is about the publish door and does
/// not govern this question.
///
/// The mode is the [`GateMode::Gate`] literal rather than an argument, and
/// the asymmetry with [`run_publish`] is measured, not forgotten:
/// `dod-publish-door` requires the *publish* door to take the mode
/// explicitly because `04-lifecycle`'s scheduled-publish runner needs
/// [`GateMode::PreAuthorized`] to drive it. No scheduled or cascaded
/// **discard** exists in any slice, so there is no caller for a
/// pre-authorized discard and no instruction asking for one. The host is
/// still a parameter, for [`discard_product_under_gate`]'s stated reason.
async fn run_discard(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    gate: &(dyn GovernanceGate + Send + Sync),
    outbox: &crate::infra::broker::EventSink,
) -> Result<HeadActOutcome, HeadActError> {
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    // The edge. `transition::guard` runs terminality first and the edge list
    // second, so a `retired` head is `ENTITY_TERMINAL` while a `published`
    // one is `ILLEGAL_TRANSITION` — two refusals for two different reasons,
    // which a single "is this legal" test would have collapsed into one.
    let decision = transition::guard(head.lifecycle_state, LifecycleState::Discarded)
        .map_err(HeadActError::Refused)?;

    // -- Phase 7, the governance gate: the pipeline's last phase, asked here
    // as it is at every other mutating door (`inst-fd-pipeline-gate-phase`).
    // It sits after the edge because `Phase::ordered()` puts `State` before
    // `GovernanceGate`, so a `published` head is `ILLEGAL_TRANSITION` rather
    // than an approval question. The two `Err` routes are `run_publish`'s
    // and carry its reasoning: a host that could not *reach* an answer is
    // infrastructure and answers 5xx, while the ceremony's own `no` is
    // `APPROVAL_REQUIRED`. --
    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    // The authorization is collapsed into the door's control flow and then
    // dropped, and that is the whole of what an ungated act does with a
    // trivial `yes`: a discard freezes no `products_entity_version` row, so
    // the `approval_ref` the verdict may name has no column to reach. The
    // day slice 05 gates a transition, the refusal arm above is already
    // wired and only the record's destination is new.
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    let write = repo::discard_product_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        inputs.expected,
        inputs.now,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if write == HeadWrite::Unmatched {
        return Err(classify_unmatched_discard(runner, inputs).await);
    }

    // A discard consumes no approval, so the floor says its edge fires the
    // hook — read off `transition::guard`'s own answer, not decided here.
    fire_invalidation_hook(runner, inputs, transition::invalidation_for(decision)).await?;

    announce_and_answer(runner, outbox, inputs, Announcement::Discarded).await
}

/// `POST /products/{id}/publish`.
///
/// See this module's doc, "The publish door", for the pipeline in order,
/// what this writes, and the three clauses of `dod-publish-door` this slice
/// cannot close.
async fn publish_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    publish_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        // This call site fixes the two things a wire request may not
        // choose. *Which host answers*: the only host the gear has until
        // slice 05 registers a materiality policy. And *which mode*:
        // `GateMode::Gate`, a literal here and nowhere else, which is
        // `inst-fd-gate-mode`'s "never a wire-visible parameter" and the
        // owner's call of 2026-08-27.
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
        GateMode::Gate,
    )
    .await
}

/// The publish door, with its governance host **and its gate mode** as
/// explicit arguments — the in-process entry point every other caller of
/// this door uses.
///
/// # Why the mode is a parameter
///
/// `dod-publish-door` (**P-D-30**): *"the door MUST take a gate mode as an
/// explicit argument"*, so that `04-lifecycle`'s scheduled-publish runner
/// can drive **this** door in [`GateMode::PreAuthorized`] rather than force
/// a second publish path into existence. An earlier revision of this
/// function fixed the mode at [`GateMode::Gate`] inside [`run_publish`] and
/// cited `inst-fd-gate-mode` as requiring that. It does not: the
/// instruction's clause is *"an internal door argument, never a wire-visible
/// parameter"*, and reading the second half as a prohibition on the first
/// left [`GateMode::PreAuthorized`] a type with no call path at all.
///
/// **Never wire-visible, structurally rather than by convention.**
/// [`GateMode`] implements no `Deserialize`: its derive list is `Debug`,
/// `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`, and `#[domain_model]` adds
/// only a marker trait impl — measured at
/// `toolkit_macros`'s own expansion, 2026-08-30. So the type **cannot** be
/// parsed out of a body, a query string or a header at all, and adding it to
/// a DTO would not compile without someone deliberately deriving the trait.
/// Beside that, it appears in no request DTO, header reader or query
/// extractor in this crate; the only `axum` handler that reaches this
/// function is [`publish_product`], and it passes the [`GateMode::Gate`]
/// literal. This function is not routed, so the set of callers that can pass
/// anything else is the set of Rust call sites in this crate — which is the
/// bound `crate::domain::governance`'s own module doc states.
///
/// # Why the host is a parameter
///
/// The **host** is a parameter because the gear's only host,
/// [`NoMaterialityPolicyGate`], never refuses under `Gate` — it authorizes,
/// naming no record, because no materiality policy is registered. That makes
/// the `APPROVAL_REQUIRED` branch unreachable through [`publish_product`]
/// and therefore untestable at the door, and an untested refusal path is one
/// that quietly stops working. This seam is also the one slice 05 fills: its
/// host arrives here as another `Arc<dyn GovernanceGate + Send + Sync>`, not
/// as a rewrite of this function.
async fn publish_product_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    mode: GateMode,
) -> Result<Response, CanonicalError> {
    let act = HeadAct {
        authz_action: crate::authz::actions::PUBLISH,
        audit_action: PUBLISH_AUDIT_ACTION,
        endpoint: publish_endpoint(product_id),
        payload_digest: bodiless_payload_digest(),
    };
    // `actor_ref`, authorization, the key, the precondition header and the
    // head read — the phases that precede the act.
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;

    // The act: every remaining phase, then the freeze, the one head-row
    // `UPDATE` and the event, on one transaction.
    let result = publish_in_one_transaction(state, &opened, gate, mode).await;

    // The answer, the replay, or the audited refusal.
    answer_head_act(state, &opened, PUBLISH_AUDIT_ACTION, result).await
}

/// `POST /products/{id}/discard`.
///
/// See this module's doc, "The discard door", for the legality rule, the
/// reservations this releases without a statement of its own, and why the
/// gate is not asked.
///
/// # The grant is `write`, not `discard`
///
/// §2 narrates this door under `product × discard`, and `crate::authz` does
/// not declare a `discard` action: `05-governance.md` §3.2's own RBAC
/// catalog rows the same door under `product × write`, and that document's
/// open-items list records the contradiction as unresolved with the decision
/// owned by that slice. This door therefore gates on the action the
/// normative catalog table currently grants it. When 05 settles the
/// question, the change is one constant here and one in `crate::authz`.
async fn discard_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    discard_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        // The same literal [`publish_product`] passes, and for the same
        // reason: the only host the gear has, and no wire input choosing it.
        &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
    )
    .await
}

/// The discard door, with its governance host as an explicit argument —
/// [`publish_product_under_gate`]'s twin, minus the mode.
///
/// # Why the host is a parameter here too
///
/// The gate phase runs on this door (`inst-fd-pipeline-gate-phase`; see
/// [`run_discard`]), and the gear's only host never refuses under
/// [`GateMode::Gate`]. That makes the phase's refusal arm unreachable
/// through [`discard_product`] and therefore untestable at the door — the
/// identical argument [`publish_product_under_gate`] makes — and a phase
/// nothing can exercise is one a reader cannot tell from a phase that is
/// absent. This seam is also where slice 05's host arrives the day it gates
/// a transition.
///
/// The **mode** is not a parameter, and [`run_discard`]'s own doc measures
/// that asymmetry against `dod-publish-door`: the explicit-mode requirement
/// is the publish door's, and no slice schedules or cascades a discard.
///
/// # Errors
///
/// Every refusal this door raises, each audited; the bare `404`; the `500` a
/// storage or gate-host failure raises.
async fn discard_product_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
) -> Result<Response, CanonicalError> {
    let act = HeadAct {
        authz_action: crate::authz::actions::WRITE,
        audit_action: DISCARD_AUDIT_ACTION,
        endpoint: discard_endpoint(product_id),
        payload_digest: bodiless_payload_digest(),
    };
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;
    let result = discard_in_one_transaction(state, &opened, gate).await;
    answer_head_act(state, &opened, DISCARD_AUDIT_ACTION, result).await
}

/// The `products_audit_log.action` token every **save** refusal is recorded
/// under, beside [`PUBLISH_AUDIT_ACTION`] and [`DISCARD_AUDIT_ACTION`].
///
/// `skus::SAVE_AUDIT_ACTION` is the SKU door's identical constant, and the
/// two must stay equal: an operator filtering the trail by `action = 'save'`
/// is asking one question of both entity kinds.
const SAVE_AUDIT_ACTION: &str = "save";

/// The concrete resource path a save claims its idempotency key under
/// (**P-D-42**), on [`publish_endpoint`]'s terms: the id is in the path, so
/// two saves of two Products under one client key are two keys.
///
/// It is the **same** string [`router`] registers the `PATCH` at, with the
/// `{id}` template resolved — a save has no act suffix to tell it from a
/// read, because the method already does.
fn save_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}")
}

/// `PATCH /bss-products/v1/products/{id}` request body: **the named field
/// set, and nothing around it**.
///
/// # Why this is a map and not five `Option` fields
///
/// A `PATCH` has to tell "the caller did not mention this field" from "the
/// caller sent this field", which five `Option`s do. What they cannot do is
/// tell either from **"the caller sent a field this door does not know"**:
/// `#[toolkit_macros::api_dto(request)]`'s expansion adds no
/// `#[serde(deny_unknown_fields)]` (`CreateProductRequest`'s own doc measures
/// that), so an unrecognized key on a typed DTO is *silently dropped*. The
/// save door is the one door where that is not merely untidy: P-D-50's
/// fail-closed rule exists precisely to refuse a published-state column
/// carrying no bucket tag, and a DTO that drops the key never refuses
/// anything — the rule would be unreachable, and the drift it exists to
/// surface would present as a silent `200` for a field nobody wrote.
///
/// So every key the caller sent arrives here, and every key is routed
/// ([`route_product_save`]). The cost, stated plainly: this `DTO` carries no
/// per-field schema, so the `OpenAPI` document describes the body in the
/// operation's own description rather than in a generated object. That is a
/// documentation loss paid for a rule; the alternative was a rule that could
/// not fire.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct SaveProductRequest {
    /// Every field the request named, keyed as the caller spelled it.
    ///
    /// `#[serde(flatten)]`, so the wire shape is the flat object a `PATCH`
    /// caller expects — `{"name": "..."}`, not `{"fields": {"name": "..."}}`.
    /// A [`BTreeMap`](std::collections::BTreeMap) rather than a hash map so
    /// the iteration order is the field names' own: the digest this door
    /// hashes for its idempotency key is taken over this object, and a
    /// key-order-dependent digest would make one request hash two ways
    /// between processes.
    #[serde(flatten)]
    pub fields: std::collections::BTreeMap<String, JsonValue>,
}

/// The digest of one parsed save request, as the claim is taken against
/// (**P-D-34**: *"the canonical rendering of the parsed request, excluding
/// the precondition header"*).
///
/// The operand is the field set itself, rendered through
/// `crate::domain::idempotency::payload_digest` exactly as
/// [`payload_digest`] renders a create's. Nothing about the transport enters
/// it — no header, no correlation id, and structurally no `If-Match`, this
/// function never being handed the headers — which is the clause that keeps a
/// client's own retry from being refused `IDEMPOTENCY_CONFLICT` the moment a
/// neighbour's write moved the head.
///
/// It is emphatically **not** [`bodiless_payload_digest`]: two different
/// saves of one head under one client key must be a conflict, not a replay of
/// each other. That is the whole reason [`HeadAct`] carries the digest.
fn save_payload_digest(request: &SaveProductRequest) -> Vec<u8> {
    idempotency::payload_digest(&JsonValue::Object(
        request
            .fields
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    ))
}

/// A field the save door **accepts on the wire**, and the physical column it
/// names.
///
/// This is a second registry beside `crate::domain::bucket`'s, and the two
/// answer different questions on purpose. That one answers *what class is
/// this column in*; this one answers *may a caller author this column at
/// all*. The gap between them is real and not hypothetical:
/// `name_normalized` is a **bucket-iii column** — §4.1 puts it there as
/// *"the same field's index operand"* — so routing a wire field of that name
/// straight through `bucket::classify` would admit a caller-written index
/// operand that no longer matches the name it is derived from. It is derived
/// here, from `name`, and never accepted; `bucket.rs`'s own doc states the
/// division ("mapping a request field to a column is the door's job, done
/// before it asks").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProductSaveField {
    /// Bucket i (§4.1): re-branding moves the row into a different
    /// `(tenant_id, brand_id, name_normalized)` scope.
    BrandId,
    /// Bucket i: the external mapping code, under AC #1's *"under the same
    /// rules"* as `skuCode`.
    ProductCode,
    /// Bucket iii: writes `name` **and** `name_normalized`, which is why the
    /// repository takes the pair as one value (`repo::SavedName`).
    Name,
    /// Bucket iii, in both directions.
    RegionScope,
    /// Bucket iii, in both directions.
    BrandScope,
}

impl ProductSaveField {
    /// The wire field this is, or `None` where the caller named something
    /// this door does not author — which is a refusal
    /// ([`unroutable_product_field`]), never a silent drop.
    fn from_wire(field: &str) -> Option<Self> {
        match field {
            "brand_id" => Some(Self::BrandId),
            "product_code" => Some(Self::ProductCode),
            "name" => Some(Self::Name),
            "region_scope" => Some(Self::RegionScope),
            "brand_scope" => Some(Self::BrandScope),
            _ => None,
        }
    }

    /// The physical column, as `products_product` spells it and as
    /// `crate::domain::bucket`'s registry keys on it.
    const fn column(self) -> &'static str {
        match self {
            Self::BrandId => "brand_id",
            Self::ProductCode => "product_code",
            Self::Name => "name",
            Self::RegionScope => "region_scope",
            Self::BrandScope => "brand_scope",
        }
    }
}

/// One save field's parsed value — the Shape phase's output, before the
/// State phase has said whether the column may be written at all.
///
/// Parsed and routed in two passes rather than one because
/// `crate::domain::validation::Phase::ordered()` puts `Shape` **before**
/// `State`: a body carrying a malformed value and an unroutable field is a
/// `VALIDATION`, not an `ILLEGAL_FIELD_MUTATION`, and a single pass would
/// answer whichever field happened to come first.
enum ProductSaveValue {
    /// A parsed `brand_id`.
    BrandId(Uuid),
    /// A `product_code`, or the request to clear it.
    ProductCode(NullableText),
    /// A `name` and the normalization derived from it.
    Name(SavedName),
    /// A `region_scope`.
    RegionScope(String),
    /// A `brand_scope`.
    BrandScope(String),
}

/// The **content** half of a save payload: rows in `02`'s tables, never
/// columns on the head.
///
/// # Why this is not two more `ProductSaveField` variants
///
/// [`ProductSaveField::column`] maps every variant to a physical column of
/// `products_product`, and `crate::domain::bucket`'s registry keys on that
/// column. Category assignments and attribute values have **no column** —
/// they are rows in `products_product_category` and
/// `products_attribute_value` — so routing them through
/// [`route_product_save`] would ask the bucket registry for a tag that cannot
/// exist, and `bucket::classify` refuses an untagged column rather than
/// defaulting. They are parsed apart and never enter the head save.
///
/// # `None` and `Some(empty)` are different acts
///
/// A payload that names no `categories` key leaves the assignment set alone;
/// one that sends `"categories": []` **clears** it. Collapsing the two would
/// make an unfiled Product unreachable through the door — a `PATCH` is a
/// per-key merge and the empty list is the only way to say *"none"*.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ContentSavePayload {
    /// The whole assignment set, replace-shaped. `None` is *untouched*.
    categories: Option<Vec<(Uuid, AssignmentRole)>>,
    /// The values to write. `None` is *untouched*; an entry is an upsert at
    /// its coordinate.
    attributes: Option<Vec<AttributeWrite>>,
}

impl ContentSavePayload {
    /// Whether the payload asks for any content write at all.
    const fn is_empty(&self) -> bool {
        self.categories.is_none() && self.attributes.is_none()
    }
}

/// One attribute value the payload writes, keyed as the caller spelled it.
///
/// The definition is named by **key** rather than by id, because that is what
/// an author knows and what every refusal quotes back
/// (`ATTRIBUTE_DEFINITION_UNKNOWN` names the key). The door resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttributeWrite {
    /// The tenant-unique definition key.
    pub(crate) key: String,
    /// `""` is absent, not null (**P-D-88** arm 2).
    pub(crate) locale: String,
    /// `""` is absent.
    pub(crate) region: String,
    /// `""` is absent.
    pub(crate) brand: String,
    /// The value.
    pub(crate) value: String,
}

/// The two wire keys the content half owns.
///
/// Named once so [`parse_product_save`] can skip them without repeating the
/// literals: a key this list forgets becomes an `unroutable_product_field`
/// refusal, which is fail-closed and therefore safe, but silently drops a
/// feature.
const CONTENT_SAVE_KEYS: [&str; 2] = ["categories", "attributes"];

/// Parse the content half, refusing anything it cannot read.
///
/// Every refusal here is `VALIDATION` with the field named, because the
/// operand is the caller's own body — the same reasoning
/// [`check_saved_shape`]'s doc gives for preferring it over
/// `INCOMPLETE_ENTITY`.
///
/// # Errors
///
/// [`DomainError::Validation`] naming each malformed entry.
fn parse_content_save(request: &SaveProductRequest) -> Result<ContentSavePayload, DomainError> {
    let mut payload = ContentSavePayload::default();
    let mut violations = Vec::new();

    if let Some(raw) = request.fields.get("categories") {
        match parse_categories(raw) {
            Ok(set) => payload.categories = Some(set),
            Err(detail) => violations.push(("categories".to_owned(), detail)),
        }
    }
    if let Some(raw) = request.fields.get("attributes") {
        match parse_attributes(raw) {
            Ok(set) => payload.attributes = Some(set),
            Err(detail) => violations.push(("attributes".to_owned(), detail)),
        }
    }

    if violations.is_empty() {
        Ok(payload)
    } else {
        Err(shape_refusal(violations))
    }
}

/// `[{"categoryId": "…", "role": "primary"}, …]`.
fn parse_categories(raw: &JsonValue) -> Result<Vec<(Uuid, AssignmentRole)>, String> {
    let items = raw
        .as_array()
        .ok_or_else(|| "categories must be an array".to_owned())?;
    let mut set = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("categories[{index}] must be an object"))?;
        let id = object
            .get("categoryId")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| format!("categories[{index}].categoryId is required"))?;
        let category_id = Uuid::parse_str(id)
            .map_err(|_| format!("categories[{index}].categoryId is not a uuid"))?;
        let role_text = object
            .get("role")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| format!("categories[{index}].role is required"))?;
        // Parsed fail-closed: `uq_products_product_category_primary` is keyed
        // on the literal `'primary'`, so a spelling the roster does not admit
        // would write a row the index cannot see.
        let role = AssignmentRole::parse(role_text)
            .ok_or_else(|| format!("categories[{index}].role must be `primary` or `secondary`"))?;
        set.push((category_id, role));
    }
    Ok(set)
}

/// `[{"key": "…", "value": "…", "locale"?: "…", "region"?: "…", "brand"?: "…"}, …]`.
///
/// The three coordinates default to `""`, which is P-D-88 arm 2's **absence**
/// and the global coordinate's own spelling — so a payload naming only `key`
/// and `value` writes the global value, which is the one
/// `inst-av-default-locale` makes mandatory at publish.
pub(crate) fn parse_attributes(raw: &JsonValue) -> Result<Vec<AttributeWrite>, String> {
    let items = raw
        .as_array()
        .ok_or_else(|| "attributes must be an array".to_owned())?;
    let mut set = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item
            .as_object()
            .ok_or_else(|| format!("attributes[{index}] must be an object"))?;
        let coordinate = |name: &str| -> Result<String, String> {
            match object.get(name) {
                None | Some(JsonValue::Null) => Ok(String::new()),
                Some(JsonValue::String(text)) => Ok(text.clone()),
                Some(_) => Err(format!("attributes[{index}].{name} must be a string")),
            }
        };
        set.push(AttributeWrite {
            key: object
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| format!("attributes[{index}].key is required"))?
                .to_owned(),
            locale: coordinate("locale")?,
            region: coordinate("region")?,
            brand: coordinate("brand")?,
            value: object
                .get("value")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| format!("attributes[{index}].value is required"))?
                .to_owned(),
        });
    }
    Ok(set)
}

/// The `Shape` phase's output: every recognized field with its parsed value,
/// and every field name this door does not author.
///
/// A named alias because the pair is returned across two functions and
/// `clippy::type_complexity` is right that the bare tuple is unreadable at a
/// signature. The two halves travel together because the `State` phase needs
/// both: the unrecognized names are refused first (the fail-closed miss), and
/// the parsed ones are routed after.
type ProductSaveFields = (Vec<(ProductSaveField, ProductSaveValue)>, Vec<String>);

/// Build the `VALIDATION` refusal one or more shape violations ride.
fn shape_refusal(violations: Vec<(String, String)>) -> DomainError {
    let mut report = ValidationReport::new();
    for (subject, detail) in violations {
        report.violate(NameShapeRule::CODE, &subject, &detail);
    }
    DomainError::Validation(report)
}

/// Read a save field's `JSON` value as a string, or name the shape violation.
fn expect_string(field: &str, value: &JsonValue) -> Result<String, (String, String)> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        (
            field.to_owned(),
            format!("{field} must be a JSON string on this door"),
        )
    })
}

/// Parse one recognized save field's value (`Phase::Shape`).
fn parse_product_value(
    field: ProductSaveField,
    value: &JsonValue,
) -> Result<ProductSaveValue, (String, String)> {
    let wire = field.column();
    match field {
        ProductSaveField::BrandId => {
            let raw = expect_string(wire, value)?;
            Uuid::parse_str(&raw)
                .map(ProductSaveValue::BrandId)
                .map_err(|_| (wire.to_owned(), "brand_id must be a UUID".to_owned()))
        }
        // `null` clears the column, and a blank-after-trim value clears it
        // too — `uq_products_product_code` is partial on
        // `product_code IS NOT NULL`, so an untrimmed `""` would be a real,
        // reservable value that two callers sending an unset field as `""`
        // would collide on. That is the create door's own F-2 fix, applied to
        // the door that can also *remove* a code.
        ProductSaveField::ProductCode => {
            if value.is_null() {
                return Ok(ProductSaveValue::ProductCode(NullableText::Clear));
            }
            let raw = expect_string(wire, value)?;
            let trimmed = raw.trim();
            Ok(ProductSaveValue::ProductCode(if trimmed.is_empty() {
                NullableText::Clear
            } else {
                NullableText::Set(trimmed.to_owned())
            }))
        }
        // The stored value is **trimmed**, which is `create_product`'s own
        // rule (`let trimmed_name = raw_name.trim().to_owned()`) reaching the
        // only other door that writes the column. Uniqueness is unaffected --
        // `name::normalize` trims and collapses whatever it is handed -- so
        // what this buys is that one operator-facing value does not depend on
        // which door wrote it: `"  Fibre  "` is `Fibre` from the create door
        // and would otherwise be `  Fibre  ` from this one. The divergence
        // would outlive the request, too: the read door serves the stored
        // value and the next publish freezes it verbatim into
        // `products_entity_version` content, so it would land in a
        // `content_digest`.
        ProductSaveField::Name => {
            let raw = expect_string(wire, value)?;
            let trimmed = raw.trim();
            let normalized = name::normalize(trimmed);
            Ok(ProductSaveValue::Name(SavedName {
                value: trimmed.to_owned(),
                normalized,
            }))
        }
        // Both scope columns are **parsed** and not merely read as strings,
        // which is the create door's own rule (`skus::scope_input_from_payload`)
        // reaching the door that can also *change* a stored scope. A value
        // carrying an empty token — `","`, `"eu,,us"` — is refused rather
        // than silently filtered, and the refusal is load-bearing beyond this
        // row: `skus::recheck_parent_containment` parses the **parent
        // Product's** stored scope on every child publish and answers a `500`
        // where it does not parse, so a Product save that stored one would
        // turn a caller's save into an operator alarm on a different
        // entity's door. The parsed value is discarded and the raw string
        // stored, the column holding the caller's own spelling; what the
        // parse buys is the refusal.
        ProductSaveField::RegionScope | ProductSaveField::BrandScope => {
            let raw = expect_string(wire, value)?;
            if ResolvedScope::parse(&raw).is_err() {
                return Err((
                    wire.to_owned(),
                    format!("{wire} contains an empty value between separators"),
                ));
            }
            Ok(match field {
                ProductSaveField::RegionScope => ProductSaveValue::RegionScope(raw),
                _ => ProductSaveValue::BrandScope(raw),
            })
        }
    }
}

/// The `Phase::Shape` pass over the whole request: every recognized field
/// parsed, every unrecognized name collected for the `State` phase below.
///
/// Both halves are collected before either is answered, so a body with two
/// malformed values reports both rather than the first — which is what
/// `ValidationReport` is for (P-D-37: the caller receives every violation,
/// the audit row records one code).
///
/// # Errors
///
/// [`DomainError::Validation`] naming each malformed field.
fn parse_product_save(request: &SaveProductRequest) -> Result<ProductSaveFields, DomainError> {
    let mut parsed = Vec::new();
    let mut unrecognized = Vec::new();
    let mut violations = Vec::new();
    for (field, value) in &request.fields {
        // The content half is parsed by `parse_content_save` and must not be
        // reported unroutable here. Skipped by name rather than by a second
        // `from_wire` arm, because a `ProductSaveField` variant would owe
        // `column()` a physical column and neither of these has one.
        if CONTENT_SAVE_KEYS.contains(&field.as_str()) {
            continue;
        }
        match ProductSaveField::from_wire(field) {
            Some(known) => match parse_product_value(known, value) {
                Ok(parsed_value) => parsed.push((known, parsed_value)),
                Err(violation) => violations.push(violation),
            },
            None => unrecognized.push(field.clone()),
        }
    }
    if violations.is_empty() {
        Ok((parsed, unrecognized))
    } else {
        Err(shape_refusal(violations))
    }
}

/// The refusal a field name this door does not author is answered with.
///
/// The registry is asked even though the door has already decided to refuse,
/// and the answer decides *which* refusal:
///
/// - **No registry row** — P-D-50's fail-closed miss, and
///   `bucket::classify`'s own `ILLEGAL_FIELD_MUTATION` naming the entity and
///   the column. This is the case the rule exists for: a published-state
///   column added without a bucket tag.
/// - **A registry row this door does not accept from a caller** — the
///   mechanical and row-identity columns (`Outside`), `cloned_from`'s
///   `CreateOnly` when slice 11 lands it, and `name_normalized`, whose value
///   is derived from `name` and never authored. Also
///   `ILLEGAL_FIELD_MUTATION`, because it is the same kind of answer — a
///   write the head door may not take — and the reason says which class it
///   was so an operator can tell an unregistered column from a
///   deliberately-unwritable one.
fn unroutable_product_field(field: &str) -> DomainError {
    match bucket::classify(CatalogEntityKind::Product, field) {
        Err(miss) => miss,
        Ok(_) => DomainError::IllegalFieldMutation(format!(
            "Product column {field} is not authored through this door: it is written by the gear \
             itself or derived from another field, and no save may name it"
        )),
    }
}

/// The refusal a bucket-i write after first publish is answered with
/// (`inst-fd-bucket-i-refusal`).
///
/// The numeral in the message is read off
/// [`bucket::FieldBucket::tag`] rather than spelled here, so the string a
/// caller reads and the registry it was refused by cannot come to name
/// different buckets. That accessor exists for exactly this: the design's own
/// roman numeral, in one place.
fn structural_after_publish(kind: &str, field: &str) -> DomainError {
    let tag = bucket::FieldBucket::Structural.tag();
    DomainError::IllegalFieldMutation(format!(
        "{kind} {field} is a bucket-{tag} identity column: it is writable only before first \
         publish, and a mis-set identity on a published entity is corrected by retire-and-clone \
         rather than by a write"
    ))
}

/// The refusal a bucket-ii write after first publish is answered with
/// (`inst-fd-bucket-ii-refusal`).
///
/// **It names slice 07's correction door and does not forward to it.** The
/// instruction is explicit that the head door refuses rather than forwards —
/// one door, one effect — so a caller is told where the act belongs and
/// re-sends it there, rather than having this door quietly perform a
/// differently-governed act on its behalf.
///
/// **No Product column carries bucket ii**, so this arm is unreachable on
/// this door. Bucket ii is no longer empty gear-wide — 03 registered
/// `metering_unit` and `usage_type_ref` on `products_sku`, where the SKU
/// door's twin of this arm is now reachable and probed — but the Product
/// table has no member and the correction door that would write one is
/// still slice 07's. It is built rather than deferred because the
/// door routes by tag: an arm that appeared only when its first column landed
/// would be a second change to this door on the day slice 07 arrives, and the
/// reason string it needs is this instruction's, not that slice's.
fn correctable_after_publish(kind: &str, field: &str) -> DomainError {
    let tag = bucket::FieldBucket::Correctable.tag();
    DomainError::IllegalFieldMutation(format!(
        "{kind} {field} is a bucket-{tag} correctable column: after first publish it is writable \
         only through the correction door (POST .../corrections, slice 07), which this door \
         names rather than forwards to"
    ))
}

/// Route one parsed field through `crate::domain::bucket` and fold it into
/// `save` — the `Phase::State` half.
///
/// # Errors
///
/// [`HeadActError::Refused`] for every bucket refusal, and
/// [`HeadActError::Db`] — the gear's internal channel, a `500` — for
/// [`bucket::FieldClass::Outside`]. That arm is **structurally unreachable**:
/// [`ProductSaveField::from_wire`] admits five wire names and every one of
/// their columns is bucket-tagged, so a value reaching it means this door's
/// own column table and the registry disagree. Its provenance is therefore
/// the gear's, not the caller's, which is exactly the test
/// `RepoError::CorruptRow`'s own doc states for choosing between the two
/// channels: a request-borne value refused through the internal channel is a
/// `500` plus a false operator alarm, and a **gear**-borne contradiction
/// reported as a caller refusal is the opposite lie.
fn route_product_field(
    head: &ProductRecord,
    field: ProductSaveField,
    value: ProductSaveValue,
    save: &mut repo::ProductHeadSave,
) -> Result<(), HeadActError> {
    let column = field.column();
    let class =
        bucket::classify(CatalogEntityKind::Product, column).map_err(HeadActError::Refused)?;
    let published = head.published_version > 0;
    match class {
        bucket::FieldClass::Bucket(bucket::FieldBucket::Structural) if published => {
            return Err(HeadActError::Refused(structural_after_publish(
                "Product", column,
            )));
        }
        bucket::FieldClass::Bucket(bucket::FieldBucket::Correctable) if published => {
            return Err(HeadActError::Refused(correctable_after_publish(
                "Product", column,
            )));
        }
        // `cloned_from`'s class: stricter than bucket-i, admitted in no
        // `UPDATE` at all rather than merely in none after first publish
        // (§4.1). No column carries it until slice 11; the arm is built for
        // the reason `correctable_after_publish` states for its own.
        bucket::FieldClass::CreateOnly => {
            return Err(HeadActError::Refused(DomainError::IllegalFieldMutation(
                format!(
                    "Product {column} is create-only: it is writable in the creating statement \
                     and in no update at all, so the lineage stays evidence rather than a claim"
                ),
            )));
        }
        bucket::FieldClass::Outside(reason) => {
            return Err(HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the save door's wire field {column} resolves to a column outside \
                 the bucket scheme ({reason:?}); the door's own field table and the registry \
                 disagree"
            )))));
        }
        bucket::FieldClass::Bucket(_) => {}
    }

    match value {
        ProductSaveValue::BrandId(brand_id) => save.brand_id = Some(brand_id),
        ProductSaveValue::ProductCode(code) => save.product_code = Some(code),
        ProductSaveValue::Name(name) => save.name = Some(name),
        ProductSaveValue::RegionScope(scope) => save.region_scope = Some(scope),
        ProductSaveValue::BrandScope(scope) => save.brand_scope = Some(scope),
    }
    Ok(())
}

/// Route **every** field the request carries, and only then hand back the
/// columns to write (`Phase::State`).
///
/// The whole-request discipline is the point: a `PATCH` half-applied because
/// its fourth field was refused would leave the head carrying part of a
/// request the caller was told had failed — a worse outcome than the refusal,
/// and one no status assertion would catch. Nothing here writes; the single
/// `UPDATE` is [`repo::save_product_head`]'s, downstream of this returning
/// `Ok`.
///
/// Unrecognized names are refused **before** the tagged ones are routed, so a
/// body mixing one of each answers the miss rather than a bucket rule — the
/// miss being the fail-closed posture P-D-50 asks for.
///
/// # Errors
///
/// See [`route_product_field`] and [`unroutable_product_field`].
fn route_product_save(
    head: &ProductRecord,
    parsed: Vec<(ProductSaveField, ProductSaveValue)>,
    unrecognized: &[String],
) -> Result<repo::ProductHeadSave, HeadActError> {
    if let Some(field) = unrecognized.first() {
        return Err(HeadActError::Refused(unroutable_product_field(field)));
    }
    let mut save = repo::ProductHeadSave::default();
    for (field, value) in parsed {
        route_product_field(head, field, value, &mut save)?;
    }
    Ok(save)
}

/// The shape rules re-run over the head **as this save would leave it**.
///
/// `Phase::Shape` is not only about the `JSON` types [`parse_product_save`]
/// checks: `NameShapeRule` judges the name that will be stored, and a save is
/// the only door besides create that can change one. Running it over the
/// post-save image rather than over the payload is what catches a rename to a
/// string that normalizes to nothing — `"\u{00a0}"`, say — which is a value
/// the payload parser has no reason to refuse and the uniqueness index cannot
/// key on.
///
/// **The SKU door has no analogue, and the asymmetry is the rule sets'.**
/// `skus::publish_revalidation_pipeline`'s two rules raise
/// `INCOMPLETE_ENTITY` and are worded for the publish re-run (*"no longer
/// publishable"*), and both of their conditions — a blank `sku_code`, an
/// unparseable scope column — are already refused at that door's parse step,
/// where the code and the wording are the save's own. Running them again over
/// the post-save image there would be a check that cannot fire. This door has
/// one registered rule, it is worded for either caller, and its operand
/// (`name`) is one the parse step has no reason to refuse.
///
/// The refusal is `VALIDATION` rather than the publish door's
/// `INCOMPLETE_ENTITY`, and the difference is the request body:
/// [`revalidation_refusal`]'s own doc argues that a publish carries none, so
/// a `VALIDATION` problem there would name a field of a request that had no
/// fields. A save carries one and the field is the caller's, so `VALIDATION`
/// naming it is exactly right.
///
/// # The image is built from the parsed payload, not from the routed save
///
/// Both operands the rules read — `name` and `brand_id` — are present in
/// [`parse_product_save`]'s output, so this needs nothing
/// [`route_product_save`] produces and runs **before** it. That is not a
/// convenience: `Phase::ordered()` puts `Shape` ahead of `State` and §3.1's
/// `inst-fd-fail-closed` stops the run at the first failing phase, so a body
/// carrying a blank `name` beside a bucket-i column on a published head owes
/// the caller `VALIDATION` and not the bucket rule's
/// `ILLEGAL_FIELD_MUTATION`. Reading the routed [`repo::ProductHeadSave`]
/// instead would put a `Shape` rule downstream of a `State` one for no
/// operand it needs.
///
/// # Every column the candidate has a slot for is overlaid
///
/// [`CreateEntityCandidate`] carries four fields and three of them are ones
/// a save can move: `brand_id`, `name` and `code`. All three are overlaid,
/// `product_code` included, even though the pipeline registers no rule that
/// reads `code` today. A save that moves or clears the code would otherwise
/// be judged against the head's **pre-save** code — an operand that is right
/// only for as long as nothing reads it, and wrong the day a rule does. The
/// two `NullableText` arms map the way [`repo::save_product_head`] will
/// write them: `Set(v)` is `Some(v)`, `Clear` is `None`.
///
/// # Errors
///
/// [`DomainError::Validation`] carrying the failing rule's own violations.
fn check_saved_shape(
    head: &ProductRecord,
    parsed: &[(ProductSaveField, ProductSaveValue)],
) -> Result<(), DomainError> {
    let mut brand_id = head.brand_id;
    let mut name = head.name.clone();
    let mut code = head.product_code.clone();
    for (_field, value) in parsed {
        match *value {
            ProductSaveValue::BrandId(id) => brand_id = id,
            ProductSaveValue::Name(ref saved) => name.clone_from(&saved.value),
            ProductSaveValue::ProductCode(ref saved) => {
                code = match *saved {
                    NullableText::Set(ref value) => Some(value.clone()),
                    NullableText::Clear => None,
                };
            }
            ProductSaveValue::RegionScope(_) | ProductSaveValue::BrandScope(_) => {}
        }
    }
    let candidate = CreateEntityCandidate {
        tenant_id: head.tenant_id,
        brand_id,
        name,
        code,
    };
    if let Some((_phase, report)) = publish_revalidation_pipeline().run(&candidate) {
        return Err(DomainError::Validation(report));
    }
    Ok(())
}

/// Which refusal a zero-row **save** write was, re-read under the act's own
/// transaction — [`classify_unmatched_publish`]'s save twin.
///
/// [`repo::save_product_head`]'s filter carries four conditions, so
/// `Unmatched` has four readings, and this reads them in the order the caller
/// can act on: a moved revision first (refetch and re-send the `ETag`), then
/// terminality, then a bucket-i write the row was published under. The last
/// arm is the read-then-write race the filter exists to close — the door
/// judged all three a moment earlier and a neighbour moved the row since.
async fn classify_unmatched_save(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
    structural: bool,
) -> HeadActError {
    match repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id).await {
        Ok(Some(head)) if head.internal_revision != inputs.expected => {
            HeadActError::Refused(DomainError::StaleRevision {
                expected: inputs.expected,
                found: head.internal_revision,
            })
        }
        Ok(Some(head)) if head.lifecycle_state.is_terminal() => {
            HeadActError::Refused(DomainError::EntityTerminal(format!(
                "no head write is admitted on a {} entity",
                head.lifecycle_state.as_str()
            )))
        }
        Ok(Some(head)) if structural && head.published_version > 0 => {
            HeadActError::Refused(structural_after_publish("Product", "identity column"))
        }
        Ok(Some(head)) => HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "save matched no row for product {} at revision {}, yet the head is {} at revision {}",
            head.product_id,
            inputs.expected,
            head.lifecycle_state.as_str(),
            head.internal_revision
        )))),
        Ok(None) => HeadActError::Vanished,
        Err(error) => HeadActError::from_repo(&error),
    }
}

/// Turn a save's storage failure into the refusal it actually was, where the
/// driver's own text names one of this table's two unique indexes.
///
/// §3.3 puts `DUPLICATE_NAME`/`DUPLICATE_CODE` in the identity phase
/// *"wherever it runs — create, save, and the publish re-run"*, so a rename
/// onto a held name is the same governed refusal here as at create rather
/// than a `500`. [`classify_insert_conflict`] is the create door's own reader
/// of that text and is reused unchanged — including its stated cost, that
/// this is a substring match over driver text and not a typed database
/// answer.
///
/// # The name in the message is the row's, not the request's
///
/// `uq_products_product_name` keys on `(tenant_id, brand_id, name_normalized)`
/// and `brand_id` is bucket i -- writable before first publish -- so a save
/// that moves **only** `brand_id` can lose that index's race while naming no
/// name at all. Reading `save.name` alone would then answer *"...already
/// holds the name "* with an empty value, naming nothing the caller could
/// look for. The value the row would carry is what collided, so an unnamed
/// save falls back to the pre-save head's own name.
fn save_conflict(
    error: &RepoError,
    head: &ProductRecord,
    save: &repo::ProductHeadSave,
) -> HeadActError {
    match classify_insert_conflict(&error.to_string()) {
        Some(InsertConflict::DuplicateName) => {
            HeadActError::Refused(DomainError::DuplicateName(format!(
                "another live Product in this tenant and brand already holds the name {}",
                save.name
                    .as_ref()
                    .map_or(head.name.as_str(), |name| name.value.as_str())
            )))
        }
        Some(InsertConflict::DuplicateCode) => HeadActError::Refused(DomainError::DuplicateCode(
            "another live Product in this tenant already holds this productCode".to_owned(),
        )),
        None => HeadActError::from_repo(error),
    }
}

/// Refuse a save that would narrow this Product out from under a **live
/// child** (`fr-parent-child-integrity`, §4.1).
///
/// # The case, in three ordinary requests
///
/// Create a Product scoped `eu,us`; create a SKU under it scoped `us` and
/// publish it; then `PATCH` the Product to `eu`. Nothing in that sequence is
/// out of band and every step is admitted on its own terms — §4.1 puts
/// `region_scope` and `brand_scope` in bucket iii *"in both directions,
/// widening and narrowing alike"*, so the head-row guard and
/// [`bucket::classify`] both let the narrowing through by design. What the
/// third request would leave behind is a `published` child scoped outside
/// its parent. The child pays for it later: its next save or re-publish is
/// refused `SCOPE_NOT_CONTAINED` by
/// `skus::recheck_parent_containment` on a request that
/// changed nothing about it. §4.1 answers that here instead — *"a narrowing
/// that would orphan a live child meets `fr-parent-child-integrity`'s
/// fail-closed check in the registered-validators phase, ahead of the
/// governance gate"* — and the check is this slice's own, named in
/// `features/foundation.md` and in §1.5's `In` list as *"the interim
/// containment check"* whose final rule lands in slice 04.
///
/// # It runs only where the save moves a scope column
///
/// A save that names neither `region_scope` nor `brand_scope` cannot orphan
/// anything, and this returns before reading a row. That is not an
/// optimization: a blanket child scan on every save would refuse an
/// unrelated `name` change against a Product whose children some earlier
/// out-of-band write had already put outside it, which is a refusal the
/// caller cannot act on and did not cause.
///
/// # The operands, and why each is the one it is
///
/// The **parent** operand is the **post-save** pair — the image this save
/// would store, not the row as read — because the question is whether the
/// save may land, and judging the pre-save pair would pass every narrowing
/// there is. The image is built by cloning the head and overwriting only the
/// columns the routing admitted, so no other column can drift into it.
///
/// The **child** operand is each child's **own stored pair**, exactly as
/// `skus::recheck_parent_containment` argues from the
/// other end: re-resolving a child against the parent would re-widen it to
/// whatever the parent now carries and turn the very narrowing this exists
/// to catch into a silent pass.
///
/// Terminal children are not read at all
/// ([`repo::find_non_terminal_skus_of_product`] excludes them in its own
/// statement): `retired` and `discarded` rows are out of use, nothing can
/// transact against them, and refusing a parent's save on their account
/// would make a tidy retirement permanently load-bearing.
///
/// # Reused, not restated
///
/// Both halves of the verdict come from the SKU module — the
/// [`crate::domain::containment::ScopePair`]s built by
/// `skus::parent_scope_pair`/`skus::sku_scope_pair`, the rule itself
/// [`crate::domain::containment::ScopePair::check_containment`], and the
/// message `skus::scope_not_contained_domain_err` — so the child's door and
/// the parent's door cannot word or code one verdict two ways. The refusal
/// therefore does **not** name the offending child: that message is the
/// shared one, and naming a SKU in it would be a second wording. The audit
/// row names the Product, and the read is ordered by `sku_code`, so the same
/// save refuses reproducibly.
///
/// # Errors
///
/// [`HeadActError::Refused`] carrying `SCOPE_NOT_CONTAINED` for the first
/// non-terminal child whose stored pair is not contained in the post-save
/// pair. [`HeadActError::Db`] on a storage failure, and on a stored scope
/// column — the parent's or a child's — that does not parse: that is stored
/// data rather than a request value, so it takes the internal channel and a
/// `500`, which is how the SKU side treats the identical breach.
/// The tenant's definition roster, **seeding the well-known five first if it
/// has none** (**P-D-104**).
///
/// # The trigger site, and why it is here
///
/// P-D-104 requires that *"the first write that could need a well-known
/// definition seeds all five first, in that write's own transaction"*, and
/// deliberately does not name the site. This is it: the content-save path, at
/// the moment a payload names `attributes`. Three things recommend it.
///
/// It is a **write**. The read-through P-D-104 withdrew made a `GET` of the
/// roster mutate, which breaks a read-only replica and bills the first reader
/// for a write it did not ask for. Nothing on a read path reaches this.
///
/// It is the **first** such write. A save naming an attribute is the earliest
/// act in the gear that can need a well-known definition: a create carries no
/// attributes, and a publish only re-reads values that already exist — which
/// implies their definitions do.
///
/// **The existence check is free on this path.** The door has to read the
/// roster anyway, to resolve each named key for the subject, so the check is
/// that read's own `is_empty()`. Only the empty case pays anything: five
/// inserts and one re-read, once per tenant, ever. A tenant with a roster pays
/// exactly what it paid before this existed.
///
/// A save naming only `categories` does **not** trigger it, because it cannot
/// need a definition.
///
/// # Errors
///
/// [`HeadActError`] as the read or the seeding raises it.
async fn well_known_roster(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
) -> Result<Vec<repo::AttributeDefinitionRecord>, HeadActError> {
    let roster = repo::attribute_definitions(runner, &inputs.scope, inputs.tenant_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    if !roster.is_empty() {
        return Ok(roster);
    }
    repo::seed_well_known_definitions(runner, &inputs.scope, inputs.tenant_id, inputs.now)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
    repo::attribute_definitions(runner, &inputs.scope, inputs.tenant_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))
}

/// Group the entity's **stored** values by definition, for the publish-time
/// default-locale rule.
///
/// # Two reads and a join in memory, not a query per definition
///
/// The value rows come back in one statement and the roster in another, and
/// the `localized` flag is read off the roster rather than off the value —
/// a value row carries no flag, and a rule that inferred *localized* from
/// *"this row names a locale"* would let an entity publish carrying one
/// French value and no global one, which is exactly the gap the rule closes.
///
/// A definition the roster does not name is **skipped**: the value's own FK
/// makes that unreachable through this gear, so treating it as a missing
/// global value would refuse a publish for a row the door cannot explain.
///
/// # Errors
///
/// [`HeadActError`] as either read raises it.
async fn carried_definitions(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    entity_id: Uuid,
) -> Result<PublishedContentSubject, HeadActError> {
    let values = repo::attribute_values_of(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        PRODUCT_ENTITY_KIND,
        entity_id,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;
    if values.is_empty() {
        return Ok(PublishedContentSubject::default());
    }
    let roster = repo::attribute_definitions(runner, &inputs.scope, inputs.tenant_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;

    let mut carried: Vec<CarriedDefinition> = Vec::new();
    for row in values {
        let Some(definition) = roster.iter().find(|d| d.definition_id == row.definition_id) else {
            continue;
        };
        let coordinate = LocalizedValue {
            locale: row.locale,
            region: row.region,
            brand: row.brand,
            value: row.value,
        };
        match carried.iter_mut().find(|c| c.key == definition.key) {
            Some(existing) => existing.values.push(coordinate),
            None => carried.push(CarriedDefinition {
                key: definition.key.clone(),
                localized: definition.localized,
                values: vec![coordinate],
            }),
        }
    }
    Ok(PublishedContentSubject { carried })
}

/// Build the subject the seven content rules judge, from the payload plus the
/// facts a rule cannot fetch for itself.
///
/// `ValidationRule::evaluate` is synchronous — **P-D-97** arm 1 keeps it that
/// way — so every cross-row operand is read here and carried. That is arm 2's
/// first form, the shipped `PrimaryCategoryRequired` + `has_primary_category`
/// pattern, with a **set** of facts where that example has one.
///
/// # Two reads, not two per entry
///
/// The named categories come back in one statement
/// ([`repo::category_states`]); the definitions come back as the tenant's
/// **whole roster** ([`repo::attribute_definitions`]) rather than one lookup
/// per key. The roster is small — five seeds plus whatever an operator added —
/// and one statement inside the mutation transaction cannot disagree with
/// itself the way N statements can under a peer's flip.
///
/// # An unresolved name is carried, never dropped
///
/// A key or an id the tenant does not have arrives as `resolved: None`, which
/// is what `CategoryResolvableRule` and `AttributeDefinitionKnownRule` refuse.
/// Dropping it here would turn a refusal into a silent no-op — the exact shape
/// `unroutable_product_field` exists to prevent on the head half.
///
/// # Errors
///
/// [`HeadActError`] as the two reads raise it.
async fn content_subject(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    head: &ProductRecord,
    payload: &ContentSavePayload,
    detector: &(dyn PiiDetector + Send + Sync),
) -> Result<ContentSaveSubject, HeadActError> {
    let mut subject = ContentSaveSubject {
        entity_region_scope: head.region_scope.clone(),
        entity_brand_scope: head.brand_scope.clone(),
        ..ContentSaveSubject::default()
    };

    if let Some(named) = payload.categories.as_ref() {
        let ids: Vec<Uuid> = named.iter().map(|(id, _)| *id).collect();
        let states = repo::category_states(runner, &inputs.scope, inputs.tenant_id, &ids)
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
        subject.assignments = named
            .iter()
            .map(|(category_id, role)| AssignmentCandidate {
                category_id: *category_id,
                role: *role,
                resolved: states
                    .iter()
                    .find(|(id, _)| id == category_id)
                    .map(|(_, state)| *state),
            })
            .collect();
    }

    if let Some(writes) = payload.attributes.as_ref() {
        // -- The content-PII write block, this feature's first of two call
        // sites (`inst-av-pii-block`: attribute free text). It runs before the
        // roster read, because a blocked write must not seed a tenant's
        // vocabulary on its way to being refused.
        //
        // The host is named at the call site rather than threaded through the
        // door's state: `NoPiiPolicyDetector` admits everything and says so,
        // and the day `10-retention-erasure` registers a real one this is the
        // line that changes. --
        for write in writes {
            content_pii_block(detector, &format!("attributes.{}", write.key), &write.value)
                .map_err(|blocked| {
                    // Raised as its own `DomainError`, not folded into a
                    // `ValidationReport`: `design/02` §3.3 puts this code **outside**
                    // the pipeline -- *"a code raised outside the pipeline needs no
                    // phase status and gets none"* -- and the seven content rules that
                    // do ride a report are the contrast, not the pattern.
                    HeadActError::Refused(DomainError::ContentPiiBlocked(blocked.into_detail()))
                })?;
        }

        let roster = well_known_roster(runner, inputs).await?;
        subject.values = writes
            .iter()
            .map(|write| ValueCandidate {
                definition_key: write.key.clone(),
                locale: write.locale.clone(),
                region: write.region.clone(),
                brand: write.brand.clone(),
                value: write.value.clone(),
                resolved: roster
                    .iter()
                    .find(|d| d.key == write.key)
                    .map(|d| ResolvedDefinition {
                        state: d.state,
                        value_type: d.value_type.clone(),
                        localized: d.localized,
                        region_scope: d.region_scope.clone(),
                        brand_scope: d.brand_scope.clone(),
                    }),
            })
            .collect();
    }

    Ok(subject)
}

/// Turn a failing content rule into the door's refusal.
///
/// # The rule's own code reaches the wire, and no `DomainError` variant is
/// needed for it
///
/// `infra::error_mapping`'s `Validation` arm renders **each violation's own
/// `code`** as the precondition violation's `type`, so `CATEGORY_RETIRED` and
/// its five siblings are attributed as themselves the moment they are raised
/// here. Group A5 reported the opposite — that they would fall back to
/// `INCOMPLETE_ENTITY` until twelve variants landed — and that was reasoning
/// from `transition_refusal`'s ladder, which is the **publish** path.
/// `a_content_rules_code_reaches_the_wire_without_a_domain_error_variant`
/// holds the correction.
///
/// What still needs a variant is a code raised **outside** a report:
/// `CATEGORY_REFERENCED`, `DEFINITION_IN_USE`, `STALE_CATEGORY_TOKEN` and
/// `TAXONOMY_LIMIT`, whose producers return domain values rather than
/// violations.
///
/// The report's per-field violations are carried whole rather than folded
/// into a message: a save carries a body, so `VALIDATION`-class attribution
/// naming the field is exactly right — [`revalidation_refusal`]'s doc argues
/// the same point in the other direction for the bodiless publish.
fn content_refusal(report: ValidationReport) -> DomainError {
    DomainError::Validation(report)
}

/// Write the content rows this save names, on the caller's transaction.
///
/// **P-D-46**: they land in the same transaction as the head `UPDATE`, so a
/// rolled-back save leaves neither. Nothing here opens one.
///
/// The assignment write is a **replace** and the value writes are upserts,
/// which is the difference between the two collections' own semantics: the
/// assignment set is the set, while a value payload is a per-coordinate merge
/// (`inst-md-write`'s shape, and the reason `attributes: []` clears nothing).
///
/// # Errors
///
/// [`HeadActError`] on a storage failure, and on the two uniqueness conflicts
/// [`repo::AssignmentWrite`] classifies — which reach the wire as `VALIDATION`
/// for the reason [`content_refusal`] gives.
async fn write_content_rows(
    runner: &impl DBRunner,
    inputs: &HeadActInputs,
    payload: &ContentSavePayload,
    now: DateTime<Utc>,
) -> Result<(), HeadActError> {
    if let Some(set) = payload.categories.as_ref() {
        let written = repo::replace_category_assignments(
            runner,
            &inputs.scope,
            inputs.tenant_id,
            inputs.product_id,
            set,
            now,
        )
        .await
        .map_err(|e| HeadActError::from_repo(&e))?;
        if written != repo::AssignmentWrite::Applied {
            let mut report = ValidationReport::new();
            report.violate(
                CategoryRoleConflictRule::CODE,
                "categories",
                match written {
                    repo::AssignmentWrite::PrimaryConflict => {
                        "a Product holds at most one primary category"
                    }
                    _ => "a Product holds one category in one role",
                },
            );
            return Err(HeadActError::Refused(content_refusal(report)));
        }
    }

    if let Some(writes) = payload.attributes.as_ref() {
        for write in writes {
            let definition = repo::attribute_definition_by_key(
                runner,
                &inputs.scope,
                inputs.tenant_id,
                &write.key,
            )
            .await
            .map_err(|e| HeadActError::from_repo(&e))?
            .ok_or_else(|| {
                // The pipeline already refused an unknown key, so this
                // is a peer removing the definition between the read
                // and the write. It is a refusal and not a 500: the
                // act was judged against a roster that has moved.
                let mut report = ValidationReport::new();
                report.violate(
                    AttributeDefinitionKnownRule::CODE,
                    format!("attributes.{}", write.key),
                    "the definition was removed while this save was in flight",
                );
                HeadActError::Refused(content_refusal(report))
            })?;
            repo::upsert_attribute_value(
                runner,
                &inputs.scope,
                inputs.tenant_id,
                repo::AttributeCoordinate {
                    entity_kind: PRODUCT_ENTITY_KIND,
                    entity_id: inputs.product_id,
                    definition_id: definition.definition_id,
                    locale: &write.locale,
                    region: &write.region,
                    brand: &write.brand,
                },
                &write.value,
                now,
            )
            .await
            .map_err(|e| HeadActError::from_repo(&e))?;
        }
    }
    Ok(())
}

/// The `entity_kind` a Product's own attribute values carry.
///
/// A literal because §7 row 20 is the live question of what that column
/// admits; an enum here would answer it from a door.
const PRODUCT_ENTITY_KIND: &str = "product";

async fn check_children_stay_contained(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    head: &ProductRecord,
    save: &repo::ProductHeadSave,
) -> Result<(), HeadActError> {
    if save.region_scope.is_none() && save.brand_scope.is_none() {
        return Ok(());
    }

    let mut image = head.clone();
    if let Some(region_scope) = save.region_scope.as_ref() {
        image.region_scope.clone_from(region_scope);
    }
    if let Some(brand_scope) = save.brand_scope.as_ref() {
        image.brand_scope.clone_from(brand_scope);
    }
    let parent_scope = crate::api::rest::skus::parent_scope_pair(&image).map_err(|column| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "bss-products: the Product's post-save {column} contains an empty token"
        ))))
    })?;

    let children = repo::find_non_terminal_skus_of_product(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    for child in &children {
        let child_scope = crate::api::rest::skus::sku_scope_pair(child).map_err(|column| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: SKU {}'s stored {column} contains an empty token",
                child.sku_id
            ))))
        })?;
        if let Err(failure) = parent_scope.check_containment(&child_scope) {
            // `create_sku`'s own translation, for this function's stated
            // reason. Its `Err` arm is the `Contained`-on-a-refusal-path
            // impossibility that function's doc describes; it answers
            // internally here rather than through `unreachable!()`, a denied
            // restriction lint in this crate.
            let domain_err = crate::api::rest::skus::scope_not_contained_domain_err(failure)
                .map_err(|_| {
                    HeadActError::Db(DbError::Sea(DbErr::Custom(
                        "bss-products: containment check reported Contained on a refusal path"
                            .to_owned(),
                    )))
                })?;
            return Err(HeadActError::Refused(domain_err));
        }
    }

    Ok(())
}

/// Publish-time narrowing: collect every non-terminal child that falls
/// outside the head's current scope and refuse naming them.
async fn refuse_narrowing_at_publish(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    head: &ProductRecord,
) -> Result<(), HeadActError> {
    let parent_scope = crate::api::rest::skus::parent_scope_pair(head).map_err(|column| {
        HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
            "bss-products: the Product's {column} contains an empty token"
        ))))
    })?;
    let children = repo::find_non_terminal_skus_of_product(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
    )
    .await
    .map_err(|e| HeadActError::from_repo(&e))?;

    let mut falling = Vec::new();
    for child in &children {
        let child_scope = crate::api::rest::skus::sku_scope_pair(child).map_err(|column| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: SKU {}'s stored {column} contains an empty token",
                child.sku_id
            ))))
        })?;
        if parent_scope.check_containment(&child_scope).is_err() {
            falling.push(child.sku_id);
        }
    }
    if falling.is_empty() {
        return Ok(());
    }
    let named = falling
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    Err(HeadActError::Refused(DomainError::ScopeNotContained(
        format!("non-terminal children fall outside the narrowed scope: {named}"),
    )))
}

/// The save act itself, every phase of it on the mutation's own transaction
/// and in `crate::domain::validation::Phase::ordered()`'s own order
/// (`cpt-cf-bss-products-dod-save-door`).
///
/// # The order, and the one place it differs from [`run_publish`]
///
/// `Idempotency`, `Precondition`, `Shape` (the `JSON` parse, then
/// [`check_saved_shape`] over the image this save would leave), `State`
/// (terminality, then bucket routing), `RegisteredValidators`
/// ([`check_children_stay_contained`]), `GovernanceGate` — the enumeration's
/// own sequence, read off `Phase::ordered()`, minus one phase.
///
/// **`Identity` has no step of its own here, and the body's phase comments
/// jump from 4 to 6 to say so** — the same way `skus::run_save`'s jump from 5
/// to 7 over the registered-validators phase it has no rule for. §3.3 puts
/// `DUPLICATE_NAME` and `DUPLICATE_CODE` in that phase *"wherever it runs —
/// create, save, and the publish re-run"*, and on this door both are decided
/// by `uq_products_product_name` and `uq_products_product_code` under the
/// head row's own `UPDATE`, read back off the driver's text by
/// [`save_conflict`]. The refusal is therefore the one §3.3 asks for, raised
/// **after** the governance gate rather than before it, which inverts §3.1's
/// order for this phase alone. Nothing is committed on either ordering — the
/// whole act is one transaction and a refusal rolls all of it back — so what
/// the inversion costs is that a colliding save is put to the gate before it
/// is refused. Answering it in place would take a pre-`UPDATE` read of both
/// indexes whose verdict the `UPDATE` would have to re-decide anyway, the
/// same read-then-write race [`classify_unmatched_save`]'s last arm exists
/// for, so the phase is left where the database settles it and the inversion
/// is recorded here rather than implied.
///
/// [`run_publish`] and [`run_discard`] ask **terminality before the
/// precondition**, which is the other way round. That is not a difference
/// this door invented: `Phase::ordered()` puts `Precondition` second and
/// `State` fourth, and terminality is a `State` rule (`Phase::State`'s own
/// doc names *"the subject's own"* terminal state, beside bucket routing).
/// The two orders answer differently in exactly one case — a stale `If-Match`
/// against a head a neighbour has since retired, which this door calls
/// `STALE_REVISION` and the publish door calls `ENTITY_TERMINAL` — and
/// `STALE_REVISION` is the answer the caller can act on. **The publish and
/// discard doors are owed the same swap**; it is not made here because those
/// doors are not this slice's subject and the change would be an unreviewed
/// behaviour change to two shipped doors.
///
/// # Everything is inside the transaction because the claim is
///
/// `Phase::Idempotency` runs first and P-D-42 puts the claim `INSERT` inside
/// the guarded mutation, so every later phase is in here too. The
/// consequence is the one [`run_publish`]'s doc states and it reaches a save
/// unchanged: a client whose save committed and whose response was lost
/// retries with the `If-Match` it still holds — the revision *before* the act
/// it never learned about — so its precondition is stale by construction. A
/// door that judged the precondition first would refuse every such retry
/// `STALE_REVISION` and never reach the stored answer.
/// `products_tests::a_replayed_save_serves_the_stored_answer_and_does_not_save_twice`
/// holds that closed.
///
/// # One head-row `UPDATE`, no version row, no edge
///
/// The head is the authoring surface in every non-terminal state
/// (`inst-fd-transition-guard`), so a save writes no
/// `products_entity_version` row and moves `published_version` not at all.
/// It takes no edge either, so [`transition::guard`] is not asked — a save
/// presents `from == to` and the guard's answer for that is
/// `TransitionDecision::NotATransition`, which is a fact about transitions
/// and not about this act.
///
/// # The invalidation hook fires, and deliberately does not read `invalidation_for`
///
/// `inst-fd-transition-bump`: *"Every transition bumps `internal_revision`
/// and fires the approval-invalidation hook **exactly as a save does**"* — so
/// a save fires it. [`transition::invalidation_for`] would answer
/// [`ApprovalInvalidation::Skip`] here, because a save reaches its
/// `NotATransition` arm, and that answer was decided for a **re-publish**:
/// the exception in the rule is *"a transition that consumes an approval in
/// the same transaction"*, and a re-publish consumes the record 05
/// `inst-gv-materiality` gives it. **A save consumes none**, so the
/// exception's reason does not reach it and the rule's main clause applies.
/// That is not this door's reading imposed on the domain module —
/// `transition::invalidation_for`'s own doc says it in as many words ("a
/// save's hook is not read off this function ... the save door owns its own
/// bump and its own hook") — and a later reader who unifies the two call
/// sites would silently drop the invalidation this `DoD` requires.
///
/// # The governance gate runs and passes trivially
///
/// §3.1's `inst-fd-pipeline-gate-phase` puts the phase at *every* mutating
/// door and has it pass where the act is ungated (**P-D-34**). Both discard
/// doors already ask it on exactly these terms; this one does the same, in
/// [`GateMode::Gate`], and the gear's default host authorizes naming no
/// record. What it buys is the case the phase exists for: slice 05 registers
/// a materiality policy on a save and the seam is already here.
///
/// # Owed: the `state` phase short-circuits where §3.3 collects
///
/// §3.3 uses **a save** as its worked example -- *"a save on a `retired` head
/// that also moves a bucket-i column satisfying `ENTITY_TERMINAL` and
/// `ILLEGAL_FIELD_MUTATION` alike ... the caller's rejection carries all of
/// them regardless; the precedence governs the one code the row stores"* --
/// and §3.1 names `state` as the only phase that may raise more than one
/// code. This door does not: terminality `?`-returns before the routing runs,
/// so a save satisfying both answers `ENTITY_TERMINAL` alone.
///
/// **It is left owed rather than built, and the reason is a measurement of
/// the wire type, not a judgement about effort.** Both codes are 409s and a
/// 409 is `toolkit_canonical_errors`' `Aborted`, whose whole context is one
/// `reason: String` (`AbortedV1`, and `with_reason` is the single builder
/// step that reaches it). There is no second slot for a second code, so
/// "carries all of them" cannot be expressed on this response at all without
/// either changing a shared platform type or demoting the joint refusal to
/// the `Validation` envelope -- which is the only multi-code shape the gear
/// has and which would answer 400 where §3.3 requires 409. Overloading
/// `detail` with the second code would not serve it either: a consumer
/// matches `reason`, exactly as `infra::error_mapping`'s `denied` doc argues
/// for `APPROVAL_REQUIRED`.
///
/// So the clause needs a carrier decided by the taxonomy's owner -- a
/// multi-code refusal shape, or §3.3's clause narrowed to the audit row's
/// precedence alone -- and this door adopts it when there is one. The audit
/// row is already correct under either reading: `ENTITY_TERMINAL` is the
/// highest-precedence code of the pair and it is what the row records today.
///
/// # Errors
///
/// [`HeadActError::Refused`] for every refusal above, each rolled back and
/// audited by [`answer_head_act`]; [`HeadActError::Vanished`] where the head
/// left the caller's scope; [`HeadActError::Db`] on storage, on an
/// unreachable gate host, and on the structurally-unreachable
/// [`bucket::FieldClass::Outside`] arm.
async fn run_save(
    runner: &(impl DBRunner + Sync),
    inputs: &HeadActInputs,
    request: &SaveProductRequest,
    gate: &(dyn GovernanceGate + Send + Sync),
    detector: &(dyn PiiDetector + Send + Sync),
    outbox: &crate::infra::broker::EventSink,
) -> Result<HeadActOutcome, HeadActError> {
    // -- Phase 1, idempotency: the claim, and the replay that ends the act
    // before any other phase is judged. --
    if let Some(replay) = claim_for_head_act(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.claim.as_ref(),
    )
    .await?
    {
        return Ok(replay);
    }

    let head = repo::find_product(runner, &inputs.scope, inputs.tenant_id, inputs.product_id)
        .await
        .map_err(|e| HeadActError::from_repo(&e))?
        .ok_or(HeadActError::Vanished)?;

    // -- Phase 2, the precondition. The head-row `UPDATE` carries the same
    // comparison in its own filter and that copy is what decides whether the
    // write lands; this one decides whether the rest of the pipeline runs. --
    if head.internal_revision != inputs.expected {
        return Err(HeadActError::Refused(DomainError::StaleRevision {
            expected: inputs.expected,
            found: head.internal_revision,
        }));
    }

    // -- Phase 3, shape: the JSON types, every violation collected, and then
    // the registered shape rules over the image this save would leave. Both
    // halves are the same phase and both run **before** the `State` phase
    // below, which is `Phase::ordered()`'s own sequence and §3.1's
    // `inst-fd-fail-closed` ("the run stops at the first failing phase").
    // The image the rules judge is built from `parsed` rather than from the
    // routed `ProductHeadSave`, and that is what lets the phase keep its
    // place: the two operands the rules read, `name` and `brand_id`, are
    // both present the moment the payload parses, so nothing here depends on
    // the routing's output. Running it after the routing instead would
    // answer `ILLEGAL_FIELD_MUTATION` to a `PATCH` carrying a blank `name`
    // beside a bucket-i column on a published head, where the design
    // requires the run to stop at `shape` with `VALIDATION`. --
    let (parsed, unrecognized) = parse_product_save(request).map_err(HeadActError::Refused)?;
    let content = parse_content_save(request).map_err(HeadActError::Refused)?;
    check_saved_shape(&head, &parsed).map_err(HeadActError::Refused)?;

    // -- Phase 4, state: terminality — which reaches every head write and
    // not only a transition (`inst-fd-terminal`, P-D-25 widened by
    // P-D-32) — then bucket routing over the whole request. --
    transition::check_head_write(head.lifecycle_state).map_err(HeadActError::Refused)?;
    let mut save = route_product_save(&head, parsed, &unrecognized)?;
    // `02`'s content rides this head's revision (C2), so a save naming only
    // `categories` or `attributes` still moves it -- and `repo::empty_save`'s
    // guard must see that as a content write rather than as a bare bump.
    save.content_moved = !content.is_empty();

    // -- Phase 6, the registered-validators phase: `fr-parent-child-integrity`
    // over the Product's live children, judged against the image this save
    // would leave and run only where the save moves a scope column. §4.1 puts
    // it exactly here — "in the registered-validators phase, ahead of the
    // governance gate". It is not a registered `Phase::RegisteredValidators`
    // rule for `skus::recheck_parent_containment`'s reason on the other side:
    // the pipeline is synchronous and judges the subject row alone, and this
    // rule's operand is a read of other rows. So it runs as that phase's
    // continuation, on this transaction. --
    check_children_stay_contained(runner, inputs, &head, &save).await?;

    // -- The same phase, `02`'s half: the seven content rules over the
    // assignments and values this payload names. A registered pipeline rather
    // than a continuation, because every operand is a fact this door can
    // prefetch (**P-D-97** arm 2's first form) -- so unlike the containment
    // check above, these collect: one save answers one rejection carrying
    // every content violation.
    //
    // It runs **before** the gate for `Phase::ordered()`'s reason, the one
    // `run_publish` states: an act that is not legal at all must be refused as
    // illegal rather than answered with an approval question. --
    if !content.is_empty() {
        let subject = content_subject(runner, inputs, &head, &content, detector).await?;
        if let Some((_phase, report)) = content_save_pipeline().run(&subject) {
            return Err(HeadActError::Refused(content_refusal(report)));
        }
    }

    // -- Phase 7, the governance gate, in `Gate` mode: asked at every
    // mutating door and passing trivially where the act is ungated
    // (`inst-fd-pipeline-gate-phase`). The two `Err` routes are
    // `run_publish`'s and carry its reasoning: a host that could not *reach*
    // an answer is infrastructure and answers 5xx, while the ceremony's own
    // `no` is `APPROVAL_REQUIRED`. --
    let verdict = gate
        .evaluate(
            GateSubject::entity_publish(
                EntityRef {
                    tenant_id: inputs.tenant_id,
                    entity_kind: CatalogEntityKind::Product,
                    entity_id: inputs.product_id,
                },
                InternalRevision::new(inputs.expected),
            ),
            GateMode::Gate,
        )
        .map_err(|e| {
            HeadActError::Db(DbError::Sea(DbErr::Custom(format!(
                "bss-products: the governance gate host failed: {e}"
            ))))
        })?;
    // Collapsed into the control flow and dropped, as at the discard doors:
    // a save freezes no version row, so the `approval_ref` the verdict may
    // name has no column to reach.
    verdict
        .into_authorization()
        .map_err(HeadActError::Refused)?;

    // -- Exactly one head-row `UPDATE`: the routed columns, the revision
    // bump and `updated_at` together, because the guard bumps
    // `internal_revision` on every admitted `UPDATE` without exception. --
    let structural = save.brand_id.is_some() || save.product_code.is_some();
    let write = repo::save_product_head(
        runner,
        &inputs.scope,
        inputs.tenant_id,
        inputs.product_id,
        inputs.expected,
        &save,
        inputs.now,
    )
    .await
    .map_err(|e| save_conflict(&e, &head, &save))?;
    if write == HeadWrite::Unmatched {
        return Err(classify_unmatched_save(runner, inputs, structural).await);
    }

    // -- `02`'s content rows, on this same transaction (**P-D-46**), so a
    // rolled-back save leaves neither the head update nor them. **After** the
    // head `UPDATE` and not before: the `UPDATE` carries the precondition, the
    // terminality filter and the bucket guard in its own `WHERE`, so it is the
    // statement that decides whether this act happens at all -- writing
    // content first would put rows down for a save the head row then refuses.
    //
    // It is also why the head write stays exactly one statement: this adds
    // rows to other tables and touches no column of `products_product`, so
    // `inst-fd-transition-bump`'s "once" is untouched. --
    write_content_rows(runner, inputs, &content, inputs.now).await?;

    // -- The approval-invalidation hook, which a save **fires**: see this
    // function's own doc for why the answer is not read off
    // `transition::invalidation_for`. --
    fire_invalidation_hook(runner, inputs, ApprovalInvalidation::Fire).await?;

    // -- Then the event, and the stored answer. --
    announce_and_answer(runner, outbox, inputs, Announcement::HeadSaved).await
}

/// Run [`run_save`] on one retried transaction —
/// [`publish_in_one_transaction`]'s save twin, on its terms exactly: the
/// claim is the transaction's first statement and therefore the collision
/// point, `DBProvider::transaction` has no contention retry, and the body is
/// safe to re-run because the claim rolls back with everything after it.
///
/// The request travels as an owned clone for [`HeadActInputs`]'s stated
/// reason: `transaction_with_retry`'s body is
/// `for<'a> FnMut(&'a DbTx<'a>) -> ...`, whose higher-ranked `'a` cannot be
/// bounded by any lifetime the caller holds, so nothing borrowed from the
/// door's state can be captured.
async fn save_in_one_transaction(
    state: &ApiState,
    opened: &OpenedHeadDoor,
    request: SaveProductRequest,
    gate: &Arc<dyn GovernanceGate + Send + Sync>,
    detector: &Arc<dyn PiiDetector + Send + Sync>,
) -> Result<HeadActOutcome, HeadActError> {
    let outbox = state.sink.clone();
    let gate = Arc::clone(gate);
    let detector = Arc::clone(detector);
    let inputs = opened.act_inputs();
    state
        .db
        .db()
        .transaction_with_retry::<HeadActOutcome, HeadActError, _, _>(
            TxConfig::default(),
            head_act_contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let gate = Arc::clone(&gate);
                let detector = Arc::clone(&detector);
                let inputs = inputs.clone();
                let request = request.clone();
                Box::pin(async move {
                    run_save(
                        tx,
                        &inputs,
                        &request,
                        gate.as_ref(),
                        detector.as_ref(),
                        &outbox,
                    )
                    .await
                })
            },
        )
        .await
}

/// `PATCH /products/{id}`: the save door.
///
/// The thin `axum` shell over [`save_product_under_gate`]. The only thing
/// decided here is the governance host, and it is decided the way
/// [`publish_product`] and [`discard_product`] decide it: the
/// [`NoMaterialityPolicyGate`] literal, so no wire input chooses one.
///
/// # The grant is `write`
///
/// A save is an ordinary head write, so it gates on `product x write` with
/// `owner_tenant_id = Some(tenant_id)` — the row is written **to** the
/// caller's own tenant, which is what makes `crate::authz::access_scope`'s
/// cross-tenant membership assertion apply. [`open_head_door`] carries both.
async fn save_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Path(product_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<SaveProductRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    // The gate host stays a literal for the reason `NoMaterialityPolicyGate`
    // states: no wire input chooses one, and a test reaches it by calling
    // `save_product_under_gate` directly. The **detector** is no longer a
    // literal: `10-retention-erasure` registered a real one
    // (`dod-pii-detector`), its policy has an operand — the tenant's
    // Legal-signed-off allow-list — and reading that operand is asynchronous,
    // which `PiiDetector::inspect` deliberately is not.
    let detector =
        crate::api::rest::retention::tenant_pii_detector(&state, ctx.subject_tenant_id()).await?;
    save_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        request,
        &SaveHosts {
            gate: &(Arc::new(NoMaterialityPolicyGate) as Arc<dyn GovernanceGate + Send + Sync>),
            detector: &detector,
        },
    )
    .await
}

/// The two hosts a save spends, bundled.
///
/// One parameter rather than two because they are the same kind of thing —
/// seams another slice fills, named at the wire handler as literals so no
/// wire input chooses one — and because eight arguments is one past what
/// `clippy::too_many_arguments` admits. Grouping the two that travel together
/// is the fix that says something rather than the one that silences a lint.
pub(crate) struct SaveHosts<'a> {
    /// `05-governance`'s gate, or [`NoMaterialityPolicyGate`] until it lands.
    pub gate: &'a Arc<dyn GovernanceGate + Send + Sync>,
    /// `10-retention-erasure`'s PII detector, or
    /// [`crate::domain::taxonomy::NoPiiPolicyDetector`] until it lands.
    pub detector: &'a Arc<dyn PiiDetector + Send + Sync>,
}

/// The save door, with its governance host as an explicit argument —
/// [`discard_product_under_gate`]'s twin, and a parameter for that function's
/// stated reason: the gate phase runs here (`inst-fd-pipeline-gate-phase`)
/// and the gear's only host never refuses under [`GateMode::Gate`], so the
/// refusal arm is unreachable through [`save_product`] and a phase nothing
/// can exercise is one a reader cannot tell from a phase that is absent.
///
/// The **mode** is not a parameter, on [`run_discard`]'s measured asymmetry:
/// `dod-publish-door`'s explicit-mode requirement is the publish door's, and
/// no slice schedules or cascades a save.
///
/// # What this door does not build, and which slice owns each
///
/// `cpt-cf-bss-products-dod-save-door` covers a **content-row half this
/// slice cannot build**, and the `DoD` therefore reads as *partial* rather
/// than met. None of it is silently omitted:
///
/// - **Category assignments** — `products_product_category` is **slice 02**'s
///   table and does not exist at this commit, so there is no row for this
///   transaction to write and no field for this door to route.
/// - **Attribute values** — `products_attribute_value`, likewise **slice
///   02**'s.
/// - **The metering declaration** — **slice 03**'s, which owns both the
///   column set and the rules over it.
///
/// When each lands it joins **this** transaction, beside the single head-row
/// `UPDATE` rather than after it, for `inst-fd-transition-bump`'s "once":
/// a content row written on a runner of its own would survive a rolled-back
/// save.
///
/// Two more clauses are absent for reasons that are not a schedule.
/// **`brand_id` against the caller's brand claims** (P-D-33) has nothing on
/// this door's `SecurityContext` to validate against —
/// [`create_product`]'s own doc measures the five fields that context carries
/// and none is a brand claim — so this door validates `brand_id`'s shape and
/// leaves the claims half owed to the identity owner, exactly as the create
/// door does. And **bucket ii and bucket iv have no columns**
/// (`crate::domain::bucket`'s module doc: §4.1 assigns none), so both arms
/// are built and neither is reachable today.
///
/// # Errors
///
/// Every refusal this door raises, each audited on its own transaction; the
/// bare `404` a miss answers; the `500` a storage or gate-host failure
/// raises.
async fn save_product_under_gate(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    product_id: Uuid,
    headers: &HeaderMap,
    request: SaveProductRequest,
    hosts: &SaveHosts<'_>,
) -> Result<Response, CanonicalError> {
    let (gate, detector) = (hosts.gate, hosts.detector);
    let act = HeadAct {
        authz_action: crate::authz::actions::WRITE,
        audit_action: SAVE_AUDIT_ACTION,
        endpoint: save_endpoint(product_id),
        payload_digest: save_payload_digest(&request),
    };
    let opened = open_head_door(state, enforcer, ctx, product_id, headers, &act).await?;

    // A save naming no field at all is refused here rather than inside the
    // act: it is a property of the request alone, needs no row to judge, and
    // admitting it would be a bare `internal_revision` bump — a write with no
    // content that still invalidates every `ETag` a client holds.
    if request.fields.is_empty() {
        let mut report = ValidationReport::new();
        report.violate(
            NameShapeRule::CODE,
            "body",
            "a save must name at least one field: an empty body would bump the revision and \
             write nothing",
        );
        return Err(audit_and_refuse(
            state,
            &opened,
            SAVE_AUDIT_ACTION,
            DomainError::Validation(report),
        )
        .await);
    }

    let result = save_in_one_transaction(state, &opened, request, gate, detector).await;
    answer_head_act(state, &opened, SAVE_AUDIT_ACTION, result).await
}

#[cfg(test)]
#[path = "products_tests.rs"]
mod products_tests;

/// The clone request: the overrides and nothing else (**P-D-75**).
///
/// Absent means copy/reset per `design/11` §3.1's disposition table. The
/// P-D-75 body also admits replacement values for the five re-validated
/// classes; their stores (`02`/`03`) do not ship, so those slots join this
/// type when the columns they replace exist — declared in the contract now,
/// carried on the wire later.
#[derive(Debug, Clone)]
#[toolkit_macros::api_dto(request)]
pub struct CloneProductRequest {
    /// Overrides the suggested name. A collision on it is the ordinary
    /// `DUPLICATE_NAME` — only the *suggested* name walks first-free.
    pub name: Option<String>,
    /// Overrides the suggested code, on the same terms.
    pub code: Option<String>,
}

/// The clone body's idempotency digest, over the parsed request
/// (`payload_digest`'s twin for this door's own DTO).
fn clone_payload_digest(request: &CloneProductRequest) -> Vec<u8> {
    let mut fields = JsonMap::new();
    if let Some(name) = request.name.as_ref() {
        fields.insert("name".to_owned(), JsonValue::String(name.clone()));
    }
    if let Some(code) = request.code.as_ref() {
        fields.insert("code".to_owned(), JsonValue::String(code.clone()));
    }
    idempotency::payload_digest(&JsonValue::Object(fields))
}

/// The concrete resource path a clone claims its idempotency key under
/// (**P-D-42**'s rule, [`publish_endpoint`]'s twin).
fn clone_endpoint(product_id: Uuid) -> String {
    format!("/bss-products/v1/products/{product_id}/clone")
}

/// One string field out of a decoded frozen rendering, whose writer put
/// JSON `null` where a roster name had no value (`canonical::Absence::Null`).
fn frozen_str(content: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    content
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

/// Read the Product clone's source per `inst-cn-door`, or the refusal the
/// state owes. The frozen half decodes through
/// [`canonical::decode_rendering`] — the renderer's own inverse, never a
/// parse of this door's own (**P-D-77**).
async fn resolve_clone_source(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<Option<ProductCloneSource>, CanonicalError> {
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&RepoError::Db(format!("clone source connection: {e}")))
    })?;
    let head = repo::find_product(&conn, scope, tenant_id, product_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let Some(head) = head else {
        return Ok(None);
    };

    if head.lifecycle_state == LifecycleState::Discarded {
        // The source's state refuses the act (P-D-75): ENTITY_TERMINAL would
        // claim a head *write*, and the bare 404 would claim the row does
        // not exist. The caller learns exactly what stands in the way.
        return Err(CanonicalError::from(DomainError::CloneSourceDiscarded(
            format!("product {product_id} is discarded and admits no clone"),
        )));
    }

    if head.lifecycle_state == LifecycleState::Draft {
        return Ok(Some(ProductCloneSource {
            brand_id: head.brand_id,
            name: head.name,
            product_code: head.product_code,
            region_scope: head.region_scope,
            brand_scope: head.brand_scope,
            read_at_version: None,
            retired: false,
        }));
    }

    // Published, deprecated or retired: the last frozen version is the read
    // surface — even where the head moved after deprecation (P-D-78). A
    // head with `published_version >= 1` and no frozen row is a store this
    // gear wrote wrong, and the alarm is ours, not the caller's.
    let frozen = repo::latest_entity_version(
        &conn,
        scope,
        tenant_id,
        VersionedEntityKind::Product,
        product_id,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;
    let Some((version, content)) = frozen else {
        return Err(repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "product {product_id} is {} with no frozen version row",
            head.lifecycle_state.as_str()
        ))));
    };
    let content = canonical::decode_rendering(&content).map_err(|e| {
        repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "frozen content of product {product_id} v{version}: {e}"
        )))
    })?;

    let brand_id = frozen_str(&content, "brand_id")
        .and_then(|s| Uuid::parse_str(&s).ok())
        .ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of product {product_id} v{version} carries no brand_id"
            )))
        })?;
    let name = frozen_str(&content, "name").ok_or_else(|| {
        repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "frozen content of product {product_id} v{version} carries no name"
        )))
    })?;

    Ok(Some(ProductCloneSource {
        brand_id,
        name,
        product_code: frozen_str(&content, "product_code"),
        // The scope keys are always rendered by this gear's own freeze, so
        // their absence is the same corruption class the brand_id/name
        // checks above refuse — never a silent empty scope on the clone.
        region_scope: frozen_str(&content, "region_scope").ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of product {product_id} v{version} carries no region_scope"
            )))
        })?,
        brand_scope: frozen_str(&content, "brand_scope").ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of product {product_id} v{version} carries no brand_scope"
            )))
        })?,
        read_at_version: Some(version),
        retired: head.lifecycle_state == LifecycleState::Retired,
    }))
}

/// One `write` scope for the clone door, against one resource type —
/// [`clone_write_scopes`]' per-resource half, refused with the create
/// door's own audited `PERMISSION_DENIED` shape.
async fn clone_scope_for(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    source_id: Uuid,
    resource: &authz_resolver_sdk::ResourceType,
) -> Result<AccessScope, CanonicalError> {
    match crate::authz::access_scope(
        enforcer,
        ctx,
        resource,
        crate::authz::actions::WRITE,
        Some(tenant_id),
        None,
        true,
    )
    .await
    {
        Ok(scope) => Ok(scope),
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            Err(refuse_clone(
                state,
                &self_scope,
                tenant_id,
                actor_ref,
                "PERMISSION_DENIED",
                source_id.to_string(),
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(authz_error_to_canonical(err, |reason| {
                ProductResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

/// The clone door's gate: `product x write` **and** `sku x write`, both
/// unconditionally (**P-D-79**) — every product clone is the family act,
/// and authorization precedes the child count the door has not yet been
/// authorized to read (P-D-30).
///
/// Returns `(product_scope, sku_scope)`: the first governs the parent's
/// reads and writes, the second the children's.
async fn clone_write_scopes(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    source_id: Uuid,
) -> Result<(AccessScope, AccessScope), CanonicalError> {
    let product_scope = clone_scope_for(
        state,
        enforcer,
        ctx,
        tenant_id,
        actor_ref,
        source_id,
        &crate::authz::resource_types::PRODUCT,
    )
    .await?;
    let sku_scope = clone_scope_for(
        state,
        enforcer,
        ctx,
        tenant_id,
        actor_ref,
        source_id,
        &crate::authz::resource_types::SKU,
    )
    .await?;
    Ok((product_scope, sku_scope))
}

/// One audited refusal of the clone door, PRODUCT-labelled — the shape every
/// arm of [`clone_product`] refuses in. With [`audit_failed_child`]'s
/// SKU-labelled twin it is the whole of the clone's audit posture: a
/// **committed** clone writes no audit row — its `ProductCreated`/
/// `SkuCreated` is the record (P-D-21) — and a **refused** one writes one,
/// carrying a single `error_code`.
///
/// @cpt-dod:cpt-cf-bss-products-dod-clone-audit:p1
async fn refuse_clone(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    error_code: &'static str,
    subject: String,
    canonical: CanonicalError,
) -> CanonicalError {
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::PRODUCT,
            error_code,
        },
        RefusalSubject::Attempted(subject),
        canonical,
    )
    .await
}

/// The parent's creating transaction for the family act (**P-D-79**):
/// [`insert_product_with_event`]'s twin with the three composite
/// differences, kept as its own function so the create door's single-entity
/// contract stays exactly what its doc states.
///
/// 1. The claim is read through [`claim_composite_idempotency`], whose
///    matching-live-claim arm is the **resume signal**, not a refusal.
/// 2. The claim's `entity_ref` is stamped with the minted parent id, in
///    this same transaction — claim, parent row, outbox row and stamp
///    commit together or not at all.
/// 3. **No answer is recorded here.** The claim stays
///    committed-and-unanswered — P-D-72's *in progress* — until the
///    children phase completes; [`clone_product`] stores the receipt then.
async fn insert_clone_parent(
    state: &ApiState,
    scope: AccessScope,
    new: NewProduct,
    claim: Option<IdempotencyClaimInput>,
    actor_ref: Uuid,
) -> Result<CloneParentOutcome, DbError> {
    let outbox = state.sink.clone();
    let tenant_id = new.tenant_id;
    state
        .db
        .db()
        .transaction_with_retry::<CloneParentOutcome, DbError, _, _>(
            TxConfig::default(),
            contention_db_err,
            move |tx| {
                let outbox = outbox.clone();
                let scope = scope.clone();
                let new = new.clone();
                let claim = claim.clone();
                Box::pin(async move {
                    if let Some(input) = claim.as_ref() {
                        match claim_composite_idempotency(tx, &scope, tenant_id, input)
                            .await
                            .map_err(|e| DbError::Sea(e.to_db_err()))?
                        {
                            CompositeClaimVerdict::Proceed => {}
                            CompositeClaimVerdict::Replay { status, body } => {
                                return Ok(CloneParentOutcome::Replay { status, body });
                            }
                            CompositeClaimVerdict::Refused(refusal) => {
                                return Ok(CloneParentOutcome::Refused(refusal));
                            }
                            CompositeClaimVerdict::Resume { entity_ref } => {
                                return Ok(CloneParentOutcome::Resume {
                                    parent_id: entity_ref,
                                });
                            }
                        }
                    }

                    let record = repo::insert_product(tx, &scope, new)
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;

                    if let Some(input) = claim.as_ref() {
                        repo::stamp_idempotency_entity_ref(
                            tx,
                            &scope,
                            tenant_id,
                            &input.endpoint,
                            &input.client_key,
                            record.product_id,
                        )
                        .await
                        .map_err(|e| DbError::Sea(e.to_db_err()))?;
                    }

                    let core = events::EventBodyCore {
                        tenant_id: record.tenant_id,
                        entity_kind: events::EntityKind::Product.as_str(),
                        entity_id: record.product_id,
                        internal_revision: record.internal_revision,
                        lifecycle_state: record.lifecycle_state.as_str(),
                    };
                    events::enqueue(
                        &outbox,
                        tx,
                        record.product_id,
                        events::PRODUCT_CREATED_PAYLOAD_TYPE,
                        &core,
                        actor_ref,
                    )
                    .await
                    .map_err(|e| {
                        DbError::Sea(DbErr::Custom(format!("enqueue ProductCreated: {e}")))
                    })?;

                    Ok(CloneParentOutcome::Created {
                        record: Box::new(record),
                    })
                })
            },
        )
        .await
}

/// [`insert_clone_parent`]'s outcome — [`CreateOutcome`]'s shape plus the
/// composite door's fourth arm.
enum CloneParentOutcome {
    /// The parent landed; the children phase runs next.
    Created {
        /// The created head, the children phase's remap target. Boxed so
        /// this variant does not dwarf its siblings (`large_enum_variant`).
        record: Box<ProductRecord>,
    },
    /// The act completed earlier; this is its stored receipt.
    Replay {
        /// The stored status.
        status: i32,
        /// The stored body.
        body: JsonValue,
    },
    /// The idempotency phase refused; nothing was written.
    Refused(DomainError),
    /// A committed-but-unanswered claim: resume the children phase against
    /// the stamped parent (P-D-72, P-D-79).
    Resume {
        /// The parent the crashed or in-flight act already created.
        parent_id: Uuid,
    },
}

/// One receipt entry for a child that landed.
fn child_created_entry(source_sku_id: Uuid, new_sku_id: Uuid, sku_code: &str) -> JsonValue {
    serde_json::json!({
        "source_sku_id": source_sku_id,
        "disposition": "created",
        "new_sku_id": new_sku_id,
        "sku_code": sku_code,
    })
}

/// One receipt entry for a child that failed alone: the owning door's code
/// verbatim, plus the collected violations where the failure carried a
/// report (P-D-72 arm 3 — no parallel taxonomy).
fn child_failed_entry(source_sku_id: Uuid, refusal: &DomainError) -> JsonValue {
    let violations: Vec<JsonValue> = match refusal {
        DomainError::Validation(report) => report
            .violations()
            .iter()
            .map(|violation| {
                serde_json::json!({
                    "code": violation.code,
                    "subject": violation.subject,
                    "detail": violation.detail,
                })
            })
            .collect(),
        other => vec![serde_json::json!({
            "code": other.code(),
            "subject": "sku",
            "detail": other.to_string(),
        })],
    };
    serde_json::json!({
        "source_sku_id": source_sku_id,
        "disposition": "failed",
        "code": refusal.code(),
        "violations": violations,
    })
}

/// Audit one failed child as the refusal it is, and keep going: the family
/// act is not refused by a failing child (`inst-cn-children` — siblings
/// land), but the child's own refusal is still the audited kind
/// (`dod-clone-audit`), SKU-labelled like every SKU-door refusal.
/// The act-wide operands every family step shares: the tenant, the acting
/// principal and the act's one instant — bundled so the family functions
/// stay under the argument bar without threading three scalars each.
#[derive(Clone, Copy)]
struct ActStamp {
    tenant_id: Uuid,
    actor_ref: Uuid,
    now: DateTime<Utc>,
}

async fn audit_failed_child(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    subject: String,
    refusal: DomainError,
) {
    let code = refusal.code();
    // The built CanonicalError is the receipt's business, not the wire's —
    // the family act still answers 201 — so the report half is dropped.
    let _refused_answer = crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: crate::authz::labels::SKU,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await;
}

/// The SKU-side source fields of one family child, read where its state
/// says ([`resolve_clone_source`]'s rule, per child).
async fn resolve_child_source(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    parent_id: Uuid,
    child: &SkuRecord,
) -> Result<SkuCloneSource, CanonicalError> {
    if child.lifecycle_state == LifecycleState::Draft {
        return Ok(SkuCloneSource {
            product_id: parent_id,
            sku_code: child.sku_code.clone(),
            region_scope: child.region_scope.clone(),
            brand_scope: child.brand_scope.clone(),
            read_at_version: None,
        });
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&RepoError::Db(format!(
            "child clone source connection: {e}"
        )))
    })?;
    let frozen = repo::latest_entity_version(
        &conn,
        scope,
        tenant_id,
        VersionedEntityKind::Sku,
        child.sku_id,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;
    let Some((version, content)) = frozen else {
        return Err(repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "sku {} is {} with no frozen version row",
            child.sku_id,
            child.lifecycle_state.as_str()
        ))));
    };
    let content = canonical::decode_rendering(&content).map_err(|e| {
        repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "frozen content of sku {} v{version}: {e}",
            child.sku_id
        )))
    })?;
    let sku_code = frozen_str(&content, "sku_code").ok_or_else(|| {
        repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "frozen content of sku {} v{version} carries no sku_code",
            child.sku_id
        )))
    })?;
    Ok(SkuCloneSource {
        product_id: parent_id,
        sku_code,
        // The same corruption class as the sku_code check above: the scope
        // keys are always rendered by this gear's own freeze.
        region_scope: frozen_str(&content, "region_scope").ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of sku {} v{version} carries no region_scope",
                child.sku_id
            )))
        })?,
        brand_scope: frozen_str(&content, "brand_scope").ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "frozen content of sku {} v{version} carries no brand_scope",
                child.sku_id
            )))
        })?,
        read_at_version: Some(version),
    })
}

/// Clone one family child through the disposition table, answering its
/// receipt entry. A `Some(existing)` short-circuits to the `created` entry
/// the resume re-entry owes for work already done (P-D-72).
async fn clone_family_child(
    state: &ApiState,
    sku_scope: &AccessScope,
    stamp: ActStamp,
    parent: &ProductRecord,
    parent_pair: &crate::domain::containment::ScopePair,
    child: &SkuRecord,
) -> Result<JsonValue, CanonicalError> {
    let ActStamp {
        tenant_id,
        actor_ref,
        now,
    } = stamp;
    let source =
        resolve_child_source(state, sku_scope, tenant_id, parent.product_id, child).await?;

    // The ordinary containment validator, against the NEW parent (§3.1's
    // remap row). The new parent is a fresh draft, so the create door's
    // PARENT_TERMINAL arm cannot fire; containment can — a retired child's
    // frozen scopes are exempt from the parent save door's sweep and may
    // genuinely exceed what the parent's frozen read now carries.
    let region_input = scope_input_from_payload(Some(source.region_scope.clone()));
    let brand_input = scope_input_from_payload(Some(source.brand_scope.clone()));
    let (Ok(region_input), Ok(brand_input)) = (region_input, brand_input) else {
        return Err(repo_error_to_canonical(&RepoError::CorruptRow(format!(
            "stored scopes of sku {} contain an empty token",
            child.sku_id
        ))));
    };
    let child_pair = parent_pair.resolve_child(region_input, brand_input);
    if let Err(failure) = parent_pair.check_containment(&child_pair) {
        let refusal = scope_not_contained_domain_err(failure)?;
        let entry = child_failed_entry(child.sku_id, &refusal);
        audit_failed_child(
            state,
            sku_scope,
            tenant_id,
            actor_ref,
            source.sku_code.clone(),
            refusal,
        )
        .await;
        return Ok(entry);
    }

    // The first-free walk (P-D-62), unflavored: SKU codes have no -revived.
    let mut code_n: u32 = 1;
    for _attempt in 0..CLONE_SUGGESTION_ATTEMPTS {
        let code = disposition::suggested_sku_code(&source, code_n);
        let new = NewSku {
            sku_id: Uuid::new_v4(),
            tenant_id,
            product_id: parent.product_id,
            sku_code: code.clone(),
            region_scope: child_pair.region.render(),
            brand_scope: child_pair.brand.render(),
            created_by: actor_ref.to_string(),
            created_at: now,
            cloned_from: Some(child.sku_id),
            cloned_from_version: source.read_at_version,
        };
        match insert_sku_with_event(state, sku_scope.clone(), new, None, actor_ref).await {
            Ok(CreateOutcome::Created { body, .. }) => {
                let new_sku_id = body
                    .get("sku_id")
                    .and_then(JsonValue::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .ok_or_else(|| {
                        repo_error_to_canonical(&RepoError::Db(
                            "a created SKU rendered without its sku_id".to_owned(),
                        ))
                    })?;
                return Ok(child_created_entry(child.sku_id, new_sku_id, &code));
            }
            // Unkeyed inserts have no idempotency phase: neither arm is
            // reachable, and saying so beats absorbing them silently.
            Ok(CreateOutcome::Replay { .. } | CreateOutcome::Refused(_)) => {
                return Err(repo_error_to_canonical(&RepoError::Db(
                    "an unkeyed child insert answered an idempotency outcome".to_owned(),
                )));
            }
            Err(db_error) => {
                let message = db_error.to_string();
                if classify_sku_insert_conflict(&message) {
                    code_n += 1;
                } else {
                    return Err(repo_error_to_canonical(&RepoError::Db(message)));
                }
            }
        }
    }

    // Cap exhausted: the child fails alone with the family's own conflict,
    // the same honesty as the lone door's cap (P-D-62).
    let refusal = DomainError::DuplicateCode(format!(
        "sku_code \"{}\" and its first {} -copy-N successors are all reserved",
        source.sku_code, CLONE_SUGGESTION_ATTEMPTS
    ));
    let entry = child_failed_entry(child.sku_id, &refusal);
    audit_failed_child(
        state,
        sku_scope,
        tenant_id,
        actor_ref,
        source.sku_code.clone(),
        refusal,
    )
    .await;
    Ok(entry)
}

/// The family act's children phase (`inst-cn-children`, P-D-72, P-D-79):
/// clone every non-`discarded` child of `source_id` under `parent`, one
/// transaction per child, skipping sources the parent's own children
/// already name — the resume re-entry and the fresh act are the same walk,
/// the fresh act's skip set merely empty.
async fn clone_family_children(
    state: &ApiState,
    sku_scope: &AccessScope,
    stamp: ActStamp,
    source_id: Uuid,
    parent: &ProductRecord,
) -> Result<Vec<JsonValue>, CanonicalError> {
    let tenant_id = stamp.tenant_id;
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&RepoError::Db(format!("family children connection: {e}")))
    })?;
    let source_children = repo::find_skus_of_product(&conn, sku_scope, tenant_id, source_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let already_cloned = repo::find_skus_of_product(&conn, sku_scope, tenant_id, parent.product_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    let mut cloned_by_source: std::collections::HashMap<Uuid, &SkuRecord> =
        std::collections::HashMap::new();
    for row in &already_cloned {
        if let Some(from) = row.cloned_from {
            cloned_by_source.entry(from).or_insert(row);
        }
    }

    let parent_pair = parent_scope_pair(parent).map_err(|column| {
        CanonicalError::internal(format!(
            "bss-products: the new parent's stored {column} contains an empty token"
        ))
        .create()
    })?;

    let mut receipt = Vec::with_capacity(source_children.len());
    for child in &source_children {
        if let Some(existing) = cloned_by_source.get(&child.sku_id) {
            receipt.push(child_created_entry(
                child.sku_id,
                existing.sku_id,
                &existing.sku_code,
            ));
            continue;
        }
        receipt
            .push(clone_family_child(state, sku_scope, stamp, parent, &parent_pair, child).await?);
    }
    Ok(receipt)
}

/// Store the family act's answer at completion — tolerant of `NotHeld`,
/// unlike the create door's in-transaction write: a concurrent same-key
/// resumer may have answered first (P-D-79 states the race), and the first
/// stored receipt is then the honest one for every later replay.
async fn answer_family_act(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    claim: &IdempotencyClaimInput,
    body: &JsonValue,
) -> Result<(), CanonicalError> {
    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&RepoError::Db(format!("family answer connection: {e}")))
    })?;
    repo::answer_idempotency_key(
        &conn,
        scope,
        tenant_id,
        &claim.endpoint,
        &claim.client_key,
        i32::from(CREATE_RESPONSE_STATUS.as_u16()),
        body.clone(),
    )
    .await
    .map(|_recorded_or_not_held| ())
    .map_err(|e| repo_error_to_canonical(&e))
}

/// Finish the family act from a landed or resumed parent: the children
/// phase, the receipt, the stored answer, the response.
/// The parent a committed-but-unanswered claim names (P-D-72): read back
/// for the resume re-entry, a missing row being a store that contradicts
/// its own stamp.
async fn resumed_parent(
    state: &ApiState,
    product_scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    parent_id: Uuid,
) -> Result<ProductRecord, CanonicalError> {
    let conn = state
        .db
        .conn()
        .map_err(|e| repo_error_to_canonical(&RepoError::Db(format!("resume connection: {e}"))))?;
    repo::find_product(&conn, product_scope, tenant_id, parent_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .ok_or_else(|| {
            repo_error_to_canonical(&RepoError::CorruptRow(format!(
                "the claim on {endpoint} names parent {parent_id}, which does not \
                 resolve in this tenant"
            )))
        })
}

/// [`finish_family_act`] behind a boxed parent — the walk hands the
/// [`CloneParentOutcome::Created`] box straight through.
async fn finish_family_act_boxed(
    state: &ApiState,
    product_scope: &AccessScope,
    sku_scope: &AccessScope,
    stamp: ActStamp,
    source_id: Uuid,
    parent: Box<ProductRecord>,
    claim: Option<&IdempotencyClaimInput>,
) -> Result<Response, CanonicalError> {
    finish_family_act(
        state,
        product_scope,
        sku_scope,
        stamp,
        source_id,
        *parent,
        claim,
    )
    .await
}

async fn finish_family_act(
    state: &ApiState,
    product_scope: &AccessScope,
    sku_scope: &AccessScope,
    stamp: ActStamp,
    source_id: Uuid,
    parent: ProductRecord,
    claim: Option<&IdempotencyClaimInput>,
) -> Result<Response, CanonicalError> {
    let tenant_id = stamp.tenant_id;
    let children = clone_family_children(state, sku_scope, stamp, source_id, &parent).await?;

    let internal_revision = parent.internal_revision;
    let mut body = serde_json::to_value(ProductView::from(parent)).map_err(|e| {
        repo_error_to_canonical(&RepoError::Db(format!("render the cloned Product: {e}")))
    })?;
    if let Some(map) = body.as_object_mut() {
        map.insert("children".to_owned(), JsonValue::Array(children));
    }

    if let Some(input) = claim {
        answer_family_act(state, product_scope, tenant_id, input, &body).await?;
    }

    let tag = preconditions::etag(InternalRevision::new(internal_revision));
    Ok((CREATE_RESPONSE_STATUS, [(ETAG, tag)], Json(body)).into_response())
}

/// `POST /bss-products/v1/products/{id}/clone` — the door `inst-cn-door`
/// states, P-D-75 shaped, and P-D-79 made the family act (`CloneDoor`,
/// `design/11` §1.7).
///
/// The act: both gates, the keyed claim on the concrete path, the source
/// read where its state says, the parent's first-free walk (P-D-62), the
/// parent's transaction (claim + row + `entity_ref` stamp + outbox), then
/// one transaction per child and the receipt stored as the answer at
/// completion (P-D-72). A committed-but-unanswered claim on a keyed retry
/// resumes the children phase instead of replaying or refusing.
///
/// @cpt-dod:cpt-cf-bss-products-dod-clone-door:p1
/// @cpt-dod:cpt-cf-bss-products-dod-clone-children:p1
/// @cpt-dod:cpt-cf-bss-products-dod-rename-rule:p1
async fn clone_product(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    headers: HeaderMap,
    Path(source_id): Path<Uuid>,
    Json(body): Json<CloneProductRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let payload_hash = clone_payload_digest(&body);

    let name_override = body.name.as_deref().map(str::trim).map(str::to_owned);
    let code_override = body.code.as_deref().map(str::trim).map(str::to_owned);

    // -- actor_ref, then the two gates: the create door's own order,
    // spent against both resources unconditionally (P-D-79). --
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let (product_scope, sku_scope) =
        clone_write_scopes(&state, &enforcer, &ctx, tenant_id, actor_ref, source_id).await?;
    let stamp = ActStamp {
        tenant_id,
        actor_ref,
        now,
    };

    // -- shape: a supplied override must survive its own trim. --
    let mut report = ValidationReport::new();
    if name_override.as_deref() == Some("") {
        report.violate("VALIDATION", "name", "name override must not be blank");
    }
    if code_override.as_deref() == Some("") {
        report.violate("VALIDATION", "code", "code override must not be blank");
    }
    if !report.is_empty() {
        let domain_err = DomainError::Validation(report);
        return Err(refuse_clone(
            &state,
            &product_scope,
            tenant_id,
            actor_ref,
            domain_err.code(),
            source_id.to_string(),
            CanonicalError::from(domain_err),
        )
        .await);
    }

    // -- the idempotency claim, on the clone's own concrete path. --
    let client_key = match idempotency_key(&headers) {
        Ok(key) => key,
        Err(domain_err) => {
            return Err(refuse_clone(
                &state,
                &product_scope,
                tenant_id,
                actor_ref,
                domain_err.code(),
                source_id.to_string(),
                CanonicalError::from(domain_err),
            )
            .await);
        }
    };
    let endpoint = clone_endpoint(source_id);
    let claim = client_key.map(|key| {
        IdempotencyClaimInput::new(
            endpoint.clone(),
            key,
            payload_hash,
            now,
            state.idempotency_retention_hours,
        )
    });

    // -- the source, read where its state says (a refused state answers
    // here). --
    let source = match resolve_clone_source(&state, &product_scope, tenant_id, source_id).await {
        Ok(Some(source)) => source,
        Ok(None) => {
            return Err(product_not_found(source_id));
        }
        Err(canonical) => {
            // The discarded refusal is the audited path; a corrupt store is
            // the 500 the mapping already classifies.
            if canonical.status_code() == 409 {
                return Err(refuse_clone(
                    &state,
                    &product_scope,
                    tenant_id,
                    actor_ref,
                    "CLONE_SOURCE_DISCARDED",
                    source_id.to_string(),
                    canonical,
                )
                .await);
            }
            return Err(canonical);
        }
    };

    // -- the parent's first-free walk (P-D-62): the index arbitrates, the
    // loop only moves to the next candidate on the exact conflict its
    // candidate owns. --
    let mut name_n: u32 = 1;
    let mut code_n: u32 = 1;
    for _attempt in 0..CLONE_SUGGESTION_ATTEMPTS {
        let name = name_override
            .clone()
            .unwrap_or_else(|| disposition::suggested_product_name(&source, name_n));
        let code = code_override
            .clone()
            .or_else(|| disposition::suggested_product_code(&source, code_n));

        let new = NewProduct {
            product_id: Uuid::new_v4(),
            tenant_id,
            brand_id: source.brand_id,
            name: name.clone(),
            name_normalized: name::normalize(&name),
            product_code: code.clone(),
            region_scope: source.region_scope.clone(),
            brand_scope: source.brand_scope.clone(),
            created_by: actor_ref.to_string(),
            created_at: now,
            cloned_from: Some(source_id),
            cloned_from_version: source.read_at_version,
        };

        match insert_clone_parent(&state, product_scope.clone(), new, claim.clone(), actor_ref)
            .await
        {
            Ok(CloneParentOutcome::Created { record }) => {
                return finish_family_act_boxed(
                    &state,
                    &product_scope,
                    &sku_scope,
                    stamp,
                    source_id,
                    record,
                    claim.as_ref(),
                )
                .await;
            }
            Ok(CloneParentOutcome::Resume { parent_id }) => {
                // The committed-but-unanswered claim (P-D-72): the parent
                // exists; re-enter the children phase against it.
                let parent =
                    resumed_parent(&state, &product_scope, tenant_id, &endpoint, parent_id).await?;
                return finish_family_act(
                    &state,
                    &product_scope,
                    &sku_scope,
                    stamp,
                    source_id,
                    parent,
                    claim.as_ref(),
                )
                .await;
            }
            Ok(CloneParentOutcome::Replay { status, body }) => {
                return Ok(replay_response(status, body));
            }
            Ok(CloneParentOutcome::Refused(domain_err)) => {
                return Err(refuse_clone(
                    &state,
                    &product_scope,
                    tenant_id,
                    actor_ref,
                    domain_err.code(),
                    name,
                    CanonicalError::from(domain_err),
                )
                .await);
            }
            Err(db_error) => {
                let message = db_error.to_string();
                match classify_insert_conflict(&message) {
                    // A suggested candidate lost its reservation: that is the
                    // walk, not a refusal (P-D-62). An *overridden* value
                    // that lost is the ordinary audited refusal, the
                    // operator's own collision.
                    Some(InsertConflict::DuplicateName) if name_override.is_none() => {
                        name_n += 1;
                    }
                    Some(InsertConflict::DuplicateCode) if code_override.is_none() => {
                        code_n += 1;
                    }
                    Some(conflict) => {
                        return Err(refuse_insert_conflict(
                            &state,
                            &product_scope,
                            tenant_id,
                            actor_ref,
                            conflict,
                            &name,
                            code.as_deref(),
                        )
                        .await);
                    }
                    None => {
                        return Err(repo_error_to_canonical(&RepoError::Db(message)));
                    }
                }
            }
        }
    }

    // The cap is operational, not semantic: surface the family's own
    // conflict rather than invent a new refusal for it.
    let exhausted = if name_override.is_none() {
        InsertConflict::DuplicateName
    } else {
        InsertConflict::DuplicateCode
    };
    Err(refuse_insert_conflict(
        &state,
        &product_scope,
        tenant_id,
        actor_ref,
        exhausted,
        &disposition::suggested_product_name(&source, name_n),
        disposition::suggested_product_code(&source, code_n).as_deref(),
    )
    .await)
}
