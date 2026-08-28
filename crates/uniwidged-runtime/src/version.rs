//! Which of two versions is newer.
//!
//! A version string is decoration until something has to *order* two of them,
//! and something now does: a registry index offers `clock 1.2.0` while this
//! machine has `clock 1.1.4`, and "is there an update" is a comparison rather
//! than a string inequality. Doing that comparison by hand is how `1.10.0`
//! comes out older than `1.9.0`, so it lives here with tests behind it.
//!
//! ## Semantic versioning, and the parts of it a widget needs
//!
//! `major.minor.patch`, with an optional `-pre-release` tail. That is all.
//! Build metadata (`+sha.abc`) is **refused** rather than ignored: semver says
//! it takes no part in ordering, so two versions differing only there are
//! equal, and a registry that lists both has two entries a person cannot tell
//! apart and a resolver cannot choose between. A publisher who wants to say
//! which build it was has the description.
//!
//! The pre-release rules are semver's, because they are the ones people
//! already expect and the ones that make a release train work:
//!
//! * `1.0.0-beta` is **older** than `1.0.0`. A pre-release is a version on its
//!   way somewhere, and shipping it over the release it precedes is the one
//!   thing this ordering exists to prevent.
//! * Identifiers are compared piece by piece, numeric pieces numerically —
//!   so `beta.2` precedes `beta.10` — and a numeric piece sorts below an
//!   alphanumeric one.
//!
//! ## Why not the `semver` crate
//!
//! Because what is wanted here is one hundred lines of total order over three
//! numbers, not a dependency with a requirement grammar, a comparator algebra
//! and a serde feature. uniwidged has no version *ranges* anywhere — an index
//! entry is an exact version and a widget asks for none — so the part of that
//! crate this would use is the part written below.

use std::cmp::Ordering;

/// A widget's version: `major.minor.patch`, with an optional pre-release tail.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// The `-beta.1` tail, without its hyphen. Empty for a release.
    ///
    /// Kept as written so that the version prints back exactly as its author
    /// typed it: a version somebody reads in a registry listing and a version
    /// they read in a manifest have to be comparable by eye as well as by
    /// [`Ord`].
    pub pre: String,
}

/// Why a version string was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// Not three dot-separated numbers.
    Shape(String),
    /// A field that is not a number, or is one with a leading zero — `01.2.3`
    /// and `1.2.3` would otherwise be two spellings of one version.
    Field(String),
    /// Build metadata, which this deliberately does not accept — see the
    /// module note.
    BuildMetadata(String),
    /// An empty identifier in the pre-release tail: `1.0.0-` or `1.0.0-a..b`.
    EmptyIdentifier(String),
    /// A character that is not a letter, a digit or a hyphen.
    Character(String),
}

impl std::fmt::Display for VersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionError::Shape(text) => write!(
                f,
                "`{text}` is not a version: versions here are `major.minor.patch`, \
                 like `1.0.0`, with an optional `-beta.1` tail"
            ),
            VersionError::Field(text) => write!(
                f,
                "`{text}` is not a version: each of the three parts must be a \
                 number without a leading zero"
            ),
            VersionError::BuildMetadata(text) => write!(
                f,
                "`{text}` carries build metadata, which uniwidged does not accept: \
                 it takes no part in ordering, so two versions differing only in it \
                 could not be told apart"
            ),
            VersionError::EmptyIdentifier(text) => write!(
                f,
                "`{text}` has an empty pre-release identifier; `1.0.0-beta.1` is the shape"
            ),
            VersionError::Character(text) => write!(
                f,
                "`{text}` has a character a version may not contain: letters, \
                 digits, dots and hyphens only"
            ),
        }
    }
}

impl std::error::Error for VersionError {}

impl Version {
    /// A release version, with no pre-release tail.
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: String::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, VersionError> {
        let refuse_shape = || VersionError::Shape(text.to_string());
        if text.contains('+') {
            return Err(VersionError::BuildMetadata(text.to_string()));
        }
        // The tail is split off first: a hyphen inside it is part of an
        // identifier, and only the *first* one separates the two halves.
        let (numbers, pre) = match text.split_once('-') {
            Some((numbers, pre)) => (numbers, pre),
            None => (text, ""),
        };

        // The count first, so that `banana` is told what the shape is rather
        // than which of three parts was not a number.
        let fields: Vec<&str> = numbers.split('.').collect();
        let [major, minor, patch] = fields.as_slice() else {
            return Err(refuse_shape());
        };
        let number = |field: &str| -> Result<u64, VersionError> {
            if field.is_empty() || (field.len() > 1 && field.starts_with('0')) {
                return Err(VersionError::Field(text.to_string()));
            }
            field
                .parse()
                .map_err(|_| VersionError::Field(text.to_string()))
        };
        let (major, minor, patch) = (number(major)?, number(minor)?, number(patch)?);

        if text.ends_with('-') || (!pre.is_empty() && pre.split('.').any(str::is_empty)) {
            return Err(VersionError::EmptyIdentifier(text.to_string()));
        }
        if !pre
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        {
            return Err(VersionError::Character(text.to_string()));
        }

        Ok(Self {
            major,
            minor,
            patch,
            pre: pre.to_string(),
        })
    }

    /// Is this a version on its way somewhere rather than one that arrived?
    pub fn is_pre_release(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            major,
            minor,
            patch,
            pre,
        } = self;
        write!(f, "{major}.{minor}.{patch}")?;
        if !pre.is_empty() {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for Version {
    type Err = VersionError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| compare_pre_release(&self.pre, &other.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Semver's pre-release ordering.
///
/// The two rules that matter, in the order they are applied: **no tail beats
/// any tail**, because `1.0.0` is the arrival of `1.0.0-rc.1`; and then
/// identifier by identifier, where a number is compared as a number and sorts
/// below anything alphanumeric.
fn compare_pre_release(left: &str, right: &str) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    let mut left = left.split('.');
    let mut right = right.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            // A shorter run of identifiers precedes a longer one that starts
            // the same way: `beta` precedes `beta.1`.
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a), Some(b)) => {
                let ordering = match (a.parse::<u64>(), b.parse::<u64>()) {
                    (Ok(a), Ok(b)) => a.cmp(&b),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => a.cmp(b),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
