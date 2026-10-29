# Tasks and repos

A task is one agent session working toward a goal, in its own folder under the tasks root. Each repo it touches
gets a git worktree on the task's branch, `fleet/<task>`, so agents never step on each other or on your checkouts.
Repos can be paths, git URLs or names Fleet Manager remembers, and an agent can ask for another repo mid-session
(you grant or refuse it from the card).

## Working with repos

Pick a repo with Browse… in the New task dialog or the side panel's Add repo. Open a repo in the picker and
choose one of its checkouts to start from the branch it is on. Repos belong to products: a repo with a URL starts
from the remote's default branch (`origin/HEAD`), and New task can start one from a local checkout when the repo
has a path.

Uncommitted changes in a checkout are not carried over: the agent starts from the last commit, and the
picker tells you how many files that leaves out.

## Branches and merging

The agent works on its own branch, `fleet/<task>`. That branch contains every commit of the branch it
started from, so merging it into that branch makes the old branch fully merged. Hover a repo chip in the
side panel to see where it started; for a repo started from a local branch it also shows the merge route.

If you kept committing on the old branch while the agent worked, merge `fleet/<task>` into the old
branch first, then merge the old branch as usual. With no new commits there, that first merge is a
fast-forward.

Stop ends the agent's session and keeps its worktree; the `fleet/<task>` branch stays, so nothing the
agent committed is lost. Archive cleans up the disk later and never deletes the branch: see
[Stop now, archive later](stop-and-archive.md).
