# The board

The board (the Kanban view) shows every agent of the active product as a card. It is driven by Claude Code hooks,
not by scraping the terminal: each agent's hooks post to the daemon, and the card moves when they do.

![The board, with cards in each column](../assets/board.png)

## Columns

| Column | What the agent is doing |
|---|---|
| Stopped | Not running any more. The card says why: stopped by you, exited, crashed, or finished its step. |
| Queued | Started, and Claude Code has not reported in yet. |
| Working | Thinking or running a tool. |
| Needs you | Waiting on you. |
| Review | Finished its turn; read what it did. |

How a card moves between them:

```mermaid
stateDiagram-v2
    state "Needs you" as NeedsYou
    [*] --> Queued: New task
    Queued --> Working: session starts
    Queued --> NeedsYou: silent at startup
    Working --> NeedsYou: permission, question, error, repo access, breakdown
    NeedsYou --> Working: answered
    Working --> Review: turn finished
    Review --> Working: new prompt
    Queued --> Stopped
    Working --> Stopped
    NeedsYou --> Stopped
    Review --> Stopped: Stop, exit, crash, step finished
```

## Why a card needs you

A card in Needs you says why:

- **Permission needed**: Claude Code asks before running a tool.
- **Question**: the agent asks you something.
- **Error**: the turn stopped with an error.
- **Startup prompt**: no hook arrived within the startup grace period, which usually means Claude's "trust this
  folder" dialog is up.
- **Repo access**: the agent asked for another repo; grant or refuse it from the card.
- **Breakdown to review**: an orchestrator proposed steps; see [Orchestrated tasks](orchestrated-tasks.md).

## Act from the card

Answer, pause, resume, stop. Answer is offered on Needs you (except a repo-access request, which is granted or
refused); Pause on Working and Review; Stop on any card that is still running.

The side panel shows the agent's live terminal (read-only) and the diff of every repo in the task, plus the
breakdown for an orchestrated task.

The board can be filtered by repo, by persona and by release.

## Agents outlive the app

They run as the interactive `claude` CLI in tmux. Close the window, keep the daemon running, come back later; or
take over directly with:

```bash
tmux -L fleet-manager attach -t <session>
```
