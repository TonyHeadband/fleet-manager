# TODO

Deferred work from the v1 design (`docs/superpowers/specs/2026-10-23-fleet-board-design.md`). `gh` is only logged in to the work GHE here, so these live in this file until they can be filed as issues on `TonyHeadband/fleet-board`.

- [ ] Add the login page that drives the `claude` CLI's own login — blocked on the Anthropic admin confirming that wrapping the CLI is acceptable for the Enterprise seat; ship behind a flag (design: Login)
- [ ] Add agent-to-agent messaging through the daemon (MCP server with `list_agents`, `send_message`, `read_inbox`) and the "Talking" column — deferred to team mode (`docs/concept.md`); for one owner, collision warnings cover most of the need; data model keeps `actor` and a `Talking` status for it
- [ ] Add bulk approve and broadcast to the fleet controls — deferred past v1
- [ ] Add the orchestrator agent, built on the same messaging tools — after messaging
- [ ] Add a local-only egui client over the same `fleet-api` contract — deferred; proves the contract is UI-agnostic
- [ ] Add team mode: a hub that aggregates per-person daemons into a team board and relays cross-owner messages (owner approves delivery by default), with login and authorization on `actor`/`owner`/`project` — final extra goal (`docs/concept.md`); v1 binds `127.0.0.1` with no auth
- [ ] Add a release pipeline for the Linux x86_64 and aarch64 binaries, with Nexus mirrors for crates and npm — deferred past v1
- [ ] Make the terminal preview interactive (bidirectional pty relay) — v1 preview is read-only, use `tmux attach` for full control

From `docs/concept.md` (2026-10-23):

- [x] Add a blocked reason to Needs you cards (permission, question, error, startup) — **v1**, decided 2026-10-23; touches finished Tasks 2–3: a field on `Agent` in `crates/fleet-api/src/model.rs` and the mapping in `crates/fleet-daemon/src/status.rs` — done in `fleet-api`, `status.rs`, `store.rs`, `ingest.rs`, `agents.rs`, and the plan is amended
- [x] Add an `owner` field to `Agent`, always the local user — cheap now, keeps team mode possible; do it with the blocked reason to change the contract once
- [ ] Sort Needs you by time waiting and ping the user (desktop or terminal notification) — v1.x
- [ ] Keep a history of agents with the full original prompt, branch and outcome (merged, discarded, redone); the card links to it — v1.x; v1 keeps only the first line in `Agent.task` (≤120 chars) so events and snapshots stay small, so the full prompt is stored separately (not on `Agent`) when this lands
- [ ] Turn Review into an inbox: diff size, files touched, test result — v1.x
- [ ] Warn when two agents' branches in the same repo change the same paths (a predicted merge conflict) — v1.x, hard but worth keeping; each agent has its own worktree, so nothing clobbers live: compare repo-relative paths across sibling worktrees (`git diff --name-only <base>...HEAD` per worktree, refreshed on `PostToolUse` of Edit/Write); open questions: base moving under them, renames, and whether overlap at file level is too noisy versus hunk level
- [ ] Add task templates with a preset permission profile — v2
- [ ] Add per-project permission profiles — v2; the daemon must still never force a mode silently (spike finding)
- [ ] Cap concurrent agents, per project and overall — v2
- [ ] Show token and time cost per task — v2; checked 2026-10-23: transcripts carry `usage` on assistant messages (hooks do not), so build it on the transcript chat view

From plan Amendment A1 (2026-10-23), multi-repo projects:

- [ ] Let an agent add a repo mid-session: MCP tools `list_repos` and `request_repo(name, why)`, a `repo_access` blocked reason, and on approval a new workspace at `<project-dir>/<repo>` on the project's branch plus a `WorkspacesChanged` event — v1.x; A1 only ships the shape (one workspace per project)
- [ ] Block file edits outside a project's workspaces with a `PreToolUse` hook that points the agent to `request_repo` — v1.x; until then an agent can edit another repo's main checkout unnoticed
- [ ] Flag configured repos whose main checkout becomes dirty while projects run (periodic `git status`) — v1.x; backstop for Bash writes the edit guard cannot see; attribution to an agent is a guess and the card must say so
- [ ] Show merge order across a project's repos in Review (default: the order repos were added, overridable) — v1.x

Chat view (2026-10-23):

- [ ] Add a chat view per agent built from its Claude Code transcript — v1.x; agents stay interactive in tmux (headless rejected, `docs/concept.md`); every hook event carries `transcript_path`, so the daemon follows that JSONL and streams `user`/`assistant` messages (text, tool_use, tool_result) over SSE; permission prompts come from the `PermissionRequest` hook, not the transcript; replies still go through `answer` (tmux send-keys); the format is internal and undocumented, so parse only `user`/`assistant`, ignore unknown record types and pin tests to captured fixtures; per-message, not per-token. Also feeds the history item (store the transcript path, which holds the full prompt) and token cost (`usage` on assistant messages)

From the v1 review (2026-10-23):

- [ ] Make repo-level Claude settings apply: merge each repo's `.claude/settings.json` permission rules (and decide on hooks and `.mcp.json`) into the generated `--settings` file, rewriting relative path rules with the repo directory prefix — v1.x; measured on claude 2.1.309: a repo `deny` rule is ignored when claude starts in the project directory, which is a safety regression against running inside the repo; until then the daemon logs a warning at spawn when a repo has such files (done 2026-10-23; Antonio chose to accept the limit for v1)
- [ ] Protect the daemon's API from agents with a per-run secret printed in the URL and required (cookie, `SameSite=Strict`) on every `/api` route except the hook route — v1.x; an agent in a tmux session can reach `127.0.0.1:<port>` with a valid Host, and the Host/Origin guard allows it
- [ ] Keep the prompt out of `ps`: start `claude` without the prompt and send it after `SessionStart` — v1.x; trade-off is the keystroke-loss risk the argv approach avoids
- [ ] Let Kill work when the repo left the config or git cannot remove the worktree (fall back to `rm -rf` plus `git worktree prune`) — v1.x
- [ ] Fail a spawn with a clear message when `claude_command[0]` is not on the daemon's PATH, instead of a card that goes Dead seconds later; add a `tmux_command` config key — v1.x
- [ ] Use one SQLite connection and recommend a local-disk `state_dir` — v1.x
- [ ] Make `pnpm dev` work through the Host/Origin guard (proxy `changeOrigin`, drop `Origin`) — v1.x
- [ ] Give `fleet-daemon` `--help`, reject unknown arguments and handle SIGTERM — v1.x
- [ ] Document Nexus mirrors for crates, npm and PyPI (`ziglang`) and the `rustup target add` download in `build.sh` — v1.x
- [ ] Add the synthetic hook fixtures promised in Task 1 (`StopFailure`, `idle_prompt`, `elicitation_*`, `agent_needs_input`) and a test for the SSE `Lagged` resync path — v1.x
- [ ] Add a label map to the schema so badges show "Permission needed" instead of `permission` — v1.x
- [ ] Verify on the real CLI that `AskUserQuestion`, plan-mode exit and MCP elicitation fire `PermissionRequest` or `agent_needs_input` (otherwise such a card stays Working while blocked), and whether `idle_prompt` fires after an Escape interrupt — spike
