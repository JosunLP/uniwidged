//! What a widget is allowed to reach, and nothing else.
//!
//! This is the security boundary, so it is written to be read: the grammar is
//! small, the matching rules are spelled out, and the tests are mostly about
//! what is **refused**. A sandbox tested only for what it permits is not
//! tested.
//!
//! ## The grammar
//!
//! ```text
//! timer                       schedule a wake other than `Never`
//! net:<host>                  HTTPS to that host, via the host's client
//! net:*.<domain>              HTTPS to subdomains of that domain
//! fs:<subpath>                read/write inside the widget's own data dir
//! secrets                     the OS keychain, namespaced to this widget
//! locale                      locale-formatted dates, times and numbers
//! measure                     measuring a string in a named style
//! command:<name>              running one declared external command
//! metrics                     CPU, memory, battery, network counters
//! ```
//!
//! Anything not on that list does not exist to a widget — WASI Preview 2 has
//! no ambient authority, so an undeclared capability is absent rather than
//! denied, and there is nothing to escape from.

use std::fmt;

/// One granted permission.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Capability {
    Timer,
    /// Exactly this host. Compared case-insensitively, because DNS is.
    Net(String),
    /// Subdomains of this domain — `*.example.com`.
    ///
    /// Does **not** include the bare domain: a widget granted
    /// `*.tiles.example.com` has not been granted `example.com`, and reading
    /// the wildcard the other way is how a map widget ends up with the
    /// account API.
    NetWildcard(String),
    /// A path inside the widget's own data directory.
    Fs(String),
    Secrets,
    Locale,
    Measure,
    Command(String),
    Metrics,
}

/// Something a widget is trying to do, as the host sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Timer,
    Net { host: String },
    Fs { path: String },
    Secrets,
    Locale,
    Measure,
    Command { name: String },
    Metrics,
}

/// Why a request was refused. Worth distinguishing, because the two mean
/// different things to a user: one is a widget doing something it never
/// declared, the other is a malformed manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    /// The widget never declared this.
    NotGranted,
    /// The request itself is not well-formed — an empty host, a path that
    /// climbs out of the widget's directory.
    Malformed(&'static str),
}

impl fmt::Display for Denied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Denied::NotGranted => write!(f, "not granted"),
            Denied::Malformed(why) => write!(f, "malformed request: {why}"),
        }
    }
}

/// The set a user approved at install time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    granted: Vec<Capability>,
}

impl Policy {
    /// Grants nothing. What a widget gets before its manifest is approved, and
    /// what the conformance suite runs every widget under.
    pub fn deny_all() -> Self {
        Self::default()
    }

    pub fn from_granted(granted: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            granted: granted.into_iter().collect(),
        }
    }

    pub fn granted(&self) -> &[Capability] {
        &self.granted
    }

    /// May the widget do this?
    pub fn check(&self, request: &Request) -> Result<(), Denied> {
        match request {
            Request::Net { host } => {
                let host =
                    normalise_host(host).ok_or(Denied::Malformed("empty or invalid host"))?;
                self.granted
                    .iter()
                    .any(|c| net_matches(c, &host))
                    .then_some(())
                    .ok_or(Denied::NotGranted)
            }
            Request::Fs { path } => {
                if !is_contained(path) {
                    return Err(Denied::Malformed("path leaves the widget's directory"));
                }
                self.granted
                    .iter()
                    .any(|c| matches!(c, Capability::Fs(prefix) if path_within(path, prefix)))
                    .then_some(())
                    .ok_or(Denied::NotGranted)
            }
            Request::Command { name } => self
                .granted
                .iter()
                .any(|c| matches!(c, Capability::Command(n) if n == name))
                .then_some(())
                .ok_or(Denied::NotGranted),
            simple => {
                let needed = match simple {
                    Request::Timer => Capability::Timer,
                    Request::Secrets => Capability::Secrets,
                    Request::Locale => Capability::Locale,
                    Request::Measure => Capability::Measure,
                    Request::Metrics => Capability::Metrics,
                    _ => unreachable!("handled above"),
                };
                self.granted
                    .contains(&needed)
                    .then_some(())
                    .ok_or(Denied::NotGranted)
            }
        }
    }

    pub fn allows(&self, request: &Request) -> bool {
        self.check(request).is_ok()
    }
}

/// Lower-cases a host and rejects the shapes that are not one.
///
/// A trailing dot is stripped: `example.com.` and `example.com` are the same
/// name to DNS, and treating them as different is a way past an exact match.
fn normalise_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.contains('/') || host.contains(':') {
        return None;
    }
    Some(host)
}

fn net_matches(capability: &Capability, host: &str) -> bool {
    match capability {
        Capability::Net(allowed) => allowed.to_ascii_lowercase() == host,
        Capability::NetWildcard(domain) => {
            let domain = domain.to_ascii_lowercase();
            // The dot is part of the check, not decoration. Without it,
            // `*.example.com` would match `notexample.com`.
            host.len() > domain.len() + 1
                && host.ends_with(&domain)
                && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
        }
        _ => false,
    }
}

/// Does this path stay inside the widget's own directory?
///
/// Checked on the request rather than after joining, because by the time a
/// `..` has been joined onto a base path the damage is a canonicalisation
/// question, and those are answered differently on every platform.
pub(crate) fn is_contained(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    // A Windows drive letter or a UNC path is absolute even without a leading
    // separator.
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return false;
    }
    !path
        .split(['/', '\\'])
        .any(|segment| segment == ".." || segment.is_empty())
}

fn path_within(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let prefix = prefix.trim_end_matches(['/', '\\']);
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && matches!(path.as_bytes()[prefix.len()], b'/' | b'\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather() -> Policy {
        Policy::from_granted([
            Capability::Timer,
            Capability::Net("api.open-meteo.com".into()),
            Capability::Fs("cache".into()),
        ])
    }

    fn net(host: &str) -> Request {
        Request::Net { host: host.into() }
    }

    #[test]
    fn a_granted_host_is_reachable() {
        assert!(weather().allows(&net("api.open-meteo.com")));
        // DNS is case-insensitive, so the check has to be too — otherwise the
        // same grant passes or fails depending on how the widget spelled it.
        assert!(weather().allows(&net("API.Open-Meteo.COM")));
        // A trailing dot is the same name.
        assert!(weather().allows(&net("api.open-meteo.com.")));
    }

    #[test]
    fn a_granted_host_does_not_grant_anything_that_merely_looks_like_it() {
        // The suffix attack, which is the one an exact-match check gets wrong
        // if it reaches for `ends_with`.
        for hostile in [
            "evil.com",
            "api.open-meteo.com.evil.com",
            "notapi.open-meteo.com",
            "open-meteo.com",
            "sub.api.open-meteo.com",
        ] {
            assert!(
                !weather().allows(&net(hostile)),
                "{hostile} must be refused"
            );
        }
    }

    #[test]
    fn a_wildcard_covers_subdomains_and_not_the_domain_itself() {
        let policy = Policy::from_granted([Capability::NetWildcard("tiles.example.com".into())]);
        assert!(policy.allows(&net("a.tiles.example.com")));
        assert!(policy.allows(&net("b.a.tiles.example.com")));
        // Reading the wildcard as covering its own domain is how a map widget
        // ends up with the account API.
        assert!(!policy.allows(&net("tiles.example.com")));
        assert!(!policy.allows(&net("example.com")));
        // The dot matters: without it this matches.
        assert!(!policy.allows(&net("eviltiles.example.com")));
    }

    #[test]
    fn a_malformed_host_is_refused_as_malformed_rather_than_ungranted() {
        // The distinction reaches the user: one is a widget overreaching, the
        // other is a broken widget.
        let policy = weather();
        assert_eq!(
            policy.check(&net("")),
            Err(Denied::Malformed("empty or invalid host"))
        );
        assert!(matches!(
            policy.check(&net("api.open-meteo.com/../evil")),
            Err(Denied::Malformed(_))
        ));
        assert!(matches!(
            policy.check(&net("api.open-meteo.com:8080")),
            Err(Denied::Malformed(_))
        ));
    }

    #[test]
    fn a_path_cannot_climb_out_of_the_widgets_directory() {
        let policy = weather();
        for hostile in [
            "../../.ssh/id_rsa",
            "cache/../../secrets",
            "/etc/passwd",
            "\\Windows\\System32",
            "C:\\Windows",
            "cache//..//x",
        ] {
            assert!(
                matches!(
                    policy.check(&Request::Fs {
                        path: hostile.into()
                    }),
                    Err(Denied::Malformed(_))
                ),
                "{hostile} must be refused as malformed"
            );
        }
    }

    #[test]
    fn a_granted_subpath_does_not_grant_its_siblings() {
        let policy = weather();
        assert!(policy.allows(&Request::Fs {
            path: "cache".into()
        }));
        assert!(policy.allows(&Request::Fs {
            path: "cache/today.json".into()
        }));
        // `starts_with` alone would let this through.
        assert!(!policy.allows(&Request::Fs {
            path: "cache-of-somebody-else".into()
        }));
        assert!(!policy.allows(&Request::Fs {
            path: "tokens".into()
        }));
    }

    #[test]
    fn deny_all_refuses_every_capability() {
        // What the conformance suite runs every widget under. If any of these
        // ever passes, the sandbox has a hole in it.
        let policy = Policy::deny_all();
        for request in [
            Request::Timer,
            Request::Secrets,
            Request::Locale,
            Request::Measure,
            Request::Metrics,
            net("example.com"),
            Request::Fs {
                path: "cache".into(),
            },
            Request::Command {
                name: "notify-send".into(),
            },
        ] {
            assert_eq!(
                policy.check(&request),
                Err(Denied::NotGranted),
                "{request:?}"
            );
        }
    }

    #[test]
    fn one_granted_capability_does_not_imply_another() {
        // Timer and network are granted together often enough that a bug here
        // would go unnoticed in normal use.
        let timer_only = Policy::from_granted([Capability::Timer]);
        assert!(timer_only.allows(&Request::Timer));
        assert!(!timer_only.allows(&net("example.com")));
        assert!(!timer_only.allows(&Request::Secrets));
    }
}
