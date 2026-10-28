# Development

Run the tests, find the design notes, and follow the commit style.

## Tests

```bash
cargo test --workspace          # daemon tests need tmux and git on PATH
bash packaging/test-install.sh
```

CI (`.github/workflows/ci.yml`) runs fmt, the tests of fleet-api, fleet-fs, fleet-log, fleet-client and fleet-gui, and a
compile check of fleet-daemon, on Linux and Windows; daemon tests need Linux with tmux.

Building for your own machine needs only Rust (`cargo build --release -p fleet-gui -p fleet-daemon`); zig and
`cargo-zigbuild` are only needed by `./build.sh` to reproduce the glibc 2.28 Linux release build. More:
[Install](../getting-started/install.md#build).

## Design notes

- [What fleet management means here](../concept.md): the problem fleet-manager manages.
- [UI internals](ui-internals.md): how both UIs are put together.
- [Architecture](../reference/architecture.md): the daemon, the app and the crates.
- [Demo media](demo.md): how the board image and animation are recorded.

## Issues and commits

- Deferred work: [GitHub issues](https://github.com/TonyHeadband/fleet-manager/issues).
- Commits follow [Conventional Commits](https://www.conventionalcommits.org/).
