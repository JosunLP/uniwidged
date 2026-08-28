//! What a granted host does not grant.
//!
//! The pure half first — reading a URL, and resolving a `Location` — because
//! the policy check and the fetch have to agree about what a host *is*, and two
//! parsers disagreeing is this module's whole threat model. Then the real half:
//! a TLS server the test starts, and a client pointed at it, so that "a
//! redirect to an ungranted host is refused" is something observed rather than
//! read off the source.

use super::*;

fn granting(hosts: &[&str]) -> Policy {
    Policy::from_granted(
        hosts
            .iter()
            .map(|host| crate::capability::Capability::Net(host.to_string())),
    )
}

#[test]
fn a_url_is_read_the_same_way_the_policy_reads_it() {
    assert_eq!(
        host_of("https://api.example.com/v1"),
        Some("api.example.com".into())
    );
    assert_eq!(
        host_of("https://API.Example.COM"),
        Some("api.example.com".into())
    );
    assert_eq!(
        host_of("https://example.com:8443/x"),
        Some("example.com".into())
    );
    assert_eq!(
        host_of("https://example.com?q=1"),
        Some("example.com".into())
    );
    assert_eq!(
        host_of("https://example.com#top"),
        Some("example.com".into())
    );
}

#[test]
fn a_url_that_is_not_https_has_no_host_at_all() {
    // Refused before the policy is consulted, so a widget is told the URL is
    // wrong rather than that its manifest is.
    for not_https in [
        "http://example.com",
        "file:///etc/passwd",
        "ftp://example.com",
        "example.com",
        "",
        "https://",
    ] {
        assert_eq!(host_of(not_https), None, "{not_https:?}");
    }
}

#[test]
fn userinfo_does_not_disguise_the_host() {
    // `https://granted.example@evil.test/` is a URL whose host is `evil.test`.
    // A reader takes the part before the `@` for the host; so does a naive
    // parser, and then the policy check passes and the request goes somewhere
    // else entirely.
    assert_eq!(
        host_of("https://granted.example@evil.test/"),
        Some("evil.test".into())
    );
    assert_eq!(
        host_of("https://a@b@evil.test/"),
        Some("evil.test".into()),
        "the last `@` wins, as it does in the URL standard"
    );
    let policy = granting(&["granted.example"]);
    assert!(allowed("https://granted.example@evil.test/", &policy).is_err());
    assert!(allowed("https://granted.example/", &policy).is_ok());
}

#[test]
fn a_location_header_is_resolved_the_way_a_server_expects() {
    let from = "https://example.com/a/b/page";
    assert_eq!(
        resolve(from, "https://other.test/x").as_deref(),
        Some("https://other.test/x"),
        "absolute wins"
    );
    assert_eq!(
        resolve(from, "/root").as_deref(),
        Some("https://example.com/root"),
        "a leading slash is the site root"
    );
    assert_eq!(
        resolve(from, "sibling").as_deref(),
        Some("https://example.com/a/b/sibling"),
        "everything after the last slash is the document, and is replaced"
    );
    assert_eq!(
        resolve("https://example.com", "thing").as_deref(),
        Some("https://example.com/thing"),
        "a URL with no path has the root as its directory"
    );
    assert_eq!(resolve(from, ""), None, "an empty Location says nothing");
}

#[test]
fn a_redirect_that_downgrades_to_http_is_carried_through_to_be_refused() {
    // Not dropped here. `resolve` returning `None` would make the chain end
    // quietly, and a widget would see a 302 as its answer; returning the URL
    // means `allowed` refuses it and says why.
    let target = resolve("https://example.com/a", "http://example.com/a").expect("carried");
    assert_eq!(target, "http://example.com/a");
    assert!(
        allowed(&target, &granting(&["example.com"])).is_err(),
        "the same host over plain HTTP is still not something to fetch"
    );
}

#[test]
fn a_grant_for_one_host_is_not_a_grant_for_a_host_that_contains_it() {
    // The suffix attack, at this layer too — the check is `Policy`'s, and this
    // is the assertion that this file really asks it.
    let policy = granting(&["example.com"]);
    for hostile in [
        "https://example.com.evil.test/",
        "https://notexample.com/",
        "https://sub.example.com/",
    ] {
        assert!(allowed(hostile, &policy).is_err(), "{hostile}");
    }
}

// ---------------------------------------------------------------------------
// Against a real server
// ---------------------------------------------------------------------------

mod server;

use server::TestServer;

/// A client that trusts only the test server's certificate.
fn client(server: &TestServer) -> Http {
    let agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::Specific(std::sync::Arc::new(vec![
                    ureq::tls::Certificate::from_der(server.certificate_der()).to_owned(),
                ])))
                .build(),
        )
        .build()
        .into();
    Http::with_agent(agent)
}

fn body_of(result: TaskResult) -> (u16, String) {
    match result {
        TaskResult::Http(response) => (
            response.status,
            String::from_utf8_lossy(&response.body).to_string(),
        ),
        other => panic!("expected a response, got {other:?}"),
    }
}

#[test]
fn a_granted_host_can_be_fetched() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    let (status, body) = body_of(client(&server).get(&server.url("/hello"), &policy));
    assert_eq!(status, 200);
    assert_eq!(body, "hello");
}

#[test]
fn a_redirect_within_the_granted_host_is_followed() {
    // The ordinary case, and the reason redirects are followed at all rather
    // than refused outright: `example.com` → `example.com/canonical` is what
    // half the internet does.
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    let (status, body) = body_of(client(&server).get(&server.url("/redirect-local"), &policy));
    assert_eq!(status, 200);
    assert_eq!(body, "hello");
}

#[test]
fn a_redirect_to_an_ungranted_host_is_refused_and_says_so() {
    // **The assertion this file exists for.** A grant for one host must not
    // become a grant for wherever that host points — and the widget need not
    // be complicit for it to happen, because the server decides.
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    match client(&server).get(&server.url("/redirect-away"), &policy) {
        TaskResult::Failed(why) => {
            assert!(
                why.contains("redirected"),
                "it should say what happened: {why}"
            );
            assert!(why.contains("evil.test"), "and where to: {why}");
        }
        other => panic!("a redirect off the granted host must not be followed, got {other:?}"),
    }
}

#[test]
fn a_redirect_that_downgrades_to_plain_http_is_refused() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    match client(&server).get(&server.url("/redirect-insecure"), &policy) {
        TaskResult::Failed(why) => assert!(why.contains("redirected"), "{why}"),
        other => panic!("a downgrade must not be followed, got {other:?}"),
    }
}

#[test]
fn a_redirect_loop_ends() {
    // Two granted URLs pointing at each other is a chain the policy cannot
    // stop, so the hop count has to.
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    match client(&server)
        .with_max_redirects(3)
        .get(&server.url("/loop"), &policy)
    {
        TaskResult::Failed(why) => assert!(why.contains("redirects"), "{why}"),
        other => panic!("a loop should end, got {other:?}"),
    }
}

#[test]
fn a_response_larger_than_the_cap_is_truncated_rather_than_read() {
    // The host reads the body before the widget does, so an endless stream
    // from a granted server is a way to exhaust the desktop rather than the
    // widget.
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    let (status, body) = body_of(
        client(&server)
            .with_max_body(64)
            .get(&server.url("/large"), &policy),
    );
    assert_eq!(status, 200);
    assert_eq!(body.len(), 64);
}

#[test]
fn a_status_a_server_returns_is_an_answer_rather_than_an_error() {
    // A widget should be able to tell "the API said 404" from "the API could
    // not be reached", and draw them differently.
    let Some(server) = TestServer::start() else {
        return;
    };
    let policy = granting(&["localhost"]);
    let (status, _) = body_of(client(&server).get(&server.url("/missing"), &policy));
    assert_eq!(status, 404);
}

#[test]
fn an_ungranted_host_is_never_even_connected_to() {
    // The check happens before the request, which is why this passes with no
    // server listening on the other name at all.
    let policy = granting(&["localhost"]);
    match Http::new().get("https://ungranted.invalid/", &policy) {
        TaskResult::Failed(why) => assert!(why.contains("not a host this widget was granted")),
        other => panic!("{other:?}"),
    }
}
