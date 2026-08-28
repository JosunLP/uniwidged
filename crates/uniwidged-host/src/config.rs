//! What the user installed, and where they put it.
//!
//! One file, read at start-up and written back when a widget is moved. It is
//! meant to be readable and hand-editable — a desktop's configuration that can
//! only be changed through the desktop is a configuration nobody can back up
//! or copy to a second machine.
//!
//! ```toml
//! [[widget]]
//! name = "system-load"
//! component = "widgets/system-load.wasm"
//! granted = ["timer", "metrics"]
//!
//! [widget.placement]
//! output = "DP-1"
//! x = 48
//! y = 48
//! width = 320
//! height = 200
//!
//! [widget.settings]
//! warn-at = 0.9
//! ```
//!
//! ## The granted list is checked against the manifest
//!
//! A user approving an installation approved *that manifest's* list. A config
//! that grants more than the manifest declares would hand a widget something
//! its author never designed for and its user never saw, so
//! [`WidgetConfig::policy`] refuses rather than obliging. Granting *less* is
//! fine and expected: declining one capability is how a user says no to part
//! of a widget without saying no to all of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uniwidged_runtime::capability::Policy;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::settings::{SettingValue, Settings};

/// Where a widget sits.
///
/// The output is stored by **name** rather than by index, because indices
/// renumber when a monitor is unplugged — and a widget that moves when you
/// dock is a widget that has lost its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    #[serde(default)]
    pub output: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Dragging and resizing off, everything else working. Ephemeris's
    /// `"locked": true`, and the setting people reach for once a widget is
    /// where they want it.
    #[serde(default)]
    pub locked: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            output: None,
            x: 48,
            y: 48,
            width: 320,
            height: 200,
            locked: false,
        }
    }
}

/// One installed widget.
///
/// `Eq` is absent because a setting's value can be a number. `PartialEq` is
/// what a test wants and is what a configuration can honestly offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetConfig {
    /// Must match the manifest's name. Also the widget's data directory and
    /// its keychain namespace.
    pub name: String,
    /// The `.wasm` component, relative to the configuration file.
    pub component: PathBuf,
    /// The manifest. Defaults to the component's path with a `.toml`
    /// extension, which is what `uniwidged install` writes.
    #[serde(default)]
    pub manifest: Option<PathBuf>,
    /// What the user approved. A subset of what the manifest declares — see
    /// the module note.
    #[serde(default)]
    pub granted: Vec<String>,
    #[serde(default)]
    pub placement: Placement,
    /// What the user set, verbatim, and only what they set — a key at its
    /// default is absent rather than written out. That keeps the file short
    /// and, more usefully, keeps a widget's own change of default from being
    /// silently overridden by a value nobody chose.
    ///
    /// Not checked against the manifest here. It is checked where it is
    /// *used*, by [`WidgetConfig::settings`], because a stale key must not
    /// stop the whole configuration from being read — see the module note.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, SettingValue>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, rename = "widget")]
    pub widgets: Vec<WidgetConfig>,
}

#[derive(Debug)]
pub enum ConfigError {
    Syntax(String),
    Io(PathBuf, String),
    /// The configuration names a widget by a name its manifest does not use.
    NameMismatch {
        config: String,
        manifest: String,
    },
    /// The configuration grants something the manifest never asked for.
    OverGranted {
        widget: String,
        capability: String,
    },
    /// A capability string that does not parse at all.
    BadCapability {
        widget: String,
        capability: String,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Syntax(why) => write!(f, "{why}"),
            ConfigError::Io(path, why) => write!(f, "{}: {why}", path.display()),
            ConfigError::NameMismatch { config, manifest } => write!(
                f,
                "the configuration calls this widget `{config}` and its manifest \
                 calls it `{manifest}`; the name decides where its data and its \
                 secrets live, so the two have to agree"
            ),
            ConfigError::OverGranted { widget, capability } => write!(
                f,
                "`{widget}` is granted `{capability}`, which its manifest never \
                 asks for — a user approving an installation approved that \
                 manifest's list, and this is not on it"
            ),
            ConfigError::BadCapability { widget, capability } => {
                write!(
                    f,
                    "`{widget}` is granted `{capability}`, which is not a capability"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::Syntax(e.message().to_string()))
    }

    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        // A configuration that does not exist yet is an empty one, not an
        // error: a desktop with no widgets is a perfectly good first run.
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Io(path.to_path_buf(), e.to_string())),
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("a config is always serialisable")
    }

    pub fn write(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ConfigError::Io(parent.to_path_buf(), e.to_string()))?;
        }
        std::fs::write(path, self.to_toml())
            .map_err(|e| ConfigError::Io(path.to_path_buf(), e.to_string()))
    }

    pub fn widget(&self, name: &str) -> Option<&WidgetConfig> {
        self.widgets.iter().find(|w| w.name == name)
    }

    pub fn widget_mut(&mut self, name: &str) -> Option<&mut WidgetConfig> {
        self.widgets.iter_mut().find(|w| w.name == name)
    }
}

impl WidgetConfig {
    /// The manifest path, defaulting beside the component.
    pub fn manifest_path(&self) -> PathBuf {
        self.manifest
            .clone()
            .unwrap_or_else(|| self.component.with_extension("toml"))
    }

    /// The policy this configuration and that manifest agree on.
    ///
    /// Refuses anything granted that the manifest does not declare — see the
    /// module note on why that is not merely tidiness.
    pub fn policy(&self, manifest: &Manifest) -> Result<Policy, ConfigError> {
        if self.name != manifest.name {
            return Err(ConfigError::NameMismatch {
                config: self.name.clone(),
                manifest: manifest.name.clone(),
            });
        }
        let mut granted = Vec::new();
        for text in &self.granted {
            // Parsed by the manifest's own parser, so the two cannot disagree
            // about what a capability string means.
            let declared = manifest
                .capabilities
                .iter()
                .find(|c| capability_text(c) == *text);
            match declared {
                Some(capability) => granted.push(capability.clone()),
                None => {
                    return Err(
                        if Manifest::parse(&format!("name = 'x'\ncapabilities = ['{text}']"))
                            .is_ok()
                        {
                            ConfigError::OverGranted {
                                widget: self.name.clone(),
                                capability: text.clone(),
                            }
                        } else {
                            ConfigError::BadCapability {
                                widget: self.name.clone(),
                                capability: text.clone(),
                            }
                        },
                    );
                }
            }
        }
        Ok(Policy::from_granted(granted))
    }

    /// A configuration granting everything the manifest asks for.
    ///
    /// What `uniwidged install` writes once the user has said yes.
    pub fn granting_all(name: &str, component: PathBuf, manifest: &Manifest) -> Self {
        Self {
            name: name.to_string(),
            component,
            manifest: None,
            granted: manifest.capabilities.iter().map(capability_text).collect(),
            placement: Placement::default(),
            // Nothing, on purpose: a fresh installation is every setting at
            // its declared default, and writing them out would freeze this
            // version's defaults into the file for good.
            settings: BTreeMap::new(),
            enabled: true,
        }
    }

    /// The values this widget should run with, and what was wrong with the
    /// file.
    ///
    /// Every key the manifest declares gets a value — the user's if it is
    /// usable, the manifest's default otherwise — and nothing else gets one.
    /// A key the widget no longer declares, or a value it would refuse, is
    /// *reported* rather than fatal: see
    /// [`Settings::resolve`](uniwidged_runtime::settings::Settings::resolve),
    /// where the difference from an over-grant is argued.
    pub fn settings(&self, manifest: &Manifest) -> (BTreeMap<String, SettingValue>, Vec<String>) {
        Settings::resolve(&manifest.settings, &self.settings)
    }

    /// Records what the user set, dropping anything that is merely the
    /// default.
    ///
    /// The asymmetry is deliberate. Storing a value equal to the default would
    /// pin this version's default into the file, so a widget that later
    /// changed its mind about a sensible starting point would never reach the
    /// people who never touched it.
    pub fn set_settings(&mut self, manifest: &Manifest, values: &BTreeMap<String, SettingValue>) {
        self.settings = values
            .iter()
            .filter(|(key, value)| {
                manifest
                    .settings
                    .iter()
                    .find(|declared| &&declared.key == key)
                    .is_none_or(|declared| &&declared.default != value)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
    }
}

/// A capability as it is written in a manifest.
///
/// Re-exported rather than written again. It moved to
/// [`uniwidged_runtime::manifest`] when a registry index came to list
/// capabilities too: three places printing a permission is three chances for
/// one of them to print it differently, and a grant that reads differently
/// from the thing granted is not the same grant.
pub use uniwidged_runtime::manifest::capability_text;

#[cfg(test)]
mod tests;
