//! The shrinking and mutation algorithms the built-in generators share, and
//! the visitor types of [`Gen::shrink`](crate::Gen::shrink).

use std::ops::ControlFlow;

use rand_core::RngCore;

use crate::{
    Source,
    source::{
        Int,
        small_magnitude,
        uniform,
    },
};

/// The control signal returned by a shrink visitor.
pub type ShrinkFlow = ControlFlow<()>;

/// A callback that receives shrink candidates in priority order.
pub type ShrinkVisitor<'a, T> = dyn FnMut(T) -> ShrinkFlow + 'a;

// --------------------------------------------------------------------------
// Shared algorithms
// --------------------------------------------------------------------------

/// Candidates for an integer in `lo..=hi`: the value closest to zero, then
/// binary steps from `value` toward it.
pub(crate) fn shrink_int<T: Int>(
    value: T,
    lo: i128,
    hi: i128,
    visitor: &mut ShrinkVisitor<'_, T>,
) -> ShrinkFlow {
    let v = value.to_i128();
    let target = 0i128.clamp(lo, hi);
    if v == target {
        return ControlFlow::Continue(());
    }
    visitor(T::from_i128(target))?;
    let mut last_emitted = target;
    let mut step = (v - target) / 2;
    while step != 0 {
        let candidate = v - step;
        if candidate != target && candidate != last_emitted {
            visitor(T::from_i128(candidate))?;
            last_emitted = candidate;
        }
        step /= 2;
    }
    let last = if v > target { v - 1 } else { v + 1 };
    if last != target && last != last_emitted {
        visitor(T::from_i128(last))?;
    }
    ControlFlow::Continue(())
}

/// A nearby integer in `lo..=hi`: a small step three times in four,
/// otherwise a fresh uniform draw.
pub(crate) fn mutate_int<T: Int>(value: T, lo: i128, hi: i128, g: &mut Source) -> T {
    let v = value.to_i128();
    let next = if g.next_u32().is_multiple_of(4) {
        lo + i128::from(uniform(g, (hi - lo) as u64))
    } else {
        let step = i128::from(small_magnitude(g));
        if g.next_u32() & 1 == 0 { v.saturating_add(step) } else { v.saturating_sub(step) }
    };
    T::from_i128(next.clamp(lo, hi))
}

/// Candidates for a float: zero, the positive value, its integer part, half.
pub(crate) fn shrink_f64(value: f64, visitor: &mut ShrinkVisitor<'_, f64>) -> ShrinkFlow {
    if value == 0.0 {
        return ControlFlow::Continue(());
    }
    let mut last_bits = None;
    let mut emit = |candidate: f64| {
        let bits = candidate.to_bits();
        if bits == value.to_bits() || last_bits == Some(bits) {
            return ControlFlow::Continue(());
        }
        last_bits = Some(bits);
        visitor(candidate)
    };
    emit(0.0)?;
    if value.is_nan() {
        emit(1.0)?;
    } else if value.is_infinite() {
        emit(if value > 0.0 { f64::MAX } else { f64::MIN })?;
    } else {
        if value < 0.0 {
            emit(-value)?;
        }
        let truncated = value.trunc();
        if truncated != value {
            emit(truncated)?;
        }
        emit(value / 2.0)?;
    }
    ControlFlow::Continue(())
}

/// A nearby float: negated, doubled, halved, nudged, or a fresh bit pattern.
pub(crate) fn mutate_f64(value: f64, g: &mut Source) -> f64 {
    match g.next_u32() % 5 {
        0 => -value,
        1 => value * 2.0,
        2 => value / 2.0,
        3 => value + (small_magnitude(g) as f64) * if g.bool() { 1.0 } else { -1.0 },
        _ => g.f64_bits(),
    }
}

/// Candidates for a sequence: drop blocks of 8, 4, 2, 1 elements (down to
/// `min` elements), then shrink each element in place.
pub(crate) fn shrink_seq<T: Clone>(
    items: &[T],
    min: usize,
    mut shrink_elem: impl FnMut(&T, &mut ShrinkVisitor<'_, T>) -> ShrinkFlow,
    visitor: &mut ShrinkVisitor<'_, Vec<T>>,
) -> ShrinkFlow {
    let len = items.len();
    for block in [8usize, 4, 2, 1] {
        if len < block || len - block < min {
            continue;
        }
        for start in 0..=len - block {
            let mut candidate = Vec::with_capacity(len - block);
            candidate.extend_from_slice(&items[..start]);
            candidate.extend_from_slice(&items[start + block..]);
            visitor(candidate)?;
        }
    }
    for (i, item) in items.iter().enumerate() {
        shrink_elem(item, &mut |smaller| {
            let mut candidate = items.to_vec();
            candidate[i] = smaller;
            visitor(candidate)
        })?;
    }
    ControlFlow::Continue(())
}

/// A nearby sequence: one element mutated, an element inserted (when a
/// generator is available), removed, duplicated, or two swapped, within
/// `min..=max` elements.
pub(crate) fn mutate_seq<T: Clone>(
    items: &[T],
    min: usize,
    max: usize,
    g: &mut Source,
    mut fresh: impl FnMut(&mut Source) -> T,
    mutate_elem: impl Fn(&T, &mut Source) -> T,
) -> Vec<T> {
    let len = items.len();
    for _ in 0..8 {
        let mut out = items.to_vec();
        match g.next_u32() % 5 {
            0 if len > 0 => {
                let i = g.index(len);
                out[i] = mutate_elem(&items[i], g);
                return out;
            },
            1 if len < max => {
                let at = g.int(0..=len);
                let value = fresh(g);
                out.insert(at, value);
                return out;
            },
            2 if len > min && len > 0 => {
                out.remove(g.index(len));
                return out;
            },
            3 if len > 0 && len < max => {
                let i = g.index(len);
                let copy = out[i].clone();
                out.insert(i, copy);
                return out;
            },
            4 if len >= 2 => {
                let a = g.index(len);
                let b = g.index(len);
                if a != b {
                    out.swap(a, b);
                    return out;
                }
            },
            _ => {},
        }
    }
    items.to_vec()
}

/// Characters shrink toward `'a'`, in binary steps along the code points.
pub(crate) fn shrink_char(value: char, visitor: &mut ShrinkVisitor<'_, char>) -> ShrinkFlow {
    let target = i64::from('a' as u32);
    let code = i64::from(value as u32);
    if code == target {
        return ControlFlow::Continue(());
    }
    visitor('a')?;
    let mut step = (code - target) / 2;
    while step != 0 {
        if let Some(c) = char::from_u32((code - step) as u32)
            && c != 'a'
        {
            visitor(c)?;
        }
        step /= 2;
    }
    ControlFlow::Continue(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::shrink::collect_candidates,
    };

    fn int_candidates(value: i32, lo: i128, hi: i128) -> Vec<i32> {
        collect_candidates(|visit| shrink_int(value, lo, hi, visit))
    }

    fn i32_candidates(value: i32) -> Vec<i32> {
        int_candidates(value, i128::from(i32::MIN), i128::from(i32::MAX))
    }

    #[test]
    fn ints_shrink_toward_zero_or_the_nearest_bound() {
        assert_eq!(i32_candidates(100), vec![0, 50, 75, 88, 94, 97, 99]);
        assert_eq!(i32_candidates(-3), vec![0, -2]);
        assert_eq!(i32_candidates(0), Vec::<i32>::new());
        assert_eq!(int_candidates(20, 10, 30), vec![10, 15, 18, 19]);
        assert_eq!(int_candidates(-20, -30, -10), vec![-10, -15, -18, -19]);
        assert_eq!(int_candidates(10, 10, 30), Vec::<i32>::new());
        assert_eq!(i32_candidates(1), vec![0]);
    }

    #[test]
    fn floats_and_chars_shrink_sensibly() {
        let floats = |value| collect_candidates(|visit| shrink_f64(value, visit));
        assert_eq!(floats(-7.5), vec![0.0, 7.5, -7.0, -3.75]);
        assert_eq!(floats(f64::NAN), vec![0.0, 1.0]);
        assert_eq!(floats(0.0), Vec::<f64>::new());
        let chars = |value| collect_candidates(|visit| shrink_char(value, visit));
        assert_eq!(chars('a'), Vec::<char>::new());
        assert_eq!(chars('e'), vec!['a', 'c', 'd']);
        assert!(chars('A').contains(&'a'));
    }

    #[test]
    fn sequences_drop_blocks_then_shrink_elements() {
        let v = vec![5i32, 6, 7];
        let element = |x: &i32, visit: &mut ShrinkVisitor<'_, i32>| {
            shrink_int(*x, i128::from(i32::MIN), i128::from(i32::MAX), visit)
        };
        let candidates = collect_candidates(|visit| shrink_seq(&v, 0, element, visit));
        assert_eq!(candidates[0], vec![7], "largest block first");
        assert_eq!(candidates[1], vec![5]);
        assert!(
            candidates.contains(&vec![6, 7])
                && candidates.contains(&vec![5, 7])
                && candidates.contains(&vec![5, 6])
        );
        assert!(candidates.contains(&vec![0, 6, 7]));
        assert!(candidates.iter().all(|c| c.len() <= 3));
        let respects_min = collect_candidates(|visit| shrink_seq(&v, 3, element, visit));
        assert!(respects_min.iter().all(|c| c.len() == 3));
    }

    #[test]
    fn mutations_stay_in_range_and_change_something() {
        let mut g = Source::new(9, 10);
        for _ in 0..500 {
            let m = mutate_int(20i32, 10, 30, &mut g);
            assert!((10..=30).contains(&m));
        }
        let changed = (0..200)
            .filter(|_| mutate_int(5i32, i128::from(i32::MIN), i128::from(i32::MAX), &mut g) != 5)
            .count();
        assert!(changed > 150, "{changed}");
    }
}
