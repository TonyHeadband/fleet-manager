-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version. Orchestrators see every specialist that
-- loads, less the ones their product hides; a repo no longer carries a roster.
ALTER TABLE products ADD COLUMN hidden_specialists TEXT NOT NULL DEFAULT '[]';
ALTER TABLE repos DROP COLUMN roster;
