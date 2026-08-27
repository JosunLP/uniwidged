//! The configuration is where a user's consent is recorded, so most of these
//! are about what it refuses.

use super::*;
use uniwidged_runtime::capability::Request;

const WEATHER_MANIFEST: &str = r#"
name = "weather"
capabilities = ["timer", "net:api.open-meteo.com", "fs:cache"]
"#;

fn weather() -> Manifest {
    Manifest::parse(WEATHER_MANIFEST).expect("valid manifest")
}

const CONFIG: &str = r#"
[[widget]]
name = "weather"
component = "widgets/weather.wasm"
granted = ["timer", "net:api.open-meteo.com"]

[widget.placement]
output = "DP-1"
x = 48
y = 96
width = 320
height = 200
"#;

#[test]
fn a_configuration_parses_into_what_it_said() {
    let config = Config::parse(CONFIG).expect("valid");
    let widget = config.widget("weather").expect("present");
    assert_eq!(widget.component, PathBuf::from("widgets/weather.wasm"));
    assert_eq!(widget.placement.output.as_deref(), Some("DP-1"));
    assert_eq!(widget.placement.y, 96);
    assert!(
        widget.enabled,
        "a widget is enabled unless it says otherwise"
    );
    assert!(!widget.placement.locked);
}

#[test]
fn a_missing_configuration_is_an_empty_desktop_rather_than_an_error() {
    // A desktop with no widgets is a perfectly good first run.
    let missing = Path::new("/nonexistent/uniwidged/config.toml");
    assert_eq!(Config::read(missing).expect("no error"), Config::default());
}

#[test]
fn a_configuration_survives_the_round_trip() {
    // The host writes this file back when a widget is dragged, so anything it
    // cannot round-trip is a setting a user would lose by moving a widget.
    let config = Config::parse(CONFIG).expect("valid");
    let again = Config::parse(&config.to_toml()).expect("re-parsed");
    assert_eq!(config, again);
}

#[test]
fn declining_a_capability_is_allowed_and_expected() {
    // Granting *less* than the manifest asks for is how a user says no to part
    // of a widget without saying no to all of it.
    let config = Config::parse(CONFIG).expect("valid");
    let policy = config
        .widget("weather")
        .unwrap()
        .policy(&weather())
        .expect("valid");
    assert!(policy.allows(&Request::Timer));
    assert!(policy.allows(&Request::Net {
        host: "api.open-meteo.com".into()
    }));
    assert!(
        !policy.allows(&Request::Fs {
            path: "cache".into()
        }),
        "the file capability was declared but not granted"
    );
}

#[test]
fn granting_more_than_the_manifest_asks_for_is_refused() {
    // A user approving an installation approved *that manifest's* list. A
    // config that grants beyond it hands a widget something its author never
    // designed for and its user never saw.
    let config = Config::parse(
        r#"
[[widget]]
name = "weather"
component = "w.wasm"
granted = ["timer", "secrets"]
"#,
    )
    .expect("parses");
    let error = config.widget("weather").unwrap().policy(&weather());
    assert!(
        matches!(error, Err(ConfigError::OverGranted { ref capability, .. }) if capability == "secrets"),
        "{error:?}"
    );
}

#[test]
fn a_capability_that_is_not_one_says_so_differently() {
    // "You granted something the manifest does not ask for" and "that is not a
    // capability" send a reader to different places.
    let config = Config::parse(
        r#"
[[widget]]
name = "weather"
component = "w.wasm"
granted = ["tiemr"]
"#,
    )
    .expect("parses");
    assert!(matches!(
        config.widget("weather").unwrap().policy(&weather()),
        Err(ConfigError::BadCapability { .. })
    ));
}

#[test]
fn a_name_that_disagrees_with_the_manifest_is_refused() {
    // The name decides the data directory and the keychain namespace. Two
    // names means a widget whose secrets are somewhere it will not look.
    let config = Config::parse(
        r#"
[[widget]]
name = "wether"
component = "w.wasm"
"#,
    )
    .expect("parses");
    assert!(matches!(
        config.widget("wether").unwrap().policy(&weather()),
        Err(ConfigError::NameMismatch { .. })
    ));
}

#[test]
fn installing_grants_exactly_what_the_manifest_declared() {
    // What `uniwidged install` writes once the user has said yes: the list
    // they were shown, and nothing else.
    let manifest = weather();
    let config =
        WidgetConfig::granting_all("weather", PathBuf::from("widgets/weather.wasm"), &manifest);
    assert_eq!(
        config.granted,
        vec!["timer", "net:api.open-meteo.com", "fs:cache"]
    );
    assert!(config.policy(&manifest).is_ok());
}

#[test]
fn a_capability_means_the_same_thing_written_as_it_does_declared() {
    // `capability_text` is the inverse of the manifest's parser. If the two
    // ever disagree, a granted capability silently stops matching the declared
    // one and the widget loses it with no error anywhere.
    for text in [
        "timer",
        "secrets",
        "locale",
        "measure",
        "metrics",
        "net:api.example.com",
        "net:*.tiles.example.com",
        "fs:cache",
        "command:notify-send",
    ] {
        let manifest =
            Manifest::parse(&format!("name = 'x'\ncapabilities = ['{text}']")).expect("parses");
        assert_eq!(
            capability_text(&manifest.capabilities[0]),
            text,
            "{text} did not survive the round trip"
        );
    }
}

#[test]
fn the_manifest_path_defaults_beside_the_component() {
    let config = Config::parse(CONFIG).expect("valid");
    assert_eq!(
        config.widget("weather").unwrap().manifest_path(),
        PathBuf::from("widgets/weather.toml")
    );
}

#[test]
fn a_placement_remembers_its_output_by_name() {
    // Indices renumber when a monitor is unplugged, and a widget that moves
    // when you dock is a widget that has lost its place.
    let config = Config::parse(CONFIG).expect("valid");
    let placement = &config.widget("weather").unwrap().placement;
    assert_eq!(placement.output.as_deref(), Some("DP-1"));
    let text = Config::parse(&config.to_toml()).unwrap();
    assert_eq!(
        text.widget("weather").unwrap().placement.output.as_deref(),
        Some("DP-1")
    );
}
