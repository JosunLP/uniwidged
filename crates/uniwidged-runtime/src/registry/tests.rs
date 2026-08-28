//! What a registry cannot make this machine do.
//!
//! The pure half first — an index that is refused, and a download that does
//! not match the entry that offered it — because those are the properties the
//! whole thing rests on. Then the real half: a TLS server serving an index, a
//! component and two signatures, so that "a widget can be installed from a
//! registry" is something observed rather than read off the source.

use super::*;
use crate::signature::SecretKey;

/// A component and a manifest, as a publisher would have them.
fn widget(name: &str, version: &str, capabilities: &[&str]) -> (Vec<u8>, String) {
    let component = format!("\0asm-not-really-{name}-{version}").into_bytes();
    let capabilities: Vec<String> = capabilities.iter().map(|c| format!("\"{c}\"")).collect();
    let manifest = format!(
        "name = \"{name}\"\nversion = \"{version}\"\n\
         description = \"a widget for a test\"\n\
         capabilities = [{}]\n",
        capabilities.join(", ")
    );
    (component, manifest)
}

/// An entry offering that widget, with the digest filled in correctly.
fn entry(name: &str, version: &str, capabilities: &[&str]) -> Entry {
    let (component, manifest) = widget(name, version, capabilities);
    Entry {
        name: name.to_string(),
        version: Version::parse(version).unwrap(),
        description: "a widget for a test".to_string(),
        capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
        component: format!("https://widgets.example/{name}-{version}.wasm"),
        manifest: format!("https://widgets.example/{name}-{version}.toml"),
        signature: Some(format!("https://widgets.example/{name}-{version}.wasm.sig")),
        digest: signature::digest(&component, &manifest),
    }
}

fn index_of(entries: &[Entry]) -> Index {
    Index::parse(&write_index(entries)).expect("an index this wrote should parse")
}

#[test]
fn an_index_round_trips_through_its_own_writer() {
    // The format has one definition, and this is what says so: a publisher
    // writing an index by hand and `registry publish` writing one have to
    // produce documents the same parser accepts.
    let entries = vec![
        entry("clock", "1.0.0", &["timer", "locale"]),
        entry("counter", "0.2.0", &["fs:state"]),
    ];
    assert_eq!(index_of(&entries).entries, entries);
}

#[test]
fn a_document_that_is_not_an_index_is_refused_by_its_first_line() {
    // The commonest way this goes wrong is not an attack: it is a proxy or a
    // hosting provider serving an HTML error page with a 200.
    assert!(matches!(
        Index::parse("<html>404</html>"),
        Err(RegistryError::Syntax(_))
    ));
    assert_eq!(
        Index::parse("index = \"something-else\""),
        Err(RegistryError::NotAnIndex)
    );
    assert_eq!(Index::parse(""), Err(RegistryError::NotAnIndex));
    assert!(
        RegistryError::NotAnIndex.to_string().contains(INDEX_MAGIC),
        "the message should say what the first line has to be"
    );
}

#[test]
fn an_entry_that_is_not_https_is_refused_before_anything_is_fetched() {
    let mut broken = entry("clock", "1.0.0", &["timer"]);
    broken.component = "http://widgets.example/clock.wasm".into();
    assert!(matches!(
        Index::parse(&write_index(&[broken])),
        Err(RegistryError::NotHttps { .. })
    ));

    // Including the signature, which is the URL somebody would least expect to
    // be checked and the one that decides whether a widget looks signed.
    let mut broken = entry("clock", "1.0.0", &["timer"]);
    broken.signature = Some("file:///etc/passwd".into());
    assert!(matches!(
        Index::parse(&write_index(&[broken])),
        Err(RegistryError::NotHttps { .. })
    ));
}

#[test]
fn an_entry_may_not_name_a_widget_something_the_host_would_refuse() {
    // The name becomes a directory in the widget store, so `../../` in one is
    // a download written wherever it likes.
    let text = write_index(&[entry("clock", "1.0.0", &["timer"])]).replace("clock", "../../etc");
    assert!(matches!(
        Index::parse(&text),
        Err(RegistryError::BadName(_))
    ));
}

#[test]
fn an_entry_may_not_list_a_capability_nobody_could_be_shown() {
    // The listing is what a person reads before deciding. A capability string
    // this host cannot parse is one it cannot explain, and printing it
    // verbatim would let a publisher write their own sentence into a consent
    // screen.
    let mut lying = entry("clock", "1.0.0", &["timer"]);
    lying.capabilities = vec!["everything, honestly".to_string()];
    assert!(matches!(
        Index::parse(&write_index(&[lying])),
        Err(RegistryError::BadCapability { .. })
    ));
}

#[test]
fn one_name_and_one_version_may_appear_only_once() {
    let entries = [
        entry("clock", "1.0.0", &["timer"]),
        entry("clock", "1.0.0", &["secrets"]),
    ];
    assert!(matches!(
        Index::parse(&write_index(&entries)),
        Err(RegistryError::Duplicate { .. })
    ));
}

#[test]
fn a_digest_that_is_not_thirty_two_bytes_is_refused() {
    let text = write_index(&[entry("clock", "1.0.0", &["timer"])]);
    let text = text
        .lines()
        .map(|line| {
            if line.starts_with("digest") {
                "digest = \"abcd\"".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(matches!(
        Index::parse(&text),
        Err(RegistryError::BadDigest(_))
    ));
}

#[test]
fn resolving_finds_the_newest_release_rather_than_the_last_line() {
    let index = index_of(&[
        entry("clock", "1.9.0", &["timer"]),
        entry("clock", "1.10.0", &["timer"]),
        entry("clock", "1.2.0", &["timer"]),
    ]);
    assert_eq!(
        index.resolve("clock", None).unwrap().version,
        Version::parse("1.10.0").unwrap()
    );
    // And an exact version, when one is asked for.
    assert_eq!(
        index
            .resolve("clock", Some(&Version::parse("1.2.0").unwrap()))
            .unwrap()
            .version
            .to_string(),
        "1.2.0"
    );
    assert!(
        index
            .resolve("clock", Some(&Version::parse("3.0.0").unwrap()))
            .is_none()
    );
    assert!(index.resolve("nothing-here", None).is_none());
}

#[test]
fn a_pre_release_is_not_what_somebody_gets_by_asking_for_the_widget() {
    let index = index_of(&[
        entry("clock", "1.0.0", &["timer"]),
        entry("clock", "2.0.0-rc.1", &["timer"]),
    ]);
    assert_eq!(
        index.resolve("clock", None).unwrap().version.to_string(),
        "1.0.0"
    );
    // But it can be asked for by name.
    assert_eq!(
        index
            .resolve("clock", Some(&Version::parse("2.0.0-rc.1").unwrap()))
            .unwrap()
            .version
            .to_string(),
        "2.0.0-rc.1"
    );
}

#[test]
fn searching_lists_each_widget_once_at_its_newest() {
    let index = index_of(&[
        entry("clock", "1.0.0", &["timer"]),
        entry("clock", "1.1.0", &["timer"]),
        entry("weather", "0.1.0", &["net:api.example.com"]),
    ]);
    let all: Vec<String> = index.search("").iter().map(|e| e.title()).collect();
    assert_eq!(all, ["clock 1.1.0", "weather 0.1.0"]);
    // By name, and by description — `a widget for a test` is every entry's.
    assert_eq!(index.search("weath").len(), 1);
    assert_eq!(index.search("for a test").len(), 2);
    assert!(index.search("nothing like this").is_empty());
}

#[test]
fn a_widget_with_only_pre_releases_is_still_findable() {
    // `resolve` skips pre-releases; a search that did the same would leave
    // somebody unable to see the widget exists at all.
    let index = index_of(&[entry("clock", "0.1.0-alpha.1", &["timer"])]);
    assert_eq!(index.search("clock").len(), 1);
}

#[test]
fn bytes_that_are_not_what_the_index_promised_are_refused() {
    let offered = entry("clock", "1.0.0", &["timer", "locale"]);
    let (component, manifest) = widget("clock", "1.0.0", &["timer", "locale"]);
    assert!(offered.verify(&component, &manifest).is_ok());

    // One byte of the component.
    let mut tampered = component.clone();
    tampered.push(0);
    assert!(matches!(
        offered.verify(&tampered, &manifest),
        Err(RegistryError::DigestMismatch { .. })
    ));

    // And the manifest, which is the half a hash over the `.wasm` alone would
    // have missed — the whole reason `signature::digest` covers both.
    let widened = manifest.replace(
        "capabilities = [\"timer\", \"locale\"]",
        "capabilities = [\"timer\", \"locale\", \"secrets\"]",
    );
    assert!(matches!(
        offered.verify(&component, &widened),
        Err(RegistryError::DigestMismatch { .. })
    ));
}

#[test]
fn a_manifest_that_disagrees_with_the_entry_is_refused_even_when_the_digest_matches() {
    // The listing said `timer`; the manifest asks for the keychain. Every hash
    // correct, every signature valid, and the consent screen a lie — so the
    // entry and the manifest have to agree before either is shown to anybody.
    let (component, manifest) = widget("clock", "1.0.0", &["timer", "secrets"]);
    let honest_digest = signature::digest(&component, &manifest);
    let lying = Entry {
        capabilities: vec!["timer".to_string()],
        digest: honest_digest,
        ..entry("clock", "1.0.0", &["timer"])
    };
    let why = lying.verify(&component, &manifest).unwrap_err();
    assert!(matches!(why, RegistryError::Disagrees { .. }), "{why}");
    assert!(why.to_string().contains("secrets"), "{why}");

    // The same for a name, which is what decides where a widget's data and its
    // secrets live.
    let (component, manifest) = widget("counter", "1.0.0", &["timer"]);
    let lying = Entry {
        digest: signature::digest(&component, &manifest),
        ..entry("clock", "1.0.0", &["timer"])
    };
    assert!(matches!(
        lying.verify(&component, &manifest),
        Err(RegistryError::Disagrees { .. })
    ));
}

#[test]
fn a_listing_describes_a_widget_in_the_words_a_consent_screen_uses() {
    let weather = entry("weather", "1.0.0", &["timer", "net:api.example.com"]);
    let summary = weather.consent_summary();
    assert_eq!(summary.len(), 2);
    assert!(
        summary[1].contains("talk to api.example.com over HTTPS"),
        "{summary:?}"
    );
    assert_eq!(
        entry("bare", "1.0.0", &[]).consent_summary(),
        ["asks for nothing at all"]
    );
}

// ---------------------------------------------------------------------------
// The signature over the list, which is a different signature from the one
// over a widget.
// ---------------------------------------------------------------------------

#[test]
fn an_index_is_read_only_when_its_publisher_signed_it() {
    let key = SecretKey::generate().unwrap();
    let text = write_index(&[entry("clock", "1.0.0", &["timer"])]);
    let signed = SignatureFile {
        signature: key.sign_index(&text),
        key: key.public(),
    };
    assert!(Index::parse_signed(&text, &signed, key.public()).is_ok());

    // One byte of the list, which is the whole point: an entry added, a URL
    // changed, a name pointed at somebody else's bytes.
    let edited = text.replace("clock", "cl0ck");
    assert!(matches!(
        Index::parse_signed(&edited, &signed, key.public()),
        Err(RegistryError::Unsigned(_))
    ));

    // Signed correctly, by somebody else. The key is pinned to the registry,
    // so a valid signature by the wrong publisher is no signature at all.
    let other = SecretKey::generate().unwrap();
    let theirs = SignatureFile {
        signature: other.sign_index(&text),
        key: other.public(),
    };
    assert!(matches!(
        Index::parse_signed(&text, &theirs, key.public()),
        Err(RegistryError::Unsigned(_))
    ));
}

#[test]
fn a_widget_signature_cannot_be_replayed_as_an_index_signature() {
    // What the domain separation in `signature` is for, checked from the side
    // that would suffer: a registry publisher's key and a widget author's key
    // make different claims, and neither may be made to make the other's.
    let key = SecretKey::generate().unwrap();
    let text = write_index(&[entry("clock", "1.0.0", &["timer"])]);
    // Signed as though the index text were a widget's manifest.
    let as_a_widget = SignatureFile {
        signature: key.sign(&[], &text),
        key: key.public(),
    };
    assert!(matches!(
        Index::parse_signed(&text, &as_a_widget, key.public()),
        Err(RegistryError::Unsigned(_))
    ));
}

// ---------------------------------------------------------------------------
// Against a real server
// ---------------------------------------------------------------------------

use crate::test_server::TestServer;

/// A fetcher that trusts only the test server's certificate.
fn fetcher(server: &TestServer) -> Fetcher {
    let agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::Specific(std::sync::Arc::new(vec![
                    ureq::tls::Certificate::from_der(server.certificate_der()).to_owned(),
                ])))
                .build(),
        )
        .build()
        .into();
    Fetcher::with_http(crate::net::Http::with_agent(agent))
}

/// A server serving one signed index and the widget it lists.
///
/// Returns the publisher's key, because pinning it is what the client does.
fn registry_on(server: &TestServer) -> (crate::signature::PublicKey, Entry, String) {
    let publisher = SecretKey::generate().unwrap();
    let author = SecretKey::generate().unwrap();
    let (component, manifest) = widget("clock", "1.0.0", &["timer", "locale"]);
    let entry = Entry {
        component: server.url("/clock-1.0.0.wasm"),
        manifest: server.url("/clock-1.0.0.toml"),
        signature: Some(server.url("/clock-1.0.0.wasm.sig")),
        digest: signature::digest(&component, &manifest),
        ..entry("clock", "1.0.0", &["timer", "locale"])
    };
    let index = write_index(std::slice::from_ref(&entry));
    let index_signature = SignatureFile {
        signature: publisher.sign_index(&index),
        key: publisher.public(),
    };
    let widget_signature = SignatureFile {
        signature: author.sign(&component, &manifest),
        key: author.public(),
    };
    server.serve("/index.toml", index.clone());
    server.serve("/index.toml.sig", index_signature.to_text());
    server.serve("/clock-1.0.0.wasm", component);
    server.serve("/clock-1.0.0.toml", manifest);
    server.serve("/clock-1.0.0.wasm.sig", widget_signature.to_text());
    (publisher.public(), entry, index)
}

#[test]
fn a_widget_can_be_fetched_from_a_registry_that_really_is_served_over_https() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let (key, offered, _) = registry_on(&server);
    let fetcher = fetcher(&server);

    let (index, text, _) = fetcher
        .index(&server.url("/index.toml"), key)
        .expect("the index should be fetched and verified");
    assert_eq!(index.entries.len(), 1);
    // The cached text is what was served, byte for byte — a cache holding a
    // re-serialisation would hold something its signature does not cover.
    assert!(text.starts_with(&format!("index = \"{INDEX_MAGIC}\"")));

    let entry = index.resolve("clock", None).expect("the clock is listed");
    assert_eq!(entry, &offered);
    let download = fetcher.widget(entry).expect("the widget should arrive");
    assert_eq!(download.parsed.name, "clock");
    assert!(download.signature.is_some());
}

#[test]
fn an_index_whose_signature_does_not_check_out_stops_everything() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let (key, _, index) = registry_on(&server);
    // Somebody on the path edits the list. The signature beside it is the one
    // the publisher made for the list they wrote.
    let tampered = index.replace("a widget for a test", "a widget for a tost");
    server.serve("/index.toml", tampered);

    let why = fetcher(&server)
        .index(&server.url("/index.toml"), key)
        .unwrap_err();
    // A refusal rather than a transport failure, which is the distinction a
    // caller with a cache acts on: a registry that is down may be worked
    // around, a registry serving a forged list may not.
    assert!(
        matches!(why, FetchError::Refused(_)),
        "{why} should be a refusal"
    );
    assert!(why.to_string().contains("does not verify"), "{why}");
}

#[test]
fn a_component_swapped_after_the_index_was_signed_is_refused() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let (key, _, _) = registry_on(&server);
    // The list is untouched and still verifies; the file it points at is not
    // the file it was pointing at.
    server.serve(
        "/clock-1.0.0.wasm",
        b"\0asm something else entirely".to_vec(),
    );

    let fetcher = fetcher(&server);
    let (index, _, _) = fetcher.index(&server.url("/index.toml"), key).unwrap();
    let why = fetcher
        .widget(index.resolve("clock", None).unwrap())
        .unwrap_err();
    assert!(why.contains("not what the index said"), "{why}");
}

#[test]
fn a_registry_that_redirects_off_its_own_host_is_refused() {
    // A registry is fetched under a grant for exactly its own host, by the
    // same code that refuses this for a widget's own requests. Otherwise the
    // index a key was pinned to could be served by somebody else entirely.
    let Some(server) = TestServer::start() else {
        return;
    };
    let (key, _, _) = registry_on(&server);
    let why = fetcher(&server)
        .index(&server.url("/redirect-away"), key)
        .unwrap_err();
    assert!(matches!(why, FetchError::Transport(_)), "{why}");
    assert!(why.to_string().contains("not granted"), "{why}");
}

#[test]
fn a_missing_index_says_so_rather_than_appearing_empty() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let (key, _, _) = registry_on(&server);
    let why = fetcher(&server)
        .index(&server.url("/missing"), key)
        .unwrap_err();
    assert!(why.to_string().contains("404"), "{why}");
}
