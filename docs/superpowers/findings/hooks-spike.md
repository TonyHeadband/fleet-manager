# Phase 0 spike: hook-driven status on the real CLI

Run on 2026-10-23 with Claude Code 2.1.309 (Enterprise seat, managed settings active), tmux 3.8, in a throwaway git worktree. Payloads are in `fixtures/hooks/` (paths and user names sanitized).

## Answers

**(a) Do hooks fire in an interactive tmux session with `--settings`?** Yes. Observed: `SessionStart` (`source: startup`), `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `Notification` (`permission_prompt`), `PostToolUse`, `PostToolUseFailure`, `Stop`, `SessionEnd` (`reason: prompt_input_exit`). Not observed, because they could not be triggered on demand: `StopFailure`, `Notification` with `idle_prompt` (about 60 s of idleness), `elicitation_dialog`, `agent_needs_input`. Their fixtures must be hand-written from the docs and marked synthetic.

**(b) Do http hooks reach loopback?** Yes, for every event except `SessionStart` and `SessionEnd`, which the docs restrict to `command` and `mcp_tool` handlers. Those two use a `command` hook that pipes stdin to the same endpoint with `curl`. An http hook's `headers` interpolate environment variables listed in `allowedEnvVars`, so a per-agent secret travels in an `X-Fleet-Token` header read from `$FLEET_TOKEN`, set on the tmux session. It does not need to be in the URL.

**(c) First run in a new worktree.** Claude shows a "trust this folder" dialog, and no hook fires before it is accepted. Trust is inherited from a parent directory: a child of an already-trusted directory starts with no dialog. So the daemon creates all worktrees under one root (`<state_dir>/worktrees`) that the user trusts once by running `claude` there. If that was not done, `SessionStart` never arrives; the 20-second no-`SessionStart` rule from Task 9 flags the card as Needs you. Claude Squad does this by scraping the pane for `"Do you trust the files in this folder?"`, which this CLI version no longer prints (it now says `"Is this a project you created or one you trust?"`), a concrete example of why pane scraping breaks.

**(d) Event order** for prompt, tool, permission, approve, stop:

`SessionStart` → `UserPromptSubmit` → `PreToolUse` → `PermissionRequest` → `Notification(permission_prompt)` about 6 s later → `PostToolUse` or `PostToolUseFailure` → `Stop` → `SessionEnd`.

`PreToolUse` fires before `PermissionRequest`, so the reducer must let `PermissionRequest` override `Working`, and `PostToolUse` or `PostToolUseFailure` clear it. A killed session emits a late `SessionEnd` (`reason: other`) that can arrive before the next session's `SessionStart`.

## Other findings

- The user's own settings default to `auto` permission mode, which raises no permission prompts, so no Needs you events appear. The daemon must not force a permission mode; to see Needs you the agent must run in `default` (manual) mode.
- Managed settings (company deny rules and a banner) coexist with `--settings` hooks; `allowManagedHooksOnly` is evidently not set.
- A dimmed "prompt suggestion" can sit in the input line; it is not typed text and must never be submitted.
- Claude Squad has no hooks at all. It hashes the pane output to detect activity and string-matches `"No, and tell Claude what to do differently"` for approval prompts.

## Amendment A1, Step 0 (2026-10-23): project directory above a worktree

Run on the same CLI (2.1.309): a plain directory `<root>/projx/` under the trusted worktrees root, with a git worktree `projx/core` inside it, `claude` started with cwd `projx`.

- **(a) Trust.** No trust dialog. Trust inherited from the trusted root holds for a plain project directory. It does **not** hold for a git worktree used directly as cwd: a worktree is its own git root, so the first real-CLI acceptance run (cwd = `<root>/<repo>/<slug>`) left both agents stuck at the trust dialog with no hooks. The project directory fixes exactly this.
- **(b) CLAUDE.md.** A `CLAUDE.md` inside `projx/core/` is **not** loaded at startup (asked for its magic word, the agent said none). With a daemon-written `projx/CLAUDE.md` containing `@core/CLAUDE.md`, the agent answered the magic word at startup. So the daemon writes that import file, one `@<repo>/CLAUDE.md` line per workspace that has a `CLAUDE.md`.
- **(c) Hooks.** `SessionStart`, `UserPromptSubmit` and `Stop` all fire, with `cwd` = the project directory.
