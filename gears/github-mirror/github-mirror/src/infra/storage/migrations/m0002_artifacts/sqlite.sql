CREATE TABLE IF NOT EXISTS gm_artifacts (
    tenant_id TEXT NOT NULL,
    id INTEGER NOT NULL,
    repo_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    size_in_bytes INTEGER NOT NULL,
    expired INTEGER NOT NULL,
    workflow_run_id INTEGER,
    head_branch TEXT,
    head_sha TEXT,
    created_at TEXT,
    updated_at TEXT,
    expires_at TEXT,
    extracted_at TEXT,
    PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_gm_artifacts_tenant_repo
    ON gm_artifacts (tenant_id, repo_id);

CREATE INDEX IF NOT EXISTS idx_gm_comments_tenant_repo_created
    ON gm_comments (tenant_id, repo_id, created_at);

CREATE INDEX IF NOT EXISTS idx_gm_comments_tenant_repo_updated
    ON gm_comments (tenant_id, repo_id, updated_at);

ALTER TABLE gm_commits ADD COLUMN html_url TEXT;

ALTER TABLE gm_pull_request_commits ADD COLUMN html_url TEXT;

CREATE TABLE IF NOT EXISTS gm_workflows (
    tenant_id TEXT NOT NULL,
    id INTEGER NOT NULL,
    repo_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    state TEXT NOT NULL,
    extracted_at TEXT,
    PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_gm_workflows_tenant_repo
    ON gm_workflows (tenant_id, repo_id);
