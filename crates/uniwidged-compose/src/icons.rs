//! The framework's own icons, as drawn paths.
//!
//! Not a font. Ephemeris learned this the hard way: its refresh symbol was a
//! Segoe Fluent Icons glyph on Windows and a drawn path everywhere else,
//! because no font on macOS or Linux could be relied on to carry it. One widget
//! with two different refresh symbols is two widgets.
//!
//! Every path is described in a unit box — `0.0..1.0` on both axes — and scaled
//! into whatever rectangle layout gave it. That is what keeps them crisp at any
//! size and at any fractional scale factor: they are outlines, resolved at
//! paint time, never bitmaps.
//!
//! They are deliberately stroked rather than filled, with a single weight, so
//! that they sit beside text of the same weight without looking heavier.

use uniwidged_render::kurbo::{Affine, Arc, BezPath, Circle, Point, Rect, Shape};
use uniwidged_view::Icon;

/// An icon in unit space, ready to be transformed into place.
pub struct IconPath {
    /// Drawn with a round cap and join, at [`IconPath::stroke_width`] of the
    /// icon's size.
    pub strokes: Vec<BezPath>,
    /// Solid parts — an arrowhead, a dot.
    pub fills: Vec<BezPath>,
}

impl IconPath {
    /// Stroke width as a fraction of the icon's box.
    ///
    /// One value for every icon: a set whose members differ in weight reads as
    /// a set of icons from different places.
    pub const STROKE_WIDTH: f64 = 0.10;
}

fn line(from: (f64, f64), to: (f64, f64)) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(from);
    path.line_to(to);
    path
}

fn polyline(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    let Some(first) = points.first() else {
        return path;
    };
    path.move_to(*first);
    for point in &points[1..] {
        path.line_to(*point);
    }
    path
}

fn polygon(points: &[(f64, f64)]) -> BezPath {
    let mut path = polyline(points);
    path.close_path();
    path
}

/// The path for an icon, in a unit box.
pub fn path(icon: Icon) -> IconPath {
    match icon {
        // Three-quarters of a circle with an arrowhead, which is the shape
        // every platform's refresh symbol converges on.
        Icon::Refresh => {
            // Three-quarters of a turn, leaving a gap at the top right for the
            // arrowhead. Angles run clockwise from three o'clock because the y
            // axis points down.
            let arc = Arc::new(
                Point::new(0.5, 0.54),
                (0.33, 0.33),
                (-50.0f64).to_radians(),
                (285.0f64).to_radians(),
                0.0,
            );
            IconPath {
                strokes: vec![arc.into_path(0.002)],
                fills: vec![polygon(&[(0.68, 0.02), (0.97, 0.24), (0.62, 0.40)])],
            }
        }
        // Three sliders. A gear at 14 px is a grey blob; sliders survive the
        // size a widget actually uses.
        Icon::Settings => IconPath {
            strokes: vec![
                line((0.08, 0.22), (0.92, 0.22)),
                line((0.08, 0.50), (0.92, 0.50)),
                line((0.08, 0.78), (0.92, 0.78)),
            ],
            fills: vec![
                Circle::new(Point::new(0.32, 0.22), 0.11).to_path(0.01),
                Circle::new(Point::new(0.66, 0.50), 0.11).to_path(0.01),
                Circle::new(Point::new(0.44, 0.78), 0.11).to_path(0.01),
            ],
        },
        Icon::Close => IconPath {
            strokes: vec![
                line((0.16, 0.16), (0.84, 0.84)),
                line((0.84, 0.16), (0.16, 0.84)),
            ],
            fills: Vec::new(),
        },
        // Points down. Callers rotate it for the other three directions rather
        // than the set carrying four near-identical entries.
        Icon::Chevron => IconPath {
            strokes: vec![polyline(&[(0.20, 0.36), (0.50, 0.68), (0.80, 0.36)])],
            fills: Vec::new(),
        },
        Icon::Check => IconPath {
            strokes: vec![polyline(&[(0.16, 0.53), (0.40, 0.77), (0.84, 0.25)])],
            fills: Vec::new(),
        },
        // A bar and a dot inside a ring: legible at 12 px, where a triangle
        // with an exclamation mark is not.
        Icon::Alert => IconPath {
            strokes: vec![
                Circle::new(Point::new(0.5, 0.5), 0.40).to_path(0.01),
                line((0.5, 0.26), (0.5, 0.55)),
            ],
            fills: vec![Circle::new(Point::new(0.5, 0.72), 0.075).to_path(0.01)],
        },
        // A ring with a slash through it — the same shape every platform uses
        // for "no connection".
        Icon::Offline => IconPath {
            strokes: vec![
                Circle::new(Point::new(0.5, 0.5), 0.40).to_path(0.01),
                line((0.22, 0.78), (0.78, 0.22)),
            ],
            fills: Vec::new(),
        },
    }
}

/// The transform that maps the unit box onto `rect`, keeping the icon square
/// and centred if the rectangle is not.
///
/// The stroke width is not returned because it does not need to be: it stays a
/// unit constant and this transform scales it, which is what gives every icon
/// the same visual weight at every size.
pub fn transform(rect: Rect) -> Affine {
    let size = rect.width().min(rect.height());
    let dx = rect.x0 + (rect.width() - size) / 2.0;
    let dy = rect.y0 + (rect.height() - size) / 2.0;
    Affine::translate((dx, dy)) * Affine::scale(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uniwidged_render::kurbo::Shape;

    const EVERY: [Icon; 7] = [
        Icon::Refresh,
        Icon::Settings,
        Icon::Close,
        Icon::Chevron,
        Icon::Check,
        Icon::Alert,
        Icon::Offline,
    ];

    #[test]
    fn every_icon_draws_something() {
        for icon in EVERY {
            let p = path(icon);
            assert!(
                !p.strokes.is_empty() || !p.fills.is_empty(),
                "{icon:?} is empty"
            );
        }
    }

    #[test]
    fn every_icon_stays_inside_its_unit_box() {
        // Icons are scaled into whatever rectangle layout gave them. One that
        // strays outside 0..1 paints over its neighbours, and at a small size
        // that reads as a rendering fault rather than as a design choice.
        let margin = IconPath::STROKE_WIDTH / 2.0;
        for icon in EVERY {
            let p = path(icon);
            for path in p.strokes.iter().chain(p.fills.iter()) {
                let b = path.bounding_box();
                assert!(
                    b.x0 >= -margin
                        && b.y0 >= -margin
                        && b.x1 <= 1.0 + margin
                        && b.y1 <= 1.0 + margin,
                    "{icon:?} extends to {b:?}"
                );
            }
        }
    }

    fn extent(icon: Icon) -> Rect {
        let p = path(icon);
        let mut bounds: Option<Rect> = None;
        for path in p.strokes.iter().chain(p.fills.iter()) {
            let b = path.bounding_box();
            bounds = Some(match bounds {
                Some(acc) => acc.union(b),
                None => b,
            });
        }
        bounds.expect("an icon has a path")
    }

    #[test]
    fn every_icon_fills_enough_of_its_box_to_be_seen() {
        // The other half of the previous test. Deliberately about the *larger*
        // dimension: a chevron is a wide, shallow shape and demanding height of
        // it would only make it a worse chevron. What matters is that no icon
        // is small in its box, next to neighbours that are not.
        for icon in EVERY {
            let b = extent(icon);
            assert!(
                b.width().max(b.height()) > 0.6,
                "{icon:?} only occupies {:.2}x{:.2} of its box",
                b.width(),
                b.height()
            );
        }
    }

    #[test]
    fn every_icon_sits_in_the_middle_of_its_box() {
        // An icon drifting towards one corner reads as misaligned against the
        // text beside it, and every icon drifting differently reads as sloppy.
        for icon in EVERY {
            let centre = extent(icon).center();
            assert!(
                (centre.x - 0.5).abs() < 0.08 && (centre.y - 0.5).abs() < 0.08,
                "{icon:?} is centred at ({:.2}, {:.2})",
                centre.x,
                centre.y
            );
        }
    }

    #[test]
    fn the_unit_box_maps_onto_a_square_inside_a_wider_rectangle() {
        // Layout gives an icon whatever box the row had; the icon must stay
        // square and centred rather than stretching to fill it.
        let affine = transform(Rect::new(10.0, 20.0, 50.0, 40.0));
        let corner = affine * Point::new(0.0, 0.0);
        let far = affine * Point::new(1.0, 1.0);
        assert!((far.x - corner.x - 20.0).abs() < 0.01, "should be 20 wide");
        assert!((far.y - corner.y - 20.0).abs() < 0.01, "should be 20 tall");
        assert!((corner.x - 20.0).abs() < 0.01, "centred horizontally");
        assert!((corner.y - 20.0).abs() < 0.01);
    }
}
