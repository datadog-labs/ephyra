//! Entity hierarchy updates: traces of `Entities::add_entities`,
//! `upsert_entities` and `remove_entities` calls on entities with parents and
//! no attributes, and a model of their result.
//!
//! Cedar keeps each entity's transitive closure next to its parents and
//! repairs it incrementally on every update. The model keeps only the
//! declared parents and recomputes reachability, so the two can be compared
//! after every call. Lean never sees this code: the differential tests send
//! Lean the closure Rust computed.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::str::FromStr;

use cedar_policy::{Entities, Entity, EntityUid};
use ephyra::{Gen, GenExt, Source, gens};

/// An entity as a call gives it: its id and its parents' ids.
pub type Decl = (u8, BTreeSet<u8>);

/// One call on an `Entities`.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Add(Vec<Decl>),
    Upsert(Vec<Decl>),
    Remove(Vec<u8>),
}

/// The entity `E::"<id>"`.
pub fn uid(id: u8) -> EntityUid {
    EntityUid::from_str(&format!(r#"E::"{id}""#)).unwrap()
}

fn entities(batch: &[Decl]) -> Vec<Entity> {
    batch
        .iter()
        .map(|(id, parents)| {
            Entity::new_no_attrs(uid(*id), parents.iter().map(|p| uid(*p)).collect())
        })
        .collect()
}

/// Runs one call on Cedar; `None` when it fails.
pub fn apply(es: Entities, op: &Op) -> Option<Entities> {
    match op {
        Op::Add(b) => es.add_entities(entities(b), None).ok(),
        Op::Upsert(b) => es.upsert_entities(entities(b), None).ok(),
        Op::Remove(ids) => es.remove_entities(ids.iter().map(|i| uid(*i))).ok(),
    }
}

/// The stored entities and their declared parents.
pub type Model = BTreeMap<u8, BTreeSet<u8>>;

/// The ids reachable from `from` through declared parents, `from` excluded
/// unless it is on a cycle. Parents need not be stored (dangling).
pub fn reachable(model: &Model, from: u8) -> BTreeSet<u8> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<u8> = model.get(&from).into_iter().flatten().copied().collect();
    while let Some(n) = stack.pop() {
        if seen.insert(n) {
            stack.extend(model.get(&n).into_iter().flatten().copied());
        }
    }
    seen
}

/// The model's result of a call, as documented: the new state, or `None`
/// when the call must fail.
/// - add: an entity already stored, or given twice, must be identical
///   (same parents), else the call fails as a duplicate;
/// - upsert: the last version of each entity wins;
/// - remove: a stored entity goes, with every edge to it;
/// - any call fails if the hierarchy gets a cycle.
pub fn step(model: &Model, op: &Op) -> Option<Model> {
    let mut next = model.clone();
    match op {
        Op::Add(b) => {
            for (id, parents) in b {
                if next.get(id).is_some_and(|stored| stored != parents) {
                    return None;
                }
                next.insert(*id, parents.clone());
            }
        },
        Op::Upsert(b) => {
            for (id, parents) in b {
                next.insert(*id, parents.clone());
            }
        },
        Op::Remove(ids) => {
            for id in ids {
                if next.remove(id).is_some() {
                    for parents in next.values_mut() {
                        parents.remove(id);
                    }
                }
            }
        },
    }
    let cyclic = next.keys().any(|&k| reachable(&next, k).contains(&k));
    if cyclic { None } else { Some(next) }
}

/// Ids are drawn from `0..IDS`: few enough that calls keep meeting the same
/// entities, as updates to one hierarchy do.
pub const IDS: u8 = 8;

fn batch(g: &mut Source) -> Vec<Decl> {
    (0..g.int(1usize..=3))
        .map(|_| {
            let parents = (0..g.int(0usize..=2)).map(|_| g.int(0..IDS)).collect();
            (g.int(0..IDS), parents)
        })
        .collect()
}

fn generate_op(g: &mut Source) -> Op {
    match g.int(0u8..=2) {
        0 => Op::Add(batch(g)),
        1 => Op::Upsert(batch(g)),
        _ => Op::Remove((0..g.int(1usize..=2)).map(|_| g.int(0..IDS)).collect()),
    }
}

/// Smaller calls: an entry or a parent fewer (measure: entries plus parents).
fn shrink_op(op: &Op, visit: &mut dyn FnMut(Op) -> ControlFlow<()>) -> ControlFlow<()> {
    let shrink_batch = |b: &Vec<Decl>, visit: &mut dyn FnMut(Vec<Decl>) -> ControlFlow<()>| {
        for i in 0..b.len() {
            if b.len() > 1 {
                let mut fewer = b.clone();
                fewer.remove(i);
                visit(fewer)?;
            }
            for p in &b[i].1 {
                let mut fewer = b.clone();
                fewer[i].1.remove(p);
                visit(fewer)?;
            }
        }
        ControlFlow::Continue(())
    };
    match op {
        Op::Add(b) => shrink_batch(b, &mut |b| visit(Op::Add(b))),
        Op::Upsert(b) => shrink_batch(b, &mut |b| visit(Op::Upsert(b))),
        Op::Remove(ids) if ids.len() > 1 => {
            for i in 0..ids.len() {
                let mut fewer = ids.clone();
                fewer.remove(i);
                visit(Op::Remove(fewer))?;
            }
            ControlFlow::Continue(())
        },
        Op::Remove(_) => ControlFlow::Continue(()),
    }
}

/// Traces of up to 16 calls. Shrinks by dropping calls, entries and parents;
/// mutates by editing the sequence of calls.
#[ephyra::generator(metrics = trace_metrics)]
pub fn traces() -> impl Gen<Value = Vec<Op>> {
    gens::vec(generate_op.with_shrink(shrink_op)).len(1..=16)
}

/// Over the model's run of a trace:
/// - `calls`: calls in the trace;
/// - `failing calls`: calls the model rejects (duplicate or cycle); high means
///   most of the trace tests error paths;
/// - `upserts naming an entity twice`: upsert batches that give one entity
///   more than once;
/// - `deepest closure`: the largest ancestor set any stored entity reaches
///   after a call; 0 or 1 means the hierarchy stays flat;
/// - `stored at end`: entities stored after the last call.
pub fn trace_metrics(ops: &Vec<Op>, _size: usize) -> Vec<(&'static str, f64)> {
    let mut model = Model::new();
    let (mut failing, mut repeated, mut deepest) = (0, 0, 0);
    for op in ops {
        if let Op::Upsert(b) = op {
            let ids: BTreeSet<u8> = b.iter().map(|(id, _)| *id).collect();
            repeated += usize::from(ids.len() < b.len());
        }
        match step(&model, op) {
            Some(next) => model = next,
            None => failing += 1,
        }
        deepest = model.keys().map(|&k| reachable(&model, k).len()).max().unwrap_or(0).max(deepest);
    }
    vec![
        ("calls", ops.len() as f64),
        ("failing calls", failing as f64),
        ("upserts naming an entity twice", repeated as f64),
        ("deepest closure", deepest as f64),
        ("stored at end", model.len() as f64),
    ]
}
