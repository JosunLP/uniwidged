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

use std::io::Write as _;
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
    let out = signature_path(&component);
    let copy = (!already_beside).then_some((beside.as_path(), manifest.as_bytes()));
    if put_in_place(copy, (&out, file.to_text().as_bytes()))? {
        println!("  wrote {} (the manifest it covers)", beside.display());
    }
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

/// Puts the manifest copy (when there is one) and the signature in place,
/// so that the pair beside the component always matches: the new pair, or
/// the old one. Answers whether the manifest copy was written.
///
/// Everything is written in full before anything is replaced — the copy, the
/// signature, and the old manifest's contents for putting back — so a failure
/// while writing changes nothing. Then the manifest, then the signature; if
/// the signature cannot be put in place, the old manifest goes back (or the
/// new copy goes, if there was none). Should even that fail, the old contents
/// are left on disk and the error says where.
fn put_in_place(
    manifest: Option<(&Path, &[u8])>,
    signature: (&Path, &[u8]),
) -> Result<bool, String> {
    // What is beside the component now, read before anything changes — a
    // manifest that is there and cannot be read is not one that is absent.
    let previous = match manifest {
        None => None,
        Some((path, _)) => match std::fs::read(path) {
            Ok(bytes) => Some(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        },
    };
    let copy = manifest
        .map(|(path, bytes)| Staged::new(path, bytes))
        .transpose()?;
    let signed = Staged::new(signature.0, signature.1)?;
    let restore = match (manifest, &previous) {
        (Some((path, _)), Some(Some(bytes))) => Some(Staged::new(path, bytes)?),
        _ => None,
    };

    let Some(copy) = copy else {
        signed.commit()?;
        return Ok(false);
    };
    let path = copy.path.clone();
    copy.commit()?;
    if let Err(why) = signed.commit() {
        let undone = match restore {
            Some(restore) => restore.commit_or_keep(),
            None => std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display())),
        };
        return Err(match undone {
            Ok(()) => why,
            Err(also) => {
                format!("{why}\n  and the manifest beside it could not be put back either: {also}")
            }
        });
    }
    Ok(true)
}

/// A file written in full beside `path`, not yet in its place.
///
/// The temporary file is created new, never opened if something is already
/// there — a symlink planted at its name in a shared directory cannot turn
/// the write into one somewhere else — and is removed again unless
/// [`Staged::commit`] puts it in place with one rename. Until then `path` is
/// as it was, however the writing went.
struct Staged {
    temporary: PathBuf,
    path: PathBuf,
    committed: bool,
}

impl Staged {
    fn new(path: &Path, bytes: &[u8]) -> Result<Self, String> {
        Self::with(path, |file| file.write_all(bytes))
    }

    fn with(
        path: &Path,
        fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
    ) -> Result<Self, String> {
        let shown = |e: std::io::Error| format!("{}: {e}", path.display());
        let mut attempt = 0u32;
        let (temporary, mut file) = loop {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".{}.{attempt}.tmp", std::process::id()));
            let temporary = path.with_file_name(name);
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => break (temporary, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 100 => {
                    attempt += 1;
                }
                Err(e) => return Err(shown(e)),
            }
        };
        // Ours from here on, so ours to remove if the writing fails.
        let staged = Staged {
            temporary,
            path: path.to_path_buf(),
            committed: false,
        };
        fill(&mut file)
            .and_then(|()| file.sync_all())
            .map_err(shown)?;
        Ok(staged)
    }

    fn commit(mut self) -> Result<(), String> {
        std::fs::rename(&self.temporary, &self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.committed = true;
        Ok(())
    }

    /// [`Staged::commit`], except that a failure keeps the written file
    /// rather than removing it, and says where it is: for contents that
    /// would otherwise be lost.
    fn commit_or_keep(mut self) -> Result<(), String> {
        let renamed = std::fs::rename(&self.temporary, &self.path);
        self.committed = true;
        renamed.map_err(|e| {
            format!(
                "{}: {e}; its previous contents are kept in {}",
                self.path.display(),
                self.temporary.display()
            )
        })
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
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
    use super::{Staged, put_in_place};
    use std::io::Write;

    #[test]
    fn a_replacement_is_whole_or_not_at_all() {
        let dir = std::env::temp_dir().join(format!("uniwidged-replace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clock.toml");
        std::fs::write(&path, "old").unwrap();
        let count = || std::fs::read_dir(&dir).unwrap().count();

        // Half written, then a failure: the old file is untouched, and the
        // temporary one is gone.
        let failed = Staged::with(&path, |file| {
            file.write_all(b"ne")?;
            Err(std::io::Error::other("the disk filled up"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(count(), 1);

        // Written but never committed: the same.
        drop(Staged::new(&path, b"new").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(count(), 1);

        // Something already at the temporary name is never written through.
        let decoy = dir.join(format!("clock.toml.{}.0.tmp", std::process::id()));
        std::fs::write(&decoy, "not yours").unwrap();
        Staged::new(&path, b"new").unwrap().commit().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&decoy).unwrap(), "not yours");
        assert_eq!(count(), 2);

        // A destination that cannot be replaced: nothing changes there either.
        let blocked = dir.join("blocked");
        std::fs::create_dir_all(blocked.join("inside")).unwrap();
        assert!(Staged::new(&blocked, b"new").unwrap().commit().is_err());
        assert!(blocked.join("inside").is_dir());
        assert_eq!(count(), 3);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_signature_that_cannot_be_put_in_place_leaves_the_old_pair() {
        let dir = std::env::temp_dir().join(format!("uniwidged-pair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("clock.toml");
        // A `.sig` path no file can be renamed onto: a directory with
        // something in it.
        let signature = dir.join("clock.wasm.sig");
        std::fs::create_dir_all(signature.join("in-the-way")).unwrap();
        let names = || {
            let mut names: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        };

        // An old manifest: it is put back, byte for byte.
        std::fs::write(&manifest, "old").unwrap();
        let failed = put_in_place(Some((&manifest, b"new")), (&signature, b"sig"));
        assert!(failed.is_err());
        assert_eq!(std::fs::read_to_string(&manifest).unwrap(), "old");
        assert_eq!(names(), ["clock.toml", "clock.wasm.sig"]);

        // No old manifest: the new copy does not stay behind on its own.
        std::fs::remove_file(&manifest).unwrap();
        assert!(put_in_place(Some((&manifest, b"new")), (&signature, b"sig")).is_err());
        assert_eq!(names(), ["clock.wasm.sig"]);

        // A manifest that is there and cannot be read stops everything first.
        std::fs::remove_dir_all(&signature).unwrap();
        std::fs::create_dir_all(manifest.join("unreadable")).unwrap();
        assert!(put_in_place(Some((&manifest, b"new")), (&signature, b"sig")).is_err());
        assert!(!signature.exists(), "nothing was signed");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn contents_that_cannot_be_put_back_are_kept_and_named() {
        let dir = std::env::temp_dir().join(format!("uniwidged-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blocked = dir.join("clock.toml");
        std::fs::create_dir_all(blocked.join("in-the-way")).unwrap();
        let why = Staged::new(&blocked, b"old")
            .unwrap()
            .commit_or_keep()
            .unwrap_err();
        let kept = why.rsplit("kept in ").next().unwrap();
        assert_eq!(std::fs::read_to_string(kept).unwrap(), "old", "{why}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
