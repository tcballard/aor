CREATE TABLE aor_users (
 id TEXT NOT NULL PRIMARY KEY,
 email TEXT NOT NULL UNIQUE,
 password_hash TEXT NOT NULL,
 verified BOOLEAN NOT NULL,
 epoch BIGINT NOT NULL DEFAULT 1
);
CREATE TABLE aor_sessions (
 token_hash TEXT NOT NULL PRIMARY KEY,
 user_id TEXT NOT NULL REFERENCES aor_users(id),
 csrf_hash TEXT NOT NULL,
 created_at BIGINT NOT NULL,
 seen_at BIGINT NOT NULL,
 expires_at BIGINT NOT NULL,
 epoch BIGINT NOT NULL
);
CREATE INDEX aor_sessions_user ON aor_sessions (user_id);
CREATE TABLE aor_api_tokens (
 token_hash TEXT NOT NULL PRIMARY KEY,
 user_id TEXT NOT NULL REFERENCES aor_users(id),
 scopes TEXT NOT NULL,
 expires_at BIGINT NOT NULL,
 epoch BIGINT NOT NULL
);
CREATE INDEX aor_api_user ON aor_api_tokens (user_id);
CREATE TABLE aor_one_time (
 token_hash TEXT NOT NULL PRIMARY KEY,
 user_id TEXT NOT NULL REFERENCES aor_users(id),
 purpose TEXT NOT NULL,
 expires_at BIGINT NOT NULL
);
