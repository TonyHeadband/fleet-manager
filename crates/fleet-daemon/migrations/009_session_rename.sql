-- Agents were stored with the tmux session's name under `tmux_session`; the field is now `session`.
-- Events embed whole agents (agent_added); tasks never hold one.
UPDATE agents SET data = replace(data, '"tmux_session":', '"session":');
UPDATE events SET data = replace(data, '"tmux_session":', '"session":');
