# Roadmap

Written with the owner; last revised 2026-10-29. This is the direction, not a plan: each step gets its own design
when it starts. Designs are kept privately; the detailed backlog is the GitHub issues.

## Vision

Fleet Manager is infrastructure that makes a small team more efficient and organised with coding agents: everyone
sees the whole operation at once, like a strategist with a fleet of planes, and flies agents by hand or puts them
on autopilot.

It gets there one developer at a time. If the app does not make solo work clearly better, a team adds nothing, so
the roadmap makes solo work excellent first and builds team features only after that. The team vision still shapes
today's decisions (see "Team-ready, not team-built" below).

- **Autopilot.** You hand a task to an orchestrator agent. It agrees the goal with you, breaks it into steps,
  launches specialists, answers their questions and permission prompts under a policy you assigned, and runs the
  operation to completion or failure while you are away from the computer.
- **Specialists.** Agents with one narrow job, such as a surgical bug-fixer. Their limits are enforced by
  Fleet Manager, not only written in their prompt. Each specialist draws on a library of skills, so it has a
  better toolbox for its job.
- **Many views, not one board.** Kanban is the first view. The operation, the plan and the audit trail get their
  own.
- **Then a team.** Each person's agents work on their own machine under their own logins; a hub joins everyone's
  work into shared views, with releases, ownership and review across people.

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
- **Solo first, team-ready, not team-built.** Solo features must not close the door on a team: the daemon is the
  only source of truth (so daemon and clients deploy apart), tasks and decisions record who owns them, data
  changes go through migrations rather than wipes once people other than the owner use it, conventions
  (specialists, skills, policies, workflow) can live in the repo, and the API can sit behind authentication.
- **Provider-neutral by design.** New fields, keys and formats are named neutrally (`agent_command`, not
  `claude_*`), even while Claude Code is the only runtime.
- **Adapters plus a neutral route.** Every capability has a provider-neutral route (MCP tools, instructions file)
  that works on any runtime. A runtime adapter, Claude Code first, may add a richer route on top (its hooks, its
  own memory, its skills), never replace the neutral one.
- Kept from v1: local-first and self-hosted; never read, store or reuse Claude login tokens outside the `claude`
  CLI; a board that shows the wrong status is a failure.

## Releases

Versions are release cuts, not phase numbers. (The "v1" in `docs/concept.md` and older specs is the first board,
which shipped as v0.1.)

| Release | What it means |
|---|---|
| v0.1 | The first board: Linux only, built from source. |
| **v0.2** | Installable on Windows and Linux: one archive per system, a setup wizard that installs what is missing. |
| **v1.0** | Solo work is excellent: the structured orchestrator runs real feature work, and Fleet Manager is developed on Fleet Manager. |
| v2.0 | The first team release. |

## Now: v0.2, installable on Windows and Linux

The cross-platform programme, so the app can be shared with people on Windows and Linux (Rocky/RHEL 8 and newer):

1. Portable baseline: per-OS folders, paths and child processes. Done.
2. Built-in terminals: each agent in a pseudo-terminal the daemon owns (ConPTY on Windows), replacing tmux;
   `fleet-attach` takes one over. Done.
3. Release bundle: one archive per system, the daemon as the only truth about installed tools, a wizard that
   installs what is missing, an update notice. Done.
4. Docs updated for all of the above. In progress.
5. Release CI: a version tag builds the archives into a draft GitHub release (#48).

Deferred out of v0.2: close to the tray (#343), making `gh` optional, and remote access with keys (Phase 2).

## Next: v1.0, the structured orchestrator

The direct orchestrator already proves the idea on bug fixes and docs updates. The structured one (brainstorm →
spec → plan → one specialist per step, with a review between steps) is what real feature work needs, and it is
the point at which the owner moves daily work into the app. It is the focus right after v0.2.

1. Run structured properly on a real feature, and build what it was missing.
2. Pass the acceptance test: a larger change run by a `structured` orchestrator with more than one specialist,
   ending in one open PR, with every agent visible on the board.
3. From then on, Fleet Manager is developed on Fleet Manager: its own work runs as orchestrated tasks instead of
   subagents hidden in one chat, so the project sees how the fleet performs on real work.

What the first cut left open, to settle along the way: `PreToolUse` enforcement of specialist limits (stored and
shown as best effort today), quick views of the operation, and several repos per task.

## After v1.0: dogfooding decides

Once the owner works in the app every day, real use ranks what comes next. The phases below are the direction;
these are the candidates expected to matter first:

- **Usage limits.** Detect when Claude hits its usage limit: the card says "limited until 14:00" instead of
  looking stuck, and the agent resumes by itself.
- **Notifications.** A desktop notification when a card needs you and the window is not focused; with the tray
  (#343).
- **The PR loop.** The card follows its pull request (checks, review comments, merged); one click starts an agent
  on the review comments.
- **Task from an issue.** Paste an issue URL: goal, repo and product fill in, and the PR links back.
- **Stuck and loop detection.** An agent idle too long, or repeating a failing call, is flagged.
- **Cost per task**, read-only from the transcripts, before budgets enforce anything.
- **Data safety.** Schema migrations with a backup before each, and export of tasks and personas.
- **Copy diagnostics.** Versions, system, tool check and recent log in one click, for bug reports.
- **Keyboard flow.** Jump to the next card that needs you; a command palette.

## Phase 1: autopilot for one user on one machine

Done when the owner can create a task in the morning, agree the goal with an orchestrator, leave, and come back
to an open PR or to a clear failure report, with every automatic decision in the audit log.

Roughly in dependency order; each step is useful on its own.

1. **Runtime boundary.** The minimal part of #25: launch, MCP attachment, instructions file, status source and
   input behind one `Runtime` interface, with Claude Code as the only implementation.
2. **Specialist definitions.** Done, except limit enforcement. A provider-neutral format: role prompt, allowed
   tools, scope, hard limits, compiled into what the runtime needs and enforced through `PreToolUse` denials.
3. **Skills.** Done: built-in skills plus user skills, installed per definition.
4. **Messaging.** Orchestrator ↔ agent messages through the daemon's MCP server, using held tool calls. Messages
   carry source agent and owner and are never treated as instructions from the user.
5. **Context and memory.** The orchestrator hands specialists a short brief, and they fetch the rest on demand.
   Start small: task memory and app memory behind neutral MCP tools (`remember`, `recall`), with the token saving
   measured, not assumed. A per-repo code index (keyword, symbols, embeddings in SQLite, a local model by default)
   is its own later step; every result carries the commit or time it was indexed.
6. **Policy and audit.** Per-task policy that answers permission prompts and questions automatically, escalates
   the rest, and records every decision. Absorbs per-task permission profiles (#55) and bulk approve (#45).
7. **Budgets.** Token, time and loop budgets per task, with live usage. Builds on #24 (OpenTelemetry).
8. **Orchestrator autonomy.** The orchestrator answers its specialists under the policy, so a task runs while you
   are away; automerge only when the policy allows it.
9. **Outgoing alerts.** A webhook (ntfy, Slack, …) for when you are away, sent by the orchestrator. One-way only;
   answering remotely is Phase 2.
10. **Operation views.** Plan and step tree, audit log, budget burn, memory browser. Absorbs the chat view (#61),
    history (#51) and review inbox (#52).
11. **Agent and skill builder.** Create and edit specialist definitions and skills in the app.

## Phase 2: remote

- Authentication on the API before anything leaves loopback (#63, #35), including a per-run secret so an agent
  cannot drive the API.
- Remote access with keys: attach to your own agents only. Taking over a worker already works locally
  (`fleet-attach`); this carries it over the network.
- Web UI back to parity with the desktop app, then used for remote access (#75).
- A remote channel to each task's orchestrator: answer its escalations, redirect it, stop the operation. The
  Phase 1 webhook becomes a round trip.

## Phase 3: a small team (v2.0)

One daemon per person and a hub that joins them into shared views. The hub aggregates events and relays messages;
it never touches terminals or tokens. It can run apart from any desktop (a container), since the daemon is already
the only truth. Expected pieces, settled when the phase starts:

- **Team board:** who, and which agents, are working on what, for which release.
- **Ownership and handoff:** a task belongs to a person, can be reassigned, and a stopped task can be picked up by
  a teammate on their own machine.
- **Collision warnings:** two people's agents in the same files, flagged before they become a merge conflict.
- **Shared conventions and memory:** the team's specialists, skills and policies versioned in the repo; repo facts
  learned by one person's agents available to everyone's.
- **Review routing:** an agent's PR goes to a teammate, shown on the board as waiting on them.
- **Cost per person and per release.**
- **Light roles:** who may approve a breakdown or change the team's policies.
- Cross-owner messages need the receiving owner's approval by default.

The open questions in `docs/concept.md` (where agents run, visibility, identity) are answered when this starts.

## Horizon: optional, no timing

- **Other providers**: the rest of #25 (Codex, Gemini, OpenCode, Aider, … and plain API keys), a Providers
  settings tab, a runtime and model picker per agent.
- **Platforms**: macOS (#77).
- **Desktop**: close to the tray (#343); possibly a Tauri front end in place of egui.
- **Running the daemon as a service**: a systemd user service (#78), or a container.
- **Open-source launch**: open contributions once the project is stable.
