//! Sampling terms from an e-graph, and keeping a varied few.
//!
//! `count` tabulates how many terms of each size every e-class holds, which
//! is finite per size even though the e-graph has cycles. `draw` then picks
//! a node in proportion to the terms it leads to and splits the size among
//! its children the same way, so a term of a given size is drawn uniformly.
//! Sizes themselves are picked uniformly, so small and large variants both
//! appear.

use std::collections::{BTreeSet, HashMap};

use egg::{EGraph, Id, Language, RecExpr, Symbol};

use crate::lang::{Facts, Sql};

/// SplitMix64: enough randomness for a showcase, and reproducible.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// An index chosen in proportion to `weights`; `None` when all are zero.
    fn weighted(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut target = self.unit() * total;
        for (i, w) in weights.iter().enumerate() {
            if target < *w {
                return Some(i);
            }
            target -= w;
        }
        weights.iter().rposition(|w| *w > 0.0)
    }
}

pub struct Counts {
    pub max: usize,
    by_class: HashMap<Id, Vec<f64>>,
    /// Symbols the sampler may not use: TRUE and FALSE when the seed has
    /// neither, since `p`, `p AND TRUE` and `p AND TRUE AND TRUE` share a
    /// class and would otherwise fill the samples with padding.
    banned: Vec<Symbol>,
}

impl Counts {
    fn allowed(&self, node: &Sql) -> bool {
        !matches!(node, Sql::Symbol(s) if self.banned.contains(s))
    }

    pub fn of(&self, class: Id) -> &[f64] {
        &self.by_class[&class]
    }

    /// Ways for `children`, in order, to be terms whose sizes sum to `budget`.
    fn ways(&self, eg: &EGraph<Sql, Facts>, children: &[Id], budget: usize) -> f64 {
        match children {
            [] => (budget == 0) as u8 as f64,
            [a] => self.of(eg.find(*a)).get(budget).copied().unwrap_or(0.0),
            [a, b] => {
                let (a, b) = (self.of(eg.find(*a)), self.of(eg.find(*b)));
                (1..budget).map(|k| a[k] * b[budget - k]).sum()
            },
            [first, rest @ ..] => {
                let first = self.of(eg.find(*first));
                (1..budget).map(|k| first[k] * self.ways(eg, rest, budget - k)).sum()
            },
        }
    }
}

/// How many terms of each size up to `max` every e-class holds, without the
/// `banned` symbols.
pub fn count(eg: &EGraph<Sql, Facts>, max: usize, banned: Vec<Symbol>) -> Counts {
    let classes: Vec<Id> = eg.classes().map(|c| c.id).collect();
    let by_class = classes.iter().map(|&c| (c, vec![0.0; max + 1])).collect();
    let mut counts = Counts { max, by_class, banned };
    // A term of size s has children of sizes below s, so one pass per size
    // in increasing order sees everything it needs.
    for size in 1..=max {
        for &class in &classes {
            let total: f64 = (eg[class].nodes.iter())
                .filter(|n| counts.allowed(n))
                .map(|n| counts.ways(eg, n.children(), size - 1))
                .sum();
            counts.by_class.get_mut(&class).unwrap()[size] = total;
        }
    }
    counts
}

/// A term of the root's class, of a size drawn uniformly from those it has.
pub fn draw(eg: &EGraph<Sql, Facts>, counts: &Counts, root: Id, rng: &mut Rng) -> Option<RecExpr<Sql>> {
    let root = eg.find(root);
    let sizes: Vec<usize> = (1..=counts.max).filter(|&s| counts.of(root)[s] > 0.0).collect();
    let size = sizes[(rng.next_u64() % sizes.len() as u64) as usize];
    let mut out = RecExpr::default();
    draw_into(eg, counts, root, size, rng, &mut out)?;
    Some(out)
}

fn draw_into(
    eg: &EGraph<Sql, Facts>,
    counts: &Counts,
    class: Id,
    size: usize,
    rng: &mut Rng,
    out: &mut RecExpr<Sql>,
) -> Option<Id> {
    let nodes = &eg[eg.find(class)].nodes;
    let weights: Vec<f64> = (nodes.iter())
        .map(|n| if counts.allowed(n) { counts.ways(eg, n.children(), size - 1) } else { 0.0 })
        .collect();
    let node = &nodes[rng.weighted(&weights)?];
    let children = node.children();
    let mut remaining = size - 1;
    let mut drawn = Vec::with_capacity(children.len());
    for (i, child) in children.iter().enumerate() {
        let rest = &children[i + 1..];
        let row = counts.of(eg.find(*child));
        let weights: Vec<f64> = (0..=remaining)
            .map(|k| if k == 0 { 0.0 } else { row[k] * counts.ways(eg, rest, remaining - k) })
            .collect();
        let k = rng.weighted(&weights)?;
        drawn.push(draw_into(eg, counts, *child, k, rng, out)?);
        remaining -= k;
    }
    let mut drawn = drawn.into_iter();
    Some(out.add(node.clone().map_children(|_| drawn.next().unwrap())))
}

/// The structural features of a term: every (parent, child) operator pair,
/// the order tables are joined in, and its size.
pub fn fingerprint(term: &RecExpr<Sql>) -> BTreeSet<String> {
    let nodes = term.as_ref();
    let op = |n: &Sql| match n {
        Sql::Symbol(_) => "leaf".to_string(),
        other => other.to_string(),
    };
    let mut features: BTreeSet<String> = BTreeSet::new();
    let mut tables = Vec::new();
    for n in nodes {
        for &c in n.children() {
            features.insert(format!("{}>{}", op(n), op(&nodes[usize::from(c)])));
        }
        if let Sql::Scan([_, alias]) = n {
            tables.push(nodes[usize::from(*alias)].to_string());
        }
    }
    features.insert(format!("order:{}", tables.join(",")));
    features.insert(format!("size:{}", nodes.len() / 4));
    features
}

fn distance(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let shared = a.intersection(b).count() as f64;
    1.0 - shared / (a.len() + b.len()).saturating_sub(shared as usize).max(1) as f64
}

/// `n` of `candidates`, each as far as possible from the seed and from those
/// already kept (greedy farthest-point selection).
pub fn spread(seed: &BTreeSet<String>, candidates: &[BTreeSet<String>], n: usize) -> Vec<usize> {
    let mut kept: Vec<usize> = Vec::new();
    let mut nearest: Vec<f64> = candidates.iter().map(|c| distance(seed, c)).collect();
    while kept.len() < n.min(candidates.len()) {
        let best = (0..candidates.len())
            .filter(|i| !kept.contains(i))
            .max_by(|&a, &b| nearest[a].total_cmp(&nearest[b]))
            .unwrap();
        kept.push(best);
        for (i, c) in candidates.iter().enumerate() {
            nearest[i] = nearest[i].min(distance(&candidates[best], c));
        }
    }
    kept
}

pub fn mean_distance(sets: &[&BTreeSet<String>]) -> f64 {
    let mut total = 0.0;
    let mut pairs = 0;
    for i in 0..sets.len() {
        for j in i + 1..sets.len() {
            total += distance(sets[i], sets[j]);
            pairs += 1;
        }
    }
    if pairs == 0 { 0.0 } else { total / pairs as f64 }
}
