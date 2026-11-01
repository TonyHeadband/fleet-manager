-- Not idempotent: Store::open runs it once, gated on PRAGMA user_version. Every Claude Code session an agent has had
-- (/clear and /resume start new ones), so the chat can follow all their transcripts; `agents.claude_session` stays the
-- latest one, which resume uses.
CREATE TABLE claude_sessions (
    agent_id TEXT NOT NULL,
    session TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, session)
);
INSERT INTO claude_sessions (agent_id, session, at)
    SELECT id, claude_session, 0 FROM agents WHERE claude_session IS NOT NULL;
