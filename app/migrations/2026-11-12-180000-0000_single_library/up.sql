-- The user's Library is now a single one, holding the volumes/chapters the
-- user added from a source. Adding an item only records it here: it's still
-- read (and its covers fetched) through its source. The old `libraries` table
-- (several user libraries, never filled) goes.
DROP TABLE IF EXISTS libraries;

CREATE TABLE library_items (
    id TEXT NOT NULL PRIMARY KEY,
    -- The source the item was added from, and the item's id there.
    integration_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    -- 'volume' or 'chapter'.
    kind TEXT NOT NULL CHECK (kind IN ('volume', 'chapter')),
    name TEXT NOT NULL,
    series_external_id TEXT NOT NULL,
    series_name TEXT NOT NULL,
    -- How many pages the item has, once known, and how many have been read.
    pages INTEGER,
    pages_read INTEGER NOT NULL DEFAULT 0,
    added_at TEXT NOT NULL,
    UNIQUE (integration_id, kind, external_id)
);
