//! The machine's other counters: its battery, its disk and its network.
//!
//! `cpu-load` and `memory-load` live in [`crate::services::Counters`]; these
//! are the three ABI 9 added beside them, under the same `metrics` grant —
//! "this widget reads how the machine is doing" is still one sentence to a
//! user. They are their own module because each one is a small parser of a
//! kernel file whose format is the part worth testing, and the parsers take
//! their input as text or as a directory so that the tests can hand them a
//! laptop, a desktop and a server without owning any of the three.
//!
//! ## What none of them do
//!
//! **Block.** Every read here is a handful of small files under `/sys` or
//! `/proc`, which the kernel answers from memory, and one `statvfs`. The
//! network rate is a rate, so it needs two samples — and it takes the second
//! one at the next call rather than by sleeping, exactly as `cpu-load` does.
//!
//! **Guess.** A battery whose kernel driver reports no rate has no time
//! remaining, not an estimate; a platform with nothing wired up says so rather
//! than answering zero, which a widget would draw as an empty battery, an empty
//! disk or a silent network — each of them a claim, and a wrong one.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// The battery
// ---------------------------------------------------------------------------

/// What a battery is doing. See `power-state` in the WIT file for why there
/// are three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    Charging,
    Discharging,
    Idle,
}

/// The machine's batteries, as one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    /// `0.0..=1.0` of what the battery holds now.
    pub charge: f32,
    pub state: PowerState,
    /// Until empty or until full, where the kernel reports a rate.
    pub minutes_left: Option<u32>,
}

/// Where Linux lists power supplies.
pub const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// The machine's battery, or `None` for a machine without one.
///
/// `Err` is for a platform that reads no power supplies at all, which is
/// different from one that read them and found no battery.
pub fn battery() -> Result<Option<Battery>, String> {
    #[cfg(target_os = "linux")]
    {
        read_batteries(Path::new(POWER_SUPPLY))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(format!(
            "this host reads no battery on {}",
            std::env::consts::OS
        ))
    }
}

/// One supply's numbers, before they are combined.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Cell {
    /// What it holds now and what it holds when full, in whatever unit its
    /// driver reports — µWh or µAh, never mixed within one supply.
    now: f64,
    full: f64,
    /// The rate it is filling or emptying at, in that unit per hour. Zero
    /// where the driver reports none.
    rate: f64,
    state: Option<PowerState>,
}

/// Every battery under `root`, combined.
///
/// A directory that does not exist is a machine with no power supplies —
/// a virtual machine, a container — and that is `Ok(None)`, the same as a
/// desktop whose only supply is its mains adapter.
pub(crate) fn read_batteries(root: &Path) -> Result<Option<Battery>, String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", root.display())),
    };
    let mut cells = Vec::new();
    // Sorted, so that two batteries are always combined in the same order and
    // the answer does not depend on what order the directory listed them in.
    let mut dirs: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        if let Some(cell) = read_cell(&dir) {
            cells.push(cell);
        }
    }
    Ok(combine(&cells))
}

/// One power supply, if it is one of the machine's own batteries.
fn read_cell(dir: &Path) -> Option<Cell> {
    let read = |name: &str| -> Option<String> {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let number = |name: &str| -> Option<f64> { read(name)?.parse::<f64>().ok() };

    if read("type")? != "Battery" {
        return None;
    }
    // A wireless mouse, a headset, a game controller: `scope` is `Device`,
    // and its battery is not the machine's. A laptop whose mouse is at 10 %
    // is not a laptop that is about to turn off.
    if read("scope").as_deref() == Some("Device") {
        return None;
    }
    // A bay with nothing in it.
    if read("present").as_deref() == Some("0") {
        return None;
    }

    let state = read("status").map(|status| match status.as_str() {
        "Charging" => PowerState::Charging,
        "Discharging" => PowerState::Discharging,
        // "Full", "Not charging", and "Unknown" — which some firmware reports
        // for a battery held at a charge limit. See `power-state`.
        _ => PowerState::Idle,
    });

    // Energy (µWh, µW) where the driver has it, charge (µAh, µA) where it
    // does not. The two are never mixed: a ratio of one to the other is a
    // voltage, not a fraction.
    let (now, full, rate) = match (number("energy_now"), number("energy_full")) {
        (Some(now), Some(full)) => (now, full, number("power_now").unwrap_or(0.0)),
        _ => match (number("charge_now"), number("charge_full")) {
            (Some(now), Some(full)) => (now, full, number("current_now").unwrap_or(0.0)),
            // Neither: some drivers report only a percentage. That is a charge
            // with no rate, and it is honest to say so.
            _ => {
                let percent = number("capacity")?;
                (percent, 100.0, 0.0)
            }
        },
    };
    if full <= 0.0 {
        return None;
    }
    Some(Cell {
        now,
        full,
        // Some drivers report the discharge rate as a negative current.
        rate: rate.abs(),
        state,
    })
}

/// Several batteries as one.
///
/// Weighted by capacity rather than averaged, because a laptop with a large
/// internal battery at 90 % and a small external one at 10 % is not at 50 %.
/// The rates add for the same reason. The state is the one that matters most
/// to somebody watching: if anything is discharging the machine is on battery,
/// and if anything is charging it is plugged in and gaining.
fn combine(cells: &[Cell]) -> Option<Battery> {
    if cells.is_empty() {
        return None;
    }
    let now: f64 = cells.iter().map(|c| c.now).sum();
    let full: f64 = cells.iter().map(|c| c.full).sum();
    let rate: f64 = cells.iter().map(|c| c.rate).sum();
    let states: Vec<PowerState> = cells.iter().filter_map(|c| c.state).collect();
    let state = if states.contains(&PowerState::Discharging) {
        PowerState::Discharging
    } else if states.contains(&PowerState::Charging) {
        PowerState::Charging
    } else {
        PowerState::Idle
    };

    let charge = (now / full).clamp(0.0, 1.0) as f32;
    let hours = match state {
        _ if rate <= 0.0 => None,
        PowerState::Discharging => Some(now / rate),
        PowerState::Charging => Some((full - now).max(0.0) / rate),
        PowerState::Idle => None,
    };
    // A day is longer than any battery lasts. A number past it is a driver
    // reporting a rate of almost nothing in the instant after a cable moved,
    // and showing "4 000 hours left" would be repeating that mistake.
    let minutes_left = hours
        .map(|h| (h * 60.0).round())
        .filter(|m| m.is_finite() && *m <= 24.0 * 60.0)
        .map(|m| m as u32);
    Some(Battery {
        charge,
        state,
        minutes_left,
    })
}

// ---------------------------------------------------------------------------
// The disk
// ---------------------------------------------------------------------------

/// The fraction of the filesystem holding the user's home directory in use.
///
/// The home directory because it is the disk a person means by "my disk":
/// where their files go, and the one whose filling up they will notice first.
/// Where `HOME` is not set — a service account, a test harness — the root
/// filesystem is the only other honest answer.
pub fn disk_load() -> Result<f32, String> {
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/".into());
    disk_load_at(&home)
}

/// The same, for the filesystem holding `path`.
pub fn disk_load_at(path: &Path) -> Result<f32, String> {
    let blocks = statvfs(path)?;
    usage(blocks).ok_or_else(|| format!("{} reports a filesystem of no size", path.display()))
}

/// What `statvfs` reports, in blocks of one size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Blocks {
    /// Every block on the filesystem.
    pub total: u64,
    /// Every block not in use, the administrator's reserve included.
    pub free: u64,
    /// What an ordinary user may still write: `free` without the reserve.
    pub available: u64,
}

/// In use, the way `df` reports it: `used / (used + available)`.
///
/// Not `used / total`, which counts the reserve as room — on an ext4 disk
/// with the default 5 % reserve that reads 95 % at the moment an ordinary
/// user's writes start failing, which is the one moment the meter must not be
/// reassuring.
pub(crate) fn usage(blocks: Blocks) -> Option<f32> {
    let used = blocks.total.saturating_sub(blocks.free);
    let room = used.checked_add(blocks.available)?;
    if room == 0 {
        return None;
    }
    Some((used as f64 / room as f64).clamp(0.0, 1.0) as f32)
}

#[cfg(unix)]
fn statvfs(path: &Path) -> Result<Blocks, String> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("{} contains a NUL byte", path.display()))?;
    // SAFETY: `c_path` is a valid NUL-terminated string for the duration of
    // the call, and `stats` is a plain-old-data struct the call fills in.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    let status = unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) };
    if status != 0 {
        return Err(format!(
            "{}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    // `f_blocks`, `f_bfree` and `f_bavail` are all in units of `f_frsize`,
    // so the ratio needs no conversion — but the casts do, because their
    // width differs between platforms.
    #[allow(clippy::unnecessary_cast)]
    Ok(Blocks {
        total: stats.f_blocks as u64,
        free: stats.f_bfree as u64,
        available: stats.f_bavail as u64,
    })
}

#[cfg(not(unix))]
fn statvfs(_path: &Path) -> Result<Blocks, String> {
    Err(format!(
        "this host reads no disk usage on {}",
        std::env::consts::OS
    ))
}

// ---------------------------------------------------------------------------
// The network
// ---------------------------------------------------------------------------

/// Bytes per second, both ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Throughput {
    pub received: u64,
    pub sent: u64,
}

/// The shortest window worth dividing by. The same as the CPU sampler's, for
/// the same reason: eight widgets asking in one turn of the loop see one
/// measurement, and a rate over three milliseconds is noise.
const MIN_WINDOW: Duration = Duration::from_millis(250);

/// A network-rate sampler. One per desktop, shared by every widget, like the
/// CPU one: the counters are the machine's, not a widget's.
#[derive(Default)]
pub struct Network {
    last: Mutex<Option<NetSample>>,
}

struct NetSample {
    at: Instant,
    received: u64,
    sent: u64,
    rate: Throughput,
}

impl Network {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rate since the last call.
    pub fn throughput(&self) -> Result<Throughput, String> {
        let (received, sent) = read_network_counters()?;
        Ok(self.advance(Instant::now(), received, sent))
    }

    /// The arithmetic of [`Self::throughput`], with the clock and the counters
    /// handed in so a test can be exact about both.
    pub(crate) fn advance(&self, now: Instant, received: u64, sent: u64) -> Throughput {
        let mut last = self
            .last
            .lock()
            .expect("the sampler lock is never held across a panic");
        let rate = match last.as_ref() {
            // Asked again too soon: the previous answer stands, and so does
            // the previous sample, so the next window is a whole one rather
            // than the remainder of this one.
            Some(previous) if now.duration_since(previous.at) < MIN_WINDOW => {
                return previous.rate;
            }
            Some(previous) => {
                let seconds = now.duration_since(previous.at).as_secs_f64();
                // A counter that went *down* is an interface that went away —
                // a VPN disconnecting, a USB adapter unplugged — taking its
                // totals with it. That window has no honest rate, and zero is
                // closer to one than an underflow is.
                let per_second = |now: u64, before: u64| {
                    now.checked_sub(before)
                        .map(|delta| (delta as f64 / seconds).round() as u64)
                        .unwrap_or(0)
                };
                Throughput {
                    received: per_second(received, previous.received),
                    sent: per_second(sent, previous.sent),
                }
            }
            // The first call. There is no window yet; totals since boot divided
            // by uptime would be a number, but not one anybody asked for.
            None => Throughput::default(),
        };
        *last = Some(NetSample {
            at: now,
            received,
            sent,
            rate,
        });
        rate
    }
}

#[cfg(target_os = "linux")]
fn read_network_counters() -> Result<(u64, u64), String> {
    let text =
        std::fs::read_to_string("/proc/net/dev").map_err(|e| format!("/proc/net/dev: {e}"))?;
    parse_net_dev(&text).ok_or_else(|| "/proc/net/dev is not in the format this host reads".into())
}

#[cfg(not(target_os = "linux"))]
fn read_network_counters() -> Result<(u64, u64), String> {
    Err(format!(
        "this host reads no network counters on {}",
        std::env::consts::OS
    ))
}

/// Total `(received, sent)` bytes across every interface but loopback.
///
/// ```text
/// Inter-|   Receive                                                |  Transmit
///  face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets ...
///     lo: 1234567    1234    0    0    0     0          0         0  1234567    1234 ...
///   eth0: 9876543    9876    0    0    0     0          0         0  1234567    4321 ...
/// ```
///
/// Loopback is left out because it is the machine talking to itself: a
/// database and its client on one laptop would otherwise read as a busy
/// network with the cable unplugged. The interface name and the first counter
/// can run together (`eth0:123`) when the counter is wide, so the split is on
/// the colon rather than on whitespace.
pub(crate) fn parse_net_dev(text: &str) -> Option<(u64, u64)> {
    let mut received = 0u64;
    let mut sent = 0u64;
    let mut any = false;
    // The two header lines have a `|` and no `:` before it; skipping by shape
    // rather than by count means a kernel that adds a header line does not
    // turn its first interface into a header.
    for line in text.lines() {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        if name.contains('|') {
            continue;
        }
        let name = name.trim();
        let fields: Vec<u64> = counters
            .split_ascii_whitespace()
            .map(|f| f.parse().ok())
            .collect::<Option<_>>()?;
        // Eight receive counters, then eight transmit ones.
        if fields.len() < 9 {
            return None;
        }
        any = true;
        if name == "lo" {
            continue;
        }
        received = received.saturating_add(fields[0]);
        sent = sent.saturating_add(fields[8]);
    }
    any.then_some((received, sent))
}

/// A human-sized rate, for a widget's label or for `doctor`: `0 B/s`,
/// `12.3 kB/s`, `4.5 MB/s`. Decimal units, because a network is sold in them.
pub fn describe_rate(bytes_per_second: u64) -> String {
    const UNITS: [&str; 4] = ["kB/s", "MB/s", "GB/s", "TB/s"];
    if bytes_per_second < 1000 {
        return format!("{bytes_per_second} B/s");
    }
    let mut value = bytes_per_second as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
