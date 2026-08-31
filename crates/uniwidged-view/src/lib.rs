//! What a widget says it wants shown.
//!
//! This is the only thing that crosses the sandbox boundary, and it is pure
//! data: a tree of nodes with a named style, some text, and children. There is
//! no colour in it, no font, no pixel and no coordinate. `ARCHITECTURE.md` §6
//! argues the case; the short version is that a screen full of third-party
//! widgets has to look like one product, and that is only true if one
//! participant decides what things look like.
//!
//! So a widget asks for [`Role::Title`], and the theme decides that is 15 px
//! semibold at 92 % opacity. A widget asks for [`Accent::Critical`], and the
//! theme decides that is the red that works on this wallpaper in this
//! appearance at this contrast setting. The widget author gets a vocabulary
//! rather than a canvas, which is a smaller thing to learn and a much smaller
//! thing to get wrong.
//!
//! The type is shared verbatim between host and guest — `uniwidged-guest`
//! builds one of these and `uniwidged-compose` consumes it — so there is no
//! schema to keep in sync, only a version to check.

pub mod build;
pub mod wire;

use uniwidged_core::NodeId;

/// One frame's worth of what a widget wants shown.
///
/// Compared against the previous frame's by plain equality. Equal means
/// nothing is submitted to the GPU at all — which is why every type in this
/// module derives [`PartialEq`] and why none of them contains a float that is
/// computed rather than declared.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    pub root: Node,
}

impl View {
    /// Has anything changed since the last frame?
    ///
    /// The whole damage model, in one line. A clock whose minute has not
    /// turned yet returns an identical tree, this says so, and the host does
    /// no layout, no shaping, no painting and no submit.
    pub fn differs_from(&self, previous: &View) -> bool {
        self != previous
    }
}

/// A node in the tree.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    /// Chosen by the author and stable across rebuilds — see
    /// [`uniwidged_core::NodeId`]. Anything clickable or readable by a screen
    /// reader must be named; decoration need not be.
    pub id: NodeId,
    pub content: Content,
    pub style: Style,
    pub semantics: Semantics,
    pub children: Vec<Node>,
}

/// What the node actually is.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Content {
    /// A container. Carries no content of its own; lays out its children.
    #[default]
    Group,
    /// A run of text. Shaping, bidi, font fallback, truncation and wrapping
    /// are all the host's — the widget supplies the string and the role.
    Text(String),
    /// One of the framework's own icons, drawn as a path so it looks the same
    /// on every platform.
    ///
    /// Ephemeris learned this the hard way: its refresh symbol was a Segoe
    /// Fluent Icons glyph on Windows and a drawn path everywhere else, and no
    /// font on macOS or Linux could be relied on to carry it. One widget with
    /// two different refresh symbols is two widgets.
    Icon(Icon),
    /// A bitmap the widget supplied through the image capability. The handle
    /// is opaque; the pixels live host-side and never cross the boundary more
    /// than once.
    Image(ImageHandle),
    /// A horizontal rule.
    Divider,
    /// Flexible empty space. The standard way to push a node to the far edge.
    Spacer,
    /// A determinate progress bar, `0.0..=1.0`.
    Progress(Ratio),
    /// A small chart of a series — a load history, a day of temperatures —
    /// drawn by the host from the values alone: a line, a faint area under
    /// it, and the most recent value at the trailing edge. The widget supplies
    /// numbers in `0.0..=1.0`; the accent, the stroke, the fill and the
    /// height are the theme's.
    ///
    /// This is as far as this framework goes towards a canvas, and it is
    /// deliberately declarative: the *series* crosses the boundary, not the
    /// pixels, so a screen of charts by different authors is still one chart
    /// style. See `ARCHITECTURE.md` §6.
    Sparkline(Series),
}

/// A series of readings for a [`Content::Sparkline`], oldest first.
///
/// Each value is a [`Ratio`], so equality is exact and total — which is what
/// lets a widget return the same history twice and have the host draw nothing
/// the second time. At most [`Series::MAX_POINTS`]: more is refused at the
/// boundary, and [`build::sparkline`] keeps the most recent ones rather than
/// asking an author to count.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series(pub Vec<Ratio>);

impl Series {
    /// The most points one chart may carry.
    ///
    /// A widget is a small panel, and 256 points is one per pixel across a
    /// wide one. Past that a chart is a table nobody can read, and the cap is
    /// what keeps a runaway history from being a host allocation.
    pub const MAX_POINTS: usize = 256;

    /// The most recent value, which is the one a chart is usually about.
    pub fn latest(&self) -> Option<Ratio> {
        self.0.last().copied()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A fraction in `0.0..=1.0`, clamped on construction.
///
/// A newtype rather than an `f32` so that it compares exactly and cannot be
/// `NaN` — both of which the damage check in [`View::differs_from`] depends on.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Ratio(f32);

impl Ratio {
    pub fn new(value: f32) -> Self {
        // `NaN` fails both comparisons and falls through to zero, which is
        // what keeps equality total.
        Self(if value > 1.0 {
            1.0
        } else if value > 0.0 {
            value
        } else {
            0.0
        })
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

/// The framework's icon set. Drawn paths, identical on every platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Refresh,
    Settings,
    Close,
    Chevron,
    Check,
    Alert,
    Offline,
}

/// A bitmap registered with the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ImageHandle(pub u32);

/// How a node is styled and laid out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub role: Role,
    pub accent: Accent,
    pub layout: Layout,
    /// Dimmed, for content that is present but secondary — a past event, a
    /// stale reading. A single named step rather than an opacity, so that the
    /// theme can make it a contrast change instead where the user has asked
    /// for high contrast.
    pub muted: bool,
}

/// A named style, resolved by the theme.
///
/// Deliberately a closed set. An open one — arbitrary font sizes and weights —
/// is how a widget collection stops looking like a widget collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Role {
    /// A plain container with no surface of its own.
    #[default]
    Plain,
    /// The widget's outermost surface: the glass body, its corner radius, its
    /// shadow. Exactly one node per widget should have this.
    Panel,
    /// A raised surface inside the panel.
    Card,
    /// A small pill — a count, a tag, a status.
    Chip,
    /// The widget's name. One per widget.
    Title,
    /// Large numeric display: a clock, a temperature, a percentage. Tabular
    /// figures, so it does not jitter as digits change.
    Display,
    /// A section heading within the widget.
    Section,
    /// Ordinary content.
    Body,
    /// Ordinary content, emphasised.
    BodyStrong,
    /// Secondary content: a timestamp, a unit, a source.
    Caption,
}

/// A semantic colour, resolved by the theme.
///
/// Named by meaning rather than by hue, so that the theme can pick a red that
/// actually reads against this wallpaper, in this appearance, at this contrast
/// setting — and so that a widget stays legible when the user switches any of
/// those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Accent {
    #[default]
    Neutral,
    /// The user's accent colour, taken from the desktop where the platform
    /// exposes it.
    Brand,
    Positive,
    Warning,
    Critical,
    /// One of the theme's categorical colours, for series that need to be
    /// distinguishable rather than meaningful — calendar sources, chart
    /// series. Wraps around the palette, so any index is valid.
    Series(u8),
}

/// How a node arranges its children and sizes itself.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub direction: Direction,
    /// Space between children, in theme steps rather than pixels — `1` is the
    /// theme's base spacing unit. Keeps rhythm consistent across widgets that
    /// have never heard of each other.
    pub gap: u8,
    pub padding: u8,
    pub align: Align,
    pub justify: Justify,
    /// Share of the leftover space along the parent's axis. `0` means "take
    /// what you need".
    pub grow: u8,
    /// Wrap text rather than truncating it. Off by default: a widget is small,
    /// and silently growing taller than the user sized it is worse than an
    /// ellipsis.
    pub wrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    #[default]
    Column,
    Row,
}

/// Alignment across the layout axis.
///
/// The default is [`Align::Stretch`], which is also flexbox's own default and
/// is what makes the common shapes work without saying anything: a row inside a
/// panel spans the panel, a divider spans the row it is in, and a label with a
/// value pushed to the far edge has an edge to be pushed to. With `Start` as
/// the default, every one of those is sized to its content instead and the
/// layout silently collapses leftwards.
///
/// Nodes that should hug their content regardless — a chip, an icon — say so by
/// having a size of their own, which overrides a parent's stretch exactly as it
/// does in CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

/// Distribution along the layout axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

/// What this node means, for input and for a screen reader.
///
/// The host builds the AccessKit tree from these, so a widget author gets
/// accessibility by describing what a node *is* rather than by doing any extra
/// work. A node with [`Interaction::None`] and no label contributes nothing to
/// the tree, which is the right answer for decoration.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Semantics {
    /// What a screen reader reads. Falls back to the node's own text.
    pub label: Option<String>,
    pub interaction: Interaction,
    pub state: State,
    /// Shown on hover, and read as a description.
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Interaction {
    #[default]
    None,
    /// Reports `clicked(id)` back to the widget.
    Button,
    /// Reports `toggled(id, bool)`.
    Toggle,
    /// A row in a list. Focusable and activatable.
    ListItem,
    /// Opens a URL. The host checks the scheme before handing it to the
    /// platform's opener — a URL that came out of somebody else's calendar
    /// server is not to be trusted with `file://` or a custom handler.
    Link(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct State {
    pub disabled: bool,
    pub selected: bool,
    pub checked: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_view_is_not_a_frame() {
        // The damage model. A clock whose minute has not turned returns the
        // same tree, and nothing reaches the GPU.
        let a = build::panel(NodeId::named("root"))
            .child(build::text("11:58"))
            .view();
        let b = build::panel(NodeId::named("root"))
            .child(build::text("11:58"))
            .view();
        assert!(!a.differs_from(&b));
    }

    #[test]
    fn a_changed_string_is_a_frame() {
        let a = build::panel(NodeId::named("root"))
            .child(build::text("11:58"))
            .view();
        let b = build::panel(NodeId::named("root"))
            .child(build::text("11:59"))
            .view();
        assert!(a.differs_from(&b));
    }

    #[test]
    fn a_ratio_is_clamped_and_never_nan() {
        // Equality has to stay total: a `NaN` here would make every frame
        // differ from itself and pin the renderer at the refresh rate.
        assert_eq!(Ratio::new(2.0).get(), 1.0);
        assert_eq!(Ratio::new(-1.0).get(), 0.0);
        assert_eq!(Ratio::new(f32::NAN).get(), 0.0);
        assert_eq!(Ratio::new(f32::NAN), Ratio::new(f32::NAN));
    }
}
