use bbtree::BBTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn main() {
    let map = BBTreeMap::<u64, u64>::new();
    for k in 0..55 {
        map.insert(k, k).unwrap(); // 55 entries fill the root leaf
    }
    // A new key: the insert splits the leaf, and the closure panics.
    let caught = catch_unwind(AssertUnwindSafe(|| {
        map.get_or_insert(55, 55, |_, _| panic!("closure panics"))
    }));
    println!("caught: {}", caught.is_err());
}
