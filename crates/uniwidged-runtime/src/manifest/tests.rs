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

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// A manifest is what a user is shown and what the host enforces, so a
/// `[[setting]]` block that does not mean something exact is refused where its
/// author can fix it. These are mostly the refusals, and the one that matters
/// most is the declared default: a manifest shipping a value its own form
/// would not let a user re-enter is broken in a way nobody notices until they
/// open the form.
fn with_setting(block: &str) -> Result<Manifest, ManifestError> {
    Manifest::parse(&format!(
        "name = \"example\"\nversion = \"1.0.0\"\n\n[[setting]]\n{block}"
    ))
}

#[test]
fn a_manifest_declares_each_kind_of_setting() {
    let manifest = Manifest::parse(
        r#"
name = "weather"
version = "1.0.0"

[[setting]]
key = "city"
label = "City"
type = "text"
default = "Berlin"

[[setting]]
key = "warn-at"
type = "number"
default = 0.8
min = 0.0
max = 1.0

[[setting]]
key = "show-date"
type = "toggle"
default = true

[[setting]]
key = "units"
type = "choice"
default = "metric"
options = ["metric", "imperial"]
"#,
    )
    .expect("a manifest with settings should parse");

    assert_eq!(manifest.settings.len(), 4);
    // In the order the manifest wrote them: that is the order the form is
    // shown in, and its author chose it.
    let keys: Vec<&str> = manifest.settings.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["city", "warn-at", "show-date", "units"]);

    assert_eq!(manifest.settings[0].label, "City");
    // A widget that wrote no label still produces a usable form rather than an
    // empty one.
    assert_eq!(manifest.settings[1].label, "warn-at");
    assert_eq!(
        manifest.settings[1].kind,
        SettingKind::Number { min: 0.0, max: 1.0 }
    );
    assert_eq!(manifest.settings[2].default, SettingValue::Toggle(true));
}

#[test]
fn an_integer_default_for_a_number_is_a_number() {
    // TOML distinguishes `1` from `1.0`, and an author writing `default = 1`
    // for a number means one. Refusing that would be a manifest error nobody
    // could see the point of.
    let manifest = with_setting("key = \"count\"\ntype = \"number\"\ndefault = 1")
        .expect("an integer default should parse");
    assert_eq!(manifest.settings[0].default, SettingValue::Number(1.0));
}

#[test]
fn a_declared_default_its_own_form_would_refuse_is_caught_at_parse_time() {
    // The interesting one. A widget shipping `default = 2` for a setting
    // capped at 1 has a value the user can never re-enter once they change it,
    // and nothing would notice until somebody opened the form.
    let error =
        with_setting("key = \"warn-at\"\ntype = \"number\"\ndefault = 2.0\nmin = 0.0\nmax = 1.0")
            .expect_err("a default outside its own range should be refused");
    let said = error.to_string();
    assert!(said.contains("declared default"), "{said}");

    let error = with_setting(
        "key = \"units\"\ntype = \"choice\"\ndefault = \"furlongs\"\noptions = [\"metric\", \"imperial\"]",
    )
    .expect_err("a default outside its own options should be refused");
    assert!(error.to_string().contains("declared default"));
}

#[test]
fn a_default_of_the_wrong_type_is_refused() {
    assert!(with_setting("key = \"n\"\ntype = \"number\"\ndefault = \"0.8\"").is_err());
    assert!(with_setting("key = \"t\"\ntype = \"toggle\"\ndefault = \"true\"").is_err());
    assert!(with_setting("key = \"s\"\ntype = \"text\"\ndefault = 1").is_err());
}

#[test]
fn a_setting_with_no_default_is_refused() {
    // There is no such thing as an unset setting: an unset one is its default.
    // That is what lets a widget read a declared key without a fallback, so a
    // manifest with no default would break the guarantee for every reader.
    let error = with_setting("key = \"city\"\ntype = \"text\"")
        .expect_err("a setting with no default should be refused");
    assert!(error.to_string().contains("no default"), "{error}");
}

#[test]
fn an_unknown_type_is_refused_rather_than_treated_as_text() {
    let error = with_setting("key = \"city\"\ntype = \"string\"\ndefault = \"Berlin\"")
        .expect_err("`string` is not a type");
    let said = error.to_string();
    assert!(said.contains("text, number, toggle or choice"), "{said}");
}

#[test]
fn a_key_that_would_not_survive_a_file_or_a_command_line_is_refused() {
    for bad in ["City", "warn_at", "2fast", "with space", "trailing-"] {
        assert!(
            with_setting(&format!(
                "key = \"{bad}\"\ntype = \"text\"\ndefault = \"x\""
            ))
            .is_err(),
            "`{bad}` should not be a key"
        );
    }
}

#[test]
fn a_key_declared_twice_is_refused() {
    // Not a manifest that means two things — one whose author will be
    // surprised by which of them wins.
    let error = Manifest::parse(
        r#"
name = "example"
version = "1.0.0"

[[setting]]
key = "city"
type = "text"
default = "Berlin"

[[setting]]
key = "city"
type = "text"
default = "Hamburg"
"#,
    )
    .expect_err("a repeated key should be refused");
    assert!(error.to_string().contains("twice"), "{error}");
}

#[test]
fn a_choice_of_one_is_refused() {
    // A choice of one is a decision its author already made, and showing the
    // user a list they cannot change is a form that wastes their time.
    let error = with_setting(
        "key = \"units\"\ntype = \"choice\"\ndefault = \"metric\"\noptions = [\"metric\"]",
    )
    .expect_err("a single-option choice should be refused");
    assert!(error.to_string().contains("at least two"), "{error}");
}

#[test]
fn a_range_no_value_can_satisfy_is_refused() {
    let error = with_setting("key = \"n\"\ntype = \"number\"\ndefault = 1.0\nmin = 2.0\nmax = 1.0")
        .expect_err("min above max should be refused");
    assert!(
        error.to_string().contains("no value can satisfy"),
        "{error}"
    );
}

#[test]
fn a_number_needs_no_bounds() {
    // A widget taking a count has no honest upper bound to write, and
    // demanding one would get an arbitrary number typed in.
    let manifest = with_setting("key = \"count\"\ntype = \"number\"\ndefault = 3")
        .expect("an unbounded number should parse");
    assert_eq!(
        manifest.settings[0].check(&SettingValue::Number(1e9)),
        Ok(())
    );
}

#[test]
fn a_widget_may_not_declare_an_unbounded_number_of_settings() {
    let many: String = (0..MAX_SETTINGS + 1)
        .map(|i| format!("[[setting]]\nkey = \"k{i}\"\ntype = \"toggle\"\ndefault = false\n\n"))
        .collect();
    let error = Manifest::parse(&format!(
        "name = \"example\"\nversion = \"1.0.0\"\n\n{many}"
    ))
    .expect_err("too many settings should be refused");
    assert!(error.to_string().contains("at most"), "{error}");
}

#[test]
fn a_manifest_with_no_settings_has_none_and_is_still_a_manifest() {
    // Which is every manifest written before this existed, and most of them
    // afterwards.
    let manifest = Manifest::parse("name = \"example\"\nversion = \"1.0.0\"").expect("parse");
    assert!(manifest.settings.is_empty());
}

// ---------------------------------------------------------------------------
// `link:<host>`
// ---------------------------------------------------------------------------

#[test]
fn a_link_grant_reads_like_a_net_grant_and_is_not_one() {
    let manifest = Manifest::parse(
        r#"
name = "weather"
version = "1.0.0"
capabilities = ["link:example.com", "link:*.docs.example.com"]
"#,
    )
    .expect("a link grant should parse");
    assert_eq!(
        manifest.capabilities,
        vec![
            Capability::Link("example.com".into()),
            Capability::LinkWildcard("docs.example.com".into()),
        ]
    );
}

#[test]
fn a_link_grant_is_refused_in_the_shapes_a_net_grant_is() {
    // The same parser, the same refusals — because they are the same reading
    // of the same thing, and a link grant that accepted a host a net grant
    // would not would be a second, looser door.
    for bad in [
        "link:",
        "link:*",
        "link:*.",
        "link:https://example.com",
        "link:example.com/path",
        "link:exa mple.com",
    ] {
        assert!(
            Manifest::parse(&format!(
                "name = \"x\"\nversion = \"1.0.0\"\ncapabilities = [\"{bad}\"]"
            ))
            .is_err(),
            "`{bad}` should not be a capability"
        );
    }
}

#[test]
fn the_consent_line_for_a_link_says_what_it_actually_does() {
    // "talk to example.com over HTTPS" and "open example.com in your browser"
    // are different things happening to a user, and a consent screen that said
    // the same words for both would be asking one question and getting
    // permission for two.
    let manifest = Manifest::parse(
        "name = \"x\"\nversion = \"1.0.0\"\ncapabilities = [\"link:example.com\", \"net:example.com\"]",
    )
    .expect("parse");
    let lines = manifest.consent_summary();
    assert!(
        lines.iter().any(|l| l.contains("open example.com")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("talk to example.com")),
        "{lines:?}"
    );
    assert_ne!(lines[0], lines[1]);
}
