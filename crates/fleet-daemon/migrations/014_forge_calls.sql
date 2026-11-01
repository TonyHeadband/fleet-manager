-- Store::open runs it once, gated on PRAGMA user_version; IF NOT EXISTS lets it run again safely if it is renumbered.
-- One row per call the daemon makes to a code forge, kept for diagnosing a refused call now and for analytics later.
-- `detail` is the failure's message, never a token.
CREATE TABLE IF NOT EXISTS forge_calls (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    persona TEXT NOT NULL,
    task TEXT,
    agent TEXT,
    forge TEXT NOT NULL,
    repo TEXT NOT NULL,
    op TEXT NOT NULL,
    ok INTEGER NOT NULL,
    detail TEXT,
    ms INTEGER NOT NULL
);
