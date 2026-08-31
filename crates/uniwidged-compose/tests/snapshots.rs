//! Golden images.
//!
//! Every visual claim in `ARCHITECTURE.md` §7 has one of these behind it, so
//! that a regression in a colour, a weight, a radius or a shadow fails CI
//! rather than shipping. They render through the whole stack — view tree →
//! layout → Parley → Vello → GPU → readback — which is the same path a widget
//! takes on a real desktop, minus the swapchain. `spikes/m0_layer_shell`
//! proves that the readback and the on-screen paths agree, which is what makes
//! these images evidence about what a user sees.
//!
//! ## They are deterministic on purpose
//!
//! Two things would otherwise make them differ between a laptop and CI for
//! reasons nobody could act on:
//!
//! * **Fonts.** The suite bundles DejaVu Sans and shapes everything in it,
//!   rather than using whatever the machine has installed.
//! * **The rasteriser.** A software Vulkan device (lavapipe) is deterministic;
//!   a GPU driver is not, quite. CI installs `mesa-vulkan-drivers`.
//!
//! ## Regenerating
//!
//! ```sh
//! UNIWIDGED_UPDATE_SNAPSHOTS=1 cargo test -p uniwidged-compose --test snapshots
//! ```
//!
//! Then **look at the diff**. A golden-image suite that is regenerated without
//! anyone examining the change is a suite that approves its own regressions.

use std::path::{Path, PathBuf};

use uniwidged_compose::text::TextEngine;
use uniwidged_compose::theme::{Appearance, Contrast, Oklch};
use uniwidged_compose::{Compose, Compositor, Theme};
use uniwidged_core::{NodeId, Size, WidgetId};
use uniwidged_render::Renderer;
use uniwidged_view::build::*;
use uniwidged_view::{Accent, View};

const FONT: &[u8] = include_bytes!("fonts/DejaVuSans.ttf");
const FAMILY: &str = "DejaVu Sans";

/// How far a pixel may drift before it counts as a change.
///
/// Not zero: the same scene through the same software rasteriser is
/// bit-identical, but a Mesa point release can move an anti-aliased edge by a
/// unit. Two is small enough that any real change — a colour, a weight, a
/// radius — is far outside it.
const TOLERANCE: u8 = 2;

/// How many pixels may sit at the tolerance before the image counts as changed.
const MAX_DIFFERING_FRACTION: f64 = 0.002;

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn compositor(theme: Theme) -> Compositor {
    Compositor::with_text_engine(theme, TextEngine::with_font(FONT.to_vec(), FAMILY))
}

/// A theme whose brand colour is fixed, so a desktop accent cannot leak in.
fn theme(appearance: Appearance, contrast: Contrast) -> Theme {
    Theme {
        appearance,
        contrast,
        brand: Oklch::new(0.62, 0.14, 250.0),
        ..Theme::default()
    }
}

struct Case {
    name: &'static str,
    view: View,
    theme: Theme,
    size: Size,
    scale: f64,
    rtl: bool,
}

impl Case {
    fn new(name: &'static str, view: View, theme: Theme) -> Self {
        Self {
            name,
            view,
            theme,
            size: Size::new(320.0, 200.0),
            scale: 1.0,
            rtl: false,
        }
    }

    fn size(mut self, w: f64, h: f64) -> Self {
        self.size = Size::new(w, h);
        self
    }

    fn scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    fn rtl(mut self) -> Self {
        self.rtl = true;
        self
    }
}

/// Every role, so that a change to any one of them is visible.
fn every_role() -> View {
    panel(NodeId::named("root"))
        .gap(1)
        .child(text("Display 62%").role(Role::Display))
        .child(text("Title").role(Role::Title))
        .child(text("Section").role(Role::Section))
        .child(text("Body strong").role(Role::BodyStrong))
        .child(text("Body").role(Role::Body))
        .child(text("Caption").role(Role::Caption))
        .child(row().gap(1).child(chip("Chip")).child(spacer()))
        .child(divider())
        .child(card().child(text("Inside a card").role(Role::Body)))
        .view()
}

/// Every semantic accent, on text and on a bar.
fn every_accent() -> View {
    let mut column = panel(NodeId::named("root")).gap(1);
    for (label, accent) in [
        ("Neutral", Accent::Neutral),
        ("Brand", Accent::Brand),
        ("Positive", Accent::Positive),
        ("Warning", Accent::Warning),
        ("Critical", Accent::Critical),
    ] {
        column = column.child(
            row()
                .gap(1)
                .child(text(label).role(Role::Caption).accent(accent))
                .child(spacer())
                .child(chip("12").accent(accent)),
        );
    }
    column.view()
}

/// The categorical palette, which has to look like one set.
fn series() -> View {
    let mut column = panel(NodeId::named("root")).gap(1);
    for i in 0..8u8 {
        column = column.child(progress(0.3 + i as f32 * 0.08).accent(Accent::Series(i)));
    }
    column.view()
}

fn all_icons() -> View {
    panel(NodeId::named("root"))
        .child(
            row().gap(2).children(
                [
                    Icon::Refresh,
                    Icon::Settings,
                    Icon::Close,
                    Icon::Chevron,
                    Icon::Check,
                    Icon::Alert,
                    Icon::Offline,
                ]
                .into_iter()
                .map(icon),
            ),
        )
        .view()
}

/// A realistic widget: the shape a real one takes.
fn system_load() -> View {
    panel(NodeId::named("root"))
        .gap(2)
        .child(
            row()
                .child(text("System").role(Role::Title))
                .child(spacer())
                .child(chip("live").accent(Accent::Positive))
                .child(
                    icon(Icon::Refresh)
                        .on_click(NodeId::named("refresh"))
                        .label("Refresh"),
                ),
        )
        .child(divider())
        .child(meter("CPU", 0.32, Accent::Neutral))
        .child(meter("Memory", 0.78, Accent::Warning))
        .child(meter("Disk", 0.94, Accent::Critical))
        .view()
}

fn meter(name: &str, value: f32, accent: Accent) -> Node {
    column()
        .child(
            row()
                .child(text(name).role(Role::Caption))
                .child(spacer())
                .child(
                    text(format!("{:.0}%", value * 100.0))
                        .role(Role::Caption)
                        .accent(accent),
                ),
        )
        .child(progress(value).accent(accent))
}

/// Charts: a history in every state a widget can hand over.
///
/// A full series in the brand accent, one that has just begun, one point, none
/// at all, and the categorical palette — so that a change to the stroke, the
/// fill, the marker or the baseline is visible in one image.
fn charts() -> View {
    // Deterministic and busy enough to have a shape: a slow wave with a fast
    // one on top, which is what a load history tends to look like.
    let wave = |n: usize| {
        (0..n).map(|i| {
            let t = i as f32 / 8.0;
            0.5 + 0.3 * (t * 0.7).sin() + 0.12 * (t * 3.1).cos()
        })
    };
    panel(NodeId::named("root"))
        .gap(2)
        .child(
            row()
                .child(text("CPU").role(Role::Caption))
                .child(spacer())
                .child(chip("64%").accent(Accent::Brand)),
        )
        .child(sparkline(wave(48)).accent(Accent::Brand).label("CPU"))
        .child(
            row()
                .child(text("Memory").role(Role::Caption))
                .child(spacer())
                .child(chip("91%").accent(Accent::Critical)),
        )
        .child(
            sparkline([0.7, 0.72, 0.8, 0.86, 0.91])
                .accent(Accent::Critical)
                .label("Memory"),
        )
        .child(text("Just started").role(Role::Caption))
        .child(
            sparkline([0.35])
                .accent(Accent::Series(2))
                .label("One point"),
        )
        .child(text("Nothing yet").role(Role::Caption).muted())
        .child(sparkline(std::iter::empty()).label("Empty"))
        .view()
}

fn truncation() -> View {
    panel(NodeId::named("root"))
        .gap(1)
        .child(text("A calendar entry far too long for this panel").role(Role::Body))
        .child(text("Short enough").role(Role::Body))
        .child(
            text("A wrapped block that runs onto several lines because it was told to wrap")
                .role(Role::Caption)
                .wrap(),
        )
        .view()
}

fn arabic() -> View {
    panel(NodeId::named("root"))
        .gap(1)
        .child(text("اجتماع الفريق").role(Role::Title))
        .child(
            row()
                .child(text("الطقس").role(Role::Body))
                .child(spacer())
                .child(text("18°").role(Role::Display)),
        )
        .child(progress(0.62).accent(Accent::Brand))
        .view()
}

fn cases() -> Vec<Case> {
    use Appearance::*;
    use Contrast::*;
    vec![
        Case::new("roles-dark", every_role(), theme(Dark, Standard)).size(220.0, 280.0),
        Case::new("roles-light", every_role(), theme(Light, Standard)).size(220.0, 280.0),
        Case::new("roles-high-contrast", every_role(), theme(Dark, High)).size(220.0, 280.0),
        Case::new("accents-dark", every_accent(), theme(Dark, Standard)).size(200.0, 170.0),
        Case::new("accents-light", every_accent(), theme(Light, Standard)).size(200.0, 170.0),
        Case::new("series-dark", series(), theme(Dark, Standard)).size(200.0, 160.0),
        Case::new("icons-dark", all_icons(), theme(Dark, Standard)).size(220.0, 60.0),
        Case::new("widget-dark", system_load(), theme(Dark, Standard)),
        Case::new("widget-light", system_load(), theme(Light, Standard)),
        Case::new("widget-high-contrast", system_load(), theme(Dark, High)),
        Case::new("widget-scale-150", system_load(), theme(Dark, Standard)).scale(1.5),
        Case::new("widget-scale-200", system_load(), theme(Dark, Standard)).scale(2.0),
        Case::new("charts-dark", charts(), theme(Dark, Standard)).size(220.0, 230.0),
        Case::new("charts-light", charts(), theme(Light, Standard)).size(220.0, 230.0),
        Case::new("charts-high-contrast", charts(), theme(Dark, High)).size(220.0, 230.0),
        Case::new("charts-rtl", charts(), theme(Dark, Standard))
            .rtl()
            .size(220.0, 230.0),
        Case::new("text-truncation", truncation(), theme(Dark, Standard)).size(220.0, 140.0),
        Case::new("rtl-arabic", arabic(), theme(Dark, Standard))
            .rtl()
            .size(240.0, 140.0),
        Case::new("ltr-arabic", arabic(), theme(Dark, Standard)).size(240.0, 140.0),
    ]
}

#[test]
fn golden_images() {
    let updating = std::env::var_os("UNIWIDGED_UPDATE_SNAPSHOTS").is_some();
    let mut renderer = match Renderer::new() {
        Ok(r) => r,
        Err(e) => {
            if std::env::var_os("UNIWIDGED_REQUIRE_GPU").is_some() {
                panic!("UNIWIDGED_REQUIRE_GPU is set but there is no adapter: {e}");
            }
            eprintln!("SKIPPED: {e}");
            return;
        }
    };

    let mut failures = Vec::new();
    let mut written = Vec::new();
    for case in cases() {
        let physical = (
            (case.size.width * case.scale).round() as u32,
            (case.size.height * case.scale).round() as u32,
        );
        let surface = renderer
            .add_readback_surface(physical.0, physical.1)
            .expect("surface");

        let mut compositor = compositor(case.theme);
        let composed = compositor.compose(
            Compose::new(&case.view, WidgetId(1), case.name, case.size)
                .scale(case.scale)
                .rtl(case.rtl),
        );
        renderer
            .render(&[(surface, &composed.scene)])
            .expect("render");
        let image = renderer.read_pixels(surface).expect("readback");
        renderer.remove_surface(surface);

        let path = snapshot_dir().join(format!("{}.png", case.name));
        if updating || !path.exists() {
            std::fs::create_dir_all(snapshot_dir()).expect("snapshot directory");
            image.write_png(&path).expect("write");
            written.push(case.name);
            continue;
        }
        if let Err(why) = compare(&image, &path) {
            // Write what was actually produced next to the reference, so the
            // failure can be looked at rather than only read about.
            let actual = snapshot_dir().join(format!("{}.actual.png", case.name));
            let _ = image.write_png(&actual);
            failures.push(format!("{}: {why}", case.name));
        }
    }

    if !written.is_empty() {
        // Not a silent success: a suite that regenerates without anyone looking
        // is a suite that approves its own regressions.
        eprintln!(
            "wrote {} reference image(s): {}",
            written.len(),
            written.join(", ")
        );
        assert!(
            updating,
            "reference images were missing and have been written — review them, \
             then run the suite again"
        );
    }
    assert!(
        failures.is_empty(),
        "golden images changed:\n  {}",
        failures.join("\n  ")
    );
}

fn compare(image: &uniwidged_render::Image, reference: &Path) -> Result<(), String> {
    let file = std::io::BufReader::new(std::fs::File::open(reference).map_err(|e| e.to_string())?);
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;

    if (info.width, info.height) != (image.width, image.height) {
        return Err(format!(
            "size changed: {}x{} against {}x{}",
            image.width, image.height, info.width, info.height
        ));
    }
    let expected = &buffer[..info.buffer_size()];
    if expected.len() != image.data.len() {
        return Err("channel layout changed".into());
    }

    let mut differing = 0usize;
    let mut worst = 0u8;
    for (got, want) in image
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.as_chunks::<4>().0)
    {
        let delta = got
            .iter()
            .zip(want)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        worst = worst.max(delta);
        if delta > TOLERANCE {
            differing += 1;
        }
    }
    let total = (image.width * image.height) as f64;
    let fraction = differing as f64 / total;
    if fraction > MAX_DIFFERING_FRACTION {
        return Err(format!(
            "{differing} of {total:.0} pixels ({:.2}%) differ by more than {TOLERANCE}, \
             worst {worst}",
            fraction * 100.0
        ));
    }
    Ok(())
}
