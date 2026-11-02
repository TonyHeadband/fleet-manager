-- Store::open runs it once, gated on PRAGMA user_version; IF NOT EXISTS lets it run again safely if it is renumbered.
-- What the daemon does when the owner answers an ask yes: the ask's id and the typed action as JSON.
CREATE TABLE IF NOT EXISTS pending_actions (
    ask INTEGER PRIMARY KEY,
    action TEXT NOT NULL
);
