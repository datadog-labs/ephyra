//! What a case did, recorded from inside a property: features and labels for
//! the report, `cover` requirements for its health warnings, and trace-only
//! values for observation files (`EPHYRA_OBSERVATIONS`, read by Tyche).
//! Generator metrics see only the case; these also see what the engine
//! returned.

use ephyra::{cover, feature, observe};

use crate::ast::{Case, Expr};
use crate::engine::Outcome;

/// Record one case and its outcome.
///
/// - `outcome`: `rows` or `error`.
/// - `result rows`: rows returned.
/// - `selectivity`: rows returned over rows in the table, for a filtered
///   query without `LIMIT` over a nonempty table; 0 keeps nothing, 1 keeps
///   everything.
/// - `filter root`: the filter's top operator, or `none`.
/// - covers: nonempty results, and filters that keep some rows but not all.
pub fn case(case: &Case, outcome: &Outcome) {
    let q = &case.query;
    feature("filter root", q.filter.as_ref().map_or("none", root));
    match outcome {
        Outcome::Error(message) => {
            feature("outcome", "error");
            observe("error", || message.clone());
        },
        Outcome::Rows(rows) => {
            feature("outcome", "rows");
            feature("result rows", rows.len());
            cover(!rows.is_empty(), "nonempty result");
            if q.filter.is_some() && q.limit.is_none() && !case.rows.is_empty() {
                let kept = rows.len() as f64 / case.rows.len() as f64;
                feature("selectivity", kept);
                cover(0.0 < kept && kept < 1.0, "filter keeps some rows, not all");
            }
        },
    }
    observe("sql", || case.to_string());
}

fn root(e: &Expr) -> &'static str {
    match e {
        Expr::Column(_) => "column",
        Expr::Literal(_) => "literal",
        Expr::IsNull(_) => "IS NULL",
        Expr::Prefix(op, _) | Expr::Binary(op, _, _) => op,
    }
}
