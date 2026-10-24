CREATE TABLE IF NOT EXISTS repos (
    name     TEXT PRIMARY KEY,
    source   TEXT NOT NULL,
    location TEXT NOT NULL UNIQUE,
    base     TEXT NOT NULL,
    added_at INTEGER NOT NULL
);
