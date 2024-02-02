// #![feature(log_syntax)]
// #![feature(trace_macros)]
// trace_macros!(false);

mod expanded_code;

use limousine_core::learned::pgm::viz::play_pgm;
use limousine_engine::prelude::*;

// create_hybrid_index! {
//     name: MyHybridIndex,
//     layout: [
//         btree_top(),
//         btree(fanout = 12),
//         btree(fanout = 12),
//         btree(fanout = 12),
//         btree(fanout = 12),
//     ]
// }

fn main() {
    let num = 600_000;

    create_hybrid_index! {
        name: BasicHybrid,
        layout: [
            btree_top(),
            pgm(epsilon=16),
            pgm(epsilon=64),
        ]
    }
    // create_hybrid_index! {
    //     name: BasicBTree,
    //     layout: [
    //         btree_top(),
    //         btree(fanout = 12),
    //         btree(fanout = 12),
    //     ]
    // }

    // let mut index: MyHybridIndex<u128, u128> = MyHybridIndex::empty();
    // let start = std::time::Instant::now();
    // for i in 0..num {
    //     index.insert(i, i * i);
    // }
    // println!("{:?} after {:?} ms", index.search(&10), start.elapsed().as_millis());

    // let start = std::time::Instant::now();
    // for i in 0..num {
    //     index.insert(i, i * i);
    // }
    // println!("{:?} after {:?} ms", index.search(&10), start.elapsed());

    // use std::collections::BTreeMap;
    // let mut index: BTreeMap<u128, u128> = BTreeMap::new();

    // let start = std::time::Instant::now();
    // for i in 0..num {
    //     index.insert(i, i * i);
    // }
    // println!("{:?} after {:?} ms", index.get(&10), start.elapsed().as_millis());
}

#[cfg(test)]
mod main_macro_tests {
    use std::{fs, time::Instant};

    use limousine_core::Entry;

    use super::*;

    #[test]
    fn basic_btree() {
        // Create the thing
        create_hybrid_index! {
            name: BasicBTree,
            layout: [
                btree_top(),
                btree(fanout = 12),
                btree(fanout = 12),
            ]
        }
        let mut index: BasicBTree<i32, i32> = BasicBTree::empty();

        // Time inserts
        let num_inserts = 600_000;
        let start_insert = Instant::now();
        for i in 0..num_inserts {
            index.insert(i, i);
        }
        println!(
            "Time to insert {} things: {} ms",
            num_inserts,
            start_insert.elapsed().as_millis()
        );

        // Time searches
        let num_searches = num_inserts;
        let start_search = Instant::now();
        for i in 0..num_searches {
            let res = index.search(&i);
            assert!(res.unwrap() == &i);
        }
        println!(
            "Time to search {} things: {} ms",
            num_searches,
            start_search.elapsed().as_millis()
        );
    }

    #[test]
    fn basic_hybrid() {
        // Create the thing
        create_hybrid_index! {
            name: BasicHybrid,
            layout: [
                btree_top(),
                pgm(epsilon=16),
                pgm(epsilon=64),
            ]
        }

        // Time building
        let num_built_from = 1_000_000;
        let numbers: Vec<u32> = (0..=num_built_from).collect();
        let entries = numbers.clone().into_iter().map(|num| Entry::new(num, num));
        let start_build = Instant::now();
        let mut index: BasicHybrid<u32, u32> = BasicHybrid::build(entries);
        println!(
            "Time to build {} things: {} ms",
            num_built_from,
            start_build.elapsed().as_millis()
        );

        // Time searching
        let start_search = Instant::now();
        for num in numbers.iter() {
            let result = index.search(num);
            assert!(result.unwrap() == num);
        }
        println!(
            "Time to search {} things: {} ms",
            num_built_from,
            start_search.elapsed().as_millis()
        );
    }
}
