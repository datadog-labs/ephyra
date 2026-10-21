// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A script of `CREATE TABLE`s and one `SELECT`, read with sqlparser into
//! the schema and a term. The fragment: SELECT [DISTINCT] columns FROM tables
//! joined by inner, cross and left joins, WHERE, a GROUP BY of exactly the
//! projected columns (read as DISTINCT), and `IN (SELECT col ...)` and
//! `EXISTS (SELECT ...)` subqueries. Anything else is refused by name.

use std::collections::{BTreeMap, BTreeSet};

use egg::{Id, RecExpr, Symbol};
use sqlparser::{
    ast::{
        BinaryOperator, ColumnOption, Distinct, Expr, GroupByExpr, JoinConstraint, JoinOperator, ObjectName,
        SelectItem, SetExpr, Statement, TableFactor, UnaryOperator, Value,
    },
    dialect::PostgreSqlDialect,
    parser::Parser,
};

use crate::lang::Sql;

pub struct Table {
    pub name: String,
    /// (name, type as written, NOT NULL)
    pub columns: Vec<(String, String, bool)>,
    /// Single-column keys: PRIMARY KEY, or UNIQUE and NOT NULL.
    pub keys: Vec<String>,
}

pub struct Input {
    pub tables: Vec<Table>,
    /// The DDL as written, to create the tables in SQLite.
    pub ddl: Vec<String>,
    pub query: String,
    pub term: RecExpr<Sql>,
    /// NOT NULL columns as `alias.column`, for tables never padded by a LEFT JOIN.
    pub not_null: BTreeSet<String>,
    /// Single-column keys by alias.
    pub keys: BTreeMap<String, Vec<String>>,
    /// Literals of the query, so random data can meet its predicates.
    pub literals: Vec<String>,
}

type Result<T> = std::result::Result<T, String>;

pub fn read(script: &str) -> Result<Input> {
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, script).map_err(|e| e.to_string())?;
    let (mut tables, mut ddl, mut query) = (Vec::new(), Vec::new(), None);
    for statement in statements {
        match statement {
            Statement::CreateTable(create) => {
                ddl.push(create.to_string());
                let (mut columns, mut keys) = (Vec::new(), Vec::new());
                for c in &create.columns {
                    let has = |f: fn(&ColumnOption) -> bool| c.options.iter().any(|o| f(&o.option));
                    let primary = has(|o| matches!(o, ColumnOption::PrimaryKey(_)));
                    // A PRIMARY KEY column is NOT NULL in Postgres.
                    let not_null = primary || has(|o| matches!(o, ColumnOption::NotNull));
                    if primary || (not_null && has(|o| matches!(o, ColumnOption::Unique(_)))) {
                        keys.push(c.name.value.clone());
                    }
                    columns.push((c.name.value.clone(), c.data_type.to_string(), not_null));
                }
                tables.push(Table { name: name(&create.name), columns, keys });
            },
            Statement::Query(q) => query = Some(q),
            other => return Err(format!("unsupported statement: {other}")),
        }
    }
    let query = query.ok_or("no SELECT in the script")?;
    let mut b = Builder {
        tables: &tables,
        scopes: Vec::new(),
        all: Vec::new(),
        padded: BTreeSet::new(),
        term: RecExpr::default(),
        literals: Vec::new(),
    };
    b.query(&query, true)?;
    let not_null = (b.all.iter())
        .filter(|(alias, _)| !b.padded.contains(alias))
        .flat_map(|(alias, t)| t.columns.iter().filter(|c| c.2).map(move |c| format!("{alias}.{}", c.0)))
        .collect();
    let keys = b.all.iter().map(|(alias, t)| (alias.clone(), t.keys.clone())).collect();
    let (term, literals) = (b.term, b.literals);
    Ok(Input { query: query.to_string(), tables, ddl, term, not_null, keys, literals })
}

fn name(n: &ObjectName) -> String {
    n.to_string().trim_matches('"').to_string()
}

type Scope<'a> = Vec<(String, &'a Table)>;

struct Builder<'a> {
    tables: &'a [Table],
    /// The FROM tables of each enclosing query, innermost last.
    scopes: Vec<Scope<'a>>,
    /// Every alias in the query; they must be distinct.
    all: Scope<'a>,
    /// Aliases on the null-supplied side of a LEFT JOIN.
    padded: BTreeSet<String>,
    term: RecExpr<Sql>,
    literals: Vec<String>,
}

impl<'a> Builder<'a> {
    fn add(&mut self, node: Sql) -> Id {
        self.term.add(node)
    }

    fn symbol(&mut self, s: &str) -> Id {
        self.add(Sql::Symbol(Symbol::from(s)))
    }

    /// The query's projection (wrapped in `distinct` when asked), or for an
    /// EXISTS subquery (`relation_only`) its FROM and WHERE alone.
    fn query(&mut self, q: &sqlparser::ast::Query, top: bool) -> Result<Id> {
        self.select(q, top, false)
    }

    fn select(&mut self, q: &sqlparser::ast::Query, top: bool, relation_only: bool) -> Result<Id> {
        if q.order_by.is_some() || q.limit_clause.is_some() || q.with.is_some() {
            return Err("ORDER BY, LIMIT and WITH are outside the fragment".into());
        }
        let SetExpr::Select(s) = q.body.as_ref() else { return Err("only a plain SELECT".into()) };
        if s.having.is_some() || matches!(s.distinct, Some(Distinct::On(_))) {
            return Err("HAVING and DISTINCT ON are outside the fragment".into());
        }
        // Scope first, so ON and WHERE can name any table of this FROM.
        let mut scope = Vec::new();
        for from in &s.from {
            for factor in std::iter::once(&from.relation).chain(from.joins.iter().map(|j| &j.relation)) {
                let TableFactor::Table { name: n, alias, .. } = factor else {
                    return Err("only base tables in FROM".into());
                };
                let table = self.tables.iter().find(|t| t.name == name(n)).ok_or(format!("no table {n}"))?;
                let alias = alias.as_ref().map_or(table.name.clone(), |a| a.name.value.clone());
                // Distinct aliases keep every `alias.column` unambiguous when
                // subqueries are moved or printed.
                if self.all.iter().any(|(a, _)| *a == alias) {
                    return Err(format!("alias {alias} is used twice; give each table its own"));
                }
                self.all.push((alias.clone(), table));
                scope.push((alias, table));
            }
        }
        self.scopes.push(scope.clone());
        let mut names = scope.iter().map(|(a, t)| (a.clone(), t.name.clone()));
        let mut rel: Option<Id> = None;
        for from in &s.from {
            let mut item = self.scan(names.next().unwrap());
            for join in &from.joins {
                let (right_alias, right_table) = names.next().unwrap();
                let right = self.scan((right_alias.clone(), right_table));
                item = match &join.join_operator {
                    JoinOperator::Join(JoinConstraint::On(on)) | JoinOperator::Inner(JoinConstraint::On(on)) => {
                        let joined = self.add(Sql::Join([item, right]));
                        let p = self.expr(on)?;
                        self.add(Sql::Filter([p, joined]))
                    },
                    JoinOperator::Left(JoinConstraint::On(on)) | JoinOperator::LeftOuter(JoinConstraint::On(on)) => {
                        self.padded.insert(right_alias);
                        let p = self.expr(on)?;
                        self.add(Sql::LeftJoin([p, item, right]))
                    },
                    JoinOperator::CrossJoin(JoinConstraint::None) => self.add(Sql::Join([item, right])),
                    other => return Err(format!("unsupported join: {other:?}")),
                };
            }
            rel = Some(match rel {
                None => item,
                Some(left) => self.add(Sql::Join([left, item])),
            });
        }
        let mut rel = rel.ok_or("no FROM")?;
        if let Some(w) = &s.selection {
            let p = self.expr(w)?;
            rel = self.add(Sql::Filter([p, rel]));
        }
        if relation_only {
            self.scopes.pop();
            return Ok(rel);
        }
        let mut children = vec![rel];
        for item in &s.projection {
            match item {
                SelectItem::UnnamedExpr(e @ (Expr::Identifier(_) | Expr::CompoundIdentifier(_))) => {
                    children.push(self.expr(e)?)
                },
                // Expanded, so reordering joins cannot reorder the columns.
                SelectItem::Wildcard(_) => {
                    let all: Vec<String> = (scope.iter())
                        .flat_map(|(a, t)| t.columns.iter().map(move |c| format!("{a}.{}", c.0)))
                        .collect();
                    for c in all {
                        children.push(self.symbol(&c));
                    }
                },
                other => return Err(format!("only columns and * are projected, not {other}")),
            }
        }
        // GROUP BY exactly the projected columns, with no aggregates, is DISTINCT.
        let grouped = match &s.group_by {
            GroupByExpr::Expressions(keys, mods) if !keys.is_empty() && mods.is_empty() => {
                let keys: BTreeSet<Id> = keys.iter().map(|k| self.expr(k)).collect::<Result<_>>()?;
                let projected: BTreeSet<Id> = children[1..].iter().copied().collect();
                if self.symbols(&keys) != self.symbols(&projected) {
                    return Err("GROUP BY is supported only over exactly the projected columns".into());
                }
                true
            },
            GroupByExpr::Expressions(keys, _) if keys.is_empty() => false,
            other => return Err(format!("unsupported GROUP BY {other}")),
        };
        self.scopes.pop();
        let project = self.add(Sql::Project(children.into_boxed_slice()));
        if s.distinct.is_some() || grouped {
            if !top {
                return Err("DISTINCT in a subquery is outside the fragment".into());
            }
            return Ok(self.add(Sql::Distinct([project])));
        }
        Ok(project)
    }

    fn symbols(&self, ids: &BTreeSet<Id>) -> BTreeSet<String> {
        ids.iter().map(|&i| self.term[i].to_string()).collect()
    }

    fn scan(&mut self, (alias, table): (String, String)) -> Id {
        let (t, a) = (self.symbol(&table), self.symbol(&alias));
        self.add(Sql::Scan([t, a]))
    }

    /// `alias.column` for a column, qualified or not: this query's tables
    /// first, then the enclosing queries' (a correlated reference).
    fn column(&mut self, qualifier: Option<&str>, col: &str) -> Result<Id> {
        for scope in self.scopes.iter().rev() {
            let found: Vec<&String> = (scope.iter())
                .filter(|(a, t)| qualifier.is_none_or(|q| q == a) && t.columns.iter().any(|c| c.0 == col))
                .map(|(a, _)| a)
                .collect();
            match found.as_slice() {
                [alias] => {
                    let s = format!("{alias}.{col}");
                    return Ok(self.symbol(&s));
                },
                [] => continue,
                _ => return Err(format!("column {col} is ambiguous")),
            }
        }
        Err(format!("no column {col}"))
    }

    fn literal(&mut self, s: String) -> Id {
        if !self.literals.contains(&s) {
            self.literals.push(s.clone());
        }
        self.symbol(&s)
    }

    fn expr(&mut self, e: &Expr) -> Result<Id> {
        let node = match e {
            Expr::Identifier(i) => return self.column(None, &i.value),
            Expr::CompoundIdentifier(parts) if parts.len() == 2 => {
                return self.column(Some(&parts[0].value), &parts[1].value)
            },
            Expr::Value(v) => {
                return match &v.value {
                    Value::Number(n, _) => Ok(self.literal(n.clone())),
                    Value::SingleQuotedString(s) => Ok(self.literal(format!("'{}'", s.replace('\'', "''")))),
                    Value::Boolean(b) => Ok(self.symbol(if *b { "TRUE" } else { "FALSE" })),
                    Value::Null => Ok(self.symbol("NULL")),
                    other => Err(format!("unsupported literal {other}")),
                }
            },
            Expr::Nested(e) => return self.expr(e),
            // `-1` parses as minus applied to `1`; a negative number is a literal of its own.
            Expr::UnaryOp { op: UnaryOperator::Minus, expr } if matches!(expr.as_ref(), Expr::Value(v) if matches!(v.value, Value::Number(..))) => {
                let Expr::Value(v) = expr.as_ref() else { unreachable!() };
                let Value::Number(n, _) = &v.value else { unreachable!() };
                return Ok(self.literal(format!("-{n}")));
            },
            Expr::UnaryOp { op: UnaryOperator::Not, expr } => Sql::Not([self.expr(expr)?]),
            Expr::IsNull(e) => Sql::IsNull([self.expr(e)?]),
            Expr::IsNotNull(e) => {
                let is_null = Sql::IsNull([self.expr(e)?]);
                let inner = self.add(is_null);
                Sql::Not([inner])
            },
            Expr::BinaryOp { left, op, right } => {
                let (l, r) = (self.expr(left)?, self.expr(right)?);
                match op {
                    BinaryOperator::And => Sql::And([l, r]),
                    BinaryOperator::Or => Sql::Or([l, r]),
                    BinaryOperator::Eq => Sql::Eq([l, r]),
                    BinaryOperator::NotEq => Sql::Ne([l, r]),
                    BinaryOperator::Lt => Sql::Lt([l, r]),
                    BinaryOperator::LtEq => Sql::Le([l, r]),
                    BinaryOperator::Gt => Sql::Gt([l, r]),
                    BinaryOperator::GtEq => Sql::Ge([l, r]),
                    BinaryOperator::Plus => Sql::Add([l, r]),
                    other => return Err(format!("unsupported operator {other}")),
                }
            },
            Expr::Between { expr, negated, low, high } => {
                let between = Sql::Between([self.expr(expr)?, self.expr(low)?, self.expr(high)?]);
                return Ok(self.negate(between, *negated));
            },
            Expr::InList { expr, list, negated } => {
                let mut children = vec![self.expr(expr)?];
                for item in list {
                    children.push(self.expr(item)?);
                }
                return Ok(self.negate(Sql::In(children.into_boxed_slice()), *negated));
            },
            Expr::InSubquery { expr, subquery, negated } => {
                let needle = self.expr(expr)?;
                let sub = self.query(subquery, false)?;
                if !matches!(&self.term[sub], Sql::Project(c) if c.len() == 2) {
                    return Err("an IN subquery selects exactly one column".into());
                }
                return Ok(self.negate(Sql::InSub([needle, sub]), *negated));
            },
            Expr::Exists { subquery, negated } => {
                let rel = self.select(subquery, false, true)?;
                return Ok(self.negate(Sql::Exists([rel]), *negated));
            },
            other => return Err(format!("unsupported expression {other}")),
        };
        Ok(self.add(node))
    }

    fn negate(&mut self, node: Sql, negated: bool) -> Id {
        let id = self.add(node);
        if negated { self.add(Sql::Not([id])) } else { id }
    }
}
