use super::*;

#[test]
fn doctor_names_every_counter_a_widget_can_read() {
    // One line per function under `metrics`, in the order the interface lists
    // them. A counter added to the WIT file and forgotten here is a question
    // `doctor` can no longer answer, so the list is asserted, not sampled.
    let lines = machine_counters(&uniwidged_runtime::services::Counters::new());
    let names: Vec<&str> = lines
        .iter()
        .map(|line| line.split(':').next().unwrap_or_default())
        .collect();
    assert_eq!(names, ["cpu", "memory", "disk", "battery", "network"]);
}

#[test]
fn a_machine_without_a_battery_is_not_reported_as_broken() {
    // On Linux every counter is readable, so nothing says `unavailable` —
    // and a CI runner with no battery must read as "none", not as a fault.
    #[cfg(target_os = "linux")]
    {
        let lines = machine_counters(&uniwidged_runtime::services::Counters::new());
        for line in &lines {
            assert!(!line.contains("unavailable"), "{line}");
        }
        let battery = lines
            .iter()
            .find(|l| l.starts_with("battery:"))
            .expect("a battery line");
        if uniwidged_runtime::machine::battery()
            .expect("readable")
            .is_none()
        {
            assert!(battery.contains("mains"), "{battery}");
        }
    }
}
