-- SQLite can't alter a CHECK constraint in place, so the table is rebuilt to
-- accept every integration type and to hold basic-auth credentials (used by
-- OPDS catalogs).
CREATE TABLE integrations_new (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    integration_type TEXT NOT NULL CHECK (integration_type IN ('kavita', 'komga', 'opds', 'suwayomi')),
    url TEXT,
    api_key TEXT,
    username TEXT,
    password TEXT
);

INSERT INTO integrations_new (id, name, integration_type, url, api_key)
SELECT id, name, integration_type, url, api_key FROM integrations;

DROP TABLE integrations;
ALTER TABLE integrations_new RENAME TO integrations;
