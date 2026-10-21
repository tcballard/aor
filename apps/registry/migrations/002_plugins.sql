CREATE TABLE plugins (
 id TEXT NOT NULL PRIMARY KEY,
 owner_id TEXT NOT NULL REFERENCES aor_users(id),
 name TEXT NOT NULL,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX plugins_owner ON plugins(owner_id);
