-- Store::open runs it once, gated on PRAGMA user_version; IF NOT EXISTS lets it run again safely if it is renumbered.
-- A persona's supervisor conversations (each is an agent of kind `supervisor`) and the task events routed to them.
CREATE TABLE IF NOT EXISTS supervisor_conversations (
    id TEXT PRIMARY KEY,
    persona TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS supervisor_conversations_persona ON supervisor_conversations (persona, last_used_at);
-- `conversation` is NULL when the persona had none yet: the event is kept and toasts, but is in no chat.
CREATE TABLE IF NOT EXISTS supervisor_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation TEXT,
    persona TEXT NOT NULL,
    task TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS supervisor_events_conversation ON supervisor_events (conversation, id);
