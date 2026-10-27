// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What a [`Statement`] means against a [`Model`]: whether it is valid
//! ([`valid`]: PostgreSQL runs it without error), and its effect ([`apply`]):
//! the model after it and, for a query, the rows PostgreSQL must return
//! ([`Expected`]).
//!
//! The model is exact only for the statements the generator produces, and
//! the language is restricted to keep it so. Each restriction is checked by
//! [`valid`] and documented where it is checked:
//!
//! - Values are written in their type's input form ([`Value::text`]), so
//!   PostgreSQL stores exactly the value: a statement writes a column only
//!   values its type admits ([`Type::admits`]); a predicate compares a
//!   column with a literal of its type, or of a type PostgreSQL compares
//!   with it exactly ([`Type::comparable`]). Comparison is
//!   [`crate::model::compare`] (C collation for text), NULL on a NULL
//!   operand.
//! - NOT NULL, CHECK constraints and unique constraints (the primary key,
//!   UNIQUE indexes) are never violated: the rows after an INSERT or UPDATE
//!   must keep them all ([`rows_ok`]). PostgreSQL checks a unique
//!   constraint row by row as it writes; UPDATE sets constants, so a
//!   conflict on the way is one in the rows after it too (an updated row
//!   that meets an old one meets it again once that one is updated). A
//!   CHECK passes on NULL.
//! - A UNIQUE index is plain (no expression, no predicate), so the model
//!   knows which rows it covers.
//! - An INSERT that does not list the identity column takes the next value
//!   of its sequence, row after row; a rollback does not give the values
//!   back ([`Model::sequences`]). Valid only while the values fit the
//!   column's type.
//! - Index entries fit the B-tree's limit and rows of `STORAGE PLAIN`
//!   columns fit a page ([`crate::model::sizes_ok`]).
//! - `ORDER BY` puts NULL last ascending and first descending unless told;
//!   rows that tie on every term come in any order, and a LIMIT that cuts a
//!   tie may return any of them ([`Expected::matches`]). LIMIT without
//!   ORDER BY returns any rows.
//! - VACUUM and REINDEX CONCURRENTLY run only outside a transaction (inside
//!   one PostgreSQL refuses them).
//! - ADD COLUMN with NOT NULL needs a default unless the table is empty;
//!   SET NOT NULL needs no NULL in the column; DROP NOT NULL is refused on
//!   a key or identity column, SET or DROP DEFAULT on an identity column.
//!   DROP COLUMN takes the indexes and CHECK constraints that read the
//!   column with it, and is not drawn for a key column.
//! - ANALYZE, crafted statistics, settings, CLUSTER, REINDEX, VACUUM,
//!   CHECKPOINT and the storage and estimation changes of
//!   [`crate::statements::Phys`] change plans and pages, never rows; those
//!   that need a table's own storage (fill factor, LOGGED, CLUSTER ON) are
//!   not drawn for a partitioned table.
//! - `ROLLBACK TO` brings back the tables as they were at the savepoint
//!   (the model keeps a copy per open savepoint); sequences stay.
//! - Foreign keys and the writes they act on: [`crate::fk`]. `DROP TABLE`
//!   and `TRUNCATE` say CASCADE (the referencing foreign keys go, the
//!   referencing tables are emptied too); a column or UNIQUE index a
//!   foreign key references is not dropped, nor a column such an index
//!   reads, `INCLUDE` too (PostgreSQL would need CASCADE, see
//!   [`fk::may_back`]); a column a trigger names is not dropped either
//!   ([`fk::trigger_column`]); extended statistics go with a column;
//!   `SET LOGGED | UNLOGGED` keeps linked tables' persistence compatible; `ALTER CONSTRAINT` runs outside a transaction only (a
//!   generator restriction: the deferred state of a constraint whose
//!   deferrability changes mid-transaction is not modelled).
//! - `INSERT .. ON CONFLICT` ([`crate::fk`]'s `upsert`, row by row as
//!   PostgreSQL runs it): the target's arbiters by inference over plain
//!   unique constraints ([`arbiters`]); DO UPDATE needs a target, sets
//!   neither the identity column, the partition key (a row would move,
//!   which PostgreSQL refuses), nor a column a foreign key references (no
//!   referential action then); a row the statement inserted or updated met
//!   again by DO UPDATE raises `21000`, not generated. Its proposed rows
//!   all take identity values, conflicting or not. Never an expected error.
//! - `RETURNING` of an INSERT, UPSERT, UPDATE, DELETE or MERGE: the rows the
//!   statement itself wrote (after BEFORE triggers, before AFTER triggers
//!   and referential actions), old and new ([`returned_rows`]); as a
//!   data-modifying CTE, the main query reads the target as it was before.
//! - MERGE: [`merge_write`].
//! - An expected error (`Statement::Expect`): an INSERT, UPDATE, DELETE or
//!   ADD FOREIGN KEY whose only error is the one expected
//!   ([`crate::fk::Errors::code`]); it changes no row, but takes identity
//!   values and leaves its values in the indexes (recorded as held). A
//!   multi-row INSERT that takes identity values is expected to fail only
//!   at the statement's end (a foreign key), since a row's error stops the
//!   values the later rows would take.

use std::cmp::Ordering;

use crate::{
    fk::{self, Change, Code, Write},
    model::{compare, identical, order_nulls, sizes_ok, unique_ok, Column, Index, Method, Model, Row, Snapshot, Table, Type, Value, MAX_COLUMNS, MAX_INDEXES, MAX_TABLES},
    statements::{method_takes, outside_transaction, CmpOp, DomOp, Trip, Insert, Lit, OrderTerm, Phys, Pred, FromTable, Merge, MergeAction, MergeKind, Returning, SetVal, Stat, Statement, Target, Update, When, Which},
};

/// The most savepoints open at once.
pub const MAX_SAVEPOINTS: usize = 3;

/// Whether `lit` may be compared with column `c{column}` of `t`.
pub fn lit_ok(t: &Table, column: u32, lit: &Lit) -> bool {
    let Some(col) = t.column(column) else {
        return false;
    };
    match lit.cast {
        None => col.ty.admits(&lit.value),
        Some(ty) => ty.comparable(col.ty) && ty.admits(&lit.value),
    }
}

pub fn pred_valid(t: &Table, p: &Pred) -> bool {
    match p {
        Pred::Cmp(c, _, v) => lit_ok(t, *c, v),
        Pred::IsNull(c, _) => t.column(*c).is_some(),
        Pred::Between(c, lo, hi) => lit_ok(t, *c, lo) && lit_ok(t, *c, hi),
        Pred::And(a, b) | Pred::Or(a, b) => pred_valid(t, a) && pred_valid(t, b),
        Pred::Not(a) => pred_valid(t, a),
    }
}

fn cmp(a: &Value, b: &Value) -> Option<Ordering> {
    (*a != Value::Null && *b != Value::Null).then(|| compare(a, b))
}

/// A predicate [`pred_valid`] for `t` on one of its rows: three-valued.
pub fn eval(t: &Table, row: &Row, p: &Pred) -> Option<bool> {
    match p {
        Pred::Cmp(c, op, v) => {
            let o = cmp(t.value(row, *c), &v.value)?;
            Some(match op {
                CmpOp::Eq => o.is_eq(),
                CmpOp::Ne => o.is_ne(),
                CmpOp::Lt => o.is_lt(),
                CmpOp::Le => o.is_le(),
                CmpOp::Gt => o.is_gt(),
                CmpOp::Ge => o.is_ge(),
            })
        },
        Pred::IsNull(c, not) => Some((*t.value(row, *c) == Value::Null) != *not),
        // `x BETWEEN a AND b` is `x >= a AND x <= b`.
        Pred::Between(c, lo, hi) => {
            let x = t.value(row, *c);
            and(cmp(x, &lo.value).map(Ordering::is_ge), cmp(x, &hi.value).map(Ordering::is_le))
        },
        Pred::And(a, b) => and(eval(t, row, a), eval(t, row, b)),
        Pred::Or(a, b) => match (eval(t, row, a), eval(t, row, b)) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        Pred::Not(a) => eval(t, row, a).map(|x| !x),
    }
}

fn and(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn selects(t: &Table, r: &Row, filter: &Option<Pred>) -> bool {
    filter.as_ref().is_none_or(|p| eval(t, r, p) == Some(true))
}

/// Whether `rows` are rows `t` may hold: values its column types admit,
/// no NULL in a NOT NULL column, no CHECK false, every unique constraint
/// kept, every index entry and row within its size limit.
pub fn rows_ok(t: &Table, rows: &[Row]) -> bool {
    rows.iter().all(|r| {
        r.len() == t.columns.len()
            && r.iter().zip(&t.columns).all(|(v, c)| c.ty.admits(v) && !(*v == Value::Null && (c.refuses_null() || t.key.contains(&c.id))))
            && t.all_checks().all(|p| eval(t, r, p) != Some(false))
            && t.partitioning.as_ref().is_none_or(|p| t.pos(p.column).is_some_and(|k| crate::model::routes(p, &r[k])))
    }) && t.uniques().iter().all(|(cols, nnd)| unique_ok(t, cols, *nnd, rows))
        && sizes_ok(t, rows)
        && gist_dates_ok(t, rows)
}

/// A known bug, excluded: `btree_gist`'s date penalty subtracts dates with
/// `date_mi`, which refuses infinite dates, so an insert into a GiST index
/// keyed on a date fails once the index has more than one page
/// (`../bugs/pg-btree-gist-infinite-date`). The index keeps an
/// infinite key after its row is deleted or updated away (in the dead
/// entry and in its inner pages' unions), and then every insert fails,
/// finite dates included. So a GiST index is keyed on a date column only
/// if the column has never held an infinite date ([`Table::infinite`]),
/// and such a column is never written one. Lifted when the target has the
/// fix ([`crate::fixed`]).
pub fn gist_dates_ok(t: &Table, rows: &[Row]) -> bool {
    if crate::fixed("btree-gist-infinite-date") {
        return true;
    }
    let keyed: Vec<usize> = t
        .indexes
        .iter()
        .filter(|i| i.method == Method::Gist)
        .flat_map(|i| i.columns.iter().enumerate().filter(move |(k, _)| !(*k == 0 && i.expr.is_some())).map(|(_, c)| c.column))
        .filter_map(|c| t.pos(c))
        .filter(|&p| t.columns[p].ty == crate::model::Type::Date)
        .collect();
    keyed.iter().all(|&p| !t.infinite.contains(&t.columns[p].id) && rows.iter().all(|r| !matches!(r[p], Value::Date(i32::MIN | i32::MAX))))
}

fn distinct<T: PartialEq>(xs: &[T]) -> bool {
    xs.iter().enumerate().all(|(k, x)| !xs[..k].contains(x))
}

/// The rows an INSERT adds to `t`, and how many values it takes of the
/// identity sequence (whose next value is `next`), or None when PostgreSQL
/// would refuse it. The rows of `t` with them must keep its constraints.
pub fn inserted_rows(t: &Table, next: i64, ins: &Insert) -> Option<(Vec<Row>, i64)> {
    let (new, taken) = inserted_new(t, next, ins)?;
    let all: Vec<Row> = t.rows.iter().cloned().chain(new.iter().cloned()).collect();
    rows_ok(t, &all).then_some((new, taken))
}

/// The rows an INSERT adds to `t` and the identity values it takes, or
/// None when it does not resolve (constraints not checked).
pub fn inserted_new(t: &Table, next: i64, ins: &Insert) -> Option<(Vec<Row>, i64)> {
    if !distinct(&ins.columns) || ins.rows.is_empty() || ins.columns.is_empty() && ins.rows.len() != 1 {
        return None;
    }
    let pos: Vec<usize> = ins.columns.iter().map(|&c| t.pos(c)).collect::<Option<_>>()?;
    let mut taken = 0;
    let mut new = Vec::with_capacity(ins.rows.len());
    for given in &ins.rows {
        if given.len() != pos.len() {
            return None;
        }
        let mut row = Vec::with_capacity(t.columns.len());
        for (k, c) in t.columns.iter().enumerate() {
            row.push(match pos.iter().position(|&p| p == k) {
                Some(j) => given[j].clone(),
                None if c.identity => {
                    let v = Value::Int(next.checked_add(taken)?);
                    taken += 1;
                    c.ty.admits(&v).then_some(v)?
                },
                None => c.default.clone().unwrap_or(Value::Null),
            });
        }
        if !row.iter().zip(&t.columns).all(|(v, c)| c.ty.admits(v)) {
            return None;
        }
        new.push(row);
    }
    Some((new, taken))
}

/// What a DML statement writes, foreign keys acting (see [`fk::write`]),
/// and the identity values it takes; None when it does not resolve.
pub fn dml_write(m: &Model, s: &Statement) -> Option<(Write, i64)> {
    match s {
        Statement::Insert(ins) => {
            let t = m.table(ins.table)?;
            let (new, taken) = inserted_new(t, next_value(m, t.id), ins)?;
            Some((fk::write(m, t.id, Change::Insert(new)), taken))
        },
        Statement::Update(u) => {
            let t = m.table(u.table)?;
            let cols: Vec<u32> = u.sets.iter().map(|(c, _)| *c).collect();
            if cols.is_empty() || !distinct(&cols) || !u.filter.as_ref().is_none_or(|p| pred_valid(t, p)) {
                return None;
            }
            if !u.sets.iter().all(|(c, v)| t.column(*c).is_some_and(|c| c.ty.admits(v))) {
                return None;
            }
            let new = updated_rows(t, u)?;
            let selected = t.rows.iter().map(|r| selects(t, r, &u.filter)).collect();
            Some((fk::write(m, t.id, Change::Update(new, selected, cols)), 0))
        },
        Statement::Delete(d) => {
            let t = m.table(d.table)?;
            if !d.filter.as_ref().is_none_or(|p| pred_valid(t, p)) {
                return None;
            }
            let gone = t.rows.iter().map(|r| selects(t, r, &d.filter)).collect();
            Some((fk::write(m, t.id, Change::Delete(gone)), 0))
        },
        Statement::Merge(mg) => merge_write(m, mg),
        // One MATCHED clause, UPDATE or DELETE (see `Statement::JoinWrite`).
        Statement::JoinWrite(mg) => {
            let [w] = &mg.whens[..] else { return None };
            if w.kind != MergeKind::Matched || !matches!(w.action, MergeAction::Update(_) | MergeAction::Delete) {
                return None;
            }
            merge_write(m, mg)
        },
        Statement::InsertSelect { table, columns, from } => {
            let t = m.table(*table)?;
            let src = source_rows(m, t, columns, from)?;
            // Rows come in the scan's order: identity values would follow
            // it, so an identity column is given.
            if t.identity().is_some_and(|c| !columns.contains(&c.id)) || !distinct(columns) {
                return None;
            }
            let ins = Insert { table: *table, columns: columns.clone(), rows: src };
            if ins.rows.is_empty() {
                // `inserted_new` refuses no rows; nothing is written.
                return Some((fk::write(m, t.id, Change::Insert(Vec::new())), 0));
            }
            let (new, taken) = inserted_new(t, next_value(m, t.id), &ins)?;
            Some((fk::write(m, t.id, Change::Insert(new)), taken))
        },
        Statement::Upsert(ins, oc) => {
            let t = m.table(ins.table)?;
            let (new, taken) = inserted_new(t, next_value(m, t.id), ins)?;
            let arbiters = arbiters(t, &oc.target)?;
            let update = match &oc.update {
                None => None,
                Some((sets, filter)) => {
                    let cols: Vec<u32> = sets.iter().map(|(c, _)| *c).collect();
                    let set_ok = |c: u32, v: &SetVal| {
                        t.column(c).is_some_and(|col| {
                            !col.identity
                                && t.partitioning.as_ref().is_none_or(|p| p.column != c)
                                && !fk::referenced_column(&m.tables, t.id, c)
                                && match v {
                                    SetVal::Const(v) => col.ty.admits(v),
                                    SetVal::Excluded(e) => t.column(*e).is_some_and(|x| x.ty == col.ty),
                                }
                        })
                    };
                    if oc.target.is_none() || cols.is_empty() || !distinct(&cols) || !sets.iter().all(|(c, v)| set_ok(*c, v)) || !filter.as_ref().is_none_or(|p| pred_valid(t, p)) {
                        return None;
                    }
                    let vals = sets
                        .iter()
                        .map(|(c, v)| {
                            let p = t.pos(*c).unwrap();
                            match v {
                                SetVal::Const(v) => (p, fk::UpsertVal::Const(v.clone())),
                                SetVal::Excluded(e) => (p, fk::UpsertVal::Excluded(t.pos(*e).unwrap())),
                            }
                        })
                        .collect();
                    Some((vals, cols, filter.clone()))
                },
            };
            Some((fk::write(m, t.id, Change::Upsert(new, arbiters, update)), taken))
        },
        _ => None,
    }
}

/// What a MERGE writes ([`Merge`]), the foreign keys acting, or None when
/// it does not resolve. Restrictions, each checked here:
///
/// - The target has no trigger and no self-referencing foreign key (the
///   rows' fates come first, then the keys' actions on the state after, as
///   for the other writes; a trigger's firing order would follow the join's
///   plan).
/// - An identity column is among the source's columns (inserted rows would
///   take its values in the plan's order).
/// - UPDATE does not set the identity column or the partition key.
/// - A clause without a condition is the last of its kind (PostgreSQL
///   refuses one after it as unreachable); INSERT only under NOT MATCHED,
///   UPDATE and DELETE never; no source value under NOT MATCHED BY SOURCE.
/// - Each target row takes the first MATCHED clause whose condition holds
///   on it, for each source row it matches (the join's order cannot change
///   which), or the first BY SOURCE clause when none matches; each source
///   row that matches none the first NOT MATCHED clause whose condition
///   holds on it. A row updated or deleted twice raises `21000`: not
///   generated ([`fk::Errors::other`]).
/// - Unique constraints: PostgreSQL checks each new row as it writes it, in
///   the plan's order, against the rows not yet deleted or updated away.
///   So no new row (inserted, or updated to another key) may share a key
///   with any row of the table before or another new row: then no order
///   meets a conflict, and none is generated otherwise.
/// The rows a source query of another table (or the target itself) gives,
/// as values of the target's `columns` (one source column of the same
/// type each), or None when it does not resolve.
pub fn source_rows(m: &Model, t: &Table, columns: &[u32], from: &FromTable) -> Option<Vec<Row>> {
    let s = m.table(from.table)?;
    if from.columns.len() != columns.len() || !from.filter.as_ref().is_none_or(|p| pred_valid(s, p)) {
        return None;
    }
    let pos: Vec<usize> = from.columns.iter().map(|c| s.pos(*c)).collect::<Option<_>>()?;
    let same = columns.iter().zip(&pos).all(|(c, &p)| t.column(*c).is_some_and(|col| col.ty == s.columns[p].ty));
    same.then(|| s.rows.iter().filter(|r| selects(s, r, &from.filter)).map(|r| pos.iter().map(|&p| r[p].clone()).collect()).collect())
}

pub fn merge_write(m: &Model, mg: &Merge) -> Option<(Write, i64)> {
    let t = m.table(mg.table)?;
    let rows_from;
    let rows: &Vec<Vec<Value>> = match &mg.from {
        Some(ft) => {
            if !mg.rows.is_empty() {
                return None;
            }
            rows_from = source_rows(m, t, &mg.columns.iter().map(|(c, _)| *c).collect::<Vec<_>>(), ft)?;
            &rows_from
        },
        None => &mg.rows,
    };
    if !t.triggers.is_empty() || t.fk.as_ref().is_some_and(|f| f.parent == t.id) {
        return None;
    }
    let cols: Vec<u32> = mg.columns.iter().map(|(c, _)| *c).collect();
    if cols.is_empty() || !distinct(&cols) || (rows.is_empty() && mg.from.is_none()) || mg.on.is_empty() || !distinct(&mg.on) || !mg.on.iter().all(|c| cols.contains(c)) || mg.whens.is_empty() || mg.whens.len() > 4 {
        return None;
    }
    let pos: Vec<usize> = mg.columns.iter().map(|(c, ty)| t.pos(*c).filter(|&p| t.columns[p].ty == *ty)).collect::<Option<_>>()?;
    if t.identity().is_some_and(|c| !cols.contains(&c.id)) {
        return None;
    }
    if !rows.iter().all(|r| r.len() == pos.len() && r.iter().zip(&pos).all(|(v, &p)| t.columns[p].ty.admits(v))) {
        return None;
    }
    // The source rows as rows of the target's shape (other columns NULL),
    // for conditions that read only source columns.
    let src: Vec<Row> = rows
        .iter()
        .map(|r| {
            let mut full = vec![Value::Null; t.columns.len()];
            for (v, &p) in r.iter().zip(&pos) {
                full[p] = v.clone();
            }
            full
        })
        .collect();
    let mut terminal = [false; 3];
    for w in &mg.whens {
        let k = w.kind as usize;
        if terminal[k] {
            return None;
        }
        terminal[k] |= w.cond.is_none();
        let cond_ok = w.cond.as_ref().is_none_or(|p| {
            pred_valid(t, p) && {
                let mut leaves = Vec::new();
                p.leaves(&mut leaves);
                w.kind != MergeKind::NotMatched || leaves.iter().all(|c| cols.contains(c))
            }
        });
        let action_ok = match (&w.action, w.kind) {
            (MergeAction::Insert, MergeKind::NotMatched) | (MergeAction::Nothing, _) => true,
            (MergeAction::Delete, MergeKind::Matched | MergeKind::BySource) => true,
            (MergeAction::Update(sets), MergeKind::Matched | MergeKind::BySource) => {
                let ids: Vec<u32> = sets.iter().map(|(c, _)| *c).collect();
                !sets.is_empty()
                    && distinct(&ids)
                    && sets.iter().all(|(c, v)| {
                        t.column(*c).is_some_and(|col| {
                            !col.identity
                                && t.partitioning.as_ref().is_none_or(|p| p.column != *c)
                                && match v {
                                    SetVal::Const(v) => col.ty.admits(v),
                                    SetVal::Excluded(e) => w.kind == MergeKind::Matched && mg.columns.iter().any(|(x, ty)| x == e && *ty == col.ty),
                                }
                        })
                    })
            },
            _ => false,
        };
        if !cond_ok || !action_ok {
            return None;
        }
    }
    let on: Vec<usize> = mg.on.iter().map(|c| t.pos(*c).unwrap()).collect();
    let matches = |r: &Row, s: &Row| on.iter().all(|&p| r[p] != Value::Null && s[p] != Value::Null && compare(&r[p], &s[p]).is_eq());
    let holds = |p: &Option<Pred>, r: &Row| p.as_ref().is_none_or(|p| eval(t, r, p) == Some(true));
    let mut twice = false;
    let mut fates: Vec<fk::Fate> = Vec::with_capacity(t.rows.len());
    let act = |w: &When, r: &Row, s: Option<&Row>| -> fk::Fate {
        match &w.action {
            MergeAction::Delete => fk::Fate::Delete,
            MergeAction::Update(sets) => {
                let mut nw = r.clone();
                for (c, v) in sets {
                    nw[t.pos(*c).unwrap()] = match v {
                        SetVal::Const(v) => v.clone(),
                        SetVal::Excluded(e) => s.map_or(Value::Null, |s| s[t.pos(*e).unwrap()].clone()),
                    };
                }
                fk::Fate::Update(nw, sets.iter().map(|(c, _)| *c).collect())
            },
            _ => fk::Fate::Keep,
        }
    };
    for r in &t.rows {
        let hits: Vec<&Row> = src.iter().filter(|s| matches(r, s)).collect();
        let mut fate = fk::Fate::Keep;
        if hits.is_empty() {
            if let Some(w) = mg.whens.iter().find(|w| w.kind == MergeKind::BySource && holds(&w.cond, r)) {
                fate = act(w, r, None);
            }
        } else {
            for s in hits {
                if let Some(w) = mg.whens.iter().find(|w| w.kind == MergeKind::Matched && holds(&w.cond, r)) {
                    let f = act(w, r, Some(s));
                    if f != fk::Fate::Keep {
                        twice |= fate != fk::Fate::Keep;
                        fate = f;
                    }
                }
            }
        }
        fates.push(fate);
    }
    let defaults: Row = t.columns.iter().map(|c| c.default.clone().unwrap_or(Value::Null)).collect();
    let mut inserted: Vec<Row> = Vec::new();
    for s in &src {
        if t.rows.iter().any(|r| matches(r, s)) {
            continue;
        }
        if let Some(w) = mg.whens.iter().find(|w| w.kind == MergeKind::NotMatched && holds(&w.cond, s)) {
            if w.action == MergeAction::Insert {
                let mut row = defaults.clone();
                for &p in &pos {
                    row[p] = s[p].clone();
                }
                inserted.push(row);
            }
        }
    }
    // No new row shares a key with an old row or another new one.
    let mut order_free = true;
    for (ucols, nnd) in t.uniques() {
        let up: Vec<usize> = ucols.iter().filter_map(|c| t.pos(*c)).collect();
        let mut all: Vec<Row> = t.rows.clone();
        all.extend(inserted.iter().cloned());
        for (o, f) in t.rows.iter().zip(&fates) {
            if let fk::Fate::Update(nw, _) = f {
                if !up.iter().all(|&p| (o[p] == Value::Null) == (nw[p] == Value::Null) && (o[p] == Value::Null || compare(&o[p], &nw[p]).is_eq())) {
                    all.push(nw.clone());
                }
            }
        }
        order_free &= crate::model::unique_ok(t, &ucols, nnd, &all);
    }
    let mut w = fk::write(m, t.id, Change::Merge(fates, inserted));
    if twice || !order_free {
        w.errors.other = true;
    }
    Some((w, 0))
}

/// The arbiters of an ON CONFLICT target on `t`: with none, every unique
/// constraint; with columns, every unique constraint on exactly those
/// columns (PostgreSQL's inference; at least one, else it refuses); with
/// `ON CONSTRAINT t{t}_pkey`, the primary key (there must be one). Unique
/// indexes are plain (no expression or predicate), so inference never
/// meets a partial one.
pub fn arbiters(t: &Table, target: &Option<Target>) -> Option<Vec<(Vec<usize>, bool)>> {
    let pos = |cols: &[u32]| cols.iter().map(|c| t.pos(*c)).collect::<Option<Vec<usize>>>();
    let all = t.uniques();
    let chosen: Vec<(Vec<u32>, bool)> = match target {
        None => all,
        Some(Target::Key) => {
            if t.key.is_empty() {
                return None;
            }
            vec![(t.key.clone(), false)]
        },
        Some(Target::Columns(cols)) => {
            if cols.is_empty() || !distinct(cols) {
                return None;
            }
            let mut want = cols.clone();
            want.sort();
            let hits: Vec<(Vec<u32>, bool)> = all
                .into_iter()
                .filter(|(c, _)| {
                    let mut c = c.clone();
                    c.sort();
                    c == want
                })
                .collect();
            if hits.is_empty() {
                return None;
            }
            hits
        },
    };
    chosen.iter().map(|(cols, nnd)| Some((pos(cols)?, *nnd))).collect()
}

/// The rows a write with RETURNING returns, from the rows it wrote: each
/// item of the old or new row (NULL where there is none; a plain column is
/// of the new row, or of the old one when there is no new one), then, for
/// a CTE read with the table's rows, those as the statement found them.
pub fn returned_rows(t: &Table, own: &[(Option<Row>, Option<Row>)], r: &Returning) -> Option<Vec<Row>> {
    let pos: Vec<usize> = r.items.iter().map(|(_, c)| t.pos(*c)).collect::<Option<_>>()?;
    let mut out: Vec<Row> = own
        .iter()
        .map(|(old, new)| {
            r.items
                .iter()
                .zip(&pos)
                .map(|((w, _), &p)| {
                    let row = match w {
                        Which::Old => old.as_ref(),
                        Which::New => new.as_ref(),
                        Which::Plain => new.as_ref().or(old.as_ref()),
                    };
                    row.map_or(Value::Null, |x| x[p].clone())
                })
                .collect()
        })
        .collect();
    if r.cte && r.snapshot {
        out.extend(t.rows.iter().map(|x| pos.iter().map(|&p| x[p].clone()).collect()));
    }
    Some(out)
}

/// The foreign key `ADD CONSTRAINT` would add to `t{table}`, if PostgreSQL
/// takes its definition, and whether the rows satisfy it.
fn fk_addable(m: &Model, table: u32, fk: &crate::model::ForeignKey) -> Option<bool> {
    let t = m.table(table)?;
    if t.fk.is_some() || !fk::def_ok(&m.tables, t, fk) {
        return None;
    }
    let mut with = t.clone();
    with.fk = Some(fk.clone());
    let mut tables = m.tables.clone();
    *tables.iter_mut().find(|x| x.id == table).unwrap() = with.clone();
    Some(fk::holds(&tables, &with))
}

/// The tables a DDL statement changes, with the tables linked to them by
/// the foreign keys it adds or drops: none of them may have deferred
/// trigger events pending ([`crate::model::TxnState::pending`]).
fn ddl_tables(m: &Model, s: &Statement) -> Vec<u32> {
    let linked = |t: u32| -> Vec<u32> {
        let mut out = vec![t];
        out.extend(fk::children(&m.tables, t));
        if let Some(f) = m.table(t).and_then(|x| x.fk.as_ref()) {
            out.push(f.parent);
        }
        out
    };
    match s {
        Statement::Truncate(t) => fk::truncated(&m.tables, *t).into_iter().flat_map(linked).collect(),
        Statement::DropTable(t) | Statement::DropColumn { table: t, .. } | Statement::AlterType { table: t, .. } | Statement::DropForeignKey(t) | Statement::ValidateForeignKey(t) => linked(*t),
        Statement::AddForeignKey { table, fk, .. } => vec![*table, fk.parent],
        Statement::CreateTable(def) => def.fk.iter().map(|f| f.parent).collect(),
        Statement::CreateIndex { table, .. }
        | Statement::DropIndex { table, .. }
        | Statement::AddColumn { table, .. }
        | Statement::SetNotNull { table, .. }
        | Statement::SetDefault { table, .. }
        | Statement::Cluster { table, .. }
        | Statement::Reindex { table, .. }
        | Statement::Phys { table, .. }
        | Statement::Trip { table, .. }
        | Statement::DropPartition { table, .. }
        | Statement::SetDomain { table, .. }
        | Statement::AddKey { table, .. }
        | Statement::DropKey(table)
        | Statement::AlterDomain { table, .. }
        | Statement::AddPartition { table, .. }
        | Statement::AddCheck { table, .. }
        | Statement::DropCheck { table, .. }
        | Statement::ValidateCheck { table, .. } => vec![*table],
        Statement::Expect(_, inner) => ddl_tables(m, inner),
        Statement::CreateTrigger { table, .. } | Statement::DropTrigger { table, .. } | Statement::EnableTrigger { table, .. } => vec![*table],
        _ => Vec::new(),
    }
}

/// The rows of `t` after an UPDATE, or None when a set does not resolve.
pub fn updated_rows(t: &Table, u: &Update) -> Option<Vec<Row>> {
    let sets: Vec<(usize, &Value)> = u.sets.iter().map(|(c, v)| Some((t.pos(*c)?, v))).collect::<Option<_>>()?;
    Some(
        t.rows
            .iter()
            .map(|r| {
                let mut r = r.clone();
                if selects(t, &r, &u.filter) {
                    for (p, v) in &sets {
                        r[*p] = (*v).clone();
                    }
                }
                r
            })
            .collect(),
    )
}

/// Whether a new table's definition is well formed: 1 to [`MAX_COLUMNS`]
/// columns of distinct ids below its `next_column`, at most one identity
/// column (an integer one without a default), defaults of the column's
/// type, a key of distinct columns, CHECK constraints that resolve; no
/// rows or indexes.
pub fn new_table_ok(t: &Table) -> bool {
    let ids: Vec<u32> = t.columns.iter().map(|c| c.id).collect();
    (1..=MAX_COLUMNS).contains(&t.columns.len())
        && distinct(&ids)
        && ids.iter().all(|&c| c < t.next_column)
        && t.columns.iter().all(column_ok)
        && t.columns.iter().filter(|c| c.identity).count() <= 1
        && distinct(&t.key)
        && t.key.iter().all(|&c| t.column(c).is_some())
        && t.checks.iter().all(|p| pred_valid(t, p))
        && t.named.is_empty()
        && t.next_check == 0
        && partitioned_ok(t)
        && t.rows.is_empty()
        && t.indexes.is_empty()
        && t.next_index == 0
        && t.widest.is_empty()
        && t.infinite.is_empty()
}

/// Whether a table's partitioning (if any) is one PostgreSQL takes: on a
/// column that is not the identity column, its key and every UNIQUE index
/// including that column (PostgreSQL refuses a unique constraint without
/// the partition key).
pub fn partitioned_ok(t: &Table) -> bool {
    let Some(p) = &t.partitioning else { return true };
    t.column(p.column).is_some_and(|c| !c.identity && crate::model::partitioning_ok(p, c.ty))
        && (t.key.is_empty() || t.key.contains(&p.column))
        && t.indexes.iter().all(|i| !i.unique || i.columns.iter().any(|k| k.column == p.column))
}

/// A column definition PostgreSQL takes.
fn column_ok(c: &Column) -> bool {
    (!c.identity || c.ty.is_int() && c.default.is_none())
        && c.default.as_ref().is_none_or(|v| *v != Value::Null && c.ty.admits(v))
        && (c.ty.varlena() || c.storage.is_none() && c.compression.is_none())
}

/// Whether an index definition is valid on `t` as it is.
pub fn index_ok(t: &Table, def: &Index) -> bool {
    let keys: Vec<u32> = def.columns.iter().map(|k| k.column).collect();
    let Some(mut types) = keys.iter().map(|&c| t.column(c).map(|c| c.ty)).collect::<Option<Vec<_>>>() else {
        return false;
    };
    let column_type = types.first().copied();
    // The first term's type is the expression's, when it has one.
    if let (Some(e), Some(t0)) = (def.expr, column_type) {
        types[0] = e.ret(t0);
    }
    let btree = def.method == Method::Btree;
    let ordered = def.columns.iter().all(|k| !k.desc && k.nulls_first.is_none());
    !keys.is_empty()
        && distinct(&keys)
        && types.iter().all(|&ty| method_takes(def.method, ty))
        && match def.method {
            Method::Btree => true,
            Method::Hash | Method::SpGist => keys.len() == 1 && ordered,
            Method::Brin | Method::Gin | Method::Gist => ordered,
        }
        && (btree || !def.unique)
        && (def.include.is_empty() || matches!(def.method, Method::Btree | Method::Gist | Method::SpGist))
        && distinct(&def.include)
        && def.include.iter().all(|c| t.column(*c).is_some() && !keys.contains(c))
        && def.expr.is_none_or(|e| column_type.is_some_and(|t0| e.applies(t0)))
        && (!def.nulls_not_distinct || def.unique)
        && def.filter.as_ref().is_none_or(|p| pred_valid(t, p))
        && def.params.fillfactor.is_none_or(|f| (10..=100).contains(&f) && !matches!(def.method, Method::Brin | Method::Gin))
        && (!def.params.no_dedup || btree)
        && def.params.pages_per_range.is_none_or(|p| p >= 1 && def.method == Method::Brin)
        // On a partitioned table a unique index includes the key.
        && (!def.unique || t.partitioning.as_ref().is_none_or(|p| keys.contains(&p.column)))
        // A unique index is plain, and only on rows that keep it.
        && (!def.unique || def.expr.is_none() && def.filter.is_none() && unique_ok(t, &keys, def.nulls_not_distinct, &t.rows))
        // Its entries fit, for the rows and for the widest values its
        // columns have held (dead row versions are indexed too).
        && def.columns.iter().map(|k| k.column).chain(def.include.iter().copied()).map(|c| t.widest.get(&c).copied().unwrap_or(0)).sum::<usize>() <= crate::model::MAX_ENTRY
        && {
            let mut with = t.clone();
            with.indexes.push(def.clone());
            sizes_ok(&with, &t.rows) && gist_dates_ok(&with, &t.rows)
        }
}

/// Whether crafted statistics name a table of `m` (and a column of it) and
/// give numbers PostgreSQL accepts.
pub fn stat_valid(m: &Model, s: &Stat) -> bool {
    match s {
        // At most 1000 tuples per page (see `crate::cases::draw_stats`).
        Stat::Relation { table, tuples, pages, .. } => m.table(*table).is_some() && tuples.is_finite() && *tuples >= -1.0 && f64::from(*tuples) <= 1000.0 * f64::from(*pages),
        Stat::Attribute { table, column, null_frac, distinct, .. } => {
            m.table(*table).is_some_and(|t| t.column(*column).is_some())
                && (0.0..=1.0).contains(null_frac)
                && distinct.is_finite()
                && *distinct >= -1.0
        },
    }
}

fn in_snapshot(m: &Model, id: u32) -> bool {
    m.savepoints.iter().any(|s| s.tables.iter().any(|t| t.id == id))
}

pub fn valid(m: &Model, s: &Statement) -> bool {
    if outside_transaction(s) && !m.savepoints.is_empty() {
        return false;
    }
    if !m.txn.pending.is_empty() && ddl_tables(m, s).iter().any(|t| m.txn.pending.contains(t)) {
        return false;
    }
    // REINDEX and CLUSTER of a partitioned table work partition by
    // partition in transactions of their own, so not inside one.
    if let Statement::Reindex { table, .. } | Statement::Cluster { table, .. } = s {
        if !m.savepoints.is_empty() && m.table(*table).is_some_and(|t| t.partitioning.is_some()) {
            return false;
        }
    }
    let table = |k: u32| m.table(k);
    let filter_ok = |t: &Table, f: &Option<Pred>| f.as_ref().is_none_or(|p| pred_valid(t, p));
    match s {
        Statement::Select(s) => table(s.table).is_some_and(|t| {
            s.columns.iter().all(|&c| t.column(c).is_some())
                && s.order.iter().all(|o| t.column(o.column).is_some())
                && filter_ok(t, &s.filter)
                && (!s.columns.is_empty() || !t.columns.is_empty())
        }),
        Statement::Count { table: k, filter } => table(*k).is_some_and(|t| filter_ok(t, filter)),
        // Not an upsert into a table with a domain column: its proposed
        // rows are coerced to the domain (and checked) before the conflict
        // check, the conflicting ones too (see [`set_domain`]).
        Statement::Upsert(i, _) if table(i.table).is_some_and(fk::has_domain) => false,
        Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::Upsert(..) | Statement::Merge(_) | Statement::JoinWrite(_) | Statement::InsertSelect { .. } => dml_write(m, s).is_some_and(|(w, _)| w.errors.none()),
        // RETURNING of a valid write, 1 to 4 items of its table's columns.
        Statement::Returning(inner, r) => {
            matches!(**inner, Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::Upsert(..) | Statement::Merge(_) | Statement::JoinWrite(_) | Statement::InsertSelect { .. })
                && (1..=4).contains(&r.items.len())
                && (r.cte || !r.snapshot)
                && inner.write_table().and_then(table).is_some_and(|t| r.items.iter().all(|(_, c)| t.column(*c).is_some()))
                && valid(m, inner)
        },
        Statement::Truncate(k) => table(*k).is_some(),
        // Not a table a trigger writes to (its function would fail).
        Statement::DropTable(k) => table(*k).is_some() && !m.tables.iter().any(|t| t.id != *k && t.triggers.iter().any(|tr| fk::audit_target(tr) == Some(*k))),
        Statement::CreateTable(def) => {
            table(def.id).is_none()
                && !in_snapshot(m, def.id)
                && m.tables.len() < MAX_TABLES
                && new_table_ok(def)
                && !def.unlogged
                && def.fk.as_ref().is_none_or(|f| fk::def_ok(&m.tables, def, f) && !fk::is_target(&m.tables, f.parent))
                && def.triggers.is_empty()
        },
        Statement::CreateIndex { table: k, def } => table(*k).is_some_and(|t| {
            t.index(def.id).is_none() && t.indexes.len() < MAX_INDEXES && index_ok(t, def)
        }),
        // Not a UNIQUE index a foreign key may depend on.
        Statement::DropIndex { table: k, index } => table(*k).is_some_and(|t| t.index(*index).is_some_and(|i| !fk::may_back(&m.tables, *k, i))),
        Statement::AddColumn { table: k, column } => table(*k).is_some_and(|t| {
            t.column(column.id).is_none()
                && t.columns.len() < MAX_COLUMNS
                && column_ok(column)
                // Identity values for existing rows would follow their
                // physical order, which the model does not know.
                && !column.identity
                && (!column.not_null || column.default.is_some() || t.rows.is_empty())
        }),
        Statement::DropColumn { table: k, column } => {
            table(*k).is_some_and(|t| {
                t.column(*column).is_some()
                    && !t.key.contains(column)
                    && t.columns.len() > 1
                    && t.partitioning.as_ref().is_none_or(|p| p.column != *column)
                    && !fk::referenced_column(&m.tables, *k, *column)
                    && !fk::trigger_column(&m.tables, *k, *column)
                    // Nor one an index a foreign key may depend on reads,
                    // `INCLUDE` too (the index would go with the column).
                    && !t.indexes.iter().any(|i| i.columns_read().contains(column) && fk::may_back(&m.tables, *k, i))
            })
        },
        Statement::SetNotNull { table: k, column, on } => table(*k).is_some_and(|t| {
            let Some(c) = t.column(*column) else {
                return false;
            };
            if *on {
                let p = t.pos(*column).unwrap();
                t.rows.iter().all(|r| r[p] != Value::Null)
            } else {
                !c.identity && !t.key.contains(column)
            }
        }),
        Statement::SetDefault { table: k, column, value } => table(*k).is_some_and(|t| {
            t.column(*column).is_some_and(|c| !c.identity && value.as_ref().is_none_or(|v| *v != Value::Null && c.ty.admits(v)))
        }),
        Statement::AlterType { table, column, ty, using } => matches!(alter_type(m, *table, *column, *ty, *using), Some(Ok(_))),
        Statement::Trip { table: k, op } => table(*k).is_some_and(|t| trip_ok(m, t, op)),
        Statement::SetDomain { table, column, dom, .. } => matches!(set_domain(m, *table, *column, dom), Some(Ok(_))),
        Statement::AddKey { table, columns, using } => matches!(add_key(m, *table, columns, *using), Some(Ok(_))),
        // Not of a table a foreign key references, itself too (it may
        // depend on the key's index: 2BP01).
        Statement::DropKey(k) => table(*k).is_some_and(|t| !t.key.is_empty()) && !m.tables.iter().any(|x| x.fk.as_ref().is_some_and(|f| f.parent == *k)),
        Statement::AlterDomain { table, column, op, .. } => matches!(alter_domain(m, *table, *column, op), Some(Ok(_))),
        Statement::DropPartition { table: k, name } => table(*k).is_some_and(|t| partition_droppable(m, t, *name)),
        Statement::AddPartition { table: k, name, bound } => table(*k).is_some_and(|t| partition_addable(t, *name, bound) == Some(true)),
        Statement::AddCheck { table: k, id, pred, .. } => table(*k).is_some_and(|t| check_addable(t, *id, pred) == Some(true)),
        Statement::DropCheck { table: k, id } | Statement::ValidateCheck { table: k, id } => table(*k).is_some_and(|t| t.named.iter().any(|(x, _)| x == id)),
        Statement::Vacuum { tables, .. } | Statement::Analyze(tables) => !tables.is_empty() && distinct(tables) && tables.iter().all(|&k| table(k).is_some()),
        Statement::Cluster { table: k, index } => {
            table(*k).is_some_and(|t| t.index(*index).is_some_and(|i| i.method == Method::Btree && i.filter.is_none()))
        },
        Statement::Reindex { table: k, index, .. } => table(*k).is_some_and(|t| index.is_none_or(|i| t.index(i).is_some())),
        Statement::Checkpoint | Statement::Set(_) => true,
        Statement::Stats(rows) => !rows.is_empty() && rows.iter().all(|r| stat_valid(m, r)),
        Statement::Savepoint(k) => *k == m.savepoints.len() && *k < MAX_SAVEPOINTS,
        Statement::RollbackTo(k) | Statement::Release(k) => *k < m.savepoints.len(),
        Statement::Rollback => !m.savepoints.is_empty(),
        Statement::Phys { table: k, op } => table(*k).is_some_and(|t| {
            (!op.storage() || t.partitioning.is_none())
                && match op {
                    Phys::Fillfactor(f) => f.is_none_or(|f| (10..=100).contains(&f)),
                    // Not of a table with an identity column: its sequence
                    // changes persistence too, a new relation file whose
                    // value a rollback takes back (the model keeps
                    // sequences outside transactions).
                    // Linked tables keep compatible persistence: SET LOGGED
                    // needs every table a foreign key links it to (either
                    // way) permanent, SET UNLOGGED every one unlogged.
                    Phys::Logged(on) => {
                        t.identity().is_none() && {
                            let mut linked = fk::children(&m.tables, t.id);
                            linked.extend(t.fk.as_ref().filter(|f| f.parent != t.id).map(|f| f.parent));
                            linked.iter().all(|l| m.table(*l).is_some_and(|l| l.unlogged != *on))
                        }
                    },
                    Phys::DropStats(_) => true,
                    Phys::Target(c, k) => t.column(*c).is_some() && (-1..=10000).contains(k),
                    Phys::NDistinct(c, x) => t.column(*c).is_some() && *x >= -100,
                    Phys::CreateStats(_, _, cols) => cols.len() >= 2 && distinct(cols) && cols.iter().all(|c| t.column(*c).is_some()),
                    Phys::ClusterOn(i) => i.is_none_or(|i| t.index(i).is_some_and(|i| i.method == Method::Btree && i.filter.is_none())),
                }
        }),
        // NOT VALID only on rows that satisfy it (see `crate::fk`).
        // Not linking a table a trigger writes to.
        Statement::AddForeignKey { table, fk, .. } => fk_addable(m, *table, fk) == Some(true) && !fk::is_target(&m.tables, *table) && !fk::is_target(&m.tables, fk.parent),
        Statement::DropForeignKey(k) | Statement::ValidateForeignKey(k) => table(*k).is_some_and(|t| t.fk.is_some()),
        Statement::AlterForeignKey { table: k, .. } => m.savepoints.is_empty() && table(*k).is_some_and(|t| t.fk.is_some()),
        // Inside a transaction (outside, PostgreSQL only warns), with
        // only savepoint 0 open: set in an inner savepoint that is then
        // released, a ROLLBACK TO does not undo it
        // (`../bugs/pg-set-constraints-released-savepoint`); a
        // named constraint must be deferrable.
        Statement::SetConstraints { table: k, .. } => {
            m.savepoints.len() == 1 && k.is_none_or(|k| table(k).is_some_and(|t| t.fk.as_ref().is_some_and(|f| f.deferral != crate::model::Deferral::No)))
        },
        Statement::CreateTrigger { table: k, trigger } => {
            table(*k).is_some_and(|t| t.triggers.iter().all(|x| x.id != trigger.id) && trigger.id >= t.next_trigger && t.triggers.len() < 4 && fk::trigger_ok(&m.tables, t, trigger))
        },
        Statement::DropTrigger { table: k, id } | Statement::EnableTrigger { table: k, id, .. } => table(*k).is_some_and(|t| t.triggers.iter().any(|x| x.id == *id)),
        Statement::Expect(code, inner) => match &**inner {
            // An INSERT that takes identity values and fails on a row
            // (not at the statement's end, as a foreign key does) takes
            // values only up to that row: only for one row.
            Statement::Insert(ins) if !matches!(code, fk::Code::ForeignKey | fk::Code::Restrict) && ins.rows.len() > 1 => {
                dml_write(m, inner).is_some_and(|(w, taken)| taken == 0 && w.errors.code() == Some(*code))
            },
            Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) => dml_write(m, inner).is_some_and(|(w, _)| w.errors.code() == Some(*code)),
            Statement::AddForeignKey { table, fk, not_valid: false } => {
                *code == fk::Code::ForeignKey && fk_addable(m, *table, fk) == Some(false) && !fk::is_target(&m.tables, *table) && !fk::is_target(&m.tables, fk.parent)
            },
            Statement::AlterType { table, column, ty, using } => matches!(alter_type(m, *table, *column, *ty, *using), Some(Err(c)) if c == *code),
            // A CHECK some row makes false: `23514` (NOT VALID would take it).
            Statement::AddCheck { table, id, pred, not_valid: false } => *code == Code::Check && table_of(m, *table).is_some_and(|t| check_addable(t, *id, pred) == Some(false)),
            // A partition whose bound takes rows the default partition
            // holds: `23514`.
            Statement::AddPartition { table, name, bound } => *code == Code::Check && table_of(m, *table).is_some_and(|t| partition_addable(t, *name, bound) == Some(false)),
            Statement::SetDomain { table, column, dom, .. } => matches!(set_domain(m, *table, *column, dom), Some(Err(c)) if c == *code),
            Statement::AddKey { table, columns, using } => matches!(add_key(m, *table, columns, *using), Some(Err(c)) if c == *code),
            Statement::AlterDomain { table, column, op, .. } => matches!(alter_domain(m, *table, *column, op), Some(Err(c)) if c == *code),
            // SET NOT NULL of a column holding NULL: `23502`.
            Statement::SetNotNull { table, column, on: true } => {
                *code == Code::NotNull && table_of(m, *table).is_some_and(|t| t.pos(*column).is_some_and(|p| t.rows.iter().any(|r| r[p] == Value::Null)))
            },
            _ => false,
        },
    }
}

/// Apply a successful write's tables to the model: the rows, the values
/// held, and the tables that may have deferred events pending.
fn apply_write(m: &mut Model, w: Write) {
    let pending = fk::pending_after(m, &w);
    for t in w.tables {
        if w.touched.contains(&t.id) {
            let rows = t.rows.clone();
            let mt = m.table_mut(t.id).unwrap();
            mt.rows = rows;
            let all = mt.rows.clone();
            mt.record(&all);
        }
    }
    m.txn.pending.extend(pending);
}

/// The next value of table `t`'s identity sequence.
pub fn next_value(m: &Model, t: u32) -> i64 {
    m.sequences.get(&t).copied().unwrap_or(1)
}

/// The rows a query must return: `groups` in order, the rows within a group
/// in any order, and of all that only the first `limit` rows (with no ORDER
/// BY, there is one group).
#[derive(Clone, Debug, PartialEq)]
pub struct Expected {
    pub groups: Vec<Vec<Row>>,
    pub limit: Option<usize>,
}

/// Rows in a total order under exact identity, for multiset comparison.
fn sort_exact(rows: &mut [Row]) {
    rows.sort_by(|a, b| a.iter().zip(b).map(|(x, y)| exact_cmp(x, y)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal));
}

/// A total order consistent with [`identical`]: kind, then value, reals by
/// their bits (every NaN alike), numerics by their text.
fn exact_cmp(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => {
            let key = |v: f64| if v.is_nan() { u64::MAX } else { v.to_bits() };
            key(*x).cmp(&key(*y))
        },
        (Value::Numeric(x), Value::Numeric(y)) => x.to_string().cmp(&y.to_string()),
        (Value::Null, Value::Null) => Ordering::Equal,
        _ if a == b => Ordering::Equal,
        _ => match compare(a, b) {
            Ordering::Equal => format!("{a:?}").cmp(&format!("{b:?}")),
            o => o,
        },
    }
}

/// Whether `part` is a sub-multiset of `whole`, rows compared exactly.
fn within(part: &[Row], whole: &[Row]) -> bool {
    let mut a = part.to_vec();
    let mut b = whole.to_vec();
    sort_exact(&mut a);
    sort_exact(&mut b);
    let same = |x: &Row, y: &Row| x.len() == y.len() && x.iter().zip(y).all(|(p, q)| identical(p, q));
    let lt = |x: &Row, y: &Row| x.iter().zip(y).map(|(p, q)| exact_cmp(p, q)).find(|o| o.is_ne()) == Some(Ordering::Less);
    let mut j = 0;
    for x in &a {
        while j < b.len() && lt(&b[j], x) {
            j += 1;
        }
        if j == b.len() || !same(&b[j], x) {
            return false;
        }
        j += 1;
    }
    true
}

impl Expected {
    pub fn len(&self) -> usize {
        let all: usize = self.groups.iter().map(Vec::len).sum();
        self.limit.map_or(all, |n| n.min(all))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether PostgreSQL's rows are a result this query may return.
    pub fn matches(&self, actual: &[Row]) -> bool {
        if actual.len() != self.len() {
            return false;
        }
        let mut at = 0;
        for g in &self.groups {
            if at == actual.len() {
                break;
            }
            let k = g.len().min(actual.len() - at);
            if !within(&actual[at..at + k], g) {
                return false;
            }
            at += k;
        }
        true
    }
}

/// Exact equality of two multisets of rows.
pub fn same_rows(a: &[Row], b: &[Row]) -> bool {
    a.len() == b.len() && within(a, b)
}

/// What a statement did: the rows a query must return, and the rows
/// written (inserted, updated or deleted: PostgreSQL's command tag count).
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub rows: Option<Expected>,
    pub written: Option<usize>,
}

fn nothing() -> Effect {
    Effect { rows: None, written: None }
}

fn order_rows(t: &Table, order: &[OrderTerm], a: &Row, b: &Row) -> Ordering {
    for o in order {
        let p = t.pos(o.column).unwrap();
        let c = order_nulls(&a[p], &b[p], o.desc, o.nulls_first);
        if c.is_ne() {
            return c;
        }
    }
    Ordering::Equal
}

/// Remove column position `p` from `t`, with what reads it.
fn drop_column(t: &mut Table, id: u32) {
    let p = t.pos(id).unwrap();
    t.columns.remove(p);
    for r in &mut t.rows {
        r.remove(p);
    }
    t.indexes.retain(|i| !i.columns_read().contains(&id));
    t.checks.retain(|c| !c.reads(id));
    t.named.retain(|(_, c)| !c.reads(id));
    t.widest.remove(&id);
    t.infinite.remove(&id);
}

fn table_of(m: &Model, k: u32) -> Option<&Table> {
    m.table(k)
}

/// Whether `ALTER TABLE t ADD CONSTRAINT t{t}_k{id} CHECK (pred)` is one
/// PostgreSQL takes (None if not drawn: a name in use, more than three,
/// a predicate that does not resolve), and whether every row satisfies it.
pub fn check_addable(t: &Table, id: u32, pred: &Pred) -> Option<bool> {
    (id >= t.next_check && t.named.len() < 3 && pred_valid(t, pred)).then(|| t.rows.iter().all(|r| eval(t, r, pred) != Some(false)))
}

/// Whether partition `t{t}_p{name}` may be detached and dropped: of RANGE
/// or LIST (the model does not know which rows a HASH partition holds),
/// not the last one, on a table no foreign key links (a detach checks the
/// references to its rows) and no trigger writes to (its rows would find
/// no partition), without an identity column (shared with the
/// partitions).
fn partition_droppable(m: &Model, t: &Table, name: u32) -> bool {
    t.partitioning.as_ref().is_some_and(|p| {
        p.strategy != crate::model::Strategy::Hash && p.parts.len() > 1 && p.position(name).is_some()
    }) && t.fk.is_none()
        && fk::children(&m.tables, t.id).is_empty()
        && !fk::is_target(&m.tables, t.id)
        && t.identity().is_none()
}

/// Whether `CREATE TABLE t{t}_p{name} PARTITION OF t bound` is one
/// PostgreSQL takes (None if not drawn: a name in use, HASH, a bound of
/// another kind, an overlap or a second DEFAULT, more than eight), and
/// whether no row the default partition holds falls in the new bound (else
/// `23514`; without a default every row is in another partition).
pub fn partition_addable(t: &Table, name: u32, bound: &crate::model::PartBound) -> Option<bool> {
    use crate::model::{PartBound, Strategy};
    let p = t.partitioning.as_ref()?;
    let ty = t.column(p.column)?.ty;
    let kind = matches!((p.strategy, bound), (Strategy::Range, PartBound::Range(..)) | (Strategy::List, PartBound::List(_)) | (Strategy::Range | Strategy::List, PartBound::Default));
    if !kind || p.position(name).is_some() || name >= 64 {
        return None;
    }
    let mut with = p.clone();
    with.parts.push(bound.clone());
    with.names.clear();
    if !crate::model::partitioning_ok(&with, ty) {
        return None;
    }
    let col = t.pos(p.column)?;
    let new = with.parts.len() - 1;
    Some(match p.parts.iter().position(|b| *b == PartBound::Default) {
        Some(d) if *bound != PartBound::Default => !t.rows.iter().any(|r| p.holds(d, &r[col]) && with.holds(new, &r[col])),
        _ => true,
    })
}

/// `ALTER TABLE t ADD CONSTRAINT t{t}_pkey PRIMARY KEY ..`: the table with
/// the key (its columns NOT NULL, as PostgreSQL 18 adds a NOT NULL
/// constraint for each, which stays when the key goes), or the one error
/// its rows raise (`23502` for a NULL, `23505` for a duplicate; None when
/// both, whose order depends on the scan), or None when not drawn: a key
/// already, 1 to 3 distinct columns, on a partitioned table one that has
/// the partition key; USING INDEX of a unique B-tree index on plain
/// columns in the default order, without INCLUDE or NULLS NOT DISTINCT,
/// not on a partitioned table (PostgreSQL refuses it there), which then is
/// the key's and no longer an index of its own.
pub fn add_key(m: &Model, k: u32, columns: &[u32], using: Option<u32>) -> Option<Result<Table, Code>> {
    let t = m.table(k)?;
    if !t.key.is_empty() || columns.is_empty() || columns.len() > 3 || !distinct(columns) {
        return None;
    }
    let mut new = t.clone();
    if let Some(i) = using {
        let ix = t.index(i)?;
        let plain = ix.unique
            && ix.method == Method::Btree
            && ix.expr.is_none()
            && ix.filter.is_none()
            && ix.include.is_empty()
            && !ix.nulls_not_distinct
            && ix.columns.iter().all(|c| !c.desc && c.nulls_first.is_none());
        if !plain || t.partitioning.is_some() || ix.columns.iter().map(|c| c.column).collect::<Vec<_>>() != columns {
            return None;
        }
        new.indexes.retain(|x| x.id != i);
    } else if !index_ok(t, &Index { unique: false, ..Index::btree(u32::MAX, columns) }) {
        return None;
    }
    new.key = columns.to_vec();
    if !partitioned_ok(&new) {
        return None;
    }
    let pos: Vec<usize> = columns.iter().map(|&c| t.pos(c)).collect::<Option<_>>()?;
    let nulls = t.rows.iter().any(|r| pos.iter().any(|&p| r[p] == Value::Null));
    let dups = !unique_ok(t, columns, false, &t.rows);
    match (nulls, dups) {
        (false, false) => {},
        (true, false) => return Some(Err(Code::NotNull)),
        (false, true) => return Some(Err(Code::Unique)),
        (true, true) => return None,
    }
    for &p in &pos {
        new.columns[p].not_null = true;
    }
    Some(Ok(new))
}

/// The one error rows raise against a column's new constraints: Ok when
/// none, None when both a NULL refused (`23502`) and a CHECK false
/// (`23514`) occur (which comes first depends on the physical order).
fn domain_errors(t: &Table, p: usize, not_null: bool, check: Option<&Pred>) -> Option<Result<(), Code>> {
    let nulls = not_null && t.rows.iter().any(|r| r[p] == Value::Null);
    let falses = check.is_some_and(|c| t.rows.iter().any(|r| eval(t, r, c) == Some(false)));
    match (nulls, falses) {
        (false, false) => Some(Ok(())),
        (true, false) => Some(Err(Code::NotNull)),
        (false, true) => Some(Err(Code::Check)),
        (true, true) => None,
    }
}

/// `CREATE DOMAIN t{t}_d{name} AS ty ..; ALTER TABLE t{t} ALTER COLUMN c
/// TYPE t{t}_d{name}`: the table with the column of the domain, or the one
/// error the rows raise, or None when not drawn. A value is checked
/// against its domain when it is coerced to it, which is before BEFORE
/// triggers run and before an upsert's conflict check (a row a trigger
/// skips, or one that conflicts, is still checked): so not on a table
/// with triggers or one a trigger writes to (and no trigger is created on
/// a table with a domain column, `fk::trigger_ok`), and no upsert into
/// such a table ([`valid`]). Otherwise the restrictions of
/// [`alter_type`] but defaults (a default is a constant of the base type,
/// coerced to the domain, and checked, at each INSERT that takes it, as
/// the model checks the rows written), a name in use, a CHECK that reads
/// another column or does not resolve.
pub fn set_domain(m: &Model, k: u32, column: u32, dom: &crate::model::Domain) -> Option<Result<Table, Code>> {
    let t = m.table(k)?;
    let p = t.pos(column)?;
    let col = &t.columns[p];
    // A known bug, excluded: a boolean clause on a column of a domain
    // over bool that an extended statistics object covers fails to plan
    // ("expression not found in statistics object",
    // `../bugs/pg-statext-bool-domain`): no domain over bool.
    if col.identity
        || col.ty == Type::Bool
        || !t.triggers.is_empty()
        || fk::is_target(&m.tables, k)
        || dom.name < t.next_check
        || t.partitioning.as_ref().is_some_and(|q| q.column == column)
        || t.fk.as_ref().is_some_and(|f| f.columns.contains(&column))
        || fk::referenced_column(&m.tables, k, column)
        || fk::trigger_column(&m.tables, k, column)
        || t.indexes.iter().any(|i| i.expr.is_some() && i.columns[0].column == column)
        || reads_backing_index(m, t, column)
        || dom.check.as_ref().is_some_and(|c| !pred_valid(t, c) || c.reads_other(column))
    {
        return None;
    }
    if let Err(code) = domain_errors(t, p, dom.not_null, dom.check.as_ref())? {
        return Some(Err(code));
    }
    let mut new = t.clone();
    new.columns[p].dom = Some(dom.clone());
    new.next_check = dom.name + 1;
    Some(Ok(new))
}

/// `ALTER DOMAIN` of the domain of column `c{column}` (see [`DomOp`]): the
/// table after it, or the one error its rows raise (a CHECK added that a
/// row breaks, `23514`; SET NOT NULL over a NULL, `23502`), or None when
/// not drawn (no domain, a CHECK to drop or validate that is not there,
/// a second one, DROP CONSTRAINT inside a transaction block).
pub fn alter_domain(m: &Model, k: u32, column: u32, op: &DomOp) -> Option<Result<Table, Code>> {
    let t = m.table(k)?;
    let p = t.pos(column)?;
    let dom = t.columns[p].dom.clone()?;
    let mut d = dom.clone();
    match op {
        DomOp::AddCheck(c, _) => {
            if dom.check.is_some() || !pred_valid(t, c) || c.reads_other(column) {
                return None;
            }
            if let Err(code) = domain_errors(t, p, false, Some(c))? {
                return Some(Err(code));
            }
            d.check = Some(c.clone());
        },
        DomOp::DropCheck | DomOp::Validate => {
            dom.check.as_ref()?;
            // A known bug, excluded: after DROP CONSTRAINT of a domain in a
            // transaction block, a statement failing in a subtransaction
            // later raises "cannot start a subtransaction when there are
            // unprocessed inval messages" (XX000) instead of its error
            // (`../bugs/pg-domain-inval-subxact`): outside a
            // transaction block only.
            if *op == DomOp::DropCheck && !m.savepoints.is_empty() {
                return None;
            }
            if *op == DomOp::DropCheck {
                d.check = None;
            }
        },
        DomOp::NotNull(on) => {
            if *on {
                if let Err(code) = domain_errors(t, p, true, None)? {
                    return Some(Err(code));
                }
            }
            d.not_null = *on;
        },
    }
    let mut new = t.clone();
    new.columns[p].dom = Some(d);
    Some(Ok(new))
}

/// Whether a round trip ([`Trip`]) names objects `t` has. A partition is
/// detached and attached again only on a table no foreign key links (a
/// detach checks the references to its rows; an attach clones the
/// table's key onto it) and without an identity column (its partitions
/// share the parent's sequence).
fn trip_ok(m: &Model, t: &Table, op: &Trip) -> bool {
    match op {
        Trip::Column(c) => t.column(*c).is_some(),
        Trip::Table => true,
        Trip::Index(i) => t.index(*i).is_some(),
        Trip::Key => !t.key.is_empty(),
        Trip::ForeignKey => t.fk.is_some(),
        Trip::Trigger(g) => t.triggers.iter().any(|x| x.id == *g),
        Trip::Partition(k, p) => {
            t.partitioning.as_ref() == Some(p) && *k < p.parts.len() && t.fk.is_none() && fk::children(&m.tables, t.id).is_empty() && t.identity().is_none()
        },
    }
}

// ------------------------------------------------------------------ ALTER COLUMN TYPE

/// How a column of type `from` converts to `to` in `ALTER COLUMN .. TYPE`:
/// Some(false) by an assignment cast (no USING needed), Some(true) by an
/// explicit cast only (`USING c::to`; without it PostgreSQL refuses with
/// `42804`). None for a pair not drawn: reals print to text and numeric
/// with a precision of their own (`float8out`, `float8_numeric`'s
/// `%.15g`), text reads as dates and booleans in many forms, and `int4` is
/// the only integer with a cast to and from boolean.
pub fn cast_needs_using(from: Type, to: Type) -> Option<bool> {
    use Type::*;
    if from == to {
        return None;
    }
    Some(match (from, to) {
        (a, b) if a.is_int() && b.is_int() => false,
        (a, Numeric | Float4 | Float8) if a.is_int() => false,
        (Numeric | Float4 | Float8, b) if b.is_int() => false,
        (Numeric, Float4 | Float8) | (Float4, Float8) | (Float8, Float4) => false,
        (Int2 | Int4 | Int8 | Numeric | Bool | Bytea | Date | Timestamp, Text) => false,
        (Date, Timestamp) | (Timestamp, Date) => false,
        (Text, Int2 | Int4 | Int8 | Numeric) | (Bool, Int4) | (Int4, Bool) => true,
        _ => return None,
    })
}

/// A value of type `from` converted to `to` (a pair [`cast_needs_using`]
/// takes) as the cast does: Err(Some(code)) for the error it raises,
/// Err(None) when the model does not know (text that is neither plainly
/// a number nor plainly not one: `int4in` takes `0x1F`, `1_000`, `+ 5 `).
/// Integers narrow or fail with `22003`; numeric and reals round to an
/// integer (numeric half away from zero, reals half to even, `rint`), NaN
/// and infinity fail (`0A000` from numeric, `22003` from a real); numeric
/// to a real goes through its text (`float8in`, which refuses overflow and
/// underflow to zero); a timestamp to its day; a date past the last
/// timestamp fails with `22008`.
pub fn convert(v: &Value, from: Type, to: Type) -> Result<Value, Option<Code>> {
    use crate::model::{Numeric, MICROS_PER_DAY};
    let int = |i: i128| -> Result<Value, Option<Code>> {
        let v = Value::Int(i64::try_from(i).map_err(|_| Some(Code::OutOfRange))?);
        if to.admits(&v) { Ok(v) } else { Err(Some(Code::OutOfRange)) }
    };
    let real = |x: f64| -> Result<Value, Option<Code>> {
        if from == Type::Float8 && to == Type::Float4 {
            let y = x as f32;
            if y.is_infinite() && !x.is_infinite() || y == 0.0 && x != 0.0 {
                return Err(Some(Code::OutOfRange));
            }
            return Ok(Value::Float(f64::from(y)));
        }
        Ok(Value::Float(x))
    };
    match (v, to) {
        (Value::Null, _) => Ok(Value::Null),
        (Value::Int(i), t) if t.is_int() => int(i128::from(*i)),
        (Value::Int(i), Type::Numeric) => Ok(Value::Numeric(Numeric::int(*i))),
        (Value::Int(i), Type::Float8) => Ok(Value::Float(*i as f64)),
        (Value::Int(i), Type::Float4) => Ok(Value::Float(f64::from(*i as f32))),
        (Value::Int(i), Type::Bool) => Ok(Value::Bool(*i != 0)),
        (Value::Bool(b), Type::Int4) => Ok(Value::Int(i64::from(*b))),
        (Value::Numeric(n), t) if t.is_int() => match n {
            Numeric::Finite { neg, digits, scale } => {
                let scale = usize::from(*scale);
                let mut padded = vec![b'0'; (scale + 1).saturating_sub(digits.len())];
                padded.extend_from_slice(digits);
                let (whole, frac) = padded.split_at(padded.len() - scale);
                let first = whole.iter().position(|&d| d != b'0').unwrap_or(whole.len());
                if whole.len() - first > 30 {
                    return Err(Some(Code::OutOfRange));
                }
                let mut i: i128 = std::str::from_utf8(&whole[first..]).unwrap().parse().unwrap_or(0);
                if frac.first().is_some_and(|&d| d >= b'5') {
                    i += 1;
                }
                int(if *neg { -i } else { i })
            },
            _ => Err(Some(Code::Unsupported)),
        },
        (Value::Numeric(n), Type::Float4 | Type::Float8) => match n {
            Numeric::NaN => Ok(Value::Float(f64::NAN)),
            Numeric::Inf => Ok(Value::Float(f64::INFINITY)),
            Numeric::NegInf => Ok(Value::Float(f64::NEG_INFINITY)),
            Numeric::Finite { digits, .. } => {
                let s = n.to_string();
                let x = if to == Type::Float4 { s.parse::<f32>().map(f64::from) } else { s.parse::<f64>() }.map_err(|_| None)?;
                if x.is_infinite() || x == 0.0 && !digits.is_empty() {
                    return Err(Some(Code::OutOfRange));
                }
                Ok(Value::Float(x))
            },
        },
        (Value::Float(x), t) if t.is_int() => {
            let r = x.round_ties_even();
            let bound = match t {
                Type::Int2 => 32768.0,
                Type::Int4 => 2147483648.0,
                _ => 9223372036854775808.0,
            };
            if r >= -bound && r < bound { int(r as i128) } else { Err(Some(Code::OutOfRange)) }
        },
        (Value::Float(x), Type::Float4 | Type::Float8) => real(*x),
        (Value::Int(_) | Value::Numeric(_) | Value::Bool(_) | Value::Bytea(_) | Value::Date(_) | Value::Timestamp(_), Type::Text) => Ok(Value::Text(v.text().unwrap())),
        (Value::Date(d), Type::Timestamp) => match *d {
            i32::MIN => Ok(Value::Timestamp(i64::MIN)),
            i32::MAX => Ok(Value::Timestamp(i64::MAX)),
            d => {
                match i64::from(d).checked_mul(MICROS_PER_DAY).map(Value::Timestamp) {
                    Some(t) if Type::Timestamp.admits(&t) => Ok(t),
                    _ => Err(Some(Code::DatetimeOverflow)),
                }
            },
        },
        (Value::Timestamp(t), Type::Date) => Ok(Value::Date(match *t {
            i64::MIN => i32::MIN,
            i64::MAX => i32::MAX,
            t => t.div_euclid(MICROS_PER_DAY) as i32,
        })),
        (Value::Text(s), t) if t.is_int() => {
            let body = s.strip_prefix('-').unwrap_or(s);
            if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) {
                let digits = body.trim_start_matches('0');
                if digits.len() > 30 {
                    return Err(Some(Code::OutOfRange));
                }
                let i: i128 = digits.parse().unwrap_or(0);
                int(if body.len() < s.len() { -i } else { i })
            } else if !s.bytes().any(|b| b.is_ascii_digit()) {
                Err(Some(Code::InvalidText))
            } else {
                Err(None)
            }
        },
        (Value::Text(s), Type::Numeric) => match Numeric::parse(s) {
            Some(n) => Ok(Value::Numeric(n)),
            None if !s.bytes().any(|b| b.is_ascii_digit())
                && !["nan", "infinity", "+infinity", "-infinity", "inf", "+inf", "-inf"].contains(&s.trim_matches(|c: char| c.is_ascii_whitespace()).to_ascii_lowercase().as_str()) =>
            {
                Err(Some(Code::InvalidText))
            },
            None => Err(None),
        },
        _ => Err(None),
    }
}

/// Whether an index that a foreign key may depend on reads column
/// `c{column}` (an `INCLUDE` column too): rebuilding it for a new type
/// drops it RESTRICT, which the foreign key's dependency stops (`2BP01`,
/// `../bugs/pg-alter-type-include-fk-index`); a key column of
/// it is the foreign key's own, excluded anyway.
fn reads_backing_index(m: &Model, t: &Table, column: u32) -> bool {
    t.indexes.iter().any(|i| i.columns_read().contains(&column) && fk::may_back(&m.tables, t.id, i))
}

/// Whether a CHECK constraint of `t` (not the column's domain's own) or a
/// partial index predicate compares column `c{column}` while it is of a
/// domain type. The parser relabels a domain column to the domain's base
/// type under an operator, and the stored definition prints that as an
/// explicit cast (`CHECK (((c0)::smallint > 0))`). ALTER COLUMN TYPE
/// rebuilds the constraint or index by re-parsing that stored text, so the
/// old base type stays pinned: a later INSERT of a value the old type
/// cannot hold fails with 22003, and the comparison keeps its old-type
/// meaning. PostgreSQL does this on purpose (ruleutils.c prints implicit
/// casts so a re-parse picks the same operator; alter_table's regression
/// test expects it), not a bug:
/// `../bugs/discarded/pg-alter-type-domain-cast-narrowing` (runner
/// clusters `*-statement-*-22003`). The model would compare with the new
/// type, so such a column is not retyped. `IS NULL` takes no cast.
fn domain_cast_pinned(t: &Table, column: u32) -> bool {
    fn compares(p: &Pred, column: u32) -> bool {
        match p {
            Pred::Cmp(c, ..) | Pred::Between(c, ..) => *c == column,
            Pred::IsNull(..) => false,
            Pred::And(a, b) | Pred::Or(a, b) => compares(a, column) || compares(b, column),
            Pred::Not(a) => compares(a, column),
        }
    }
    t.column(column).is_some_and(|c| c.dom.is_some())
        && (t.checks.iter().chain(t.named.iter().map(|(_, p)| p)).any(|p| compares(p, column))
            || t.indexes.iter().any(|i| i.filter.as_ref().is_some_and(|f| compares(f, column))))
}

/// `ALTER TABLE t{k} ALTER COLUMN c{column} TYPE ty [USING ..]`: the table
/// after it (Ok), the one error PostgreSQL must raise (Err), or None when
/// the statement is not drawn. Not drawn (each a refusal or a rebuild the
/// model does not follow):
/// - the identity column (its sequence changes type too), the partition
///   key (`42P16`), a column of a foreign key either way (its types must
///   match), one a trigger reads or writes (a WHEN refuses with `0A000`;
///   functions assign and copy typed values), one an expression index
///   reads, or one an index of a method that does not take the new type
///   reads;
/// - a column whose CHECK constraints or partial index predicates do not
///   compare with the new type as the model does ([`pred_valid`]):
///   PostgreSQL re-parses their text against the new type;
/// - a column of a domain type that a table CHECK constraint or a partial
///   index's predicate compares ([`domain_cast_pinned`]);
/// - a column with a default: PostgreSQL keeps the default's constant
///   under an implicit cast, and at the next ALTER strips that cast and
///   converts the constant again from its first type
///   (`strip_implicit_coercions`), so the outcome depends on the default's
///   history (a numeric default of a column now text converts to int4
///   without USING; one that does not convert fails only at an INSERT);
/// - rows the result breaks (a NOT NULL, a CHECK, a unique constraint,
///   sizes), or rows with conversion errors of more than one code (which
///   one comes first depends on the physical order), or with an error and
///   another row that a CHECK would refuse.
/// The table is rewritten: only live rows are copied, so the column's
/// widest value is that of its new values.
pub fn alter_type(m: &Model, k: u32, column: u32, ty: Type, using: bool) -> Option<Result<Table, Code>> {
    let t = m.table(k)?;
    let p = t.pos(column)?;
    let col = &t.columns[p];
    let needs = cast_needs_using(col.ty, ty)?;
    if col.identity
        || t.partitioning.as_ref().is_some_and(|q| q.column == column)
        || t.fk.as_ref().is_some_and(|f| f.columns.contains(&column))
        || fk::referenced_column(&m.tables, k, column)
        || fk::trigger_column(&m.tables, k, column)
        || t.indexes.iter().any(|i| i.columns_read().contains(&column) && (i.expr.is_some() && i.columns[0].column == column || !method_takes(i.method, ty)))
        || reads_backing_index(m, t, column)
    {
        return None;
    }
    if col.default.is_some() || domain_cast_pinned(t, column) {
        return None;
    }
    if needs && !using {
        return Some(Err(Code::Mismatch));
    }
    let mut new = t.clone();
    let c = &mut new.columns[p];
    c.ty = ty;
    c.dom = None;
    c.storage = None;
    if !ty.varlena() {
        c.compression = None;
    }
    if !new.all_checks().all(|c| pred_valid(&new, c)) || !new.indexes.iter().all(|i| i.filter.as_ref().is_none_or(|f| pred_valid(&new, f))) {
        return None;
    }
    let mut codes: Vec<Code> = Vec::new();
    let mut converted: Vec<Row> = Vec::new();
    for r in &t.rows {
        match convert(&r[p], col.ty, ty) {
            Ok(v) => {
                let mut r = r.clone();
                r[p] = v;
                converted.push(r);
            },
            Err(None) => return None,
            Err(Some(code)) => codes.push(code),
        }
    }
    if let Some(&code) = codes.first() {
        let checks_pass = converted.iter().all(|r| new.all_checks().all(|c| eval(&new, r, c) != Some(false)));
        return (codes.iter().all(|&c| c == code) && checks_pass).then_some(Err(code));
    }
    if !rows_ok(&new, &converted) {
        return None;
    }
    new.rows = converted;
    new.widest.insert(column, new.rows.iter().map(|r| r[p].width()).max().unwrap_or(0));
    new.infinite.remove(&column);
    if new.rows.iter().any(|r| matches!(r[p], Value::Date(i32::MIN | i32::MAX))) {
        new.infinite.insert(column);
    }
    Some(Ok(new))
}

/// Apply a statement [`valid`] against `m` to it.
pub fn apply(m: &mut Model, s: &Statement) -> Effect {
    match s {
        Statement::Select(s) => {
            let t = m.table(s.table).unwrap();
            let pos: Vec<usize> = if s.columns.is_empty() { (0..t.columns.len()).collect() } else { s.columns.iter().map(|&c| t.pos(c).unwrap()).collect() };
            let mut hits: Vec<&Row> = t.rows.iter().filter(|r| selects(t, r, &s.filter)).collect();
            let project = |r: &Row| -> Row { pos.iter().map(|&p| r[p].clone()).collect() };
            let mut groups: Vec<Vec<Row>> = Vec::new();
            if s.order.is_empty() {
                groups.push(hits.iter().map(|r| project(r)).collect());
            } else {
                hits.sort_by(|a, b| order_rows(t, &s.order, a, b));
                let mut last: Option<&Row> = None;
                for r in hits {
                    if last.is_none_or(|l| order_rows(t, &s.order, l, r).is_ne()) {
                        groups.push(Vec::new());
                    }
                    groups.last_mut().unwrap().push(project(r));
                    last = Some(r);
                }
            }
            Effect { rows: Some(Expected { groups, limit: s.limit.map(|n| n as usize) }), written: None }
        },
        Statement::Count { table, filter } => {
            let t = m.table(*table).unwrap();
            let n = t.rows.iter().filter(|r| selects(t, r, filter)).count();
            Effect { rows: Some(Expected { groups: vec![vec![vec![Value::Int(n as i64)]]], limit: None }), written: None }
        },
        Statement::Returning(inner, r) => {
            let (w, _) = dml_write(m, inner).expect("a valid write");
            let t = m.table(inner.write_table().unwrap()).unwrap();
            let rows = returned_rows(t, &w.own, r).expect("valid items");
            apply(m, inner);
            Effect { rows: Some(Expected { groups: vec![rows], limit: None }), written: None }
        },
        Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::Upsert(..) | Statement::Merge(_) | Statement::JoinWrite(_) | Statement::InsertSelect { .. } => {
            let (w, taken) = dml_write(m, s).expect("a valid write");
            let n = w.written;
            if taken > 0 {
                let t = s.write_table().unwrap();
                let next = next_value(m, t);
                m.sequences.insert(t, next + taken);
            }
            apply_write(m, w);
            Effect { rows: None, written: Some(n) }
        },
        Statement::Truncate(k) => {
            for t in fk::truncated(&m.tables, *k) {
                m.table_mut(t).unwrap().rows.clear();
            }
            nothing()
        },
        Statement::CreateTable(def) => {
            m.tables.push(def.clone());
            m.next_table = m.next_table.max(def.id + 1);
            if def.identity().is_some() {
                m.sequences.insert(def.id, 1);
            }
            nothing()
        },
        Statement::DropTable(k) => {
            m.tables.retain(|t| t.id != *k);
            for t in &mut m.tables {
                if t.fk.as_ref().is_some_and(|f| f.parent == *k) {
                    t.fk = None;
                    m.txn.named.remove(&t.id);
                }
            }
            m.txn.named.remove(k);
            nothing()
        },
        Statement::CreateIndex { table, def } => {
            let t = m.table_mut(*table).unwrap();
            t.indexes.push(def.clone());
            t.next_index = t.next_index.max(def.id + 1);
            nothing()
        },
        Statement::DropIndex { table, index } => {
            m.table_mut(*table).unwrap().indexes.retain(|i| i.id != *index);
            nothing()
        },
        Statement::AddColumn { table, column } => {
            let t = m.table_mut(*table).unwrap();
            let v = column.default.clone().unwrap_or(Value::Null);
            for r in &mut t.rows {
                r.push(v.clone());
            }
            t.columns.push(column.clone());
            t.next_column = t.next_column.max(column.id + 1);
            t.widest.insert(column.id, v.width());
            if matches!(v, Value::Date(i32::MIN | i32::MAX)) {
                t.infinite.insert(column.id);
            }
            nothing()
        },
        Statement::DropColumn { table, column } => {
            let t = m.table_mut(*table).unwrap();
            drop_column(t, *column);
            // A foreign key on the column goes with it.
            if t.fk.as_ref().is_some_and(|f| f.columns.contains(column)) {
                t.fk = None;
                m.txn.named.remove(table);
            }
            nothing()
        },
        Statement::SetNotNull { table, column, on } => {
            let t = m.table_mut(*table).unwrap();
            let p = t.pos(*column).unwrap();
            t.columns[p].not_null = *on;
            nothing()
        },
        Statement::SetDefault { table, column, value } => {
            let t = m.table_mut(*table).unwrap();
            let p = t.pos(*column).unwrap();
            t.columns[p].default = value.clone();
            nothing()
        },
        Statement::Trip { .. } | Statement::ValidateCheck { .. } => nothing(),
        Statement::AddCheck { table, id, pred, .. } => {
            let t = m.table_mut(*table).unwrap();
            t.named.push((*id, pred.clone()));
            t.next_check = t.next_check.max(id + 1);
            nothing()
        },
        Statement::DropCheck { table, id } => {
            m.table_mut(*table).unwrap().named.retain(|(x, _)| x != id);
            nothing()
        },
        Statement::AddKey { table, columns, using } => {
            let Some(Ok(new)) = add_key(m, *table, columns, *using) else { unreachable!() };
            *m.table_mut(*table).unwrap() = new;
            nothing()
        },
        Statement::DropKey(k) => {
            // The NOT NULL constraints of its columns stay.
            let t = m.table_mut(*k).unwrap();
            for c in std::mem::take(&mut t.key) {
                let p = t.pos(c).unwrap();
                t.columns[p].not_null = true;
            }
            nothing()
        },
        Statement::SetDomain { table, column, dom, .. } => {
            let Some(Ok(new)) = set_domain(m, *table, *column, dom) else { unreachable!() };
            *m.table_mut(*table).unwrap() = new;
            nothing()
        },
        Statement::AlterDomain { table, column, op, .. } => {
            let Some(Ok(new)) = alter_domain(m, *table, *column, op) else { unreachable!() };
            *m.table_mut(*table).unwrap() = new;
            nothing()
        },
        Statement::DropPartition { table, name } => {
            let t = m.table_mut(*table).unwrap();
            let mut p = t.partitioning.clone().unwrap();
            let k = p.position(*name).unwrap();
            let col = t.pos(p.column).unwrap();
            t.rows.retain(|r| !p.holds(k, &r[col]));
            p.names = (0..p.parts.len()).map(|j| p.name(j)).collect();
            p.parts.remove(k);
            p.names.remove(k);
            t.partitioning = Some(p);
            nothing()
        },
        Statement::AddPartition { table, name, bound } => {
            let t = m.table_mut(*table).unwrap();
            let p = t.partitioning.as_mut().unwrap();
            p.names = (0..p.parts.len()).map(|j| p.name(j)).collect();
            p.parts.push(bound.clone());
            p.names.push(*name);
            nothing()
        },
        Statement::AlterType { table, column, ty, using } => {
            let Some(Ok(new)) = alter_type(m, *table, *column, *ty, *using) else { unreachable!() };
            *m.table_mut(*table).unwrap() = new;
            nothing()
        },
        Statement::Vacuum { .. }
        | Statement::Analyze(_)
        | Statement::Cluster { .. }
        | Statement::Reindex { .. }
        | Statement::Checkpoint
        | Statement::Set(_)
        | Statement::Stats(_)
        | Statement::ValidateForeignKey(_) => nothing(),
        Statement::Phys { table, op } => {
            if let Phys::Logged(on) = op {
                m.table_mut(*table).unwrap().unlogged = !on;
            }
            nothing()
        },
        Statement::AddForeignKey { table, fk, .. } => {
            m.table_mut(*table).unwrap().fk = Some(fk.clone());
            nothing()
        },
        Statement::DropForeignKey(t) => {
            m.table_mut(*t).unwrap().fk = None;
            m.txn.named.remove(t);
            nothing()
        },
        Statement::AlterForeignKey { table, deferral } => {
            m.table_mut(*table).unwrap().fk.as_mut().unwrap().deferral = *deferral;
            nothing()
        },
        Statement::SetConstraints { table, deferred } => {
            match table {
                Some(t) => {
                    m.txn.named.insert(*t, *deferred);
                },
                None => {
                    m.txn.all = Some(*deferred);
                    m.txn.named.clear();
                    // Every deferred event fires now.
                    if !*deferred {
                        m.txn.pending.clear();
                    }
                },
            }
            nothing()
        },
        Statement::CreateTrigger { table, trigger } => {
            let t = m.table_mut(*table).unwrap();
            t.triggers.push(trigger.clone());
            t.next_trigger = t.next_trigger.max(trigger.id + 1);
            nothing()
        },
        Statement::DropTrigger { table, id } => {
            m.table_mut(*table).unwrap().triggers.retain(|x| x.id != *id);
            nothing()
        },
        Statement::EnableTrigger { table, id, on } => {
            let t = m.table_mut(*table).unwrap();
            t.triggers.iter_mut().find(|x| x.id == *id).unwrap().enabled = *on;
            nothing()
        },
        Statement::Expect(_, inner) => {
            // Nothing stays but the identity values taken and the values
            // the indexes saw.
            if let Some((w, taken)) = dml_write(m, inner) {
                if let Statement::Insert(ins) = &**inner {
                    if taken > 0 {
                        let next = next_value(m, ins.table);
                        m.sequences.insert(ins.table, next + taken);
                    }
                }
                for t in &w.tables {
                    if w.touched.contains(&t.id) {
                        m.table_mut(t.id).unwrap().record(&t.rows);
                    }
                }
            }
            nothing()
        },
        Statement::Savepoint(_) => {
            m.savepoints.push(Snapshot { tables: m.tables.clone(), txn: m.txn.clone() });
            nothing()
        },
        Statement::RollbackTo(k) => {
            m.savepoints.truncate(k + 1);
            m.tables = m.savepoints[*k].tables.clone();
            m.txn = m.savepoints[*k].txn.clone();
            nothing()
        },
        Statement::Release(k) => {
            m.savepoints.truncate(*k);
            if *k == 0 {
                m.txn = Default::default();
            }
            nothing()
        },
        Statement::Rollback => {
            m.tables = m.savepoints.swap_remove(0).tables;
            m.savepoints.clear();
            m.txn = Default::default();
            nothing()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Action, Deferral, Domain, ForeignKey, Numeric, Type};

    #[test]
    fn unique_constraints_follow_postgres_equality() {
        let mut t = Table::new(0, &[Type::Float8, Type::Numeric, Type::Int4]);
        t.indexes.push(Index { unique: true, ..Index::btree(0, &[0]) });
        let row = |a: Value, b: Value| vec![a, b, Value::Null];
        // -0 = 0 and NaN = NaN.
        assert!(!rows_ok(&t, &[row(Value::Float(0.0), Value::Null), row(Value::Float(-0.0), Value::Null)]));
        assert!(!rows_ok(&t, &[row(Value::Float(f64::NAN), Value::Null), row(Value::Float(-f64::NAN), Value::Null)]));
        assert!(rows_ok(&t, &[row(Value::Null, Value::Null), row(Value::Null, Value::Null)]));
        t.indexes[0].nulls_not_distinct = true;
        assert!(!rows_ok(&t, &[row(Value::Null, Value::Null), row(Value::Null, Value::Null)]));
        // 1.0 = 1.00.
        t.indexes = vec![Index { unique: true, ..Index::btree(1, &[1]) }];
        let n = |s: &str| Value::Numeric(Numeric::parse(s).unwrap());
        assert!(!rows_ok(&t, &[row(Value::Null, n("1.0")), row(Value::Null, n("1.00"))]));
    }

    #[test]
    fn a_column_an_index_a_foreign_key_depends_on_reads_stays() {
        // `CREATE UNIQUE INDEX t0_i0 ON t0 (c1) INCLUDE (c0)` and t1's key
        // references t0 (c1): the constraint depends on the index, so
        // DROP COLUMN c0 needs CASCADE (2BP01, run 20261021-1716).
        let mut parent = Table::new(0, &[Type::Int8, Type::Date]);
        parent.indexes.push(Index { unique: true, include: vec![0], ..Index::btree(0, &[1]) });
        let mut child = Table::new(1, &[Type::Date]);
        child.fk = Some(ForeignKey {
            columns: vec![0],
            parent: 0,
            parent_columns: vec![1],
            on_delete: Action::NoAction,
            delete_set: vec![],
            on_update: Action::NoAction,
            full: false,
            deferral: Deferral::No,
        });
        let mut m = Model { tables: vec![parent, child], ..Model::default() };
        let drop = Statement::DropColumn { table: 0, column: 0 };
        assert!(!valid(&m, &drop));
        assert!(!valid(&m, &Statement::DropIndex { table: 0, index: 0 }));
        // A partial unique index is never the one.
        m.tables[0].indexes[0].filter = Some(Pred::IsNull(1, false));
        assert!(valid(&m, &drop));
        m.tables[0].indexes[0].filter = None;
        m.tables[1].fk = None;
        assert!(valid(&m, &drop));
    }

    #[test]
    fn alter_type_of_a_domain_column_a_check_compares_is_not_drawn() {
        // ../bugs/discarded/pg-alter-type-domain-cast-narrowing, minimal.sql:
        // CREATE DOMAIN small_id AS smallint; CREATE TABLE t (id small_id CHECK (id > 0));
        // ALTER TABLE t ALTER COLUMN id TYPE bigint; INSERT INTO t VALUES (100000) -> 22003,
        // the CHECK being `((id)::smallint > 0)` after the rebuild.
        let positive = Pred::Cmp(0, CmpOp::Gt, Lit::plain(Value::Int(0)));
        let mut t = Table::new(0, &[Type::Int2]);
        t.checks.push(positive.clone());
        let mut m = Model { tables: vec![t], ..Model::default() };
        // the control: a plain smallint column is retyped, its CHECK compares as bigint
        let new = alter_type(&m, 0, 0, Type::Int8, false).expect("drawn").expect("no error");
        assert_eq!(new.columns[0].ty, Type::Int8);
        m.tables[0].columns[0].dom = Some(Domain { name: 0, not_null: false, check: None });
        assert!(alter_type(&m, 0, 0, Type::Int8, false).is_none());
        assert!(!valid(&m, &Statement::AlterType { table: 0, column: 0, ty: Type::Int8, using: false }));
        // a partial index's predicate pins it too; IS NULL takes no cast
        m.tables[0].checks = vec![Pred::IsNull(0, true)];
        assert!(alter_type(&m, 0, 0, Type::Int8, false).is_some());
        m.tables[0].indexes.push(Index { filter: Some(positive.clone()), ..Index::btree(0, &[0]) });
        assert!(alter_type(&m, 0, 0, Type::Int8, false).is_none());
        // the domain's own CHECK is not the table's: dropped with the domain
        m.tables[0].indexes.clear();
        m.tables[0].columns[0].dom = Some(Domain { name: 0, not_null: false, check: Some(positive) });
        assert!(alter_type(&m, 0, 0, Type::Int8, false).is_some());
    }

    #[test]
    fn identity_values_come_from_the_sequence() {
        let mut m = Model::default();
        let mut def = Table::new(0, &[Type::Int2, Type::Text]);
        def.columns[0].identity = true;
        let create = Statement::CreateTable(def);
        assert!(valid(&m, &create));
        apply(&mut m, &create);
        let ins = Statement::Insert(Insert { table: 0, columns: vec![1], rows: vec![vec![Value::Text("a".into())], vec![Value::Null]] });
        for s in [Statement::Savepoint(0), ins.clone(), Statement::Rollback, ins] {
            assert!(valid(&m, &s), "{s}");
            apply(&mut m, &s);
        }
        assert_eq!(m.tables[0].rows, [vec![Value::Int(3), Value::Text("a".into())], vec![Value::Int(4), Value::Null]]);
        m.sequences.insert(0, i64::from(i16::MAX));
        let one = Statement::Insert(Insert { table: 0, columns: vec![1], rows: vec![vec![Value::Null], vec![Value::Null]] });
        assert!(!valid(&m, &one), "the second value is out of range");
    }
}
