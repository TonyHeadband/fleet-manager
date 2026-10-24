# v1 acceptance checklist

Run against the real `claude`, from a normal terminal (not from inside another Claude Code session). Tick every box; record any card that showed a wrong status as a bug.

## Prerequisites

- tmux 3.2 or newer on `PATH` (`tmux -V`), and `claude` logged in.
- `~/.config/fleet-manager/config.toml` with two small throwaway repos. To see Needs you cards the agents must run in manual permission mode, because the daemon never forces a mode:

  ```toml
  claude_command = ["claude", "--permission-mode", "default"]
  [[repos]]
  name = "core"
  path = "~/scratch/core"
  [[repos]]
  name = "portal"
  path = "~/scratch/portal"
  ```
- Trust the worktrees root once: `mkdir -p ~/.local/share/fleet-manager/worktrees && cd ~/.local/share/fleet-manager/worktrees && claude`, accept "trust this folder", `/exit`. Every project directory below it inherits that trust; a git worktree used directly would not.
- Start the daemon: `fleet-daemon-<arch>-linux` (or `./build.sh`, then `dist/`). Open the printed URL.

## Checklist

- [ ] The page loads, shows the five columns (Queued, Working, Needs you, Talking, Review) and a live indicator.
- [ ] Spawn an agent in `core` with a prompt that edits a file and runs a Bash command. After spawning, `ls <state_dir>/worktrees/<project-dir>` shows exactly the starting repo (`core`), and the card shows one `core` badge.
- [ ] The card goes Queued, then Working, with no "waiting at startup prompt" (trust inherited).
- [ ] At the Bash permission prompt the card moves to Needs you with a `permission` badge and the tool name in its activity.
- [ ] Answer `1` from the card: it returns to Working, then Review when the agent finishes, with the `Stop` activity "finished".
- [ ] In Review the card's changes indicator is set (the agent edited a file), and a second spawn in `portal` that only answers a question shows no changes.
- [ ] The repo filter: selecting `portal` shows only the portal agent; selecting both shows both; clearing shows all.
- [ ] Clicking a card opens a read-only terminal preview that follows the agent; `tmux -L fleet-manager attach -t <session>` shows the same session. The Diff tab shows each repo's changes (committed, uncommitted and untracked files) and Refresh reloads it.
- [ ] Pause a Working agent: the card shows Paused and the agent stops; Resume continues it. A Needs you card has no Pause button (Escape would deny the pending request).
- [ ] Kill an agent (Kill, then Confirm Kill): the card disappears, its tmux session, project directory, worktree and branch are gone (`git -C ~/scratch/core branch --list 'fleet/*'`).
- [ ] Restart the daemon while two agents run (one Working, one Needs you): both cards come back in their columns with the activity "status unverified since daemon restart", and the next hook event replaces it with the real activity. A Review card is left alone.
- [ ] Close the browser tab and reopen it: the board shows the current state (snapshot) without a reload loop.

## Known limits in v1

- **Repo-level Claude settings do not apply.** Claude starts in the project directory, one level above each repo, and loads `.claude/settings.json`, `.claude/settings.local.json` and `.mcp.json` only from there. Measured: a repo's `deny` rule for `Bash(touch:*)` was ignored and the permission prompt appeared. Your user and managed settings still apply. Do not rely on a repo's own deny rules for agents started here (#62).
- **An agent can reach the daemon's API.** Anything running in an agent's tmux session can call `http://127.0.0.1:<port>` (answer, kill, spawn). In manual permission mode it needs a Bash approval to do so, but an allow rule for `curl` removes that. A per-run secret is planned (#63).
- **Hook events sent while the daemon is down are lost.** After a restart a Working or Needs you card says "status unverified since daemon restart" until its next event.
- The prompt is an argument of the `claude` process, so other users on the same host can see it in `ps`.

- A repo is added to a project only at spawn; adding repos mid-session is v1.x (since shipped: Add repo in the side panel and the agent's `request_repo`).
- The login page is not built; use the `claude` login already in `~/.claude`.
- No authentication: the daemon refuses any listen address that is not loopback.
