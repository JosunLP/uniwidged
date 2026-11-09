//! What the storage refuses, and what the calendar gets right.
//!
//! The storage half is written the way the capability tests are: mostly about
//! refusal, because a directory a widget cannot leave is only worth having if
//! somebody has tried to leave it. The calendar half is about the dates that
//! are wrong in the obvious implementation — a century that is not a leap
//! year, an instant before 1970, and midnight on a twelve-hour clock.

use super::*;

/// A directory that removes itself.
///
/// Hand-rolled rather than `tempfile`: this crate's dependency list is short
/// on purpose, and what is needed here is six lines.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    /// The widget's own directory, which deliberately does not exist yet.
    fn dir(&self) -> DataDir {
        DataDir::new(self.0.join("widgets/example"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_widgets_first_run_finds_nothing_rather_than_an_error() {
    // The normal case for a caching widget, and the one an implementation
    // reporting I/O errors gets wrong: the directory does not exist yet
    // because nothing has ever been written to it.
    let scratch = Scratch::new("first-run");
    let dir = scratch.dir();
    assert_eq!(dir.read("cache/today.json"), Ok(None));
    assert!(
        !dir.path().exists(),
        "a widget that only reads should leave nothing behind"
    );
}

#[test]
fn what_a_widget_writes_is_what_it_reads_back() {
    let scratch = Scratch::new("roundtrip");
    let dir = scratch.dir();
    dir.write("cache/today.json", b"{\"c\":18}").expect("write");
    assert_eq!(
        dir.read("cache/today.json"),
        Ok(Some(b"{\"c\":18}".to_vec()))
    );
    // And the second write replaces rather than appends.
    dir.write("cache/today.json", b"{}").expect("rewrite");
    assert_eq!(dir.read("cache/today.json"), Ok(Some(b"{}".to_vec())));
}

#[test]
fn a_write_leaves_no_temporary_behind() {
    // The rename is what makes an interrupted widget safe — a widget stopped
    // at its deadline must not leave half a document where its own next run
    // will parse one — and a temporary that survives is a file the widget can
    // then see in its own directory.
    let scratch = Scratch::new("atomic");
    let dir = scratch.dir();
    dir.write("state.bin", b"x").expect("write");
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .expect("the directory exists now")
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert_eq!(
        left.len(),
        1,
        "exactly the file that was asked for: {left:?}"
    );
    assert_eq!(left[0], "state.bin");
}

#[test]
fn a_symlink_out_of_the_directory_is_refused() {
    // The textual check in `capability` cannot see this one: `cache/escape`
    // contains no `..` and is not absolute. Only the filesystem knows where it
    // goes, which is why the check is made again after resolving.
    #[cfg(unix)]
    {
        let scratch = Scratch::new("symlink");
        let dir = scratch.dir();
        let outside = scratch.0.join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("secret"), b"not yours").expect("secret");
        dir.write("cache/keep", b"anything")
            .expect("create the dir");

        std::os::unix::fs::symlink(outside.join("secret"), dir.path().join("cache/escape"))
            .expect("symlink");
        assert_eq!(dir.read("cache/escape"), Err(StoreError::Escape));

        std::os::unix::fs::symlink(&outside, dir.path().join("cache/elsewhere"))
            .expect("symlink a directory");
        assert_eq!(
            dir.write("cache/elsewhere/planted", b"x"),
            Err(StoreError::Escape),
            "a symlinked directory is not a way out either"
        );
        assert!(
            !outside.join("planted").exists(),
            "nothing may be written outside the widget's directory"
        );
    }
}

#[test]
fn a_file_larger_than_the_limit_is_refused_in_both_directions() {
    let scratch = Scratch::new("large");
    let dir = scratch.dir().with_max_file_bytes(64);
    assert_eq!(
        dir.write("big", &[0u8; 65]),
        Err(StoreError::TooLarge { limit: 64 })
    );
    assert!(!dir.path().join("big").exists(), "nothing was written");

    // The other direction is not the widget's doing: its directory is
    // somewhere a user can reach, and a host that read whatever it found there
    // would let a widget's own folder become a way to allocate a gigabyte in
    // the desktop process.
    dir.write("planted", b"small").expect("write");
    std::fs::write(dir.path().join("planted"), vec![0u8; 65]).expect("grow it from outside");
    assert_eq!(dir.read("planted"), Err(StoreError::TooLarge { limit: 64 }));
}

#[test]
fn subdirectories_are_created_on_the_way() {
    let scratch = Scratch::new("nested");
    let dir = scratch.dir();
    dir.write("a/b/c/deep.txt", b"here").expect("write");
    assert_eq!(dir.read("a/b/c/deep.txt"), Ok(Some(b"here".to_vec())));
}

#[test]
fn two_widgets_cannot_see_each_others_files() {
    // The whole point of the directory being per widget. Nothing here enforces
    // it beyond the root each one is given, which is why the host names the
    // directory after the manifest and the manifest is strict about names.
    let scratch = Scratch::new("neighbours");
    let one = DataDir::new(scratch.0.join("widgets/one"));
    let two = DataDir::new(scratch.0.join("widgets/two"));
    one.write("notes", b"mine").expect("write");
    assert_eq!(two.read("notes"), Ok(None));
}

// ---------------------------------------------------------------------------
// The calendar
// ---------------------------------------------------------------------------

fn utc() -> Locale {
    Locale::default()
}

#[test]
fn the_epoch_and_the_days_around_it() {
    assert_eq!(utc().format_date(0), "1970-01-01");
    assert_eq!(utc().format_time(0), "00:00");
    // Before 1970. Truncating division puts this on the wrong day, which is
    // the bug this test exists for.
    assert_eq!(utc().format_date(-1), "1969-12-31");
    assert_eq!(utc().format_time(-1), "23:59");
}

#[test]
fn the_leap_years_that_are_usually_wrong() {
    // 2000 is a leap year and 2100 is not, and an implementation that checks
    // only `year % 4` gets the second one wrong — in 2100, which is after
    // everybody's test suite has stopped running.
    assert_eq!(utc().format_date(951_782_400), "2000-02-29");
    assert_eq!(utc().format_date(4_107_542_400), "2100-03-01");
    assert_eq!(utc().format_date(4_107_456_000), "2100-02-28");
}

#[test]
fn a_known_instant_is_written_three_ways() {
    // 2026-08-28 14:05:00 UTC.
    let instant = 1_787_925_900;
    let iso = Locale::default();
    assert_eq!(iso.format_date(instant), "2026-08-28");
    assert_eq!(iso.format_time(instant), "14:05");

    let british = Locale {
        date_order: DateOrder::Dmy,
        ..Locale::default()
    };
    assert_eq!(british.format_date(instant), "28/08/2026");

    let american = Locale {
        date_order: DateOrder::Mdy,
        hour_cycle: HourCycle::H12,
        ..Locale::default()
    };
    assert_eq!(american.format_date(instant), "08/28/2026");
    assert_eq!(american.format_time(instant), "2:05 PM");
}

#[test]
fn midnight_and_noon_on_a_twelve_hour_clock() {
    // Both are `0 % 12`, and the naive version prints "0:00".
    let clock = Locale {
        hour_cycle: HourCycle::H12,
        ..Locale::default()
    };
    assert_eq!(clock.format_time(0), "12:00 AM");
    assert_eq!(clock.format_time(12 * 3600), "12:00 PM");
    assert_eq!(clock.format_time(13 * 3600), "1:00 PM");
    assert_eq!(clock.format_time(11 * 3600 + 59 * 60), "11:59 AM");
}

#[test]
fn the_offset_can_carry_an_instant_into_another_day() {
    let ahead = Locale {
        utc_offset_seconds: 13 * 3600,
        ..Locale::default()
    };
    // 2026-08-28 14:05 UTC is the 29th in Auckland.
    assert_eq!(ahead.format_date(1_787_925_900), "2026-08-29");
    assert_eq!(ahead.format_time(1_787_925_900), "03:05");

    let behind = Locale {
        utc_offset_seconds: -8 * 3600,
        ..Locale::default()
    };
    assert_eq!(behind.format_date(0), "1969-12-31");
    assert_eq!(behind.format_time(0), "16:00");
}

#[test]
fn the_locale_table_reads_posix_names() {
    // `en_US` is the one region split worth having: it is the only English
    // locale that writes the month first, and the table would be wrong for
    // every other one without it.
    let american = Locale::from_lang(Some("en_US.UTF-8"));
    assert_eq!(american.date_order, DateOrder::Mdy);
    assert_eq!(american.hour_cycle, HourCycle::H12);

    for british in ["en_GB.UTF-8", "en_AU", "de_DE.UTF-8@euro", "fr_FR"] {
        let locale = Locale::from_lang(Some(british));
        assert_eq!(locale.date_order, DateOrder::Dmy, "{british}");
        assert_eq!(locale.hour_cycle, HourCycle::H23, "{british}");
    }

    assert_eq!(Locale::from_lang(Some("ja_JP")).date_order, DateOrder::Ymd);
    assert_eq!(Locale::from_lang(Some("sv_SE")).date_order, DateOrder::Ymd);

    // `C` means "assume nothing", and an unset environment says nothing at
    // all. Both get the order nobody misreads rather than a guess.
    assert_eq!(Locale::from_lang(Some("C")).date_order, DateOrder::Ymd);
    assert_eq!(Locale::from_lang(None).date_order, DateOrder::Ymd);
    assert_eq!(Locale::from_lang(Some("")).date_order, DateOrder::Ymd);

    // Anything unrecognised is day-first, which is what most of the world
    // writes — a wrong guess here is a date read in the wrong order, so the
    // fallback should be the common case rather than the developer's.
    assert_eq!(Locale::from_lang(Some("xx_YY")).date_order, DateOrder::Dmy);
}

#[test]
fn the_system_locale_is_a_locale() {
    // It cannot be asserted against a value — the machine decides — but it can
    // be asserted to be sane, which is what catches a `tm_gmtoff` read from an
    // uninitialised struct.
    let locale = Locale::system();
    assert!(
        locale.utc_offset_seconds.abs() <= 26 * 3600,
        "no timezone is {} seconds from UTC",
        locale.utc_offset_seconds
    );
    assert_eq!(locale.format_date(0).len(), 10);
}

// ---------------------------------------------------------------------------
// The CPU sampler
// ---------------------------------------------------------------------------

#[test]
fn the_aggregate_line_of_proc_stat_is_read_the_way_the_kernel_writes_it() {
    // user 100, nice 10, system 50, idle 800, iowait 40, irq 0, softirq 0,
    // steal 0. Busy is everything but idle and iowait: 160 of 1000.
    let (busy, total) = parse_cpu_line("cpu  100 10 50 800 40 0 0 0 0 0").expect("parsed");
    assert_eq!((busy, total), (160, 1000));
}

#[test]
fn the_guest_fields_are_not_counted_twice() {
    // `guest` and `guest_nice` are already inside `user` and `nice`. Summing
    // all ten fields is how a machine running VMs comes to report 120 % busy,
    // and it is the mistake this line is written to avoid.
    let with_guests = parse_cpu_line("cpu  100 10 50 800 40 0 0 0 90 5").expect("parsed");
    let without = parse_cpu_line("cpu  100 10 50 800 40 0 0 0 0 0").expect("parsed");
    assert_eq!(with_guests, without);
}

#[test]
fn an_old_kernel_with_only_four_fields_still_parses() {
    let (busy, total) = parse_cpu_line("cpu  100 10 50 840").expect("parsed");
    assert_eq!((busy, total), (160, 1000));
}

#[test]
fn anything_that_is_not_the_aggregate_line_is_refused() {
    // `cpu0` is one core, not the machine, and reading it as the machine would
    // report an eight-core box as whatever core zero happened to be doing.
    for line in [
        "cpu0 100 10 50 800",
        "intr 12345",
        "",
        "cpu",
        "cpu  100 ten 50 800",
    ] {
        assert!(parse_cpu_line(line).is_none(), "{line:?} should be refused");
    }
}

#[test]
fn a_load_is_a_fraction_and_a_second_reading_is_stable() {
    let sampler = Counters::new();
    let first = match sampler.cpu_load() {
        Ok(first) => first,
        // Linux, macOS and Windows all read CPU counters, so a refusal there
        // is a bug rather than a platform without one.
        Err(why) if cfg!(any(target_os = "linux", target_os = "macos", windows)) => {
            panic!("this platform should read CPU counters: {why}")
        }
        // Elsewhere the refusal is the documented answer, and it carries a
        // reason rather than a zero a widget would draw as an idle machine.
        Err(_) => return,
    };
    assert!(
        (0.0..=1.0).contains(&first),
        "a load outside 0..1 is not a fraction: {first}"
    );
    // Immediately again: inside the minimum window, so the same answer rather
    // than one computed over a few microseconds.
    assert_eq!(sampler.cpu_load().expect("second"), first);
}

#[test]
fn the_sampler_never_blocks() {
    // The invariant that matters on this host: a widget calling `cpu_load`
    // inside `view()` is holding its deadline open. An implementation that
    // slept between two reads — which is the obvious way to measure a load —
    // would take a tenth of a second here.
    let sampler = Counters::new();
    let began = std::time::Instant::now();
    for _ in 0..10 {
        let _ = sampler.cpu_load();
    }
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "ten samples took {:?}",
        began.elapsed()
    );
}

// ---------------------------------------------------------------------------
// Running a command
// ---------------------------------------------------------------------------
//
// This is a hole in the sandbox that a user deliberately opened, so the tests
// are about how wide it is. Everything here runs a real process.

use crate::tasks::{CommandOutput, TaskResult};

fn ran(result: TaskResult) -> CommandOutput {
    match result {
        TaskResult::Command(output) => output,
        other => panic!("expected a command result, got {other:?}"),
    }
}

#[test]
fn a_command_runs_and_its_output_comes_back() {
    let output = ran(Commands::new().run("echo", &["hello".into(), "world".into()]));
    assert_eq!(output.status, 0);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "hello world"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn a_failing_command_is_a_result_and_not_a_failure() {
    // The distinction a widget has to be able to draw: `git status` in a
    // directory that is not a repository *ran*, and said no. That is different
    // from `git` not being installed, and a widget should say so differently.
    let output = ran(Commands::new().run("false", &[]));
    assert_ne!(output.status, 0);

    let missing = Commands::new().run("uniwidged-no-such-command", &[]);
    match missing {
        TaskResult::Failed(why) => assert!(
            why.contains("not installed"),
            "a missing command should say so plainly, got {why:?}"
        ),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn there_is_no_shell_anywhere() {
    // The injection everybody expects, and the reason `Command::new` is used
    // directly. If any of these were passed through a shell, the semicolon
    // would start a second command and the output would not be the literal
    // string.
    for hostile in [
        "hello; touch /tmp/uniwidged-should-not-exist",
        "$(touch /tmp/uniwidged-should-not-exist)",
        "`id`",
        "a && b",
        "* ?",
    ] {
        let output = ran(Commands::new().run("echo", &[hostile.to_string()]));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            hostile,
            "the argument was interpreted rather than passed"
        );
    }
    assert!(
        !std::path::Path::new("/tmp/uniwidged-should-not-exist").exists(),
        "a shell ran something"
    );
}

#[test]
fn a_command_cannot_be_a_path() {
    // The manifest refuses a path at parse time and the policy refuses a name
    // that is not exactly a granted one. This is the third check, and it is
    // the one that holds if either of those is ever reached another way — a
    // widget naming `./something` inside its own data directory would
    // otherwise be running code it wrote itself.
    for hostile in ["/bin/echo", "./echo", "../echo", "sub/echo", ""] {
        match Commands::new().run(hostile, &[]) {
            TaskResult::Failed(why) => assert!(why.contains("not a command name"), "{why}"),
            other => panic!("{hostile:?} should not have run, got {other:?}"),
        }
    }
}

#[test]
fn a_command_gets_almost_no_environment() {
    // A desktop process's environment routinely holds session tokens, and a
    // widget's command inheriting it would be the widget inheriting it. `PATH`
    // is the one exception, because a command has to be findable.
    let output = ran(Commands::new().run("env", &[]));
    let environment = String::from_utf8_lossy(&output.stdout);
    let names: Vec<&str> = environment
        .lines()
        .filter_map(|line| line.split('=').next())
        .filter(|name| !name.is_empty())
        .collect();
    assert_eq!(
        names,
        vec!["PATH"],
        "a command should see PATH and nothing else, saw {names:?}"
    );
}

#[test]
fn a_command_that_will_not_stop_is_killed() {
    // The bound on how long one widget can occupy a worker. Without it a
    // widget granted `command:sleep` could hold one for as long as it liked,
    // and eight of them could hold the whole budget forever.
    let began = Instant::now();
    let output = ran(Commands::new()
        .with_timeout(Duration::from_millis(300))
        .run("sleep", &["30".into()]));
    assert_eq!(output.status, -1, "killed, rather than exited");
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "it took {:?}",
        began.elapsed()
    );
}

#[test]
fn a_command_that_says_too_much_is_truncated_rather_than_refused() {
    // Truncated, because a widget reading the first line of something should
    // get it even if what is upstream has gone mad — and capped, because
    // otherwise the output is in the desktop's memory and then across the
    // sandbox boundary.
    let output = ran(Commands::new()
        .with_max_output(64)
        .run("yes", &["spam".into()]));
    assert_eq!(output.stdout.len(), 64);
    // And the process was not left blocked on a pipe nobody was reading, which
    // is what would happen if the cap simply stopped reading.
    assert_eq!(
        output.status, -1,
        "`yes` is stopped by the timeout or a signal"
    );
}

#[test]
fn a_command_that_writes_a_lot_still_finishes() {
    // The deadlock this guards against is real and common: waiting for the
    // process before reading its output stalls the moment the child fills the
    // 64 KB pipe buffer. Half a megabyte is comfortably past that.
    let output = ran(Commands::new()
        .with_timeout(Duration::from_secs(20))
        .run("head", &["-c".into(), "500000".into(), "/dev/zero".into()]));
    assert_eq!(output.status, 0, "it should have exited normally");
    // Half a megabyte written, the default quarter-megabyte cap kept — and,
    // crucially, the process still reached its own end rather than blocking
    // forever on a pipe nobody was draining.
    assert_eq!(output.stdout.len(), crate::tasks::MAX_OUTPUT);
}

#[test]
fn a_command_gets_no_standard_input() {
    // A command that waits for input would otherwise occupy a worker until its
    // timeout. `cat` with no argument reads stdin and stops at end of file,
    // which is immediate when there is nothing there.
    let began = Instant::now();
    let output = ran(Commands::new()
        .with_timeout(Duration::from_secs(5))
        .run("cat", &[]));
    assert_eq!(output.status, 0);
    assert!(output.stdout.is_empty());
    assert!(began.elapsed() < Duration::from_secs(2));
}

// ---------------------------------------------------------------------------
// The quota
// ---------------------------------------------------------------------------

/// The per-file cap bounds one call; this bounds a loop. Both are needed, and
/// the tests below are mostly about the widget that is *not* misbehaving —
/// refusing a widget that rewrites its own cache file would break every honest
/// widget and leave the loop working.

#[test]
fn a_widget_cannot_fill_a_disk_one_file_at_a_time() {
    let scratch = Scratch::new("quota");
    let dir = scratch.dir().with_max_total_bytes(1024);
    for i in 0..8 {
        dir.write(&format!("chunk{i}"), &[0u8; 128]).expect("write");
    }
    assert_eq!(dir.used_bytes(), Ok(1024));
    let error = dir
        .write("one-more", &[0u8; 1])
        .expect_err("should be refused");
    assert!(matches!(
        error,
        StoreError::Full {
            limit: 1024,
            used: 1024
        }
    ));
    // Refused, and nothing written: a quota that leaves the file behind is a
    // quota that can be exceeded by ignoring the error.
    assert_eq!(dir.read("one-more"), Ok(None));
}

#[test]
fn rewriting_the_same_file_at_full_size_is_not_refused() {
    let scratch = Scratch::new("quota-rewrite");
    let dir = scratch.dir().with_max_total_bytes(1024);
    dir.write("cache", &[0u8; 1024]).expect("write");
    // The ordinary case for the widget this quota exists to bound: a cache
    // that is rewritten every hour and stays the same size. Checking the
    // directory as it stands would refuse it forever.
    dir.write("cache", &[1u8; 1024]).expect("rewrite");
    assert_eq!(dir.read("cache"), Ok(Some(vec![1u8; 1024])));
    assert_eq!(dir.used_bytes(), Ok(1024));
}

#[test]
fn a_smaller_rewrite_makes_room() {
    let scratch = Scratch::new("quota-shrink");
    let dir = scratch.dir().with_max_total_bytes(1024);
    dir.write("cache", &[0u8; 1000]).expect("write");
    assert!(dir.write("other", &[0u8; 100]).is_err());
    dir.write("cache", &[0u8; 100]).expect("shrink");
    dir.write("other", &[0u8; 100]).expect("now there is room");
    assert_eq!(dir.used_bytes(), Ok(200));
}

#[test]
fn nested_directories_are_counted() {
    let scratch = Scratch::new("quota-nested");
    let dir = scratch.dir().with_max_total_bytes(1024);
    dir.write("a/b/c/deep", &[0u8; 900]).expect("write");
    assert_eq!(dir.used_bytes(), Ok(900));
    assert!(matches!(
        dir.write("shallow", &[0u8; 200]),
        Err(StoreError::Full { .. })
    ));
}

#[test]
fn an_empty_directory_uses_nothing_and_a_missing_one_is_not_an_error() {
    let scratch = Scratch::new("quota-empty");
    // Nothing has been written, so the root does not exist. A widget's first
    // write must not fail because the walk found no directory.
    assert_eq!(scratch.dir().used_bytes(), Ok(0));
    let dir = scratch.dir();
    dir.write("first", b"x").expect("write");
    assert_eq!(dir.used_bytes(), Ok(1));
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_directory_is_not_charged_to_the_widget() {
    let scratch = Scratch::new("quota-symlink");
    let dir = scratch.dir().with_max_total_bytes(1024);
    dir.write("real", &[0u8; 100]).expect("write");
    let outside = scratch.0.join("elsewhere");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("huge"), vec![0u8; 100_000]).expect("huge");
    std::os::unix::fs::symlink(
        outside.join("huge"),
        scratch.0.join("widgets/example/pointer"),
    )
    .expect("symlink");
    // Counting the target would let a user who symlinked something in hand the
    // widget a bill for it, and would let a widget shrink its own apparent
    // usage by pointing at an empty file elsewhere. Neither is the widget's
    // storage. Writing *through* it is what the containment checks refuse.
    assert_eq!(dir.used_bytes(), Ok(100));
}

#[test]
fn the_default_quota_is_generous_enough_not_to_be_hit_by_accident() {
    // A number, asserted, so that lowering it is a decision somebody makes
    // rather than a typo. Thirty-two megabytes: far more than a widget's
    // cache, far less than an amount worth noticing.
    assert_eq!(DEFAULT_MAX_TOTAL_BYTES, 32 * 1024 * 1024);
    assert!(DEFAULT_MAX_TOTAL_BYTES > DEFAULT_MAX_FILE_BYTES as u64);
}

#[test]
fn an_ordinary_write_does_not_walk_the_directory() {
    // A write happens inside a widget's `view`, on its deadline, and walking
    // the directory to answer "is there room?" is work proportional to what is
    // already there. The first version did that on every write; this asserts
    // that it does not, by making the walk *impossible* — a directory the
    // process cannot read would fail one — and checking the write still lands.
    //
    // Asserted as a property rather than a timing, because a timing test of a
    // filesystem measures the machine.
    let scratch = Scratch::new("quota-cheap");
    let dir = scratch.dir().with_max_total_bytes(1024);
    dir.write("first", &[0u8; 100])
        .expect("the first write walks once");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch.0.join("widgets/example");
        let readable = std::fs::metadata(&root).expect("metadata").permissions();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o300))
            .expect("make it unlistable");
        // Unlistable, so a walk cannot succeed — and the write does.
        let wrote = dir.write("second", &[0u8; 100]);
        std::fs::set_permissions(&root, readable).expect("restore");
        wrote.expect("a write under quota should not need a walk");
    }
    assert_eq!(dir.used_bytes(), Ok(200));
}

#[test]
fn a_refusal_recounts_before_it_refuses() {
    // The cache can drift high — a user deleted a cache file by hand, a
    // `--purge` was interrupted — and a quota that refused on a stale number
    // would leave a widget permanently unable to write with an empty
    // directory. So the walk happens exactly where it decides something.
    let scratch = Scratch::new("quota-stale");
    let dir = scratch.dir().with_max_total_bytes(1024);
    dir.write("big", &[0u8; 1000]).expect("write");
    assert!(matches!(
        dir.write("more", &[0u8; 100]),
        Err(StoreError::Full { .. })
    ));

    // Deleted from outside, which the cache cannot know about.
    std::fs::remove_file(scratch.0.join("widgets/example/big")).expect("remove");
    dir.write("more", &[0u8; 100])
        .expect("the recount should find the room");
    assert_eq!(dir.used_bytes(), Ok(100));
}

#[test]
fn a_write_that_failed_is_not_counted() {
    // A write charged before it landed would shrink a widget's room for good,
    // one failure at a time. The refusal here is the per-file cap, which is
    // checked before anything touches the disk.
    let scratch = Scratch::new("quota-failed");
    let dir = scratch
        .dir()
        .with_max_total_bytes(1024)
        .with_max_file_bytes(64);
    dir.write("small", &[0u8; 10]).expect("write");
    for _ in 0..100 {
        assert!(matches!(
            dir.write("toobig", &[0u8; 65]),
            Err(StoreError::TooLarge { .. })
        ));
    }
    // A hundred refusals later there is still the room there always was. Had
    // each one been counted, the quota would show 6 510 bytes used and the
    // next write would be refused.
    for i in 0..15 {
        dir.write(&format!("k{i}"), &[0u8; 64]).expect("still room");
    }
    assert_eq!(dir.used_bytes(), Ok(10 + 15 * 64));
}

#[test]
fn a_clone_of_the_handle_shares_the_count() {
    // A widget that restarts is handed the same `Services`, and its `DataDir`
    // is cloned into each fresh instance. Two counts that disagreed would let
    // a widget double its quota by failing.
    let scratch = Scratch::new("quota-clone");
    let dir = scratch.dir().with_max_total_bytes(1024);
    let restarted = dir.clone();
    dir.write("a", &[0u8; 600]).expect("write");
    assert!(matches!(
        restarted.write("b", &[0u8; 600]),
        Err(StoreError::Full { .. })
    ));
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[test]
fn memory_in_use_is_what_the_kernel_could_not_hand_out() {
    // `MemTotal - MemAvailable`, not `MemTotal - MemFree`. The page cache is
    // in the difference between the two, and counting it is how a monitor
    // reads 95 % on an idle machine.
    let (used, total) = parse_meminfo(
        "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    9000000 kB\nBuffers:          500000 kB\n",
    )
    .expect("parsed");
    assert_eq!(total, 16_000_000);
    assert_eq!(used, 7_000_000);
}

#[test]
fn a_kernel_without_mem_available_is_refused_rather_than_guessed() {
    // Before 3.14 there was no `MemAvailable` and every tool estimated its
    // own from `Cached` and `Buffers`, each wrongly in its own way. A refusal
    // is honest; an estimate is a number a widget would draw.
    assert_eq!(
        parse_meminfo("MemTotal:       16000000 kB\nMemFree:         1000000 kB\n"),
        None
    );
    assert_eq!(parse_meminfo(""), None);
    assert_eq!(
        parse_meminfo("MemTotal: banana kB\nMemAvailable: 1 kB\n"),
        None
    );
}

#[test]
fn the_memory_reading_is_a_fraction_and_does_not_block() {
    // The same two properties `cpu_load` promises. On a platform with no
    // `/proc/meminfo` the honest answer is a refusal, which is also accepted.
    let counters = Counters::new();
    let began = Instant::now();
    let answer = counters.memory_load();
    assert!(
        began.elapsed() < Duration::from_millis(50),
        "memory_load blocked"
    );
    if let Ok(load) = answer {
        assert!((0.0..=1.0).contains(&load), "{load}");
        // No machine running this test has *nothing* in use.
        assert!(load > 0.0, "{load}");
    }
}
