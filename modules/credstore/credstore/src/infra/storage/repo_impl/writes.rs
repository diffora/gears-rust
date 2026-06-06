//! Write-path repo methods: `insert_provisioning`, `mark_active`,
//! `touch`, `delete_by_id`, `reap_provisioning`.

use credstore_sdk::SharingMode;
use modkit_db::secure::{SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt};
use modkit_security::AccessScope;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::secret::model::{NewSecret, SecretRow, SecretStatus};
use crate::infra::storage::entity;
use crate::infra::storage::repo_impl::helpers::map_scope_err;
use crate::infra::storage::repo_impl::{SecretRepoImpl, entity_to_model, sharing_to_i16};

pub(super) async fn insert_provisioning(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    new: &NewSecret,
) -> Result<(), DomainError> {
    use sea_orm::ActiveValue;
    let conn = repo.db.conn()?;
    let now = OffsetDateTime::now_utc();
    let am = entity::secrets::ActiveModel {
        id: ActiveValue::Set(new.id),
        tenant_id: ActiveValue::Set(new.tenant_id.0),
        reference: ActiveValue::Set(new.reference.as_ref().to_owned()),
        sharing: ActiveValue::Set(sharing_to_i16(new.sharing)),
        owner_id: ActiveValue::Set(new.owner_id.0),
        status: ActiveValue::Set(SecretStatus::Provisioning.as_smallint()),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
        version: ActiveValue::NotSet,
    };
    // scope_unchecked: INSERT cannot subtree-clamp on a row that doesn't exist yet.
    entity::secrets::Entity::insert(am)
        .secure()
        .scope_unchecked(scope)
        .map_err(map_scope_err)?
        .exec(&conn)
        .await
        .map_err(map_scope_err)?;
    Ok(())
}

pub(super) async fn mark_active(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    id: Uuid,
) -> Result<(), DomainError> {
    let conn = repo.db.conn()?;
    let now = OffsetDateTime::now_utc();
    let rows_affected = entity::secrets::Entity::update_many()
        .col_expr(
            entity::secrets::Column::Status,
            Expr::value(SecretStatus::Active.as_smallint()),
        )
        .col_expr(entity::secrets::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(entity::secrets::Column::Id.eq(id))
                .add(entity::secrets::Column::Status.eq(SecretStatus::Provisioning.as_smallint())),
        )
        .secure()
        .scope_with(scope)
        .exec(&conn)
        .await
        .map_err(map_scope_err)?
        .rows_affected;
    if rows_affected == 0 {
        return Err(DomainError::Conflict);
    }
    Ok(())
}

pub(super) async fn touch(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    id: Uuid,
    sharing: SharingMode,
    expected_version: Option<i64>,
) -> Result<Option<SecretRow>, DomainError> {
    let conn = repo.db.conn()?;
    let now = OffsetDateTime::now_utc();
    // Atomic, id-keyed: version = version + 1, set sharing, stamp updated_at.
    // Keyed by id (the row found by find_for_write) so it needs no sharing-class
    // filter and works for both private (sharing unchanged) and non-private
    // (tenant<->shared) rows. When expected_version is set, the bump is gated on
    // version = expected so a stale optimistic-lock write commits 0 rows.
    let mut filter = Condition::all()
        .add(entity::secrets::Column::Id.eq(id))
        .add(entity::secrets::Column::Status.eq(SecretStatus::Active.as_smallint()));
    if let Some(v) = expected_version {
        filter = filter.add(entity::secrets::Column::Version.eq(v));
    }
    let rows_affected = entity::secrets::Entity::update_many()
        .col_expr(
            entity::secrets::Column::Sharing,
            Expr::value(sharing_to_i16(sharing)),
        )
        .col_expr(entity::secrets::Column::UpdatedAt, Expr::value(now))
        .col_expr(
            entity::secrets::Column::Version,
            Expr::col(entity::secrets::Column::Version).add(1_i64),
        )
        .filter(filter)
        .secure()
        .scope_with(scope)
        .exec(&conn)
        .await
        .map_err(map_scope_err)?
        .rows_affected;
    if rows_affected == 0 {
        return Ok(None);
    }
    // Re-read the updated row by id.
    let row = entity::secrets::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(entity::secrets::Column::Id.eq(id)))
        .one(&conn)
        .await
        .map_err(map_scope_err)?;
    row.map(entity_to_model).transpose()
}

pub(super) async fn delete_by_id(
    repo: &SecretRepoImpl,
    scope: &AccessScope,
    id: Uuid,
    expected_version: Option<i64>,
) -> Result<(), DomainError> {
    let conn = repo.db.conn()?;
    let mut filter = Condition::all().add(entity::secrets::Column::Id.eq(id));
    if let Some(v) = expected_version {
        filter = filter.add(entity::secrets::Column::Version.eq(v));
    }
    let rows_affected = entity::secrets::Entity::delete_many()
        .filter(filter)
        .secure()
        .scope_with(scope)
        .exec(&conn)
        .await
        .map_err(map_scope_err)?
        .rows_affected;
    if rows_affected == 0 {
        return Err(DomainError::NotFound);
    }
    Ok(())
}

pub(super) async fn reap_provisioning(
    repo: &SecretRepoImpl,
    older_than_secs: u64,
) -> Result<u64, DomainError> {
    let conn = repo.db.conn()?;
    // Clamp before the cast: a u64 beyond i64::MAX would wrap to a negative
    // duration and reap rows from the future. i64::MAX seconds is effectively
    // "never reap", the safe saturating bound.
    let secs = i64::try_from(older_than_secs).unwrap_or(i64::MAX);
    let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(secs);
    let rows_affected = entity::secrets::Entity::delete_many()
        .filter(
            Condition::all()
                .add(entity::secrets::Column::Status.eq(SecretStatus::Provisioning.as_smallint()))
                .add(entity::secrets::Column::CreatedAt.lt(cutoff)),
        )
        .secure()
        .scope_with(&AccessScope::allow_all())
        .exec(&conn)
        .await
        .map_err(map_scope_err)?
        .rows_affected;
    Ok(rows_affected)
}
