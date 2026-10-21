// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! AST v0: one table `t` with columns `c0, c1, ...`, some rows, and one
//! `SELECT` over it. `Display` renders runnable SQL.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Integer,
    Real,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Null,
    Integer(i64),
    /// Always finite: SQL has no literal for NaN or infinity.
    Real(f64),
    Text(String),
}

pub const PREFIX: [&str; 2] = ["NOT", "-"];
/// No `%`: COMPAT.md lists it as unsupported. No `||`: Turso's own generator
/// avoids it for tursodatabase/turso#4860.
pub const BINARY: [&str; 13] =
    ["=", "<>", "<", "<=", ">", ">=", "AND", "OR", "+", "-", "*", "/", "LIKE"];

/// Operators are their SQL text, taken from [`PREFIX`] and [`BINARY`].
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Column(usize),
    Literal(Literal),
    Prefix(&'static str, Box<Expr>),
    IsNull(Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// Projected column indices; `None` is `*`.
    pub columns: Option<Vec<usize>>,
    pub filter: Option<Expr>,
    /// `Some(desc)` orders by every output column, so ties are identical
    /// rows and the order is fully determined.
    pub order: Option<bool>,
    /// Only with `order`, so the rows kept are determined.
    pub limit: Option<u32>,
}

#[derive(Clone, PartialEq)]
pub struct Case {
    pub schema: Vec<Type>,
    pub rows: Vec<Vec<Literal>>,
    pub query: Query,
}

impl Expr {
    pub fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Column(_) | Expr::Literal(_) => vec![],
            Expr::Prefix(_, e) | Expr::IsNull(e) => vec![e],
            Expr::Binary(_, a, b) => vec![a, b],
        }
    }

    pub fn depth(&self) -> usize {
        self.children().iter().map(|c| c.depth()).max().map_or(1, |d| d + 1)
    }

    pub fn nodes(&self) -> usize {
        1 + self.children().iter().map(|c| c.nodes()).sum::<usize>()
    }

    pub fn columns(&self, out: &mut Vec<usize>) {
        if let Expr::Column(i) = self {
            out.push(*i);
        }
        self.children().into_iter().for_each(|c| c.columns(out));
    }
}

impl Case {
    /// The statements that build the table, one per string.
    pub fn setup(&self) -> Vec<String> {
        let mut out = vec![self.create()];
        for row in &self.rows {
            let values: Vec<String> = row.iter().map(|v| v.to_string()).collect();
            out.push(format!("INSERT INTO t VALUES ({})", values.join(", ")));
        }
        out
    }

    /// The query as SQL.
    pub fn select(&self) -> String {
        self.query.sql(self.schema.len())
    }

    /// `CREATE TABLE t(...)` for the schema.
    pub fn create(&self) -> String {
        create(&self.schema)
    }
}

/// `CREATE TABLE t(...)` for `schema`.
pub fn create(schema: &[Type]) -> String {
    let columns: Vec<String> = (schema.iter().enumerate())
        .map(|(i, ty)| format!("c{i} {}", format!("{ty:?}").to_uppercase()))
        .collect();
    format!("CREATE TABLE t({})", columns.join(", "))
}

impl Query {
    /// The query as SQL, over a table of `arity` columns.
    pub fn sql(&self, arity: usize) -> String {
        let q = self;
        let output = q.columns.clone().unwrap_or_else(|| (0..arity).collect());
        let names: Vec<String> = output.iter().map(|i| format!("c{i}")).collect();
        let mut sql = match &q.columns {
            None => "SELECT * FROM t".to_string(),
            Some(_) => format!("SELECT {} FROM t", names.join(", ")),
        };
        if let Some(p) = &q.filter {
            sql += &format!(" WHERE {p}");
        }
        if let Some(desc) = q.order {
            let dir = if desc { " DESC" } else { "" };
            let keys: Vec<String> = names.iter().map(|n| format!("{n}{dir}")).collect();
            sql += &format!(" ORDER BY {}", keys.join(", "));
        }
        if let Some(n) = q.limit {
            sql += &format!(" LIMIT {n}");
        }
        sql
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Literal::Null => write!(f, "NULL"),
            Literal::Integer(n) if *n < 0 => write!(f, "({n})"),
            Literal::Integer(n) => write!(f, "{n}"),
            // `{:?}` keeps a fraction or exponent, so `3.0` stays REAL.
            Literal::Real(x) if x.is_sign_negative() => write!(f, "({x:?})"),
            Literal::Real(x) => write!(f, "{x:?}"),
            Literal::Text(s) => write!(f, "'{}'", s.replace('\'', "''")),
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Expr::Column(i) => write!(f, "c{i}"),
            Expr::Literal(l) => write!(f, "{l}"),
            Expr::Prefix(op, e) => write!(f, "({op} {e})"),
            Expr::IsNull(e) => write!(f, "({e} IS NULL)"),
            Expr::Binary(op, a, b) => write!(f, "({a} {op} {b})"),
        }
    }
}

impl fmt::Display for Case {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for statement in self.setup() {
            writeln!(f, "{statement};")?;
        }
        write!(f, "{};", self.select())
    }
}

/// Counterexamples print as the SQL script.
impl fmt::Debug for Case {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "\n{self}")
    }
}
