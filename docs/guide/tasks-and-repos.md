# Tasks and repos

A task is one agent session working toward a goal, in its own folder under the tasks root. Each repo it touches
gets a git worktree on the task's branch, `fleet/<task>`, so agents never step on each other or on your checkouts.
A task's repos come from its product, and an agent can ask for another of the product's repos mid-session (you
grant or refuse it from the card).

## Task titles

A card shows the task's title, not its whole goal. Type one in New task's **Title** field, or leave it empty and
the agent names the task itself, once, as it starts (the `set_title` tool). Until it does, the card shows the
goal's first sentence, cut short. A title typed in New task is never replaced. The side panel shows the title with
the full goal under it.

## Working with repos

Repos belong to products, and a product's repos change only in Management → Products. New task offers the
selected product's repos to pick from; a repo the product lacks is added in Products first. A repo with a URL
starts from the remote's default branch (`origin/HEAD`), and New task can start one from a local checkout when
the repo has a path.

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
