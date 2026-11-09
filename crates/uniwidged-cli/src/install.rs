//! Installing a widget, wherever it came from.
//!
//! One path, used by `uniwidged install <file>` and by `uniwidged add <name>`
//! after a download. That is the point of the file: a widget fetched from a
//! registry must go through the *same* provenance check, the same consent
//! screen and the same configuration write as one somebody sent by email.
//! Downloading is not trusting, and a second install path is how a project
//! ends up with two answers to "was this signed by somebody you trust".
//!
//! ## What is preserved when a widget is installed over itself
//!
//! Where it sits, what it was configured with, and whether it was switched
//! off. An installation is a new *version of a widget*, not a new widget, and
//! somebody who has spent five minutes putting a widget where they want it
//! should not have it jump back to the corner because its author shipped a
//! bug fix.
//!
//! What is **not** preserved is the grant, on an explicit `install`: the list
//! is printed and agreed to again, because it may have changed and the whole
//! design says a user approved *that manifest's* list. An `update` is the
//! other way round — see [`Grant::AsBefore`], where the new manifest is
//! required to ask for nothing the old grant did not already cover.

use std::path::{Path, PathBuf};

use uniwidged_host::config::{Config, WidgetConfig, capability_text};
use uniwidged_runtime::manifest::Manifest;

use crate::trust;

/// What the widget being installed should be granted.
pub enum Grant {
    /// Everything its manifest declares, having shown the user the list.
    /// What an explicit installation does.
    Everything,
    /// Exactly what the previous version was granted, and refuse if the new
    /// manifest wants anything more.
    ///
    /// An unattended upgrade may not widen a permission. A version that asks
    /// for more is not refused *forever* — it is refused *silently*, and the
    /// user is told to run `uniwidged add`, where the new list is printed and
    /// agreed to like any other.
    AsBefore(Vec<String>),
}

/// `uniwidged install <widget.wasm> [--manifest F] [--allow-unsigned]`
pub fn command(args: &[String]) -> Result<(), String> {
    let component = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged install <widget.wasm>`")?;
    // Absolute, because it is recorded — but not resolved: a symlink is read
    // with the manifest and signature beside the link, as `sign` wrote them.
    let named = PathBuf::from(component);
    std::fs::metadata(&named).map_err(|e| format!("{component}: {e}"))?;
    let component = std::path::absolute(&named).map_err(|e| format!("{component}: {e}"))?;
    let manifest_path = manifest_for(&component, args)?;
    let allow_unsigned = args.iter().any(|a| a == "--allow-unsigned");
    install(
        &crate::config_path(args),
        &component,
        &manifest_path,
        allow_unsigned,
        Grant::Everything,
    )?;
    Ok(())
}

/// Where the manifest for `component` is.
///
/// `--manifest` if it was given. Otherwise, in order:
///
/// 1. **A build in a crate's `target/` directory** takes the crate's own
///    `widget.toml`. That is the source of truth while somebody is developing:
///    `sign` leaves a copy beside the component for publishing, and a copy
///    from the last signing must never shadow the manifest the author has
///    since edited. Not for a symlink, inside `target/` or pointing into
///    it: a link with a manifest beside it uses that manifest (rule 2), and
///    one with nothing beside it falls back to the crate's. `..` is resolved
///    first. In a
///    workspace the member is found by artifact name; a build no single crate
///    owns, with nothing beside it, has no manifest until `--manifest` names
///    one. A `Cargo.toml` between the component and `target/` makes it part
///    of that crate, not a build.
/// 2. **`<component>.toml` beside it**, as named — what a registry download
///    or a published set looks like. Checked by the name given rather than a
///    resolved symlink, so a linked component is read with the manifest
///    beside the link.
/// 3. **The `widget.toml` of the crate the component sits in**, searching
///    upward and stopping at the first directory with a `Cargo.toml`, so it
///    never wanders into some other project's manifest.
///
/// The first case is the one the authoring guide's first commands produce:
/// `cargo build` puts no manifest beside its output, and refusing that sent
/// every new author to copy a file by hand. Whatever is found is said out
/// loud when it is not the file beside the component.
pub fn manifest_for(component: &Path, args: &[String]) -> Result<PathBuf, String> {
    if let Some(explicit) = crate::flag(args, "--manifest") {
        // Absolute, because `install` records it: the desktop reads the
        // configuration from wherever it was started, and a relative path
        // would then name a different file or none.
        return PathBuf::from(&explicit)
            .canonicalize()
            .map_err(|e| format!("{explicit}: {e}"));
    }
    let (found, beside) = find_manifest(component);
    match found {
        Some(path) => {
            if path != beside {
                println!("  manifest: {}", path.display());
            }
            Ok(path)
        }
        None => Err(format!(
            "no manifest for {}: looked for {} beside it, and for the widget.toml of \
             the crate it was built in.\n  `--manifest <path>` names one.",
            component.display(),
            beside.display()
        )),
    }
}

/// The search behind [`manifest_for`], without the printing. Answers what was
/// found, and the path beside the component as it was named.
pub fn find_manifest(component: &Path) -> (Option<PathBuf>, PathBuf) {
    // Absolute for the searches upward: a bare `my_widget.wasm` run from
    // `target/…/release/` has no ancestors to search otherwise.
    let resolved = std::path::absolute(component).unwrap_or_else(|_| component.to_path_buf());
    let real = component
        .canonicalize()
        .unwrap_or_else(|_| resolved.clone());
    let beside = resolved.with_extension("toml");

    // The directory as the system resolves it, the file name as given: `..`
    // means what it means on disk (`target/../published/x.wasm` is not in
    // `target/`), while a symlinked *file* is still read as named.
    let located = match (
        resolved.parent().map(Path::canonicalize),
        resolved.file_name(),
    ) {
        (Some(Ok(dir)), Some(name)) => dir.join(name),
        _ => resolved.clone(),
    };

    // 1. Named by a path inside `target/`: the crate's manifest. Not for a
    //    symlink, wherever it sits: a link kept with a manifest of its own is
    //    read (and signed) with that one, so rule 2 goes first for it.
    let is_link = std::fs::symlink_metadata(&resolved).is_ok_and(|m| m.file_type().is_symlink());
    // For a link, whose build it is depends on where it points, not where it
    // sits: `A/target/linked.wasm` pointing at crate B's build is B's.
    let build = if is_link {
        crate_of(&real)
    } else {
        crate_of(&located)
    };
    if !is_link && let Some(Some(manifest)) = &build {
        return (Some(manifest.clone()), beside);
    }

    // 2. Beside it, as named.
    if beside.is_file() {
        return (Some(beside.clone()), beside);
    }

    // A build — for a link, the build it points at — takes its crate's
    // manifest; one no single crate owns has no answer to guess at.
    if let Some(manifest) = build {
        return (manifest, beside);
    }

    // 3. The crate it sits in, without leaving that crate.
    for dir in real.ancestors().skip(1) {
        let candidate = dir.join("widget.toml");
        if candidate.is_file() {
            return (Some(candidate), beside);
        }
        if dir.join("Cargo.toml").is_file() {
            break;
        }
    }
    (None, beside)
}

/// The manifest of the crate whose build output `component` is, if it is one.
///
/// The nearest `target/` directory with a `Cargo.toml` beside it — unless a
/// `Cargo.toml` comes first, which makes the component part of *that* crate
/// (staged inside somebody else's build directory, say) and not a build.
/// In a workspace, whose members share one `target/`, the member that built
/// the component is found by its artifact name, searching the whole
/// workspace, hidden directories included; if no single crate owns it, or
/// part of the workspace cannot be read, there is no answer, and
/// `--manifest` has to say.
///
/// `None`: not a build. `Some(None)`: a build with no manifest to be sure of.
fn crate_of(component: &Path) -> Option<Option<PathBuf>> {
    let stem = component.file_stem()?.to_str()?;
    for dir in component.ancestors().skip(1) {
        if dir.file_name().is_some_and(|name| name == "target")
            && let Some(krate) = dir.parent()
            && krate.join("Cargo.toml").is_file()
        {
            return Some(manifest_of_crate(krate, stem));
        }
        if dir.join("Cargo.toml").is_file() {
            return None;
        }
    }
    None
}

fn manifest_of_crate(krate: &Path, artifact: &str) -> Option<PathBuf> {
    // A root that cannot be read or parsed might be a workspace, and then its
    // manifest might belong to some other member: no answer, rather than one.
    let root: toml::Table = std::fs::read_to_string(krate.join("Cargo.toml"))
        .ok()?
        .parse()
        .ok()?;
    let is_workspace = root.contains_key("workspace");
    if !is_workspace {
        let manifest = krate.join("widget.toml");
        return manifest.is_file().then_some(manifest);
    }
    // What cargo calls the artifact: `[lib] name`, else the package name with
    // `-` as `_`.
    let builds = |table: &toml::Table| {
        let lib = table
            .get("lib")
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str());
        let package = table
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str());
        lib.map(str::to_string)
            .or_else(|| package.map(|p| p.replace('-', "_")))
            .is_some_and(|name| name == artifact)
    };
    // The whole workspace, however deep its members sit: a member missed
    // would leave some other crate looking like the only owner. A workspace
    // too large to search has no owner this can be sure of.
    const MOST_DIRECTORIES: usize = 10_000;
    let mut owners = Vec::new();
    let mut pending = vec![krate.to_path_buf()];
    let mut visited = 0;
    while let Some(dir) = pending.pop() {
        visited += 1;
        if visited > MOST_DIRECTORIES {
            return None;
        }
        // No `Cargo.toml` is no crate. One that is there and cannot be read
        // or parsed might be the other owner, so it ends the search unsure.
        match std::fs::read_to_string(dir.join("Cargo.toml")) {
            Ok(text) => {
                let table: toml::Table = text.parse().ok()?;
                // Every crate that could have built it counts, widget or not:
                // one without a `widget.toml` is still a crate the build may
                // belong to, and then no manifest found elsewhere is its.
                if builds(&table) {
                    owners.push(dir.join("widget.toml"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        // A directory that cannot be read is a part of the workspace not
        // searched, and an owner found elsewhere might not be the only one.
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return None;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return None;
            };
            // Build output and git's own store hold no members. Every other
            // directory is searched, hidden ones too: a workspace may list a
            // member at `.widgets/clock`.
            let name = entry.file_name();
            let Ok(kind) = entry.file_type() else {
                return None;
            };
            // A crate's build output — `target/` beside a `Cargo.toml` — and
            // git's own store hold no members. A member that happens to be
            // called `target` is still searched.
            let build_output = name == "target" && dir.join("Cargo.toml").is_file();
            if kind.is_dir() && !build_output && name != ".git" {
                pending.push(entry.path());
            }
        }
    }
    match owners.as_slice() {
        [only] if only.is_file() => Some(only.clone()),
        _ => None,
    }
}

/// Installs a widget that is already on disk, and reports what was recorded.
///
/// The component is recorded by the path given, so a widget installed from a
/// download directory that is later cleared stops working. `uniwidged add`
/// copies into the widget store first for exactly that reason.
pub fn install(
    config_path: &Path,
    component: &Path,
    manifest_path: &Path,
    allow_unsigned: bool,
    grant: Grant,
) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&text).map_err(|e| e.to_string())?;

    // Provenance before consent. The list below is only worth reading if it is
    // the list the author wrote — a signature covering the component alone
    // would let anybody widen it, which is why `signature::digest` covers both.
    let signed_by = match trust::provenance(component, manifest_path)? {
        trust::Provenance::Trusted(who) => Some(who),
        trust::Provenance::Refused(why) => {
            // Never installable, `--allow-unsigned` or not: a signature that
            // does not verify is a positive claim that something is wrong,
            // where a missing one is only an absence.
            return Err(format!(
                "{} has a signature and it does not check out: {why}",
                component.display()
            ));
        }
        trust::Provenance::Unsigned if allow_unsigned => None,
        trust::Provenance::Unsigned => {
            return Err(format!(
                "{} is not signed.\n  \
                 If you built it yourself, `uniwidged install --allow-unsigned` \
                 says so out loud.\n  \
                 If somebody sent it to you, ask them for the signature and \
                 their key, then `uniwidged trust <key>`.",
                component.display()
            ));
        }
    };

    let declared: Vec<String> = manifest.capabilities.iter().map(capability_text).collect();
    let granted = match &grant {
        Grant::Everything => declared.clone(),
        Grant::AsBefore(before) => {
            // The check that makes an unattended upgrade safe. Anything the
            // new manifest asks for and the old grant does not cover is a
            // permission nobody agreed to.
            let new: Vec<&String> = declared.iter().filter(|c| !before.contains(c)).collect();
            if !new.is_empty() {
                return Err(format!(
                    "this version of `{}` asks for something the installed one \
                     did not: {}.\n  That is a decision rather than an update, so \
                     `uniwidged add {}` will show you the whole list and ask.",
                    manifest.name,
                    new.iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    manifest.name
                ));
            }
            // Intersected, not carried over whole: a capability the old
            // version had and the new one dropped must not stay granted, or
            // the configuration would grant something no manifest declares.
            before
                .iter()
                .filter(|c| declared.contains(c))
                .cloned()
                .collect()
        }
    };

    if matches!(grant, Grant::Everything) {
        describe(&manifest, signed_by.as_deref());
    }

    let path = config_path.to_path_buf();
    let mut config = Config::read(&path).map_err(|e| e.to_string())?;
    let name = manifest.name.clone();

    // Where it sat, what it was set to, and whether it was switched off. See
    // the module note: a new version of a widget is not a new widget.
    let previous = config.widget(&name).cloned();
    config.widgets.retain(|w| w.name != name);
    let mut entry = WidgetConfig::granting_all(&name, component.to_path_buf(), &manifest);
    entry.granted = granted;
    // Recorded only when it is not where the configuration would look anyway,
    // so the common case stays one line shorter in a file people read.
    if manifest_path != component.with_extension("toml") {
        entry.manifest = Some(manifest_path.to_path_buf());
    }
    if let Some(previous) = previous {
        entry.placement = previous.placement;
        entry.settings = previous.settings;
        entry.enabled = previous.enabled;
    }
    config.widgets.push(entry);
    config.write(&path).map_err(|e| e.to_string())?;
    crate::nudge_desktop();
    println!("  installed; recorded in {}", path.display());
    if matches!(grant, Grant::Everything) {
        println!("  to decline one of those, remove it from `granted` there.\n");
    }
    Ok(manifest)
}

/// The consent screen: everything the runtime will enforce, and nothing that
/// is not on the manifest.
pub fn describe(manifest: &Manifest, signed_by: Option<&str>) {
    println!("\n  {} {}", manifest.name, manifest.version);
    match signed_by {
        Some(who) => println!("  signed by {who}"),
        None => println!(
            "  NOT SIGNED — you are trusting this because you chose to, and \
             nothing here checked anything"
        ),
    }
    if !manifest.description.is_empty() {
        println!("  {}", manifest.description);
    }
    println!("\n  This widget wants to:");
    for line in manifest.consent_summary() {
        println!("    · {line}");
    }
    println!(
        "  It may use up to {} MB of memory.\n",
        manifest.limits.memory_bytes / (1024 * 1024)
    );
    // "store passwords in your keychain" is a sentence whose weight depends
    // entirely on what the keychain is, and a user cannot be asked to consent
    // to something they have not been told. So the one capability whose
    // backing store is a judgement call names it here, at the moment of the
    // decision, rather than only in `doctor` where nobody looks first.
    if manifest
        .capabilities
        .iter()
        .any(|c| matches!(c, uniwidged_runtime::Capability::Secrets))
    {
        use uniwidged_runtime::SecretStore;
        println!(
            "  Its keychain here is {}.\n",
            uniwidged_runtime::FileKeyring::system(crate::paths::keyring_dir()).describe()
        );
    }

    if !manifest.settings.is_empty() {
        // Said at installation because it is the moment somebody is looking:
        // a widget whose knobs are only discoverable by running another
        // command is a widget whose knobs nobody finds.
        println!(
            "  It can be configured: {}",
            manifest
                .settings
                .iter()
                .map(|setting| setting.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!("  `uniwidged configure {}`\n", manifest.name);
    }
}

#[cfg(test)]
mod tests {
    /// Which *file* was found, compared as the system resolves it: a temporary
    /// directory is `/private/var/…` behind `/var/…` on macOS and has an 8.3
    /// short name on Windows, and `canonicalize` there adds a `\\?\` prefix. The
    /// question these tests ask is which manifest, not how its path is spelled.
    macro_rules! assert_same {
        ($found:expr, $expected:expr $(,)?) => {{
            let canon = |p: Option<std::path::PathBuf>| p.map(|p| p.canonicalize().unwrap_or(p));
            assert_eq!(canon($found), canon($expected));
        }};
    }

    use super::find_manifest;

    #[test]
    fn only_a_build_of_the_crate_takes_the_crates_manifest() {
        let root = std::env::temp_dir().join(format!("uniwidged-builds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: std::path::PathBuf, text: &str| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };

        // `..` out of `target/` is out of it: the published copy's own wins.
        let krate = root.join("plain");
        write(krate.join("Cargo.toml"), "");
        write(krate.join("widget.toml"), "");
        write(krate.join("target/release/x.wasm"), "");
        write(krate.join("published/x.wasm"), "");
        write(krate.join("published/x.toml"), "");
        let escaped = krate.join("target/../published/x.wasm");
        assert_same!(
            find_manifest(&escaped).0,
            Some(escaped.with_extension("toml"))
        );

        // A crate staged inside another's `target/` is that crate's, not a build.
        let staged = krate.join("target/package/staged");
        write(staged.join("Cargo.toml"), "");
        write(staged.join("y.wasm"), "");
        write(staged.join("y.toml"), "");
        assert_same!(
            find_manifest(&staged.join("y.wasm")).0,
            Some(staged.join("y.toml"))
        );

        // A workspace: the member that built the artifact, by its name — not
        // the root's manifest, though the root is a widget too.
        let space = root.join("space");
        write(
            space.join("Cargo.toml"),
            "[workspace]\nmembers = [\"members/*\", \".widgets/clock\", \"a/b/c/d/deep\", \"widgets/target\"]\n[package]\nname = \"root-widget\"\n",
        );
        write(space.join("widget.toml"), "");
        write(
            space.join("members/clock/Cargo.toml"),
            "[package]\nname = \"my-clock\"\n",
        );
        write(space.join("members/clock/widget.toml"), "");
        write(
            space.join("members/odd/Cargo.toml"),
            "[package]\nname = \"odd\"\n[lib]\nname = \"renamed\"\n",
        );
        write(space.join("members/odd/widget.toml"), "");
        let release = space.join("target/wasm32-wasip2/release");
        for artifact in ["my_clock", "renamed", "root_widget", "nobody"] {
            write(release.join(format!("{artifact}.wasm")), "");
        }
        let found = |artifact: &str| find_manifest(&release.join(format!("{artifact}.wasm"))).0;
        assert_same!(
            found("my_clock"),
            Some(space.join("members/clock/widget.toml"))
        );
        assert_same!(
            found("renamed"),
            Some(space.join("members/odd/widget.toml"))
        );
        assert_same!(found("root_widget"), Some(space.join("widget.toml")));
        // Built by nobody it can name: no guess, so `--manifest` has to say.
        assert_same!(found("nobody"), None);

        // Two crates claiming one artifact name — the root, and a member far
        // below it — is no single owner, however deep the member sits.
        write(
            space.join("a/b/c/d/deep/Cargo.toml"),
            "[package]\nname = \"deep\"\n[lib]\nname = \"root_widget\"\n",
        );
        write(space.join("a/b/c/d/deep/widget.toml"), "");
        assert_same!(found("root_widget"), None);

        // A member in a hidden directory is as much a member: it and the root
        // both claim `renamed` here, so neither is the owner.
        write(
            space.join(".widgets/clock/Cargo.toml"),
            "[package]\nname = \"hidden\"\n[lib]\nname = \"renamed\"\n",
        );
        write(space.join(".widgets/clock/widget.toml"), "");
        assert_same!(found("renamed"), None);

        // A `Cargo.toml` that is there and cannot be read could be another
        // owner, so the search ends unsure rather than with the one it saw.
        // (A directory by that name: unreadable even to root.)
        std::fs::create_dir_all(space.join("members/unreadable/Cargo.toml")).unwrap();
        assert_same!(found("my_clock"), None);
        std::fs::remove_dir_all(space.join("members/unreadable")).unwrap();
        assert_same!(
            found("my_clock"),
            Some(space.join("members/clock/widget.toml"))
        );

        // A crate that builds the same artifact without being a widget is
        // still a crate the build may be: no manifest is borrowed for it.
        write(
            space.join("members/plain/Cargo.toml"),
            "[package]\nname = \"plain\"\n[lib]\nname = \"my_clock\"\n",
        );
        assert_same!(found("my_clock"), None);
        std::fs::remove_dir_all(space.join("members/plain")).unwrap();

        // A member called `target` is a member, not build output: its claim
        // counts.
        write(
            space.join("widgets/target/Cargo.toml"),
            "[package]\nname = \"t\"\n[lib]\nname = \"my_clock\"\n",
        );
        write(space.join("widgets/target/widget.toml"), "");
        assert_same!(found("my_clock"), None);

        // A root `Cargo.toml` that cannot be parsed may be a workspace whose
        // manifest belongs to another member: no answer, not the root's.
        let broken = root.join("broken");
        write(broken.join("Cargo.toml"), "[workspace\nmembers = [");
        write(broken.join("widget.toml"), "");
        write(broken.join("target/release/z.wasm"), "");
        assert_same!(find_manifest(&broken.join("target/release/z.wasm")).0, None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cargo_build_finds_the_crates_own_manifest_and_stops_at_its_root() {
        let root = std::env::temp_dir().join(format!("uniwidged-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let krate = root.join("project/hello");
        let release = krate.join("target/wasm32-wasip2/release");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::write(krate.join("Cargo.toml"), "").unwrap();
        let component = release.join("hello.wasm");
        std::fs::write(&component, b"").unwrap();

        // Nothing yet: and an unrelated manifest *above* the crate root is
        // not somebody else's widget to borrow.
        std::fs::write(root.join("project/widget.toml"), "").unwrap();
        assert_same!(find_manifest(&component).0, None);

        std::fs::write(krate.join("widget.toml"), "").unwrap();
        assert_same!(find_manifest(&component).0, Some(krate.join("widget.toml")));

        // A copy left beside the build by an earlier `sign` does not shadow
        // the crate's manifest, which the author may have edited since.
        std::fs::write(release.join("hello.toml"), "").unwrap();
        assert_same!(find_manifest(&component).0, Some(krate.join("widget.toml")));

        // Outside a build directory — a published set — the one beside wins.
        let published = root.join("published");
        std::fs::create_dir_all(&published).unwrap();
        std::fs::write(published.join("hello.wasm"), b"").unwrap();
        std::fs::write(published.join("hello.toml"), "").unwrap();
        assert_same!(
            find_manifest(&published.join("hello.wasm")).0,
            Some(published.join("hello.toml"))
        );

        // A link is read with the manifest beside the link, not the target's.
        #[cfg(unix)]
        {
            let linked = root.join("linked");
            std::fs::create_dir_all(&linked).unwrap();
            std::os::unix::fs::symlink(published.join("hello.wasm"), linked.join("mine.wasm"))
                .unwrap();
            std::fs::write(linked.join("mine.toml"), "").unwrap();
            assert_same!(
                find_manifest(&linked.join("mine.wasm")).0,
                Some(linked.join("mine.toml"))
            );
        }

        // A link into a crate's `target/`: the manifest beside the link wins
        // when there is one, and the crate's is the fallback when there is not.
        #[cfg(unix)]
        {
            let into_build = root.join("into-build");
            std::fs::create_dir_all(&into_build).unwrap();
            std::os::unix::fs::symlink(&component, into_build.join("dev.wasm")).unwrap();
            assert_same!(
                find_manifest(&into_build.join("dev.wasm")).0,
                Some(krate.join("widget.toml"))
            );
            std::fs::write(into_build.join("dev.toml"), "").unwrap();
            assert_same!(
                find_manifest(&into_build.join("dev.wasm")).0,
                Some(into_build.join("dev.toml"))
            );
        }

        // A link in one crate's `target/` pointing at another crate's build
        // is the other crate's, when nothing sits beside it.
        #[cfg(unix)]
        {
            let other = root.join("project/other");
            std::fs::create_dir_all(other.join("target/release")).unwrap();
            std::fs::write(other.join("Cargo.toml"), "").unwrap();
            std::fs::write(other.join("widget.toml"), "").unwrap();
            std::fs::write(other.join("target/release/b.wasm"), b"").unwrap();
            let foreign = release.join("foreign.wasm");
            std::os::unix::fs::symlink(other.join("target/release/b.wasm"), &foreign).unwrap();
            assert_same!(find_manifest(&foreign).0, Some(other.join("widget.toml")));
        }

        // The same for a link that itself sits inside `target/`.
        #[cfg(unix)]
        {
            let inside = release.join("linked.wasm");
            std::os::unix::fs::symlink(&component, &inside).unwrap();
            assert_same!(find_manifest(&inside).0, Some(krate.join("widget.toml")));
            std::fs::write(release.join("linked.toml"), "").unwrap();
            assert_same!(find_manifest(&inside).0, Some(release.join("linked.toml")));
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
