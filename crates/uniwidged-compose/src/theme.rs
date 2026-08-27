//! What a named style actually is.
//!
//! Every visual decision in uniwidged lands here, which is what makes a screen
//! of unrelated widgets look like one product. A widget names
//! [`uniwidged_view::Role::Title`]; this decides that is 15 px at weight 600
//! and 92 % opacity. A widget names [`uniwidged_view::Accent::Critical`]; this
//! decides which red is legible in the current appearance.
//!
//! ## Colours are authored in Oklch
//!
//! Not because it is fashionable, but because a widget theme is mostly *ramps*
//! — a foreground at four opacities, an accent at three lightnesses — and an
//! sRGB ramp that measures even does not look even. Oklch lightness is
//! perceptual, so a step is a step. Conversion to the surface's colour space
//! happens once, at paint time, which is also where Display P3 is picked up
//! where the surface offers it.
//!
//! ## Status
//!
//! Skeleton — the token vocabulary is fixed, the values land in M2.

/// The resolved appearance: what light or dark actually means on this desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// The user's contrast preference, taken from the platform where it exposes
/// one.
///
/// Not a cosmetic setting: at [`Contrast::High`] the theme drops translucency
/// and muting in favour of solid fills and full-opacity text, because a
/// widget's whole visual language is transparency and transparency is exactly
/// what fails for a user who asked for contrast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Contrast {
    #[default]
    Standard,
    High,
}

#[derive(Debug, Clone, Default)]
pub struct Theme {
    pub appearance: Appearance,
    pub contrast: Contrast,
    /// The desktop's accent colour where the platform exposes one, as Oklch.
    pub brand: Option<[f32; 3]>,
    /// The base spacing unit in logical pixels. `gap(2)` is two of these.
    pub step: f32,
}
