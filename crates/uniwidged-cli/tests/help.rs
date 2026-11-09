//! `uniwidged help <command>` and `uniwidged <command> --help`.
//!
//! The question somebody asks before any other, and the one a binary most
//! often answers by doing something instead — `install --help` used to look
//! for a file called `--help`.

use std::process::Command;

fn uniwidged(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_uniwidged"))
        .args(args)
        // Nowhere real: help must never read or write a configuration.
        .env("UNIWIDGED_CONFIG", "/nonexistent/uniwidged/config.toml")
        .output()
        .expect("the CLI should be runnable");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn every_command_answers_help_with_its_own_usage() {
    for command in [
        "run",
        "new",
        "check",
        "install",
        "registry",
        "search",
        "add",
        "update",
        "list",
        "enable",
        "disable",
        "remove",
        "keygen",
        "sign",
        "trust",
        "untrust",
        "trusted",
        "lock",
        "unlock",
        "move",
        "configure",
        "reload",
        "log",
        "status",
        "autostart",
        "doctor",
        "snapshot",
        "completions",
        "help",
    ] {
        let (ok, said) = uniwidged(&["help", command]);
        assert!(ok, "`help {command}` failed: {said}");
        assert!(
            said.starts_with(&format!("uniwidged {command} — ")),
            "{said}"
        );
        assert!(
            said.contains("USAGE:"),
            "`help {command}` has no usage line: {said}"
        );
    }
}

#[test]
fn help_after_a_command_is_a_question_and_not_an_argument() {
    let (ok, said) = uniwidged(&["install", "--help"]);
    assert!(ok, "{said}");
    assert!(said.contains("--manifest"), "{said}");
    assert!(
        !said.contains("No such file"),
        "it went looking for a file: {said}"
    );

    let (ok, said) = uniwidged(&["move", "clock", "-h"]);
    assert!(ok, "{said}");
    assert!(said.contains("--output"), "{said}");
}

#[test]
fn help_for_something_that_is_not_a_command_says_so() {
    let (ok, said) = uniwidged(&["help", "frobnicate"]);
    assert!(!ok);
    assert!(said.contains("not a command"), "{said}");
}

#[test]
fn a_flag_value_that_looks_like_help_is_still_a_value() {
    // `--config -h` names a file called `-h`; the command runs rather than
    // printing its help and claiming success.
    let (_, said) = uniwidged(&["list", "--config", "-h"]);
    assert!(!said.starts_with("uniwidged list — "), "{said}");
    // A value consumed, and then a question: help, and nothing run.
    let (ok, said) = uniwidged(&["remove", "clock", "--config", "-h", "--help"]);
    assert!(ok && said.starts_with("uniwidged remove — "), "{said}");
    // After a switch, though, it is a question.
    let (ok, said) = uniwidged(&["list", "--names", "--help"]);
    assert!(ok && said.starts_with("uniwidged list — "), "{said}");
}
