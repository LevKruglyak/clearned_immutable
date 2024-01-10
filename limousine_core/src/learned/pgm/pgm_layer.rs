use super::{pgm_inner::PGMInner, pgm_model::LinearModel};
use crate::{
    common::{
        bounded::{KeyBounded, StaticBounded},
        linked_list::{LinkedList, LinkedNode},
        macros::impl_node_layer,
    },
    component::{Address, Key, Model, NodeLayer, Value},
    learned::generic::Segmentation,
    Entry,
};
use generational_arena::{Arena, Index};
use std::{borrow::Borrow, ops::Bound};

/// Shorthands for the types containing core "interesting data"
type PGMNode<K, V, const EPSILON: usize> = LinkedNode<PGMInner<K, V, EPSILON>, Index>;

/// A PGMLayer with internals optimized for usage as an in-memory structure
pub struct MemoryPGMLayer<K: Key, V: Value, const EPSILON: usize, PA> {
    pub inner: LinkedList<PGMInner<K, V, EPSILON>, PA>,
}

/// Implement the addressing and mutability constraints required by a NodeLayer
/// NOTE: Since we use LinkedList internally, this is easy
impl<K: Key, V: Value, const EPSILON: usize, PA: Address> NodeLayer<K, Index, PA>
    for MemoryPGMLayer<K, V, EPSILON, PA>
{
    type Node = <LinkedList<PGMInner<K, V, EPSILON>, PA> as NodeLayer<K, Index, PA>>::Node;

    impl_node_layer!(Index);
}

impl<K: Key, V: Value, const EPSILON: usize, PA: Address> MemoryPGMLayer<K, V, EPSILON, PA> {
    /// Make an empty layer
    /// NOTE: This actually means a layer with a sentinel at the end, because _all_ layers should have
    /// sentinels at the end
    pub fn new() -> Self {
        Self {
            inner: LinkedList::new(PGMInner::sentinel()),
        }
    }

    /// Wipe this layer and rebuild it with the data in iter
    pub fn fill(&mut self, iter: impl Iterator<Item = Entry<K, V>>) {
        self.inner.clear(PGMInner::sentinel());
        let blueprint = LinearModel::<K, EPSILON>::make_segmentation(iter);
        for (model, entries) in blueprint {
            let mut innards = PGMInner::from_model_n_vec(model, entries);
            let new_ptr = self.inner.append_before_sentinel(innards);
        }
    }

    /// Given the layer that is supposed to sit under this layer, fill this layer making sure
    /// to update the parents of the lower layer as needed
    pub fn fill_from_beneath<B>(&mut self, base: &mut B)
    where
        V: Address,
        B: NodeLayer<K, V, Index>,
    {
        // Just make two passes through the data for simplicity
        // First pass: build the layer
        let test = base.mut_range(Bound::Unbounded, Bound::Unbounded);
        let vec: Vec<Entry<K, V>> = test.map(|x| Entry::new(x.key(), x.address())).collect();
        self.fill(vec.into_iter());
        // Second pass: set parent pointer of base layer
        let mut parent_ptr = self.inner.first();
        let mut next_parent_ptr = self.inner.deref(parent_ptr).next();
        for view in base.mut_range(Bound::Unbounded, Bound::Unbounded) {
            if next_parent_ptr.is_none() || &view.key() < self.deref(next_parent_ptr.unwrap()).lower_bound() {
                view.set_parent(parent_ptr);
            } else {
                parent_ptr = next_parent_ptr.unwrap();
                next_parent_ptr = self.inner.deref(parent_ptr).next();
                view.set_parent(parent_ptr);
            }
        }
    }

    /// Assume that base B has had some potentially large continguous change.
    /// We will handle this by simply replacing all nodes in this layer who have a child participating in the change.
    /// `poison_head` is the address of the first node that needs to be replaced in this layer
    /// `poison_tail` is the address of the last node (INCLUSIVE) that needs to be replaced in this layer
    /// `data_head` is the address of the first piece of data in the new node filling in the gap
    /// `data_tail` is the address of the last piece of data in the new node filling in the gap
    pub fn replace<B>(&mut self, base: &mut B, poison_head: Index, poison_tail: Index, data_head: V, data_tail: V)
    where
        V: Address,
        B: NodeLayer<K, V, Index>,
    {
        // Inefficient but correct
        // First let's construct a vector of all the things we're adding
        let mut bot_ptr = Some(data_head.clone());
        let mut entries: Vec<Entry<K, V>> = vec![];
        while bot_ptr.is_some() {
            let node = base.deref(bot_ptr.unwrap());
            entries.push(Entry::new(node.lower_bound().clone(), bot_ptr.unwrap()));
            if bot_ptr == Some(data_tail.clone()) {
                // Exit early
                break;
            }
            bot_ptr = node.next();
        }
        println!("Replace is seeing {} entries", entries.len());
        // Now we can train new nodes over this added data
        let blueprint = LinearModel::<K, EPSILON>::make_segmentation(entries.into_iter());
        let new_innards: Vec<PGMInner<K, V, EPSILON>> = blueprint
            .into_iter()
            .map(|(model, entries)| PGMInner::from_model_n_vec(model, entries))
            .collect();
        // Replace all the nodes in the parent layer
        let (new_parent_head, new_parent_tail) =
            self.inner
                .replace(poison_head, poison_tail, new_innards.clone().into_iter());
        // Finally we need to set the parent pointers in the bottom layer
        let mut kid = data_head;
        let mut kite = new_parent_head;
        loop {
            let next_kite = self.deref(kite).next();
            let kid_key = base.deref(kid).lower_bound();
            let is_match = kite == new_parent_tail // Kids guaranteed to fall into new range
                || match next_kite {
                    Some(next_ix) => {
                        let next_bound = self.deref(next_ix).lower_bound();
                        kid_key < next_bound
                    },
                    None => true,
                };
            if !is_match {
                kite = next_kite.unwrap();
            }
            base.deref_mut(kid).set_parent(kite);
            if kid == data_tail {
                break;
            }
            kid = base.deref(kid).next().unwrap();
        }
    }
}

#[cfg(test)]
mod pgm_layer_tests {
    use super::*;
    use crate::learned::generic::LearnedModel;
    use kdam::{tqdm, Bar, BarExt};
    use rand::{distributions::Uniform, Rng};

    /// It's easier to write tests if we fix these
    const EPSILON: usize = 8;
    type KType = usize;
    type VType = usize;

    /// Helper function to generate random entries
    fn generate_random_entries(num_entries: usize, lb: usize, ub: usize) -> Vec<Entry<KType, VType>> {
        let range = Uniform::from(lb..ub);
        let mut random_values: Vec<usize> = rand::thread_rng().sample_iter(&range).take(num_entries).collect();
        random_values.sort();
        random_values.dedup();
        let entries: Vec<Entry<KType, VType>> = random_values
            .into_iter()
            .enumerate()
            .map(|(ix, key)| Entry::new(key, ix))
            .collect();
        entries
    }

    /// Helper function to make a simple layer
    fn make_simple_layer(num_elements: usize) -> MemoryPGMLayer<KType, VType, EPSILON, Index> {
        let entries = generate_random_entries(num_elements, KType::MIN, KType::MAX);
        let mut layer = MemoryPGMLayer::<KType, VType, EPSILON, Index>::new();
        layer.fill(entries.into_iter());
        layer
    }

    /// Helper function to make a base layer and a layer on top of it
    fn make_two_layers(
        num_elements: usize,
    ) -> (
        MemoryPGMLayer<KType, VType, EPSILON, Index>,
        MemoryPGMLayer<KType, Index, EPSILON, Index>,
    ) {
        let mut beneath = make_simple_layer(num_elements);
        let mut layer = MemoryPGMLayer::<KType, Index, EPSILON, Index>::new();
        layer.fill_from_beneath::<MemoryPGMLayer<KType, VType, EPSILON, Index>>(&mut beneath);
        (beneath, layer)
    }

    /// Helper function to generate a random replace
    /// NOTE: Has the side-effect of actually deleting + replacing stuff in the base layer
    /// NOTE: DOES NOT do anything to the top layer
    /// Returns: The poison head, poison tail, data_head, data_tail (confusing bc they all have the same type)
    fn generate_fake_replace(
        beneath: &mut MemoryPGMLayer<KType, VType, EPSILON, Index>,
        above: &MemoryPGMLayer<KType, Index, EPSILON, Index>,
    ) -> (Index, Index, Index, Index) {
        // Inefficient but comprehensible
        // First get the number of nodes in the beneath layer
        let mut bot_ptr = Some(beneath.inner.first());
        let mut num_bot_nodes: usize = 0;
        while bot_ptr.is_some() {
            let mem_node = beneath.deref(bot_ptr.unwrap());
            num_bot_nodes += 1;
            bot_ptr = mem_node.next();
        }

        // Then pick a random node to start replacing at, a random number of elements to replace, and a random number of new elements to train on
        let start_replace_ix: usize = rand::thread_rng().gen_range(0..(num_bot_nodes - 2)); // 2 arbitrary
        let mut num_replace: usize = rand::thread_rng().gen_range(2..(num_bot_nodes / 10)); // 10 is arbitrary
                                                                                            // TODO: Need -2 here because test doesn't construct sentinel. See linked_list note about standardizing api
        num_replace = num_replace.min(num_bot_nodes - start_replace_ix - 2);
        let num_new = rand::thread_rng().gen_range(100..1000); // _everything_ is arbitrary

        // Translate the first replacing node and last replace
        // NOTE: These exist in the _bottom_ layer
        let mut start_replace_address: Option<Index> = None;
        let mut end_replace_address: Option<Index> = None;
        let mut bot_ptr = Some(beneath.inner.first());
        let mut ix: usize = 0;
        while bot_ptr.is_some() {
            if ix == start_replace_ix {
                start_replace_address = bot_ptr;
            }
            if ix == start_replace_ix + num_replace - 1 {
                end_replace_address = bot_ptr;
            }
            let mem_node = beneath.deref(bot_ptr.unwrap());
            bot_ptr = mem_node.next();
            ix += 1;
        }
        assert!(start_replace_address.is_some());
        assert!(end_replace_address.is_some());
        let start_replace_address = start_replace_address.unwrap();
        let end_replace_address = end_replace_address.unwrap();

        // Get the poison bounds
        let start_replace_node = beneath.deref(start_replace_address);
        let end_replace_node = beneath.deref(end_replace_address);
        let poison_head = start_replace_node.parent().unwrap();
        let poison_tail = end_replace_node.parent().unwrap();

        // Generate linked-list of new entries
        let min_new = start_replace_node.lower_bound().clone();
        let max_new = end_replace_node.lower_bound();
        let new_entries = generate_random_entries(num_new, min_new, *max_new);
        let blueprint = LinearModel::<KType, EPSILON>::make_segmentation(new_entries.into_iter());
        let mut first_added: Option<Index> = None;
        let mut last_added: Option<Index> = None;
        let mut num_gen = 0;
        for (model, entries) in blueprint {
            let innards = PGMInner::from_model_n_vec(model, entries);
            let temp_node = PGMNode::init(innards, None, last_added, None);
            let new_address = beneath.inner.arena.insert(temp_node);
            if first_added.is_none() {
                first_added = Some(new_address);
            }
            if last_added.is_some() {
                let mut node = beneath.deref_mut(last_added.unwrap());

                node.set_next(Some(new_address));
            }
            last_added = Some(new_address);
            num_gen += 1;
        }
        let first_added = first_added.unwrap();
        let last_added = last_added.unwrap();

        // Find data_start
        let mut data_start_address = above.deref(poison_head).inner.data[0].value;
        if data_start_address == start_replace_address {
            // This is the edge case where the leftmost bound of our replace is the very first thing in our poison head
            // In this case we need to pass in the address into the new nodes.
            data_start_address = first_added;
        }

        // Fix the linked list
        let start_replace_node = beneath.deref(start_replace_address);
        let end_replace_node = beneath.deref(end_replace_address);
        let prev_addr = start_replace_node.previous().clone();
        let next_addr = end_replace_node.next();
        if start_replace_address == beneath.inner.first() {
            beneath.inner.first = first_added;
        } else {
            let prev_node = beneath.deref_mut(prev_addr.unwrap());
            prev_node.set_next(Some(first_added));
        }
        let last_new_node = beneath.deref_mut(last_added);
        last_new_node.set_next(next_addr);

        // Calculate debug value
        let mut start_count = 0;
        let mut test_ptr = data_start_address;
        while test_ptr != first_added {
            let node = beneath.deref(test_ptr);
            test_ptr = node.next().unwrap();
            start_count += 1;
        }

        // Find data_end
        let mut data_end_address = end_replace_address;
        let mut next_data_end_address = beneath.deref(data_end_address).next();
        let mut end_count = 0;
        while next_data_end_address.is_some() {
            let node = beneath.deref(next_data_end_address.unwrap());
            if node.parent() != Some(poison_tail) {
                break;
            }
            end_count += 1;
            data_end_address = next_data_end_address.unwrap();
            next_data_end_address = beneath.deref(data_end_address).next();
        }

        // Sanity
        println!(
            "Replace should be seeing {} + {} + {} = {} entries",
            start_count,
            num_gen,
            end_count,
            start_count + num_gen + end_count
        );

        // Clean-up the deleted elements
        let mut finished = false;
        let mut bot_ptr = start_replace_address;
        while !finished {
            finished = bot_ptr == end_replace_address;
            let next_address = beneath.deref(bot_ptr).next();
            beneath.inner.arena.remove(bot_ptr);
            if next_address.is_some() {
                bot_ptr = next_address.unwrap();
            }
        }

        (poison_head, poison_tail, data_start_address, data_end_address)
    }

    /// Helper function to ensure a memory node has a model that works
    fn test_mem_node_model<V: Value>(mem_node: &PGMNode<KType, V, EPSILON>) {
        let node = &mem_node.inner;
        for (ix, entry) in node.entries().iter().enumerate() {
            let pred_ix = node.approximate(&entry.key);
            assert!(pred_ix.lo <= ix && ix < pred_ix.hi);
        }
    }

    /// Helper function to make sure a layer is normal
    /// Normal here means that the sizes are consistent, and each node is well-approximated
    fn test_is_layer_normal<V: Value>(layer: &MemoryPGMLayer<KType, V, EPSILON, Index>, size_hint: Option<usize>) {
        // First lets check that the total size of all nodes in layer is what we expect
        let mut ptr = Some(layer.inner.first());
        let mut seen: usize = 0;
        while ptr.is_some() {
            let mem_node = layer.deref(ptr.unwrap());
            seen += mem_node.inner.data.len();
            ptr = mem_node.next();
        }
        if size_hint.is_some() {
            assert!(seen == size_hint.unwrap());
        } else {
            assert!(seen > 0);
        }
        // Then for each node lets check that all its entries are well-approximated
        let mut ptr = Some(layer.inner.first());
        let mut seen: usize = 0;
        while ptr.is_some() {
            let mem_node = layer.deref(ptr.unwrap());
            test_mem_node_model(mem_node);
            ptr = mem_node.next();
        }
    }

    /// Helper function to make sure a pair of layers is normal.
    /// NOTE: Normal here applies to the relationship between the layers, not
    /// the layers themselves. That is, normalcy is about how the parents relate
    /// to children and how indexing through the layers functions.
    fn assert_layers_are_normal(
        beneath: &MemoryPGMLayer<KType, VType, EPSILON, Index>,
        above: &MemoryPGMLayer<KType, Index, EPSILON, Index>,
    ) {
        // First lets check that every node in the bottom layer has a parent, and that that parent has a key <= our key,
        // and that the next parent either doesn't exist or has a key > our key
        let mut bot_ptr = Some(beneath.inner.first());
        while bot_ptr.is_some() {
            let mem_node = beneath.deref(bot_ptr.unwrap());
            assert!(mem_node.parent().is_some());
            let parent_node = above.deref(mem_node.parent().unwrap());
            assert!(mem_node.lower_bound() <= mem_node.lower_bound());
            if parent_node.next().is_some() {
                let uncle_node = above.deref(parent_node.next().unwrap());
                assert!(mem_node.lower_bound() < uncle_node.lower_bound());
            }
            bot_ptr = mem_node.next();
        }
        // Now we'll try indexing into the lower level through the first level
        let mut bot_ptr = Some(beneath.inner.first());
        while bot_ptr.is_some() {
            let mem_node = beneath.deref(bot_ptr.unwrap());
            let entries = mem_node.inner.entries();
            let parent_node = above.deref(mem_node.parent().unwrap());
            for entry in entries {
                let mut approx = parent_node.inner.approximate(&entry.key);
                approx.hi = approx.hi.min(parent_node.inner.entries().len());
                let mut found = false;
                for ix in approx.lo..approx.hi {
                    let value = parent_node.inner.entries()[ix].value;
                    if value == bot_ptr.unwrap() {
                        found = true;
                        break;
                    }
                }
                assert!(found);
            }
            bot_ptr = mem_node.next();
        }
    }

    /// This tests the basic functionalities of fill as nothing more than a bunch of wrappers.
    /// Specifically, given an iterator over entries, we should be able to build a layer, which is a
    /// connected list of nodes, with accurate models and data.
    #[test]
    fn basic_fill() {
        let size: usize = 10_000;
        let layer = make_simple_layer(size);
        test_is_layer_normal::<VType>(&layer, Some(size));
    }

    /// This test the functionality of building a layer on top of a layer below it.
    #[test]
    fn fill_from_beneath() {
        let beneath_size: usize = 100_000;
        let mut beneath = make_simple_layer(beneath_size);
        let mut layer = MemoryPGMLayer::<KType, Index, EPSILON, Index>::new();
        layer.fill_from_beneath::<MemoryPGMLayer<KType, VType, EPSILON, Index>>(&mut beneath);
        assert_layers_are_normal(&beneath, &layer);
    }

    /// Runs a single trial of our replacement correctness test
    fn test_replace_trial(num_elements: usize) {
        let (mut beneath, mut above) = make_two_layers(num_elements);
        let (poison_head, poison_tail, data_head, data_tail) = generate_fake_replace(&mut beneath, &above);
        // Let's make sure that the beneath layer is still normal
        test_is_layer_normal(&beneath, None);
        // Magic!
        above.replace(&mut beneath, poison_head, poison_tail, data_head, data_tail);
        // Magic?
        assert_layers_are_normal(&beneath, &above);
        test_is_layer_normal(&beneath, None);
        test_is_layer_normal(&above, None);
    }

    #[test]
    fn test_replace() {
        let num_elements: usize = 1_000_000;
        let num_trials: usize = 100;
        let mut pb = tqdm!(total = num_trials);
        for _ in 0..num_trials {
            pb.update(1);
            test_replace_trial(num_elements);
        }
    }
}
