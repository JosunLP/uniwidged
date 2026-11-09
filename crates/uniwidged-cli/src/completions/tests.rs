//! The scripts, and the table they are written from.
//!
//! The scripts themselves are checked by the shells where a shell is on the
//! machine — `bash -n` is a parse, which is the whole of what can go wrong in
//! a generated script — and by content where one is not.

use super::*;

/// Every command `main` dispatches on, by hand, so the table cannot drift
/// from it without this test noticing. The list in `main` is a `match`, which
/// nothing can introspect; this is the next best thing.
const DISPATCHED: &[&str] = &[
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
];

#[test]
fn every_dispatched_command_has_a_row_and_nothing_else_does() {
    let mut table: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    let mut dispatched: Vec<&str> = DISPATCHED.to_vec();
    table.sort_unstable();
    dispatched.sort_unstable();
    assert_eq!(table, dispatched);
}

#[test]
fn every_script_names_every_command_and_every_flag() {
    for (shell, script) in [("bash", bash()), ("zsh", zsh()), ("fish", fish())] {
        for command in COMMANDS {
            assert!(
                script.contains(command.name),
                "{shell}: no completion for `{}`",
                command.name
            );
            for flag in command.flags {
                // Fish spells `--purge` as `-l purge`.
                let spelled = if shell == "fish" {
                    flag.trim_start_matches('-').to_string()
                } else {
                    flag.to_string()
                };
                assert!(
                    script.contains(&spelled),
                    "{shell}: `{}` lost its `{flag}`",
                    command.name
                );
            }
        }
        // And widget names come from the binary, not from a list somebody
        // typed into the script.
        assert!(script.contains("uniwidged list --names"), "{shell}");
    }
}

/// Hands a script to `<shell> -n`, which parses without running.
///
/// `None` where the shell is not on this machine.
fn parsed_by(shell: &str, script: &str) -> Option<std::process::Output> {
    use std::io::Write;
    let mut child = std::process::Command::new(shell)
        .arg("-n")
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write");
    Some(child.wait_with_output().expect("wait"))
}

#[test]
fn the_bash_script_parses() {
    // A parse is the whole of what can go wrong in a generated script, and
    // bash is on every machine this test runs on.
    let script = bash();
    // On Windows, `bash` on `PATH` is WSL's launcher, which on a machine with
    // no distribution installed answers every script with a sentence about
    // installing one. Git for Windows' bash is the one a Windows user runs
    // these completions in, so that is the one asked.
    let shell = if cfg!(windows) {
        r"C:\Program Files\Git\bin\bash.exe"
    } else {
        "bash"
    };
    let output = parsed_by(shell, &script).expect("bash is on this machine");
    assert!(
        output.status.success(),
        "bash refused the script: {}\n{script}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_other_shells_parse_where_they_are_installed() {
    // Not on every machine, and not skipped silently: a run that could not
    // check says so on stderr, which is the same bargain the platform tests
    // make.
    for (shell, script) in [("zsh", zsh()), ("fish", fish())] {
        let Some(output) = parsed_by(shell, &script) else {
            eprintln!("skipping: no {shell} on this machine");
            continue;
        };
        assert!(
            output.status.success(),
            "{shell} refused the script: {}\n{script}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn a_shell_this_cannot_write_for_is_named_in_the_refusal() {
    let why = command(&["powershell".to_string()]).expect_err("refused");
    assert!(why.contains("powershell") && why.contains("bash"), "{why}");
    assert!(command(&[]).is_err());
}

#[test]
fn the_zsh_script_is_a_compdef_and_the_fish_script_disables_file_completion() {
    // The two facts each shell needs from the first line: zsh's autoloader
    // reads `#compdef` to know what the function completes, and fish
    // completes files for everything unless told not to.
    assert!(zsh().starts_with("#compdef uniwidged\n"));
    assert!(fish().contains("complete -c uniwidged -f\n"));
}
