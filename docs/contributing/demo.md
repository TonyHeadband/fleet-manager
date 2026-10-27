# Demo media

The board image and animation in the README and these docs (`docs/assets/board.png`, `docs/assets/demo.gif`, `docs/assets/demo.webm`)
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

- `docs/assets/demo.webm` (the docs site's hero) and `docs/assets/demo.gif` (the README's, which GitHub cannot play as
  video). The GIF is encoded at full width and 15 fps, and steps down only to stay under 5 MB,
- `docs/assets/board.png`, the frame at `still_at`,
- `demo/demo.lock`, the fingerprint of the inputs the media was recorded from.

Regenerating is a deliberate change: look at the media, then commit `docs/assets/` and `demo/demo.lock` together.
Recordings are never byte-identical, so commit them only when something the demo shows has changed.

## What drives it

- `demo/shots/<name>/`: one folder per shot (`pie` is the one recorded; `scripts/demo.sh --shot <dir>` picks another).
  Everything the shot needs is in it:
  - `shot.toml`: the timeline. The product, the repos (`[[repo]]`: name, the seed commit's message and optional
    author, `origin`, `roster`), the persona, when each task is started, when each answer and follow-up is sent,
    when the side panel opens on which tab and when it closes, the clip `length` and the `still_at` moment for the
    PNG. `fleet-demo-director` plays it over the daemon's HTTP API, the same calls a click on a card makes.
  - `seed/<repo>/`: the files each repo starts with. Before the GUI starts, the director copies them into the temp
    world, commits them (`git init`, one commit), creates the persona and the product with those repos, and the
    script names the product as `active_product` in the temp `gui.toml`, so the recording opens on its board.
  - `lock-inputs`: tracked paths outside the folder that the shot's media also depends on, one per line (empty
    when none). Required for an official shot.
  - `scenarios/`: one file per task. The demo agent (`fleet-demo-agent`, set as the temp daemon's `claude_command`
    with `--shot <dir>`) picks the scenario whose `task` matches its prompt and plays it: transcript lines, tool
    calls, questions and permission prompts that wait for an answer, and files it writes and commits (from
    `content/`, relative to the shot folder).
- `demo/bin/gh`: a stand-in `gh` that reports logged in.

The GUI is steered only through a cue file (the `FLEET_GUI_CUES` environment variable, set by the script): the
director appends `open <agent-id> terminal|diff` or `close`, and the GUI acts on it as it would on a click. There is
no synthetic input. Tune the shot in its `shot.toml` and scenarios; the gate's `demo` step (`cargo test -p fleet-demo`)
checks the pacing.

## The CI check

```bash
scripts/demo.sh --check
```

It records nothing and needs only `git` and `sha256sum`. It hashes the tracked inputs that shape the media (listed in
`COMMON_INPUTS` in the script: the GUI and the crates it renders through, the daemon's status reducer, the hook
fixtures, the demo crate, the `gh` stand-in, `Cargo.lock` and the script itself) plus, per shot, its own folder and
the paths listed in its `lock-inputs` file, and compares each result with its line of `demo/demo.lock`
(`<name> sha256:<hex>`). When the pie differs the media is stale: run `scripts/demo.sh` and commit the result. Any
other shot that differs only prints `warning: demo shot <name> is stale, ...` and the check still passes.
CI runs the check when one of those paths changes; it never records or commits media itself.

## Recording any shot

```bash
scripts/demo.sh [--shot DIR] [--out DIR] [--size WxH] [--length D] [--still-at D] [--formats LIST]
scripts/demo.sh --all | --check | --lock [--shot DIR]
```

A checkout outside this repo can call the script with its own shot folder. Relative `--shot` and `--out` paths are
relative to the caller's directory. Everything is built from this repo's sources, in a throwaway world, as above.

- `--shot DIR`: the shot folder (default `demo/shots/pie`).
- `--out DIR`: where the files go. Required for a shot outside this repo's `demo/shots/`; for one inside it, the
  default is the shot's `[output] dir`, relative to the repo root.
- `--size WxH`: the window and capture size. Default: the shot's `size`, else `1400x800`.
- `--length D`, `--still-at D`: the clip length and the still's moment (`25s`, `1500ms`), replacing the shot's.
  The pacing rules are not checked again; they are the shot author's to keep.
- `--formats LIST`: comma-separated subset of `webm gif png mp4`. Default: the shot's `[output] formats`, else
  `mp4,png`. `mp4` is the trimmed clip as recorded; `gif` steps down in size to stay under 5 MB.
- `--all`: every shot directly under `demo/shots/`, sorted by name, each to its own `[output] dir`.
- `--check`, `--lock`: the lock described above. `--lock` alone rewrites every line; with `--shot` only that shot's.

A shot is **official** when its folder is directly under this repo's `demo/shots/`. Only official shots read or
write `demo/demo.lock`, and a recording rewrites its line only when none of `--out`, `--size`, `--length`,
`--still-at` or `--formats` was given: those make a scratch copy.

A shot folder holds `shot.toml`, `seed/`, `scenarios/` and `content/` as above, plus an optional `lock-inputs`.
`shot.toml` takes, besides the timeline:

```toml
size = "1400x800"                  # default

[output]                           # optional
dir = "docs/assets"                # relative to the repo root; used by an official shot without --out
stem = "demo"                      # the video's file stem; default: the folder's name
still = "board"                    # the PNG's file stem; default: stem
formats = ["webm", "gif", "png"]   # default: ["mp4", "png"]
```

The files are `<out>/<stem>.<webm|gif|mp4>` and `<out>/<still>.png`. The pie writes
`docs/assets/{demo.webm,demo.gif,board.png}`; the other official shots write WebM and PNG under `docs/assets/shots/`.
MP4 is only for when you ask for it. `fleet-demo-director --shot DIR --describe` prints the resolved settings.
