//! What the host can actually serve.
//!
//! [`Policy`](crate::capability::Policy) decides whether a widget *may* do
//! something; this decides whether the host *can*. The two are deliberately
//! separate, and the separation is visible to a widget: a capability it never
//! declared answers `not-granted`, and one it declared and the user approved
//! but this host has nothing behind answers `unavailable`. A widget that
//! conflated them would send its author to fix a manifest that is already
//! right.
//!
//! So [`Services`] is what an embedder hands the runtime, and every field is
//! optional. A test that supplies nothing gets the same sandbox a desktop
//! does, minus the storage — which is why most of this crate's tests can go on
//! constructing one with [`Services::none`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything the host offers on a widget's behalf.
///
/// Cheap to clone: the supervisor keeps one per widget and hands a copy to
/// each fresh instance, because a widget that restarts must find its data
/// where it left it.
///
/// The shared handles are `Arc`, and the types behind them are `Sync`, because
/// Wasmtime requires a store's data to be `Send`. Nothing here actually
/// crosses a thread — one process, one loop, `ARCHITECTURE.md` §5, and the
/// locks are never contended — but a claim the compiler checks is worth more
/// than one it does not.
#[derive(Clone, Default)]
pub struct Services {
    /// The widget's own directory. `None` makes `fs` answer `unavailable`.
    pub data: Option<DataDir>,
    /// How to write a date or a time. `None` makes `locale` answer
    /// `unavailable`.
    pub locale: Option<Locale>,
    /// The host's fonts, for a widget's own truncation decisions.
    pub measure: Option<Arc<dyn TextMetrics>>,
    /// Machine counters. Shared by every widget on the host, because it is one
    /// machine — and because two widgets sampling separately would each see
    /// half the window and report noise.
    pub metrics: Option<Arc<CpuSampler>>,
}

impl std::fmt::Debug for Services {
    /// Hand-written because a measurer is a trait object. It still says
    /// whether one is present, which is the whole of what a reader wants from
    /// this type in a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("data", &self.data)
            .field("locale", &self.locale)
            .field("measure", &self.measure.is_some())
            .field("metrics", &self.metrics.is_some())
            .finish()
    }
}

impl Services {
    /// A host that can serve nothing. Every granted capability answers
    /// `unavailable`, which is what the conformance suite asserts.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_data_dir(mut self, dir: DataDir) -> Self {
        self.data = Some(dir);
        self
    }

    pub fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = Some(locale);
        self
    }

    pub fn with_text_metrics(mut self, metrics: Arc<dyn TextMetrics>) -> Self {
        self.measure = Some(metrics);
        self
    }

    pub fn with_metrics(mut self, sampler: Arc<CpuSampler>) -> Self {
        self.metrics = Some(sampler);
        self
    }
}

/// How wide a string is, as the host's own layout would measure it.
///
/// A trait rather than a concrete type because the fonts live two crates away,
/// in `uniwidged-compose`, and the sandbox must not depend on the renderer.
/// The host owns both and joins them.
///
/// The answer has to be the number the compose pass would arrive at — same
/// font, same size, same weight, same digit spacing — or a widget deciding its
/// own truncation is deciding against a guess. `uniwidged-compose` has a test
/// that the two agree.
pub trait TextMetrics: Send + Sync {
    /// Logical pixels. Never negative; zero for an empty string.
    fn width(&self, text: &str, role: uniwidged_view::Role) -> f32;
}

// ---------------------------------------------------------------------------
// The data directory
// ---------------------------------------------------------------------------

/// The most one widget may put in one file, unless the embedder says otherwise.
///
/// Not a disk quota — it is the bound on what one host call can allocate.
/// Without it a widget could ask the host to read a file somebody else put in
/// its directory and make the *host* allocate a gigabyte, which no guest
/// memory cap would catch.
pub const DEFAULT_MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

/// One widget's own directory on disk.
///
/// The path a widget asks for is relative and stays relative: it is refused
/// unless [`crate::capability::is_contained`] accepts it, joined onto this
/// root, and then checked *again* after the filesystem has resolved it. The
/// second check is the one that matters, because the first cannot see a
/// symlink — and a widget's directory is somewhere a user can reach with a
/// file manager.
#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
    max_file_bytes: usize,
}

/// Why a read or a write did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The path resolved outside the widget's own directory. Only reachable
    /// through a symlink, since the textual check runs first.
    Escape,
    TooLarge {
        limit: usize,
    },
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Escape => write!(f, "that path leaves the widget's own directory"),
            StoreError::TooLarge { limit } => {
                write!(f, "a widget's file may be at most {limit} bytes")
            }
            StoreError::Io(why) => write!(f, "{why}"),
        }
    }
}

impl DataDir {
    /// The directory is not created here. A widget that only ever reads should
    /// leave nothing behind, so the first *write* creates it.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }

    pub fn with_max_file_bytes(mut self, bytes: usize) -> Self {
        self.max_file_bytes = bytes;
        self
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn max_file_bytes(&self) -> usize {
        self.max_file_bytes
    }

    /// Reads a file. `Ok(None)` means it is not there.
    ///
    /// Absence is not an error, and saying so in the type is the point: the
    /// first run of a caching widget finds nothing, and that is the normal
    /// case rather than the failure case.
    pub fn read(&self, relative: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if !crate::capability::is_contained(relative) {
            return Err(StoreError::Escape);
        }
        let Some(root) = existing_root(&self.root)? else {
            // Nothing has ever been written, so nothing can be read. Reporting
            // this as I/O would make a widget's first run look broken.
            return Ok(None);
        };
        let path = root.join(relative);
        let resolved = match path.canonicalize() {
            Ok(resolved) => resolved,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::Io(e.to_string())),
        };
        if !resolved.starts_with(&root) {
            return Err(StoreError::Escape);
        }
        let length = std::fs::metadata(&resolved)
            .map_err(|e| StoreError::Io(e.to_string()))?
            .len();
        if length > self.max_file_bytes as u64 {
            return Err(StoreError::TooLarge {
                limit: self.max_file_bytes,
            });
        }
        match std::fs::read(&resolved) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Io(e.to_string())),
        }
    }

    /// Writes a file, creating the directory and any parents on the way.
    ///
    /// Through a temporary file and a rename, so that a widget interrupted at
    /// its deadline — which happens, by design — cannot leave half a JSON
    /// document where its own next run will try to parse one.
    pub fn write(&self, relative: &str, bytes: &[u8]) -> Result<(), StoreError> {
        if !crate::capability::is_contained(relative) {
            return Err(StoreError::Escape);
        }
        if bytes.len() > self.max_file_bytes {
            return Err(StoreError::TooLarge {
                limit: self.max_file_bytes,
            });
        }
        std::fs::create_dir_all(&self.root).map_err(|e| StoreError::Io(e.to_string()))?;
        let root = self
            .root
            .canonicalize()
            .map_err(|e| StoreError::Io(e.to_string()))?;

        let path = root.join(relative);
        let (parent, name) = match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => (parent.to_path_buf(), name.to_os_string()),
            // `is_contained` above refuses everything with no final
            // component, so this is unreachable rather than merely unlikely.
            _ => return Err(StoreError::Escape),
        };
        std::fs::create_dir_all(&parent).map_err(|e| StoreError::Io(e.to_string()))?;
        let parent = parent
            .canonicalize()
            .map_err(|e| StoreError::Io(e.to_string()))?;
        if !parent.starts_with(&root) {
            return Err(StoreError::Escape);
        }

        let mut temporary = parent.join(&name);
        temporary.as_mut_os_string().push(".uniwidged-tmp");
        // A previous interruption may have left one, and writing through a
        // stale symlink is exactly what the checks above exist to prevent.
        let _ = std::fs::remove_file(&temporary);
        std::fs::write(&temporary, bytes).map_err(|e| StoreError::Io(e.to_string()))?;
        std::fs::rename(&temporary, parent.join(&name)).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            StoreError::Io(e.to_string())
        })
    }
}

/// The root, resolved, or `None` if it does not exist yet.
fn existing_root(root: &Path) -> Result<Option<PathBuf>, StoreError> {
    match root.canonicalize() {
        Ok(root) => Ok(Some(root)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io(e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Machine counters
// ---------------------------------------------------------------------------

/// How busy the machine's CPUs have been.
///
/// ## Why it samples rather than measures
///
/// There is no such thing as instantaneous CPU load: the kernel counts ticks,
/// and a load is the difference between two counts divided by the time between
/// them. So the first call has no previous count to subtract and answers with
/// the average since boot — a real number, and the only honest one available —
/// and every call after it answers for the window since the last sample.
///
/// **It never blocks.** The alternative implementation sleeps 100 ms between
/// two reads, which on this host would mean a widget holding its deadline open
/// inside `view()`. See `instance.rs`: that is the one thing a host function
/// may not do.
///
/// A second call arriving sooner than [`MIN_WINDOW`] gets the previous answer
/// rather than a new one computed over three milliseconds, which would be
/// noise rather than a measurement.
pub struct CpuSampler {
    last: Mutex<Option<Sample>>,
}

/// The shortest window worth dividing by.
///
/// Below this the delta is a handful of ticks and the answer swings between 0
/// and 1 depending on where the sample landed. Eight widgets asking in the
/// same turn of the loop all see one measurement, which is also what they
/// should see: it is one machine.
const MIN_WINDOW: Duration = Duration::from_millis(250);

struct Sample {
    at: Instant,
    busy: u64,
    total: u64,
    /// What was reported for this sample, so a repeat within [`MIN_WINDOW`]
    /// answers consistently rather than recomputing over nothing.
    load: f32,
}

impl Default for CpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuSampler {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    /// Busy fraction of all CPUs, from 0.0 to 1.0.
    ///
    /// `Err` carries something a user could act on, and is what a platform
    /// with no counters wired up answers.
    pub fn load(&self) -> Result<f32, String> {
        let (busy, total) = read_cpu_counters()?;
        let now = Instant::now();
        let mut last = self
            .last
            .lock()
            .expect("the sampler lock is never held across a panic");

        let load = match last.as_ref() {
            // Asked again too soon. The previous answer is a measurement; a
            // new one over three milliseconds would not be.
            Some(previous) if now.duration_since(previous.at) < MIN_WINDOW => previous.load,
            Some(previous) => {
                let elapsed = total.saturating_sub(previous.total);
                let working = busy.saturating_sub(previous.busy);
                // A counter that did not advance — possible if the clock tick
                // has not landed — leaves the answer where it was rather than
                // dividing by zero.
                if elapsed == 0 {
                    previous.load
                } else {
                    (working as f32 / elapsed as f32).clamp(0.0, 1.0)
                }
            }
            // The first call. Since boot is the only window there is, and it
            // is a real one.
            None if total > 0 => (busy as f32 / total as f32).clamp(0.0, 1.0),
            None => 0.0,
        };

        *last = Some(Sample {
            at: now,
            busy,
            total,
            load,
        });
        Ok(load)
    }
}

/// `(busy, total)` jiffies across all CPUs.
#[cfg(target_os = "linux")]
fn read_cpu_counters() -> Result<(u64, u64), String> {
    let stat = std::fs::read_to_string("/proc/stat").map_err(|e| format!("/proc/stat: {e}"))?;
    let line = stat
        .lines()
        .next()
        .ok_or_else(|| "/proc/stat is empty".to_string())?;
    parse_cpu_line(line).ok_or_else(|| format!("/proc/stat begins with {line:?}"))
}

/// Platforms whose counters are not wired up say so rather than answering
/// zero, which a widget would draw as an idle machine.
#[cfg(not(target_os = "linux"))]
fn read_cpu_counters() -> Result<(u64, u64), String> {
    Err(format!(
        "this host reads no CPU counters on {}",
        std::env::consts::OS
    ))
}

/// The aggregate line of `/proc/stat`, as `(busy, total)`.
///
/// ```text
/// cpu  user nice system idle iowait irq softirq steal guest guest_nice
/// ```
///
/// `idle` and `iowait` are the not-busy fields; everything else counts as
/// work. `guest` and `guest_nice` are **already included** in `user` and
/// `nice` — adding them again is the classic way to report 120 % load on a
/// machine running virtual machines — so the total is taken over the first
/// eight fields only.
fn parse_cpu_line(line: &str) -> Option<(u64, u64)> {
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields
        .take(8)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    // user, nice, system, idle — the four every kernel since 2.6 reports. The
    // rest are optional and simply absent on an old or unusual one.
    if values.len() < 4 {
        return None;
    }
    let total: u64 = values.iter().sum();
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((total.saturating_sub(idle), total))
}

// ---------------------------------------------------------------------------
// Dates and times
// ---------------------------------------------------------------------------

/// Whether a clock reads 13:00 or 1:00 PM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HourCycle {
    H23,
    H12,
}

/// The order a date's parts are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    /// `2026-08-28`. Also the fallback for an unknown locale, because it is
    /// the one order nobody misreads.
    Ymd,
    /// `28/08/2026`.
    Dmy,
    /// `08/28/2026`.
    Mdy,
}

/// How this desktop writes a date and a time.
///
/// ## Deliberately small
///
/// This is not CLDR and does not pretend to be. It settles the three things
/// that make a formatted date wrong rather than merely unidiomatic — the UTC
/// offset, the hour cycle and the field order — and stops there, in numerals,
/// with no month names and no weekday names in any language.
///
/// The alternative was a full ICU dependency with its locale data, for a
/// framework whose first four widgets show a clock and a number. When a widget
/// needs "Freitag, 28. August", this type is what grows; nothing above it has
/// to change, because a widget never sees the parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// Seconds to add to UTC. Fixed at the moment it is determined, so a
    /// desktop that crosses a daylight-saving boundary is told about it the
    /// same way it is told about a theme change — see `Event::EnvironmentChanged`.
    pub utc_offset_seconds: i32,
    pub hour_cycle: HourCycle,
    pub date_order: DateOrder,
}

impl Default for Locale {
    /// UTC, 24-hour, ISO order. What a test wants and what a host falls back
    /// to when the environment says nothing.
    fn default() -> Self {
        Self {
            utc_offset_seconds: 0,
            hour_cycle: HourCycle::H23,
            date_order: DateOrder::Ymd,
        }
    }
}

impl Locale {
    /// What this machine's environment asks for.
    ///
    /// The offset comes from the C library, so it is the one the rest of the
    /// desktop uses, including daylight saving. The conventions come from
    /// `LC_TIME`/`LC_ALL`/`LANG` through the coarse table in
    /// [`Locale::from_lang`].
    pub fn system() -> Self {
        let lang = ["LC_ALL", "LC_TIME", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok())
            .filter(|value| !value.is_empty());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self {
            utc_offset_seconds: system_utc_offset(now),
            ..Self::from_lang(lang.as_deref())
        }
    }

    /// The conventions of a POSIX locale name, at UTC.
    ///
    /// A coarse table by language, with the one region split that matters
    /// often enough to be worth it: `en_US` writes the month first and counts
    /// to twelve, and no other English-speaking region does either.
    /// Everything unrecognised falls back to day-month-year, which is what
    /// most of the world writes — except that `C` and `POSIX`, which mean
    /// "make no assumptions", get ISO order instead.
    pub fn from_lang(lang: Option<&str>) -> Self {
        let Some(lang) = lang else {
            return Self::default();
        };
        // `de_DE.UTF-8@euro` → `de_DE`.
        let tag = lang
            .split(['.', '@'])
            .next()
            .unwrap_or(lang)
            .replace('-', "_");
        if tag == "C" || tag == "POSIX" || tag.is_empty() {
            return Self::default();
        }
        if tag == "en" || tag.starts_with("en_US") {
            return Self {
                hour_cycle: HourCycle::H12,
                date_order: DateOrder::Mdy,
                ..Self::default()
            };
        }
        let language = tag.split('_').next().unwrap_or(&tag);
        let date_order = match language {
            // The languages whose everyday written date is year-first.
            "ja" | "zh" | "ko" | "hu" | "lt" | "sv" => DateOrder::Ymd,
            _ => DateOrder::Dmy,
        };
        Self {
            hour_cycle: HourCycle::H23,
            date_order,
            ..Self::default()
        }
    }

    /// `07:05`, or `7:05 AM`.
    ///
    /// No seconds: a widget showing them asks to be woken sixty times as
    /// often, and `ARCHITECTURE.md` §5 is about what that costs. A widget that
    /// genuinely wants them can format its own.
    pub fn format_time(&self, epoch_seconds: i64) -> String {
        let (_, _, _, hour, minute, _) = self.civil(epoch_seconds);
        match self.hour_cycle {
            HourCycle::H23 => format!("{hour:02}:{minute:02}"),
            HourCycle::H12 => {
                let suffix = if hour < 12 { "AM" } else { "PM" };
                let twelve = match hour % 12 {
                    0 => 12,
                    other => other,
                };
                format!("{twelve}:{minute:02} {suffix}")
            }
        }
    }

    /// `2026-08-28`, `28/08/2026` or `08/28/2026`.
    pub fn format_date(&self, epoch_seconds: i64) -> String {
        let (year, month, day, _, _, _) = self.civil(epoch_seconds);
        match self.date_order {
            DateOrder::Ymd => format!("{year:04}-{month:02}-{day:02}"),
            DateOrder::Dmy => format!("{day:02}/{month:02}/{year:04}"),
            DateOrder::Mdy => format!("{month:02}/{day:02}/{year:04}"),
        }
    }

    /// Year, month, day, hour, minute, second — in this locale's own offset.
    pub fn civil(&self, epoch_seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
        let local = epoch_seconds.saturating_add(self.utc_offset_seconds as i64);
        // Euclidean, not truncating: an instant before 1970 must land on the
        // day before, and `-1 / 86400` is zero.
        let days = local.div_euclid(86_400);
        let seconds = local.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        (
            year,
            month,
            day,
            (seconds / 3600) as u32,
            ((seconds % 3600) / 60) as u32,
            (seconds % 60) as u32,
        )
    }
}

/// Days since 1970-01-01 to a calendar date, proleptic Gregorian.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every day this can
/// be handed and has no branches for leap years — the era arithmetic makes
/// them fall out. Worth using rather than deriving: the obvious version is
/// wrong in 2100, and nobody's test suite runs in 2100.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153; // [0, 11], March-based
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// This machine's offset from UTC at an instant, in seconds.
///
/// Through the C library rather than by reading `/etc/localtime`: the point is
/// to agree with everything else on the desktop, daylight saving and all, and
/// `localtime_r` is what everything else uses.
#[cfg(unix)]
fn system_utc_offset(epoch_seconds: i64) -> i32 {
    let mut out: libc::tm = unsafe { std::mem::zeroed() };
    // `time_t` is 64-bit here and 32-bit on platforms this does not build for,
    // so the cast is a no-op on the ones that matter and not redundant.
    let time = epoch_seconds as libc::time_t;
    // SAFETY: `localtime_r` writes into a `tm` this function owns and reads a
    // `time_t` passed by value; both outlive the call and neither is aliased.
    // It initialises the zone from `TZ` on first use — glibc and musl both do,
    // and there is no `tzset` in the `libc` crate to call explicitly — so the
    // environment is read once per process. Nothing here writes to the
    // environment, so the `tzset`/`setenv` data race is not reachable.
    let ok = unsafe { !libc::localtime_r(&time, &mut out).is_null() };
    if ok { out.tm_gmtoff as i32 } else { 0 }
}

/// Platforms whose shell is still a milestone get UTC, and say so by being
/// obviously wrong rather than plausibly wrong.
#[cfg(not(unix))]
fn system_utc_offset(_epoch_seconds: i64) -> i32 {
    0
}

#[cfg(test)]
mod tests;
