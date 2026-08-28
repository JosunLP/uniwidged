//! Somewhere to find widgets.
//!
//! Signature verification answered *is this the thing its author published*.
//! It never answered the question before it: **where does a widget come from
//! in the first place?** Until this module, the answer was "somebody sends you
//! a `.wasm` file", which works exactly as well as it sounds.
//!
//! A registry is one signed TOML document listing widgets, served over HTTPS.
//! That is the whole of it. No accounts, no API, no server to run: a registry
//! can be a file on a static host, which is the point — a project that needs
//! infrastructure to distribute its own widgets does not get distributed.
//!
//! ```toml
//! index = "uniwidged-registry-v1"
//!
//! [[widget]]
//! name = "clock"
//! version = "1.0.0"
//! description = "The time and the date, in your locale"
//! capabilities = ["timer", "locale"]
//! component = "https://widgets.example/clock-1.0.0.wasm"
//! manifest = "https://widgets.example/clock-1.0.0.toml"
//! signature = "https://widgets.example/clock-1.0.0.wasm.sig"
//! digest = "a2f1…"
//! ```
//!
//! ## Two signatures, and why they are not the same signature
//!
//! The index is signed by **whoever publishes the list**, and the key is
//! pinned when the registry is added — `uniwidged registry add <url> --key
//! <hex>`. A widget is signed by **whoever wrote it**, and that key lives in
//! the machine's `trusted-keys` file. They are different claims by different
//! people and are checked separately:
//!
//! * The index signature says *this list is the one its publisher wrote*. It
//!   is what stops a mirror, a cache or anybody on the path from adding an
//!   entry, changing a URL, or pointing a name at somebody else's bytes.
//! * The widget signature says *this component is the one its author
//!   published*, and is checked by `install` exactly as it is for a widget
//!   that arrived by email. **A registry cannot vouch a widget past it.**
//!   Downloading is not trusting.
//!
//! ## What `digest` is, and why it is that digest
//!
//! It is [`signature::digest`] — the same domain-separated hash over the
//! component **and** its manifest that a widget's own signature covers. Not a
//! plain hash of the `.wasm`, for the reason that function exists: a hash over
//! the component alone lets somebody keep the bytes and widen the manifest,
//! and the manifest is the whole of what a user is asked to approve.
//!
//! So an index entry and a signature speak about the same object, and the
//! entry's `capabilities` line — the one a person reads in a listing before
//! deciding to install anything — cannot say one thing while the manifest the
//! runtime enforces says another. [`Entry::verify`] is where that is enforced,
//! and it refuses a download whose manifest disagrees with the entry that
//! offered it, even when everything is signed by everybody correctly.
//!
//! ## What a registry deliberately is not
//!
//! Not a package manager. There is no dependency resolution, because widgets
//! have no dependencies; no version ranges, because an entry is an exact
//! version and nothing asks for a range; and no lock file, because the index
//! *is* the lock — a name and a version resolve to a digest, and the digest is
//! what is checked after the bytes arrive.

use crate::capability::{Capability, Policy};
use crate::manifest::Manifest;
use crate::net::host_of;
use crate::signature::{self, PublicKey, SignatureFile};
use crate::version::Version;

/// The first key of an index, so a file that is not one is refused with a
/// sentence rather than with a missing-field error.
pub const INDEX_MAGIC: &str = "uniwidged-registry-v1";

/// The most index a registry may serve.
///
/// An index is text listing widgets; a megabyte is some thousands of them. The
/// cap is here because the alternative is a registry — or anybody who has
/// taken one over — being able to make this process read until it dies.
pub const MAX_INDEX_BYTES: usize = 1024 * 1024;

/// The most component a registry may serve.
///
/// Larger than [`crate::net::MAX_BODY`], which bounds what a *widget* may pull
/// into the desktop while it runs; this bounds a download a person asked for
/// and is waiting on. A widget component is a few hundred kilobytes of WASM in
/// practice, and the reference widgets are under one; sixteen mebibytes is
/// room for something unusual and still a bound.
pub const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;

/// One widget, as a registry offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub version: Version,
    pub description: String,
    /// What the manifest declares, as the manifest writes it.
    ///
    /// Repeated in the index so that a listing can be read — and a person can
    /// decide — without downloading anything. It is **not** the source of
    /// truth: the manifest is, and [`Entry::verify`] refuses a download whose
    /// manifest declares something else.
    pub capabilities: Vec<String>,
    pub component: String,
    pub manifest: String,
    /// Where the widget's own detached signature is, when it has one.
    ///
    /// Optional because a registry may carry unsigned widgets and a person may
    /// choose to install one with `--allow-unsigned`, exactly as they would a
    /// file somebody sent them. Absent is not "trusted anyway".
    pub signature: Option<String>,
    /// [`signature::digest`] of the component and the manifest — see the
    /// module note on why it is that digest and not a hash of the `.wasm`.
    pub digest: [u8; 32],
}

/// A registry's list of widgets, after it has been parsed and checked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub entries: Vec<Entry>,
}

/// Why an index — or something an index promised — was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    Syntax(String),
    /// The `index = "uniwidged-registry-v1"` line is missing or is something
    /// else. Usually an HTML error page a proxy served with a 200.
    NotAnIndex,
    /// An entry naming a widget by a name the host would refuse anyway.
    BadName(String),
    BadVersion {
        name: String,
        why: String,
    },
    /// A URL that is not `https://`. Checked here rather than at the fetch, so
    /// an index carrying one is refused whole instead of most of the way
    /// through a download.
    NotHttps {
        name: String,
        url: String,
    },
    BadDigest(String),
    /// Two entries with one name and one version. Which of them a resolver
    /// would pick is a coin toss, so neither is picked.
    Duplicate {
        name: String,
        version: String,
    },
    /// A capability string this host does not know, in the line a person is
    /// shown *before* installing. Refused rather than displayed verbatim: a
    /// listing that can print arbitrary text is a listing that can lie about
    /// what a widget asks for.
    BadCapability {
        name: String,
        text: String,
    },
    /// The index does not verify against the key this registry was pinned to.
    Unsigned(String),
    /// The bytes arrived and are not what the index said they would be.
    DigestMismatch {
        name: String,
    },
    /// The manifest that arrived describes a different widget from the entry
    /// that offered it.
    Disagrees {
        entry: String,
        manifest: String,
    },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Syntax(why) => write!(f, "{why}"),
            RegistryError::NotAnIndex => write!(
                f,
                "that is not a uniwidged registry index: it must start with \
                 `index = \"{INDEX_MAGIC}\"`"
            ),
            RegistryError::BadName(name) => {
                write!(f, "`{name}` is not a usable widget name")
            }
            RegistryError::BadVersion { name, why } => {
                write!(f, "the version of `{name}` is unusable: {why}")
            }
            RegistryError::NotHttps { name, url } => write!(
                f,
                "`{name}` points at `{url}`, which is not an https:// URL"
            ),
            RegistryError::BadDigest(name) => write!(
                f,
                "the digest of `{name}` is not sixty-four hexadecimal characters"
            ),
            RegistryError::Duplicate { name, version } => write!(
                f,
                "this index offers `{name} {version}` twice, and nothing can \
                 choose between them"
            ),
            RegistryError::BadCapability { name, text } => write!(
                f,
                "`{name}` lists `{text}`, which is not a capability this host \
                 knows — so nobody could be shown what it means"
            ),
            RegistryError::Unsigned(why) => write!(
                f,
                "this index does not verify against the key this registry was \
                 added with: {why}"
            ),
            RegistryError::DigestMismatch { name } => write!(
                f,
                "`{name}` is not what the index said it would be. The index is \
                 signed, so either it is out of date or something replaced the \
                 file it points at — either way this is not the widget you were \
                 offered"
            ),
            RegistryError::Disagrees { entry, manifest } => write!(
                f,
                "the index offered `{entry}` and the manifest that arrived says \
                 `{manifest}`; what a listing shows must be what a runtime enforces"
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

impl Index {
    /// Reads an index, refusing everything it cannot make sense of.
    ///
    /// Strict for the same reason [`Manifest::parse`] is: what this returns is
    /// shown to a person who then decides something, and an entry that was
    /// half understood is an entry that was mis-shown.
    pub fn parse(text: &str) -> Result<Self, RegistryError> {
        let raw: RawIndex =
            toml::from_str(text).map_err(|e| RegistryError::Syntax(e.message().to_string()))?;
        if raw.index != INDEX_MAGIC {
            return Err(RegistryError::NotAnIndex);
        }

        let mut entries = Vec::with_capacity(raw.widgets.len());
        for widget in raw.widgets {
            let entry = Entry::from_raw(widget)?;
            if entries
                .iter()
                .any(|other: &Entry| other.name == entry.name && other.version == entry.version)
            {
                return Err(RegistryError::Duplicate {
                    name: entry.name,
                    version: entry.version.to_string(),
                });
            }
            entries.push(entry);
        }
        Ok(Self { entries })
    }

    /// Reads an index and checks that its publisher signed *this* text.
    ///
    /// The signature covers the bytes as they arrived, so this takes the text
    /// rather than the parsed index: a signature over a re-serialisation would
    /// verify a document nobody signed.
    pub fn parse_signed(
        text: &str,
        signature: &SignatureFile,
        key: PublicKey,
    ) -> Result<Self, RegistryError> {
        // Before the parse, not after: an index that does not verify is not a
        // document to look inside.
        signature::verify_index(text, signature, key)
            .map_err(|why| RegistryError::Unsigned(why.to_string()))?;
        Self::parse(text)
    }

    /// The newest version of one widget, or the exact version asked for.
    ///
    /// "Newest" skips pre-releases unless one was asked for by name: somebody
    /// running `uniwidged add clock` wants the clock, not its release
    /// candidate.
    pub fn resolve(&self, name: &str, version: Option<&Version>) -> Option<&Entry> {
        match version {
            Some(wanted) => self
                .entries
                .iter()
                .find(|e| e.name == name && &e.version == wanted),
            None => self
                .entries
                .iter()
                .filter(|e| e.name == name && !e.version.is_pre_release())
                .max_by(|a, b| a.version.cmp(&b.version)),
        }
    }

    /// Every widget matching a term, newest version of each, in name order.
    ///
    /// An empty term lists everything, which is what `uniwidged search` with no
    /// argument does — a registry nobody can browse is one nobody finds
    /// anything in.
    pub fn search(&self, term: &str) -> Vec<&Entry> {
        let term = term.to_lowercase();
        let mut names: Vec<&str> = self
            .entries
            .iter()
            .filter(|e| {
                term.is_empty()
                    || e.name.to_lowercase().contains(&term)
                    || e.description.to_lowercase().contains(&term)
            })
            .map(|e| e.name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .filter_map(|name| {
                self.resolve(name, None)
                    .or_else(|| newest_of_any(self, name))
            })
            .collect()
    }
}

/// The newest version of a widget that has only pre-releases.
///
/// `resolve` skips those on purpose; a search that hid such a widget entirely
/// would leave a person unable to see it exists at all.
fn newest_of_any<'a>(index: &'a Index, name: &str) -> Option<&'a Entry> {
    index
        .entries
        .iter()
        .filter(|e| e.name == name)
        .max_by(|a, b| a.version.cmp(&b.version))
}

impl Entry {
    fn from_raw(raw: RawWidget) -> Result<Self, RegistryError> {
        let name = raw.name;
        // The same rule the manifest applies, and for the same reason: the
        // name becomes a directory here too, in the widget store the download
        // is written to.
        if !crate::manifest::is_usable_name(&name) {
            return Err(RegistryError::BadName(name));
        }
        let version = Version::parse(&raw.version).map_err(|why| RegistryError::BadVersion {
            name: name.clone(),
            why: why.to_string(),
        })?;

        for url in [
            Some(&raw.component),
            Some(&raw.manifest),
            raw.signature.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            // Read by the *same* function the `net` capability reads a URL
            // with, so a registry URL cannot be understood one way here and
            // another way when it is fetched.
            if host_of(url).is_none() {
                return Err(RegistryError::NotHttps {
                    name,
                    url: url.clone(),
                });
            }
        }

        for text in &raw.capabilities {
            if crate::manifest::parse_capability(text).is_err() {
                return Err(RegistryError::BadCapability {
                    name,
                    text: text.clone(),
                });
            }
        }

        let digest = signature::hex32(&raw.digest).ok_or(RegistryError::BadDigest(name.clone()))?;

        Ok(Self {
            name,
            version,
            description: raw.description,
            capabilities: raw.capabilities,
            component: raw.component,
            manifest: raw.manifest,
            signature: raw.signature,
            digest,
        })
    }

    /// `name 1.2.3`, for a listing.
    pub fn title(&self) -> String {
        format!("{} {}", self.name, self.version)
    }

    /// What this entry says the widget will ask for, in the words `install`
    /// uses.
    ///
    /// Parsed rather than printed: the strings come off the network, and a
    /// listing that echoes them is one a publisher can write a sentence into.
    pub fn consent_summary(&self) -> Vec<String> {
        let capabilities: Vec<Capability> = self
            .capabilities
            .iter()
            .filter_map(|text| crate::manifest::parse_capability(text).ok())
            .collect();
        if capabilities.is_empty() {
            return vec!["asks for nothing at all".to_string()];
        }
        capabilities.iter().map(crate::manifest::describe).collect()
    }

    /// Are these the bytes this entry promised?
    ///
    /// Three checks, and the third is the one that is easy to leave out: the
    /// digest matches, the manifest parses, and the manifest **agrees with the
    /// entry** about what this widget is and what it asks for. Without the
    /// third, an index could show a person `clock, formats dates` and hand
    /// them a component whose manifest asks for their keychain — every
    /// signature valid, every hash correct, and the consent screen a lie.
    pub fn verify(&self, component: &[u8], manifest_text: &str) -> Result<Manifest, RegistryError> {
        if signature::digest(component, manifest_text) != self.digest {
            return Err(RegistryError::DigestMismatch {
                name: self.name.clone(),
            });
        }
        let manifest =
            Manifest::parse(manifest_text).map_err(|why| RegistryError::Syntax(why.to_string()))?;

        let disagrees = |manifest_side: String| RegistryError::Disagrees {
            entry: format!(
                "{} {} ({})",
                self.name,
                self.version,
                self.capabilities.join(", ")
            ),
            manifest: manifest_side,
        };
        if manifest.name != self.name || manifest.version != self.version.to_string() {
            return Err(disagrees(format!("{} {}", manifest.name, manifest.version)));
        }
        let declared: Vec<String> = manifest
            .capabilities
            .iter()
            .map(crate::manifest::capability_text)
            .collect();
        let listed: Vec<String> = self
            .capabilities
            .iter()
            .filter_map(|text| crate::manifest::parse_capability(text).ok())
            .map(|c| crate::manifest::capability_text(&c))
            .collect();
        if declared != listed {
            return Err(disagrees(format!(
                "{} {} ({})",
                manifest.name,
                manifest.version,
                declared.join(", ")
            )));
        }
        Ok(manifest)
    }
}

/// Fetching, under the same rules a widget's own requests are fetched under.
///
/// The transport is [`crate::net::Http`] and the permission is a [`Policy`]
/// granting exactly one host: the registry's. That is not decoration. It means
/// a registry index answering `302 Location: https://elsewhere.test/` is
/// refused by the code that already refuses it for widgets, rather than by a
/// second rule somebody would have had to remember to write — and a download
/// that walks off the host a person pinned a key to is precisely the thing the
/// index signature would then be checked against the wrong publisher for.
///
/// Everything here runs on the thread that called it. It is a person waiting
/// at a command line, not the desktop loop.
pub struct Fetcher {
    http: crate::net::Http,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Self {
        Self::with_http(crate::net::Http::new())
    }

    /// A fetcher over a client the caller prepared — a corporate proxy's
    /// certificate store, or a test's own server. See
    /// [`crate::net::Http::with_agent`].
    pub fn with_http(http: crate::net::Http) -> Self {
        Self {
            http: http.with_max_body(MAX_COMPONENT_BYTES),
        }
    }

    /// A fetcher whose **only** trust anchor is the certificate in this PEM
    /// file.
    ///
    /// For a registry inside an organisation, served under a certificate from
    /// an authority the public trust stores have never heard of. It replaces
    /// the default roots rather than adding to them, and that is the honest
    /// behaviour to offer: adding would mean carrying a copy of the Mozilla
    /// bundle in this binary and quietly deciding when to update it, and an
    /// internal deployment that pins its own authority usually means to talk
    /// to its own registry and nothing else.
    ///
    /// It is also what makes an end-to-end test possible at all: the test
    /// starts a real TLS server and hands its certificate to a real client.
    pub fn trusting(pem: &[u8]) -> Result<Self, String> {
        let certificate = ureq::tls::Certificate::from_pem(pem)
            .map_err(|e| format!("that is not a PEM certificate: {e}"))?;
        let agent = ureq::Agent::config_builder()
            // As `Http::new`, and for the same reason: redirects are followed
            // by our own code so that every hop is checked again.
            .max_redirects(0)
            .timeout_global(Some(crate::net::TIMEOUT))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::Specific(std::sync::Arc::new(vec![
                        certificate,
                    ])))
                    .build(),
            )
            .build()
            .into();
        Ok(Self::with_http(crate::net::Http::with_agent(agent)))
    }

    /// One URL, under a grant for its own host and nothing else.
    pub fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let host = host_of(url).ok_or_else(|| format!("`{url}` is not an https:// URL"))?;
        let policy = Policy::from_granted([Capability::Net(host)]);
        match self.http.get(url, &policy) {
            crate::tasks::TaskResult::Http(response) if response.status == 200 => Ok(response.body),
            crate::tasks::TaskResult::Http(response) => Err(format!(
                "{url} answered {} rather than 200",
                response.status
            )),
            crate::tasks::TaskResult::Failed(why) => Err(format!("{url}: {why}")),
            _ => Err(format!("{url}: the transport answered something else")),
        }
    }

    /// The same, as text, refusing bytes that are not UTF-8.
    fn text(&self, url: &str, cap: usize) -> Result<String, String> {
        let bytes = self.get(url)?;
        if bytes.len() > cap {
            return Err(format!("{url}: larger than {cap} bytes"));
        }
        String::from_utf8(bytes).map_err(|_| format!("{url}: not text"))
    }

    /// Fetches an index and checks it against the key this registry was pinned
    /// to.
    ///
    /// Returns the text as well as the parsed index, because the text is what
    /// gets cached: a cache holding a re-serialisation would hold something the
    /// signature beside it does not cover.
    ///
    /// The two ways this fails are [`FetchError`]'s two variants, and they are
    /// different facts — see there.
    pub fn index(&self, url: &str, key: PublicKey) -> Result<(Index, String, String), FetchError> {
        let text = self
            .text(url, MAX_INDEX_BYTES)
            .map_err(FetchError::Transport)?;
        let signature_url = format!("{url}.sig");
        let signature_text = self
            .text(&signature_url, 4096)
            .map_err(FetchError::Transport)?;
        let signature = SignatureFile::parse(&signature_text)
            .map_err(|why| FetchError::Refused(RegistryError::Unsigned(why.to_string())))?;
        let index = Index::parse_signed(&text, &signature, key).map_err(FetchError::Refused)?;
        Ok((index, text, signature_text))
    }

    /// Fetches one widget: its component, its manifest, and its signature when
    /// the entry names one.
    ///
    /// The digest is checked here, before anything is written anywhere — the
    /// caller receives bytes that are already known to be what the index
    /// offered, or an error.
    pub fn widget(&self, entry: &Entry) -> Result<Download, String> {
        let component = self.get(&entry.component)?;
        let manifest = self.text(&entry.manifest, MAX_INDEX_BYTES)?;
        let signature = match &entry.signature {
            Some(url) => Some(self.text(url, 4096)?),
            None => None,
        };
        let parsed = entry
            .verify(&component, &manifest)
            .map_err(|why| why.to_string())?;
        Ok(Download {
            component,
            manifest,
            signature,
            parsed,
        })
    }
}

/// Why an index did not arrive.
///
/// The distinction is the whole reason this is not a `String`. A registry that
/// is unreachable is an ordinary fact about a network, and a command that
/// already has a cached index should use it and carry on. A registry that
/// answers with a list its publisher did not sign is **not** ordinary: falling
/// back to the cache there would turn somebody actively serving a forged index
/// into a line of grey text nobody reads. One is a warning, the other stops
/// what you were doing.
#[derive(Debug)]
pub enum FetchError {
    /// It did not arrive: no network, a 404, a timeout, a body too large.
    Transport(String),
    /// It arrived and was refused.
    Refused(RegistryError),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Transport(why) => write!(f, "{why}"),
            FetchError::Refused(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// A widget that arrived and is what the index said it was.
///
/// Not yet installed, and deliberately not: verifying a digest says the bytes
/// match the list, and says nothing about whether their author is somebody
/// this machine trusts. That question is `install`'s, and it is asked again.
#[derive(Debug)]
pub struct Download {
    pub component: Vec<u8>,
    pub manifest: String,
    /// The `.sig` file's text, when the entry offered one.
    pub signature: Option<String>,
    /// The manifest, parsed — and already checked to agree with the entry.
    pub parsed: Manifest,
}

#[derive(Debug, serde::Deserialize)]
struct RawIndex {
    #[serde(default)]
    index: String,
    #[serde(default, rename = "widget")]
    widgets: Vec<RawWidget>,
}

#[derive(Debug, serde::Deserialize)]
struct RawWidget {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    capabilities: Vec<String>,
    component: String,
    manifest: String,
    #[serde(default)]
    signature: Option<String>,
    digest: String,
}

/// Writes an index, which is how the tests and `uniwidged registry publish`
/// make one.
///
/// The serialiser is here rather than in the CLI so that the format has one
/// definition: a publisher writing an index by hand and this writing one have
/// to produce the same document, and the parser above is the arbiter of both.
pub fn write_index(entries: &[Entry]) -> String {
    let mut text = format!("index = \"{INDEX_MAGIC}\"\n");
    for entry in entries {
        text.push_str("\n[[widget]]\n");
        text.push_str(&format!("name = {}\n", quoted(&entry.name)));
        text.push_str(&format!(
            "version = {}\n",
            quoted(&entry.version.to_string())
        ));
        if !entry.description.is_empty() {
            text.push_str(&format!("description = {}\n", quoted(&entry.description)));
        }
        let capabilities: Vec<String> = entry.capabilities.iter().map(|c| quoted(c)).collect();
        text.push_str(&format!("capabilities = [{}]\n", capabilities.join(", ")));
        text.push_str(&format!("component = {}\n", quoted(&entry.component)));
        text.push_str(&format!("manifest = {}\n", quoted(&entry.manifest)));
        if let Some(signature) = &entry.signature {
            text.push_str(&format!("signature = {}\n", quoted(signature)));
        }
        text.push_str(&format!(
            "digest = {}\n",
            quoted(&signature::to_hex(&entry.digest))
        ));
    }
    text
}

/// A TOML basic string.
///
/// Small enough to write, and written rather than pulled in: the values here
/// are names, versions and URLs, all of which the parsers above have already
/// restricted to characters with no meaning inside quotes. The escaping is
/// still done, because "already restricted" is a property somebody could
/// change in another file.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests;
