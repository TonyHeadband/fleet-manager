# Management

Management is where the fleet is configured, one tab per area:

- **Overview**: every product at a glance, and where you pick the active product. See [Overview](#overview).
- **Products**: name, repos, and each repo's specialist roster. See
  [Products and releases](products-and-releases.md).
- **Personas**: create, re-login, rename, delete. See [Personas](personas.md).
- **Instructions**: one Markdown file given to every agent as its `CLAUDE.md`, the replacement for a global one.
- **Agent settings**: permission allow/deny rules and env merged into every agent's Claude settings.
- **Orchestrators**, **Specialists** and **Skills**: built-in and user definitions, edited as Markdown. See
  [Orchestrated tasks](orchestrated-tasks.md).
- **Workflow**: what a repo's `.fleet/workflow.yml` is. See [Workflow](#workflow).

The app keeps the last tab you opened while it runs.

## Overview

Overview is read-only: it shows each product as one row, so you can see the whole fleet without switching the active
product.

Each row has:

- The product's name. Click it to make that product the active product, the same as picking it in the drawer's
  product switcher. The active product's row says **active**.
- Its tasks counted by where they stand: **Working**, **Needs you** and **Review** count tasks whose agent is in
  that board column; **Merged** counts tasks that are merged or marked done by hand. A task whose agent is in none
  of those columns, such as a stopped one that was never merged, is not counted.
- Each release not yet released, with its version and the same two bars as the Releases view: tasks merged, and
  days used or overdue. See [Releases](products-and-releases.md#releases).

The **Product** filter above the rows shows one product or **All products** (the default).

## Workflow

Workflow explains the `.fleet/workflow.yml` file each repo keeps on its base branch, with a two-step example. The
tab does not edit the file: add it to the repo and push it. What the file holds and when it is read is in
[The workflow file](orchestrated-tasks.md#the-workflow-file).

## Where it is stored

The agent instructions and settings edited in Management live next to the config, in
`~/.config/fleet-manager/agent/` (`instructions.md`, `settings.json`); orchestrator, specialist and skill
definitions in `~/.config/fleet-manager/fleet/`. The active product picked on Overview is saved as
`active_product` in `~/.config/fleet-manager/gui.toml`.
