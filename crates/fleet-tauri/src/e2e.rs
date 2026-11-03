//! Only in builds with the `e2e` feature, which CI's smoke test makes and no release does. WebDriver turns on
//! WebView2's remote debugging through `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, which WebView2 ignores in an elevated
//! app (GitHub's Windows runners run elevated). Flags set in code are honoured there, so this build passes the
//! variable's flags to the main window itself.

/// What wry passes when a window sets no browser arguments of its own; setting any replaces it, so it is kept.
const WRY_DEFAULT: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// The main window's browser arguments for `env` (the variable's value): wry's defaults, then the driver's flags.
/// `None` when the variable is unset or blank, so the window keeps wry's own.
pub fn browser_args(env: Option<&str>) -> Option<String> {
    let flags = env.map(str::trim).filter(|f| !f.is_empty())?;
    Some(format!("{WRY_DEFAULT} {flags}"))
}

#[cfg(test)]
mod tests {
    use super::browser_args;

    #[test]
    fn the_drivers_flags_follow_wrys_defaults() {
        assert_eq!(
            browser_args(Some("--remote-debugging-port=0 ")).as_deref(),
            Some("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=0")
        );
    }

    #[test]
    fn no_flags_leave_the_window_as_wry_makes_it() {
        assert_eq!(browser_args(None), None);
        assert_eq!(browser_args(Some("  ")), None);
    }

    /// A release build must never pass the environment's flags on: in an elevated app, that is what WebView2 refuses.
    #[test]
    fn the_feature_is_off_by_default() {
        let manifest: toml::Value =
            toml::from_str(include_str!("../Cargo.toml")).expect("fleet-tauri's Cargo.toml");
        let default = manifest["features"]["default"]
            .as_array()
            .expect("[features] default");
        assert!(default.is_empty(), "{default:?}");
    }
}
