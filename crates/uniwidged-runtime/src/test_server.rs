//! A TLS server the transport tests point at.
//!
//! A hundred lines rather than a dependency, and worth it: the alternative is
//! testing an HTTPS client against the real internet, which makes the suite
//! slow, flaky, and silently green on a machine with no network. Here the
//! server answers a fixed set of paths — including the redirects the security
//! properties are about — plus whatever [`TestServer::serve`] was given, and
//! the client is given its certificate and no other.
//!
//! ## Why it is not inside one module's tests
//!
//! It was, until a registry needed it: an index, a component and two
//! signatures, all over HTTPS, are what `uniwidged add` fetches, and testing
//! that against anything but a real TLS server tests something else. A second
//! copy would have been a second server, drifting from this one — so it moved
//! here and is offered behind the `test-support` feature, which is off unless
//! a `[dev-dependencies]` line asks for it. Nothing in a release build sees it.
//!
//! The certificate comes from the `openssl` command rather than a crate,
//! because generating one is a test convenience and not worth a dependency in
//! the shipped tree. Without `openssl` the tests skip, and
//! `UNIWIDGED_REQUIRE_NET=1` turns that into a failure.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What the server will answer at a path it was given by name.
type Routes = Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;

pub struct TestServer {
    port: u16,
    certificate: Vec<u8>,
    stop: Arc<AtomicBool>,
    directory: std::path::PathBuf,
    routes: Routes,
}

impl TestServer {
    /// `None` when `openssl` is not there to make a certificate with.
    pub fn start() -> Option<Self> {
        use std::sync::atomic::AtomicU32;
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "uniwidged-tls-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).ok()?;

        // A CA and a leaf it signed, not one self-signed certificate.
        // webpki refuses a CA certificate used as an end-entity — the error is
        // `CaUsedAsEndEntity`, and it is correct: a trust anchor and a server
        // identity are different things, and `openssl req -x509` makes the
        // former by default. The client below trusts the CA; the server
        // presents the leaf.
        let ca_key = directory.join("ca.key");
        let ca_cert = directory.join("ca.pem");
        let leaf_key = directory.join("leaf.key");
        let leaf_csr = directory.join("leaf.csr");
        let leaf_cert = directory.join("leaf.pem");
        let extensions = directory.join("leaf.ext");
        std::fs::write(
            &extensions,
            // `CA:FALSE` and `serverAuth` are what make this a leaf rather than
            // another authority.
            "subjectAltName=DNS:localhost\n\
             basicConstraints=critical,CA:FALSE\n\
             extendedKeyUsage=serverAuth\n",
        )
        .ok()?;

        let openssl = |args: Vec<&std::ffi::OsStr>| -> bool {
            matches!(
                std::process::Command::new("openssl")
                    .args(args)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status(),
                Ok(status) if status.success()
            )
        };
        use std::ffi::OsStr;
        let made = openssl(vec![
            OsStr::new("req"),
            OsStr::new("-x509"),
            OsStr::new("-newkey"),
            OsStr::new("rsa:2048"),
            OsStr::new("-nodes"),
            OsStr::new("-days"),
            OsStr::new("1"),
            OsStr::new("-subj"),
            OsStr::new("/CN=uniwidged test CA"),
            OsStr::new("-keyout"),
            ca_key.as_os_str(),
            OsStr::new("-out"),
            ca_cert.as_os_str(),
        ]) && openssl(vec![
            OsStr::new("req"),
            OsStr::new("-newkey"),
            OsStr::new("rsa:2048"),
            OsStr::new("-nodes"),
            OsStr::new("-subj"),
            OsStr::new("/CN=localhost"),
            OsStr::new("-keyout"),
            leaf_key.as_os_str(),
            OsStr::new("-out"),
            leaf_csr.as_os_str(),
        ]) && openssl(vec![
            OsStr::new("x509"),
            OsStr::new("-req"),
            OsStr::new("-in"),
            leaf_csr.as_os_str(),
            OsStr::new("-CA"),
            ca_cert.as_os_str(),
            OsStr::new("-CAkey"),
            ca_key.as_os_str(),
            OsStr::new("-CAcreateserial"),
            OsStr::new("-days"),
            OsStr::new("1"),
            OsStr::new("-out"),
            leaf_cert.as_os_str(),
            OsStr::new("-extfile"),
            extensions.as_os_str(),
        ]);

        if !made {
            assert!(
                std::env::var("UNIWIDGED_REQUIRE_NET").is_err(),
                "UNIWIDGED_REQUIRE_NET is set and no certificate could be made"
            );
            eprintln!("skipping: no openssl to make a test certificate with");
            return None;
        }

        use rustls::pki_types::pem::PemObject;
        // What the client will trust.
        let ca_der = rustls::pki_types::CertificateDer::from_pem_file(&ca_cert).ok()?;
        // What the server will present.
        let leaf_der = rustls::pki_types::CertificateDer::from_pem_file(&leaf_cert).ok()?;
        let private = rustls::pki_types::PrivateKeyDer::from_pem_file(&leaf_key).ok()?;

        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .ok()?
        .with_no_client_auth()
        .with_single_cert(vec![leaf_der], private)
        .ok()?;

        // Port zero: the operating system picks one nothing else is using,
        // which is what lets these tests run beside each other.
        let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        let stop = Arc::new(AtomicBool::new(false));

        let running = Arc::clone(&stop);
        let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
        let serving = Arc::clone(&routes);
        let config = Arc::new(config);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if running.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(mut stream) = stream else { continue };
                let config = Arc::clone(&config);
                let serving = Arc::clone(&serving);
                std::thread::spawn(move || {
                    let Ok(mut tls) = rustls::ServerConnection::new(config) else {
                        return;
                    };
                    let mut socket = rustls::Stream::new(&mut tls, &mut stream);
                    let mut request = [0u8; 2048];
                    let Ok(read) = socket.read(&mut request) else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request[..read]);
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let served = serving
                        .lock()
                        .ok()
                        .and_then(|routes| routes.get(&path).cloned());
                    let _ = socket.write_all(&match served {
                        Some((status, body)) => response(status, "", &body),
                        None => answer(&path),
                    });
                    let _ = socket.flush();
                });
            }
        });

        Some(Self {
            port,
            certificate: ca_der.to_vec(),
            stop,
            directory,
            routes,
        })
    }

    /// Answers `path` with these bytes, from now on.
    ///
    /// Bytes rather than a string, because what a registry serves is a `.wasm`
    /// component — and a test that could only serve text would be a test that
    /// never carried the thing being installed.
    pub fn serve(&self, path: &str, body: impl Into<Vec<u8>>) {
        self.answer_with(path, 200, body);
    }

    /// The same, with a status of the caller's choosing.
    pub fn answer_with(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.routes
            .lock()
            .expect("the routes are only ever locked briefly")
            .insert(path.to_string(), (status, body.into()));
    }

    /// Stops answering a path, so a test can watch a fetch fail.
    pub fn forget(&self, path: &str) {
        self.routes
            .lock()
            .expect("the routes are only ever locked briefly")
            .remove(path);
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.port)
    }

    /// The **authority** to trust, not the server's own certificate — see the
    /// note in `start` on why those are different things.
    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }

    /// The same authority as PEM, for a client that takes a file.
    ///
    /// The CLI is one: a registry under a private authority is configured with
    /// `UNIWIDGED_REGISTRY_CA`, which names a file, so a test driving the real
    /// binary needs one to name.
    pub fn certificate_pem(&self) -> String {
        let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
        let base64 = base64(&self.certificate);
        for line in base64.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
            out.push('\n');
        }
        out.push_str("-----END CERTIFICATE-----\n");
        out
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Twenty lines rather than a dependency, for the one place a certificate has
/// to be written out as text.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let bits = u32::from(block[0]) << 16 | u32::from(block[1]) << 8 | u32::from(block[2]);
        for index in 0..4 {
            // A chunk of one byte carries two characters, of two bytes three;
            // the rest is padding.
            if index <= chunk.len() {
                out.push(ALPHABET[(bits >> (18 - index * 6) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The fixed set of answers. Each path is one property under test.
fn answer(path: &str) -> Vec<u8> {
    let (status, headers, body): (&str, String, String) = match path {
        "/hello" => ("200 OK", String::new(), "hello".into()),
        "/missing" => ("404 Not Found", String::new(), "gone".into()),
        // Within the granted host: the ordinary case, and it must be followed.
        "/redirect-local" => ("302 Found", "Location: /hello\r\n".into(), String::new()),
        // Off the granted host: must not be followed, however the widget asked.
        "/redirect-away" => (
            "302 Found",
            "Location: https://evil.test/steal\r\n".into(),
            String::new(),
        ),
        // The same host, downgraded. Also must not be followed.
        "/redirect-insecure" => (
            "302 Found",
            "Location: http://localhost/hello\r\n".into(),
            String::new(),
        ),
        // Points at itself: the policy cannot stop this, so the hop count must.
        "/loop" => ("302 Found", "Location: /loop\r\n".into(), String::new()),
        "/large" => ("200 OK", String::new(), "x".repeat(64 * 1024)),
        _ => ("404 Not Found", String::new(), String::new()),
    };
    let status = status
        .split_whitespace()
        .next()
        .and_then(|code| code.parse().ok())
        .unwrap_or(200);
    response(status, &headers, body.as_bytes())
}

/// One HTTP/1.1 answer, with a body that need not be text.
fn response(status: u16, headers: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        302 => "Found",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Something",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}
