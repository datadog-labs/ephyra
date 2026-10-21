//! Buckets URLPatternInit-input differences: where urlpattern-polyfill and
//! Deno agree and ada differs, by the first differing component, with the
//! smallest example per bucket.
use std::collections::BTreeMap;

use ephyra::{Gen, Source};
use ephyra_ada::init_inputs::{ada, component_name, deno, reference, InitCases};

fn main() {
    let names = [
        "protocol", "username", "password", "hostname", "port", "pathname", "search", "hash",
        "baseURL",
    ];
    let mut buckets: BTreeMap<String, (usize, String, usize)> = BTreeMap::new();
    let (mut total, mut agreeing) = (0, 0);
    for i in 0..20_000u64 {
        let inputs = InitCases.generate(&mut Source::new(i, (i % 100) as usize));
        let a = ada(&inputs);
        let p = reference(&inputs);
        let Some(d) = deno(&inputs) else { continue };
        for (k, input) in inputs.iter().enumerate() {
            total += 1;
            if p[k] != d[k] {
                continue;
            }
            agreeing += 1;
            if a[k] == p[k] {
                continue;
            }
            let key = if a[k].0 != p[k].0 {
                format!("status ada={} refs={}", a[k].0, p[k].0)
            } else {
                let c = (0..8).find(|&c| a[k].1[c] != p[k].1[c]).unwrap();
                format!("component {}", component_name(c))
            };
            let shown: Vec<String> = names
                .iter()
                .zip(input)
                .filter_map(|(n, f)| f.as_ref().map(|v| format!("{n}={v:?}")))
                .collect();
            let text =
                format!("{{{}}}\n      ada  {:?}\n      refs {:?}", shown.join(", "), a[k], p[k]);
            let len = text.len();
            let e = buckets.entry(key).or_insert((0, text.clone(), len));
            e.0 += 1;
            if len < e.2 {
                e.1 = text;
                e.2 = len;
            }
        }
    }
    println!("{total} inputs, references agree on {agreeing}");
    for (k, (n, ex, _)) in buckets {
        println!("{n:6} {k}\n    {ex}");
    }
}
