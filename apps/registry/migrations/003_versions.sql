CREATE TABLE versions (
 id TEXT NOT NULL PRIMARY KEY,
 owner_id TEXT NOT NULL REFERENCES aor_users(id),
 plugin_id TEXT NOT NULL REFERENCES plugins(id),
 name TEXT NOT NULL,
 version BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX versions_owner ON versions(owner_id);
