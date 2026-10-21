//! Cases with queries over their final state: the input of the
//! SQLancer-style oracles (`tests/sqlancer.rs`). A [`Case`] (a setup and
//! statements) and queries ([`crate::expr::Query`]) that refer only to the
//! tables and columns that exist after the statements: the model
//! [`final_model`] gives, through [`crate::semantics::apply`].
//!
//! The generator [`case_queries`] draws with `gens::dependent` (the case,
//! then queries on its final schema) and never runs PostgreSQL.

use std::ops::ControlFlow;

use ephyra::{
    gens::{self, Dependent},
    Gen, Source,
};

use crate::{
    cases::{self, final_model, well_formed, Case, Cases},
    expr::{queries_cost, Queries, Query, Schema},
};

/// The schema of the tables after the case's statements.
pub fn final_schema(case: &Case) -> Schema {
    Schema::of(&final_model(case))
}

/// A case is well formed, and every query is valid on its final schema.
#[ephyra::generator]
pub fn probe_well_formed(v: &(Case, Vec<Query>)) -> bool {
    well_formed(&v.0) && {
        let schema = final_schema(&v.0);
        v.1.iter().all(|q| q.valid(&schema))
    }
}

/// Cases and 1 to 6 queries on their final state (none when no table is
/// left).
#[ephyra::generator(probe_well_formed, metrics = probe_metrics)]
pub fn case_queries() -> impl Gen<Value = (Case, Vec<Query>)> {
    CaseQueries(gens::dependent(Cases, queries_for))
}

fn queries_for(case: &Case) -> Queries {
    Queries(final_schema(case))
}

/// Generation is `gens::dependent(Cases, ..)`; shrinking and mutation are
/// its own, to repair queries rather than redraw them.
pub struct CaseQueries(Dependent<Cases, fn(&Case) -> Queries>);

/// The queries kept against a smaller case's final schema: a query whose
/// table is gone is dropped, a column that is gone becomes NULL. Never
/// costlier ([`Query::repair`]).
pub fn repair_queries(schema: &Schema, qs: &[Query]) -> Vec<Query> {
    qs.iter().filter_map(|q| q.repair(schema)).collect()
}

/// The measure every shrink step strictly decreases, lexicographically:
/// the case's ([`cases::measure`]), then the queries' total cost.
pub fn probe_measure(v: &(Case, Vec<Query>)) -> ((u64, u64), u64) {
    (cases::measure(&v.0), queries_cost(&v.1))
}

impl Gen for CaseQueries {
    type Value = (Case, Vec<Query>);

    fn generate(&self, g: &mut Source) -> Self::Value {
        self.0.generate(g)
    }

    /// First the queries, the case kept (lower query cost); then the case,
    /// by [`Cases`]' own steps (each lower in the case's measure, every
    /// candidate well formed), with the queries repaired against the
    /// smaller case's final schema, which never raises their cost. So every
    /// candidate is lower in [`probe_measure`], a lexicographic order on
    /// natural numbers, and every chain ends.
    fn shrink(&self, value: &Self::Value, visit: &mut dyn FnMut(Self::Value) -> ControlFlow<()>) -> ControlFlow<()> {
        let (case, qs) = value;
        queries_for(case).shrink(qs, &mut |q| visit((case.clone(), q)))?;
        let here = cases::measure(case);
        Cases.shrink(case, &mut |c| {
            if cases::measure(&c) >= here {
                return ControlFlow::Continue(());
            }
            let q = repair_queries(&final_schema(&c), qs);
            visit((c, q))
        })
    }

    /// Half the time the queries change ([`Queries`]' mutation); else the
    /// case changes ([`Cases`]' mutation) and the queries are repaired
    /// against its final schema, or drawn anew when none is left.
    fn mutate(&self, value: &Self::Value, g: &mut Source) -> Self::Value {
        let (case, qs) = value;
        if g.bool() {
            return (case.clone(), queries_for(case).mutate(qs, g));
        }
        let c = Cases.mutate(case, g);
        let gen = queries_for(&c);
        let q = repair_queries(&gen.0, qs);
        let q = if q.is_empty() {
            crate::fell_back(&crate::QUERY_FALLBACKS);
            gen.generate(g)
        } else {
            q
        };
        (c, q)
    }
}

/// Numbers about a case and its queries (no PostgreSQL):
///
/// - queries; joins, subqueries, aggregate queries, grouped queries,
///   compound queries, CTE queries, ordered queries: how many queries have
///   each; max sources: the most sources of one SELECT; nodes: expression
///   nodes over all queries.
/// - statements: the case's statements; final tables: tables after them;
///   final rows: rows in the tables after them.
pub fn probe_metrics(v: &(Case, Vec<Query>), _size: usize) -> Vec<(&'static str, f64)> {
    let (case, qs) = v;
    let m = final_model(case);
    let count = |f: &dyn Fn(&Query) -> bool| qs.iter().filter(|q| f(q)).count() as f64;
    let mut nodes = 0usize;
    let mut subqueries = 0usize;
    for q in qs {
        for s in q.selects() {
            for e in s.exprs() {
                e.walk(&mut |x| {
                    nodes += 1;
                    subqueries += x.subqueries().len().min(1);
                });
            }
        }
    }
    vec![
        ("queries", qs.len() as f64),
        ("joins", count(&|q| !q.select.joins.is_empty())),
        ("subqueries", subqueries as f64),
        ("aggregate queries", count(&|q| q.select.aggregate())),
        ("grouped queries", count(&|q| !q.select.group.is_empty())),
        ("compound queries", count(&|q| !q.compound.is_empty())),
        ("cte queries", count(&|q| !q.with.is_empty())),
        ("ordered queries", count(&|q| !q.order.is_empty())),
        ("max sources", qs.iter().map(|q| q.select.sources().len()).max().unwrap_or(0) as f64),
        ("nodes", nodes as f64),
        ("statements", case.statements.len() as f64),
        ("final tables", m.tables.len() as f64),
        ("final rows", m.tables.iter().map(|t| t.rows.len()).sum::<usize>() as f64),
    ]
}
