# Roadmap

Written 2026-10-24 with Antonio. This is the direction, not a plan: each phase gets its own design when it starts.
Designs are kept privately; the detailed backlog is the GitHub issues.
It replaces the Roadmap section of `docs/concept.md`.

## Vision

fleet-manager (named fleet-board until 2026-10-24) is a strategist's view of a fleet of coding agents, like a tactics game with a
fleet of planes. You see the whole operation at once and either fly agents by hand or put them on autopilot.

- **Autopilot.** You hand a task to an orchestrator agent. It agrees the goal with you, breaks it into steps,
  launches specialists, answers their questions and permission prompts under a policy you assigned, and runs the
  operation to completion or failure while you are away from the computer.
- **Specialists.** Agents with one narrow job, such as a surgical bug-fixer. Their limits are enforced by
  fleet-manager, not only written in their prompt. Each specialist draws on a library of skills, so it has a
  better toolbox for its job.
- **Many views, not one board.** Kanban is the first view. The operation, the plan and the audit trail get their
  own. This is why the name is fleet-manager.
- **Later, beyond one machine.** Take over a session from the web UI, then share one view of sessions with other
  people while each person's agents work independently.

The scarce resource stays the developer's attention (`docs/concept.md`). Autopilot spends less of it; it must
never spend it badly, by doing something you would have refused.

## Principles and anti-goals

- **Autopilot acts only inside a policy you wrote.** Anything outside it is escalated. Every automatic decision is
  logged with the rule that allowed it. The daemon still never changes a permission mode silently.
- **Budgets are hard stops.** Tokens, wall-clock time and maximum loops per task. Hitting one stops the
  task and alerts you; it never quietly carries on.
- **Specialist limits are enforced** (allowed paths, max files touched, tools, budget). Where a runtime cannot
  enforce a limit, the UI says "best effort".
- **Default stop is an open PR.** Automerge exists only as an explicit policy option.
- **Escalation stops only the agent concerned.** The rest of the fleet keeps going.
- **Remotely, you talk to the orchestrator by default.** When a task has one, escalations, alerts and your
  replies go through it; it relays and stays accountable for the operation. Taking over a worker directly is
  always possible, as an explicit override that the orchestrator is told about. A task without an
  orchestrator is reached worker by worker.
- **Single user, single machine first.** Remote and multi-user come after Phase 1 is fully usable.
- **Provider-neutral by design.** New fields, keys and formats are named neutrally (`agent_command`, not
  `claude_*`), even while Claude Code is the only runtime.
- **Adapters plus a neutral route.** Every capability has a provider-neutral route (MCP tools, instructions file)
  that works on any runtime. A runtime adapter, Claude Code first, may add a richer route on top (its hooks, its
  own memory, its skills), never replace the neutral one.
- Kept from v1: local-first and self-hosted; never read, store or reuse Claude login tokens outside the `claude`
  CLI; a board that shows the wrong status is a failure.

## Releases

Versions are release cuts across the phases, not phase numbers. Today is v0.1. (The "v1" in `docs/concept.md` and
older specs is the first board, which shipped as v0.1.)

- **v1.0**: the first release that passes both tests of the orchestrator round: a simple bug fixed by a
  `direct` orchestrator and one specialist, and a larger change run by a `structured` orchestrator with more than
  one specialist, each ending in an open PR with every agent visible on the board. Claude Code is the only runtime
  it is tested on; other providers come later (Horizon). It lands inside Phase 1, before autopilot.
  v1.0 is the public face (README, docs, demo media), so the road to it is, in order (Antonio, 2026-10-26):
  1. Orchestrator round up to the `direct` test passing and merged, then paused.
  2. **Restructure and rename**: Product › Release › Task › Step. A *product* (e.g. fleet-manager) holds releases
     and the remembered repos (moved from personas). A *release* is a version with a calendar span; its progress is
     tasks merged out of total, beside days used. A *task* is the former project, renamed; it is done when its PR is
     merged, and an orchestrated task's progress is steps done out of steps planned. *Step* and *agent*
     (orchestrator, specialist) keep their names. No data migration: the database is wiped (personas kept).
  3. Orchestrator round resumed: the `structured` test, on the renamed model.
  4. Demo agent media (#179), then the README and docs rewrite, run as a `direct` orchestrated task by the
     built-in `docs` specialist: a second test of the orchestrator flow, done in the app rather than by hand.
- **After v1.0, fleet-manager is developed on fleet-manager.** Its own work moves into the app (orchestrated
  tasks instead of subagents hidden in one chat), so the project gets full insight into how the fleet performs
  on real work, and dogfooding drives the rest of Phase 1.

## Now: finish the foundation

In flight or next, before Phase 1 starts:

- Folder picker: done (4e80b25). Rename to fleet-manager: done; the icon stays the five status bars, now also
  shown in the window and taskbar. Next, the app shell: drawer, views,
  Settings, footer messages and one logger. The app
  shell is where new views land.
- Personas: done. One set of accounts per task (a Claude login, a GitHub login on one host, a commit identity),
  a first-run wizard, and the Management view (Personas, Instructions, Agent settings), which is where the Phase 1
  fleet pieces (specialists, skills, policies) land as tabs.
  A persona is also the entry point for Phase 2 and 3: the person who logs in later owns several personas.
- The open v0.1.x bugs on GitHub (`bug` label).

## Phase 1: autopilot for one user on one machine

Done when Antonio can create a task in the morning, agree the goal with an orchestrator, leave, and come back
to an open PR or to a clear failure report, with every automatic decision in the audit log.

Roughly in dependency order; each step is useful on its own.

1. **Runtime boundary.** The minimal part of #25: launch, MCP attachment, instructions file, status source and
   input behind one `Runtime` interface, with Claude Code as the only implementation. Needed so specialists and
   orchestrators are not tied to one CLI.
2. **Specialist definitions.** (Done, except limit enforcement; see "First cut of 8" below.) A provider-neutral
   format: role prompt, allowed tools, scope, hard limits. The daemon compiles it into what the runtime needs
   (`CLAUDE.md` / `AGENTS.md`, generated settings, hooks) and enforces the limits through `PreToolUse` denials.
   Ships a built-in set (bug-fixer, test-writer, reviewer, …); user definitions sit next to it. Absorbs task
   templates (#54) and the edit guard outside workspaces (#58).
3. **Skills.** (Done: built-in superpowers copies plus user skills, installed per definition.) A skill library
   specialists equip: reusable, provider-neutral instructions and helper scripts for one kind of work (bisecting a
   regression, writing a migration, …). Built-in skills plus user skills; a specialist lists the skills it gets,
   and the daemon installs only those into the agent's runtime (Claude Code skills directory, or the instructions
   file for runtimes without skills).
4. **Messaging.** Orchestrator ↔ agent messages through the daemon's MCP server, using held tool calls (up to
   about 40 min, measured in a spike). Messages carry source agent and owner and are never
   treated as instructions from the user.
5. **Context and memory.** Essential for autopilot: the orchestrator hands specialists a short brief, and they
   fetch the rest on demand instead of spending tokens re-reading folders and re-learning what siblings found.
   - *Stores:* task memory (decisions, findings, conventions, dead ends; lives with the task) and app
     memory (preferences, repo facts, lessons that outlast a task).
   - *Index:* one per repo, built at the base commit and shared by every worktree on that repo, updated when
     agents edit. Hybrid retrieval: keyword (SQLite FTS5), code symbols and vector embeddings (`sqlite-vec` in
     the existing SQLite). A local embedding model by default, so it works offline and costs no API tokens; a
     provider's embedding API is optional.
   - *Writing:* agents write freely, at any point in the step, not only at the end. The orchestrator curates:
     prunes, merges, promotes task findings to app memory, and pushes a new finding to running agents it
     concerns. At the end of each step it asks the specialist for a debrief.
   - *Routes:* neutral MCP tools (`remember`, `recall`, `search_code`) for every runtime. The Claude Code adapter
     also captures Claude's own memory writes and harvests findings before context compaction and at session end,
     through its hooks.
   - *Risks:* a stale index gives confident wrong answers, so every result carries the commit or time it was
     indexed; agents may grep anyway, so specialist definitions steer them to the tools; the token saving is
     measured (same task with and without), not assumed.
6. **Policy and audit.** Per-task policy that answers permission prompts and questions automatically, escalates
   the rest, and records every decision. Absorbs per-task permission profiles (#55) and bulk approve (#45).
7. **Budgets and cost.** Token, time and loop budgets per task, with live usage. Builds on #24 (OpenTelemetry)
   and the transcript `usage` records. Absorbs "token and time cost per task" and "cap concurrent agents".
8. **Orchestrator.** (First cut done, below.) Orchestrator types, chosen at task creation, are workflows plus a
   temperament and a policy. A "structured" type runs brainstorm → spec → plan → one specialist per step with a
   review between steps (the subagent-driven process this repo is built with, made durable); a "quick fix" type
   goes straight to one bug-fixer. Every type: agree the goal with you → step breakdown → specialists launched and
   supervised → open PR, or automerge when the policy allows it → completion or failure report. Specs and plans are
   files you can read. The orchestrator is itself an agent on a runtime. First test: rebuild a feature of this repo
   from its own plan on autopilot and compare.
   *First cut (2026-10-25):* types
   `direct` and `structured`, a breakdown Antonio approves, specialists launched through the daemon, one task
   branch and one PR behind a test gate, the board grouping a task with a rolled-up status, and Management tabs
   for orchestrators, specialists and skills. No autopilot: every prompt still reaches Antonio. Still open:
   `PreToolUse` enforcement of specialist limits (stored and shown as best effort), the quick views of the
   operation, several repos per task, and the acceptance runs.
9. **Notifications.** Desktop notification or toast when the window is not focused; an outgoing webhook (ntfy,
   Slack, …) for when you are away, sent by the orchestrator. One-way only; answering remotely is Phase 2.
10. **Operation views.** Plan and step tree, audit log, budget burn, memory browser. Absorbs the chat view (#61), history (#51) and
    Review inbox (#52) where the operation needs them.
11. **Agent and skill builder.** Create and edit specialist definitions and skills in the app, drafted with the
    provider you select.

## Phase 2: remote

- Per-run secret and auth on the API before anything leaves loopback (#63, #35).
- Web UI back to parity with egui, then used for remote access (#75).
- A remote channel to each task's orchestrator: answer its escalations, redirect it, stop the operation. The
  Phase 1 webhook becomes a round trip.
- Watch workers remotely (board, terminal, diffs, memory). Take over a worker as an explicit override: the
  interactive terminal (bidirectional pty relay), with the orchestrator told so it does not fight you.

## Phase 3: multi-user

The team hub from `docs/concept.md`: one daemon per person, a hub that aggregates them into shared views, and
cross-owner messages that the receiving owner approves by default. Each person's agents work independently;
the hub detects and surfaces cross-user interference instead of letting agents act on other people's work. The
open questions in `docs/concept.md` (where agents run, visibility, identity) are answered when this starts.

## Horizon: optional, no timing

These happen eventually, in any order, when they are worth it.

- **Other providers**: the rest of #25 (Codex, Gemini, OpenCode, Aider, … and plain API keys), a Providers
  settings tab, a runtime and model picker per agent.
- **Platforms**: macOS (#77), Windows through WSL2 (#76).
- **Release**: release binaries (#48), systemd user service (#78), CI on pull requests, contributing guide, open-source launch.
