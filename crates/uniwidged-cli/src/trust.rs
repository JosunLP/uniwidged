//! Signing widgets, and deciding whose signatures count.
//!
//! Four commands and one file. `keygen` and `sign` are for whoever publishes a
//! widget; `trust` and `untrust` are for whoever installs one; and the file is
//! `trusted-keys` in the configuration directory, one key per line, meant to be
//! read and edited by the person it protects.
//!
//! There is no key server, no certificate chain and no revocation protocol.
//! Everything larger than a file of keys needs infrastructure this project does
//! not have and could not honestly run, and a security mechanism whose
//! operation nobody can inspect is one nobody should rely on.

use std::path::{Path, PathBuf};

use uniwidged_runtime::signature::{PublicKey, SecretKey, SignatureFile, TrustedKeys};

/// Reads the trusted-keys file, or an empty set if there is none yet.
pub fn trusted() -> Result<TrustedKeys, String> {
    let path = crate::paths::trusted_keys_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => TrustedKeys::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
        // Trusting nothing yet is the correct starting state, not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TrustedKeys::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn write_trusted(keys: &TrustedKeys) -> Result<PathBuf, String> {
    let path = crate::paths::trusted_keys_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, keys.to_text()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// `uniwidged keygen [--out FILE]`
pub fn keygen(args: &[String]) -> Result<(), String> {
    let out = crate::flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("uniwidged-signing-key"));
    if out.exists() {
        return Err(format!(
            "{} already exists. A signing key is not something to overwrite by \
             accident: move it aside first",
            out.display()
        ));
    }
    let key = SecretKey::generate().map_err(|e| e.to_string())?;
    write_private(&out, &format!("{}\n", key.to_hex()))?;

    println!("\n  secret key: {}", out.display());
    println!("  public key: {}", key.public().to_hex());
    println!(
        "\n  Keep the secret key. Anybody who has it can publish widgets in \
         your name.\n  Give people the public key; they add it with \
         `uniwidged trust <key>`.\n"
    );
    Ok(())
}

/// Writes a file only its owner can read.
///
/// A signing key with default permissions is a signing key every process on a
/// shared machine can read. Set before the bytes are written rather than after,
/// or there is a window where it is world-readable.
fn write_private(path: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// `uniwidged sign <widget.wasm> --key FILE`
pub fn sign(args: &[String]) -> Result<(), String> {
    let component = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which widget? `uniwidged sign <widget.wasm> --key <file>`")?;
    let component = PathBuf::from(component);
    let key_path = crate::flag(args, "--key")
        .ok_or("which key? `uniwidged sign <widget.wasm> --key <file>`")?;

    let key_text = std::fs::read_to_string(&key_path).map_err(|e| format!("{key_path}: {e}"))?;
    let key = SecretKey::from_hex(key_text.trim()).map_err(|e| format!("{key_path}: {e}"))?;

    let manifest_path = crate::install::manifest_for(&component, args)?;
    let (bytes, manifest) = read_widget(&component, &manifest_path)?;

    // Both, always. A signature over the component alone lets somebody widen
    // the manifest and keep the signature — see `uniwidged_runtime::signature`.
    let file = SignatureFile {
        signature: key.sign(&bytes, &manifest),
        key: key.public(),
    };
    // The manifest copy first, the signature last: if writing the copy fails,
    // the old signature still matches the old manifest beside it, rather than
    // a new signature sitting next to a manifest it does not cover.
    // What a user installs is three files side by side — the component, its
    // manifest and the signature — and the signature only verifies against
    // the manifest bytes it covered. When those came from the crate's
    // `widget.toml`, put that exact text beside the component, so that what
    // gets published is the triple that verifies rather than two of its three.
    let beside = component.with_extension("toml");
    let already_beside = beside
        .canonicalize()
        .is_ok_and(|b| manifest_path.canonicalize().is_ok_and(|m| m == b));
    if !already_beside {
        write_replacing(&beside, &manifest)?;
        println!("  wrote {} (the manifest it covers)", beside.display());
    }
    let out = signature_path(&component);
    write_replacing(&out, file.to_text())?;
    println!(
        "  signed {} and {}",
        component.display(),
        manifest_path.display()
    );
    println!("  wrote {}", out.display());

    println!("  with  {}", key.public().to_hex());
    println!("  publish all three: the .wasm, the .toml and the .sig, side by side");
    Ok(())
}

/// Writes `path` whole or not at all: the bytes go to a temporary file in the
/// same directory, which then replaces `path` in one rename. A write that
/// fails halfway leaves the old file as it was, never a truncated one.
fn write_replacing(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let written =
        std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map_err(shown)
}

/// `uniwidged trust <hex key> [--name NAME]`
pub fn trust(args: &[String]) -> Result<(), String> {
    let key = args
        .first()
        .filter(|a| !a.starts_with('-'))
        .ok_or("which key? `uniwidged trust <public key> [--name <name>]`")?;
    let key = PublicKey::from_hex(key).map_err(|e| e.to_string())?;
    let name = crate::flag(args, "--name").unwrap_or_default();

    let mut keys = trusted()?;
    keys.trust(key, &name);
    let path = write_trusted(&keys)?;
    println!(
        "  trusting {}… {}\n  recorded in {}",
        key.short(),
        if name.is_empty() {
            String::new()
        } else {
            format!("({name})")
        },
        path.display()
    );
    Ok(())
}

/// `uniwidged untrust <hex key>`
pub fn untrust(args: &[String]) -> Result<(), String> {
    let key = args
        .first()
        .ok_or("which key? `uniwidged untrust <public key>`")?;
    let key = PublicKey::from_hex(key).map_err(|e| e.to_string())?;

    let mut keys = trusted()?;
    if !keys.forget(key) {
        return Err(format!("{}… was not trusted anyway", key.short()));
    }
    write_trusted(&keys)?;
    println!(
        "  no longer trusting {}…\n  widgets it signed stay installed; they \
         will not verify next time",
        key.short()
    );
    Ok(())
}

/// `uniwidged trusted` — what this machine will accept.
pub fn list(_args: &[String]) -> Result<(), String> {
    let keys = trusted()?;
    if keys.is_empty() {
        println!(
            "  this machine trusts no signing keys, so every widget has to be \
             installed with --allow-unsigned"
        );
        return Ok(());
    }
    for (key, name) in keys.keys() {
        println!("  {}  {name}", key.to_hex());
    }
    Ok(())
}

/// Where a widget's detached signature lives: beside it, with `.sig` appended.
pub fn signature_path(component: &Path) -> PathBuf {
    let mut path = component.as_os_str().to_os_string();
    path.push(".sig");
    PathBuf::from(path)
}

/// Reads a component and its manifest — the two things a signature covers.
pub fn read_widget(component: &Path, manifest: &Path) -> Result<(Vec<u8>, String), String> {
    let bytes = std::fs::read(component).map_err(|e| format!("{}: {e}", component.display()))?;
    let manifest =
        std::fs::read_to_string(manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    Ok((bytes, manifest))
}

/// What `install` and `check` report about a widget's provenance.
pub enum Provenance {
    /// Signed by a key this machine trusts, under this name.
    Trusted(String),
    /// There is a signature and it does not verify. Never installable.
    Refused(String),
    /// There is no signature at all.
    Unsigned,
}

/// Checks a widget's signature, if it has one.
pub fn provenance(component: &Path, manifest: &Path) -> Result<Provenance, String> {
    let signature = signature_path(component);
    let text = match std::fs::read_to_string(&signature) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Provenance::Unsigned),
        Err(e) => return Err(format!("{}: {e}", signature.display())),
    };
    let file = SignatureFile::parse(&text).map_err(|e| format!("{}: {e}", signature.display()))?;
    let (bytes, manifest) = read_widget(component, manifest)?;

    Ok(match trusted()?.verify(&bytes, &manifest, &file) {
        // A key trusted without a name is reported by its first characters,
        // which is at least something a person can compare against what they
        // were sent.
        Ok("") => Provenance::Trusted(format!("{}…", file.key.short())),
        Ok(name) => Provenance::Trusted(name.to_string()),
        Err(why) => Provenance::Refused(why.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::write_replacing;

    #[test]
    fn a_replacement_is_whole_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("uniwidged-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clock.toml");
        std::fs::write(&path, "old").unwrap();

        write_replacing(&path, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(left.len(), 1, "the temporary file is gone");

        // A destination that cannot be replaced: the old contents survive.
        let blocked = dir.join("blocked");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(write_replacing(&blocked, "new").is_err());
        assert!(blocked.join("inside").is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
