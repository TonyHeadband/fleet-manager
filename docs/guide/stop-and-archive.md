# Stop now, archive later

Ending an agent and cleaning up after it are two separate actions, so stopping never costs you work.

Stop ends the agent and keeps everything on disk: worktrees, branches, the task folder and its record. Archive,
from the Tasks view, cleans up a stopped task's disk after showing exactly what it will delete and what stays;
branches are never deleted, and the record stays in the history.

## The chats stay

An archive deletes the agents, their sessions and their folders. It does not touch the transcripts in the persona's
Claude config folder, but they do not last: Claude Code's own cleanup (`cleanupPeriodDays`) prunes them later, and
deleting the persona removes them. So before anything is deleted, Archive copies each agent's chat into Fleet
Manager's own state folder, so a task's conversations survive:

- Where: `transcripts/<task-id>/<agent-id>/` in the state folder (`state_dir`, see
  [Configuration](../reference/configuration.md)).
- What: the prompt that opened the chat (`PROMPT.md`), one `<session-id>.jsonl` transcript per Claude session, and a
  `chat.json` with the session list and when the agent stopped. Expect a few MB per task.
- In the plan: the archive confirmation lists this folder as staying, with its size. It is not counted in the space
  the archive frees.
- Never deleted: Archive does not remove this folder.
- If the copy fails, nothing is deleted, and the task's log says why.

An archived agent's chat history can still be read through the chat history API
(`GET /api/agents/{id}/chat/history`). The task page does not show it yet.

A task with an open pull request cannot be archived: merge or close the PR first.
