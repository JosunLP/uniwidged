//! Shaping, measuring and painting text.
//!
//! Wraps [`parley`] — shaping, line breaking, bidi, font fallback — and bridges
//! its output into a [`Scene`]. Nothing above this module touches Parley.
//!
//! ## Why not `ferrum-text`
//!
//! Ferrum UI has a Parley wrapper and it is a good one; this crate uses its
//! layout sibling unchanged. But it sets only font size and a generic family,
//! and two of uniwidged's stated quality commitments need more:
//!
//! * **Weight.** A title at 600 against a body at 400 is most of what makes a
//!   hierarchy readable, and at [`Contrast::High`](super::theme::Contrast) it
//!   is *all* of it, because opacity is gone.
//! * **Tabular figures.** A clock rendered in proportional digits changes width
//!   as the minute turns, and the whole line shifts. `tnum` fixes that, and it
//!   is the difference between a clock that sits still and one that twitches
//!   once a second.
//!
//! So this is Parley directly. The glyph bridge below is the technique from
//! `ferrum-text`, which is worth naming: `parley::FontData`, `peniko::FontData`
//! and the font type `Scene::draw_glyphs` expects are all the same
//! `linebender_resource_handle::FontData`, so a run's font passes straight
//! through with **no conversion at all**.

use std::collections::HashMap;
use std::sync::Arc;

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, FontFeature,
    FontFeatures, FontWeight, GenericFamily, Layout, LayoutContext, LineHeight,
    PositionedLayoutItem, StyleProperty,
};
use uniwidged_render::kurbo::Affine;
use uniwidged_render::peniko::{self, Fill};
use uniwidged_render::{Scene, vello};

use crate::theme::TextStyle;

/// How a run of text should be shaped. Everything that changes the *glyphs*,
/// as opposed to their colour — colour is applied at paint time, so one
/// shaping serves every colour it is ever drawn in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shaping {
    pub size: f32,
    pub weight: f32,
    pub line_height: f32,
    /// Fixed-width digits. On for anything numeric that updates.
    pub tabular: bool,
}

impl Shaping {
    pub fn from_style(style: TextStyle, tabular: bool) -> Self {
        Self {
            size: style.size,
            weight: style.weight,
            line_height: style.line_height,
            tabular,
        }
    }

    /// A cache key. Floats go in as bits, so two shapings that are equal are
    /// keyed the same and `NaN` — which cannot occur here — would simply miss.
    fn key(&self, text: &str, max_width: Option<f32>, align: Align) -> (u64, String) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        self.size.to_bits().hash(&mut hasher);
        self.weight.to_bits().hash(&mut hasher);
        self.line_height.to_bits().hash(&mut hasher);
        self.tabular.hash(&mut hasher);
        max_width.map(f32::to_bits).hash(&mut hasher);
        (align as u8).hash(&mut hasher);
        (hasher.finish(), text.to_owned())
    }
}

/// Where a line sits inside the width it was given.
///
/// In *physical* terms: the caller has already swapped leading for trailing
/// where the layout is mirrored, so this module never reasons about reading
/// direction for placement. It still does for *shaping* — an Arabic run inside
/// a line is Parley's business, and Parley handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

impl Align {
    fn to_parley(self) -> Alignment {
        match self {
            Align::Start => Alignment::Start,
            Align::Center => Alignment::Center,
            Align::End => Alignment::End,
        }
    }
}

/// A shaped run, ready to measure or paint.
///
/// `Arc` rather than `Rc`, and the difference is load-bearing rather than
/// stylistic: the engine is shared into the WASM sandbox behind the `measure`
/// capability, and Wasmtime requires a store's data to be `Send`. Nothing here
/// actually crosses a thread — see `ARCHITECTURE.md` §5, which has one loop —
/// but asserting `Send` on something built from `Rc` would be a lie the
/// compiler cannot check for us.
#[derive(Clone)]
pub struct Shaped {
    inner: Arc<Layout<peniko::Brush>>,
}

impl Shaped {
    pub fn width(&self) -> f32 {
        self.inner.width()
    }

    pub fn height(&self) -> f32 {
        self.inner.height()
    }

    /// Paints with its top-left at `origin`.
    pub fn paint(&self, scene: &mut Scene, origin: (f64, f64), colour: peniko::Color) {
        let transform = Affine::translate(origin);
        let brush = peniko::Brush::Solid(colour);
        for line in self.inner.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                scene
                    .draw_glyphs(run.font())
                    .font_size(run.font_size())
                    .brush(&brush)
                    .transform(transform)
                    .draw(
                        Fill::NonZero,
                        glyph_run.positioned_glyphs().map(|g| vello::Glyph {
                            id: g.id,
                            x: g.x,
                            y: g.y,
                        }),
                    );
            }
        }
    }
}

/// The single ellipsis character, not three dots.
///
/// Three periods are three glyphs with the font's own spacing between them,
/// which is wider and reads as a pause rather than as a truncation.
const ELLIPSIS: char = '…';

/// How many shaped layouts to keep. Cleared wholesale when exceeded, so a
/// widget whose text changes every second cannot grow this without bound.
const CACHE_CAP: usize = 1024;

/// Owns Parley's font and layout contexts, and the shaping cache.
///
/// Expensive to build — it enumerates the installed fonts — and cheap to reuse,
/// so there is exactly one per host. Eight widgets each enumerating the system
/// fonts at start-up would be visible.
pub struct TextEngine {
    fonts: FontContext,
    layout: LayoutContext<peniko::Brush>,
    cache: HashMap<u64, (String, Shaped)>,
    /// A family to use instead of the desktop's UI font.
    ///
    /// Only the golden-image suite sets this. A snapshot rendered with
    /// whatever font happens to be installed is a snapshot of that machine,
    /// not of this code: the images would differ between a developer's laptop
    /// and CI for reasons neither could act on.
    family: Option<String>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> Self {
        Self {
            fonts: FontContext::new(),
            layout: LayoutContext::new(),
            cache: HashMap::new(),
            family: None,
        }
    }

    /// An engine that shapes everything in one supplied font.
    ///
    /// For tests that compare pixels. See the note on [`TextEngine::family`].
    pub fn with_font(data: Vec<u8>, family: &str) -> Self {
        let mut engine = Self::new();
        engine
            .fonts
            .collection
            .register_fonts(peniko::Blob::new(std::sync::Arc::new(data)), None);
        engine.family = Some(family.to_string());
        engine
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Shapes `text`, wrapping at `max_width` when one is given.
    pub fn shape(
        &mut self,
        text: &str,
        shaping: Shaping,
        max_width: Option<f32>,
        align: Align,
    ) -> Shaped {
        let (hash, owned) = shaping.key(text, max_width, align);
        if let Some((cached_text, shaped)) = self.cache.get(&hash)
            && cached_text == &owned
        {
            return shaped.clone();
        }

        let mut builder = self.layout.ranged_builder(&mut self.fonts, text, 1.0, true);
        // `SystemUi` rather than `SansSerif`: a widget sits on the desktop and
        // should be set in the same face as the desktop's own chrome, not in
        // whatever the generic sans-serif happens to resolve to.
        let family = match &self.family {
            Some(name) => FontFamily::Single(FontFamilyName::Named(name.as_str().into())),
            None => FontFamily::Single(FontFamilyName::Generic(GenericFamily::SystemUi)),
        };
        builder.push_default(StyleProperty::FontFamily(family));
        builder.push_default(StyleProperty::FontSize(shaping.size));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(shaping.weight)));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
            shaping.line_height,
        )));
        if shaping.tabular {
            // `tnum` gives every digit the same advance. Without it a clock
            // changes width as the minute turns and the line it sits in shifts.
            const TABULAR: &[FontFeature] =
                &[FontFeature::new(parley::setting::Tag::new(b"tnum"), 1)];
            builder.push_default(StyleProperty::FontFeatures(FontFeatures::from(TABULAR)));
        }
        let mut built: Layout<peniko::Brush> = builder.build(text);
        built.break_all_lines(max_width);
        built.align(align.to_parley(), AlignmentOptions::default());

        if self.cache.len() >= CACHE_CAP {
            self.cache.clear();
        }
        let shaped = Shaped {
            inner: Arc::new(built),
        };
        self.cache.insert(hash, (owned, shaped.clone()));
        shaped
    }

    /// The natural size of one unwrapped line.
    pub fn measure(&mut self, text: &str, shaping: Shaping) -> (f32, f32) {
        let shaped = self.shape(text, shaping, None, Align::Start);
        (shaped.width(), shaped.height())
    }

    /// `text` shortened with an ellipsis until it fits in `max_width`.
    ///
    /// Returns the string to shape, not a shaped run, because the caller
    /// decides the alignment and may not paint it at all.
    ///
    /// The search is a bisection over *character* boundaries. Doing it by byte
    /// would split a multi-byte character, and doing it one character at a time
    /// is a shaping pass per character — on a long line of Japanese that is
    /// hundreds of passes for one label.
    pub fn ellipsise(&mut self, text: &str, shaping: Shaping, max_width: f32) -> String {
        if max_width <= 0.0 {
            return String::new();
        }
        if self.measure(text, shaping).0 <= max_width {
            return text.to_owned();
        }
        let chars: Vec<char> = text.chars().collect();
        // `hi` is the first length known not to fit; `lo` the longest known to.
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let mut candidate: String = chars[..mid].iter().collect();
            candidate.push(ELLIPSIS);
            if self.measure(&candidate, shaping).0 <= max_width {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if lo == 0 {
            // Not even one character and an ellipsis fits. The ellipsis alone
            // still says "there is more here", which an empty string does not.
            return ELLIPSIS.to_string();
        }
        let mut out: String = chars[..lo].iter().collect();
        // A truncation that ends mid-space reads as a missing word rather than
        // a cut one.
        while out.ends_with(' ') {
            out.pop();
        }
        out.push(ELLIPSIS);
        out
    }
}

#[cfg(test)]
mod tests;
