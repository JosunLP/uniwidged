use super::*;

use std::path::PathBuf;

/// A fake `/sys/class/power_supply`, removed when the test ends.
struct Supplies(PathBuf);

impl Supplies {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "uniwidged-power-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        Self(dir)
    }

    /// One supply, as `(file, contents)` pairs.
    fn add(&self, name: &str, files: &[(&str, &str)]) -> &Self {
        let dir = self.0.join(name);
        std::fs::create_dir_all(&dir).expect("supply");
        for (file, contents) in files {
            // The kernel ends every attribute with a newline, and a parser
            // that forgets to trim it is the bug this reproduces.
            std::fs::write(dir.join(file), format!("{contents}\n")).expect("attribute");
        }
        self
    }

    fn read(&self) -> Option<Battery> {
        read_batteries(&self.0).expect("readable")
    }
}

impl Drop for Supplies {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_desktop_has_no_battery_and_that_is_not_an_error() {
    // The common case, and the one `ok(none)` exists for: a power widget on a
    // desktop says "on mains", it does not say "unavailable".
    let supplies = Supplies::new("desktop");
    supplies.add("AC", &[("type", "Mains"), ("online", "1")]);
    assert_eq!(supplies.read(), None);

    // And a machine with no power supplies at all — a VM, a container — is
    // the same answer rather than a refusal.
    assert_eq!(
        read_batteries(&supplies.0.join("does-not-exist")).expect("absent is fine"),
        None
    );
}

#[test]
fn a_discharging_laptop_reports_its_charge_and_the_time_it_has_left() {
    // 30 Wh of 60, losing 10 W: half full, three hours.
    let supplies = Supplies::new("laptop");
    supplies.add(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("present", "1"),
            ("energy_now", "30000000"),
            ("energy_full", "60000000"),
            ("power_now", "10000000"),
        ],
    );
    let battery = supplies.read().expect("a battery");
    assert_eq!(battery.state, PowerState::Discharging);
    assert!((battery.charge - 0.5).abs() < 1e-6);
    assert_eq!(battery.minutes_left, Some(180));
}

#[test]
fn a_charging_battery_counts_down_to_full_rather_than_to_empty() {
    // 45 of 60 Wh, gaining 30 W: a quarter of an hour to go, not ninety
    // minutes — which is what dividing what it holds by the rate would say.
    let supplies = Supplies::new("charging");
    supplies.add(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Charging"),
            ("energy_now", "45000000"),
            ("energy_full", "60000000"),
            ("power_now", "30000000"),
        ],
    );
    let battery = supplies.read().expect("a battery");
    assert_eq!(battery.state, PowerState::Charging);
    assert_eq!(battery.minutes_left, Some(30));
}

#[test]
fn a_driver_that_reports_charge_rather_than_energy_is_read_in_its_own_unit() {
    // µAh and µA. Mixing them with µWh would give a ratio that is a voltage.
    let supplies = Supplies::new("charge-units");
    supplies.add(
        "BAT1",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("charge_now", "1000000"),
            ("charge_full", "4000000"),
            // Negative, as some drivers report a discharge.
            ("current_now", "-2000000"),
        ],
    );
    let battery = supplies.read().expect("a battery");
    assert!((battery.charge - 0.25).abs() < 1e-6);
    assert_eq!(battery.minutes_left, Some(30));
}

#[test]
fn a_driver_with_only_a_percentage_gives_a_charge_and_no_time() {
    let supplies = Supplies::new("capacity-only");
    supplies.add(
        "battery",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("capacity", "73"),
        ],
    );
    let battery = supplies.read().expect("a battery");
    assert!((battery.charge - 0.73).abs() < 1e-6);
    // No rate, so no estimate. A guess would be a number a widget draws.
    assert_eq!(battery.minutes_left, None);
}

#[test]
fn a_mouse_battery_is_not_the_machines() {
    // `scope = Device` is a peripheral's. A desktop with a wireless mouse at
    // 10 % is not a laptop about to turn off.
    let supplies = Supplies::new("peripheral");
    supplies.add(
        "hidpp_battery_0",
        &[
            ("type", "Battery"),
            ("scope", "Device"),
            ("status", "Discharging"),
            ("capacity", "10"),
        ],
    );
    assert_eq!(supplies.read(), None);
}

#[test]
fn an_empty_bay_is_not_a_battery() {
    let supplies = Supplies::new("empty-bay");
    supplies.add(
        "BAT1",
        &[
            ("type", "Battery"),
            ("present", "0"),
            ("energy_now", "0"),
            ("energy_full", "0"),
        ],
    );
    assert_eq!(supplies.read(), None);
}

#[test]
fn two_batteries_are_weighted_by_what_they_hold() {
    // A large internal battery at 90 % and a small external one at 10 % are
    // not a machine at 50 %. 54 + 2 of 60 + 20 is 70 %.
    let supplies = Supplies::new("two");
    supplies
        .add(
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", "Not charging"),
                ("energy_now", "54000000"),
                ("energy_full", "60000000"),
            ],
        )
        .add(
            "BAT1",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("energy_now", "2000000"),
                ("energy_full", "20000000"),
                ("power_now", "7000000"),
            ],
        );
    let battery = supplies.read().expect("a battery");
    assert!((battery.charge - 0.7).abs() < 1e-6, "{}", battery.charge);
    // One of them is discharging, so the machine is on battery — the state
    // that matters most to somebody watching wins.
    assert_eq!(battery.state, PowerState::Discharging);
    // 56 Wh at 7 W: eight hours.
    assert_eq!(battery.minutes_left, Some(480));
}

#[test]
fn full_not_charging_and_unknown_are_all_idle() {
    for status in ["Full", "Not charging", "Unknown"] {
        let supplies = Supplies::new(&status.replace(' ', "-"));
        supplies.add(
            "BAT0",
            &[
                ("type", "Battery"),
                ("status", status),
                ("energy_now", "60000000"),
                ("energy_full", "60000000"),
                ("power_now", "0"),
            ],
        );
        let battery = supplies.read().expect("a battery");
        assert_eq!(battery.state, PowerState::Idle, "{status}");
        assert_eq!(battery.minutes_left, None, "{status}");
    }
}

#[test]
fn a_rate_of_almost_nothing_is_not_four_thousand_hours() {
    // The instant after a cable moves, some drivers report a rate of a few
    // microwatts. Dividing by it is arithmetic, not an estimate.
    let supplies = Supplies::new("tiny-rate");
    supplies.add(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("energy_now", "50000000"),
            ("energy_full", "60000000"),
            ("power_now", "10"),
        ],
    );
    assert_eq!(supplies.read().expect("a battery").minutes_left, None);
}

#[test]
fn reading_this_machines_battery_does_not_fail_or_block() {
    // Whatever this machine is. Linux, macOS and Windows can all answer; a
    // refusal is only acceptable on a platform with no reader.
    let began = Instant::now();
    let answer = battery();
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "battery blocked"
    );
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        let answer = answer.expect("this platform can always answer");
        if let Some(battery) = answer {
            assert!((0.0..=1.0).contains(&battery.charge));
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = answer;
}

// ---------------------------------------------------------------------------
// The disk
// ---------------------------------------------------------------------------

#[test]
fn the_administrators_reserve_counts_as_used() {
    // 1000 blocks, 100 free, of which 50 are reserved: an ordinary user has 50
    // left and 900 are in use — 900 / 950, not 900 / 1000.
    let used = usage(Blocks {
        total: 1000,
        free: 100,
        available: 50,
    })
    .expect("a size");
    assert!((used - 900.0 / 950.0).abs() < 1e-6, "{used}");

    // And when the reserve is all that is left, the meter is full — which is
    // the moment writes start failing, and the one it must not soften.
    let full = usage(Blocks {
        total: 1000,
        free: 50,
        available: 0,
    })
    .expect("a size");
    assert_eq!(full, 1.0);
}

#[test]
fn a_filesystem_of_no_size_is_refused_rather_than_divided_by() {
    // `proc`, `sysfs` and friends report zero blocks.
    assert_eq!(
        usage(Blocks {
            total: 0,
            free: 0,
            available: 0
        }),
        None
    );
}

#[test]
fn this_machines_disk_is_a_fraction() {
    #[cfg(unix)]
    {
        let used = disk_load_at(&std::env::temp_dir()).expect("statvfs");
        assert!((0.0..=1.0).contains(&used), "{used}");
        assert!(disk_load_at(Path::new("/definitely/not/a/path")).is_err());
    }
}

// ---------------------------------------------------------------------------
// The network
// ---------------------------------------------------------------------------

const NET_DEV: &str = "\
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 5000000    1000    0    0    0     0          0         0  5000000    1000    0    0    0     0       0          0
  eth0: 1000000    2000    0    0    0     0          0         0   200000    1500    0    0    0     0       0          0
wlan0:3000000    3000    0    0    0     0          0         0   400000    1200    0    0    0     0       0          0
";

#[test]
fn loopback_is_the_machine_talking_to_itself_and_is_not_counted() {
    // `wlan0:3000000` has no space after the colon — a wide counter runs into
    // the name — and is still read.
    assert_eq!(parse_net_dev(NET_DEV), Some((4_000_000, 600_000)));
}

#[test]
fn a_machine_with_only_loopback_is_silent_rather_than_unreadable() {
    let only_lo = "\
Inter-|   Receive |  Transmit
 face |bytes ...|bytes ...
    lo: 5000 10 0 0 0 0 0 0 5000 10 0 0 0 0 0 0
";
    assert_eq!(parse_net_dev(only_lo), Some((0, 0)));
}

#[test]
fn a_file_in_another_format_is_refused() {
    assert_eq!(parse_net_dev(""), None);
    assert_eq!(parse_net_dev("eth0: 1 2 3\n"), None);
    assert_eq!(
        parse_net_dev("eth0: a b c d e f g h i j k l m n o p\n"),
        None
    );
}

#[test]
fn the_first_reading_is_zero_and_the_next_is_a_rate() {
    let network = Network::new();
    let start = Instant::now();
    assert_eq!(network.advance(start, 1_000, 500), Throughput::default());
    // Two seconds later, 4 000 bytes in and 1 000 out: 2 000 and 500 a second.
    let rate = network.advance(start + Duration::from_secs(2), 5_000, 1_500);
    assert_eq!(
        rate,
        Throughput {
            received: 2_000,
            sent: 500
        }
    );
}

#[test]
fn asking_again_too_soon_gets_the_last_answer_and_keeps_the_window() {
    let network = Network::new();
    let start = Instant::now();
    network.advance(start, 0, 0);
    let first = network.advance(start + Duration::from_secs(1), 1_000, 1_000);
    // Ten milliseconds later, with a burst in between: the answer does not
    // swing, because a rate over ten milliseconds is noise.
    let again = network.advance(start + Duration::from_millis(1_010), 900_000, 900_000);
    assert_eq!(first, again);
    // And the next real window is measured from the last sample *taken*, not
    // from the call that was turned away, so nothing is lost.
    let later = network.advance(start + Duration::from_secs(2), 2_000, 2_000);
    assert_eq!(
        later,
        Throughput {
            received: 1_000,
            sent: 1_000
        }
    );
}

#[test]
fn an_interface_that_went_away_is_zero_not_an_underflow() {
    // A VPN disconnects and takes its totals with it, so the sum goes down.
    let network = Network::new();
    let start = Instant::now();
    network.advance(start, 10_000_000, 10_000_000);
    let rate = network.advance(start + Duration::from_secs(1), 1_000, 20_000_000);
    assert_eq!(rate.received, 0);
    assert_eq!(rate.sent, 10_000_000);
}

#[test]
fn reading_this_machines_network_does_not_block() {
    let network = Network::new();
    let began = Instant::now();
    let answer = network.throughput();
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "network blocked"
    );
    // A first reading is a window of nothing, whichever reader answered it.
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    assert_eq!(
        answer.expect("this platform reads network counters"),
        Throughput::default()
    );
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = answer;
}

#[test]
fn a_rate_reads_the_way_a_network_is_sold() {
    assert_eq!(describe_rate(0), "0 B/s");
    assert_eq!(describe_rate(999), "999 B/s");
    assert_eq!(describe_rate(1_000), "1.0 kB/s");
    assert_eq!(describe_rate(12_345), "12.3 kB/s");
    // Rounds up into the next unit rather than printing "1000.0 kB/s".
    assert_eq!(describe_rate(999_990), "1.0 MB/s");
    assert_eq!(describe_rate(4_500_000), "4.5 MB/s");
    assert!(describe_rate(u64::MAX).ends_with("TB/s"));
}
