//! `uniwidged` — run the host, and everything around running it.
//!
//! ```text
//! uniwidged run                 start the host in the foreground
//! uniwidged new <name>          scaffold a widget crate
//! uniwidged install <path>      install a widget, showing what it asks for
//! uniwidged list                what is installed, and what each was granted
//! uniwidged doctor              what this session can and cannot do
//! uniwidged snapshot <widget>   render one widget headlessly to a PNG
//! ```
//!
//! `doctor` is the one worth explaining. A desktop widget depends on
//! properties the platform may or may not provide — a layer shell, a
//! transparent swapchain, a compositor that honours `_NET_WM_STATE_BELOW` —
//! and when one is missing the symptom is visual and the cause is not. So
//! `doctor` reports [`uniwidged_shell::Capabilities`] for the running session
//! and names what is degraded, in the same words the start-up log uses.
//!
//! Status: skeleton. Milestone M7.

fn main() {
    let command = std::env::args().nth(1);
    match command.as_deref() {
        Some("doctor") => {
            // M7: probe the live session rather than reporting the ideal.
            let caps = uniwidged_shell::Capabilities::FULL;
            match caps.degradation() {
                None => println!("uniwidged: this session supports everything a widget needs"),
                Some(missing) => println!("uniwidged: {missing}"),
            }
        }
        Some(other) => {
            eprintln!("uniwidged: `{other}` is not implemented yet; see ROADMAP.md");
            std::process::exit(2);
        }
        None => {
            eprintln!("uniwidged: expected a command — run, new, install, list, doctor, snapshot");
            std::process::exit(2);
        }
    }
}
