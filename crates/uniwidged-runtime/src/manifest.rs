//! What a widget declares about itself.
//!
//! The manifest is the whole of what a user is asked to approve, so it is
//! deliberately small and deliberately readable. Everything in it is a claim
//! the runtime then enforces — nothing here is advisory.
//!
//! ```toml
//! name = "weather"
//! version = "0.2.0"
//! description = "Today's forecast for one place"
//! capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
//! memory-mb = 32
//!
//! [[setting]]
//! key = "city"
//! label = "City"
//! type = "text"
//! default = "Berlin"
//! ```
//!
//! ## Why capabilities are strings
//!
//! Because they are shown to a person. `net:api.open-meteo.com` is a sentence
//! somebody can agree or object to; a bitfield is not. The parser below is
//! strict for the same reason: a capability that does not parse is refused
//! rather than ignored, because a manifest a user approved must mean exactly
//! what it said.

use serde::Deserialize;

use crate::capability::Capability;
use crate::limits::Limits;
use crate::settings::{
    MAX_LABEL_LEN, MAX_SETTINGS, Setting, SettingKind, SettingValue, is_usable_key,
};

/// The ABI version this host speaks. See `wit/uniwidged.wit`.
pub const ABI_VERSION: u32 = 9;

/// The oldest ABI this host still runs.
///
/// **The 1.x promise.** ABI 9 is the interface uniwidged 1.0 shipped with, and
/// it is the floor for every 1.x release: a widget built against it keeps
/// running on every later 1.x host. The interface may grow — new host
/// functions, which an older widget simply does not import — and each
/// addition raises [`ABI_VERSION`] so that a widget *using* one is refused by
/// an older host in a sentence rather than failing to link. Nothing raises
/// this number before 2.0.
///
/// Before 1.0 the check was equality, because the interface moved eight times
/// in four days and every move broke something. A range is what an interface
/// that has stopped breaking can afford.
pub const ABI_OLDEST: u32 = 9;

// A floor above the ceiling would refuse every widget, including the ones this
// very build produces. Caught by the compiler rather than by a user.
const _: () = assert!(ABI_OLDEST <= ABI_VERSION);

/// Whether this host runs a widget built against `abi`.
pub fn abi_supported(abi: u32) -> bool {
    (ABI_OLDEST..=ABI_VERSION).contains(&abi)
}

/// The most memory a widget may ask for.
///
/// Not a technical limit — a cap on what can be *asked for* without a
/// conversation. A widget wanting more than this is doing something a widget
/// does not normally do, and the manifest should not be able to wave that
/// through.
pub const MAX_MEMORY_MB: u32 = 256;

/// A widget's declared identity, needs and limits.
///
/// `Eq` and not `Ord`: a setting's default can be a number, and `f64` has no
/// total equality. `PartialEq` is what the tests need and is what a manifest
/// can honestly offer.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Also the widget's directory name and its keychain namespace, which is
    /// why it is restricted to characters that are safe in both.
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<Capability>,
    /// What the user may change, declared by the widget and enforced by the
    /// host — see `crate::settings`. Ordered as the manifest wrote them,
    /// because that is the order the form is shown in and its author chose it.
    pub settings: Vec<Setting>,
    pub limits: Limits,
}

#[derive(Debug, Deserialize)]
struct Raw {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default, rename = "memory-mb")]
    memory_mb: Option<u32>,
    #[serde(default, rename = "setting")]
    settings: Vec<RawSetting>,
}

/// One `[[setting]]` block, before it is checked.
///
/// `default` is `Option` rather than required so that the error for a missing
/// one can say what is missing; `toml::Value` rather than a typed field
/// because what is valid depends on `type`, which is another field of the same
/// table.
#[derive(Debug, Deserialize)]
struct RawSetting {
    key: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    default: Option<toml::Value>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    options: Option<Vec<String>>,
}

/// Why a manifest was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    Syntax(String),
    /// The name is used as a directory and as a keychain namespace.
    BadName(String),
    UnknownCapability(String),
    MalformedCapability(String),
    MemoryTooLarge(u32),
    /// A version that is not a version.
    ///
    /// Empty is allowed and means "unversioned"; anything present has to be
    /// orderable, because an update check is a comparison and not a string
    /// inequality — see [`crate::version`].
    BadVersion(String),
    /// A `[[setting]]` block that does not describe a setting. One variant
    /// rather than eight, because every one of them is "this manifest's author
    /// has to change this line" and the string says which line and why.
    BadSetting(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Syntax(why) => write!(f, "{why}"),
            ManifestError::BadName(name) => write!(
                f,
                "`{name}` is not a usable widget name: it becomes a directory \
                 and a keychain namespace, so it must be 1–64 characters of \
                 lowercase letters, digits and hyphens, starting with a letter"
            ),
            ManifestError::UnknownCapability(text) => {
                write!(f, "`{text}` is not a capability this host knows")
            }
            ManifestError::MalformedCapability(text) => {
                write!(f, "`{text}` is not a well-formed capability")
            }
            ManifestError::MemoryTooLarge(mb) => write!(
                f,
                "a widget may ask for at most {MAX_MEMORY_MB} MB; this one asks for {mb}"
            ),
            ManifestError::BadVersion(why) => write!(f, "{why}"),
            ManifestError::BadSetting(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let raw: Raw =
            toml::from_str(text).map_err(|e| ManifestError::Syntax(e.message().to_string()))?;

        if !is_usable_name(&raw.name) {
            return Err(ManifestError::BadName(raw.name));
        }
        // A version is optional and, when present, has to be one: `0.1.0` and
        // `0.1` are both things people type, and only one of them can be
        // compared with the version a registry offers.
        if !raw.version.is_empty()
            && let Err(why) = crate::version::Version::parse(&raw.version)
        {
            return Err(ManifestError::BadVersion(why.to_string()));
        }
        let memory_mb = raw.memory_mb.unwrap_or(32);
        if memory_mb > MAX_MEMORY_MB {
            return Err(ManifestError::MemoryTooLarge(memory_mb));
        }

        let capabilities = raw
            .capabilities
            .iter()
            .map(|text| parse_capability(text))
            .collect::<Result<Vec<_>, _>>()?;

        let settings = parse_settings(&raw.settings)?;

        Ok(Manifest {
            name: raw.name,
            version: raw.version,
            description: raw.description,
            capabilities,
            settings,
            limits: Limits {
                memory_bytes: memory_mb as usize * 1024 * 1024,
                ..Limits::default()
            },
        })
    }

    /// The lines a user is shown before approving an installation.
    ///
    /// One capability per line, in the words the manifest used, because that
    /// is the thing being agreed to. A widget that declares nothing says so
    /// explicitly rather than showing an empty list, which reads as a bug.
    pub fn consent_summary(&self) -> Vec<String> {
        if self.capabilities.is_empty() {
            return vec!["asks for nothing at all".to_string()];
        }
        self.capabilities.iter().map(describe).collect()
    }

    /// The version as something orderable, or `None` when the manifest gave
    /// none.
    ///
    /// The string is what a person reads and is kept as written; this is what
    /// decides whether a registry is offering something newer. Parsing cannot
    /// fail here — [`Manifest::parse`] refused anything that would.
    pub fn semver(&self) -> Option<crate::version::Version> {
        crate::version::Version::parse(&self.version).ok()
    }
}

/// A capability as it is written in a manifest.
///
/// The inverse of the capability parser [`Manifest::parse`] uses; they are
/// tested against each other so
/// a capability cannot mean one thing when declared and another when granted.
///
/// It lives beside the parser rather than beside its first caller. There were
/// two readings of a URL once — see [`crate::net::host_of`] — and one printer
/// with two parsers is the same shape of mistake: what a registry index lists,
/// what a consent screen shows and what a configuration grants have to be the
/// same string or they are not the same permission.
pub fn capability_text(capability: &Capability) -> String {
    match capability {
        Capability::Timer => "timer".into(),
        Capability::Net(host) => format!("net:{host}"),
        Capability::NetWildcard(domain) => format!("net:*.{domain}"),
        Capability::Fs(path) => format!("fs:{path}"),
        Capability::Link(host) => format!("link:{host}"),
        Capability::LinkWildcard(domain) => format!("link:*.{domain}"),
        Capability::Secrets => "secrets".into(),
        Capability::Locale => "locale".into(),
        Capability::Measure => "measure".into(),
        Capability::Command(name) => format!("command:{name}"),
        Capability::Metrics => "metrics".into(),
    }
}

/// Is this usable as a directory name and a keychain namespace?
///
/// Strict on purpose. The name reaches a filesystem path and a platform
/// credential store, and both have their own opinions about separators, case
/// and length — so the intersection of what is safe everywhere is what is
/// allowed, rather than what happens to work on the developer's machine.
pub(crate) fn is_usable_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return false;
    }
    if name.ends_with('-') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Turns `[[setting]]` blocks into declarations the host can enforce.
///
/// Strict, and for the same reason the capability parser is: a manifest is
/// what a user is shown and what the host then enforces, so a line that does
/// not mean something exact is refused where its author can still fix it
/// rather than being interpreted for them later.
///
/// The default is checked against the setting's own rules, so a `number`
/// declared with `default = 2` and `max = 1` fails here — at the moment its
/// author writes it, rather than the first time a user opens a form and finds
/// a value the widget already had that the form will not let them re-enter.
fn parse_settings(raw: &[RawSetting]) -> Result<Vec<Setting>, ManifestError> {
    let bad = |why: String| ManifestError::BadSetting(why);

    if raw.len() > MAX_SETTINGS {
        return Err(bad(format!(
            "a widget may declare at most {MAX_SETTINGS} settings; this one \
             declares {}",
            raw.len()
        )));
    }

    let mut settings: Vec<Setting> = Vec::with_capacity(raw.len());
    for entry in raw {
        if !is_usable_key(&entry.key) {
            return Err(bad(format!(
                "`{}` is not a usable setting key: it is a bare key in a file \
                 people edit and an argument on a command line, so it must be \
                 1–32 characters of lowercase letters, digits and hyphens, \
                 starting with a letter",
                entry.key
            )));
        }
        // Two settings with one key is not a manifest that means two things —
        // it is one whose author will be surprised by which one wins.
        if settings.iter().any(|s| s.key == entry.key) {
            return Err(bad(format!(
                "`{}` is declared twice; a key names one setting",
                entry.key
            )));
        }
        let label = entry.label.clone().unwrap_or_else(|| entry.key.clone());
        if label.len() > MAX_LABEL_LEN {
            return Err(bad(format!(
                "the label for `{}` may be at most {MAX_LABEL_LEN} characters",
                entry.key
            )));
        }

        let kind = match entry.kind.as_str() {
            "text" => SettingKind::Text,
            "toggle" => SettingKind::Toggle,
            "number" => {
                // Unbounded by default rather than refused: a widget taking a
                // count has no honest upper bound to write, and demanding one
                // would get an arbitrary number typed in.
                let min = entry.min.unwrap_or(f64::MIN);
                let max = entry.max.unwrap_or(f64::MAX);
                // `min > max` rather than `!(min <= max)`: with two `f64` the
                // two differ when either is `NaN`, and a `NaN` bound is caught
                // by the default check below with a message about the value
                // rather than about the range.
                if min > max {
                    return Err(bad(format!(
                        "`{}` has min {min} above max {max}, which no value can satisfy",
                        entry.key
                    )));
                }
                SettingKind::Number { min, max }
            }
            "choice" => {
                let options = entry.options.clone().unwrap_or_default();
                if options.len() < 2 {
                    return Err(bad(format!(
                        "`{}` is a choice and needs at least two options; a \
                         choice of one is a decision its author already made",
                        entry.key
                    )));
                }
                SettingKind::Choice { options }
            }
            other => {
                return Err(bad(format!(
                    "`{}` is declared as `{other}`; a setting is text, number, \
                     toggle or choice",
                    entry.key
                )));
            }
        };

        let Some(default) = entry.default.as_ref() else {
            return Err(bad(format!(
                "`{}` has no default. Every declared key always has a value — \
                 that is what lets a widget read one without a fallback — so \
                 there is no such thing as a setting that is unset",
                entry.key
            )));
        };
        let default = value_from_toml(&entry.key, default)?;

        let setting = Setting {
            key: entry.key.clone(),
            label,
            kind,
            default,
        };
        // The declared default has to be a value the setting would accept from
        // a user. One that is not is a manifest that ships a value its own
        // form would refuse.
        setting
            .check(&setting.default)
            .map_err(|e| bad(format!("{e} — and that is the declared default")))?;
        settings.push(setting);
    }
    Ok(settings)
}

/// A TOML scalar as a setting value.
///
/// Integers become numbers, because TOML distinguishes `1` from `1.0` and a
/// manifest author writing `default = 1` for a number means one. Everything
/// else — a table, an array, a datetime — is refused rather than stringified.
fn value_from_toml(key: &str, value: &toml::Value) -> Result<SettingValue, ManifestError> {
    match value {
        toml::Value::String(text) => Ok(SettingValue::Text(text.clone())),
        toml::Value::Boolean(toggle) => Ok(SettingValue::Toggle(*toggle)),
        toml::Value::Float(number) => Ok(SettingValue::Number(*number)),
        toml::Value::Integer(number) => Ok(SettingValue::Number(*number as f64)),
        other => Err(ManifestError::BadSetting(format!(
            "the default for `{key}` is {}, and a setting is text, a number or \
             a toggle",
            other.type_str()
        ))),
    }
}

pub(crate) fn parse_capability(text: &str) -> Result<Capability, ManifestError> {
    let malformed = || ManifestError::MalformedCapability(text.to_string());
    match text.split_once(':') {
        None => match text {
            "timer" => Ok(Capability::Timer),
            "secrets" => Ok(Capability::Secrets),
            "locale" => Ok(Capability::Locale),
            "measure" => Ok(Capability::Measure),
            "metrics" => Ok(Capability::Metrics),
            _ => Err(ManifestError::UnknownCapability(text.to_string())),
        },
        Some((kind, value)) => {
            if value.is_empty() {
                return Err(malformed());
            }
            match kind {
                "net" => match value.strip_prefix("*.") {
                    // A bare `*` would be "the whole internet" spelled as a
                    // wildcard, which is the one thing host-scoping exists to
                    // prevent.
                    Some(domain) if is_host(domain) => {
                        Ok(Capability::NetWildcard(domain.to_ascii_lowercase()))
                    }
                    Some(_) => Err(malformed()),
                    None if is_host(value) => Ok(Capability::Net(value.to_ascii_lowercase())),
                    None => Err(malformed()),
                },
                // The same reading as `net`, deliberately spelled out again
                // rather than shared: they are two grants, and a reader has to
                // be able to see that neither implies the other.
                "link" => match value.strip_prefix("*.") {
                    Some(domain) if is_host(domain) => {
                        Ok(Capability::LinkWildcard(domain.to_ascii_lowercase()))
                    }
                    Some(_) => Err(malformed()),
                    None if is_host(value) => Ok(Capability::Link(value.to_ascii_lowercase())),
                    None => Err(malformed()),
                },
                "fs" => {
                    // Control characters are checked because TOML's basic
                    // strings escape them: `"a\b"` in a manifest is a
                    // backspace, not a backslash, and a path carrying one is
                    // not something to create on a user's disk.
                    if value.starts_with('/')
                        || value.contains("..")
                        || value.contains('\\')
                        || value.chars().any(char::is_control)
                    {
                        return Err(malformed());
                    }
                    Ok(Capability::Fs(value.to_string()))
                }
                "command" => {
                    if value.contains('/')
                        || value.contains('\\')
                        || value.chars().any(char::is_control)
                    {
                        return Err(malformed());
                    }
                    Ok(Capability::Command(value.to_string()))
                }
                _ => Err(ManifestError::UnknownCapability(text.to_string())),
            }
        }
    }
}

/// Does this look like a hostname, rather than a URL or a pattern?
fn is_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

pub(crate) fn describe(capability: &Capability) -> String {
    match capability {
        Capability::Timer => "update itself on a schedule".to_string(),
        Capability::Net(host) => format!("talk to {host} over HTTPS"),
        Capability::NetWildcard(domain) => {
            format!("talk to any subdomain of {domain} over HTTPS")
        }
        Capability::Fs(path) => format!("read and write `{path}` in its own folder"),
        Capability::Link(host) => format!("open {host} in your browser when you click a link"),
        Capability::LinkWildcard(domain) => {
            format!("open any subdomain of {domain} in your browser when you click a link")
        }
        Capability::Secrets => "store passwords in your keychain".to_string(),
        Capability::Locale => "format dates and times for your locale".to_string(),
        Capability::Measure => "measure text".to_string(),
        Capability::Command(name) => format!("run the `{name}` command"),
        Capability::Metrics => "read CPU, memory, disk, battery and network counters".to_string(),
    }
}

#[cfg(test)]
mod tests;
