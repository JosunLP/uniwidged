//! Turning placed boxes into a scene.
//!
//! Everything in `ARCHITECTURE.md` §7 that is not a property of Vello itself is
//! decided here: that a shadow is a real Gaussian blur rather than stacked
//! strokes, that a gradient is interpolated in premultiplied space, that a
//! hairline stays one pixel, that a truncated string ends in a single ellipsis.
//!
//! ## Mirroring, not transforming
//!
//! A right-to-left layout folds x across the panel's centre. That is done to
//! the *rectangles*, before anything is drawn — never as a transform on the
//! scene, which would mirror the glyphs too and produce text nobody can read.
//! Ephemeris drew the same distinction and for the same reason. Shaping stays
//! Parley's business: an Arabic run inside a line is bidi, which it handles.
//!
//! ## Scale is applied once
//!
//! The whole scene is built in logical pixels and appended under an
//! `Affine::scale`. Vello rasterises outlines analytically, so a scaled
//! transform costs nothing in sharpness — there is no bitmap anywhere to
//! resample.

use uniwidged_render::Scene;
use uniwidged_render::kurbo::{
    Affine, BezPath, Cap, Circle, Join, Point, Rect, RoundedRect, Stroke,
};
use uniwidged_render::peniko::{
    Brush, Color, ColorStop, Extend, Fill, Gradient, color::AlphaColor,
};
use uniwidged_view::{Content, Justify, Role};

use crate::icons;
use crate::layout::Placed;
use crate::text::{Align, Shaping, TextEngine};
use crate::theme::Theme;

/// Everything the paint pass needs that is not the tree itself.
pub struct Painter<'a> {
    pub theme: &'a Theme,
    /// The surface's logical width, needed to fold x for a mirrored layout.
    pub width: f64,
    pub rtl: bool,
    /// Device pixels per logical pixel, applied once to the whole scene.
    pub scale: f64,
}

impl Painter<'_> {
    fn transform(&self) -> Affine {
        Affine::scale(self.scale)
    }

    /// Folds a rectangle across the surface's centre for a mirrored layout.
    ///
    /// Public because hit rectangles have to be folded the same way and by the
    /// same code: a click arrives in the coordinates the user sees.
    pub fn mirror_rect(&self, rect: Rect) -> Rect {
        self.mirror(rect)
    }

    fn mirror(&self, rect: Rect) -> Rect {
        if !self.rtl {
            return rect;
        }
        Rect::new(self.width - rect.x1, rect.y0, self.width - rect.x0, rect.y1)
    }

    /// Paints the whole tree into `scene`.
    pub fn paint(&self, scene: &mut Scene, root: &Placed<'_>, text: &mut TextEngine) {
        self.paint_node(scene, root, text);
    }

    fn paint_node(&self, scene: &mut Scene, placed: &Placed<'_>, text: &mut TextEngine) {
        let role = placed.node.style.role;
        let rect = self.mirror(placed.rect);

        if let Some(surface) = self.theme.surface(role) {
            self.paint_surface(scene, rect, &surface);
        }

        match &placed.node.content {
            Content::Text(string) => self.paint_text(scene, placed, string, text),
            Content::Icon(icon) => self.paint_icon(scene, placed, *icon),
            Content::Divider => self.paint_divider(scene, rect),
            Content::Progress(ratio) => self.paint_progress(scene, placed, ratio.get()),
            Content::Sparkline(series) => self.paint_sparkline(scene, placed, &series.0),
            Content::Group | Content::Spacer | Content::Image(_) => {}
        }

        for child in &placed.children {
            self.paint_node(scene, child, text);
        }
    }

    fn paint_surface(&self, scene: &mut Scene, rect: Rect, surface: &crate::theme::Surface) {
        // A radius larger than the box would invert the corner arcs. `999.0` is
        // how a pill asks for "as round as you can", so clamp rather than
        // treating it as an error.
        let radius = surface.radius.min(rect.height() as f32 / 2.0) as f64;
        let body = RoundedRect::from_rect(rect, radius);

        // The shadow first, so the glass sits on top of it. A real Gaussian
        // blur, not a stack of translucent strokes — the difference is obvious
        // at any size and is the whole reason Vello is here.
        if surface.shadow.0 > 0.0 {
            scene.draw_blurred_rounded_rect(
                self.transform() * Affine::translate((0.0, surface.shadow.1 as f64)),
                rect,
                AlphaColor::from_rgba8(0, 0, 0, (surface.shadow_alpha * 255.0) as u8),
                radius,
                surface.shadow.0 as f64,
            );
        }

        let brush = if surface.fill_top == surface.fill_bottom {
            Brush::Solid(rgba(surface.fill_top))
        } else {
            Brush::Gradient(
                Gradient::new_linear(Point::new(rect.x0, rect.y0), Point::new(rect.x0, rect.y1))
                    .with_stops([
                        ColorStop {
                            offset: 0.0,
                            color: rgba(surface.fill_top).into(),
                        },
                        ColorStop {
                            offset: 1.0,
                            color: rgba(surface.fill_bottom).into(),
                        },
                    ])
                    .with_extend(Extend::Pad),
            )
        };
        scene.fill(Fill::NonZero, self.transform(), &brush, None, &body);

        let (colour, width) = surface.border;
        if width > 0.0 && colour.3 > 0.0 {
            // Inset by half the stroke so the hairline lands *inside* the box.
            // A centred stroke puts half of itself outside, which on a
            // translucent panel shows as a faint halo against the wallpaper.
            let inset = (width / 2.0) as f64;
            let inner = RoundedRect::from_rect(
                Rect::new(
                    rect.x0 + inset,
                    rect.y0 + inset,
                    rect.x1 - inset,
                    rect.y1 - inset,
                ),
                (radius - inset).max(0.0),
            );
            scene.stroke(
                &Stroke::new(width as f64),
                self.transform(),
                rgba(colour),
                None,
                &inner,
            );
        }
    }

    fn paint_text(
        &self,
        scene: &mut Scene,
        placed: &Placed<'_>,
        string: &str,
        engine: &mut TextEngine,
    ) {
        let node = placed.node;
        let role = node.style.role;
        let style = self.theme.text(role);
        let shaping = Shaping::from_style(style, matches!(role, Role::Display | Role::Chip));
        let content = placed.content;
        let available = content.width() as f32;

        let string = if node.style.layout.wrap {
            string.to_owned()
        } else {
            // Truncation is decided here because only now is the final width
            // known — see the note in `crate::layout`.
            engine.ellipsise(string, shaping, available)
        };

        let align = match (self.rtl, node.style.layout.justify) {
            (_, Justify::Center) => Align::Center,
            // In a mirrored layout the leading edge is the right one, so Start
            // and End swap. Doing it here means nothing below this line has to
            // know the layout can be mirrored at all.
            (false, Justify::Start | Justify::SpaceBetween) => Align::Start,
            (false, Justify::End) => Align::End,
            (true, Justify::Start | Justify::SpaceBetween) => Align::End,
            (true, Justify::End) => Align::Start,
        };

        // The width is passed in both cases, wrapping or not. Wrapped text
        // needs it to break; unwrapped text needs it to *align* — there is no
        // trailing edge to align against without one — and it cannot wrap
        // anyway, having just been cut to fit.
        let shaped = engine.shape(&string, shaping, Some(available.max(0.0)), align);
        // Vertically centred in the content box. Text laid out at the top edge
        // sits visibly high next to an icon in the same row.
        let y = content.y0 + (content.height() - shaped.height() as f64) / 2.0;
        let origin = self.mirror(Rect::new(
            content.x0,
            y,
            content.x0 + available as f64,
            y + shaped.height() as f64,
        ));

        let colour = rgba(
            self.theme
                .foreground(node.style.accent, role, node.style.muted),
        );
        let colour = if node.semantics.state.disabled {
            // Disabled is a state, not a colour: dimming the resolved colour
            // keeps the hue, so a disabled critical action still reads as the
            // critical one.
            colour.multiply_alpha(0.45)
        } else {
            colour
        };
        shaped.paint(
            scene,
            (origin.x0 * self.scale, origin.y0 * self.scale),
            colour,
        );
    }

    fn paint_icon(&self, scene: &mut Scene, placed: &Placed<'_>, icon: uniwidged_view::Icon) {
        let rect = self.mirror(placed.rect);
        let affine = self.transform() * icons::transform(rect);
        let path = icons::path(icon);
        let colour = rgba(self.theme.foreground(
            placed.node.style.accent,
            placed.node.style.role,
            placed.node.style.muted,
        ));
        // The stroke width stays in unit space: the transform scales it along
        // with the path, so one constant gives every icon the same visual
        // weight at every size.
        let stroke = Stroke::new(icons::IconPath::STROKE_WIDTH)
            .with_caps(Cap::Round)
            .with_join(Join::Round);
        for path in &path.strokes {
            scene.stroke(&stroke, affine, colour, None, path);
        }
        for path in &path.fills {
            scene.fill(Fill::NonZero, affine, colour, None, path);
        }
    }

    fn paint_divider(&self, scene: &mut Scene, rect: Rect) {
        let (colour, thickness) = self.theme.divider();
        scene.fill(
            Fill::NonZero,
            self.transform(),
            rgba(colour),
            None,
            &Rect::new(
                rect.x0,
                rect.y0,
                rect.x1,
                rect.y0 + thickness.max(1.0) as f64,
            ),
        );
    }

    fn paint_progress(&self, scene: &mut Scene, placed: &Placed<'_>, value: f32) {
        let rect = self.mirror(placed.rect);
        let radius = rect.height() / 2.0;
        scene.fill(
            Fill::NonZero,
            self.transform(),
            rgba(self.theme.track()),
            None,
            &RoundedRect::from_rect(rect, radius),
        );
        if value <= 0.0 {
            return;
        }
        // The filled part grows from the leading edge, which is the right one
        // in a mirrored layout.
        let filled = rect.width() * value as f64;
        let bar = if self.rtl {
            Rect::new(rect.x1 - filled, rect.y0, rect.x1, rect.y1)
        } else {
            Rect::new(rect.x0, rect.y0, rect.x0 + filled, rect.y1)
        };
        // A fill narrower than its own corner radius would draw a lens rather
        // than a bar, so keep it at least as wide as the track is tall.
        let bar = if bar.width() < rect.height() {
            let short = rect.height();
            if self.rtl {
                Rect::new(rect.x1 - short, rect.y0, rect.x1, rect.y1)
            } else {
                Rect::new(rect.x0, rect.y0, rect.x0 + short, rect.y1)
            }
        } else {
            bar
        };
        scene.fill(
            Fill::NonZero,
            self.transform(),
            rgba(
                self.theme
                    .foreground(placed.node.style.accent, Role::Body, false),
            ),
            None,
            &RoundedRect::from_rect(bar, radius),
        );
    }
}

impl Painter<'_> {
    /// A line over a faint area, oldest point at the leading edge and the most
    /// recent at the trailing one, with a dot on the latest value.
    ///
    /// The series is drawn on the theme's terms and nothing else: the widget
    /// chose an accent, and the stroke, the fill, the marker and the height are
    /// all resolved here. Two charts by two authors are therefore the same
    /// chart, which is the whole argument of `ARCHITECTURE.md` §6 applied to
    /// the one node that could have been a canvas.
    fn paint_sparkline(
        &self,
        scene: &mut Scene,
        placed: &Placed<'_>,
        points: &[uniwidged_view::Ratio],
    ) {
        let rect = self.mirror(placed.rect);
        let stroke = self.theme.sparkline_stroke() as f64;
        let colour = rgba(self.theme.foreground(
            placed.node.style.accent,
            Role::Body,
            placed.node.style.muted,
        ));

        // The baseline is drawn whatever the series holds, so an empty chart —
        // a widget's first run, before it has a history — still shows where
        // the chart will be rather than an unexplained gap.
        let (track, thickness) = (self.theme.track(), 1.0);
        scene.fill(
            Fill::NonZero,
            self.transform(),
            rgba(track),
            None,
            &Rect::new(rect.x0, rect.y1 - thickness, rect.x1, rect.y1),
        );
        if points.is_empty() {
            return;
        }

        // Inset by the marker's radius on every side, so a value of 1.0 is a
        // dot inside the box rather than half a dot clipped by the node above.
        let radius = stroke * 1.25;
        let inner = Rect::new(
            rect.x0 + radius,
            rect.y0 + radius,
            rect.x1 - radius,
            rect.y1 - thickness - radius,
        );
        let at = |index: usize, value: f32| -> Point {
            // One point sits at the trailing edge: the latest value is the one
            // a glance is after, and a chart of one reading is that reading.
            let along = if points.len() > 1 {
                index as f64 / (points.len() - 1) as f64
            } else {
                1.0
            };
            // Leading edge to trailing edge, which in a mirrored layout runs
            // right to left. The rectangle was folded above; this folds the
            // direction of time to match.
            let x = if self.rtl {
                inner.x1 - along * inner.width()
            } else {
                inner.x0 + along * inner.width()
            };
            Point::new(x, inner.y1 - value as f64 * inner.height())
        };

        let mut line = BezPath::new();
        for (index, point) in points.iter().enumerate() {
            let p = at(index, point.get());
            if index == 0 {
                line.move_to(p);
            } else {
                line.line_to(p);
            }
        }

        if points.len() > 1 {
            // The area first, so the line sits on top of it.
            let mut area = line.clone();
            area.line_to(Point::new(at(points.len() - 1, 0.0).x, inner.y1));
            area.line_to(Point::new(at(0, 0.0).x, inner.y1));
            area.close_path();
            scene.fill(
                Fill::NonZero,
                self.transform(),
                colour.multiply_alpha(self.theme.sparkline_fill_alpha()),
                None,
                &area,
            );
            scene.stroke(
                &Stroke::new(stroke)
                    .with_caps(Cap::Round)
                    .with_join(Join::Round),
                self.transform(),
                colour,
                None,
                &line,
            );
        }

        // The latest value, marked. It is what the chart is about.
        let last = at(points.len() - 1, points[points.len() - 1].get());
        scene.fill(
            Fill::NonZero,
            self.transform(),
            colour,
            None,
            &Circle::new(last, radius),
        );
    }
}

fn rgba((r, g, b, a): (u8, u8, u8, f32)) -> Color {
    AlphaColor::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}
