//! Geometry and colour, re-exported rather than redefined.
//!
//! The box a layout computes, the shape the renderer fills and the rectangle a
//! click is tested against are one type, because they come from one crate.
//! Ferrum UI resolves the same `kurbo` and `peniko` versions, so a value can
//! cross between the two frameworks without a conversion.
//!
//! Coordinates are **logical pixels** with the origin at the widget surface's
//! top-left corner. Physical pixels exist in exactly one place — the surface —
//! and `uniwidged-shell` converts at that boundary. Nothing above it is
//! DPI-aware.

pub use kurbo::{Affine, Insets, Point, Rect, RoundedRect, Size, Vec2};
pub use peniko::color::{AlphaColor, DynamicColor, Srgb};
pub use peniko::{Brush, Color, Gradient};

/// A rectangle in **physical** pixels, as a window system places surfaces.
///
/// Deliberately a different type from [`Rect`]: mixing the two silently is the
/// classic fractional-scaling bug, where a widget lands a third of a pixel off
/// on a 150 % display and every edge in it goes soft. Converting takes a scale
/// factor, so the conversion is always visible at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysicalRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The same rectangle in logical pixels, at `scale` device pixels per
    /// logical pixel.
    pub fn to_logical(self, scale: f64) -> Rect {
        Rect::new(
            self.x as f64 / scale,
            self.y as f64 / scale,
            (self.x as f64 + self.width as f64) / scale,
            (self.y as f64 + self.height as f64) / scale,
        )
    }

    /// Rounds outwards, so a surface is never a fraction of a pixel too small
    /// for what it has to show.
    pub fn from_logical(r: Rect, scale: f64) -> Self {
        let x = (r.x0 * scale).floor();
        let y = (r.y0 * scale).floor();
        Self {
            x: x as i32,
            y: y as i32,
            width: ((r.x1 * scale).ceil() - x).max(0.0) as u32,
            height: ((r.y1 * scale).ceil() - y).max(0.0) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logical_rect_survives_a_round_trip_through_physical_pixels() {
        // 150 % scaling is where the fractional-pixel bugs live.
        let logical = Rect::new(10.0, 20.0, 210.0, 140.0);
        let physical = PhysicalRect::from_logical(logical, 1.5);
        assert_eq!(physical, PhysicalRect::new(15, 30, 300, 180));
        assert_eq!(physical.to_logical(1.5), logical);
    }

    #[test]
    fn a_fractional_rect_rounds_outwards_rather_than_shrinking() {
        // Rounding to nearest here would hand back a surface one pixel short,
        // and the widget's bottom edge would be clipped rather than soft.
        let physical = PhysicalRect::from_logical(Rect::new(0.4, 0.4, 10.2, 10.2), 1.0);
        assert_eq!(physical, PhysicalRect::new(0, 0, 11, 11));
    }
}
