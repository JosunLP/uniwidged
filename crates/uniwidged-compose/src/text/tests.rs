//! Text tests. These need real fonts but no GPU and no window.

use super::*;

fn body() -> Shaping {
    Shaping {
        size: 13.0,
        weight: 400.0,
        line_height: 1.35,
        tabular: false,
    }
}

#[test]
fn text_has_a_positive_extent() {
    let mut engine = TextEngine::new();
    let (w, h) = engine.measure("Widget", body());
    assert!(w > 0.0, "width should be positive, got {w}");
    assert!(h >= 13.0, "13px text should be at least that tall, got {h}");
}

#[test]
fn weight_is_actually_applied() {
    // The whole reason this module exists rather than using `ferrum-text`. If
    // bold and regular shape to the same width, the hierarchy is invisible and
    // at high contrast there is no hierarchy left at all.
    let mut engine = TextEngine::new();
    let regular = engine.measure("Handgloves", body());
    let bold = engine.measure(
        "Handgloves",
        Shaping {
            weight: 700.0,
            ..body()
        },
    );
    assert!(
        bold.0 > regular.0,
        "bold should be wider than regular: {} vs {}",
        bold.0,
        regular.0
    );
}

#[test]
fn tabular_figures_do_not_change_width_as_the_digits_change() {
    // A clock in proportional digits shifts its whole line once a second.
    let mut engine = TextEngine::new();
    let tabular = Shaping {
        tabular: true,
        ..body()
    };
    let widths: Vec<f32> = ["00:00", "11:11", "18:48", "23:59"]
        .iter()
        .map(|t| engine.measure(t, tabular).0)
        .collect();
    let lowest = widths.iter().cloned().fold(f32::MAX, f32::min);
    let highest = widths.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        highest - lowest < 0.5,
        "tabular digits varied from {lowest} to {highest}: {widths:?}"
    );
}

#[test]
fn shaping_is_cached_across_repeated_measurement() {
    // Every widget re-measures its unchanged text on every rebuild. Reshaping
    // it each time would be the most expensive thing an idle desktop does.
    let mut engine = TextEngine::new();
    engine.measure("Tuesday, 27 August", body());
    assert_eq!(engine.cache_len(), 1);
    engine.measure("Tuesday, 27 August", body());
    assert_eq!(engine.cache_len(), 1, "the same text reshaped");
    engine.measure("Wednesday, 28 August", body());
    assert_eq!(engine.cache_len(), 2);
}

#[test]
fn a_different_weight_is_a_different_shaping() {
    // Weight changes glyph outlines and advances, so it has to be part of the
    // cache key — sharing a shaping across weights would silently draw the
    // wrong one.
    let mut engine = TextEngine::new();
    engine.measure("Title", body());
    engine.measure(
        "Title",
        Shaping {
            weight: 600.0,
            ..body()
        },
    );
    assert_eq!(engine.cache_len(), 2);
}

#[test]
fn text_that_fits_is_left_exactly_alone() {
    let mut engine = TextEngine::new();
    let text = "Short";
    let natural = engine.measure(text, body()).0;
    assert_eq!(engine.ellipsise(text, body(), natural + 10.0), text);
}

#[test]
fn text_that_does_not_fit_is_ellipsised_to_fit() {
    let mut engine = TextEngine::new();
    let text = "A rather long calendar entry that will not fit";
    let limit = 80.0;
    let cut = engine.ellipsise(text, body(), limit);
    assert!(cut.ends_with('…'), "{cut:?} should end with an ellipsis");
    assert!(cut.chars().count() < text.chars().count());
    assert!(
        engine.measure(&cut, body()).0 <= limit,
        "{cut:?} is still {} wide, limit {limit}",
        engine.measure(&cut, body()).0
    );
}

#[test]
fn ellipsising_keeps_as_much_as_will_fit() {
    // A bisection that is off by one throws away a character that fits, and
    // over a column of rows that reads as a rendering fault.
    let mut engine = TextEngine::new();
    let text = "Standup with the platform team";
    let limit = 120.0;
    let cut = engine.ellipsise(text, body(), limit);
    let kept = cut.chars().count() - 1;
    let one_more: String = text
        .chars()
        .take(kept + 1)
        .chain(std::iter::once('…'))
        .collect();
    assert!(
        engine.measure(&one_more, body()).0 > limit,
        "one more character would still have fitted: {one_more:?}"
    );
}

#[test]
fn ellipsising_never_splits_a_character() {
    // Truncating by byte would cut a multi-byte character in half, which is a
    // panic in the best case and a replacement glyph in the worst.
    let mut engine = TextEngine::new();
    for text in [
        "Übermorgen früh Grüße",
        "会議の予定が入っています",
        "اجتماع الفريق غدا",
    ] {
        for limit in [8.0, 20.0, 45.0, 90.0] {
            let cut = engine.ellipsise(text, body(), limit);
            // Reaching here at all means no byte-boundary panic; and every
            // character in the result has to have come from the input.
            assert!(
                cut.chars().all(|c| c == '…' || text.contains(c)),
                "{cut:?} invented a character from {text:?}"
            );
        }
    }
}

#[test]
fn a_hopeless_width_still_says_there_is_more() {
    // An empty string reads as "nothing here". A bare ellipsis reads as
    // "something here that does not fit", which is the truth.
    let mut engine = TextEngine::new();
    assert_eq!(engine.ellipsise("Something", body(), 1.0), "…");
    assert_eq!(engine.ellipsise("Something", body(), 0.0), "");
}

#[test]
fn a_truncation_does_not_end_on_a_space() {
    // "Standup with …" reads as a missing word; "Standup with…" reads as a
    // cut one.
    let mut engine = TextEngine::new();
    let cut = engine.ellipsise("Standup with the platform team", body(), 95.0);
    assert!(!cut.contains(" …"), "{cut:?} ends on a space");
}

#[test]
fn wrapping_at_a_width_produces_more_than_one_line() {
    let mut engine = TextEngine::new();
    let text = "A tooltip long enough that it has to wrap onto several lines";
    let unwrapped = engine.shape(text, body(), None, Align::Start);
    let wrapped = engine.shape(text, body(), Some(120.0), Align::Start);
    assert!(wrapped.width() <= 120.5);
    assert!(
        wrapped.height() > unwrapped.height() * 1.5,
        "wrapped text should be several lines tall"
    );
}
