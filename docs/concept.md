# fleet-manager: what fleet management means here

This sits above the design spec (`docs/superpowers/specs/2026-10-23-fleet-manager-design.md`). The spec says how fleet-manager is built; this says what problem it manages and in what order the features come.

## Framing

Fleet management everywhere covers the same six jobs: inventory, telemetry, dispatch, health and lifecycle, policy, audit and cost. For Claude Code agents they map to:

| Job | fleet-manager |
|---|---|
| Inventory | Which agents exist: repo, branch, worktree, tmux session |
| Telemetry | Hook-driven status: Working, Needs you, Review |
| Dispatch | Spawn an agent from a repo and a prompt |
| Health / lifecycle | Dead sessions, reconcile on restart, worktree cleanup |
| Policy | Permission profiles, concurrency caps (v2) |
| Audit / cost | History, tokens and time per task (v1.x / v2) |

In a vehicle fleet the scarce resource is the vehicles. In an agent fleet agents are cheap, and the scarce resource is **the developer's attention**, spent in two places:

1. **Unblocking.** An agent in Needs you is idle until its owner answers.
2. **Reviewing.** Finished work arrives faster than diffs get read.

So fleet-manager is not a monitor. It routes the owner's attention and keeps work moving through review.

## Scope: one developer, 5–6 agents

fleet-manager serves one developer running about 5–6 agents in parallel. At that size a board is enough. Orchestrators, bulk actions and policy engines are not needed yet.

Team use (several people, each owning agents, agents coordinating across owners) is the **final extra goal**, not a current requirement. See "Keeping team mode possible".

## Pains and what fixes them

| Pain | Fix | When |
|---|---|---|
| An agent was blocked and nobody noticed | Blocked reason on the card: permission, question, error, startup | v1 |
| | Needs you sorted by time waiting, plus a desktop or terminal ping | v1.x |
| Lost track of what an agent was asked | Task (first line of the prompt, up to 120 chars) and one-line activity on the card | v1 |
| | History of agents with the full prompt, branch and outcome; the card links to it | v1.x |
| | Chat view of the session, read from the Claude Code transcript (not from the tmux pane) | v1.x |
| Finished diffs pile up | Diff view | v1 |
| | Review inbox: diff size, files touched, test result | v1.x |
| | Overlap warning when two agents' branches change the same paths | v1.x |

The blocked reason is in v1 because the status reducer already sees which event moved the card to Needs you. The cost is one field and a mapping.

Each agent works in its own worktree, so two agents never overwrite each other's files while they run. The real risk is later: two branches of the same repo change the same paths, and the second merge conflicts. The overlap warning predicts that conflict while both agents are still running, when it is cheapest to fix. It compares repo-relative changed paths across sibling worktrees (`git diff --name-only <base>...HEAD`, refreshed when a `PostToolUse` on a file-editing tool arrives). This is harder than it looks: the base branch moves, files get renamed, and file-level overlap may be too noisy compared with hunk-level overlap. It needs no agent-to-agent messaging, which covers most of what messaging would do for a single owner.

## Projects and repos

A project is an idea. A solo project is run by one agent session; an orchestrated project (since 2026-10-25,
`docs/superpowers/specs/2026-10-25-orchestrator-specialists-design.md`) is run by an orchestrator agent that launches
specialist agents, each in its own worktree on a sub-branch, and ends in one PR. The rest of this section describes
the solo shape, which is also the orchestrator's. A solo project starts with one repo, and how much will change is not known up front, so repos are added while it runs. Each repo a project touches is a workspace: a worktree on the project's branch inside the project directory, which is `claude`'s cwd. A repo added later appears as a sibling directory that is already in the session's scope. Plan Amendment A1 puts this shape into v1 with one workspace per project.

Adding repos is v1.x and works in three layers:
1. The agent asks (`request_repo`), and the owner approves on a Needs you card.
2. A hook blocks file edits outside the project's workspaces and points the agent to `request_repo`.
3. A scan flags configured repos whose main checkout gets dirty, which catches Bash writes the hook cannot see. It cannot be sure which agent did it, and says so.

Review shows one changeset per project with a diff per repo, in merge order.

## Roadmap

Superseded on 2026-10-24 by `ROADMAP.md`, which moves the orchestrator to Phase 1 (autopilot for one user). Kept below as the v1 record.

1. **v1 (in progress):** board, spawn, answer/pause/kill, read-only preview, blocked reason.
2. **v1.x, solo polish:** Needs you queue and ping, history, Review inbox, collision warnings.
3. **v2, policy and cost:** task templates, per-project permission profiles, a concurrent-agent cap, token and time cost per task.
4. **Final extra goal, team mode:** see below.

Agents always run as the interactive `claude` CLI in tmux (Antonio, 2026-10-23). Headless mode (`claude -p` with stream-json) was considered and rejected: tmux keeps the real terminal and `tmux attach` as the way to take over.

Already deferred in the spec and still deferred: bulk approve and broadcast, the orchestrator, the egui client, the interactive terminal.

## Keeping team mode possible

The target shape is **one daemon per person plus a team hub**. Each developer's agents run under their own Claude login on their own machine, which is today's daemon. The hub aggregates events into a team board and relays messages, and never touches terminals or tokens. A central server running everyone's agents is ruled out by the anti-goal on Claude tokens.

Choices to keep now so the hub can be added without a rewrite:

- Every command carries `actor` and `project` (done).
- Agents carry an `owner`, the local user (`$USER`) for now. It is a plain string today; in team mode it must be the same identity as the command's `actor`.
- A person owns **personas** (since 2026-10-25, `docs/superpowers/specs/2026-10-25-personas-design.md`): named bundles of accounts (a Claude login, a GitHub login on one host, a commit identity), and every project belongs to one. The person is still the local OS user and `owner` is unchanged (`$USER`); a persona says which accounts a project runs on, not who owns it. In team mode the person is what logs in, and owns several personas.
- Events have sequence numbers, so a daemon can forward them to a hub and resume after disconnecting (in the spec).
- The UI talks to a configured fleet endpoint. It never assumes the daemon is on localhost or that only one daemon exists.
- Messages, when added, carry their source agent and owner, and an agent never treats them as instructions from the user (in the spec).

Questions for team mode, unanswered on purpose:

- Where agents run: developer workstations or shared Linux boxes.
- Visibility: whether teammates see prompts and terminal previews, or only status and touched files.
- What "direct" means. The leaning is visibility for everyone plus lead dispatch into the assignee's queue (the assignee accepts, and their own daemon spawns the agent). A lead does not control other people's agents directly, because that is acting through someone else's Claude seat.
- Identity: reuse existing SSO or give the hub its own login.
- Cross-owner messages are a prompt-injection channel. By default they reach the receiving owner as a Talking card to approve, with auto-delivery opt-in per project, plus rate limits and a loop guard.
