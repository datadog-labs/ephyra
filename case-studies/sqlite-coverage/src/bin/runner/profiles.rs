//! Loop profiles: how a worker tests, the generator (`case_queries`) the
//! same for all. A profile ([`Profile`], the table [`PROFILES`]) says which
//! inputs are mutants and of what, which pool of seeds they come from, how
//! many queries a fresh input's database gets, which oracles run on an
//! input, and how often its SQLite counters are read.
//!
//! A worker's loop ([`Loop`]) is a profile on a storage ([`StorageKind`]):
//! `NAME` keeps the case's database in memory, as every oracle did before;
//! `NAME+wal` (in `--profiles`, `gen+wal:1`) runs the same loop with the
//! database in a WAL file (ephyra-sqlite's `sqlancer::Storage::Wal`: the
//! pager's file paths, `wal.c` and `os_unix.c`, which an in-memory database
//! never runs). Inputs are the same; a memory loop's oracles rotate over the
//! first [`MEMORY_ORACLES`] of [`ORACLES`] (`n % 7`, or all seven), a WAL
//! loop's over all eight, the storage oracle (memory against a WAL file)
//! the eighth. A loop's name, with its `+wal`, is its `profile` tag.
//!
//! Input `n` is a mutant when `n * mutants mod 10 < mutants` (`mutants` in
//! tenths: every other input at 5, nine in ten at 9), and its pool has a
//! seed; else it is a fresh draw from seed `s` at size `n % 101`, extended
//! to the profile's or the worker's query range ([`crate::worker`]), and
//! rebuilt from its seed by `runner replay`. A mutant ([`draw`]) of a seed
//! the pool picks is, `recase` tenths of the time, the seed's queries on a
//! fresh case (`queries_on`); else, `splice` tenths of the time, the seed's
//! case with the first queries of a second seed and the last of a third,
//! repaired to its schema; then a stack of `stack` mutations (log-uniform
//! from `lo` to `hi`, from 1 to `hi + 1` less one when `lo` is 0) on the
//! `target`: both (`case_queries`' own mutation), the case (`Cases`'
//! mutation, the queries repaired by `queries_on`), or the queries (`Queries`'
//! mutation on the case's final schema, the case kept); `Any` picks one of
//! the three per mutant.
//!
//! The pools ([`Seeds`]), each a worker's own: `Rare` ([`Pool`]) keeps
//! inputs by the counter, rare against `D/reference.tsv` or new, that they
//! are among the first to reach, and picks the least reached of 4 such
//! counters; `Novel` ([`Novel`]) keeps any input that reaches a counter its
//! worker had not, and picks by energy: 1 for a new seed, times 0.9 each
//! time it is picked, plus 1 when a mutant of it reaches a new counter;
//! `NearMiss` ([`NearMiss`]) keeps, as a ring, inputs whose oracles skipped
//! comparisons ([`near_miss`]) and the shrunk inputs of failures, and picks
//! one at random. Seed counts and file sizes are capped ([`MAX_SEEDS`],
//! [`MAX_SEED_BYTES`]).

use std::collections::HashMap;

use ephyra::{Gen, Source};
use ephyra_sqlite::{
    cases::Cases,
    expr::{Queries, Query},
    probes::{case_queries, final_schema},
    sqlancer::Tally,
};
use sqlite_coverage::{mix, queries_on};

use crate::{worker::extend, Input, MEMORY_ORACLES, ORACLES};

/// Inputs kept per target counter (`Rare`), seeds in all, and the largest
/// file kept.
const PER_TARGET: usize = 8;
pub const MAX_SEEDS: usize = 3000;
pub const MAX_SEED_BYTES: usize = 2 << 20;

/// A sampling profile reads the counters of one input in this many.
pub const COVERAGE_SAMPLE: u64 = 50;

/// What a mutation changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Both,
    Case,
    Queries,
    /// One of the three, a third of the time each.
    Any,
}

/// Where a profile's mutants come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoolKind {
    None,
    Rare,
    Novel,
    NearMiss,
}

/// Which oracles run on input `n`: `n % 7` (`Rotate`), or every one (of a
/// WAL loop: `n % 8`, or all eight; see [`Loop::oracles`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Oracles {
    Rotate,
    All,
}

/// A worker's loop settings (see the module documentation).
#[derive(Debug)]
pub struct Profile {
    pub name: &'static str,
    pub pool: PoolKind,
    /// Tenths of the inputs that are mutants.
    pub mutants: u64,
    /// Tenths of the mutants that are a seed's queries on a fresh case.
    pub recase: u64,
    /// Tenths of the other mutants that are splices of three seeds.
    pub splice: u64,
    /// Mutations stacked on a mutant, `lo` to `hi`.
    pub stack: (u32, u32),
    pub target: Target,
    /// The query range of fresh inputs, over the worker's `--queries`.
    pub queries: Option<(usize, usize)>,
    pub oracles: Oracles,
    /// One input in this many has its counters read.
    pub sample: u64,
}

const GEN: Profile = Profile {
    name: "gen",
    pool: PoolKind::None,
    mutants: 0,
    recase: 0,
    splice: 0,
    stack: (0, 0),
    target: Target::Both,
    queries: None,
    oracles: Oracles::Rotate,
    sample: COVERAGE_SAMPLE,
};

/// The profiles. `gen` is the fresh mode of old and `rare` the targeted one.
/// `havoc` and `splice` are out of the default allocation since 2026-10-11
/// (every profile reached at least 99.8% of the union of their counters, so
/// coverage gave them no case), but stay selectable.
pub const PROFILES: [Profile; 7] = [
    GEN,
    Profile { name: "gen-deep", queries: Some((300, 1000)), oracles: Oracles::All, sample: 1, ..GEN },
    Profile { name: "rare", pool: PoolKind::Rare, mutants: 5, recase: 7, stack: (4, 4), sample: 1, ..GEN },
    Profile { name: "novel", pool: PoolKind::Novel, mutants: 9, stack: (1, 4), sample: 1, ..GEN },
    Profile { name: "havoc", pool: PoolKind::Novel, mutants: 9, stack: (1, 32), target: Target::Any, sample: 1, ..GEN },
    Profile { name: "splice", pool: PoolKind::Novel, mutants: 9, splice: 10, stack: (0, 4), sample: 1, ..GEN },
    Profile { name: "near-miss", pool: PoolKind::NearMiss, mutants: 7, stack: (1, 8), sample: 1, ..GEN },
];

impl Profile {
    /// The mode of old it is or stands nearest: `fresh` without a pool.
    pub fn mode(&self) -> &'static str {
        if self.pool == PoolKind::None {
            "fresh"
        } else {
            "targeted"
        }
    }

    /// Whether input `n` is to be a mutant.
    pub fn is_mutant(&self, n: u64) -> bool {
        self.pool != PoolKind::None && n.wrapping_mul(self.mutants) % 10 < self.mutants
    }

    /// The oracles (indexes into [`ORACLES`]) that run on input `n` of a
    /// memory loop: of the first [`MEMORY_ORACLES`].
    pub fn oracles(&self, n: u64) -> Vec<usize> {
        self.oracles_of(n, MEMORY_ORACLES)
    }

    fn oracles_of(&self, n: u64, k: usize) -> Vec<usize> {
        match self.oracles {
            Oracles::Rotate => vec![(n % k as u64) as usize],
            Oracles::All => (0..k).collect(),
        }
    }
}

/// Where a loop's oracles keep the case's database.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StorageKind {
    Memory,
    /// A WAL file in the worker's directory (`NAME+wal`).
    Wal,
}

impl StorageKind {
    /// Its `storage` tag and state-file value: `memory` or `wal`.
    pub fn tag(self) -> &'static str {
        match self {
            StorageKind::Memory => "memory",
            StorageKind::Wal => "wal",
        }
    }
}

/// A worker's loop: a profile on a storage, named `NAME` or `NAME+wal`.
#[derive(Clone, Debug)]
pub struct Loop {
    pub profile: &'static Profile,
    pub storage: StorageKind,
    pub name: String,
}

impl Loop {
    pub fn mode(&self) -> &'static str {
        self.profile.mode()
    }

    /// The oracles that run on input `n`: the profile's, of the first
    /// [`MEMORY_ORACLES`] in memory, of all [`ORACLES`] (storage too) in a
    /// WAL file.
    pub fn oracles(&self, n: u64) -> Vec<usize> {
        match self.storage {
            StorageKind::Memory => self.profile.oracles(n),
            StorageKind::Wal => self.profile.oracles_of(n, ORACLES.len()),
        }
    }
}

/// The loop named `name`: a profile ([`by_name`]), `+wal` for WAL storage.
pub fn lookup(name: &str) -> Result<Loop, String> {
    let (base, storage) = match name.split_once('+') {
        None => (name, StorageKind::Memory),
        Some((base, "wal")) => (base, StorageKind::Wal),
        Some((_, other)) => return Err(format!("unknown storage `+{other}` in `{name}` (known: +wal)")),
    };
    let profile = by_name(base)?;
    let name = match storage {
        StorageKind::Memory => profile.name.to_string(),
        StorageKind::Wal => format!("{}+wal", profile.name),
    };
    Ok(Loop { profile, storage, name })
}

/// The profile named `name`, or `fresh` and `targeted`, the modes of old.
pub fn by_name(name: &str) -> Result<&'static Profile, String> {
    let name = match name {
        "fresh" => "gen",
        "targeted" => "rare",
        n => n,
    };
    PROFILES.iter().find(|p| p.name == name).ok_or_else(|| {
        let known: Vec<&str> = PROFILES.iter().map(|p| p.name).collect();
        format!("unknown profile `{name}` (known: {}; fresh and targeted stand for gen and rare)", known.join(", "))
    })
}

/// The loop of each worker, in order, from `NAME[+wal]:COUNT,..`.
pub fn parse(spec: &str) -> Result<Vec<Loop>, String> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, count) = part.split_once(':').ok_or_else(|| format!("`{part}` in --profiles is not NAME:COUNT"))?;
        let count: usize = count.parse().map_err(|_| format!("`{part}` in --profiles: `{count}` is not a count"))?;
        let l = lookup(name)?;
        out.extend(std::iter::repeat_n(l, count));
    }
    if out.is_empty() {
        return Err(format!("--profiles `{spec}` gives no workers"));
    }
    Ok(out)
}

/// Whether an oracle skipped comparisons on an input near a difference:
/// both sides failed, or one passed a limit, overflowed, had JSON refused,
/// summed reals or grouped ambiguously. Not `known_skips` (known SQLite
/// behaviour: about 7 an input at 1-300 queries, which would make every
/// input a near miss), `hint_skips`, `nondeterministic_skips` (a case
/// SQLite refuses for a date function reading the current time in an
/// index: no comparison was near anything, and a pool keeping it would
/// draw mutants of it, as run 20261012-0317's rare worker did) or
/// `inapplicable`.
pub fn near_miss(t: &Tally) -> bool {
    t.both_failed + t.overflow_skips + t.limit_skips + t.json_skips + t.real_sum_skips + t.group_skips > 0
}

/// The `Rare` pool: inputs by the rare or new counter they first reached,
/// and how often the worker has reached each.
pub struct Pool {
    count: HashMap<usize, u32>,
    limit: u32,
    reached: HashMap<usize, u64>,
    seeds: Vec<Input>,
    targets: HashMap<usize, Vec<usize>>,
    list: Vec<usize>,
    /// Rare and new counters reached.
    pub rare: u64,
    pub new: u64,
}

impl Pool {
    /// A pool against a reference of `draws` draws that reached each
    /// counter `count` times.
    pub fn new(count: HashMap<usize, u32>, draws: u64) -> Pool {
        Pool {
            count,
            limit: (draws / 1000).max(1) as u32,
            reached: HashMap::new(),
            seeds: Vec::new(),
            targets: HashMap::new(),
            list: Vec::new(),
            rare: 0,
            new: 0,
        }
    }

    pub fn load(dir: &std::path::Path) -> Pool {
        let text = std::fs::read_to_string(dir.join("reference.tsv")).expect("the reference (runner reference)");
        let count: HashMap<usize, u32> = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .filter_map(|(c, k)| Some((c.parse().ok()?, k.parse().ok()?)))
            .collect();
        let draws: u64 =
            text.lines().next().and_then(|l| l.trim_start_matches("# ").split(' ').next()?.parse().ok()).unwrap_or(crate::worker::REFERENCE);
        Pool::new(count, draws)
    }

    /// A seed for the least reached of 4 targets.
    fn pick(&self, s: u64) -> Option<usize> {
        if self.list.is_empty() {
            return None;
        }
        let c = (0..4).map(|k| self.list[(mix(s, k) % self.list.len() as u64) as usize]).min_by_key(|c| self.reached[c])?;
        let parents = &self.targets[&c];
        Some(parents[(mix(s, 9) % parents.len() as u64) as usize])
    }

    /// Count what `input` reached; keep it as a seed for the targets it is
    /// among the first to reach.
    pub fn observe(&mut self, input: Input, hits: &[(usize, u64)]) {
        let mut keep_for = Vec::new();
        for &(c, _) in hits {
            if self.count.get(&c).is_some_and(|&k| k > self.limit) {
                continue;
            }
            let n = self.reached.entry(c).or_default();
            if *n == 0 {
                if self.count.contains_key(&c) {
                    self.rare += 1;
                } else {
                    self.new += 1;
                }
            }
            *n += 1;
            if self.targets.get(&c).is_none_or(|ps| ps.len() < PER_TARGET) {
                keep_for.push(c);
            }
        }
        if keep_for.is_empty() || self.seeds.len() >= MAX_SEEDS || input.0.file.0.len() > MAX_SEED_BYTES {
            return;
        }
        let at = self.seeds.len();
        self.seeds.push(input);
        for c in keep_for {
            let ps = self.targets.entry(c).or_default();
            if ps.is_empty() {
                self.list.push(c);
            }
            ps.push(at);
        }
    }
}

/// The `Novel` pool: inputs that reached a counter first, with their energy.
#[derive(Default)]
pub struct Novel {
    seeds: Vec<(Input, f64)>,
}

impl Novel {
    /// A seed, drawn with chances in proportion to energy.
    fn pick(&self, r: u64) -> Option<usize> {
        let total: f64 = self.seeds.iter().map(|s| s.1).sum();
        if self.seeds.is_empty() || total <= 0.0 {
            return None;
        }
        let mut u = (r % 1_000_000) as f64 / 1_000_000.0 * total;
        for (i, s) in self.seeds.iter().enumerate() {
            u -= s.1;
            if u < 0.0 {
                return Some(i);
            }
        }
        Some(self.seeds.len() - 1)
    }

    /// Charge the parents of an input that reached `new` counters first
    /// (credit them when it did), and keep it when it did: in place of the
    /// seed of least energy once the pool is full.
    pub fn observe(&mut self, input: Input, new: usize, parents: &[usize]) {
        for &p in parents {
            let e = &mut self.seeds[p].1;
            *e = (*e * 0.9 + if new > 0 { 1.0 } else { 0.0 }).clamp(0.01, 16.0);
        }
        if new == 0 || input.0.file.0.len() > MAX_SEED_BYTES {
            return;
        }
        if self.seeds.len() < MAX_SEEDS {
            self.seeds.push((input, 1.0));
        } else if let Some(i) = (0..self.seeds.len()).min_by(|&a, &b| self.seeds[a].1.total_cmp(&self.seeds[b].1)) {
            self.seeds[i] = (input, 1.0);
        }
    }
}

/// The `NearMiss` pool: a ring of near misses and shrunk failures.
#[derive(Default)]
pub struct NearMiss {
    seeds: Vec<Input>,
    next: usize,
}

impl NearMiss {
    pub fn keep(&mut self, input: Input) {
        if input.0.file.0.len() > MAX_SEED_BYTES {
            return;
        }
        if self.seeds.len() < MAX_SEEDS {
            self.seeds.push(input);
        } else {
            self.seeds[self.next] = input;
            self.next = (self.next + 1) % MAX_SEEDS;
        }
    }
}

/// A worker's pool.
pub enum Seeds {
    Rare(Pool),
    Novel(Novel),
    NearMiss(NearMiss),
}

impl Seeds {
    /// An empty pool of `kind` (`Rare` loads the reference from `dir`).
    pub fn new(kind: PoolKind, dir: &std::path::Path) -> Option<Seeds> {
        match kind {
            PoolKind::None => None,
            PoolKind::Rare => Some(Seeds::Rare(Pool::load(dir))),
            PoolKind::Novel => Some(Seeds::Novel(Novel::default())),
            PoolKind::NearMiss => Some(Seeds::NearMiss(NearMiss::default())),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Seeds::Rare(p) => p.seeds.len(),
            Seeds::Novel(p) => p.seeds.len(),
            Seeds::NearMiss(p) => p.seeds.len(),
        }
    }

    fn get(&self, i: usize) -> &Input {
        match self {
            Seeds::Rare(p) => &p.seeds[i],
            Seeds::Novel(p) => &p.seeds[i].0,
            Seeds::NearMiss(p) => &p.seeds[i],
        }
    }

    /// The `k`th seed picked for input seed `s`.
    fn pick(&self, s: u64, k: u64) -> Option<usize> {
        match self {
            Seeds::Rare(p) => (k == 0).then(|| p.pick(s)).flatten(),
            Seeds::Novel(p) => p.pick(mix(s, 0x5eed + k)),
            Seeds::NearMiss(p) => (!p.seeds.is_empty()).then(|| (mix(s, 0x5eed + k) % p.seeds.len() as u64) as usize),
        }
    }
}

/// `lo` to `hi`, log-uniformly by `r` (from 1 to `hi + 1`, less one, when
/// `lo` is 0).
fn stack(lo: u32, hi: u32, r: u64) -> u32 {
    if lo >= hi {
        return lo;
    }
    let shift = u32::from(lo == 0);
    let (a, b) = (f64::from(lo + shift).ln(), f64::from(hi + shift).ln());
    let u = (r % 1_000_000) as f64 / 1_000_000.0;
    ((a + u * (b - a)).exp().round() as u32).clamp(lo + shift, hi + shift) - shift
}

/// Input `n` of profile `p`, from input seed `s` at `size`: a mutant of the
/// pool's seeds when `n` is to be one and the pool has a seed, else a fresh
/// draw extended to the profile's query range or else `range`. The seeds a
/// mutant came from (none for a fresh draw).
pub fn draw(p: &Profile, seeds: Option<&Seeds>, s: u64, size: usize, n: u64, range: Option<(usize, usize)>) -> (Input, Vec<usize>) {
    if p.is_mutant(n) {
        if let Some(m) = seeds.and_then(|seeds| mutant(p, seeds, s, size)) {
            return m;
        }
    }
    let mut v = case_queries().generate(&mut Source::new(s, size));
    if let Some(r) = p.queries.or(range) {
        extend(&mut v, s, size, r);
    }
    (v, Vec::new())
}

fn mutant(p: &Profile, seeds: &Seeds, s: u64, size: usize) -> Option<(Input, Vec<usize>)> {
    let gen = case_queries();
    let first = seeds.pick(s, 0)?;
    let parent = seeds.get(first);
    let mut g = Source::new(mix(s, 10), size);
    if mix(s, 11) % 10 < p.recase {
        return Some((queries_on(Cases.generate(&mut Source::new(mix(s, 12), size)), &parent.1, &mut g), vec![first]));
    }
    let mut parents = vec![first];
    let mut v = if mix(s, 13) % 10 < p.splice {
        let (a, b) = (seeds.pick(s, 1)?, seeds.pick(s, 2)?);
        parents.extend([a, b]);
        let (qa, qb) = (&seeds.get(a).1, &seeds.get(b).1);
        let (i, j) = ((mix(s, 14) % (qa.len() as u64 + 1)) as usize, (mix(s, 15) % (qb.len() as u64 + 1)) as usize);
        let mixed: Vec<Query> = qa[..i].iter().chain(&qb[j..]).cloned().collect();
        queries_on(parent.0.clone(), &mixed, &mut g)
    } else {
        parent.clone()
    };
    let k = stack(p.stack.0, p.stack.1, mix(s, 16));
    let target = match p.target {
        Target::Any => [Target::Both, Target::Case, Target::Queries][(mix(s, 17) % 3) as usize],
        t => t,
    };
    match target {
        Target::Queries if k > 0 => {
            let qs = Queries(final_schema(&v.0));
            for _ in 0..k {
                v.1 = qs.mutate(&v.1, &mut g);
            }
        },
        Target::Case => {
            for _ in 0..k {
                let c = Cases.mutate(&v.0, &mut g);
                v = queries_on(c, &v.1, &mut g);
            }
        },
        _ => {
            for _ in 0..k {
                v = gen.mutate(&v, &mut g);
            }
        },
    }
    Some((v, parents))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ephyra::{Gen, Source};
    use ephyra_sqlite::{probes::case_queries, sqlancer::Tally};
    use sqlite_coverage::mix;

    use super::{by_name, draw, lookup, parse, stack, NearMiss, Novel, Pool, PoolKind, Seeds, StorageKind, PROFILES};
    use crate::worker::extend;

    /// `gen` draws what the fresh mode of old drew, input for input
    /// (crash replay rebuilds them from their seed), under the same oracle.
    #[test]
    fn gen_is_the_old_fresh_mode() {
        let gen = by_name("gen").unwrap();
        for range in [None, Some((1, 300))] {
            for n in 1..200u64 {
                let s = mix(mix(7, 3), n);
                let size = (n % 101) as usize;
                let mut old = case_queries().generate(&mut Source::new(s, size));
                if let Some(r) = range {
                    extend(&mut old, s, size, r);
                }
                let (new, parents) = draw(gen, None, s, size, n, range);
                assert!(parents.is_empty());
                assert_eq!(format!("{new:?}"), format!("{old:?}"), "input {n}");
                assert_eq!(gen.oracles(n), vec![(n % 7) as usize]);
            }
        }
    }

    /// Every profile draws inputs from a small pool, mutants as often as it
    /// says, without a panic.
    #[test]
    fn every_profile_draws() {
        let gen = case_queries();
        let fresh: Vec<_> = (0..12).map(|i| gen.generate(&mut Source::new(mix(1, i), 40))).collect();
        let mut rare = Pool::new(HashMap::from([(1, 1), (2, 1)]), 1000);
        let (mut novel, mut near) = (Novel::default(), NearMiss::default());
        for (i, v) in fresh.iter().enumerate() {
            rare.observe(v.clone(), &[(i % 3, 1), (100 + i, 1)]);
            novel.observe(v.clone(), 1, &[]);
            near.keep(v.clone());
        }
        let pools = [Seeds::Rare(rare), Seeds::Novel(novel), Seeds::NearMiss(near)];
        for p in &PROFILES {
            let seeds = match p.pool {
                PoolKind::None => None,
                PoolKind::Rare => Some(&pools[0]),
                PoolKind::Novel => Some(&pools[1]),
                PoolKind::NearMiss => Some(&pools[2]),
            };
            let inputs = if p.queries.is_some() { 30 } else { 300 };
            let mut mutants = 0;
            for n in 1..=inputs {
                let s = mix(mix(5, 1), n);
                let (_, parents) = draw(p, seeds, s, (n % 101) as usize, n, Some((1, 30)));
                mutants += u64::from(!parents.is_empty());
                assert_eq!(!parents.is_empty(), p.is_mutant(n), "{} input {n}", p.name);
            }
            assert_eq!(mutants, inputs * p.mutants / 10, "{}", p.name);
        }
    }

    #[test]
    fn stacks_stay_in_range() {
        for (lo, hi) in [(0, 4), (1, 32), (4, 4), (1, 8)] {
            let ks: Vec<u32> = (0..2000).map(|i| stack(lo, hi, mix(3, i))).collect();
            assert!(ks.iter().all(|&k| lo <= k && k <= hi));
            assert!(ks.contains(&lo) && ks.contains(&hi), "{lo}-{hi}");
        }
    }

    #[test]
    fn profiles_parse() {
        let ws = parse("gen:4,gen-deep:2,rare:2,novel:3,havoc:2,splice:1,near-miss:1").unwrap();
        assert_eq!(ws.len(), 15);
        assert_eq!(ws[0].name, "gen");
        assert_eq!(ws[4].name, "gen-deep");
        assert_eq!(ws[14].name, "near-miss");
        assert!(ws.iter().all(|w| w.storage == StorageKind::Memory));
        let old: Vec<String> = parse("fresh:2,targeted:1").unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(old, ["gen", "gen", "rare"]);
        let wal = parse("gen:1, gen+wal:1,novel+wal:2,fresh+wal:1").unwrap();
        let names: Vec<&str> = wal.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["gen", "gen+wal", "novel+wal", "novel+wal", "gen+wal"]);
        assert_eq!((wal[1].profile.name, wal[1].storage, wal[1].storage.tag()), ("gen", StorageKind::Wal, "wal"));
        assert_eq!(wal[0].storage.tag(), "memory");
        assert!(parse("gen+disk:1").unwrap_err().contains("unknown storage `+disk`"));
        assert!(parse("fuzzy+wal:1").unwrap_err().contains("unknown profile `fuzzy`"));
        assert_eq!(by_name("targeted").unwrap().mode(), "targeted");
        assert_eq!(by_name("gen-deep").unwrap().mode(), "fresh");
        assert!(parse("gen:1,fuzzy:2").unwrap_err().contains("unknown profile `fuzzy`"));
        assert!(parse("gen").unwrap_err().contains("NAME:COUNT"));
        assert!(parse("gen:x").unwrap_err().contains("not a count"));
        assert!(parse("gen:0").is_err());
    }

    /// The default allocation of deploy/runner.env.example: 15 workers,
    /// two of them WAL loops, no havoc or splice.
    #[test]
    fn the_default_allocation_parses() {
        let env = include_str!("../../../deploy/runner.env.example");
        let line = env.lines().find_map(|l| l.strip_prefix("PROFILES=")).expect("a PROFILES line");
        let ws = parse(line.trim_matches('"')).unwrap();
        assert_eq!(ws.len(), 15);
        assert_eq!(ws.iter().filter(|w| w.storage == StorageKind::Wal).count(), 2);
        assert!(ws.iter().all(|w| !matches!(w.profile.name, "havoc" | "splice")));
    }

    /// A WAL loop's oracles rotate over all eight (storage the eighth),
    /// a memory loop's over the first seven, as before.
    #[test]
    fn wal_loops_add_the_storage_oracle() {
        assert_eq!(crate::ORACLES[crate::MEMORY_ORACLES].0, "storage");
        let (gen, wal) = (lookup("gen").unwrap(), lookup("gen+wal").unwrap());
        for n in 0..40u64 {
            assert_eq!(gen.oracles(n), vec![(n % 7) as usize]);
            assert_eq!(wal.oracles(n), vec![(n % 8) as usize]);
        }
        assert_eq!(lookup("gen-deep").unwrap().oracles(3), (0..7).collect::<Vec<_>>());
        assert_eq!(lookup("gen-deep+wal").unwrap().oracles(3), (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn near_misses_are_skips() {
        assert!(!super::near_miss(&Tally { known_skips: 4, hint_skips: 3, inapplicable: 2, queries: 5, ..Tally::default() }));
        assert!(!super::near_miss(&Tally { nondeterministic_skips: 1, ..Tally::default() }));
        assert!(super::near_miss(&Tally { overflow_skips: 1, ..Tally::default() }));
    }
}
