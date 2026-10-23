// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Foreign keys: which definitions PostgreSQL takes ([`def_ok`]), and what
//! a write does once they act ([`write`]): the referential actions
//! (CASCADE, SET NULL, SET DEFAULT) applied through every table they reach,
//! and the errors PostgreSQL raises (`23503` from NO ACTION and the
//! referencing side's checks, `23001` from RESTRICT, or the `23502`,
//! `23514`, `23505` of an action's own writes).
//!
//! The model is exact under these restrictions, each checked where it is
//! documented:
//!
//! - A table has at most one foreign key, `t{t}_fk`, and the foreign keys
//!   form a forest (no cycle but a table referencing itself, [`def_ok`]).
//!   So the actions of one write reach every table along one path only,
//!   and the order in which PostgreSQL fires them (by trigger name, row by
//!   row, nested per action query) cannot change what they do: the model
//!   applies them breadth first ([`write`]).
//! - Every foreign key holds after every statement, deferred or not: a
//!   statement that would leave a violation behind is either refused by
//!   PostgreSQL at its end (all the violated constraints immediate, no
//!   other error possible: an expected error, `Statement::Expect`) or not
//!   generated (a deferred constraint violated, whose check would wait
//!   for COMMIT). `ADD CONSTRAINT .. NOT VALID` is valid only on rows that
//!   already satisfy it. So a check that fires finds a violation exactly
//!   when the state after the statement has one, or a RESTRICT finds a
//!   referencing row of a key that changed ([`write`]); the
//!   referencing-side shortcuts of `RI_FKey_fk_upd_check_required` (which
//!   skip checks on unchanged keys) cannot change an outcome.
//! - A referenced key that changes compares bytewise
//!   (`RI_FKey_pk_upd_check_required`: `1.0` to `1.00`, `0` to `-0` is a
//!   change, which an ON UPDATE action propagates), while referencing rows
//!   match a key by the type's equality.
//! - Writes set constants, so within one statement no key leaves a row
//!   and comes back in another (it would collide with the row that keeps
//!   it); NO ACTION's re-check for a replacement key and RESTRICT differ
//!   only in that bytewise case.
//! - Types of the two sides are the same ([`types_match`]).
//! - A self-referencing key's columns are apart from the key they
//!   reference, so its actions never change a referenced key again.
//! - No self-referencing key on a partitioned table (a bug candidate,
//!   see [`def_ok`]).
//! - No self-referencing UPDATE whose rows reference a key it changes: a
//!   row written earlier in the transaction is checked even with its
//!   reference unchanged, and whether before or after the cascade reaches
//!   it depends on the rows' physical order ([`write`]). Nor one, with an
//!   ON UPDATE action that writes rows, that makes a row reference a key
//!   it changes: that row's check and the key's action also fire in
//!   physical row order, the check first raising `23503`, the action
//!   first rewriting the row so that the check skips it ([`write`]).
//! - No SET NULL or CASCADE action whose UPDATE a BEFORE trigger turns
//!   back to the row's old reference: PostgreSQL skips the check of an
//!   unchanged key on a row older than the transaction and keeps the
//!   orphan, which the docs leave to the trigger's author ([`write`]).
//! - `SET NULL (..)` and `SET DEFAULT (..)` column lists only with MATCH
//!   SIMPLE (a partly NULL key would fail MATCH FULL).
//! - Inside a transaction, a table that may have deferred trigger events
//!   pending (a write to a table linked by a foreign key in deferred mode,
//!   until `SET CONSTRAINTS ALL IMMEDIATE` or the end of the transaction)
//!   takes no DDL: PostgreSQL refuses it (`55006`). Over-approximated
//!   ([`crate::model::TxnState::pending`]).

use std::{cmp::Ordering, collections::BTreeSet};

use crate::model::{compare, identical, Action, Deferral, ForeignKey, Index, Model, Row, Table, TrigAction, Trigger, Type, Value};

/// How many rounds of actions one write may take before the model gives
/// up (the statement is then not generated).
const MAX_ROUNDS: usize = 64;

/// The errors a write would raise, by kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Errors {
    /// A foreign key violated whose check is immediate (`23503`).
    pub fk: bool,
    /// A foreign key violated whose check is deferred.
    pub deferred: bool,
    /// A RESTRICT action that found a referencing row (`23001`).
    pub restrict: bool,
    /// `23502`.
    pub not_null: bool,
    /// `23514`: a CHECK constraint, or a row no partition takes.
    pub check: bool,
    /// `23505`.
    pub unique: bool,
    /// Anything the model does not predict an error for (sizes, a known
    /// bug's exclusion, an unresolved value, too many rounds).
    pub other: bool,
}

/// An error a statement is expected to raise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    ForeignKey,
    Restrict,
    NotNull,
    Check,
    Unique,
    /// The errors of `ALTER COLUMN .. TYPE` (`crate::semantics::convert`):
    /// `22003`, `22P02`, `42804` (no assignment cast and no USING), `22008`
    /// (a date past the last timestamp), `0A000` (NaN or infinity to an
    /// integer). Never a write's.
    OutOfRange,
    InvalidText,
    Mismatch,
    DatetimeOverflow,
    Unsupported,
}

impl Code {
    pub const ALL: [Code; 5] = [Code::ForeignKey, Code::Restrict, Code::NotNull, Code::Check, Code::Unique];

    /// The PL/pgSQL condition name.
    pub fn condition(self) -> &'static str {
        match self {
            Code::ForeignKey => "foreign_key_violation",
            Code::Restrict => "restrict_violation",
            Code::NotNull => "not_null_violation",
            Code::Check => "check_violation",
            Code::Unique => "unique_violation",
            Code::OutOfRange => "numeric_value_out_of_range",
            Code::InvalidText => "invalid_text_representation",
            Code::Mismatch => "datatype_mismatch",
            Code::DatetimeOverflow => "datetime_field_overflow",
            Code::Unsupported => "feature_not_supported",
        }
    }
}

impl Errors {
    pub fn none(&self) -> bool {
        *self == Errors::default()
    }

    /// The one error PostgreSQL must raise: exactly one kind, none the
    /// model cannot place.
    pub fn code(&self) -> Option<Code> {
        if self.other || self.deferred {
            return None;
        }
        let kinds = [(self.fk, Code::ForeignKey), (self.restrict, Code::Restrict), (self.not_null, Code::NotNull), (self.check, Code::Check), (self.unique, Code::Unique)];
        let on: Vec<Code> = kinds.iter().filter(|(b, _)| *b).map(|(_, c)| *c).collect();
        (on.len() == 1).then(|| on[0])
    }
}

/// Whether two column types may be the two sides of a foreign key: the
/// same type (integers of different widths make an ON UPDATE CASCADE cast
/// the new key to the referencing type, which fails out of range even when
/// no row references it).
pub fn types_match(a: Type, b: Type) -> bool {
    a == b
}

fn distinct(xs: &[u32]) -> bool {
    xs.iter().enumerate().all(|(k, x)| !xs[..k].contains(x))
}

fn find<'a>(tables: &'a [Table], id: u32) -> Option<&'a Table> {
    tables.iter().find(|t| t.id == id)
}

/// Whether `child` (with `fk`) may hold this foreign key among `tables`
/// (which may or may not hold `child` itself): the parent exists, the
/// referenced columns are a unique constraint of it, the types match,
/// persistence allows it, the graph stays a forest.
pub fn def_ok(tables: &[Table], child: &Table, fk: &ForeignKey) -> bool {
    let parent = if fk.parent == child.id { child } else {
        match find(tables, fk.parent) {
            Some(p) => p,
            None => return false,
        }
    };
    let n = fk.columns.len();
    (1..=3).contains(&n)
        && fk.parent_columns.len() == n
        && distinct(&fk.columns)
        && parent.uniques().iter().any(|(cols, _)| *cols == fk.parent_columns)
        && fk.columns.iter().zip(&fk.parent_columns).all(|(c, p)| match (child.column(*c), parent.column(*p)) {
            (Some(c), Some(p)) => !c.identity && types_match(c.ty, p.ty),
            _ => false,
        })
        && (fk.parent != child.id || fk.columns.iter().all(|c| !fk.parent_columns.contains(c)))
        // Not a self-reference on a partitioned table: an UPDATE that moves
        // a row to another partition checks the moved row before the
        // cascade updates it (`../bugs/postgresql/pg-partition-selfref-cascade-move`).
        && (fk.parent != child.id || child.partitioning.is_none())
        && (fk.delete_set.is_empty() || matches!(fk.on_delete, Action::SetNull | Action::SetDefault) && !fk.full)
        && distinct(&fk.delete_set)
        && fk.delete_set.iter().all(|c| fk.columns.contains(c))
        // No BEFORE trigger of the parent sets a referenced column.
        && !trigger_sets(parent, &fk.parent_columns)
        // A permanent table references only permanent ones.
        && (child.unlogged || !parent.unlogged)
        // No cycle: the parent's own chain never comes back to the child.
        && (fk.parent == child.id || !reaches(tables, fk.parent, child.id))
}

/// Whether following foreign keys up from `from` reaches `to`.
fn reaches(tables: &[Table], from: u32, to: u32) -> bool {
    let mut at = from;
    for _ in 0..=tables.len() {
        if at == to {
            return true;
        }
        match find(tables, at).and_then(|t| t.fk.as_ref()) {
            Some(fk) if fk.parent != at => at = fk.parent,
            _ => return false,
        }
    }
    true
}

/// Positions of columns in a table.
fn positions(t: &Table, cols: &[u32]) -> Option<Vec<usize>> {
    cols.iter().map(|&c| t.pos(c)).collect()
}

fn key_cmp(a: &[Value], b: &[Value]) -> Ordering {
    a.iter().zip(b).map(|(x, y)| compare(x, y)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
}

/// Whether every row of `child` satisfies its foreign key against `parent`
/// (the same table for a self-reference): MATCH SIMPLE passes a key with
/// any NULL, MATCH FULL only an all-NULL one; any other key must equal a
/// parent row's.
pub fn satisfied(child: &Table, parent: &Table, fk: &ForeignKey) -> bool {
    keeps(child, parent, fk).is_some_and(|k| k.iter().all(|b| *b))
}

/// Which rows of `child` keep its foreign key against `parent` (see
/// [`satisfied`]); None when a column is missing.
pub fn keeps(child: &Table, parent: &Table, fk: &ForeignKey) -> Option<Vec<bool>> {
    let cp = positions(child, &fk.columns)?;
    let pp = positions(parent, &fk.parent_columns)?;
    let mut keys: Vec<Vec<Value>> = parent.rows.iter().map(|r| pp.iter().map(|&p| r[p].clone()).collect::<Vec<_>>()).filter(|k: &Vec<Value>| k.iter().all(|v| *v != Value::Null)).collect();
    keys.sort_by(|a, b| key_cmp(a, b));
    Some(
        child
            .rows
            .iter()
            .map(|r| {
                let k: Vec<Value> = cp.iter().map(|&p| r[p].clone()).collect();
                let nulls = k.iter().filter(|v| **v == Value::Null).count();
                if nulls == k.len() {
                    return true;
                }
                if nulls > 0 {
                    return !fk.full;
                }
                keys.binary_search_by(|x| key_cmp(x, &k)).is_ok()
            })
            .collect(),
    )
}

/// Whether table `t`'s foreign key (if any) holds among `tables`.
pub fn holds(tables: &[Table], t: &Table) -> bool {
    match &t.fk {
        None => true,
        Some(fk) => {
            let parent = if fk.parent == t.id { Some(t) } else { find(tables, fk.parent) };
            parent.is_some_and(|p| satisfied(t, p, fk))
        },
    }
}

/// Whether the check of `t{child}`'s foreign key is deferred now: a
/// deferrable constraint inside a transaction, as `SET CONSTRAINTS` left
/// it (by name, else ALL), else as declared; outside a transaction, as
/// declared (`INITIALLY DEFERRED` waits for the statement's own commit,
/// which is no different for the model, but a PL/pgSQL block cannot catch
/// that error).
pub fn deferred_now(m: &Model, child: u32, fk: &ForeignKey) -> bool {
    match fk.deferral {
        Deferral::No => false,
        d if m.savepoints.is_empty() => d == Deferral::Deferred,
        d => m.txn.named.get(&child).copied().or(m.txn.all).unwrap_or(d == Deferral::Deferred),
    }
}

/// The tables whose foreign keys reference `t` (other than itself).
pub fn children(tables: &[Table], t: u32) -> Vec<u32> {
    tables.iter().filter(|c| c.id != t && c.fk.as_ref().is_some_and(|f| f.parent == t)).map(|c| c.id).collect()
}

/// `t` and every table whose foreign keys reach it, down the forest
/// (what `TRUNCATE t CASCADE` empties).
pub fn truncated(tables: &[Table], t: u32) -> Vec<u32> {
    let mut out = vec![t];
    let mut k = 0;
    while k < out.len() {
        for c in children(tables, out[k]) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        k += 1;
    }
    out
}

/// Whether a column of `t` belongs to a key some foreign key references.
pub fn referenced_column(tables: &[Table], t: u32, column: u32) -> bool {
    tables.iter().any(|c| c.fk.as_ref().is_some_and(|f| f.parent == t && f.parent_columns.contains(&column)))
}

/// Whether index `i` of table `t` may be the one a foreign key referencing
/// `t` depends on. PostgreSQL picks the first unique index (by OID) with
/// neither a predicate nor expressions whose key columns are the
/// referenced ones in any order (`transformFkeyCheckAttrs`), and the
/// constraint depends on it: dropping it, or a column it reads (a key or
/// an `INCLUDE` column), needs CASCADE (`2BP01`), even when the column is
/// the referencing one of a self-reference. The model does not track which
/// index that was, so it takes every one that qualifies.
pub fn may_back(tables: &[Table], t: u32, i: &Index) -> bool {
    let mut cols: Vec<u32> = i.columns.iter().map(|c| c.column).collect();
    cols.sort();
    i.unique
        && i.filter.is_none()
        && i.expr.is_none()
        && tables.iter().any(|c| {
            c.fk.as_ref().is_some_and(|f| {
                f.parent == t && {
                    let mut p = f.parent_columns.clone();
                    p.sort();
                    p == cols
                }
            })
        })
}

/// A row change on one table, for the actions it sets off.
enum Event {
    Deleted(Row),
    Updated(Row, Row),
}

/// What a write leaves: every table after it and its actions, the rows the
/// statement itself wrote (the command tag's count), the tables changed,
/// and the errors PostgreSQL raises.
pub struct Write {
    pub tables: Vec<Table>,
    pub written: usize,
    pub touched: BTreeSet<u32>,
    pub errors: Errors,
    /// The rows the statement itself wrote, old and new (what RETURNING
    /// reads), in order.
    pub own: Vec<(Option<Row>, Option<Row>)>,
}

/// The rows of `t` after the statement's own change: kind of the write.
pub enum Change {
    /// Rows appended.
    Insert(Vec<Row>),
    /// The new rows of the table (same length), which positions the
    /// statement selected, and the columns it sets.
    Update(Vec<Row>, Vec<bool>, Vec<u32>),
    /// Which positions are deleted.
    Delete(Vec<bool>),
    /// `INSERT .. ON CONFLICT`: the proposed rows in order, the arbiters
    /// (positions of a unique constraint's columns, NULLS NOT DISTINCT),
    /// and for DO UPDATE the sets (a position and a constant or the
    /// position of the proposed row's value) and the condition on the
    /// existing row. See [`upsert`].
    Upsert(Vec<Row>, Vec<(Vec<usize>, bool)>, Option<(Vec<(usize, UpsertVal)>, Vec<u32>, Option<crate::statements::Pred>)>),
    /// A MERGE's outcome on a table without triggers: each row's fate, in
    /// order, and the rows inserted.
    Merge(Vec<Fate>, Vec<Row>),
}

/// What a MERGE does to a target row: nothing, an update to a new row
/// (setting these columns), a delete.
#[derive(Clone, Debug, PartialEq)]
pub enum Fate {
    Keep,
    Update(Row, Vec<u32>),
    Delete,
}

/// A value DO UPDATE sets: a constant, or the proposed row's value at a
/// position.
#[derive(Clone, Debug, PartialEq)]
pub enum UpsertVal {
    Const(Value),
    Excluded(usize),
}

/// Whether rows `a` and `b` conflict on a unique constraint over positions
/// `pos`: equal there (by the types' equality), with no NULL unless NULLS
/// NOT DISTINCT (then NULL meets NULL).
pub fn conflict(a: &Row, b: &Row, pos: &[usize], nnd: bool) -> bool {
    pos.iter().all(|&p| match (&a[p], &b[p]) {
        (Value::Null, Value::Null) => nnd,
        (Value::Null, _) | (_, Value::Null) => false,
        (x, y) => compare(x, y).is_eq(),
    })
}

/// The table an AFTER trigger writes to.
pub fn audit_target(tr: &Trigger) -> Option<u32> {
    match &tr.action {
        TrigAction::Audit { target, .. } => Some(*target),
        _ => None,
    }
}

/// A table's triggers in the order PostgreSQL fires them: by name.
pub fn sorted(t: &Table) -> Vec<&Trigger> {
    let mut v: Vec<&Trigger> = t.triggers.iter().collect();
    v.sort_by_key(|tr| tr.name(t.id));
    v
}

/// A row on its way into `t` (kind 0 insert, 1 update, setting
/// `setcols`) through the table's enabled BEFORE ROW triggers, in name
/// order, each WHEN seeing the row as the triggers before it left it; None
/// when one skips it.
pub fn before_row(t: &Table, kind: usize, mut row: Row, setcols: &[u32]) -> Option<Row> {
    for tr in sorted(t) {
        if !tr.before || !tr.enabled || !tr.events[kind] {
            continue;
        }
        if kind == 1 && !tr.update_of.is_empty() && !tr.update_of.iter().any(|c| setcols.contains(c)) {
            continue;
        }
        if tr.when.as_ref().is_some_and(|p| crate::semantics::eval(t, &row, p) != Some(true)) {
            continue;
        }
        match &tr.action {
            TrigAction::Set(c, v) => row[t.pos(*c)?] = v.clone(),
            TrigAction::Skip => return None,
            TrigAction::Audit { .. } => {},
        }
    }
    Some(row)
}

/// Whether table `t` (among `tables`) may have trigger `tr`: a BEFORE
/// trigger is a row trigger on INSERT and UPDATE that sets a column (not
/// the partition key: the row would move, which PostgreSQL refuses) or,
/// on INSERT only, skips the row; an AFTER trigger writes to another table
/// that no foreign key links, that has no trigger and no identity column
/// (its rows are a multiset whatever the firing order), columns of the
/// same types; a statement trigger has one event and its transition
/// table, no column list, no WHEN; a row trigger's WHEN reads NEW (OLD
/// for a DELETE trigger, which has no other event). The table is not
/// itself a trigger's target, nor partitioned (an UPDATE that moves a row
/// fires INSERT and DELETE row triggers on the partitions, which the
/// model does not follow); a column a trigger sets is not one a foreign
/// key references ([`trigger_sets`]).
/// Whether a column of `t` is of a domain type.
pub fn has_domain(t: &Table) -> bool {
    t.columns.iter().any(|c| c.dom.is_some())
}

pub fn trigger_ok(tables: &[Table], t: &Table, tr: &Trigger) -> bool {
    let n_events = tr.events.iter().filter(|e| **e).count();
    let cols_ok = tr.update_of.iter().all(|c| t.column(*c).is_some()) && distinct(&tr.update_of) && (tr.update_of.is_empty() || tr.events[1]);
    let when_ok = tr.when.as_ref().is_none_or(|p| tr.row && crate::semantics::pred_valid(t, p));
    let base = n_events >= 1 && (!tr.events[2] || n_events == 1) && cols_ok && when_ok && !is_target(tables, t.id) && t.partitioning.is_none() && t.triggers.iter().all(|x| x.id != tr.id || x == tr)
        // Not on a table with a domain column, nor writing to one (see
        // `crate::semantics::set_domain`).
        && !has_domain(t)
        && audit_target(tr).is_none_or(|a| tables.iter().find(|x| x.id == a).is_none_or(|x| !has_domain(x)));
    base && match &tr.action {
        TrigAction::Set(c, v) => {
            tr.before
                && tr.row
                && !tr.events[2]
                && t.column(*c).is_some_and(|col| !col.identity && col.ty.admits(v) && *v != Value::Null)
                && t.partitioning.as_ref().is_none_or(|p| p.column != *c)
                && !referenced_column(tables, t.id, *c)
        },
        TrigAction::Skip => tr.before && tr.row && tr.events == [true, false, false],
        TrigAction::Audit { target, cols } => {
            let Some(g) = find(tables, *target) else { return false };
            let dst: Vec<u32> = cols.iter().map(|(_, d)| *d).collect();
            !tr.before
                && (tr.row || n_events == 1 && tr.update_of.is_empty() && tr.when.is_none())
                && *target != t.id
                && g.fk.is_none()
                && g.triggers.is_empty()
                && g.identity().is_none()
                && !tables.iter().any(|x| x.fk.as_ref().is_some_and(|f| f.parent == *target))
                && !cols.is_empty()
                && distinct(&dst)
                && cols.iter().all(|(s, d)| matches!((t.column(*s), g.column(*d)), (Some(a), Some(b)) if a.ty == b.ty))
        },
    }
}

/// Whether a BEFORE trigger of `t` sets one of `cols` (then no foreign
/// key may reference them: an action's UPDATE would change the referenced
/// key again, and the order of the actions would matter).
pub fn trigger_sets(t: &Table, cols: &[u32]) -> bool {
    t.triggers.iter().any(|tr| matches!(&tr.action, TrigAction::Set(c, _) if cols.contains(c)))
}

/// Whether some table's trigger writes to `t`.
pub fn is_target(tables: &[Table], t: u32) -> bool {
    tables.iter().any(|x| x.triggers.iter().any(|tr| audit_target(tr) == Some(t)))
}

/// Whether a column of `t` is one a trigger reads or writes (PostgreSQL
/// refuses to drop a column a WHEN or UPDATE OF names; a function that
/// names a column fails once it is gone).
pub fn trigger_column(tables: &[Table], t: u32, column: u32) -> bool {
    let own = tables.iter().find(|x| x.id == t).is_some_and(|x| {
        x.triggers.iter().any(|tr| {
            tr.update_of.contains(&column)
                || tr.when.as_ref().is_some_and(|p| p.reads(column))
                || match &tr.action {
                    TrigAction::Set(c, _) => *c == column,
                    TrigAction::Skip => false,
                    TrigAction::Audit { cols, .. } => cols.iter().any(|(s, _)| *s == column),
                }
        })
    });
    own || tables.iter().any(|x| x.triggers.iter().any(|tr| matches!(&tr.action, TrigAction::Audit { target, cols } if *target == t && cols.iter().any(|(_, d)| *d == column))))
}

/// The errors rows of `t` would raise as its constraints check them.
pub fn row_errors(t: &Table, rows: &[Row], e: &mut Errors) {
    for r in rows {
        if r.len() != t.columns.len() || !r.iter().zip(&t.columns).all(|(v, c)| c.ty.admits(v)) {
            e.other = true;
            return;
        }
        if r.iter().zip(&t.columns).any(|(v, c)| *v == Value::Null && (c.refuses_null() || t.key.contains(&c.id))) {
            e.not_null = true;
        }
        if t.all_checks().any(|p| crate::semantics::eval(t, r, p) == Some(false)) {
            e.check = true;
        }
        if let Some(p) = &t.partitioning {
            if !t.pos(p.column).is_some_and(|k| crate::model::routes(p, &r[k])) {
                e.check = true;
            }
        }
    }
    if !t.uniques().iter().all(|(cols, nnd)| crate::model::unique_ok(t, cols, *nnd, rows)) {
        e.unique = true;
    }
    if !crate::model::sizes_ok(t, rows) || !crate::semantics::gist_dates_ok(t, rows) {
        e.other = true;
    }
}

/// The write of `change` to table `t{top}` of `m`, with every action it
/// sets off, and what PostgreSQL makes of it.
pub fn write(m: &Model, top: u32, change: Change) -> Write {
    // Only the tables the write can reach (the top table and every table
    // whose foreign keys lead to it), and the top table's parent.
    let mut reach = truncated(&m.tables, top);
    if let Some(f) = m.table(top).and_then(|t| t.fk.as_ref()) {
        reach.push(f.parent);
    }
    // The tables their triggers write to.
    let targets: Vec<u32> = m.tables.iter().filter(|t| reach.contains(&t.id)).flat_map(|t| t.triggers.iter().filter_map(audit_target)).collect();
    reach.extend(targets);
    let mut tables: Vec<Table> = m.tables.iter().filter(|t| reach.contains(&t.id)).cloned().collect();
    // Every row change, for the AFTER triggers: (table, kind 0 insert, 1
    // update, 2 delete, old row, new row, the columns the UPDATE set).
    let mut log: Vec<(u32, usize, Option<Row>, Option<Row>, Vec<u32>)> = Vec::new();
    let mut touched = BTreeSet::from([top]);
    let mut errors = Errors::default();
    let ti = tables.iter().position(|t| t.id == top).expect("the written table");
    let mut own: Vec<(Option<Row>, Option<Row>)> = Vec::new();
    let (written, first): (usize, Vec<Event>) = {
        let t = &mut tables[ti];
        match change {
            Change::Upsert(rows, arbiters, update) => upsert(t, rows, &arbiters, update.as_ref(), &mut log, &mut own, &mut errors),
            Change::Merge(fates, inserted) => {
                let mut n = 0;
                let mut evs = Vec::new();
                let mut rows = Vec::with_capacity(t.rows.len() + inserted.len());
                for (o, f) in std::mem::take(&mut t.rows).into_iter().zip(fates) {
                    match f {
                        Fate::Keep => rows.push(o),
                        Fate::Update(nw, setcols) => {
                            n += 1;
                            own.push((Some(o.clone()), Some(nw.clone())));
                            log.push((top, 1, Some(o.clone()), Some(nw.clone()), setcols));
                            evs.push(Event::Updated(o, nw.clone()));
                            rows.push(nw);
                        },
                        Fate::Delete => {
                            n += 1;
                            own.push((Some(o.clone()), None));
                            log.push((top, 2, Some(o.clone()), None, Vec::new()));
                            evs.push(Event::Deleted(o));
                        },
                    }
                }
                for r in inserted {
                    n += 1;
                    own.push((None, Some(r.clone())));
                    log.push((top, 0, None, Some(r.clone()), Vec::new()));
                    rows.push(r);
                }
                t.rows = rows;
                (n, evs)
            },
            Change::Insert(rows) => {
                let kept: Vec<Row> = rows.into_iter().filter_map(|r| before_row(t, 0, r, &[])).collect();
                for r in &kept {
                    log.push((top, 0, None, Some(r.clone()), Vec::new()));
                    own.push((None, Some(r.clone())));
                }
                let n = kept.len();
                t.rows.extend(kept);
                (n, Vec::new())
            },
            Change::Update(new, selected, setcols) => {
                let mut n = 0;
                let mut evs = Vec::new();
                let mut rows = Vec::with_capacity(new.len());
                for ((o, nw), sel) in t.rows.iter().zip(new).zip(&selected) {
                    if !*sel {
                        rows.push(o.clone());
                        continue;
                    }
                    match before_row(t, 1, nw, &setcols) {
                        Some(nw) => {
                            n += 1;
                            own.push((Some(o.clone()), Some(nw.clone())));
                            log.push((top, 1, Some(o.clone()), Some(nw.clone()), setcols.clone()));
                            evs.push(Event::Updated(o.clone(), nw.clone()));
                            rows.push(nw);
                        },
                        None => rows.push(o.clone()),
                    }
                }
                t.rows = rows;
                (n, evs)
            },
            Change::Delete(gone) => {
                let mut evs = Vec::new();
                let mut keep = Vec::new();
                for (r, g) in std::mem::take(&mut t.rows).into_iter().zip(&gone) {
                    if *g {
                        own.push((Some(r.clone()), None));
                        log.push((top, 2, Some(r.clone()), None, Vec::new()));
                        evs.push(Event::Deleted(r));
                    } else {
                        keep.push(r);
                    }
                }
                t.rows = keep;
                (evs.len(), evs)
            },
        }
    };
    // A self-referencing UPDATE whose rows reference a key it changes:
    // a row of the statement written in this transaction is checked even
    // with its reference unchanged, before or after the cascade reaches
    // it, by the order of the rows (not modelled: excluded).
    // Likewise a row the UPDATE makes reference a key it changes, when
    // the ON UPDATE action writes rows: its check (`RI_FKey_check_upd`)
    // and the key's action fire in the rows' physical order. Check first,
    // the key is gone (`23503`); action first, it finds the row's new
    // version and sets or cascades it, and the check then skips the
    // superseded version (no error). The model, which applies actions to
    // the state after the statement, would predict the second.
    if let Some(f) = tables[ti].fk.clone().filter(|f| f.parent == top) {
        let t = &tables[ti];
        if let (Some(pp), Some(cp)) = (positions(t, &f.parent_columns), positions(t, &f.columns)) {
            let mut changed: Vec<Vec<Value>> = Vec::new();
            let mut refs: Vec<Vec<Value>> = Vec::new();
            for e in &first {
                if let Event::Updated(o, n) = e {
                    let ko: Vec<Value> = pp.iter().map(|&k| o[k].clone()).collect();
                    if !pp.iter().all(|&k| identical(&o[k], &n[k]) && n[k] != Value::Null) {
                        changed.push(ko);
                    }
                    let kr: Vec<Value> = cp.iter().map(|&k| o[k].clone()).collect();
                    if kr.iter().all(|v| *v != Value::Null) {
                        refs.push(kr);
                    }
                    let kn: Vec<Value> = cp.iter().map(|&k| n[k].clone()).collect();
                    if matches!(f.on_update, Action::Cascade | Action::SetNull | Action::SetDefault) && kn.iter().all(|v| *v != Value::Null) {
                        refs.push(kn);
                    }
                }
            }
            if changed.iter().any(|c| c.iter().all(|v| *v != Value::Null) && refs.iter().any(|r| key_cmp(c, r).is_eq())) {
                errors.other = true;
            }
        }
    }
    // NOT NULL, CHECK, the partitions and unique constraints check each
    // row as it is written, before any action can change it again: the
    // statement's own rows here, each action's below.
    row_errors(&tables[ti], &tables[ti].rows, &mut errors);
    let mut versions: Vec<(u32, Row)> = Vec::new();
    // RESTRICT: (child table, the old keys it must not reference).
    let mut restrict: Vec<(u32, Vec<Vec<Value>>)> = Vec::new();
    let mut queue: Vec<(u32, Vec<Event>)> = vec![(top, first)];
    let mut rounds = 0;
    while let Some((p, events)) = queue.pop() {
        if events.is_empty() {
            continue;
        }
        rounds += 1;
        if rounds > MAX_ROUNDS {
            errors.other = true;
            break;
        }
        let kids: Vec<u32> = tables.iter().filter(|c| c.fk.as_ref().is_some_and(|f| f.parent == p)).map(|c| c.id).collect();
        for c in kids {
            let parent = find(&tables, p).unwrap().clone();
            let ci = tables.iter().position(|t| t.id == c).unwrap();
            let fk = tables[ci].fk.clone().unwrap();
            let (Some(pp), Some(cp)) = (positions(&parent, &fk.parent_columns), positions(&tables[ci], &fk.columns)) else {
                errors.other = true;
                continue;
            };
            let key = |r: &Row| -> Vec<Value> { pp.iter().map(|&k| r[k].clone()).collect() };
            // The events that set off an action: an old key without NULL,
            // and for an update, a new key bytewise different.
            let mut keyed: Vec<(Vec<Value>, Option<Vec<Value>>)> = Vec::new();
            for e in &events {
                let (old, new) = match e {
                    Event::Deleted(o) => (key(o), None),
                    Event::Updated(o, n) => (key(o), Some(key(n))),
                };
                if old.iter().any(|v| *v == Value::Null) {
                    continue;
                }
                if let Some(n) = &new {
                    if old.iter().zip(n).all(|(a, b)| identical(a, b) && *b != Value::Null) {
                        continue;
                    }
                }
                keyed.push((old, new));
            }
            if keyed.is_empty() {
                continue;
            }
            keyed.sort_by(|a, b| key_cmp(&a.0, &b.0));
            let child = &mut tables[ci];
            let shape = Table { rows: Vec::new(), ..child.clone() };
            let mut next: Vec<Event> = Vec::new();
            let mut restricted: Vec<Vec<Value>> = Vec::new();
            let mut keep: Vec<Row> = Vec::with_capacity(child.rows.len());
            let defaults: Vec<Value> = child.columns.iter().map(|col| col.default.clone().unwrap_or(Value::Null)).collect();
            for r in std::mem::take(&mut child.rows) {
                let k: Vec<Value> = cp.iter().map(|&x| r[x].clone()).collect();
                let hit = if k.iter().any(|v| *v == Value::Null) { None } else { keyed.binary_search_by(|x| key_cmp(&x.0, &k)).ok() };
                let Some(h) = hit else {
                    keep.push(r);
                    continue;
                };
                let (old, new) = &keyed[h];
                let action = if new.is_some() { fk.on_update } else { fk.on_delete };
                let set_ids: Vec<u32> = if new.is_none() && !fk.delete_set.is_empty() { fk.delete_set.clone() } else { fk.columns.clone() };
                let set_cols: Vec<usize> = set_ids.iter().map(|c| shape.pos(*c).unwrap()).collect();
                match action {
                    Action::NoAction => keep.push(r),
                    Action::Restrict => {
                        restricted.push(old.clone());
                        keep.push(r);
                    },
                    Action::Cascade if new.is_none() => {
                        log.push((c, 2, Some(r.clone()), None, Vec::new()));
                        next.push(Event::Deleted(r));
                    },
                    Action::Cascade | Action::SetNull | Action::SetDefault => {
                        let mut n = r.clone();
                        if action == Action::Cascade {
                            for (x, v) in cp.iter().zip(new.as_ref().unwrap()) {
                                n[*x] = v.clone();
                            }
                        } else {
                            for &x in &set_cols {
                                n[x] = if action == Action::SetNull { Value::Null } else { defaults[x].clone() };
                            }
                        }
                        // The action's UPDATE goes through the child's
                        // BEFORE triggers (none of which skips an update).
                        let set: Vec<Value> = cp.iter().map(|&x| n[x].clone()).collect();
                        match before_row(&shape, 1, n, &set_ids) {
                            Some(n) => {
                                // A BEFORE trigger that sets the row's
                                // reference back to its old key (equal by
                                // the type's equality, no NULL): the check
                                // `RI_FKey_fk_upd_check_required` skips it
                                // unless the row was written in this
                                // transaction, so PostgreSQL either raises
                                // `23503` or leaves the row referencing a
                                // key that is gone (documented: "it is the
                                // trigger programmer's responsibility",
                                // trigger.sgml). The model tracks neither
                                // which rows this transaction wrote nor
                                // violated states: excluded. SET DEFAULT
                                // re-checks the old key (`ri_restrict`),
                                // which the model's final check matches.
                                let got: Vec<Value> = cp.iter().map(|&x| n[x].clone()).collect();
                                if action != Action::SetDefault
                                    && !set.iter().zip(&got).all(|(a, b)| identical(a, b))
                                    && got.iter().all(|v| *v != Value::Null)
                                    && key_cmp(&got, &k).is_eq()
                                {
                                    errors.other = true;
                                }
                                log.push((c, 1, Some(r.clone()), Some(n.clone()), set_ids.clone()));
                                versions.push((c, n.clone()));
                                next.push(Event::Updated(r, n.clone()));
                                keep.push(n);
                            },
                            None => keep.push(r),
                        }
                    },
                }
            }
            child.rows = keep;
            if !restricted.is_empty() {
                restrict.push((c, restricted));
            }
            if !next.is_empty() {
                touched.insert(c);
                queue.push((c, next));
            }
        }
    }
    for (c, r) in &versions {
        let t = find(&tables, *c).unwrap();
        row_errors(&Table { rows: Vec::new(), ..t.clone() }, std::slice::from_ref(r), &mut errors);
    }
    // AFTER triggers: each row change (row triggers) or each row of the
    // transition table (statement triggers) adds a row to the target.
    for (tid, kind, old, new, setcols) in &log {
        let t = m.table(*tid).unwrap();
        for tr in sorted(t) {
            if tr.before || !tr.enabled || !tr.events[*kind] {
                continue;
            }
            let TrigAction::Audit { target, cols } = &tr.action else { continue };
            let row = if *kind == 2 { old.as_ref().unwrap() } else { new.as_ref().unwrap() };
            if tr.row {
                if *kind == 1 && !tr.update_of.is_empty() && !tr.update_of.iter().any(|c| setcols.contains(c)) {
                    continue;
                }
                if tr.when.as_ref().is_some_and(|p| crate::semantics::eval(t, row, p) != Some(true)) {
                    continue;
                }
            }
            let Some(gi) = tables.iter().position(|x| x.id == *target) else {
                errors.other = true;
                continue;
            };
            let g = &mut tables[gi];
            let mut out: Row = g.columns.iter().map(|col| col.default.clone().unwrap_or(Value::Null)).collect();
            for (src, dst) in cols {
                match (t.pos(*src), g.pos(*dst)) {
                    (Some(a), Some(b)) => out[b] = row[a].clone(),
                    _ => errors.other = true,
                }
            }
            g.rows.push(out);
            touched.insert(*target);
        }
    }
    // RESTRICT: a referencing row of a key that changed, after the actions.
    for (c, olds) in &restrict {
        let child = find(&tables, *c).unwrap();
        let fk = child.fk.as_ref().unwrap();
        let cp = positions(child, &fk.columns).unwrap();
        if child.rows.iter().any(|r| {
            let k: Vec<Value> = cp.iter().map(|&x| r[x].clone()).collect();
            k.iter().all(|v| *v != Value::Null) && olds.iter().any(|o| key_cmp(o, &k).is_eq())
        }) {
            errors.restrict = true;
        }
    }
    // Every foreign key whose either side changed must hold after.
    for t in &tables {
        let Some(fk) = &t.fk else { continue };
        if !(touched.contains(&t.id) || touched.contains(&fk.parent)) {
            continue;
        }
        if !holds(&tables, t) {
            if deferred_now(m, t.id, fk) {
                errors.deferred = true;
            } else {
                errors.fk = true;
            }
        }
    }
    for t in &tables {
        if touched.contains(&t.id) {
            row_errors(t, &t.rows, &mut errors);
        }
    }
    Write { tables, written, touched, errors, own }
}

type Log = Vec<(u32, usize, Option<Row>, Option<Row>, Vec<u32>)>;

/// `INSERT .. ON CONFLICT` on `t`, row by row as PostgreSQL runs it: each
/// proposed row goes through the BEFORE INSERT triggers, then NOT NULL and
/// CHECK (raised even for a row that conflicts); a row that conflicts on an
/// arbiter with a row of the table (one inserted earlier by the statement
/// included) is skipped (DO NOTHING) or updates that row when the condition
/// holds on it (DO UPDATE: through the BEFORE UPDATE triggers); else it is
/// inserted. Updating a row this statement inserted or updated raises
/// `21000` (not modelled: [`Errors::other`]). Unique constraints are
/// checked after each row, as PostgreSQL checks each row as it writes it.
fn upsert(
    t: &mut Table,
    rows: Vec<Row>,
    arbiters: &[(Vec<usize>, bool)],
    update: Option<&(Vec<(usize, UpsertVal)>, Vec<u32>, Option<crate::statements::Pred>)>,
    log: &mut Log,
    own: &mut Vec<(Option<Row>, Option<Row>)>,
    errors: &mut Errors,
) -> (usize, Vec<Event>) {
    let top = t.id;
    let mut n = 0;
    let mut evs = Vec::new();
    let mut mine: Vec<usize> = Vec::new();
    let uniques: Vec<(Vec<u32>, bool)> = t.uniques();
    let unique_now = |t: &Table| uniques.iter().all(|(cols, nnd)| crate::model::unique_ok(t, cols, *nnd, &t.rows));
    for r in rows {
        let Some(r) = before_row(t, 0, r, &[]) else { continue };
        row_errors(&Table { rows: Vec::new(), ..t.clone() }, std::slice::from_ref(&r), errors);
        let hit = t.rows.iter().position(|x| arbiters.iter().any(|(pos, nnd)| conflict(x, &r, pos, *nnd)));
        match (hit, update) {
            (None, _) => {
                log.push((top, 0, None, Some(r.clone()), Vec::new()));
                own.push((None, Some(r.clone())));
                mine.push(t.rows.len());
                t.rows.push(r);
                n += 1;
            },
            (Some(_), None) => continue,
            (Some(h), Some((sets, setcols, filter))) => {
                if mine.contains(&h) {
                    errors.other = true;
                    continue;
                }
                let old = t.rows[h].clone();
                if filter.as_ref().is_some_and(|p| crate::semantics::eval(t, &old, p) != Some(true)) {
                    continue;
                }
                let mut nw = old.clone();
                for (p, v) in sets {
                    nw[*p] = match v {
                        UpsertVal::Const(v) => v.clone(),
                        UpsertVal::Excluded(q) => r[*q].clone(),
                    };
                }
                let Some(nw) = before_row(t, 1, nw, setcols) else { continue };
                log.push((top, 1, Some(old.clone()), Some(nw.clone()), setcols.clone()));
                own.push((Some(old.clone()), Some(nw.clone())));
                evs.push(Event::Updated(old, nw.clone()));
                t.rows[h] = nw;
                mine.push(h);
                n += 1;
            },
        }
        if !unique_now(t) {
            errors.unique = true;
        }
    }
    (n, evs)
}

/// The tables a successful write inside a transaction may leave deferred
/// trigger events on: both sides of every foreign key in deferred mode
/// that the write touched.
pub fn pending_after(m: &Model, w: &Write) -> Vec<u32> {
    let mut out = Vec::new();
    if m.savepoints.is_empty() {
        return out;
    }
    for t in &w.tables {
        if let Some(fk) = &t.fk {
            if (w.touched.contains(&t.id) || w.touched.contains(&fk.parent)) && fk.deferral != Deferral::No && deferred_now(m, t.id, fk) {
                out.push(t.id);
                out.push(fk.parent);
            }
        }
    }
    out
}
