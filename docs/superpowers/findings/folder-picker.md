# Folder picker and continue-from-checkout: real run

Run 2026-10-24 (Task 6), against the release build of the uncommitted folder-picker work. Spec:
`docs/superpowers/specs/2026-10-24-folder-picker-design.md`.

## Summary

| Check | Result | Evidence |
|---|---|---|
| (a) First-run Browse in folder mode picks the projects root | PASS | `a1`–`a3`; `gui.toml` got `last_pick_dir` |
| (b) New project Browse: `foo` with branch badge; expanded `foo` (`main`) and `foo-feat` (`feat`, "1 uncommitted file won't be included"); `fleet-projects` greyed | PASS | `b2`–`b5` |
| (c) Picking `foo-feat` starts from its HEAD; `wip.txt` absent; `foo`, its branches and `origin/main` unchanged | PASS | git output below, `c1`–`c3` |
| (d) Chip tooltip shows `started from feat` and the merge-back note | PASS | `d1` |
| (e) Remembered chip `foo` starts from `foo`'s unpushed commit | PASS | git output below, `e1`, `e2` |
| Gates | PASS | see "Gates" |

## How it was run

- Release build: `cargo build --release -p fleet-gui -p fleet-daemon`. Window on `Xvfb :90 -screen 0 1400x800x24 +extension GLX -nolisten tcp`, captured by window id with `import -window`.
- Clicks and typing: a Python `ctypes` script driving `libXtst` (XTEST pointer and key events) plus `XSetInputFocus`, kept in the session scratchpad, not in the repo.
- No real `claude`: `claude_command` was the repo's test double `crates/fleet-daemon/tests/bin/claude` (prints its cwd/args, then `cat -v`). The cards therefore sat in "Startup prompt"; that is the stub, not the feature.

### Fixture (inside the throwaway `HOME`)

- `remote/foo.git`: bare remote, `main` = `7f9ffd1 Initial commit`.
- `dev/foo`: clone on `main`, one unpushed commit `7d0617a Unpushed work on main`.
- `dev/foo-feat`: `git worktree add -b feat`, one commit `3cd8f88 Feature commit on feat`, plus untracked `wip.txt`.
- `dev/notes`: a plain folder. `fleet-projects`: the projects root.
- For the look check only, after (e): a linked worktree at a deliberately long path on a long branch name (`v1`, `v2`).

### Isolation

- `HOME` = `mktemp -d /tmp/fb-fp.XXXXXX`/home (the tmux socket path must stay under ~108 bytes, so not the scratchpad). Own `~/.config/fleet-board/config.toml`: `listen = "127.0.0.1:7950"`, `state_dir` and `projects_root` inside the temp dir, the fake `claude_command`. Own `gui.toml`.
- Private tmux: `FLEET_TMUX_SOCKET=fbfp`, `TMUX_TMPDIR` in the temp dir.
- (a) ran with no config (first-run). The GUI binary was copied alone into a temp folder so it had no `fleet-daemon` beside it, and Continue was **not** pressed: with no config the client defaults to port 7878, where the owner's daemon is live. After (a) the GUI was stopped and the config written by hand.
- The owner's daemon on `127.0.0.1:7878` stayed up and was never contacted; `~/.config/fleet-board` and `~/fleet-projects` were never read or written. No credentials were copied.
- Stopped afterwards, by recorded pid only: the two GUIs, the daemon the second GUI started, Xvfb; the private tmux server by its socket. The temp dir was removed.

## (a) First-run Browse

The first-run field held `~/fleet-projects`. Browse opened "Choose a folder" in that folder (tilde expanded, `a1`). Up showed `dev` and `fleet-projects` (`a2`); clicking `fleet-projects` entered it, Select closed the modal and filled the field with `<tmp>/home/fleet-projects`; the trust command under it followed (`a3`). `gui.toml` then held `last_pick_dir = "<tmp>/home/fleet-projects"`.

## (b) Repo mode listing

- New project → Browse opened "Choose a repo" at the remembered folder, which is the projects root (`b1`, see Concerns V1).
- Home listing (`b2`): `dev`, and `fleet-projects` in the low text colour with a "fleet project" label; its tooltip says it holds agents' copies. Clicking it in repo mode does nothing.
- `dev` (`b3`): `foo` with a `main` badge, `foo-feat` with a `feat` badge, `notes` plain.
- Clicking `foo` expanded its checkouts (`b4`): `main 7d0617a Unpushed work on main` at `<tmp>/home/dev/foo`, and `feat 3cd8f88 Feature commit on feat` at `<tmp>/home/dev/foo-feat` with the orange badge "1 uncommitted file won't be included". Hovering the `feat` row shows "Merge fleet/<project> into feat first (a fast-forward if you did not commit there meanwhile), then merge feat as usual." (`b5`). Select stays disabled until a checkout is chosen.

## (c) Start from `foo-feat`

Chose the `feat` checkout (`c1`), Select filled the New project field (`c2`), Start project. Project `continue-the-feat-work` appeared (`c3`). Paths below have the temp dir replaced by `<tmp>`.

```
$ git -C <proj>/foo log -1 --format='%H %s'
3cd8f88125946f5b156337f641a022d9104deb0b Feature commit on feat
$ git -C dev/foo-feat rev-parse HEAD
3cd8f88125946f5b156337f641a022d9104deb0b
$ git -C <proj>/foo rev-parse --abbrev-ref HEAD
fleet/continue-the-feat-work
$ ls -A <proj>/foo
feat.txt  .git  README.md
$ test -e <proj>/foo/wip.txt
absent
$ git -C <proj>/foo status --porcelain
(empty)
```

`dev/foo` before and after the start (identical):

```
$ git -C dev/foo rev-parse --abbrev-ref HEAD           -> main
$ git -C dev/foo rev-parse HEAD main feat origin/main
7d0617a5145642cae0d6a863d8dd420a2037cd8b
7d0617a5145642cae0d6a863d8dd420a2037cd8b
3cd8f88125946f5b156337f641a022d9104deb0b
7f9ffd1c2a021d5c390170ad4dfde39621a11d0d
$ git -C dev/foo-feat rev-parse --abbrev-ref HEAD; rev-parse HEAD   -> feat, 3cd8f88…
$ git -C dev/foo status --porcelain       -> (empty)
$ git -C dev/foo-feat status --porcelain  -> ?? wip.txt
```

After: `git worktree list` added only `<tmp>/home/fleet-projects/continue-the-feat-work/foo 3cd8f88 [fleet/continue-the-feat-work]`; `git branch -a` added only `fleet/continue-the-feat-work`. The bare remote's `main` stayed `7f9ffd1`, and `dev/foo/.git/FETCH_HEAD` does not exist (a path repo is not fetched). `GET /api/repos` on the temp daemon: `[{"name":"foo","source":"path","location":"<tmp>/home/dev/foo"}]`, remembered once, by the main checkout.

## (d) Chip tooltip

Side panel on that card, hovering the `foo` chip (`d1`):

> fleet/continue-the-feat-work, started from feat at 3cd8f88
>
> Merge fleet/continue-the-feat-work into feat first (a fast-forward if you did not commit there meanwhile), then merge feat as usual.

## (e) Remembered chip

New project showed `foo` as a remembered chip; clicked it (`e1`), left the path field empty, Start project.

```
$ git -C <proj2>/foo log -1 --format='%H %s'
7d0617a5145642cae0d6a863d8dd420a2037cd8b Unpushed work on main
$ git -C <proj2>/foo rev-parse --abbrev-ref HEAD
fleet/main-checkout-work
$ git -C dev/foo rev-parse HEAD origin/main
7d0617a5145642cae0d6a863d8dd420a2037cd8b
7f9ffd1c2a021d5c390170ad4dfde39621a11d0d
```

So the start was `foo`'s unpushed HEAD, not `origin/main`. Afterwards `dev/foo` was still on `main` at `7d0617a`, `feat` at `3cd8f88`, `origin/main` at `7f9ffd1`, no `FETCH_HEAD`. The chip tooltip read "fleet/main-checkout-work, started from main at 7d0617a" plus the note for `main` (`e2`).

## Gates (after the run)

- `cargo test --workspace`: exit 0, 541 passed, 0 failed (41 test binaries). The known flaky `gui_lifecycle` test (#20) did not fail this time.
- `cargo clippy --workspace --all-targets -- -D warnings -A clippy::large_enum_variant`: clean.
- Colour grep over `crates/fleet-gui/src` outside `theme.rs`: no output.
- `cd web && pnpm build`: built. The only `web/` change is the generated `web/src/api/generated/Workspace.ts`.

## Concerns found

No functional failures. Items below are about the spec and the look.

Spec gap:

- **S1. No dirty marker on folder rows.** The spec's Goals say a checkout folder "shows a badge with its current branch, and a dirty marker when it has uncommitted files". In the listing `foo-feat` shows only `feat` (`b3`); the count appears only after expanding the repo. The plan narrowed this (dirty only on checkout rows, since `git status` is slow on the network home). This should be either accepted in the spec or added.

Visual:

- **V1. The first repo-mode Browse opens inside the projects root.** First-run Select saves the projects root as `last_pick_dir`, so the next Browse (New project) opens in the folder of agents' copies: "No folders here" with Select disabled (`b1`). Suggest not remembering a folder under the excluded root, or falling back to Home when it is.
- **V2. Long paths are cut at the end, hiding the part that tells them apart.** The checkout path line ends in `…/a-rather-long-folder-name-to-check-truncation/ne…` (`v1`), and the path bar shows the start of a long path and cuts off the current folder's name (`/tmp/…/a-rather-long-folder-name-to-chec`). Eliding in the middle or keeping the tail (folder name) would read better. Same in the New project field: about 200 px wide, so the picked `<tmp>/home/dev/foo-feat` shows as `/tmp/fb-fp…/home/dev/fo` (`c2`); `foo` and `foo-feat` look identical there.
- **V3. Chosen checkout looks the same as the expanded repo row.** Both use the same selected fill (`c1`, `v2`), with no accent, check mark or bolder edge, and the footer hint becomes empty once a checkout is chosen. Showing "Select: <path>" in the footer, or an accent on the chosen row, would make clear what Select takes. In light theme the hover fill of a checkout row is white, lighter than the list background, while the selected fill is darker (`b5` vs `c1`), so hover and selection go in opposite directions.
- **V4. Branch badge blends into the expanded row.** The `main` badge on the expanded `foo` row has almost the same fill as the row highlight, in both themes (`b4`, `v2`).
- **V5. No expand affordance.** Repo rows and folder rows look alike apart from the badge; there is no chevron or folder/repo icon, so it is not obvious that clicking `foo` expands checkouts while clicking `notes` navigates.
- **V6. Light-theme attention badge contrast is low.** "1 uncommitted file won't be included" is `#D97706` on `#EFE3D6`: about 2.5:1, below 4.5:1 for small text. Dark theme measures about 5.4:1. This is the shared `BadgeTone::Attention` style, so it also affects "Startup prompt" on cards.
- **V7. A long branch name squeezes the commit subject** to one letter plus ellipsis ("In…", `v1`). Acceptable, but the branch could be truncated first.
- **V8. The greyed `fleet-projects` row is only slightly lighter than normal rows** in light theme (`b2`). The "fleet project" label carries most of the meaning.

## Captures (`folder-picker/`)

| File | Shows |
|---|---|
| `a1-first-run-picker-opens-dark.png` | First-run Browse: "Choose a folder" opened at `~/fleet-projects` |
| `a2-first-run-picker-home-dark.png` | After Up: `dev`, `fleet-projects` (no greying in folder mode before a root exists) |
| `a3-first-run-filled-dark.png` | First-run field filled with the picked root |
| `b1-repo-picker-opens-in-projects-root-light.png` | "Choose a repo" opening in the projects root (V1) |
| `b2-home-projects-root-greyed-light.png` | `fleet-projects` greyed with "fleet project" |
| `b3-dev-branch-badges-light.png` | `foo` `main`, `foo-feat` `feat`, `notes` |
| `b4-foo-expanded-light.png` | `foo` expanded: `main` and `feat` checkouts, dirty note |
| `b5-checkout-merge-tooltip-light.png` | Merge-back tooltip on the `feat` checkout row |
| `c1-feat-chosen-light.png` | `feat` chosen, Select enabled |
| `c2-new-project-filled-light.png` | New project with the picked path (V2) |
| `c3-board-after-start-light.png` | Board with the new project |
| `d1-chip-tooltip-feat-light.png` | Chip tooltip: started from feat, merge-back note |
| `e1-remembered-chip-selected-light.png` | New project with the remembered `foo` chip selected |
| `e2-chip-tooltip-main-light.png` | Chip tooltip of the second project: started from main at 7d0617a |
| `v1-picker-long-paths-dark.png` | Dark: long worktree path and branch (V2, V7) |
| `v2-picker-chosen-dark.png` | Dark: `feat` chosen |

The terminal text visible behind `d1` includes the throwaway daemon's agent token; that daemon and its state were deleted after the run.
