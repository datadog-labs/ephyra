// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! What a [`Statement`] means against a [`Model`]: whether it is valid
//! ([`valid`]), and its effect ([`apply`]): the model after it and, for a
//! SELECT, the rows SQLite must return ([`Expected`]).
//!
//! The model is exact only for the statements the generator produces, and
//! the language is restricted to keep it so. Each restriction is checked by
//! [`valid`] and documented where it is checked:
//!
//! - Comparisons convert their operands as SQLite does, by the column's
//!   affinity (a literal has none; datatype3.html section 4.2), and the
//!   model follows it ([`operands`]): under a numeric affinity (INTEGER,
//!   REAL, NUMERIC) text that reads as a number is that number
//!   ([`crate::model::numeric_text`], `sqlite3AtoF`'s grammar); under TEXT,
//!   when either side is text, a number on the other is its text
//!   ([`crate::model::real_text`]); under none (BLOB affinity) nothing
//!   changes. A comparison is valid ([`comparable`]) unless the model
//!   cannot tell the conversion: text it is unsure reads as a number (more
//!   than 15 significant digits, a large exponent), a real under TEXT that
//!   it does not render (only multiples of 1/64 below 2^20 are). Affinities:
//!   an INTEGER PRIMARY KEY is INTEGER; in a STRICT table INT and INTEGER
//!   are INTEGER, REAL REAL, TEXT TEXT, BLOB and ANY none (ANY keeps `'1'`
//!   text: `'1' = 1` is false, checked in the tests); a column of a table
//!   that is not STRICT has its declared type's ([`crate::model::DECLS`],
//!   by SQLite's rules, `FLOATING POINT` INTEGER, `STRING` and `ANY`
//!   NUMERIC), none without one. Then values compare by
//!   [`crate::model::compare_coll`] under the column's collating sequence
//!   (the literal has none; datatype3.html section 7.1): BINARY, SQLite's
//!   order of values with text by its bytes in the database encoding;
//!   NOCASE and RTRIM, text by its UTF-8 bytes folded or trimmed (SQLite
//!   registers them for UTF-8 only); NULL on a NULL operand. Checked
//!   against SQLite for every declared type, values and literals of every
//!   class. ORDER BY sorts a column under its sequence, a WITHOUT ROWID key
//!   tells rows apart under its columns' (`'a'` and `'A'` clash under
//!   NOCASE), and a UNIQUE index under its own (an index column's
//!   `COLLATE`, else the column's).
//! - Written values are converted only in ways the model follows exactly
//!   ([`convert`]): STRICT INT takes integers and integral reals of at most
//!   2^53 in magnitude (stored as the integer), STRICT REAL takes reals and
//!   integers (stored as reals, `-0.0` as `0.0`), other STRICT types take
//!   their own class, ANY and untyped columns take anything unchanged, an
//!   INTEGER PRIMARY KEY takes an integer or NULL; a declared type stores
//!   by its affinity ([`crate::model::store`]: numbers as text under TEXT,
//!   text that reads as a number as it and integral reals as integers under
//!   INTEGER and NUMERIC, as reals under REAL), never text the model is
//!   unsure of under a numeric affinity, nor a real it does not render
//!   under TEXT. Text that SQLite would convert to a number is never
//!   written to STRICT INT or REAL.
//! - NOT NULL and STRICT are never violated: a NOT NULL column is listed in
//!   every INSERT with non-NULL values, and so is an INTEGER PRIMARY KEY
//!   declared NOT NULL (whether NULL becomes a new rowid before the NOT
//!   NULL check is a detail left out).
//! - UNIQUE indexes are never violated: an INSERT or UPDATE is valid
//!   only when the rows after it keep every UNIQUE index of the table
//!   ([`unique_rows`]: rows agree on its columns only where one is NULL,
//!   values compared as in the index), and `CREATE UNIQUE INDEX` only on
//!   rows that already do.
//! - Keys never change and never conflict: UPDATE sets no INTEGER PRIMARY
//!   KEY and no WITHOUT ROWID primary key column; an INSERT gives every
//!   primary key column of a WITHOUT ROWID table a non-NULL value distinct
//!   (by [`crate::model::compare`]) from every other row's, and every explicit
//!   rowid is new. Rowids are given as integers or NULL only (SQLite
//!   converts or rejects anything else).
//! - An INSERT that needs a new rowid when the largest rowid is
//!   `i64::MAX` is left out: SQLite then picks a random unused rowid.
//!   Otherwise a new rowid is the largest rowid in the table plus one (1 in
//!   an empty table), row after row, as checked in the tests.
//! - LIMIT without ORDER BY returns any rows of the result, so the check is
//!   that the rows returned are that many of the expected ones; with ORDER
//!   BY, rows tied on the ORDER BY columns come in any order, and a LIMIT
//!   that cuts a tie may return any of them ([`Expected::matches`]).
//! - VACUUM changes no row, but gives a rowid table without an INTEGER
//!   PRIMARY KEY and without indexes new rowids 1, 2, … in rowid order
//!   (SQLite copies such a table with `OP_NewRowid`; any other table keeps
//!   its rowids). `PRAGMA incremental_vacuum` changes nothing logical.
//!   Both checked in the tests.
//! - ANALYZE writes `sqlite_stat1` and `sqlite_stat4`, tables the model
//!   leaves out: only crafted statistics ([`Statement::Stats`]) write
//!   them, no statement reads them, and the queries' schemas
//!   (`crate::expr::Schema::of`) never list them. Statistics change how
//!   SQLite plans a query, never its result: crafted ones may be any
//!   numbers ([`stat_valid`] only asks that they name the table's own
//!   indexes).
//! - VACUUM runs only outside a transaction (inside one SQLite refuses
//!   it). `ROLLBACK TO` brings back the tables, views and triggers as they
//!   were at the savepoint (the model keeps a copy per open savepoint).
//! - Conflict resolution (`INSERT OR x`, `REPLACE INTO`, `UPDATE OR x`, a
//!   primary key's `ON CONFLICT x`), UPSERT and triggers with bodies are
//!   modelled row by row ([`write_rows`]): an INSERT's rows in statement
//!   order, each skipped (IGNORE), or with the rows it conflicts with
//!   deleted first (REPLACE: their foreign key actions run, and DELETE
//!   triggers with `PRAGMA recursive_triggers` on), or turned into the DO
//!   UPDATE of the row there; ABORT, FAIL and ROLLBACK only where nothing
//!   conflicts. Where the order SQLite checks constraints in would matter
//!   (a row conflicting with two rows under UPSERT, constraints resolved
//!   differently, an UPDATE OR x conflicting on several rows), the
//!   statement is left out. Triggers fire per row, each step a statement of
//!   its own; with several rows an UPDATE or DELETE fires at most one
//!   trigger that writes, with literals only, so that every row runs the
//!   same steps and the order SQLite takes the rows in does not matter.
//!   Triggers fire no cycle, write no table foreign keys reach from a table
//!   with triggers, and a RAISE(ABORT) never fires.

use std::cmp::Ordering;

use crate::{
    model::{column_coll, column_value, compare_coll, numeric_text, real_text, store, stored, unique_ok, Action, Cons, Db, Rule, Step, Term, ColType, Coll, Encoding, ForeignKey, Key, Model, Named, Numeric, Row, Table, Value, DECLS, MAX_COLUMNS},
    statements::{literal, CmpOp, Excluded, Insert, Journal, NewColumn, Pred, Reindex, Resolve, Stat, StatOf, Statement, Update},
};

pub use crate::model::{affinity, Affinity};

/// Text SQL can hold: no NUL, which ends a statement's text.
fn sql_safe(v: &Value) -> bool {
    !matches!(v, Value::Text(s) if s.contains('\0'))
}

/// Whether `v` may be compared with column `c` of `t` (see the module
/// documentation): a column of numeric affinity with anything but text the
/// model is unsure reads as a number or not ([`numeric_text`]), a TEXT
/// column with anything but a real it does not render ([`real_text`]), a
/// column without affinity with anything.
pub fn comparable(t: &Table, c: usize, v: &Value) -> bool {
    c < t.columns
        && sql_safe(v)
        && match (affinity(t, c), v) {
            (_, Value::Null | Value::Blob(_)) | (Affinity::None, _) => true,
            (Affinity::Text, Value::Text(_) | Value::Int(_)) => true,
            (Affinity::Text, Value::Real(x)) => real_text(*x).is_some(),
            (_, Value::Int(_) | Value::Real(_)) => true,
            (_, Value::Text(s)) => numeric_text(s) != Numeric::Unsure,
        }
}

/// The operands of a comparison of a column of affinity `aff` (holding
/// `x`) with a literal `v`, after the affinity SQLite applies
/// (datatype3.html section 4.2: the column's, as the literal has none):
/// under a numeric affinity a text that reads as a number becomes it (on
/// both sides; a stored text never does); under TEXT, when either side is
/// text, a number on the other becomes text; else neither changes.
fn operands(aff: Affinity, x: &Value, v: &Value) -> (Value, Value) {
    let num = |v: &Value| match v {
        Value::Text(s) => match numeric_text(s) {
            Numeric::Int(i) => Value::Int(i),
            Numeric::Real(r) => Value::Real(r),
            _ => v.clone(),
        },
        _ => v.clone(),
    };
    let text = |v: &Value| match v {
        Value::Int(i) => Value::Text(i.to_string()),
        Value::Real(r) => real_text(*r).map_or_else(|| v.clone(), Value::Text),
        _ => v.clone(),
    };
    match aff {
        a if a.numeric() => (num(x), num(v)),
        Affinity::Text if matches!(x, Value::Text(_)) || matches!(v, Value::Text(_)) => (text(x), text(v)),
        _ => (x.clone(), v.clone()),
    }
}

/// Whether column `c` is part of the key: the INTEGER PRIMARY KEY or a
/// WITHOUT ROWID primary key column.
pub fn is_key(t: &Table, c: usize) -> bool {
    match &t.key {
        Key::Rowid => false,
        Key::IntegerPrimaryKey(k) => *k == c,
        Key::WithoutRowid(pk) => pk.contains(&c),
    }
}

/// The value SQLite stores when `v` is written to column `c` (not an
/// INTEGER PRIMARY KEY), or `None` when the generator never writes it
/// there: it violates NOT NULL or STRICT, or SQLite would convert it in a
/// way left out of the model.
pub fn convert(t: &Table, c: usize, v: &Value) -> Option<Value> {
    if !sql_safe(v) || matches!(v, Value::Real(x) if x.is_nan()) {
        return None;
    }
    if *v == Value::Null {
        return (!t.not_null[c]).then_some(Value::Null);
    }
    let Some(types) = &t.strict else {
        return store(affinity(t, c), v);
    };
    match (types[c], v) {
        (ColType::Any, _) => Some(v.clone()),
        (ColType::Blob, Value::Blob(_)) | (ColType::Text, Value::Text(_)) => Some(v.clone()),
        (ColType::Int | ColType::Integer, Value::Int(_)) => Some(v.clone()),
        // Integral reals SQLite converts losslessly; 2^53 keeps every one
        // exact both ways.
        (ColType::Int | ColType::Integer, Value::Real(x)) if x.fract() == 0.0 && x.abs() <= 9_007_199_254_740_992.0 => {
            Some(Value::Int(*x as i64))
        },
        // A real equal to an integer is stored as that integer and read
        // back as a real: -0.0 comes back as 0.0.
        (ColType::Real, Value::Real(x)) => Some(Value::Real(if *x == 0.0 { 0.0 } else { *x })),
        (ColType::Real, Value::Int(i)) => Some(Value::Real(*i as f64)),
        _ => None,
    }
}

fn cmp3(aff: Affinity, coll: Coll, x: &Value, op: CmpOp, v: &Value, enc: Encoding) -> Option<bool> {
    if *x == Value::Null || *v == Value::Null {
        return None;
    }
    let (x, v) = operands(aff, x, v);
    let o = compare_coll(&x, &v, enc, coll);
    Some(match op {
        CmpOp::Eq => o == Ordering::Equal,
        CmpOp::Ne => o != Ordering::Equal,
        CmpOp::Lt => o == Ordering::Less,
        CmpOp::Le => o != Ordering::Greater,
        CmpOp::Gt => o == Ordering::Greater,
        CmpOp::Ge => o != Ordering::Less,
    })
}

fn and3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn or3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// A predicate's value on a row, in SQL's three-valued logic (`None` is
/// NULL).
pub fn eval(t: &Table, row: &Row, p: &Pred, enc: Encoding) -> Option<bool> {
    match p {
        Pred::Cmp(c, op, v) => cmp3(affinity(t, *c), column_coll(t, *c), &column_value(t, row, *c), *op, v, enc),
        Pred::IsNull(c, not) => Some((column_value(t, row, *c) == Value::Null) != *not),
        Pred::Between(c, lo, hi) => {
            let x = column_value(t, row, *c);
            let (aff, coll) = (affinity(t, *c), column_coll(t, *c));
            and3(cmp3(aff, coll, &x, CmpOp::Ge, lo, enc), cmp3(aff, coll, &x, CmpOp::Le, hi, enc))
        },
        Pred::And(a, b) => and3(eval(t, row, a, enc), eval(t, row, b, enc)),
        Pred::Or(a, b) => or3(eval(t, row, a, enc), eval(t, row, b, enc)),
        Pred::Not(a) => eval(t, row, a, enc).map(|x| !x),
    }
}

fn selects(t: &Table, row: &Row, p: &Option<Pred>, enc: Encoding) -> bool {
    p.as_ref().is_none_or(|p| eval(t, row, p, enc) == Some(true))
}

pub fn pred_valid(t: &Table, p: &Pred) -> bool {
    match p {
        Pred::Cmp(c, _, v) => comparable(t, *c, v),
        Pred::IsNull(c, _) => *c < t.columns,
        Pred::Between(c, lo, hi) => comparable(t, *c, lo) && comparable(t, *c, hi),
        Pred::And(a, b) | Pred::Or(a, b) => pred_valid(t, a) && pred_valid(t, b),
        Pred::Not(a) => pred_valid(t, a),
    }
}

fn distinct(cs: &[usize]) -> bool {
    cs.iter().enumerate().all(|(k, c)| !cs[..k].contains(c))
}

fn key_order(t: &Table, a: &Row, b: &Row, enc: Encoding) -> Ordering {
    match &t.key {
        Key::WithoutRowid(pk) => crate::model::compare_key(t, a, b, pk, enc),
        _ => a.rowid.cmp(&b.rowid),
    }
}

/// Whether an INSERT's columns and rows have the shape the model takes:
/// distinct columns of `t`, none generated, every NOT NULL and key column
/// listed, rows of the right length.
fn insert_shape_ok(t: &Table, ins: &Insert) -> bool {
    let listed = |c: usize| ins.columns.contains(&c);
    (!ins.columns.is_empty() || ins.rowid)
        && ins.columns.iter().all(|&c| c < t.columns)
        && distinct(&ins.columns)
        && ins.columns.iter().all(|&c| t.generated[c].is_none())
        && (!ins.rowid || t.key == Key::Rowid)
        && !ins.rows.is_empty()
        && ins.rows.iter().all(|r| r.len() == ins.columns.len() + usize::from(ins.rowid))
        // A NOT NULL column (an INTEGER PRIMARY KEY too) is always listed.
        && (0..t.columns).all(|c| !t.not_null[c] || listed(c))
        && match &t.key {
            Key::WithoutRowid(pk) => pk.iter().all(|&c| listed(c)),
            _ => true,
        }
}

/// The rows an INSERT adds to `t`, in statement order, with their rowids
/// (0 for a WITHOUT ROWID table, renumbered when added), or `None` when the
/// INSERT is not valid.
pub fn inserted_rows(t: &Table, ins: &Insert, enc: Encoding) -> Option<Vec<Row>> {
    let ipk = match t.key {
        Key::IntegerPrimaryKey(k) => Some(k),
        _ => None,
    };
    if !insert_shape_ok(t, ins) {
        return None;
    }
    let mut out: Vec<Row> = Vec::with_capacity(ins.rows.len());
    let mut top = t.rows.last().map(|r| r.rowid);
    for r in &ins.rows {
        let (given, vals) = if ins.rowid { (Some(&r[0]), &r[1..]) } else { (None, &r[..]) };
        let mut row = Row { rowid: 0, values: t.default.iter().map(|d| d.clone().unwrap_or(Value::Null)).collect() };
        let mut given = given.cloned();
        for (&c, v) in ins.columns.iter().zip(vals) {
            if Some(c) == ipk {
                given = Some(v.clone());
            } else {
                row.values[c] = convert(t, c, v)?;
            }
        }
        if let Key::WithoutRowid(pk) = &t.key {
            if pk.iter().any(|&c| row.values[c] == Value::Null) {
                return None;
            }
            let clash = |o: &Row| key_order(t, o, &row, enc) == Ordering::Equal;
            if t.rows.binary_search_by(|o| key_order(t, o, &row, enc)).is_ok() || out.iter().any(clash) {
                return None;
            }
        } else {
            row.rowid = match given {
                None | Some(Value::Null) => match top {
                    Some(i64::MAX) => return None,
                    Some(m) => m + 1,
                    None => 1,
                },
                Some(Value::Int(i)) => i,
                Some(_) => return None,
            };
            if ipk.is_some_and(|k| t.not_null[k]) && matches!(given, None | Some(Value::Null)) {
                return None;
            }
            if t.rows.binary_search_by_key(&row.rowid, |o| o.rowid).is_ok() || out.iter().any(|o| o.rowid == row.rowid) {
                return None;
            }
            top = Some(top.map_or(row.rowid, |m| m.max(row.rowid)));
        }
        regenerate(t, &mut row);
        out.push(row);
    }
    if !checks_hold(t, &out, enc) {
        return None;
    }
    let all: Vec<Row> = t.rows.iter().chain(&out).cloned().collect();
    unique_rows(t, &all, enc).then_some(out)
}

/// Whether no CHECK of `t` is false on any of `rows` (NULL passes).
pub fn checks_hold(t: &Table, rows: &[Row], enc: Encoding) -> bool {
    t.checks.iter().all(|p| rows.iter().all(|r| eval(t, r, p, enc) != Some(false)))
}

/// Whether every view selects from a table that is there, columns it has:
/// SQLite checks the whole schema after DROP COLUMN, RENAME COLUMN and
/// RENAME TO, and a view left over from a dropped table fails it.
pub fn views_resolve(m: &Model) -> bool {
    m.views.iter().all(|v| v.table.is_none_or(|t| m.tables.get(t).is_some_and(|t| t.db == Db::Main && v.columns.iter().all(|&c| c < t.columns))))
}

/// `p` with each column `c` as `f(c)`.
pub fn map_pred(p: &Pred, f: &dyn Fn(usize) -> usize) -> Pred {
    match p {
        Pred::Cmp(c, op, v) => Pred::Cmp(f(*c), *op, v.clone()),
        Pred::IsNull(c, n) => Pred::IsNull(f(*c), *n),
        Pred::Between(c, lo, hi) => Pred::Between(f(*c), lo.clone(), hi.clone()),
        Pred::And(a, b) => Pred::And(Box::new(map_pred(a, f)), Box::new(map_pred(b, f))),
        Pred::Or(a, b) => Pred::Or(Box::new(map_pred(a, f)), Box::new(map_pred(b, f))),
        Pred::Not(a) => Pred::Not(Box::new(map_pred(a, f))),
    }
}

fn pred_uses(p: &Pred, c: usize) -> bool {
    let mut out = Vec::new();
    p.leaves(&mut out);
    out.contains(&c)
}

/// A literal SQLite reads as a constant DEFAULT of ADD COLUMN
/// (`sqlite3ValueFromExpr`): one token, or a negative number; no NUL, no
/// value that reads as the current time, no `-0.0` and no infinity.
pub fn constant_default(v: &Value) -> bool {
    sql_safe(v)
        && !crate::model::reads_as_now(v)
        && match v {
            Value::Real(x) => x.is_finite() && !(*x == 0.0 && x.is_sign_negative()) && !literal(v).starts_with('('),
            _ => true,
        }
}

/// Whether `ALTER TABLE t{k} DROP CONSTRAINT name` drops a constraint of
/// `t` (its first by that name, `Table::names`) as the model has it: not
/// a PRIMARY KEY (SQLite refuses), and a foreign key only without SET
/// DEFAULT actions (SQLite's `getConstraint` ends the constraint at that
/// DEFAULT, and the SQL left does not parse: the SQLite bug
/// sqlite-drop-constraint-fk-set-default) and without orphans (none for
/// a deferred one to leave behind).
pub fn droppable_constraint(m: &Model, k: usize, t: &Table, name: &str) -> bool {
    match t.names.iter().find(|n| n.name.eq_ignore_ascii_case(name)) {
        Some(Named { column, cons: Cons::NotNull, .. }) => column.is_some_and(|c| c < t.columns && t.not_null[c]),
        Some(Named { cons: Cons::Fk(cols), .. }) => t.fks.iter().find(|f| f.columns == *cols).is_some_and(|f| {
            f.on_delete != Action::SetDefault && f.on_update != Action::SetDefault && orphans(m, k, f) == 0
        }),
        Some(Named { cons: Cons::Bare, .. }) => true,
        Some(Named { cons: Cons::Fixed, .. }) | None => false,
    }
}

/// Whether ADD COLUMN may give `t` the DEFAULT `v` in a database of
/// encoding `enc`: never a BLOB in a UTF-16 database whose table has rows.
/// The rows written before the ALTER read the default, and SQLite (trunk
/// before 178677e141) converts it to text as UTF-8 there (`valueFromExpr`
/// leaves its encoding UTF-8), where a stored copy is converted as UTF-16:
/// a text expression index over the column (`trim(c)`, `format(.., c)`)
/// is computed one way by CREATE INDEX and another by a later write, which
/// leaves "row 1 missing from index" (the known SQLite bug
/// sqlite-alter-default-blob-encoding; `sqlite_agrees_with_the_model`).
pub fn default_ok(enc: crate::model::Encoding, t: &Table, v: &Value) -> bool {
    enc == crate::model::Encoding::Utf8 || t.rows.is_empty() || !matches!(v, Value::Blob(_))
}

/// The column ADD COLUMN gives `t` alone, as a one-column table, for
/// [`convert`].
pub fn new_column_table(t: &Table, d: &NewColumn) -> Table {
    let mut probe = Table::new(1, Key::Rowid);
    probe.strict = t.strict.as_ref().map(|_| vec![d.ty.unwrap_or(ColType::Any)]);
    probe.decl = vec![d.decl];
    probe.coll = vec![d.coll];
    probe
}

/// The value rows read in the column `ALTER TABLE .. ADD COLUMN` adds to
/// `t`, or `None` when SQLite rejects it or the model leaves it out: a
/// STRICT table's column has a STRICT type, and a default of exactly that
/// type's class (no conversion); another table's column a type of
/// [`DECLS`] or none, and a default its affinity stores as the model
/// knows ([`store`]); NOT NULL a default that is not NULL, unless the
/// table is empty; at most [`MAX_COLUMNS`] columns.
pub fn added_default(t: &Table, d: &NewColumn) -> Option<Value> {
    let typed_ok = match (&t.strict, d.ty, d.decl) {
        (Some(_), Some(_), None) => true,
        (None, None, decl) => decl.is_none_or(|x| x < DECLS.len()),
        _ => false,
    };
    if t.columns >= MAX_COLUMNS || !typed_ok {
        return None;
    }
    match &d.default {
        None | Some(Value::Null) => (!d.not_null || t.rows.is_empty()).then_some(Value::Null),
        Some(v) => {
            if !constant_default(v) || d.ty.is_some_and(|ty| ty != ColType::Any && (!ty.admits(v) || *v == Value::Null)) {
                return None;
            }
            convert(&new_column_table(t, d), 0, v)
        },
    }
}

/// Table `k` of `m` without column `col`, the later columns one down,
/// in the table, its indexes, checks, views and triggers (see
/// [`Statement::DropColumn`]; [`valid`] makes sure nothing else names
/// the column).
fn drop_column(m: &mut Model, k: usize, col: usize) {
    let down = |c: usize| c - usize::from(c > col);
    for v in &mut m.views {
        if v.table == Some(k) {
            v.columns.iter_mut().for_each(|c| *c = down(*c));
        }
    }
    for g in &mut m.triggers {
        if g.table == k {
            g.column = g.column.map(down);
        }
    }
    for g in &mut m.rules {
        map_rule_columns(g, k, &down);
    }
    for (j, t) in m.tables.iter_mut().enumerate() {
        for f in &mut t.fks {
            if j == k {
                f.columns.iter_mut().for_each(|c| *c = down(*c));
            }
            if f.parent == k {
                f.parent_columns.iter_mut().for_each(|c| *c = down(*c));
            }
        }
    }
    let t = &mut m.tables[k];
    t.names.retain(|n| n.column != Some(col));
    for n in &mut t.names {
        n.column = n.column.map(down);
        if let Cons::Fk(cols) = &mut n.cons {
            cols.iter_mut().for_each(|c| *c = down(*c));
        }
    }
    t.columns -= 1;
    t.not_null.remove(col);
    t.decl.remove(col);
    t.coll.remove(col);
    t.default.remove(col);
    t.generated.remove(col);
    t.generated.iter_mut().for_each(|g| *g = g.map(down));
    if let Some(types) = &mut t.strict {
        types.remove(col);
    }
    for r in &mut t.rows {
        r.values.remove(col);
    }
    for i in &mut t.indexes {
        i.columns.iter_mut().for_each(|c| c.0 = down(c.0));
        i.coll.iter_mut().for_each(|c| c.0 = down(c.0));
        i.filter = i.filter.as_ref().map(|p| map_pred(p, &down));
    }
    t.checks = t.checks.iter().map(|p| map_pred(p, &down)).collect();
    t.key = match &t.key {
        Key::Rowid => Key::Rowid,
        Key::IntegerPrimaryKey(c) => Key::IntegerPrimaryKey(down(*c)),
        Key::WithoutRowid(pk) => Key::WithoutRowid(pk.iter().map(|&c| down(c)).collect()),
    };
}

/// How many schema objects name column `c` of table `k`: its table's
/// indexes (their columns, expressions and predicates), checks and
/// generated columns, the views and file triggers of it, the triggers
/// ([`rule_columns`]) and the foreign keys (as child or parent): what a
/// RENAME COLUMN of it rewrites (`renameColumnFunc` on each of them).
pub fn column_refs(m: &Model, k: usize, c: usize) -> usize {
    let Some(t) = m.tables.get(k) else {
        return 0;
    };
    t.indexes.iter().filter(|i| i.columns.iter().any(|&(x, _)| x == c) || i.filter.as_ref().is_some_and(|p| pred_uses(p, c))).count()
        + t.checks.iter().filter(|p| pred_uses(p, c)).count()
        + t.generated.iter().filter(|g| **g == Some(c)).count()
        + m.views.iter().filter(|v| v.table == Some(k) && v.columns.contains(&c)).count()
        + m.triggers.iter().filter(|g| g.table == k && g.column == Some(c)).count()
        + m.rules.iter().filter(|g| rule_columns(g, k).contains(&c)).count()
        + t.fks.iter().filter(|f| f.columns.contains(&c)).count()
        + m.tables.iter().map(|o| o.fks.iter().filter(|f| f.parent == k && f.parent_columns.contains(&c)).count()).sum::<usize>()
}

/// Whether column `c` of table `k` may be dropped: not the only column,
/// not part of the key, in no index (its columns, expression or
/// predicate), check, view or trigger, and the schema resolves.
pub fn droppable(m: &Model, k: usize, c: usize) -> bool {
    let Some(t) = m.tables.get(k) else {
        return false;
    };
    t.columns >= 2
        && c < t.columns
        && !is_key(t, c)
        && t.indexes.iter().all(|i| i.columns.iter().all(|&(x, _)| x != c) && i.filter.as_ref().is_none_or(|p| !pred_uses(p, c)))
        && t.checks.iter().all(|p| !pred_uses(p, c))
        && m.views.iter().all(|v| v.table != Some(k) || !v.columns.contains(&c))
        && m.triggers.iter().all(|g| g.table != k || g.column != Some(c))
        && m.rules.iter().all(|g| !rule_columns(g, k).contains(&c))
        // SQLite does not rename NEW.c of a trigger on a view of it.
        && m.rules.iter().all(|g| !g.view || m.views.get(g.table).is_none_or(|v| v.table != Some(k)))
        && t.fks.iter().all(|f| !f.columns.contains(&c))
        && t.generated.iter().all(|g| *g != Some(c))
        && m.tables.iter().all(|o| o.fks.iter().all(|f| f.parent != k || !f.parent_columns.contains(&c)))
        // SQLite fails a RENAME COLUMN of a TEMP table that a trigger (in
        // the TEMP schema, as every trigger on or writing one is) names,
        // "error in trigger w0: no such column: OLD.c0" (3.51.0 and trunk
        // 5bd093e261a7; bugs/sqlite/sqlite-temp-rename-column-twice):
        // no later column, which the drop renames, then.
        && (t.db != Db::Temp || c + 1 == t.columns || m.rules.iter().all(|g| (g.view || g.table != k) && rule_targets(g).all(|b| b != k)))
        && views_resolve(m)
}

/// Whether column `c` of table `k` may be renamed (and back, see
/// [`Statement::RenameColumn`]): as for the renames after a DROP COLUMN
/// ([`droppable`]), no trigger on a view of the table (SQLite does not
/// rename its `NEW.c`), no trigger naming a TEMP table (the known SQLite
/// bug bugs/sqlite/sqlite-temp-rename-column-twice), and the schema
/// resolves.
pub fn renamable(m: &Model, k: usize, c: usize) -> bool {
    let Some(t) = m.tables.get(k) else {
        return false;
    };
    c < t.columns
        && m.rules.iter().all(|g| !g.view || m.views.get(g.table).is_none_or(|v| v.table != Some(k)))
        && (t.db != Db::Temp || m.rules.iter().all(|g| (g.view || g.table != k) && rule_targets(g).all(|b| b != k)))
        && views_resolve(m)
}

/// Row `r` of `t` with its generated columns as they read: a copy of
/// their column.
pub fn regenerate(t: &Table, r: &mut Row) {
    for c in 0..t.columns {
        if let Some(src) = t.generated[c] {
            let v = column_value(t, r, src);
            r.values[c] = v;
        }
    }
}

/// Whether `rows` of `t` keep every UNIQUE index of `t` ([`unique_ok`]).
pub fn unique_rows(t: &Table, rows: &[Row], enc: Encoding) -> bool {
    t.indexes.iter().all(|i| !i.unique || unique_ok(t, i, rows, enc))
}

/// The rows of `t` after `UPDATE`'s sets on the rows its filter selects.
fn updated_rows(t: &Table, u: &Update, enc: Encoding) -> Option<Vec<Row>> {
    let sets: Vec<(usize, Value)> = u.sets.iter().map(|(c, v)| Some((*c, convert(t, *c, v)?))).collect::<Option<_>>()?;
    let mut rows = t.rows.clone();
    for r in &mut rows {
        if selects(t, r, &u.filter, enc) {
            for (c, v) in &sets {
                r.values[*c] = v.clone();
            }
            regenerate(t, r);
        }
    }
    Some(rows)
}

/// The most savepoints open at once.
pub const MAX_SAVEPOINTS: usize = 3;

/// The most tables, and indexes per table, statements create.
pub const MAX_SCHEMA_TABLES: usize = 12;
pub const MAX_SCHEMA_INDEXES: usize = 8;

/// Whether `t` may be created as a new table: well formed, without rows or
/// indexes.
pub fn new_table_ok(t: &Table) -> bool {
    t.rows.is_empty()
        && t.indexes.is_empty()
        && Model { encoding: Encoding::Utf8, tables: vec![t.clone()], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() }.well_formed()
}

/// Whether a crafted `sqlite_stat1` row names a table of `m` and one of
/// its indexes (the primary key only of a WITHOUT ROWID table), with the
/// row count and at most one number per column of the index after it (one
/// in all for the table's own row). The numbers themselves may be anything
/// SQLite parses: huge, zero, increasing where ANALYZE's decrease.
pub fn stat_valid(m: &Model, r: &Stat) -> bool {
    let Some(t) = m.tables.get(r.table) else {
        return false;
    };
    let columns = match r.of {
        StatOf::Table => 0,
        StatOf::Index(k) => match t.indexes.get(k) {
            Some(i) => i.columns.len(),
            None => return false,
        },
        StatOf::PrimaryKey => match &t.key {
            Key::WithoutRowid(pk) => pk.len(),
            _ => return false,
        },
    };
    (1..=1 + columns).contains(&r.counts.len()) && (r.of != StatOf::Table || !r.unordered && !r.noskipscan)
}

/// The plain UPDATE an `UPDATE .. FROM` ([`crate::statements::UpdateFrom`])
/// is: its sets that read the FROM row given that row's values (as they are
/// before the UPDATE: SQLite materializes the join first), its filter
/// false (`c0 IS NULL AND c0 IS NOT NULL`) when the row is missing. `None`
/// when the FROM table is no rowid table of `m` other than the target, or
/// lacks a column read, or the filter does not resolve against the target
/// (checked here since a missing row's plain UPDATE has another filter).
pub fn resolve_from(m: &Model, u: &Update) -> Option<Update> {
    let Some(f) = &u.from else {
        return Some(u.clone());
    };
    let src = m.tables.get(f.table)?;
    if matches!(src.key, Key::WithoutRowid(_)) || f.table == u.table || u.table >= m.tables.len() {
        return None;
    }
    // The filter resolves against the target whether or not the FROM row
    // is there: SQLite prepares it either way, and a missing row replaces
    // it below.
    if !u.filter.as_ref().is_none_or(|p| pred_valid(&m.tables[u.table], p)) {
        return None;
    }
    let mut plain = Update { from: None, ..u.clone() };
    if !f.reads.iter().all(|&(k, c)| k < u.sets.len() && c < src.columns) || !distinct(&f.reads.iter().map(|&(k, _)| k).collect::<Vec<_>>()) {
        return None;
    }
    match src.rows.iter().find(|r| r.rowid == f.rowid) {
        Some(r) => {
            for &(k, c) in &f.reads {
                plain.sets[k].1 = column_value(src, r, c);
            }
        },
        None => plain.filter = Some(Pred::And(Box::new(Pred::IsNull(0, false)), Box::new(Pred::IsNull(0, true)))),
    }
    Some(plain)
}

pub fn valid(m: &Model, s: &Statement) -> bool {
    if let Statement::Update(u) = s {
        if u.from.is_some() {
            return resolve_from(m, u).is_some_and(|p| valid(m, &Statement::Update(p)));
        }
    }
    if row_by_row(m, s) {
        return write_rows(&mut scratch(m), s).is_some();
    }
    valid_plain(m, s) && written_table(s).is_none_or(|k| !fk_relevant(m, k) || write_with_fks(&mut scratch(m), s).is_some())
}

fn valid_plain(m: &Model, s: &Statement) -> bool {
    let table = |k: usize| m.tables.get(k);
    match s {
        Statement::Select(s) => table(s.table).is_some_and(|t| {
            s.columns.iter().all(|&c| c < t.columns)
                && s.order.iter().all(|&(c, _)| c < t.columns)
                && s.filter.as_ref().is_none_or(|p| pred_valid(t, p))
        }),
        // No value SQLite's date functions read as the current time
        // (`crate::model::reads_as_now`): an expression index of one fails.
        Statement::Insert(ins) => table(ins.table)
            .is_some_and(|t| returning_ok(t, &ins.returning) && ins.rows.iter().flatten().all(|v| !crate::model::reads_as_now(v)) && inserted_rows(t, ins, m.encoding).is_some()),
        Statement::Update(u) => table(u.table).is_some_and(|t| {
            let cols: Vec<usize> = u.sets.iter().map(|(c, _)| *c).collect();
            !cols.is_empty()
                && distinct(&cols)
                && u.or != Some(Resolve::ReplaceInto)
                && returning_ok(t, &u.returning)
                && u.sets.iter().all(|(c, v)| *c < t.columns && !is_key(t, *c) && t.generated[*c].is_none() && convert(t, *c, v).is_some() && !crate::model::reads_as_now(v))
                && u.filter.as_ref().is_none_or(|p| pred_valid(t, p))
                // With constant sets, a UNIQUE conflict on the way (SQLite
                // checks row by row) is one in the rows after it too.
                && ((t.indexes.iter().all(|i| !i.unique) && t.checks.is_empty())
                    || updated_rows(t, u, m.encoding).is_some_and(|r| unique_rows(t, &r, m.encoding) && checks_hold(t, &r, m.encoding)))
        }),
        Statement::Delete(d) => {
            table(d.table).is_some_and(|t| returning_ok(t, &d.returning) && d.filter.as_ref().is_none_or(|p| pred_valid(t, p)))
        },
        // VACUUM fails inside a transaction.
        Statement::Vacuum => m.savepoints.is_empty(),
        Statement::IncrementalVacuum(_) | Statement::CacheSize(_) | Statement::AnalysisLimit(_) => true,
        Statement::Optimize(_) | Statement::AutomaticIndex(_) | Statement::ReverseUnordered(_) => true,
        Statement::Stats(rows) => !rows.is_empty() && rows.iter().all(|r| stat_valid(m, r)),
        Statement::Analyze(target) => match target {
            None => true,
            Some((k, None)) => table(*k).is_some(),
            Some((k, Some(i))) => table(*k).is_some_and(|t| *i < t.indexes.len()),
        },
        Statement::Savepoint(k) => *k == m.savepoints.len() && *k < MAX_SAVEPOINTS,
        // Never without a rollback journal (journal_mode OFF): what the
        // transaction wrote stays, or not, as the cache spilled.
        Statement::RollbackTo(k) => *k < m.savepoints.len() && !m.session.journal_off && memory_rollback_ok(m, *k) && crate::vtabs::rollback_to_ok(&m.savepoints[*k].vtabs, &m.vtabs),
        // The commit fails while a deferred foreign key has an orphan;
        // inside a BEGIN, `RELEASE s0` would leave the transaction open.
        Statement::Release(k) => *k < m.savepoints.len() && (*k > 0 || (!m.session.begun && (!m.foreign_keys || deferred_orphans(m) == 0))),
        Statement::Rollback => !m.savepoints.is_empty() && !m.session.journal_off && memory_rollback_ok(m, 0),
        Statement::Commit => !m.savepoints.is_empty() && (!m.foreign_keys || deferred_orphans(m) == 0),
        Statement::Begin(_) => m.savepoints.is_empty(),
        // Outside a transaction: a journal mode change into or out of WAL
        // fails inside one, a checkpoint inside a write transaction fails,
        // VACUUM fails inside one.
        Statement::JournalMode(_) | Statement::WalCheckpoint(_) | Statement::AutoVacuum(_) | Statement::VacuumInto(_) => m.savepoints.is_empty(),
        // Changing it closes the TEMP database, its triggers with it.
        // Its tables and views too.
        Statement::TempStore(_) => m.savepoints.is_empty() && m.rules.iter().all(|g| !g.temp && g.db != Db::Temp) && m.tables.iter().all(|t| t.db != Db::Temp),
        Statement::SecureDelete(n) => *n <= 2,
        // "Safety level may not be changed inside a transaction".
        Statement::Synchronous(n) => *n <= 3 && m.savepoints.is_empty(),
        Statement::ShrinkMemory => true,
        Statement::Attach(_) => m.savepoints.is_empty() && !m.session.aux,
        Statement::Detach => m.savepoints.is_empty() && m.session.aux && detachable(m),
        Statement::AnalyzeDb(db) => *db != Db::Aux || m.session.aux,
        Statement::Virtual { s, kind } => {
            crate::vtabs::kind_ok(&m.vtabs, s, kind) && crate::vtabs::valid(&m.vtabs, s, !m.savepoints.is_empty()) && crate::vtabs::vocab_ok(s, m.encoding != crate::model::Encoding::Utf8)
        },
        Statement::DbPragma { db, k, table: t } => {
            (*db != Db::Aux || m.session.aux) && *k < crate::statements::DB_PRAGMAS.len() && crate::statements::DB_PRAGMAS[*k].1 == t.is_some() && t.is_none_or(|k| table(k).is_some_and(|t| t.db == *db))
        },
        Statement::CellSizeCheck(_) | Statement::MmapSize(_) | Statement::LockingMode(_) => true,
        Statement::JournalSizeLimit(_) | Statement::WalAutocheckpoint(_) | Statement::CacheSpill(_) => true,
        Statement::Check { table: t, .. } => t.is_none_or(|k| table(k).is_some()),
        Statement::Count { table: k, filter } => {
            table(*k).is_some_and(|t| filter.as_ref().is_none_or(|p| pred_valid(t, p)))
        },
        Statement::CreateTable { table: k, def } => {
            *k == m.tables.len() && *k < MAX_SCHEMA_TABLES && (def.db != Db::Aux || m.session.aux) && new_table_ok(def) && def.default.iter().all(Option::is_none) && def.generated.iter().all(Option::is_none) && def.checks.is_empty() && def.fks.iter().all(|f| fk_ok(m, def, f))
        },
        // Never a parent: its children's keys would name no table.
        // No other table's trigger writes it.
        Statement::DropTable(k) => {
            *k + 1 == m.tables.len() && !m.tables.iter().any(|t| t.fks.iter().any(|f| f.parent == *k)) && m.rules.iter().all(|g| !(g.temp && !g.view && g.table == *k) && ((!g.view && g.table == *k) || rule_targets(g).all(|b| b != *k)))
        },
        Statement::CreateIndex { table: k, index, def, db } => table(*k).is_some_and(|t| {
            let cols: Vec<usize> = def.columns.iter().map(|&(c, _)| c).collect();
            // No DESC column: below schema format 4 SQLite ignores DESC, the
            // index VACUUM builds in its new (format 4) database then differs
            // from the original, so VACUUM copies the table without the
            // transfer optimization and renumbers its rowids; the model does
            // not know the file's format.
            // A partial index's predicate is never evaluated by the model
            // (no statement depends on which rows the index holds); it
            // is a predicate of the table so that it resolves.
            *index == t.indexes.len()
                && t.db == *db
                && *index < MAX_SCHEMA_INDEXES
                && !cols.is_empty()
                && distinct(&cols)
                && cols.iter().all(|&c| c < t.columns)
                && def.columns.iter().all(|&(_, d)| !d)
                && def.filter.as_ref().is_none_or(|p| pred_valid(t, p))
                && def.coll_ok()
                && (!def.unique || (def.expr.is_none() && def.filter.is_none() && unique_ok(t, def, &t.rows, m.encoding)))
        }),
        // Never the last parent key of a foreign key.
        Statement::DropIndex { table: k, index, db } => table(*k).is_some_and(|t| {
            *index + 1 == t.indexes.len()
                && t.db == *db && m.tables.iter().all(|c| c.fks.iter().all(|f| f.parent != *k || parent_unique_in(t, &t.indexes[..*index], &f.parent_columns)))
        }),
        Statement::ForeignKeys(on) => m.savepoints.is_empty() && (!*on || m.tables.iter().enumerate().all(|(k, t)| t.fks.iter().all(|f| orphans(m, k, f) == 0))),
        Statement::FkCheck(t) => t.is_none_or(|k| table(k).is_some()),
        // A REFERENCES column has no default (SQLite refuses one with
        // foreign keys on), and its parent does not reach the table.
        Statement::AddColumn { table: k, column, def } => table(*k).is_some_and(|t| {
            *column == t.columns
                && added_default(t, def).is_some()
                && def.default.as_ref().is_none_or(|v| default_ok(m.encoding, t, v))
                // A generated column: a copy of a column (not generated
                // itself), without a type, a default, NOT NULL or a key,
                // in a table that is not STRICT.
                && def.generated.is_none_or(|(src, _)| {
                    src < t.columns && t.generated[src].is_none() && t.strict.is_none() && def.decl.is_none() && def.default.is_none() && !def.not_null && def.references.is_none()
                })
                && def.references.as_ref().is_none_or(|f| {
                    let mut probe = Table::new(t.columns + 1, t.key.clone());
                    probe.db = t.db;
                    probe.strict = t.strict.as_ref().map(|ty| ty.iter().copied().chain(def.ty).collect());
                    probe.decl = t.decl.iter().copied().chain([def.decl]).collect();
                    probe.coll = t.coll.iter().copied().chain([def.coll]).collect();
                    f.columns == [*column] && def.default.as_ref().is_none_or(|v| *v == Value::Null) && f.parent != *k && !fk_reaches(m, f.parent, *k) && fk_ok(m, &probe, f)
                })
        }),
        Statement::DropColumn { table: k, column, columns } => table(*k).is_some_and(|t| t.columns == *columns) && droppable(m, *k, *column),
        Statement::RenameColumn { table: k, column } => renamable(m, *k, *column),
        Statement::NotNull { table: k, column: c, on } => table(*k).is_some_and(|t| {
            *c < t.columns && t.generated[*c].is_none() && t.not_null[*c] != *on && (!*on || t.rows.iter().all(|r| column_value(t, r, *c) != Value::Null))
        }),
        Statement::AddCheck { table: k, index, check } => table(*k).is_some_and(|t| {
            *index == t.checks.len() && *index < MAX_CHECKS && check.depth() <= 3 && pred_valid(t, check) && t.rows.iter().all(|r| eval(t, r, check, m.encoding) != Some(false))
        }),
        Statement::DropCheck { table: k, check } => table(*k).is_some_and(|t| *check + 1 == t.checks.len()),
        Statement::DropConstraint { table: k, name } => table(*k).is_some_and(|t| droppable_constraint(m, *k, t, name)),
        Statement::SwapTables { a, b, indexes } => {
            // Of main's tables: a rename stays in its schema.
            a != b && table(*a).is_some_and(|t| t.indexes == indexes.0 && t.db == Db::Main) && table(*b).is_some_and(|t| t.indexes == indexes.1 && t.db == Db::Main) && views_resolve(m)
        },
        Statement::CreateRule(g) => g.id == next_rule_id(m) && m.rules.len() < MAX_RULES && rule_ok(m, g),
        Statement::DropRule(id) => m.rules.iter().any(|g| g.id == *id),
        Statement::RecursiveTriggers(_) => true,
        Statement::Reindex(r) => match *r {
            Reindex::All | Reindex::Coll(_) => true,
            Reindex::Table(k) => table(k).is_some(),
            Reindex::Index(k, i) => table(k).is_some_and(|t| i < t.indexes.len()),
        },
        Statement::ViewWrite { view, write } => view_write(&mut scratch(m), *view, write).is_some(),
    }
}

/// The most checks a table has.
pub const MAX_CHECKS: usize = 3;

/// The rows a SELECT must return: `groups` in order, the rows within a
/// group in any order, and of all that only the first `limit` rows (with
/// no ORDER BY, there is one group).
#[derive(Clone, Debug, PartialEq)]
pub struct Expected {
    pub groups: Vec<Vec<Vec<Value>>>,
    pub limit: Option<usize>,
}

/// A value under exact equality: reals by their bits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Exact {
    Null,
    Int(i64),
    Real(u64),
    Text(String),
    Blob(Vec<u8>),
}

fn exact(row: &[Value]) -> Vec<Exact> {
    row.iter()
        .map(|v| match v {
            Value::Null => Exact::Null,
            Value::Int(i) => Exact::Int(*i),
            Value::Real(x) => Exact::Real(x.to_bits()),
            Value::Text(s) => Exact::Text(s.clone()),
            Value::Blob(b) => Exact::Blob(b.clone()),
        })
        .collect()
}

/// Whether `part` is a sub-multiset of `whole`, rows compared exactly.
fn within(part: &[Vec<Value>], whole: &[Vec<Value>]) -> bool {
    let mut a: Vec<_> = part.iter().map(|r| exact(r)).collect();
    let mut b: Vec<_> = whole.iter().map(|r| exact(r)).collect();
    a.sort();
    b.sort();
    let mut j = 0;
    for x in &a {
        while j < b.len() && b[j] < *x {
            j += 1;
        }
        if j == b.len() || b[j] != *x {
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

    /// Whether SQLite's rows are a result this SELECT may return.
    pub fn matches(&self, actual: &[Vec<Value>]) -> bool {
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

/// What a statement did: the rows a SELECT must return, and the rows
/// written (inserted, updated or deleted).
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub rows: Option<Expected>,
    pub written: usize,
}

fn renumber(t: &mut Table) {
    if matches!(t.key, Key::WithoutRowid(_)) {
        for (k, r) in t.rows.iter_mut().enumerate() {
            r.rowid = k as i64 + 1;
        }
    }
}

/// A copy of `m` to try a write on: its tables, and whether a transaction
/// is open (not the saved models).
fn scratch(m: &Model) -> Model {
    let open = || Model { encoding: m.encoding, tables: Vec::new(), views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: m.foreign_keys, rules: Vec::new(), recursive_triggers: m.recursive_triggers, session: m.session, vtabs: Vec::new() };
    Model { tables: m.tables.clone(), views: m.views.clone(), rules: m.rules.clone(), savepoints: if m.savepoints.is_empty() { Vec::new() } else { vec![open()] }, ..open() }
}

/// The table an INSERT, UPDATE or DELETE writes.
fn written_table(s: &Statement) -> Option<usize> {
    match s {
        Statement::Insert(i) => Some(i.table),
        Statement::Update(u) => Some(u.table),
        Statement::Delete(d) => Some(d.table),
        _ => None,
    }
}

/// Whether a write to table `k` is checked by SQLite against foreign keys:
/// they are on, and `k` is a child or a parent.
pub fn fk_relevant(m: &Model, k: usize) -> bool {
    m.foreign_keys && (!m.tables[k].fks.is_empty() || m.tables.iter().any(|t| t.fks.iter().any(|f| f.parent == k)))
}

/// The values of columns `cols` of row `r` (the rowid for an INTEGER
/// PRIMARY KEY).
fn key_of(t: &Table, r: &Row, cols: &[usize]) -> Vec<Value> {
    cols.iter().map(|&c| column_value(t, r, c)).collect()
}

fn cmp_keys(a: &[Value], b: &[Value], colls: &[Coll], enc: Encoding) -> Ordering {
    a.iter().zip(b).zip(colls).map(|((x, y), k)| compare_coll(x, y, enc, *k)).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal)
}

fn same_set(a: &[usize], b: &[usize]) -> bool {
    a.len() == b.len() && a.iter().all(|c| b.contains(c))
}

/// Whether columns `cols` of `p` are a parent key SQLite accepts
/// (`sqlite3FkLocateIndex`): its INTEGER PRIMARY KEY, the columns of its
/// WITHOUT ROWID primary key, or of a UNIQUE index whose sequences are the
/// columns' own, in any order.
pub fn parent_unique(p: &Table, cols: &[usize]) -> bool {
    parent_unique_in(p, &p.indexes, cols)
}

fn parent_unique_in(p: &Table, indexes: &[crate::model::Index], cols: &[usize]) -> bool {
    let key = match &p.key {
        Key::IntegerPrimaryKey(k) => cols == [*k],
        Key::WithoutRowid(pk) => same_set(pk, cols),
        Key::Rowid => false,
    };
    key || indexes.iter().any(|i| {
        let ic: Vec<usize> = i.columns.iter().map(|&(c, _)| c).collect();
        i.unique && i.expr.is_none() && i.filter.is_none() && same_set(&ic, cols) && ic.iter().all(|&c| crate::model::index_coll(p, i, c) == column_coll(p, c))
    })
}

/// Whether foreign key `f` of `child` is one the model follows (see
/// [`ForeignKey`]) and names a parent key of `m` SQLite accepts.
pub fn fk_ok(m: &Model, child: &Table, f: &ForeignKey) -> bool {
    let Some(p) = m.tables.get(f.parent) else {
        return false;
    };
    // A parent of the child's schema: REFERENCES names a table there.
    p.db == child.db
        && !f.columns.is_empty()
        && f.columns.len() == f.parent_columns.len()
        && distinct(&f.columns)
        && distinct(&f.parent_columns)
        && f.columns.iter().all(|&c| c < child.columns && !is_key(child, c) && child.generated.get(c).is_none_or(Option::is_none))
        && f.parent_columns.iter().all(|&c| c < p.columns && p.generated.get(c).is_none_or(Option::is_none))
        && f.columns.iter().zip(&f.parent_columns).all(|(&c, &pc)| affinity(child, c) == affinity(p, pc) && column_coll(child, c) == column_coll(p, pc))
        && parent_unique(p, &f.parent_columns)
}

/// Whether table `b` reaches table `a` by foreign keys (child to parent),
/// `b` itself included.
pub fn fk_reaches(m: &Model, b: usize, a: usize) -> bool {
    let mut seen = vec![false; m.tables.len()];
    let mut stack = vec![b];
    while let Some(t) = stack.pop() {
        if t == a {
            return true;
        }
        if t < seen.len() && !std::mem::replace(&mut seen[t], true) {
            stack.extend(m.tables[t].fks.iter().map(|f| f.parent));
        }
    }
    false
}

/// The rows of table `k` that foreign key `f` of it finds no parent row
/// for (a row with NULL in any of its columns has none to find).
pub fn orphans(m: &Model, k: usize, f: &ForeignKey) -> usize {
    let (c, p) = (&m.tables[k], &m.tables[f.parent]);
    let enc = m.encoding;
    let colls: Vec<Coll> = f.parent_columns.iter().map(|&pc| column_coll(p, pc)).collect();
    let mut keys: Vec<Vec<Value>> = p.rows.iter().map(|r| key_of(p, r, &f.parent_columns)).filter(|k| !k.contains(&Value::Null)).collect();
    keys.sort_by(|a, b| cmp_keys(a, b, &colls, enc));
    c.rows
        .iter()
        .map(|r| key_of(c, r, &f.columns))
        .filter(|ck| !ck.contains(&Value::Null) && keys.binary_search_by(|pk| cmp_keys(pk, ck, &colls, enc)).is_err())
        .count()
}

/// Whether the rows an INSERT into table `k` adds have their parents,
/// when foreign keys are on (a quick check for drawing; [`valid`] is the
/// judge).
pub fn fk_rows_ok(m: &Model, k: usize, ins: &Insert) -> bool {
    if !m.foreign_keys || m.tables[k].fks.is_empty() {
        return true;
    }
    let t = &m.tables[k];
    let txn = !m.savepoints.is_empty();
    let Some(rows) = inserted_rows(t, ins, m.encoding) else {
        return false;
    };
    t.fks.iter().filter(|f| !(txn && f.deferred)).all(|f| {
        let p = &m.tables[f.parent];
        let colls: Vec<Coll> = f.parent_columns.iter().map(|&pc| column_coll(p, pc)).collect();
        rows.iter().all(|r| {
            let ck = key_of(t, r, &f.columns);
            ck.contains(&Value::Null) || p.rows.iter().any(|pr| cmp_keys(&key_of(p, pr, &f.parent_columns), &ck, &colls, m.encoding) == Ordering::Equal)
        })
    })
}

/// Whether no foreign key of a child or a parent among `tables` has an
/// orphan; inside a transaction a deferred one may have (SQLite checks it
/// at the commit).
pub fn fk_consistent(m: &Model, tables: &[usize]) -> bool {
    let txn = !m.savepoints.is_empty();
    m.tables.iter().enumerate().all(|(k, t)| t.fks.iter().all(|f| (txn && f.deferred) || !(tables.contains(&k) || tables.contains(&f.parent)) || orphans(m, k, f) == 0))
}

/// The orphans of deferred foreign keys.
pub fn deferred_orphans(m: &Model) -> usize {
    m.tables.iter().enumerate().map(|(k, t)| t.fks.iter().filter(|f| f.deferred).map(|f| orphans(m, k, f)).sum::<usize>()).sum()
}

/// The foreign key actions of rows of table `k` deleted (`None`) or
/// updated, as SQLite runs them (each a trigger of the child: CASCADE
/// deletes the child rows of a deleted parent or gives them its new key,
/// SET NULL and SET DEFAULT write NULL or the defaults, RESTRICT fails at
/// once; an UPDATE acts only where the key changed, under `IS` with the
/// parent column's sequence), the child rows' own children in turn; then
/// every constraint of the tables written. `None` when SQLite fails the
/// statement, or the model leaves it out: a child row two actions reach
/// (their order would matter), RESTRICT on a child another action reached.
fn fk_actions(m: &mut Model, k: usize, events: Vec<(Row, Option<Row>)>) -> Option<()> {
    let enc = m.encoding;
    let mut queue: std::collections::VecDeque<(usize, Row, Option<Row>)> = events.into_iter().map(|(o, n)| (k, o, n)).collect();
    let mut touched: Vec<(usize, i64)> = Vec::new();
    // The child rows actions reached, as they were before: a later action
    // that would have reached one too, had the earlier one not deleted it
    // or rewritten its key, reaches it in SQLite's order as well.
    let mut reached: Vec<(usize, Row)> = Vec::new();
    let mut written = vec![k];
    while let Some((t, old, new)) = queue.pop_front() {
        for c in 0..m.tables.len() {
            for fi in 0..m.tables[c].fks.len() {
                let f = m.tables[c].fks[fi].clone();
                if f.parent != t {
                    continue;
                }
                let p = &m.tables[t];
                let colls: Vec<Coll> = f.parent_columns.iter().map(|&pc| column_coll(p, pc)).collect();
                let old_key = key_of(p, &old, &f.parent_columns);
                if old_key.contains(&Value::Null) {
                    continue;
                }
                let (action, new_key) = match &new {
                    None => (f.on_delete, None),
                    Some(n) => {
                        let new_key = key_of(p, n, &f.parent_columns);
                        let same = old_key.iter().zip(&new_key).zip(&colls).all(|((a, b), coll)| *b != Value::Null && compare_coll(a, b, enc, *coll) == Ordering::Equal);
                        if same {
                            continue;
                        }
                        (f.on_update, Some(new_key))
                    },
                };
                let refers = |child: &Table, r: &Row| {
                    let ck = key_of(child, r, &f.columns);
                    !ck.contains(&Value::Null) && cmp_keys(&ck, &old_key, &colls, enc) == Ordering::Equal
                };
                let hits: Vec<usize> = (0..m.tables[c].rows.len()).filter(|&i| refers(&m.tables[c], &m.tables[c].rows[i])).collect();
                // RESTRICT where another action reached the child: their
                // order would matter.
                if action == Action::Restrict && (!hits.is_empty() || touched.iter().any(|&(x, _)| x == c)) {
                    return None;
                }
                // A row another action reached first (deleted, or its key
                // rewritten) that this one reaches too: their order would
                // matter (run 1313 cte w4-67870, a CASCADE and a SET
                // DEFAULT on one child row, the SET DEFAULT failing a CHECK
                // first in SQLite).
                if action != Action::NoAction && reached.iter().any(|(x, r)| *x == c && refers(&m.tables[c], r)) {
                    return None;
                }
                if hits.is_empty() || action == Action::NoAction {
                    continue;
                }
                if !written.contains(&c) {
                    written.push(c);
                }
                for &i in &hits {
                    let id = (c, m.tables[c].rows[i].rowid);
                    if touched.contains(&id) {
                        return None;
                    }
                    touched.push(id);
                    reached.push((c, m.tables[c].rows[i].clone()));
                }
                if action == Action::Cascade && new_key.is_none() {
                    for &i in hits.iter().rev() {
                        let r = m.tables[c].rows.remove(i);
                        queue.push_back((c, r, None));
                    }
                    continue;
                }
                let child = &m.tables[c];
                let values: Vec<Value> = f
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(j, &cc)| {
                        let v = match (action, &new_key) {
                            (Action::Cascade, Some(nk)) => nk[j].clone(),
                            (Action::SetDefault, _) => child.default[cc].clone().unwrap_or(Value::Null),
                            _ => Value::Null,
                        };
                        convert(child, cc, &v)
                    })
                    .collect::<Option<_>>()?;
                for &i in &hits {
                    let before = m.tables[c].rows[i].clone();
                    let mut after = before.clone();
                    for (&cc, v) in f.columns.iter().zip(&values) {
                        after.values[cc] = v.clone();
                    }
                    regenerate(&m.tables[c], &mut after);
                    m.tables[c].rows[i] = after.clone();
                    queue.push_back((c, before, Some(after)));
                }
            }
        }
    }
    for &t in &written {
        let table = &m.tables[t];
        if !unique_rows(table, &table.rows, enc) || !checks_hold(table, &table.rows, enc) {
            return None;
        }
        renumber(&mut m.tables[t]);
    }
    fk_consistent(m, &written).then_some(())
}

/// An INSERT, UPDATE or DELETE of a table foreign keys check
/// ([`fk_relevant`]): the write, its actions ([`fk_actions`]), and no
/// orphan after it (the model never leaves one with foreign keys on, so
/// deferred keys are as immediate ones); `None` when it fails.
fn write_with_fks(m: &mut Model, s: &Statement) -> Option<Effect> {
    let k = written_table(s)?;
    let enc = m.encoding;
    let t = &m.tables[k];
    let (events, updated): (Vec<(Row, Option<Row>)>, Vec<usize>) = match s {
        Statement::Delete(d) => (t.rows.iter().filter(|r| selects(t, r, &d.filter, enc)).map(|r| (r.clone(), None)).collect(), Vec::new()),
        Statement::Update(u) => (Vec::new(), (0..t.rows.len()).filter(|&i| selects(t, &t.rows[i], &u.filter, enc)).collect()),
        _ => (Vec::new(), Vec::new()),
    };
    let olds: Vec<Row> = updated.iter().map(|&i| t.rows[i].clone()).collect();
    m.foreign_keys = false;
    let effect = apply(m, s);
    m.foreign_keys = true;
    let mut events = events;
    events.extend(olds.into_iter().zip(&updated).map(|(o, &i)| (o, Some(m.tables[k].rows[i].clone()))));
    fk_actions(m, k, events)?;
    Some(effect)
}

// ------------------------------------------------------- row by row writes

/// Whether a write is modelled row by row ([`write_rows`]): an INSERT with
/// a conflict resolution or UPSERT clauses, an `UPDATE OR IGNORE|REPLACE`.
fn row_by_row(m: &Model, s: &Statement) -> bool {
    written_table(s).is_some_and(|k| k < m.tables.len() && (has_rules(m, k) || (matches!(s, Statement::Insert(_)) && m.tables[k].key_conflict.is_some() && m.tables[k].key != Key::Rowid))) || resolves(s)
}

fn resolves(s: &Statement) -> bool {
    match s {
        Statement::Insert(i) => i.or.is_some() || !i.upsert.is_empty(),
        Statement::Update(u) => u.or.is_some_and(|r| matches!(r, Resolve::Ignore | Resolve::Replace)),
        _ => false,
    }
}

fn returning_ok(t: &Table, r: &Option<Vec<usize>>) -> bool {
    r.as_ref().is_none_or(|cs| cs.iter().all(|&c| c < t.columns))
}

/// Whether rows `a` and `b` of `t` agree on every column of UNIQUE index
/// `i`, none NULL, as the index compares them.
fn index_clash(t: &Table, i: &crate::model::Index, a: &Row, b: &Row, enc: Encoding) -> bool {
    i.columns.iter().all(|&(c, _)| {
        let (x, y) = (stored(t, c, &column_value(t, a, c)), stored(t, c, &column_value(t, b, c)));
        x != Value::Null && y != Value::Null && compare_coll(&x, &y, enc, crate::model::index_coll(t, i, c)) == Ordering::Equal
    })
}

/// The rows of `t` (but the one at `skip`) that row `r` conflicts with:
/// `(constraint, position)`, constraint 0 the key (rowid, INTEGER PRIMARY
/// KEY or WITHOUT ROWID primary key), `1 + i` UNIQUE index `i`.
fn conflicts(t: &Table, r: &Row, skip: Option<usize>, enc: Encoding) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (pos, o) in t.rows.iter().enumerate() {
        if Some(pos) == skip {
            continue;
        }
        if key_order(t, o, r, enc) == Ordering::Equal {
            out.push((0, pos));
        }
        for (k, i) in t.indexes.iter().enumerate() {
            if i.unique && index_clash(t, i, o, r, enc) {
                out.push((1 + k, pos));
            }
        }
    }
    out
}

/// The constraint an ON CONFLICT target names (see [`conflicts`]), when
/// exactly one has its columns: the INTEGER PRIMARY KEY, the WITHOUT ROWID
/// primary key, or a UNIQUE index; each term's COLLATE, if any, the
/// index's own.
pub fn upsert_target(t: &Table, target: &[(usize, Option<Coll>)]) -> Option<usize> {
    let cols: Vec<usize> = target.iter().map(|&(c, _)| c).collect();
    if cols.is_empty() || !distinct(&cols) || cols.iter().any(|&c| c >= t.columns) {
        return None;
    }
    let mut found = Vec::new();
    match &t.key {
        Key::IntegerPrimaryKey(k) if cols == [*k] => found.push(0),
        Key::WithoutRowid(pk) if same_set(pk, &cols) => found.push(0),
        _ => {},
    }
    for (k, i) in t.indexes.iter().enumerate() {
        let ic: Vec<usize> = i.columns.iter().map(|&(c, _)| c).collect();
        // An index holding the INTEGER PRIMARY KEY names it by its column
        // number, the target by the rowid's: they never match.
        if i.unique && same_set(&ic, &cols) && !ic.iter().any(|&c| t.key == Key::IntegerPrimaryKey(c)) {
            found.push(1 + k);
        }
    }
    let coll_ok = target.iter().all(|&(c, k)| {
        k.is_none_or(|k| match found.first() {
            Some(&n) if n > 0 => crate::model::index_coll(t, &t.indexes[n - 1], c) == k,
            _ => column_coll(t, c) == k,
        })
    });
    (found.len() == 1 && coll_ok).then(|| found[0])
}

/// Where row `r` goes in `t`'s rows.
fn insert_at(t: &mut Table, r: Row, enc: Encoding) {
    let at = t.rows.partition_point(|o| key_order(t, o, &r, enc) == Ordering::Less);
    t.rows.insert(at, r);
}

/// An INSERT row by row, in statement order: each row built as
/// [`inserted_rows`] builds it (a new rowid one above the largest there
/// when it is made, before any row is deleted), then, when it conflicts
/// with rows there: skipped (`OR IGNORE`), the rows deleted first (`OR
/// REPLACE`, `REPLACE INTO`), or what the ON CONFLICT clause that applies
/// says (DO NOTHING, or DO UPDATE of the row there, when its WHERE holds);
/// else it fails. The model leaves out (`None`) a row that conflicts with
/// two rows under UPSERT, or by two constraints one of which a clause
/// names (which SQLite checks first would matter), a CHECK that fails (OR
/// IGNORE would skip it). The deletions and updates are `events` for the
/// foreign key actions; the rows written counted.
fn insert_each(m: &mut Model, ins: &Insert, events: &mut Vec<(Row, Option<Row>)>, depth: usize) -> Option<usize> {
    let enc = m.encoding;
    let k = ins.table;
    let t = m.tables.get(k)?;
    if !insert_shape_ok(t, ins) || ins.rows.iter().flatten().any(crate::model::reads_as_now) || (ins.or.is_some() && !ins.upsert.is_empty()) || !returning_ok(t, &ins.returning) {
        return None;
    }
    // Each clause but the last names its constraint, a distinct one; DO
    // UPDATE sets columns that are not key nor generated.
    let mut targets: Vec<Option<usize>> = Vec::new();
    for (n, u) in ins.upsert.iter().enumerate() {
        let target = match &u.target {
            Some(cs) => Some(upsert_target(t, cs)?),
            None if n + 1 == ins.upsert.len() => None,
            None => return None,
        };
        if target.is_some() && targets.contains(&target) {
            return None;
        }
        targets.push(target);
        if let Some((sets, p)) = &u.update {
            let cols: Vec<usize> = sets.iter().map(|(c, _)| *c).collect();
            let ok = !cols.is_empty()
                && distinct(&cols)
                && sets.iter().all(|(c, e)| {
                    *c < t.columns
                        && !is_key(t, *c)
                        && t.generated[*c].is_none()
                        && match e {
                            Excluded::Yes => true,
                            Excluded::No(v) => convert(t, *c, v).is_some() && !crate::model::reads_as_now(v),
                        }
                })
                && p.as_ref().is_none_or(|p| pred_valid(t, p));
            if !ok {
                return None;
            }
        }
    }
    let ipk = match t.key {
        Key::IntegerPrimaryKey(c) => Some(c),
        _ => None,
    };
    let mut n = 0;
    let mut fired = Fired { outer_or: ins.or, outer_upsert: !ins.upsert.is_empty(), ..Fired::default() };
    for r in &ins.rows {
        let t = &m.tables[k];
        let (given, vals) = if ins.rowid { (Some(&r[0]), &r[1..]) } else { (None, &r[..]) };
        let mut row = Row { rowid: 0, values: t.default.iter().map(|d| d.clone().unwrap_or(Value::Null)).collect() };
        let mut given = given.cloned();
        for (&c, v) in ins.columns.iter().zip(vals) {
            if Some(c) == ipk {
                given = Some(v.clone());
            } else {
                row.values[c] = convert(t, c, v)?;
            }
        }
        match &t.key {
            Key::WithoutRowid(pk) => {
                if pk.iter().any(|&c| row.values[c] == Value::Null) {
                    return None;
                }
            },
            _ => {
                row.rowid = match given {
                    None | Some(Value::Null) => match t.rows.last().map(|r| r.rowid) {
                        Some(i64::MAX) => return None,
                        Some(m) => m + 1,
                        None => 1,
                    },
                    Some(Value::Int(i)) => i,
                    Some(_) => return None,
                };
                if ipk.is_some_and(|c| t.not_null[c]) && matches!(given, None | Some(Value::Null)) {
                    return None;
                }
            },
        }
        regenerate(t, &mut row);
        if fire(m, k, 0, true, None, Some(&row), &[], depth, &mut fired)? {
            continue;
        }
        let t = &m.tables[k];
        if !checks_hold(t, std::slice::from_ref(&row), enc) {
            return None;
        }
        let hits = conflicts(t, &row, None, enc);
        if hits.is_empty() {
            insert_at(&mut m.tables[k], row.clone(), enc);
            n += 1;
            fire(m, k, 0, false, None, Some(&row), &[], depth, &mut fired)?;
            continue;
        }
        if !ins.upsert.is_empty() {
            let pos = hits[0].1;
            if hits.iter().any(|&(_, p)| p != pos) {
                return None;
            }
            let cons: Vec<usize> = hits.iter().map(|&(c, _)| c).collect();
            let named = targets.iter().position(|x| x.is_some_and(|c| cons.contains(&c)));
            let clause = match named {
                Some(j) if cons.iter().all(|&c| targets[j] == Some(c)) => j,
                Some(_) => return None,
                None => targets.iter().position(Option::is_none)?,
            };
            let Some((sets, p)) = &ins.upsert[clause].update else {
                continue;
            };
            let old = t.rows[pos].clone();
            if p.as_ref().is_some_and(|p| eval(t, &old, p, enc) != Some(true)) {
                continue;
            }
            let mut new = old.clone();
            for (c, e) in sets {
                new.values[*c] = match e {
                    Excluded::Yes => row.values[*c].clone(),
                    Excluded::No(v) => convert(t, *c, v)?,
                };
            }
            regenerate(t, &mut new);
            if !conflicts(t, &new, Some(pos), enc).is_empty() || !checks_hold(t, std::slice::from_ref(&new), enc) {
                return None;
            }
            let cols: Vec<usize> = sets.iter().map(|(c, _)| *c).collect();
            // SQLite runs DO UPDATE as an UPDATE with ABORT
            // (`sqlite3UpsertDoUpdate`), which overrides the conflict
            // resolution of its triggers' steps and of the tables they
            // write (a key's own `ON CONFLICT REPLACE` included).
            fired.outer_or = Some(Resolve::Abort);
            if fire(m, k, 1, true, Some(&old), Some(&new), &cols, depth, &mut fired)? {
                fired.outer_or = ins.or;
                continue;
            }
            m.tables[k].rows[pos] = new.clone();
            events.push((old.clone(), Some(new.clone())));
            n += 1;
            fire(m, k, 1, false, Some(&old), Some(&new), &cols, depth, &mut fired)?;
            fired.outer_or = ins.or;
            continue;
        }
        // Each constraint's resolution: the statement's, else the key's
        // own for the key, else ABORT; all the same, or the order SQLite
        // checks them in would matter.
        let resolution = |c: usize| ins.or.or(if c == 0 && t.key != Key::Rowid { t.key_conflict } else { None });
        let kind = |r: Option<Resolve>| match r {
            Some(Resolve::Ignore) => 1,
            Some(r) if r.replaces() => 2,
            _ => 0,
        };
        let first = resolution(hits[0].0);
        if hits.iter().any(|&(c, _)| kind(resolution(c)) != kind(first)) {
            return None;
        }
        match first {
            Some(Resolve::Ignore) => continue,
            Some(r) if r.replaces() => {
                let mut gone: Vec<usize> = hits.iter().map(|&(_, p)| p).collect();
                gone.sort_unstable();
                gone.dedup();
                // DELETE triggers fire with recursive triggers on, for one
                // row only (which SQLite deletes first would matter).
                let triggered = m.recursive_triggers && m.rules.iter().any(|g| g.table == k && g.event == 2);
                if triggered && gone.len() > 1 {
                    return None;
                }
                // Their steps run with REPLACE (`sqlite3GenerateRowDelete`
                // with OE_Replace), also when the key's own ON CONFLICT
                // REPLACE is what replaces.
                fired.outer_or = Some(Resolve::Replace);
                for &p in gone.iter().rev() {
                    let old = m.tables[k].rows[p].clone();
                    if triggered && fire(m, k, 2, true, Some(&old), None, &[], depth, &mut fired)? {
                        return None;
                    }
                    m.tables[k].rows.remove(p);
                    events.push((old.clone(), None));
                    if triggered {
                        fire(m, k, 2, false, Some(&old), None, &[], depth, &mut fired)?;
                    }
                }
                fired.outer_or = ins.or;
                insert_at(&mut m.tables[k], row.clone(), enc);
                n += 1;
                fire(m, k, 0, false, None, Some(&row), &[], depth, &mut fired)?;
            },
            _ => return None,
        }
    }
    renumber(&mut m.tables[k]);
    Some(n)
}

/// `UPDATE OR IGNORE|REPLACE`: as a plain UPDATE when nothing conflicts;
/// else only of one row (with more, which row SQLite updates first would
/// matter), which is left as it is (IGNORE), or updated once the rows it
/// conflicts with are deleted (REPLACE). A CHECK that fails is left out.
fn update_each(m: &mut Model, u: &Update, events: &mut Vec<(Row, Option<Row>)>, depth: usize) -> Option<usize> {
    let enc = m.encoding;
    let k = u.table;
    let t = m.tables.get(k)?;
    let cols: Vec<usize> = u.sets.iter().map(|(c, _)| *c).collect();
    let shape = !cols.is_empty()
        && distinct(&cols)
        && u.sets.iter().all(|(c, v)| *c < t.columns && !is_key(t, *c) && t.generated[*c].is_none() && convert(t, *c, v).is_some() && !crate::model::reads_as_now(v))
        && u.filter.as_ref().is_none_or(|p| pred_valid(t, p))
        && returning_ok(t, &u.returning);
    if !shape {
        return None;
    }
    let hits: Vec<usize> = (0..t.rows.len()).filter(|&i| selects(t, &t.rows[i], &u.filter, enc)).collect();
    let after = updated_rows(t, u, enc)?;
    // Row by row with UPDATE triggers: each row's BEFORE UPDATE triggers
    // (a RAISE(IGNORE) leaves it as it is), its update, its AFTER UPDATE
    // triggers; nothing may conflict. With several rows, as for DELETE.
    if m.rules.iter().any(|g| !g.view && g.table == k && g.event == 1) {
        let mut fired = Fired { outer_or: u.or, ..Fired::default() };
        let mut n = 0;
        for &i in &hits {
            let (old, new) = (m.tables[k].rows[i].clone(), after[i].clone());
            if fire(m, k, 1, true, Some(&old), Some(&new), &cols, depth, &mut fired)? {
                continue;
            }
            m.tables[k].rows[i] = new.clone();
            n += 1;
            events.push((old.clone(), Some(new.clone())));
            fire(m, k, 1, false, Some(&old), Some(&new), &cols, depth, &mut fired)?;
        }
        let t = &m.tables[k];
        if (hits.len() > 1 && (fired.writers.len() > 1 || fired.row_terms)) || !unique_rows(t, &t.rows, enc) || !checks_hold(t, &t.rows, enc) {
            return None;
        }
        return Some(n);
    }
    if !checks_hold(t, &after, enc) {
        return None;
    }
    if unique_rows(t, &after, enc) {
        events.extend(hits.iter().map(|&i| (t.rows[i].clone(), Some(after[i].clone()))));
        m.tables[k].rows = after;
        return Some(hits.len());
    }
    let [p] = hits[..] else {
        return None;
    };
    if m.recursive_triggers && m.rules.iter().any(|g| g.table == k && g.event == 2) {
        return None;
    }
    match u.or {
        Some(Resolve::Ignore) => Some(0),
        Some(Resolve::Replace) => {
            let (old, new) = (t.rows[p].clone(), after[p].clone());
            let mut gone: Vec<usize> = conflicts(t, &new, Some(p), enc).into_iter().map(|(_, q)| q).collect();
            gone.sort_unstable();
            gone.dedup();
            let t = &mut m.tables[k];
            for &q in gone.iter().rev() {
                let o = t.rows.remove(q);
                events.push((o, None));
            }
            let at = t.rows.iter().position(|r| key_order(t, r, &old, enc) == Ordering::Equal)?;
            t.rows[at] = new.clone();
            renumber(t);
            events.push((old, Some(new)));
            Some(1)
        },
        _ => None,
    }
}

/// A write modelled row by row ([`row_by_row`]), then its foreign key
/// actions when foreign keys check the table; `None` when it fails or the
/// model leaves it out.
fn write_rows(m: &mut Model, s: &Statement) -> Option<Effect> {
    reset_work();
    let k = written_table(s)?;
    if k >= m.tables.len() {
        return None;
    }
    let fk = fk_relevant(m, k);
    if fk && has_rules(m, k) {
        return None;
    }
    let mut events = Vec::new();
    let n = exec(m, s, 0, &mut events)?;
    if fk {
        fk_actions(m, k, events)?;
    }
    Some(Effect { rows: None, written: n })
}

// ------------------------------------------------------------- triggers

/// The most triggers with bodies statements create, and steps per body.
pub const MAX_RULES: usize = 6;
pub const MAX_STEPS: usize = 3;

/// The most work ([`exec`]'s cost: statements, and the rows of the tables
/// they write) one INSERT, UPDATE or DELETE may take with the statements
/// its triggers run, nested. Triggers fire no cycle, but a statement of
/// many rows whose triggers write tables of many rows, each firing
/// triggers of its own, multiplies: past this the model leaves the
/// statement out (run 2000-writes: a DELETE of a view over 294 rows, its
/// INSTEAD OF trigger updating them all, each update's trigger writing a
/// table of 272 rows three times, never finished modelling).
pub const MAX_WORK: usize = 20_000;

thread_local! {
    static WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A new top level statement's budget ([`MAX_WORK`]).
fn reset_work() {
    WORK.with(|w| w.set(0));
}

/// Spend `cost` of the budget; false once it is gone.
fn spend(cost: usize) -> bool {
    WORK.with(|w| {
        let n = w.get().saturating_add(cost);
        w.set(n);
        n <= MAX_WORK
    })
}

/// The id the next trigger takes: one above the largest.
pub fn next_rule_id(m: &Model) -> usize {
    m.rules.iter().map(|g| g.id + 1).max().unwrap_or(0)
}

/// The tables trigger `g`'s steps write.
/// Whether trigger `g` names its table's schema (main for a view's) and,
/// unless TEMP, writes only tables of that schema: the body of a trigger
/// that is not TEMP resolves its tables in the trigger's own schema
/// (`fixSelectCb` fixes every source's schema, `sqlite3TriggerStepSrc`
/// every step's target), so another schema's table is "no such table"
/// when it fires; a TEMP trigger's resolve as any statement's do.
pub fn rule_dbs_ok(m: &Model, g: &Rule) -> bool {
    let on = if g.view { Some(Db::Main) } else { m.tables.get(g.table).map(|t| t.db) };
    on == Some(g.db) && (g.temp || rule_targets(g).all(|b| m.tables.get(b).is_some_and(|t| t.db == g.db)))
}

/// Whether `DETACH DATABASE aux` may run: its tables are the last ones, so
/// that the others keep their numbers, and no TEMP trigger is on one or
/// writes one (SQLite would keep such a trigger, orphaned).
pub fn detachable(m: &Model) -> bool {
    let first = m.tables.iter().position(|t| t.db == Db::Aux).unwrap_or(m.tables.len());
    m.tables[first..].iter().all(|t| t.db == Db::Aux)
        && m.rules.iter().all(|g| !g.temp || ((g.view || m.tables.get(g.table).is_some_and(|t| t.db != Db::Aux)) && rule_targets(g).all(|b| m.tables.get(b).is_some_and(|t| t.db != Db::Aux))))
}

/// Whether a rollback to savepoint `k` (0 for ROLLBACK) can undo what it
/// must: after `PRAGMA journal_mode = OFF` (ever) an in-memory database
/// keeps no journal, so never with a table outside main now or at the
/// savepoint.
fn memory_rollback_ok(m: &Model, k: usize) -> bool {
    !m.session.memory_off || (!m.session.memory_main && m.tables.iter().all(|t| t.db == Db::Main) && m.savepoints.get(k).is_none_or(|s| s.tables.iter().all(|t| t.db == Db::Main)))
}

fn rule_targets(g: &Rule) -> impl Iterator<Item = usize> + '_ {
    g.body.iter().filter_map(|s| match s {
        Step::Insert { table, .. } | Step::Update { table, .. } | Step::Delete { table, .. } => Some(*table),
        _ => None,
    })
}

/// Whether table `from` reaches table `to` by triggers (a trigger on a
/// table to the tables its steps write), `from` itself included.
fn rule_reaches(m: &Model, from: usize, to: usize) -> bool {
    let mut seen = vec![false; m.tables.len()];
    let mut stack = vec![from];
    while let Some(t) = stack.pop() {
        if t == to {
            return true;
        }
        if t < seen.len() && !std::mem::replace(&mut seen[t], true) {
            for g in m.rules.iter().filter(|g| !g.view && g.table == t) {
                stack.extend(rule_targets(g));
            }
        }
    }
    false
}

/// The columns of table `k` trigger `g` names (of its own table, or of a
/// table a step writes).
pub fn rule_columns(g: &Rule, k: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let own = !g.view && g.table == k;
    if own {
        out.extend(&g.of);
        if let Some(p) = &g.when {
            p.leaves(&mut out);
        }
    }
    for s in &g.body {
        let terms: Vec<&Term> = match s {
            Step::Insert { values, .. } => values.iter().collect(),
            Step::Update { sets, .. } => sets.iter().map(|(_, t)| t).collect(),
            _ => Vec::new(),
        };
        if own {
            out.extend(terms.iter().filter_map(|t| if let Term::Row(c) = t { Some(*c) } else { None }));
            match s {
                Step::Abort(p) => p.leaves(&mut out),
                Step::Select(c) => out.push(*c),
                _ => {},
            }
        }
        match s {
            Step::Insert { table, columns, upsert, .. } if *table == k => {
                out.extend(columns);
                for u in upsert {
                    out.extend(u.target.iter().flatten().map(|&(c, _)| c));
                    if let Some((sets, p)) = &u.update {
                        out.extend(sets.iter().map(|(c, _)| *c));
                        if let Some(p) = p {
                            p.leaves(&mut out);
                        }
                    }
                }
            },
            Step::Update { table, sets, filter, .. } if *table == k => {
                out.extend(sets.iter().map(|(c, _)| *c));
                if let Some(p) = filter {
                    p.leaves(&mut out);
                }
            },
            Step::Delete { table, filter: Some(p) } if *table == k => p.leaves(&mut out),
            _ => {},
        }
    }
    out
}

/// Trigger `g` with each column `c` of table `k` as `f(c)`.
fn map_rule_columns(g: &mut Rule, k: usize, f: &dyn Fn(usize) -> usize) {
    let own = !g.view && g.table == k;
    if own {
        g.of.iter_mut().for_each(|c| *c = f(*c));
        g.when = g.when.as_ref().map(|p| map_pred(p, f));
    }
    let term = |t: &mut Term| {
        if let Term::Row(c) = t {
            if own {
                *c = f(*c);
            }
        }
    };
    for s in &mut g.body {
        match s {
            Step::Insert { table, columns, values, upsert, .. } => {
                values.iter_mut().for_each(term);
                if *table == k {
                    columns.iter_mut().for_each(|c| *c = f(*c));
                    for u in upsert {
                        u.target.iter_mut().flatten().for_each(|(c, _)| *c = f(*c));
                        if let Some((sets, p)) = &mut u.update {
                            sets.iter_mut().for_each(|(c, _)| *c = f(*c));
                            *p = p.as_ref().map(|p| map_pred(p, f));
                        }
                    }
                }
            },
            Step::Update { table, sets, filter, .. } => {
                sets.iter_mut().for_each(|(_, t)| term(t));
                if *table == k {
                    sets.iter_mut().for_each(|(c, _)| *c = f(*c));
                    *filter = filter.as_ref().map(|p| map_pred(p, f));
                }
            },
            Step::Delete { table, filter } if *table == k => *filter = filter.as_ref().map(|p| map_pred(p, f)),
            Step::Abort(p) if own => *p = map_pred(p, f),
            Step::Select(c) if own => *c = f(*c),
            _ => {},
        }
    }
}

/// Trigger `g` with each table `t` as `f(t)`.
fn map_rule_tables(g: &mut Rule, f: &dyn Fn(usize) -> usize) {
    if !g.view {
        g.table = f(g.table);
    }
    for s in &mut g.body {
        if let Step::Insert { table, .. } | Step::Update { table, .. } | Step::Delete { table, .. } = s {
            *table = f(*table);
        }
    }
}

/// Whether column `c` of `t` may be read off the row a trigger sees: not
/// generated, not the INTEGER PRIMARY KEY (NEW of which BEFORE INSERT
/// does not know yet).
fn row_column_ok(t: &Table, c: usize) -> bool {
    c < t.columns && t.generated[c].is_none() && !matches!(t.key, Key::IntegerPrimaryKey(k) if k == c)
}

/// Whether trigger `g` may be created in `m` (see [`Rule`]): on a table,
/// the only one of its table, event and time; its steps write other
/// tables, columns they have (not key nor generated for UPDATE), which
/// reach its own table by no trigger (so that triggers fire no cycle);
/// its row's columns as [`row_column_ok`].
pub fn rule_ok(m: &Model, g: &Rule) -> bool {
    if !rule_dbs_ok(m, g) {
        return false;
    }
    if g.view {
        return view_rule_ok(m, g);
    }
    let Some(t) = m.tables.get(g.table) else {
        return false;
    };
    let pred_ok = |p: &Pred| {
        let mut cs = Vec::new();
        p.leaves(&mut cs);
        pred_valid(t, p) && cs.iter().all(|&c| row_column_ok(t, c)) && p.depth() <= 3
    };
    let term_ok = |b: &Table, c: usize, x: &Term| match x {
        Term::Row(r) => row_column_ok(t, *r),
        Term::Lit(v) => convert(b, c, v).is_some() && !crate::model::reads_as_now(v),
    };
    let step_ok = |s: &Step| match s {
        Step::Insert { table, columns, values, upsert, .. } => m.tables.get(*table).is_some_and(|b| {
            *table != g.table
                && upsert.len() <= 1
                && upsert.iter().all(|u| {
                    u.target.is_none() && u.update.as_ref().is_none_or(|(sets, p)| sets.iter().all(|(c, _)| *c < b.columns && !is_key(b, *c) && b.generated[*c].is_none()) && p.is_none())
                })
                && !columns.is_empty()
                && distinct(columns)
                && columns.len() == values.len()
                && columns.iter().zip(values).all(|(&c, x)| c < b.columns && b.generated[c].is_none() && term_ok(b, c, x))
        }),
        Step::Update { table, sets, filter, .. } => m.tables.get(*table).is_some_and(|b| {
            let cs: Vec<usize> = sets.iter().map(|(c, _)| *c).collect();
            *table != g.table
                && !cs.is_empty()
                && distinct(&cs)
                && sets.iter().all(|(c, x)| *c < b.columns && !is_key(b, *c) && b.generated[*c].is_none() && term_ok(b, *c, x))
                && filter.as_ref().is_none_or(|p| pred_valid(b, p) && p.depth() <= 3)
        }),
        Step::Delete { table, filter } => {
            m.tables.get(*table).is_some_and(|b| *table != g.table && filter.as_ref().is_none_or(|p| pred_valid(b, p) && p.depth() <= 3))
        },
        Step::Ignore => g.before,
        Step::Abort(p) => pred_ok(p),
        Step::Select(c) => row_column_ok(t, *c),
    };
    g.event <= 2
        && (g.event == 1 || g.of.is_empty())
        && distinct(&g.of)
        && g.of.iter().all(|&c| c < t.columns)
        && g.when.as_ref().is_none_or(pred_ok)
        && !m.rules.iter().any(|o| o.id == g.id || (o.table == g.table && o.event == g.event && o.before == g.before))
        && (1..=MAX_STEPS).contains(&g.body.len())
        && g.body.iter().all(step_ok)
        && rule_targets(g).all(|b| !rule_reaches(m, b, g.table))
}

/// Whether INSTEAD OF trigger `g` may be created on view `v{g.table}`: a
/// view statements may write ([`writable_view`]), the only trigger of its
/// event there, without WHEN, OF or RAISE; its steps as a table
/// trigger's, a value of the row only of a column of the view, and for
/// INSERT and DELETE only (the row an UPDATE sees holds literals SQLite
/// may have converted).
fn view_rule_ok(m: &Model, g: &Rule) -> bool {
    let Some((k, cols)) = writable_view(m, g.table) else {
        return false;
    };
    let t = &m.tables[k];
    let row_ok = |c: usize| g.event != 1 && cols.contains(&c) && row_column_ok(t, c);
    let term_ok = |b: &Table, c: usize, x: &Term| match x {
        Term::Row(r) => row_ok(*r),
        Term::Lit(v) => convert(b, c, v).is_some() && !crate::model::reads_as_now(v),
    };
    let step_ok = |s: &Step| match s {
        Step::Insert { table, columns, values, upsert, .. } => m.tables.get(*table).is_some_and(|b| {
            upsert.is_empty()
                && !columns.is_empty() && distinct(columns) && columns.len() == values.len() && columns.iter().zip(values).all(|(&c, x)| c < b.columns && b.generated[c].is_none() && term_ok(b, c, x))
        }),
        Step::Update { table, sets, filter, .. } => m.tables.get(*table).is_some_and(|b| {
            let cs: Vec<usize> = sets.iter().map(|(c, _)| *c).collect();
            !cs.is_empty()
                && distinct(&cs)
                && sets.iter().all(|(c, x)| *c < b.columns && !is_key(b, *c) && b.generated[*c].is_none() && term_ok(b, *c, x))
                && filter.as_ref().is_none_or(|p| pred_valid(b, p) && p.depth() <= 3)
        }),
        Step::Delete { table, filter } => m.tables.get(*table).is_some_and(|b| filter.as_ref().is_none_or(|p| pred_valid(b, p) && p.depth() <= 3)),
        Step::Select(c) => cols.contains(c),
        Step::Ignore | Step::Abort(_) => false,
    };
    g.event <= 2
        && g.of.is_empty()
        && g.when.is_none()
        && !m.rules.iter().any(|o| o.id == g.id || (o.view && o.table == g.table && o.event == g.event))
        && (1..=MAX_STEPS).contains(&g.body.len())
        && g.body.iter().all(step_ok)
}

/// The triggers of table `k` for `event` at `before`, of an UPDATE of
/// columns `cols`.
fn rules_for(m: &Model, k: usize, event: u8, before: bool, cols: &[usize]) -> Vec<Rule> {
    m.rules.iter().filter(|g| !g.view && g.table == k && g.event == event && g.before == before && (g.of.is_empty() || g.of.iter().any(|c| cols.contains(c)))).cloned().collect()
}

/// What the triggers fired for one row did, for the order rules of a
/// statement of several rows: the triggers whose steps wrote, and whether
/// any wrote a value of the row.
#[derive(Default)]
struct Fired {
    writers: Vec<usize>,
    row_terms: bool,
    /// The firing statement's conflict resolution, and whether it has
    /// ON CONFLICT clauses.
    outer_or: Option<Resolve>,
    outer_upsert: bool,
}

/// Fire the triggers of table `k` for `event` at `before` for one row
/// (`old` and `new` as the event has them), their steps in order, each
/// write a statement of its own ([`exec`], which fires further
/// triggers); `Some(true)` when a RAISE(IGNORE) leaves the row out,
/// `None` when a step fails, a RAISE(ABORT) fires, or a step writes a
/// table foreign keys check (the model leaves those out).
#[allow(clippy::too_many_arguments)]
fn fire(m: &mut Model, k: usize, event: u8, before: bool, old: Option<&Row>, new: Option<&Row>, cols: &[usize], depth: usize, fired: &mut Fired) -> Option<bool> {
    let enc = m.encoding;
    let rules = rules_for(m, k, event, before, cols);
    if rules.is_empty() {
        return Some(false);
    }
    // NEW.c and OLD.c have the column's collating sequence but no
    // affinity (`TK_TRIGGER`): a comparison converts neither side.
    let mut bare = m.tables[k].clone_shape();
    bare.strict = None;
    bare.decl = vec![None; bare.columns];
    for g in rules {
        let row = if event == 2 { old } else { new }?.clone();
        if g.when.as_ref().is_some_and(|p| eval(&bare, &row, p, enc) != Some(true)) {
            continue;
        }
        let t = &m.tables[k];
        let vals: Vec<Value> = (0..t.columns).map(|c| column_value(t, &row, c)).collect();
        if g.body.iter().any(|s| matches!(s, Step::Abort(p) if eval(&bare, &row, p, enc) == Some(true))) {
            return None;
        }
        if run_steps(m, &g, &vals, depth, fired)? {
            return Some(true);
        }
    }
    Some(false)
}

/// A trigger's steps in order, `vals` the values of the row it sees (by
/// column), each write a statement of its own ([`exec`], which fires
/// further triggers); `Some(true)` at a RAISE(IGNORE). A step writing a
/// table foreign keys check is followed by their actions, as any
/// statement is (`write_with_fks`), when neither it nor a table they
/// reach has triggers (the model fires none from an action).
fn run_steps(m: &mut Model, g: &Rule, vals: &[Value], depth: usize, fired: &mut Fired) -> Option<bool> {
    for s in &g.body {
        let value = |x: &Term| match x {
            Term::Lit(v) => v.clone(),
            Term::Row(c) => vals[*c].clone(),
        };
        let st = match s {
            Step::Ignore => return Some(true),
            Step::Abort(_) | Step::Select(_) => continue,
            // The firing statement's conflict resolution, if any, overrides
            // the step's (`codeTriggerProgram`); under UPSERT the model
            // leaves a step's out.
            Step::Insert { or, .. } | Step::Update { or, .. } if or.is_some() && fired.outer_upsert => return None,
            Step::Insert { or: Some(_), upsert, .. } if !upsert.is_empty() => return None,
            Step::Insert { upsert, .. } if !upsert.is_empty() && fired.outer_or.is_some() => return None,
            Step::Insert { table, columns, values, or, upsert } => Statement::Insert(Insert {
                table: *table,
                columns: columns.clone(),
                rows: vec![values.iter().map(value).collect()],
                or: fired.outer_or.or(*or),
                upsert: upsert.clone(),
                ..Default::default()
            }),
            Step::Update { table, sets, filter, or } => Statement::Update(Update {
                table: *table,
                sets: sets.iter().map(|(c, x)| (*c, value(x))).collect(),
                filter: filter.clone(),
                or: fired.outer_or.or(*or).map(|r| if r == Resolve::ReplaceInto { Resolve::Replace } else { r }),
                ..Default::default()
            }),
            Step::Delete { table, filter } => Statement::Delete(crate::statements::Delete { table: *table, filter: filter.clone(), returning: None }),
        };
        if !fired.writers.contains(&g.id) {
            fired.writers.push(g.id);
        }
        fired.row_terms |= match s {
            Step::Insert { values, .. } => values.iter().any(|x| matches!(x, Term::Row(_))),
            Step::Update { sets, .. } => sets.iter().any(|(_, x)| matches!(x, Term::Row(_))),
            _ => false,
        };
        let b = written_table(&st)?;
        let fk = fk_relevant(m, b);
        if fk && fk_children(m, b).into_iter().any(|c| has_rules(m, c)) {
            return None;
        }
        let mut ev = Vec::new();
        exec(m, &st, depth + 1, &mut ev)?;
        if fk {
            fk_actions(m, b, ev)?;
        }
    }
    Some(false)
}

/// The table view `v` selects from and its columns, when it has both and
/// the table has the columns: a view statements may write through its
/// INSTEAD OF triggers.
pub fn writable_view(m: &Model, v: usize) -> Option<(usize, &[usize])> {
    let view = m.views.get(v)?;
    let t = view.table?;
    let table = m.tables.get(t)?;
    // A view of main's (the file's views read main's tables only).
    (table.db == Db::Main && !view.columns.is_empty() && view.columns.iter().all(|&c| c < table.columns)).then_some((t, &view.columns[..]))
}

/// An INSERT, UPDATE or DELETE of view `v` ([`Statement::ViewWrite`]):
/// the INSTEAD OF trigger of its event fired for each row (the inserted
/// ones, in order; the rows of the view's table the filter selects, over
/// the view's columns), the row it sees as the view has it: an inserted
/// value as written (SQLite applies no affinity on a view, but reads an
/// integer as a real in a column of REAL affinity), NULL in a
/// column not listed; an updated one the row's values with the sets'
/// literals. Several rows of an UPDATE or DELETE as for a table.
fn view_write(m: &mut Model, v: usize, s: &Statement) -> Option<usize> {
    reset_work();
    let enc = m.encoding;
    let (k, cols) = writable_view(m, v)?;
    let cols = cols.to_vec();
    let event = match s {
        Statement::Insert(_) => 0,
        Statement::Update(u) if u.from.is_none() => 1,
        Statement::Delete(_) => 2,
        _ => return None,
    };
    let g = m.rules.iter().find(|g| g.view && g.table == v && g.event == event)?.clone();
    let t = &m.tables[k];
    let in_view = |p: &Pred| {
        let mut ls = Vec::new();
        p.leaves(&mut ls);
        pred_valid(t, p) && ls.iter().all(|c| cols.contains(c))
    };
    let mut fired = Fired::default();
    let rows: Vec<Vec<Value>> = match s {
        Statement::Insert(i) => {
            let ok = !i.rowid
                && !i.columns.is_empty()
                && distinct(&i.columns)
                && i.columns.iter().all(|c| cols.contains(c))
                && !i.rows.is_empty()
                && i.rows.iter().all(|r| r.len() == i.columns.len() && r.iter().all(|v| sql_safe(v) && !crate::model::reads_as_now(v)))
                && i.or.is_none()
                && i.upsert.is_empty()
                && i.returning.is_none();
            if !ok {
                return None;
            }
            i.rows
                .iter()
                .map(|r| {
                    let mut vals = vec![Value::Null; t.columns];
                    for (&c, v) in i.columns.iter().zip(r) {
                        vals[c] = match v {
                            // NEW.c of a column of REAL affinity reads an
                            // integer as a real (`OP_RealAffinity` on a
                            // TK_TRIGGER column), the view's column having
                            // its table column's affinity.
                            Value::Int(n) if affinity(t, c) == Affinity::Real => Value::Real(*n as f64),
                            _ => v.clone(),
                        };
                    }
                    vals
                })
                .collect()
        },
        Statement::Update(u) => {
            let sc: Vec<usize> = u.sets.iter().map(|(c, _)| *c).collect();
            let ok = !sc.is_empty()
                && distinct(&sc)
                && u.sets.iter().all(|(c, v)| cols.contains(c) && sql_safe(v) && !crate::model::reads_as_now(v))
                && u.filter.as_ref().is_none_or(in_view)
                && u.or.is_none()
                && u.returning.is_none();
            if !ok {
                return None;
            }
            t.rows
                .iter()
                .filter(|r| selects(t, r, &u.filter, enc))
                .map(|r| {
                    let mut vals: Vec<Value> = (0..t.columns).map(|c| column_value(t, r, c)).collect();
                    for (c, v) in &u.sets {
                        vals[*c] = v.clone();
                    }
                    vals
                })
                .collect()
        },
        Statement::Delete(d) => {
            if !d.filter.as_ref().is_none_or(in_view) || d.returning.is_some() {
                return None;
            }
            t.rows.iter().filter(|r| selects(t, r, &d.filter, enc)).map(|r| (0..t.columns).map(|c| column_value(t, r, c)).collect()).collect()
        },
        _ => return None,
    };
    for vals in &rows {
        if run_steps(m, &g, vals, 1, &mut fired)? {
            return None;
        }
    }
    if event != 0 && rows.len() > 1 && fired.row_terms {
        return None;
    }
    Some(rows.len())
}

/// Table `k` and the tables foreign keys make its children, theirs, and so
/// on: the tables its foreign key actions may write.
fn fk_children(m: &Model, k: usize) -> Vec<usize> {
    let mut out = vec![k];
    let mut at = 0;
    while at < out.len() {
        let p = out[at];
        for (c, t) in m.tables.iter().enumerate() {
            if !out.contains(&c) && t.fks.iter().any(|f| f.parent == p) {
                out.push(c);
            }
        }
        at += 1;
    }
    out
}

/// Whether table `k` has triggers with bodies.
fn has_rules(m: &Model, k: usize) -> bool {
    m.rules.iter().any(|g| !g.view && g.table == k)
}

/// An INSERT, UPDATE or DELETE row by row, firing triggers; `events` the
/// rows deleted and updated (for foreign key actions).
fn exec(m: &mut Model, s: &Statement, depth: usize, events: &mut Vec<(Row, Option<Row>)>) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    // Each statement costs one plus the rows of the table it writes (it
    // reads them all, for its filter or its constraint checks).
    let cost = 1 + written_table(s).and_then(|k| m.tables.get(k)).map_or(0, |t| t.rows.len());
    if !spend(cost) {
        return None;
    }
    match s {
        Statement::Insert(i) => insert_each(m, i, events, depth),
        Statement::Update(u) => update_each(m, u, events, depth),
        Statement::Delete(d) => delete_each(m, d, events, depth),
        _ => None,
    }
}

/// A DELETE row by row: each row its filter selects (on the rows before
/// it), its BEFORE DELETE triggers (a RAISE(IGNORE) keeps it), then
/// deleted, then its AFTER DELETE triggers. With several rows, the order
/// SQLite takes them in must not matter: at most one trigger writes, and
/// only literals (each row then runs the same steps).
fn delete_each(m: &mut Model, d: &crate::statements::Delete, events: &mut Vec<(Row, Option<Row>)>, depth: usize) -> Option<usize> {
    let enc = m.encoding;
    let k = d.table;
    let t = m.tables.get(k)?;
    if !returning_ok(t, &d.returning) || !d.filter.as_ref().is_none_or(|p| pred_valid(t, p)) {
        return None;
    }
    let hits: Vec<Row> = t.rows.iter().filter(|r| selects(t, r, &d.filter, enc)).cloned().collect();
    let mut fired = Fired::default();
    let mut gone = Vec::new();
    for r in &hits {
        if fire(m, k, 2, true, Some(r), None, &[], depth, &mut fired)? {
            continue;
        }
        gone.push(r.clone());
        fire(m, k, 2, false, Some(r), None, &[], depth, &mut fired)?;
    }
    if hits.len() > 1 && (fired.writers.len() > 1 || fired.row_terms) {
        return None;
    }
    let t = &mut m.tables[k];
    let mut rows = std::mem::take(&mut t.rows);
    rows.retain(|r| !gone.iter().any(|g| key_order(t, g, r, enc) == Ordering::Equal));
    t.rows = rows;
    renumber(t);
    let n = gone.len();
    events.extend(gone.into_iter().map(|r| (r, None)));
    Some(n)
}

/// Apply a statement [`valid`] against `m` to it.
pub fn apply(m: &mut Model, s: &Statement) -> Effect {
    if let Statement::Update(u) = s {
        if u.from.is_some() {
            let plain = resolve_from(m, u).expect("a valid UPDATE FROM");
            return apply(m, &Statement::Update(plain));
        }
    }
    let enc = m.encoding;
    if row_by_row(m, s) {
        return write_rows(m, s).expect("a valid write");
    }
    if written_table(s).is_some_and(|k| fk_relevant(m, k)) {
        return write_with_fks(m, s).expect("a valid write");
    }
    match s {
        Statement::Select(s) => {
            let t = &m.tables[s.table];
            let cols: Vec<usize> = if s.columns.is_empty() { (0..t.columns).collect() } else { s.columns.clone() };
            let mut hits: Vec<&Row> = t.rows.iter().filter(|r| selects(t, r, &s.filter, enc)).collect();
            let key = |r: &Row| -> Vec<Value> { s.order.iter().map(|&(c, _)| column_value(t, r, c)).collect() };
            let order = |a: &[Value], b: &[Value]| {
                for (k, &(c, desc)) in s.order.iter().enumerate() {
                    let o = compare_coll(&a[k], &b[k], enc, column_coll(t, c));
                    if o != Ordering::Equal {
                        return if desc { o.reverse() } else { o };
                    }
                }
                Ordering::Equal
            };
            let project = |r: &Row| -> Vec<Value> { cols.iter().map(|&c| column_value(t, r, c)).collect() };
            let mut groups: Vec<Vec<Vec<Value>>> = Vec::new();
            if s.order.is_empty() {
                groups.push(hits.iter().map(|r| project(r)).collect());
            } else {
                hits.sort_by(|a, b| order(&key(a), &key(b)));
                let mut last: Option<Vec<Value>> = None;
                for r in hits {
                    let k = key(r);
                    if last.as_ref().is_none_or(|l| order(l, &k) != Ordering::Equal) {
                        groups.push(Vec::new());
                    }
                    groups.last_mut().unwrap().push(project(r));
                    last = Some(k);
                }
            }
            Effect { rows: Some(Expected { groups, limit: s.limit.map(|n| n as usize) }), written: 0 }
        },
        Statement::Insert(ins) => {
            let t = &mut m.tables[ins.table];
            let new = inserted_rows(t, ins, enc).expect("a valid INSERT");
            let n = new.len();
            // Sorted against the table, its rows taken out (no clone of it).
            let mut rows = std::mem::take(&mut t.rows);
            rows.extend(new);
            rows.sort_by(|a, b| key_order(t, a, b, enc));
            t.rows = rows;
            renumber(t);
            Effect { rows: None, written: n }
        },
        Statement::Update(u) => {
            let t = &mut m.tables[u.table];
            let sets: Vec<(usize, Value)> = u.sets.iter().map(|(c, v)| (*c, convert(t, *c, v).expect("a valid UPDATE"))).collect();
            let hit: Vec<bool> = t.rows.iter().map(|r| selects(t, r, &u.filter, enc)).collect();
            let mut n = 0;
            let shape = t.generated.iter().any(Option::is_some).then(|| t.clone_shape());
            for (r, h) in t.rows.iter_mut().zip(hit) {
                if h {
                    n += 1;
                    for (c, v) in &sets {
                        r.values[*c] = v.clone();
                    }
                    if let Some(shape) = &shape {
                        regenerate(shape, r);
                    }
                }
            }
            Effect { rows: None, written: n }
        },
        Statement::Delete(d) => {
            let t = &mut m.tables[d.table];
            let hit: Vec<bool> = t.rows.iter().map(|r| selects(t, r, &d.filter, enc)).collect();
            let before = t.rows.len();
            let mut it = hit.into_iter();
            t.rows.retain(|_| !it.next().unwrap());
            renumber(t);
            Effect { rows: None, written: before - t.rows.len() }
        },
        // VACUUM, the auto-vacuum mode changed or not, renumbers the same.
        Statement::Vacuum | Statement::AutoVacuum(_) => {
            // VACUUM copies a rowid table without an INTEGER PRIMARY KEY
            // and without indexes with new rowids, 1, 2, … in rowid order
            // (`xferOptimization` uses OP_NewRowid there); any other table
            // keeps its rowids.
            // Of main only: VACUUM rebuilds the main database.
            for t in &mut m.tables {
                if t.key == Key::Rowid && t.indexes.is_empty() && t.db == Db::Main {
                    for (k, r) in t.rows.iter_mut().enumerate() {
                        r.rowid = k as i64 + 1;
                    }
                }
            }
            Effect { rows: None, written: 0 }
        },
        // ANALYZE writes only `sqlite_stat1` and `sqlite_stat4`, which no
        // statement or query reads; the statistics change plans, never
        // results.
        Statement::IncrementalVacuum(_)
        | Statement::CacheSize(_)
        | Statement::Analyze(_)
        | Statement::AnalysisLimit(_)
        | Statement::Stats(_)
        | Statement::Optimize(_)
        | Statement::AutomaticIndex(_)
        | Statement::ReverseUnordered(_) => {
            Effect { rows: None, written: 0 }
        },
        Statement::Savepoint(_) | Statement::Begin(_) => {
            let snapshot = Model {
                encoding: m.encoding,
                tables: m.tables.clone(),
                views: m.views.clone(),
                triggers: m.triggers.clone(),
                savepoints: Vec::new(),
                foreign_keys: m.foreign_keys,
                rules: m.rules.clone(),
                recursive_triggers: m.recursive_triggers,
                session: m.session,
                vtabs: m.vtabs.clone(),
            };
            m.savepoints.push(snapshot);
            if matches!(s, Statement::Begin(_)) {
                m.session.begun = true;
            }
            Effect { rows: None, written: 0 }
        },
        Statement::RollbackTo(k) => {
            m.savepoints.truncate(k + 1);
            let saved = m.savepoints[*k].clone();
            m.tables = saved.tables;
            m.views = saved.views;
            m.triggers = saved.triggers;
            m.rules = saved.rules;
            m.vtabs = saved.vtabs;
            Effect { rows: None, written: 0 }
        },
        Statement::Release(k) => {
            m.savepoints.truncate(*k);
            Effect { rows: None, written: 0 }
        },
        Statement::Commit => {
            m.savepoints.clear();
            m.session.begun = false;
            Effect { rows: None, written: 0 }
        },
        // WAL is refused in memory, where the mode stays: OFF stays off.
        Statement::JournalMode(j) => {
            m.session.memory_off |= *j == Journal::Off;
            if *j != Journal::Wal {
                m.session.journal_off = *j == Journal::Off;
            }
            m.session.wal = *j == Journal::Wal;
            Effect { rows: None, written: 0 }
        },
        Statement::LockingMode(x) => {
            m.session.exclusive = *x;
            Effect { rows: None, written: 0 }
        },
        Statement::WalCheckpoint(_)
        | Statement::SecureDelete(_)
        | Statement::CellSizeCheck(_)
        | Statement::MmapSize(_)
        | Statement::TempStore(_)
        | Statement::JournalSizeLimit(_)
        | Statement::WalAutocheckpoint(_)
        | Statement::CacheSpill(_)
        | Statement::VacuumInto(_)
        | Statement::Synchronous(_)
        | Statement::ShrinkMemory
        | Statement::AnalyzeDb(_)
        | Statement::DbPragma { .. } => Effect { rows: None, written: 0 },
        Statement::Virtual { s, .. } => {
            crate::vtabs::apply(&mut m.vtabs, s);
            Effect { rows: crate::vtabs::answer(s).map(|r| Expected { groups: vec![r], limit: None }), written: 0 }
        },
        Statement::Attach(_) => {
            m.session.aux = true;
            Effect { rows: None, written: 0 }
        },
        // Its tables (the last ones) and their triggers go with it.
        Statement::Detach => {
            m.session.aux = false;
            m.tables.retain(|t| t.db != Db::Aux);
            m.rules.retain(|g| g.temp || g.db != Db::Aux);
            Effect { rows: None, written: 0 }
        },
        Statement::Rollback => {
            let saved = m.savepoints.swap_remove(0);
            m.savepoints.clear();
            m.session.begun = false;
            m.tables = saved.tables;
            m.views = saved.views;
            m.triggers = saved.triggers;
            m.rules = saved.rules;
            m.vtabs = saved.vtabs;
            Effect { rows: None, written: 0 }
        },
        // Every database the statements leave is valid (the property checks
        // it after every write), so the check answers `ok`.
        Statement::Check { .. } => {
            Effect { rows: Some(Expected { groups: vec![vec![vec![Value::Text("ok".into())]]], limit: None }), written: 0 }
        },
        Statement::CreateTable { table, def } => {
            m.tables.push(Table { names: crate::writer::table_names(def, Some(&format!("t{table}"))), ..def.clone() });
            Effect { rows: None, written: 0 }
        },
        // The table's triggers go with it. A view on it stays in the schema
        // (SQLite does not check views when a table is dropped) and is never
        // queried.
        Statement::DropTable(k) => {
            m.tables.pop();
            m.triggers.retain(|g| g.table != *k);
            m.rules.retain(|g| g.view || g.table != *k);
            Effect { rows: None, written: 0 }
        },
        Statement::CreateIndex { table, def, .. } => {
            m.tables[*table].indexes.push(def.clone());
            Effect { rows: None, written: 0 }
        },
        Statement::DropIndex { table, .. } => {
            m.tables[*table].indexes.pop();
            Effect { rows: None, written: 0 }
        },
        Statement::Count { table, filter } => {
            let t = &m.tables[*table];
            let n = t.rows.iter().filter(|r| selects(t, r, filter, enc)).count();
            Effect { rows: Some(Expected { groups: vec![vec![vec![Value::Int(n as i64)]]], limit: None }), written: 0 }
        },
        Statement::AddColumn { table, def, .. } => {
            let t = &mut m.tables[*table];
            let v = added_default(t, def).expect("a valid ADD COLUMN");
            t.columns += 1;
            if let Some(types) = &mut t.strict {
                types.push(def.ty.expect("a STRICT type"));
            }
            t.decl.push(def.decl);
            t.coll.push(def.coll);
            t.not_null.push(def.not_null);
            for r in &mut t.rows {
                r.values.push(v.clone());
            }
            t.default.push((v != Value::Null).then_some(v));
            t.fks.extend(def.references.clone());
            t.generated.push(def.generated.map(|(src, _)| src));
            // Its named constraints after the other columns', before the
            // table constraints.
            let at = t.names.iter().position(|n| n.column.is_none()).unwrap_or(t.names.len());
            let added = crate::statements::added_names(*table, t.columns - 1, def);
            t.names.splice(at..at, added);
            if def.generated.is_some() {
                let shape = t.clone_shape();
                for r in &mut t.rows {
                    regenerate(&shape, r);
                }
            }
            Effect { rows: None, written: 0 }
        },
        Statement::DropColumn { table, column, .. } => {
            drop_column(m, *table, *column);
            Effect { rows: None, written: 0 }
        },
        Statement::RenameColumn { .. } => Effect { rows: None, written: 0 },
        Statement::NotNull { table, column, on } => {
            let t = &mut m.tables[*table];
            t.not_null[*column] = *on;
            // DROP NOT NULL drops a named one's name with it.
            t.names.retain(|n| *on || n.column != Some(*column) || n.cons != Cons::NotNull);
            Effect { rows: None, written: 0 }
        },
        Statement::ForeignKeys(on) => {
            m.foreign_keys = *on;
            Effect { rows: None, written: 0 }
        },
        Statement::FkCheck(_) => Effect { rows: None, written: 0 },
        Statement::AddCheck { table, check, .. } => {
            m.tables[*table].checks.push(check.clone());
            Effect { rows: None, written: 0 }
        },
        Statement::DropCheck { table, .. } => {
            m.tables[*table].checks.pop();
            Effect { rows: None, written: 0 }
        },
        Statement::DropConstraint { table, name } => {
            let t = &mut m.tables[*table];
            let at = t.names.iter().position(|n| n.name.eq_ignore_ascii_case(name)).expect("a valid DROP CONSTRAINT");
            let n = t.names.remove(at);
            match n.cons {
                Cons::NotNull => t.not_null[n.column.expect("a column's NOT NULL")] = false,
                Cons::Fk(cols) => {
                    let j = t.fks.iter().position(|f| f.columns == cols).expect("its foreign key");
                    t.fks.remove(j);
                },
                Cons::Bare | Cons::Fixed => {},
            }
            Effect { rows: None, written: 0 }
        },
        // Views and triggers follow their table to its new name.
        Statement::SwapTables { a, b, .. } => {
            m.tables.swap(*a, *b);
            let swap = |t: usize| if t == *a { *b } else if t == *b { *a } else { t };
            for t in &mut m.tables {
                for f in &mut t.fks {
                    f.parent = swap(f.parent);
                }
            }
            for v in &mut m.views {
                v.table = v.table.map(swap);
            }
            for g in &mut m.triggers {
                g.table = swap(g.table);
            }
            for g in &mut m.rules {
                map_rule_tables(g, &swap);
            }
            Effect { rows: None, written: 0 }
        },
        Statement::CreateRule(g) => {
            m.rules.push(g.clone());
            Effect { rows: None, written: 0 }
        },
        Statement::DropRule(id) => {
            m.rules.retain(|g| g.id != *id);
            Effect { rows: None, written: 0 }
        },
        Statement::RecursiveTriggers(on) => {
            m.recursive_triggers = *on;
            Effect { rows: None, written: 0 }
        },
        Statement::Reindex(_) => Effect { rows: None, written: 0 },
        Statement::ViewWrite { view, write } => {
            let n = view_write(m, *view, write).expect("a valid view write");
            Effect { rows: None, written: n }
        },
    }
}

/// A table's full contents as SQLite returns them for `SELECT rowid, *`
/// (rowid tables) or `SELECT *` (WITHOUT ROWID tables), in key order.
pub fn contents(t: &Table) -> Vec<Vec<Value>> {
    t.rows
        .iter()
        .map(|r| {
            let mut out = Vec::with_capacity(t.columns + 1);
            if !matches!(t.key, Key::WithoutRowid(_)) {
                out.push(Value::Int(r.rowid));
            }
            out.extend((0..t.columns).map(|c| column_value(t, r, c)));
            out
        })
        .collect()
}

/// Exact equality of rows (reals by bits).
pub fn same_rows(a: &[Vec<Value>], b: &[Vec<Value>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| exact(x) == exact(y))
}

/// What the model relies on about SQLite, checked against SQLite itself.
#[cfg(test)]
mod tests {
    use rusqlite::{types::ValueRef, Connection};

    use crate::{model::Value, statements::literal};

    fn rows(db: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut st = db.prepare(sql).unwrap();
        let n = st.column_count();
        let out = st
            .query_map([], |r| {
                Ok((0..n)
                    .map(|k| match r.get_ref(k).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(i) => Value::Int(i),
                        ValueRef::Real(x) => Value::Real(x),
                        ValueRef::Text(t) => Value::Text(String::from_utf8(t.to_vec()).unwrap()),
                        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
                    })
                    .collect())
            })
            .unwrap();
        out.map(Result::unwrap).collect()
    }

    fn ints(db: &Connection, sql: &str) -> Vec<i64> {
        rows(db, sql)
            .into_iter()
            .map(|r| match r[0] {
                Value::Int(i) => i,
                _ => panic!("{sql}"),
            })
            .collect()
    }

    #[test]
    fn real_literals_read_back_exactly() {
        let db = Connection::open_in_memory().unwrap();
        let mut s: u64 = 1;
        let mut next = || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            s ^ (s >> 29)
        };
        let mut xs: Vec<f64> = vec![0.0, -0.0, 5e-324, -5e-324, f64::MIN_POSITIVE, f64::MAX, f64::MIN, f64::INFINITY, f64::NEG_INFINITY, 1e50, 1e-50, 9.223_372_036_854_776e18];
        for k in 0..20_000 {
            let bits = next();
            xs.push(match k % 3 {
                0 => f64::from_bits(bits),
                1 => (bits >> 11) as f64 / (1u64 << 53) as f64 * 100.0 - 50.0,
                _ => (1.0 + (bits >> 11) as f64 / (1u64 << 53) as f64) * 10f64.powi((bits % 101) as i32 - 50),
            });
        }
        for x in xs.into_iter().filter(|x| !x.is_nan()) {
            let got = rows(&db, &format!("SELECT {}", literal(&Value::Real(x))));
            assert!(matches!(got[0][0], Value::Real(y) if y.to_bits() == x.to_bits()), "{x:?} read back as {:?}", got[0][0]);
        }
        let got = rows(&db, &format!("SELECT {}, {}", literal(&Value::Int(i64::MIN)), literal(&Value::Int(i64::MAX))));
        assert_eq!(got, [[Value::Int(i64::MIN), Value::Int(i64::MAX)]]);
    }

    #[test]
    fn strict_any_has_no_affinity_and_conversions_are_as_modelled() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE a(x ANY) STRICT; INSERT INTO a VALUES ('1'), (1), (x'31');
             CREATE TABLE s(i INT, r REAL, u ANY) STRICT;
             INSERT INTO s VALUES (3.0, 7, 1), (-0.0, -0.0, -0.0), (9007199254740992.0, 9223372036854775807, 2);
             CREATE TABLE u(x); INSERT INTO u VALUES (-0.0);",
        )
        .unwrap();
        assert_eq!(ints(&db, "SELECT x = 1 FROM a"), [0, 1, 0]);
        assert_eq!(ints(&db, "SELECT x = '1' FROM a"), [1, 0, 0]);
        let got = rows(&db, "SELECT i, r FROM s");
        assert_eq!(got[0], [Value::Int(3), Value::Real(7.0)]);
        assert!(matches!(got[1][..], [Value::Int(0), Value::Real(z)] if z.to_bits() == 0));
        assert_eq!(got[2], [Value::Int(1 << 53), Value::Real(9_223_372_036_854_775_807i64 as f64)]);
        assert!(matches!(rows(&db, "SELECT x FROM u")[0][0], Value::Real(z) if z.to_bits() == (-0.0f64).to_bits()));
    }

    /// Values of every class, those that affinities convert above all, and
    /// literals to compare them with.
    fn affinity_pool() -> Vec<Value> {
        use crate::model::Affinity;
        let mut out = vec![
            Value::Null,
            Value::Int(0),
            Value::Int(i64::MIN),
            Value::Int(i64::MAX),
            Value::Int(1 << 47),
            Value::Int((1 << 47) - 1),
            Value::Int(-(1 << 47)),
            Value::Int((1 << 53) + 1),
            Value::Real(-0.0),
            Value::Real(0.5),
            Value::Real(3.0),
            Value::Real(-2.25),
            Value::Real(1e300),
            Value::Real(f64::INFINITY),
            Value::Real(f64::NEG_INFINITY),
            Value::Real(-9_223_372_036_854_775_808.0),
            Value::Real(9_223_372_036_854_775_808.0),
            Value::Real(140_737_488_355_328.0),
            Value::Real(0.1),
            Value::Real(123_456.015_625),
            Value::Text(String::new()),
            Value::Text("abc".into()),
            Value::Text("ABC".into()),
            Value::Blob(vec![]),
            Value::Blob(vec![0x31]),
        ];
        let mut g = ephyra::Source::new(7, 50);
        for _ in 0..120 {
            out.push(Value::Text(crate::cases::number_text(&mut g)));
        }
        for aff in [Affinity::Text, Affinity::Numeric, Affinity::None] {
            for _ in 0..40 {
                out.push(crate::cases::converted(&mut g, aff));
                out.push(crate::cases::fresh_for(&mut g, aff));
            }
        }
        out
    }

    #[test]
    fn affinities_store_and_compare_as_modelled() {
        use crate::{
            model::{real_text, store, Key, Row, Table, DECLS},
            semantics::{comparable, eval},
            statements::{CmpOp, Pred},
        };
        let db = Connection::open_in_memory().unwrap();
        let n = DECLS.len() + 1;
        let cols: Vec<String> = (0..n)
            .map(|c| match DECLS.get(c) {
                Some((name, _)) => format!("c{c} {name}"),
                None => format!("c{c}"),
            })
            .collect();
        db.execute_batch(&format!("CREATE TABLE a({})", cols.join(", "))).unwrap();
        let mut t = Table::new(n, Key::Rowid);
        t.decl = (0..n).map(|c| (c < DECLS.len()).then_some(c)).collect();
        let pool = affinity_pool();
        let names = (0..n).map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
        let mut known = 0;
        for (r, v) in pool.iter().enumerate() {
            let lits = vec![literal(v); n].join(", ");
            db.execute_batch(&format!("INSERT INTO a(rowid, {names}) VALUES ({r}, {lits})")).unwrap();
            let got = rows(&db, &format!("SELECT * FROM a WHERE rowid = {r}")).remove(0);
            for (c, sqlite) in got.iter().enumerate() {
                let aff = crate::model::affinity(&t, c);
                if let Some(w) = store(aff, v) {
                    known += 1;
                    let same = match (&w, sqlite) {
                        (Value::Real(a), Value::Real(b)) => a.to_bits() == b.to_bits(),
                        (a, b) => a == b,
                    };
                    assert!(same, "{v:?} into c{c} {:?} ({aff:?}): model {w:?}, SQLite {sqlite:?}", DECLS.get(c));
                }
            }
            t.rows.push(Row { rowid: r as i64, values: got });
        }
        assert!(known > pool.len() * n * 9 / 10, "{known} of {}", pool.len() * n);
        let mut compared = 0;
        for v in &pool {
            for op in CmpOp::ALL {
                let items: Vec<String> = (0..n).map(|c| format!("c{c} {} {}", op.sql(), literal(v))).collect();
                let got = rows(&db, &format!("SELECT {} FROM a ORDER BY rowid", items.join(", ")));
                for (row, sqlite) in t.rows.iter().zip(&got) {
                    for (c, answer) in sqlite.iter().enumerate() {
                        if !comparable(&t, c, v) {
                            continue;
                        }
                        compared += 1;
                        let model = match eval(&t, row, &Pred::Cmp(c, op, v.clone()), crate::model::Encoding::Utf8) {
                            None => Value::Null,
                            Some(b) => Value::Int(i64::from(b)),
                        };
                        assert_eq!(model, *answer, "c{c} {:?} holding {:?} {} {}", DECLS.get(c), row.values[c], op.sql(), literal(v));
                    }
                }
            }
        }
        assert!(compared > 100_000, "{compared}");
        for k in -4000..4000 {
            for d in [1.0, 2.0, 8.0, 64.0] {
                let x = f64::from(k) / d * 7.0;
                if let Some(text) = real_text(x) {
                    let got = rows(&db, &format!("SELECT CAST({} AS TEXT)", literal(&Value::Real(x))));
                    assert_eq!(got[0][0], Value::Text(text), "{x}");
                }
            }
        }
    }

    /// Comparisons and orders under each collating sequence are the
    /// model's, in UTF-8 and UTF-16 databases: NOCASE folds only ASCII
    /// capitals and stops at a NUL (texts equal up to one compare by
    /// length), RTRIM trims only spaces, and both compare UTF-8 bytes
    /// (where a UTF-16 database's BINARY order differs: U+FF5E against an
    /// emoji); texts equal under NOCASE or RTRIM fold alike
    /// ([`crate::model::fold`]); and a UNIQUE index tells texts apart under
    /// its own sequence.
    #[test]
    fn collations_compare_as_modelled() {
        use crate::model::{compare_coll, fold, Coll, Encoding};
        let texts = [
            "", "a", "A", "a ", "A  ", "ab", "aB", "b", "B", "é", "É", "\u{ff5e}", "😀", " a", "a\t", "Ж", "ж", "_", "[", "a\0z", "A\0b", "a\0", "a\0 ", "a\0bc", "a \0", "\0", "ab\0",
        ];
        // Text with a NUL as `char(..)`: SQL text ends at one.
        let text = |s: &str| {
            if s.contains('\0') {
                format!("char({})", s.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(", "))
            } else {
                literal(&Value::Text(s.into()))
            }
        };
        for (enc, pragma) in [(Encoding::Utf8, "UTF-8"), (Encoding::Utf16le, "UTF-16le"), (Encoding::Utf16be, "UTF-16be")] {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(&format!("PRAGMA encoding = '{pragma}'; CREATE TABLE t(x)")).unwrap();
            for coll in Coll::ALL {
                for a in texts {
                    for b in texts {
                        let (x, y) = (Value::Text(a.into()), Value::Text(b.into()));
                        let sql = format!("SELECT ({} < {} COLLATE {n}), ({0} = {1} COLLATE {n})", text(a), text(b), n = coll.name());
                        let r = rows(&db, &sql);
                        let o = compare_coll(&x, &y, enc, coll);
                        assert_eq!(r[0], [Value::Int(i64::from(o.is_lt())), Value::Int(i64::from(o.is_eq()))], "{a:?} vs {b:?} under {coll:?} in {pragma}");
                        if o.is_eq() {
                            assert_eq!(fold(a.as_bytes()), fold(b.as_bytes()), "{a:?} and {b:?}, equal under {coll:?}");
                        }
                    }
                }
            }
            assert!(fold(b"a") != fold(b"b") && fold(b"a") != fold(b"a\0") && fold(b"a\0") != fold(b"A \0"));
            db.execute_batch("CREATE TABLE u(x COLLATE NOCASE UNIQUE); INSERT INTO u VALUES ('ab')").unwrap();
            assert!(db.execute_batch("INSERT INTO u VALUES ('AB')").is_err());
            assert!(db.execute_batch("INSERT INTO u VALUES ('ab ')").is_ok());
        }
    }

    #[test]
    fn new_rowids_follow_the_largest() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE b(x INTEGER PRIMARY KEY, y); INSERT INTO b VALUES (-5, 1), (-3, 2); INSERT INTO b(y) VALUES (3), (4);
             CREATE TABLE c(y); INSERT INTO c(rowid, y) VALUES (7, 0); DELETE FROM c; INSERT INTO c(y) VALUES (1);",
        )
        .unwrap();
        assert_eq!(ints(&db, "SELECT x FROM b"), [-5, -3, -2, -1]);
        assert_eq!(ints(&db, "SELECT rowid FROM c"), [1]);
    }

    #[test]
    fn vacuum_renumbers_only_plain_tables_without_indexes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE p(a); INSERT INTO p(rowid, a) VALUES (-7, 1), (5, 2), (100, 3);
             CREATE TABLE q(a); CREATE INDEX qi ON q(a); INSERT INTO q(rowid, a) VALUES (-7, 1), (5, 2);
             CREATE TABLE r(k INTEGER PRIMARY KEY, a); INSERT INTO r VALUES (-7, 1), (5, 2);
             VACUUM; PRAGMA incremental_vacuum(3);",
        )
        .unwrap();
        assert_eq!(ints(&db, "SELECT rowid FROM p"), [1, 2, 3]);
        assert_eq!(ints(&db, "SELECT rowid FROM q ORDER BY rowid"), [-7, 5]);
        assert_eq!(ints(&db, "SELECT rowid FROM r"), [-7, 5]);
    }

    #[test]
    fn rowid_lookup_of_minus_two_to_the_63_finds_it() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE c(x INTEGER PRIMARY KEY); INSERT INTO c VALUES (-9223372036854775808), (1);").unwrap();
        // SQLite 3.53.4 and earlier missed this row in the rowid lookup,
        // though a scan finds it equal; trunk (2662a68f0d5c) finds it, so the
        // model compares the real -2^63 with an INTEGER PRIMARY KEY like any
        // other value.
        assert_eq!(ints(&db, "SELECT x FROM c WHERE x = -9.223372036854776e18"), [i64::MIN]);
        assert_eq!(ints(&db, "SELECT x FROM c WHERE +x = -9.223372036854776e18"), [i64::MIN]);
        // Range lookups agree with comparison.
        assert_eq!(ints(&db, "SELECT x FROM c WHERE x <= -9.223372036854776e18"), [i64::MIN]);
        assert_eq!(ints(&db, "SELECT x FROM c WHERE x > -9.223372036854776e18"), [1]);
    }

    /// An empty model with tables `defs` (the schema made in `db` too)
    /// and views `views`, then each statement run by both: SQLite runs it
    /// exactly when the model says it is valid; the tables agree at the end.
    fn agree(defs: Vec<crate::model::Table>, views: Vec<crate::model::View>, statements: &[crate::statements::Statement]) -> crate::model::Model {
        use crate::{model::{Encoding, Model}, semantics::{apply, contents, same_rows, valid}, writer::{create_table, create_view}};
        let db = Connection::open_in_memory().unwrap();
        for (k, t) in defs.iter().enumerate() {
            db.execute_batch(&create_table(&format!("t{k}"), t)).unwrap();
        }
        for (k, v) in views.iter().enumerate() {
            db.execute_batch(&create_view(&format!("v{k}"), v)).unwrap();
        }
        let mut m = Model { encoding: Encoding::Utf8, tables: defs, views, triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        for s in statements {
            let sql = s.to_string();
            let ok = valid(&m, s);
            let ran = db.execute_batch(&sql);
            assert_eq!(ok, ran.is_ok(), "{sql}: the model says {}, SQLite {ran:?}", if ok { "valid" } else { "invalid" });
            if ok {
                apply(&mut m, s);
            }
        }
        for (k, t) in m.tables.iter().enumerate() {
            let sql = match &t.key {
                crate::model::Key::WithoutRowid(pk) => format!("SELECT * FROM t{k} ORDER BY {}", pk.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ")),
                _ => format!("SELECT rowid, * FROM t{k} ORDER BY rowid"),
            };
            let actual = rows(&db, &sql);
            assert!(same_rows(&actual, &contents(t)), "t{k}: model {:?}, SQLite {actual:?}", contents(t));
        }
        m
    }

    /// DO UPDATE runs as an UPDATE with ABORT, which overrides the
    /// conflict resolution of the steps its triggers run and of the
    /// tables they write: a key's ON CONFLICT REPLACE does not replace
    /// (run 2000-writes w7-112922: `UNIQUE constraint failed: t8.c3`).
    /// The insert path keeps the key's REPLACE.
    #[test]
    fn upsert_do_update_runs_its_triggers_with_abort() {
        use crate::{
            model::{Key, Rule, Step, Table, Term},
            statements::{Excluded, Insert, Resolve, Statement, Upsert},
        };
        let t = Table::new(2, Key::WithoutRowid(vec![0]));
        let mut u = Table::new(2, Key::WithoutRowid(vec![1]));
        u.key_conflict = Some(Resolve::Replace);
        let rule = |id, event| Rule { id, table: 0, view: false, temp: false, db: crate::model::Db::Main, before: true, event, of: Vec::new(), when: None, body: vec![Step::Insert { table: 1, columns: vec![0, 1], values: vec![Term::Row(1), Term::Lit(Value::Text("k".into()))], or: None, upsert: Vec::new() }] };
        let ins = |rows: Vec<Vec<Value>>, update: bool| {
            Statement::Insert(Insert { table: 0, columns: vec![0, 1], rows, upsert: vec![Upsert { target: None, update: update.then(|| (vec![(1, Excluded::Yes)], None)) }], ..Default::default() })
        };
        let m = agree(
            vec![t, u],
            Vec::new(),
            &[
                Statement::CreateRule(rule(0, 0)),
                Statement::CreateRule(rule(1, 1)),
                // Each insert path's step replaces the row of t1 there.
                ins(vec![vec![Value::Int(1), Value::Int(10)], vec![Value::Int(2), Value::Int(20)]], false),
                ins(vec![vec![Value::Int(3), Value::Int(30)]], true),
                // The insert path replaces the row of t1, then DO UPDATE's
                // trigger conflicts on it.
                ins(vec![vec![Value::Int(1), Value::Int(11)]], true),
            ],
        );
        assert_eq!(m.tables[1].rows.len(), 1);
    }

    /// Two foreign keys of one child row to one parent row, a CASCADE and
    /// a SET DEFAULT: SQLite runs the SET DEFAULT first, which fails the
    /// CHECK ADD CONSTRAINT put on the column (run 1313 cte w4-67870:
    /// `CHECK constraint failed: k0`), where the model ran the CASCADE
    /// first and the SET DEFAULT then found no row. A row two actions
    /// reach is left out also when the first deleted it.
    #[test]
    fn a_row_two_actions_reach_is_left_out_when_the_first_deletes_it() {
        use crate::{
            model::{Action, ForeignKey, Key, Table},
            statements::{Delete, Insert, Pred, Statement},
        };
        let p = Table::new(2, Key::WithoutRowid(vec![1, 0]));
        let mut c = Table::new(4, Key::Rowid);
        c.not_null[3] = true;
        let fk = |columns, on_delete| ForeignKey { columns, parent: 0, parent_columns: vec![1, 0], on_delete, on_update: Action::NoAction, deferred: false };
        c.fks = vec![fk(vec![3, 0], Action::Cascade), fk(vec![1, 2], Action::SetDefault)];
        let ins = |table, columns: Vec<usize>, row: Vec<i64>| Statement::Insert(Insert { table, columns, rows: vec![row.into_iter().map(Value::Int).collect()], ..Default::default() });
        agree(
            vec![p, c],
            Vec::new(),
            &[
                ins(0, vec![0, 1], vec![1, 2]),
                Statement::AddCheck { table: 1, index: 0, check: Pred::IsNull(2, true) },
                ins(1, vec![0, 1, 2, 3], vec![1, 2, 1, 2]),
                Statement::Delete(Delete { table: 0, filter: None, returning: None }),
            ],
        );
    }

    /// ALTER COLUMN SET NOT NULL on a child column a SET NULL or SET
    /// DEFAULT (NULL) action writes: the action fails when it reaches a
    /// row, and only then.
    #[test]
    fn set_not_null_fails_set_null_and_set_default_actions() {
        use crate::{
            model::{Action, ForeignKey, Key, Table},
            statements::{Delete, Insert, Pred, Statement},
        };
        for action in [Action::SetNull, Action::SetDefault] {
            let p = Table::new(1, Key::WithoutRowid(vec![0]));
            let mut c = Table::new(2, Key::Rowid);
            c.fks = vec![ForeignKey { columns: vec![0], parent: 0, parent_columns: vec![0], on_delete: action, on_update: Action::NoAction, deferred: false }];
            let ins = |table, row: Vec<i64>| Statement::Insert(Insert { table, columns: (0..row.len()).collect(), rows: vec![row.into_iter().map(Value::Int).collect()], ..Default::default() });
            let del = |k| Statement::Delete(Delete { table: 0, filter: Some(Pred::Cmp(0, crate::statements::CmpOp::Eq, Value::Int(k))), returning: None });
            let m = agree(
                vec![p, c],
                Vec::new(),
                &[
                    ins(0, vec![1]),
                    ins(0, vec![2]),
                    ins(1, vec![1, 10]),
                    Statement::NotNull { table: 1, column: 0, on: true },
                    del(1),
                    del(2),
                ],
            );
            assert_eq!(m.tables[0].rows.len(), 1);
        }
    }

    /// The steps of the DELETE triggers a REPLACE fires run with REPLACE,
    /// also when it is the key's own ON CONFLICT REPLACE that replaces.
    #[test]
    fn key_replace_runs_delete_triggers_with_replace() {
        use crate::{
            model::{Key, Rule, Step, Table, Term},
            statements::{Insert, Resolve, Statement},
        };
        let mut t = Table::new(2, Key::WithoutRowid(vec![0]));
        t.key_conflict = Some(Resolve::Replace);
        let u = Table::new(2, Key::WithoutRowid(vec![0]));
        let ins = |table, rows: Vec<Vec<Value>>| Statement::Insert(Insert { table, columns: vec![0, 1], rows, ..Default::default() });
        let rule = Rule { id: 0, table: 0, view: false, temp: false, db: crate::model::Db::Main, before: false, event: 2, of: Vec::new(), when: None, body: vec![Step::Insert { table: 1, columns: vec![0, 1], values: vec![Term::Lit(Value::Int(1)), Term::Row(1)], or: Some(Resolve::Ignore), upsert: Vec::new() }] };
        let m = agree(
            vec![t, u],
            Vec::new(),
            &[
                Statement::RecursiveTriggers(true),
                ins(1, vec![vec![Value::Int(1), Value::Text("old".into())]]),
                Statement::CreateRule(rule),
                ins(0, vec![vec![Value::Int(1), Value::Text("x".into())]]),
                ins(0, vec![vec![Value::Int(1), Value::Text("y".into())]]),
            ],
        );
        assert_eq!(m.tables[1].rows[0].values[1], Value::Text("x".into()));
    }

    /// NEW.c of a view's column of REAL affinity reads an integer as a
    /// real (run 2000-writes w7-126880: -9223372036854775808 through a
    /// STRICT REAL column's view into a STRICT INTEGER column: `cannot
    /// store REAL value`); text stays text.
    #[test]
    fn view_insert_reads_integers_as_reals_in_real_columns() {
        use crate::{
            model::{ColType, Key, Rule, Step, Table, Term, View},
            statements::{Insert, Statement},
        };
        let mut t = Table::new(3, Key::Rowid);
        t.strict = Some(vec![ColType::Integer, ColType::Any, ColType::Real]);
        let log = Table::new(1, Key::Rowid);
        let rule = |id, table, column| Rule { id, table: 0, view: true, temp: false, db: crate::model::Db::Main, before: false, event: 0, of: Vec::new(), when: None, body: vec![Step::Insert { table, columns: vec![column], values: vec![Term::Row(2)], or: None, upsert: Vec::new() }] };
        let write = |rows: Vec<Value>| Statement::ViewWrite { view: 0, write: Box::new(Statement::Insert(Insert { table: 0, columns: vec![2], rows: rows.into_iter().map(|v| vec![v]).collect(), ..Default::default() })) };
        let m = agree(
            vec![t, log],
            vec![View { table: Some(0), columns: vec![2], pad: 0 }],
            &[
                Statement::CreateRule(rule(0, 1, 0)),
                write(vec![Value::Int(7), Value::Text("12".into()), Value::Null, Value::Int(i64::MIN)]),
                Statement::DropRule(0),
                Statement::CreateRule(rule(0, 0, 0)),
                write(vec![Value::Int(92319)]),
                write(vec![Value::Int(i64::MIN)]),
            ],
        );
        assert_eq!(m.tables[1].rows[0].values[0], Value::Real(7.0));
    }

    /// A write whose triggers multiply (a view's DELETE over many rows,
    /// its trigger updating them all, each update's trigger writing a
    /// table of many rows, which grows) is left out once past [`super::MAX_WORK`],
    /// quickly (run 2000-writes hang-w3: generation never finished).
    #[test]
    fn multiplying_triggers_are_left_out_quickly() {
        use crate::{
            model::{Encoding, Key, Model, Row, Rule, Step, Table, Term, View},
            statements::{Delete, Statement},
        };
        let table = |n: i64| {
            let mut t = Table::new(2, Key::Rowid);
            t.rows = (1..=n).map(|r| Row { rowid: r, values: vec![Value::Int(r), Value::Int(0)] }).collect();
            t
        };
        let mut m = Model { encoding: Encoding::Utf8, tables: vec![table(300), table(300)], views: vec![View { table: Some(0), columns: vec![0, 1], pad: 0 }], triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        let update = |table, c| Step::Update { table, sets: vec![(c, Term::Lit(Value::Int(1)))], filter: None, or: None };
        m.rules.push(Rule { id: 0, table: 0, view: true, temp: false, db: crate::model::Db::Main, before: false, event: 2, of: Vec::new(), when: None, body: vec![Step::Update { table: 0, sets: vec![(1, Term::Row(1))], filter: None, or: None }] });
        m.rules.push(Rule { id: 1, table: 0, view: false, temp: false, db: crate::model::Db::Main, before: false, event: 1, of: Vec::new(), when: None, body: vec![update(1, 0), Step::Insert { table: 1, columns: vec![0, 1], values: vec![Term::Lit(Value::Int(2)), Term::Lit(Value::Int(3))], or: None, upsert: Vec::new() }, update(1, 1)] });
        let s = Statement::ViewWrite { view: 0, write: Box::new(Statement::Delete(Delete::default())) };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(super::valid(&m, &s)).unwrap());
        let ok = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("modelled within 20 s");
        assert!(!ok);
    }
}
