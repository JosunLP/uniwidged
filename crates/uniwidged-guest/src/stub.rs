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

    pub fn take_response(&self, _token: u32) -> Result<Vec<u8>, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn read_file(&self, _path: &str) -> Result<Option<Vec<u8>>, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn write_file(&self, _path: &str, _bytes: &[u8]) -> Result<(), Denied> {
        Err(Denied::NotGranted)
    }

    pub fn get_secret(&self, _key: &str) -> Result<Vec<u8>, Denied> {
        Err(Denied::NotGranted)
    }

    pub fn set_secret(&self, _key: &str, _value: &[u8]) -> Result<(), Denied> {
        Err(Denied::NotGranted)
    }

    pub fn cpu_load(&self) -> Result<f32, Denied> {
        Err(Denied::NotGranted)
    }

    /// Goes nowhere off-target, which is what a test wants.
    pub fn log(&self, _message: &str) {}
}
