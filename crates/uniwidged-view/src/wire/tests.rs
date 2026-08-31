//! The arena arrives from a sandbox, so most of these are about what is
//! **refused**. A validator tested only for what it accepts is not tested.

use super::*;
use crate::Ratio;
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
        .child(sparkline([0.1, 0.5, 0.25, 1.0]))
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
fn a_series_past_the_cap_is_refused_rather_than_drawn() {
    // `build::sparkline` keeps the recent end, so a Rust widget cannot send
    // this. A hand-built arena — a widget in another language, or a hostile
    // one — can, and it is a request for the host to allocate and then to
    // draw 100 000 line segments inside somebody's frame budget.
    let mut tree = pair();
    tree.nodes[1].content = Content::Sparkline(Series(vec![Ratio::new(0.5); 100_000]));
    assert_eq!(
        tree.into_view(),
        Err(TreeError::SeriesTooLong {
            node: 1,
            points: 100_000
        })
    );
    // And exactly at the cap is fine — the limit is a limit, not a limit
    // minus one.
    let mut tree = pair();
    tree.nodes[1].content = Content::Sparkline(Series(vec![Ratio::new(0.5); Series::MAX_POINTS]));
    assert!(tree.into_view().is_ok());
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
        TreeError::SeriesTooLong {
            node: 7,
            points: 100_000,
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

// ---------------------------------------------------------------------------
// Random arenas
// ---------------------------------------------------------------------------
//
// The tests above are the cases somebody thought of. This is the part that
// covers the ones nobody did: thousands of arenas built from random indices,
// checked against the properties the decoder promises rather than against
// expected values.
//
// It is deliberately not a fuzzing framework. The generator is twenty lines and
// the seed is fixed, so a failure is reproducible from the test name alone and
// the suite costs milliseconds — a fuzzer that has to be run separately is a
// fuzzer nobody runs before merging.

/// xorshift64*. Deterministic, seeded per test, and good enough to shake out
/// index arithmetic — which is all this needs, since the *interesting* inputs
/// are near boundaries and those are generated deliberately below.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next() % bound }
    }
}

/// An arena of `count` nodes whose child indices are drawn from a range that
/// straddles every boundary the decoder checks: before the parent, after it,
/// exactly at it, and past the end of the arena.
fn arbitrary_arena(rng: &mut Rng, count: usize) -> Tree {
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        let children = (0..rng.below(4))
            .map(|_| rng.below(count as u64 + 2) as u32)
            .collect();
        nodes.push(WireNode {
            children,
            ..WireNode::default()
        });
    }
    Tree { nodes }
}

#[test]
fn no_arena_can_make_the_decoder_panic_or_hang() {
    // The property that matters most, because the decoder runs on hostile
    // input by definition: whatever a widget sends, the host answers rather
    // than dying. A recursive decoder discovers a cycle by running out of
    // stack, and on a desktop process that takes the desktop with it.
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut accepted = 0;
    let mut refused = 0;
    for _ in 0..4000 {
        let count = 1 + rng.below(12) as usize;
        match arbitrary_arena(&mut rng, count).into_view() {
            Ok(_) => accepted += 1,
            Err(_) => refused += 1,
        }
    }
    // Both outcomes have to occur, or the generator is producing junk the
    // decoder rejects out of hand and this test is measuring nothing.
    assert!(accepted > 0, "no random arena was ever accepted");
    assert!(refused > 0, "no random arena was ever refused");
}

#[test]
fn every_accepted_arena_is_a_tree_that_round_trips() {
    // What "accepted" has to mean: exactly the nodes that were reachable, each
    // once, in a shape that flattens back to an equivalent arena. A decoder
    // that duplicated a subtree or dropped one would pass every hand-written
    // test above and fail here.
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    let mut checked = 0;
    for _ in 0..4000 {
        let count = 1 + rng.below(12) as usize;
        let arena = arbitrary_arena(&mut rng, count);
        let Ok(view) = arena.clone().into_view() else {
            continue;
        };
        checked += 1;

        // Every node in the arena is in the tree, once.
        fn count_nodes(node: &Node) -> usize {
            1 + node.children.iter().map(count_nodes).sum::<usize>()
        }
        assert_eq!(
            count_nodes(&view.root),
            arena.nodes.len(),
            "an accepted arena has no unreachable nodes and no duplicates"
        );

        // And flattening it gives an arena the decoder accepts again, with the
        // same shape.
        let again = view.flatten();
        assert_eq!(again.nodes.len(), arena.nodes.len());
        let reparsed = again
            .clone()
            .into_view()
            .expect("a flattened tree is always acceptable");
        assert_eq!(reparsed.flatten(), again, "the round trip is stable");
    }
    assert!(checked > 50, "only {checked} arenas were accepted");
}

#[test]
fn every_refusal_names_a_node_that_exists() {
    // An error is a diagnostic somebody has to act on, so the index in it has
    // to mean something. An off-by-one here points a widget author at the
    // wrong node, which is worse than pointing at none.
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..4000 {
        let count = 1 + rng.below(12) as usize;
        let arena = arbitrary_arena(&mut rng, count);
        let size = arena.nodes.len();
        let Err(error) = arena.into_view() else {
            continue;
        };
        let node = match error {
            TreeError::TooLarge { .. } => continue,
            TreeError::TextTooLong { node, .. }
            | TreeError::SeriesTooLong { node, .. }
            | TreeError::ChildOutOfRange { node, .. }
            | TreeError::ChildNotAfterParent { node, .. }
            | TreeError::SharedChild { node, .. }
            | TreeError::Unreachable { node } => node,
        };
        assert!(node < size, "{error:?} names node {node} of {size}");
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn a_valid_arena_is_never_refused() {
    // The other direction, and the one a too-strict checker fails: arenas
    // built the way `flatten` builds them — every child after its parent, each
    // node attached exactly once — must always be accepted. A widget whose
    // legitimate view is rejected has no way to find out why.
    let mut rng = Rng(0x5DEE_CE66_D000_0005);
    for _ in 0..2000 {
        let count = 1 + rng.below(20) as usize;
        let mut nodes: Vec<WireNode> = (0..count).map(|_| WireNode::default()).collect();
        // Attach each node to a uniformly chosen earlier one, which produces
        // every shape from a single chain to a flat fan.
        for child in 1..count {
            let parent = rng.below(child as u64) as usize;
            nodes[parent].children.push(child as u32);
        }
        let arena = Tree { nodes };
        assert!(
            arena.clone().into_view().is_ok(),
            "a well-formed arena was refused: {arena:?}"
        );
    }
}
