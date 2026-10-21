CREATE TABLE manifest_uploads (
 version_id TEXT NOT NULL PRIMARY KEY REFERENCES versions(id),
 revision BIGINT NOT NULL,
 manifest TEXT NOT NULL,
 status TEXT NOT NULL,
 findings TEXT NOT NULL,
 submitted_at BIGINT NOT NULL
);
CREATE TABLE publications (
 version_id TEXT NOT NULL PRIMARY KEY REFERENCES versions(id),
 revision BIGINT NOT NULL,
 published_at BIGINT NOT NULL
);
