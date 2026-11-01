# Demo stand-ins

`crates/fleet-demo` and `demo/` are scripted stand-ins for `claude`, `gh` and GitHub. They put the real daemon and the
real app in front of agents that behave the same way every run, with no real agent, GitHub account or network
involved. Three things use them:

- the Tauri app's smoke test (`scripts/tauri-smoke.sh`, run by CI),
- `scripts/ui-compare.sh`, the egui app and the web UI side by side on one throwaway daemon,
- the demo media in the README and these docs. It is recorded from these stand-ins in the private
  fleet-manager-marketing repo, which pins this repo as a submodule; the files in `docs/assets/` are copied from there.

The gate's `demo` step (`cargo test -p fleet-demo`) tests them, so a daemon change that breaks them fails here, not
at the next recording.

## What drives it

- `demo/scenarios/`: one file per task. The demo agent (`fleet-demo-agent`, set as the temp daemon's
  `claude_command`) picks the scenario whose `task` matches its prompt and plays it: transcript lines, tool calls,
  questions and permission prompts that wait for an answer, and files it writes and commits (from
  `demo/scenarios/content/`).
- `demo/timeline.toml`: the shot. The persona, when each task is started, when each answer and follow-up is
  sent, when the side panel opens on which tab and when it closes, the clip `length` and the `still_at` moment for
  the PNG. `fleet-demo-director` plays it over the daemon's HTTP API, the same calls a click on a card makes. Before
  the GUI starts it also creates the product `pie` with the demo's bakery repo, so a recording can open on that
  product's board.
- `demo/bin/gh`: a stand-in `gh` that reports logged in.
- `fleet-demo-forge`: a stand-in for GitHub's REST API, reached through `FLEET_FORGE_API`, that answers the persona's
  login check.

The GUI is steered only through a cue file (the `FLEET_GUI_CUES` environment variable): the director appends
`open <agent-id> terminal|diff` or `close`, and the GUI acts on it as it would on a click. There is no synthetic
input. The gate's `demo` step also checks the timeline's pacing.
