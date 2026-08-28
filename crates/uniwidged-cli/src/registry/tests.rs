//! The parts of the registry commands that are decisions rather than files.
//!
//! Everything here is pure: what a registries file may say, how `name@version`
//! is read, and which registry an update takes a widget from. The half that
//! needs a server is `tests/registry.rs`, which starts one.

use super::*;

fn key() -> PublicKey {
    PublicKey::from_hex(&"ab".repeat(32)).unwrap()
}

#[test]
fn a_registries_file_reads_back_what_was_written() {
    let registries = vec![
        Registry {
            name: "example".into(),
            url: "https://widgets.example/index.toml".into(),
            key: key(),
        },
        Registry {
            name: "work".into(),
            url: "https://widgets.internal/index.toml".into(),
            key: key(),
        },
    ];
    let text = {
        // The writer's body, without touching the real configuration
        // directory: what is under test is the round trip, not the path.
        let mut text = String::new();
        for registry in &registries {
            text.push_str(&format!(
                "\n[[registry]]\nname = \"{}\"\nurl = \"{}\"\nkey = \"{}\"\n",
                registry.name,
                registry.url,
                registry.key.to_hex()
            ));
        }
        text
    };
    assert_eq!(parse_registries(&text).unwrap(), registries);
}

#[test]
fn a_registry_that_is_not_https_is_refused_where_it_is_read() {
    // `registries.toml` is meant to be edited by hand, so the file is a place
    // somebody can write anything — including a plain-HTTP URL, over which an
    // index and its signature would both be whatever the network said.
    let text = "[[registry]]\nname = \"x\"\nurl = \"http://widgets.example/index.toml\"\n\
                key = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n";
    let why = parse_registries(text).unwrap_err();
    assert!(why.contains("https"), "{why}");
}

#[test]
fn a_registry_name_becomes_a_file_so_it_is_held_to_the_same_rule_a_widget_is() {
    assert!(is_usable_registry_name("example"));
    assert!(is_usable_registry_name("widgets-example-com"));
    for bad in ["", "../etc", "Example", "9lives", "trailing-", "with space"] {
        assert!(!is_usable_registry_name(bad), "{bad:?}");
    }
    let text = "[[registry]]\nname = \"../../etc\"\nurl = \"https://x.example/i.toml\"\n\
                key = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n";
    assert!(parse_registries(text).is_err());
}

#[test]
fn a_key_that_is_not_a_key_stops_the_file_rather_than_that_line() {
    // Skipping the line would leave a registry in the list with no key, and
    // "no key" must never come to mean "any key".
    let text = "[[registry]]\nname = \"x\"\nurl = \"https://x.example/i.toml\"\nkey = \"nope\"\n";
    assert!(parse_registries(text).is_err());
}

#[test]
fn an_empty_file_reads_as_no_registries_rather_than_an_error() {
    assert_eq!(parse_registries("").unwrap(), Vec::new());
}

#[test]
fn a_name_may_carry_the_version_somebody_wants() {
    assert_eq!(split_version("clock").unwrap(), ("clock".into(), None));
    assert_eq!(
        split_version("clock@1.2.3").unwrap(),
        ("clock".into(), Some(Version::new(1, 2, 3)))
    );
    // A version that is not one is refused here, where the person is looking,
    // rather than by finding nothing in the index.
    let why = split_version("clock@banana").unwrap_err();
    assert!(why.contains("major.minor.patch"), "{why}");
}

#[test]
fn an_update_takes_the_newest_offer_across_registries() {
    use uniwidged_runtime::registry::write_index;

    let entry = |name: &str, version: &str| -> Entry {
        let manifest = format!("name = \"{name}\"\nversion = \"{version}\"\n");
        Entry {
            name: name.into(),
            version: Version::parse(version).unwrap(),
            description: String::new(),
            capabilities: Vec::new(),
            component: format!("https://a.example/{name}-{version}.wasm"),
            manifest: format!("https://a.example/{name}-{version}.toml"),
            signature: None,
            digest: uniwidged_runtime::signature::digest(b"component", &manifest),
        }
    };
    let index = |entries: &[Entry]| Index::parse(&write_index(entries)).unwrap();
    let registry = |name: &str| Registry {
        name: name.into(),
        url: format!("https://{name}.example/index.toml"),
        key: key(),
    };

    let indexes = vec![
        (registry("one"), index(&[entry("clock", "1.0.0")])),
        (registry("two"), index(&[entry("clock", "1.4.0")])),
    ];
    let (from, found) = newest(&indexes, "clock").expect("both offer it");
    assert_eq!(from.name, "two");
    assert_eq!(found.version.to_string(), "1.4.0");
    assert!(newest(&indexes, "nothing-here").is_none());
}

#[test]
fn an_age_is_reported_in_the_words_somebody_would_use() {
    assert_eq!(roughly(Duration::from_secs(10)), "less than a minute");
    assert_eq!(roughly(Duration::from_secs(300)), "5 minutes");
    assert_eq!(roughly(Duration::from_secs(4 * 3600)), "4 hours");
}
