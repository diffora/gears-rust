CREATE TABLE IF NOT EXISTS gm_artifacts (
    tenant_id UUID NOT NULL,
    id BIGINT NOT NULL,
    repo_id BIGINT NOT NULL,
    name VARCHAR(1024) NOT NULL,
    size_in_bytes BIGINT NOT NULL,
    expired BOOLEAN NOT NULL,
    workflow_run_id BIGINT,
    head_branch VARCHAR(512),
    head_sha VARCHAR(64),
    created_at VARCHAR(64),
    updated_at VARCHAR(64),
    expires_at VARCHAR(64),
    extracted_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_gm_artifacts_tenant_repo
    ON gm_artifacts (tenant_id, repo_id);

CREATE INDEX IF NOT EXISTS idx_gm_comments_tenant_repo_created
    ON gm_comments (tenant_id, repo_id, created_at);

CREATE INDEX IF NOT EXISTS idx_gm_comments_tenant_repo_updated
    ON gm_comments (tenant_id, repo_id, updated_at);

ALTER TABLE gm_commits ADD COLUMN IF NOT EXISTS html_url VARCHAR(1024);

ALTER TABLE gm_pull_request_commits ADD COLUMN IF NOT EXISTS html_url VARCHAR(1024);

CREATE TABLE IF NOT EXISTS gm_workflows (
    tenant_id UUID NOT NULL,
    id BIGINT NOT NULL,
    repo_id BIGINT NOT NULL,
    name VARCHAR(1024) NOT NULL,
    path VARCHAR(1024) NOT NULL,
    state VARCHAR(64) NOT NULL,
    extracted_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_gm_workflows_tenant_repo
    ON gm_workflows (tenant_id, repo_id);
