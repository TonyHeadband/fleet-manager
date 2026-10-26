-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version. The fresh start of products and releases:
-- everything but the personas is dropped. Agents and events are made again as 001 makes them, so a new database
-- (001 to 007) and an upgraded one end with one schema.
DROP TABLE agents;
DROP TABLE events;
DROP TABLE projects;
DROP TABLE repos;

CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    token TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    data TEXT NOT NULL
);
CREATE TABLE tasks (
    id   TEXT PRIMARY KEY,
    data TEXT NOT NULL
);
CREATE TABLE products (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    default_persona TEXT NULL,
    created_at      INTEGER NOT NULL
);
CREATE TABLE releases (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    product     TEXT NOT NULL,
    version     TEXT NOT NULL,
    start       TEXT NOT NULL,
    target      TEXT NOT NULL,
    released_at INTEGER NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE (product, version)
);
-- A repo has a local path, a clone URL, or both. Its name is unique within its product.
CREATE TABLE repos (
    product  TEXT NOT NULL,
    name     TEXT NOT NULL,
    path     TEXT NULL,
    url      TEXT NULL,
    roster   TEXT NOT NULL DEFAULT '[]',
    added_at INTEGER NOT NULL,
    PRIMARY KEY (product, name),
    CHECK (path IS NOT NULL OR url IS NOT NULL)
);
