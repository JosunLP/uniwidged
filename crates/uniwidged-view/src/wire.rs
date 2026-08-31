//! The flat form a view tree takes crossing the sandbox boundary.
//!
//! WIT has no recursive types — a record cannot contain a list of itself — so
//! the tree travels as an arena: a flat list of nodes, each naming its children
//! by index, with node 0 as the root.
//!
//! That is not only a workaround. A flat list is one allocation on each side
//! instead of one per node, and it is what lets the host check the whole
//! structure *before* touching it. A recursive decoder discovers a cycle by
//! running out of stack, which on a desktop process means the widget takes the
//! desktop with it.
//!
//! ## What is checked, and why each one matters
//!
//! The arena arrives from a sandbox, so it is hostile input until proven
//! otherwise. [`Tree::into_view`] rejects:
//!
//! * **A child index out of range** — a read past the end.
//! * **A child index that is not greater than its parent's** — this is the one
//!   that matters. Requiring the arena to be topologically ordered makes a
//!   cycle *impossible to express* rather than merely detected, so there is no
//!   visited-set to get wrong and no depth at which the check gives up.
//! * **A node with two parents** — a DAG is not a tree, and the second visit
//!   would duplicate a subtree, including its ids.
//! * **An unreachable node** — the widget built something it did not attach.
//!   Silently dropping it hides a bug that will look like a rendering fault.
//! * **Too many nodes, a string too long, or a series too long** — a widget
//!   with a memory cap can still ask the host to allocate on its behalf.
//!
//! Depth needs no separate limit: the ordering rule means depth cannot exceed
//! the node count, and the node count is capped.

use crate::{Content, Node, Semantics, Series, Style, View};
use uniwidged_core::NodeId;

/// The most nodes one widget may describe.
///
/// A widget is a small panel. A thousand nodes is already far more than any
/// real one — the reference system-load widget uses fourteen — and the cap
/// exists so that a runaway loop in guest code costs a rejected frame rather
/// than the host's memory.
pub const MAX_NODES: usize = 1024;

/// The longest string a single node may carry.
///
/// Generous for a label and far short of what it takes to matter. A widget
/// that wants to show more than this has a scrolling problem, not a limit
/// problem.
pub const MAX_TEXT: usize = 4096;

/// A view tree in its flat, wire form.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    /// Node 0 is the root. An empty list is a widget with nothing to show yet,
    /// which is a normal state before its first data arrives.
    pub nodes: Vec<WireNode>,
}

/// One node of the arena.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WireNode {
    pub id: u32,
    pub content: Content,
    pub style: Style,
    pub semantics: Semantics,
    /// Indices into [`Tree::nodes`], each strictly greater than this node's own.
    pub children: Vec<u32>,
}

/// Why an arena was refused.
///
/// Each variant names the node it was found at, because "the widget sent a bad
/// tree" is not something anybody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    /// More nodes than [`MAX_NODES`].
    TooLarge { nodes: usize },
    /// A string longer than [`MAX_TEXT`].
    TextTooLong { node: usize, length: usize },
    /// A sparkline with more points than [`Series::MAX_POINTS`].
    SeriesTooLong { node: usize, points: usize },
    /// A child index past the end of the arena.
    ChildOutOfRange { node: usize, child: u32 },
    /// A child index that does not come after its parent. This is what a cycle
    /// looks like from here — see the module note.
    ChildNotAfterParent { node: usize, child: u32 },
    /// A node named as a child by two different parents.
    SharedChild { node: usize, child: u32 },
    /// A node no path from the root reaches.
    Unreachable { node: usize },
}

impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TreeError::TooLarge { nodes } => {
                write!(f, "the view has {nodes} nodes; the limit is {MAX_NODES}")
            }
            TreeError::TextTooLong { node, length } => write!(
                f,
                "node {node} carries {length} bytes of text; the limit is {MAX_TEXT}"
            ),
            TreeError::SeriesTooLong { node, points } => write!(
                f,
                "node {node} charts {points} points; the limit is {}",
                Series::MAX_POINTS
            ),
            TreeError::ChildOutOfRange { node, child } => {
                write!(f, "node {node} names child {child}, which does not exist")
            }
            TreeError::ChildNotAfterParent { node, child } => write!(
                f,
                "node {node} names child {child}, which does not come after it — \
                 children must be later in the arena than their parent"
            ),
            TreeError::SharedChild { node, child } => write!(
                f,
                "node {child} is named as a child by node {node} and by another; \
                 a view is a tree, not a graph"
            ),
            TreeError::Unreachable { node } => {
                write!(f, "node {node} is not attached to the root")
            }
        }
    }
}

impl std::error::Error for TreeError {}

impl View {
    /// Flattens into the wire form, depth first.
    ///
    /// Children always land after their parent, which is the invariant the
    /// host's validation relies on — so a tree this produced always passes it.
    /// [`round_trips`](Tree::into_view) is asserted by the tests.
    pub fn flatten(&self) -> Tree {
        let mut tree = Tree { nodes: Vec::new() };
        push(&mut tree, &self.root);
        tree
    }
}

fn push(tree: &mut Tree, node: &Node) -> u32 {
    let index = tree.nodes.len() as u32;
    tree.nodes.push(WireNode {
        id: node.id.0,
        content: node.content.clone(),
        style: node.style.clone(),
        semantics: node.semantics.clone(),
        children: Vec::new(),
    });
    // The children are pushed first and their indices collected afterwards,
    // because a child's index is only known once it is in the arena.
    let children: Vec<u32> = node.children.iter().map(|c| push(tree, c)).collect();
    tree.nodes[index as usize].children = children;
    index
}

impl Tree {
    /// Validates and rebuilds the tree.
    ///
    /// See the module note for what is checked. An empty arena yields an empty
    /// view rather than an error: a widget with nothing to show yet is normal.
    pub fn into_view(self) -> Result<View, TreeError> {
        if self.nodes.len() > MAX_NODES {
            return Err(TreeError::TooLarge {
                nodes: self.nodes.len(),
            });
        }
        if self.nodes.is_empty() {
            return Ok(View::default());
        }

        let count = self.nodes.len();
        // One pass over the arena settles every structural rule. Doing it up
        // front means the rebuild below cannot encounter anything surprising,
        // and so needs no error path of its own.
        let mut parents: Vec<Option<usize>> = vec![None; count];
        for (index, node) in self.nodes.iter().enumerate() {
            if let Content::Text(text) = &node.content
                && text.len() > MAX_TEXT
            {
                return Err(TreeError::TextTooLong {
                    node: index,
                    length: text.len(),
                });
            }
            if let Some(label) = &node.semantics.label
                && label.len() > MAX_TEXT
            {
                return Err(TreeError::TextTooLong {
                    node: index,
                    length: label.len(),
                });
            }
            if let Content::Sparkline(series) = &node.content
                && series.len() > Series::MAX_POINTS
            {
                return Err(TreeError::SeriesTooLong {
                    node: index,
                    points: series.len(),
                });
            }
            for &child in &node.children {
                let child_index = child as usize;
                if child_index >= count {
                    return Err(TreeError::ChildOutOfRange { node: index, child });
                }
                if child_index <= index {
                    return Err(TreeError::ChildNotAfterParent { node: index, child });
                }
                if parents[child_index].is_some() {
                    return Err(TreeError::SharedChild { node: index, child });
                }
                parents[child_index] = Some(index);
            }
        }
        // Node 0 is the root and has no parent; everything else must have one.
        // Reachability follows: with every non-root node having exactly one
        // parent and every parent earlier in the arena, following parents from
        // any node terminates at 0.
        for (index, parent) in parents.iter().enumerate().skip(1) {
            if parent.is_none() {
                return Err(TreeError::Unreachable { node: index });
            }
        }

        Ok(View {
            root: rebuild(&self.nodes, 0),
        })
    }
}

fn rebuild(nodes: &[WireNode], index: usize) -> Node {
    let wire = &nodes[index];
    Node {
        id: NodeId(wire.id),
        content: wire.content.clone(),
        style: wire.style.clone(),
        semantics: wire.semantics.clone(),
        children: wire
            .children
            .iter()
            .map(|&child| rebuild(nodes, child as usize))
            .collect(),
    }
}

#[cfg(test)]
mod tests;
