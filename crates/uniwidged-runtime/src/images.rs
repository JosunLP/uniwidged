//! The bitmaps a widget has handed the host to draw.
//!
//! `content.image(u32)` had been in the interface since the first version, and
//! there was no way to obtain a `u32` that meant anything: every `image` node
//! ever built drew nothing. This is the other half. A widget hands over a PNG
//! — its weather icon, its avatar, a chart it fetched — and gets a handle back;
//! the host decodes it once, here, and keeps the pixels.
//!
//! ## Why it needs no capability
//!
//! Every other host function is behind a manifest line because it *reaches*
//! somewhere: the network, a directory, the keychain, a process. This reaches
//! nothing. The bytes came from the widget and go to the widget's own surface,
//! and a user asked to approve "may draw pictures on itself" would rightly ask
//! what the alternative was. `log` and `setting` are the other two functions
//! with the same shape.
//!
//! What has to be bounded instead is the host's memory, because a decoded
//! image is the one thing a widget can ask the host to allocate that is larger
//! than what it sent. So: a PNG is at most [`MAX_ENCODED`] bytes and
//! [`MAX_DIMENSION`] pixels a side, and a widget may hold [`MAX_IMAGES`] at
//! once. The first two are facts about the request and answer `malformed`; the
//! third is a fact about the widget's budget and answers `unavailable` — the
//! same split `fs` makes between a file too large and a directory too full.
//!
//! ## Why PNG and nothing else
//!
//! One format is one decoder, and a decoder is a parser running on bytes a
//! widget chose — which is the shape of code that has had the most CVEs filed
//! against it in the history of desktop software. The `png` crate is pure Rust,
//! is already in the graph for the renderer's readback, and PNG is the format
//! every tool can write. JPEG can be added the day a widget needs a photograph.

use std::collections::HashMap;
use std::sync::Arc;

use uniwidged_view::Bitmap;

/// The largest PNG a widget may hand over, encoded.
pub const MAX_ENCODED: usize = 1024 * 1024;

/// The most pixels an image may have on a side.
///
/// A widget is a small panel; 1024 is four times the width of a large one, and
/// it caps a decode at 4 MiB. A widget that wants a wallpaper is a different
/// program.
pub const MAX_DIMENSION: u32 = 1024;

/// How many images one widget may hold at once.
///
/// A weather widget has a dozen condition icons; a calendar has an avatar or
/// two. Sixteen is room for those and not for a widget that registers a frame
/// of animation per tick and forgets to release it — which is the bug this cap
/// exists to turn into a refusal.
pub const MAX_IMAGES: usize = 16;

/// Why a PNG was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Too large encoded, too large decoded, or not a PNG the decoder accepts.
    /// A fact about the bytes: the widget's author can fix it.
    Malformed(String),
    /// The widget already holds [`MAX_IMAGES`]. A fact about the budget: the
    /// widget should release one first.
    Full,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Malformed(why) => write!(f, "{why}"),
            ImageError::Full => write!(
                f,
                "this widget already holds {MAX_IMAGES} images; release one first"
            ),
        }
    }
}

/// One widget's images, by handle.
#[derive(Debug, Default)]
pub struct Images {
    by_handle: HashMap<u32, Bitmap>,
    /// Handles count up and are never reused within an instance, so a widget
    /// that keeps a stale one after `release` draws nothing rather than
    /// somebody else's later picture.
    next: u32,
}

impl Images {
    /// Decodes and keeps a PNG, answering its handle.
    pub fn register(&mut self, png: &[u8]) -> Result<u32, ImageError> {
        if self.by_handle.len() >= MAX_IMAGES {
            return Err(ImageError::Full);
        }
        if png.len() > MAX_ENCODED {
            return Err(ImageError::Malformed(format!(
                "a PNG may be at most {MAX_ENCODED} bytes; this one is {}",
                png.len()
            )));
        }
        let bitmap = decode(png)?;
        // Handles start at 1: zero is `ImageHandle::default()`, and a node
        // built with a default handle should draw nothing rather than the
        // first thing the widget registered.
        self.next += 1;
        let handle = self.next;
        self.by_handle.insert(handle, bitmap);
        Ok(handle)
    }

    /// Forgets one. `false` when there was nothing to forget.
    pub fn release(&mut self, handle: u32) -> bool {
        self.by_handle.remove(&handle).is_some()
    }

    pub fn get(&self, handle: u32) -> Option<&Bitmap> {
        self.by_handle.get(&handle)
    }

    pub fn len(&self) -> usize {
        self.by_handle.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_handle.is_empty()
    }

    /// Every image, for a host handing them to its renderer.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &Bitmap)> {
        self.by_handle
            .iter()
            .map(|(handle, bitmap)| (*handle, bitmap))
    }
}

/// Decodes a PNG to straight-alpha RGBA8, whatever it was stored as.
///
/// The header is checked *before* the pixels are allocated: a PNG claiming to
/// be 100 000 pixels a side is refused by its dimensions, not by an allocation
/// that fails or succeeds depending on the machine.
fn decode(png: &[u8]) -> Result<Bitmap, ImageError> {
    let malformed = |e: png::DecodingError| ImageError::Malformed(format!("not a PNG: {e}"));
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    // Palette and 16-bit become 8-bit RGB or RGBA; `STRIP_16` keeps the top
    // byte, which is what every viewer does with a 16-bit PNG on an 8-bit
    // display. Greyscale stays greyscale — the decoder has no transformation
    // for that — and is widened below, so what is kept is one layout however
    // the file was written.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(malformed)?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(ImageError::Malformed(format!(
            "a PNG may be at most {MAX_DIMENSION} pixels a side; this one is {width}x{height}"
        )));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| ImageError::Malformed("the PNG's size overflows".into()))?;
    let mut buffer = vec![0u8; size];
    let frame = reader.next_frame(&mut buffer).map_err(malformed)?;
    buffer.truncate(frame.buffer_size());
    if frame.bit_depth != png::BitDepth::Eight {
        return Err(ImageError::Malformed(format!(
            "the PNG decoded to {:?} bits; eight were expected",
            frame.bit_depth
        )));
    }
    let pixels = (width as usize) * (height as usize);
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        // `EXPAND` turns a palette into RGB or RGBA before it gets here.
        png::ColorType::Indexed => {
            return Err(ImageError::Malformed(
                "the PNG's palette was not expanded".into(),
            ));
        }
    };
    // Said rather than assumed, because a buffer of the wrong stride handed
    // to a renderer is a picture that is skewed rather than wrong.
    if rgba.len() != pixels * 4 {
        return Err(ImageError::Malformed(format!(
            "the PNG decoded to {} bytes; {} were expected for {width}x{height}",
            rgba.len(),
            pixels * 4
        )));
    }
    Ok(Bitmap {
        width,
        height,
        rgba: Arc::new(rgba),
    })
}

#[cfg(test)]
mod tests;
