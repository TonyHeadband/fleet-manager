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

Agent tests never run the real `claude`: they use a scripted stand-in. CI (`.github/workflows/ci.yml`) runs fmt,
the whole workspace's tests and a check that nothing changed the tree, on Linux and Windows (both required) and
macOS (reported, not blocking). The Pages and Demo media workflows run only when started by hand for now.

`.fleet/workflow.yml` lists the checks the Fleet app's pull-request gate runs on a branch, and which changed paths
trigger each.

The web UI is not built: the daemon serves a placeholder page. It returns, as an option for remote access, in a
later release.

## Design notes

- [What fleet management means here](../concept.md): the problem Fleet Manager manages.
- [UI internals](ui-internals.md): how both UIs are put together.
- [Architecture](../reference/architecture.md): the daemon, the app and the crates.
- [Demo media](demo.md): how the board image and animation are recorded.

## Issues and commits

- Deferred work: [GitHub issues](https://github.com/TonyHeadband/fleet-manager/issues).
- Commits follow [Conventional Commits](https://www.conventionalcommits.org/).
