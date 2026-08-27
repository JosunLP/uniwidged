//! The arena arrives from a sandbox, so most of these are about what is
//! **refused**. A validator tested only for what it accepts is not tested.

use super::*;
use crate::build::*;
use uniwidged_core::NodeId;

fn sample() -> View {
    panel(NodeId::named("root"))
        .child(text("Weather").role(Role::Title))
        .child(divider())
        .child(
            row()
                .child(text("18°").role(Role::Display))
                .child(spacer())
                .child(chip("now").on_click(NodeId::named("go"))),
        )
        .child(progress(0.4))
        .view()
}

/// A minimal valid arena: a root with one child.
fn pair() -> Tree {
    Tree {
        nodes: vec![
            WireNode {
                id: 1,
                children: vec![1],
                ..Default::default()
            },
            WireNode {
                id: 2,
                ..Default::default()
            },
        ],
    }
}

#[test]
fn a_tree_survives_the_round_trip_exactly() {
    // Everything downstream assumes the host reconstructs what the guest
    // built. If this drifts, a widget renders as something its author never
    // described.
    let original = sample();
    let rebuilt = original.flatten().into_view().expect("valid");
    assert_eq!(rebuilt, original);
}

#[test]
fn flattening_always_puts_children_after_their_parent() {
    // The invariant the whole validation rests on, asserted at its source: a
    // tree this produced must always pass `into_view`.
    let tree = sample().flatten();
    for (index, node) in tree.nodes.iter().enumerate() {
        for &child in &node.children {
            assert!(
                child as usize > index,
                "node {index} names child {child}, which is not after it"
            );
        }
    }
    assert!(tree.into_view().is_ok());
}

#[test]
fn an_empty_arena_is_a_widget_with_nothing_to_show_yet() {
    // Normal before the first fetch returns. Treating it as an error would
    // make every widget's first frame a failure.
    assert_eq!(Tree::default().into_view(), Ok(View::default()));
}

#[test]
fn a_cycle_cannot_even_be_expressed() {
    // The point of requiring children to come later in the arena. There is no
    // visited-set to get wrong and no depth at which the check gives up — a
    // cycle is simply not a valid arena.
    let tree = Tree {
        nodes: vec![
            WireNode {
                children: vec![1],
                ..Default::default()
            },
            WireNode {
                children: vec![0],
                ..Default::default()
            },
        ],
    };
    assert_eq!(
        tree.into_view(),
        Err(TreeError::ChildNotAfterParent { node: 1, child: 0 })
    );
}

#[test]
fn a_node_naming_itself_is_refused() {
    let tree = Tree {
        nodes: vec![WireNode {
            children: vec![0],
            ..Default::default()
        }],
    };
    assert_eq!(
        tree.into_view(),
        Err(TreeError::ChildNotAfterParent { node: 0, child: 0 })
    );
}

#[test]
fn a_child_index_past_the_end_is_refused() {
    let mut tree = pair();
    tree.nodes[0].children = vec![7];
    assert_eq!(
        tree.into_view(),
        Err(TreeError::ChildOutOfRange { node: 0, child: 7 })
    );
}

#[test]
fn a_node_with_two_parents_is_refused() {
    // A view is a tree. Rebuilding a shared node would duplicate the subtree
    // and its ids with it, so a click would report an id that exists twice.
    let tree = Tree {
        nodes: vec![
            WireNode {
                children: vec![1, 2],
                ..Default::default()
            },
            WireNode {
                children: vec![3],
                ..Default::default()
            },
            WireNode {
                children: vec![3],
                ..Default::default()
            },
            WireNode::default(),
        ],
    };
    assert_eq!(
        tree.into_view(),
        Err(TreeError::SharedChild { node: 2, child: 3 })
    );
}

#[test]
fn a_node_attached_to_nothing_is_refused() {
    // Dropping it silently would hide a bug in the widget that then looks like
    // a rendering fault in the framework.
    let mut tree = pair();
    tree.nodes.push(WireNode::default());
    assert_eq!(tree.into_view(), Err(TreeError::Unreachable { node: 2 }));
}

#[test]
fn an_oversized_arena_is_refused_before_anything_is_allocated() {
    // A widget has its own memory cap, but nothing stops it asking the *host*
    // to allocate on its behalf.
    let tree = Tree {
        nodes: vec![WireNode::default(); MAX_NODES + 1],
    };
    assert_eq!(
        tree.into_view(),
        Err(TreeError::TooLarge {
            nodes: MAX_NODES + 1
        })
    );
}

#[test]
fn an_arena_at_the_limit_is_still_accepted() {
    // Off-by-one in the other direction: a widget that happens to hit the cap
    // exactly must not be refused.
    let mut nodes = vec![WireNode::default(); MAX_NODES];
    nodes[0].children = (1..MAX_NODES as u32).collect();
    assert!(Tree { nodes }.into_view().is_ok());
}

#[test]
fn an_enormous_string_is_refused() {
    let mut tree = pair();
    tree.nodes[1].content = Content::Text("x".repeat(MAX_TEXT + 1));
    assert_eq!(
        tree.into_view(),
        Err(TreeError::TextTooLong {
            node: 1,
            length: MAX_TEXT + 1
        })
    );
}

#[test]
fn an_enormous_accessibility_label_is_refused_too() {
    // The label is a second string on every node, and a validator that
    // checked only the visible one would leave the obvious way round it.
    let mut tree = pair();
    tree.nodes[1].semantics.label = Some("x".repeat(MAX_TEXT + 1));
    assert!(matches!(
        tree.into_view(),
        Err(TreeError::TextTooLong { node: 1, .. })
    ));
}

#[test]
fn a_deep_chain_is_bounded_by_the_node_count_alone() {
    // Depth needs no separate limit: children come later in the arena, so
    // depth cannot exceed the node count, and the node count is capped. This
    // is the deepest tree that can be expressed at all.
    let nodes: Vec<WireNode> = (0..MAX_NODES)
        .map(|i| WireNode {
            children: if i + 1 < MAX_NODES {
                vec![i as u32 + 1]
            } else {
                Vec::new()
            },
            ..Default::default()
        })
        .collect();
    let view = Tree { nodes }.into_view().expect("valid, if deep");
    let mut depth = 0;
    let mut node = &view.root;
    while let Some(child) = node.children.first() {
        depth += 1;
        node = child;
    }
    assert_eq!(depth, MAX_NODES - 1);
}

#[test]
fn every_refusal_says_which_node_and_why() {
    // "The widget sent a bad tree" is not something anybody can act on.
    let messages = [
        TreeError::TooLarge { nodes: 2000 }.to_string(),
        TreeError::TextTooLong {
            node: 3,
            length: 99999,
        }
        .to_string(),
        TreeError::ChildOutOfRange { node: 1, child: 9 }.to_string(),
        TreeError::ChildNotAfterParent { node: 4, child: 2 }.to_string(),
        TreeError::SharedChild { node: 2, child: 5 }.to_string(),
        TreeError::Unreachable { node: 6 }.to_string(),
    ];
    for message in messages {
        assert!(
            message.chars().any(|c| c.is_ascii_digit()),
            "{message:?} names no node"
        );
        assert!(message.len() > 25, "{message:?} explains nothing");
    }
}
