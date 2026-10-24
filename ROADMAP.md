# Roadmap

Written 2026-10-24 with Antonio. This is the direction, not a plan: each phase gets its own design in
`docs/superpowers/specs/` when it starts, and the detailed backlog stays in `TODO.md` and the GitHub issues.
It replaces the Roadmap section of `docs/concept.md`.

## Vision

fleet-manager (today fleet-board) is a strategist's view of a fleet of coding agents, like a tactics game with a
fleet of planes. You see the whole operation at once and either fly agents by hand or put them on autopilot.

- **Autopilot.** You hand a project to an orchestrator agent. It agrees the goal with you, breaks it into tasks,
  launches specialists, answers their questions and permission prompts under a policy you assigned, and runs the
  operation to completion or failure while you are away from the computer.
- **Specialists.** Agents with one narrow job, such as a surgical bug-fixer. Their limits are enforced by
  fleet-manager, not only written in their prompt. Each specialist draws on a library of skills, so it has a
  better toolbox for its job.
- **Many views, not one board.** Kanban is the first view. The operation, the plan and the audit trail get their
  own. This is why the name becomes fleet-manager.
- **Later, beyond one machine.** Take over a session from the web UI, then share one view of sessions with other
  people while each person's agents work independently.

The scarce resource stays the developer's attention (`docs/concept.md`). Autopilot spends less of it; it must
never spend it badly, by doing something you would have refused.

## Principles and anti-goals

- **Autopilot acts only inside a policy you wrote.** Anything outside it is escalated. Every automatic decision is
  logged with the rule that allowed it. The daemon still never changes a permission mode silently.
- **Budgets are hard stops.** Tokens, wall-clock time and maximum loops per project. Hitting one stops the
  project and alerts you; it never quietly carries on.
- **Specialist limits are enforced** (allowed paths, max files touched, tools, budget). Where a runtime cannot
  enforce a limit, the UI says "best effort".
- **Default stop is an open PR.** Automerge exists only as an explicit policy option.
- **Escalation stops only the agent concerned.** The rest of the fleet keeps going.
- **Single user, single machine first.** Remote and multi-user come after Phase 1 is fully usable.
- **Provider-neutral by design.** New fields, keys and formats are named neutrally (`agent_command`, not
  `claude_*`), even while Claude Code is the only runtime.
- Kept from v1: local-first and self-hosted; never read, store or reuse Claude login tokens outside the `claude`
  CLI; a board that shows the wrong status is a failure.

## Now: finish the foundation

In flight or next, before Phase 1 starts:

- Folder picker: done (4e80b25). Next, the app shell: drawer, views,
  Settings, footer messages and one logger (`docs/superpowers/specs/2026-10-24-app-shell-design.md`). The app
  shell is where new views land.
- The open v0.1.x bugs on GitHub (`bug` label) and the v0.1.x items in `TODO.md`.

## Phase 1: autopilot for one user on one machine

Done when Antonio can create a project in the morning, agree the goal with an orchestrator, leave, and come back
to an open PR or to a clear failure report, with every automatic decision in the audit log.

Roughly in dependency order; each step is useful on its own.

1. **Runtime boundary.** The minimal part of #25: launch, MCP attachment, instructions file, status source and
   input behind one `Runtime` interface, with Claude Code as the only implementation. Needed so specialists and
   orchestrators are not tied to one CLI.
2. **Specialist definitions.** A provider-neutral format: role prompt, allowed tools, scope, hard limits. The
   daemon compiles it into what the runtime needs (`CLAUDE.md` / `AGENTS.md`, generated settings, hooks) and
   enforces the limits through `PreToolUse` denials. Ships a built-in set (bug-fixer, test-writer, reviewer, …);
   user definitions sit next to it. Absorbs "task templates" and the edit guard outside workspaces from `TODO.md`.
3. **Skills.** A skill library specialists equip: reusable, provider-neutral instructions and helper scripts for
   one kind of work (bisecting a regression, writing a migration, …). Built-in skills plus user skills; a
   specialist lists the skills it gets, and the daemon installs only those into the agent's runtime (Claude Code
   skills directory, or the instructions file for runtimes without skills).
4. **Messaging.** Orchestrator ↔ agent messages through the daemon's MCP server, using held tool calls (up to
   about 40 min, `docs/superpowers/findings/mcp-spike.md`). Messages carry source agent and owner and are never
   treated as instructions from the user.
5. **Policy and audit.** Per-project policy that answers permission prompts and questions automatically, escalates
   the rest, and records every decision. Absorbs "per-project permission profiles" and "bulk approve" from
   `TODO.md`.
6. **Budgets and cost.** Token, time and loop budgets per project, with live usage. Builds on #24 (OpenTelemetry)
   and the transcript `usage` records. Absorbs "token and time cost per task" and "cap concurrent agents".
7. **Orchestrator.** Orchestrator types (presets of temperament and policy, chosen at project creation). Flow:
   brainstorming session with you → task breakdown → specialists launched and supervised → open PR, or automerge
   when the policy allows it → completion or failure report. The orchestrator is itself an agent on a runtime.
8. **Notifications.** Desktop notification or toast when the window is not focused; an outgoing webhook (ntfy,
   Slack, …) for when you are away. One-way only; answering remotely is Phase 2.
9. **Operation views.** Plan and task tree, audit log, budget burn. Absorbs the chat view, history and Review
   inbox from `TODO.md` where the operation needs them.
10. **Agent and skill builder.** Create and edit specialist definitions and skills in the app, drafted with the
    provider you select.

## Phase 2: remote

- Per-run secret and auth on the API before anything leaves loopback (`TODO.md`).
- Web UI back to parity with egui, then used for remote access.
- Take over an open session remotely: interactive terminal (bidirectional pty relay).
- Answer escalations remotely: the Phase 1 webhook becomes a round trip.

## Phase 3: multi-user

The team hub from `docs/concept.md`: one daemon per person, a hub that aggregates them into shared views, and
cross-owner messages that the receiving owner approves by default. Each person's agents work independently;
the hub detects and surfaces cross-user interference instead of letting agents act on other people's work. The
open questions in `docs/concept.md` (where agents run, visibility, identity) are answered when this starts.

## Horizon: optional, no timing

These happen eventually, in any order, when they are worth it.

- **Rename to fleet-manager**: crates, binaries, config and state paths, repo, docs. Comes with a new icon that
  replaces `packaging/fleet-board.svg`: a simple Phosphor-style line logo, the initials FM inside a cube, legible
  at small sizes and in both themes.
- **Other providers**: the rest of #25 (Codex, Gemini, OpenCode, Aider, … and plain API keys), a Providers
  settings tab, a runtime and model picker per agent.
- **Platforms**: macOS, Windows through WSL2 (`TODO.md`).
- **Release**: release binaries, systemd user service, CI on pull requests, contributing guide, open-source launch.
