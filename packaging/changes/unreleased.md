## What's new since v0.2

- **Every specialist is the orchestrator's to use.** A new specialist reaches orchestrated tasks with nothing to set: there is no roster to pick on a repo or in New task anymore. To keep one away from a product, tick it under **Hidden specialists** in Management → Products.
- **Specialist nicknames**, just for fun: each persona can name its specialists in Management → Personas, and the board and breakdown show them as `Ferris (rust)`. Agents and logs keep the real names.
- **PR styles.** An orchestrator's pull request now follows a PR style (title pattern, sections, version note, release-notes line), picked per product in Management → Products or pinned by a repo's `.fleet/workflow.yml`; `open_pr` checks it before running the tests. The default style, `plain`, checks nothing; `minimal` asks for a Summary. A `.fleet/workflow.yml` with a `pr:` section is invalid to v0.2 daemons, so add it only after upgrading.
- **Short task titles.** A card shows a few words instead of the whole goal: type a title in New task, or leave it empty and the agent names the task as it starts. The side panel still shows the full goal.
- **Permission mode and effort per agent.** Every agent now starts with an explicit permission mode and effort, set per definition, per product and per breakdown step; a step can only make the mode stricter.
- **No more ghost tasks.** Restarting Fleet Manager no longer lists an orchestrated task's specialists and restarted orchestrators as tasks of their own, and removes the ones earlier restarts added.
- **One build folder per task.** A repo's `.fleet/workflow.yml` can set `env:` for all of a task's agents and its gate; `CARGO_TARGET_DIR: "{task}/target"` makes every specialist build into one folder instead of 12 to 26 GB each. The new `pr_gate_build` setting (`shared` or `clean`, also a product override) picks whether the gate reuses it, Management → Products shows each repo's env, and Archive removes the build folder. A `.fleet/workflow.yml` with `env:` is invalid to older daemons, so add it only after upgrading.
- **Task limits.** Limit how many orchestrated tasks a product, and how many products, have working at once.
- **Resumes say why they stop.** When Fleet Manager restarts and an agent's Claude session left no transcript, the agent now stops with "could not resume: nothing to resume: the Claude session left no transcript" instead of exiting with no reason, and an agent whose resume exits at once keeps its last screen for you to read.

Upgrading changes the database: a v0.2 build cannot open it afterwards.
