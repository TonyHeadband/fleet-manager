# Personas

One set of accounts per task. A persona bundles a Claude login, a GitHub login on one host (e.g. `github.com` or a
GitHub Enterprise host) and a commit identity. Each task belongs to one persona, chosen when it is created, and its
agent, clones and fetches use only that persona's logins: nothing comes from your own `~/.claude`, `gh` config or an
ambient token.

Repos belong to products, not personas; a repo's remote must be HTTPS on the host of the persona that uses it (SSH
is refused for now, #156). The logins stay inside the `claude` and `gh` CLIs, as plain files in the persona's own
folder (mode 0700 on Linux; an encrypted store is #161); Fleet Manager never reads or stores a token.

Switch the active persona in the drawer; filter the board by persona.

## Create and manage personas

The setup wizard creates the first persona: see [First run](../getting-started/first-run.md). After that,
Management → Personas creates more (**+ New persona**), and logs in again, renames or deletes one.

A persona whose login breaks later is flagged in the drawer and in Management, and spawning on it fails with
"Log in to … again"; nothing falls back to your own login. To give a persona a Claude plugin, run `claude` with
`CLAUDE_CONFIG_DIR` set to its `claude` folder.

## Specialist nicknames

A persona can give each specialist a nickname, just for fun: Management → Personas → the persona →
**Specialist nicknames**, one field per specialist that loads, then **Save nicknames**. The board and the breakdown
then show that persona's specialists as `Ferris (rust)` instead of `rust`. Agents, prompts, briefs and the log keep
the real names, and a specialist with no nickname shows its name.

A nickname is one line of at most 32 characters; a blank one is removed. They are kept in the persona's folder, in
`nicknames.json` beside its `claude` and `gh` folders, so each persona has its own.
