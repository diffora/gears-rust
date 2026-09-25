//! Secure persistence and typed contention failures.
pub mod entity;
pub mod migrations;
pub mod repo;
use crate::infra::error_mapping::DomainError;
use sea_orm::DbErr;
#[derive(Debug, Clone, thiserror::Error)]
pub enum RepoError {
    #[error("pricing storage: {0}")]
    Db(String),
    #[error("pricing storage {context}: {source}")]
    Driver {
        context: String,
        #[source]
        source: DbErr,
    },
    #[error("corrupt pricing row: {0}")]
    CorruptRow(String),
    #[error("{code}")]
    Conflict { code: &'static str },
}
impl RepoError {
    /// Keep database error variants intact for contention classification.
    #[must_use]
    pub fn to_db_err(&self) -> DbErr {
        match self {
            Self::Driver { source, .. } => source.clone(),
            _ => DbErr::Custom(self.to_string()),
        }
    }
}
impl From<toolkit_db::DbError> for RepoError {
    fn from(error: toolkit_db::DbError) -> Self {
        match error {
            toolkit_db::DbError::Sea(source) => Self::Driver {
                context: "transaction".into(),
                source,
            },
            other => Self::Db(other.to_string()),
        }
    }
}
/// Transport mapping; authoring doors add their conflict vocabulary in the next task.
#[must_use]
pub fn repo_failure(error: &RepoError) -> DomainError {
    DomainError::Internal(error.to_string())
}
