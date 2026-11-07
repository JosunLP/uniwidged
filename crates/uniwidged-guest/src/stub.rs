//! The host, as it looks when there is no host.
//!
//! A widget's own unit tests run natively, where there is no component and no
//! capabilities. Every method here answers the way an ungranted one does on a
//! real host, so a widget that handles refusal properly — and every widget
//! should, because a user can decline anything — is exercised by its own tests
//! rather than only in production.

/// The host, as a widget sees it.
#[derive(Debug, Default)]
pub struct Cx {
    _private: (),
}

/// What a finished command left behind. See the on-target version for what
/// each field means; off-target nothing ever produces one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim_end().to_string()
    }

    pub fn succeeded(&self) -> bool {
        self.status == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskResult {
    Command(CommandOutput),
    Http(HttpResponse),
    Failed(String),
}

/// What a battery is doing.
///
/// Three states rather than the kernel's five: "full" and "not charging" are
/// the same sentence to somebody glancing at a widget — plugged in, and not
/// gaining — and so is a battery held below full by a charge limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    /// Plugged in, and gaining.
    Charging,
    /// Running on its own charge.
    Discharging,
    /// Plugged in and not gaining.
    Idle,
}

/// The machine's batteries, as one. See [`Cx::battery`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    /// `0.0..=1.0` of what the battery holds now. A worn battery that is
    /// full reads `1.0`.
    pub charge: f32,
    pub state: PowerState,
    /// Minutes until empty while discharging, until full while charging.
    /// `None` where the kernel reports no rate — common for the first seconds
    /// after a cable moves, and always when idle. Show nothing rather than a
    /// guess.
    pub minutes_left: Option<u32>,
}

/// Bytes per second, summed over every interface but loopback. See
/// [`Cx::network_throughput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Throughput {
    pub received: u64,
    pub sent: u64,
}

/// One of your widget's own settings, as your manifest declared it and your
/// user set it.
///
/// There is no `Choice` variant: a choice's value *is* text, and the set it
/// has to come from is your manifest's business. The host has already checked
/// it, so matching on the strings you listed needs no arm for one you did not.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Text(String),
    Number(f64),
    Toggle(bool),
}

impl SettingValue {
    /// The text of a `text` or `choice` setting, or `None` for the others.
    ///
    /// A convenience for the common shape — `matches!` on a variant you
    /// declared is the honest read, and this is what saves writing it for a
    /// choice you are about to compare against a literal.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            SettingValue::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            SettingValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_toggle(&self) -> Option<bool> {
        match self {
            SettingValue::Toggle(toggle) => Some(*toggle),
            _ => None,
        }
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    /// Your manifest never asked for this.
    NotGranted,
    Malformed(String),
    /// Granted, but this host cannot serve it.
    Unavailable(String),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denied::NotGranted => write!(f, "not granted"),
            Denied::Malformed(why) => write!(f, "malformed: {why}"),
            Denied::Unavailable(why) => write!(f, "unavailable: {why}"),
        }
    }
}

impl Cx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format_time(&self, _epoch_seconds: i64) -> Result<String, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn format_date(&self, _epoch_seconds: i64) -> Result<String, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn measure_text(&self, _text: &str, _role: uniwidged_view::Role) -> Result<f32, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn fetch(&self, _url: &str) -> Result<u32, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn run_command(&self, _name: &str, _args: &[&str]) -> Result<u32, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn take_result(&self, _token: u32) -> Result<TaskResult, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn read_file(&self, _path: &str) -> Result<Option<Vec<u8>>, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn write_file(&self, _path: &str, _bytes: &[u8]) -> Result<(), Denied> {
        Err(Denied::NotGranted)
    }

    pub fn get_secret(&self, _key: &str) -> Result<Option<Vec<u8>>, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn set_secret(&self, _key: &str, _value: &[u8]) -> Result<(), Denied> {
        Err(Denied::NotGranted)
    }

    pub fn remove_secret(&self, _key: &str) -> Result<bool, Denied> {
        Err(Denied::NotGranted)
    }

    /// Off-target there is no host and therefore no configuration, which
    /// reads to a widget exactly like a manifest that declared nothing. A
    /// widget's own tests can still exercise the branch by constructing its
    /// state directly.
    pub fn setting(&self, _key: &str) -> Option<SettingValue> {
        None
    }

    pub fn cpu_load(&self) -> Result<f32, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn memory_load(&self) -> Result<f32, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn battery(&self) -> Result<Option<Battery>, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn disk_load(&self) -> Result<f32, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn network_throughput(&self) -> Result<Throughput, Denied> {
        Err(Denied::NotGranted)
    }

    /// Goes nowhere off-target, which is what a test wants.
    pub fn log(&self, _message: &str) {}

    /// Off-target there is nothing to draw on, which reads to a widget as a
    /// host that could not take the image — the same branch a widget needs
    /// for a PNG it fetched that turned out not to be one.
    pub fn register_image(&self, _png: &[u8]) -> Result<uniwidged_view::ImageHandle, Denied> {
        Err(Denied::Unavailable("no host off-target".into()))
    }

    pub fn release_image(&self, _handle: uniwidged_view::ImageHandle) -> bool {
        false
    }
}
