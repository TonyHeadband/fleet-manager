-- Store::open runs it once, gated on PRAGMA user_version; IF NOT EXISTS lets it run again safely if it is renumbered.
-- Rooms where the fleet talks: one per product, and one per supervisor conversation while it has tasks running.
CREATE TABLE IF NOT EXISTS rooms (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    product TEXT,
    conversation TEXT,
    persona TEXT,
    created_at INTEGER NOT NULL,
    archived_at INTEGER
);
CREATE INDEX IF NOT EXISTS rooms_conversation ON rooms (conversation);
-- `member` is `owner`, `daemon` or an agent id; `read_upto` is the id of the last post the member has read.
CREATE TABLE IF NOT EXISTS room_members (
    room TEXT NOT NULL,
    member TEXT NOT NULL,
    joined_at INTEGER NOT NULL,
    left_at INTEGER,
    read_upto INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (room, member)
);
CREATE INDEX IF NOT EXISTS room_members_member ON room_members (member);
-- `sender` and `recipient` use the member forms; a NULL recipient is everyone.
CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    room TEXT NOT NULL,
    sender TEXT NOT NULL,
    recipient TEXT,
    kind TEXT NOT NULL,
    reply_to INTEGER,
    text TEXT NOT NULL,
    at INTEGER NOT NULL,
    answered_at INTEGER,
    breakdown_version INTEGER
);
CREATE INDEX IF NOT EXISTS posts_room ON posts (room, id);
