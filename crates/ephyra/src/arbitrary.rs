// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! [`Arbitrary`]: the default generator of a type, and [`any`].

use std::collections::{
    BTreeMap,
    BTreeSet,
    HashMap,
    HashSet,
};

use crate::{
    Source,
    gens::{
        self,
        BoxGen,
        Gen,
        GenExt,
    },
    hooks::ShrinkVisitor,
};

/// Types with a default generator. Use [`any`] to obtain it.
///
/// The built-in defaults are documented mixes of visible generators, so the
/// baseline distribution is explicit and can be rebuilt with the same
/// combinators:
///
/// - integers: `frequency([(1, edges(..)), (7, int(..))])`, so one draw in
///   eight is a range end, `0`, `±1` or a neighbour of an end;
/// - floats: `frequency([(1, specials), (2, small round numbers), (2, unit
///   interval), (3, random bit patterns)])`, where the specials are `0`,
///   `-0`, `±1`, `±0.5`, NaN, `±inf`, `MIN`, `MAX`, `EPSILON`,
///   `MIN_POSITIVE` and the smallest subnormal;
/// - collections: length uniform in `0..=size`, elements from their
///   defaults.
///
/// Every default carries the shrinker and mutator of the generators it is
/// built from.
///
/// Implement this for your own types when one generator is the obvious
/// default; otherwise pass generators explicitly.
pub trait Arbitrary: Sized + 'static {
    /// The default generator for this type.
    fn arbitrary() -> BoxGen<Self>;
}

/// The default generator for `T`.
pub fn any<T: Arbitrary>() -> BoxGen<T> {
    T::arbitrary()
}

macro_rules! impl_arbitrary_int {
    ($($t:ty),*) => {$(
        impl Arbitrary for $t {
            fn arbitrary() -> BoxGen<Self> {
                // The edges one time in eight. The hooks are those of the full
                // range, which hold for every value of the type.
                let full = gens::int::<$t>(..);
                gens::frequency(vec![(1, gens::edges::<$t>(..).boxed()), (7, full.boxed())])
                    .with_shrink(move |value, visit| full.shrink(value, visit))
                    .with_mutate(move |value, g| full.mutate(value, g))
                    .boxed()
            }
        }
    )*};
}
impl_arbitrary_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl Arbitrary for bool {
    fn arbitrary() -> BoxGen<Self> {
        gens::bool().boxed()
    }
}

/// Floats: special values, small integers plus a simple fraction, the unit
/// interval and raw bit patterns.
macro_rules! impl_arbitrary_float {
    ($($t:ident: $unit:ident, $bits:ident);*) => {$(
        impl Arbitrary for $t {
            fn arbitrary() -> BoxGen<Self> {
                // `pick_from` shrinks toward earlier entries, so list them in
                // the order the float shrinker reduces: by magnitude, the
                // positive value before the negative one, NaN last. Any other
                // order lets the two shrinkers undo each other (0.5 -> 1.0 -> 0.5).
                const SPECIALS: [$t; 14] = [
                    0.0,
                    -0.0,
                    $t::from_bits(1), // the smallest subnormal
                    $t::MIN_POSITIVE,
                    $t::EPSILON,
                    0.5,
                    -0.5,
                    1.0,
                    -1.0,
                    $t::MAX,
                    $t::MIN,
                    $t::INFINITY,
                    $t::NEG_INFINITY,
                    $t::NAN,
                ];
                const FRACTIONS: [$t; 4] = [0.0, 0.5, 0.25, 0.1];
                // Special values shrink to the plain ones at the start of the
                // list, not to all before them: NaN is not simpler than the
                // smallest subnormal.
                let special = |g: &mut Source| *g.pick(&SPECIALS);
                let plain = |value: &$t, visitor: &mut ShrinkVisitor<'_, $t>| {
                    let index = SPECIALS.iter().position(|s| s == value).unwrap_or(0);
                    SPECIALS[..index.min(6)].iter().try_for_each(|s| visitor(*s))
                };
                // The mix below only generates. The hooks are the bit patterns',
                // which hold for every value of the type, with the plain special
                // values first.
                let bits = gens::$bits();
                gens::frequency(vec![
                    (1, special.boxed()),
                    (
                        2,
                        (gens::int(-100i32..=100), gens::int(0usize..4))
                            .map(|(i, f)| i as $t + FRACTIONS[f])
                            .boxed(),
                    ),
                    (2, gens::$unit().boxed()),
                    (3, bits.boxed()),
                ])
                .with_shrink(move |value, visit| {
                    plain(value, visit)?;
                    bits.shrink(value, visit)
                })
                .with_mutate(move |value, g| bits.mutate(value, g))
                .boxed()
            }
        }
    )*};
}
impl_arbitrary_float!(f64: f64, f64_bits; f32: f32, f32_bits);

impl Arbitrary for char {
    fn arbitrary() -> BoxGen<Self> {
        gens::char().boxed()
    }
}

impl Arbitrary for String {
    fn arbitrary() -> BoxGen<Self> {
        gens::string().boxed()
    }
}

impl Arbitrary for () {
    fn arbitrary() -> BoxGen<Self> {
        gens::constant(()).boxed()
    }
}

impl<T: Arbitrary + Clone> Arbitrary for Vec<T> {
    fn arbitrary() -> BoxGen<Self> {
        gens::vec(any::<T>()).boxed()
    }
}

impl<T: Arbitrary> Arbitrary for Option<T> {
    fn arbitrary() -> BoxGen<Self> {
        gens::option(any::<T>()).boxed()
    }
}

impl<T: Arbitrary, E: Arbitrary> Arbitrary for Result<T, E> {
    fn arbitrary() -> BoxGen<Self> {
        gens::result(any::<T>(), any::<E>()).boxed()
    }
}

impl<T: Arbitrary + Clone> Arbitrary for Box<T> {
    fn arbitrary() -> BoxGen<Self> {
        any::<T>().iso(Box::new, |b| (**b).clone()).boxed()
    }
}

impl<T: Arbitrary + Clone, const N: usize> Arbitrary for [T; N] {
    fn arbitrary() -> BoxGen<Self> {
        std::array::from_fn::<BoxGen<T>, N, _>(|_| any::<T>()).boxed()
    }
}

impl<K: Arbitrary + std::hash::Hash + Eq + Clone + Send + Sync, V: Arbitrary + Clone + Send + Sync>
    Arbitrary for HashMap<K, V>
{
    fn arbitrary() -> BoxGen<Self> {
        gens::vec((any::<K>(), any::<V>())).collect().boxed()
    }
}

impl<K: Arbitrary + Ord + Clone + Send + Sync, V: Arbitrary + Clone + Send + Sync> Arbitrary
    for BTreeMap<K, V>
{
    fn arbitrary() -> BoxGen<Self> {
        gens::vec((any::<K>(), any::<V>())).collect().boxed()
    }
}

impl<T: Arbitrary + std::hash::Hash + Eq + Clone + Send + Sync> Arbitrary for HashSet<T> {
    fn arbitrary() -> BoxGen<Self> {
        gens::vec(any::<T>()).collect().boxed()
    }
}

impl<T: Arbitrary + Ord + Clone + Send + Sync> Arbitrary for BTreeSet<T> {
    fn arbitrary() -> BoxGen<Self> {
        gens::vec(any::<T>()).collect().boxed()
    }
}

macro_rules! impl_arbitrary_tuple {
    ($($T:ident),+) => {
        impl<$($T: Arbitrary + Clone),+> Arbitrary for ($($T,)+) {
            fn arbitrary() -> BoxGen<Self> {
                ($(any::<$T>(),)+).boxed()
            }
        }
    };
}
impl_arbitrary_tuple!(A);
impl_arbitrary_tuple!(A, B);
impl_arbitrary_tuple!(A, B, C);
impl_arbitrary_tuple!(A, B, C, D);
impl_arbitrary_tuple!(A, B, C, D, E);
impl_arbitrary_tuple!(A, B, C, D, E, F);
impl_arbitrary_tuple!(A, B, C, D, E, F, G);
impl_arbitrary_tuple!(A, B, C, D, E, F, G, H);

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            Gen,
            Source,
        },
    };

    #[test]
    fn any_float_has_no_two_step_shrink_cycle() {
        // Found by ephyra-agent: the special values shrank toward earlier
        // entries of their list, the float shrinker the other way (0.5 -> 1.0 -> 0.5).
        fn check<T: Arbitrary + Clone + std::fmt::Debug>(values: &[T]) {
            let g = crate::any::<T>();
            let key = |v: &T| format!("{v:?}");
            for v in values {
                for c in g.shrink_candidates(v) {
                    let back = g.shrink_candidates(&c).iter().any(|d| key(d) == key(v));
                    assert!(!back, "{v:?} -> {c:?} -> {v:?}");
                }
            }
        }
        check(&[
            0.5f64,
            -0.5,
            1.0,
            -1.0,
            f64::MAX,
            f64::MIN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ]);
        check(&[
            0.5f32,
            -0.5,
            1.0,
            -1.0,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ]);
    }

    #[test]
    fn std_types_have_default_generators() {
        let mut g = Source::new(1, 8);
        let _: (i32, u64, bool, f64, char, String) = any().generate(&mut g);
        let v: Vec<Option<u8>> = any().generate(&mut g);
        assert!(v.len() <= 8);
        let _: Result<Box<i8>, ()> = any().generate(&mut g);
        let _: [bool; 4] = any().generate(&mut g);
        let _: HashMap<u8, i16> = any().generate(&mut g);
        let _: BTreeSet<char> = any().generate(&mut g);
        let _: (i8, i16, i32, i64, isize, u8, u16, u32) = any().generate(&mut g);
    }

    #[test]
    fn default_mixes_hit_edges_and_specials() {
        let mut g = Source::new(5, 10);
        let (mut min, mut max, mut nan) = (0, 0, 0);
        for _ in 0..4000 {
            match any::<i32>().generate(&mut g) {
                i32::MIN => min += 1,
                i32::MAX => max += 1,
                _ => {},
            }
            if any::<f64>().generate(&mut g).is_nan() {
                nan += 1;
            }
        }
        assert!(min > 20 && max > 20 && nan > 5, "min={min} max={max} nan={nan}");
    }
}
