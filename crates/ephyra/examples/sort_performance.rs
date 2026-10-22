// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Targeted testing: hunting for slow inputs. Run with
//! `cargo run -p ephyra --example sort_performance`.
//!
//! The property never fails; it reports the number of swaps insertion sort
//! performs through `ephyra::target`, normalised by the list length so
//! longer lists are not automatically "better". After the random phase the
//! runner mutates the highest-scoring inputs with the generator's mutator,
//! here a domain mutator layered over the generic one, to push the score
//! up. The report names the worst input it found: for insertion sort that
//! is a descending list, where the swap count approaches n(n-1)/2.

use ephyra::{
    Config,
    Gen,
    GenExt,
    Source,
    check,
    gens,
    target,
};

fn insertion_sort_swaps(array: &mut [i32]) -> usize {
    let mut swaps = 0;
    for i in 1..array.len() {
        let mut j = i;
        while j > 0 && array[j - 1] > array[j] {
            array.swap(j - 1, j);
            swaps += 1;
            j -= 1;
        }
    }
    swaps
}

fn main() {
    let config = Config::default().max_size(12).name("sort_performance");
    // A domain mutator: move one element one place toward descending order.
    // Explicitly fall back to the generic vec mutator (perturb, swap, ...)
    // when the chosen pair is already in descending order.
    let base = gens::vec(gens::int(0i32..=100)).len(12..=12);
    let fallback = base.clone();
    let inputs = base.with_mutate(move |v: &Vec<i32>, g: &mut Source| {
        let i = g.index(v.len() - 1);
        if v[i] < v[i + 1] {
            let mut w = v.clone();
            w.swap(i, i + 1);
            w
        } else {
            fallback.mutate(v, g)
        }
    });
    let report = check(&config, inputs, |mut v: Vec<i32>| {
        let n = v.len();
        let swaps = insertion_sort_swaps(&mut v);
        let worst = n * (n - 1) / 2;
        target(swaps as f64 / worst as f64);
        v.windows(2).all(|w| w[0] <= w[1])
    });
    print!("{report}");
    let summary = report.target.as_ref().expect("the property reports scores");
    println!(
        "worst case found: {} swaps out of a possible 66 ({:.0}%)",
        (summary.best_score * 66.0).round(),
        summary.best_score * 100.0
    );
    assert!(report.passed());
}
