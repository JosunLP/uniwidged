//! The machine counters behind `metrics`, on macOS and Windows.
//!
//! Linux reads text files under `/proc` and `/sys`, and those parsers live
//! beside the samplers in `services` and `machine` because they are most of
//! the logic. The other two platforms answer through system calls instead, and
//! this file is those calls and nothing else: each function returns the same
//! raw numbers the Linux reader does, so the sampling, the windowing and the
//! rounding above it are one implementation for every platform.
//!
//! | | macOS | Windows |
//! |---|---|---|
//! | CPU | `host_statistics(HOST_CPU_LOAD_INFO)` | `GetSystemTimes` |
//! | memory | `host_statistics64(HOST_VM_INFO64)`, `hw.memsize` | `GlobalMemoryStatusEx` |
//! | disk | `statvfs` (in `machine`) | `GetDiskFreeSpaceExW` |
//! | network | `getifaddrs` → `if_data` | `GetIfTable2` |
//! | battery | IOKit `AppleSmartBattery` | `GetSystemPowerStatus` |
//!
//! Every answer means what the Linux one means — in particular, "memory in
//! use" is what the system could not hand to a new allocation without evicting
//! something, not "not free", and loopback is never network traffic.

#[cfg(any(target_os = "macos", windows))]
use crate::machine::{Battery, PowerState};

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

/// `(busy, total)` ticks across every CPU.
// `libc` marks the Mach host calls deprecated in favour of the `mach2` crate;
// they are the same symbols, and one crate for two calls is not worth it.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub(crate) fn cpu() -> Result<(u64, u64), String> {
    let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: the host port is this process's; `info` is the struct the flavor
    // names and `count` its size in `natural_t`s.
    let status = unsafe {
        #[allow(deprecated)]
        libc::host_statistics(
            libc::mach_host_self(),
            libc::HOST_CPU_LOAD_INFO,
            (&mut info as *mut libc::host_cpu_load_info).cast(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS {
        return Err(format!("host_statistics answered {status}"));
    }
    let ticks = info.cpu_ticks.map(u64::from);
    let total: u64 = ticks.iter().sum();
    let idle = ticks[libc::CPU_STATE_IDLE as usize];
    Ok((total.saturating_sub(idle), total))
}

/// `(used, total)` in bytes.
///
/// Available is free, inactive, speculative and purgeable pages: what the
/// kernel can hand out without paging anything out or compressing it — the
/// same question `MemAvailable` answers on Linux, and close to what Activity
/// Monitor calls "memory used" from the other side.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub(crate) fn memory() -> Result<(u64, u64), String> {
    let mut total: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: a NUL-terminated name and a `u64` of the size given.
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut total as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(format!("hw.memsize: {}", std::io::Error::last_os_error()));
    }
    let mut vm: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: as `cpu`, for the 64-bit VM statistics.
    let status = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            (&mut vm as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS {
        return Err(format!("host_statistics64 answered {status}"));
    }
    // SAFETY: a constant the kernel sets once at start-up.
    #[allow(deprecated)]
    let page = unsafe { libc::vm_page_size } as u64;
    let available = (u64::from(vm.free_count)
        + u64::from(vm.inactive_count)
        + u64::from(vm.speculative_count)
        + u64::from(vm.purgeable_count))
        * page;
    Ok((total.saturating_sub(available.min(total)), total))
}

/// Total `(received, sent)` bytes across every interface but loopback.
///
/// The per-interface counters in `if_data` are 32 bits on macOS and wrap at
/// 4 GiB; the sampler above treats a counter that went down as a window with
/// no honest rate, so a wrap costs one reading rather than a nonsense spike.
#[cfg(target_os = "macos")]
pub(crate) fn network() -> Result<(u64, u64), String> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` allocates the list and `freeifaddrs` frees it,
    // below, on every path.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return Err(format!("getifaddrs: {}", std::io::Error::last_os_error()));
    }
    let (mut received, mut sent) = (0u64, 0u64);
    let mut at = list;
    while !at.is_null() {
        // SAFETY: a node of the list `getifaddrs` returned, alive until freed.
        let entry = unsafe { &*at };
        let is_link = !entry.ifa_addr.is_null()
            && i32::from(unsafe { (*entry.ifa_addr).sa_family }) == libc::AF_LINK;
        let loopback = entry.ifa_flags & libc::IFF_LOOPBACK as u32 != 0;
        if is_link && !loopback && !entry.ifa_data.is_null() {
            // SAFETY: for an `AF_LINK` entry, `ifa_data` is its `if_data`.
            let data = unsafe { &*(entry.ifa_data as *const libc::if_data) };
            received += u64::from(data.ifi_ibytes);
            sent += u64::from(data.ifi_obytes);
        }
        at = entry.ifa_next;
    }
    // SAFETY: the list from above, freed once.
    unsafe { libc::freeifaddrs(list) };
    Ok((received, sent))
}

/// The internal battery, or `None` on a Mac without one.
///
/// Read from the `AppleSmartBattery` service in the I/O registry rather than
/// through `IOPowerSources`, because the registry gives the numbers directly:
/// a charge, whether it is charging, whether the charger is connected, and the
/// minutes the system itself estimates.
#[cfg(target_os = "macos")]
pub(crate) fn battery() -> Result<Option<Battery>, String> {
    macos_battery::read()
}

#[cfg(target_os = "macos")]
mod macos_battery {
    use super::{Battery, PowerState};
    use std::ffi::{c_char, c_void};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type IoObject = u32;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> IoObject;
        fn IORegistryEntryCreateCFProperties(
            entry: IoObject,
            properties: *mut CFDictionaryRef,
            allocator: *const c_void,
            options: u32,
        ) -> i32;
        fn IOObjectRelease(object: IoObject) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFDictionaryGetValue(dictionary: CFDictionaryRef, key: *const c_void) -> CFTypeRef;
        fn CFNumberGetValue(number: CFTypeRef, kind: i64, out: *mut c_void) -> u8;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
        fn CFGetTypeID(object: CFTypeRef) -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFRelease(object: CFTypeRef);
    }

    /// `kCFStringEncodingUTF8` and `kCFNumberSInt64Type`.
    const UTF8: u32 = 0x0800_0100;
    const SINT64: i64 = 4;

    /// One property out of the battery's dictionary.
    struct Properties(CFDictionaryRef);

    impl Properties {
        fn value(&self, key: &std::ffi::CStr) -> CFTypeRef {
            // SAFETY: a static UTF-8 key, released straight after the lookup;
            // the value is borrowed from the dictionary this struct holds.
            unsafe {
                let key = CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), UTF8);
                if key.is_null() {
                    return std::ptr::null();
                }
                let value = CFDictionaryGetValue(self.0, key);
                CFRelease(key);
                value
            }
        }

        fn number(&self, key: &std::ffi::CStr) -> Option<i64> {
            let value = self.value(key);
            // SAFETY: typed before it is read.
            unsafe {
                if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
                    return None;
                }
                let mut out: i64 = 0;
                (CFNumberGetValue(value, SINT64, (&mut out as *mut i64).cast()) != 0).then_some(out)
            }
        }

        fn flag(&self, key: &std::ffi::CStr) -> Option<bool> {
            let value = self.value(key);
            // SAFETY: typed before it is read.
            unsafe {
                if value.is_null() || CFGetTypeID(value) != CFBooleanGetTypeID() {
                    return None;
                }
                Some(CFBooleanGetValue(value) != 0)
            }
        }
    }

    impl Drop for Properties {
        fn drop(&mut self) {
            // SAFETY: a dictionary this struct was given ownership of.
            unsafe { CFRelease(self.0) };
        }
    }

    pub(super) fn read() -> Result<Option<Battery>, String> {
        // SAFETY: a static service name; the matching dictionary is consumed by
        // `IOServiceGetMatchingService`, and the service is released below.
        let service = unsafe {
            let matching = IOServiceMatching(c"AppleSmartBattery".as_ptr());
            if matching.is_null() {
                return Err("IOKit would not build a matching dictionary".into());
            }
            IOServiceGetMatchingService(0, matching)
        };
        if service == 0 {
            // No such service is a Mac with no battery: a Mac mini, an iMac,
            // a CI runner. Normal, and `ok(none)`.
            return Ok(None);
        }
        let mut dictionary: CFDictionaryRef = std::ptr::null();
        // SAFETY: a live service, and an out-pointer for the dictionary.
        let status = unsafe {
            let status =
                IORegistryEntryCreateCFProperties(service, &mut dictionary, std::ptr::null(), 0);
            IOObjectRelease(service);
            status
        };
        if status != 0 || dictionary.is_null() {
            return Err(format!(
                "the battery's properties could not be read ({status})"
            ));
        }
        let properties = Properties(dictionary);
        let (Some(current), Some(max)) = (
            properties.number(c"CurrentCapacity"),
            properties.number(c"MaxCapacity"),
        ) else {
            return Ok(None);
        };
        if max <= 0 {
            return Ok(None);
        }
        let charging = properties.flag(c"IsCharging").unwrap_or(false);
        let plugged = properties.flag(c"ExternalConnected").unwrap_or(false);
        let state = match (charging, plugged) {
            (true, _) => PowerState::Charging,
            (false, true) => PowerState::Idle,
            (false, false) => PowerState::Discharging,
        };
        // 65535 is the registry's "still estimating".
        let minutes_left = properties
            .number(c"TimeRemaining")
            .filter(|m| (1..65535).contains(m) && state != PowerState::Idle)
            .map(|m| m as u32);
        Ok(Some(Battery {
            charge: (current as f32 / max as f32).clamp(0.0, 1.0),
            state,
            minutes_left,
        }))
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn filetime(time: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// `(busy, total)` in 100-nanosecond units across every CPU.
///
/// The kernel time `GetSystemTimes` reports *includes* the idle time, so the
/// total is kernel + user and busy is that minus idle — adding idle again is
/// the classic way to report a machine half as busy as it is.
#[cfg(windows)]
pub(crate) fn cpu() -> Result<(u64, u64), String> {
    use windows_sys::Win32::Foundation::FILETIME;
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut idle, mut kernel, mut user) = (zero, zero, zero);
    // SAFETY: three out-parameters of the type asked for.
    if unsafe {
        windows_sys::Win32::System::Threading::GetSystemTimes(&mut idle, &mut kernel, &mut user)
    } == 0
    {
        return Err(format!(
            "GetSystemTimes: {}",
            std::io::Error::last_os_error()
        ));
    }
    let total = filetime(kernel) + filetime(user);
    Ok((total.saturating_sub(filetime(idle)), total))
}

/// `(used, total)` in bytes: total physical memory, less what Windows reports
/// as available — standby and free pages, the same question as `MemAvailable`.
#[cfg(windows)]
pub(crate) fn memory() -> Result<(u64, u64), String> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: a zeroed struct with its length set, as the call requires.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(format!(
            "GlobalMemoryStatusEx: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok((
        status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
        status.ullTotalPhys,
    ))
}

/// The volume holding `path`, as `statvfs` would describe it, in bytes.
#[cfg(windows)]
pub(crate) fn disk(path: &std::path::Path) -> Result<crate::machine::Blocks, String> {
    let wide: Vec<u16> = path
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: a NUL-terminated path and three out-parameters.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            &mut total,
            &mut free,
        )
    };
    if ok == 0 {
        return Err(format!(
            "{}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(crate::machine::Blocks {
        total,
        free,
        available,
    })
}

/// Total `(received, sent)` bytes across every physical interface that is up.
///
/// `GetIfTable2` lists filter and pseudo-interfaces stacked over each adapter
/// as well as the adapter itself, each with the same traffic counted again;
/// summing all of them reports several times what crossed the wire. So only
/// rows marked as hardware are counted — which also leaves out loopback.
#[cfg(windows)]
pub(crate) fn network() -> Result<(u64, u64), String> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIfTable2, MIB_IF_TABLE2,
    };
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: the call allocates the table, freed below.
    let status = unsafe { GetIfTable2(&mut table) };
    if status != 0 || table.is_null() {
        return Err(format!(
            "GetIfTable2: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ));
    }
    // SAFETY: `NumEntries` rows follow the header, as the call documents.
    let rows = unsafe {
        std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
    };
    /// `IfOperStatusUp`, and the `HardwareInterface` bit of the flags.
    const UP: i32 = 1;
    let (mut received, mut sent) = (0u64, 0u64);
    for row in rows {
        let hardware = row.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
        if hardware && row.OperStatus == UP {
            received += row.InOctets;
            sent += row.OutOctets;
        }
    }
    // SAFETY: the table from above, freed once.
    unsafe { FreeMibTable(table.cast()) };
    Ok((received, sent))
}

/// The battery, or `None` on a machine without one.
#[cfg(windows)]
pub(crate) fn battery() -> Result<Option<Battery>, String> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: a plain out-parameter.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return Err(format!(
            "GetSystemPowerStatus: {}",
            std::io::Error::last_os_error()
        ));
    }
    /// `BATTERY_FLAG_NO_BATTERY`, `BATTERY_FLAG_UNKNOWN`, `BATTERY_FLAG_CHARGING`.
    const NO_BATTERY: u8 = 128;
    const UNKNOWN: u8 = 255;
    const CHARGING: u8 = 8;
    if status.BatteryFlag == UNKNOWN
        || status.BatteryFlag & NO_BATTERY != 0
        || status.BatteryLifePercent > 100
    {
        return Ok(None);
    }
    let state = if status.BatteryFlag & CHARGING != 0 {
        PowerState::Charging
    } else if status.ACLineStatus == 1 {
        PowerState::Idle
    } else {
        PowerState::Discharging
    };
    // Seconds, and `u32::MAX` while Windows is still estimating. Only ever an
    // estimate to empty, so it is only reported while discharging.
    let minutes_left = (state == PowerState::Discharging && status.BatteryLifeTime != u32::MAX)
        .then_some(status.BatteryLifeTime / 60);
    Ok(Some(Battery {
        charge: f32::from(status.BatteryLifePercent) / 100.0,
        state,
        minutes_left,
    }))
}
