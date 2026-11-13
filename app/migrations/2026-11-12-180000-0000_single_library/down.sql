DROP TABLE library_items;

CREATE TABLE libraries (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    external_id TEXT,
    folder TEXT,
    cover TEXT,
    kind TEXT NOT NULL DEFAULT 'comic' CHECK (kind IN ('manga', 'comic', 'ebook')),
    integration_id TEXT REFERENCES integrations(id)
);
