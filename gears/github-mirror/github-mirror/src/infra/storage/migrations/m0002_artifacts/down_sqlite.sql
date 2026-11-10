DROP TABLE IF EXISTS gm_workflows;

ALTER TABLE gm_commits DROP COLUMN html_url;

ALTER TABLE gm_pull_request_commits DROP COLUMN html_url;

DROP INDEX IF EXISTS idx_gm_comments_tenant_repo_created;

DROP INDEX IF EXISTS idx_gm_comments_tenant_repo_updated;

DROP TABLE IF EXISTS gm_artifacts;
