-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version. The repo's `.fleet/workflow.yml` replaced
-- the per-repo test command.
ALTER TABLE repos DROP COLUMN test_command;
