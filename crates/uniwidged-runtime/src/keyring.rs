//! The keychain, namespaced to one widget.
//!
//! `secrets` was the last capability whose gate had nothing behind it, and the
//! reason it was last is that it is the only one where getting the *storage*
//! wrong is worse than not having it. A widget that cannot reach a keychain
//! shows a sign-in button. A widget whose token was written to a file anybody
//! could read has already leaked it, and nothing later undoes that.
//!
//! ## What this is, and what it is not
//!
//! [`SecretStore`] is the seam. Behind it, today, is [`FileKeyring`]: one
//! file, encrypted end to end with XChaCha20-Poly1305 under a key that lives
//! in a second file with mode `0600`. That is a real improvement on the two
//! alternatives a framework usually ships — a plain-text file, or nothing —
//! and it is honestly *not* a platform keychain:
//!
//! * **It protects against** a backup, a synced dotfiles repository, a
//!   screen-share of a terminal, another user on a multi-user machine, and
//!   anybody who ends up with the store file without the key file. It also
//!   protects against *editing*: the whole store is one AEAD message, so a
//!   store somebody changed fails to open rather than decrypting to something
//!   a widget then believes.
//! * **It does not protect against** a process already running as you. Nothing
//!   that keeps its key on the same disk can, and saying otherwise is how
//!   people come to store things here that they should not. A session keychain
//!   — the Secret Service on Linux, DPAPI on Windows, the macOS keychain — can,
//!   because the key is held by a daemon that the login unlocked and that can
//!   refuse. Those are backends behind this same trait; see `ROADMAP.md`.
//!
//! [`Keyring::describe`] returns that sentence, and `uniwidged doctor` prints
//! it, because a user deciding whether to let a widget store a password is
//! entitled to know which of the two they have.
//!
//! ## What happens on a widget's deadline
//!
//! Every call here is made from inside a widget's `view`, on its sixteen-
//! millisecond deadline, so what these functions cost is charged to the
//! widget. That shapes two decisions.
//!
//! A **read is memory**. The store is opened once — decrypted, checked,
//! kept — and every `get` after that is a lookup in a `Vec`. Re-reading and
//! re-decrypting a file on every frame would be the host spending a widget's
//! budget on the host's own storage choice, which is the mistake `Tasks::start`
//! made by spawning a thread per request.
//!
//! A **write is one `write` and one `rename`, and no `fsync`**. The first
//! version had one, and it is exactly the kind of thing that looks careful and
//! is not: an `fsync` waits on a device, so it is unbounded, and under a
//! parallel test run it interrupted widgets for the crime of storing a token.
//! The rename is what actually matters — it is atomic, so the failure a
//! keychain cannot survive (a torn file that no longer opens) cannot happen.
//! What a power cut can cost is the most recent secret, which is a sign-in to
//! do again.
//!
//! The write still happens *before* the call returns, rather than behind it:
//! a widget told its credential was stored has to be a widget whose credential
//! is stored, and write-behind trades that away to buy microseconds.
//!
//! ## Namespacing
//!
//! A widget never names another widget. The handle it is served through
//! carries its own namespace — the same shape as `TaskHandle`, and for the
//! same reason: a host function that cannot be called without saying whose it
//! is cannot be called on somebody else's behalf by mistake.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

/// The most one secret may be, in bytes.
///
/// A secret is a token, a password or a key. Sixty-four kilobytes is far more
/// than any of those and far short of a way to use the keychain as a
/// filesystem — which matters because the whole store is decrypted into memory
/// to read one entry.
pub const MAX_SECRET_BYTES: usize = 64 * 1024;

/// The most keys one widget may hold.
///
/// Not a limit anybody legitimately reaches: a widget has an account or two.
/// It exists so that a widget writing a key per view cannot grow the store
/// without bound, and it is refused rather than evicted, because silently
/// dropping the oldest credential is a bug whose symptom appears a week later.
pub const MAX_KEYS_PER_WIDGET: usize = 64;

/// The longest a key name may be.
pub const MAX_KEY_LEN: usize = 64;

/// Why a keychain operation did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyringError {
    /// The key name is not one this store accepts. A fact about the request:
    /// the widget's author can fix it, and the message says how.
    BadKey(String),
    /// The value is larger than [`MAX_SECRET_BYTES`], or the widget already
    /// holds [`MAX_KEYS_PER_WIDGET`].
    TooMuch(String),
    /// The store exists and could not be opened or written — a permission, a
    /// full disk, a key file somebody replaced. A fact about *this host*.
    Unavailable(String),
}

impl std::fmt::Display for KeyringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyringError::BadKey(why) | KeyringError::TooMuch(why) => write!(f, "{why}"),
            KeyringError::Unavailable(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for KeyringError {}

/// Somewhere secrets can be kept.
///
/// Every method takes the widget's namespace explicitly rather than binding
/// one at construction, because a desktop has one store and many widgets. The
/// binding happens in [`Keyring`], which is what a widget is actually served
/// through.
pub trait SecretStore: Send + Sync {
    /// The value, or `None` if this widget never set that key.
    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, KeyringError>;
    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<(), KeyringError>;
    /// Whether something was actually removed.
    fn remove(&self, namespace: &str, key: &str) -> Result<bool, KeyringError>;
    /// Everything the user should be told about where their secrets are going,
    /// in one line. Printed by `uniwidged doctor` and by `install`.
    fn describe(&self) -> String;
}

/// One widget's view of the keychain.
///
/// Cheap to clone, and carries the namespace, so no host function can reach
/// another widget's secrets: there is no argument through which to name one.
#[derive(Clone)]
pub struct Keyring {
    store: Arc<dyn SecretStore>,
    namespace: String,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The namespace is a widget name and is not secret; nothing else here
        // is printable without printing what this type exists to keep.
        f.debug_struct("Keyring")
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl Keyring {
    pub fn new(store: Arc<dyn SecretStore>, namespace: impl Into<String>) -> Self {
        Self {
            store,
            namespace: namespace.into(),
        }
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, KeyringError> {
        check_key(key)?;
        self.store.get(&self.namespace, key)
    }

    pub fn set(&self, key: &str, value: &[u8]) -> Result<(), KeyringError> {
        check_key(key)?;
        if value.len() > MAX_SECRET_BYTES {
            return Err(KeyringError::TooMuch(format!(
                "a secret may be at most {MAX_SECRET_BYTES} bytes"
            )));
        }
        self.store.set(&self.namespace, key, value)
    }

    pub fn remove(&self, key: &str) -> Result<bool, KeyringError> {
        check_key(key)?;
        self.store.remove(&self.namespace, key)
    }

    /// What this widget's secrets are actually kept in, in one line.
    pub fn describe(&self) -> String {
        self.store.describe()
    }
}

/// What a key name may be.
///
/// The same alphabet a manifest name may use, and for a related reason: a key
/// ends up on both sides of a separator in the store's own encoding, and a
/// name that could contain one would be a name that could claim to be two.
/// Bounded, because a widget must not be able to grow the store one long name
/// at a time.
fn check_key(key: &str) -> Result<(), KeyringError> {
    if key.is_empty() {
        return Err(KeyringError::BadKey(
            "a secret's key may not be empty".into(),
        ));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(KeyringError::BadKey(format!(
            "a secret's key may be at most {MAX_KEY_LEN} characters"
        )));
    }
    if !key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(KeyringError::BadKey(
            "a secret's key may hold only letters, digits, `.`, `_` and `-`".into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The file-backed store
// ---------------------------------------------------------------------------

/// The domain separator mixed into every message this store authenticates.
///
/// So that a ciphertext produced here can never be presented to anything else
/// this project comes to encrypt, and a future format cannot be fed a v1
/// message. The version is *inside* the authenticated data, not only in the
/// header a reader parses.
const AAD: &[u8] = b"uniwidged.keyring.v1";

/// The first line of the store file, so a person who finds it knows what it is.
const MAGIC: &str = "uniwidged-keyring v1";

/// A keychain in two files: one holding the key, one holding everything else.
///
/// The store is rewritten whole on every change. That is not a scale
/// compromise being ignored — the file holds tens of entries by construction
/// ([`MAX_KEYS_PER_WIDGET`] per widget) — it is what lets the entire store be
/// one AEAD message, which is what makes editing it detectable and which key
/// names exist unreadable. A per-entry format leaks the list of key names to
/// anybody holding the file, and the names are frequently the interesting part
/// (`github.token` says more than its ciphertext).
pub struct FileKeyring {
    store_path: PathBuf,
    key_path: PathBuf,
    /// The decrypted store, and the key it was opened with.
    ///
    /// Read once and then kept, because **every one of these calls happens
    /// inside a widget's `view`, on its sixteen-millisecond deadline**. Going
    /// to disk, deriving nothing and decrypting on every `get_secret` is work
    /// the widget is charged for and did not ask for — the same mistake
    /// `Tasks::start` made by spawning a thread per request, and with the same
    /// symptom: a widget interrupted mid-call because the host chose to do
    /// something slow on its behalf.
    ///
    /// Caching is sound because this process is the only writer. A keychain
    /// edited underneath a running desktop is not a case to support: the file
    /// is one AEAD message, so an edit could only *break* it, and the desktop
    /// would then be holding the last good copy.
    ///
    /// The whole thing is under one lock. Two widgets setting a secret in the
    /// same turn of the loop is not possible today — there is one thread — but
    /// a lost update here is a credential that silently disappears, which is
    /// the kind of bug that gets blamed on the widget.
    open: Mutex<Option<Open>>,
}

/// A keychain that has been opened: everything in it, and the key it was
/// opened with — if there was anything to open.
///
/// `key` is `None` where the store does not exist yet, which is a desktop on
/// which no widget has stored a secret. Generating one then would leave a
/// keychain key behind on a machine that has no keychain, and a widget that
/// only ever *reads* should leave nothing at all — the same rule as the data
/// directory. It is generated by the first write.
struct Open {
    key: Option<[u8; 32]>,
    entries: Vec<Entry>,
}

impl FileKeyring {
    /// Nothing is created here, and nothing is created by reading: a desktop
    /// on which no widget ever stores a secret leaves no keychain behind, key
    /// file included. The first `set` creates all of it.
    pub fn new(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref();
        Self {
            store_path: dir.join("keyring.enc"),
            key_path: dir.join("keyring.key"),
            open: Mutex::new(None),
        }
    }

    pub fn store_path(&self) -> &Path {
        &self.store_path
    }

    pub fn key_path(&self) -> &Path {
        &self.key_path
    }

    /// The store's key, generated on first use.
    ///
    /// Written `0600` before anything is put in it, and *refused* if it is
    /// found with wider permissions than that. A key file somebody else can
    /// read is a store somebody else can read, and continuing while saying
    /// nothing would make the encryption decorative. The check is not that the
    /// mode equals `0600` — a user may legitimately have it `0400` — but that
    /// group and other have nothing.
    fn key(&self) -> Result<[u8; 32], KeyringError> {
        match std::fs::read(&self.key_path) {
            Ok(bytes) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = std::fs::metadata(&self.key_path)
                        .map_err(io("reading the keychain's key"))?
                        .permissions()
                        .mode();
                    if mode & 0o077 != 0 {
                        return Err(KeyringError::Unavailable(format!(
                            "{} is readable by more than you (mode {:o}); \
                             `chmod 600` it, or delete it to start a new keychain",
                            self.key_path.display(),
                            mode & 0o777
                        )));
                    }
                }
                let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    KeyringError::Unavailable(format!(
                        "{} is not a keychain key: it should be exactly 32 bytes",
                        self.key_path.display()
                    ))
                })?;
                Ok(key)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.new_key(),
            Err(e) => Err(KeyringError::Unavailable(format!(
                "{}: {e}",
                self.key_path.display()
            ))),
        }
    }

    fn new_key(&self) -> Result<[u8; 32], KeyringError> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| {
            KeyringError::Unavailable(format!("this machine gave no entropy for a new key: {e}"))
        })?;
        if let Some(parent) = self.key_path.parent() {
            std::fs::create_dir_all(parent).map_err(io("creating the keychain's directory"))?;
        }
        write_private(&self.key_path, &key)?;
        Ok(key)
    }

    /// Everything in the store, as `(namespace, key) -> value`.
    ///
    /// An absent file is an empty store rather than an error: the first widget
    /// to ask for a secret finds nothing, which is its normal case.
    fn read_all(&self, key: &[u8; 32]) -> Result<Vec<Entry>, KeyringError> {
        let text = match std::fs::read_to_string(&self.store_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(KeyringError::Unavailable(format!(
                    "{}: {e}",
                    self.store_path.display()
                )));
            }
        };
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err(KeyringError::Unavailable(format!(
                "{} is not a uniwidged keychain",
                self.store_path.display()
            )));
        }
        let (Some(nonce), Some(sealed)) = (lines.next(), lines.next()) else {
            return Err(KeyringError::Unavailable(format!(
                "{} is truncated",
                self.store_path.display()
            )));
        };
        let nonce = unhex(nonce.trim()).ok_or_else(|| corrupt(&self.store_path))?;
        let sealed = unhex(sealed.trim()).ok_or_else(|| corrupt(&self.store_path))?;
        if nonce.len() != 24 {
            return Err(corrupt(&self.store_path));
        }
        let cipher = XChaCha20Poly1305::new(key.into());
        // The one error a user is likely to actually hit here is a key file
        // from a different keychain, so the message says so rather than
        // reporting an authentication tag.
        let plain = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &sealed,
                    aad: AAD,
                },
            )
            .map_err(|_| {
                KeyringError::Unavailable(format!(
                    "{} does not open with {} — it was written by a different \
                     key, or something changed it",
                    self.store_path.display(),
                    self.key_path.display()
                ))
            })?;
        decode(&plain).ok_or_else(|| corrupt(&self.store_path))
    }

    fn write_all(&self, key: &[u8; 32], entries: &[Entry]) -> Result<(), KeyringError> {
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|e| {
            KeyringError::Unavailable(format!("this machine gave no entropy for a nonce: {e}"))
        })?;
        let cipher = XChaCha20Poly1305::new(key.into());
        let plain = encode(entries);
        let sealed = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: AAD,
                },
            )
            .map_err(|_| KeyringError::Unavailable("the keychain could not be sealed".into()))?;
        let body = format!("{MAGIC}\n{}\n{}\n", hex(&nonce), hex(&sealed));
        if let Some(parent) = self.store_path.parent() {
            std::fs::create_dir_all(parent).map_err(io("creating the keychain's directory"))?;
        }
        write_private(&self.store_path, body.as_bytes())
    }
}

/// One stored secret.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    namespace: String,
    key: String,
    value: Vec<u8>,
}

impl FileKeyring {
    /// Runs `f` against the open store, opening it first if it is not.
    ///
    /// The one place the cache is filled, so there is one answer to "when is
    /// the file read?": once, on the first call that needs it. A failure to
    /// open is *not* cached — a user who fixes the mode on their key file
    /// should not have to restart the desktop.
    fn with_open<T>(
        &self,
        f: impl FnOnce(&mut Open) -> Result<T, KeyringError>,
    ) -> Result<T, KeyringError> {
        let mut guard = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(match self.store_path.try_exists() {
                // Nothing stored yet. Not an error, and not a reason to
                // generate a key: this is the state a desktop starts in.
                Ok(false) => Open {
                    key: None,
                    entries: Vec::new(),
                },
                Ok(true) => {
                    let key = self.key()?;
                    let entries = self.read_all(&key)?;
                    Open {
                        key: Some(key),
                        entries,
                    }
                }
                Err(e) => {
                    return Err(KeyringError::Unavailable(format!(
                        "{}: {e}",
                        self.store_path.display()
                    )));
                }
            });
        }
        f(guard.as_mut().expect("just opened"))
    }

    /// The key this store writes with, generating one on the first write.
    fn writing_key(&self, open: &mut Open) -> Result<[u8; 32], KeyringError> {
        match open.key {
            Some(key) => Ok(key),
            None => {
                let key = self.key()?;
                open.key = Some(key);
                Ok(key)
            }
        }
    }
}

impl SecretStore for FileKeyring {
    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, KeyringError> {
        self.with_open(|open| {
            Ok(open
                .entries
                .iter()
                .find(|e| e.namespace == namespace && e.key == key)
                .map(|e| e.value.clone()))
        })
    }

    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<(), KeyringError> {
        self.with_open(|open| {
            match open
                .entries
                .iter_mut()
                .find(|e| e.namespace == namespace && e.key == key)
            {
                Some(existing) => {
                    if existing.value == value {
                        // A widget that stores the same token every hour is
                        // the ordinary case, and rewriting the file for it is
                        // a disk write charged to that widget's deadline for
                        // no change at all.
                        return Ok(());
                    }
                    existing.value = value.to_vec();
                }
                None => {
                    let mine = open
                        .entries
                        .iter()
                        .filter(|e| e.namespace == namespace)
                        .count();
                    if mine >= MAX_KEYS_PER_WIDGET {
                        return Err(KeyringError::TooMuch(format!(
                            "a widget may keep at most {MAX_KEYS_PER_WIDGET} secrets"
                        )));
                    }
                    open.entries.push(Entry {
                        namespace: namespace.to_string(),
                        key: key.to_string(),
                        value: value.to_vec(),
                    });
                }
            }
            // The file is written before the call returns, so a widget told
            // its secret was stored is a widget whose secret is stored. The
            // in-memory copy is a cache of the file, never the other way
            // round: a `set` that reported success and lost the write on a
            // crash is the failure this type exists to prevent.
            let key = self.writing_key(open)?;
            self.write_all(&key, &open.entries)
        })
    }

    fn remove(&self, namespace: &str, key: &str) -> Result<bool, KeyringError> {
        self.with_open(|open| {
            let before = open.entries.len();
            open.entries
                .retain(|e| !(e.namespace == namespace && e.key == key));
            if open.entries.len() == before {
                return Ok(false);
            }
            let key = self.writing_key(open)?;
            self.write_all(&key, &open.entries)?;
            Ok(true)
        })
    }

    fn describe(&self) -> String {
        format!(
            "an encrypted file at {} — safe from a backup or another user, \
             not from a program already running as you",
            self.store_path.display()
        )
    }
}

/// A keychain that keeps nothing past the process.
///
/// For a test, and for an embedder that runs widgets and should not leave a
/// keychain on somebody's disk. A widget cannot tell it from the real one
/// within a single run, which is the point: the behaviour under test is the
/// same, and only the durability is not.
///
/// It is deliberately **not** what `uniwidged snapshot` uses. That command
/// gives a widget no keychain at all, for the same reason it gives one no data
/// directory: a widget handed empty storage it will find empty every time is
/// being told something false, and `unavailable` is the truth.
#[derive(Default)]
pub struct MemoryKeyring {
    entries: Mutex<Vec<Entry>>,
}

impl MemoryKeyring {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryKeyring {
    fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, KeyringError> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        Ok(entries
            .iter()
            .find(|e| e.namespace == namespace && e.key == key)
            .map(|e| e.value.clone()))
    }

    fn set(&self, namespace: &str, key: &str, value: &[u8]) -> Result<(), KeyringError> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries
            .iter_mut()
            .find(|e| e.namespace == namespace && e.key == key)
        {
            Some(existing) => existing.value = value.to_vec(),
            None => {
                if entries.iter().filter(|e| e.namespace == namespace).count()
                    >= MAX_KEYS_PER_WIDGET
                {
                    return Err(KeyringError::TooMuch(format!(
                        "a widget may keep at most {MAX_KEYS_PER_WIDGET} secrets"
                    )));
                }
                entries.push(Entry {
                    namespace: namespace.to_string(),
                    key: key.to_string(),
                    value: value.to_vec(),
                });
            }
        }
        Ok(())
    }

    fn remove(&self, namespace: &str, key: &str) -> Result<bool, KeyringError> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let before = entries.len();
        entries.retain(|e| !(e.namespace == namespace && e.key == key));
        Ok(entries.len() != before)
    }

    fn describe(&self) -> String {
        "memory only — forgotten when the desktop stops".into()
    }
}

// ---------------------------------------------------------------------------
// The plaintext encoding
// ---------------------------------------------------------------------------

/// `namespace \t key \t hex(value) \n`, one entry per line.
///
/// Never written to disk in this form — it is what the AEAD is given — so the
/// only requirement is that it round-trips exactly. Tab-separated is safe
/// because both names are checked against an alphabet that excludes it, and
/// the value is hex because it is arbitrary bytes and a newline in the middle
/// of one would end the line.
fn encode(entries: &[Entry]) -> Vec<u8> {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&entry.namespace);
        out.push('\t');
        out.push_str(&entry.key);
        out.push('\t');
        out.push_str(&hex(&entry.value));
        out.push('\n');
    }
    out.into_bytes()
}

fn decode(bytes: &[u8]) -> Option<Vec<Entry>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut entries = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let mut fields = line.split('\t');
        let namespace = fields.next()?;
        let key = fields.next()?;
        let value = fields.next()?;
        if fields.next().is_some() {
            return None;
        }
        entries.push(Entry {
            namespace: namespace.to_string(),
            key: key.to_string(),
            value: unhex(value)?,
        });
    }
    Some(entries)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Files that only you can read
// ---------------------------------------------------------------------------

/// Writes a file nobody else can read, and never widens one that exists.
///
/// The mode is set **before** the bytes go in — `OpenOptions::mode` applies at
/// creation — because a `chmod` afterwards leaves a window in which the file
/// exists with the process's umask on it, and a window is all it takes on a
/// shared machine. The write goes through a temporary file and a rename for
/// the same reason `DataDir::write` does: a keychain half-written when the
/// process was killed is a keychain that no longer opens, and this one holds
/// the only copy of something.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), KeyringError> {
    let mut temporary = path.to_path_buf();
    temporary.as_mut_os_string().push(".tmp");
    let _ = std::fs::remove_file(&temporary);
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(io(&format!("creating {}", temporary.display())))?;
        use std::io::Write;
        file.write_all(bytes)
            .map_err(io(&format!("writing {}", temporary.display())))?;
    }
    // There is deliberately **no `fsync`** here, and it was in the first
    // version. `set_secret` is called from inside a widget's `view`, on its
    // sixteen-millisecond deadline, and an `fsync` is not bounded work: it
    // waits on a device. On the machine this was written on it took long
    // enough, under a parallel test run, to have widgets interrupted for
    // storing a token — the sandbox punishing a widget for something the host
    // chose to do on its behalf, which is the same mistake `Tasks::start` made
    // by spawning a thread per request.
    //
    // What is kept is what matters: the write goes to a temporary file and is
    // *renamed* over the store, which is atomic, so a reader sees the whole
    // old file or the whole new one and never a torn one. A power cut can
    // therefore cost the most recent secret — a sign-in to do again — and
    // cannot leave a keychain that will not open.
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        KeyringError::Unavailable(format!("{}: {e}", path.display()))
    })
}

fn io(what: &str) -> impl Fn(std::io::Error) -> KeyringError + '_ {
    move |e| KeyringError::Unavailable(format!("{what}: {e}"))
}

fn corrupt(path: &Path) -> KeyringError {
    KeyringError::Unavailable(format!("{} is not readable as a keychain", path.display()))
}

#[cfg(test)]
mod tests;
