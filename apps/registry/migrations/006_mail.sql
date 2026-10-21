CREATE TABLE aor_mail (
 id TEXT NOT NULL PRIMARY KEY,
 sender TEXT NOT NULL,
 recipient TEXT NOT NULL,
 purpose TEXT NOT NULL,
 token_digest TEXT NOT NULL,
 expires_at BIGINT NOT NULL,
 created_at BIGINT NOT NULL,
 first_attempt_at BIGINT NOT NULL,
 state TEXT NOT NULL
);
