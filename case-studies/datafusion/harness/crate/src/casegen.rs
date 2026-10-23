// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Random case generator: small tables over shared value pools, and typed SQL
//! queries (joins, subqueries, aggregates, windows, set operations) rendered to text.
//! Every query is valid by construction under DataFusion's typing; values are
//! kept small so arithmetic never overflows and comparisons collide often.

use ephyra::Source;

use crate::case::{Case, Col, Table, Ty, Variant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum T {
    Int,
    Float,
    Str,
    Bool,
}

fn t_of(ty: Ty) -> T {
    match ty {
        Ty::I64 | Ty::I32 => T::Int,
        Ty::F64 => T::Float,
        Ty::Str => T::Str,
        Ty::Bool => T::Bool,
    }
}

#[derive(Clone, Debug)]
struct ColRef {
    sql: String,
    t: T,
}

#[derive(Clone, Default)]
struct Scope {
    cols: Vec<ColRef>,
    /// columns of enclosing queries (correlation)
    outer: Vec<ColRef>,
    /// Some(..) while generating in an aggregate context: group-key expressions
    keys: Option<Vec<ColRef>>,
}

pub struct Gen<'a> {
    g: &'a mut Source,
    tables: Vec<Table>,
    alias: usize,
    /// subquery nesting budget
    sub_depth: u32,
}

const STRS: &[&str] = &["a", "b", "A", "ab", "", "b ", "ba"];
const FLOATS: &[&str] = &["0", "1.5", "-2", "3", "0.5", "NaN"];

impl<'a> Gen<'a> {
    fn chance(&mut self, p: f64) -> bool {
        self.g.bool_with(p)
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        self.g.int(lo..=hi)
    }
    fn idx(&mut self, n: usize) -> usize {
        self.g.index(n)
    }
    fn next_alias(&mut self, p: &str) -> String {
        self.alias += 1;
        format!("{p}{}", self.alias)
    }

    // ---------- data ----------

    fn value(&mut self, ty: Ty) -> Option<String> {
        if self.chance(0.15) {
            return None;
        }
        Some(match ty {
            Ty::I64 | Ty::I32 => self.range(-2, 4).to_string(),
            Ty::F64 => FLOATS[self.idx(FLOATS.len() - 1) + usize::from(self.chance(0.1))].to_string(),
            Ty::Str => STRS[self.idx(STRS.len())].to_string(),
            Ty::Bool => self.g.bool().to_string(),
        })
    }

    fn table(&mut self, i: usize) -> Table {
        let ncols = self.range(2, 4) as usize;
        let mut cols = vec![Col { name: "c0".into(), ty: if self.chance(0.8) { Ty::I64 } else { Ty::I32 } }];
        for c in 1..ncols {
            let ty = *self.g.pick(&[Ty::I64, Ty::I64, Ty::I32, Ty::Str, Ty::Str, Ty::F64, Ty::Bool]);
            cols.push(Col { name: format!("c{c}"), ty });
        }
        let nrows = if self.chance(0.05) { 0 } else { self.range(1, 10) as usize };
        let rows: Vec<Vec<Option<String>>> =
            (0..nrows).map(|_| cols.iter().map(|c| c.ty).collect::<Vec<_>>().into_iter().map(|t| self.value(t)).collect()).collect();
        let nparts = self.range(1, 3) as usize;
        let mut parts = vec![0usize; nparts];
        for _ in 0..nrows {
            let p = self.idx(nparts);
            parts[p] += 1;
        }
        let batch = *self.g.pick(&[1usize, 2, 3, 100]);
        Table { name: format!("t{i}"), cols, rows, parts, batch }
    }

    // ---------- expressions ----------

    fn lit(&mut self, t: T) -> String {
        if self.chance(0.07) {
            return match t {
                T::Int => "CAST(NULL AS BIGINT)",
                T::Float => "CAST(NULL AS DOUBLE)",
                T::Str => "CAST(NULL AS VARCHAR)",
                T::Bool => "CAST(NULL AS BOOLEAN)",
            }
            .into();
        }
        match t {
            T::Int => self.range(-2, 4).to_string(),
            T::Float => match FLOATS[self.idx(FLOATS.len() - 1)] {
                s if s.contains('.') => s.to_string(),
                s => format!("{s}.0"),
            },
            T::Str => format!("'{}'", STRS[self.idx(STRS.len())]),
            T::Bool => if self.g.bool() { "true" } else { "false" }.into(),
        }
    }

    fn column(&mut self, t: T, sc: &Scope) -> Option<String> {
        let use_outer = !sc.outer.is_empty() && self.chance(0.3);
        let pool = if use_outer { &sc.outer } else { &sc.cols };
        let cands: Vec<&ColRef> = pool.iter().filter(|c| c.t == t).collect();
        if cands.is_empty() {
            let cands: Vec<&ColRef> = sc.cols.iter().filter(|c| c.t == t).collect();
            if cands.is_empty() {
                return None;
            }
            return Some(cands[self.g.index(cands.len())].sql.clone());
        }
        Some(cands[self.g.index(cands.len())].sql.clone())
    }

    fn leaf(&mut self, t: T, sc: &Scope) -> String {
        if let Some(keys) = &sc.keys {
            let cands: Vec<ColRef> = keys.iter().filter(|c| c.t == t).cloned().collect();
            if !cands.is_empty() && self.chance(0.5) {
                return cands[self.idx(cands.len())].sql.clone();
            }
            if self.chance(0.85) {
                return self.aggregate(t, sc);
            }
            return self.lit(t);
        }
        if self.chance(0.9) {
            if let Some(c) = self.column(t, sc) {
                return c;
            }
        }
        self.lit(t)
    }

    fn row_scope(sc: &Scope) -> Scope {
        Scope { cols: sc.cols.clone(), outer: sc.outer.clone(), keys: None }
    }

    fn aggregate(&mut self, t: T, sc: &Scope) -> String {
        let rs = Self::row_scope(sc);
        let saved = self.sub_depth;
        if self.chance(0.95) {
            self.sub_depth = 0;
        }
        let r = self.aggregate_inner(t, &rs);
        self.sub_depth = saved;
        r
    }

    fn aggregate_inner(&mut self, t: T, rs: &Scope) -> String {
        let rs = rs.clone();
        let call = match t {
            T::Int => match self.idx(7) {
                0 => "count(*)".to_string(),
                1 => format!("count({})", self.expr_any(1, &rs)),
                2 => format!("count(DISTINCT {})", self.expr_any(1, &rs)),
                3 => format!("sum({})", self.expr(T::Int, 1, &rs)),
                4 => format!("sum(DISTINCT {})", self.expr(T::Int, 1, &rs)),
                5 => format!("min({})", self.expr(T::Int, 1, &rs)),
                _ => format!("max({})", self.expr(T::Int, 1, &rs)),
            },
            T::Float => match self.idx(3) {
                0 => format!("avg({})", self.expr(T::Int, 1, &rs)),
                1 => format!("min({})", self.expr(T::Float, 1, &rs)),
                _ => format!("max({})", self.expr(T::Float, 1, &rs)),
            },
            T::Str => match self.idx(5) {
                0 | 3 => format!("min({})", self.expr(T::Str, 1, &rs)),
                1 | 4 => format!("max({})", self.expr(T::Str, 1, &rs)),
                _ => {
                    let e = self.expr(T::Str, 1, &rs);
                    format!("string_agg({e}, ',' ORDER BY {e})")
                }
            },
            T::Bool => match self.idx(2) {
                0 => format!("bool_and({})", self.expr(T::Bool, 1, &rs)),
                _ => format!("bool_or({})", self.expr(T::Bool, 1, &rs)),
            },
        };
        if self.chance(0.2) && !call.starts_with("string_agg") {
            let f = self.expr(T::Bool, 1, &rs);
            format!("{call} FILTER (WHERE {f})")
        } else {
            call
        }
    }

    fn expr_any(&mut self, d: u32, sc: &Scope) -> String {
        let t = *self.g.pick(&[T::Int, T::Int, T::Str, T::Bool, T::Float]);
        self.expr(t, d, sc)
    }

    pub fn expr(&mut self, t: T, d: u32, sc: &Scope) -> String {
        if d == 0 || self.chance(0.3) {
            return self.leaf(t, sc);
        }
        let d1 = d - 1;
        // shared forms
        match self.idx(10) {
            0 => {
                let p = self.expr(T::Bool, d1, sc);
                let a = self.expr(t, d1, sc);
                let b = self.expr(t, d1, sc);
                return format!("CASE WHEN {p} THEN {a} ELSE {b} END");
            }
            1 => {
                let a = self.expr(t, d1, sc);
                let b = self.expr(t, d1, sc);
                return format!("COALESCE({a}, {b})");
            }
            2 if t != T::Bool => {
                let a = self.expr(t, d1, sc);
                let b = self.expr(t, d1, sc);
                return format!("NULLIF({a}, {b})");
            }
            3 if self.sub_depth > 0 && sc.keys.is_none() && self.chance(0.4) => {
                return self.scalar_subquery(t, sc);
            }
            _ => {}
        }
        match t {
            T::Int => match self.idx(10) {
                0 | 1 => format!("({} + {})", self.expr(T::Int, d1, sc), self.expr(T::Int, d1, sc)),
                2 => format!("({} - {})", self.expr(T::Int, d1, sc), self.expr(T::Int, d1, sc)),
                3 => format!("({} * {})", self.expr(T::Int, d1, sc), self.expr(T::Int, d1, sc)),
                4 => format!("({} / NULLIF({}, 0))", self.expr(T::Int, d1, sc), self.expr(T::Int, d1, sc)),
                5 => format!("({} % NULLIF({}, 0))", self.expr(T::Int, d1, sc), self.expr(T::Int, d1, sc)),
                6 => format!("abs({})", self.expr(T::Int, d1, sc)),
                7 => format!("length({})", self.expr(T::Str, d1, sc)),
                8 => {
                    let f = if self.g.bool() { "greatest" } else { "least" };
                    format!("{f}({}, {})", self.expr(T::Int, d1, sc), self.expr(T::Int, d1, sc))
                }
                _ => format!("(- {})", self.expr(T::Int, d1, sc)),
            },
            T::Float => match self.idx(4) {
                0 => format!("({} + {})", self.expr(T::Float, d1, sc), self.expr(T::Float, d1, sc)),
                1 => format!("({} * {})", self.expr(T::Float, d1, sc), self.expr(T::Float, d1, sc)),
                2 => format!("CAST({} AS DOUBLE)", self.expr(T::Int, d1, sc)),
                _ => format!("abs({})", self.expr(T::Float, d1, sc)),
            },
            T::Str => match self.idx(7) {
                0 => format!("upper({})", self.expr(T::Str, d1, sc)),
                1 => format!("lower({})", self.expr(T::Str, d1, sc)),
                2 => format!("({} || {})", self.expr(T::Str, d1, sc), self.expr(T::Str, d1, sc)),
                3 => format!("substr({}, {}, 2)", self.expr(T::Str, d1, sc), self.range(1, 2)),
                4 => format!("trim({})", self.expr(T::Str, d1, sc)),
                5 => format!("CAST({} AS VARCHAR)", self.expr(T::Int, d1, sc)),
                _ => format!("concat({}, {})", self.expr(T::Str, d1, sc), self.expr(T::Str, d1, sc)),
            },
            T::Bool => self.pred(d, sc),
        }
    }

    fn cmp_op(&mut self) -> &'static str {
        *self.g.pick(&["=", "<>", "<", "<=", ">", ">="])
    }

    fn pred(&mut self, d: u32, sc: &Scope) -> String {
        if d == 0 {
            return self.leaf(T::Bool, sc);
        }
        let d1 = d - 1;
        let sub_ok = self.sub_depth > 0 && sc.keys.is_none();
        match self.idx(16) {
            0 | 1 | 2 => {
                let t = *self.g.pick(&[T::Int, T::Int, T::Str, T::Float]);
                let op = self.cmp_op();
                format!("({} {op} {})", self.expr(t, d1, sc), self.expr(t, d1, sc))
            }
            3 | 4 => format!("({} AND {})", self.pred(d1, sc), self.pred(d1, sc)),
            5 | 6 => format!("({} OR {})", self.pred(d1, sc), self.pred(d1, sc)),
            7 => format!("(NOT {})", self.pred(d1, sc)),
            8 => {
                let n = if self.g.bool() { "" } else { "NOT " };
                format!("({} IS {n}NULL)", self.expr_any(d1, sc))
            }
            9 => {
                let t = *self.g.pick(&[T::Int, T::Str, T::Bool]);
                let n = if self.g.bool() { "" } else { "NOT " };
                format!("({} IS {n}DISTINCT FROM {})", self.expr(t, d1, sc), self.expr(t, d1, sc))
            }
            10 => {
                let n = if self.g.bool() { "" } else { "NOT " };
                format!(
                    "({} {n}BETWEEN {} AND {})",
                    self.expr(T::Int, d1, sc),
                    self.expr(T::Int, d1, sc),
                    self.expr(T::Int, d1, sc)
                )
            }
            11 => {
                let t = *self.g.pick(&[T::Int, T::Str]);
                let n = if self.g.bool() { "" } else { "NOT " };
                let k = self.range(1, 4);
                let items: Vec<String> = (0..k).map(|_| self.lit(t)).collect();
                format!("({} {n}IN ({}))", self.expr(t, d1, sc), items.join(", "))
            }
            12 => {
                let pat = *self.g.pick(&["a%", "%a", "_", "%b%", "", "A%", "%"]);
                let op = *self.g.pick(&["LIKE", "NOT LIKE", "ILIKE"]);
                format!("({} {op} '{pat}')", self.expr(T::Str, d1, sc))
            }
            13 if sub_ok => self.exists(sc),
            14 if sub_ok => self.in_subquery(sc),
            15 => {
                let w = *self.g.pick(&["TRUE", "FALSE", "NOT TRUE", "UNKNOWN"]);
                format!("({} IS {w})", self.pred(d1, sc))
            }
            _ => self.leaf(T::Bool, sc),
        }
    }

    // ---------- subqueries ----------

    fn sub_scope(&mut self, sc: &Scope) -> (String, Scope) {
        let ti = self.idx(self.tables.len());
        let a = self.next_alias("s");
        let cols = self.tables[ti]
            .cols
            .iter()
            .map(|c| ColRef { sql: format!("{a}.{}", c.name), t: t_of(c.ty) })
            .collect();
        let mut outer = sc.cols.clone();
        outer.extend(sc.outer.iter().cloned());
        (format!("{} AS {a}", self.tables[ti].name), Scope { cols, outer, keys: None })
    }

    fn sub_where(&mut self, inner: &Scope) -> String {
        if self.chance(0.75) {
            self.sub_depth -= 1;
            let p = self.pred(2, inner);
            self.sub_depth += 1;
            format!(" WHERE {p}")
        } else {
            String::new()
        }
    }

    fn scalar_subquery(&mut self, t: T, sc: &Scope) -> String {
        let (from, inner) = self.sub_scope(sc);
        let w = self.sub_where(&inner);
        let agg_scope = Scope { keys: Some(vec![]), ..inner.clone() };
        self.sub_depth -= 1;
        let a = self.aggregate(t, &agg_scope);
        self.sub_depth += 1;
        format!("(SELECT {a} FROM {from}{w})")
    }

    fn exists(&mut self, sc: &Scope) -> String {
        let (from, inner) = self.sub_scope(sc);
        let w = self.sub_where(&inner);
        let n = if self.g.bool() { "" } else { "NOT " };
        format!("({n}EXISTS (SELECT 1 FROM {from}{w}))")
    }

    fn in_subquery(&mut self, sc: &Scope) -> String {
        let (from, inner) = self.sub_scope(sc);
        let t = *self.g.pick(&[T::Int, T::Int, T::Str]);
        self.sub_depth -= 1;
        let item = self.expr(t, 1, &Scope { outer: vec![], ..inner.clone() });
        self.sub_depth += 1;
        let w = self.sub_where(&inner);
        let lhs = self.expr(t, 1, sc);
        match self.idx(4) {
            0 => format!("({lhs} NOT IN (SELECT {item} FROM {from}{w}))"),
            1 => {
                let op = self.cmp_op();
                let q = *self.g.pick(&["ANY", "ALL"]);
                format!("({lhs} {op} {q} (SELECT {item} FROM {from}{w}))")
            }
            _ => format!("({lhs} IN (SELECT {item} FROM {from}{w}))"),
        }
    }

    // ---------- FROM ----------

    fn table_ref(&mut self, depth: u32) -> (String, Vec<ColRef>) {
        if depth > 0 && self.chance(0.2) {
            let a = self.next_alias("d");
            let (q, types, _) = self.select(None, depth - 1, false);
            let cols = types.iter().enumerate().map(|(i, t)| ColRef { sql: format!("{a}.x{i}"), t: *t }).collect();
            return (format!("({q}) AS {a}"), cols);
        }
        let ti = self.idx(self.tables.len());
        let a = self.next_alias("a");
        let cols = self.tables[ti]
            .cols
            .iter()
            .map(|c| ColRef { sql: format!("{a}.{}", c.name), t: t_of(c.ty) })
            .collect();
        (format!("{} AS {a}", self.tables[ti].name), cols)
    }

    fn join_cond(&mut self, l: &[ColRef], r: &[ColRef]) -> String {
        let saved = self.sub_depth;
        if self.chance(0.95) {
            self.sub_depth = 0;
        }
        let r = self.join_cond_inner(l, r);
        self.sub_depth = saved;
        r
    }

    fn join_cond_inner(&mut self, l: &[ColRef], r: &[ColRef]) -> String {
        let both = Scope { cols: l.iter().chain(r).cloned().collect(), ..Default::default() };
        let mut conds = Vec::new();
        let pairs: Vec<(String, String)> = l
            .iter()
            .flat_map(|a| r.iter().filter(move |b| b.t == a.t && a.t != T::Bool).map(move |b| (a.sql.clone(), b.sql.clone())))
            .collect();
        if !pairs.is_empty() && self.chance(0.8) {
            let k = if self.chance(0.25) { 2 } else { 1 };
            for _ in 0..k {
                let (a, b) = &pairs[self.idx(pairs.len())];
                conds.push(format!("{a} = {b}"));
            }
            if self.chance(0.3) {
                conds.push(self.pred(1, &both));
            }
        } else if !pairs.is_empty() && self.chance(0.5) {
            // single range predicate (piecewise merge join / NLJ shapes)
            let (a, b) = &pairs[self.idx(pairs.len())];
            let op = *self.g.pick(&["<", "<=", ">", ">="]);
            conds.push(format!("{a} {op} {b}"));
        } else {
            conds.push(self.pred(2, &both));
        }
        conds.join(" AND ")
    }

    fn from(&mut self, depth: u32) -> (String, Vec<ColRef>) {
        let (mut sql, mut cols) = self.table_ref(depth);
        let n = *self.g.pick(&[0usize, 1, 1, 1, 2, 2]);
        for _ in 0..n {
            let (rs, rc) = self.table_ref(depth);
            let kind = *self.g.pick(&[
                "JOIN", "JOIN", "LEFT JOIN", "LEFT JOIN", "RIGHT JOIN", "FULL JOIN", "CROSS JOIN", "LEFT SEMI JOIN",
                "LEFT ANTI JOIN",
            ]);
            if kind == "CROSS JOIN" {
                sql = format!("{sql} CROSS JOIN {rs}");
                cols.extend(rc);
            } else {
                let on = self.join_cond(&cols, &rc);
                sql = format!("{sql} {kind} {rs} ON {on}");
                if !kind.contains("SEMI") && !kind.contains("ANTI") {
                    cols.extend(rc);
                }
            }
        }
        (sql, cols)
    }

    // ---------- SELECT ----------

    /// Returns (sql, output types, ordered-by-all-columns).
    fn select(&mut self, want: Option<Vec<T>>, depth: u32, top: bool) -> (String, Vec<T>, bool) {
        if depth > 0 && self.chance(0.15) {
            // set operation
            let (l, types, _) = self.select_core(want.clone(), depth - 1);
            let (r, _, _) = self.select_core(Some(types.clone()), depth - 1);
            let op = *self.g.pick(&["UNION", "UNION ALL", "UNION ALL", "INTERSECT", "EXCEPT", "INTERSECT ALL", "EXCEPT ALL"]);
            let q = format!("SELECT * FROM (({l}) {op} ({r})) AS u{}", self.alias);
            self.alias += 1;
            return self.order_limit(q, types, top);
        }
        let (q, types, _) = self.select_core(want, depth);
        self.order_limit(q, types, top)
    }

    fn order_limit(&mut self, q: String, types: Vec<T>, top: bool) -> (String, Vec<T>, bool) {
        let p = if top { 0.35 } else { 0.15 };
        if self.chance(p) {
            let keys: Vec<String> = (1..=types.len())
                .map(|i| {
                    let dir = *self.g.pick(&["", " DESC", " ASC NULLS FIRST", " DESC NULLS LAST"]);
                    format!("{i}{dir}")
                })
                .collect();
            let mut q = format!("{q} ORDER BY {}", keys.join(", "));
            if self.chance(0.6) {
                q += &format!(" LIMIT {}", self.range(0, 5));
                if self.chance(0.3) {
                    q += &format!(" OFFSET {}", self.range(0, 3));
                }
            }
            return (q, types, true);
        }
        (q, types, false)
    }

    fn select_core(&mut self, want: Option<Vec<T>>, depth: u32) -> (String, Vec<T>, bool) {
        let (from, cols) = self.from(depth);
        let sc = Scope { cols: cols.clone(), outer: vec![], keys: None };
        let where_ = if self.chance(0.6) { format!(" WHERE {}", self.pred(3, &sc)) } else { String::new() };
        let types: Vec<T> = match &want {
            Some(w) => w.clone(),
            None => {
                let n = self.range(1, 4);
                (0..n).map(|_| *self.g.pick(&[T::Int, T::Int, T::Str, T::Bool, T::Float])).collect()
            }
        };
        let mode = self.idx(10);
        let mut items = Vec::new();
        let mut tail = String::new();
        let mut distinct = "";
        if mode < 4 {
            // aggregate query
            let nk = *self.g.pick(&[0usize, 1, 1, 2, 2, 3]);
            let mut keys = Vec::new();
            let saved = self.sub_depth;
            self.sub_depth = 0; // no subqueries in GROUP BY keys (parser/planner reject them)
            for _ in 0..nk {
                let t = *self.g.pick(&[T::Int, T::Int, T::Str, T::Bool]);
                let e = if self.chance(0.75) { self.column(t, &sc) } else { None }.unwrap_or_else(|| self.expr(t, 1, &sc));
                // a bare integer literal in GROUP BY is a column position, not a value
                if e.parse::<i64>().is_err() && !keys.iter().any(|k: &ColRef| k.sql == e) {
                    keys.push(ColRef { sql: e, t });
                }
            }
            self.sub_depth = saved;
            let asc = Scope { keys: Some(keys.clone()), ..sc.clone() };
            for &t in &types {
                items.push(self.expr(t, 1, &asc));
            }
            if !keys.is_empty() {
                let ks: Vec<String> = keys.iter().map(|k| k.sql.clone()).collect();
                tail += &match self.idx(10) {
                    0 if ks.len() >= 2 => format!(" GROUP BY ROLLUP({})", ks.join(", ")),
                    1 if ks.len() >= 2 => format!(" GROUP BY CUBE({})", ks.join(", ")),
                    2 if ks.len() >= 2 => format!(" GROUP BY GROUPING SETS (({}), ({}), ())", ks[0], ks[1..].join(", ")),
                    _ => format!(" GROUP BY {}", ks.join(", ")),
                };
            }
            if self.chance(0.3) {
                tail += &format!(" HAVING {}", self.pred(1, &asc));
            }
        } else if mode < 6 {
            // window functions over a plain select
            for &t in &types {
                if self.chance(0.5) {
                    items.push(self.window(t, &sc));
                } else {
                    items.push(self.expr(t, 2, &sc));
                }
            }
        } else {
            for &t in &types {
                items.push(self.expr(t, 2, &sc));
            }
            if self.chance(0.2) {
                distinct = "DISTINCT ";
            }
        }
        let items: Vec<String> = items.into_iter().enumerate().map(|(i, e)| format!("{e} AS x{i}")).collect();
        (format!("SELECT {distinct}{} FROM {from}{where_}{tail}", items.join(", ")), types, false)
    }

    /// Window expressions whose value does not depend on the order of peers:
    /// ranking functions and aggregates with the default RANGE frame.
    fn window(&mut self, t: T, sc: &Scope) -> String {
        let saved = self.sub_depth;
        self.sub_depth = 0; // subqueries in window clauses are not supported
        let r = self.window_inner(t, sc);
        self.sub_depth = saved;
        r
    }

    fn window_inner(&mut self, t: T, sc: &Scope) -> String {
        let part = if self.chance(0.7) {
            let pt = *self.g.pick(&[T::Int, T::Str, T::Bool]);
            format!("PARTITION BY {}", self.expr(pt, 1, sc))
        } else {
            String::new()
        };
        let ot = *self.g.pick(&[T::Int, T::Str, T::Float]);
        let ord_e = self.expr(ot, 1, sc);
        let dir = *self.g.pick(&["", " DESC", " NULLS FIRST"]);
        let ord = format!("ORDER BY {ord_e}{dir}");
        match t {
            T::Int => match self.idx(5) {
                // rank() is UInt64; cast so the column composes like any other Int
                0 => format!("CAST(rank() OVER ({part} {ord}) AS BIGINT)"),
                1 => format!("CAST(dense_rank() OVER ({part} {ord}) AS BIGINT)"),
                2 => format!("count(*) OVER ({part})"),
                3 => format!("sum({}) OVER ({part} {ord})", self.expr(T::Int, 1, sc)),
                _ => format!("max({}) OVER ({part})", self.expr(T::Int, 1, sc)),
            },
            T::Float => format!("min({}) OVER ({part} {ord})", self.expr(T::Float, 1, sc)),
            T::Str => format!("max({}) OVER ({part} {ord})", self.expr(T::Str, 1, sc)),
            T::Bool => format!("(count(*) OVER ({part} {ord}) > 1)"),
        }
    }
}

// ---------- variants ----------

fn kv(k: &str, v: &str) -> (String, String) {
    (k.to_string(), v.to_string())
}

pub const RULES: &[&str] = &[
    "rewrite_set_comparison",
    "optimize_unions",
    "unions_to_filter",
    "simplify_expressions",
    "replace_distinct_aggregate",
    "eliminate_join",
    "decorrelate_predicate_subquery",
    "scalar_subquery_to_join",
    "decorrelate_lateral_join",
    "extract_equijoin_predicate",
    "eliminate_duplicated_expr",
    "eliminate_filter",
    "eliminate_cross_join",
    "eliminate_limit",
    "propagate_empty_relation",
    "filter_null_join_keys",
    "eliminate_outer_join",
    "push_down_limit",
    "push_down_filter",
    "eliminate_aggregate_distinct",
    "single_distinct_aggregation_to_group_by",
    "eliminate_group_by_constant",
    "common_sub_expression_eliminate",
    "extract_leaf_expressions",
    "push_down_leaf_projections",
    "optimize_projections",
];

pub fn base_variant() -> Variant {
    Variant { name: "base".into(), settings: vec![kv("datafusion.execution.target_partitions", "4")], remove_rules: vec![] }
}

/// Configuration variants. Every setting here is documented in
/// datafusion/common/src/config.rs as affecting only performance / plan shape.
pub fn variant(g: &mut Source) -> Variant {
    let e = "datafusion.execution.";
    let o = "datafusion.optimizer.";
    let tp = |n: &str| kv("datafusion.execution.target_partitions", n);
    // a quarter of the variants remove one logical optimizer rule
    let (name, settings): (&str, Vec<(String, String)>) = match g.index(20) {
        0 => ("tp1", vec![tp("1")]),
        1 => ("tp3", vec![tp("3")]),
        2 => ("tp16", vec![tp("16")]),
        3 => ("batch_small", vec![tp("4"), kv(&format!("{e}batch_size"), *g.pick(&["1", "2", "3"]))]),
        4 => ("smj", vec![tp("4"), kv(&format!("{o}prefer_hash_join"), "false")]),
        5 => ("pwmj", vec![tp("4"), kv(&format!("{o}enable_piecewise_merge_join"), "true")]),
        6 => (
            "no_repartition",
            vec![
                tp("4"),
                kv(&format!("{o}enable_round_robin_repartition"), "false"),
                kv(&format!("{o}repartition_aggregations"), "false"),
                kv(&format!("{o}repartition_joins"), "false"),
                kv(&format!("{o}repartition_windows"), "false"),
                kv(&format!("{o}repartition_sorts"), "false"),
            ],
        ),
        7 => (
            "no_join_reorder",
            vec![tp("4"), kv(&format!("{o}join_reordering"), "false"), kv(&format!("{o}top_down_join_key_reordering"), "false")],
        ),
        8 => ("no_dynamic_filter", vec![tp("4"), kv(&format!("{o}enable_dynamic_filter_pushdown"), "false")]),
        9 => (
            "partitioned_hash_join",
            vec![
                tp("4"),
                kv(&format!("{o}hash_join_single_partition_threshold"), "0"),
                kv(&format!("{o}hash_join_single_partition_threshold_rows"), "0"),
            ],
        ),
        10 => (
            "skip_partial_agg",
            vec![
                tp("4"),
                kv(&format!("{e}skip_partial_aggregation_probe_rows_threshold"), "0"),
                kv(&format!("{e}skip_partial_aggregation_probe_ratio_threshold"), "0.0"),
            ],
        ),
        11 => (
            "join_prefilter",
            vec![
                tp("4"),
                kv(&format!("{e}enable_join_integer_prefilter"), "true"),
                kv(&format!("{e}join_integer_prefilter_min_pruning_ratio"), "0.0"),
                kv(&format!("{e}join_integer_prefilter_sample_rows"), "1"),
            ],
        ),
        12 => (
            "no_topk",
            vec![
                tp("4"),
                kv(&format!("{o}enable_topk_aggregation"), "false"),
                kv(&format!("{o}enable_window_limits"), "false"),
                kv(&format!("{o}enable_topk_repartition"), "false"),
            ],
        ),
        13 => ("no_coalesce", vec![tp("4"), kv(&format!("{e}coalesce_batches"), "false")]),
        14 => ("max_passes1", vec![tp("4"), kv(&format!("{o}max_passes"), "1")]),
        _ => {
            let r = RULES[g.index(RULES.len())];
            return Variant { name: format!("no_rule_{r}"), settings: vec![tp("4")], remove_rules: vec![r.into()] };
        }
    };
    Variant { name: name.into(), settings, remove_rules: vec![] }
}

pub fn case(seed: u64) -> Case {
    let mut src = Source::new(seed, 0);
    let ntables = *src.pick(&[1usize, 2, 2, 3, 3]);
    let mut cg = Gen { g: &mut src, tables: vec![], alias: 0, sub_depth: 1 };
    let tables: Vec<Table> = (0..ntables).map(|i| cg.table(i)).collect();
    cg.tables = tables;
    let depth = *cg.g.pick(&[0u32, 1, 1, 2]);
    let (query, _, ordered) = cg.select(None, depth, true);
    let tables = std::mem::take(&mut cg.tables);
    let nv = 3;
    let variants = (0..nv).map(|_| variant(&mut src)).collect();
    Case { tables, query, ordered, base: base_variant(), variants }
}
