// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! One property: **every built-in generator keeps its contract.**
//!
//! The contract is what `Gen` documents and what the search relies on, for a generator
//! with support `S` (the values it documents it can produce):
//!
//! 1. a seed replays the same value;
//! 2. `generate` gives a value in `S`, unless it discards (marks its `Source`, which the
//!    runner skips);
//! 3. `mutate` keeps a value in `S`, along a chain of mutations as the fuzzing loops do,
//!    unless it discards;
//! 4. every shrink candidate is in `S` and differs from its input;
//! 5. shrinking settles: a walk over shrink candidates never comes back to a value;
//! 6. `generate` spreads over the support: when the support has two values (two distinct
//!    `starts`), draws are not all the same value.
//!
//! The generators and their arguments are drawn by ephyra itself, edges first. Every bug in
//! `regressions/` whose class is a broken generator contract is caught by this one test;
//! see CONTRIBUTING.md, "Bugs: a property for the class, and a regression entry".
use ephyra::{
    Config,
    Gen,
    GenExt,
    Source,
    TestResult,
    any,
    check,
    gens,
    source::Int,
};

mod common;
use common::keeps_contract;

/// Two bounds in order, each an edge of the type half of the time.
fn bounds<T: Int>(g: &mut Source) -> (T, T) {
    let bound = |g: &mut Source| {
        if g.bool() { gens::edges::<T>(..).generate(g) } else { gens::int::<T>(..).generate(g) }
    };
    let (a, b) = (bound(g), bound(g));
    if a <= b { (a, b) } else { (b, a) }
}

/// A finite `f64`, the type's edges a third of the time.
fn finite_f64(g: &mut Source) -> f64 {
    const EDGES: [f64; 8] =
        [0.0, -0.0, 1.0, -1.0, f64::MAX, f64::MIN, f64::MIN_POSITIVE, f64::EPSILON];
    match g.int(0u8..3) {
        0 => EDGES[g.int(0..EDGES.len())],
        1 => g.f64() * 2e6 - 1e6,
        _ => {
            loop {
                let x = g.f64_bits();
                if x.is_finite() {
                    break x;
                }
            }
        },
    }
}

/// The catalogue: each entry builds one built-in generator from arguments drawn from `g`,
/// with its documented support, and checks the contract.
type Entry = (&'static str, fn(&mut Source, (u64, usize)) -> Result<(), String>);

const CATALOGUE: &[Entry] = &[
    ("int::<u8>", |g, s| {
        let (lo, hi) = bounds::<u8>(g);
        keeps_contract(&gens::int(lo..=hi), |v| (lo..=hi).contains(v), &[lo, hi], s)
    }),
    ("int::<i64>", |g, s| {
        let (lo, hi) = bounds::<i64>(g);
        keeps_contract(&gens::int(lo..=hi), |v| (lo..=hi).contains(v), &[lo, hi], s)
    }),
    ("int::<u64> (exclusive end)", |g, s| {
        let (lo, hi) = bounds::<u64>(g);
        if lo == hi {
            return Ok(());
        }
        keeps_contract(&gens::int(lo..hi), |v| (lo..hi).contains(v), &[lo], s)
    }),
    ("edges::<i32>", |g, s| {
        let (lo, hi) = bounds::<i32>(g);
        let set = [lo, hi, lo.saturating_add(1), hi.saturating_sub(1), 0, 1, -1];
        keeps_contract(
            &gens::edges(lo..=hi),
            |v| (lo..=hi).contains(v) && set.contains(v),
            &[lo, hi],
            s,
        )
    }),
    ("f64", |_, s| {
        keeps_contract(
            &gens::f64(),
            |v| (0.0..1.0).contains(v),
            &[0.0, 0.99999, 1.0 - f64::EPSILON / 2.0],
            s,
        )
    }),
    ("f32", |_, s| {
        keeps_contract(
            &gens::f32(),
            |v| (0.0..1.0).contains(v),
            &[0.0, 0.99999, 1.0 - f32::EPSILON / 2.0],
            s,
        )
    }),
    ("f64_range", |g, s| {
        let (a, b) = (finite_f64(g), finite_f64(g));
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        // -0.0 and 0.0 print apart but are one value of the range
        let starts = if lo < hi { vec![lo, hi] } else { vec![lo] };
        keeps_contract(&gens::f64_range(lo..=hi), |v| (lo..=hi).contains(v), &starts, s)
    }),
    ("f64_bits", |_, s| {
        let starts =
            [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, f64::MAX, f64::MIN, f64::from_bits(1)];
        keeps_contract(&gens::f64_bits(), |_| true, &starts, s)
    }),
    ("f32_bits", |_, s| {
        let starts =
            [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, f32::MAX, f32::MIN, f32::from_bits(1)];
        keeps_contract(&gens::f32_bits(), |_| true, &starts, s)
    }),
    ("any::<f64>", |_, s| {
        let starts = [0.5, -0.5, 1.0, f64::MAX, f64::INFINITY, f64::NEG_INFINITY, f64::NAN];
        keeps_contract(&any::<f64>(), |_| true, &starts, s)
    }),
    ("any::<f32>", |_, s| {
        let starts = [0.5, -0.5, 1.0, f32::MAX, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
        keeps_contract(&any::<f32>(), |_| true, &starts, s)
    }),
    ("char classes", |_, s| {
        keeps_contract(&gens::char(), |_| true, &['\0', '\u{10FFFF}'], s)?;
        keeps_contract(&gens::ascii_char(), |c| (' '..='~').contains(c), &[' ', '~'], s)?;
        keeps_contract(&gens::alphanumeric(), |c| c.is_ascii_alphanumeric(), &['0', 'Z', 'z'], s)
    }),
    ("vec(int).len(min..=max)", |g, s| {
        let (min, max) = bounds::<u8>(g);
        let (min, max) = (usize::from(min % 7), usize::from(max % 7).max(usize::from(min % 7)));
        let (lo, hi) = bounds::<i8>(g);
        let support =
            |v: &Vec<i8>| (min..=max).contains(&v.len()) && v.iter().all(|x| (lo..=hi).contains(x));
        keeps_contract(
            &gens::vec(gens::int(lo..=hi)).len(min..=max),
            support,
            &[vec![lo; min], vec![hi; max]],
            s,
        )
    }),
    ("string_len(alphanumeric)", |g, s| {
        let (min, max) = (g.int(0usize..=6), g.int(0usize..=6));
        let (min, max) = (min.min(max), min.max(max));
        let support = |v: &String| {
            (min..=max).contains(&v.chars().count()) && v.chars().all(|c| c.is_ascii_alphanumeric())
        };
        keeps_contract(
            &gens::string_len(gens::alphanumeric(), min..=max),
            support,
            &["a".repeat(min)],
            s,
        )
    }),
    ("option, result", |g, s| {
        let (lo, hi) = bounds::<i16>(g);
        let inner = |v: &i16| (lo..=hi).contains(v);
        keeps_contract(
            &gens::option(gens::int(lo..=hi)),
            |v| v.as_ref().is_none_or(inner),
            &[None, Some(hi)],
            s,
        )?;
        let support = |v: &Result<i16, char>| {
            match v {
                Ok(x) => inner(x),
                Err(c) => (' '..='~').contains(c),
            }
        };
        keeps_contract(
            &gens::result(gens::int(lo..=hi), gens::ascii_char()),
            support,
            &[Ok(hi), Err('~')],
            s,
        )
    }),
    ("pick_from", |g, s| {
        let values: Vec<i32> = (0..g.int(1usize..=8)).map(|_| g.int(-50..=50)).collect();
        keeps_contract(&gens::pick_from(values.clone()), |v| values.contains(v), &values, s)
    }),
    ("filter", |g, s| {
        let m = g.int(2u32..=7);
        keeps_contract(
            &gens::int(0u32..=1000).filter("multiple", move |x| x % m == 0),
            |x| x % m == 0,
            &[0],
            s,
        )
    }),
    ("one_of, structured alternatives", |g, s| {
        // a tag and a length that go together: a hook that mixes the parts of two
        // alternatives gives a value that neither can make
        let (a, b) = (g.int(1usize..=4), g.int(5usize..=8));
        let alternative = |tag: u8, n: usize| {
            (gens::constant(tag), gens::vec(gens::int(0u8..=9)).len(n..=n)).boxed()
        };
        let choice = gens::one_of(vec![alternative(4, a), alternative(6, b)]);
        let support =
            |(t, v): &(u8, Vec<u8>)| (*t == 4 && v.len() == a) || (*t == 6 && v.len() == b);
        keeps_contract(&choice, support, &[(4, vec![0; a]), (6, vec![9; b])], s)
    }),
    ("dependent, inner filter", |g, s| {
        // a vector of at least `n` elements: the filter can run out of attempts at a small size
        let top = g.int(1usize..=6);
        let pair = gens::dependent(gens::int(1usize..=top), |n| {
            let n = *n;
            gens::vec(gens::int(0u8..=9)).filter("long enough", move |v| v.len() >= n)
        });
        let support = |(n, v): &(usize, Vec<u8>)| (1..=top).contains(n) && v.len() >= *n;
        keeps_contract(&pair, support, &[(top, vec![9; top])], s)
    }),
    ("recursive", |g, s| {
        let max_depth = g.int(0usize..=4);
        let tree =
            gens::recursive(max_depth, gens::int(0u8..=9).map(|x| vec![x]).boxed(), |inner| {
                gens::vec(inner).len(0..=3).map(|vs: Vec<Vec<u8>>| vs.concat()).boxed()
            });
        keeps_contract(&tree, |v| v.iter().all(|x| *x <= 9), &[], s)
    }),
];

#[test]
fn every_builtin_generator_keeps_its_contract() {
    let config = Config::default().seed(40).tests(1500).no_regressions();
    let case = (gens::int(0..CATALOGUE.len()), gens::int(0u64..=u64::MAX), gens::int(0usize..=40));
    check(&config, case, |(entry, seed, size)| {
        let (name, contract) = CATALOGUE[entry];
        // the generator's arguments come from their own source, so they shrink with the seed
        let mut args = Source::new(seed.rotate_left(17), size);
        match contract(&mut args, (seed, size)) {
            Ok(()) => TestResult::Pass,
            Err(why) => TestResult::fail(format!("{name}: {why}")),
        }
    })
    .assert_passed();
}
