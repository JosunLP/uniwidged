//! What a widget's PNG is allowed to make the host do.
//!
//! A decoder is a parser running on bytes the widget chose, so most of these
//! are about refusals — and about the one thing a refusal must never be, which
//! is an allocation sized by the file's header.

use super::*;

/// A PNG written by hand, so the test does not depend on an encoder's choices.
pub(crate) fn png(width: u32, height: u32, colour_type: u8, rows: &[&[u8]]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    }
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in bytes {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, colour_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    let mut raw = Vec::new();
    for row in rows {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    chunk(&mut out, b"IDAT", &deflate_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A zlib stream of stored (uncompressed) blocks — no compressor needed.
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(65535).collect()
    };
    for (i, block) in blocks.iter().enumerate() {
        out.push(if i + 1 == blocks.len() { 1 } else { 0 });
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

#[test]
fn an_rgba_png_decodes_to_its_own_pixels() {
    let file = png(2, 1, 6, &[&[255, 0, 0, 255, 0, 0, 255, 128]]);
    let mut images = Images::default();
    let handle = images.register(&file).expect("decoded");
    let bitmap = images.get(handle).expect("kept");
    assert_eq!((bitmap.width, bitmap.height), (2, 1));
    assert_eq!(&bitmap.rgba[..], &[255, 0, 0, 255, 0, 0, 255, 128]);
}

#[test]
fn every_colour_type_arrives_as_rgba8() {
    // Greyscale, RGB and palette all become the one layout a renderer takes,
    // so what is kept has one stride however the file was written.
    let grey = png(1, 1, 0, &[&[200]]);
    let rgb = png(1, 1, 2, &[&[10, 20, 30]]);
    let mut images = Images::default();
    let g = images.register(&grey).expect("grey");
    assert_eq!(&images.get(g).unwrap().rgba[..], &[200, 200, 200, 255]);
    let c = images.register(&rgb).expect("rgb");
    assert_eq!(&images.get(c).unwrap().rgba[..], &[10, 20, 30, 255]);
    let grey_alpha = png(1, 1, 4, &[&[77, 128]]);
    let ga = images.register(&grey_alpha).expect("grey+alpha");
    assert_eq!(&images.get(ga).unwrap().rgba[..], &[77, 77, 77, 128]);
}

#[test]
fn handles_start_at_one_and_are_never_reused() {
    // Zero is `ImageHandle::default()`, which must draw nothing. And a stale
    // handle after a release must not become somebody's later picture.
    let file = png(1, 1, 6, &[&[1, 2, 3, 4]]);
    let mut images = Images::default();
    let first = images.register(&file).expect("first");
    assert_eq!(first, 1);
    assert!(images.release(first));
    assert!(!images.release(first), "released twice");
    let second = images.register(&file).expect("second");
    assert_ne!(second, first);
    assert!(images.get(first).is_none());
}

#[test]
fn a_widget_may_hold_sixteen_and_the_seventeenth_is_a_budget_refusal() {
    let file = png(1, 1, 6, &[&[1, 2, 3, 4]]);
    let mut images = Images::default();
    for _ in 0..MAX_IMAGES {
        images.register(&file).expect("within the budget");
    }
    assert_eq!(images.register(&file), Err(ImageError::Full));
    // Releasing one makes room for one.
    assert!(images.release(1));
    assert!(images.register(&file).is_ok());
}

#[test]
fn a_header_claiming_a_huge_image_is_refused_before_anything_is_allocated() {
    // The header says 100 000 pixels a side and the file is fifty bytes. A
    // decoder that trusted the header would ask for 40 GB.
    let file = png(100_000, 100_000, 6, &[]);
    let mut images = Images::default();
    match images.register(&file) {
        Err(ImageError::Malformed(why)) => assert!(why.contains("1024"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_zero_sized_image_and_a_file_too_large_are_both_malformed() {
    let mut images = Images::default();
    assert!(matches!(
        images.register(&png(0, 1, 6, &[])),
        Err(ImageError::Malformed(_))
    ));
    let huge = vec![0u8; MAX_ENCODED + 1];
    match images.register(&huge) {
        Err(ImageError::Malformed(why)) => assert!(why.contains("bytes"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn bytes_that_are_not_a_png_are_refused_with_the_decoders_reason() {
    let mut images = Images::default();
    match images.register(b"GIF89a not a png at all") {
        Err(ImageError::Malformed(why)) => assert!(why.contains("not a PNG"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Truncated after a valid header: the decoder finds out mid-stream.
    let row = [0u8; 16];
    let file = png(4, 4, 6, &[&row, &row, &row, &row]);
    let cut = &file[..file.len() / 2];
    assert!(matches!(
        images.register(cut),
        Err(ImageError::Malformed(_))
    ));
}
