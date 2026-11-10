DROP TABLE IF EXISTS gm_workflows;

ALTER TABLE gm_commits DROP COLUMN html_url;

ALTER TABLE gm_pull_request_commits DROP COLUMN html_url;

DROP INDEX idx_gm_comments_tenant_repo_created ON gm_comments;

DROP INDEX idx_gm_comments_tenant_repo_updated ON gm_comments;

DROP TABLE IF EXISTS gm_artifacts;
