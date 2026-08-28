//! The orderings a string comparison gets wrong.

use super::*;

fn v(text: &str) -> Version {
    Version::parse(text).unwrap_or_else(|e| panic!("`{text}` should parse: {e}"))
}

#[test]
fn a_version_is_three_numbers() {
    let version = v("1.2.3");
    assert_eq!((version.major, version.minor, version.patch), (1, 2, 3));
    assert_eq!(version.pre, "");
    assert!(!version.is_pre_release());
    assert_eq!(version.to_string(), "1.2.3");
}

#[test]
fn a_pre_release_tail_is_kept_as_written() {
    let version = v("1.0.0-rc.1");
    assert_eq!(version.pre, "rc.1");
    assert!(version.is_pre_release());
    // Round-trips, so a version a person reads in a listing is the one their
    // manifest holds.
    assert_eq!(version.to_string(), "1.0.0-rc.1");
    assert_eq!(v("2.0.0-x-y.1").to_string(), "2.0.0-x-y.1");
}

#[test]
fn ten_is_after_nine_which_a_string_comparison_gets_wrong() {
    assert!(v("1.10.0") > v("1.9.0"));
    assert!(
        "1.10.0" < "1.9.0",
        "the mistake this type exists to prevent"
    );
    assert!(v("1.0.10") > v("1.0.9"));
    assert!(v("10.0.0") > v("9.99.99"));
}

#[test]
fn the_fields_are_compared_in_order() {
    assert!(v("2.0.0") > v("1.99.99"));
    assert!(v("1.2.0") > v("1.1.99"));
    assert!(v("1.1.2") > v("1.1.1"));
    assert_eq!(v("1.1.1"), v("1.1.1"));
}

#[test]
fn a_pre_release_is_older_than_the_release_it_precedes() {
    // The whole reason the tail takes part in the ordering at all: an update
    // check that got this backwards would offer `1.0.0-beta` to somebody
    // already running `1.0.0`.
    assert!(v("1.0.0-beta") < v("1.0.0"));
    assert!(v("1.0.0-rc.9") < v("1.0.0"));
    // But still newer than everything before it.
    assert!(v("1.0.0-alpha") > v("0.9.9"));
}

#[test]
fn pre_release_identifiers_are_compared_semvers_way() {
    // Numeric identifiers numerically — `beta.10` is after `beta.2`, which is
    // the other place a string comparison is wrong.
    assert!(v("1.0.0-beta.10") > v("1.0.0-beta.2"));
    // Alphabetically otherwise.
    assert!(v("1.0.0-alpha") < v("1.0.0-beta"));
    // A numeric identifier sorts below an alphanumeric one.
    assert!(v("1.0.0-1") < v("1.0.0-alpha"));
    // A shorter run precedes a longer one that starts the same way.
    assert!(v("1.0.0-beta") < v("1.0.0-beta.1"));
    // The example chain from the semver specification, in order.
    let chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
    ];
    for pair in chain.windows(2) {
        assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
    }
}

#[test]
fn sorting_finds_the_newest() {
    let mut versions: Vec<Version> = ["1.9.0", "1.10.0", "1.10.0-rc.1", "0.1.0", "2.0.0-alpha"]
        .iter()
        .map(|t| v(t))
        .collect();
    versions.sort();
    let text: Vec<String> = versions.iter().map(Version::to_string).collect();
    assert_eq!(
        text,
        ["0.1.0", "1.9.0", "1.10.0-rc.1", "1.10.0", "2.0.0-alpha"]
    );
    assert_eq!(versions.iter().max().unwrap(), &v("2.0.0-alpha"));
}

#[test]
fn a_version_that_is_not_one_is_refused_rather_than_guessed_at() {
    for shape in ["1.2", "1", "1.2.3.4", "", "v1.2.3", "banana"] {
        assert!(Version::parse(shape).is_err(), "`{shape}` should not parse");
    }
    assert!(matches!(Version::parse("1.2"), Err(VersionError::Shape(_))));
    assert!(matches!(
        Version::parse("1.2.x"),
        Err(VersionError::Field(_))
    ));
}

#[test]
fn a_leading_zero_is_refused_so_a_version_has_one_spelling() {
    // `01.2.3` and `1.2.3` would compare equal and print differently, which is
    // two names for one thing in a listing people read.
    assert!(matches!(
        Version::parse("01.2.3"),
        Err(VersionError::Field(_))
    ));
    assert!(matches!(
        Version::parse("1.02.3"),
        Err(VersionError::Field(_))
    ));
    // Zero itself is a number, and `0.1.0` is where every widget starts.
    assert_eq!(v("0.1.0").major, 0);
}

#[test]
fn build_metadata_is_refused_rather_than_ignored() {
    // Semver says it takes no part in ordering. Accepting it would let a
    // registry list two entries nobody can tell apart and nothing can choose
    // between.
    assert!(matches!(
        Version::parse("1.0.0+sha.abc"),
        Err(VersionError::BuildMetadata(_))
    ));
    assert!(matches!(
        Version::parse("1.0.0-rc.1+build"),
        Err(VersionError::BuildMetadata(_))
    ));
}

#[test]
fn an_empty_pre_release_identifier_is_refused() {
    for text in ["1.0.0-", "1.0.0-a..b", "1.0.0-.a"] {
        assert!(
            matches!(Version::parse(text), Err(VersionError::EmptyIdentifier(_))),
            "`{text}`"
        );
    }
}

#[test]
fn a_version_may_not_carry_a_separator_or_a_space() {
    // The version reaches a file name in the widget store, so what it may
    // contain is not only a matter of taste.
    for text in ["1.0.0-a/b", "1.0.0-a b", "1.0.0-a:b"] {
        assert!(
            matches!(Version::parse(text), Err(VersionError::Character(_))),
            "`{text}`"
        );
    }
}

#[test]
fn the_message_says_what_the_shape_is() {
    let why = Version::parse("banana").unwrap_err().to_string();
    assert!(why.contains("major.minor.patch"), "{why}");
}
