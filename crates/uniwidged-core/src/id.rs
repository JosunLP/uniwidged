//! Identities that survive a rebuild.
//!
//! A widget returns a fresh view tree whenever anything changes, so identity
//! cannot come from the tree's shape — it has to be declared. Two things
//! depend on that:
//!
//! * **Accessibility.** AccessKit addresses nodes by id. If the id of the row
//!   a screen reader is on changes when the row above it updates, focus jumps.
//! * **Input.** A press and the release that completes it are two events
//!   against two separately built trees, and they have to agree on what was
//!   pressed.
//!
//! So [`NodeId`] is chosen by the widget author, not generated. It is a plain
//! integer rather than a string because it crosses the sandbox boundary on
//! every frame, and because an author who wants names can hash them.

/// An installed widget instance.
///
/// Assigned by the host at install time and stable across restarts — it names
/// the widget's data directory and its keychain namespace, so it cannot be a
/// process-local counter. Two instances of the same widget (two clocks in two
/// timezones) get two of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WidgetId(pub u64);

impl std::fmt::Display for WidgetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "widget:{:016x}", self.0)
    }
}

/// A node inside one widget's view tree, named by its author.
///
/// Scoped to the widget: node 1 in the clock and node 1 in the calendar are
/// different nodes, and the host qualifies them with the [`WidgetId`] before
/// handing anything to AccessKit.
/// Defaults to [`NodeId::ANONYMOUS`]: most nodes in a tree are decoration,
/// and naming every one of them would be noise that hides the ones that matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NodeId(pub u32);

impl NodeId {
    /// A node the author did not name.
    ///
    /// Decoration — a divider, a background, a spacer — that nothing can focus
    /// or click and no screen reader needs to address. Every anonymous node
    /// shares this id, which is exactly why nothing addressable may use it.
    pub const ANONYMOUS: NodeId = NodeId(0);

    pub const fn is_anonymous(self) -> bool {
        self.0 == Self::ANONYMOUS.0
    }
}

impl NodeId {
    /// A node named by a string, for authors who would rather not count.
    ///
    /// FNV-1a, chosen because it is four lines and deterministic across
    /// builds, platforms and the sandbox boundary — a hasher from the standard
    /// library is none of those. Collisions merge two nodes' identity, which
    /// costs a screen reader its focus and nothing worse.
    pub const fn named(name: &str) -> Self {
        let bytes = name.as_bytes();
        let mut hash: u32 = 0x811c_9dc5;
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u32;
            hash = hash.wrapping_mul(0x0100_0193);
            i += 1;
        }
        // Never collide with `ANONYMOUS`: a named node is addressable by
        // definition, and the anonymous id is the one thing that is not.
        Self(if hash == 0 { 1 } else { hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_gives_the_same_id_every_time() {
        // Stable across builds, or a screen reader loses focus on upgrade.
        assert_eq!(NodeId::named("today"), NodeId::named("today"));
        assert_ne!(NodeId::named("today"), NodeId::named("tomorrow"));
    }

    #[test]
    fn a_named_node_is_never_anonymous() {
        for name in ["", "a", "row-0", "sehr langer name mit umlauten äöü"] {
            assert!(!NodeId::named(name).is_anonymous(), "{name:?}");
        }
    }

    #[test]
    fn widget_ids_print_readably_for_logs_and_paths() {
        assert_eq!(WidgetId(0x2a).to_string(), "widget:000000000000002a");
    }
}
