//! Equivalent variants of a seed query: the e-graph saturated with
//! [`crate::lang::rules`] from the seed's term, terms drawn uniformly by
//! size ([`crate::sample`]), decoded, kept when valid on the schema
//! (ephyra-postgresql's own type checker) and distinct as SQL, and the most
//! structurally different few of those.

use std::time::Duration;

use egg::{BackoffScheduler, Runner};
use ephyra_postgresql::expr::{Query, Schema};

use crate::{
    lang::{decode, encode, rules},
    sample::{self, Rng},
};

/// What drawing variants of one seed did.
#[derive(Debug, Default)]
pub struct Report {
    pub classes: usize,
    pub nodes: usize,
    pub drawn: usize,
    /// Drawn terms with no AST form (a rule bug).
    pub undecodable: Vec<String>,
    /// Decoded queries the type checker refused (a rule that changes a
    /// type, or one that needs a type it cannot see).
    pub invalid: Vec<String>,
}

/// Up to `n` variants of `seed` (never the seed itself), from `samples`
/// draws seeded by `rng_seed`.
pub fn variants(seed: &Query, schema: &Schema, n: usize, samples: usize, rng_seed: u64) -> (Vec<Query>, Report) {
    let enc = encode(seed);
    let runner: Runner<crate::lang::Sql, crate::lang::Types> = Runner::default()
        .with_expr(&enc.term)
        .with_node_limit(20_000)
        .with_iter_limit(10)
        .with_time_limit(Duration::from_secs(3))
        .with_scheduler(BackoffScheduler::default())
        .run(&rules());
    let (eg, root) = (&runner.egraph, runner.roots[0]);
    // Each slot is drawn from its own e-class, of a size up to half again
    // its seed's, and the root assembled from them.
    let crate::lang::Sql::Q(slots) = enc.term.as_ref().last().unwrap().clone() else { unreachable!() };
    let slot_size = |id: egg::Id| subterm_size(&enc.term, id);
    let max = slots.iter().map(|&s| slot_size(s)).max().unwrap_or(1);
    let counts = sample::count(eg, max + max / 2 + 4, Vec::new());
    let slot_classes: Vec<egg::Id> = slots.iter().map(|&s| eg.lookup_expr(&extract(&enc.term, s)).expect("the slot is in the e-graph")).collect();
    let _ = root;
    let mut report = Report { classes: eg.number_of_classes(), nodes: eg.total_number_of_nodes(), ..Report::default() };
    let seed_sql = seed.sql();
    let mut rng = Rng::new(rng_seed);
    let mut kept: Vec<(String, Query, std::collections::BTreeSet<String>)> = Vec::new();
    for _ in 0..samples {
        // Most slots keep their seed term so a variant differs in a few
        // places; each is redrawn with probability 1/2.
        let mut term = egg::RecExpr::default();
        let mut ids = Vec::new();
        let mut ok = true;
        for (k, &class) in slot_classes.iter().enumerate() {
            let sub = if rng.next_u64() % 2 == 0 {
                extract(&enc.term, slots[k])
            } else {
                match sample::draw_upto(eg, &counts, class, slot_size(slots[k]) + slot_size(slots[k]) / 2 + 4, &mut rng) {
                    Some(t) => t,
                    None => {
                        ok = false;
                        break;
                    },
                }
            };
            ids.push(append(&mut term, &sub));
        }
        if !ok {
            continue;
        }
        term.add(crate::lang::Sql::Q(ids.into()));
        report.drawn += 1;
        match decode(&term, &enc) {
            Err(e) => report.undecodable.push(e),
            Ok(q) if !q.valid(schema) => report.invalid.push(q.sql()),
            Ok(q) => {
                let sql = q.sql();
                if sql != seed_sql && !kept.iter().any(|k| k.0 == sql) {
                    kept.push((sql, q, sample::fingerprint(&term)));
                }
            },
        }
    }
    let fingerprints: Vec<_> = kept.iter().map(|k| k.2.clone()).collect();
    let chosen = sample::spread(&sample::fingerprint(&enc.term), &fingerprints, n);
    (chosen.into_iter().map(|i| kept[i].1.clone()).collect(), report)
}

/// The subterm of `term` rooted at `id`, as its own expression.
fn extract(term: &egg::RecExpr<crate::lang::Sql>, id: egg::Id) -> egg::RecExpr<crate::lang::Sql> {
    let mut out = egg::RecExpr::default();
    fn go(t: &egg::RecExpr<crate::lang::Sql>, id: egg::Id, out: &mut egg::RecExpr<crate::lang::Sql>) -> egg::Id {
        use egg::Language;
        let n = t[id].clone().map_children(|c| go(t, c, out));
        out.add(n)
    }
    go(term, id, &mut out);
    out
}

fn subterm_size(term: &egg::RecExpr<crate::lang::Sql>, id: egg::Id) -> usize {
    use egg::Language;
    1 + term[id].children().iter().map(|&c| subterm_size(term, c)).sum::<usize>()
}

/// Append `sub` to `term`, returning its root's id there.
fn append(term: &mut egg::RecExpr<crate::lang::Sql>, sub: &egg::RecExpr<crate::lang::Sql>) -> egg::Id {
    use egg::Language;
    let mut map = Vec::with_capacity(sub.as_ref().len());
    for n in sub.as_ref() {
        let n = n.clone().map_children(|c| map[usize::from(c)]);
        map.push(term.add(n));
    }
    *map.last().unwrap()
}
