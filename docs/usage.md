# Using Fleet Manager

## Working with repos

Pick a repo with Browse… in the New task dialog or the side panel's Add repo. Open a repo in the picker and
choose one of its checkouts to start from the branch it is on. Repos belong to products: a repo with a URL starts
from the remote's default branch (`origin/HEAD`), and New task can start one from a local checkout when the repo
has a path.

Uncommitted changes in a checkout are not carried over: the agent starts from the last commit, and the
picker tells you how many files that leaves out.

The agent works on its own branch, `fleet/<task>`. That branch contains every commit of the branch it
started from, so merging it into that branch makes the old branch fully merged. Hover a repo chip in the
side panel to see where it started; for a repo started from a local branch it also shows the merge route.

If you kept committing on the old branch while the agent worked, merge `fleet/<task>` into the old
branch first, then merge the old branch as usual. With no new commits there, that first merge is a
fast-forward.

Kill ends the agent's session and removes its worktree; the `fleet/<task>` branch stays, so nothing the
agent committed is lost.
