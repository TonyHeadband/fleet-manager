# Management

Management is where the fleet is configured:

- **Products**: name, repos, and each repo's specialist roster. See
  [Products and releases](products-and-releases.md).
- **Personas**: create, re-login, rename, delete. See [Personas](personas.md).
- **Instructions**: one Markdown file given to every agent as its `CLAUDE.md`, the replacement for a global one.
- **Agent settings**: permission allow/deny rules and env merged into every agent's Claude settings.
- **Orchestrators**, **Specialists** and **Skills**: built-in and user definitions, edited as Markdown. See
  [Orchestrated tasks](orchestrated-tasks.md).

## Where it is stored

The agent instructions and settings edited in Management live next to the config, in
`~/.config/fleet-manager/agent/` (`instructions.md`, `settings.json`); orchestrator, specialist and skill
definitions in `~/.config/fleet-manager/fleet/`.
