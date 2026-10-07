//! The green tree: immutable, position-independent syntax shared by `Arc`.
//!
//! This is the representation that makes incremental reparsing cheap. An
//! element records how many bytes it covers (its *width*) but never where it
//! sits, so a subtree that an edit did not touch is still valid at its new
//! offset and can be reused as-is — an `Arc` clone, not a copy. Absolute
//! positions are recovered on the way down by summing widths, which is what the
//! public [`Node`](crate::Node) view does.
//!
//! A node's kind and width live in its parent's child array rather than behind
//! the `Arc`, so each node is exactly one allocation (the `Arc<[Green]>` of its
//! children) and finding the child that covers an offset reads one contiguous
//! array without dereferencing any child.

use alloc::sync::Arc;
use alloc::vec::Vec;

/// One element of the green tree: an interior node (it has a child array) or a
/// leaf token (it has none).
#[derive(Clone)]
pub(crate) struct Green<K> {
    /// The node's children; `None` marks a leaf token. An empty node has an
    /// empty array, which keeps "node with no children" distinct from "token".
    children: Option<Arc<[Green<K>]>>,
    /// Bytes of source covered: the token's length, or the sum of the
    /// children's widths.
    pub(crate) width: u32,
    pub(crate) kind: K,
}

impl<K> Green<K> {
    /// A leaf token of `width` bytes.
    #[inline]
    pub(crate) const fn token(kind: K, width: u32) -> Self {
        Self {
            children: None,
            width,
            kind,
        }
    }

    /// An interior node over `children`, its width the sum of theirs.
    pub(crate) fn node(kind: K, children: Arc<[Green<K>]>) -> Self {
        let width = children
            .iter()
            .fold(0u32, |sum, child| sum.saturating_add(child.width));
        Self::with_width(kind, width, children)
    }

    /// An interior node whose width the caller already knows — the splice path,
    /// which derives it from the replaced node's width and the edit delta
    /// instead of re-summing a possibly wide child array.
    #[inline]
    pub(crate) const fn with_width(kind: K, width: u32, children: Arc<[Green<K>]>) -> Self {
        Self {
            children: Some(children),
            width,
            kind,
        }
    }

    /// Whether this element is an interior node (possibly an empty one).
    #[inline]
    pub(crate) const fn is_node(&self) -> bool {
        self.children.is_some()
    }

    /// The children of a node; empty for a token.
    #[inline]
    pub(crate) fn children(&self) -> &[Green<K>] {
        match &self.children {
            Some(children) => children,
            None => &[],
        }
    }
}

impl<K: Copy> Green<K> {
    /// Replaces the descendant reached by following child `indices` with
    /// `node`, adding `delta` to the width of every node on the way down.
    ///
    /// Each child array on the path is updated in place when this tree is its
    /// only owner, and copied first when a snapshot still shares it — so an
    /// edit to an unshared tree costs the length of the path, not the width of
    /// the arrays along it, and a shared snapshot is never disturbed. Returns
    /// `false`, having replaced nothing, if the path runs into a token.
    pub(crate) fn graft(
        &mut self,
        indices: impl Iterator<Item = usize>,
        node: Green<K>,
        delta: i64,
    ) -> bool {
        let mut element = self;
        for index in indices {
            let Some(children) = element.children.as_mut() else {
                return false;
            };
            element.width = shift(element.width, delta);
            let Some(child) = Arc::make_mut(children).get_mut(index) else {
                return false;
            };
            element = child;
        }
        *element = node;
        true
    }

    /// The first token of non-zero width in this subtree, as `(kind, width)`.
    ///
    /// Zero-width elements are skipped because they cover no byte: the token
    /// this returns is the one that covers the subtree's first byte.
    pub(crate) fn first_leaf(&self) -> Option<(K, u32)> {
        let mut element = self;
        loop {
            if !element.is_node() {
                return (element.width > 0).then_some((element.kind, element.width));
            }
            element = element.children().iter().find(|child| child.width > 0)?;
        }
    }

    /// The last token of non-zero width in this subtree, as `(kind, width)` —
    /// the token covering the subtree's last byte.
    pub(crate) fn last_leaf(&self) -> Option<(K, u32)> {
        let mut element = self;
        loop {
            if !element.is_node() {
                return (element.width > 0).then_some((element.kind, element.width));
            }
            element = element
                .children()
                .iter()
                .rev()
                .find(|child| child.width > 0)?;
        }
    }
}

impl<K: PartialEq> Green<K> {
    /// Structural equality: same kinds, widths, and shape, all the way down.
    ///
    /// Iterative, so it is safe on trees of any depth, and it skips any pair of
    /// subtrees that are the same allocation — after an incremental edit most of
    /// two trees being compared typically are.
    pub(crate) fn same(&self, other: &Self) -> bool {
        if !shallow_eq(self, other) {
            return false;
        }
        let mut pending: Vec<SiblingPair<'_, K>> = Vec::new();
        pending.push((self.children(), other.children()));
        while let Some((left, right)) = pending.pop() {
            if left.len() != right.len() {
                return false;
            }
            for (a, b) in left.iter().zip(right) {
                if !shallow_eq(a, b) {
                    return false;
                }
                if let (Some(x), Some(y)) = (&a.children, &b.children) {
                    if !Arc::ptr_eq(x, y) {
                        pending.push((x, y));
                    }
                }
            }
        }
        true
    }
}

/// `width + delta`, clamped to the representable range. An edit never moves a
/// width out of range; the clamp only keeps the arithmetic total.
#[inline]
fn shift(width: u32, delta: i64) -> u32 {
    let shifted = i64::from(width) + delta;
    u32::try_from(shifted.max(0)).unwrap_or(u32::MAX)
}

/// Two child arrays still to be compared by [`Green::same`].
type SiblingPair<'a, K> = (&'a [Green<K>], &'a [Green<K>]);

/// Compares one element without looking at its children.
#[inline]
fn shallow_eq<K: PartialEq>(a: &Green<K>, b: &Green<K>) -> bool {
    a.kind == b.kind && a.width == b.width && a.is_node() == b.is_node()
}

impl<K> Drop for Green<K> {
    /// Frees a subtree without recursion.
    ///
    /// The default drop glue recurses once per level of nesting, which would
    /// overflow the stack on a pathologically deep tree. Instead, every child
    /// array this element uniquely owns is detached onto a heap worklist and
    /// released there. Arrays still shared with another tree — after an edit,
    /// nearly all of them — are released with a single reference-count
    /// decrement and never walked.
    fn drop(&mut self) {
        let Some(mut children) = self.children.take() else {
            return;
        };
        let Some(slots) = Arc::get_mut(&mut children) else {
            return; // shared: dropping our reference only decrements the count
        };
        if slots.iter().all(|slot| slot.children.is_none()) {
            return; // tokens only: the default glue does not recurse
        }
        let mut worklist: Vec<Arc<[Green<K>]>> = Vec::new();
        detach(slots, &mut worklist);
        drop(children);
        while let Some(mut array) = worklist.pop() {
            if let Some(slots) = Arc::get_mut(&mut array) {
                detach(slots, &mut worklist);
            }
        }
    }
}

/// Moves every child array out of `slots` onto `worklist`, so that the arrays
/// holding them drop without recursing.
fn detach<K>(slots: &mut [Green<K>], worklist: &mut Vec<Arc<[Green<K>]>>) {
    for slot in slots {
        if let Some(children) = slot.children.take() {
            worklist.push(children);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn node(kind: u8, children: Vec<Green<u8>>) -> Green<u8> {
        Green::node(kind, children.into())
    }

    #[test]
    fn test_node_width_sums_children() {
        let n = node(0, vec![Green::token(1, 3), Green::token(2, 4)]);
        assert_eq!(n.width, 7);
        assert!(n.is_node());
        assert_eq!(n.children().len(), 2);
    }

    #[test]
    fn test_empty_node_is_distinct_from_token() {
        let empty = node(0, vec![]);
        assert!(empty.is_node());
        assert_eq!(empty.width, 0);
        assert!(!Green::token(0u8, 0).is_node());
    }

    #[test]
    fn test_first_and_last_leaf_skip_zero_width_elements() {
        let n = node(
            0,
            vec![
                node(1, vec![]),
                Green::token(2, 0),
                node(3, vec![Green::token(4, 2), Green::token(5, 1)]),
                Green::token(6, 0),
            ],
        );
        assert_eq!(n.first_leaf(), Some((4, 2)));
        assert_eq!(n.last_leaf(), Some((5, 1)));
        assert_eq!(node(0, vec![Green::token(1, 0)]).first_leaf(), None);
        assert_eq!(Green::token(9u8, 0).last_leaf(), None);
    }

    #[test]
    fn test_same_compares_structure_and_short_circuits_shared_arrays() {
        let shared: Arc<[Green<u8>]> = vec![Green::token(1, 1)].into();
        let a = node(0, vec![Green::with_width(2, 1, Arc::clone(&shared))]);
        let b = node(0, vec![Green::with_width(2, 1, shared)]);
        assert!(a.same(&b));

        let c = node(0, vec![node(2, vec![Green::token(1, 1)])]);
        assert!(a.same(&c));

        let different_kind = node(0, vec![node(2, vec![Green::token(7, 1)])]);
        assert!(!a.same(&different_kind));

        let token_vs_node = node(0, vec![Green::token(2, 1)]);
        assert!(!a.same(&token_vs_node));
    }

    #[test]
    fn test_graft_updates_unshared_arrays_in_place() {
        let mut tree = node(
            0,
            vec![Green::token(1, 1), node(2, vec![Green::token(3, 2)])],
        );
        let before = tree.children().as_ptr();
        assert!(tree.graft([1, 0].into_iter(), Green::token(4, 5), 3));
        assert_eq!(tree.children().as_ptr(), before); // same allocation
        assert_eq!(tree.width, 6);
        assert_eq!(tree.children()[1].width, 5);
        assert_eq!(tree.children()[1].children()[0].kind, 4);
    }

    #[test]
    fn test_graft_copies_shared_arrays() {
        let mut tree = node(
            0,
            vec![Green::token(1, 1), node(2, vec![Green::token(3, 2)])],
        );
        let snapshot = tree.clone();
        assert!(tree.graft([1, 0].into_iter(), Green::token(4, 1), -1));
        assert_eq!(tree.width, 2);
        // The snapshot still sees the old tree.
        assert_eq!(snapshot.width, 3);
        assert_eq!(snapshot.children()[1].children()[0].kind, 3);
        // The untouched sibling is shared, not copied.
        assert!(tree.children()[0].same(&snapshot.children()[0]));
    }

    #[test]
    fn test_graft_refuses_a_path_through_a_token() {
        let mut tree = node(0, vec![Green::token(1, 1)]);
        assert!(!tree.graft([0, 0].into_iter(), Green::token(2, 1), 0));
        assert!(!tree.graft([5].into_iter(), Green::token(2, 1), 0));
        assert_eq!(tree.children()[0].kind, 1);
    }

    #[test]
    fn test_deep_tree_drops_without_stack_overflow() {
        let mut tree = node(0, vec![Green::token(1, 1)]);
        for _ in 0..200_000 {
            tree = node(0, vec![tree]);
        }
        drop(tree);
    }

    #[test]
    fn test_drop_leaves_shared_subtrees_intact() {
        let inner = node(1, vec![Green::token(2, 1)]);
        let keep = inner.clone();
        drop(node(0, vec![inner]));
        assert_eq!(keep.children().len(), 1);
        assert_eq!(keep.width, 1);
    }
}
