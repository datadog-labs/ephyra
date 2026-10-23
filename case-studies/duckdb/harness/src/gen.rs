// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Random schema + data + query generator (valid by construction, deterministic results by
//! construction), renderer and shrinker.
//!
//! Determinism rules the generator obeys (each one exists because the comparison would otherwise see
//! legitimately different answers from two plans):
//! - LIMIT/OFFSET and order-sensitive window functions (row_number, lag, lead, first/last_value, ntile,
//!   ROWS frames) only appear with an ORDER BY over *all* columns of the row, so ties are identical rows.
//! - No float sources of rounding: DOUBLE values are multiples of 1/4 with small magnitude, and there is
//!   no float division/multiplication, so sums are exact in any order. avg (a division) is wrapped in
//!   floor() so its result is an integer again: a raw avg (e.g. 29.5/3) fed into a further sum/avg
//!   rounds differently depending on the summation order, which legitimately differs between plans.
//! - No -0.0 (it equals 0.0 but prints differently, so GROUP BY/DISTINCT/min/max may keep either):
//!   no negation/multiplication/ceil/round on DOUBLE, no -0.0 literal.
//! - No order-sensitive aggregates (string_agg, list, first, any_value), no volatile functions.

use ephyra::Source;
use std::ops::ControlFlow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Text,
    Bool,
    Dbl,
}
use Ty::*;

impl Ty {
    pub fn sql(self) -> &'static str {
        match self {
            Int => "INTEGER",
            Text => "VARCHAR",
            Bool => "BOOLEAN",
            Dbl => "DOUBLE",
        }
    }
    fn simple(self) -> &'static str {
        match self {
            Int => "0",
            Text => "''",
            Bool => "true",
            Dbl => "0.0::DOUBLE",
        }
    }
}

#[derive(Clone, Debug)]
pub struct E {
    pub ty: Ty,
    pub k: K,
}

#[derive(Clone, Debug)]
pub enum K {
    Col(String),
    Lit(String),
    /// template with `$0`, `$1`, ... placeholders for the arguments
    Op(String, Vec<E>),
    /// aggregate call; template over args, optional FILTER
    Agg(String, Vec<E>, Option<Box<E>>),
    /// window call: template over args then keys; keys (PARTITION/ORDER) are never shrunk
    Win(String, Vec<E>, Vec<E>),
    /// subquery expression: template with `$0..` for args and `$Q` for the query
    Sub(String, Vec<E>, Box<Query>),
}

#[derive(Clone, Debug)]
pub struct Query {
    pub ctes: Vec<(String, String, Query)>, // name, "MATERIALIZED "/"NOT MATERIALIZED "/"", query
    pub body: Body,
    pub order: Option<Order>,
}

#[derive(Clone, Debug)]
pub enum Body {
    Sel(Box<Select>),
    Set(&'static str, Box<Body>, Box<Body>),
}

#[derive(Clone, Debug)]
pub struct Order {
    /// (column index, desc, nulls first); rendering appends any column missing here, so ORDER BY
    /// always covers every output column
    pub keys: Vec<(usize, bool, bool)>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Select {
    pub distinct: bool,
    pub items: Vec<E>,
    pub from: Vec<From>,
    pub where_: Option<E>,
    pub group: Vec<E>,
    pub agg: bool,
    pub having: Option<E>,
}

#[derive(Clone, Debug)]
pub struct From {
    pub join: &'static str,
    pub src: Src,
    pub alias: String,
    pub on: Option<E>,
}

#[derive(Clone, Debug)]
pub enum Src {
    Table(String),
    Sub(Box<Query>),
}

#[derive(Clone, Debug)]
pub enum Fill {
    Rows(Vec<Vec<String>>),
    Range(u32, Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Table {
    pub name: String,
    pub cols: Vec<Ty>,
    pub fill: Fill,
    pub index: Option<usize>,
    pub post: Vec<String>, // DELETE/UPDATE statements run after the fill
}

#[derive(Clone)]
pub enum Variant {
    NoOpt,
    Disable(Vec<&'static str>),
    /// EXPLAIN (SQL) round-trip: the query vs the SQL that `EXPLAIN (SQL)` exports from its optimized plan
    /// (`debug_verify_statement='explain_sql'`), both with these optimizers disabled
    Explain(Vec<&'static str>),
    /// one debug setting from DEBUG_VARIANTS that changes the execution strategy or vector representation
    /// but must not change the result
    Debug(usize),
}

/// (tag, SET, statement restoring the harness default). Each setting is documented in
/// src/common/settings.json and used as a whole-suite config in test/configs/*.json.
pub const DEBUG_VARIANTS: &[(&str, &str, &str)] = &[
    ("dictionary_expression", "SET debug_verify_vector='dictionary_expression'", "RESET debug_verify_vector"),
    ("dictionary_operator", "SET debug_verify_vector='dictionary_operator'", "RESET debug_verify_vector"),
    ("constant_operator", "SET debug_verify_vector='constant_operator'", "RESET debug_verify_vector"),
    ("sequence_operator", "SET debug_verify_vector='sequence_operator'", "RESET debug_verify_vector"),
    ("nested_shuffle", "SET debug_verify_vector='nested_shuffle'", "RESET debug_verify_vector"),
    ("variant_vector", "SET debug_verify_vector='variant_vector'", "RESET debug_verify_vector"),
    ("shredded_vector", "SET debug_verify_vector='shredded_vector'", "RESET debug_verify_vector"),
    ("force_external", "SET debug_force_external=true", "RESET debug_force_external"),
    ("window_separate", "SET debug_window_mode='separate'", "RESET debug_window_mode"),
    ("window_combine", "SET debug_window_mode='combine'", "RESET debug_window_mode"),
    ("no_cross_product", "SET debug_force_no_cross_product=true", "RESET debug_force_no_cross_product"),
    ("verification_projection", "SET debug_verification_projection=true", "RESET debug_verification_projection"),
    ("scan_synchronous", "SET debug_physical_table_scan_execution_strategy='synchronous'", "RESET debug_physical_table_scan_execution_strategy"),
    ("scan_task_executor", "SET debug_physical_table_scan_execution_strategy='task_executor'", "RESET debug_physical_table_scan_execution_strategy"),
    ("fetch_row", "SET debug_force_fetch_row=true", "RESET debug_force_fetch_row"),
    ("no_caching_operators", "SET debug_enable_caching_operators=false", "RESET debug_enable_caching_operators"),
    ("order_sort_key", "SET debug_order_verification='create_sort_key'", "RESET debug_order_verification"),
    ("order_variant", "SET debug_order_verification='variant'", "RESET debug_order_verification"),
    ("threads_1", "SET threads=1", "SET threads=2"),
    ("threads_8", "SET threads=8", "SET threads=2"),
];

/// Self-checking debug settings (they throw on a violated invariant, which the crash oracle reports),
/// turned on for both runs of a case: (SET, reset).
pub const CHECKS: &[(&str, &str)] = &[
    ("SET debug_verify_stats=true", "RESET debug_verify_stats"),
    ("SET debug_verify_serializer=true", "RESET debug_verify_serializer"),
    ("SET debug_verify_column_bindings=true", "RESET debug_verify_column_bindings"),
    ("SET debug_verification_mode='verify_vectors'", "RESET debug_verification_mode"),
    ("SET debug_verification_mode='verify_functions'", "RESET debug_verification_mode"),
    ("SET debug_verify_aggregate_state_export=true", "RESET debug_verify_aggregate_state_export"),
];

/// the statement that undoes `set` (one of the SETs above, or a disabled_optimizers/explain switch)
pub fn reset_of(set: &str) -> Option<&'static str> {
    DEBUG_VARIANTS.iter().find(|d| d.1 == set).map(|d| d.2).or_else(|| CHECKS.iter().find(|c| c.0 == set).map(|c| c.1))
}

fn opt_mask(l: &[&'static str]) -> u64 {
    l.iter().map(|n| OPTIMIZERS.iter().position(|o| o == n).map_or(0, |i| 1u64 << i)).fold(0, |a, b| a | b)
}

/// Compact Debug: ephyra's shrinker only accepts candidates whose Debug text is not longer, so
/// NoOpt must print longer than "every optimizer disabled" for the localization steps to be taken.
impl std::fmt::Debug for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Variant::NoOpt => write!(f, "NoOpt(enable_optimizer=false; all passes off)"),
            Variant::Disable(l) => write!(f, "Disable({:x})", opt_mask(l)),
            Variant::Explain(l) => write!(f, "Explain({:x})", opt_mask(l)),
            Variant::Debug(i) => write!(f, "Debug({i})"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Case {
    pub tables: Vec<Table>,
    pub query: Query,
    pub variant: Variant,
    /// indexes into CHECKS, on for both runs
    pub checks: Vec<usize>,
}

/// Optimizer names (`src/include/duckdb/common/enums/optimizer_type.hpp`, lowercase as listed by
/// `duckdb_optimizers()`).
pub const OPTIMIZERS: &[&str] = &[
    "expression_rewriter", "filter_pullup", "filter_pushdown", "empty_result_pullup", "cte_filter_pusher",
    "regex_range", "in_clause", "join_order", "deliminator", "unnest_rewriter", "unused_columns",
    "statistics_propagation", "common_subexpressions", "common_aggregate", "column_lifetime",
    "build_side_probe_side", "limit_pushdown", "top_n", "compressed_materialization", "duplicate_groups",
    "reorder_filter", "sampling_pushdown", "join_filter_pushdown", "materialized_cte",
    "aggregate_function_rewriter", "late_materialization", "cte_inlining", "row_group_pruner",
    "top_n_window_elimination", "common_subplan", "join_elimination", "window_self_join", "projection_pullup",
    "outer_join_simplification", "window_rewriter", "partitioned_execution", "partial_aggregate_pushdown",
    "grouping_sets", "type_pushdown", "scalar_fn_pushdown", "distinct_aggregate_rewrite", "aggregate_reuse",
    "projection_placement", "redundant_order_keys", "disjunctive_join_rewriter",
];

// ---------------------------------------------------------------- generation

type Scope = Vec<(String, Ty)>;

struct Cx<'a> {
    g: &'a mut Source,
    tables: Vec<(String, Vec<Ty>, u64)>, // name, column types, rows
    ctes: Vec<(String, Vec<Ty>, u64)>, // name, types, work weight of one reference
    alias: usize,
    prod: u64,
}

const PROD_CAP: u64 = 1_000_000;

fn pick<T: Clone>(g: &mut Source, xs: &[T]) -> T {
    g.pick(xs).clone()
}

fn rand_ty(g: &mut Source) -> Ty {
    *g.weighted(&[(4, Int), (3, Text), (2, Bool), (3, Dbl)])
}

pub fn lit(g: &mut Source, ty: Ty) -> String {
    let s: &[&str] = match ty {
        Int => &["0", "1", "2", "3", "-1", "7", "CAST(NULL AS INTEGER)"],
        Text => &["''", "'a'", "'b'", "'ab'", "'A'", "'ba'", "CAST(NULL AS VARCHAR)"],
        Bool => &["true", "false", "CAST(NULL AS BOOLEAN)"],
        Dbl => &["0.0::DOUBLE", "1.0::DOUBLE", "-1.5::DOUBLE", "2.25::DOUBLE", "0.5::DOUBLE", "'nan'::DOUBLE", "CAST(NULL AS DOUBLE)"],
    };
    pick(g, s).to_string()
}

/// a stored value, in the same pools as the literals
fn value(g: &mut Source, ty: Ty) -> String {
    if g.bool_with(0.15) {
        return "NULL".into();
    }
    let s: &[&str] = match ty {
        Int => &["0", "1", "2", "3", "-1", "7"],
        Text => &["''", "'a'", "'b'", "'ab'", "'A'", "'ba'"],
        Bool => &["true", "false"],
        Dbl => &["0.0", "1.0", "-1.5", "2.25", "0.5", "'nan'"],
    };
    pick(g, s).to_string()
}

fn range_expr(g: &mut Source, ty: Ty) -> String {
    let p = pick(g, &[5u32, 7, 11, 1_000_000]);
    let a = pick(g, &[1u32, 3, 7]);
    let b = g.int(0u32..=4);
    let m = pick(g, &[2u32, 3, 5, 10, 100]);
    let v = match ty {
        Int => format!("(i * {a} + {b}) % {m} - 1"),
        Text => format!("['a','b','ab','','A'][((i * {a} + {b}) % 5) + 1]"),
        Bool => format!("(i * {a} + {b}) % 3 = 0"),
        Dbl => format!("((i * {a} + {b}) % {m}) / 4.0 - 1"),
    };
    format!("CASE WHEN i % {p} = 0 THEN NULL ELSE {v} END")
}

fn gen_tables(g: &mut Source) -> Vec<Table> {
    let n = *g.weighted(&[(3, 1usize), (5, 2), (3, 3)]);
    let mut big = false;
    let mut out = vec![];
    for t in 0..n {
        let ncols = g.int(1usize..=4);
        let mut cols = vec![Int];
        for _ in 1..ncols {
            cols.push(rand_ty(g));
        }
        let fill = if g.bool_with(0.3) {
            let mut sizes = vec![5u32, 50, 300];
            if !big {
                sizes.extend([1500, 3000]);
            }
            let rows = pick(g, &sizes);
            big |= rows >= 1000;
            Fill::Range(rows, cols.iter().map(|&c| range_expr(g, c)).collect())
        } else {
            let rows = g.int(0usize..=10);
            Fill::Rows((0..rows).map(|_| cols.iter().map(|&c| value(g, c)).collect()).collect())
        };
        let index = if g.bool_with(0.15) { Some(g.index(cols.len())) } else { None };
        let mut post = vec![];
        if g.bool_with(0.15) {
            post.push(format!("DELETE FROM t{t} WHERE c0 = {}", g.int(-1i32..=2)));
        }
        if g.bool_with(0.15) {
            post.push(format!("UPDATE t{t} SET c0 = c0 + 1 WHERE c0 % 2 = {}", g.int(0i32..=1)));
        }
        out.push(Table { name: format!("t{t}"), cols, fill, index, post });
    }
    out
}

fn gen_variant(g: &mut Source) -> Variant {
    if g.bool_with(0.2) {
        return Variant::Debug(g.index(DEBUG_VARIANTS.len()));
    }
    if g.bool_with(0.35) {
        // exported SQL comes from the optimized plan: disabling passes exports other plan shapes
        let n = *g.weighted(&[(5, 0usize), (3, 1), (2, 3)]);
        let mut v: Vec<&'static str> = (0..n).map(|_| *g.pick(OPTIMIZERS)).collect();
        v.sort();
        v.dedup();
        return Variant::Explain(v);
    }
    if g.bool_with(0.2) {
        Variant::NoOpt
    } else {
        let n = *g.weighted(&[(5, 1usize), (3, 2), (2, 4)]);
        let mut v: Vec<&'static str> = (0..n).map(|_| *g.pick(OPTIMIZERS)).collect();
        v.sort();
        v.dedup();
        Variant::Disable(v)
    }
}

pub fn gen_case(g: &mut Source) -> Case {
    let tables = gen_tables(g);
    let variant = gen_variant(g);
    let checks = if g.bool_with(0.2) { vec![g.index(CHECKS.len())] } else { vec![] };
    let tinfo = tables
        .iter()
        .map(|t| (t.name.clone(), t.cols.clone(), match &t.fill { Fill::Range(n, _) => *n as u64, Fill::Rows(r) => r.len() as u64 }))
        .collect();
    let mut cx = Cx { g, tables: tinfo, ctes: vec![], alias: 0, prod: 1 };
    let (query, _) = gen_query(&mut cx, &vec![], None, 0, true);
    Case { tables, query, variant, checks }
}

fn e(ty: Ty, k: K) -> E {
    E { ty, k }
}
fn op(ty: Ty, t: &str, args: Vec<E>) -> E {
    e(ty, K::Op(t.to_string(), args))
}

#[derive(Clone)]
enum Mode<'a> {
    Row,
    /// aggregate context: leaves are group keys, aggregates (over the row scope) or literals
    Agg(&'a [E]),
}

fn gen_expr(cx: &mut Cx, scope: &Scope, ty: Ty, depth: u32, mode: &Mode, qdepth: u32) -> E {
    // leaves
    if depth == 0 || cx.g.bool_with(0.3) {
        return leaf(cx, scope, ty, mode, qdepth);
    }
    // subquery expressions
    if qdepth < 2 && cx.g.bool_with(0.08) {
        let outer: Scope = match mode {
            Mode::Row => scope.clone(),
            Mode::Agg(keys) => keys.iter().map(|k| (render_e(k), k.ty)).collect(),
        };
        if let Some(x) = gen_subexpr(cx, &outer, ty, depth, mode, qdepth) {
            return x;
        }
    }
    let d = depth - 1;
    if cx.g.bool_with(0.15) {
        return gen_nested_expr(cx, scope, ty, d, mode, qdepth);
    }
    macro_rules! r {
        ($t:expr) => {
            gen_expr(cx, scope, $t, d, mode, qdepth)
        };
    }
    match ty {
        Int => {
            let c = cx.g.int(0u32..=15);
            match c {
                0 => op(Int, "($0 + $1)", vec![r!(Int), r!(Int)]),
                1 => op(Int, "($0 - $1)", vec![r!(Int), r!(Int)]),
                2 => op(Int, "($0 * $1)", vec![r!(Int), r!(Int)]),
                3 => {
                    let k = pick(cx.g, &[2, 3, 7]);
                    op(Int, &format!("($0 % {k})"), vec![r!(Int)])
                }
                4 => op(Int, "abs($0)", vec![r!(Int)]),
                5 => op(Int, "length($0)", vec![r!(Text)]),
                6 => op(Int, "coalesce($0, $1)", vec![r!(Int), r!(Int)]),
                7 => op(Int, "nullif($0, $1)", vec![r!(Int), r!(Int)]),
                8 => op(Int, "greatest($0, $1)", vec![r!(Int), r!(Int)]),
                9 => op(Int, "least($0, $1)", vec![r!(Int), r!(Int)]),
                10 => op(Int, "CAST($0 AS INTEGER)", vec![r!(Bool)]),
                11 => op(Int, "(CASE WHEN $0 THEN $1 ELSE $2 END)", vec![r!(Bool), r!(Int), r!(Int)]),
                12 => op(Int, "(CASE WHEN $0 THEN $1 END)", vec![r!(Bool), r!(Int)]),
                13 => op(Int, "(CASE $0 WHEN $1 THEN $2 ELSE $3 END)", vec![r!(Int), r!(Int), r!(Int), r!(Int)]),
                14 => op(Int, "strpos($0, $1)", vec![r!(Text), r!(Text)]),
                _ => op(Int, "(- $0)", vec![r!(Int)]),
            }
        }
        Text => {
            let c = cx.g.int(0u32..=13);
            match c {
                0 => op(Text, "($0 || $1)", vec![r!(Text), r!(Text)]),
                1 => op(Text, "upper($0)", vec![r!(Text)]),
                2 => op(Text, "lower($0)", vec![r!(Text)]),
                3 => op(Text, "substr($0, CAST($1 AS INTEGER), CAST($2 AS INTEGER))", vec![r!(Text), r!(Int), r!(Int)]),
                4 => op(Text, "replace($0, $1, $2)", vec![r!(Text), r!(Text), r!(Text)]),
                5 => op(Text, "CAST($0 AS VARCHAR)", vec![r!(Int)]),
                6 => op(Text, "left($0, CAST($1 AS INTEGER))", vec![r!(Text), r!(Int)]),
                7 => op(Text, "reverse($0)", vec![r!(Text)]),
                8 => op(Text, "trim($0)", vec![r!(Text)]),
                9 => op(Text, "coalesce($0, $1)", vec![r!(Text), r!(Text)]),
                10 => op(Text, "(CASE WHEN $0 THEN $1 ELSE $2 END)", vec![r!(Bool), r!(Text), r!(Text)]),
                11 => op(Text, "concat($0, $1)", vec![r!(Text), r!(Text)]),
                12 => op(Text, "nullif($0, $1)", vec![r!(Text), r!(Text)]),
                _ => op(Text, "CAST($0 AS VARCHAR)", vec![r!(Bool)]),
            }
        }
        Bool => {
            let c = cx.g.int(0u32..=15);
            let t = rand_ty(cx.g);
            match c {
                0..=3 => {
                    let o = pick(cx.g, &["=", "<>", "<", "<=", ">", ">="]);
                    op(Bool, &format!("($0 {o} $1)"), vec![r!(t), r!(t)])
                }
                4 => op(Bool, "($0 AND $1)", vec![r!(Bool), r!(Bool)]),
                5 => op(Bool, "($0 OR $1)", vec![r!(Bool), r!(Bool)]),
                6 => op(Bool, "(NOT $0)", vec![r!(Bool)]),
                7 => op(Bool, "($0 IS NULL)", vec![r!(t)]),
                8 => op(Bool, "($0 IS NOT NULL)", vec![r!(t)]),
                9 => {
                    let o = pick(cx.g, &["IS DISTINCT FROM", "IS NOT DISTINCT FROM"]);
                    op(Bool, &format!("($0 {o} $1)"), vec![r!(t), r!(t)])
                }
                10 => op(Bool, "($0 BETWEEN $1 AND $2)", vec![r!(t), r!(t), r!(t)]),
                11 => op(Bool, "($0 IN ($1, $2))", vec![r!(t), r!(t), r!(t)]),
                12 => op(Bool, "($0 NOT IN ($1, $2, $3))", vec![r!(t), r!(t), r!(t), r!(t)]),
                13 => {
                    let p = pick(cx.g, &["'a%'", "'%b'", "'_'", "'%'", "'a_'", "'%a%'"]);
                    let o = pick(cx.g, &["LIKE", "NOT LIKE", "ILIKE"]);
                    op(Bool, &format!("($0 {o} {p})"), vec![r!(Text)])
                }
                14 => {
                    let f = pick(cx.g, &["starts_with", "contains", "suffix"]);
                    op(Bool, &format!("{f}($0, $1)"), vec![r!(Text), r!(Text)])
                }
                _ => op(Bool, "coalesce($0, $1)", vec![r!(Bool), r!(Bool)]),
            }
        }
        Dbl => {
            let c = cx.g.int(0u32..=8);
            match c {
                0 => op(Dbl, "($0 + $1)", vec![r!(Dbl), r!(Dbl)]),
                1 => op(Dbl, "($0 - $1)", vec![r!(Dbl), r!(Dbl)]),
                2 => op(Dbl, "abs($0)", vec![r!(Dbl)]),
                3 => op(Dbl, "CAST($0 AS DOUBLE)", vec![r!(Int)]),
                4 => op(Dbl, "floor($0)", vec![r!(Dbl)]),
                5 => op(Dbl, "coalesce($0, $1)", vec![r!(Dbl), r!(Dbl)]),
                6 => op(Dbl, "(CASE WHEN $0 THEN $1 ELSE $2 END)", vec![r!(Bool), r!(Dbl), r!(Dbl)]),
                7 => op(Dbl, "greatest($0, $1)", vec![r!(Dbl), r!(Dbl)]),
                _ => op(Dbl, "least($0, $1)", vec![r!(Dbl), r!(Dbl)]),
            }
        }
    }
}

/// Scalar functions that go through LIST/STRUCT/DATE/DECIMAL values (list and struct literals, lambdas,
/// nested comparisons, date parts) but return one of the four column types, so they combine with every
/// other construct. Deterministic: no list built from rows without list_sort, no float rounding.
fn gen_nested_expr(cx: &mut Cx, scope: &Scope, ty: Ty, d: u32, mode: &Mode, qdepth: u32) -> E {
    macro_rules! r {
        ($t:expr) => {
            gen_expr(cx, scope, $t, d, mode, qdepth)
        };
    }
    // no subqueries: inside a lambda body they are rejected by the binder
    macro_rules! r_nosub {
        ($t:expr) => {
            gen_expr(cx, scope, $t, d, mode, 2)
        };
    }
    match ty {
        Int => match cx.g.int(0u32..=10) {
            0 => op(Int, "CAST(len(list_filter([$0, $1, $2], lambda x: x IS NOT NULL)) AS INTEGER)", vec![r!(Int), r!(Int), r!(Int)]),
            1 => op(Int, "CAST(list_sum([$0, $1]) AS INTEGER)", vec![r!(Int), r!(Int)]),
            2 => op(Int, "[$0, $1, $2][CAST($3 % 4 AS INTEGER)]", vec![r!(Int), r!(Int), r!(Int), r!(Int)]),
            3 => op(Int, "list_position([$0, $1], $2)", vec![r!(Int), r!(Int), r!(Int)]),
            4 => op(Int, "struct_pack(a := $0, b := $1).a", vec![r!(Int), r!(Text)]),
            5 => op(Int, "CAST(date_part('day', DATE '2024-02-27' + CAST($0 AS INTEGER)) AS INTEGER)", vec![r!(Int)]),
            6 => op(Int, "CAST(bit_count($0) AS INTEGER)", vec![r!(Int)]),
            7 => op(Int, "CAST(CAST($0 AS DECIMAL(9,2)) * 2 AS INTEGER)", vec![r!(Int)]),
            8 => op(Int, "CAST(list_unique([$0, $1, $2]) AS INTEGER)", vec![r!(Int), r!(Int), r!(Int)]),
            9 => op(Int, "CAST(len(string_split($0, 'a')) AS INTEGER)", vec![r!(Text)]),
            _ => op(Int, "list_max(list_transform([$0, $1], lambda x: x + $2))", vec![r!(Int), r!(Int), r_nosub!(Int)]),
        },
        Text => match cx.g.int(0u32..=9) {
            0 => op(Text, "array_to_string(list_transform([$0, $1], lambda x: x || 'z'), ',')", vec![r!(Text), r!(Text)]),
            1 => op(Text, "array_to_string(list_sort(string_split($0, 'a')), '|')", vec![r!(Text)]),
            2 => op(Text, "regexp_replace($0, 'a+', $1)", vec![r!(Text), r!(Text)]),
            3 => op(Text, "lpad($0, 3, 'x')", vec![r!(Text)]),
            4 => op(Text, "strftime(DATE '2024-02-27' + CAST($0 AS INTEGER), '%m-%d')", vec![r!(Int)]),
            5 => op(Text, "CAST([$0, $1] AS VARCHAR)", vec![r!(Int), r!(Int)]),
            6 => op(Text, "CAST(struct_pack(k := $0, v := $1) AS VARCHAR)", vec![r!(Text), r!(Bool)]),
            7 => op(Text, "CAST(CAST($0 AS DECIMAL(6,2)) * 0.25 AS VARCHAR)", vec![r!(Int)]),
            8 => op(Text, "dayname(DATE '2024-02-27' + CAST($0 AS INTEGER))", vec![r!(Int)]),
            _ => op(Text, "printf('%s-%d', $0, $1)", vec![r!(Text), r!(Int)]),
        },
        Bool => {
            let t = rand_ty(cx.g);
            let t2 = rand_ty(cx.g);
            let o = pick(cx.g, &["=", "<>", "<", "<=", ">", ">=", "IS DISTINCT FROM", "IS NOT DISTINCT FROM"]);
            match cx.g.int(0u32..=7) {
                0 => op(Bool, "list_contains([$0, $1], $2)", vec![r!(t), r!(t), r!(t)]),
                1 => op(Bool, &format!("([$0, $1] {o} [$2, $3])"), vec![r!(t), r!(t), r!(t), r!(t)]),
                2 => op(Bool, &format!("(struct_pack(a := $0, b := $1) {o} struct_pack(a := $2, b := $3))"), vec![r!(t), r!(t2), r!(t), r!(t2)]),
                3 => op(Bool, &format!("(($0, $1) {o} ($2, $3))"), vec![r!(t), r!(t2), r!(t), r!(t2)]),
                4 => op(Bool, "regexp_matches($0, 'a+b?')", vec![r!(Text)]),
                5 => op(Bool, "isnan($0)", vec![r!(Dbl)]),
                6 => op(Bool, "($0 SIMILAR TO 'a.*')", vec![r!(Text)]),
                _ => op(Bool, "list_has_any([$0], [$1, $2])", vec![r!(t), r!(t), r!(t)]),
            }
        }
        Dbl => match cx.g.int(0u32..=3) {
            0 => op(Dbl, "CAST(CAST($0 AS DECIMAL(9,2)) * 0.25 AS DOUBLE)", vec![r!(Int)]),
            1 => op(Dbl, "list_max([$0, $1])", vec![r!(Dbl), r!(Dbl)]),
            2 => op(Dbl, "list_sum([$0, $1, $2])", vec![r!(Dbl), r!(Dbl), r!(Dbl)]),
            _ => op(Dbl, "[$0, $1][CAST($2 % 3 AS INTEGER)]", vec![r!(Dbl), r!(Dbl), r!(Int)]),
        },
    }
}

fn leaf(cx: &mut Cx, scope: &Scope, ty: Ty, mode: &Mode, qdepth: u32) -> E {
    match mode {
        Mode::Row => {
            let cols: Vec<&(String, Ty)> = scope.iter().filter(|c| c.1 == ty).collect();
            if !cols.is_empty() && cx.g.bool_with(0.75) {
                let c = cols[cx.g.index(cols.len())];
                return e(ty, K::Col(c.0.clone()));
            }
            e(ty, K::Lit(lit(cx.g, ty)))
        }
        Mode::Agg(keys) => {
            let ks: Vec<&E> = keys.iter().filter(|k| k.ty == ty).collect();
            if !ks.is_empty() && cx.g.bool_with(0.4) {
                return ks[cx.g.index(ks.len())].clone();
            }
            if cx.g.bool_with(0.8) {
                return gen_agg(cx, scope, ty, qdepth);
            }
            e(ty, K::Lit(lit(cx.g, ty)))
        }
    }
}

fn gen_agg(cx: &mut Cx, scope: &Scope, ty: Ty, qdepth: u32) -> E {
    let (t, args): (&str, Vec<Ty>) = match ty {
        Int => {
            let a = rand_ty(cx.g);
            pick(
                cx.g,
                &[
                    ("count(*)", vec![]),
                    ("count($0)", vec![a]),
                    ("count(DISTINCT $0)", vec![a]),
                    ("sum($0)", vec![Int]),
                    ("sum(DISTINCT $0)", vec![Int]),
                    ("min($0)", vec![Int]),
                    ("max($0)", vec![Int]),
                    ("list_sort(list(DISTINCT $0)$F)[1]", vec![Int]),
                    ("quantile_disc($0, 0.5)", vec![Int]),
                    ("bit_or($0)", vec![Int]),
                    ("(min(struct_pack(a := $0, b := $1))$F).b", vec![Int, Int]),
                ],
            )
        }
        Text => pick(
            cx.g,
            &[
                ("min($0)", vec![Text]),
                ("max($0)", vec![Text]),
                ("max(DISTINCT $0)", vec![Text]),
                ("array_to_string(list_sort(list(DISTINCT $0)$F), ',')", vec![Text]),
                ("(max(struct_pack(a := $0, b := $1))$F).b", vec![Int, Text]),
            ],
        ),
        Bool => pick(cx.g, &[("bool_and($0)", vec![Bool]), ("bool_or($0)", vec![Bool]), ("min($0)", vec![Bool])]),
        Dbl => pick(
            cx.g,
            &[
                ("avg($0)", vec![Int]),
                ("avg($0)", vec![Dbl]),
                ("sum($0)", vec![Dbl]),
                ("min($0)", vec![Dbl]),
                ("max($0)", vec![Dbl]),
                ("median($0)", vec![Int]),
                ("quantile_disc($0, 0.25)", vec![Dbl]),
            ],
        ),
    };
    // min/max over a STRUCT copies its arguments, which fails for subqueries
    // ("Serialization Error: Cannot copy BoundSubqueryExpression")
    let aq = if t.contains("struct_pack") { 2 } else { qdepth };
    let args = args.into_iter().map(|a| gen_expr(cx, scope, a, 2, &Mode::Row, aq)).collect();
    let filter = if cx.g.bool_with(0.15) { Some(Box::new(gen_expr(cx, scope, Bool, 2, &Mode::Row, qdepth))) } else { None };
    let a = e(ty, K::Agg(t.to_string(), args, filter));
    if t.starts_with("avg") {
        // see the module doc: keep DOUBLE values exact
        return e(ty, K::Op("floor($0)".into(), vec![a]));
    }
    a
}

/// `allow_sensitive`: at most one order-sensitive window per SELECT. Two of them (or ROWS frames) may
/// pair their values differently across tied identical rows, so the row multiset would depend on the plan.
fn gen_window(cx: &mut Cx, local: &Scope, full: &Scope, ty: Ty, qdepth: u32, allow_sensitive: bool) -> (E, bool) {
    // (template, arg types, order-sensitive)
    let (mut f, mut args, mut sensitive): (&str, Vec<Ty>, bool) = match ty {
        Int => pick(
            cx.g,
            &[
                ("row_number()", vec![], true),
                ("rank()", vec![], false),
                ("dense_rank()", vec![], false),
                ("count(*)", vec![], false),
                ("sum($0)", vec![Int], false),
                ("min($0)", vec![Int], false),
                ("max($0)", vec![Int], false),
                ("lag($0)", vec![Int], true),
                ("lead($0, 2)", vec![Int], true),
                ("first_value($0)", vec![Int], true),
                ("last_value($0)", vec![Int], true),
                ("ntile(2)", vec![], true),
            ],
        ),
        Text => pick(cx.g, &[("min($0)", vec![Text], false), ("max($0)", vec![Text], false), ("lag($0)", vec![Text], true), ("first_value($0)", vec![Text], true)]),
        Bool => pick(cx.g, &[("bool_or($0)", vec![Bool], false), ("lead($0)", vec![Bool], true)]),
        Dbl => pick(cx.g, &[("avg($0)", vec![Int], false), ("sum($0)", vec![Dbl], false), ("lag($0)", vec![Dbl], true)]),
    };
    if sensitive && !allow_sensitive {
        (f, args, sensitive) = match ty {
            Int => ("count(*)", vec![], false),
            Text => ("max($0)", vec![Text], false),
            Bool => ("bool_or($0)", vec![Bool], false),
            Dbl => ("sum($0)", vec![Dbl], false),
        };
    }
    let args: Vec<E> = args.into_iter().map(|a| gen_expr(cx, full, a, 2, &Mode::Row, qdepth)).collect();
    let n = args.len();
    let mut keys = vec![];
    let mut t = format!("{f} OVER (");
    if cx.g.bool_with(0.5) {
        let c = &local[cx.g.index(local.len())];
        t.push_str(&format!("PARTITION BY ${n} "));
        keys.push(e(c.1, K::Col(c.0.clone())));
    }
    // order: all row columns (total up to identical rows), one column, or none
    let mode = if sensitive { 2 } else { *cx.g.weighted(&[(2, 0), (1, 1), (2, 2)]) };
    let mut ord = vec![];
    if mode == 1 {
        ord.push(local[cx.g.index(local.len())].clone());
    } else if mode == 2 {
        ord = local.clone();
        // rotate so the leading key varies
        let r = cx.g.index(ord.len());
        ord.rotate_left(r);
    }
    if !ord.is_empty() {
        t.push_str("ORDER BY ");
        for (i, c) in ord.iter().enumerate() {
            let dir = pick(cx.g, &["", " DESC", " NULLS FIRST", " DESC NULLS LAST"]);
            t.push_str(&format!("{}${}{dir}", if i > 0 { ", " } else { "" }, n + keys.len()));
            keys.push(e(c.1, K::Col(c.0.clone())));
        }
    }
    if mode == 2 && !sensitive && allow_sensitive && cx.g.bool_with(0.4) {
        sensitive = true;
        let fr = pick(
            cx.g,
            &[
                " ROWS BETWEEN 1 PRECEDING AND CURRENT ROW",
                " ROWS BETWEEN UNBOUNDED PRECEDING AND 1 FOLLOWING",
                " ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING",
                " ROWS BETWEEN 2 PRECEDING AND 1 PRECEDING",
            ],
        );
        t.push_str(fr);
    }
    t.push(')');
    if f.starts_with("avg") {
        // see the module doc: keep DOUBLE values exact
        t = format!("floor({t})");
    }
    (e(ty, K::Win(t, args, keys)), sensitive)
}

fn gen_subexpr(cx: &mut Cx, outer: &Scope, ty: Ty, depth: u32, mode: &Mode, qdepth: u32) -> Option<E> {
    let d = depth.saturating_sub(1);
    match ty {
        Bool => {
            let c = cx.g.int(0u32..=3);
            match c {
                0 | 1 => {
                    let (q, _) = gen_query(cx, outer, None, qdepth + 1, false);
                    let t = if c == 0 { "EXISTS ($Q)" } else { "(NOT EXISTS ($Q))" };
                    Some(e(Bool, K::Sub(t.into(), vec![], Box::new(q))))
                }
                _ => {
                    let t = rand_ty(cx.g);
                    let lhs = gen_expr(cx, outer_scope_for(mode, outer), t, d, mode, qdepth);
                    let (q, _) = gen_query(cx, outer, Some(vec![t]), qdepth + 1, false);
                    let tmpl = pick(cx.g, &["($0 IN ($Q))", "($0 NOT IN ($Q))", "($0 > ANY ($Q))", "($0 = ALL ($Q))", "($0 <= ANY ($Q))"]);
                    Some(e(Bool, K::Sub(tmpl.into(), vec![lhs], Box::new(q))))
                }
            }
        }
        _ => {
            // scalar subquery: a global aggregate, so exactly one row
            let (q, _) = gen_query_agg1(cx, outer, ty, qdepth + 1);
            Some(e(ty, K::Sub("($Q)".into(), vec![], Box::new(q))))
        }
    }
}

fn outer_scope_for<'a>(_mode: &Mode, outer: &'a Scope) -> &'a Scope {
    outer
}

fn gen_src(cx: &mut Cx, qdepth: u32) -> (Src, Vec<(String, Ty)>) {
    if qdepth < 2 && cx.g.bool_with(0.15) {
        let (q, tys) = gen_query(cx, &vec![], None, qdepth + 1, false);
        let cols = tys.iter().enumerate().map(|(i, t)| (format!("a{i}"), *t)).collect();
        return (Src::Sub(Box::new(q)), cols);
    }
    let ok_ctes: Vec<usize> = (0..cx.ctes.len()).filter(|&i| cx.ctes[i].2.saturating_mul(cx.prod) <= PROD_CAP).collect();
    if !ok_ctes.is_empty() && cx.g.bool_with(0.4) {
        let (n, tys, w) = cx.ctes[ok_ctes[cx.g.index(ok_ctes.len())]].clone();
        cx.prod = cx.prod.saturating_mul(w);
        let cols = tys.iter().enumerate().map(|(i, t)| (format!("a{i}"), *t)).collect();
        return (Src::Table(n), cols);
    }
    // Bound the work: the product of the sizes of every table reference in the case stays under
    // PROD_CAP (otherwise unoptimized plans, or low-cardinality joins, take seconds).
    let w = |i: usize, cx: &Cx| cx.tables[i].2.max(1);
    let cands: Vec<usize> = (0..cx.tables.len()).filter(|&i| w(i, cx).saturating_mul(cx.prod) <= PROD_CAP).collect();
    if cands.is_empty() {
        // budget spent: an inline VALUES table of 2-3 rows, typed like a random table
        let (_, tys, _) = cx.tables[cx.g.index(cx.tables.len())].clone();
        let nrows = cx.g.int(2usize..=3);
        let rows: Vec<String> = (0..nrows)
            .map(|_| format!("({})", tys.iter().map(|&t| format!("CAST({} AS {})", value(cx.g, t), t.sql())).collect::<Vec<_>>().join(", ")))
            .collect();
        let names: Vec<String> = (0..tys.len()).map(|i| format!("col{i} AS c{i}")).collect();
        cx.prod = cx.prod.saturating_mul(nrows as u64);
        let cols = tys.iter().enumerate().map(|(i, t)| (format!("c{i}"), *t)).collect();
        return (Src::Table(format!("(SELECT {} FROM (VALUES {}))", names.join(", "), rows.join(", "))), cols);
    }
    let i = cands[cx.g.index(cands.len())];
    cx.prod = cx.prod.saturating_mul(w(i, cx));
    let (n, tys, _) = cx.tables[i].clone();
    let cols = tys.iter().enumerate().map(|(i, t)| (format!("c{i}"), *t)).collect();
    (Src::Table(n), cols)
}

fn gen_from(cx: &mut Cx, qdepth: u32) -> (Vec<From>, Scope) {
    let n = *cx.g.weighted(&[(9, 1usize), (8, 2), (3, 3)]);
    let mut from = vec![];
    let mut local: Scope = vec![];
    for i in 0..n {
        let (src, cols) = gen_src(cx, qdepth);
        cx.alias += 1;
        let alias = format!("x{}", cx.alias);
        let cols: Scope = cols.into_iter().map(|(c, t)| (format!("{alias}.{c}"), t)).collect();
        if i == 0 {
            from.push(From { join: "", src, alias, on: None });
            local.extend(cols);
            continue;
        }
        let join = *cx.g.weighted(&[
            (30, "JOIN"),
            (20, "LEFT JOIN"),
            (8, "RIGHT JOIN"),
            (8, "FULL JOIN"),
            (6, "CROSS JOIN"),
            (8, ","),
            (6, "SEMI JOIN"),
            (6, "ANTI JOIN"),
        ]);
        let on = if join == "CROSS JOIN" || join == "," {
            None
        } else {
            let mut both = local.clone();
            both.extend(cols.iter().cloned());
            let pairs: Vec<(usize, usize)> = (0..local.len())
                .flat_map(|a| (0..cols.len()).map(move |b| (a, b)))
                .filter(|&(a, b)| local[a].1 == cols[b].1)
                .collect();
            let mut cond = if !pairs.is_empty() && cx.g.bool_with(0.75) {
                let (a, b) = pairs[cx.g.index(pairs.len())];
                let o = pick(cx.g, &["=", "=", "=", "<", ">=", "<>", "IS NOT DISTINCT FROM"]);
                op(Bool, &format!("($0 {o} $1)"), vec![e(local[a].1, K::Col(local[a].0.clone())), e(cols[b].1, K::Col(cols[b].0.clone()))])
            } else {
                gen_expr(cx, &both, Bool, 2, &Mode::Row, qdepth + 1) // no subqueries in ON
            };
            if cx.g.bool_with(0.3) {
                let extra = gen_expr(cx, &both, Bool, 2, &Mode::Row, 9);
                cond = op(Bool, "($0 AND $1)", vec![cond, extra]);
            }
            Some(cond)
        };
        if join != "SEMI JOIN" && join != "ANTI JOIN" {
            local.extend(cols);
        }
        from.push(From { join, src, alias, on });
    }
    (from, local)
}

fn gen_select(cx: &mut Cx, outer: &Scope, want: Option<Vec<Ty>>, qdepth: u32, force_agg: bool) -> (Select, Vec<Ty>) {
    let (from, local) = gen_from(cx, qdepth);
    let mut full = local.clone();
    full.extend(outer.iter().cloned());
    let where_ = if cx.g.bool_with(0.55) { Some(gen_expr(cx, &full, Bool, 3, &Mode::Row, qdepth)) } else { None };
    let tys = want.unwrap_or_else(|| {
        let n = *cx.g.weighted(&[(3, 1usize), (4, 2), (3, 3), (2, 4)]);
        (0..n).map(|_| rand_ty(cx.g)).collect()
    });
    let agg = force_agg || cx.g.bool_with(0.3);
    let mut group = vec![];
    let mut having = None;
    let items: Vec<E>;
    if agg {
        if !force_agg && !local.is_empty() {
            let nk = cx.g.int(0usize..=2);
            for _ in 0..nk {
                let c = &local[cx.g.index(local.len())];
                let k = if cx.g.bool_with(0.8) { e(c.1, K::Col(c.0.clone())) } else { gen_expr(cx, &local, c.1, 1, &Mode::Row, 9) };
                // a literal key would be a GROUP BY ordinal
                group.push(if matches!(k.k, K::Lit(_)) { e(c.1, K::Col(c.0.clone())) } else { k });
            }
        }
        let keys = group.clone();
        let m = Mode::Agg(&keys);
        if force_agg {
            // a global aggregate: exactly one row
            items = tys.iter().map(|&t| gen_agg(cx, &local, t, qdepth)).collect();
        } else {
            items = tys.iter().map(|&t| gen_expr(cx, &local, t, 2, &m, qdepth)).collect();
        }
        if !force_agg && cx.g.bool_with(0.3) {
            having = Some(gen_expr(cx, &local, Bool, 2, &m, qdepth));
        }
    } else {
        let mut sensitive_used = false;
        let mut v = vec![];
        for &t in &tys {
            if !local.is_empty() && cx.g.bool_with(0.15) {
                let (w, sens) = gen_window(cx, &local, &full, t, qdepth, !sensitive_used);
                sensitive_used |= sens;
                v.push(w);
            } else {
                v.push(gen_expr(cx, &full, t, 3, &Mode::Row, qdepth));
            }
        }
        items = v;
    }
    let distinct = !force_agg && cx.g.bool_with(0.12);
    (Select { distinct, items, from, where_, group, agg, having }, tys)
}

fn gen_query_agg1(cx: &mut Cx, outer: &Scope, ty: Ty, qdepth: u32) -> (Query, Vec<Ty>) {
    let (s, tys) = gen_select(cx, outer, Some(vec![ty]), qdepth, true);
    (Query { ctes: vec![], body: Body::Sel(Box::new(s)), order: None }, tys)
}

fn gen_order(cx: &mut Cx, n: usize, top: bool) -> Order {
    let mut idx: Vec<usize> = (0..n).collect();
    let r = cx.g.index(n);
    idx.rotate_left(r);
    if n > 1 && cx.g.bool() {
        idx.reverse();
    }
    let keys = idx.into_iter().map(|i| (i, cx.g.bool(), cx.g.bool())).collect();
    let limit = if !top || cx.g.bool_with(0.5) { Some(cx.g.int(0u32..=8)) } else { None };
    let offset = if limit.is_some() && cx.g.bool_with(0.25) { Some(cx.g.int(1u32..=3)) } else { None };
    Order { keys, limit, offset }
}

fn gen_query(cx: &mut Cx, outer: &Scope, want: Option<Vec<Ty>>, qdepth: u32, top: bool) -> (Query, Vec<Ty>) {
    let mut ctes = vec![];
    let saved = cx.ctes.len();
    if top && cx.g.bool_with(0.2) {
        for _ in 0..cx.g.int(1usize..=2) {
            let p0 = cx.prod;
            let (q, tys) = gen_query(cx, &vec![], None, 1, false);
            let w = (cx.prod / p0.max(1)).max(1);
            let name = format!("w{}", cx.ctes.len());
            let m = pick(cx.g, &["", "", "MATERIALIZED ", "NOT MATERIALIZED "]);
            cx.ctes.push((name.clone(), tys, w));
            ctes.push((name, m.to_string(), q));
        }
    }
    let (body, tys) = if qdepth < 2 && cx.g.bool_with(0.12) {
        let (l, tys) = gen_select(cx, outer, want, qdepth, false);
        let (r, _) = gen_select(cx, outer, Some(tys.clone()), qdepth, false);
        let o = pick(cx.g, &["UNION", "UNION ALL", "EXCEPT", "INTERSECT", "EXCEPT ALL", "INTERSECT ALL"]);
        (Body::Set(o, Box::new(Body::Sel(Box::new(l))), Box::new(Body::Sel(Box::new(r)))), tys)
    } else {
        let (s, tys) = gen_select(cx, outer, want, qdepth, false);
        (Body::Sel(Box::new(s)), tys)
    };
    let order = if (top && cx.g.bool_with(0.35)) || (!top && cx.g.bool_with(0.08)) { Some(gen_order(cx, tys.len(), top)) } else { None };
    cx.ctes.truncate(saved);
    (Query { ctes, body, order }, tys)
}

// ---------------------------------------------------------------- rendering

fn subst(t: &str, args: &[String]) -> String {
    // replace $N (multi-digit) left to right
    let mut out = String::new();
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let n: usize = t[i + 1..j].parse().unwrap();
            out.push_str(args.get(n).map(|s| s.as_str()).unwrap_or("NULL"));
            i = j;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

pub fn render_e(x: &E) -> String {
    match &x.k {
        K::Col(c) => c.clone(),
        K::Lit(l) => l.clone(),
        K::Op(t, a) => subst(t, &a.iter().map(render_e).collect::<Vec<_>>()),
        K::Agg(t, a, f) => {
            // `$F` marks where the FILTER clause goes when the aggregate call is not the whole template
            let s = subst(t, &a.iter().map(render_e).collect::<Vec<_>>());
            let fc = f.as_ref().map_or(String::new(), |f| format!(" FILTER (WHERE {})", render_e(f)));
            if s.contains("$F") { s.replace("$F", &fc) } else { s + &fc }
        }
        K::Win(t, a, k) => subst(t, &a.iter().chain(k.iter()).map(render_e).collect::<Vec<_>>()),
        K::Sub(t, a, q) => subst(t, &a.iter().map(render_e).collect::<Vec<_>>()).replace("$Q", &render_q(q)),
    }
}

fn render_sel(s: &Select) -> String {
    let mut o = String::from("SELECT ");
    if s.distinct {
        o.push_str("DISTINCT ");
    }
    let items: Vec<String> = s.items.iter().enumerate().map(|(i, x)| format!("{} AS a{i}", render_e(x))).collect();
    o.push_str(&items.join(", "));
    o.push_str(" FROM ");
    for f in &s.from {
        if !f.join.is_empty() {
            o.push_str(if f.join == "," { ", " } else { " " });
            if f.join != "," {
                o.push_str(f.join);
                o.push(' ');
            }
        }
        match &f.src {
            Src::Table(n) => o.push_str(n),
            Src::Sub(q) => o.push_str(&format!("({})", render_q(q))),
        }
        o.push_str(" AS ");
        o.push_str(&f.alias);
        if let Some(c) = &f.on {
            o.push_str(&format!(" ON {}", render_e(c)));
        } else if !f.join.is_empty() && f.join != "," && f.join != "CROSS JOIN" {
            o.push_str(" ON true");
        }
    }
    if let Some(w) = &s.where_ {
        o.push_str(&format!(" WHERE {}", render_e(w)));
    }
    if !s.group.is_empty() {
        o.push_str(&format!(" GROUP BY {}", s.group.iter().map(render_e).collect::<Vec<_>>().join(", ")));
    }
    if let Some(h) = &s.having {
        o.push_str(&format!(" HAVING {}", render_e(h)));
    }
    o
}

fn render_body(b: &Body) -> String {
    match b {
        Body::Sel(s) => render_sel(s),
        Body::Set(o, l, r) => format!("({}) {o} ({})", render_body(l), render_body(r)),
    }
}

fn body_width(b: &Body) -> usize {
    match b {
        Body::Sel(s) => s.items.len(),
        Body::Set(_, l, _) => body_width(l),
    }
}

pub fn render_q(q: &Query) -> String {
    let mut o = String::new();
    if !q.ctes.is_empty() {
        o.push_str("WITH ");
        let c: Vec<String> = q.ctes.iter().map(|(n, m, q)| format!("{n} AS {m}({})", render_q(q))).collect();
        o.push_str(&c.join(", "));
        o.push(' ');
    }
    o.push_str(&render_body(&q.body));
    if let Some(ord) = &q.order {
        let w = body_width(&q.body);
        let mut keys: Vec<(usize, bool, bool)> = ord.keys.iter().cloned().filter(|k| k.0 < w).collect();
        keys.dedup_by_key(|k| k.0);
        for i in 0..w {
            if !keys.iter().any(|k| k.0 == i) {
                keys.push((i, false, false));
            }
        }
        let ks: Vec<String> = keys
            .iter()
            .map(|(i, d, n)| format!("{}{}{}", i + 1, if *d { " DESC" } else { "" }, if *n { " NULLS FIRST" } else { " NULLS LAST" }))
            .collect();
        o.push_str(&format!(" ORDER BY {}", ks.join(", ")));
        if let Some(l) = ord.limit {
            o.push_str(&format!(" LIMIT {l}"));
        }
        if let Some(f) = ord.offset {
            o.push_str(&format!(" OFFSET {f}"));
        }
    }
    o
}

pub fn render_setup(ts: &[Table]) -> Vec<String> {
    let mut out = vec![];
    for t in ts {
        let cols: Vec<String> = t.cols.iter().enumerate().map(|(i, c)| format!("c{i} {}", c.sql())).collect();
        out.push(format!("CREATE TABLE {}({})", t.name, cols.join(", ")));
        match &t.fill {
            Fill::Rows(rows) => {
                if !rows.is_empty() {
                    let r: Vec<String> = rows.iter().map(|r| format!("({})", r.join(", "))).collect();
                    out.push(format!("INSERT INTO {} VALUES {}", t.name, r.join(", ")));
                }
            }
            Fill::Range(n, ex) => out.push(format!("INSERT INTO {} SELECT {} FROM range({n}) r(i)", t.name, ex.join(", "))),
        }
        if let Some(i) = t.index {
            if i < t.cols.len() {
                out.push(format!("CREATE INDEX {}_i ON {}(c{i})", t.name, t.name));
            }
        }
        out.extend(t.post.iter().cloned());
    }
    out
}

/// statements run before the default-settings query (and still in effect for the variant)
pub fn render_base(c: &Case) -> Vec<String> {
    let mut out: Vec<String> = c.checks.iter().map(|&i| CHECKS[i].0.to_string()).collect();
    match &c.variant {
        Variant::Explain(l) if !l.is_empty() => out.push(format!("SET disabled_optimizers='{}'", l.join(","))),
        _ => {}
    }
    out
}

pub fn render_variant(v: &Variant) -> Vec<String> {
    match v {
        Variant::NoOpt => vec!["SET enable_optimizer=false".into()],
        Variant::Disable(l) => vec![format!("SET disabled_optimizers='{}'", l.join(","))],
        Variant::Explain(_) => vec!["SET debug_verify_statement='explain_sql'".into()],
        Variant::Debug(i) => vec![DEBUG_VARIANTS[*i].1.into()],
    }
}

// ---------------------------------------------------------------- shrinking

fn cands_e(x: &E) -> Vec<E> {
    let mut out = vec![];
    let simple = x.ty.simple();
    if !matches!(&x.k, K::Lit(l) if l == simple) {
        out.push(e(x.ty, K::Lit(simple.into())));
    }
    if !matches!(&x.k, K::Lit(_)) {
        out.push(e(x.ty, K::Lit(format!("CAST(NULL AS {})", x.ty.sql()))));
    }
    let args: &[E] = match &x.k {
        K::Op(_, a) | K::Agg(_, a, _) | K::Win(_, a, _) | K::Sub(_, a, _) => a,
        _ => &[],
    };
    // a same-typed child replaces the node
    for a in args {
        // (never unwrap floor(avg(..)): the floor keeps DOUBLE values exact, see the module doc)
        let avg_floor = matches!(&x.k, K::Op(t, _) if t == "floor($0)") && matches!(&a.k, K::Agg(t, ..) if t.starts_with("avg"));
        if a.ty == x.ty && !matches!(x.k, K::Agg(..) | K::Win(..)) && !avg_floor {
            out.push(a.clone());
        }
    }
    for (i, a) in args.iter().enumerate() {
        for c in cands_e(a) {
            let mut y = x.clone();
            match &mut y.k {
                K::Op(_, v) | K::Agg(_, v, _) | K::Win(_, v, _) | K::Sub(_, v, _) => v[i] = c,
                _ => unreachable!(),
            }
            out.push(y);
        }
    }
    match &x.k {
        K::Agg(t, a, Some(f)) => {
            out.push(e(x.ty, K::Agg(t.clone(), a.clone(), None)));
            for c in cands_e(f) {
                out.push(e(x.ty, K::Agg(t.clone(), a.clone(), Some(Box::new(c)))));
            }
        }
        K::Sub(t, a, q) => {
            for c in cands_q(q) {
                out.push(e(x.ty, K::Sub(t.clone(), a.clone(), Box::new(c))));
            }
        }
        _ => {}
    }
    out
}

fn cands_sel(s: &Select) -> Vec<Select> {
    let mut out = vec![];
    let mut push = |f: &dyn Fn(&mut Select)| {
        let mut y = s.clone();
        f(&mut y);
        out.push(y);
    };
    if s.distinct {
        push(&|y| y.distinct = false);
    }
    if s.where_.is_some() {
        push(&|y| y.where_ = None);
    }
    if s.having.is_some() {
        push(&|y| y.having = None);
    }
    for i in 0..s.group.len() {
        push(&|y| {
            y.group.remove(i);
        });
    }
    if s.items.len() > 1 {
        for i in 0..s.items.len() {
            push(&|y| {
                y.items.remove(i);
            });
        }
    }
    for i in 1..s.from.len() {
        push(&|y| {
            y.from.remove(i);
        });
        if s.from[i].join != "JOIN" && s.from[i].on.is_some() {
            push(&|y| y.from[i].join = "JOIN");
        }
    }
    if s.from.len() > 1 {
        push(&|y| {
            y.from.remove(0);
            y.from[0].join = "";
            y.from[0].on = None;
        });
    }
    for i in 0..s.items.len() {
        for c in cands_e(&s.items[i]) {
            let mut y = s.clone();
            y.items[i] = c;
            out.push(y);
        }
    }
    for i in 0..s.group.len() {
        for c in cands_e(&s.group[i]) {
            let mut y = s.clone();
            y.group[i] = c;
            out.push(y);
        }
    }
    if let Some(w) = &s.where_ {
        for c in cands_e(w) {
            let mut y = s.clone();
            y.where_ = Some(c);
            out.push(y);
        }
    }
    if let Some(w) = &s.having {
        for c in cands_e(w) {
            let mut y = s.clone();
            y.having = Some(c);
            out.push(y);
        }
    }
    for i in 0..s.from.len() {
        if let Some(w) = &s.from[i].on {
            for c in cands_e(w) {
                let mut y = s.clone();
                y.from[i].on = Some(c);
                out.push(y);
            }
        }
        if let Src::Sub(q) = &s.from[i].src {
            for c in cands_q(q) {
                let mut y = s.clone();
                y.from[i].src = Src::Sub(Box::new(c));
                out.push(y);
            }
        }
    }
    out
}

fn cands_body(b: &Body) -> Vec<Body> {
    match b {
        Body::Sel(s) => cands_sel(s).into_iter().map(|s| Body::Sel(Box::new(s))).collect(),
        Body::Set(o, l, r) => {
            let mut out = vec![(**l).clone(), (**r).clone()];
            for c in cands_body(l) {
                out.push(Body::Set(o, Box::new(c), r.clone()));
            }
            for c in cands_body(r) {
                out.push(Body::Set(o, l.clone(), Box::new(c)));
            }
            out
        }
    }
}

fn cands_q(q: &Query) -> Vec<Query> {
    let mut out = vec![];
    if let Some(o) = &q.order {
        out.push(Query { order: None, ..q.clone() });
        if o.offset.is_some() {
            let mut y = q.clone();
            y.order.as_mut().unwrap().offset = None;
            out.push(y);
        }
    }
    for i in 0..q.ctes.len() {
        let mut y = q.clone();
        y.ctes.remove(i);
        out.push(y);
        if !q.ctes[i].1.is_empty() {
            let mut y = q.clone();
            y.ctes[i].1 = String::new();
            out.push(y);
        }
    }
    for b in cands_body(&q.body) {
        out.push(Query { body: b, ..q.clone() });
    }
    for i in 0..q.ctes.len() {
        for c in cands_q(&q.ctes[i].2) {
            let mut y = q.clone();
            y.ctes[i].2 = c;
            out.push(y);
        }
    }
    out
}

pub fn shrink_case(c: &Case, visit: &mut dyn FnMut(Case) -> ControlFlow<()>) -> ControlFlow<()> {
    // localize the optimizer: no optimizer -> every optimizer disabled -> halves -> single ones
    match &c.variant {
        Variant::NoOpt => visit(Case { variant: Variant::Disable(OPTIMIZERS.to_vec()), ..c.clone() })?,
        Variant::Disable(l) if l.len() > 1 => {
            let h = l.len() / 2;
            visit(Case { variant: Variant::Disable(l[..h].to_vec()), ..c.clone() })?;
            visit(Case { variant: Variant::Disable(l[h..].to_vec()), ..c.clone() })?;
            if l.len() <= 4 {
                for o in l {
                    visit(Case { variant: Variant::Disable(vec![o]), ..c.clone() })?;
                }
            }
        }
        Variant::Explain(l) if !l.is_empty() => {
            visit(Case { variant: Variant::Explain(vec![]), ..c.clone() })?;
            if l.len() > 1 {
                for o in l {
                    visit(Case { variant: Variant::Explain(vec![o]), ..c.clone() })?;
                }
            }
        }
        _ => {}
    }
    if !c.checks.is_empty() {
        visit(Case { checks: vec![], ..c.clone() })?;
    }
    for (i, t) in c.tables.iter().enumerate() {
        let mut with = |f: &dyn Fn(&mut Table)| {
            let mut y = c.clone();
            f(&mut y.tables[i]);
            visit(y)
        };
        if !t.post.is_empty() {
            with(&|t| t.post.clear())?;
        }
        if t.index.is_some() {
            with(&|t| t.index = None)?;
        }
        match &t.fill {
            Fill::Range(n, ex) if *n > 1 => {
                let (n, ex) = (*n, ex.clone());
                with(&|t| t.fill = Fill::Range(n / 2, ex.clone()))?;
                if n > 20 {
                    with(&|t| t.fill = Fill::Range(n / 10, ex.clone()))?;
                }
            }
            Fill::Rows(rows) if !rows.is_empty() => {
                let rows = rows.clone();
                if rows.len() > 1 {
                    let h = rows.len() / 2;
                    with(&|t| t.fill = Fill::Rows(rows[..h].to_vec()))?;
                    with(&|t| t.fill = Fill::Rows(rows[h..].to_vec()))?;
                }
                for r in 0..rows.len() {
                    with(&|t| {
                        let mut v = rows.clone();
                        v.remove(r);
                        t.fill = Fill::Rows(v)
                    })?;
                }
            }
            _ => {}
        }
    }
    for q in cands_q(&c.query) {
        visit(Case { query: q, ..c.clone() })?;
    }
    // drop trailing unused tables last (renaming is not needed: names stay t0..)
    ControlFlow::Continue(())
}
