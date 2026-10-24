# Projects first: no global repo list — design

Date: 2026-10-24. Scope: `fleet-daemon`, `fleet-api`, `fleet-client`, `fleet-gui`. The web UI stays frozen.

## Why

v0.1 hangs everything on a fixed `[[repos]]` list in `config.toml`, and on a fresh machine the app stops on "could not start its daemon" because that file does not exist. Antonio's development process is different. The unit is a **project**: a goal, big or small. Starting one kicks off an agent. The agent may need no repo at all, start a new repo, or need access to existing repos, which it asks for or Antonio adds. There is no "root repo" to configure up front.

## Goals

- The app launches with no config at all.
  - The first launch asks one question: where projects live. The default is `~/fleet-projects`.
  - After that it lands on an empty board.
- A project is a folder under the projects root, one subfolder per project, named from the goal's slug. Every agent has one.
- A project can start with zero repos. The agent then works in an empty project folder and may `git init` a new repo in a subfolder.
- Repos are attached to a project, never configured globally. There are three ways:
  1. picked in the New project dialog;
  2. added by hand later from the side panel;
  3. requested by the agent through an MCP tool and granted by Antonio on a Needs-you card.
- A repo is identified by a **path** to an existing clone, a **git URL**, or the **name** of a remembered repo.
- Each repo has one base clone:
  - for a path, the user's own clone;
  - for a URL, one clone that fleet-board makes in its own store.
- Every project gets a **worktree** of the base, on its own branch, never a copy.
- Every repo ever attached is **remembered**, so the next attach is a pick, not typing.
- The agent can never silently get a repo. A request waits for Antonio's yes, and he can edit the target before granting.

## Non-goals

- A settings page. That comes later; changing the root then affects only new projects.
- The edit guard that blocks writes outside the project folder (`TODO.md`, v1.x). Not part of this work.
- Removing remembered repos or pruning unused URL clones from the UI. A manual follow-up.
- Web UI support for any of this.
- Team mode, or several owners.

## Anti-goals

- **Destroying work on Kill.** A repo the agent created with `git init` inside the project folder has no other copy. Kill must never delete it.
- **Cloning a repo twice.** One base clone per remembered repo.
- **A request that hangs the agent with no visible card.** A pending repo request must always show as a Needs-you card with its reason.
- **Fetching anything at runtime that the user didn't ask for.** Only a URL Antonio typed or granted is cloned, with his normal git credentials. Air-gapped servers work like a terminal `git clone`.

## Decided

### Config and first launch
- `config.toml` becomes optional. A missing file means all defaults.
- A new key, `projects_root`, defaults to `~/fleet-projects`. `[[repos]]` is still read when present, only to seed the remembered list once (see Storage), and is otherwise ignored. A v0.1 config keeps loading.
- Both the daemon and `fleet-client`'s `ClientConfig` load a missing file as defaults.
- The GUI checks at startup whether `config.toml` exists. If it does not, it shows a **first-run screen** before starting the daemon:
  - "Where should projects live?", prefilled with `~/fleet-projects`;
  - a Continue button.
- On Continue the GUI creates the folder, writes `config.toml` with `projects_root = "<path>"` plus the commented template, and starts the daemon as today.

### Projects
- The project folder is `<projects_root>/<slug>`, using the first free `<slug>`, `<slug>-2`, …. It replaces today's `<state_dir>/worktrees/<slug>`.
  - Agents already running keep their old `workdir`; nothing migrates.
- The project folder holds:
  - one subfolder per attached repo (a worktree);
  - any repo the agent inits;
  - the daemon's `CLAUDE.md`.
- **The project's `CLAUDE.md` is regenerated whenever its repos change.** It lists each repo subfolder and its branch, and imports each repo's own `CLAUDE.md`. It tells the agent:
  - to start a new repo, make a subfolder and `git init` it;
  - to work in an existing repo it does not have, call the `request_repo` tool. Never clone or copy repos itself.
- **Claude's "trust this folder":** trust is inherited from a trusted parent, so trusting `projects_root` once covers every project.
  - The first-run screen shows the one-time command (`cd <root> && claude`, choose "Yes, I trust this folder", `/exit`) with a copy button.
  - Claude's dialog pre-selects "No, exit" and shows no numbers: the trust choice is the second option (Down, then Enter). Enter alone, or `1`, quits Claude (measured in the real run, `docs/superpowers/findings/projects-first.md` (g)).
  - If it is skipped, the existing startup-prompt detection turns the first card into Needs you "waiting at startup prompt". Antonio picks the second option, "Yes, I trust this folder" (Down, then Enter), in the agent's terminal (`tmux -L fleet-board attach -t <session>`), not with `1` in the reply box.

### Repos
- A source string is resolved in this order:
  1. the name of a remembered repo;
  2. a path (starts with `/` or `~`, or is an existing directory): must be a git repo;
  3. a URL (contains `://`, or matches `user@host:path`).
  Anything else is an error that says which forms are accepted.
- **Repo name:** the remembered name is the directory name (path) or the last URL segment without `.git`, made safe with the existing dir-name rule. A clash with a different source gets `-2`, `-3`, ….
- **URL base clone:** `<state_dir>/repos/<name>`, cloned with plain `git clone`, so the user's SSH keys and credential helpers apply. A clone failure is reported verbatim and nothing is remembered.
- **Attaching a repo to a project:**
  1. `git fetch` on the base. A URL base always fetches. A path base fetches only if it has an `origin` remote. A failed fetch is a warning, not an error.
  2. Create the worktree at `<project>/<name>` on branch `<branch_prefix>/<project-slug>`, starting from the remote default branch (`origin/HEAD`) when it exists, else the base's `HEAD`.
  3. Record a `Workspace`.
  4. Regenerate `CLAUDE.md`.
  5. If the agent is running, tell it the repo name and path by typing a short line into its session, the same way answers are sent.
- **Attaching a repo the project already has** does nothing and returns the existing workspace.
- **Agent-initialised repos:**
  - On each reconcile, a direct subfolder of a project folder that is a real git repo (a `.git` directory, not a worktree's `.git` file) and is not yet a workspace gets registered:
    - remembered as a path repo;
    - added as a `Workspace` with its current branch and `base` = git's empty-tree hash, so its diff shows everything;
    - marked `owned: true`.
  - It then shows on the card and in Diff like any other repo.

### The agent asking: MCP
- The daemon serves a minimal MCP server over HTTP at `/internal/mcp/{agent_id}`, authenticated with the agent's existing `X-Fleet-Token`. It is JSON-RPC: `initialize`, `tools/list`, `tools/call`.
- Each agent is started with `--mcp-config <agent_dir>/mcp.json` pointing at it. The file is mode 0600 and holds the token.
- Tools:
  - `list_repos()` returns the remembered repos (name and source) and the ones this project has (name and path).
  - `request_repo(repo, why)` takes `repo` as a name, path or URL.
    - If the project already has it, the tool returns at once with its path.
    - Otherwise it records a pending request on the agent (`repo_request: {repo, why}`, status Needs you, blocked reason `repo_access`) and **holds the tool call open** until the request is decided.
    - When granted, it returns the worktree path; when refused, it returns the refusal and its reason. It also returns an error if the tool call outlives the timeout measured in Task 1.
    - Only one pending request per agent. A second call while one is open returns an error.
- **Task 1 is a spike on the real CLI** before anything is built on this:
  - `--mcp-config` with an `http` server and a header;
  - how long a tool call may block before Claude Code gives up.
  - If blocking is not viable, the fallback is: `request_repo` returns "requested, wait" immediately, and the grant is delivered by typing into the session.

### Granting and refusing
- New endpoints:
  - `POST /api/agents/{id}/repos` `{actor, repo}`: add a repo by hand.
  - `POST /api/agents/{id}/repo-request/grant` `{actor, repo}`: `repo` may be edited from what was asked.
  - `POST /api/agents/{id}/repo-request/refuse` `{actor, reason}`.
- Grant attaches the repo exactly as a manual add does, then answers the held tool call.
- Refuse answers the held call with the reason.
- Both clear `repo_request`, after which hooks move the status on as usual.

### Kill
- Remove worktrees from their bases, as today; the branches survive.
- Delete the daemon's files (`CLAUDE.md`, the agent dir).
- Delete the project folder **only if nothing else is left in it**.
- If an owned (agent-initialised) repo or any other file remains, keep the folder and report "kept <path>: it contains <repo/files>" in the response, which the GUI shows in the problem line.

### Storage (SQLite, in the existing `fleet.db`)
- New table `repos`: `name` (primary key), `source` (`path` or `url`), `location` (the user's path or the URL), `base` (the clone path used for worktrees), `added_at`.
- Workspaces stay on the `Agent` JSON as today. `Workspace` gains `owned: bool` (default false).
- `Agent` gains `repo_request: Option<RepoRequest { repo, why }>`. `BlockedReason` gains `RepoAccess`.
- On first open after upgrade, rows are inserted for a v0.1 config's `[[repos]]` that are not already remembered.

### API and schema changes (`fleet-api`)
- `SpawnRequest`: `repo: String` becomes `repos: Vec<String>` (may be empty; each entry is a name, path or URL). The `prompt` is unchanged.
- `RepoList` becomes `{ repos: Vec<RepoInfo { name, source, location }> }`.
- New request types:
  - `AddRepoRequest {actor, repo}`;
  - `GrantRepoRequest {actor, repo}`;
  - `RefuseRepoRequest {actor, reason}`.
- UiSchema:
  - a `Condition::BlockedReasonIn`;
  - the Answer action hidden while the blocked reason is `repo_access`;
  - the badge label map gets "Repo access".

### GUI
- **First-run screen,** described above.
- **New project dialog** (the New agent dialog, renamed):
  - Goal: the prompt.
  - Repos: a multi-select of remembered repos, plus a text field "path or git URL" with Add that appends chips. Optional.
  - Start project: Ctrl+Enter, as now.
- **Side panel:**
  - **Repos** row under the title: one chip per workspace (name, branch on hover), plus an "Add repo" control with the same picker.
  - **When `repo_request` is set,** the reply box is replaced by a **Repo request** box:
    - "Wants `<repo>`: `<why>`";
    - an editable field prefilled with the requested repo;
    - Grant (primary) and Refuse (quiet danger, with an optional reason).
- The card shows the `Repo access` badge like the other blocked reasons. The remembered list comes from `GET /api/repos`.

## Left to the builder

- The exact MCP JSON-RPC shapes beyond the three methods, and the tool descriptions' wording.
- How the held tool call waits (a `tokio::sync::oneshot` per agent in the service is the obvious one).
- Chip and picker visuals, using the existing primitives and tokens.
- The reconcile cadence for detecting agent-initialised repos (the existing 5 s tick is fine).
