DELETE FROM libraries WHERE integration_id IN (SELECT id FROM integrations WHERE integration_type <> 'kavita');
DELETE FROM integrations WHERE integration_type <> 'kavita';

CREATE TABLE integrations_old (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    integration_type TEXT NOT NULL CHECK (integration_type IN ('kavita')),
    url TEXT,
    api_key TEXT
);

INSERT INTO integrations_old (id, name, integration_type, url, api_key)
SELECT id, name, integration_type, url, api_key FROM integrations;

DROP TABLE integrations;
ALTER TABLE integrations_old RENAME TO integrations;
