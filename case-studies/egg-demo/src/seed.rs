//! Generated seeds: a schema and a query inside the fragment [`crate::parse`]
//! reads, for the e-graph to rewrite. Built valid by construction; shrink
//! candidates and mutants are filtered through [`well_formed`], so a shrink
//! that drops something still referenced is never offered.

use std::{collections::BTreeSet, fmt, ops::ControlFlow};

use ephyra::{Gen, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Real,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub ty: Ty,
    pub not_null: bool,
    pub unique: bool,
    /// A foreign-key-style column `t{j}_id` holding ids of table `j`. Only
    /// steers join conditions; the DDL declares no constraint for it.
    pub refs: Option<usize>,
}

/// Table `t{i}`: `id INTEGER PRIMARY KEY`, then `columns`.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableRef {
    pub table: usize,
    pub alias: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Inner,
    Left,
    Cross,
}

/// A join; `on` is `None` exactly for a cross join.
#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    pub kind: Kind,
    pub source: TableRef,
    pub on: Option<Pred>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ColRef {
    pub alias: String,
    pub column: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Col(ColRef),
    Int(i64),
    Real(f64),
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// One table in a subquery: `IN (SELECT column FROM source WHERE filter)`
/// or `EXISTS (SELECT * FROM source WHERE filter)`. The filter may read the
/// enclosing query (a correlated subquery).
#[derive(Clone, Debug, PartialEq)]
pub struct Sub {
    pub source: TableRef,
    pub column: String,
    pub filter: Option<Pred>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pred {
    Cmp(Cmp, Value, Value),
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
    Not(Box<Pred>),
    IsNull(ColRef),
    Between(ColRef, i64, i64),
    InList(ColRef, Vec<Value>),
    InSub(ColRef, Box<Sub>),
    Exists(Box<Sub>),
}

#[derive(Clone, PartialEq)]
pub struct Seed {
    pub tables: Vec<Table>,
    pub distinct: bool,
    pub columns: Vec<ColRef>,
    pub from: TableRef,
    pub joins: Vec<Join>,
    pub filter: Option<Pred>,
}

fn ty_name(ty: Ty) -> &'static str {
    match ty {
        Ty::Int => "INTEGER",
        Ty::Real => "REAL",
        Ty::Text => "TEXT",
    }
}

impl fmt::Display for ColRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.alias, self.column)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Col(c) => write!(f, "{c}"),
            Value::Int(n) => write!(f, "{n}"),
            // `{:?}` keeps the fraction, so `2.0` stays REAL.
            Value::Real(x) => write!(f, "{x:?}"),
            Value::Text(s) => write!(f, "'{s}'"),
        }
    }
}

impl fmt::Display for Sub {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "FROM t{} AS {}", self.source.table, self.source.alias)?;
        match &self.filter {
            Some(p) => write!(f, " WHERE {p}"),
            None => Ok(()),
        }
    }
}

impl fmt::Display for Pred {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Pred::Cmp(op, a, b) => {
                let op = match op {
                    Cmp::Eq => "=",
                    Cmp::Ne => "<>",
                    Cmp::Lt => "<",
                    Cmp::Le => "<=",
                    Cmp::Gt => ">",
                    Cmp::Ge => ">=",
                };
                write!(f, "{a} {op} {b}")
            },
            Pred::And(a, b) => write!(f, "({a} AND {b})"),
            Pred::Or(a, b) => write!(f, "({a} OR {b})"),
            Pred::Not(a) => write!(f, "NOT ({a})"),
            Pred::IsNull(c) => write!(f, "{c} IS NULL"),
            Pred::Between(c, lo, hi) => write!(f, "{c} BETWEEN {lo} AND {hi}"),
            Pred::InList(c, items) => {
                let items: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "{c} IN ({})", items.join(", "))
            },
            Pred::InSub(c, sub) => write!(f, "{c} IN (SELECT {}.{} {sub})", sub.source.alias, sub.column),
            Pred::Exists(sub) => write!(f, "EXISTS (SELECT * {sub})"),
        }
    }
}

/// A script `crate::parse::read` takes: the DDL, then the query.
impl fmt::Display for Seed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, t) in self.tables.iter().enumerate() {
            let mut cols = vec!["id INTEGER PRIMARY KEY".to_string()];
            for c in &t.columns {
                let not_null = if c.not_null { " NOT NULL" } else { "" };
                let unique = if c.unique { " UNIQUE" } else { "" };
                cols.push(format!("{} {}{not_null}{unique}", c.name, ty_name(c.ty)));
            }
            writeln!(f, "CREATE TABLE t{i} ({});", cols.join(", "))?;
        }
        let columns: Vec<String> = self.columns.iter().map(|c| c.to_string()).collect();
        write!(f, "SELECT {}{} ", if self.distinct { "DISTINCT " } else { "" }, columns.join(", "))?;
        write!(f, "FROM t{} AS {}", self.from.table, self.from.alias)?;
        for j in &self.joins {
            let kind = match j.kind {
                Kind::Inner => "JOIN",
                Kind::Left => "LEFT JOIN",
                Kind::Cross => "CROSS JOIN",
            };
            write!(f, " {kind} t{} AS {}", j.source.table, j.source.alias)?;
            if let Some(on) = &j.on {
                write!(f, " ON {on}")?;
            }
        }
        if let Some(p) = &self.filter {
            write!(f, " WHERE {p}")?;
        }
        write!(f, ";")
    }
}

/// Counterexamples print as the script.
impl fmt::Debug for Seed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "\n{self}")
    }
}

impl Seed {
    fn table(&self, alias: &str, scope: &[&TableRef]) -> Option<&Table> {
        scope.iter().find(|s| s.alias == alias).and_then(|s| self.tables.get(s.table))
    }

    /// The type of `c` among `scope`, if it names a column there.
    fn ty(&self, c: &ColRef, scope: &[&TableRef]) -> Option<Ty> {
        let t = self.table(&c.alias, scope)?;
        if c.column == "id" {
            return Some(Ty::Int);
        }
        t.columns.iter().find(|col| col.name == c.column).map(|col| col.ty)
    }

    fn sources(&self) -> Vec<&TableRef> {
        std::iter::once(&self.from).chain(self.joins.iter().map(|j| &j.source)).collect()
    }
}

/// A seed is well formed when every table has distinct column names; the
/// FROM tables, joins and subqueries all have distinct aliases; every column
/// reference names a column in scope (a join's ON sees the tables up to it,
/// the WHERE all of them, a subquery its own table and the enclosing
/// query's); comparisons and IN compare values of one type; a subquery sits
/// only in the WHERE and holds no subquery itself; and the query projects
/// at least one column of its FROM tables.
#[ephyra::generator]
pub fn well_formed(seed: &Seed) -> bool {
    let distinct_names = seed.tables.iter().all(|t| {
        let names: BTreeSet<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
        names.len() == t.columns.len() && !names.contains("id")
    });
    let mut aliases: Vec<&str> = seed.sources().iter().map(|s| s.alias.as_str()).collect();
    if let Some(p) = &seed.filter {
        subs(p, &mut |s| aliases.push(&s.source.alias));
    }
    let unique = aliases.len() == aliases.iter().collect::<BTreeSet<_>>().len();
    let sources = seed.sources();
    let tables_exist = sources.iter().all(|s| s.table < seed.tables.len());
    let joins_ok = seed.joins.iter().enumerate().all(|(i, j)| match (&j.on, j.kind) {
        (None, Kind::Cross) => true,
        (Some(on), Kind::Inner | Kind::Left) => pred_ok(seed, on, &sources[..i + 2], false),
        _ => false,
    });
    distinct_names
        && unique
        && tables_exist
        && joins_ok
        && !seed.columns.is_empty()
        && seed.columns.iter().all(|c| seed.ty(c, &sources).is_some())
        && seed.filter.as_ref().is_none_or(|p| pred_ok(seed, p, &sources, true))
}

/// Visit every subquery of `p`.
fn subs<'a>(p: &'a Pred, f: &mut dyn FnMut(&'a Sub)) {
    match p {
        Pred::And(a, b) | Pred::Or(a, b) => {
            subs(a, f);
            subs(b, f);
        },
        Pred::Not(a) => subs(a, f),
        Pred::InSub(_, s) | Pred::Exists(s) => {
            f(s);
            if let Some(q) = &s.filter {
                subs(q, f);
            }
        },
        _ => {},
    }
}

fn value_ty(seed: &Seed, v: &Value, scope: &[&TableRef]) -> Option<Ty> {
    match v {
        Value::Col(c) => seed.ty(c, scope),
        Value::Int(_) => Some(Ty::Int),
        Value::Real(_) => Some(Ty::Real),
        Value::Text(_) => Some(Ty::Text),
    }
}

fn pred_ok(seed: &Seed, p: &Pred, scope: &[&TableRef], allow_sub: bool) -> bool {
    match p {
        Pred::Cmp(_, a, b) => {
            matches!((value_ty(seed, a, scope), value_ty(seed, b, scope)), (Some(x), Some(y)) if x == y)
        },
        Pred::And(a, b) | Pred::Or(a, b) => {
            pred_ok(seed, a, scope, allow_sub) && pred_ok(seed, b, scope, allow_sub)
        },
        Pred::Not(a) => pred_ok(seed, a, scope, allow_sub),
        Pred::IsNull(c) => seed.ty(c, scope).is_some(),
        Pred::Between(c, _, _) => seed.ty(c, scope) == Some(Ty::Int),
        Pred::InList(c, items) => {
            let ty = seed.ty(c, scope);
            ty.is_some() && !items.is_empty() && items.iter().all(|v| value_ty(seed, v, scope) == ty)
        },
        Pred::InSub(_, sub) | Pred::Exists(sub) => {
            if !allow_sub || sub.source.table >= seed.tables.len() {
                return false;
            }
            let mut inner: Vec<&TableRef> = vec![&sub.source];
            inner.extend(scope.iter().copied());
            let column = ColRef { alias: sub.source.alias.clone(), column: sub.column.clone() };
            let column_ok = match p {
                Pred::InSub(c, _) => seed.ty(c, scope).is_some() && seed.ty(&column, &inner) == seed.ty(c, scope),
                _ => seed.ty(&column, &inner).is_some(),
            };
            column_ok && sub.filter.as_ref().is_none_or(|q| pred_ok(seed, q, &inner, false))
        },
    }
}

#[ephyra::generator(well_formed, metrics = seed_metrics)]
pub fn seeds() -> impl Gen<Value = Seed> {
    SeedGen
}

pub struct SeedGen;

const TEXTS: [&str; 4] = ["a", "b", "pro", "free"];

fn int(g: &mut Source) -> i64 {
    g.int(-1..=4i64)
}

fn text(g: &mut Source) -> String {
    g.pick(&TEXTS).to_string()
}

/// Every column in `scope` as (reference, type).
fn columns(tables: &[Table], scope: &[&TableRef]) -> Vec<(ColRef, Ty)> {
    let mut out = Vec::new();
    for s in scope {
        out.push((ColRef { alias: s.alias.clone(), column: "id".into() }, Ty::Int));
        for c in &tables[s.table].columns {
            out.push((ColRef { alias: s.alias.clone(), column: c.name.clone() }, c.ty));
        }
    }
    out
}

fn literal(g: &mut Source, ty: Ty) -> Value {
    match ty {
        Ty::Int => Value::Int(int(g)),
        Ty::Real => Value::Real(g.int(-2..=8i64) as f64 / 2.0),
        Ty::Text => Value::Text(text(g)),
    }
}

/// A column of `cols` of type `ty`, when there is one.
fn column_of(g: &mut Source, cols: &[(ColRef, Ty)], ty: Ty) -> Option<ColRef> {
    let of: Vec<&ColRef> = cols.iter().filter(|(_, t)| *t == ty).map(|(c, _)| c).collect();
    (!of.is_empty()).then(|| of[g.index(of.len())].clone())
}

struct Ctx<'a> {
    tables: &'a [Table],
    next_alias: usize,
}

impl Ctx<'_> {
    fn alias(&mut self) -> String {
        self.next_alias += 1;
        format!("s{}", self.next_alias)
    }

    /// A comparison between two columns of `scope`, `left` and `right`, of
    /// one type: the join condition shape.
    fn equi(&self, g: &mut Source, left: &[&TableRef], right: &TableRef) -> Pred {
        // A foreign-key edge either way: `right.t{j}_id = left.id` where the
        // left table is `j`, or `left.t{j}_id = right.id`.
        let id = |s: &TableRef| ColRef { alias: s.alias.clone(), column: "id".into() };
        let fk = |from: &TableRef, to: &TableRef| -> Vec<ColRef> {
            (self.tables[from.table].columns.iter())
                .filter(|c| c.refs == Some(to.table))
                .map(|c| ColRef { alias: from.alias.clone(), column: c.name.clone() })
                .collect()
        };
        let mut edges: Vec<(ColRef, ColRef)> = Vec::new();
        for l in left {
            edges.extend(fk(right, l).into_iter().map(|c| (c, id(l))));
            edges.extend(fk(l, right).into_iter().map(|c| (id(right), c)));
        }
        if !edges.is_empty() && g.bool_with(0.75) {
            let (a, b) = edges[g.index(edges.len())].clone();
            return Pred::Cmp(Cmp::Eq, Value::Col(a), Value::Col(b));
        }
        let (l, r) = (columns(self.tables, left), columns(self.tables, &[right]));
        for _ in 0..8 {
            let (a, ta) = &r[g.index(r.len())];
            if let Some(b) = column_of(g, &l, *ta) {
                return Pred::Cmp(Cmp::Eq, Value::Col(a.clone()), Value::Col(b));
            }
        }
        // `id` is on both sides, so this always types.
        let a = ColRef { alias: right.alias.clone(), column: "id".into() };
        let b = ColRef { alias: left[g.index(left.len())].alias.clone(), column: "id".into() };
        Pred::Cmp(Cmp::Eq, Value::Col(a), Value::Col(b))
    }

    fn pred(&mut self, g: &mut Source, scope: &[&TableRef], depth: usize, allow_sub: bool) -> Pred {
        if depth > 1 && g.bool_with(0.5) {
            let a = self.pred(g, scope, depth - 1, allow_sub);
            return match g.int(0..=2u8) {
                0 => Pred::Not(Box::new(a)),
                1 => Pred::And(Box::new(a), Box::new(self.pred(g, scope, depth - 1, allow_sub))),
                _ => Pred::Or(Box::new(a), Box::new(self.pred(g, scope, depth - 1, allow_sub))),
            };
        }
        let cols = columns(self.tables, scope);
        let (c, ty) = cols[g.index(cols.len())].clone();
        match g.int(0..=9u8) {
            0 => Pred::IsNull(c),
            1 if ty == Ty::Int => {
                let lo = int(g);
                Pred::Between(c, lo, lo + g.int(0..=3i64))
            },
            2 => {
                let n = g.int(2..=3usize);
                Pred::InList(c, (0..n).map(|_| literal(g, ty)).collect())
            },
            3 | 4 if allow_sub => self.sub(g, scope, c, ty),
            5 => {
                let other = column_of(g, &cols, ty).map(Value::Col).unwrap_or_else(|| literal(g, ty));
                Pred::Cmp(*g.pick(&[Cmp::Eq, Cmp::Ne, Cmp::Lt]), Value::Col(c), other)
            },
            _ => {
                let op = *g.pick(&[Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge]);
                Pred::Cmp(op, Value::Col(c), literal(g, ty))
            },
        }
    }

    /// `c IN (SELECT ...)` or `EXISTS (SELECT ...)` over a random table,
    /// correlated with the enclosing query about half the time.
    fn sub(&mut self, g: &mut Source, scope: &[&TableRef], c: ColRef, ty: Ty) -> Pred {
        let source = TableRef { table: g.index(self.tables.len()), alias: self.alias() };
        let own = columns(self.tables, &[&source]);
        let correlated = g.bool().then(|| self.equi(g, scope, &source));
        let local = g.bool_with(0.6).then(|| self.pred(g, &[&source], 1, false));
        let filter = match (correlated, local) {
            (Some(a), Some(b)) => Some(Pred::And(Box::new(a), Box::new(b))),
            (a, b) => a.or(b),
        };
        if g.bool() {
            if let Some(col) = column_of(g, &own, ty) {
                let sub = Sub { source, column: col.column, filter };
                return Pred::InSub(c, Box::new(sub));
            }
        }
        let column = own[g.index(own.len())].0.column.clone();
        Pred::Exists(Box::new(Sub { source, column, filter }))
    }
}

/// The schema: up to six tables of up to six columns, both growing with the
/// size. A column is a foreign-key-style `t{j}_id` about a quarter of the
/// time, else typed INTEGER, REAL or TEXT; some are NOT NULL, some UNIQUE
/// (a key when also NOT NULL, the case the e-graph's key rules need).
fn schema(g: &mut Source) -> Vec<Table> {
    let n = g.int(1..=(2 + g.size() / 3).min(6));
    let width = (1 + g.size() / 3).min(6);
    (0..n)
        .map(|_| {
            let mut columns: Vec<Column> = Vec::new();
            for k in 1..=g.int(1..=width) {
                let target = g.index(n);
                let name = format!("t{target}_id");
                if n > 1 && g.bool_with(0.25) && !columns.iter().any(|c| c.name == name) {
                    let not_null = g.bool();
                    columns.push(Column { name, ty: Ty::Int, not_null, unique: false, refs: Some(target) });
                    continue;
                }
                let ty = match g.int(0..=19u8) {
                    0..=10 => Ty::Int,
                    11..=14 => Ty::Real,
                    _ => Ty::Text,
                };
                let (not_null, unique) = (g.bool_with(0.4), g.bool_with(0.15));
                columns.push(Column { name: format!("c{k}"), ty, not_null, unique, refs: None });
            }
            Table { columns }
        })
        .collect()
}

impl SeedGen {
    fn query(&self, g: &mut Source, tables: Vec<Table>) -> Seed {
        let mut ctx = Ctx { tables: &tables, next_alias: 0 };
        let from = TableRef { table: g.index(tables.len()), alias: "a0".into() };
        let mut joins: Vec<Join> = Vec::new();
        let n = g.int(0..=(g.size() / 3).min(3));
        for k in 1..=n {
            let used: Vec<usize> = std::iter::once(from.table).chain(joins.iter().map(|j| j.source.table)).collect();
            let fresh: Vec<usize> = (0..tables.len()).filter(|t| !used.contains(t)).collect();
            let table = if !fresh.is_empty() && g.bool_with(0.8) {
                fresh[g.index(fresh.len())]
            } else {
                g.index(tables.len())
            };
            let source = TableRef { table, alias: format!("a{k}") };
            let kind = *g.pick(&[Kind::Inner, Kind::Inner, Kind::Left, Kind::Left, Kind::Cross]);
            let on = (kind != Kind::Cross).then(|| {
                let scope: Vec<&TableRef> = std::iter::once(&from).chain(joins.iter().map(|j| &j.source)).collect();
                let equi = ctx.equi(g, &scope, &source);
                if g.bool_with(0.3) {
                    let mut with: Vec<&TableRef> = scope.clone();
                    with.push(&source);
                    let extra = ctx.pred(g, &with, 1, false);
                    Pred::And(Box::new(equi), Box::new(extra))
                } else {
                    equi
                }
            });
            joins.push(Join { kind, source, on });
        }
        let scope: Vec<&TableRef> = std::iter::once(&from).chain(joins.iter().map(|j| &j.source)).collect();
        let cols = columns(&tables, &scope);
        let k = g.int(1..=3usize);
        let mut columns: Vec<ColRef> = (0..k).map(|_| cols[g.index(cols.len())].0.clone()).collect();
        columns.dedup();
        let depth = g.int(1..=3usize);
        let filter = g.bool_with(0.85).then(|| ctx.pred(g, &scope, depth, true));
        let distinct = g.bool_with(0.3);
        drop(scope);
        Seed { tables: tables.clone(), distinct, columns, from, joins, filter }
    }
}

impl Gen for SeedGen {
    type Value = Seed;

    fn generate(&self, g: &mut Source) -> Seed {
        let tables = schema(g);
        self.query(g, tables)
    }

    /// Simpler queries: a dropped join, filter, projected column or DISTINCT;
    /// a filter replaced by one of its parts; a subquery without its filter;
    /// a literal moved toward zero; then a smaller schema, the last table or
    /// a column that nothing reads. Every
    /// candidate is smaller, and only well-formed ones are offered.
    fn shrink(&self, seed: &Seed, visit: &mut dyn FnMut(Seed) -> ControlFlow<()>) -> ControlFlow<()> {
        let mut out: Vec<Seed> = Vec::new();
        for i in (0..seed.joins.len()).rev() {
            let mut s = seed.clone();
            s.joins.remove(i);
            out.push(s);
        }
        if let Some(p) = &seed.filter {
            out.push(Seed { filter: None, ..seed.clone() });
            out.extend(smaller(p).into_iter().map(|p| Seed { filter: Some(p), ..seed.clone() }));
        }
        for (i, j) in seed.joins.iter().enumerate() {
            for on in j.on.iter().flat_map(smaller) {
                let mut s = seed.clone();
                s.joins[i].on = Some(on);
                out.push(s);
            }
        }
        if seed.distinct {
            out.push(Seed { distinct: false, ..seed.clone() });
        }
        if seed.columns.len() > 1 {
            for i in 0..seed.columns.len() {
                let mut s = seed.clone();
                s.columns.remove(i);
                out.push(s);
            }
        }
        // The schema: the last table, or a column, when nothing reads it.
        if seed.tables.len() > 1 {
            let mut s = seed.clone();
            s.tables.pop();
            out.push(s);
        }
        for (t, table) in seed.tables.iter().enumerate() {
            for c in 0..table.columns.len() {
                let mut s = seed.clone();
                s.tables[t].columns.remove(c);
                out.push(s);
            }
        }
        for s in out.into_iter().filter(well_formed) {
            visit(s)?;
        }
        ControlFlow::Continue(())
    }

    /// One part regenerated over the same schema: the filter, the
    /// projection, a join's kind, or the whole query.
    fn mutate(&self, seed: &Seed, g: &mut Source) -> Seed {
        let mut s = seed.clone();
        match g.int(0..=3u8) {
            0 => {
                let fresh = self.query(g, seed.tables.clone());
                s.filter = fresh.filter;
            },
            1 => s.distinct = !s.distinct,
            2 if !s.joins.is_empty() => {
                let i = g.index(s.joins.len());
                s.joins[i].kind = match s.joins[i].kind {
                    Kind::Inner => Kind::Left,
                    Kind::Left => Kind::Inner,
                    Kind::Cross => Kind::Cross,
                };
            },
            _ => return self.query(g, seed.tables.clone()),
        }
        if well_formed(&s) { s } else { self.query(g, seed.tables.clone()) }
    }
}

/// `p` without one part: either side of AND/OR, the operand of NOT, a
/// subquery without its filter, an IN list one item shorter, a literal
/// moved toward zero.
fn smaller(p: &Pred) -> Vec<Pred> {
    let mut out = Vec::new();
    match p {
        Pred::And(a, b) | Pred::Or(a, b) => {
            out.push((**a).clone());
            out.push((**b).clone());
            let rebuild = |a: Pred, b: Pred| match p {
                Pred::And(..) => Pred::And(Box::new(a), Box::new(b)),
                _ => Pred::Or(Box::new(a), Box::new(b)),
            };
            out.extend(smaller(a).into_iter().map(|x| rebuild(x, (**b).clone())));
            out.extend(smaller(b).into_iter().map(|y| rebuild((**a).clone(), y)));
        },
        Pred::Not(a) => {
            out.push((**a).clone());
            out.extend(smaller(a).into_iter().map(|x| Pred::Not(Box::new(x))));
        },
        Pred::InSub(c, s) if s.filter.is_some() => {
            out.push(Pred::InSub(c.clone(), Box::new(Sub { filter: None, ..(**s).clone() })))
        },
        Pred::Exists(s) if s.filter.is_some() => out.push(Pred::Exists(Box::new(Sub { filter: None, ..(**s).clone() }))),
        Pred::InList(c, items) if items.len() > 1 => {
            out.push(Pred::InList(c.clone(), items[..items.len() - 1].to_vec()))
        },
        Pred::Cmp(op, a, Value::Int(n)) if *n != 0 => out.push(Pred::Cmp(*op, a.clone(), Value::Int(0))),
        _ => {},
    }
    out
}

/// - tables, joins, left joins, cross joins, projected columns: raw counts.
/// - subqueries, correlated subqueries: raw counts (a subquery is correlated
///   when its filter names a column outside its own table).
/// - filter depth: 0 without a filter.
/// - distinct, self-join: indicators (a self-join scans one table twice).
/// - nullable share: nullable columns over all non-key columns; 0 with none.
/// - columns, real columns, foreign keys: raw counts over the schema,
///   besides each table's `id`.
/// - unique keys, unique nullable: UNIQUE columns that are NOT NULL (a key),
///   and those that are not.
/// - foreign-key joins: joins whose ON includes `t{j}_id = id` of table `j`.
pub fn seed_metrics(seed: &Seed, _size: usize) -> Vec<(&'static str, f64)> {
    let count = |k: Kind| seed.joins.iter().filter(|j| j.kind == k).count() as f64;
    let (mut subqueries, mut correlated) = (0.0, 0.0);
    if let Some(p) = &seed.filter {
        subs(p, &mut |s| {
            subqueries += 1.0;
            let reads_outside = s.filter.as_ref().is_some_and(|q| aliases_read(q).iter().any(|a| *a != s.source.alias));
            if reads_outside {
                correlated += 1.0;
            }
        });
    }
    let tables: Vec<usize> = seed.sources().iter().map(|s| s.table).collect();
    let self_join = tables.len() != tables.iter().collect::<BTreeSet<_>>().len();
    let all: Vec<&Column> = seed.tables.iter().flat_map(|t| &t.columns).collect();
    let nullable = all.iter().filter(|c| !c.not_null).count() as f64;
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let fk_join = |j: &Join| {
        let sources = seed.sources();
        let table_of = |alias: &str| sources.iter().find(|s| s.alias == alias).map(|s| s.table);
        let edge = |a: &ColRef, b: &ColRef| {
            b.column == "id"
                && table_of(&a.alias).and_then(|t| seed.tables.get(t)).is_some_and(|t| {
                    t.columns.iter().any(|c| c.name == a.column && c.refs.is_some() && c.refs == table_of(&b.alias))
                })
        };
        match &j.on {
            Some(Pred::Cmp(Cmp::Eq, Value::Col(a), Value::Col(b))) => edge(a, b) || edge(b, a),
            Some(Pred::And(p, _)) => matches!(p.as_ref(), Pred::Cmp(Cmp::Eq, Value::Col(a), Value::Col(b)) if edge(a, b) || edge(b, a)),
            _ => false,
        }
    };
    let fks = all.iter().filter(|c| c.refs.is_some()).count() as f64;
    let reals = all.iter().filter(|c| c.ty == Ty::Real).count() as f64;
    let keys = all.iter().filter(|c| c.unique && c.not_null).count() as f64;
    let unique_nullable = all.iter().filter(|c| c.unique && !c.not_null).count() as f64;
    vec![
        ("tables", seed.tables.len() as f64),
        ("columns", all.len() as f64),
        ("real columns", reals),
        ("foreign keys", fks),
        ("unique keys", keys),
        ("unique nullable", unique_nullable),
        ("foreign-key joins", seed.joins.iter().filter(|j| fk_join(j)).count() as f64),
        ("joins", seed.joins.len() as f64),
        ("left joins", count(Kind::Left)),
        ("cross joins", count(Kind::Cross)),
        ("projected columns", seed.columns.len() as f64),
        ("subqueries", subqueries),
        ("correlated subqueries", correlated),
        ("filter depth", seed.filter.as_ref().map_or(0.0, |p| depth(p) as f64)),
        ("distinct", flag(seed.distinct)),
        ("self-join", flag(self_join)),
        ("nullable share", if all.is_empty() { 0.0 } else { nullable / all.len() as f64 }),
    ]
}

fn depth(p: &Pred) -> usize {
    match p {
        Pred::And(a, b) | Pred::Or(a, b) => 1 + depth(a).max(depth(b)),
        Pred::Not(a) => 1 + depth(a),
        _ => 1,
    }
}

fn aliases_read(p: &Pred) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut col = |c: &ColRef| {
        out.insert(c.alias.clone());
    };
    fn walk(p: &Pred, col: &mut dyn FnMut(&ColRef)) {
        let value = |v: &Value, col: &mut dyn FnMut(&ColRef)| {
            if let Value::Col(c) = v {
                col(c)
            }
        };
        match p {
            Pred::Cmp(_, a, b) => {
                value(a, col);
                value(b, col);
            },
            Pred::And(a, b) | Pred::Or(a, b) => {
                walk(a, col);
                walk(b, col);
            },
            Pred::Not(a) => walk(a, col),
            Pred::IsNull(c) | Pred::Between(c, _, _) | Pred::InSub(c, _) => col(c),
            Pred::InList(c, items) => {
                col(c);
                items.iter().for_each(|v| value(v, col));
            },
            Pred::Exists(_) => {},
        }
    }
    walk(p, &mut col);
    out
}
