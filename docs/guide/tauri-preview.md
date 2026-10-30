# Fleet Manager on Tauri (preview)

The next Fleet Manager app is built on Tauri: the web UI in a native window, with the same daemon. On Linux it
already does what the current app does from launch to the board: the boot view, the first-run wizard (folders,
Requirements, your first persona and its logins), the close dialog, the restart offer for an older background
service and the new-version notice. Tasks, Releases, Management and Settings come next. Until then the releases keep
shipping the current app.

## What you need

A current Linux with WebKitGTK 4.1: Ubuntu 22.04, Fedora 36 or later. Older systems keep the daemon on its own and
use the web UI in a browser.

## Build and run it

From the repository, with Rust, Node 22 and pnpm 10:

```bash
pnpm --dir web install
pnpm --dir web build
cargo build --release -p fleet-daemon -p fleet-client -p fleet-tauri --bins
cd crates/fleet-tauri && ../../web/node_modules/.bin/tauri build --bundles deb,rpm
```

That leaves:

- `target/release/fleet-gui-tauri`, the app, with `fleet-daemon` and `fleet-attach` beside it. Run it from there.
- A `.deb` and an `.rpm` under `target/release/bundle/` that install all three to `/usr/bin`.

CI builds the same packages on every pull request and keeps them for a week as the run's
`fleet-manager-tauri-linux` artifact.

## Beside the current app

Both apps can be installed together, and they share your settings in `gui.toml`: the active persona and product,
the close choice, a skipped version. Run one at a time. The Tauri app adds its own app menu entry when the wizard's
"Add an app menu shortcut" is ticked.
