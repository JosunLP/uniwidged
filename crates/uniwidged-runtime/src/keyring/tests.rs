//! What the keychain keeps, and what it refuses to pretend.
//!
//! Two halves. The first is the ordinary contract every backend owes: a key
//! that was never set is absent rather than an error, a namespace is a wall,
//! and a value survives a restart. The second is what the *file* backend has
//! to get right, and it is the half that would be silently wrong — that the
//! bytes on disk are not the secret, that a store somebody edited refuses to
//! open rather than decrypting to something a widget believes, and that a key
//! file anybody can read stops the whole thing instead of making the
//! encryption decorative.

use super::*;

/// A directory that removes itself. Same six lines as `services::tests`, and
/// deliberately not shared: a test helper reaching across module boundaries is
/// how a test comes to depend on somebody else's fixture.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "uniwidged-keyring-{what}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }

    fn keyring(&self) -> FileKeyring {
        FileKeyring::new(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Both backends, so that every contract test below runs against each and the
/// memory one cannot quietly drift into being easier to satisfy.
fn backends(scratch: &Scratch) -> Vec<(&'static str, Arc<dyn SecretStore>)> {
    vec![
        ("file", Arc::new(scratch.keyring())),
        ("memory", Arc::new(MemoryKeyring::new())),
    ]
}

#[test]
fn a_key_that_was_never_set_is_absent_rather_than_an_error() {
    let scratch = Scratch::new("absent");
    for (which, store) in backends(&scratch) {
        let ring = Keyring::new(store, "weather");
        // The whole reason for the ABI 4 → 5 change: a widget's first run has
        // nothing stored, and it must not have to read an error string to
        // find out whether that is "not signed in yet" or "this host is
        // broken".
        assert_eq!(ring.get("token"), Ok(None), "{which}");
    }
}

#[test]
fn what_is_set_comes_back() {
    let scratch = Scratch::new("roundtrip");
    for (which, store) in backends(&scratch) {
        let ring = Keyring::new(store, "weather");
        ring.set("token", b"hunter2").expect("set");
        assert_eq!(ring.get("token"), Ok(Some(b"hunter2".to_vec())), "{which}");
        // Overwriting is not appending: a widget refreshing a token must not
        // end up with two.
        ring.set("token", b"hunter3").expect("set again");
        assert_eq!(ring.get("token"), Ok(Some(b"hunter3".to_vec())), "{which}");
    }
}

#[test]
fn arbitrary_bytes_survive() {
    let scratch = Scratch::new("bytes");
    // Not a token: a widget may store a key, and a key is bytes. The store's
    // own encoding is line-based, so a value holding a newline or a tab is
    // exactly what would break it.
    let awkward: Vec<u8> = (0u8..=255).collect();
    for (which, store) in backends(&scratch) {
        let ring = Keyring::new(store, "keys");
        ring.set("private", &awkward).expect("set");
        assert_eq!(ring.get("private"), Ok(Some(awkward.clone())), "{which}");
    }
}

#[test]
fn one_widget_cannot_read_anothers() {
    let scratch = Scratch::new("namespace");
    for (which, store) in backends(&scratch) {
        let mine = Keyring::new(store.clone(), "weather");
        let yours = Keyring::new(store, "calendar");
        mine.set("token", b"mine").expect("set");
        // Same key name, different widget. There is no argument through which
        // to name somebody else's namespace, so this is the whole of what a
        // widget could try.
        assert_eq!(yours.get("token"), Ok(None), "{which}");
        yours.set("token", b"yours").expect("set");
        assert_eq!(mine.get("token"), Ok(Some(b"mine".to_vec())), "{which}");
    }
}

#[test]
fn removing_is_idempotent_and_says_which_it_was() {
    let scratch = Scratch::new("remove");
    for (which, store) in backends(&scratch) {
        let ring = Keyring::new(store, "weather");
        ring.set("token", b"x").expect("set");
        assert_eq!(ring.remove("token"), Ok(true), "{which}");
        assert_eq!(ring.get("token"), Ok(None), "{which}");
        // Signing out twice is not a bug a widget should have to guard
        // against, so the second removal is `false` rather than an error.
        assert_eq!(ring.remove("token"), Ok(false), "{which}");
    }
}

#[test]
fn a_removal_leaves_the_neighbours_alone() {
    let scratch = Scratch::new("remove-others");
    for (which, store) in backends(&scratch) {
        let mine = Keyring::new(store.clone(), "weather");
        let yours = Keyring::new(store, "calendar");
        mine.set("token", b"mine").expect("set");
        yours.set("token", b"yours").expect("set");
        assert_eq!(mine.remove("token"), Ok(true), "{which}");
        assert_eq!(yours.get("token"), Ok(Some(b"yours".to_vec())), "{which}");
    }
}

#[test]
fn a_key_name_is_checked_before_the_store_is_touched() {
    let scratch = Scratch::new("bad-keys");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    for bad in [
        "",
        "with space",
        "with/slash",
        "with\ttab",
        "with\nnewline",
        "../escape",
        "ünïcödé",
    ] {
        assert!(
            matches!(ring.get(bad), Err(KeyringError::BadKey(_))),
            "`{bad}` should not be a key"
        );
        assert!(
            matches!(ring.set(bad, b"x"), Err(KeyringError::BadKey(_))),
            "`{bad}` should not be a key"
        );
    }
    // A tab is the store's own field separator, so a key containing one is
    // the case that would corrupt the encoding rather than merely look odd.
    // Nothing was written by any of the above.
    assert!(!scratch.keyring().store_path().exists());
}

#[test]
fn a_key_name_may_be_the_things_widgets_actually_use() {
    let scratch = Scratch::new("good-keys");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    for good in ["token", "api.key", "oauth_refresh", "user-2", "A1"] {
        assert_eq!(ring.set(good, b"x"), Ok(()), "`{good}` should be a key");
    }
}

#[test]
fn a_key_name_and_a_value_are_both_bounded() {
    let scratch = Scratch::new("bounds");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    assert_eq!(ring.set(&"k".repeat(MAX_KEY_LEN), b"x"), Ok(()));
    assert!(matches!(
        ring.set(&"k".repeat(MAX_KEY_LEN + 1), b"x"),
        Err(KeyringError::BadKey(_))
    ));
    assert_eq!(ring.set("big", &vec![0u8; MAX_SECRET_BYTES]), Ok(()));
    assert!(matches!(
        ring.set("big", &vec![0u8; MAX_SECRET_BYTES + 1]),
        Err(KeyringError::TooMuch(_))
    ));
}

#[test]
fn a_widget_writing_a_key_per_view_is_refused_rather_than_grown() {
    let scratch = Scratch::new("too-many");
    for (which, store) in backends(&scratch) {
        let ring = Keyring::new(store.clone(), "leaky");
        for i in 0..MAX_KEYS_PER_WIDGET {
            ring.set(&format!("k{i}"), b"x").expect("set");
        }
        assert!(
            matches!(ring.set("one-more", b"x"), Err(KeyringError::TooMuch(_))),
            "{which}"
        );
        // Refused, not evicted: silently dropping the oldest credential is a
        // bug whose symptom appears a week later.
        assert_eq!(ring.get("k0"), Ok(Some(b"x".to_vec())), "{which}");
        // And the limit is per widget, not per store — a busy neighbour must
        // not be able to lock everybody else out.
        let other = Keyring::new(store, "tidy");
        assert_eq!(other.set("token", b"x"), Ok(()), "{which}");
    }
}

// --- the file backend in particular ---------------------------------------

#[test]
fn a_secret_survives_the_process_that_wrote_it() {
    let scratch = Scratch::new("persist");
    Keyring::new(Arc::new(scratch.keyring()), "weather")
        .set("token", b"hunter2")
        .expect("set");
    // A completely fresh `FileKeyring` over the same directory: the same thing
    // that happens when the desktop is restarted.
    let reopened = Keyring::new(Arc::new(scratch.keyring()), "weather");
    assert_eq!(reopened.get("token"), Ok(Some(b"hunter2".to_vec())));
}

#[test]
fn the_secret_is_not_on_disk_in_the_clear() {
    let scratch = Scratch::new("opaque");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    ring.set("github.token", b"ghp_notreallyatoken")
        .expect("set");
    let bytes = std::fs::read(scratch.keyring().store_path()).expect("store");
    assert!(
        !contains(&bytes, b"ghp_notreallyatoken"),
        "the value is in the file"
    );
    // The key *name* is as telling as the value — `github.token` says most of
    // it — so the whole store is one message rather than a per-entry format
    // with the names in the open.
    assert!(
        !contains(&bytes, b"github.token"),
        "the key name is in the file"
    );
    assert!(
        !contains(&bytes, b"weather"),
        "the widget's name is in the file"
    );
}

#[test]
fn a_store_somebody_edited_refuses_to_open() {
    let scratch = Scratch::new("tamper");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    ring.set("token", b"hunter2").expect("set");

    let path = scratch.keyring().store_path().to_path_buf();
    let text = std::fs::read_to_string(&path).expect("store");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // One flipped bit in the ciphertext. Without an AEAD this would decrypt to
    // *something*, and the widget would believe it.
    let sealed = &mut lines[2];
    let first = sealed.remove(0);
    sealed.insert(0, if first == 'a' { 'b' } else { 'a' });
    std::fs::write(&path, lines.join("\n")).expect("rewrite");

    let reopened = Keyring::new(Arc::new(scratch.keyring()), "weather");
    assert!(matches!(
        reopened.get("token"),
        Err(KeyringError::Unavailable(_))
    ));
}

#[test]
fn a_store_from_a_different_key_refuses_to_open() {
    let scratch = Scratch::new("wrong-key");
    Keyring::new(Arc::new(scratch.keyring()), "weather")
        .set("token", b"hunter2")
        .expect("set");
    // What happens when somebody restores a backup of the store without the
    // key, or copies one machine's keychain to another. It must say so rather
    // than report an empty keychain, which would look exactly like a widget
    // having been signed out.
    let key_path = scratch.keyring().key_path().to_path_buf();
    std::fs::remove_file(&key_path).expect("remove key");
    let reopened = Keyring::new(Arc::new(scratch.keyring()), "weather");
    assert!(matches!(
        reopened.get("token"),
        Err(KeyringError::Unavailable(_))
    ));
}

#[test]
fn nothing_is_written_until_a_secret_is() {
    let scratch = Scratch::new("lazy");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    assert_eq!(ring.get("token"), Ok(None));
    // A desktop where no widget was granted `secrets` — or one where every
    // widget asked and had nothing — leaves *nothing* behind. Not the store,
    // and not the key either: a key file on a machine with no keychain is a
    // file somebody has to wonder about.
    assert!(!scratch.keyring().store_path().exists());
    assert!(!scratch.keyring().key_path().exists());
    // The first write creates both.
    ring.set("token", b"hunter2").expect("set");
    assert!(scratch.keyring().store_path().exists());
    assert!(scratch.keyring().key_path().exists());
}

#[cfg(unix)]
#[test]
fn both_files_are_created_readable_only_by_their_owner() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("modes");
    Keyring::new(Arc::new(scratch.keyring()), "weather")
        .set("token", b"hunter2")
        .expect("set");
    for path in [
        scratch.keyring().key_path().to_path_buf(),
        scratch.keyring().store_path().to_path_buf(),
    ] {
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "{} is mode {:o}",
            path.display(),
            mode & 0o777
        );
    }
}

#[cfg(unix)]
#[test]
fn a_key_file_anybody_can_read_stops_the_keychain() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new("wide-open");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    ring.set("token", b"hunter2").expect("set");

    let key_path = scratch.keyring().key_path().to_path_buf();
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    // Continuing here would make the encryption decorative: the key is beside
    // the store and anybody who can read one can read both. The message has to
    // name the file, because the fix is a `chmod` the user has to type.
    let reopened = Keyring::new(Arc::new(scratch.keyring()), "weather");
    let error = reopened.get("token").expect_err("should refuse");
    let KeyringError::Unavailable(why) = error else {
        panic!("expected unavailable");
    };
    assert!(why.contains(&key_path.display().to_string()), "{why}");

    // And a stricter mode than 0600 is fine: somebody may legitimately have
    // made it read-only.
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o400)).expect("chmod");
    assert_eq!(
        Keyring::new(Arc::new(scratch.keyring()), "weather").get("token"),
        Ok(Some(b"hunter2".to_vec()))
    );
}

#[test]
fn the_encoding_round_trips_everything_the_store_can_hold() {
    let entries = vec![
        Entry {
            namespace: "weather".into(),
            key: "token".into(),
            value: b"hunter2".to_vec(),
        },
        Entry {
            namespace: "calendar".into(),
            key: "oauth.refresh".into(),
            // The bytes that would end a line or a field if the encoding put
            // them there raw.
            value: vec![0, b'\n', b'\t', 255],
        },
        Entry {
            namespace: "empty".into(),
            key: "nothing".into(),
            value: Vec::new(),
        },
    ];
    assert_eq!(decode(&encode(&entries)), Some(entries));
    assert_eq!(decode(&encode(&[])), Some(Vec::new()));
}

#[test]
fn a_plaintext_that_is_not_the_encoding_is_rejected_rather_than_guessed() {
    // Only reachable through a key that authenticates a message this code did
    // not write, which is a contradiction — so this is about the decoder
    // being total rather than about a threat.
    assert_eq!(decode(b"no tabs here\n"), None);
    assert_eq!(decode(b"a\tb\tnot-hex\n"), None);
    assert_eq!(decode(b"a\tb\t00\textra\n"), None);
    assert_eq!(decode(&[0xff, 0xfe]), None);
}

#[test]
fn hex_round_trips() {
    assert_eq!(
        unhex(&hex(&[0, 1, 15, 16, 254, 255])),
        Some(vec![0, 1, 15, 16, 254, 255])
    );
    assert_eq!(unhex("abc"), None, "an odd number of digits is not hex");
    assert_eq!(unhex("zz"), None);
}

#[test]
fn a_memory_keyring_says_it_forgets() {
    // `uniwidged doctor` prints this, and a user deciding whether to let a
    // widget store a password is entitled to know which backend they have.
    assert!(MemoryKeyring::new().describe().contains("forgotten"));
    let scratch = Scratch::new("describe");
    assert!(scratch.keyring().describe().contains("not from a program"));
}

#[test]
fn a_view_full_of_reads_costs_no_disk_and_no_time() {
    // `get_secret` may be called on every frame, from inside a widget's
    // `view`, on a sixteen-millisecond deadline. So a read has to be memory
    // and nothing else — and the bound below is safe to assert on any machine
    // precisely *because* it is: a hundred reads that go to disk fail this on
    // a loaded runner, and a hundred reads that do not cannot.
    let scratch = Scratch::new("reads");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    ring.set("token", b"hunter2").expect("set");

    let began = std::time::Instant::now();
    for _ in 0..100 {
        assert_eq!(ring.get("token"), Ok(Some(b"hunter2".to_vec())));
    }
    let took = began.elapsed();
    assert!(
        took < std::time::Duration::from_millis(16),
        "a hundred reads took {took:?}; that is a widget's whole frame"
    );
}

#[test]
fn reading_a_secret_does_not_touch_the_disk_at_all() {
    // The cache exists because `get_secret` is called from inside `view` and
    // may be called on every frame. Re-reading and re-decrypting the file each
    // time would be work charged to the widget, so this asserts the property
    // directly rather than the timing: with the store's files taken away, an
    // already-open keychain still answers.
    let scratch = Scratch::new("no-disk");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    ring.set("token", b"hunter2").expect("set");
    std::fs::remove_file(scratch.keyring().store_path()).expect("remove store");
    std::fs::remove_file(scratch.keyring().key_path()).expect("remove key");
    assert_eq!(ring.get("token"), Ok(Some(b"hunter2".to_vec())));
}

#[test]
fn a_key_file_fixed_by_hand_is_picked_up_without_a_restart() {
    // The other side of caching: a *failure* to open must not be remembered,
    // or the `chmod` the error message told the user to run would do nothing
    // until they restarted the desktop.
    let scratch = Scratch::new("recover");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    // A store that exists beside a key file that is not a key. Reading it
    // cannot be answered at all — an empty keychain and an unopenable one look
    // identical to a widget, and they are not the same fact.
    ring.set("token", b"first").expect("set");
    std::fs::write(scratch.keyring().key_path(), b"nowhere near 32 bytes").expect("bad key");
    let fresh = Keyring::new(Arc::new(scratch.keyring()), "weather");
    assert!(matches!(
        fresh.get("token"),
        Err(KeyringError::Unavailable(_))
    ));

    // The `chmod`-or-delete the message told the user to do. The *same* handle
    // has to pick it up: caching the failure would mean the fix did nothing
    // until the desktop was restarted.
    std::fs::remove_file(scratch.keyring().key_path()).expect("remove");
    std::fs::remove_file(scratch.keyring().store_path()).expect("remove");
    assert_eq!(
        fresh.get("token"),
        Ok(None),
        "the same handle should recover"
    );
    fresh.set("token", b"hunter2").expect("set");
    assert_eq!(fresh.get("token"), Ok(Some(b"hunter2".to_vec())));
}

#[test]
fn storing_the_same_value_again_does_not_rewrite_the_store() {
    // A widget that stores its token on a schedule is the ordinary case, and
    // the ordinary case must not be a disk write charged to that widget's
    // deadline. Asserted through the file's modification time rather than by
    // timing, so it is a fact rather than a measurement.
    let scratch = Scratch::new("idempotent");
    let ring = Keyring::new(Arc::new(scratch.keyring()), "weather");
    ring.set("token", b"hunter2").expect("set");
    let path = scratch.keyring().store_path().to_path_buf();
    let before = std::fs::metadata(&path).expect("metadata").modified().ok();
    // A different inode would show up as a different modification time; the
    // point is that the write did not happen at all.
    ring.set("token", b"hunter2").expect("set again");
    let after = std::fs::metadata(&path).expect("metadata").modified().ok();
    assert_eq!(
        before, after,
        "an unchanged secret should not rewrite the store"
    );
    ring.set("token", b"hunter3").expect("change it");
    assert_eq!(ring.get("token"), Ok(Some(b"hunter3".to_vec())));
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The custody a Windows desktop actually uses. Everything above runs against
/// [`FileKeyring::new`]; this is the one place [`FileKeyring::system`]'s DPAPI
/// path is exercised, on the Windows CI runner.
#[cfg(windows)]
#[test]
fn a_dpapi_sealed_key_round_trips_and_is_not_the_key() {
    let sealed = dpapi::seal(&[7u8; 32]).expect("DPAPI seals");
    assert_ne!(
        sealed.as_slice(),
        &[7u8; 32],
        "sealing must change the bytes"
    );
    assert_eq!(
        dpapi::unseal(&sealed).expect("DPAPI unseals"),
        vec![7u8; 32]
    );

    let scratch = Scratch::new("dpapi");
    FileKeyring::system(&scratch.0)
        .set("w", "token", b"s3cret")
        .expect("a sealed keychain stores");
    let reopened = FileKeyring::system(&scratch.0);
    assert_eq!(
        reopened.get("w", "token").expect("it opens again"),
        Some(b"s3cret".to_vec())
    );
    let on_disk = std::fs::read(scratch.0.join("keyring.dpapi")).expect("the sealed key file");
    assert_ne!(
        on_disk.len(),
        32,
        "the key file must hold a DPAPI blob, not the key"
    );
    assert!(
        !scratch.0.join("keyring.key").exists(),
        "no raw key file beside a sealed one"
    );
}
