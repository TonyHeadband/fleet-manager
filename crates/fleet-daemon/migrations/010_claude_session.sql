-- The Claude Code session each agent runs, so a daemon restart can resume it with `claude --resume`.
ALTER TABLE agents ADD COLUMN claude_session TEXT NULL;
