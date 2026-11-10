//! Finding a widget, and taking it.
//!
//! Everything here runs the real binary against a real TLS server: an index
//! that was really signed, a component that really arrives over HTTPS, and a
//! configuration that really ends up naming it. The properties being checked
//! are the ones a registry could otherwise quietly get wrong —
//!
//! * an index that is not signed by the pinned key is not read at all;
//! * a component swapped after the index was signed is refused;
//! * a registry cannot install a widget past the machine's own trusted keys;
//! * an update keeps where a widget sat and what it was set to;
//! * an update that would widen a permission is *not* applied unattended.
//!
//! The publisher's key and the author's key are two different keys throughout,
//! because that is the distinction the whole design rests on.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use uniwidged_runtime::registry::{Entry, write_index};
use uniwidged_runtime::signature::{self, SecretKey, SignatureFile};
use uniwidged_runtime::test_server::TestServer;
use uniwidged_runtime::version::Version;

/// A configuration directory that removes itself, standing in for a user's.
struct Machine {
    root: PathBuf,
    /// The authority the test server presents, written where the CLI can be
    /// pointed at it.
    ca: PathBuf,
}

impl Machine {
    fn new(what: &str, server: &TestServer) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "uniwidged-registry-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a scratch directory");
        let ca = root.join("ca.pem");
        std::fs::write(&ca, server.certificate_pem()).expect("the authority");
        Self { root, ca }
    }

    fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_uniwidged"))
            .args(args)
            .env("UNIWIDGED_CONFIG", self.config())
            // The test server's own authority, and only it. A registry inside
            // an organisation is configured exactly this way.
            .env("UNIWIDGED_REGISTRY_CA", &self.ca)
            .output()
            .expect("the CLI should be runnable")
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "`uniwidged {}` failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn fails(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            !output.status.success(),
            "`uniwidged {}` should have failed, and said: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout)
        );
        String::from_utf8_lossy(&output.stderr).to_string()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The clock, built once per test run and read into memory.
///
/// Built rather than looked for: a suite that skips when an artefact is
/// missing eventually always skips, and nobody notices until it matters.
fn clock() -> Vec<u8> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../widgets/clock");
    let built = Command::new(env!("CARGO"))
        .current_dir(&source)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("cargo");
    assert!(built.success(), "the clock widget should build");
    std::fs::read(source.join("target/wasm32-wasip2/release/clock_widget.wasm"))
        .expect("the component")
}

/// A manifest for the clock at a version, asking for what it is given.
fn manifest_of(version: &str, capabilities: &[&str]) -> String {
    let capabilities: Vec<String> = capabilities.iter().map(|c| format!("\"{c}\"")).collect();
    format!(
        "name = \"clock\"\nversion = \"{version}\"\n\
         description = \"The time and the date, in your locale\"\n\
         capabilities = [{}]\nmemory-mb = 8\n",
        capabilities.join(", ")
    )
}

/// A registry: one publisher, one author, and whatever versions are asked for.
struct Published {
    publisher: SecretKey,
    author: SecretKey,
    component: Vec<u8>,
}

impl Published {
    fn new() -> Self {
        Self {
            publisher: SecretKey::generate().unwrap(),
            author: SecretKey::generate().unwrap(),
            component: clock(),
        }
    }

    /// Serves an index offering these versions, and every file it names.
    fn serve(&self, server: &TestServer, versions: &[(&str, &[&str])]) -> String {
        let mut entries = Vec::new();
        for (version, capabilities) in versions {
            let manifest = manifest_of(version, capabilities);
            let stem = format!("clock-{version}");
            server.serve(&format!("/{stem}.wasm"), self.component.clone());
            server.serve(&format!("/{stem}.toml"), manifest.clone());
            server.serve(
                &format!("/{stem}.wasm.sig"),
                SignatureFile {
                    signature: self.author.sign(&self.component, &manifest),
                    key: self.author.public(),
                }
                .to_text(),
            );
            entries.push(Entry {
                name: "clock".into(),
                version: Version::parse(version).unwrap(),
                description: "The time and the date, in your locale".into(),
                capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                component: server.url(&format!("/{stem}.wasm")),
                manifest: server.url(&format!("/{stem}.toml")),
                signature: Some(server.url(&format!("/{stem}.wasm.sig"))),
                digest: signature::digest(&self.component, &manifest),
            });
        }
        let index = write_index(&entries);
        server.serve("/index.toml", index.clone());
        server.serve(
            "/index.toml.sig",
            SignatureFile {
                signature: self.publisher.sign_index(&index),
                key: self.publisher.public(),
            }
            .to_text(),
        );
        index
    }

    /// Adds the registry and trusts the author, which is where every test
    /// below starts.
    fn set_up(&self, machine: &Machine, server: &TestServer) {
        machine.ok(&[
            "registry",
            "add",
            &server.url("/index.toml"),
            "--key",
            &self.publisher.public().to_hex(),
            "--name",
            "example",
        ]);
        machine.ok(&[
            "trust",
            &self.author.public().to_hex(),
            "--name",
            "the author",
        ]);
    }
}

#[test]
fn a_widget_is_found_and_installed_from_a_registry() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("install", &server);
    let published = Published::new();
    published.serve(&server, &[("1.0.0", &["timer", "locale"])]);

    let said = machine.ok(&[
        "registry",
        "add",
        &server.url("/index.toml"),
        "--key",
        &published.publisher.public().to_hex(),
        "--name",
        "example",
    ]);
    assert!(said.contains("1 widget offered"), "{said}");

    // What a person sees before deciding: the version, what it does, and the
    // permissions in the words the consent screen will use.
    let said = machine.ok(&["search", "clock"]);
    assert!(said.contains("clock 1.0.0"), "{said}");
    assert!(said.contains("The time and the date"), "{said}");
    assert!(said.contains("format dates and times"), "{said}");

    // A registry cannot vouch a widget past the machine's own trusted keys.
    let why = machine.fails(&["add", "clock"]);
    assert!(
        why.contains("not signed") || why.contains("does not trust"),
        "{why}"
    );

    machine.ok(&[
        "trust",
        &published.author.public().to_hex(),
        "--name",
        "the author",
    ]);
    let said = machine.ok(&["add", "clock"]);
    assert!(said.contains("signed by the author"), "{said}");
    assert!(said.contains("installed"), "{said}");
    assert!(machine.ok(&["list"]).contains("clock"));

    // And it was copied into the store rather than left wherever it landed.
    let stored = machine.root.join("store/clock-1.0.0.wasm");
    assert!(stored.exists(), "{} should exist", stored.display());
    let recorded = std::fs::read_to_string(machine.config()).unwrap();
    // Whatever separator this platform writes between the two.
    let stored_as = std::path::Path::new("store").join("clock-1.0.0.wasm");
    assert!(
        recorded.contains(&*stored_as.to_string_lossy()),
        "{recorded}"
    );
}

#[test]
fn a_registry_pinned_to_the_wrong_key_is_not_read_at_all() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("wrong-key", &server);
    let published = Published::new();
    published.serve(&server, &[("1.0.0", &["timer", "locale"])]);

    let somebody_else = SecretKey::generate().unwrap();
    let why = machine.fails(&[
        "registry",
        "add",
        &server.url("/index.toml"),
        "--key",
        &somebody_else.public().to_hex(),
    ]);
    assert!(why.contains("does not verify"), "{why}");
}

#[test]
fn an_index_edited_after_it_was_signed_is_refused() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("edited-index", &server);
    let published = Published::new();
    let index = published.serve(&server, &[("1.0.0", &["timer", "locale"])]);
    published.set_up(&machine, &server);

    // Somebody on the path widens what the listing claims. The signature
    // beside it is the publisher's, over the list they wrote.
    server.serve(
        "/index.toml",
        index.replace(
            "\"timer\", \"locale\"",
            "\"timer\", \"locale\", \"secrets\"",
        ),
    );
    let why = machine.fails(&["search", "clock", "--refresh"]);
    assert!(why.contains("does not verify"), "{why}");
}

#[test]
fn a_component_swapped_after_the_index_was_signed_is_refused() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("swapped", &server);
    let published = Published::new();
    published.serve(&server, &[("1.0.0", &["timer", "locale"])]);
    published.set_up(&machine, &server);

    // The list still verifies; the file it points at is not the file it was
    // pointing at when it was signed.
    server.serve("/clock-1.0.0.wasm", b"\0asm not the clock at all".to_vec());
    let why = machine.fails(&["add", "clock"]);
    assert!(why.contains("not what the index said"), "{why}");
    assert!(
        !machine.config().exists()
            || !std::fs::read_to_string(machine.config())
                .unwrap()
                .contains("clock"),
        "and nothing was installed"
    );
}

#[test]
fn an_update_is_offered_applied_and_keeps_where_the_widget_sat() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("update", &server);
    let published = Published::new();
    published.serve(&server, &[("1.0.0", &["timer", "locale"])]);
    published.set_up(&machine, &server);
    machine.ok(&["add", "clock"]);

    // Five minutes of somebody's afternoon: put it where they want it.
    machine.ok(&["move", "clock", "--x", "900", "--y", "40"]);
    assert!(machine.ok(&["list"]).contains("at 900,40"));
    assert!(
        machine
            .ok(&["update", "--check"])
            .contains("newest version")
    );

    published.serve(
        &server,
        &[
            ("1.0.0", &["timer", "locale"]),
            ("1.1.0", &["timer", "locale"]),
        ],
    );
    let said = machine.ok(&["update", "--check", "--refresh"]);
    assert!(said.contains("1.0.0 → 1.1.0"), "{said}");
    // `--check` looks and does not touch.
    assert!(machine.ok(&["list"]).contains("at 900,40"));
    assert!(!machine.root.join("store/clock-1.1.0.wasm").exists());

    let said = machine.ok(&["update"]);
    assert!(said.contains("1.0.0 → 1.1.0"), "{said}");
    let listed = machine.ok(&["list"]);
    assert!(
        listed.contains("at 900,40"),
        "the widget should not move: {listed}"
    );
    let recorded = std::fs::read_to_string(machine.config()).unwrap();
    assert!(recorded.contains("clock-1.1.0.wasm"), "{recorded}");
}

#[test]
fn an_update_that_asks_for_more_is_reported_rather_than_applied() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("widening", &server);
    let published = Published::new();
    published.serve(&server, &[("1.0.0", &["timer", "locale"])]);
    published.set_up(&machine, &server);
    machine.ok(&["add", "clock"]);

    // The new version wants the keychain. Nobody has agreed to that, and an
    // unattended command is the last place to decide it.
    published.serve(
        &server,
        &[
            ("1.0.0", &["timer", "locale"]),
            ("2.0.0", &["timer", "locale", "secrets"]),
        ],
    );
    let said = machine.ok(&["update", "--refresh"]);
    assert!(
        said.contains("asks for something the installed one did not"),
        "{said}"
    );
    assert!(said.contains("secrets"), "{said}");
    assert!(
        said.contains("uniwidged add clock"),
        "it should say the way through: {said}"
    );

    // Still on the version that was agreed to.
    let recorded = std::fs::read_to_string(machine.config()).unwrap();
    assert!(recorded.contains("clock-1.0.0.wasm"), "{recorded}");
    assert!(!recorded.contains("secrets"), "{recorded}");

    // And `add`, which shows the whole list, is the way to take it.
    let said = machine.ok(&["add", "clock"]);
    assert!(said.contains("store passwords in your keychain"), "{said}");
    let recorded = std::fs::read_to_string(machine.config()).unwrap();
    assert!(recorded.contains("clock-2.0.0.wasm"), "{recorded}");
}

#[test]
fn a_publisher_writes_an_index_with_the_tool_and_a_machine_reads_it() {
    // `registry publish` and `registry add` are the two ends of the format,
    // and this is the only test that runs both — so a change to the writer
    // that the parser would refuse fails here.
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("publish", &server);
    let component = clock();

    // A publisher's directory: the widget, its manifest, its signature.
    let workshop = machine.root.join("workshop");
    std::fs::create_dir_all(&workshop).unwrap();
    std::fs::write(workshop.join("clock.wasm"), &component).unwrap();
    std::fs::write(
        workshop.join("clock.toml"),
        manifest_of("1.0.0", &["timer", "locale"]),
    )
    .unwrap();
    let author_key = workshop.join("author-key");
    let author = machine.ok(&["keygen", "--out", author_key.to_str().unwrap()]);
    let author = public_key_from(&author);
    machine.ok(&[
        "sign",
        workshop.join("clock.wasm").to_str().unwrap(),
        "--key",
        author_key.to_str().unwrap(),
    ]);

    // And the publisher's own key, which is a different key.
    let publisher_key = workshop.join("publisher-key");
    let publisher = machine.ok(&["keygen", "--out", publisher_key.to_str().unwrap()]);
    let publisher = public_key_from(&publisher);

    let index = workshop.join("index.toml");
    let said = machine.ok(&[
        "registry",
        "publish",
        "--key",
        publisher_key.to_str().unwrap(),
        "--base-url",
        &server.url(""),
        "--out",
        index.to_str().unwrap(),
        workshop.join("clock.wasm").to_str().unwrap(),
    ]);
    assert!(said.contains("listing clock 1.0.0"), "{said}");

    // Serve what it wrote, exactly as a static host would.
    server.serve("/index.toml", std::fs::read(&index).unwrap());
    server.serve(
        "/index.toml.sig",
        std::fs::read(format!("{}.sig", index.display())).unwrap(),
    );
    server.serve("/clock-1.0.0.wasm", component);
    server.serve(
        "/clock-1.0.0.toml",
        std::fs::read(workshop.join("clock.toml")).unwrap(),
    );
    server.serve(
        "/clock-1.0.0.wasm.sig",
        std::fs::read(workshop.join("clock.wasm.sig")).unwrap(),
    );

    machine.ok(&[
        "registry",
        "add",
        &server.url("/index.toml"),
        "--key",
        &publisher,
        "--name",
        "example",
    ]);
    machine.ok(&["trust", &author]);
    let said = machine.ok(&["add", "clock"]);
    assert!(said.contains("installed"), "{said}");
    assert!(machine.ok(&["list"]).contains("clock"));
}

#[test]
fn publishing_refuses_a_widget_whose_signature_does_not_cover_it() {
    // A publisher's mistake rather than an attack: the manifest was edited
    // after the widget was signed, which is the ordinary shape of it. Listing
    // it would publish a download every client rejects — and the client's
    // refusal would look like tampering to whoever tried to install it.
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("publish-unsigned", &server);
    let workshop = machine.root.join("workshop");
    std::fs::create_dir_all(&workshop).unwrap();
    std::fs::write(workshop.join("clock.wasm"), clock()).unwrap();
    std::fs::write(
        workshop.join("clock.toml"),
        manifest_of("1.0.0", &["timer", "locale"]),
    )
    .unwrap();
    let author_key = workshop.join("author-key");
    machine.ok(&["keygen", "--out", author_key.to_str().unwrap()]);
    machine.ok(&[
        "sign",
        workshop.join("clock.wasm").to_str().unwrap(),
        "--key",
        author_key.to_str().unwrap(),
    ]);

    // One more capability, after the signature was made.
    std::fs::write(
        workshop.join("clock.toml"),
        manifest_of("1.0.0", &["timer", "locale", "secrets"]),
    )
    .unwrap();

    let publisher_key = workshop.join("publisher-key");
    machine.ok(&["keygen", "--out", publisher_key.to_str().unwrap()]);
    let why = machine.fails(&[
        "registry",
        "publish",
        "--key",
        publisher_key.to_str().unwrap(),
        "--base-url",
        &server.url(""),
        "--out",
        workshop.join("index.toml").to_str().unwrap(),
        workshop.join("clock.wasm").to_str().unwrap(),
    ]);
    assert!(why.contains("does not match this widget"), "{why}");
    assert!(
        !workshop.join("index.toml").exists(),
        "and no index was written"
    );
}

#[test]
fn a_registry_this_machine_does_not_read_offers_nothing() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("none", &server);
    let why = machine.fails(&["search", "clock"]);
    assert!(why.contains("no registries"), "{why}");
    let why = machine.fails(&["add", "clock"]);
    assert!(why.contains("no registries"), "{why}");
    // And `update` says so rather than answering "everything is up to date",
    // which would be true and would read as the opposite.
    let why = machine.fails(&["update"]);
    assert!(
        why.contains("no registries") || why.contains("nothing is installed"),
        "{why}"
    );
}

#[test]
fn a_registry_can_be_listed_and_forgotten() {
    let Some(server) = TestServer::start() else {
        return;
    };
    let machine = Machine::new("forget", &server);
    let published = Published::new();
    published.serve(&server, &[("1.0.0", &["timer", "locale"])]);
    published.set_up(&machine, &server);

    let said = machine.ok(&["registry", "list"]);
    assert!(said.contains("example"), "{said}");
    assert!(
        said.contains(&published.publisher.public().to_hex()),
        "{said}"
    );
    assert!(said.contains("cached"), "{said}");

    machine.ok(&["registry", "remove", "example"]);
    assert!(machine.ok(&["registry", "list"]).contains("no registries"));
    // The cache went with it; nothing is left claiming to be a signed list.
    assert!(!machine.root.join("registry-cache/example.toml").exists());
}

/// The public key `uniwidged keygen` printed.
fn public_key_from(said: &str) -> String {
    said.lines()
        .find_map(|line| line.trim().strip_prefix("public key: "))
        .expect("keygen prints the public key")
        .trim()
        .to_string()
}
