// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

use std::collections::BTreeMap;
use std::ops::Bound::*;
use std::panic::catch_unwind;
use bbtree::file::{FileMap, FileWriter};
use bbtree::unsync::BBTreeMap;

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    for n in [0u64, 1, 5, 1000] {
        let std_panics = catch_unwind(|| { let m: BTreeMap<u64, u64> = (0..n).map(|k| (k, k)).collect(); m.range(10..5).count() }).is_err();
        let unsync_panics = catch_unwind(|| { let m: BBTreeMap<u64, u64> = (0..n).map(|k| (k, k)).collect(); m.range(10..5).count() }).is_err();
        let sync_panics = catch_unwind(|| { let m: bbtree::BBTreeMap<u64, u64> = bbtree::BBTreeMap::new(); for k in 0..n { m.insert(k, k).unwrap(); } m.range_cloned(10..5).count() }).is_err();
        let file_panics = catch_unwind(|| {
            let m: BBTreeMap<u64, u64> = (0..n).map(|k| (k, k)).collect();
            let buf = FileWriter::from(&m).to_buf();
            let f = FileMap::<u64, u64>::open(&buf).unwrap();
            f.range((Excluded(5u64), Excluded(5u64))).count() + f.range(10..5).count()
        }).is_err();
        println!("n={n}: std {std_panics} unsync {unsync_panics} sync {sync_panics} file {file_panics}");
    }
}
