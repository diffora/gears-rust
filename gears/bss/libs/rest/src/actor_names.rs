//! Actor names through Account Management, for the BSS reads.
//!
//! A read collects the actor ids of its whole page or document and calls
//! [`ActorNames::resolve`] once, or [`ActorNames::fill`] over a response whose
//! types implement [`ActorFields`]. The names come from AM's public user read
//! (`AccountManagementClient::list_users` with an id-set filter), with the
//! caller's own context and tenant: AM decides what this caller may see, and no
//! privileged context is used.
//!
//! - No name is stored or cached. A rename shows on the next read.
//! - The ids are deduplicated and read in chunks of `IdpUserPagination::MAX_TOP`,
//!   at most four chunks at once, inside one 2 s budget for the response.
//! - A read never fails because of AM: a refused, absent, failed or late lookup
//!   becomes an [`ActorName`] that carries no label.
//! - A gear's system actors read [`SYSTEM_LABEL`] and never reach AM.
//!
//! AM is a soft dependency. The client is looked up in the hub at each lookup,
//! so the registration order of the gears cannot turn names off, and a process
//! without AM reads every name as [`ActorName::Unavailable`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use account_management_sdk::{AccountManagementClient, IdpUserFilterField, IdpUserPagination};
use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use toolkit::ClientHub;
use toolkit_canonical_errors::CanonicalError;
use toolkit_odata::Page;
use toolkit_odata::filter::{FilterNode, ODataValue};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The SDK types of the [`ActorDirectory`] boundary, so that a gear's test fake needs no other
/// dependency.
pub use account_management_sdk::{IdpUser, ListUsersQuery};

/// The most AM lookups one response runs at once.
const LOOKUP_CONCURRENCY: usize = 4;
/// The SDK's largest id-set chunk. Each chunk may follow the provider's pagination.
const LOOKUP_BATCH_SIZE: usize = IdpUserPagination::MAX_TOP as usize;
/// One budget for the whole response, queued lookups included.
const LOOKUP_BUDGET: Duration = Duration::from_secs(2);

/// The label of a gear's system actor.
pub const SYSTEM_LABEL: &str = "System";

/// A current name, or the reason that no name can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorName {
    /// The current non-blank display name, full name, or username from the `IdP`.
    Resolved(String),
    /// AM refused this caller access to the profile. No permission is added.
    Restricted,
    /// AM found no user visible to this caller. This does not prove a deletion.
    NotFound,
    /// No AM client, an AM or provider failure, the budget ran out, or the profile is invalid.
    Unavailable,
    /// One of the gear's own system actors. AM is not asked.
    System,
}

impl ActorName {
    /// The text a response shows: the resolved name, or [`SYSTEM_LABEL`] for a system actor.
    /// `None` when no name is available now.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Resolved(name) => Some(name),
            Self::System => Some(SYSTEM_LABEL),
            Self::Restricted | Self::NotFound | Self::Unavailable => None,
        }
    }
}

/// The narrow boundary to AM's public user read. Tests supply a fake.
#[async_trait]
pub trait ActorDirectory: Send + Sync {
    /// Read one filtered page in the caller's tenant, with the caller's rights.
    ///
    /// # Errors
    /// Returns AM's authorization, absence and provider errors unchanged.
    async fn list_users(
        &self,
        ctx: &SecurityContext,
        query: ListUsersQuery,
    ) -> Result<Page<IdpUser>, CanonicalError>;
}

/// The ids an id-set lookup of [`ActorNames::resolve`] asks for, in ascending order. Empty for
/// a query without an id-set filter. A test's [`ActorDirectory`] reads its request with it.
#[must_use]
pub fn queried_ids(query: &ListUsersQuery) -> Vec<Uuid> {
    let Some(FilterNode::InList {
        field: IdpUserFilterField::Id,
        values,
    }) = &query.filter
    else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|value| match value {
            ODataValue::Uuid(id) => Some(*id),
            _ => None,
        })
        .collect()
}

/// A response value that shows actor ids, each with a `*_name` sibling.
///
/// A gear implements it for its response types. The containers of the response
/// (`Vec`, `Option`) are implemented here, so a nested value delegates to them.
pub trait ActorFields {
    /// Push every actor id the value shows.
    fn actor_ids(&self, ids: &mut Vec<Uuid>);
    /// Set every `*_name` sibling from `names`, with [`label`].
    fn fill_names(&mut self, names: &BTreeMap<Uuid, ActorName>);
}

impl<T: ActorFields> ActorFields for Vec<T> {
    fn actor_ids(&self, ids: &mut Vec<Uuid>) {
        for value in self {
            value.actor_ids(ids);
        }
    }
    fn fill_names(&mut self, names: &BTreeMap<Uuid, ActorName>) {
        for value in self {
            value.fill_names(names);
        }
    }
}

impl<T: ActorFields> ActorFields for Option<T> {
    fn actor_ids(&self, ids: &mut Vec<Uuid>) {
        if let Some(value) = self {
            value.actor_ids(ids);
        }
    }
    fn fill_names(&mut self, names: &BTreeMap<Uuid, ActorName>) {
        if let Some(value) = self {
            value.fill_names(names);
        }
    }
}

/// The `*_name` a response shows for `id`: its label among `names`, or `None`.
#[must_use]
pub fn label(names: &BTreeMap<Uuid, ActorName>, id: Uuid) -> Option<String> {
    names.get(&id).and_then(ActorName::label).map(str::to_owned)
}

/// AM through the client hub, looked up at each call.
struct AmDirectory {
    hub: Arc<ClientHub>,
}

#[async_trait]
impl ActorDirectory for AmDirectory {
    async fn list_users(
        &self,
        ctx: &SecurityContext,
        query: ListUsersQuery,
    ) -> Result<Page<IdpUser>, CanonicalError> {
        let am = self
            .hub
            .try_get::<dyn AccountManagementClient>()
            .ok_or_else(|| CanonicalError::service_unavailable().create())?;
        am.list_users(ctx, ctx.subject_tenant_id(), query).await
    }
}

/// Read-side names only. They are not part of a transaction, a pin, an audit row
/// or a stored record.
#[derive(Clone)]
pub struct ActorNames {
    directory: Arc<dyn ActorDirectory>,
    system_ids: BTreeSet<Uuid>,
}

impl ActorNames {
    /// AM through the process client hub. `system_ids` are the gear's own system actors.
    #[must_use]
    pub fn from_hub(hub: Arc<ClientHub>, system_ids: &[Uuid]) -> Self {
        Self::with_directory(Arc::new(AmDirectory { hub }), system_ids)
    }

    /// A given directory, for tests and transport-level fakes.
    #[must_use]
    pub fn with_directory(directory: Arc<dyn ActorDirectory>, system_ids: &[Uuid]) -> Self {
        Self {
            directory,
            system_ids: system_ids.iter().copied().collect(),
        }
    }

    /// The name of every id, in one bounded set of lookups.
    ///
    /// The caller has already authorized and loaded the records the ids come
    /// from. A system id reads [`ActorName::System`] and is not sent to AM. No
    /// queued chunk starts after the shared deadline, and a lookup still running
    /// at the deadline is cancelled. No profile or error detail is logged.
    pub async fn resolve(
        &self,
        ctx: &SecurityContext,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> BTreeMap<Uuid, ActorName> {
        let (system, unique): (BTreeSet<_>, BTreeSet<_>) =
            ids.into_iter().partition(|id| self.system_ids.contains(id));
        let unique: Vec<_> = unique.into_iter().collect();
        let deadline = tokio::time::Instant::now() + LOOKUP_BUDGET;
        let chunks: Vec<_> = unique
            .chunks(LOOKUP_BATCH_SIZE)
            .map(<[Uuid]>::to_vec)
            .collect();
        let mut names: BTreeMap<_, _> = stream::iter(chunks)
            .map(|ids| async move { self.resolve_chunk(ctx, &ids, deadline).await })
            .buffer_unordered(LOOKUP_CONCURRENCY)
            .flat_map(stream::iter)
            .collect()
            .await;
        names.extend(system.into_iter().map(|id| (id, ActorName::System)));
        names
    }

    /// Collect every actor id of `value`, [`Self::resolve`] them once, and set every `*_name`.
    ///
    /// A value without ids makes no lookup. A name that cannot be shown is `None`.
    pub async fn fill<T: ActorFields + ?Sized>(&self, ctx: &SecurityContext, value: &mut T) {
        let mut ids = Vec::new();
        value.actor_ids(&mut ids);
        let names = if ids.is_empty() {
            BTreeMap::new()
        } else {
            self.resolve(ctx, ids).await
        };
        value.fill_names(&names);
    }

    /// Read the whole filtered set before claiming that an id is absent. Every
    /// page must make progress and return only new requested ids. A provider that
    /// drifts or fails in pagination never shows an unrelated profile and never
    /// implies a deletion.
    async fn resolve_chunk(
        &self,
        ctx: &SecurityContext,
        ids: &[Uuid],
        deadline: tokio::time::Instant,
    ) -> BTreeMap<Uuid, ActorName> {
        let mut names = BTreeMap::new();
        let mut remaining: BTreeSet<_> = ids.iter().copied().collect();
        let mut seen_cursors = BTreeSet::new();
        let mut cursor = None;
        let failure = loop {
            if tokio::time::Instant::now() >= deadline {
                break ActorName::Unavailable;
            }
            let Ok(mut query) = ListUsersQuery::with_ids(ids.iter().copied()) else {
                break ActorName::Unavailable;
            };
            let Ok(pagination) = IdpUserPagination::new(query.pagination.top(), cursor) else {
                break ActorName::Unavailable;
            };
            query.pagination = pagination;
            let result =
                tokio::time::timeout_at(deadline, self.directory.list_users(ctx, query)).await;
            let page = match result {
                Ok(Ok(page)) => page,
                Ok(Err(
                    CanonicalError::PermissionDenied { .. }
                    | CanonicalError::Unauthenticated { .. },
                )) => break ActorName::Restricted,
                Ok(Err(CanonicalError::NotFound { .. })) => break ActorName::NotFound,
                _ => break ActorName::Unavailable,
            };
            let returned: BTreeSet<_> = page.items.iter().map(|user| user.id).collect();
            if returned.len() != page.items.len() || !returned.is_subset(&remaining) {
                break ActorName::Unavailable;
            }
            for user in page.items {
                remaining.remove(&user.id);
                names.insert(user.id, project_name(&user));
            }
            let Some(next) = page.page_info.next_cursor else {
                break ActorName::NotFound;
            };
            if remaining.is_empty() {
                break ActorName::NotFound;
            }
            if returned.is_empty() || !seen_cursors.insert(next.clone()) {
                break ActorName::Unavailable;
            }
            cursor = Some(next);
        };
        names.extend(remaining.into_iter().map(|id| (id, failure.clone())));
        names
    }
}

/// The provider's own fields, in order: display name, then first and last name,
/// then username. A value of only whitespace counts as missing.
fn project_name(user: &IdpUser) -> ActorName {
    if let Some(name) = user
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return ActorName::Resolved(name.to_owned());
    }
    let full_name = [user.first_name.as_deref(), user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !full_name.is_empty() {
        return ActorName::Resolved(full_name);
    }
    let username = user.username.trim();
    if username.is_empty() {
        ActorName::Unavailable
    } else {
        ActorName::Resolved(username.to_owned())
    }
}

#[cfg(test)]
#[path = "actor_names_tests.rs"]
mod tests;
