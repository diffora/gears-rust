#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use github_mirror::domain::error::DomainError;
use github_mirror::domain::ports::github::{
    ActionsListing, CommitDetail, CommitListing, FetchOptions, FetchedRepository, ForceMode,
    GithubPort, IssueDetail, IssueDetailWants, IssueListing, ListCursor, MetadataListing,
    PullDetail, PullListing, RepoRef,
};
use github_mirror::domain::repo::{
    CommentRecord, CommitRecord, IssueRecord, ListingFilter, PageWindow, PullRequestRecord,
    RepoRecord, RepoRunStatus, SessionStatus, WorkflowJobRecord,
};
use tokio_util::sync::CancellationToken;
use toolkit_odata::ODataQuery;
use toolkit_security::{AccessScope, SecurityContext};
use uuid::Uuid;

const OWNER: &str = "rust-lang";
const NAME: &str = "rust";
const REPO_ID: i64 = 42;

const ISSUES: i64 = 300;
const COMMENTS: i64 = 200;
const PULL_REQUESTS: i64 = 150;
const COMMITS: i64 = 100;

#[derive(Default)]
struct RefineLog(Mutex<BTreeMap<String, usize>>);

impl RefineLog {
    fn add(&self, key: String) {
        *self.0.lock().unwrap().entry(key).or_default() += 1;
    }

    fn calls(&self) -> BTreeMap<String, usize> {
        self.0.lock().unwrap().clone()
    }
}

struct Recording {
    inner: common::FakeGithub,
    log: Arc<RefineLog>,
    stop: Option<(CancellationToken, usize)>,
    calls: AtomicUsize,
}

impl Recording {
    fn new(
        fixture: &FetchedRepository,
        log: Arc<RefineLog>,
        stop: Option<(CancellationToken, usize)>,
    ) -> Self {
        Self {
            inner: common::FakeGithub {
                result: Some(fixture.clone()),
            },
            log,
            stop,
            calls: AtomicUsize::new(0),
        }
    }

    fn record(&self, key: String) {
        self.log.add(key);
        let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some((cancel, _)) = self.stop.as_ref().filter(|(_, after)| *after == calls) {
            cancel.cancel();
        }
    }
}

#[async_trait]
impl GithubPort for Recording {
    async fn fetch_repository_metadata(
        &self,
        owner: &str,
        name: &str,
        options: &FetchOptions,
    ) -> Result<RepoRecord, DomainError> {
        self.inner
            .fetch_repository_metadata(owner, name, options)
            .await
    }

    async fn list_issues(
        &self,
        repo: RepoRef<'_>,
        cursor: ListCursor<'_>,
        options: &FetchOptions,
    ) -> Result<IssueListing, DomainError> {
        self.inner.list_issues(repo, cursor, options).await
    }

    async fn refine_issue(
        &self,
        repo: RepoRef<'_>,
        number: i64,
        wants: IssueDetailWants,
        options: &FetchOptions,
    ) -> Result<IssueDetail, DomainError> {
        self.record(format!("issue {number}"));
        self.inner.refine_issue(repo, number, wants, options).await
    }

    async fn list_pull_requests(
        &self,
        repo: RepoRef<'_>,
        cursor: ListCursor<'_>,
        options: &FetchOptions,
    ) -> Result<PullListing, DomainError> {
        self.inner.list_pull_requests(repo, cursor, options).await
    }

    async fn refine_pull_request(
        &self,
        repo: RepoRef<'_>,
        number: i64,
        options: &FetchOptions,
    ) -> Result<PullDetail, DomainError> {
        self.record(format!("pull {number}"));
        self.inner.refine_pull_request(repo, number, options).await
    }

    async fn list_commits(
        &self,
        repo: RepoRef<'_>,
        cursor: ListCursor<'_>,
        options: &FetchOptions,
    ) -> Result<CommitListing, DomainError> {
        self.inner.list_commits(repo, cursor, options).await
    }

    async fn refine_commit(
        &self,
        repo: RepoRef<'_>,
        sha: &str,
        with_ci: bool,
        options: &FetchOptions,
    ) -> Result<CommitDetail, DomainError> {
        self.record(format!("commit {sha}"));
        self.inner.refine_commit(repo, sha, with_ci, options).await
    }

    async fn list_metadata(
        &self,
        repo: RepoRef<'_>,
        options: &FetchOptions,
    ) -> Result<MetadataListing, DomainError> {
        self.inner.list_metadata(repo, options).await
    }

    async fn list_actions(
        &self,
        repo: RepoRef<'_>,
        options: &FetchOptions,
    ) -> Result<ActionsListing, DomainError> {
        self.inner.list_actions(repo, options).await
    }

    async fn refine_workflow_run(
        &self,
        repo: RepoRef<'_>,
        run_id: i64,
        options: &FetchOptions,
    ) -> Result<Vec<WorkflowJobRecord>, DomainError> {
        self.inner.refine_workflow_run(repo, run_id, options).await
    }

    async fn clear_cache(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        owner: &str,
        name: Option<&str>,
        repo_ids: &[i64],
    ) -> Result<u64, DomainError> {
        self.inner
            .clear_cache(scope, tenant_id, owner, name, repo_ids)
            .await
    }

    async fn expire_cache(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        fetched_before: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, DomainError> {
        self.inner
            .expire_cache(scope, tenant_id, fetched_before)
            .await
    }

    async fn cache_size(
        &self,
        scope: &AccessScope,
        tenant_id: uuid::Uuid,
        owner: &str,
        name: &str,
        repo_ids: &[i64],
    ) -> Result<u64, DomainError> {
        self.inner
            .cache_size(scope, tenant_id, owner, name, repo_ids)
            .await
    }

    async fn rate_limit(
        &self,
    ) -> Result<Vec<github_mirror::domain::ports::github::RateLimitQuota>, DomainError> {
        self.inner.rate_limit().await
    }
}

fn stamp(i: i64) -> String {
    let base = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    (base + chrono::Duration::seconds(i)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn generated_repository() -> FetchedRepository {
    let mut repo = common::fetched_repository();
    repo.issues = (0..ISSUES)
        .map(|i| IssueRecord {
            author_login: Some(format!("user{}", i % 50)),
            author_json: None,
            assignees_json: None,
            labels_json: None,
            comments_count: None,
            locked: None,
            node_id: None,
            id: 1_000_000 + i,
            repo_id: REPO_ID,
            number: 1 + i,
            title: format!("issue {i}"),
            body: None,
            state: if i % 3 == 0 { "closed" } else { "open" }.to_owned(),
            is_pull_request: false,
            created_at: stamp(i),
            updated_at: stamp(i),
            closed_at: None,
            html_url: None,
        })
        .collect();
    repo.comments = (0..COMMENTS)
        .map(|i| CommentRecord {
            id: 2_000_000 + i,
            repo_id: REPO_ID,
            issue_number: 1 + i % ISSUES,
            author_login: Some(format!("user{}", i % 50)),
            body: Some(format!("comment {i}")),
            created_at: stamp(i),
            updated_at: stamp(i),
            html_url: None,
        })
        .collect();
    repo.pull_requests = (0..PULL_REQUESTS)
        .map(|i| PullRequestRecord {
            author_login: Some(format!("user{}", i % 50)),
            author_json: None,
            assignees_json: None,
            labels_json: None,
            comments_count: None,
            locked: None,
            requested_reviewers_json: None,
            node_id: None,
            id: 3_000_000 + i,
            repo_id: REPO_ID,
            number: ISSUES + 1 + i,
            title: format!("pr {i}"),
            body: None,
            state: if i % 2 == 0 { "closed" } else { "open" }.to_owned(),
            draft: false,
            merged: i % 2 == 0,
            head_sha: Some(format!("h{i}")),
            base_sha: Some("b1".to_owned()),
            lines_added: 1,
            lines_removed: 1,
            created_at: stamp(i),
            updated_at: stamp(i),
            closed_at: None,
            merged_at: None,
            html_url: None,
            head_ref: Some(format!("feature-{i}")),
            base_ref: Some("master".to_owned()),
        })
        .collect();
    repo.commits = (0..COMMITS)
        .map(|i| CommitRecord {
            repo_id: REPO_ID,
            sha: format!("{i:040x}"),
            message: format!("commit {i}"),
            author_login: Some(format!("user{}", i % 50)),
            committer_login: None,
            authored_at: Some(stamp(i)),
            committed_at: Some(stamp(i)),
            additions: 1,
            deletions: 0,
            html_url: None,
        })
        .collect();
    repo.review_comments.clear();
    repo.reviews.clear();
    repo.pull_request_files.clear();
    repo.review_threads.clear();
    repo.pull_request_commits.clear();
    repo.commit_files.clear();
    repo.commit_statuses.clear();
    repo.check_runs.clear();
    repo.commit_comments.clear();
    repo.issue_reactions.clear();
    repo.issue_timeline.clear();
    repo.issue_events.clear();
    repo
}

#[derive(Debug, PartialEq, Eq)]
struct Mirrored {
    issues: Vec<i64>,
    pull_requests: Vec<i64>,
    commits: Vec<String>,
    comments: Vec<i64>,
}

async fn mirrored(service: &common::ConcreteService, ctx: &SecurityContext) -> Mirrored {
    let window = PageWindow::first(1_000);
    let filter = ListingFilter::default();
    let mut issues: Vec<i64> = service
        .list_issues(ctx, OWNER, NAME, window, filter)
        .await
        .expect("issues must list")
        .0
        .items
        .into_iter()
        .map(|issue| issue.id)
        .collect();
    let mut pull_requests: Vec<i64> = service
        .list_pull_requests(ctx, OWNER, NAME, window, filter)
        .await
        .expect("pull requests must list")
        .0
        .items
        .into_iter()
        .map(|pull| pull.id)
        .collect();
    let mut commits: Vec<String> = service
        .list_commits(ctx, OWNER, NAME, window, None)
        .await
        .expect("commits must list")
        .0
        .items
        .into_iter()
        .map(|commit| commit.sha)
        .collect();
    let mut comments: Vec<i64> = service
        .list_repo_comments(ctx, OWNER, NAME, window, filter)
        .await
        .expect("comments must list")
        .items
        .into_iter()
        .map(|comment| comment.id)
        .collect();
    issues.sort_unstable();
    pull_requests.sort_unstable();
    commits.sort_unstable();
    comments.sort_unstable();
    Mirrored {
        issues,
        pull_requests,
        commits,
        comments,
    }
}

async fn run_status(service: &common::ConcreteService, ctx: &SecurityContext) -> RepoRunStatus {
    service
        .list_repo_sync_status(ctx, &ODataQuery::default(), None)
        .await
        .expect("run statuses must list")
        .items[0]
        .status
}

async fn session_status(
    service: &common::ConcreteService,
    ctx: &SecurityContext,
    id: Uuid,
) -> SessionStatus {
    service
        .get_session(ctx, id)
        .await
        .expect("the session must be found")
        .status
}

#[tokio::test]
async fn repeated_interruptions_resume_to_the_clean_state_refining_each_entity_once() {
    let fixture = generated_repository();
    let ctx = common::caller_in(Uuid::new_v4());

    let clean_log = Arc::new(RefineLog::default());
    let clean = common::service_with_github(
        common::inmem_db().await,
        "https://api.github.com",
        Arc::new(Recording::new(&fixture, Arc::clone(&clean_log), None)),
    );
    let mut clean_pump = common::SyncPump::take(&clean).await;
    let queued = clean
        .enqueue_sync(&ctx, OWNER, NAME, None, ForceMode::None, None)
        .await
        .expect("the clean sync must queue");
    assert_eq!(clean_pump.drain(&clean).await, 1);
    let clean_session = clean
        .get_session(&ctx, queued.session_id)
        .await
        .expect("the clean session must be found");
    assert_eq!(
        clean_session.status,
        SessionStatus::Complete,
        "the clean run must finish: error {:?}, summary {:?}",
        clean_session.error,
        clean_session.summary_json
    );
    let expected = mirrored(&clean, &ctx).await;
    assert_eq!(expected.issues.len(), fixture.issues.len());
    assert_eq!(expected.pull_requests.len(), fixture.pull_requests.len());
    assert_eq!(expected.commits.len(), fixture.commits.len());
    assert_eq!(expected.comments.len(), fixture.comments.len());
    assert_eq!(run_status(&clean, &ctx).await, RepoRunStatus::Complete);
    let clean_refinements = clean_log.calls();

    let log = Arc::new(RefineLog::default());
    let db = common::inmem_db().await;
    for (round, after) in [1usize, 50, 100, 150].into_iter().enumerate() {
        let stopped = CancellationToken::new();
        let service = common::service_with_github(
            db.clone(),
            "https://api.github.com",
            Arc::new(Recording::new(
                &fixture,
                Arc::clone(&log),
                Some((stopped.clone(), after)),
            )),
        );
        let mut pump = common::SyncPump::take(&service).await;
        let session_id = if round == 0 {
            service
                .enqueue_sync(&ctx, OWNER, NAME, None, ForceMode::None, None)
                .await
                .expect("the first interrupted sync must queue")
                .session_id
        } else {
            let resumed = service
                .resume_incomplete_syncs(&ctx, None, ForceMode::None)
                .await
                .expect("resume must queue the repository again");
            assert!(resumed.refused.is_empty(), "round {round}");
            assert_eq!(resumed.session_ids.len(), 1, "round {round}");
            resumed.session_ids[0]
        };
        assert_eq!(pump.drain_under(&service, &stopped).await, 1);

        assert_eq!(
            session_status(&service, &ctx, session_id).await,
            SessionStatus::Interrupted,
            "round {round}: a run cut off after {after} refinements must report it"
        );
        assert_eq!(
            run_status(&service, &ctx).await,
            RepoRunStatus::InProgress,
            "round {round}: the repository stays in progress for resume"
        );
    }

    let service = common::service_with_github(
        db,
        "https://api.github.com",
        Arc::new(Recording::new(&fixture, Arc::clone(&log), None)),
    );
    let mut pump = common::SyncPump::take(&service).await;
    let resumed = service
        .resume_incomplete_syncs(&ctx, None, ForceMode::None)
        .await
        .expect("the final resume must queue");
    assert_eq!(resumed.session_ids.len(), 1);
    assert_eq!(pump.drain(&service).await, 1);
    assert_eq!(
        session_status(&service, &ctx, resumed.session_ids[0]).await,
        SessionStatus::Complete
    );

    assert_eq!(
        mirrored(&service, &ctx).await,
        expected,
        "four interruptions and resumes must end in the state one clean run reaches"
    );
    let refinements = log.calls();
    let refined_again: Vec<&String> = refinements
        .iter()
        .filter(|(key, calls)| **calls > clean_refinements.get(*key).copied().unwrap_or(0))
        .map(|(key, _)| key)
        .collect();
    assert!(
        refined_again.is_empty(),
        "a resume must continue where the last run stopped, not refine these again: \
         {refined_again:?}"
    );
    assert_eq!(
        refinements, clean_refinements,
        "the interrupted runs together must refine exactly what one clean run refines"
    );
    assert_eq!(run_status(&service, &ctx).await, RepoRunStatus::Complete);
    let sessions = service
        .list_sessions(&ctx, &ODataQuery::default())
        .await
        .expect("sessions must list");
    assert_eq!(
        sessions.items.len(),
        5,
        "four interrupted sessions and the one that finished"
    );
}
