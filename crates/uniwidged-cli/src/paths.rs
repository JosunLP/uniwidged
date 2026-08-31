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

/// One widget's own data directory.
///
/// Named by the widget, which is why the manifest is strict about what a name
/// may contain: the name is a directory on disk, and two widgets sharing one
/// would share their data. The host joins this itself; the CLI needs it to say
/// where a removed widget's data went.
pub fn widget_data_dir(name: &str) -> PathBuf {
    data_root().join(name)
}

/// The keys this machine trusts to sign widgets.
///
/// Beside the configuration, and meant to be read: it is the whole of the
/// project's trust model, and a security mechanism nobody can inspect is one
/// nobody should rely on.
pub fn trusted_keys_file() -> PathBuf {
    beside_config("trusted-keys")
}

/// Where every widget's own directory lives.
///
/// Beside the configuration rather than under `XDG_DATA_HOME`, for the reason
/// in the module note: a user who copies their configuration directory to a
/// second machine should get their widgets *and* what those widgets remembered,
/// and splitting the two across two trees is how half of it gets left behind.
///
/// One widget's own directory is `<root>/<its manifest name>`, joined by the
/// host rather than here — which is why the manifest is strict about what a
/// name may contain: the name becomes a directory, and two widgets sharing one
/// would share their data.
///
/// It follows `UNIWIDGED_CONFIG` when that is set, so a test desktop — or two
/// desktops running side by side — cannot write into the real one.
pub fn data_root() -> PathBuf {
    beside_config("widgets")
}

/// Where the keychain behind `secrets` lives.
///
/// Beside the configuration, not under it: a user who copies their
/// configuration directory to a second machine should get their widgets and
/// what those widgets remembered, and a keychain left behind is a desktop full
/// of widgets asking to sign in again.
///
/// It is two files — the store and its key — and both are written `0600`. They
/// are in the same directory on purpose and it is worth being plain about what
/// that does and does not buy: see `uniwidged_runtime::keyring`, and
/// `uniwidged doctor`, which prints the honest sentence.
///
/// Follows `UNIWIDGED_CONFIG` like everything else here, so a test desktop
/// cannot write into the real one.
pub fn keyring_dir() -> PathBuf {
    match std::env::var_os("UNIWIDGED_CONFIG") {
        Some(explicit) => PathBuf::from(explicit)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf(),
        None => config_dir(),
    }
}

/// The registries this machine reads.
///
/// Beside `trusted-keys`, and meant to be read for the same reason: it is the
/// other half of where a widget comes from, and a list of places somebody's
/// desktop fetches code from should be one they can look at.
pub fn registries_file() -> PathBuf {
    beside_config("registries.toml")
}

/// The last index fetched from each registry, and its signature.
///
/// A cache and nothing more: deleting it costs one fetch. It is verified again
/// every time it is read — see `registry::read_cache` — because a file being
/// local is not a reason to believe it.
pub fn registry_cache_dir() -> PathBuf {
    beside_config("registry-cache")
}

/// Where widgets downloaded from a registry are kept.
///
/// The configuration records where a component *is*, so a download has to land
/// somewhere that outlives the command that fetched it. Beside everything
/// else, so that copying a configuration directory to a second machine brings
/// the widgets with it — which is the same argument as `data_root`.
pub fn widget_store_dir() -> PathBuf {
    beside_config("store")
}

/// Where files that belong to a *running* desktop go — the pid file, the log,
/// what `uniwidged status` reads.
///
/// The same directory as the configuration, which is what makes them follow
/// `UNIWIDGED_CONFIG`: two desktops side by side get two logs and two pid
/// files, and a test gets neither of the real ones.
pub fn state_dir() -> PathBuf {
    keyring_dir()
}

/// A path in the configuration directory, following `UNIWIDGED_CONFIG` when it
/// is set.
///
/// The one place that rule is written down. It was three copies of the same
/// four lines before there were six things beside the configuration, and a
/// seventh that forgot would be a test writing into somebody's real desktop.
fn beside_config(name: &str) -> PathBuf {
    match std::env::var_os("UNIWIDGED_CONFIG") {
        Some(explicit) => PathBuf::from(explicit)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(name),
        None => config_dir().join(name),
    }
}
