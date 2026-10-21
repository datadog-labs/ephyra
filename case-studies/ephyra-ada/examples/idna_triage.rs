// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Buckets ada-vs-tr46 IDNA differences over generated domains, with the
//! shortest example per bucket. to_ascii: compares success and output.
//! to_unicode: compares output.
use std::collections::BTreeMap;

use ephyra::{Gen, Source};
use ephyra_ada::idna_direct::{ada, reference, IdnaCases};

fn main() {
    let mut buckets: BTreeMap<&str, (usize, String)> = BTreeMap::new();
    let mut total = 0;
    for i in 0..20_000u64 {
        let domains = IdnaCases.generate(&mut Source::new(i, (i % 120) as usize));
        let a = ada(&domains);
        let r = reference(&domains);
        for (k, d) in domains.iter().enumerate() {
            total += 1;
            let key = if a[k].to_ascii_ok != r[k].to_ascii_ok {
                "to_ascii success differs"
            } else if a[k].to_ascii_ok && a[k].to_ascii != r[k].to_ascii {
                "to_ascii output differs"
            } else if a[k].to_unicode != r[k].to_unicode {
                "to_unicode output differs"
            } else {
                continue;
            };
            let text = format!(
                "{d:?}\n      ada  ok={} ascii={:?} uni={:?}\n      tr46 ok={} ascii={:?} uni={:?}",
                a[k].to_ascii_ok,
                a[k].to_ascii,
                a[k].to_unicode,
                r[k].to_ascii_ok,
                r[k].to_ascii,
                r[k].to_unicode
            );
            let e = buckets.entry(key).or_insert((0, text.clone()));
            e.0 += 1;
            if d.len() < e.1.lines().next().unwrap().len() {
                e.1 = text;
            }
        }
    }
    println!("{total} domains");
    for (k, (n, ex)) in &buckets {
        println!("{n:6} {k}\n    {ex}");
    }
}
