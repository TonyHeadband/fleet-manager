-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version.
CREATE TABLE personas (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    host         TEXT NOT NULL,
    commit_name  TEXT NOT NULL,
    commit_email TEXT NOT NULL,
    github_auth  TEXT NOT NULL,
    ready        INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

-- Repo names are unique per persona, not globally; existing rows belong to no persona until one adopts them.
-- A URL may be remembered by several personas (each clones it with its own login); a path belongs to one only.
CREATE TABLE repos_new (
    persona  TEXT NULL,
    name     TEXT NOT NULL,
    source   TEXT NOT NULL,
    location TEXT NOT NULL,
    base     TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    UNIQUE (persona, name)
);
INSERT INTO repos_new (persona, name, source, location, base, added_at)
    SELECT NULL, name, source, location, base, added_at FROM repos;
DROP TABLE repos;
ALTER TABLE repos_new RENAME TO repos;
-- SQLite treats NULLs as distinct in UNIQUE (persona, name); this keeps names unique among unadopted repos too.
CREATE UNIQUE INDEX repos_name_per_persona ON repos (COALESCE(persona, ''), name);
CREATE UNIQUE INDEX repos_location_per_persona ON repos (COALESCE(persona, ''), location);
CREATE UNIQUE INDEX repos_path_location ON repos (location) WHERE source = 'path';
