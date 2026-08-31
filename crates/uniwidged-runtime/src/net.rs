//! HTTPS, for a widget that was granted a host.
//!
//! The `net` capability is host-scoped — `net:api.open-meteo.com`, never
//! `network` — because that is the only form a user can meaningfully consent
//! to. This file is what makes the scoping true rather than decorative, and
//! nearly all of it is about one thing:
//!
//! ## Redirects are re-checked, every hop
//!
//! A grant for `api.example.com` is a grant for `api.example.com`. If that host
//! answers `302 Location: https://evil.test/`, following it would turn the
//! grant into "anywhere `api.example.com` cares to point", and the user who
//! approved one hostname would have approved all of them. Worse, the widget
//! never has to be complicit: a compromised or merely careless server does it.
//!
//! So redirects are followed **by this code, not by the client library**, and
//! every hop goes back through the same [`Policy`] the first request did. A hop
//! to an ungranted host stops the chain and reports what happened, which is a
//! fact the widget's author can act on and its user might want to know.
//!
//! ## What a widget cannot do
//!
//! * **Speak anything but HTTPS.** Plain HTTP is refused before the policy is
//!   even consulted — a new framework has no reason to make unencrypted
//!   requests possible, and a downgrade *via redirect* is the interesting case
//!   this refuses too.
//! * **Set a header, a method, or a body.** The interface is a URL. There are
//!   no cookies, no credentials, and nothing a widget could use to be tracked
//!   across the requests of other widgets.
//! * **Take the desktop's time or memory.** A request has a deadline and a
//!   response has a cap, and both are the host's.

use std::time::Duration;

use crate::capability::{Policy, Request};
use crate::tasks::{HttpResponse, TaskResult};

/// How long one request may take, including every redirect.
///
/// A widget never waits on this — the whole request runs on a worker — so the
/// bound is on how long one widget may occupy that worker, not on how long a
/// user is kept waiting.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// The most body one response may carry.
///
/// A widget's own memory is capped by its manifest, but the *host* reads the
/// body first. Without this, a granted host serving an endless stream is a way
/// to exhaust the desktop rather than the widget.
pub const MAX_BODY: usize = 4 * 1024 * 1024;

/// How many redirects to follow before giving up.
///
/// Enough for the ordinary `example.com` → `www.example.com` → canonical-path
/// chain, and short enough that a loop between two granted hosts ends.
pub const MAX_REDIRECTS: usize = 5;

/// What uniwidged calls itself to a server.
///
/// Named rather than anonymous, and deliberately carrying no version of the
/// widget or of the machine: a server should be able to see what is calling it
/// and nothing about who.
const USER_AGENT: &str = concat!("uniwidged/", env!("CARGO_PKG_VERSION"));

/// The client behind the `net` capability.
pub struct Http {
    agent: ureq::Agent,
    timeout: Duration,
    max_body: usize,
    max_redirects: usize,
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Http {
    pub fn new() -> Self {
        Self::with_agent(
            ureq::Agent::config_builder()
                // Followed here instead — see the module note. This is the
                // single most important line in the file.
                .max_redirects(0)
                .timeout_global(Some(TIMEOUT))
                .user_agent(USER_AGENT)
                .build()
                .into(),
        )
    }

    /// A client over an agent the embedder prepared.
    ///
    /// The default trusts Mozilla's root bundle, which is predictable and the
    /// same everywhere. A desktop behind a corporate proxy, or one with an
    /// internal CA the user installed, wants the *platform* store instead —
    /// and that is a decision for whoever is packaging uniwidged rather than
    /// for this file, so it is a constructor.
    ///
    /// It is also what the tests use, pointing it at a server they started
    /// themselves: a transport that can only be exercised against the real
    /// internet is one that is not exercised.
    pub fn with_agent(agent: ureq::Agent) -> Self {
        Self {
            agent,
            timeout: TIMEOUT,
            max_body: MAX_BODY,
            max_redirects: MAX_REDIRECTS,
        }
    }

    pub fn with_max_body(mut self, bytes: usize) -> Self {
        self.max_body = bytes;
        self
    }

    pub fn with_max_redirects(mut self, hops: usize) -> Self {
        self.max_redirects = hops;
        self
    }

    /// Fetches a URL. **Called on a worker thread, never on the loop.**
    ///
    /// `policy` is the widget's own, and is consulted again at every redirect.
    pub fn get(&self, url: &str, policy: &Policy) -> TaskResult {
        let mut url = url.to_string();
        for hop in 0..=self.max_redirects {
            // Every hop, including the first: the caller checked the first URL
            // too, and checking it twice is cheaper than a rule that holds only
            // when somebody remembered to apply it.
            if let Err(refused) = allowed(&url, policy) {
                return TaskResult::Failed(if hop == 0 {
                    refused
                } else {
                    format!("redirected to somewhere this widget was not granted: {refused}")
                });
            }

            let response = match self.agent.get(&url).call() {
                Ok(response) => response,
                // ureq treats a 4xx or 5xx as an error by default; those are
                // answers, and a widget should see the status rather than a
                // sentence about it.
                Err(ureq::Error::StatusCode(code)) => {
                    return TaskResult::Http(HttpResponse {
                        status: code,
                        body: Vec::new(),
                    });
                }
                Err(e) => return TaskResult::Failed(describe(&e)),
            };

            let status = response.status().as_u16();
            if let Some(next) = redirect_target(&url, &response) {
                if hop == self.max_redirects {
                    return TaskResult::Failed(format!(
                        "more than {} redirects",
                        self.max_redirects
                    ));
                }
                url = next;
                continue;
            }

            use std::io::Read;
            let mut body = Vec::new();
            // Capped by the reader rather than by checking a header: a server
            // may lie about `Content-Length`, or send none at all.
            let read = response
                .into_body()
                .into_reader()
                .take(self.max_body as u64)
                .read_to_end(&mut body);
            if let Err(e) = read {
                return TaskResult::Failed(format!("the response ended badly: {e}"));
            }
            return TaskResult::Http(HttpResponse { status, body });
        }
        TaskResult::Failed(format!("more than {} redirects", self.max_redirects))
    }
}

/// May this widget reach this URL?
///
/// `Err` carries the reason, because the two ways to fail mean different things
/// to a widget author: a URL that is not HTTPS is theirs to fix, and a host
/// that was not granted is their manifest's.
fn allowed(url: &str, policy: &Policy) -> Result<(), String> {
    let Some(host) = host_of(url) else {
        return Err(format!("`{url}` is not an https:// URL"));
    };
    policy
        .check(&Request::Net { host: host.clone() })
        .map_err(|_| format!("`{host}` is not a host this widget was granted"))
}

/// Where a response says to go next, resolved against the URL it came from.
///
/// `None` for anything that is not a redirect, or a redirect with no usable
/// `Location`. A relative `Location` is resolved against the current URL, which
/// is what every server assumes and what makes `Location: /v2/thing` work.
fn redirect_target(from: &str, response: &ureq::http::Response<ureq::Body>) -> Option<String> {
    let status = response.status().as_u16();
    if !(300..400).contains(&status) {
        return None;
    }
    let location = response.headers().get("location")?.to_str().ok()?.trim();
    resolve(from, location)
}

/// Resolves a `Location` against the URL it was served from.
///
/// Deliberately small, and deliberately refusing what it does not understand: a
/// URL parser that guesses is a URL parser that eventually disagrees with the
/// one the policy check uses, and the two disagreeing is the whole of this
/// file's threat model.
fn resolve(from: &str, location: &str) -> Option<String> {
    if location.is_empty() {
        return None;
    }
    // Absolute. Any scheme at all — including `http:` — is returned as it
    // stands, so that `allowed` refuses it rather than this function silently
    // dropping a downgrade.
    if location.contains("://") {
        return Some(location.to_string());
    }
    let rest = from.strip_prefix("https://")?;
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    if let Some(absolute) = location.strip_prefix('/') {
        return Some(format!("https://{authority}/{absolute}"));
    }
    // Relative to the current directory. Everything after the last separator
    // is the document, and is replaced.
    let directory = match path.rfind('/') {
        Some(at) => &path[..=at],
        None => "/",
    };
    Some(format!("https://{authority}{directory}{location}"))
}

/// The host part of an `https://` URL, lower-cased.
///
/// The same reading the capability check uses, and it lives here so the two
/// cannot drift: a redirect judged against one parse and fetched against
/// another is exactly the hole this file exists to close.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest
        .split(['/', '?', '#'])
        .next()?
        // `user@host` — the userinfo is discarded, and the *last* `@` wins,
        // because `https://evil.test@granted.example/` is a URL whose host is
        // `granted.example` and whose purpose is to look like something else.
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// A transport failure, in words a widget author can act on.
fn describe(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(_) => "the request timed out".to_string(),
        ureq::Error::HostNotFound => "that host could not be found".to_string(),
        ureq::Error::ConnectionFailed => "could not connect to that host".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
