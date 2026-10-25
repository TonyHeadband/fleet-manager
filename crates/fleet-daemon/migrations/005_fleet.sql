-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version.
ALTER TABLE repos ADD COLUMN roster TEXT NOT NULL DEFAULT '[]';
ALTER TABLE repos ADD COLUMN test_command TEXT NULL;
