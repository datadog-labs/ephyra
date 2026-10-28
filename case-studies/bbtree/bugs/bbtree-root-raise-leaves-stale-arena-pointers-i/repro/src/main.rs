use std::collections::BTreeMap;
use bbtree_mmap::BBTreeMapFile;

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let map: BTreeMap<u64, String> = (0..200u64).map(|k| (k, "x".repeat(100))).collect();
    BBTreeMapFile::<u64, String>::create(&out, &map).unwrap();
}
