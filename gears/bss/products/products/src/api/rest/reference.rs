//! The watermark door, the producer registry's two ops, and the reference
//! predicate — `design/07-reference-signal.md` §2 (`dod-watermark-door`,
//! `dod-reference-predicate`; P-D-71, P-D-87).
//!
//! # The post is a complete set, and its four refusals
//!
//! `POST /bss-products/v1/reference-watermarks` (`reference_signal x post`,
//! S2S) is the out-of-process binding of
//! [`WatermarkPosts`](bss_products_sdk::watermarks::WatermarkPosts);
//! [`InProcessWatermarkPosts`] is the in-process one, and both pass the
//! same gate. An unregistered poster is `PRODUCER_UNREGISTERED`; an older
//! `watermark_at` is `WATERMARK_REGRESSION`; an **equal** one carrying a
//! different set is `WATERMARK_CONFLICT`, told apart from the admitted
//! idempotent replay by the stored `set_hash` (**P-D-71**); and one above
//! the receiving clock plus `watermark_skew_tolerance_minutes` is
//! `WATERMARK_FUTURE`, **alerted as well as refused**.
//!
//! **The future bound's chain is why it is `p1` rather than hygiene**: one
//! accepted future-dated post makes its producer read permanently fresh, so
//! the staleness alarm never fires; every later legitimate post is refused
//! `WATERMARK_REGRESSION`, freezing its member set; and every SKU outside
//! that frozen set then reads **fresh-zero** — the never-falsely-free
//! invariant inverted by one timestamp.
//!
//! # The predicate: four verdicts, an OR and never a sum
//!
//! [`evaluate_reference`] runs over every **registered** producer of the
//! tenant and returns the OR verdict with the per-producer detail. A fresh
//! watermark containing the SKU gives `referenced`; a fresh one omitting it
//! gives that producer zero; a stale one gives
//! `conservatively_referenced(stale)` — the condition
//! `reference_watermark_stale` fires on, this evaluation emitting nothing
//! (**P-D-59**); never-received gives
//! `conservatively_referenced(never_received)` under a **distinct** flag
//! and no alarm. **Fresh-zero requires every registered producer to be
//! fresh AND omitting the SKU**, and with zero registered producers the
//! answer is `no_producers` — conservative, distinct from fresh-zero, and
//! defensive-unreachable in v1 only because the last-producer retirement is
//! refused, which is a rule a later decision could relax.
//!
//! A boolean OR and **not a sum**: a count would make two producers holding
//! one SKU look like two references, and the contract is a complete set per
//! producer, not a tally.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-watermark-door:p1
//! @cpt-dod:cpt-cf-bss-products-dod-reference-predicate:p1
//!
//! # Why `dod-reference-audit` is reached and not claimed
//!
//! It obliges a row for *"every producer registration and retirement, every
//! correction on either lane, and every watermark post — **accepted or
//! refused**"*. Three of those four acts have doors here and all three now
//! audit both outcomes. The **corrections** lane does not: its door,
//! `POST /skus/{skuId}/corrections`, is declared by `design/07` and
//! registered nowhere, so there is no act to audit and no ceremony
//! reference to carry. The `DoD`'s join clause — the audit row carrying the
//! same `ceremony_ref` the evidence table stores — arrives with that door.
//!
//! @cpt-cf-bss-products-dod-reference-audit

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use toolkit::api::OpenApiRegistry;
use toolkit::api::canonical_prelude::{CanonicalError, resource_error};
use toolkit::api::operation_builder::OperationBuilder;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use bss_products_sdk::watermarks::{WatermarkAck, WatermarkPost, WatermarkPosts};

use crate::api::rest::dto::{
    PostWatermarkRequest, ProducerView, RegisterProducerRequest, WatermarkAckView,
};
use crate::api::rest::{ApiState, repo_error_to_canonical, require_authenticated};
use crate::domain::canonical;
use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;
use crate::infra::storage::repo::{self, RefusalSubject};

/// `OpenAPI` tag for the reference surface's operations.
const TAG: &str = "BSS Products";

/// The canonical-error identity of this surface's refusals.
#[resource_error(gts_id!("cf.bss.products.reference_signal.v1~"))]
struct ReferenceResource;

/// One producer's verdict for one SKU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProducerVerdict {
    /// Fresh, and the set holds the SKU.
    Referenced,
    /// Fresh, and the set omits it — the only verdict that frees.
    FreshZero,
    /// The watermark is older than the freshness threshold.
    ConservativelyReferencedStale,
    /// The producer has never posted — a **distinct** flag, and no alarm.
    ConservativelyReferencedNeverReceived,
}

/// The predicate's answer: the OR verdict and the per-producer detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceEvaluation {
    /// `true` unless every registered producer is fresh and omits the SKU.
    pub referenced: bool,
    /// The fourth verdict: no registered producer at all. Conservative,
    /// and distinct from fresh-zero.
    pub no_producers: bool,
    /// One entry per registered producer.
    pub per_producer: Vec<(String, ProducerVerdict)>,
}

/// Evaluate the reference predicate for one SKU (`dod-reference-predicate`).
///
/// # Errors
///
/// [`crate::infra::storage::RepoError`] as the reads raise it.
pub async fn evaluate_reference(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    now: DateTime<Utc>,
    freshness: std::time::Duration,
) -> Result<ReferenceEvaluation, crate::infra::storage::RepoError> {
    let producers: Vec<String> = repo::reference_producers(runner, scope, tenant_id)
        .await?
        .into_iter()
        .filter(|row| row.state == crate::domain::states::ProducerState::Registered)
        .map(|row| row.producer)
        .collect();

    if producers.is_empty() {
        return Ok(ReferenceEvaluation {
            referenced: true,
            no_producers: true,
            per_producer: Vec::new(),
        });
    }

    let stale_before =
        now - chrono::Duration::from_std(freshness).unwrap_or_else(|_| chrono::Duration::zero());
    let mut per_producer = Vec::with_capacity(producers.len());
    let mut referenced = false;
    for producer in producers {
        let verdict =
            match repo::find_reference_watermark(runner, scope, tenant_id, &producer).await? {
                None => ProducerVerdict::ConservativelyReferencedNeverReceived,
                Some(watermark) if watermark.watermark_at < stale_before => {
                    ProducerVerdict::ConservativelyReferencedStale
                }
                Some(_) => {
                    if repo::reference_member_exists(runner, scope, tenant_id, &producer, sku_id)
                        .await?
                    {
                        ProducerVerdict::Referenced
                    } else {
                        ProducerVerdict::FreshZero
                    }
                }
            };
        // The OR, never a sum: anything but a fresh omission holds the SKU.
        if verdict != ProducerVerdict::FreshZero {
            referenced = true;
        }
        per_producer.push((producer, verdict));
    }

    Ok(ReferenceEvaluation {
        referenced,
        no_producers: false,
        per_producer,
    })
}

/// Return the pinned production connection before a transaction checks one
/// out again. `DbConn` is not `Drop`, so a bare `drop` reads as a mistake to
/// clippy; the named function says what the line is for.
fn return_pinned<T>(conn: T) {
    let _returned = conn;
}

/// The hex digest of a posted set — the `set_hash` column's operand, taken
/// over the canonical rendering of the **sorted** id list so two posts of
/// one set hash alike whatever order they arrived in (P-D-80's rule for a
/// keyed collection, applied to the only collection this door carries).
fn set_hash(sku_ids: &[Uuid]) -> String {
    let mut ids: Vec<String> = sku_ids.iter().map(ToString::to_string).collect();
    ids.sort();
    ids.dedup();
    let rendering = canonical::canonical_rendering(
        &serde_json::Value::Array(ids.into_iter().map(serde_json::Value::String).collect()),
        canonical::Absence::Omit,
    );
    let digest = canonical::content_digest(&rendering);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            hex.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            hex.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
            hex
        })
}

/// Build the Axum router for the watermark door and the producer registry.
pub(crate) fn router(state: Arc<ApiState>, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-products/v1/reference-watermarks")
        .operation_id("bss_products.post_reference_watermark")
        .summary("Post a reference watermark")
        .description(
            "Records one registered producer's COMPLETE SKU set as of watermark_at, replacing \
             the stored set. Refuses an unregistered poster PRODUCER_UNREGISTERED (403), an \
             older watermark_at WATERMARK_REGRESSION, an equal watermark_at carrying a different \
             set WATERMARK_CONFLICT, and one above the receiving clock plus the configured skew \
             WATERMARK_FUTURE. An equal watermark_at with the same set is the admitted \
             idempotent replay. Gates on reference_signal x post (S2S).",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<PostWatermarkRequest>(openapi, "The producer's complete set.")
        .handler(post_watermark)
        .json_response_with_schema::<WatermarkAckView>(
            openapi,
            StatusCode::OK,
            "The stored watermark.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(Router::new(), openapi);
    register_producer_routes(router, openapi).layer(Extension(state))
}

/// Register the two membership ops (P-D-87 arm 3's routes).
fn register_producer_routes(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::post("/bss-products/v1/reference-producers")
        .operation_id("bss_products.register_reference_producer")
        .summary("Register a reference producer")
        .description(
            "Adds a producer to the tenant's registered set, widening the predicate's \
             quantifier. A registering producer starts never-received, so onboarding can only \
             tighten: a re-registered producer finds no watermark, its retirement having cleared \
             one (P-D-87). Gates on reference_producer x write.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .json_request::<RegisterProducerRequest>(openapi, "The producer to register.")
        .handler(register_producer)
        .json_response_with_schema::<ProducerView>(
            openapi,
            StatusCode::CREATED,
            "The registered producer.",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi);

    OperationBuilder::post("/bss-products/v1/reference-producers/{producer}/retirements")
        .operation_id("bss_products.retire_reference_producer")
        .summary("Retire a reference producer")
        .description(
            "Moves a producer to retired and CLEARS its watermark and member rows in the same \
             transaction (P-D-87): surviving rows would let retire-then-re-register inside the \
             freshness window read fresh against a stale set. The producer row itself stays, so \
             the registration history is not lost. Gates on reference_producer x write.",
        )
        .tag(TAG)
        .authenticated()
        .no_license_required()
        .path_param("producer", "The producer to retire.")
        .handler(retire_producer)
        .json_response_with_schema::<ProducerView>(openapi, StatusCode::OK, "The retired producer.")
        .error_400(openapi)
        .error_401(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_503(openapi)
        .register(router, openapi)
}

/// Which of the surface's two labels a call spends, and for what — one
/// operand instead of three, so the gate stays under the argument bar.
#[derive(Clone, Copy)]
struct GateTarget {
    resource: &'static authz_resolver_sdk::ResourceType,
    label: &'static str,
    action: &'static str,
}

/// The watermark door's target: `reference_signal x post`.
const POST_TARGET: GateTarget = GateTarget {
    resource: &crate::authz::resource_types::REFERENCE_SIGNAL,
    label: crate::authz::labels::REFERENCE_SIGNAL,
    action: crate::authz::actions::POST,
};

/// The membership ops' target: `reference_producer x write`.
const PRODUCER_TARGET: GateTarget = GateTarget {
    resource: &crate::authz::resource_types::REFERENCE_PRODUCER,
    label: crate::authz::labels::REFERENCE_PRODUCER,
    action: crate::authz::actions::WRITE,
};

/// The reference surface's gate.
async fn reference_scope(
    state: &ApiState,
    enforcer: &authz_resolver_sdk::PolicyEnforcer,
    ctx: &SecurityContext,
    tenant_id: Uuid,
    actor_ref: Uuid,
    target: GateTarget,
    subject: String,
) -> Result<AccessScope, CanonicalError> {
    let GateTarget {
        resource,
        label,
        action,
    } = target;
    match crate::authz::access_scope(enforcer, ctx, resource, action, Some(tenant_id), None, true)
        .await
    {
        Ok(scope) => Ok(scope),
        Err(crate::authz::AuthzError::Denied(reason)) => {
            let self_scope = AccessScope::for_tenant(tenant_id);
            Err(crate::api::rest::audit_refusal_and_report(
                state,
                &self_scope,
                crate::api::rest::RefusalAuditContext {
                    tenant_id,
                    actor_ref,
                    subject_kind: label,
                    error_code: "PERMISSION_DENIED",
                },
                RefusalSubject::Attempted(subject),
                ReferenceResource::permission_denied()
                    .with_reason(reason)
                    .create(),
            )
            .await)
        }
        Err(err @ crate::authz::AuthzError::Unavailable(_)) => {
            Err(crate::api::rest::authz_error_to_canonical(err, |reason| {
                ReferenceResource::permission_denied()
                    .with_reason(reason)
                    .create()
            }))
        }
    }
}

/// What the post claimed, as the answer needs it — grouped for
/// [`AcceptedAct`]'s reason: five loose operands, three of which a call site
/// could transpose silently.
struct PostedFacts<'a> {
    /// The acting tenant.
    tenant_id: Uuid,
    /// The pseudonymous actor.
    actor_ref: Uuid,
    /// The producer, as the refusals name it.
    subject: &'a str,
    /// The instant the producer claimed.
    watermark_at: DateTime<Utc>,
    /// How many members the post carried.
    member_count: usize,
}

/// Turn the watermark transaction's verdict into the caller's answer.
///
/// Lifted out of [`record_watermark`] because that function crossed clippy's
/// `too_many_lines` floor when the accepted post gained its audit row — and
/// because the verdict-to-answer mapping is a table, which reads better as
/// one.
async fn answer_watermark(
    state: &ApiState,
    scope: &AccessScope,
    posted: PostedFacts<'_>,
    verdict: WatermarkTxVerdict,
) -> Result<WatermarkAck, CanonicalError> {
    let PostedFacts {
        tenant_id,
        actor_ref,
        subject,
        watermark_at,
        member_count,
    } = posted;
    match verdict {
        WatermarkTxVerdict::Written => Ok(WatermarkAck {
            watermark_at,
            member_count,
            replayed: false,
        }),
        WatermarkTxVerdict::Replayed => Ok(WatermarkAck {
            watermark_at,
            member_count,
            replayed: true,
        }),
        WatermarkTxVerdict::Regression { stored_at } => {
            let refusal = DomainError::WatermarkRegression(format!(
                "watermark_at {watermark_at} is older than the stored {stored_at}"
            ));
            Err(refuse_reference(
                state,
                scope,
                tenant_id,
                actor_ref,
                crate::authz::labels::REFERENCE_SIGNAL,
                subject.to_owned(),
                refusal,
            )
            .await)
        }
        WatermarkTxVerdict::Conflict => {
            let refusal = DomainError::WatermarkConflict(format!(
                "watermark_at {watermark_at} was already posted with a different set"
            ));
            Err(refuse_reference(
                state,
                scope,
                tenant_id,
                actor_ref,
                crate::authz::labels::REFERENCE_SIGNAL,
                subject.to_owned(),
                refusal,
            )
            .await)
        }
    }
}

/// The five operands an accepted act's audit row needs, grouped because
/// they always travel together and three are strings a call site could
/// transpose without the compiler noticing.
struct AcceptedAct<'a> {
    /// The acting tenant.
    tenant_id: Uuid,
    /// The pseudonymous actor.
    actor_ref: Uuid,
    /// The `products_audit_log.action` token.
    action: &'a str,
    /// The subject kind the row records under.
    label: &'static str,
    /// The subject itself.
    subject: String,
}

/// Write the audit row an **accepted** act of this surface owes
/// (`dod-reference-audit`).
///
/// # Why every accepted act, and not only the refusals
///
/// The `DoD` obliges a row for *"every producer registration and retirement,
/// every correction on either lane, and every watermark post — **accepted or
/// refused**"*, and this surface had only the refusal half. The accepted
/// half is the one the investigation needs: ingestion is audit-plane by
/// design (`inst-ws-no-event` — *"watermarks arrive continuously and are
/// queryable state, not domain history"*), so **no event records a post**,
/// and the watermark table is overwritten in place. An accepted post that
/// left no row would make the `WATERMARK_FUTURE` chain uninvestigable,
/// because the row it overwrote was the only other record of what the
/// producer claimed.
///
/// # It rides the caller's runner
///
/// Every caller passes the runner its own act ran on, so the evidence and
/// the act commit together — a row written on a second connection could
/// survive an act that rolled back, which is the failure an audit plane
/// exists to not have.
///
/// # Errors
///
/// [`RepoError`] where the audit row could not be written. **The caller must
/// not swallow it**: `01` §4.4 makes an unwritable audit row the act's own
/// failure, answered `AUDIT_UNAVAILABLE`.
async fn audit_accepted_act(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &AccessScope,
    act: AcceptedAct<'_>,
    now: DateTime<Utc>,
) -> Result<(), crate::infra::storage::RepoError> {
    let AcceptedAct {
        tenant_id,
        actor_ref,
        action,
        label,
        subject,
    } = act;
    repo::write_keyed_act_audit(
        runner,
        scope,
        repo::AuditCommon {
            audit_id: Uuid::new_v4(),
            tenant_id,
            actor_ref,
            action: action.to_owned(),
            subject_kind: label.to_owned(),
            reason: None,
            correlation_id: None,
            written_at: now,
        },
        subject,
    )
    .await
}

/// One audited refusal of the reference surface.
async fn refuse_reference(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    label: &'static str,
    subject: String,
    refusal: DomainError,
) -> CanonicalError {
    let code = refusal.code();
    crate::api::rest::audit_refusal_and_report(
        state,
        scope,
        crate::api::rest::RefusalAuditContext {
            tenant_id,
            actor_ref,
            subject_kind: label,
            error_code: code,
        },
        RefusalSubject::Attempted(subject),
        CanonicalError::from(refusal),
    )
    .await
}

/// The post's core, shared by both bindings: the registered check, the
/// three timestamp verdicts, and the replace-in-one-transaction write.
async fn record_watermark(
    state: &ApiState,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    post: WatermarkPost,
    now: DateTime<Utc>,
    skew: std::time::Duration,
) -> Result<WatermarkAck, CanonicalError> {
    let WatermarkPost {
        producer,
        watermark_at,
        sku_ids,
    } = post;
    let producer = producer.trim().to_owned();
    let subject = producer.clone();
    if producer.is_empty() {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "producer", "producer must not be blank");
        return Err(refuse_reference(
            state,
            scope,
            tenant_id,
            actor_ref,
            crate::authz::labels::REFERENCE_SIGNAL,
            subject,
            DomainError::Validation(report),
        )
        .await);
    }

    // The set bound, shared by both bindings so the in-process one cannot
    // bypass an HTTP-layer body limit: a single post above the operator's
    // own row ceiling is an accident, not a catalog — refused before
    // anything is hashed or written.
    let max_set_size = state.bulk_max_rows_per_batch as usize;
    if sku_ids.len() > max_set_size {
        let mut report = ValidationReport::new();
        report.violate(
            "VALIDATION",
            "sku_ids",
            format!(
                "sku_ids carries {} entries, above the configured ceiling of {max_set_size}",
                sku_ids.len()
            ),
        );
        return Err(refuse_reference(
            state,
            scope,
            tenant_id,
            actor_ref,
            crate::authz::labels::REFERENCE_SIGNAL,
            subject,
            DomainError::Validation(report),
        )
        .await);
    }

    // The contract is a set: a duplicated id is deduplicated once at the
    // door, so the hash, the stored rows and the acked member_count all
    // describe the same set instead of three different readings of one Vec.
    let mut members = sku_ids;
    members.sort_unstable();
    members.dedup();

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "watermark door connection: {e}"
        )))
    })?;

    let registered = repo::reference_producers(&conn, scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .into_iter()
        .any(|row| {
            row.producer == producer
                && row.state == crate::domain::states::ProducerState::Registered
        });
    if !registered {
        let refusal = DomainError::ProducerUnregistered(format!(
            "\"{producer}\" is not in this tenant's registered producer set"
        ));
        return Err(refuse_reference(
            state,
            scope,
            tenant_id,
            actor_ref,
            crate::authz::labels::REFERENCE_SIGNAL,
            subject,
            refusal,
        )
        .await);
    }

    // The future bound, alerted as well as refused.
    let ceiling =
        now + chrono::Duration::from_std(skew).unwrap_or_else(|_| chrono::Duration::zero());
    if watermark_at > ceiling {
        tracing::warn!(
            %producer,
            %watermark_at,
            "bss-products: watermark_future"
        );
        let refusal = DomainError::WatermarkFuture(format!(
            "watermark_at {watermark_at} is above the receiving clock plus the configured skew"
        ));
        return Err(refuse_reference(
            state,
            scope,
            tenant_id,
            actor_ref,
            crate::authz::labels::REFERENCE_SIGNAL,
            subject,
            refusal,
        )
        .await);
    }

    return_pinned(conn);

    // The regression/conflict/replay verdict and the replacing write run in
    // ONE serializable transaction: an older post that loses the race to a
    // newer one re-reads the winner's row on retry and is refused
    // WATERMARK_REGRESSION, instead of silently regressing the head the way
    // a check on a plain connection allowed.
    let hash = set_hash(&members);
    let member_count = members.len();
    let scope_for_tx = scope.clone();
    let producer_for_tx = producer.clone();
    let members_for_tx = members;
    let hash_for_tx = hash.clone();
    let verdict = state
        .db
        .db()
        .transaction_with_retry::<WatermarkTxVerdict, toolkit_db::DbError, _, _>(
            toolkit_db::secure::TxConfig::serializable(),
            crate::api::rest::contention_db_err,
            move |tx| {
                let scope = scope_for_tx.clone();
                let producer = producer_for_tx.clone();
                let members = members_for_tx.clone();
                let hash = hash_for_tx.clone();
                Box::pin(async move {
                    let stored = repo::find_reference_watermark(tx, &scope, tenant_id, &producer)
                        .await
                        .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                    if let Some(stored) = stored {
                        if watermark_at < stored.watermark_at {
                            return Ok(WatermarkTxVerdict::Regression {
                                stored_at: stored.watermark_at,
                            });
                        }
                        if watermark_at == stored.watermark_at {
                            // P-D-71: the stored hash is what tells the
                            // admitted replay from the conflict.
                            return Ok(if stored.set_hash == hash {
                                WatermarkTxVerdict::Replayed
                            } else {
                                WatermarkTxVerdict::Conflict
                            });
                        }
                    }
                    repo::post_reference_watermark(
                        tx,
                        &scope,
                        tenant_id,
                        repo::PostedWatermark {
                            producer: &producer,
                            watermark_at,
                            posted_at: now,
                            set_hash: &hash,
                            members: &members,
                        },
                    )
                    .await
                    .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                    // The accepted post's audit row, inside the same
                    // serializable transaction as the write it records.
                    // Ingestion emits NO broker event by design
                    // (`inst-ws-no-event`) and the watermark row is
                    // OVERWRITTEN in place, so this row is the only record
                    // of what the producer claimed — without it the
                    // `WATERMARK_FUTURE` chain is uninvestigable.
                    audit_accepted_act(
                        tx,
                        &scope,
                        AcceptedAct {
                            tenant_id,
                            actor_ref,
                            action: "reference_watermark_post",
                            label: crate::authz::labels::REFERENCE_PRODUCER,
                            subject: producer.clone(),
                        },
                        now,
                    )
                    .await
                    .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                    Ok(WatermarkTxVerdict::Written)
                })
            },
        )
        .await
        .map_err(|e| {
            repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
        })?;

    answer_watermark(
        state,
        scope,
        PostedFacts {
            tenant_id,
            actor_ref,
            subject: &subject,
            watermark_at,
            member_count,
        },
        verdict,
    )
    .await
}

/// What the watermark transaction decided — carried out of the closure so
/// the audited refusals (which want the door's state and scope) run outside
/// the retry loop rather than auditing once per attempt.
enum WatermarkTxVerdict {
    /// The head and its member set were replaced.
    Written,
    /// The admitted idempotent replay: equal instant, equal set hash.
    Replayed,
    /// The posted instant is older than the stored head.
    Regression {
        /// The stored head's instant, named in the refusal.
        stored_at: DateTime<Utc>,
    },
    /// Equal instant, different set hash.
    Conflict,
}

/// `POST /bss-products/v1/reference-watermarks`.
async fn post_watermark(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<PostWatermarkRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = reference_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        POST_TARGET,
        body.producer.trim().to_owned(),
    )
    .await?;

    let ack = record_watermark(
        &state,
        &scope,
        tenant_id,
        actor_ref,
        WatermarkPost {
            producer: body.producer,
            watermark_at: body.watermark_at,
            sku_ids: body.sku_ids,
        },
        now,
        state.watermark_skew_tolerance,
    )
    .await?;

    Ok((
        StatusCode::OK,
        Json(WatermarkAckView {
            watermark_at: ack.watermark_at,
            member_count: ack.member_count,
            replayed: ack.replayed,
        }),
    )
        .into_response())
}

/// `POST /bss-products/v1/reference-producers`.
async fn register_producer(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    Json(body): Json<RegisterProducerRequest>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let producer = body.producer.trim().to_owned();
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = reference_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        PRODUCER_TARGET,
        producer.clone(),
    )
    .await?;

    if producer.is_empty() {
        let mut report = ValidationReport::new();
        report.violate("VALIDATION", "producer", "producer must not be blank");
        return Err(refuse_reference(
            &state,
            &scope,
            tenant_id,
            actor_ref,
            crate::authz::labels::REFERENCE_PRODUCER,
            producer,
            DomainError::Validation(report),
        )
        .await);
    }

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "producer door connection: {e}"
        )))
    })?;
    repo::register_reference_producer(&conn, &scope, tenant_id, &producer, None, now)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?;
    audit_accepted_act(
        &conn,
        &scope,
        AcceptedAct {
            tenant_id,
            actor_ref,
            action: "reference_producer_register",
            label: crate::authz::labels::REFERENCE_PRODUCER,
            subject: producer.clone(),
        },
        now,
    )
    .await
    .map_err(|e| repo_error_to_canonical(&e))?;

    Ok((
        StatusCode::CREATED,
        Json(ProducerView {
            producer,
            state: "registered".to_owned(),
        }),
    )
        .into_response())
}

/// `POST /bss-products/v1/reference-producers/{producer}/retirements`.
async fn retire_producer(
    Extension(state): Extension<Arc<ApiState>>,
    Extension(enforcer): Extension<authz_resolver_sdk::PolicyEnforcer>,
    extension_ctx: Option<Extension<SecurityContext>>,
    axum::extract::Path(producer): axum::extract::Path<String>,
) -> Result<Response, CanonicalError> {
    let ctx = require_authenticated(extension_ctx)?;
    let tenant_id = ctx.subject_tenant_id();
    let now = canonical::write_instant(Utc::now());
    let actor_ref =
        crate::api::rest::resolve_creator_actor_ref(&state, tenant_id, ctx.subject_id(), now)
            .await?;
    let scope = reference_scope(
        &state,
        &enforcer,
        &ctx,
        tenant_id,
        actor_ref,
        PRODUCER_TARGET,
        producer.clone(),
    )
    .await?;

    let conn = state.db.conn().map_err(|e| {
        repo_error_to_canonical(&crate::infra::storage::RepoError::Db(format!(
            "producer door connection: {e}"
        )))
    })?;
    let known = repo::reference_producers(&conn, &scope, tenant_id)
        .await
        .map_err(|e| repo_error_to_canonical(&e))?
        .into_iter()
        .any(|row| row.producer == producer);
    if !known {
        return Err(
            ReferenceResource::not_found("no such producer in the caller's tenant")
                .with_resource(producer)
                .create(),
        );
    }
    return_pinned(conn);

    let scope_for_tx = scope.clone();
    let producer_for_tx = producer.clone();
    state
        .db
        .db()
        .transaction_with_retry::<(), toolkit_db::DbError, _, _>(
            toolkit_db::secure::TxConfig::default(),
            crate::api::rest::contention_db_err,
            move |tx| {
                let scope = scope_for_tx.clone();
                let producer = producer_for_tx.clone();
                Box::pin(async move {
                    repo::retire_reference_producer(tx, &scope, tenant_id, &producer)
                        .await
                        .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                    // Inside the retirement's own transaction: the retirement
                    // clears the watermark and member rows too (P-D-87 arm
                    // 2), so an audit row on a second connection could
                    // outlive a rollback that put them back.
                    audit_accepted_act(
                        tx,
                        &scope,
                        AcceptedAct {
                            tenant_id,
                            actor_ref,
                            action: "reference_producer_retire",
                            label: crate::authz::labels::REFERENCE_PRODUCER,
                            subject: producer.clone(),
                        },
                        now,
                    )
                    .await
                    .map_err(|e| toolkit_db::DbError::Sea(e.to_db_err()))?;
                    Ok(())
                })
            },
        )
        .await
        .map_err(|e| {
            repo_error_to_canonical(&crate::infra::storage::RepoError::Db(e.to_string()))
        })?;

    Ok((
        StatusCode::OK,
        Json(ProducerView {
            producer,
            state: "retired".to_owned(),
        }),
    )
        .into_response())
}

/// The in-process binding, registered in `ClientHub` at boot — the default
/// deployment mode (P-D-15). Runs the identical gate and core.
pub(crate) struct InProcessWatermarkPosts {
    /// The door's own state.
    pub(crate) state: Arc<ApiState>,
    /// The platform PEP, the same instance the routers layer.
    pub(crate) enforcer: authz_resolver_sdk::PolicyEnforcer,
}

#[async_trait]
impl WatermarkPosts for InProcessWatermarkPosts {
    async fn post(
        &self,
        ctx: &SecurityContext,
        tenant_id: Uuid,
        post: WatermarkPost,
    ) -> Result<WatermarkAck, CanonicalError> {
        let now = canonical::write_instant(Utc::now());
        let actor_ref = crate::api::rest::resolve_creator_actor_ref(
            &self.state,
            tenant_id,
            ctx.subject_id(),
            now,
        )
        .await?;
        let scope = reference_scope(
            &self.state,
            &self.enforcer,
            ctx,
            tenant_id,
            actor_ref,
            POST_TARGET,
            post.producer.trim().to_owned(),
        )
        .await?;
        record_watermark(
            &self.state,
            &scope,
            tenant_id,
            actor_ref,
            post,
            now,
            self.state.watermark_skew_tolerance,
        )
        .await
    }
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod reference_tests;
