use credstore_sdk::{OwnerId, SecretRef, SharingMode, TenantId};
use modkit_macros::domain_model;
use uuid::Uuid;

#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStatus {
    Provisioning,
    Active,
}
impl SecretStatus {
    #[must_use]
    pub fn as_smallint(self) -> i16 {
        match self {
            Self::Provisioning => 1,
            Self::Active => 2,
        }
    }
    #[must_use]
    pub fn from_smallint(v: i16) -> Option<Self> {
        match v {
            1 => Some(Self::Provisioning),
            2 => Some(Self::Active),
            _ => None,
        }
    }
}

#[domain_model]
#[derive(Debug, Clone)]
pub struct SecretRow {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub reference: String,
    pub sharing: SharingMode,
    pub owner_id: OwnerId,
    pub status: SecretStatus,
    /// Monotonic version (optimistic-locking groundwork); 1 on create.
    pub version: i64,
}

/// Optimistic-concurrency precondition for a write, parsed from `If-Match`.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePrecondition {
    /// `If-Match: *` — the target secret must already exist.
    Exists,
    /// `If-Match: "<version>"` — the current version must equal this value.
    Version(i64),
}

#[domain_model]
#[derive(Debug, Clone)]
pub struct NewSecret {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub reference: SecretRef,
    pub sharing: SharingMode,
    pub owner_id: OwnerId,
}

#[cfg(test)]
mod tests {
    use super::SecretStatus;

    #[test]
    fn secret_status_smallint_round_trips() {
        for s in [SecretStatus::Provisioning, SecretStatus::Active] {
            assert_eq!(SecretStatus::from_smallint(s.as_smallint()), Some(s));
        }
        assert_eq!(SecretStatus::Provisioning.as_smallint(), 1);
        assert_eq!(SecretStatus::Active.as_smallint(), 2);
    }

    #[test]
    fn secret_status_from_smallint_rejects_out_of_domain() {
        assert_eq!(SecretStatus::from_smallint(0), None);
        assert_eq!(SecretStatus::from_smallint(3), None);
        assert_eq!(SecretStatus::from_smallint(-1), None);
    }
}
