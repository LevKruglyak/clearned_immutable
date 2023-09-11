mod layer;

// use crate::BaseComponent;
// use crate::InternalComponent;
// use crate::Key;
// use crate::NodeLayer;
// use crate::TopComponent;
// use crate::Value;
// use layer::MemoryBTreeLayer;
// use std::borrow::Borrow;
// use std::collections::BTreeMap;
use crate::kv::StaticBounded;
use crate::{common::entry::Entry, component::*};
use layer::*;
use std::ops::{Bound, RangeBounds};

// -------------------------------------------------------
//                  Internal Component
// -------------------------------------------------------

pub struct BTreeInternalComponent<K: Clone, B: NodeLayer<K>, const FANOUT: usize> {
    inner: MemoryBTreeLayer<K, B::Address, FANOUT>,
}

impl<K, B: NodeLayer<K>, const FANOUT: usize> NodeLayer<K> for BTreeInternalComponent<K, B, FANOUT>
where
    K: StaticBounded,
{
    type Node = <MemoryBTreeLayer<K, B::Address, FANOUT> as NodeLayer<K>>::Node;
    type Address = <MemoryBTreeLayer<K, B::Address, FANOUT> as NodeLayer<K>>::Address;

    fn deref(&self, ptr: Self::Address) -> &Self::Node {
        self.inner.deref(ptr)
    }

    fn deref_mut(&mut self, ptr: Self::Address) -> &mut Self::Node {
        self.inner.deref_mut(ptr)
    }

    type Iter<'n> = <MemoryBTreeLayer<K, B::Address, FANOUT> as NodeLayer<K>>::Iter<'n>;

    fn range<'n>(
        &'n self,
        start: Bound<Self::Address>,
        end: Bound<Self::Address>,
    ) -> Self::Iter<'n> {
        self.inner.range(start, end)
    }

    fn full_range<'n>(&'n self) -> Self::Iter<'n> {
        self.inner.full_range()
    }
}

impl<K, B: NodeLayer<K>, const FANOUT: usize> InternalComponent<K, B>
    for BTreeInternalComponent<K, B, FANOUT>
where
    K: StaticBounded,
{
    fn search(&self, _: &B, ptr: Self::Address, key: &K) -> B::Address {
        let node = unsafe { ptr.as_ref() };

        node.inner.search_lub(key).clone()
    }

    fn insert<'n>(
        &'n mut self,
        base: &B,
        ptr: Self::Address,
        prop: PropogateInsert<K, B>,
    ) -> Option<PropogateInsert<K, Self>> {
        match prop {
            PropogateInsert::Single(key, address) => self
                .inner
                .insert(key, address, ptr)
                .map(|(key, address)| PropogateInsert::Single(key, address)),
            PropogateInsert::Rebuild => {
                self.inner.fill(base.full_range());

                Some(PropogateInsert::Rebuild)
            }
        }
    }

    fn len(&self) -> usize {
        self.inner.nodes.len()
    }

    fn memory_size(&self) -> usize {
        self.inner.alloc.allocated_bytes_including_metadata()
    }
}

impl<K, B: NodeLayer<K>, const FANOUT: usize> InternalComponentInMemoryBuild<K, B>
    for BTreeInternalComponent<K, B, FANOUT>
where
    K: StaticBounded,
{
    fn build(base: &B) -> Self {
        let mut result = MemoryBTreeLayer::empty();
        result.fill(base.full_range());

        Self { inner: result }
    }
}

// -------------------------------------------------------
//                  Base Component
// -------------------------------------------------------

pub struct BTreeBaseComponent<K, V, const FANOUT: usize> {
    inner: MemoryBTreeLayer<K, V, FANOUT>,
}

impl<K, V: Clone, const FANOUT: usize> NodeLayer<K> for BTreeBaseComponent<K, V, FANOUT>
where
    K: StaticBounded,
    V: 'static,
{
    type Node = <MemoryBTreeLayer<K, V, FANOUT> as NodeLayer<K>>::Node;
    type Address = <MemoryBTreeLayer<K, V, FANOUT> as NodeLayer<K>>::Address;

    fn deref(&self, ptr: Self::Address) -> &Self::Node {
        self.inner.deref(ptr)
    }

    fn deref_mut(&mut self, ptr: Self::Address) -> &mut Self::Node {
        self.inner.deref_mut(ptr)
    }

    type Iter<'n> = <MemoryBTreeLayer<K, V, FANOUT> as NodeLayer<K>>::Iter<'n>;

    fn range<'n>(
        &'n self,
        start: Bound<Self::Address>,
        end: Bound<Self::Address>,
    ) -> Self::Iter<'n> {
        self.inner.range(start, end)
    }

    fn full_range<'n>(&'n self) -> Self::Iter<'n> {
        self.inner.full_range()
    }
}

impl<K, V: Clone, const FANOUT: usize> BaseComponent<K, V, Self>
    for BTreeBaseComponent<K, V, FANOUT>
where
    K: StaticBounded,
    V: 'static,
{
    fn insert<'n>(
        &'n mut self,
        ptr: Self::Address,
        key: K,
        value: V,
    ) -> Option<PropogateInsert<K, Self>> {
        if let Some((key, address)) = self.inner.insert(key, value, ptr) {
            Some(PropogateInsert::Single(key, address))
        } else {
            None
        }
    }

    fn search(&self, ptr: Self::Address, key: &K) -> Option<&V> {
        let node = unsafe { ptr.as_ref() };
        node.inner.search_exact(key)
    }

    fn len(&self) -> usize {
        self.inner.nodes.len()
    }

    fn memory_size(&self) -> usize {
        self.inner.alloc.allocated_bytes_including_metadata()
    }
}

impl<K, V, const FANOUT: usize> BaseComponentInMemoryBuild<K, V>
    for BTreeBaseComponent<K, V, FANOUT>
where
    K: StaticBounded,
{
    fn empty() -> Self {
        let mut result = MemoryBTreeLayer::empty();
        result.add_node(MemoryBTreeNode::empty());

        Self { inner: result }
    }

    fn build(iter: impl Iterator<Item = Entry<K, V>>) -> Self {
        let mut result = MemoryBTreeLayer::empty();
        result.fill(iter);

        Self { inner: result }
    }
}