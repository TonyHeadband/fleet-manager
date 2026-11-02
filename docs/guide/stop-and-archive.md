# Stop now, archive later

Ending an agent and cleaning up after it are two separate actions, so stopping never costs you work.

Stop ends the agent and keeps everything on disk: worktrees, branches, the task folder and its record. Archive,
from the Tasks view, cleans up a stopped task's disk after showing exactly what it will delete and what stays;
branches are never deleted, and the record stays in the history.

Each agent's chat stays readable too. Archive first copies its prompt and Claude transcripts to
`transcripts/<task-id>/<agent-id>/` in the daemon's state folder, listed in the plan as staying, and never deletes
that folder. If the copy fails, nothing is removed and the task's log says why.

A task with an open pull request cannot be archived: merge or close the PR first.
