//! Where things live.
//!
//! The conventional per-user location of each platform, and nothing clever.
//! Ephemeris's `PortableHost` picked the same three, for the same reason: a
//! user who backs up their configuration directory should get their widgets
//! back, and one who looks for them should find them where everything else is.

use std::path::PathBuf;

/// The configuration directory.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("uniwidged");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/uniwidged");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // XDG_CONFIG_HOME wins, then the specified default.
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("uniwidged");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config/uniwidged");
        }
    }
    PathBuf::from(".")
}

pub fn config_file() -> PathBuf {
    // Overridable for testing and for running two desktops side by side, which
    // is how the M7 end-to-end check works.
    if let Some(explicit) = std::env::var_os("UNIWIDGED_CONFIG") {
        return PathBuf::from(explicit);
    }
    config_dir().join("config.toml")
}

/// A widget's own data directory. Named by the widget, which is why the
/// manifest is strict about what a name may contain.
///
/// Unused so far: the `fs` and `secrets` capabilities pass their check and then
/// answer `unavailable`, because the storage behind them is still to be written
/// — see M7 in `ROADMAP.md`. Kept here because the *shape* of the answer is
/// settled and putting it somewhere else later would move a security-relevant
/// path.
#[allow(dead_code)]
pub fn widget_data_dir(name: &str) -> PathBuf {
    config_dir().join("widgets").join(name)
}
