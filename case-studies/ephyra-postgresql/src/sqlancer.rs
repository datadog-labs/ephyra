// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SQLancer-style metamorphic oracles: PostgreSQL compared with itself on
//! queries ([`crate::expr::Query`]) over the state a [`Case`] leaves. Test
//! support for `tests/sqlancer.rs` and `examples/sqlancer.rs` (which run
//! exactly these functions); the generators never call it.
//!
//! Each oracle loads the case's setup into the thread's fresh schema, runs
//! its statements (as `tests/cases.rs` does), commits a transaction they
//! leave open, then runs its queries. Each returns what it compared
//! ([`Tally`], comparisons per variant) or a description of the
//! disagreement. Below, `F` is the FROM clause as written, `p` the WHERE
//! clause, `K` the GROUP BY terms, `h` HAVING, `I` the items.
//!
//! - [`norec`], Non-optimizing Reference Engine Construction (Rigger and
//!   Su, ESEC/FSE 2020): `SELECT count(*) FROM F WHERE p` against `SELECT
//!   coalesce(sum(CASE WHEN (p) IS TRUE THEN 1 ELSE 0 END), 0) FROM F`, whose
//!   predicate is a value computed for every row, not a filter. Variants
//!   `norec`, `norec-join` (with a join).
//! - [`tlp`], Ternary Logic Partitioning (Rigger and Su, OOPSLA 2020): a
//!   query against the union of its three partitions on a predicate `x`
//!   (`x`, `NOT x`, `x IS NULL`), by the query's shape: `tlp-where` (rows as
//!   a multiset), `tlp-distinct` (as sets), `tlp-agg` (one aggregate:
//!   `count` and `sum` combined by `sum` and cast back, `min`, `max`,
//!   `bool_and`, `bool_or` by themselves; not of DISTINCT), `tlp-group`
//!   (GROUP BY whose items are keys, as sets), `tlp-having` (HAVING
//!   partitioned, rows as a multiset); `tlp-union`, the partitions as one
//!   compound. Inapplicable to other aggregate queries and to queries with
//!   window functions (a partition of WHERE changes the windows'
//!   partitions), counted.
//! - [`plans`], differential query plans: the query as written against the
//!   same query under planner settings that change its plan and never its
//!   result (`plans-<name>`, see [`PLAN_VARIANTS`]): scans, joins and
//!   aggregation methods switched off, parallel plans forced (with and
//!   without parallel hash and append), the join order as written, the
//!   genetic optimizer, 64 kB of work memory (spills), no key reordering.
//! - [`order`], ORDER BY and LIMIT, for queries with an order:
//!   `order-sorted` (the rows in order by every term), `order-multiset`
//!   (the same rows without ORDER BY), `order-limit` (LIMIT and OFFSET
//!   pick the rows the unlimited order puts there).
//! - [`setops`], set operations, for compound queries: `setops-def`, the
//!   compound against its definition computed from its SELECTs' rows;
//!   `setops-swap`, the operands of a commutative operator swapped.
//! - [`ctes`], common table expressions: `cte-materialized` and
//!   `cte-not-materialized`, every CTE with that hint.
//! - [`windows`], window functions: `window-def`, each window item of a
//!   query against its definition computed from the rows it reads (see
//!   [`window_values`]), row by row.
//! - [`generic`], prepared statements: the query with its typed literals
//!   made parameters ([`parameterize`]: `PREPARE p(int4, ..) AS .. $1 ..`,
//!   one parameter per distinct literal, so that a grouped query's items
//!   still match its keys), executed with the literals as arguments under
//!   `plan_cache_mode = force_custom_plan` (`generic-custom`: planned for
//!   the values) and `force_generic_plan` (`generic-generic`: planned once
//!   for any values, so no partial index or constant folding that needs
//!   them).
//! - [`views`], views, the deparser and EXPLAIN: `views-select`, `SELECT *
//!   FROM v` of `CREATE VIEW v(v0, ..) AS q` against q; `views-viewdef-pretty`
//!   and `views-viewdef`, the text of `pg_get_viewdef(v, true|false)` run as
//!   a query against q (the deparser must say what the view means);
//!   `views-fixpoint`, that text made a view again deparses, after at most
//!   two more rounds, to itself (reparsing normalizes some of what the first
//!   text keeps, see `views_of`); `explain-{text,json,xml,yaml}`, `EXPLAIN
//!   (COSTS OFF, VERBOSE, [SETTINGS | MEMORY | GENERIC_PLAN,] FORMAT ..) q`
//!   must succeed (VERBOSE deparses every
//!   expression of the plan); `explain-analyze-text`, `EXPLAIN (ANALYZE,
//!   BUFFERS, WAL, FORMAT TEXT) q` must succeed (not on the parallel
//!   crash's shape below); `explain-analyze-rows`, the top node of
//!   `EXPLAIN (ANALYZE, FORMAT JSON) q` produced q's rows (actual rows times
//!   loops).
//!
//! How results compare: as values PostgreSQL compares equal ([`Canon`]:
//! reals with `-0 = 0` and every NaN alike, numerics by value whatever
//! their scale), since a query may pick any of equal values (module docs
//! of [`crate::expr`]); rows as a multiset, or in order when the query
//! orders by every column.
//!
//! Errors: the original (first) query of a comparison must not fail except
//! as below, or the generator wrote an invalid query; when it succeeds,
//! every variant must succeed.
//!
//! Exclusions, each narrow:
//!
//! - Run-time errors whose occurrence depends on the plan
//!   ([`expected_error`]: integer and real overflow, values out of range,
//!   NaN or infinity cast to an integer, infinite dates subtracted). Which
//!   rows an expression is evaluated on, and in what order a conjunction's
//!   terms are, is the planner's, and a TLP partition evaluates the
//!   predicate where the original does not. An original that fails so is
//!   skipped with its query; a comparison where exactly one side fails so
//!   is skipped. Counted in [`Tally::error_skips`].
//! - The statement timeout (10 s, [`crate::pg::Session::fresh`]): counted
//!   in [`Tally::timeouts`].
//! - Known PostgreSQL bugs, skipped where they show ([`known_error`],
//!   counted in [`Tally::known_skips`]):
//!   - "unrecognized node type: <n>" (XX000): a planner use-after-free of a
//!     partial Path that `add_partial_path` frees while a Gather built in
//!     `apply_scanjoin_target_to_paths` still points to it, reachable since
//!     8300d3ad4aa (2026-03-09); discussed on pgsql-hackers ("Use-after-free
//!     of a shared Path in add_path()/add_partial_path()"), not fixed on
//!     master 158b6d5109 (`../bugs/candidates/pg-unrecognized-node-10`).
//!   - "cannot fetch row before WindowObject's mark position" (XX000): a
//!     ROWS frame excluding the current row with EXCLUDE TIES and an offset
//!     of 2 FOLLOWING or more, BUG #19732 (patch v4, not committed;
//!     `../bugs/candidates/pg-window-exclude-ties-mark`). Its wrong-result
//!     form is not recognized: `window-def` reports it.
//!   - EXPLAIN ANALYZE of a parallel plan whose Parallel Append children
//!     share a SubPlan with a Materialize node fails an assertion in
//!     shm_toc_insert (master since a241bc4fede,
//!     `../bugs/postgresql/pg-parallel-material-dup-toc`): with a
//!     partitioned table and a subquery expression ([`parallel_material_bug`]),
//!     `explain-analyze-rows` runs without parallel workers (counted).
//!   - "corrupt MVNDistinct entry" (XX000) from the planner's group estimate
//!     (estimate_multivariate_ndistinct) for grouping sets over a column and
//!     an expression of it on a table with extended statistics, master and
//!     18.6, BUG #19772 (`../bugs/postgresql/pg-mvndistinct-grouping-sets`).
//!   - Sorted grouping sets (GroupAggregate) leave a grouping column with
//!     its value, not NULL, in a set that omits it, when the row has a NULL
//!     and a later input column is projected: a PG 19 regression, likely
//!     from c456e391138 (`../bugs/postgresql/pg-plans-sortagg-cte`). Which
//!     rows it touches cannot be told from the query, so on a query with
//!     grouping sets a difference of rows between the original and a variant
//!     is a known skip (`agree`); errors are still failures. This also hides
//!     any other grouping-sets wrong result until the target has the fix.
//!   - `pg_get_viewdef(.., true)` drops the parentheses of an operator
//!     expression left of `IN (subquery)` ([`sublink_paren_bug`],
//!     `../bugs/candidates/pg-viewdef-sublink-paren`): on such a query the
//!     pretty text's error or other rows are a known skip, the raw text is
//!     checked, and the fixed point starts from the raw text.
//!   - `pg_get_viewdef` (both forms) prints a GROUP BY or ORDER BY term
//!     that negates an `int4` or `numeric` literal as `(- 3)`, which parses
//!     as the constant -3, a column position
//!     ([`negated_sort_constant_bug`],
//!     `../bugs/postgresql/pg-viewdef-negated-sort-constant`, BUG #19767): on such a
//!     query an error or other rows from either text are a known skip, and
//!     the fixed point is skipped.

use std::{cmp::Ordering, collections::BTreeMap};

use crate::{
    cases::{final_model, Case},
    expr::{AggFunc, Bound, Exclude, Expr, FrameMode, Materialized, Query, Schema, Select, SetOp, WinCall, WinFunc},
    model::{order_nulls, Numeric, Value},
    pg::{lost, with_session, Cell, Session},
    setup::load,
    statements::returns_rows,
};

pub type Rows = Vec<Vec<Cell>>;
pub type Outcome = Result<Rows, String>;

/// What the oracles compared and skipped over some inputs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    pub queries: usize,
    /// Comparisons made, per variant.
    pub comparisons: BTreeMap<&'static str, usize>,
    /// Queries a shape-specific oracle does not apply to.
    pub inapplicable: usize,
    /// Comparisons or queries skipped for a plan-dependent run-time error.
    pub error_skips: usize,
    pub timeouts: usize,
    /// Comparisons or queries skipped for a known PostgreSQL bug.
    pub known_skips: usize,
}

impl Tally {
    pub fn add(&mut self, other: &Tally) {
        self.queries += other.queries;
        for (k, v) in &other.comparisons {
            *self.comparisons.entry(k).or_insert(0) += v;
        }
        self.inapplicable += other.inapplicable;
        self.error_skips += other.error_skips;
        self.timeouts += other.timeouts;
        self.known_skips += other.known_skips;
    }

    pub fn total(&self) -> usize {
        self.comparisons.values().sum()
    }
}

/// Whether an error is one whose occurrence depends on the plan (which
/// rows an expression is evaluated on): overflow, a value out of range, a
/// NaN or infinity cast to an integer, infinite dates subtracted.
pub fn expected_error(e: &str) -> bool {
    ["out of range", "overflow", "cannot convert NaN", "cannot convert infinity", "cannot subtract infinite"].iter().any(|m| e.contains(m))
        || e.contains("(22003)")
        || e.contains("(22008)")
}

/// Whether an error is a known PostgreSQL bug (module docs); not once the
/// target has its fix ([`crate::fixed`]).
pub fn known_error(e: &str) -> bool {
    e.contains("unrecognized node type:") && !crate::fixed("unrecognized-node-10")
        || e.contains("cannot fetch row before WindowObject's mark position") && !crate::fixed("window-exclude-ties-mark")
        || e.contains("corrupt MVNDistinct entry") && !crate::fixed("mvndistinct-grouping-sets")
}

/// Whether a query shows the known pretty-printing bug of
/// `pg_get_viewdef(.., true)` (module docs): the left operand of `IN
/// (subquery)` an expression that binds looser than IN (a comparison, IS,
/// a boolean operator, BETWEEN, LIKE, an IN list), which pretty printing
/// leaves without its parentheses. Not once the target has its fix.
pub fn sublink_paren_bug(q: &Query) -> bool {
    fn looser(e: &Expr) -> bool {
        matches!(
            e,
            Expr::Cmp(..) | Expr::Distinct(..) | Expr::And(..) | Expr::Or(..) | Expr::Not(_) | Expr::IsNull(..) | Expr::Is(..) | Expr::Between { .. } | Expr::InList(..) | Expr::Like { .. }
            // NOT IN and NOT EXISTS are stored as NOT (..): a NOT, printed bare.
            | Expr::InQuery(_, _, true) | Expr::Exists(_, true)
        )
    }
    let mut found = false;
    for sel in all_selects(q) {
        for e in sel.exprs() {
            e.walk(&mut |x| found |= matches!(x, Expr::InQuery(a, ..) if looser(a)));
        }
    }
    !crate::fixed("viewdef-sublink-paren") && found
}

/// Whether a query shows the known deparsing bug of `pg_get_viewdef` in
/// both forms (module docs): a GROUP BY or ORDER BY term that is a unary
/// minus of a non-negative `int4` or `numeric` literal (`(- '3'::int4)`)
/// deparses as `(- 3)`, which parses as the constant -3, a position. Not
/// once the target has its fix.
pub fn negated_sort_constant_bug(q: &Query) -> bool {
    fn negated_constant(e: &Expr) -> bool {
        match e {
            Expr::Neg(a) => match &**a {
                Expr::Lit(v, crate::model::Type::Int4 | crate::model::Type::Numeric) => *v != Value::Null,
                inner => negated_constant(inner),
            },
            _ => false,
        }
    }
    let ordered = !q.order.is_empty() && q.selects().iter().any(|sel| sel.items.iter().any(negated_constant));
    let grouped = all_selects(q).iter().any(|sel| sel.group.iter().any(negated_constant));
    !crate::fixed("viewdef-negated-sort-constant") && (ordered || grouped)
}

/// Every SELECT of a query: its CTEs', its own, its members' and those of
/// every subquery in them.
fn all_selects(q: &Query) -> Vec<&Select> {
    let mut out: Vec<&Select> = q.with.iter().map(|c| &c.query).chain(q.selects()).collect();
    let mut k = 0;
    while k < out.len() {
        let subs: Vec<&Query> = out[k].exprs().into_iter().flat_map(|e| e.subqueries()).collect();
        for sq in subs {
            out.extend(all_selects(sq));
        }
        let from = out[k].from_selects();
        out.extend(from);
        k += 1;
    }
    out
}

fn timeout(e: &str) -> bool {
    e.contains("statement timeout") || e.contains("57014")
}

/// A value under the equality PostgreSQL compares results by: reals with
/// `-0 = 0` and every NaN alike, numerics by value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Canon {
    Null,
    Int(i64),
    Float(u64),
    Numeric(String),
    Text(String),
    Bool(bool),
    Bytea(Vec<u8>),
    Date(i32),
    Timestamp(i64),
    Other(String, Vec<u8>),
}

fn canon_numeric(n: &Numeric) -> String {
    match n {
        Numeric::Finite { neg, digits, scale } => {
            let (mut d, mut s) = (digits.clone(), *scale);
            while s > 0 && d.last() == Some(&b'0') {
                d.pop();
                s -= 1;
            }
            // Zero has no digits: every scale of it is one value.
            if d.is_empty() {
                s = 0;
            }
            Numeric::finite(*neg, d, s).to_string()
        },
        _ => n.to_string(),
    }
}

pub fn canon(c: &Cell) -> Canon {
    match c {
        Cell::Other(t, b) => Canon::Other(t.clone(), b.clone()),
        Cell::Value(v) => match v {
            Value::Null => Canon::Null,
            Value::Int(i) => Canon::Int(*i),
            Value::Float(x) if x.is_nan() => Canon::Float(u64::MAX),
            Value::Float(x) if *x == 0.0 => Canon::Float(0),
            Value::Float(x) => Canon::Float(x.to_bits()),
            Value::Numeric(n) => Canon::Numeric(canon_numeric(n)),
            Value::Text(s) => Canon::Text(s.clone()),
            Value::Bool(b) => Canon::Bool(*b),
            Value::Bytea(b) => Canon::Bytea(b.clone()),
            Value::Date(d) => Canon::Date(*d),
            Value::Timestamp(t) => Canon::Timestamp(*t),
        },
    }
}

fn canon_rows(rows: &Rows) -> Vec<Vec<Canon>> {
    rows.iter().map(|r| r.iter().map(canon).collect()).collect()
}

/// How two results are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Multiset,
    Set,
    Ordered,
}

fn same(mode: Mode, a: &Rows, b: &Rows) -> bool {
    let (mut x, mut y) = (canon_rows(a), canon_rows(b));
    match mode {
        Mode::Ordered => x == y,
        Mode::Multiset => {
            x.sort();
            y.sort();
            x == y
        },
        Mode::Set => {
            x.sort();
            x.dedup();
            y.sort();
            y.dedup();
            x == y
        },
    }
}

fn mode_of(q: &Query) -> Mode {
    if q.order.is_empty() {
        Mode::Multiset
    } else {
        Mode::Ordered
    }
}

fn show(rows: &Rows) -> String {
    let s = format!("{:?}", canon_rows(rows));
    if s.len() > 1500 {
        // At a character boundary: rows hold text with multibyte characters.
        let mut k = 1500;
        while !s.is_char_boundary(k) {
            k -= 1;
        }
        format!("{}.. ({} rows)", &s[..k], rows.len())
    } else {
        s
    }
}

/// Run a query.
pub fn run(s: &mut Session, sql: &str) -> Outcome {
    s.query(sql)
}

/// The original of a comparison: its rows, None when it is skipped (a
/// plan-dependent error or a timeout), a failure for any other error.
fn original(s: &mut Session, sql: &str, t: &mut Tally) -> Result<Option<Rows>, String> {
    match run(s, sql) {
        Ok(rows) => Ok(Some(rows)),
        Err(e) if lost(&e) => Err(format!("the server went away running {sql}: {e}")),
        Err(e) if expected_error(&e) => {
            t.error_skips += 1;
            Ok(None)
        },
        Err(e) if known_error(&e) => {
            t.known_skips += 1;
            Ok(None)
        },
        Err(e) if timeout(&e) => {
            t.timeouts += 1;
            Ok(None)
        },
        Err(e) => Err(format!("the original query fails: {sql}\n  {e}")),
    }
}

thread_local! {
    /// Whether the query being checked has grouping sets on a target
    /// without the fix of the sorted grouping-sets bug (module docs): its
    /// results may differ between plans, so a difference is a known skip.
    static GROUPING_SETS_ROWS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a query reads grouping sets anywhere (its CTEs, members and
/// subqueries included).
fn has_grouping_sets(q: &Query) -> bool {
    all_selects(q).iter().any(|sel| sel.sets.is_some())
}

/// Compare a variant's outcome with the original's rows.
fn agree(t: &mut Tally, name: &'static str, mode: Mode, osql: &str, orig: &Rows, vsql: &str, variant: &Outcome) -> Result<(), String> {
    match variant {
        Err(e) if lost(e) => Err(format!("{name}: the server went away running the variant\n  {vsql}\n  {e}")),
        Err(e) if expected_error(e) => {
            t.error_skips += 1;
            Ok(())
        },
        Err(e) if known_error(e) => {
            t.known_skips += 1;
            Ok(())
        },
        Err(e) if timeout(e) => {
            t.timeouts += 1;
            Ok(())
        },
        Err(e) => Err(format!("{name}: the original succeeds, the variant fails\n  original: {osql}\n  variant:  {vsql}\n  {e}")),
        Ok(rows) => {
            *t.comparisons.entry(name).or_insert(0) += 1;
            if same(mode, orig, rows) {
                Ok(())
            } else if GROUPING_SETS_ROWS.get() {
                t.known_skips += 1;
                Ok(())
            } else {
                Err(format!("{name}: results differ\n  original: {osql}\n  -> {}\n  variant:  {vsql}\n  -> {}", show(orig), show(rows)))
            }
        },
    }
}

/// Load the case and run its statements, committing a transaction they
/// leave open.
pub fn prepare(s: &mut Session, case: &Case) -> Result<(), String> {
    s.fresh()?;
    load(s, &case.setup)?;
    for (k, st) in case.statements.iter().enumerate() {
        let sql = st.to_string();
        let r = if returns_rows(st) { s.query(&sql).map(|_| ()) } else { s.execute(&sql).map(|_| ()) };
        r.map_err(|e| format!("statement {k} of the case: {sql}: {e}"))?;
    }
    if !final_model(case).savepoints.is_empty() {
        s.execute("COMMIT")?;
    }
    Ok(())
}

/// Run `check` on every query, the case prepared once.
fn each(case: &Case, queries: &[Query], mut check: impl FnMut(&mut Session, &Query, &mut Tally) -> Result<(), String>) -> Result<Tally, String> {
    with_session(|s| {
        prepare(s, case)?;
        let schema = Schema::of(&final_model(case));
        let mut tally = Tally::default();
        for q in queries {
            if !q.valid(&schema) {
                return Err(format!("a query is not valid on the case's final schema: {}", q.sql()));
            }
            tally.queries += 1;
            GROUPING_SETS_ROWS.set(has_grouping_sets(q) && !crate::fixed("sorted-grouping-sets-unnulled"));
            let r = check(s, q, &mut tally);
            GROUPING_SETS_ROWS.set(false);
            r?;
        }
        Ok(tally)
    })
}

/// `SELECT .. FROM F` of a select, at the top level, with the query's CTEs.
fn select_sql(q: &Query, sel: &Select, filter: Option<String>, rest: bool) -> String {
    let mut s = format!("{}SELECT {}{} FROM {}", q.with_sql(), if sel.distinct { "DISTINCT " } else { "" }, sel.items_sql(0), sel.from_sql(0));
    if let Some(f) = filter {
        s.push_str(&format!(" WHERE {f}"));
    }
    if rest {
        s.push_str(&sel.group_sql(0));
    }
    s
}

// ------------------------------------------------------------------ NoREC

pub fn norec(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        for sel in q.selects() {
            let Some(p) = &sel.filter else {
                t.inapplicable += 1;
                continue;
            };
            let (p, from, w) = (p.sql(0), sel.from_sql(0), q.with_sql());
            let osql = format!("{w}SELECT count(*) FROM {from} WHERE {p}");
            let vsql = format!("{w}SELECT coalesce(sum(CASE WHEN ({p}) IS TRUE THEN 1 ELSE 0 END), 0)::int8 FROM {from}");
            let Some(orig) = original(s, &osql, t)? else { continue };
            let variant = run(s, &vsql);
            let name = if sel.joins.is_empty() { "norec" } else { "norec-join" };
            agree(t, name, Mode::Multiset, &osql, &orig, &vsql, &variant)?;
        }
        Ok(())
    })
}

// -------------------------------------------------------------------- TLP

fn partitions(x: &str) -> [String; 3] {
    [format!("({x})"), format!("NOT ({x})"), format!("({x}) IS NULL")]
}

/// Rows of several queries, concatenated; the first error.
fn concat(s: &mut Session, sqls: &[String]) -> Outcome {
    let mut out = Vec::new();
    for sql in sqls {
        out.extend(run(s, sql)?);
    }
    Ok(out)
}

pub fn tlp(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        let sel = &q.select;
        let w = q.with_sql();
        // A partition of WHERE changes the windows' partitions.
        if sel.items.iter().any(crate::expr::has_window) {
            t.inapplicable += 1;
            return Ok(());
        }
        // A single aggregate, partitioned on WHERE.
        if let (true, [Expr::Agg(f, distinct, arg)], Some(p)) = (sel.group.is_empty() && sel.having.is_none(), &sel.items[..], &sel.filter) {
            let combine = match f {
                AggFunc::Count | AggFunc::Sum if !distinct => "sum",
                AggFunc::Min => "min",
                AggFunc::Max => "max",
                AggFunc::BoolAnd => "bool_and",
                AggFunc::BoolOr => "bool_or",
                _ => {
                    t.inapplicable += 1;
                    return Ok(());
                },
            };
            let item = sel.items[0].sql(0);
            let from = sel.from_sql(0);
            let osql = format!("{w}SELECT {item} FROM {from}");
            let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
            let parts = partitions(&p.sql(0)).map(|x| format!("SELECT {item} AS r FROM {from} WHERE {x}"));
            // `sum` of partial counts and sums is a numeric: cast back to
            // the original's type.
            let ty = crate::expr::type_of(&sel.items[0]);
            let _ = arg;
            let combined = if combine == "sum" { format!("CAST(sum(r) AS {})", ty.name()) } else { format!("{combine}(r)") };
            let vsql = format!("{w}SELECT {combined} FROM ({}) AS u", parts.join(" UNION ALL "));
            let variant = run(s, &vsql);
            return agree(t, "tlp-agg", Mode::Multiset, &osql, &orig, &vsql, &variant);
        }
        if sel.aggregate() && sel.having.is_none() && !sel.group.is_empty() {
            // GROUP BY keys only (no aggregate among the items), on WHERE.
            let Some(p) = &sel.filter else {
                t.inapplicable += 1;
                return Ok(());
            };
            if sel.items.iter().any(Expr::has_agg) {
                t.inapplicable += 1;
                return Ok(());
            }
            let osql = select_sql(q, sel, None, true);
            let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
            let parts: Vec<String> = partitions(&p.sql(0)).iter().map(|x| select_sql(q, sel, Some(x.clone()), true)).collect();
            let variant = concat(s, &parts);
            agree(t, "tlp-group", Mode::Set, &osql, &orig, &parts.join("; "), &variant)?;
            let union = parts.iter().map(|x| format!("({})", x.trim_start_matches(&w))).collect::<Vec<_>>().join(" UNION ");
            let usql = format!("{w}{union}");
            let variant = run(s, &usql);
            return agree(t, "tlp-union", Mode::Set, &osql, &orig, &usql, &variant);
        }
        if let Some(h) = &sel.having {
            // HAVING partitioned: whole groups kept or dropped.
            let base = Select { having: None, ..sel.clone() };
            let osql = select_sql(q, &base, sel.filter.as_ref().map(|p| p.sql(0)), true);
            let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
            let parts: Vec<String> = partitions(&h.sql(0)).iter().map(|x| format!("{osql} HAVING {x}")).collect();
            let variant = concat(s, &parts);
            let mode = if sel.distinct { Mode::Set } else { Mode::Multiset };
            return agree(t, "tlp-having", mode, &osql, &orig, &parts.join("; "), &variant);
        }
        if sel.aggregate() {
            t.inapplicable += 1;
            return Ok(());
        }
        let Some(p) = &sel.filter else {
            t.inapplicable += 1;
            return Ok(());
        };
        let osql = select_sql(q, sel, None, false);
        let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
        let parts: Vec<String> = partitions(&p.sql(0)).iter().map(|x| select_sql(q, sel, Some(x.clone()), false)).collect();
        let (name, mode, op) = if sel.distinct { ("tlp-distinct", Mode::Set, " UNION ") } else { ("tlp-where", Mode::Multiset, " UNION ALL ") };
        let variant = concat(s, &parts);
        agree(t, name, mode, &osql, &orig, &parts.join("; "), &variant)?;
        let union = parts.iter().map(|x| format!("({})", x.trim_start_matches(&w))).collect::<Vec<_>>().join(op);
        let usql = format!("{w}{union}");
        let variant = run(s, &usql);
        agree(t, "tlp-union", mode, &osql, &orig, &usql, &variant)
    })
}

// ------------------------------------------------------------------ plans

/// Planner settings that change a plan and never a result, by name.
pub const PLAN_VARIANTS: [(&str, &[(&str, &str)]); 17] = [
    ("plans-partitionwise", &[("enable_partitionwise_join", "on"), ("enable_partitionwise_aggregate", "on")]),
    ("plans-noprune", &[("enable_partition_pruning", "off")]),
    ("plans-seqscan", &[("enable_indexscan", "off"), ("enable_bitmapscan", "off"), ("enable_indexonlyscan", "off"), ("enable_tidscan", "off")]),
    ("plans-indexscan", &[("enable_seqscan", "off"), ("enable_bitmapscan", "off")]),
    ("plans-bitmapscan", &[("enable_seqscan", "off"), ("enable_indexscan", "off"), ("enable_indexonlyscan", "off")]),
    ("plans-nestloop", &[("enable_hashjoin", "off"), ("enable_mergejoin", "off")]),
    ("plans-mergejoin", &[("enable_hashjoin", "off"), ("enable_nestloop", "off")]),
    ("plans-hashjoin", &[("enable_mergejoin", "off"), ("enable_nestloop", "off")]),
    ("plans-sortagg", &[("enable_hashagg", "off")]),
    ("plans-hashagg", &[("enable_sort", "off"), ("enable_incremental_sort", "off")]),
    ("plans-nomaterial", &[("enable_material", "off"), ("enable_memoize", "off")]),
    (
        "plans-parallel",
        &[
            ("debug_parallel_query", "on"),
            ("parallel_setup_cost", "0"),
            ("parallel_tuple_cost", "0"),
            ("min_parallel_table_scan_size", "0"),
            ("min_parallel_index_scan_size", "0"),
            ("max_parallel_workers_per_gather", "4"),
        ],
    ),
    ("plans-written-order", &[("join_collapse_limit", "1"), ("from_collapse_limit", "1")]),
    ("plans-geqo", &[("geqo", "on"), ("geqo_threshold", "2"), ("geqo_seed", "0.5")]),
    // Sorts, hashes and aggregation spill to disk.
    ("plans-tinymem", &[("work_mem", "64"), ("hash_mem_multiplier", "1"), ("enable_hashagg", "on")]),
    // No reordering of GROUP BY and DISTINCT keys, no presorted
    // aggregates, no self-join elimination.
    (
        "plans-noreorder",
        &[("enable_group_by_reordering", "off"), ("enable_distinct_reordering", "off"), ("enable_presorted_aggregate", "off"), ("enable_self_join_elimination", "off")],
    ),
    // Parallel plans without parallel hash or append, the leader not
    // participating.
    (
        "plans-parallel-noshare",
        &[
            ("debug_parallel_query", "on"),
            ("parallel_setup_cost", "0"),
            ("parallel_tuple_cost", "0"),
            ("min_parallel_table_scan_size", "0"),
            ("enable_parallel_hash", "off"),
            ("enable_parallel_append", "off"),
            ("parallel_leader_participation", "off"),
        ],
    ),
];

/// Run `sql` with these settings for one transaction, then roll back.
fn run_with(s: &mut Session, settings: &[(&str, &str)], sql: &str) -> Outcome {
    let set: String = settings.iter().map(|(k, v)| format!("SET LOCAL {k} = {v}; ")).collect();
    s.execute(&format!("BEGIN; {set}"))?;
    let out = run(s, sql);
    let end = s.execute("ROLLBACK");
    match (out, end) {
        (Err(e), _) => Err(e),
        (Ok(_), Err(e)) => Err(e),
        (Ok(rows), Ok(_)) => Ok(rows),
    }
}

pub fn plans(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        let osql = q.sql();
        let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
        for (name, settings) in PLAN_VARIANTS {
            let variant = run_with(s, settings, &osql);
            agree(t, name, mode_of(q), &osql, &orig, &osql, &variant)?;
        }
        Ok(())
    })
}

// ------------------------------------------------------------------ order

/// PostgreSQL's order of two cells for one ORDER BY term.
pub fn cmp_cells(a: &Cell, b: &Cell, desc: bool, nulls_first: Option<bool>) -> Ordering {
    match (a, b) {
        (Cell::Value(x), Cell::Value(y)) => order_nulls(x, y, desc, nulls_first),
        _ => canon(a).cmp(&canon(b)),
    }
}

pub fn order(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        if q.order.is_empty() {
            t.inapplicable += 1;
            return Ok(());
        }
        let osql = q.sql();
        let Some(rows) = original(s, &osql, t)? else { return Ok(()) };
        // Sorted by every term.
        let sorted = rows.windows(2).all(|w| {
            q.order
                .iter()
                .map(|&(k, desc, nulls)| cmp_cells(&w[0][k - 1], &w[1][k - 1], desc, nulls))
                .find(|o| o.is_ne())
                .is_none_or(|o| o.is_lt())
        });
        *t.comparisons.entry("order-sorted").or_insert(0) += 1;
        if !sorted {
            return Err(format!("order-sorted: rows out of order\n  {osql}\n  -> {}", show(&rows)));
        }
        let unordered = Query { order: Vec::new(), limit: None, ..q.clone() };
        let usql = unordered.sql();
        match q.limit {
            None => agree(t, "order-multiset", Mode::Multiset, &osql, &rows, &usql, &run(s, &usql)),
            Some(l) => {
                let full = Query { limit: None, ..q.clone() };
                let fsql = full.sql();
                let window = run(s, &fsql).map(|all| all.into_iter().skip(l.offset as usize).take(l.count as usize).collect());
                agree(t, "order-limit", Mode::Ordered, &osql, &rows, &fsql, &window)
            },
        }
    })
}

// ----------------------------------------------------------------- setops

/// A set operation on two results, as PostgreSQL defines it, values equal
/// as it compares them (a deduplicated row is any of its equals).
pub fn set_op(op: SetOp, left: Rows, right: Rows) -> Rows {
    let key = |r: &Vec<Cell>| r.iter().map(canon).collect::<Vec<_>>();
    let count = |rows: &Rows| {
        let mut m: BTreeMap<Vec<Canon>, (usize, Vec<Cell>)> = BTreeMap::new();
        for r in rows {
            m.entry(key(r)).or_insert((0, r.clone())).0 += 1;
        }
        m
    };
    let (l, r) = (count(&left), count(&right));
    let mut out = Vec::new();
    match op {
        SetOp::UnionAll => {
            out = left;
            out.extend(right);
        },
        SetOp::Union => {
            let mut all = l.clone();
            for (k, v) in r {
                all.entry(k).or_insert(v);
            }
            out.extend(all.into_values().map(|(_, row)| row));
        },
        SetOp::Intersect | SetOp::IntersectAll => {
            for (k, (n, row)) in &l {
                if let Some((m, _)) = r.get(k) {
                    let times = if op == SetOp::Intersect { 1 } else { (*n).min(*m) };
                    out.extend(std::iter::repeat_n(row.clone(), times));
                }
            }
        },
        SetOp::Except | SetOp::ExceptAll => {
            for (k, (n, row)) in &l {
                let m = r.get(k).map_or(0, |x| x.0);
                let times = if op == SetOp::Except { usize::from(m == 0) } else { n.saturating_sub(m) };
                out.extend(std::iter::repeat_n(row.clone(), times));
            }
        },
    }
    out
}

pub fn setops(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        if q.compound.is_empty() {
            t.inapplicable += 1;
            return Ok(());
        }
        let whole = Query { order: Vec::new(), limit: None, ..q.clone() };
        let osql = whole.sql();
        let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
        let w = q.with_sql();
        let mut acc = match run(s, &format!("{w}{}", q.select.sql_at(0))) {
            Ok(rows) => rows,
            Err(e) => return agree(t, "setops-def", Mode::Multiset, &osql, &orig, "first SELECT", &Err(e)),
        };
        for (op, m) in &q.compound {
            let msql = format!("{w}{}", m.sql_at(0));
            match run(s, &msql) {
                Ok(rows) => acc = set_op(*op, acc, rows),
                Err(e) => return agree(t, "setops-def", Mode::Multiset, &osql, &orig, &msql, &Err(e)),
            }
        }
        agree(t, "setops-def", Mode::Multiset, &osql, &orig, "(computed from the SELECTs' rows)", &Ok(acc))?;
        if let [(op, m)] = &q.compound[..] {
            if matches!(op, SetOp::Union | SetOp::UnionAll | SetOp::Intersect | SetOp::IntersectAll) {
                let swapped = Query { select: m.clone(), compound: vec![(*op, q.select.clone())], order: Vec::new(), limit: None, ..q.clone() };
                let vsql = swapped.sql();
                agree(t, "setops-swap", Mode::Multiset, &osql, &orig, &vsql, &run(s, &vsql))?;
            }
        }
        Ok(())
    })
}

// ------------------------------------------------------------------- CTEs

pub fn ctes(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        if q.with.is_empty() {
            t.inapplicable += 1;
            return Ok(());
        }
        let osql = q.sql();
        let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
        for (name, hint) in [("cte-materialized", Materialized::Yes), ("cte-not-materialized", Materialized::No)] {
            let vsql = format!("{}{}", q.with_sql_as(Some(hint)), q.sql_at(0));
            agree(t, name, mode_of(q), &osql, &orig, &vsql, &run(s, &vsql))?;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- windows

/// One row as a window reads it: its partition values, its order values
/// and its arguments.
pub type WindowRow = (Vec<Cell>, Vec<Cell>, Vec<Cell>);

fn peers(a: &WindowRow, b: &WindowRow) -> bool {
    a.1.iter().zip(&b.1).all(|(x, y)| canon(x) == canon(y))
}

fn order_rows(w: &WinCall, a: &WindowRow, b: &WindowRow) -> Ordering {
    w.order.iter().zip(a.1.iter().zip(&b.1)).map(|((_, desc, nulls), (x, y))| cmp_cells(x, y, *desc, *nulls)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
}

/// The values of a window function over rows, as SQL defines them, in the
/// order of `rows`: rows partitioned by equal partition values, ordered by
/// the order terms (peers: equal on every term), each row's frame by the
/// frame clause (the default `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT
/// ROW`) less what EXCLUDE removes. Functions whose values depend on the
/// order of peers have a total order (`crate::expr`), so peers hold equal
/// values and any order among them gives the same rows. None for an
/// aggregate the oracle does not compute (`sum`, `avg`).
pub fn window_values(w: &WinCall, rows: &[WindowRow]) -> Option<Vec<Cell>> {
    let mut out: Vec<Cell> = vec![Cell::Value(Value::Null); rows.len()];
    let mut parts: BTreeMap<Vec<Canon>, Vec<usize>> = BTreeMap::new();
    for (k, r) in rows.iter().enumerate() {
        parts.entry(r.0.iter().map(canon).collect()).or_default().push(k);
    }
    for (_, mut idx) in parts {
        idx.sort_by(|&a, &b| order_rows(w, &rows[a], &rows[b]));
        let n = idx.len();
        // Peer groups: group[i] and each group's first and last position.
        let mut group = vec![0usize; n];
        let mut bounds: Vec<(usize, usize)> = Vec::new();
        for i in 0..n {
            if i == 0 || !peers(&rows[idx[i - 1]], &rows[idx[i]]) {
                bounds.push((i, i));
            }
            let g = bounds.len() - 1;
            bounds[g].1 = i;
            group[i] = g;
        }
        // A row's frame as up to three runs of positions (half-open, in
        // order): the bounds' range less what EXCLUDE removes. Runs rather
        // than a list of positions, and aggregates over runs from prefix
        // counts and a segment tree, so a partition of n rows costs
        // O(n log n), not O(n²): a whole-partition frame over a 35 000-row
        // cross join took minutes, past the runner's hang limit.
        let frame_of = |i: usize| -> [(usize, usize); 3] {
            let (mode, start, end, exclude) = match &w.frame {
                Some(f) => (f.mode, f.start, f.end, f.exclude),
                None => (FrameMode::Range, Bound::UnboundedPreceding, Bound::CurrentRow, Exclude::NoOthers),
            };
            let g = group[i] as i64;
            let at = |b: Bound, is_start: bool| -> i64 {
                match (mode, b) {
                    (_, Bound::UnboundedPreceding) => 0,
                    (_, Bound::UnboundedFollowing) => n as i64 - 1,
                    (FrameMode::Rows, Bound::Preceding(k)) => i as i64 - i64::from(k),
                    (FrameMode::Rows, Bound::Following(k)) => i as i64 + i64::from(k),
                    (FrameMode::Rows, Bound::CurrentRow) => i as i64,
                    (_, Bound::CurrentRow) => {
                        if is_start {
                            bounds[g as usize].0 as i64
                        } else {
                            bounds[g as usize].1 as i64
                        }
                    },
                    (_, Bound::Preceding(k)) | (_, Bound::Following(k)) => {
                        let h = if matches!(b, Bound::Preceding(_)) { g - i64::from(k) } else { g + i64::from(k) };
                        if h < 0 {
                            if is_start { 0 } else { -1 }
                        } else if h as usize >= bounds.len() {
                            if is_start { n as i64 } else { n as i64 - 1 }
                        } else if is_start {
                            bounds[h as usize].0 as i64
                        } else {
                            bounds[h as usize].1 as i64
                        }
                    },
                }
            };
            let lo = at(start, true).clamp(0, n as i64) as usize;
            let hi = (at(end, false).min(n as i64 - 1) + 1).max(0) as usize;
            let (first, last) = bounds[group[i]];
            // Removed: [a, b); with EXCLUDE TIES the row itself stays.
            let (a, b) = match exclude {
                Exclude::NoOthers => (hi, hi),
                Exclude::CurrentRow => (i, i + 1),
                Exclude::Group | Exclude::Ties => (first, last + 1),
            };
            let run = |s: usize, e: usize| if s < e { (s, e) } else { (0, 0) };
            let own = if exclude == Exclude::Ties && lo <= i && i < hi { (i, i + 1) } else { (0, 0) };
            [run(lo, hi.min(a)), own, run(lo.max(b), hi)]
        };
        let arg = |j: usize| rows[idx[j]].2[0].clone();
        // The argument at position j when it is not NULL.
        let val = |j: usize| rows[idx[j]].2.first().filter(|c| **c != Cell::Value(Value::Null));
        let int = |k: i64| Cell::Value(Value::Int(k));
        let null = Cell::Value(Value::Null);
        let size = |f: &[(usize, usize)]| f.iter().map(|(s, e)| e - s).sum::<usize>();
        // The k-th position (from 0) of a frame.
        let nth = |f: &[(usize, usize)], mut k: usize| {
            for &(s, e) in f {
                if k < e - s {
                    return Some(s + k);
                }
                k -= e - s;
            }
            None
        };
        // Prefix counts of non-NULL and true arguments, for counts and
        // bool_and/bool_or over runs.
        let (mut nonnull, mut trues) = (vec![0usize; n + 1], vec![0usize; n + 1]);
        for j in 0..n {
            nonnull[j + 1] = nonnull[j] + usize::from(val(j).is_some());
            trues[j + 1] = trues[j] + usize::from(matches!(val(j), Some(Cell::Value(Value::Bool(true)))));
        }
        let over = |p: &[usize], f: &[(usize, usize)]| f.iter().map(|&(s, e)| p[e] - p[s]).sum::<usize>();
        // min and max: a segment tree of the extreme non-NULL position, ties
        // to the first position for min and the last for max (as
        // `min_by`/`max_by` over the frame in order).
        let max = matches!(w.func, WinFunc::Agg(AggFunc::Max));
        let pick = |x: Option<usize>, y: Option<usize>| -> Option<usize> {
            match (x, y) {
                (Some(a), Some(b)) => {
                    let o = cmp_cells(val(a)?, val(b)?, false, None);
                    let o = if max { o.reverse() } else { o };
                    Some(match o {
                        Ordering::Less => a,
                        Ordering::Greater => b,
                        Ordering::Equal if max => a.max(b),
                        Ordering::Equal => a.min(b),
                    })
                },
                (x, None) => x,
                (None, y) => y,
            }
        };
        let tree: Vec<Option<usize>> = if matches!(w.func, WinFunc::Agg(AggFunc::Min | AggFunc::Max)) {
            let mut t = vec![None; 2 * n];
            for j in 0..n {
                t[n + j] = val(j).map(|_| j);
            }
            for k in (1..n).rev() {
                t[k] = pick(t[2 * k], t[2 * k + 1]);
            }
            t
        } else {
            Vec::new()
        };
        let extreme = |f: &[(usize, usize)]| -> Option<usize> {
            let mut best = None;
            for &(s, e) in f {
                let (mut l, mut r) = (s + n, e + n);
                while l < r {
                    if l & 1 == 1 {
                        best = pick(best, tree[l]);
                        l += 1;
                    }
                    if r & 1 == 1 {
                        r -= 1;
                        best = pick(best, tree[r]);
                    }
                    l >>= 1;
                    r >>= 1;
                }
            }
            best
        };
        for i in 0..n {
            let (first, last) = bounds[group[i]];
            let v = match w.func {
                WinFunc::RowNumber => int(i as i64 + 1),
                WinFunc::Rank => int(first as i64 + 1),
                WinFunc::DenseRank => int(group[i] as i64 + 1),
                WinFunc::PercentRank => Cell::Value(Value::Float(if n > 1 { first as f64 / (n - 1) as f64 } else { 0.0 })),
                WinFunc::CumeDist => Cell::Value(Value::Float((last + 1) as f64 / n as f64)),
                WinFunc::Ntile(k) => {
                    // Buckets of n / k rows, the first n % k one larger.
                    let k = k as usize;
                    let (base, extra) = (n / k, n % k);
                    let big = extra * (base + 1);
                    let b = if i < big { i / (base + 1) } else { extra + (i - big) / base };
                    Cell::Value(Value::Int(b as i64 + 1))
                },
                WinFunc::Lag(k) => if i >= k as usize { arg(i - k as usize) } else { null.clone() },
                WinFunc::Lead(k) => if i + (k as usize) < n { arg(i + k as usize) } else { null.clone() },
                WinFunc::FirstValue => nth(&frame_of(i), 0).map_or(null.clone(), arg),
                WinFunc::LastValue => {
                    let f = frame_of(i);
                    size(&f).checked_sub(1).and_then(|k| nth(&f, k)).map_or(null.clone(), arg)
                },
                WinFunc::NthValue(k) => nth(&frame_of(i), k as usize - 1).map_or(null.clone(), arg),
                WinFunc::Agg(f) => {
                    let frame = frame_of(i);
                    let count = over(&nonnull, &frame);
                    match f {
                        AggFunc::Count if w.args.is_empty() => int(size(&frame) as i64),
                        AggFunc::Count => int(count as i64),
                        AggFunc::Min | AggFunc::Max => extreme(&frame).map_or(null.clone(), arg),
                        AggFunc::BoolAnd | AggFunc::BoolOr => {
                            let t = over(&trues, &frame);
                            if count == 0 {
                                null.clone()
                            } else if f == AggFunc::BoolAnd {
                                Cell::Value(Value::Bool(t == count))
                            } else {
                                Cell::Value(Value::Bool(t > 0))
                            }
                        },
                        _ => return None,
                    }
                },
            };
            out[idx[i]] = v;
        }
    }
    Some(out)
}

pub fn windows(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        let mut any = false;
        for sel in q.selects() {
            for item in &sel.items {
                let Expr::Window(w) = item else { continue };
                any = true;
                if matches!(w.func, WinFunc::Agg(AggFunc::Sum | AggFunc::Avg)) {
                    t.inapplicable += 1;
                    continue;
                }
                // The rows as the window reads them, and its values.
                let mut cols: Vec<String> = Vec::new();
                cols.extend(w.partition.iter().map(|e| e.sql(0)));
                cols.extend(w.order.iter().map(|(e, ..)| e.sql(0)));
                cols.extend(w.args.iter().map(|e| e.sql(0)));
                cols.push(item.sql(0));
                let mut sql = format!("{}SELECT {} FROM {}", q.with_sql(), cols.join(", "), sel.from_sql(0));
                if let Some(p) = &sel.filter {
                    sql.push_str(&format!(" WHERE {}", p.sql(0)));
                }
                let Some(rows) = original(s, &sql, t)? else { continue };
                let (np, no, na) = (w.partition.len(), w.order.len(), w.args.len());
                let input: Vec<WindowRow> = rows.iter().map(|r| (r[..np].to_vec(), r[np..np + no].to_vec(), r[np + no..np + no + na].to_vec())).collect();
                let Some(values) = window_values(w, &input) else { continue };
                let got: Rows = rows.iter().map(|r| r[..np + no + na].iter().cloned().chain(std::iter::once(r[np + no + na].clone())).collect()).collect();
                let want: Rows = rows.iter().zip(values).map(|(r, v)| r[..np + no + na].iter().cloned().chain(std::iter::once(v)).collect()).collect();
                agree(t, "window-def", Mode::Multiset, &sql, &got, "(computed from the rows)", &Ok(want))?;
            }
        }
        if !any {
            t.inapplicable += 1;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- generic

/// The type names a typed literal may carry.
const TYPE_NAMES: [&str; 11] = ["int2", "int4", "int8", "float4", "float8", "numeric", "text", "bool", "bytea", "date", "timestamp"];

/// `sql` with each typed literal (`'..'::type`, not NULL) replaced by a
/// parameter `$k`, identical literals by the same one, and the parameters'
/// literals (`'..'`) and types, in order. Quotes in a literal are doubled,
/// so a scan from the start finds every literal's end; quoted text without
/// a cast (LIKE and regular expression patterns) stays.
pub fn parameterize(sql: &str) -> (String, Vec<(String, &'static str)>) {
    let b = sql.as_bytes();
    let (mut out, mut params) = (String::with_capacity(sql.len()), Vec::<(String, &'static str)>::new());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\'' {
            let start = i;
            while i < b.len() && b[i] != b'\'' {
                i += 1;
            }
            out.push_str(&sql[start..i]);
            continue;
        }
        let start = i;
        i += 1;
        loop {
            if i >= b.len() {
                break;
            }
            if b[i] == b'\'' {
                if b.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                i += 1;
                break;
            }
            i += 1;
        }
        let lit = &sql[start..i];
        let ty = sql[i..].strip_prefix("::").and_then(|rest| {
            TYPE_NAMES.iter().copied().filter(|t| rest.starts_with(t) && !rest[t.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).max_by_key(|t| t.len())
        });
        match ty {
            Some(t) => {
                let k = match params.iter().position(|(l, pt)| l == lit && *pt == t) {
                    Some(k) => k,
                    None => {
                        params.push((lit.to_string(), t));
                        params.len() - 1
                    },
                };
                out.push_str(&format!("${}", k + 1));
                i += 2 + t.len();
            },
            None => out.push_str(lit),
        }
    }
    (out, params)
}

/// Run a parameterized query as a prepared statement under a plan cache
/// mode, then deallocate it.
fn run_prepared(s: &mut Session, sql: &str, params: &[(String, &str)], mode: &str) -> Outcome {
    let types = params.iter().map(|(_, t)| *t).collect::<Vec<_>>().join(", ");
    let decl = if params.is_empty() { String::new() } else { format!("({types})") };
    s.execute(&format!("PREPARE ephyra_p{decl} AS {sql}"))?;
    let args = if params.is_empty() { String::new() } else { format!("({})", params.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", ")) };
    let out = s.execute(&format!("SET plan_cache_mode = {mode}")).and_then(|_| run(s, &format!("EXECUTE ephyra_p{args}")));
    let end = s.execute("DEALLOCATE ephyra_p; RESET plan_cache_mode");
    match (out, end) {
        (Err(e), _) | (Ok(_), Err(e)) => Err(e),
        (Ok(rows), Ok(_)) => Ok(rows),
    }
}

pub fn generic(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |s, q, t| {
        let osql = q.sql();
        let (psql, params) = parameterize(&osql);
        if params.is_empty() {
            t.inapplicable += 1;
            return Ok(());
        }
        let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
        for (name, mode) in [("generic-custom", "force_custom_plan"), ("generic-generic", "force_generic_plan")] {
            let variant = run_prepared(s, &psql, &params, mode);
            agree(t, name, mode_of(q), &osql, &orig, &psql, &variant)?;
        }
        Ok(())
    })
}

// ------------------------------------------------------------------ views

const VIEW: &str = "ephyra_v";
const VIEW2: &str = "ephyra_v2";
const VIEW3: &str = "ephyra_v3";
const VIEW4: &str = "ephyra_v4";

/// The text of a view's definition, `pg_get_viewdef(view, pretty)`, with
/// its trailing semicolon.
fn viewdef(s: &mut Session, view: &str, pretty: bool) -> Result<String, String> {
    match s.query(&format!("SELECT pg_get_viewdef('{view}'::regclass, {pretty})"))?.as_slice() {
        [row] => match row.as_slice() {
            [Cell::Value(Value::Text(t))] => Ok(t.clone()),
            other => Err(format!("pg_get_viewdef gives {other:?}")),
        },
        rows => Err(format!("pg_get_viewdef gives {} rows", rows.len())),
    }
}

/// A variant that is a statement whose only requirement is to succeed (an
/// EXPLAIN): counted as a comparison when it does, a failure for any error
/// but the skipped ones.
fn succeeds<T>(t: &mut Tally, name: &'static str, osql: &str, vsql: &str, out: Result<T, String>) -> Result<Option<T>, String> {
    match out {
        Err(e) if lost(&e) => Err(format!("{name}: the server went away running\n  {vsql}\n  {e}")),
        Err(e) if expected_error(&e) => {
            t.error_skips += 1;
            Ok(None)
        },
        Err(e) if known_error(&e) => {
            t.known_skips += 1;
            Ok(None)
        },
        Err(e) if timeout(&e) => {
            t.timeouts += 1;
            Ok(None)
        },
        Err(e) => Err(format!("{name}: the original succeeds, the variant fails\n  original: {osql}\n  variant:  {vsql}\n  {e}")),
        Ok(x) => {
            *t.comparisons.entry(name).or_insert(0) += 1;
            Ok(Some(x))
        },
    }
}

/// The top plan node's actual rows times its loops, from `EXPLAIN (ANALYZE,
/// FORMAT JSON)`: the first `"Actual Rows"` and `"Actual Loops"` of the
/// text are the top node's (a node's own fields come before its `"Plans"`,
/// and a key's quotes cannot occur inside a JSON string, where quotes are
/// escaped).
pub fn top_actual_rows(json: &str) -> Option<f64> {
    let field = |key: &str| -> Option<f64> {
        let at = json.find(&format!("\"{key}\": "))? + key.len() + 4;
        let rest = &json[at..];
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    Some(field("Actual Rows")? * field("Actual Loops")?)
}

/// The text of a one-row, one-column result of a text-like type (EXPLAIN
/// in JSON, XML or YAML, or text in rows).
fn explain_text(rows: &Rows) -> String {
    rows.iter()
        .flat_map(|r| r.iter())
        .map(|c| match c {
            Cell::Value(Value::Text(t)) => t.clone(),
            Cell::Other(_, b) => String::from_utf8_lossy(b).into_owned(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn views(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    let model = final_model(case);
    let schema = Schema::of(&model);
    let partitioned = model.tables.iter().any(|t| t.partitioning.is_some());
    each(case, queries, |s, q, t| {
        let osql = q.sql();
        let Some(orig) = original(s, &osql, t)? else { return Ok(()) };
        let width = q.types(&schema).map_or(0, |ty| ty.len());
        let r = views_of(s, q, t, &osql, &orig, width);
        let dropped = s.execute(&format!("DROP VIEW IF EXISTS {VIEW4}, {VIEW3}, {VIEW2}, {VIEW}"));
        r?;
        dropped.map(|_| ())?;
        explains(s, t, &osql, &orig, partitioned && parallel_material_bug(q))
    })
}

/// Whether a query has the shape of the known crash of EXPLAIN ANALYZE on
/// master since a241bc4fede (module docs): a subquery expression, which
/// over a partitioned table becomes one SubPlan shared by the Parallel
/// Append's children. The caller adds that a table is partitioned. Not once
/// the target has the fix.
pub fn parallel_material_bug(q: &Query) -> bool {
    !crate::fixed("parallel-material-dup-toc") && all_selects(q).iter().any(|sel| sel.exprs().iter().any(|e| !e.subqueries().is_empty()))
}

/// The view checks of one query whose rows are `orig`.
fn views_of(s: &mut Session, q: &Query, t: &mut Tally, osql: &str, orig: &Rows, width: usize) -> Result<(), String> {
    // Named columns: the items' own names may repeat (`a0.c1, a1.c1`), which
    // a view refuses.
    let cols = (0..width).map(|k| format!("v{k}")).collect::<Vec<_>>().join(", ");
    let create = format!("CREATE VIEW {VIEW}({cols}) AS {osql}");
    s.execute(&create).map_err(|e| format!("views: CREATE VIEW fails on a query that runs\n  {create}\n  {e}"))?;
    let vsql = format!("SELECT * FROM {VIEW}");
    agree(t, "views-select", Mode::Multiset, osql, orig, &vsql, &run(s, &vsql))?;
    let pretty = viewdef(s, VIEW, true)?;
    let raw = viewdef(s, VIEW, false)?;
    // Known deparse bugs: on a query that has the shape of one, the text it
    // garbles still runs, and a lost server is still a failure, but an error
    // or other rows are a known-bug skip. [`sublink_paren_bug`] garbles the
    // pretty text only: the raw text is checked as always, and the fixed
    // point starts from it. [`negated_sort_constant_bug`] garbles both,
    // and the fixed point (which parses the text again) is skipped.
    let paren = sublink_paren_bug(q);
    let negated = negated_sort_constant_bug(q);
    for (name, def) in [("views-viewdef-pretty", &pretty), ("views-viewdef", &raw)] {
        let dsql = def.trim().trim_end_matches(';').to_string();
        let out = run(s, &dsql);
        let garbled = negated || paren && name == "views-viewdef-pretty";
        if garbled && out.as_ref().map_or_else(|e| !lost(e), |rows| !same(Mode::Multiset, orig, rows)) {
            t.known_skips += 1;
            continue;
        }
        agree(t, name, Mode::Multiset, osql, orig, &dsql, &out)?;
    }
    if negated {
        t.known_skips += 1;
        return Ok(());
    }
    // The deparsed text is a fixed point once parsed again: a view made
    // from it (v2) deparses to text whose view (v3) deparses to the same
    // text, or, after one more round (v4), that text. Not v against v2:
    // reparsing normalizes what the first text keeps (pretty printing
    // drops the parentheses of nested ANDs, which then parse flat; `- 2`
    // parses as the constant -2; ORDER BY terms by position that name
    // equal items become one term). Nor always v2 against v3: an item
    // `- 2` named `?column?` deparses without an alias as `'-2'::integer`,
    // which parses named `int4`, and the next text has `AS int4`.
    let create_from = |s: &mut Session, view: &str, text: &str| -> Result<(String, String), String> {
        let create = format!("CREATE VIEW {view} AS {}", text.trim().trim_end_matches(';'));
        s.execute(&create).map_err(|e| format!("views-fixpoint: CREATE VIEW fails on the deparsed text\n  original: {osql}\n  {create}\n  {e}"))?;
        Ok((viewdef(s, view, true)?, viewdef(s, view, false)?))
    };
    let pick = |d: &(String, String)| if paren { d.1.clone() } else { d.0.clone() };
    let second = create_from(s, VIEW2, &pick(&(pretty.clone(), raw.clone())))?;
    let mut third = create_from(s, VIEW3, &pick(&second))?;
    let mut prev = second;
    if third != prev {
        let fourth = create_from(s, VIEW4, &pick(&third))?;
        prev = std::mem::replace(&mut third, fourth);
    }
    for (form, a, b) in [("pretty", &prev.0, &third.0), ("raw", &prev.1, &third.1)] {
        *t.comparisons.entry("views-fixpoint").or_insert(0) += 1;
        if a != b {
            return Err(format!("views-fixpoint: the deparsed text is no fixed point ({form})\n  original: {osql}\n  first:  {pretty}\n  before: {a}\n  after:  {b}"));
        }
    }
    Ok(())
}

/// EXPLAIN of a query whose rows are `orig`: VERBOSE in every format must
/// succeed (it deparses every expression of the plan), and ANALYZE's top
/// node must have produced the query's rows.
fn explains(s: &mut Session, t: &mut Tally, osql: &str, orig: &Rows, serial: bool) -> Result<(), String> {
    // Each format with other options: what the settings are, planner
    // memory, the plan planned as generic.
    for (name, format) in [("explain-text", "FORMAT TEXT"), ("explain-json", "SETTINGS, FORMAT JSON"), ("explain-xml", "MEMORY, FORMAT XML"), ("explain-yaml", "GENERIC_PLAN, FORMAT YAML")] {
        let esql = format!("EXPLAIN (COSTS OFF, VERBOSE, {format}) {osql}");
        succeeds(t, name, osql, &esql, run(s, &esql))?;
    }
    // ANALYZE in text, which prints each node's run-time details (sorts,
    // hashes, buffers, WAL) its own way, must succeed too.
    let tsql = format!("EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, BUFFERS ON, WAL ON, FORMAT TEXT) {osql}");
    if !serial {
        succeeds(t, "explain-analyze-text", osql, &tsql, run(s, &tsql))?;
    }
    // Buffers, WAL, memory and the serialized output counted too: the top
    // node's fields still come first.
    let asql = format!("EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY ON, BUFFERS ON, WAL ON, MEMORY ON, SERIALIZE TEXT, FORMAT JSON) {osql}");
    // The known crash ([`parallel_material_bug`]): such a query is analyzed
    // without parallel workers.
    if serial {
        s.execute("SET max_parallel_workers_per_gather = 0")?;
        t.known_skips += 1;
    }
    let out = run(s, &asql);
    if serial {
        s.execute("RESET max_parallel_workers_per_gather")?;
    }
    let Some(rows) = succeeds(t, "explain-analyze-rows", osql, &asql, out)? else { return Ok(()) };
    let json = explain_text(&rows);
    let Some(actual) = top_actual_rows(&json) else {
        return Err(format!("explain-analyze-rows: no top node rows in\n  {asql}\n  {json}"));
    };
    if actual.round() as usize != orig.len() {
        return Err(format!("explain-analyze-rows: the top node has {actual} rows, the query {}\n  {asql}\n  {}", orig.len(), &json[..json.len().min(3000)]));
    }
    Ok(())
}

/// How often drawn queries fail outright (other than expected errors):
/// the generator's validity, measured on PostgreSQL.
pub fn query_errors(case: &Case, queries: &[Query]) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    each(case, queries, |s, q, _| {
        let sql = q.sql();
        if let Err(e) = run(s, &sql) {
            if !expected_error(&e) && !timeout(&e) {
                out.push((sql, e));
            }
        }
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use ephyra::{Gen, Source};

    use super::*;
    use crate::probes::case_queries;

    #[test]
    fn literals_become_parameters() {
        let (sql, params) = parameterize("SELECT 'a''b'::text, x LIKE 'p%', '5'::int4 + '5'::int4, NULL::int8, '1'::int8, 'x'::timestamp");
        assert_eq!(sql, "SELECT $1, x LIKE 'p%', $2 + $2, NULL::int8, $3, $4");
        assert_eq!(params, [("'a''b'".to_string(), "text"), ("'5'".into(), "int4"), ("'1'".into(), "int8"), ("'x'".into(), "timestamp")]);
    }

    #[test]
    fn numerics_canonicalize_by_value() {
        let n = |x: &str| canon(&Cell::Value(Value::Numeric(Numeric::parse(x).unwrap())));
        assert_eq!(n("0.000"), n("0"));
        assert_eq!(n("1.50"), n("1.5"));
        assert_ne!(n("10"), n("1"));
    }

    /// Every drawn query runs, failing at most with an expected error.
    #[test]
    fn drawn_queries_run() {
        let gen = case_queries();
        let mut failures = Vec::new();
        let mut total = 0;
        for seed in 0..300u64 {
            let mut g = Source::new(seed, (seed % 100) as usize);
            let (case, qs) = gen.generate(&mut g);
            total += qs.len();
            for (sql, e) in query_errors(&case, &qs).unwrap() {
                failures.push(format!("seed {seed}: {e}\n  {sql}"));
            }
        }
        assert!(failures.is_empty(), "{} of {total} queries fail:\n{}", failures.len(), failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
    }

    /// The window values by the definition, a frame as the list of its
    /// positions (O(n²) per partition): the reference [`window_values`]
    /// must agree with.
    fn window_values_naive(w: &WinCall, rows: &[WindowRow]) -> Option<Vec<Cell>> {
        let mut out: Vec<Cell> = vec![Cell::Value(Value::Null); rows.len()];
        let mut parts: BTreeMap<Vec<Canon>, Vec<usize>> = BTreeMap::new();
        for (k, r) in rows.iter().enumerate() {
            parts.entry(r.0.iter().map(canon).collect()).or_default().push(k);
        }
        for (_, mut idx) in parts {
            idx.sort_by(|&a, &b| order_rows(w, &rows[a], &rows[b]));
            let n = idx.len();
            // Peer groups: group[i] and each group's first and last position.
            let mut group = vec![0usize; n];
            let mut bounds: Vec<(usize, usize)> = Vec::new();
            for i in 0..n {
                if i == 0 || !peers(&rows[idx[i - 1]], &rows[idx[i]]) {
                    bounds.push((i, i));
                }
                let g = bounds.len() - 1;
                bounds[g].1 = i;
                group[i] = g;
            }
            let frame_of = |i: usize| -> Vec<usize> {
                let (mode, start, end, exclude) = match &w.frame {
                    Some(f) => (f.mode, f.start, f.end, f.exclude),
                    None => (FrameMode::Range, Bound::UnboundedPreceding, Bound::CurrentRow, Exclude::NoOthers),
                };
                let g = group[i] as i64;
                let at = |b: Bound, is_start: bool| -> i64 {
                    match (mode, b) {
                        (_, Bound::UnboundedPreceding) => 0,
                        (_, Bound::UnboundedFollowing) => n as i64 - 1,
                        (FrameMode::Rows, Bound::Preceding(k)) => i as i64 - i64::from(k),
                        (FrameMode::Rows, Bound::Following(k)) => i as i64 + i64::from(k),
                        (FrameMode::Rows, Bound::CurrentRow) => i as i64,
                        (_, Bound::CurrentRow) => {
                            if is_start {
                                bounds[g as usize].0 as i64
                            } else {
                                bounds[g as usize].1 as i64
                            }
                        },
                        (_, Bound::Preceding(k)) | (_, Bound::Following(k)) => {
                            let h = if matches!(b, Bound::Preceding(_)) { g - i64::from(k) } else { g + i64::from(k) };
                            if h < 0 {
                                if is_start { 0 } else { -1 }
                            } else if h as usize >= bounds.len() {
                                if is_start { n as i64 } else { n as i64 - 1 }
                            } else if is_start {
                                bounds[h as usize].0 as i64
                            } else {
                                bounds[h as usize].1 as i64
                            }
                        },
                    }
                };
                let (lo, hi) = (at(start, true).max(0), at(end, false).min(n as i64 - 1));
                (lo..=hi)
                    .map(|j| j as usize)
                    .filter(|&j| match exclude {
                        Exclude::NoOthers => true,
                        Exclude::CurrentRow => j != i,
                        Exclude::Group => group[j] != group[i],
                        Exclude::Ties => j == i || group[j] != group[i],
                    })
                    .collect()
            };
            let arg = |j: usize| rows[idx[j]].2[0].clone();
            let int = |k: i64| Cell::Value(Value::Int(k));
            let null = Cell::Value(Value::Null);
            for i in 0..n {
                let (first, last) = bounds[group[i]];
                let v = match w.func {
                    WinFunc::RowNumber => int(i as i64 + 1),
                    WinFunc::Rank => int(first as i64 + 1),
                    WinFunc::DenseRank => int(group[i] as i64 + 1),
                    WinFunc::PercentRank => Cell::Value(Value::Float(if n > 1 { first as f64 / (n - 1) as f64 } else { 0.0 })),
                    WinFunc::CumeDist => Cell::Value(Value::Float((last + 1) as f64 / n as f64)),
                    WinFunc::Ntile(k) => {
                        // Buckets of n / k rows, the first n % k one larger.
                        let (k, nn) = (k as usize, n);
                        let (base, extra) = (nn / k, nn % k);
                        let mut b = 0;
                        let mut end = 0;
                        loop {
                            end += base + usize::from(b < extra);
                            b += 1;
                            if i < end || b >= k {
                                break;
                            }
                        }
                        Cell::Value(Value::Int(b as i64))
                    },
                    WinFunc::Lag(k) => if i >= k as usize { arg(i - k as usize) } else { null.clone() },
                    WinFunc::Lead(k) => if i + (k as usize) < n { arg(i + k as usize) } else { null.clone() },
                    WinFunc::FirstValue => frame_of(i).first().map_or(null.clone(), |&j| arg(j)),
                    WinFunc::LastValue => frame_of(i).last().map_or(null.clone(), |&j| arg(j)),
                    WinFunc::NthValue(k) => frame_of(i).get(k as usize - 1).map_or(null.clone(), |&j| arg(j)),
                    WinFunc::Agg(f) => {
                        let frame = frame_of(i);
                        let vals: Vec<Cell> = frame.iter().filter_map(|&j| rows[idx[j]].2.first().cloned()).filter(|c| *c != Cell::Value(Value::Null)).collect();
                        match f {
                            AggFunc::Count if w.args.is_empty() => int(frame.len() as i64),
                            AggFunc::Count => int(vals.len() as i64),
                            AggFunc::Min => vals.iter().min_by(|a, b| cmp_cells(a, b, false, None)).cloned().unwrap_or(null.clone()),
                            AggFunc::Max => vals.iter().max_by(|a, b| cmp_cells(a, b, false, None)).cloned().unwrap_or(null.clone()),
                            AggFunc::BoolAnd | AggFunc::BoolOr => {
                                let bs: Vec<bool> = vals.iter().map(|c| matches!(c, Cell::Value(Value::Bool(true)))).collect();
                                if bs.is_empty() {
                                    null.clone()
                                } else if f == AggFunc::BoolAnd {
                                    Cell::Value(Value::Bool(bs.iter().all(|&b| b)))
                                } else {
                                    Cell::Value(Value::Bool(bs.iter().any(|&b| b)))
                                }
                            },
                            _ => return None,
                        }
                    },
                };
                out[idx[i]] = v;
            }
        }
        Some(out)
    }

    /// `window_values` agrees with the reference on random rows, functions
    /// and frames, and a whole-partition frame over 40 000 rows (a
    /// 200-row table's cross join: minutes with the reference, a runner
    /// hang) takes well under a second.
    #[test]
    fn window_values_agree_with_the_definition_and_scale() {
        use crate::{expr::Frame, model::Type};
        let mut x = 0x9e3779b97f4a7c15u64;
        let mut r = |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        let lit = Expr::Lit(Value::Int(0), Type::Int8);
        let bounds = |k: u64| match k % 5 {
            0 => Bound::UnboundedPreceding,
            1 => Bound::Preceding((k / 5 % 3) as u32),
            2 => Bound::CurrentRow,
            3 => Bound::Following((k / 5 % 3) as u32),
            _ => Bound::UnboundedFollowing,
        };
        let num = |t: &str| Cell::Value(Value::Numeric(Numeric::parse(t).unwrap()));
        let mut checked = 0;
        while checked < 3000 {
            let func = match r(16) {
                0 => WinFunc::RowNumber,
                1 => WinFunc::Rank,
                2 => WinFunc::DenseRank,
                3 => WinFunc::PercentRank,
                4 => WinFunc::CumeDist,
                5 => WinFunc::Ntile(1 + r(7) as u32),
                6 => WinFunc::Lag(r(3) as u32),
                7 => WinFunc::Lead(r(3) as u32),
                8 => WinFunc::FirstValue,
                9 => WinFunc::LastValue,
                10 => WinFunc::NthValue(1 + r(4) as u32),
                11 => WinFunc::Agg(AggFunc::Count),
                12 => WinFunc::Agg(AggFunc::Min),
                13 => WinFunc::Agg(AggFunc::Max),
                14 => WinFunc::Agg(AggFunc::BoolAnd),
                _ => WinFunc::Agg(AggFunc::BoolOr),
            };
            let frame = (r(4) > 0).then(|| Frame {
                mode: [FrameMode::Rows, FrameMode::Range, FrameMode::Groups][r(3) as usize],
                start: bounds(r(15)),
                end: bounds(r(15)),
                exclude: [Exclude::NoOthers, Exclude::CurrentRow, Exclude::Group, Exclude::Ties][r(4) as usize],
            });
            if frame.is_some_and(|f| !f.ok()) {
                continue;
            }
            let (np, no) = (r(2) as usize, r(3) as usize);
            let args = if func == WinFunc::Agg(AggFunc::Count) && r(2) == 0 || matches!(func, WinFunc::RowNumber | WinFunc::Rank | WinFunc::DenseRank | WinFunc::PercentRank | WinFunc::CumeDist | WinFunc::Ntile(_)) {
                vec![]
            } else {
                vec![lit.clone()]
            };
            let w = WinCall { func, args: args.clone(), partition: vec![lit.clone(); np], order: (0..no).map(|_| (lit.clone(), r(2) == 0, [None, Some(true), Some(false)][r(3) as usize])).collect(), frame };
            let bools = matches!(func, WinFunc::Agg(AggFunc::BoolAnd | AggFunc::BoolOr));
            let rows: Vec<WindowRow> = (0..r(14))
                .map(|_| {
                    let key = |r: &mut dyn FnMut(u64) -> u64| if r(5) == 0 { Cell::Value(Value::Null) } else { Cell::Value(Value::Int(r(3) as i64)) };
                    let p = (0..np).map(|_| key(&mut r)).collect();
                    let o = (0..no).map(|_| key(&mut r)).collect();
                    // Equal numerics of different scales: which of equal
                    // values min and max give shows.
                    let a = args.iter().map(|_| match r(5) {
                        0 => Cell::Value(Value::Null),
                        _ if bools => Cell::Value(Value::Bool(r(2) == 0)),
                        1 => num("1.0"),
                        2 => num("1.00"),
                        k => num(&k.to_string()),
                    });
                    (p, o, a.collect())
                })
                .collect();
            let (got, want) = (window_values(&w, &rows), window_values_naive(&w, &rows));
            assert_eq!(format!("{got:?}"), format!("{want:?}"), "{w:?}\n{rows:?}");
            checked += 1;
        }
        // One partition, the default frame: every row's frame is all rows.
        let n = 40_000;
        let rows: Vec<WindowRow> = (0..n).map(|k| (vec![], vec![], vec![Cell::Value(Value::Text(format!("v{}", (k * 7919) % n)))])).collect();
        let w = WinCall { func: WinFunc::Agg(AggFunc::Min), args: vec![lit.clone()], partition: vec![], order: vec![], frame: None };
        let t = std::time::Instant::now();
        let values = window_values(&w, &rows).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
        assert!(values.iter().all(|v| *v == Cell::Value(Value::Text("v0".into()))));
    }
}
