//! Who wrote this widget, and did anybody change it since?
//!
//! A widget is somebody else's code, and the sandbox is what makes running it
//! reasonable *once it is running*. This is the question before that: is this
//! the thing its author published, and is its author somebody the user decided
//! to trust?
//!
//! ## What is signed
//!
//! **The component and its manifest together**, never the component alone.
//! That is the whole design, and getting it wrong is the interesting failure:
//! the manifest is what the user is shown and what the runtime enforces, so a
//! signature covering only the `.wasm` lets anybody take a signed widget,
//! replace `capabilities = []` with `capabilities = ["fs:", "net:*.evil.test"]`,
//! and hand it on with a signature that still verifies. The user would then be
//! shown a consent screen the author never wrote and approve it on the
//! strength of the author's name.
//!
//! The two are length-prefixed into the digest rather than concatenated. Without
//! that, bytes can be moved from the end of one field to the start of the next
//! without changing the hash — so a different (component, manifest) pair would
//! carry the same valid signature.
//!
//! And the digest is domain-separated, so a signature over a widget can never
//! be replayed as a signature over anything else the project comes to sign.
//!
//! ## What this deliberately is not
//!
//! Not a certificate hierarchy, not a revocation protocol, and not a
//! transparency log. A user has a file of keys they added by hand, one line
//! each, and a widget is signed by one of them or it is not. That is small
//! enough to be read and audited by the person it protects, and everything
//! larger needs infrastructure this project does not have and cannot honestly
//! run.

use ed25519_dalek::{Signer, Verifier};
use sha2::{Digest, Sha256};

/// Prepended to every digest, so a signature over a widget cannot be replayed
/// as a signature over something else. The version is part of it: changing what
/// goes into the digest must invalidate every old signature rather than
/// silently accepting them against new rules.
const DOMAIN: &[u8] = b"uniwidged-widget-signature-v1\0";

/// The same, for a registry index.
///
/// A *different* string, which is the whole reason the domain exists. Without
/// it, a signature over some widget's bytes could be presented as a signature
/// over an index — and a registry publisher's key and a widget author's key
/// are two different claims that must never be usable for each other's
/// sentence. See [`crate::registry`], where the two are argued at length.
const INDEX_DOMAIN: &[u8] = b"uniwidged-registry-index-v1\0";

/// The first token of a `.sig` file, so a file that is not one is refused with
/// a sentence rather than a parse error.
const MAGIC: &str = "uniwidged-sig-v1";

/// What a signature covers.
///
/// Length-prefixed and domain-separated — see the module note, where both are
/// the point rather than ceremony.
pub fn digest(component: &[u8], manifest: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update((component.len() as u64).to_le_bytes());
    hasher.update(component);
    hasher.update((manifest.len() as u64).to_le_bytes());
    hasher.update(manifest.as_bytes());
    hasher.finalize().into()
}

/// What a signature over a registry index covers.
///
/// The text exactly as it was served, byte for byte. Not a parse and not a
/// re-serialisation: a signature checked against a document this program
/// rewrote is a signature over something nobody signed, and every difference
/// between the two — a dropped comment, a re-ordered table, a normalised
/// number — would be a hole.
pub fn index_digest(index: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(INDEX_DOMAIN);
    hasher.update((index.len() as u64).to_le_bytes());
    hasher.update(index.as_bytes());
    hasher.finalize().into()
}

/// Was this index signed by the key this registry was pinned to?
///
/// The key is an argument rather than a lookup in [`TrustedKeys`], and that is
/// the design: trusting somebody to *publish a list* is not trusting them to
/// *write a widget*. A registry's key is pinned when the registry is added and
/// is used for nothing else, so taking over a registry does not let anybody
/// sign a widget, and a widget author's key does not let them rewrite a list.
pub fn verify_index(
    index: &str,
    signature: &SignatureFile,
    key: PublicKey,
) -> Result<(), VerifyError> {
    if signature.key != key {
        return Err(VerifyError::UntrustedKey(signature.key));
    }
    let verifying =
        ed25519_dalek::VerifyingKey::from_bytes(&key.0).map_err(|_| VerifyError::MalformedKey)?;
    verifying
        .verify(
            &index_digest(index),
            &ed25519_dalek::Signature::from_bytes(&signature.signature.0),
        )
        .map_err(|_| VerifyError::BadSignature)
}

/// Is this signature intact under the key it names?
///
/// **Not** an answer to "should this widget be installed" — that is
/// [`TrustedKeys::verify`], which asks whether the key is one this machine
/// decided to trust, and asks it *first* so that "signature valid" can never
/// be reported about a key nobody trusts.
///
/// This is the narrower question a *publisher* has: I am about to list this
/// widget in an index, and its `.sig` should be a signature over these exact
/// bytes. A signature that does not check out here is a file somebody has to
/// re-sign, and listing it would publish a download every client will reject.
pub fn verify_intact(
    component: &[u8],
    manifest: &str,
    signature: &SignatureFile,
) -> Result<(), VerifyError> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(&signature.key.0)
        .map_err(|_| VerifyError::MalformedKey)?;
    key.verify(
        &digest(component, manifest),
        &ed25519_dalek::Signature::from_bytes(&signature.signature.0),
    )
    .map_err(|_| VerifyError::BadSignature)
}

/// A public key somebody may have signed a widget with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        Ok(Self(from_hex(text)?))
    }

    pub fn to_hex(self) -> String {
        to_hex(&self.0)
    }

    /// The first eight characters, for showing a person which key was meant.
    ///
    /// A full key is sixty-four characters and nobody reads one; eight is
    /// enough to tell two of a user's own keys apart, which is all this is for.
    /// It is **not** an identifier — verification always uses the whole key.
    pub fn short(self) -> String {
        self.to_hex()[..8].to_string()
    }
}

/// A detached signature over [`digest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; 64]);

impl Signature {
    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        Ok(Self(from_hex(text)?))
    }

    pub fn to_hex(self) -> String {
        to_hex(&self.0)
    }
}

/// A signing key. Kept by whoever publishes widgets, and by nobody else.
pub struct SecretKey(ed25519_dalek::SigningKey);

impl SecretKey {
    /// A fresh key from the operating system's entropy.
    pub fn generate() -> Result<Self, KeyError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|_| KeyError::NoEntropy)?;
        Ok(Self(ed25519_dalek::SigningKey::from_bytes(&seed)))
    }

    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        Ok(Self(ed25519_dalek::SigningKey::from_bytes(&from_hex(
            text,
        )?)))
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0.to_bytes())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key().to_bytes())
    }

    pub fn sign(&self, component: &[u8], manifest: &str) -> Signature {
        Signature(self.0.sign(&digest(component, manifest)).to_bytes())
    }

    /// Signs a registry index. See [`index_digest`] on what is covered.
    pub fn sign_index(&self, index: &str) -> Signature {
        Signature(self.0.sign(&index_digest(index)).to_bytes())
    }
}

/// A `.sig` file: one signature and the key that made it.
///
/// The key is written beside the signature so that a person can see *which*
/// key a widget claims to be signed by before deciding whether to trust it.
/// Carrying it changes nothing about the check — an untrusted key is refused
/// however convincingly it signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureFile {
    pub signature: Signature,
    pub key: PublicKey,
}

impl SignatureFile {
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let line = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .ok_or(KeyError::NotASignature)?;
        let mut parts = line.split_whitespace();
        if parts.next() != Some(MAGIC) {
            return Err(KeyError::NotASignature);
        }
        let signature = Signature::from_hex(parts.next().ok_or(KeyError::NotASignature)?)?;
        let key = PublicKey::from_hex(parts.next().ok_or(KeyError::NotASignature)?)?;
        if parts.next().is_some() {
            return Err(KeyError::NotASignature);
        }
        Ok(Self { signature, key })
    }

    pub fn to_text(&self) -> String {
        format!(
            "{MAGIC} {} {}\n",
            self.signature.to_hex(),
            self.key.to_hex()
        )
    }
}

/// The keys a user has decided to trust, one line each.
///
/// ```text
/// # ~/.config/uniwidged/trusted-keys
/// 3d40…  jane's widgets
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedKeys {
    keys: Vec<(PublicKey, String)>,
}

impl TrustedKeys {
    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let mut keys = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, name) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            keys.push((PublicKey::from_hex(key)?, name.trim().to_string()));
        }
        Ok(Self { keys })
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from(
            "# Keys this machine trusts to sign widgets. One per line:\n\
             #   <64 hex characters>  a name for you to recognise it by\n\
             # Added with `uniwidged trust`. Removing a line withdraws trust\n\
             # from anything that key signed, on the next install.\n",
        );
        for (key, name) in &self.keys {
            out.push_str(&format!("{}  {name}\n", key.to_hex()));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn keys(&self) -> &[(PublicKey, String)] {
        &self.keys
    }

    /// Adds a key, or renames one already there.
    ///
    /// Idempotent on purpose: `uniwidged trust` run twice should not leave two
    /// lines, and a file with a key twice should not make it twice as trusted.
    pub fn trust(&mut self, key: PublicKey, name: &str) {
        match self.keys.iter_mut().find(|(known, _)| *known == key) {
            Some((_, existing)) => *existing = name.to_string(),
            None => self.keys.push((key, name.to_string())),
        }
    }

    /// Withdraws trust. `true` if the key was there.
    pub fn forget(&mut self, key: PublicKey) -> bool {
        let before = self.keys.len();
        self.keys.retain(|(known, _)| *known != key);
        self.keys.len() != before
    }

    /// Is this widget signed by somebody this machine trusts?
    ///
    /// Returns the name the user gave the key, so the answer can be reported as
    /// "signed by jane's widgets" rather than as sixty-four hex characters.
    pub fn verify(
        &self,
        component: &[u8],
        manifest: &str,
        signature: &SignatureFile,
    ) -> Result<&str, VerifyError> {
        // Trust first. Checking the signature against an untrusted key and
        // *then* asking whether the key is trusted would be correct too, but it
        // invites a later refactor that reports "signature valid" for a key
        // nobody trusts — which is exactly the sentence a user should never see.
        let (_, name) = self
            .keys
            .iter()
            .find(|(known, _)| *known == signature.key)
            .ok_or(VerifyError::UntrustedKey(signature.key))?;

        let key = ed25519_dalek::VerifyingKey::from_bytes(&signature.key.0)
            .map_err(|_| VerifyError::MalformedKey)?;
        key.verify(
            &digest(component, manifest),
            &ed25519_dalek::Signature::from_bytes(&signature.signature.0),
        )
        .map_err(|_| VerifyError::BadSignature)?;
        Ok(name)
    }
}

/// Why a key or a signature file could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Not the right number of hex characters, or not hex at all.
    Malformed,
    NotASignature,
    NoEntropy,
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Malformed => write!(
                f,
                "that is not a uniwidged key: they are hexadecimal, and a \
                 public key is 64 characters"
            ),
            KeyError::NotASignature => write!(
                f,
                "that is not a uniwidged signature file; one line reading \
                 `{MAGIC} <signature> <key>` is the whole format"
            ),
            KeyError::NoEntropy => {
                write!(f, "the operating system would not provide random bytes")
            }
        }
    }
}

impl std::error::Error for KeyError {}

/// Why a widget was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// Signed by a key this machine has not been told to trust. The commonest
    /// answer, and not an accusation: it is what every widget looks like before
    /// its author's key is added.
    UntrustedKey(PublicKey),
    /// A trusted key, and a signature that does not match — so the component or
    /// its manifest is not what was signed.
    BadSignature,
    MalformedKey,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::UntrustedKey(key) => write!(
                f,
                "signed by {}…, which this machine does not trust. If you know \
                 whose key that is, `uniwidged trust` it",
                key.short()
            ),
            VerifyError::BadSignature => write!(
                f,
                "the signature does not match this widget: either the component \
                 or its manifest has been changed since it was signed"
            ),
            VerifyError::MalformedKey => write!(f, "that signature names an unusable key"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Bytes as lower-case hexadecimal.
///
/// Public because a digest is written into a registry index as well as read
/// out of one, and two spellings of the same thirty-two bytes is a comparison
/// that fails for no reason a person can see.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Thirty-two bytes of hexadecimal, or nothing.
///
/// The digest reader for [`crate::registry`], which wants an `Option` rather
/// than this module's [`KeyError`]: a bad digest in an index is a fact about
/// the index, and it says so in its own words.
pub fn hex32(text: &str) -> Option<[u8; 32]> {
    from_hex::<32>(text).ok()
}

/// Hex to a fixed-size array, refusing anything else.
///
/// Fixed-size rather than a `Vec`: a key of the wrong length is a different
/// key, and accepting one padded or truncated is how a check comes to pass on
/// something nobody meant.
fn from_hex<const N: usize>(text: &str) -> Result<[u8; N], KeyError> {
    let text = text.trim();
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(KeyError::Malformed);
    }
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| KeyError::Malformed)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
