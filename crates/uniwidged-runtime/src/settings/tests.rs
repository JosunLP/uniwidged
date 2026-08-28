//! What a setting will and will not hold.
//!
//! The whole point of declaring settings rather than handing a widget a string
//! map is that a widget can never be given a value it cannot handle. That is
//! only true if the checking is right, so most of this is about the values
//! that must be refused — and about the two that are refused in *different*
//! places, because who can fix them differs: a bad default is the widget
//! author's and is caught by the manifest parser, and a bad value is the
//! user's and is caught before it is written.

use super::*;

fn number(key: &str, min: f64, max: f64, default: f64) -> Setting {
    Setting {
        key: key.into(),
        label: key.into(),
        kind: SettingKind::Number { min, max },
        default: SettingValue::Number(default),
    }
}

fn choice(key: &str, options: &[&str], default: &str) -> Setting {
    Setting {
        key: key.into(),
        label: key.into(),
        kind: SettingKind::Choice {
            options: options.iter().map(|o| o.to_string()).collect(),
        },
        default: SettingValue::Text(default.into()),
    }
}

fn toggle(key: &str, default: bool) -> Setting {
    Setting {
        key: key.into(),
        label: key.into(),
        kind: SettingKind::Toggle,
        default: SettingValue::Toggle(default),
    }
}

fn text(key: &str, default: &str) -> Setting {
    Setting {
        key: key.into(),
        label: key.into(),
        kind: SettingKind::Text,
        default: SettingValue::Text(default.into()),
    }
}

#[test]
fn a_value_of_the_wrong_type_is_refused_by_every_kind() {
    // The property the widget's freedom from fallbacks rests on. If any of
    // these passed, a widget matching on `SettingValue::Number` for a key it
    // declared as a number would have a branch that can be reached.
    let cases: Vec<(Setting, SettingValue)> = vec![
        (number("n", 0.0, 1.0, 0.5), SettingValue::Text("0.5".into())),
        (number("n", 0.0, 1.0, 0.5), SettingValue::Toggle(true)),
        (toggle("t", false), SettingValue::Text("true".into())),
        (toggle("t", false), SettingValue::Number(1.0)),
        (text("s", ""), SettingValue::Number(1.0)),
        (text("s", ""), SettingValue::Toggle(true)),
        (choice("c", &["a", "b"], "a"), SettingValue::Number(1.0)),
    ];
    for (setting, value) in cases {
        assert!(
            setting.check(&value).is_err(),
            "{} should not hold {value:?}",
            setting.key
        );
    }
}

#[test]
fn a_number_outside_its_declared_range_is_refused() {
    let warn = number("warn-at", 0.0, 1.0, 0.8);
    assert_eq!(warn.check(&SettingValue::Number(0.0)), Ok(()));
    assert_eq!(warn.check(&SettingValue::Number(1.0)), Ok(()));
    assert!(warn.check(&SettingValue::Number(-0.1)).is_err());
    assert!(warn.check(&SettingValue::Number(1.1)).is_err());
    // `NaN` fails both comparisons rather than passing one, which is what
    // keeps every comparison a widget makes afterwards meaningful.
    assert!(warn.check(&SettingValue::Number(f64::NAN)).is_err());
    assert!(warn.check(&SettingValue::Number(f64::INFINITY)).is_err());
}

#[test]
fn a_choice_outside_its_options_is_refused_and_the_message_lists_them() {
    let units = choice("units", &["metric", "imperial"], "metric");
    assert_eq!(units.check(&SettingValue::Text("imperial".into())), Ok(()));
    let error = units
        .check(&SettingValue::Text("furlongs".into()))
        .expect_err("not an option");
    // The list, because the whole value of a closed set is that somebody can
    // be told what the set is.
    assert!(error.0.contains("metric"), "{error}");
    assert!(error.0.contains("imperial"), "{error}");
}

#[test]
fn text_is_bounded() {
    let city = text("city", "Berlin");
    assert_eq!(
        city.check(&SettingValue::Text("x".repeat(MAX_TEXT_BYTES))),
        Ok(())
    );
    assert!(
        city.check(&SettingValue::Text("x".repeat(MAX_TEXT_BYTES + 1)))
            .is_err()
    );
}

// --- what a command line can say ------------------------------------------

#[test]
fn a_command_line_string_is_read_against_the_declared_type() {
    // A command line has only strings, so this is the one place a type is
    // recovered — and it is recovered from the *declaration*, so `--set
    // warn-at=true` is refused rather than stored as the text "true".
    assert_eq!(
        number("n", 0.0, 1.0, 0.5).parse("0.25"),
        Ok(SettingValue::Number(0.25))
    );
    assert!(number("n", 0.0, 1.0, 0.5).parse("true").is_err());
    assert!(number("n", 0.0, 1.0, 0.5).parse("").is_err());
    // Parsed *and* checked: a number that reads fine and is out of range must
    // not reach the file.
    assert!(number("n", 0.0, 1.0, 0.5).parse("9").is_err());

    assert_eq!(
        toggle("t", false).parse("true"),
        Ok(SettingValue::Toggle(true))
    );
    assert_eq!(
        toggle("t", false).parse("false"),
        Ok(SettingValue::Toggle(false))
    );
    // `yes`, `1` and `on` all read as true to somebody and as a typo to
    // somebody else. A configuration that guesses is one nobody can predict.
    for guessable in ["yes", "1", "on", "TRUE", "True"] {
        assert!(
            toggle("t", false).parse(guessable).is_err(),
            "`{guessable}` should not be a toggle"
        );
    }
}

#[test]
fn text_takes_whatever_it_was_given_including_something_that_looks_typed() {
    // That is what text means, and a text setting that quietly became a number
    // because somebody's city was called `1` would be a bug nobody would guess.
    assert_eq!(
        text("s", "").parse("42"),
        Ok(SettingValue::Text("42".into()))
    );
    assert_eq!(
        text("s", "").parse("true"),
        Ok(SettingValue::Text("true".into()))
    );
}

#[test]
fn a_choice_is_parsed_as_text_and_then_checked() {
    let units = choice("units", &["metric", "imperial"], "metric");
    assert_eq!(
        units.parse("metric"),
        Ok(SettingValue::Text("metric".into()))
    );
    assert!(units.parse("Metric").is_err(), "options are exact");
}

// --- resolving a file against a manifest ----------------------------------

#[test]
fn an_unset_key_gets_its_default_and_a_set_one_is_kept() {
    let declared = vec![number("warn-at", 0.0, 1.0, 0.8), text("label", "CPU")];
    let mut configured = BTreeMap::new();
    configured.insert("label".to_string(), SettingValue::Text("Load".into()));

    let (resolved, complaints) = Settings::resolve(&declared, &configured);
    assert_eq!(complaints, Vec::<String>::new());
    // Every declared key has a value. That is what lets a widget read one
    // without a fallback, so it is the property worth asserting directly.
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved["warn-at"], SettingValue::Number(0.8));
    assert_eq!(resolved["label"], SettingValue::Text("Load".into()));
}

#[test]
fn a_stale_key_is_complained_about_rather_than_refused_or_dropped_silently() {
    // The difference from an over-grant, which refuses. An over-grant is a
    // security fact — the file exceeds what the user approved. A stale setting
    // is an upgrade artefact: a widget dropped a knob in its new version, and
    // refusing to start the desktop over that would make every upgrade a
    // support question.
    let declared = vec![text("city", "Berlin")];
    let mut configured = BTreeMap::new();
    configured.insert("city".to_string(), SettingValue::Text("Hamburg".into()));
    configured.insert("units".to_string(), SettingValue::Text("metric".into()));

    let (resolved, complaints) = Settings::resolve(&declared, &configured);
    assert_eq!(
        resolved.len(),
        1,
        "the stale key is not passed to the widget"
    );
    assert_eq!(complaints.len(), 1);
    assert!(complaints[0].contains("units"), "{complaints:?}");
}

#[test]
fn a_value_the_setting_would_refuse_falls_back_and_says_so() {
    // What a hand-edited file produces. The widget gets something usable and
    // somebody is told which line is being ignored — where silently using the
    // default would make a typo invisible.
    let declared = vec![number("warn-at", 0.0, 1.0, 0.8)];
    let mut configured = BTreeMap::new();
    configured.insert("warn-at".to_string(), SettingValue::Number(9.0));

    let (resolved, complaints) = Settings::resolve(&declared, &configured);
    assert_eq!(resolved["warn-at"], SettingValue::Number(0.8));
    assert_eq!(complaints.len(), 1);
    assert!(complaints[0].contains("warn-at"), "{complaints:?}");
    assert!(complaints[0].contains("default"), "{complaints:?}");
}

#[test]
fn a_widget_that_declares_nothing_resolves_to_nothing() {
    let (resolved, complaints) = Settings::resolve(&[], &BTreeMap::new());
    assert!(resolved.is_empty());
    assert!(complaints.is_empty());
}

// --- the live handle ------------------------------------------------------

#[test]
fn a_change_reaches_a_handle_that_was_already_cloned() {
    // The property the whole design rests on: reconfiguring a widget must not
    // mean restarting it, and the instance was handed its `Settings` when it
    // started. A copy would keep answering with the old number, plausibly, and
    // nothing would look wrong.
    let settings = Settings::new(BTreeMap::from([(
        "warn-at".to_string(),
        SettingValue::Number(0.8),
    )]));
    let held_by_the_instance = settings.clone();
    assert_eq!(
        held_by_the_instance.get("warn-at"),
        Some(SettingValue::Number(0.8))
    );

    settings.replace(BTreeMap::from([(
        "warn-at".to_string(),
        SettingValue::Number(0.5),
    )]));
    assert_eq!(
        held_by_the_instance.get("warn-at"),
        Some(SettingValue::Number(0.5))
    );
}

#[test]
fn an_undeclared_key_reads_as_absent() {
    let settings = Settings::empty();
    assert_eq!(settings.get("anything"), None);
    assert_eq!(Settings::default().get("anything"), None);
}

// --- keys -----------------------------------------------------------------

#[test]
fn a_key_is_something_that_survives_a_toml_file_and_a_command_line() {
    for good in ["city", "warn-at", "series2", "a"] {
        assert!(is_usable_key(good), "`{good}` should be a key");
    }
    for bad in [
        "",
        "City",
        "warn_at",
        "2fast",
        "-leading",
        "trailing-",
        "with space",
        "with.dot",
        &"x".repeat(MAX_KEY_LEN + 1),
    ] {
        assert!(!is_usable_key(bad), "`{bad}` should not be a key");
    }
}

#[test]
fn a_value_writes_itself_the_way_a_file_would() {
    // `uniwidged configure` prints these and `config.toml` holds them, so the
    // shortest round-tripping form is the right one — `0.8`, not
    // `0.8000000000000000444`.
    assert_eq!(SettingValue::Number(0.8).display(), "0.8");
    assert_eq!(SettingValue::Number(1.0).display(), "1");
    assert_eq!(SettingValue::Toggle(true).display(), "true");
    assert_eq!(SettingValue::Text("Berlin".into()).display(), "Berlin");
}

#[test]
fn the_untagged_encoding_round_trips_each_shape() {
    // `config.toml` is meant to be hand-edited, so a value is written bare:
    // `warn-at = 0.8`, not `warn-at = { number = 0.8 }`. The order of the
    // variants is what makes reading it back unambiguous, and getting that
    // wrong would read `true` as the text "true".
    for value in [
        SettingValue::Toggle(true),
        SettingValue::Number(0.8),
        SettingValue::Text("Berlin".into()),
    ] {
        let map = BTreeMap::from([("k".to_string(), value.clone())]);
        let text = toml::to_string(&map).expect("serialise");
        let back: BTreeMap<String, SettingValue> = toml::from_str(&text).expect("deserialise");
        assert_eq!(back["k"], value, "{text}");
    }
}
