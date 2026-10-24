# fleet-board: design

Status: draft for approval. Marks: **[decided]** = said by Antonio, **[my call]** = chosen by the author, open to override, **[verify]** = depends on an external fact not yet checked.

## Intent and why

Antonio runs many Claude Code agents across many repos and loses track of which agent is doing what and what state it is in. VS Code handles multi-project, multi-agent work badly. Claude Squad (Go, TUI, tmux + git worktree per agent) has the right session model but is per-repo and shows status by scraping panes. Vibe Kanban (Rust + React, Apache-2.0) has the right UI idea, a board where agents' work moves by status, but it is sunsetting and is not a base to build on. **[decided]** fleet-board is a new app, inspired by both, not a fork.

The product is a live, accurate, fleet-wide board plus simple controls to direct agents. It is free and open source. It is for Antonio first and maybe for their team later. **[decided]**

## Goals

- One board showing every agent across all projects, with a project filter. Cards move between columns automatically as status changes. **[decided]**
- Status is accurate. A card never says "Working" while the agent is blocked on a prompt. **[decided]**
- Fleet controls. At minimum: direct an agent to work on something with a simple command (pick a repo, give a prompt, an agent starts in its own tmux session and git worktree). From a card: answer, pause, kill. **[decided]**
- Agents run the real `claude` CLI inside tmux, one tmux session per agent. **[decided]**
- A login page that drives Claude Code's own CLI login. See Login. **[decided]**
- A clear client/daemon contract so a web UI, and later a local-only egui client or multi-user mode, can be added without changing the backend. **[decided]**

## Non-goals

- Not a code editor. Editing stays in the user's IDE and in the agents' terminals. **[my call]**
- Not a hosted or cloud service. Self-hosted, local-first. **[my call]**
- No support for agents other than Claude Code in v1. **[my call]**

## Anti-goals (a failure even if it technically works)

- A board that shows the wrong status.
- Reading, storing, or reusing Claude login tokens outside the `claude` CLI itself.
- Company code, prompts, repo names or internal URLs ending up in this public personal repo. The repo is Antonio's personal one **[decided]**, so anything work-specific stays out of it and in private config.

## v1 scope

In:
- Daemon, with the contract below.
- Web UI: the mixed board (columns below), project filter, a spawn-agent command, per-card answer/pause/kill, a live preview of an agent's terminal.
- Login page.
- Per-agent tmux session and git worktree management.
- Hook-driven status (see Status model).

Out for v1, but the data model must not block them:
- Agent-to-agent messaging and the "Talking" column.
- Bulk approve and broadcast.
- The orchestrator agent.
- egui client and multi-user mode.

Columns **[my call]**: Queued, Working, Needs you, Talking (empty in v1), Review. Each card shows project tag, task, a one-line current activity, and time waiting if blocked. Card contents and column names are the author's guess.

## Architecture

### Daemon

A Rust daemon (Axum on Tokio) owns all state: agents, tmux sessions, worktrees, status, and later messages. UIs hold no logic. State lives in SQLite, a **[my call]** because this is a local tool with nothing to deploy.

Agent lifecycle: the daemon creates a git worktree on a new branch for the repo, starts a tmux session running `claude` in it, and tracks the session by id. On startup it re-discovers sessions with `tmux ls`, so agents survive daemon restarts. tmux 3.8 is installed in `~/.local/bin` on Antonio's machine because Rocky 8 ships 2.7. **[verify]** the minimum tmux version the daemon needs.

### The contract

Three parts, defined once in a shared Rust crate (working name `fleet-api`) that serves every client. **[decided]** that the contract is clear and shared, details are **[my call]**:

1. Commands and queries: JSON over HTTP, for example `spawn_agent`, `send_prompt`, `pause_agent`, `kill_agent`, `list_agents`. The OpenAPI schema is generated from the Rust types.
2. Event stream: WebSocket or SSE of typed events such as `agent.status_changed` and `agent.needs_input`. Each event has a monotonically increasing sequence number, so a client can fetch a snapshot, then resume from a sequence number after reconnecting.
3. Terminal stream per agent, for the live preview.

TypeScript types for the React UI are generated from the Rust crate so the two cannot drift. A future egui client depends on the crate directly.

Every command carries an `actor` and a `project` scope from v1, even with a single user, so permissions can be added later without changing the API shape.

### Access modes

Same API, different listener:
- Local-only: Unix socket, trust via file permissions. Natural for egui or a CLI.
- Web: TCP with login and sessions, behind TLS terminated by a proxy.
- Multi-user: later, adds identity and authorization on top of the existing `actor`.

### Frontend

React, TypeScript and Vite, built into the daemon binary and served by it. **[decided]** React/TypeScript; embedding in the binary is **[my call]**. The terminal preview uses xterm.js. **[my call]**

The UI is a pure client. A different UI must be replaceable without backend changes.

## Status model

Status comes from two sources:
1. Claude Code hooks. The daemon configures `claude` per agent to call a small command on events (permission or notification prompt, tool use, agent finished). That command posts the event to the daemon over a local Unix socket. This is the primary source for Working, Needs you and Review. The hook ingress is internal, not part of the client contract.
2. tmux as backup. `capture-pane` for the preview and a liveness check that detects dead sessions.

**[verify]** the exact current Claude Code hook event names and how hooks are configured per session. The mapping from events to columns cannot be finalized until this is checked.

Do not infer status by scraping pane text.

## Login

The login page starts `claude` in a tmux pane and relays the login URL and code prompt through the page. Tokens stay in `~/.claude`. The daemon never reads or stores them. **[decided]** that the page connects to Claude Code login. The approach is the author's design.

**[verify]** with the Anthropic admin that wrapping the real CLI this way is acceptable for the Enterprise seat. As the author understands Anthropic's terms, a Claude login may only be used by Claude Code itself. If this is not acceptable, fall back to an API key from the org.

## Constraints

- Rust daemon, cross-compiled. **[decided]** "Rust mandatory, multi-compiling". tmux is POSIX-only, so Windows is out unless Antonio says otherwise. **[my call]**
- Open source, free. License: Apache-2.0. **[my call]**
- Self-hosted, no managed cloud services, air-gapped friendly: the UI is embedded in the binary and nothing is fetched at runtime. Dependencies come through crates.io and npm, which in an air-gapped build would be mirrored through Nexus. **[my call]**
- Agent messages, when added, must be marked as coming from another agent so an agent never treats them as instructions from the user. Needs rate limits and a loop guard. **[my call]**

## Left to the builder

- Crate and module layout, error handling, test strategy.
- Exact REST paths, schema field names, SSE vs WebSocket.
- Auth mechanism for the web mode.
- Worktree location and cleanup policy (Claude Squad uses `~/.claude-squad/worktrees`).
- Database schema.

## Calls made after the builder review **[my call]**

- Phase 0 is a time-boxed spike, before everything else: prove the status model end to end. Start `claude` in tmux with `--settings <generated file>` carrying the hooks, post events to a Unix socket, and map them to the five columns. The `claude` CLI on Antonio's machine (2.1.309) accepts `--settings` and `--mcp-config`, so per-agent hook config does not need to touch the worktree's `.claude/` files. Checked against the Claude Code hooks docs on 2026-10-23: events `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest` (fires immediately when a permission prompt appears), `PostToolUse`, `PostToolUseFailure`, `Notification` (types include `permission_prompt` after about 6s, `idle_prompt` after about 60s, `elicitation_dialog`, `agent_needs_input`), `Stop`, `StopFailure`, `SessionEnd` all exist, and handlers can be `command` or `http` (POST of the event JSON). Still **[verify]** by the spike on the real CLI: that `--settings` hooks fire in an interactive tmux session, that http hooks reach a loopback URL, and how to handle the first-run "trust this folder" prompt, which no hook covers. If a state has no reliable signal, stop and revisit the no-scraping rule.
- Pause: send an interrupt (Escape) to the agent and mark the card paused. Resume sends a short continue prompt. No SIGSTOP.
- Answer: v1 types the reply into the pane with `tmux send-keys`.
- Repos: a configured list of paths in `~/.config/fleet-board/config.toml`, no directory scanning.
- Branches: `<prefix>/<task-slug>` branched from the repo's current HEAD, prefix configurable. A dirty main checkout does not matter because worktrees start from HEAD. A repo with no remote works, and push is simply unavailable.
- Review: the agent has stopped and its branch has changes. v1 only shows the diff. It has no PR or merge action.
- Events use SSE. Type generation uses utoipa for OpenAPI and ts-rs for TypeScript. The SQLite crate is the builder's choice.
- tmux: developed and tested against 3.x. 2.7 (Rocky 8's package) is not supported.

## Decisions from Antonio **[decided]** (accepted the recommendations)

- v1 runs local-only: the daemon binds to `127.0.0.1`, with no auth. Remote access waits until real login exists.
- Login is not in v1. v1 uses the `claude` login already in `~/.claude`. The login page is built last, behind a flag, after the Anthropic admin confirms that wrapping the CLI is acceptable for the Enterprise seat.
- The terminal preview is read-only. Answering is from the card, and full control is `tmux attach`.
- Cross-compile targets are Linux x86_64 and aarch64. There is no release pipeline in v1. The build needs Node as well as Cargo, so an air-gapped build needs Nexus mirrors for crates and npm.

## Other

- Repo location: `~/fleet-board` is assumed. **[my call]**
- Name: fleet-board. **[decided]**
