//! `uniwidged new` — a widget that builds, before it does anything.
//!
//! The scaffold's job is to get somebody to a **running widget on their
//! desktop** in two commands, because the gap between "I read the guide" and
//! "something appeared" is where most frameworks lose people. So what it
//! generates is not a stub: it draws, it responds to a click, it schedules
//! itself, and it has a test — all of which the author then edits rather than
//! writes from nothing.
//!
//! ## The SDK dependency
//!
//! `uniwidged-guest` is not on crates.io yet, so a generated widget has to
//! reach it by path. When `new` is run inside a checkout it finds one and says
//! so; when it is run anywhere else it writes a version requirement and a
//! comment explaining what to do until that resolves, and `--sdk <path>`
//! settles it explicitly. Guessing silently — or writing a path that does not
//! exist — would produce a project whose first `cargo build` fails for a
//! reason nothing in it explains.

use std::path::{Path, PathBuf};

use uniwidged_runtime::manifest::Manifest;

/// Where a generated widget gets the SDK from.
enum Sdk {
    /// A checkout, found by walking up from here.
    Checkout(PathBuf),
    /// Nowhere yet. The generated manifest says so in a comment.
    Unpublished,
}

impl Sdk {
    /// Looks for this repository's own crates.
    ///
    /// **From where the command was run, and only then from where the project
    /// is being written.** The first version searched only the output
    /// directory, which meant `uniwidged new demo --dir /tmp/x` from inside a
    /// checkout found nothing and generated a project that could not build —
    /// found by the test that builds what it generates, which is what that
    /// test is for. Somebody running this from a checkout is working on this
    /// project wherever they are putting the result.
    ///
    /// An explicit `--sdk` overrides both, for the case where neither guess is
    /// right.
    fn find(explicit: Option<&str>, from: &[&Path]) -> Self {
        if let Some(path) = explicit {
            return Sdk::Checkout(PathBuf::from(path));
        }
        for start in from {
            for ancestor in start.ancestors() {
                // By the *crate* rather than by a marker file: a directory
                // named `uniwidged` that is not a checkout would produce a
                // project that cannot build, and what matters is whether the
                // dependency is actually there.
                let crates = ancestor.join("crates");
                if crates.join("uniwidged-guest/Cargo.toml").is_file() {
                    return Sdk::Checkout(crates);
                }
            }
        }
        Sdk::Unpublished
    }

    /// The `[dependencies]` block for a widget at `widget_dir`.
    fn dependencies(&self, widget_dir: &Path) -> String {
        match self {
            Sdk::Checkout(crates) => {
                let relative = |name: &str| {
                    let target = crates.join(name);
                    relative_path(widget_dir, &target)
                        .unwrap_or_else(|| target.display().to_string())
                };
                format!(
                    "uniwidged-guest = {{ path = \"{}\" }}\n\
                     uniwidged-core = {{ path = \"{}\" }}\n\
                     uniwidged-view = {{ path = \"{}\" }}\n",
                    relative("uniwidged-guest"),
                    relative("uniwidged-core"),
                    relative("uniwidged-view"),
                )
            }
            Sdk::Unpublished => concat!(
                "# uniwidged is not on crates.io yet. Until it is, point these at a\n",
                "# checkout:  path = \"/path/to/uniwidged/crates/uniwidged-guest\"\n",
                "uniwidged-guest = \"0.0.1\"\n",
                "uniwidged-core = \"0.0.1\"\n",
                "uniwidged-view = \"0.0.1\"\n",
            )
            .to_string(),
        }
    }

    fn describe(&self) -> String {
        match self {
            Sdk::Checkout(crates) => format!("the SDK at {}", crates.display()),
            Sdk::Unpublished => {
                "uniwidged-guest 0.0.1 — unpublished, so edit Cargo.toml to point at a checkout"
                    .to_string()
            }
        }
    }
}

/// `../..`-style path from `from` to `to`, when both are absolute.
///
/// Written out rather than pulled in, because the only case that matters is a
/// widget directory beside a checkout, and a dependency for that would be a
/// dependency in everything this tool ships.
fn relative_path(from: &Path, to: &Path) -> Option<String> {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    // One shared component is the filesystem root, which every absolute path
    // has. A "relative" path from there is `../../../..` and then the whole of
    // the other path, which is longer than the absolute one and harder to
    // read — so the caller falls back. Nothing shared at all means two roots,
    // where no relative path exists.
    if shared <= 1 {
        return None;
    }
    let mut path = String::new();
    for _ in shared..from.len() {
        path.push_str("../");
    }
    for (index, component) in to[shared..].iter().enumerate() {
        if index > 0 {
            path.push('/');
        }
        path.push_str(&component.as_os_str().to_string_lossy());
    }
    Some(path)
}

/// Creates a widget project.
pub fn create(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|name| !name.starts_with('-'))
        .ok_or("what should it be called? `uniwidged new <name>`")?;

    // Validated by the runtime's own parser, so that a name `new` accepts is a
    // name the host will run — the name becomes a directory and a keychain
    // namespace, and finding that out at install time would be too late.
    let capabilities = collect(args, "--capability");
    let manifest_text = widget_toml(name, &capabilities);
    let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;

    let parent = match crate::flag(args, "--dir") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let here = std::env::current_dir().map_err(|e| e.to_string())?;
    let absolute = if dir.is_absolute() {
        dir.clone()
    } else {
        here.join(&dir)
    };
    let sdk = Sdk::find(
        crate::flag(args, "--sdk").as_deref(),
        &[here.as_path(), absolute.as_path()],
    );

    write(&dir.join("widget.toml"), &manifest_text)?;
    write(&dir.join("Cargo.toml"), &cargo_toml(name, &sdk, &absolute))?;
    write(&dir.join("src/lib.rs"), &lib_rs(name))?;
    write(&dir.join(".gitignore"), "/target\n")?;
    write(&dir.join("README.md"), &readme(name))?;

    println!("\n  created {}", dir.display());
    println!("  against {}", sdk.describe());
    if manifest.capabilities.is_empty() {
        println!("  it asks for nothing, which is where every widget should start");
    } else {
        println!("  it asks to:");
        for line in manifest.consent_summary() {
            println!("    · {line}");
        }
    }
    println!(
        "\n  cd {name}\n  \
         cargo test                                    # its logic, natively\n  \
         cargo build --release --target wasm32-wasip2  # the component\n  \
         uniwidged install target/wasm32-wasip2/release/{}.wasm --allow-unsigned\n  \
         uniwidged run\n\n  \
         `--allow-unsigned` is for your own builds. To publish, \
         `uniwidged keygen`\n  once and `uniwidged sign` each release; the \
         people installing it\n  `uniwidged trust` your public key.\n",
        name.replace('-', "_")
    );
    Ok(())
}

/// Every value of a repeated flag, in order.
fn collect(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, arg)| *arg == name)
        .filter_map(|(index, _)| args.get(index + 1).cloned())
        .collect()
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

fn widget_toml(name: &str, capabilities: &[String]) -> String {
    let list = if capabilities.is_empty() {
        // Not an empty list with no explanation: the first question a widget
        // author has is "how do I reach the network", and the answer belongs
        // where they will look for it.
        "# Everything your widget may reach, and the whole of what your users\n\
         # are asked to approve. Host-scoped on purpose — `net:api.example.com`\n\
         # is a sentence somebody can agree to, and `network` is not.\n\
         #\n\
         #   timer            ask to be woken\n\
         #   locale           format dates and times\n\
         #   measure          measure your own text\n\
         #   metrics          CPU and memory counters\n\
         #   fs:<subpath>     read and write one folder of your own\n\
         #   net:<host>       HTTPS to exactly that host\n\
         #   secrets          the keychain, namespaced to you\n\
         #   command:<name>   run one declared external command\n\
         #   link:<host>      open that host in the browser, when a user\n\
         #                    clicks a link you drew\n\
         capabilities = []\n"
            .to_string()
    } else {
        format!(
            "capabilities = [{}]\n",
            capabilities
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         description = \"A uniwidged widget\"\n\
         {list}\
         # The most memory this widget may use. Shown to your users before they\n\
         # install it, so ask for what you need and not what you might.\n\
         memory-mb = 16\n\
         \n\
         # What your users can change. Declared here rather than parsed by your\n\
         # widget, so the host checks the type and the range before you are\n\
         # started — which is why the code reading this has no branch for\n\
         # somebody typing nonsense, and needs none.\n\
         #\n\
         # `type` is text, number, toggle or choice. Every setting needs a\n\
         # `default`: an unset setting is its default, which is what lets you\n\
         # read one without a fallback. A number takes `min` and `max`; a\n\
         # choice takes `options`.\n\
         [[setting]]\n\
         key = \"greeting\"\n\
         label = \"What to say\"\n\
         type = \"text\"\n\
         default = \"Hello\"\n"
    )
}

fn cargo_toml(name: &str, sdk: &Sdk, dir: &Path) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\n\
         # Its own workspace. A widget is built for `wasm32-wasip2`, and being a\n\
         # member of some other workspace means being built for that one's target.\n\
         [workspace]\n\n\
         [lib]\n\
         # What the component model needs. There is no `main`.\n\
         crate-type = [\"cdylib\"]\n\n\
         [dependencies]\n\
         {}\n\
         [profile.release]\n\
         # A widget is loaded into a desktop that is already running. Size is\n\
         # what the user notices; throughput is not.\n\
         opt-level = \"s\"\n\
         lto = true\n\
         strip = true\n",
        sdk.dependencies(dir)
    )
}

fn lib_rs(name: &str) -> String {
    format!(
        r##"//! {name} — a uniwidged widget.
//!
//! You describe; the host decides what it looks like. There are no colours,
//! no fonts, no pixels and no coordinates here, and that is what makes a
//! screenful of widgets by different authors look like one product.

use uniwidged_guest::prelude::*;

/// What `widget.toml` declares as this setting's default.
///
/// Only reachable on a host older than your manifest, since a declared key
/// always has a value — but a fallback that matches the manifest costs a line
/// and makes that impossible to get wrong.
const GREETING: &str = "Hello";

pub struct Widget_ {{
    clicks: u32,
    /// Read in `start` and again on `Configured`, never in `view`: `view` runs
    /// on a deadline and may run sixty times a second, and this changes when
    /// somebody changes it and at no other time.
    greeting: String,
}}

impl Default for Widget_ {{
    fn default() -> Self {{
        Self {{
            clicks: 0,
            greeting: GREETING.to_string(),
        }}
    }}
}}

impl Widget for Widget_ {{
    /// Called once, before the first view. Load anything you remembered here,
    /// and read your settings.
    fn start(&mut self, cx: &Cx) -> Result<(), String> {{
        self.read_settings(cx);
        Ok(())
    }}

    /// Describe what should be on screen.
    ///
    /// Called whenever the host thinks something may have changed. Return the
    /// same tree and nothing is drawn at all — so build it fresh every time
    /// and let the comparison do the work.
    fn view(&mut self, _cx: &Cx) -> View {{
        panel(NodeId::named("root"))
            .gap(2)
            .child(text(self.greeting.clone()).role(Role::Title))
            .child(
                text(self.clicks.to_string())
                    .role(Role::Display)
                    .id(NodeId::named("clicks")),
            )
            .child(
                chip("Click me")
                    .accent(Accent::Brand)
                    .on_click(NodeId::named("button"))
                    .label("Count one more"),
            )
            .view()
    }}

    /// React. Return `true` if your view would now differ.
    ///
    /// You are told *which node* was activated, never where the pointer was:
    /// the host did the layout, so the host does the hit-testing.
    fn on_event(&mut self, event: &Event, cx: &Cx) -> bool {{
        match event {{
            Event::Clicked(id) if *id == NodeId::named("button") => {{
                self.clicks += 1;
                true
            }}
            // The user changed one of your settings. Read them again.
            Event::Configured => {{
                let was = std::mem::take(&mut self.greeting);
                self.read_settings(cx);
                was != self.greeting
            }}
            _ => false,
        }}
    }}

    /// When you would like to be called again.
    ///
    /// `Never` is right far more often than it looks — this widget changes
    /// when it is clicked and at no other time, and a widget that asks for a
    /// timer it does not need is the one thing that makes a desktop full of
    /// them cost something.
    fn wake(&self) -> Wake {{
        Wake::Never
    }}
}}

impl Widget_ {{
    /// Reads what `widget.toml` declared.
    ///
    /// The `_ =>` arm is only reachable on a host that does not know your
    /// manifest: the host checks the declared type before you see a value, so
    /// a key you declared as text cannot arrive as anything else.
    fn read_settings(&mut self, cx: &Cx) {{
        self.greeting = match cx.setting("greeting") {{
            Some(SettingValue::Text(text)) => text,
            _ => GREETING.to_string(),
        }};
    }}
}}

uniwidged_guest::export!(Widget_);

#[cfg(test)]
mod tests {{
    use super::*;

    /// Everything except the host works natively, so your widget's own logic
    /// is testable with no WASM toolchain at all.
    #[test]
    fn clicking_the_button_counts() {{
        let mut widget = Widget_::default();
        assert!(widget.on_event(&Event::Clicked(NodeId::named("button")), &Cx::new()));
        assert_eq!(widget.clicks, 1);
    }}

    #[test]
    fn clicking_anything_else_does_not() {{
        let mut widget = Widget_::default();
        assert!(!widget.on_event(&Event::Clicked(NodeId::named("elsewhere")), &Cx::new()));
    }}

    #[test]
    fn a_host_that_knows_no_settings_gets_the_declared_default() {{
        // Off-target `Cx::setting` answers `None`, which is what a host older
        // than your manifest looks like.
        let mut widget = Widget_ {{
            clicks: 0,
            greeting: String::new(),
        }};
        widget.read_settings(&Cx::new());
        assert_eq!(widget.greeting, GREETING);
    }}
}}
"##
    )
}

fn readme(name: &str) -> String {
    let underscored = name.replace('-', "_");
    format!(
        "# {name}\n\n\
         A [uniwidged](https://github.com/j-pfalzgraf/uniwidged) widget.\n\n\
         ```sh\n\
         cargo test                                    # its logic, natively\n\
         cargo build --release --target wasm32-wasip2  # the component\n\
         uniwidged check target/wasm32-wasip2/release/{underscored}.wasm\n\
         uniwidged install target/wasm32-wasip2/release/{underscored}.wasm --allow-unsigned\n\
         uniwidged configure {name}                     # the knobs it declares\n\
         uniwidged run\n\
         ```\n\n\
         `--allow-unsigned` is for your own builds. To publish, run\n\
         `uniwidged keygen` once and `uniwidged sign` each release, and give\n\
         people your public key so they can `uniwidged trust` it.\n\n\
         `widget.toml` is what your users are asked to approve. Everything in\n\
         it is enforced by the host, so ask for what you need and nothing more.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_climbs_out_and_back_in() {
        assert_eq!(
            relative_path(
                Path::new("/home/me/widgets/demo"),
                Path::new("/home/me/uniwidged/crates/uniwidged-guest")
            )
            .as_deref(),
            Some("../../uniwidged/crates/uniwidged-guest")
        );
    }

    #[test]
    fn a_path_to_a_sibling_does_not_climb_further_than_it_has_to() {
        assert_eq!(
            relative_path(Path::new("/a/b/demo"), Path::new("/a/b/crates/g")).as_deref(),
            Some("../crates/g")
        );
    }

    #[test]
    fn two_roots_have_no_relative_path_between_them() {
        // On Windows, `C:\` and `D:\`. The caller falls back to an absolute
        // path, which is correct and which a naive `../..` count would not be.
        assert_eq!(relative_path(Path::new("a/b"), Path::new("/c/d")), None);
    }

    #[test]
    fn a_generated_manifest_is_one_the_runtime_accepts() {
        // The generator and the host agree about names and capabilities
        // because they use the same parser. A scaffold that produced something
        // `uniwidged install` then refused would be worse than no scaffold.
        let text = widget_toml("my-widget", &["timer".into(), "fs:cache".into()]);
        let manifest = Manifest::parse(&text).expect("generated manifests parse");
        assert_eq!(manifest.name, "my-widget");
        assert_eq!(manifest.capabilities.len(), 2);
        assert_eq!(manifest.limits.memory_bytes, 16 * 1024 * 1024);
    }

    #[test]
    fn a_generated_manifest_with_no_capabilities_still_parses() {
        let manifest = Manifest::parse(&widget_toml("plain", &[])).expect("parses");
        assert!(manifest.capabilities.is_empty());
        assert_eq!(manifest.consent_summary(), vec!["asks for nothing at all"]);
    }

    #[test]
    fn a_name_the_host_would_refuse_is_refused_here() {
        // Uppercase, a leading digit, a path separator: all become a directory
        // and a keychain namespace, and all are refused at the point somebody
        // can still change their mind cheaply.
        for bad in ["Widget", "9lives", "my widget", "../escape", ""] {
            assert!(
                Manifest::parse(&widget_toml(bad, &[])).is_err(),
                "{bad:?} should not be a widget name"
            );
        }
    }
}
