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
//!   foreign key references is not dropped (PostgreSQL would need
//!   CASCADE); `SET LOGGED | UNLOGGED` keeps linked tables' persistence
//!   compatible; `ALTER CONSTRAINT` runs outside a transaction only (a
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
    fk::{self, Change, Write},
    model::{compare, identical, order_nulls, sizes_ok, unique_ok, Column, Index, Method, Model, Row, Snapshot, Table, Value, MAX_COLUMNS, MAX_INDEXES, MAX_TABLES},
    statements::{method_takes, outside_transaction, CmpOp, Insert, Lit, OrderTerm, Phys, Pred, FromTable, Merge, MergeAction, MergeKind, Returning, SetVal, Stat, Statement, Target, Update, When, Which},
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
            && r.iter().zip(&t.columns).all(|(v, c)| c.ty.admits(v) && !(*v == Value::Null && (c.not_null || c.identity || t.key.contains(&c.id))))
            && t.checks.iter().all(|p| eval(t, r, p) != Some(false))
            && t.partitioning.as_ref().is_none_or(|p| t.pos(p.column).is_some_and(|k| crate::model::routes(p, &r[k])))
    }) && t.uniques().iter().all(|(cols, nnd)| unique_ok(t, cols, *nnd, rows))
        && sizes_ok(t, rows)
        && gist_dates_ok(t, rows)
}

/// A known bug, excluded: `btree_gist`'s date penalty subtracts dates with
/// `date_mi`, which refuses infinite dates, so an insert into a GiST index
/// keyed on a date fails once the index has more than one page
/// (`../bugs/postgresql/pg-btree-gist-infinite-date`). The index keeps an
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
        Statement::DropTable(t) | Statement::DropColumn { table: t, .. } | Statement::DropForeignKey(t) | Statement::ValidateForeignKey(t) => linked(*t),
        Statement::AddForeignKey { table, fk, .. } => vec![*table, fk.parent],
        Statement::CreateTable(def) => def.fk.iter().map(|f| f.parent).collect(),
        Statement::CreateIndex { table, .. }
        | Statement::DropIndex { table, .. }
        | Statement::AddColumn { table, .. }
        | Statement::SetNotNull { table, .. }
        | Statement::SetDefault { table, .. }
        | Statement::Cluster { table, .. }
        | Statement::Reindex { table, .. }
        | Statement::Phys { table, .. } => vec![*table],
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
        // Not a UNIQUE index on the columns a foreign key references (it
        // may be the one the constraint depends on).
        Statement::DropIndex { table: k, index } => table(*k).is_some_and(|t| {
            t.index(*index).is_some_and(|i| {
                let mut cols: Vec<u32> = i.columns.iter().map(|c| c.column).collect();
                cols.sort();
                !i.unique || !m.tables.iter().any(|c| c.fk.as_ref().is_some_and(|f| f.parent == *k && {
                    let mut p = f.parent_columns.clone();
                    p.sort();
                    p == cols
                }))
            })
        }),
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
        // (`../bugs/candidates/pg-set-constraints-released-savepoint`); a
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
    t.widest.remove(&id);
    t.infinite.remove(&id);
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
    use crate::model::{Numeric, Type};

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
