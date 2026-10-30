# Development

Set up a build, run the tests, find the design notes, and follow the commit style.

Fleet Manager is under continuous development, so unsolicited pull requests are not accepted yet; they will open
once it reaches a stable stage. To help now, open an issue asking to contribute and say what you have in mind.

## Set up

- Rust stable (`rustup`), with `rustfmt`.
- git on `PATH`; on Windows, [Git for Windows](https://git-scm.com/downloads), whose Git Bash runs the scripts
  and the tests' shells.
- Windows: the Visual Studio Build Tools with the "Desktop development with C++" workload, which brings the MSVC
  linker and the Windows SDK. Without the SDK, linking fails on missing system libraries such as `kernel32.lib`.
- Linux release builds only: [`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild) and a zig
  (`python3 -m venv ~/.cache/fleet-manager/zig-venv && ~/.cache/fleet-manager/zig-venv/bin/pip install ziglang`).

## Build and run

```bash
cargo build -p fleet-daemon -p fleet-gui
cargo run -p fleet-gui          # the app starts the fleet-daemon it finds next to itself (or on PATH)
```

Release archives, as published:

```bash
scripts/bundle.sh               # dist/fleet-manager-v<version>-<os>-x64.{zip,tar.gz} for this OS
./build.sh                      # Linux: fmt, the daemon and app scripts, then the Linux archive
```

On Linux the binaries are linked against glibc 2.28 through zig, so one build runs on Rocky/RHEL 8 and every
distribution above it. On Windows `bundle.sh` builds natively and zips with `bsdtar`. The archive holds `fleet-gui`,
`fleet-daemon`, `fleet-attach`, `README.txt`, `LICENSE` and `NOTICE` (plus the icons on Linux) in one
`fleet-manager/` folder.

`cargo clean` is always safe: `target/` holds only build output. Test binaries and incremental caches make it grow
by tens of gigabytes over time, so clean it now and then.

## Tests

```bash
cargo test --workspace
bash scripts/test-bundle.sh     # the archive layout, without a release build
```

Agent tests never run the real `claude`: they use a scripted stand-in. CI runs one workflow per OS:
`.github/workflows/ci-linux.yml` (fmt, the web UI, the workspace's tests, the Tauri packages and their smoke test)
and `.github/workflows/ci-windows.yml` (the workspace's tests), each with a check that nothing changed the tree. A
release ships each OS's archive once that OS's CI passed on the tagged commit (`release.yml`).

Run the tests on the machine you work on before merging; that is the gate for a pull request. CI runs on every pull
request and push to `main` but nothing waits for it: a red run on `main` is fixed forward. The only hard gate is a
release tag, which needs green Linux and Windows CI on its commit (see below). The Pages and Demo media workflows run only when started by hand for now.

`.fleet/workflow.yml` lists the checks the Fleet app's pull-request gate runs on a branch, and which changed paths
trigger each.

The web UI is not built: the daemon serves a placeholder page. It returns, as an option for remote access, in a
later release.

## Versions and release notes

One logical change per pull request; a release ships whatever reached `main` since the last tag. Versions are set
by hand. Until 1.0 the counting is compact, as Cargo reads `0.y.z`: `y` takes breaking changes, `z` everything
else.

| Since the last tag | Bump |
| --- | --- |
| A breaking change: a database migration, or an API or behaviour change | minor, `0.3.0` |
| Features and fixes only | patch, `0.2.1` |

From 1.0 on it is usual semver: breaking changes bump the major, features the minor, fixes the patch.

The `conventional-semver-pre1.0` PR style (see [Pull request rules](../guide/orchestrated-tasks.md#pull-request-rules))
encodes these same rules for orchestrated tasks. Nothing changes for humans.

A pull request a user would notice adds its line to `packaging/changes/unreleased.md`, one line per paragraph
(GitHub keeps every line break). A release whose migrations change the database says that the previous version
cannot open it afterwards.

## Cut a release

1. Bump `version` in `[workspace.package]` of `Cargo.toml` (and the `fleet-api` version in
   `crates/fleet-daemon/Cargo.toml`), run `cargo update -w`, rename `packaging/changes/unreleased.md` to
   `packaging/changes/v<version>.md` (its heading names the version) and start a new empty `unreleased.md`, and
   merge to `main`.
2. Wait for CI on that commit to pass.
3. Tag it and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
4. `.github/workflows/release.yml` checks that the tag matches the version, that the commit is on `main` (or a
   `release/*` branch) and that its CI passed, builds both archives with `scripts/bundle.sh`, and creates a
   **draft** release with the archives, `SHA256SUMS` and notes (`packaging/changes/v<version>.md`, then the download and feedback section from
   `packaging/release-notes.md`).
5. Download both archives from the draft, run the [release checklist](acceptance/gui.md), then publish the
   draft. Only a published release reaches the README's download link and the app's update notice.

A manual run of the Release workflow (Actions → Release → Run workflow) builds the archives as workflow
artifacts without creating a release, to try the build.

### Pre-releases

A version with a suffix, such as `0.3.0-beta.1`, is a pre-release. Set it in `Cargo.toml` on any branch, then tag
`v0.3.0-beta.1` there. It skips the branch and CI checks, and its notes may be `packaging/changes/unreleased.md`
when there is no `v<version>.md` yet. GitHub marks the
release as a pre-release, so the README's download link and the app's update notice keep offering the last
release.

A fix for the last release that cannot wait for what is on `main` goes on a `release/<major>.<minor>` branch made
from its tag (`git switch -c release/0.2 v0.2.0`), gets CI on push, and is tagged there as `v0.2.1`. Merge it into
`main` afterwards.

## Design notes

- [What fleet management means here](../concept.md): the problem Fleet Manager manages.
- [UI internals](ui-internals.md): how both UIs are put together.
- [Architecture](../reference/architecture.md): the daemon, the app and the crates.
- [Demo media](demo.md): how the board image and animation are recorded.

## Issues and commits

- Deferred work: [GitHub issues](https://github.com/TonyHeadband/fleet-manager/issues).
- Commits follow [Conventional Commits](https://www.conventionalcommits.org/).
