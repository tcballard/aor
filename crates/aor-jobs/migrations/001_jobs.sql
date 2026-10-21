CREATE TABLE aor_jobs (
 id TEXT NOT NULL PRIMARY KEY,
 job_type TEXT NOT NULL,
 payload_version BIGINT NOT NULL,
 payload TEXT NOT NULL,
 dedup_key TEXT NOT NULL,
 queue TEXT NOT NULL,
 state TEXT NOT NULL,
 attempt BIGINT NOT NULL,
 max_attempts BIGINT NOT NULL,
 run_at BIGINT NOT NULL,
 lease_expires_at BIGINT NOT NULL,
 lease_generation BIGINT NOT NULL,
 created_at BIGINT NOT NULL,
 updated_at BIGINT NOT NULL,
 last_error TEXT NOT NULL,
 UNIQUE (job_type, dedup_key)
);
CREATE INDEX aor_jobs_due ON aor_jobs(queue, state, run_at);
