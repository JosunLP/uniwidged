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

/// The ABI version this host speaks. See `wit/uniwidged.wit`.
pub const ABI_VERSION: u32 = 3;

/// The most memory a widget may ask for.
///
/// Not a technical limit — a cap on what can be *asked for* without a
/// conversation. A widget wanting more than this is doing something a widget
/// does not normally do, and the manifest should not be able to wave that
/// through.
pub const MAX_MEMORY_MB: u32 = 256;

/// A widget's declared identity, needs and limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Also the widget's directory name and its keychain namespace, which is
    /// why it is restricted to characters that are safe in both.
    pub name: String,
    pub version: String,
    pub description: String,
    pub capabilities: Vec<Capability>,
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
        let memory_mb = raw.memory_mb.unwrap_or(32);
        if memory_mb > MAX_MEMORY_MB {
            return Err(ManifestError::MemoryTooLarge(memory_mb));
        }

        let capabilities = raw
            .capabilities
            .iter()
            .map(|text| parse_capability(text))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Manifest {
            name: raw.name,
            version: raw.version,
            description: raw.description,
            capabilities,
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
}

/// Is this usable as a directory name and a keychain namespace?
///
/// Strict on purpose. The name reaches a filesystem path and a platform
/// credential store, and both have their own opinions about separators, case
/// and length — so the intersection of what is safe everywhere is what is
/// allowed, rather than what happens to work on the developer's machine.
fn is_usable_name(name: &str) -> bool {
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

fn parse_capability(text: &str) -> Result<Capability, ManifestError> {
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

fn describe(capability: &Capability) -> String {
    match capability {
        Capability::Timer => "update itself on a schedule".to_string(),
        Capability::Net(host) => format!("talk to {host} over HTTPS"),
        Capability::NetWildcard(domain) => {
            format!("talk to any subdomain of {domain} over HTTPS")
        }
        Capability::Fs(path) => format!("read and write `{path}` in its own folder"),
        Capability::Secrets => "store passwords in your keychain".to_string(),
        Capability::Locale => "format dates and times for your locale".to_string(),
        Capability::Measure => "measure text".to_string(),
        Capability::Command(name) => format!("run the `{name}` command"),
        Capability::Metrics => "read CPU, memory and battery counters".to_string(),
    }
}

#[cfg(test)]
mod tests;
