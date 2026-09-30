CREATE TABLE editions (
    id TEXT NOT NULL PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    published_at TEXT NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    CONSTRAINT positive_version CHECK (version > 0)
);
CREATE INDEX editions_published ON editions (published_at);
