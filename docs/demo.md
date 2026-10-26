# Run the demo

The README's board image and animation (`docs/assets/board.png`, `docs/assets/demo.gif`, `docs/assets/demo.webm`)
are recorded from the real daemon and GUI, driven by a scripted stand-in for `claude`. Nothing in them is mocked
up by hand, and no real agent, GitHub account or network is involved.

## Record

```bash
scripts/demo.sh
```

It needs, all from distro packages plus the Rust toolchain: `Xvfb`, `ffmpeg` (with libx264 and libvpx), `tmux`
3.2 or newer, `git`, `xwininfo`, Mesa software GL, `cargo` and `sha256sum`. It names whatever is missing and stops.

The script builds the daemon, the GUI and the demo binaries (release), then runs everything in a throwaway world: a
temp `HOME` under `/tmp` with its own `config.toml` and `gui.toml` (dark theme), its own tmux socket, the first
free port from 7990 and the first free X display from `:90`. Your own daemon, tmux server and settings are never
read or touched. It records the window with `ffmpeg`, and on exit (also on failure or Ctrl-C) it stops only the
processes it started and removes the temp folder.

It writes:

- `docs/assets/demo.webm` and `docs/assets/demo.gif` (the GIF is kept under 5 MB),
- `docs/assets/board.png`, the frame at `still_at`,
- `demo/demo.lock`, the fingerprint of the inputs the media was recorded from.

Regenerating is a deliberate change: look at the media, then commit `docs/assets/` and `demo/demo.lock` together.
Recordings are never byte-identical, so commit them only when something the demo shows has changed.

## What drives it

- `demo/scenarios/`: one file per project. The demo agent (`fleet-demo-agent`, set as the temp daemon's
  `claude_command`) picks the scenario whose `task` matches its prompt and plays it: transcript lines, tool calls,
  questions and permission prompts that wait for an answer, and files it writes and commits (from
  `demo/scenarios/content/`).
- `demo/timeline.toml`: the shot. The persona, when each project is started, when each answer and follow-up is
  sent, when the side panel opens on which tab and when it closes, the clip `length` and the `still_at` moment for
  the PNG. `fleet-demo-director` plays it over the daemon's HTTP API, the same calls a click on a card makes.
- `demo/bin/gh`: a stand-in `gh` that reports logged in.

The GUI is steered only through a cue file (the `FLEET_GUI_CUES` environment variable, set by the script): the
director appends `open <agent-id> terminal|diff` or `close`, and the GUI acts on it as it would on a click. There is
no synthetic input. Tune the shot in the timeline and scenarios; the gate's `demo` step (`cargo test -p fleet-demo`)
checks the pacing.

## The CI check

```bash
scripts/demo.sh --check
```

It records nothing and needs only `git` and `sha256sum`. It hashes the tracked inputs that shape the media (listed in
`DEMO_INPUTS` in the script: the GUI and the crates it renders through, the daemon's status reducer, the hook
fixtures, the demo crate, the scenarios, timeline and `gh` stand-in, `Cargo.lock` and the script itself) and compares
the result with `demo/demo.lock`. When they differ the media is stale: run `scripts/demo.sh` and commit the result.
CI runs the check when one of those paths changes; it never records or commits media itself.
