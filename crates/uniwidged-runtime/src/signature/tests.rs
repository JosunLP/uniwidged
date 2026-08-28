//! What a signature does not cover.
//!
//! The happy path is two lines and is the least interesting thing here. What
//! matters is everything a signature must *stop*, and the one that motivates
//! the whole design: a manifest swapped under a valid signature would show a
//! user a consent screen its author never wrote, and they would approve it on
//! the strength of the author's name.

use super::*;

const COMPONENT: &[u8] = b"\0asm\x01\x00\x00\x00 pretend this is a widget";
const MANIFEST: &str = "name = \"clock\"\ncapabilities = [\"timer\"]\n";

fn signed() -> (SecretKey, TrustedKeys, SignatureFile) {
    let key = SecretKey::generate().expect("entropy");
    let mut trusted = TrustedKeys::default();
    trusted.trust(key.public(), "the author");
    let file = SignatureFile {
        signature: key.sign(COMPONENT, MANIFEST),
        key: key.public(),
    };
    (key, trusted, file)
}

#[test]
fn a_widget_signed_by_a_trusted_key_is_accepted_and_names_its_signer() {
    let (_key, trusted, file) = signed();
    assert_eq!(
        trusted.verify(COMPONENT, MANIFEST, &file),
        Ok("the author"),
        "the answer is the name the user gave the key, not sixty-four hex \
         characters"
    );
}

#[test]
fn changing_the_manifest_breaks_the_signature() {
    // **The reason the digest covers both.** A signature over the component
    // alone would still verify here, and the user would be shown a consent
    // screen the author never wrote — approving `net:*.evil.test` on the
    // strength of a name they recognised.
    let (_key, trusted, file) = signed();
    let widened = "name = \"clock\"\ncapabilities = [\"timer\", \"net:*.evil.test\"]\n";
    assert_eq!(
        trusted.verify(COMPONENT, widened, &file),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn changing_the_component_breaks_the_signature() {
    let (_key, trusted, file) = signed();
    let mut tampered = COMPONENT.to_vec();
    tampered.push(b'!');
    assert_eq!(
        trusted.verify(&tampered, MANIFEST, &file),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn bytes_cannot_be_moved_between_the_component_and_the_manifest() {
    // Without length prefixes the digest is `component ‖ manifest`, and moving
    // the last byte of one to the front of the other leaves it unchanged — so
    // a different pair carries the same valid signature. Contrived to write and
    // trivial to exploit once it exists.
    let component = b"abcdef";
    let manifest = "ghijkl";
    let shifted_component = b"abcde";
    let shifted_manifest = "fghijkl";
    assert_ne!(
        digest(component, manifest),
        digest(shifted_component, shifted_manifest),
        "the fields must not run into each other"
    );
}

#[test]
fn a_signature_from_an_untrusted_key_is_refused_however_good_it_is() {
    // A perfectly valid signature by somebody nobody asked for. This is what
    // every widget looks like before its author's key is added, so the message
    // has to be an invitation rather than an accusation.
    let stranger = SecretKey::generate().expect("entropy");
    let file = SignatureFile {
        signature: stranger.sign(COMPONENT, MANIFEST),
        key: stranger.public(),
    };
    let (_key, trusted, _) = signed();

    match trusted.verify(COMPONENT, MANIFEST, &file) {
        Err(VerifyError::UntrustedKey(key)) => {
            assert_eq!(key, stranger.public());
            let said = VerifyError::UntrustedKey(key).to_string();
            assert!(said.contains("uniwidged trust"), "{said}");
            assert!(
                !said.contains("valid"),
                "an untrusted key must never be reported as a valid signature: {said}"
            );
        }
        other => panic!("a stranger's key must be refused, got {other:?}"),
    }
}

#[test]
fn a_signature_claiming_somebody_elses_key_is_refused() {
    // The obvious forgery: sign with your own key, write the trusted key's
    // name in the file. The key in the file is what the signature is checked
    // against, so this fails on the signature rather than on trust.
    let (author, trusted, _) = signed();
    let forger = SecretKey::generate().expect("entropy");
    let forged = SignatureFile {
        signature: forger.sign(COMPONENT, MANIFEST),
        key: author.public(),
    };
    assert_eq!(
        trusted.verify(COMPONENT, MANIFEST, &forged),
        Err(VerifyError::BadSignature)
    );
}

#[test]
fn a_signature_over_something_else_cannot_be_replayed_over_a_widget() {
    // Domain separation. If this project ever signs anything but widgets — a
    // registry index, a configuration bundle — a signature over one must not
    // verify over the other. The prefix is what guarantees it, and this is the
    // assertion that it is actually applied.
    let plain = Sha256::digest(COMPONENT);
    assert_ne!(
        digest(COMPONENT, ""),
        <[u8; 32]>::from(plain),
        "the digest must not be a bare hash of the component"
    );
}

#[test]
fn an_empty_trust_file_trusts_nothing() {
    let empty = TrustedKeys::default();
    let key = SecretKey::generate().expect("entropy");
    let file = SignatureFile {
        signature: key.sign(COMPONENT, MANIFEST),
        key: key.public(),
    };
    assert!(empty.is_empty());
    assert!(matches!(
        empty.verify(COMPONENT, MANIFEST, &file),
        Err(VerifyError::UntrustedKey(_))
    ));
}

#[test]
fn trusting_twice_leaves_one_line_and_the_newer_name() {
    // `uniwidged trust` run twice should not leave two lines, and a file with a
    // key twice should not make it twice as trusted.
    let key = SecretKey::generate().expect("entropy");
    let mut trusted = TrustedKeys::default();
    trusted.trust(key.public(), "first name");
    trusted.trust(key.public(), "second name");
    assert_eq!(trusted.keys().len(), 1);
    assert_eq!(trusted.keys()[0].1, "second name");
}

#[test]
fn withdrawing_trust_takes_effect() {
    let (_key, mut trusted, file) = signed();
    assert!(trusted.verify(COMPONENT, MANIFEST, &file).is_ok());
    assert!(trusted.forget(file.key));
    assert!(matches!(
        trusted.verify(COMPONENT, MANIFEST, &file),
        Err(VerifyError::UntrustedKey(_))
    ));
    assert!(
        !trusted.forget(file.key),
        "and forgetting it again is a no-op"
    );
}

#[test]
fn the_trust_file_round_trips_through_text() {
    let a = SecretKey::generate().expect("entropy");
    let b = SecretKey::generate().expect("entropy");
    let mut trusted = TrustedKeys::default();
    trusted.trust(a.public(), "jane's widgets");
    trusted.trust(b.public(), "");

    let text = trusted.to_text();
    assert_eq!(TrustedKeys::parse(&text).expect("parses"), trusted);
    assert!(
        text.starts_with('#'),
        "a file a user is expected to edit says what it is"
    );
}

#[test]
fn the_trust_file_ignores_comments_and_blank_lines() {
    let key = SecretKey::generate().expect("entropy");
    let text = format!(
        "# a comment\n\n   \n{}  jane   # trailing note\n",
        key.public().to_hex()
    );
    let trusted = TrustedKeys::parse(&text).expect("parses");
    assert_eq!(trusted.keys().len(), 1);
    assert_eq!(trusted.keys()[0].1, "jane");
}

#[test]
fn a_trust_file_with_a_bad_key_is_refused_rather_than_partly_read() {
    // Silently skipping a line a user wrote is how somebody comes to believe
    // they trust a key they do not.
    for bad in ["not-hex-at-all", "abcd", &"a".repeat(63), &"a".repeat(65)] {
        assert!(
            TrustedKeys::parse(bad).is_err(),
            "{bad:?} should not parse as a trusted key"
        );
    }
}

#[test]
fn a_signature_file_round_trips_and_refuses_anything_else() {
    let (_key, _trusted, file) = signed();
    let text = file.to_text();
    assert_eq!(SignatureFile::parse(&text).expect("parses"), file);

    for bad in [
        "",
        "# only a comment\n",
        "something-else abcd abcd\n",
        &format!("uniwidged-sig-v1 {}\n", file.signature.to_hex()),
        &format!("{}\n", file.signature.to_hex()),
        &format!("{} and an extra word\n", text.trim()),
    ] {
        assert!(
            SignatureFile::parse(bad).is_err(),
            "{bad:?} should not parse as a signature"
        );
    }
}

#[test]
fn a_key_survives_being_written_down_and_read_back() {
    let key = SecretKey::generate().expect("entropy");
    let hex = key.to_hex();
    assert_eq!(hex.len(), 64);
    let again = SecretKey::from_hex(&hex).expect("parses");
    assert_eq!(again.public(), key.public());
    assert_eq!(
        PublicKey::from_hex(&key.public().to_hex()).expect("parses"),
        key.public()
    );
}

#[test]
fn hex_that_is_the_wrong_length_or_not_hex_is_refused() {
    // A truncated or padded key is a different key, and accepting one is how a
    // check comes to pass on something nobody meant.
    for bad in [
        "",
        "zz",
        &"ab".repeat(31),
        &"ab".repeat(33),
        &format!("{}g", "a".repeat(63)),
    ] {
        assert_eq!(
            PublicKey::from_hex(bad),
            Err(KeyError::Malformed),
            "{bad:?}"
        );
    }
}

#[test]
fn two_generated_keys_differ() {
    // A generator that returned a constant would pass every other test here.
    let a = SecretKey::generate().expect("entropy");
    let b = SecretKey::generate().expect("entropy");
    assert_ne!(a.public(), b.public());
    assert_ne!(a.to_hex(), b.to_hex());
}

#[test]
fn every_refusal_says_what_to_do_about_it() {
    let key = SecretKey::generate().expect("entropy").public();
    for said in [
        VerifyError::UntrustedKey(key).to_string(),
        VerifyError::BadSignature.to_string(),
        VerifyError::MalformedKey.to_string(),
        KeyError::Malformed.to_string(),
        KeyError::NotASignature.to_string(),
        KeyError::NoEntropy.to_string(),
    ] {
        assert!(said.len() > 25, "{said:?}");
    }
}
