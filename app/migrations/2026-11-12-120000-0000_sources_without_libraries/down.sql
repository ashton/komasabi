CREATE TABLE jobs (
    id TEXT NOT NULL PRIMARY KEY,
    job_type TEXT NOT NULL CHECK (job_type IN ('sync_integration_libraries')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    integration_id TEXT,
    error TEXT,
    created_at TIMESTAMP NOT NULL,
    started_at TIMESTAMP,
    finished_at TIMESTAMP
);

CREATE TABLE integrations_old (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    integration_type TEXT NOT NULL CHECK (integration_type IN ('kavita', 'komga', 'opds', 'suwayomi')),
    url TEXT,
    api_key TEXT,
    username TEXT,
    password TEXT
);

INSERT INTO integrations_old (id, name, integration_type, url, api_key, username, password)
SELECT id, name, integration_type, url, api_key, username, password FROM integrations
WHERE integration_type <> 'local_folder';

-- Local folders become local libraries again (their id doubles as the
-- external id, as before).
INSERT INTO libraries (id, name, external_id, folder)
SELECT id, name, id, folder FROM integrations WHERE integration_type = 'local_folder';

DROP TABLE integrations;
ALTER TABLE integrations_old RENAME TO integrations;
