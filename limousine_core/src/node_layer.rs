// Type dependence hierarchy

use std::ops::Bound;

use crate::iter::Iter;
use crate::traits::*;

/// A `LinkedNode` is a model in a `NodeLayer`, representing a set of entries above a
/// lower bound. In addition to storing a pointer to its neighbor, it also stores a
/// pointer to its parent, which is in a different layer.
///
/// In order to avoid circular type dependencies during composition, it is generic over
/// its own address type, as well as its parent type. (SA, PA respectively)
pub trait Node<K, SA>: 'static + KeyBounded<K>
where
    SA: Address,
{
    // Address to the next node in the current component
    fn next(&self) -> Option<SA>;

    fn previous(&self) -> Option<SA>;

    // // Address to the parent node in the above component
    // fn parent(&self) -> Option<PA>;
    //
    // fn set_parent(&mut self, parent: PA);
}

/// A `NodeLayer` is has the interface of a linked list of key-bounded nodes which implement the
/// `Model` trait. It's assumed that a `NodeLayer` is always non-empty, and thus should always have
/// a `first` and `last` node.
pub trait NodeLayer<K, SA, PA>: 'static + Sized
where
    K: Copy,
    SA: Address,
    PA: Address,
{
    /// Node type stored in the layer. Each node roughly represents a model in the hybrid index
    /// which indexes some finite/lower-bounded collection of `Keyed` elements.
    type Node: Node<K, SA>;

    /// Immutable address dereference which returns a reference to a node.
    fn node_ref(&self, ptr: SA) -> impl AsRef<Self::Node>;

    /// Mutable address dereference which returns a reference to a node.
    // fn deref_mut(&mut self, ptr: SA) -> &mut Self::Node;

    fn parent(&self, ptr: SA) -> Option<PA>;

    fn set_parent(&mut self, ptr: SA, parent: PA);

    unsafe fn set_parent_unsafe(&self, ptr: SA, parent: PA);

    /// Get the lower bound of a node. This could be overridden by some layers which might have a
    /// more optimal way of mapping the address to the lower bound.
    fn lower_bound(&self, ptr: SA) -> K {
        *self.node_ref(ptr).as_ref().lower_bound()
    }

    fn next(&self, ptr: SA) -> Option<SA> {
        self.node_ref(ptr).as_ref().next()
    }

    /// First node in the current node layer
    fn first(&self) -> SA;

    /// Last node in the current node layer
    fn last(&self) -> SA;

    /// An immutable iterator over the layer, returning (Key, Address) pairs
    fn range(&self, start: Bound<SA>, end: Bound<SA>) -> Iter<'_, K, Self, SA, PA> {
        Iter::range(self, start, end)
    }
}

macro_rules! impl_node_layer {
    ($SA:ty, $PA:ty) => {
        fn node_ref(&self, ptr: $SA) -> impl AsRef<Self::Node> {
            self.inner.node_ref(ptr)
        }

        fn parent(&self, ptr: $SA) -> Option<$PA> {
            self.inner.parent(ptr)
        }

        fn set_parent(&mut self, ptr: $SA, parent: $PA) {
            self.inner.set_parent(ptr, parent)
        }

        unsafe fn set_parent_unsafe(&self, ptr: $SA, parent: $PA) {
            self.inner.set_parent_unsafe(ptr, parent)
        }

        fn lower_bound(&self, ptr: $SA) -> K {
            self.inner.lower_bound(ptr)
        }

        fn next(&self, ptr: $SA) -> Option<$SA> {
            self.inner.next(ptr)
        }

        fn first(&self) -> $SA {
            self.inner.first()
        }

        fn last(&self) -> $SA {
            self.inner.last()
        }
    };
}

pub(crate) use impl_node_layer;
