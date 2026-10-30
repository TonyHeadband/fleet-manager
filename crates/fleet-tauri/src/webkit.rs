//! WebKitGTK's GPU-buffer (DMA-BUF) renderer fails with NVIDIA's proprietary driver under Wayland: the window
//! closes at once with "Error 71 (Protocol error) dispatching to Wayland display". On exactly that setup the app
//! turns the renderer off, unless the person chose for themselves. Narrow or drop this once a WebKitGTK or NVIDIA
//! release fixes it: nothing is written to disk, so an update carries the change.

pub const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// Whether to turn the renderer off: a Wayland session with the NVIDIA driver loaded, and no choice of the person's.
pub fn needs_workaround(chosen: bool, wayland: bool, nvidia: bool) -> bool {
    !chosen && wayland && nvidia
}

/// Applies the workaround to this process. Call before any thread starts: the environment is not safe to change
/// once others may read it. True when it was applied, for the log.
pub fn apply() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
    let nvidia = std::path::Path::new("/sys/module/nvidia").exists();
    let apply = needs_workaround(std::env::var_os(VAR).is_some(), wayland, nvidia);
    if apply {
        std::env::set_var(VAR, "1");
    }
    apply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_nvidia_under_wayland_gets_the_workaround() {
        assert!(needs_workaround(false, true, true));
        assert!(!needs_workaround(false, false, true), "X11 renders fine");
        assert!(
            !needs_workaround(false, true, false),
            "other drivers render fine"
        );
    }

    #[test]
    fn a_choice_of_the_person_s_stands() {
        assert!(!needs_workaround(true, true, true));
    }
}
