# Management

Management is where the fleet is configured, one tab per area:

- **Overview**: every product at a glance, and where you pick the active product. See [Overview](#overview).
- **Products**: name, repos, hidden specialists, and the product's overrides. See
  [Products and releases](products-and-releases.md) and [Overrides](#overrides).
- **Personas**: create, re-login, rename, delete. See [Personas](personas.md).
- **Instructions**: one Markdown file given to every agent as its `CLAUDE.md`, the replacement for a global one.
- **Agent settings**: permission allow/deny rules and env merged into every agent's Claude settings.
- **Orchestrators**, **Specialists** and **Skills**: built-in and user definitions, edited as Markdown. See
  [Orchestrated tasks](orchestrated-tasks.md).
- **PR styles**: the rules a pull request follows, built-in and yours, edited like the definitions above. See
  [Pull request rules](orchestrated-tasks.md#pull-request-rules).
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

## Overrides

A product can set a few settings for itself. Everything it does not set follows the global value. The **Overrides**
section on the Products tab lists the product's overrides, one row each: section, key, value.

| Section | Key | Value |
| --- | --- | --- |
| `models` | An orchestrator or specialist name | A model: a name such as `sonnet` or `opus`, or a full model ID such as `claude-sonnet-5-5` |
| `limits` | `max_breakdown_versions`, `max_steps`, `max_parallel_specialists` or `max_pr_attempts` | A whole number above zero |
| `pr` | `style`, `attribution`, `draft` or `labels` | `style`: a PR style that loads. `attribution`: one line. `draft`: `true` or `false`. `labels`: comma-separated |

Pick the section and the key from the dropdowns and type the value. The key list for `models` is the orchestrators
and specialists that load. Beside each row the app shows the value the override replaces and where that value comes
from: `product`, `global`, `definition` or `default`.

**Save overrides** sends the whole list and the daemon replaces what it stores. The daemon checks every row. If it
refuses one, the app shows its message and keeps your rows, so you can fix them and save again. A stored row that
no longer applies, because its definition was deleted, is shown with the reason. Remove it and save.

New agents use a changed override. Agents already running keep the model they started with.

## Internal resources

A product's detail page in **Products** has an **Internal repo URL** field and a read-only **Internal resources**
section: the product's private notes repo, where orchestrated tasks keep their designs and plans.

Leave the URL empty for a local-only repo the app keeps. To use your own repo, enter its HTTPS URL, such as
`https://github.com/org/notes.git`, and choose a default persona: the app clones it with that persona's GitHub login,
and tasks push their notes to it. The app refuses to save when:

- the product has no default persona (`an internal repo URL needs a default persona`);
- the URL carries credentials, is SSH or plain HTTP, or is on a different host from the default persona's. The host
  check also runs when you change the default persona while a URL is set;
- a task that is not archived still has its `task/<task-name>` branch in the notes repo (`tasks still have notes in
  the internal repo: <task names>`). Archive those tasks first;
- the clone fails, or the remote has branches but no `main`. The message from git is shown, and nothing changes.

Clearing the URL stops pushing; the repo and its history stay, local-only. The same task check applies.

In **Internal resources**, pick a branch (`main` holds what past pull requests merged; each task has a
`task/<task-name>` branch), then a file to see its text and its last commit. **Refresh**, or closing and reopening
the section, fetches it again and keeps the branch and file you had if they still exist. See
[Internal resources](orchestrated-tasks.md#internal-resources).

## Models

An orchestrator or specialist definition can name a model in its front matter:

```yaml
model: sonnet
```

An agent runs on the first of these that is set:

1. The product's `models` override for its definition.
2. The definition's own `model:`.
3. The persona's default, which is Claude's own choice.

The side panel of a fleet agent shows the result as **Model: sonnet**, or **Model: persona default** when nothing
sets one. A solo agent has no model line.

The built-in definitions carry a `model:`. A user file with a built-in's name replaces the built-in as a whole, so a
copy you made earlier does not get a `model:` the built-in has since gained. Add the line to your copy, or use
**Reset to built-in** to drop it. See [Orchestrated tasks](orchestrated-tasks.md#definitions).

## Workflow

Workflow explains the `.fleet/workflow.yml` file each repo keeps on its base branch, with a two-step example. The
tab does not edit the file: add it to the repo and push it. What the file holds and when it is read is in
[The workflow file](orchestrated-tasks.md#the-workflow-file).

## Where it is stored

The agent instructions and settings edited in Management live in the config folder
(`~/.config/fleet-manager` on Linux, `%USERPROFILE%\AppData\Roaming\fleet-manager` on Windows; see
[Where your data lives](../getting-started/install.md#where-your-data-lives)): `agent/` holds `instructions.md`
and `settings.json`, and `fleet/` the orchestrator, specialist, skill and PR style definitions. The active product picked on
Overview is saved as `active_product` in `gui.toml`, in the same folder.
