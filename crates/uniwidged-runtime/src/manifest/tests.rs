//! A manifest is what a user approves, so the parser is strict and most of
//! these are about what it refuses.

use super::*;

const WEATHER: &str = r#"
name = "weather"
version = "0.2.0"
description = "Today's forecast for one place"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
memory-mb = 48
"#;

#[test]
fn a_complete_manifest_parses_into_what_it_said() {
    let manifest = Manifest::parse(WEATHER).expect("valid");
    assert_eq!(manifest.name, "weather");
    assert_eq!(manifest.version, "0.2.0");
    assert_eq!(
        manifest.capabilities,
        vec![
            Capability::Timer,
            Capability::Net("api.open-meteo.com".into()),
            Capability::Fs("cache".into()),
        ]
    );
    assert_eq!(manifest.limits.memory_bytes, 48 * 1024 * 1024);
}

#[test]
fn a_widget_that_declares_nothing_is_perfectly_valid() {
    // The most common case for a widget that only shows what the host tells
    // it. It should need no ceremony at all.
    let manifest = Manifest::parse(r#"name = "clock""#).expect("valid");
    assert!(manifest.capabilities.is_empty());
    assert_eq!(manifest.limits.memory_bytes, 32 * 1024 * 1024);
}

#[test]
fn an_unknown_capability_is_refused_rather_than_ignored() {
    // A manifest a user approved has to mean exactly what it said. Silently
    // dropping a line they read is the one outcome worth refusing outright —
    // and a typo in a real capability would otherwise ship as "no permission".
    let manifest = Manifest::parse(
        r#"
name = "x"
capabilities = ["tiemr"]
"#,
    );
    assert_eq!(
        manifest,
        Err(ManifestError::UnknownCapability("tiemr".into()))
    );
}

#[test]
fn network_access_must_name_a_host() {
    // "this widget uses the network" is not something anybody can consent to.
    for text in [
        "net:",
        "net:*",
        "net:*.",
        "net:https://example.com",
        "net:.com",
        "net:a..b",
    ] {
        let manifest = Manifest::parse(&format!("name = \"x\"\ncapabilities = [\"{text}\"]"));
        assert!(
            manifest.is_err(),
            "{text} should not parse, got {manifest:?}"
        );
    }
}

#[test]
fn a_wildcard_names_a_domain_and_not_the_internet() {
    let manifest = Manifest::parse(
        r#"
name = "maps"
capabilities = ["net:*.tiles.example.com"]
"#,
    )
    .expect("valid");
    assert_eq!(
        manifest.capabilities,
        vec![Capability::NetWildcard("tiles.example.com".into())]
    );
}

#[test]
fn a_host_is_lower_cased_so_two_spellings_are_one_grant() {
    let manifest = Manifest::parse(
        r#"
name = "x"
capabilities = ["net:API.Example.COM"]
"#,
    )
    .expect("valid");
    assert_eq!(
        manifest.capabilities,
        vec![Capability::Net("api.example.com".into())]
    );
}

#[test]
fn a_filesystem_capability_cannot_climb_out_at_declaration_time() {
    // The request path is checked too, but a manifest asking for `../..` is a
    // widget whose intent is visible before it ever runs.
    // Single-quoted TOML literals, so that a backslash reaches the parser as a
    // backslash: in a basic string `"a\b"` is a backspace, and the test would
    // silently be checking something else.
    for text in [
        "fs:",
        "fs:/etc",
        "fs:../secrets",
        "fs:cache/../..",
        r"fs:a\b",
        "fs:with\u{7f}control",
    ] {
        let manifest = Manifest::parse(&format!("name = 'x'\ncapabilities = ['{text}']"));
        assert!(manifest.is_err(), "{text:?} should not parse");
    }
}

#[test]
fn a_command_capability_names_a_command_not_a_path() {
    assert!(Manifest::parse("name = \"x\"\ncapabilities = [\"command:/bin/sh\"]").is_err());
    let ok =
        Manifest::parse("name = \"x\"\ncapabilities = [\"command:notify-send\"]").expect("valid");
    assert_eq!(
        ok.capabilities,
        vec![Capability::Command("notify-send".into())]
    );
}

#[test]
fn a_name_has_to_be_safe_as_a_path_and_as_a_keychain_namespace() {
    // It becomes both, and the two disagree about separators, case and length,
    // so what is allowed is the intersection rather than whatever works on the
    // developer's machine.
    for bad in [
        "",
        "Weather",
        "9lives",
        "-leading",
        "trailing-",
        "with space",
        "with/slash",
        "with\\backslash",
        "with.dot",
        "..",
        "with:colon",
    ] {
        let manifest = Manifest::parse(&format!("name = {bad:?}"));
        assert!(manifest.is_err(), "{bad:?} should not be a usable name");
    }
    for good in ["a", "weather", "system-load", "calendar2"] {
        assert!(
            Manifest::parse(&format!("name = {good:?}")).is_ok(),
            "{good:?} should be usable"
        );
    }
}

#[test]
fn a_name_longer_than_a_path_component_is_refused() {
    let long = "a".repeat(65);
    assert!(Manifest::parse(&format!("name = {long:?}")).is_err());
    let limit = "a".repeat(64);
    assert!(Manifest::parse(&format!("name = {limit:?}")).is_ok());
}

#[test]
fn an_outsized_memory_request_is_refused() {
    // Not a technical limit: a cap on what can be asked for without a
    // conversation. A widget wanting a gigabyte is doing something a widget
    // does not do.
    let manifest = Manifest::parse(&format!("name = \"x\"\nmemory-mb = {}", MAX_MEMORY_MB + 1));
    assert_eq!(
        manifest,
        Err(ManifestError::MemoryTooLarge(MAX_MEMORY_MB + 1))
    );
    assert!(Manifest::parse(&format!("name = \"x\"\nmemory-mb = {MAX_MEMORY_MB}")).is_ok());
}

#[test]
fn broken_toml_says_what_is_broken() {
    let manifest = Manifest::parse("name = ");
    assert!(matches!(manifest, Err(ManifestError::Syntax(_))));
}

#[test]
fn the_consent_summary_is_a_sentence_a_person_can_answer() {
    // This is the whole of what a user is asked. If it reads as jargon, the
    // permission model is decorative.
    let manifest = Manifest::parse(WEATHER).expect("valid");
    let lines = manifest.consent_summary();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().any(|l| l.contains("api.open-meteo.com")));
    for line in &lines {
        assert!(
            line.chars().next().is_some_and(|c| c.is_lowercase()),
            "{line:?} should read as a continuation of \"this widget wants to…\""
        );
        assert!(!line.contains("Capability"), "{line:?} leaks a type name");
    }
}

#[test]
fn a_widget_that_asks_for_nothing_says_so() {
    // An empty list in a consent dialog reads as a bug rather than as
    // reassurance.
    let manifest = Manifest::parse(r#"name = "clock""#).expect("valid");
    assert_eq!(manifest.consent_summary(), vec!["asks for nothing at all"]);
}

#[test]
fn every_refusal_explains_itself() {
    let messages = [
        ManifestError::BadName("Weather".into()).to_string(),
        ManifestError::UnknownCapability("tiemr".into()).to_string(),
        ManifestError::MalformedCapability("net:".into()).to_string(),
        ManifestError::MemoryTooLarge(9999).to_string(),
    ];
    for message in messages {
        assert!(message.len() > 30, "{message:?} explains nothing");
    }
}
