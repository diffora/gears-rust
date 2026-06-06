//! Read-only repo methods: `resolve_for_get`, `find_own`, `inventory`,
//! `scope_includes_tenant`.

use credstore_sdk::{OwnerId, SecretRef, SharingMode, TenantId};
use modkit_db::secure::{ScopeError, SecureEntityExt};
use modkit_security::access_scope::ScopeFilter;
use modkit_security::{AccessScope, pep_properties};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, FromQueryResult, QuerySelect};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::ports::metrics::SecretCounts;
use crate::domain::secret::model::{SecretRow, SecretStatus};
use crate::infra::canonical_mapping::classify_db_err_to_domain;
use crate::infra::storage::entity;
use crate::infra::storage::repo_impl::helpers::map_scope_err;
use crate::infra::storage::repo_impl::{
    SecretRepoImpl, entity_to_model, sharing_from_i16, sharing_to_i16,
};

/// Minimal projection for inventory aggregate rows.
#[derive(Debug, FromQueryResult)]
struct InventoryRow {
    sharing: i16,
    status: i16,
    c: i64,
}

/// Minimal projection for distinct-tenant count.
#[derive(Debug, FromQueryResult)]
struct TenantCount {
    t: i64,
}

fn scope_err_to_domain(e: ScopeError) -> DomainError {
    match e {
        ScopeError::Db(db) => classify_db_err_to_domain(db),
        other => map_scope_err(other),
    }
}

pub(super) async fn resolve_for_get(
    repo: &SecretRepoImpl,
    req_tenant: TenantId,
    subject: OwnerId,
    key: &SecretRef,
    chain: &[Uuid],
) -> Result<Option<SecretRow>, DomainError> {
    let conn = repo.db.conn()?;
    let req = req_tenant.0;
    // resolve_for_get applies its own chain + sharing predicates;
    // PDP authorization runs upstream. allow_all skips the scope WHERE clamp.
    let rows = entity::secrets::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(entity::secrets::Column::Reference.eq(key.as_ref())))
        .filter(
            Condition::all()
                .add(entity::secrets::Column::Status.eq(SecretStatus::Active.as_smallint())),
        )
        .filter(Condition::all().add(entity::secrets::Column::TenantId.is_in(chain.to_vec())))
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(
                            entity::secrets::Column::Sharing
                                .eq(sharing_to_i16(SharingMode::Private)),
                        )
                        .add(entity::secrets::Column::OwnerId.eq(subject.0)),
                )
                .add(entity::secrets::Column::Sharing.eq(sharing_to_i16(SharingMode::Shared)))
                .add(
                    Condition::all()
                        .add(
                            entity::secrets::Column::Sharing
                                .eq(sharing_to_i16(SharingMode::Tenant)),
                        )
                        .add(entity::secrets::Column::TenantId.eq(req)),
                ),
        )
        .all(&conn)
        .await
        .map_err(scope_err_to_domain)?;

    // Winner: closest tenant in chain; private beats non-private at same level.
    let pos = |t: Uuid| chain.iter().position(|c| *c == t).unwrap_or(usize::MAX);
    let best = rows.into_iter().min_by(|a, b| {
        pos(a.tenant_id).cmp(&pos(b.tenant_id)).then(
            (a.sharing != sharing_to_i16(SharingMode::Private))
                .cmp(&(b.sharing != sharing_to_i16(SharingMode::Private))),
        )
    });
    best.map(entity_to_model).transpose()
}

pub(super) async fn find_own(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    tenant: TenantId,
    subject: OwnerId,
    key: &SecretRef,
) -> Result<Option<SecretRow>, DomainError> {
    let conn = repo.db.conn()?;
    let rows = entity::secrets::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity::secrets::Column::Reference.eq(key.as_ref()))
                .add(entity::secrets::Column::TenantId.eq(tenant.0))
                .add(entity::secrets::Column::Status.eq(SecretStatus::Active.as_smallint()))
                .add(
                    Condition::any()
                        .add(
                            Condition::all()
                                .add(
                                    entity::secrets::Column::Sharing
                                        .eq(sharing_to_i16(SharingMode::Private)),
                                )
                                .add(entity::secrets::Column::OwnerId.eq(subject.0)),
                        )
                        .add(entity::secrets::Column::Sharing.is_in([
                            sharing_to_i16(SharingMode::Tenant),
                            sharing_to_i16(SharingMode::Shared),
                        ])),
                ),
        )
        .all(&conn)
        .await
        .map_err(scope_err_to_domain)?;

    // Prefer the private row if both exist.
    let best = rows
        .into_iter()
        .min_by_key(|r| i32::from(r.sharing != sharing_to_i16(SharingMode::Private)));
    best.map(entity_to_model).transpose()
}

pub(super) async fn find_for_write(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    tenant: TenantId,
    subject: OwnerId,
    key: &SecretRef,
    sharing: SharingMode,
) -> Result<Option<SecretRow>, DomainError> {
    let conn = repo.db.conn()?;
    // Address the row by the target sharing class only — the same identity the
    // partial unique indexes enforce. A private write must NOT match a coexisting
    // tenant/shared row (and vice-versa); they are distinct secrets per design.
    let class = if sharing == SharingMode::Private {
        Condition::all()
            .add(entity::secrets::Column::Sharing.eq(sharing_to_i16(SharingMode::Private)))
            .add(entity::secrets::Column::OwnerId.eq(subject.0))
    } else {
        Condition::all().add(entity::secrets::Column::Sharing.is_in([
            sharing_to_i16(SharingMode::Tenant),
            sharing_to_i16(SharingMode::Shared),
        ]))
    };
    let row = entity::secrets::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity::secrets::Column::Reference.eq(key.as_ref()))
                .add(entity::secrets::Column::TenantId.eq(tenant.0))
                .add(entity::secrets::Column::Status.eq(SecretStatus::Active.as_smallint()))
                .add(class),
        )
        .one(&conn)
        .await
        .map_err(scope_err_to_domain)?;
    row.map(entity_to_model).transpose()
}

#[allow(clippy::cognitive_complexity)]
pub(super) async fn inventory(repo: &SecretRepoImpl) -> Result<SecretCounts, DomainError> {
    let conn = repo.db.conn()?;

    // Aggregate counts grouped by sharing + status.
    let rows: Vec<InventoryRow> = entity::secrets::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .project_all(&conn, |q| {
            q.select_only()
                .column(entity::secrets::Column::Sharing)
                .column(entity::secrets::Column::Status)
                .column_as(entity::secrets::Column::Id.count(), "c")
                .group_by(entity::secrets::Column::Sharing)
                .group_by(entity::secrets::Column::Status)
                .into_model::<InventoryRow>()
        })
        .await
        .map_err(scope_err_to_domain)?;

    let mut counts = SecretCounts::default();
    for r in rows {
        // Decode through the typed enums (not magic numbers) so a future encoding
        // change can't silently miscount; an unknown encoding is logged, not dropped.
        match SecretStatus::from_smallint(r.status) {
            Some(SecretStatus::Provisioning) => counts.provisioning += r.c,
            Some(SecretStatus::Active) => match sharing_from_i16(r.sharing) {
                Some(SharingMode::Private) => counts.private += r.c,
                Some(SharingMode::Tenant) => counts.tenant += r.c,
                Some(SharingMode::Shared) => counts.shared += r.c,
                None => tracing::warn!(
                    sharing = r.sharing,
                    "inventory: unknown sharing encoding, row not counted"
                ),
            },
            None => tracing::warn!(
                status = r.status,
                "inventory: unknown status encoding, row not counted"
            ),
        }
    }

    // Distinct-tenant count for active rows.
    let tenant_rows: Vec<TenantCount> = entity::secrets::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(
            Condition::all()
                .add(entity::secrets::Column::Status.eq(SecretStatus::Active.as_smallint())),
        )
        .project_all(&conn, |q| {
            q.select_only()
                .column_as(
                    Expr::col(entity::secrets::Column::TenantId).count_distinct(),
                    "t",
                )
                .into_model::<TenantCount>()
        })
        .await
        .map_err(scope_err_to_domain)?;

    if let Some(row) = tenant_rows.into_iter().next() {
        counts.tenants = row.t;
    }
    Ok(counts)
}

pub(super) async fn scope_includes_tenant(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    tenant: Uuid,
) -> Result<bool, DomainError> {
    if scope.is_unconstrained() {
        return Ok(true);
    }
    if scope.is_deny_all() {
        return Ok(false);
    }
    // Fast path: direct UUID match via Eq/In filters.
    if scope.contains_uuid(pep_properties::OWNER_TENANT_ID, tenant) {
        return Ok(true);
    }
    // Slow path: InTenantSubtree — query tenant_closure directly.
    // This is independent of credstore_secrets so an empty secrets table
    // does not produce a false-negative.
    let conn = repo.db.conn()?;
    for constraint in scope.constraints() {
        for filter in constraint.filters() {
            if let ScopeFilter::InTenantSubtree(sf) = filter {
                let Some(root) = sf.root_tenant_id().as_uuid() else {
                    continue;
                };
                let mut filter = Condition::all()
                    .add(entity::tenant_closure::Column::AncestorId.eq(root))
                    .add(entity::tenant_closure::Column::DescendantId.eq(tenant));
                if sf.respect_barriers() {
                    filter = filter.add(entity::tenant_closure::Column::Barrier.eq(0_i16));
                }
                let row = entity::tenant_closure::Entity::find()
                    .secure()
                    .scope_with(&AccessScope::allow_all())
                    .filter(filter)
                    .one(&conn)
                    .await
                    .map_err(scope_err_to_domain)?;
                if row.is_some() {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
