-- A source's collections (libraries, reading lists, categories, …) are no
-- longer synced into the libraries table: each source lists its own live. A
-- local folder becomes a source of its own (`local_folder`, its path in
-- `folder`), so the integrations table is rebuilt to accept that type and
-- every local library moves into it. The libraries table stays, for the
-- user's own Library. The jobs table, which only recorded library syncs, goes.
CREATE TABLE integrations_new (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    integration_type TEXT NOT NULL CHECK (integration_type IN ('kavita', 'komga', 'local_folder', 'opds', 'suwayomi')),
    url TEXT,
    api_key TEXT,
    username TEXT,
    password TEXT,
    folder TEXT
);

INSERT INTO integrations_new (id, name, integration_type, url, api_key, username, password)
SELECT id, name, integration_type, url, api_key, username, password FROM integrations;

INSERT INTO integrations_new (id, name, integration_type, folder)
SELECT id, name, 'local_folder', folder FROM libraries
WHERE integration_id IS NULL AND folder IS NOT NULL;

DELETE FROM libraries WHERE integration_id IS NULL AND folder IS NOT NULL;

-- `libraries.integration_id` references the table by name, so it points at
-- the rebuilt one once it's renamed.
DROP TABLE integrations;
ALTER TABLE integrations_new RENAME TO integrations;

DROP TABLE jobs;
