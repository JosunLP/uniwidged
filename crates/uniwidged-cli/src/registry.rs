//! `uniwidged registry`, `search`, `add` and `update` — somewhere to find
//! widgets.
//!
//! The format, the two signatures and what each of them buys are argued in
//! [`uniwidged_runtime::registry`]. This is the part a person touches: the
//! list of registries this machine reads, the cache, the widget store, and the
//! four commands.
//!
//! ```text
//! uniwidged registry add https://widgets.example/index.toml --key <hex>
//! uniwidged search clock
//! uniwidged add clock
//! uniwidged update
//! ```
//!
//! ## Three files, all readable
//!
//! * `registries.toml` — what this machine reads, and whose key each one is
//!   pinned to. Editable by hand, like `trusted-keys` beside it.
//! * `registry-cache/<name>.toml` and its `.sig` — the last index fetched,
//!   byte for byte as it was served, so the signature beside it still covers
//!   it. **Verified again every time it is read**, because a cache that is
//!   trusted because it is local is a cache anybody with write access owns.
//! * `store/<name>-<version>.wasm` and friends — the widgets themselves. A
//!   download is copied here before it is installed, so clearing a temporary
//!   directory cannot leave a configuration pointing at nothing.
//!
//! ## What is not here
//!
//! No accounts, no publishing API, no telemetry, and nothing that phones home
//! on its own. Every fetch in this file is one somebody typed a command to
//! start — a desktop that quietly polls a registry is a desktop that tells
//! somebody else when it is switched on.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use uniwidged_host::config::Config;
use uniwidged_runtime::manifest::Manifest;
use uniwidged_runtime::registry::{Entry, FetchError, Fetcher, Index};
use uniwidged_runtime::signature::{PublicKey, SecretKey, SignatureFile};
use uniwidged_runtime::version::Version;

use crate::install::{self, Grant};

/// A certificate authority to trust *instead of* the default roots.
///
/// For a registry inside an organisation, under a certificate no public trust
/// store has heard of. It names a PEM file, it applies to registry fetches
/// only, and it replaces the default roots rather than adding to them — see
/// [`Fetcher::trusting`], where that choice is argued.
const EXTRA_CA: &str = "UNIWIDGED_REGISTRY_CA";

/// The client every fetch in this file goes through.
fn fetcher() -> Result<Fetcher, String> {
    match std::env::var_os(EXTRA_CA) {
        None => Ok(Fetcher::new()),
        Some(path) => {
            let pem = std::fs::read(&path)
                .map_err(|e| format!("{EXTRA_CA}: {}: {e}", Path::new(&path).display()))?;
            Fetcher::trusting(&pem).map_err(|e| format!("{EXTRA_CA}: {e}"))
        }
    }
}

/// How old a cached index may be before a command refreshes it.
///
/// Long enough that `search` twice in a minute does not fetch twice, short
/// enough that "there is an update" is a fact about today. `--refresh` and
/// `--offline` are the two overrides, in either direction.
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// One registry this machine reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    /// What this machine calls it — a directory-safe name, because it becomes
    /// the cache file's name.
    pub name: String,
    pub url: String,
    /// The key its index is signed with, pinned when it was added.
    ///
    /// Pinned rather than looked up in `trusted-keys`: publishing a list and
    /// writing a widget are different claims, and a key that can do one must
    /// not thereby be able to do the other. See
    /// [`uniwidged_runtime::registry`].
    pub key: PublicKey,
}

/// `uniwidged registry <add|remove|list|publish>`
pub fn command(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("add") => add_registry(&args[1..]),
        Some("remove") => remove_registry(&args[1..]),
        Some("list") | None => list_registries(args.iter().any(|a| a == "--names")),
        Some("publish") => publish(&args[1..]),
        Some(other) => Err(format!(
            "`{other}` is not something `uniwidged registry` does. \
             Try `add`, `remove`, `list` or `publish`."
        )),
    }
}

fn add_registry(args: &[String]) -> Result<(), String> {
    let url = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which registry? `uniwidged registry add <url> --key <public key>`")?;
    let key = crate::flag(args, "--key").ok_or(
        "a registry has to be pinned to the key its index is signed with: \
         `uniwidged registry add <url> --key <public key>`",
    )?;
    let key = PublicKey::from_hex(&key).map_err(|e| e.to_string())?;
    let name = match crate::flag(args, "--name") {
        Some(name) => name,
        // The host is the obvious name and the one a person would have
        // chosen. Punctuation becomes hyphens because the name is a file.
        None => uniwidged_runtime::net::host_of(url)
            .ok_or_else(|| format!("`{url}` is not an https:// URL"))?
            .replace('.', "-"),
    };
    if !is_usable_registry_name(&name) {
        return Err(format!(
            "`{name}` is not a usable registry name: it becomes a file, so \
             lowercase letters, digits and hyphens only"
        ));
    }

    let mut registries = read_registries()?;
    registries.retain(|r| r.name != name);
    registries.push(Registry {
        name: name.clone(),
        url: url.clone(),
        key,
    });
    write_registries(&registries)?;

    // Fetched now rather than at the next command, so that a wrong key or a
    // mistyped URL is somebody's mistake to fix while they are still looking
    // at it.
    println!("  reading {url}");
    let registry = registries.last().expect("just pushed");
    let index = refresh(registry).map_err(|why| why.to_string())?;
    println!(
        "  added `{name}`, pinned to {}…\n  {} widget{} offered",
        registry.key.short(),
        index.entries.len(),
        if index.entries.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn remove_registry(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .ok_or("which registry? `uniwidged registry remove <name>`")?;
    let mut registries = read_registries()?;
    let before = registries.len();
    registries.retain(|r| &r.name != name && &r.url != name);
    if registries.len() == before {
        return Err(format!("`{name}` is not a registry this machine reads"));
    }
    write_registries(&registries)?;
    let _ = std::fs::remove_file(cache_file(name));
    let _ = std::fs::remove_file(cache_signature(name));
    println!("  no longer reading `{name}`; widgets installed from it stay installed");
    Ok(())
}

fn list_registries(names_only: bool) -> Result<(), String> {
    let registries = read_registries()?;
    if names_only {
        // One name per line, for a shell completion or a script. Nothing to
        // read is nothing printed — a sentence here would complete to a
        // sentence.
        for registry in &registries {
            println!("{}", registry.name);
        }
        return Ok(());
    }
    if registries.is_empty() {
        println!(
            "  this machine reads no registries.\n  \
             `uniwidged registry add <url> --key <public key>` adds one."
        );
        return Ok(());
    }
    for registry in &registries {
        println!("  {}  {}", registry.name, registry.url);
        println!("    pinned to {}", registry.key.to_hex());
        match cached_at(&registry.name) {
            Some(age) => println!("    cached, {} old", roughly(age)),
            None => println!("    not fetched yet"),
        }
    }
    Ok(())
}

/// `uniwidged search [term] [--refresh] [--offline]`
pub fn search(args: &[String]) -> Result<(), String> {
    let term = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_default();
    let registries = read_registries()?;
    if registries.is_empty() {
        return Err(
            "this machine reads no registries, so there is nothing to search. \
             `uniwidged registry add <url> --key <public key>` adds one."
                .into(),
        );
    }

    let mut found = 0;
    for registry in &registries {
        let index = load(registry, args)?;
        let matches = index.search(&term);
        if matches.is_empty() {
            continue;
        }
        println!("\n  {} ({})", registry.name, registry.url);
        for entry in matches {
            found += 1;
            println!("    {}", entry.title());
            if !entry.description.is_empty() {
                println!("      {}", entry.description);
            }
            // The permissions, in the words the consent screen will use.
            // Somebody deciding what to install wants them here, not after.
            for line in entry.consent_summary() {
                println!("      · {line}");
            }
        }
    }
    if found == 0 {
        println!(
            "  nothing matching `{term}`. `uniwidged search` with no term \
             lists everything."
        );
    } else {
        println!("\n  `uniwidged add <name>` installs one.\n");
    }
    Ok(())
}

/// `uniwidged add <name>[@version] [--registry N] [--allow-unsigned]`
pub fn add(args: &[String]) -> Result<(), String> {
    let wanted = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged add <name>` — `uniwidged search` lists them")?;
    let (name, version) = split_version(wanted)?;

    let (registry, entry) = resolve(&name, version.as_ref(), args)?;
    println!("  {} from {}", entry.title(), registry.name);
    let download = fetcher()?
        .widget(&entry)
        .map_err(|why| format!("{}: {why}", entry.title()))?;
    let (component, manifest_path) = store(&entry, &download)?;

    install::install(
        &component,
        &manifest_path,
        args.iter().any(|a| a == "--allow-unsigned"),
        Grant::Everything,
    )?;
    Ok(())
}

/// `uniwidged update [name] [--check] [--allow-unsigned]`
///
/// Reports what is newer and applies it, unless `--check` says only to look.
/// An upgrade that would widen a permission is reported and **not** applied —
/// see [`Grant::AsBefore`].
pub fn update(args: &[String]) -> Result<(), String> {
    let only = args.first().filter(|a| !a.starts_with('-')).cloned();
    let check_only = args.iter().any(|a| a == "--check");

    let config = Config::read(&crate::paths::config_file()).map_err(|e| e.to_string())?;
    let installed: Vec<&uniwidged_host::config::WidgetConfig> = config
        .widgets
        .iter()
        .filter(|w| only.as_ref().is_none_or(|name| &w.name == name))
        .collect();
    if installed.is_empty() {
        return Err(match only {
            Some(name) => format!("`{name}` is not installed; try `uniwidged list`"),
            None => "nothing is installed, so there is nothing to update".into(),
        });
    }

    let registries = read_registries()?;
    if registries.is_empty() {
        // Said rather than answered with "everything is up to date", which
        // would be true and would mean the opposite of what somebody reads
        // into it.
        return Err(
            "this machine reads no registries, so nothing can say whether \
             anything is newer. `uniwidged registry add <url> --key <public key>` \
             adds one."
                .into(),
        );
    }
    let mut indexes = Vec::new();
    for registry in &registries {
        indexes.push((registry.clone(), load(registry, args)?));
    }

    let mut newer = 0;
    for widget in installed {
        let manifest = read_manifest(widget)?;
        let here = manifest.semver();
        let Some((registry, entry)) = newest(&indexes, &widget.name) else {
            continue;
        };
        // A widget whose manifest has no version cannot be compared, and
        // guessing "newer" would be an unattended replacement of somebody's
        // widget on no evidence.
        let Some(here) = here else {
            println!(
                "  {} has no version in its manifest, so nothing can say whether \
                 {} is newer",
                widget.name,
                entry.title()
            );
            continue;
        };
        if entry.version <= here {
            continue;
        }
        newer += 1;
        println!(
            "\n  {} {here} → {} ({})",
            widget.name, entry.version, registry.name
        );
        if check_only {
            continue;
        }

        let download = fetcher()?
            .widget(entry)
            .map_err(|why| format!("{}: {why}", entry.title()))?;
        let (component, manifest_path) = store(entry, &download)?;
        // The granted list, not the declared one: an update may not quietly
        // widen what a user agreed to.
        match install::install(
            &component,
            &manifest_path,
            args.iter().any(|a| a == "--allow-unsigned"),
            Grant::AsBefore(widget.granted.clone()),
        ) {
            Ok(_) => {}
            Err(why) => println!("  not applied: {why}"),
        }
    }

    if newer == 0 {
        println!("  everything installed is the newest version offered");
    } else if check_only {
        println!("\n  `uniwidged update` applies these.\n");
    }
    Ok(())
}

/// `uniwidged registry publish --key F --base-url U [--out F] <widget.wasm>…`
///
/// Writes a signed index for widgets that are already signed and already on
/// disk. It is here rather than in a script because the index format has one
/// definition and one writer — see [`uniwidged_runtime::registry::write_index`].
///
/// It reads each widget the way `install` will: the manifest is parsed, the
/// digest is the one a signature covers, and a widget whose own signature does
/// not verify against its own key is refused rather than listed. A registry
/// that publishes a broken widget has published a download that every client
/// will reject.
fn publish(args: &[String]) -> Result<(), String> {
    let key_path = crate::flag(args, "--key")
        .ok_or("which key signs the index? `uniwidged registry publish --key <file>`")?;
    let base = crate::flag(args, "--base-url").ok_or(
        "where will these files be served from? \
         `uniwidged registry publish --base-url https://widgets.example/`",
    )?;
    let base = base.trim_end_matches('/').to_string();
    if uniwidged_runtime::net::host_of(&base).is_none() {
        return Err(format!("`{base}` is not an https:// URL"));
    }
    let out = crate::flag(args, "--out").unwrap_or_else(|| "index.toml".to_string());
    let key_text = std::fs::read_to_string(&key_path).map_err(|e| format!("{key_path}: {e}"))?;
    let key = SecretKey::from_hex(key_text.trim()).map_err(|e| format!("{key_path}: {e}"))?;

    let components: Vec<&String> = args
        .iter()
        .filter(|a| a.ends_with(".wasm"))
        .collect::<Vec<_>>();
    if components.is_empty() {
        return Err("which widgets? `uniwidged registry publish … <widget.wasm>…`".into());
    }

    let mut entries = Vec::new();
    for component in components {
        let component = PathBuf::from(component);
        let manifest_path = component.with_extension("toml");
        let (bytes, manifest_text) = crate::trust::read_widget(&component, &manifest_path)?;
        let manifest = Manifest::parse(&manifest_text).map_err(|e| e.to_string())?;
        let version = manifest.semver().ok_or_else(|| {
            format!(
                "{} has no version, and a registry entry is a name *and* a version",
                manifest_path.display()
            )
        })?;
        let file = file_stem(&manifest.name, &version);
        // The widget's own signature, checked for being *intact* rather than
        // for being trusted: a publisher listing a `.sig` that does not cover
        // these bytes has published a download every client will reject.
        let signature_path = crate::trust::signature_path(&component);
        let signature = match std::fs::read_to_string(&signature_path) {
            Ok(text) => {
                let signed = SignatureFile::parse(&text)
                    .map_err(|e| format!("{}: {e}", signature_path.display()))?;
                uniwidged_runtime::signature::verify_intact(&bytes, &manifest_text, &signed)
                    .map_err(|e| format!("{}: {e}", signature_path.display()))?;
                Some(format!("{base}/{file}.wasm.sig"))
            }
            // No signature at all is a choice a publisher may make; whoever
            // installs it will have to say `--allow-unsigned` out loud.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{}: {e}", signature_path.display())),
        };
        entries.push(Entry {
            name: manifest.name.clone(),
            version,
            description: manifest.description.clone(),
            capabilities: manifest
                .capabilities
                .iter()
                .map(uniwidged_host::config::capability_text)
                .collect(),
            component: format!("{base}/{file}.wasm"),
            manifest: format!("{base}/{file}.toml"),
            signature,
            digest: uniwidged_runtime::signature::digest(&bytes, &manifest_text),
        });
        println!("  listing {} {}", manifest.name, manifest.version);
    }

    let text = uniwidged_runtime::registry::write_index(&entries);
    // Written through the parser rather than only through the writer: an index
    // this cannot read is one no client could either.
    Index::parse(&text).map_err(|e| format!("the index this wrote does not parse: {e}"))?;
    let signature = SignatureFile {
        signature: key.sign_index(&text),
        key: key.public(),
    };
    std::fs::write(&out, &text).map_err(|e| format!("{out}: {e}"))?;
    std::fs::write(format!("{out}.sig"), signature.to_text())
        .map_err(|e| format!("{out}.sig: {e}"))?;
    println!("\n  wrote {out} and {out}.sig");
    println!("  signed with {}", key.public().to_hex());
    println!(
        "  serve both from {base}/, beside the widgets.\n  \
         Whoever reads it runs `uniwidged registry add {base}/{out} --key {}`\n",
        key.public().to_hex()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The list of registries
// ---------------------------------------------------------------------------

pub fn read_registries() -> Result<Vec<Registry>, String> {
    let path = crate::paths::registries_file();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // Reading none yet is the correct starting state, not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    parse_registries(&text).map_err(|why| format!("{}: {why}", path.display()))
}

fn parse_registries(text: &str) -> Result<Vec<Registry>, String> {
    #[derive(serde::Deserialize)]
    struct Raw {
        #[serde(default, rename = "registry")]
        registries: Vec<RawRegistry>,
    }
    #[derive(serde::Deserialize)]
    struct RawRegistry {
        name: String,
        url: String,
        key: String,
    }
    let raw: Raw = toml::from_str(text).map_err(|e| e.message().to_string())?;
    raw.registries
        .into_iter()
        .map(|registry| {
            if !is_usable_registry_name(&registry.name) {
                return Err(format!("`{}` is not a usable registry name", registry.name));
            }
            if uniwidged_runtime::net::host_of(&registry.url).is_none() {
                return Err(format!("`{}` is not an https:// URL", registry.url));
            }
            Ok(Registry {
                key: PublicKey::from_hex(&registry.key).map_err(|e| e.to_string())?,
                name: registry.name,
                url: registry.url,
            })
        })
        .collect()
}

fn write_registries(registries: &[Registry]) -> Result<(), String> {
    let path = crate::paths::registries_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut text = String::from(
        "# Registries this machine reads. Each is pinned to the key its index\n\
         # is signed with — that key says who publishes the list, and is not\n\
         # the same thing as trusting whoever wrote the widgets on it.\n\
         # Added with `uniwidged registry add`.\n",
    );
    for registry in registries {
        text.push_str(&format!(
            "\n[[registry]]\nname = \"{}\"\nurl = \"{}\"\nkey = \"{}\"\n",
            registry.name,
            registry.url,
            registry.key.to_hex()
        ));
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A registry name becomes a file name, so it is held to the same rule a
/// widget name is.
fn is_usable_registry_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ---------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------

fn cache_file(name: &str) -> PathBuf {
    crate::paths::registry_cache_dir().join(format!("{name}.toml"))
}

fn cache_signature(name: &str) -> PathBuf {
    crate::paths::registry_cache_dir().join(format!("{name}.toml.sig"))
}

fn cached_at(name: &str) -> Option<Duration> {
    std::fs::metadata(cache_file(name))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|when| SystemTime::now().duration_since(when).ok())
}

/// The index for one registry, fetching it if the cache is missing or old.
///
/// `--refresh` fetches whatever the cache says; `--offline` refuses to fetch
/// at all, which is what a machine with no network wants rather than a twenty
/// second timeout per registry.
fn load(registry: &Registry, args: &[String]) -> Result<Index, String> {
    let refresh_asked = args.iter().any(|a| a == "--refresh");
    let offline = args.iter().any(|a| a == "--offline");
    let stale = cached_at(&registry.name).is_none_or(|age| age > CACHE_TTL);

    if !offline && (refresh_asked || stale) {
        match refresh(registry) {
            Ok(index) => return Ok(index),
            // A registry that is *down* is not a reason to stop working: say
            // so and use what is here, which is still signed. A registry
            // serving a list nobody signed is a different fact entirely, and
            // quietly falling back to the cache would reduce somebody actively
            // forging an index to a line of grey text — so that one stops
            // here. See `uniwidged_runtime::registry::FetchError`.
            Err(FetchError::Transport(why)) if cache_file(&registry.name).exists() => {
                eprintln!("  ({}: {why}; using the cached index)", registry.name);
            }
            Err(why) => return Err(format!("{}: {why}", registry.name)),
        }
    }
    read_cache(registry)
}

/// Reads the cache, and checks its signature again.
///
/// The check is not ceremony. The cache is a file in a directory, and anything
/// that can write there could otherwise add a widget to the list a person is
/// about to install from.
fn read_cache(registry: &Registry) -> Result<Index, String> {
    let path = cache_file(&registry.name);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e} — `uniwidged search --refresh` fetches it",
            path.display()
        )
    })?;
    let signature_path = cache_signature(&registry.name);
    let signature_text = std::fs::read_to_string(&signature_path)
        .map_err(|e| format!("{}: {e}", signature_path.display()))?;
    let signature = SignatureFile::parse(&signature_text)
        .map_err(|e| format!("{}: {e}", signature_path.display()))?;
    Index::parse_signed(&text, &signature, registry.key)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Fetches an index and writes it to the cache, verified.
fn refresh(registry: &Registry) -> Result<Index, FetchError> {
    let (index, text, signature) = fetcher()
        .map_err(FetchError::Transport)?
        .index(&registry.url, registry.key)?;
    let directory = crate::paths::registry_cache_dir();
    let io =
        |path: &Path, e: std::io::Error| FetchError::Transport(format!("{}: {e}", path.display()));
    std::fs::create_dir_all(&directory).map_err(|e| io(&directory, e))?;
    // The text exactly as served — a re-serialisation would not be what the
    // signature beside it covers.
    std::fs::write(cache_file(&registry.name), &text)
        .map_err(|e| io(&cache_file(&registry.name), e))?;
    std::fs::write(cache_signature(&registry.name), &signature)
        .map_err(|e| io(&cache_signature(&registry.name), e))?;
    Ok(index)
}

// ---------------------------------------------------------------------------
// Resolving and storing
// ---------------------------------------------------------------------------

/// `name` or `name@1.2.3`.
fn split_version(wanted: &str) -> Result<(String, Option<Version>), String> {
    match wanted.split_once('@') {
        None => Ok((wanted.to_string(), None)),
        Some((name, version)) => {
            let version = Version::parse(version).map_err(|e| e.to_string())?;
            Ok((name.to_string(), Some(version)))
        }
    }
}

/// Which registry offers this widget, and which entry.
///
/// A name offered by two registries is **not** resolved by picking one. The
/// two are different widgets that happen to share a name, and choosing
/// silently is choosing whose code runs on somebody's desktop.
fn resolve(
    name: &str,
    version: Option<&Version>,
    args: &[String],
) -> Result<(Registry, Entry), String> {
    let registries = read_registries()?;
    if registries.is_empty() {
        return Err("this machine reads no registries. \
             `uniwidged registry add <url> --key <public key>` adds one."
            .into());
    }
    if let Some(wanted) = crate::flag(args, "--registry") {
        let registry = registries
            .into_iter()
            .find(|r| r.name == wanted)
            .ok_or_else(|| format!("`{wanted}` is not a registry this machine reads"))?;
        let index = load(&registry, args)?;
        let entry = index
            .resolve(name, version)
            .ok_or_else(|| {
                format!(
                    "`{registry_name}` does not offer {name}",
                    registry_name = registry.name
                )
            })?
            .clone();
        return Ok((registry, entry));
    }

    let mut found: Vec<(Registry, Entry)> = Vec::new();
    for registry in registries {
        let index = load(&registry, args)?;
        if let Some(entry) = index.resolve(name, version) {
            found.push((registry, entry.clone()));
        }
    }
    match found.len() {
        0 => Err(format!(
            "no registry this machine reads offers `{name}`{}. \
             `uniwidged search {name}` looks again.",
            version.map(|v| format!(" at {v}")).unwrap_or_default()
        )),
        1 => Ok(found.remove(0)),
        _ => Err(format!(
            "`{name}` is offered by {} — they are different widgets that share \
             a name, so say which: `uniwidged add {name} --registry <name>`",
            found
                .iter()
                .map(|(registry, _)| registry.name.as_str())
                .collect::<Vec<_>>()
                .join(" and ")
        )),
    }
}

fn file_stem(name: &str, version: &Version) -> String {
    format!("{name}-{version}")
}

/// Writes a verified download into the widget store.
///
/// Into the store rather than a temporary directory, because the configuration
/// records where a component *is*: a widget installed out of `/tmp` stops
/// working the next time the machine is restarted, and the symptom is a
/// desktop with a hole in it.
fn store(
    entry: &Entry,
    download: &uniwidged_runtime::registry::Download,
) -> Result<(PathBuf, PathBuf), String> {
    let directory = crate::paths::widget_store_dir();
    std::fs::create_dir_all(&directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let stem = file_stem(&entry.name, &entry.version);
    let component = directory.join(format!("{stem}.wasm"));
    let manifest = directory.join(format!("{stem}.toml"));

    std::fs::write(&component, &download.component)
        .map_err(|e| format!("{}: {e}", component.display()))?;
    std::fs::write(&manifest, &download.manifest)
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    let signature_path = crate::trust::signature_path(&component);
    match &download.signature {
        Some(text) => std::fs::write(&signature_path, text)
            .map_err(|e| format!("{}: {e}", signature_path.display()))?,
        // A file left over from a previous version would be a signature over
        // different bytes, which `install` would report as tampering.
        None => {
            let _ = std::fs::remove_file(&signature_path);
        }
    }
    Ok((component, manifest))
}

fn read_manifest(widget: &uniwidged_host::config::WidgetConfig) -> Result<Manifest, String> {
    let path = widget.manifest_path();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Manifest::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The newest offer of one widget across every registry.
fn newest<'a>(indexes: &'a [(Registry, Index)], name: &str) -> Option<(&'a Registry, &'a Entry)> {
    indexes
        .iter()
        .filter_map(|(registry, index)| index.resolve(name, None).map(|entry| (registry, entry)))
        .max_by(|(_, a), (_, b)| a.version.cmp(&b.version))
}

/// A duration in the words somebody would use.
fn roughly(age: Duration) -> String {
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "less than a minute".into(),
        1..=90 => format!("{minutes} minutes"),
        _ => format!("{} hours", minutes / 60),
    }
}

#[cfg(test)]
mod tests;
