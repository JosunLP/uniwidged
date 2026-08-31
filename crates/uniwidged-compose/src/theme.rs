//! What a named style actually is.
//!
//! Every visual decision in uniwidged lands here, which is what makes a screen
//! of unrelated widgets look like one product. A widget names
//! [`Role::Title`]; this decides that is 15 px at
//! weight 600 and 92 % opacity. A widget names
//! [`Accent::Critical`]; this decides which
//! red is legible in the current appearance.
//!
//! ## Colours are authored in Oklch
//!
//! Not for fashion. A widget theme is mostly *ramps* — a foreground at four
//! opacities, an accent at three lightnesses — and an sRGB ramp that measures
//! even does not look even: the same numeric step is a large perceptual jump in
//! the dark end and a small one in the light. Oklch lightness is perceptual, so
//! a step is a step, and the muted variant of a colour is the same colour
//! rather than a different one that happens to be darker.
//!
//! Conversion to sRGB happens once, here, at resolve time.
//!
//! ## High contrast is not a cosmetic setting
//!
//! A widget's whole visual language is translucency, and translucency is
//! exactly what fails for someone who asked the system for more contrast. At
//! [`Contrast::High`] the theme drops it: opacities go to 1, surfaces become
//! solid, borders get heavier, and the muted step becomes a hue change rather
//! than a fade.

use uniwidged_view::{Accent, Role};

/// A colour in Oklch: perceptual lightness, chroma, hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    /// 0.0 (black) to 1.0 (white), perceptually uniform.
    pub l: f32,
    /// 0 upwards; ~0.37 is about as saturated as sRGB can go.
    pub c: f32,
    /// Degrees, 0–360.
    pub h: f32,
}

impl Oklch {
    pub const fn new(l: f32, c: f32, h: f32) -> Self {
        Self { l, c, h }
    }

    /// The same hue at a different lightness. The operation the whole ramp
    /// idea rests on: in Oklch this stays recognisably the same colour.
    pub fn at_lightness(self, l: f32) -> Self {
        Self { l, ..self }
    }

    /// Converts to sRGB, gamut-clipped by reducing chroma rather than by
    /// clamping channels.
    ///
    /// Clamping channels is what most conversions do and it shifts the hue: an
    /// out-of-gamut orange clamps its red first and slides towards yellow.
    /// Pulling chroma in keeps the hue and the lightness, which is what the
    /// caller asked for by naming a colour in Oklch at all.
    ///
    /// The search is a bisection rather than repeated scaling. Scaling by a
    /// fixed factor overshoots by up to that factor, and for a palette that
    /// means neighbouring entries lose different amounts of chroma and the set
    /// stops looking like a set. The theme's own colours are all inside the
    /// gamut anyway — [`Theme`] is tested for it — so this is a guard for
    /// values that come from elsewhere, such as a desktop accent colour.
    pub fn to_srgb(self) -> (u8, u8, u8) {
        let direct = oklch_to_linear_srgb(self.l, self.c, self.h);
        if in_gamut(direct) {
            return encode(direct);
        }
        let (mut lo, mut hi) = (0.0f32, self.c);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if in_gamut(oklch_to_linear_srgb(self.l, mid, self.h)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        encode(oklch_to_linear_srgb(self.l, lo, self.h))
    }

    /// Is this colour reproducible in sRGB as written?
    ///
    /// Used by the theme's own tests: a palette that depends on clipping is a
    /// palette whose author did not choose what shipped.
    pub fn is_in_srgb_gamut(self) -> bool {
        in_gamut(oklch_to_linear_srgb(self.l, self.c, self.h))
    }

    /// The same lightness and hue, at the most chroma sRGB can actually show.
    ///
    /// Needed wherever a lightness is computed rather than authored — the
    /// gamut narrows sharply towards both ends of the lightness range, so
    /// moving a colour lighter for contrast can move it out of gamut without
    /// anybody noticing. Applying this at the point the lightness changes
    /// keeps every colour the theme hands out reproducible by construction.
    pub fn clamped_to_gamut(self) -> Self {
        if self.is_in_srgb_gamut() {
            return self;
        }
        let (mut lo, mut hi) = (0.0f32, self.c);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if in_gamut(oklch_to_linear_srgb(self.l, mid, self.h)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Self { c: lo, ..self }
    }
}

// The published Oklab ↔ linear-sRGB matrices, digit for digit as Björn
// Ottosson gives them. `f32` cannot hold all of them, and clippy is right about
// that — but trimming them to what it can hold would make the constants no
// longer match the source a reader would check them against, for a rounding
// difference far below one 8-bit channel step.
#[allow(clippy::excessive_precision)]
fn oklch_to_linear_srgb(l: f32, c: f32, h: f32) -> [f32; 3] {
    let radians = h.to_radians();
    let (a, b) = (c * radians.cos(), c * radians.sin());

    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;

    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);

    [
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    ]
}

fn in_gamut(linear: [f32; 3]) -> bool {
    // A hair of slack, so a colour that lands on the boundary through floating
    // point does not lose chroma to a rounding error.
    linear.iter().all(|c| *c >= -0.0005 && *c <= 1.0005)
}

fn encode(linear: [f32; 3]) -> (u8, u8, u8) {
    let gamma = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let encoded = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0 + 0.5) as u8
    };
    (gamma(linear[0]), gamma(linear[1]), gamma(linear[2]))
}

/// Light or dark, as the desktop reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// The user's contrast preference. See the module note on why this is not
/// cosmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Contrast {
    #[default]
    Standard,
    High,
}

/// A resolved text style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    /// CSS-style numeric weight, 100–900.
    pub weight: f32,
    /// Multiplied into the foreground colour's alpha.
    pub opacity: f32,
    /// Multiple of the font size.
    pub line_height: f32,
}

/// A resolved surface: the glass, a card, a chip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Surface {
    /// Top of the vertical gradient, as `(r, g, b, a)`.
    pub fill_top: (u8, u8, u8, f32),
    pub fill_bottom: (u8, u8, u8, f32),
    pub radius: f32,
    /// The hairline along the edge. Zero width means none.
    pub border: ((u8, u8, u8, f32), f32),
    /// Blur radius and vertical offset of the drop shadow, both zero for none.
    pub shadow: (f32, f32),
    /// Alpha of the shadow's black.
    pub shadow_alpha: f32,
}

/// Everything a widget's named styles resolve against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    pub contrast: Contrast,
    /// The desktop's accent colour where the platform exposes one.
    pub brand: Oklch,
    /// The base spacing unit in logical pixels. `gap(2)` is two of these.
    pub step: f32,
    /// What [`Role::Body`] is; every other role is a ratio of it.
    pub base_font_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            appearance: Appearance::Dark,
            contrast: Contrast::Standard,
            // A blue close to what most desktops ship, until a platform tells
            // us otherwise. The chroma is what sRGB can actually reach at the
            // lightnesses this gets used at — 0.19 looks better on paper and
            // is outside the gamut at both of them.
            brand: Oklch::new(0.62, 0.14, 250.0),
            // Four is the unit almost every design system converges on, and
            // two of them is the smallest gap that reads as a gap.
            step: 4.0,
            base_font_size: 13.0,
        }
    }
}

impl Theme {
    pub fn light() -> Self {
        Self {
            appearance: Appearance::Light,
            ..Default::default()
        }
    }

    pub fn high_contrast(self) -> Self {
        Self {
            contrast: Contrast::High,
            ..self
        }
    }

    fn is_dark(&self) -> bool {
        self.appearance == Appearance::Dark
    }

    fn is_high_contrast(&self) -> bool {
        self.contrast == Contrast::High
    }

    /// `gap(2)` and `padding(2)` in logical pixels.
    pub fn spacing(&self, steps: u8) -> f32 {
        steps as f32 * self.step
    }

    /// The size, weight and opacity of a named role.
    pub fn text(&self, role: Role) -> TextStyle {
        let base = self.base_font_size;
        // Ratios rather than absolute sizes, so that a user who scales the
        // base size scales the whole hierarchy with it and the rhythm holds.
        let (ratio, weight, opacity) = match role {
            Role::Display => (2.15, 300.0, 1.00),
            Role::Title => (1.15, 600.0, 0.92),
            Role::Section => (0.92, 600.0, 0.70),
            Role::BodyStrong => (1.00, 600.0, 0.95),
            Role::Body => (1.00, 400.0, 0.88),
            Role::Caption => (0.85, 400.0, 0.62),
            Role::Chip => (0.80, 600.0, 0.95),
            // Not text roles; a string inside one of them reads as body.
            Role::Plain | Role::Panel | Role::Card => (1.00, 400.0, 0.88),
        };
        TextStyle {
            size: base * ratio,
            weight,
            // Opacity is how the hierarchy is built, and it is exactly what
            // fails at high contrast — there the hierarchy has to come from
            // size and weight alone.
            opacity: if self.is_high_contrast() {
                1.0
            } else {
                opacity
            },
            line_height: if role == Role::Display { 1.15 } else { 1.35 },
        }
    }

    /// The foreground colour a role's text is drawn in.
    pub fn foreground(&self, accent: Accent, role: Role, muted: bool) -> (u8, u8, u8, f32) {
        let style = self.text(role);
        let (r, g, b) = self.accent(accent).to_srgb();
        let mut alpha = style.opacity;
        if muted {
            alpha *= if self.is_high_contrast() { 0.85 } else { 0.55 };
        }
        (r, g, b, alpha)
    }

    /// A semantic accent as a concrete colour, chosen for this appearance.
    ///
    /// The lightness differs between light and dark on purpose: the same red
    /// that reads clearly on a dark panel is muddy on a light one, and a theme
    /// that ships one red for both has picked a compromise nobody asked for.
    pub fn accent(&self, accent: Accent) -> Oklch {
        let dark = self.is_dark();
        let boost = if self.is_high_contrast() { 0.08 } else { 0.0 };
        // Clamped on the way out. The authored chromas below are all inside
        // the gamut already — `every_colour_the_theme_can_produce_is_inside_
        // the_srgb_gamut` checks it — but `Brand` takes its chroma from the
        // *desktop*, at a lightness this function chooses, and those two have
        // never met. Without this, a high-contrast dark session pushed the
        // default blue to L=0.80 where sRGB has less room, and the result was
        // clipped silently.
        self.raw_accent(accent, dark, boost).clamped_to_gamut()
    }

    fn raw_accent(&self, accent: Accent, dark: bool, boost: f32) -> Oklch {
        match accent {
            Accent::Neutral => {
                // The foreground against the panel, not a hue: near-white on
                // dark, near-black on light, with a trace of the panel's own
                // coolness so it does not look like a sticker.
                let l = if dark { 0.96 } else { 0.22 };
                Oklch::new(l, 0.006, 250.0)
            }
            Accent::Brand => {
                let l = (if dark { 0.72 } else { 0.52 }) + boost * if dark { 1.0 } else { -1.0 };
                self.brand.at_lightness(l)
            }
            // Chroma varies with appearance as well as lightness, because the
            // sRGB gamut narrows sharply towards both ends of the lightness
            // range. Authoring one chroma for both would put half of these
            // outside the gamut, where they would be silently pulled back in —
            // unevenly, so a palette meant to look like a set would not.
            Accent::Positive => Oklch::new(
                if dark { 0.78 } else { 0.52 },
                if dark { 0.17 } else { 0.145 },
                148.0,
            ),
            Accent::Warning => Oklch::new(
                if dark { 0.83 } else { 0.60 },
                if dark { 0.15 } else { 0.12 },
                78.0,
            ),
            // The one accent whose chroma is the same in both appearances:
            // sRGB has room for it at either lightness, so there is nothing to
            // trade away.
            Accent::Critical => Oklch::new(if dark { 0.70 } else { 0.52 }, 0.18, 25.0),
            Accent::Series(index) => {
                // Distinguishable rather than meaningful. Walking the hue
                // circle by the golden angle keeps neighbouring indices far
                // apart, which is what a legend actually needs; a fixed list of
                // eight would repeat and put two similar colours side by side
                // on the ninth.
                //
                // The chroma is the *narrowest* the gamut allows anywhere on
                // that circle, not the widest — a palette whose members differ
                // in saturation because some hues had more room reads as a
                // mistake rather than as a palette.
                let hue = (index as f32 * 137.507_76) % 360.0;
                Oklch::new(
                    if dark { 0.76 } else { 0.55 },
                    if dark { 0.115 } else { 0.09 },
                    hue,
                )
            }
        }
    }

    /// The surface a container role paints, or `None` for a role that paints
    /// nothing.
    pub fn surface(&self, role: Role) -> Option<Surface> {
        let dark = self.is_dark();
        let solid = self.is_high_contrast();
        match role {
            Role::Panel => {
                // The glass. Translucent so the wallpaper shows through, which
                // is the whole visual idea — and the first thing to go when
                // the user asks for contrast.
                let (top, bottom) = if dark {
                    (
                        Oklch::new(0.30, 0.012, 250.0),
                        Oklch::new(0.24, 0.012, 250.0),
                    )
                } else {
                    (
                        Oklch::new(0.97, 0.006, 250.0),
                        Oklch::new(0.93, 0.006, 250.0),
                    )
                };
                let alpha = if solid {
                    1.0
                } else if dark {
                    0.74
                } else {
                    0.80
                };
                let border_l = if dark { 0.98 } else { 0.30 };
                Some(Surface {
                    fill_top: with_alpha(top, alpha),
                    fill_bottom: with_alpha(bottom, alpha),
                    radius: 14.0,
                    border: (
                        with_alpha(
                            Oklch::new(border_l, 0.0, 0.0),
                            if solid { 0.9 } else { 0.18 },
                        ),
                        if solid { 2.0 } else { 1.0 },
                    ),
                    shadow: (16.0, 5.0),
                    shadow_alpha: if dark { 0.45 } else { 0.22 },
                })
            }
            Role::Card => {
                let fill = if dark {
                    Oklch::new(0.38, 0.010, 250.0)
                } else {
                    Oklch::new(0.99, 0.004, 250.0)
                };
                let alpha = if solid { 1.0 } else { 0.55 };
                Some(Surface {
                    fill_top: with_alpha(fill, alpha),
                    fill_bottom: with_alpha(fill, alpha),
                    radius: 9.0,
                    border: (
                        with_alpha(Oklch::new(if dark { 0.98 } else { 0.30 }, 0.0, 0.0), 0.10),
                        if solid { 1.5 } else { 0.0 },
                    ),
                    shadow: (0.0, 0.0),
                    shadow_alpha: 0.0,
                })
            }
            Role::Chip => {
                let fill = if dark {
                    Oklch::new(0.45, 0.012, 250.0)
                } else {
                    Oklch::new(0.88, 0.010, 250.0)
                };
                Some(Surface {
                    fill_top: with_alpha(fill, if solid { 1.0 } else { 0.65 }),
                    fill_bottom: with_alpha(fill, if solid { 1.0 } else { 0.65 }),
                    // Half the chip's height at the default font size, which is
                    // what makes it read as a pill rather than a small box.
                    radius: 999.0,
                    border: ((0, 0, 0, 0.0), 0.0),
                    shadow: (0.0, 0.0),
                    shadow_alpha: 0.0,
                })
            }
            _ => None,
        }
    }

    /// The horizontal and vertical padding a chip gives itself.
    ///
    /// Proportional to the text rather than fixed, so a pill stays a pill when
    /// the user scales the base size — a chip with 6 px of padding around 26 px
    /// text is a box with rounded corners, not a pill.
    pub fn chip_padding(&self) -> (f32, f32) {
        let size = self.text(Role::Chip).size;
        (size * 0.55, size * 0.20)
    }

    /// How thick a progress bar is.
    pub fn progress_thickness(&self) -> f32 {
        (self.base_font_size * 0.46).round().max(3.0)
    }

    /// How tall a [`uniwidged_view::Content::Sparkline`] is.
    ///
    /// Two lines of body text: tall enough that a swing from 20 % to 80 % is
    /// visible at a glance, short enough that a chart does not become the
    /// widget. A ratio of the font size, like everything else, so a user who
    /// scales the text scales the chart with it.
    pub fn sparkline_height(&self) -> f32 {
        (self.base_font_size * 2.0).round().max(12.0)
    }

    /// The width of a sparkline's line, in logical pixels.
    ///
    /// Heavier than a hairline on purpose — a one-pixel line over a
    /// translucent panel disappears against a busy wallpaper — and thicker
    /// still at high contrast, where weight is what carries the hierarchy.
    pub fn sparkline_stroke(&self) -> f32 {
        let stroke = (self.base_font_size * 0.12).max(1.5);
        if self.is_high_contrast() {
            stroke + 0.5
        } else {
            stroke
        }
    }

    /// The opacity of the area under a sparkline's line.
    ///
    /// The area is what makes a chart read as a *quantity* rather than a
    /// squiggle; it is faint so that the line stays the subject.
    pub fn sparkline_fill_alpha(&self) -> f32 {
        if self.is_high_contrast() { 0.32 } else { 0.18 }
    }

    /// The rule a [`uniwidged_view::Content::Divider`] is drawn with.
    pub fn divider(&self) -> ((u8, u8, u8, f32), f32) {
        let dark = self.is_dark();
        let solid = self.is_high_contrast();
        (
            with_alpha(
                Oklch::new(if dark { 0.98 } else { 0.25 }, 0.0, 0.0),
                if solid { 0.7 } else { 0.14 },
            ),
            1.0,
        )
    }

    /// The unfilled part of a progress bar.
    pub fn track(&self) -> (u8, u8, u8, f32) {
        with_alpha(
            Oklch::new(if self.is_dark() { 0.98 } else { 0.20 }, 0.0, 0.0),
            if self.is_high_contrast() { 0.45 } else { 0.13 },
        )
    }
}

fn with_alpha(colour: Oklch, alpha: f32) -> (u8, u8, u8, f32) {
    let (r, g, b) = colour.to_srgb();
    (r, g, b, alpha)
}

#[cfg(test)]
mod tests;
