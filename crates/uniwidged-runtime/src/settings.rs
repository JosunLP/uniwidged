//! What a widget can be configured with.
//!
//! `Event::Configured` has been in `wit/uniwidged.wit` since the first version
//! and nothing has ever produced it, for the plain reason that there was
//! nothing to configure. This is that — the same shape of gap `Event::
//! Delivered` was before M7, and closed the same way: by building the thing
//! the variant was waiting for rather than by deleting the variant.
//!
//! ## The manifest declares; the host validates; the widget reads
//!
//! A widget does not parse its own configuration. It declares what it takes:
//!
//! ```toml
//! [[setting]]
//! key = "warn-at"
//! label = "Warn above"
//! type = "number"
//! default = 0.8
//! min = 0.0
//! max = 1.0
//! ```
//!
//! and reads it back as a [`SettingValue`] of exactly the declared type. That
//! division is the whole design, and it buys three things a string map does
//! not:
//!
//! * **A widget can never receive a value it cannot handle.** The host checks
//!   the type, the range and the option list before a widget is started, so
//!   there is no branch for "the user typed `banana` into a number" — the user
//!   was told at the moment they typed it.
//! * **The user can be shown a form.** A `toggle` is a switch, a `choice` is a
//!   list, and a `number` with bounds is a slider. None of that is derivable
//!   from a widget that parses its own strings.
//! * **A stale key is visible.** The declared set is the authority, so a
//!   configuration holding something the manifest no longer declares is a fact
//!   somebody can be told about — see [`Settings::resolve`], which reports
//!   rather than either dropping silently or refusing to start.
//!
//! ## Why this is not a capability
//!
//! A widget's own settings are not something it reaches *out* for. There is no
//! argument through which to name another widget's, the values came from this
//! widget's own manifest, and a user who declined it would be declining the
//! widget rather than a permission. `log` is the other function with the same
//! shape and the same answer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most settings one widget may declare.
///
/// Not a limit anybody legitimately reaches — a widget has a handful of knobs
/// — and it exists because the list is rendered as a form and stored in the
/// desktop's own configuration file.
pub const MAX_SETTINGS: usize = 32;

/// The longest a text setting's value may be.
pub const MAX_TEXT_BYTES: usize = 1024;

/// The longest a key or a label may be.
pub const MAX_KEY_LEN: usize = 32;
pub const MAX_LABEL_LEN: usize = 64;

/// One value a widget was configured with.
///
/// `choice` is deliberately absent: a choice's value *is* text, and the set it
/// has to come from is the manifest's business. A widget matching on the
/// string it gets back needs no arm for a value that is not one of its own
/// options, because it cannot be given one.
///
/// Serialised untagged, so `config.toml` holds `warn-at = 0.8` rather than
/// `warn-at = { number = 0.8 }`. The file is meant to be hand-edited, and a
/// tagged encoding of three primitives is a file nobody wants to edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    // Order matters for an untagged enum: `true` must be read as a toggle
    // rather than coerced, and a number must not be read as text.
    Toggle(bool),
    Number(f64),
    Text(String),
}

impl PartialEq for SettingValue {
    /// Hand-written because `f64` is not `Eq`. Two numbers that are both `NaN`
    /// are not equal, which is correct and is also unreachable: a `NaN` cannot
    /// be written in TOML and is refused by [`Setting::check`].
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SettingValue::Toggle(a), SettingValue::Toggle(b)) => a == b,
            (SettingValue::Number(a), SettingValue::Number(b)) => a == b,
            (SettingValue::Text(a), SettingValue::Text(b)) => a == b,
            _ => false,
        }
    }
}

impl SettingValue {
    /// The word the manifest would use for this shape, for an error message.
    pub fn type_name(&self) -> &'static str {
        match self {
            SettingValue::Text(_) => "text",
            SettingValue::Number(_) => "number",
            SettingValue::Toggle(_) => "toggle",
        }
    }

    /// How the value is written in a configuration file and on a terminal.
    pub fn display(&self) -> String {
        match self {
            SettingValue::Text(text) => text.clone(),
            // `0.8` rather than `0.8000000000000000444`: the shortest string
            // that round-trips, which is what `{}` on an `f64` gives.
            SettingValue::Number(number) => format!("{number}"),
            SettingValue::Toggle(toggle) => toggle.to_string(),
        }
    }
}

/// What kind of thing a setting is, and what values it accepts.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingKind {
    Text,
    /// Bounds are part of the declaration rather than the widget's own check,
    /// because the user is told at the moment they type — and because a
    /// bounded number is a slider and an unbounded one is a text box.
    Number {
        min: f64,
        max: f64,
    },
    Toggle,
    Choice {
        options: Vec<String>,
    },
}

impl SettingKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            SettingKind::Text => "text",
            SettingKind::Number { .. } => "number",
            SettingKind::Toggle => "toggle",
            SettingKind::Choice { .. } => "choice",
        }
    }
}

/// One setting a widget declared.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub key: String,
    /// What the user is shown. Defaults to the key, because a widget that did
    /// not write one should still produce a usable form rather than an empty
    /// label.
    pub label: String,
    pub kind: SettingKind,
    /// What the widget gets when the user has set nothing, which is the
    /// overwhelmingly common case. Every declared key therefore always has a
    /// value, which is why `setting` returning `none` means "never declared"
    /// and nothing else.
    pub default: SettingValue,
}

/// Why a setting was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingError(pub String);

impl std::fmt::Display for SettingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SettingError {}

impl Setting {
    /// Is this a value this setting may hold?
    ///
    /// The one place the question is answered, called by the manifest parser
    /// on the declared default and by the host on everything a user sets — so
    /// a default that would be refused if a user typed it is refused when its
    /// author writes it, which is where the author can still fix it.
    pub fn check(&self, value: &SettingValue) -> Result<(), SettingError> {
        let wrong_type = || {
            SettingError(format!(
                "`{}` is {}, and that is {}",
                self.key,
                self.kind.type_name(),
                value.type_name()
            ))
        };
        match (&self.kind, value) {
            (SettingKind::Text, SettingValue::Text(text)) => {
                if text.len() > MAX_TEXT_BYTES {
                    return Err(SettingError(format!(
                        "`{}` may be at most {MAX_TEXT_BYTES} bytes",
                        self.key
                    )));
                }
                Ok(())
            }
            (SettingKind::Toggle, SettingValue::Toggle(_)) => Ok(()),
            (SettingKind::Number { min, max }, SettingValue::Number(number)) => {
                // `NaN` fails both comparisons, which is the answer that keeps
                // every later comparison meaningful.
                if !(number >= min && number <= max) {
                    return Err(SettingError(format!(
                        "`{}` must be between {min} and {max}, and that is {number}",
                        self.key
                    )));
                }
                Ok(())
            }
            (SettingKind::Choice { options }, SettingValue::Text(text)) => {
                if !options.iter().any(|option| option == text) {
                    return Err(SettingError(format!(
                        "`{}` must be one of {}, and that is `{text}`",
                        self.key,
                        options.join(", ")
                    )));
                }
                Ok(())
            }
            _ => Err(wrong_type()),
        }
    }

    /// Reads a value from what somebody typed on a command line.
    ///
    /// A command line has only strings, so this is where `"true"` becomes a
    /// toggle and `"0.8"` a number — against the *declared* type, so `--set
    /// warn-at=true` is refused as the wrong type rather than stored as the
    /// text `"true"`. A text setting takes whatever it was given, including
    /// something that looks like a number, because that is what text means.
    pub fn parse(&self, text: &str) -> Result<SettingValue, SettingError> {
        let value = match &self.kind {
            SettingKind::Text | SettingKind::Choice { .. } => SettingValue::Text(text.to_string()),
            SettingKind::Toggle => match text {
                // Only these. `yes`, `1` and `on` all read as true to somebody
                // and as a typo to somebody else, and a configuration that
                // guesses is one nobody can predict.
                "true" => SettingValue::Toggle(true),
                "false" => SettingValue::Toggle(false),
                other => {
                    return Err(SettingError(format!(
                        "`{}` is a toggle: it is `true` or `false`, not `{other}`",
                        self.key
                    )));
                }
            },
            SettingKind::Number { .. } => match text.parse::<f64>() {
                Ok(number) if number.is_finite() => SettingValue::Number(number),
                _ => {
                    return Err(SettingError(format!(
                        "`{}` is a number, and `{text}` is not one",
                        self.key
                    )));
                }
            },
        };
        self.check(&value)?;
        Ok(value)
    }

    /// The line `uniwidged configure` prints for this setting.
    pub fn describe(&self) -> String {
        match &self.kind {
            SettingKind::Text => "text".to_string(),
            SettingKind::Toggle => "true or false".to_string(),
            SettingKind::Number { min, max } => format!("a number from {min} to {max}"),
            SettingKind::Choice { options } => format!("one of {}", options.join(", ")),
        }
    }
}

/// What one widget is actually running with.
///
/// Shared with the running instance rather than copied into it, so that a
/// change reaches a widget that is already started. The alternative is
/// restarting the widget to reconfigure it, which throws away everything it
/// had loaded to change a number.
///
/// `Arc<Mutex<_>>` for the reason the rest of this crate uses one: Wasmtime
/// requires a store's data to be `Send`, and a claim the compiler checks is
/// worth more than one a comment makes. Nothing crosses a thread — there is
/// one loop — and the lock is never contended.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    values: std::sync::Arc<std::sync::Mutex<BTreeMap<String, SettingValue>>>,
}

impl Settings {
    /// A widget with nothing configured. Every lookup answers `None`, which a
    /// widget reads as "never declared" — correct, because nothing was.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn new(values: BTreeMap<String, SettingValue>) -> Self {
        Self {
            values: std::sync::Arc::new(std::sync::Mutex::new(values)),
        }
    }

    pub fn get(&self, key: &str) -> Option<SettingValue> {
        self.values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// Replaces every value at once.
    ///
    /// All of them together rather than one at a time, because a widget reads
    /// its settings inside one `view` and two of them changing should not be
    /// two frames — and because a half-applied configuration is a state
    /// nobody designed.
    pub fn replace(&self, values: BTreeMap<String, SettingValue>) {
        *self.values.lock().unwrap_or_else(|e| e.into_inner()) = values;
    }

    pub fn snapshot(&self) -> BTreeMap<String, SettingValue> {
        self.values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The values a widget should run with, and what was wrong with the file.
    ///
    /// Every declared key gets a value: the user's if there is a usable one,
    /// the manifest's default otherwise. Nothing else gets one.
    ///
    /// **Complaints are reported rather than fatal**, and that is a deliberate
    /// difference from an over-grant, which refuses. An over-grant is a
    /// security fact — the user approved a list and the file exceeds it — and
    /// obliging it would hand a widget something nobody agreed to. A stale
    /// setting is an *upgrade artefact*: a widget dropped a knob in its new
    /// version, and refusing to start the desktop over that would make every
    /// upgrade a support question. So the key is ignored, the default is used,
    /// and somebody is told.
    ///
    /// Nothing is written back. The user's file keeps what they put in it
    /// until they change it, because a desktop that silently deletes a line
    /// somebody typed is worse than one that explains why it is ignoring it.
    pub fn resolve(
        declared: &[Setting],
        configured: &BTreeMap<String, SettingValue>,
    ) -> (BTreeMap<String, SettingValue>, Vec<String>) {
        let mut resolved = BTreeMap::new();
        let mut complaints = Vec::new();

        for setting in declared {
            let value = match configured.get(&setting.key) {
                None => setting.default.clone(),
                Some(value) => match setting.check(value) {
                    Ok(()) => value.clone(),
                    Err(why) => {
                        complaints.push(format!("{why}; using the default instead"));
                        setting.default.clone()
                    }
                },
            };
            resolved.insert(setting.key.clone(), value);
        }

        for key in configured.keys() {
            if !declared.iter().any(|setting| &setting.key == key) {
                complaints.push(format!(
                    "`{key}` is set and this widget does not declare it; ignored"
                ));
            }
        }
        (resolved, complaints)
    }
}

/// Is this usable as a setting's key?
///
/// The same alphabet a widget's name uses, and for a related reason: the key
/// is a bare TOML key in a file people hand-edit and an argument on a command
/// line, and the intersection of what is unambiguous in both is small.
pub fn is_usable_key(key: &str) -> bool {
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return false;
    }
    if !key.starts_with(|c: char| c.is_ascii_lowercase()) || key.ends_with('-') {
        return false;
    }
    key.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests;
