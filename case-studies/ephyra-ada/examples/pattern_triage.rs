// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Buckets the differences between ada's URLPattern and the polyfill over
//! generated cases, printing the shortest case per bucket.
use std::collections::BTreeMap;

use ephyra::{Gen, Source};
use ephyra_ada::pattern::{ada, deno, reference, PatternCases, COMPONENTS};

fn main() {
    let mut buckets: BTreeMap<String, (usize, String, usize)> = BTreeMap::new();
    for i in 0..20_000u64 {
        let case = PatternCases.generate(&mut Source::new(i, (i % 100) as usize));
        let (a, p) = (ada(&case), reference(&case));
        let key = if a.ok != p.ok {
            format!("compiles ada={} polyfill={}", a.ok, p.ok)
        } else if a.components != p.components {
            let k = (0..8).find(|&k| a.components[k] != p.components[k]).unwrap();
            format!("component {}", COMPONENTS[k])
        } else if let Some((x, y)) = a.inputs.iter().zip(&p.inputs).find(|(x, y)| x != y) {
            if x.exec != y.exec {
                format!("exec ada={} polyfill={}", x.exec, y.exec)
            } else {
                "groups".to_owned()
            }
        } else {
            continue;
        };
        let Some(d) = deno(&case) else { continue };
        let side = |x: &ephyra_ada::pattern::PatternResult| {
            (x.ok, x.components.clone(), x.inputs.iter().map(|i| i.exec).collect::<Vec<_>>())
        };
        let verdict = if side(&d) == side(&a) {
            "deno=ada"
        } else if side(&d) == side(&p) {
            "deno=polyfill"
        } else {
            "deno=neither"
        };
        let key = format!("{key} [{verdict}]");
        let shown = format!(
            "{case:?}\n      ada  {:?} {:?}\n      pol  {:?} {:?}\n      deno {:?} {:?}",
            a.components,
            a.inputs.iter().map(|i| i.exec).collect::<Vec<_>>(),
            p.components,
            p.inputs.iter().map(|i| i.exec).collect::<Vec<_>>(),
            d.components,
            d.inputs.iter().map(|i| i.exec).collect::<Vec<_>>()
        );
        let len = format!("{case:?}").len();
        let e = buckets.entry(key).or_insert((0, shown.clone(), len));
        e.0 += 1;
        if len < e.2 {
            e.1 = shown;
            e.2 = len;
        }
    }
    for (k, (n, ex, _)) in buckets {
        println!("{n:6} {k}\n    {ex}");
    }
}
