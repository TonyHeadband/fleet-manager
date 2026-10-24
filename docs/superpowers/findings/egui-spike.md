# egui spike (2026-10-23)

Machine: Rocky 8.10, X11 session `DISPLAY=:1` at 2560x1440, NVIDIA A40 (vGPU) with direct OpenGL 4.6, mesa 23.1. eframe/egui 0.37.0, Rust 1.99.1.

## Answers

- **Does a native window open and render?** Yes on both the desktop X display (`:1`) and a virtual one (`Xvfb :99 -screen 0 1400x800x24 +extension GLX`, software GL). Screenshots: `egui-spike-desktop.png`, `egui-spike-xvfb.png`. A root-window capture of `:1` comes out empty (the compositor hides it); capture the window by id (`xwininfo -root -tree`, then `import -window <id>`).
- **API change in eframe 0.37.** `App` now requires `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame)`, not `update(ctx, frame)`, and `CentralPanel::show` takes a `&mut Ui`. The default backend is wgpu; this project selects `glow` with `default-features = false, features = ["glow", "x11", "wayland", "default_fonts"]`.
- **Linking.** The release binary needs at most `GLIBC_2.28`. Its only dynamic libraries are libc, libm, libdl, libpthread, libgcc_s: X11, GL, EGL and xkbcommon are loaded at runtime, so a build never needs their development packages and the glibc floor does not rise.
- **Zig cross-build.** `cargo zigbuild --release -p fleet-gui --target x86_64-unknown-linux-gnu.2.28` works (35 s), the binary's maximum is again `GLIBC_2.28`, and it opens and renders on Xvfb (`egui-spike-zig-built.png`). So `build.sh` builds the GUI with zig, like the daemon; the native fallback stays as a safety net.
- **egui_kittest 0.37.** `Harness::new_ui`, `get_by_label(..).click()`, `harness.run()` work headlessly in a normal `cargo test`. Tests assert behaviour through accessibility labels.
- **Windows compile check.** `cargo check -p fleet-gui --target x86_64-pc-windows-gnu` passes (the spike has no Unix-only code yet).
- **First build time.** Cold release build about 33 s with the shared target directory warm for most dependencies.
