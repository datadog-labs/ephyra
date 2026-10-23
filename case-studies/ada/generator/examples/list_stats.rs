// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! How often the URLPattern list oracle has something to check.
use ephyra::{Gen, Source};
use ephyra_ada::pattern_list::{plain_kinds, run, ListCases};

fn main() {
    let (mut cases, mut list_ok, mut paths, mut wins, mut regexp, mut multi, mut plain_pairs) =
        (0, 0, 0, 0, 0, 0, 0);
    let mut failing = Vec::new();
    for i in 0..20_000u64 {
        let case = ListCases.generate(&mut Source::new(i, (i % 200) as usize));
        cases += 1;
        let r = run(&case);
        if !r.list_ok {
            if failing.len() < 5 {
                failing.push(case.patterns.clone());
            }
            continue;
        }
        list_ok += 1;
        for p in &r.paths {
            paths += 1;
            let m: Vec<usize> =
                (0..case.patterns.len()).filter(|&k| p.alone[k].is_some()).collect();
            if p.route.is_some() {
                wins += 1;
            }
            if p.regexp {
                regexp += 1;
            }
            if m.len() > 1 {
                multi += 1;
                if m.iter().filter(|&&k| plain_kinds(&case.patterns[k]).is_some()).count() > 1 {
                    plain_pairs += 1;
                }
            }
        }
    }
    println!("cases {cases}, list compiles {list_ok}, paths {paths}, with a winner {wins}, regexp winners {regexp}, several matching {multi}, several plain matching {plain_pairs}");
    println!("lists that do not compile, e.g.: {failing:?}");
}
