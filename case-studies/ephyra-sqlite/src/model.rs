// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The logical database a generated file holds ([`Model`]) and the layout
//! choices it is written with ([`Layout`]). The writer (`crate::writer`)
//! turns the pair into bytes, the reader (`crate::reader`) turns those bytes
//! back into the same pair, and the generator's shrinker and mutator work on
//! the pair, never on bytes.

use std::{borrow::Cow, cmp::Ordering};

use crate::statements::Pred;

/// A column value. Reals are never NaN (SQLite reads a stored NaN as NULL).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// The database text encoding (header offset 56).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16le,
    Utf16be,
}

impl Encoding {
    pub fn code(self) -> u32 {
        match self {
            Encoding::Utf8 => 1,
            Encoding::Utf16le => 2,
            Encoding::Utf16be => 3,
        }
    }

    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Encoding::Utf8),
            2 => Some(Encoding::Utf16le),
            3 => Some(Encoding::Utf16be),
            _ => None,
        }
    }

    pub fn encode(self, s: &str) -> Vec<u8> {
        match self {
            Encoding::Utf8 => s.as_bytes().to_vec(),
            Encoding::Utf16le | Encoding::Utf16be => {
                let mut out = Vec::with_capacity(2 * s.len());
                let le = self == Encoding::Utf16le;
                for unit in s.encode_utf16() {
                    out.extend_from_slice(&if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
                }
                out
            },
        }
    }

    pub fn decode(self, b: &[u8]) -> Option<String> {
        let units = |f: fn([u8; 2]) -> u16| -> Option<String> {
            if !b.len().is_multiple_of(2) {
                return None;
            }
            String::from_utf16(&b.as_chunks::<2>().0.iter().map(|c| f(*c)).collect::<Vec<_>>()).ok()
        };
        match self {
            Encoding::Utf8 => String::from_utf8(b.to_vec()).ok(),
            Encoding::Utf16le => units(u16::from_le_bytes),
            Encoding::Utf16be => units(u16::from_be_bytes),
        }
    }
}

/// A database: tables, each with its secondary indexes, then views and
/// triggers. Names are positional: table `i` is `t{i}`, its columns `c0,
/// c1, …`, its index `k` is `t{i}_i{k}`; view `k` is `v{k}`, trigger `k`
/// is `g{k}`.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub encoding: Encoding,
    pub tables: Vec<Table>,
    pub views: Vec<View>,
    pub triggers: Vec<Trigger>,
    /// The model as it was at each open savepoint, outermost first: state
    /// statements keep (see [`crate::semantics`]); a file's model has none.
    pub savepoints: Vec<Model>,
    /// `PRAGMA foreign_keys`: whether SQLite enforces foreign keys (on
    /// for a new connection: the bundled SQLite is built with
    /// `SQLITE_DEFAULT_FOREIGN_KEYS=1`; not part of a savepoint).
    pub foreign_keys: bool,
    /// Triggers statements create, with bodies that write (see [`Rule`]),
    /// named by their ids.
    pub rules: Vec<Rule>,
    /// `PRAGMA recursive_triggers` (off for a new connection; not part of
    /// a savepoint): REPLACE's deletions fire DELETE triggers only when on.
    pub recursive_triggers: bool,
    /// The connection's storage settings that decide which statements are
    /// valid (see [`Session`]).
    pub session: Session,
    /// The virtual tables statements make, `vt{k}` (see
    /// [`crate::vtabs`]); part of a savepoint.
    pub vtabs: Vec<crate::vtabs::VTable>,
}

/// Connection state the storage statements leave (not part of a
/// savepoint): nothing a query reads, only which statements stay valid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Session {
    /// `PRAGMA journal_mode = OFF` was the last journal mode set other
    /// than WAL (which memory refuses, the mode staying OFF): no rollback
    /// journal, so ROLLBACK and ROLLBACK TO are never drawn.
    pub journal_off: bool,
    /// The last journal mode set was WAL (refused in memory, taken in a
    /// file): checkpoints are drawn more.
    pub wal: bool,
    /// `PRAGMA locking_mode = EXCLUSIVE` was the last locking mode set.
    pub exclusive: bool,
    /// The open transaction began with BEGIN (`crate::statements::Statement::Begin`):
    /// `RELEASE s0` would not end it, COMMIT and ROLLBACK do.
    pub begun: bool,
    /// `ATTACH DATABASE ':memory:' AS aux` ran and no DETACH since: tables
    /// may be made in [`Db::Aux`].
    pub aux: bool,
    /// `PRAGMA journal_mode = OFF` was ever set: it applies to every
    /// database of the connection, and an in-memory one (the attached
    /// `aux`, a TEMP database in memory) never takes a rollback journal
    /// again (`sqlite3PagerSetJournalMode` keeps a memory database's OFF),
    /// so a rollback cannot undo writes to their tables.
    pub memory_off: bool,
    /// The case's file is empty: main is a database in memory, not a file
    /// deserialized there, and keeps OFF as `aux` does once
    /// [`Session::memory_off`] (a ROLLBACK then undoes nothing: a table
    /// made in the transaction stays in the schema, an FTS5 table's index
    /// loses what its content table kept, "malformed inverted index").
    pub memory_main: bool,
}

/// The schema a table lives in. Names are unique over all of them (table
/// `t{k}` is in one only), so an unqualified name resolves to it wherever
/// it is (`sqlite3LocateTable` searches TEMP, then main, then attached
/// databases); DDL names the schema where SQLite needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Db {
    #[default]
    Main,
    /// `CREATE TEMP TABLE`.
    Temp,
    /// The attached in-memory database `aux`.
    Aux,
}

impl Db {
    pub const ALL: [Db; 3] = [Db::Main, Db::Temp, Db::Aux];

    pub fn name(self) -> &'static str {
        match self {
            Db::Main => "main",
            Db::Temp => "temp",
            Db::Aux => "aux",
        }
    }
}

/// `CREATE TRIGGER w{id} BEFORE|AFTER INSERT|UPDATE [OF c..]|DELETE ON
/// t{table} FOR EACH ROW [WHEN p] BEGIN step; ..; END`: `when` over the
/// row the trigger sees (NEW for INSERT and UPDATE, OLD for DELETE, its
/// columns prefixed so), each step in order (see [`Step`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: usize,
    /// The table, or for an INSTEAD OF trigger the view (`v{table}`).
    pub table: usize,
    /// `INSTEAD OF` on a view ([`crate::semantics::writable_view`]).
    pub view: bool,
    /// `CREATE TEMP TRIGGER` (kept in the temp schema, fired the same).
    pub temp: bool,
    /// The schema of its table (main for a view's): a trigger that is not
    /// TEMP lives there, `CREATE TRIGGER aux.w{id} .. ON t{k}`, and its
    /// body's tables resolve there only (the DbFixer, `fixSelectCb`).
    pub db: Db,
    pub before: bool,
    /// 0 INSERT, 1 UPDATE, 2 DELETE.
    pub event: u8,
    /// `UPDATE OF` these columns (any column when empty).
    pub of: Vec<usize>,
    pub when: Option<crate::statements::Pred>,
    pub body: Vec<Step>,
}

/// A step of a trigger's body: a write to another table, with values of
/// the row the trigger sees ([`Term::Row`]) or literals; `SELECT
/// RAISE(IGNORE)` (BEFORE triggers: the row is left as it is, the
/// remaining steps and triggers skipped); `SELECT RAISE(ABORT, 'k') WHERE
/// p` (the statement fails where `p` holds of the row; never drawn so);
/// `SELECT NEW.c` (OLD for DELETE).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Insert { table: usize, columns: Vec<usize>, values: Vec<Term>, or: Option<crate::statements::Resolve>, upsert: Vec<crate::statements::Upsert> },
    Update { table: usize, sets: Vec<(usize, Term)>, filter: Option<crate::statements::Pred>, or: Option<crate::statements::Resolve> },
    Delete { table: usize, filter: Option<crate::statements::Pred> },
    Ignore,
    Abort(crate::statements::Pred),
    Select(usize),
}

/// A value a step writes: a literal, or a column of the row the trigger
/// sees.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    Lit(Value),
    Row(usize),
}

/// `CREATE VIEW v{k} AS SELECT c.., 'xx…' FROM t{table}`: the columns of a
/// table (none without one), then a text literal of `pad` x's when `pad` >
/// 0; `SELECT 1` when there is nothing to select.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub table: Option<usize>,
    /// Distinct columns of `table`.
    pub columns: Vec<usize>,
    pub pad: usize,
}

/// `CREATE TRIGGER g{k} BEFORE|AFTER INSERT|UPDATE|DELETE ON t{table}
/// BEGIN SELECT NEW.c{column}; END` (OLD for DELETE; `SELECT 1` without a
/// column).
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub table: usize,
    pub after: bool,
    /// 0 INSERT, 1 UPDATE, 2 DELETE.
    pub event: u8,
    pub column: Option<usize>,
}

/// A STRICT table's declared column type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColType {
    Any,
    Blob,
    Int,
    Integer,
    Real,
    Text,
}

impl ColType {
    pub const ALL: [ColType; 6] = [ColType::Any, ColType::Blob, ColType::Int, ColType::Integer, ColType::Real, ColType::Text];

    pub fn name(self) -> &'static str {
        match self {
            ColType::Any => "ANY",
            ColType::Blob => "BLOB",
            ColType::Int => "INT",
            ColType::Integer => "INTEGER",
            ColType::Real => "REAL",
            ColType::Text => "TEXT",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == s)
    }

    /// Whether a model value may sit in a column of this type. A REAL
    /// column holds reals (some stored as integers, see [`stored`]).
    pub fn admits(self, v: &Value) -> bool {
        matches!(
            (self, v),
            (_, Value::Null)
                | (ColType::Any, _)
                | (ColType::Blob, Value::Blob(_))
                | (ColType::Int | ColType::Integer, Value::Int(_))
                | (ColType::Real, Value::Real(_))
                | (ColType::Text, Value::Text(_))
        )
    }
}

/// A column's type affinity (datatype3.html section 3): `None` is BLOB
/// affinity, which converts nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    None,
    Text,
    Numeric,
    Integer,
    Real,
}

impl Affinity {
    /// INTEGER, REAL or NUMERIC: text that reads as a number is stored, and
    /// compared, as that number.
    pub fn numeric(self) -> bool {
        matches!(self, Affinity::Numeric | Affinity::Integer | Affinity::Real)
    }
}

/// A collating sequence (datatype3.html section 7): BINARY compares text
/// by its bytes in the database encoding; NOCASE by its UTF-8 bytes with
/// the 26 ASCII capitals folded to lower case; RTRIM by its UTF-8 bytes
/// without trailing spaces (SQLite registers both only for UTF-8, and
/// converts text of a UTF-16 database to compare it). Only text is
/// collated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coll {
    Binary,
    Nocase,
    Rtrim,
}

impl Coll {
    pub const ALL: [Coll; 3] = [Coll::Binary, Coll::Nocase, Coll::Rtrim];

    pub fn name(self) -> &'static str {
        match self {
            Coll::Binary => "BINARY",
            Coll::Nocase => "NOCASE",
            Coll::Rtrim => "RTRIM",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    /// Text `t` (UTF-8 bytes) as the sequence compares it: two texts
    /// compare under it as their keys compare by bytes, so are equal under
    /// it exactly when their keys are. BINARY: the bytes. RTRIM: the bytes
    /// without trailing spaces (`rtrimCollFunc`: 0x20 only, not tabs or
    /// other whitespace). NOCASE: the bytes with the 26 ASCII capitals in
    /// lower case (`sqlite3StrNICmp`, nothing beyond ASCII), up to the
    /// first NUL, where `sqlite3StrNICmp` stops: two texts equal up to a
    /// NUL in both compare by their lengths (`nocaseCollatingFunc`), which
    /// follow the NUL in the key as 8 bytes big-endian (`'a\0z'` and
    /// `'a\0b'` are equal, `'a\0'` below both, all three below `'ab'`).
    /// In a UTF-16 database BINARY orders the encoded bytes instead
    /// ([`compare`]); NOCASE and RTRIM compare UTF-8 in every database.
    pub fn key(self, t: &[u8]) -> Cow<'_, [u8]> {
        match self {
            Coll::Binary => Cow::Borrowed(t),
            Coll::Rtrim => Cow::Borrowed(trim_spaces(t)),
            Coll::Nocase => match t.iter().position(|&b| b == 0) {
                Some(i) => {
                    let mut k = t[..i].to_ascii_lowercase();
                    k.push(0);
                    k.extend((t.len() as u64).to_be_bytes());
                    Cow::Owned(k)
                },
                None if t.iter().any(u8::is_ascii_uppercase) => Cow::Owned(t.to_ascii_lowercase()),
                None => Cow::Borrowed(t),
            },
        }
    }
}

/// `t` without trailing spaces, as RTRIM compares it.
fn trim_spaces(t: &[u8]) -> &[u8] {
    let n = t.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    &t[..n]
}

/// Text `t` with what NOCASE and RTRIM leave aside taken out, for
/// comparisons that take whichever of texts equal under one of them SQLite
/// picks: two texts equal under either sequence (or linked by a chain of
/// such equalities) fold alike. Without a NUL, the text without trailing
/// spaces under NOCASE's key ([`Coll::key`]); with one, its NOCASE key up
/// to and with the first NUL, as the texts NOCASE takes for equal there
/// differ only in length, which RTRIM then takes away.
pub fn fold(t: &[u8]) -> Cow<'_, [u8]> {
    match t.iter().position(|&b| b == 0) {
        Some(i) => {
            let mut k = Coll::Nocase.key(&t[..i]).into_owned();
            k.push(0);
            Cow::Owned(k)
        },
        None => Coll::Nocase.key(trim_spaces(t)),
    }
}

/// Column `c`'s collating sequence: its declared one, BINARY without.
pub fn column_coll(t: &Table, c: usize) -> Coll {
    t.coll.get(c).copied().flatten().unwrap_or(Coll::Binary)
}

/// The collating sequence of column `c` in index `i`: the index's own
/// `COLLATE` for it, else the column's.
pub fn index_coll(t: &Table, i: &Index, c: usize) -> Coll {
    i.coll.iter().find(|(x, _)| *x == c).map_or_else(|| column_coll(t, c), |&(_, k)| k)
}

/// SQLite's order of values under a collating sequence: [`compare`], but
/// two texts by `coll` other than BINARY ([`Coll::key`]).
pub fn compare_coll(a: &Value, b: &Value, enc: Encoding, coll: Coll) -> Ordering {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) if coll != Coll::Binary => coll.key(x.as_bytes()).cmp(&coll.key(y.as_bytes())),
        _ => compare(a, b, enc),
    }
}

/// Declared column types of tables that are not STRICT, written as they
/// are, each with the affinity SQLite's rules give it (datatype3.html
/// section 3.1, in order: a name containing `INT` is INTEGER; else `CHAR`,
/// `CLOB` or `TEXT`, TEXT; else `BLOB`, none; else `REAL`, `FLOA` or
/// `DOUB`, REAL; else NUMERIC), the oddities included: `FLOATING POINT`
/// holds `INT`, `STRING` none of them, and `ANY` outside a STRICT table is
/// NUMERIC. No name holds `", "`, which separates column definitions.
pub const DECLS: [(&str, Affinity); 34] = [
    ("INT", Affinity::Integer),
    ("INTEGER", Affinity::Integer),
    ("TINYINT", Affinity::Integer),
    ("SMALLINT", Affinity::Integer),
    ("BIGINT", Affinity::Integer),
    ("UNSIGNED BIG INT", Affinity::Integer),
    ("INT8", Affinity::Integer),
    ("int2", Affinity::Integer),
    ("FLOATING POINT", Affinity::Integer),
    ("INTERVAL", Affinity::Integer),
    ("TEXT", Affinity::Text),
    ("VARCHAR(20)", Affinity::Text),
    ("CHARACTER(10)", Affinity::Text),
    ("NVARCHAR(100)", Affinity::Text),
    ("varying character(255)", Affinity::Text),
    ("CLOB", Affinity::Text),
    ("TEXTBLOB", Affinity::Text),
    ("BLOB", Affinity::None),
    ("blob(16)", Affinity::None),
    ("REAL", Affinity::Real),
    ("DOUBLE", Affinity::Real),
    ("DOUBLE PRECISION", Affinity::Real),
    ("FLOAT", Affinity::Real),
    ("float(53)", Affinity::Real),
    ("NUMERIC", Affinity::Numeric),
    ("DECIMAL(10,5)", Affinity::Numeric),
    ("BOOLEAN", Affinity::Numeric),
    ("DATE", Affinity::Numeric),
    ("DATETIME", Affinity::Numeric),
    ("STRING", Affinity::Numeric),
    ("ANY", Affinity::Numeric),
    ("JSON", Affinity::Numeric),
    ("numeric(5)", Affinity::Numeric),
    ("TIMESTAMP", Affinity::Numeric),
];

/// Column `c`'s affinity: INTEGER for an INTEGER PRIMARY KEY; in a STRICT
/// table INTEGER for INT and INTEGER, REAL, TEXT, and none for BLOB and
/// ANY; else its declared type's ([`DECLS`]), none without one.
pub fn affinity(t: &Table, c: usize) -> Affinity {
    if matches!(t.key, Key::IntegerPrimaryKey(k) if k == c) {
        return Affinity::Integer;
    }
    match &t.strict {
        Some(types) => match types[c] {
            ColType::Int | ColType::Integer => Affinity::Integer,
            ColType::Real => Affinity::Real,
            ColType::Text => Affinity::Text,
            ColType::Blob | ColType::Any => Affinity::None,
        },
        None => t.decl.get(c).copied().flatten().map_or(Affinity::None, |d| DECLS[d].1),
    }
}

/// How SQLite reads a text as a number, as `applyNumericAffinity` does
/// (`sqlite3AtoF`: leading and trailing ASCII whitespace, a sign, digits
/// with a decimal point before, among or after them, an exponent; no hex,
/// no `inf`): not a number, an integer (no point, no exponent), a real,
/// or unsure, where the model leaves it out (more than 15 significant
/// digits, or an exponent past 20 either way, where rounding or range
/// might differ; an integer of more than 18 digits).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Numeric {
    Not,
    Int(i64),
    Real(f64),
    Unsure,
}

pub fn numeric_text(s: &str) -> Numeric {
    let b = s.as_bytes();
    let ws = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    let digits = |mut i: usize| {
        let at = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        (i, i - at)
    };
    let mut i = 0;
    while i < b.len() && ws(b[i]) {
        i += 1;
    }
    let neg = i < b.len() && b[i] == b'-';
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_at = i;
    let (mut j, int_n) = digits(i);
    let (mut frac_at, mut frac_n, mut point) = (j, 0, false);
    if j < b.len() && b[j] == b'.' {
        let (k, n) = digits(j + 1);
        if int_n == 0 && n == 0 {
            return Numeric::Not;
        }
        (frac_at, frac_n, point) = (j + 1, n, true);
        j = k;
    } else if int_n == 0 {
        return Numeric::Not;
    }
    let mut exp: i64 = 0;
    let mut has_exp = false;
    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
        let mut k = j + 1;
        let eneg = k < b.len() && b[k] == b'-';
        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
        }
        let (end, n) = digits(k);
        if n > 0 {
            if n > 4 {
                return Numeric::Unsure;
            }
            exp = s[k..end].parse::<i64>().unwrap_or(0);
            if eneg {
                exp = -exp;
            }
            has_exp = true;
            j = end;
        }
    }
    let mut k = j;
    while k < b.len() && ws(b[k]) {
        k += 1;
    }
    if k != b.len() {
        return Numeric::Not;
    }
    let int_part = &s[int_at..int_at + int_n];
    let frac_part = &s[frac_at..frac_at + frac_n];
    if !point && !has_exp {
        if int_n > 18 {
            return Numeric::Unsure;
        }
        let v: i64 = int_part.parse().unwrap_or(0);
        return Numeric::Int(if neg { -v } else { v });
    }
    let all = format!("{int_part}{frac_part}");
    let significant = all.trim_start_matches('0').trim_end_matches('0').len();
    if significant > 15 || exp.abs() > 20 || all.len() > 40 {
        return Numeric::Unsure;
    }
    let text = format!("{}{}.{}e{exp}", if neg { "-" } else { "" }, if int_n == 0 { "0" } else { int_part }, if frac_n == 0 { "0" } else { frac_part });
    match text.parse::<f64>() {
        Ok(x) if x.is_finite() => Numeric::Real(x),
        _ => Numeric::Unsure,
    }
}

/// `sqlite3VdbeIntegerAffinity`: a real that is an integer strictly within
/// the 64-bit range becomes that integer (`-0.0` becomes 0).
pub fn int_affinity(x: f64) -> Value {
    if x.fract() == 0.0 && x > -9_223_372_036_854_775_808.0 && x < 9_223_372_036_854_775_808.0 {
        Value::Int(x as i64)
    } else {
        Value::Real(x)
    }
}

/// The text SQLite renders a real as (`%!.17g`), for the reals the model
/// renders: finite, below 2^20 in magnitude, and a multiple of 1/64, whose
/// decimal expansion is short and exact (`3.0`, `-0.015625`; `-0.0` is
/// `0.0`). `None` for any other real.
pub fn real_text(x: f64) -> Option<String> {
    if !x.is_finite() || x.abs() >= 1_048_576.0 || (x * 64.0).fract() != 0.0 {
        return None;
    }
    Some(if x.fract() == 0.0 { format!("{}.0", x as i64) } else { format!("{x}") })
}

/// The value SQLite stores when `v` is written to a column of affinity
/// `aff` (in a table that is not STRICT), and reads back: TEXT renders
/// numbers as text; INTEGER and NUMERIC read text that is a number as it
/// ([`numeric_text`]) and an integral real in range as an integer
/// ([`int_affinity`]); REAL reads such text and integers as reals (stored
/// as integers when integral and small, see [`stored`]; `-0.0` comes back
/// as `0.0`); none converts nothing; blobs and NULL are never converted.
/// `None` when the model cannot tell: text it is unsure of under a numeric
/// affinity, a real it does not render under TEXT.
pub fn store(aff: Affinity, v: &Value) -> Option<Value> {
    Some(match (aff, v) {
        (_, Value::Null | Value::Blob(_)) | (Affinity::None, _) => v.clone(),
        (Affinity::Text, Value::Int(i)) => Value::Text(i.to_string()),
        (Affinity::Text, Value::Real(x)) => Value::Text(real_text(*x)?),
        (Affinity::Text, Value::Text(_)) => v.clone(),
        (Affinity::Numeric | Affinity::Integer, Value::Int(_)) => v.clone(),
        (Affinity::Numeric | Affinity::Integer, Value::Real(x)) => int_affinity(*x),
        (Affinity::Real, Value::Int(i)) => Value::Real(*i as f64),
        (Affinity::Real, Value::Real(x)) => Value::Real(if *x == 0.0 { 0.0 } else { *x }),
        (_, Value::Text(s)) => match numeric_text(s) {
            Numeric::Not => v.clone(),
            Numeric::Unsure => return None,
            Numeric::Int(i) if aff == Affinity::Real => Value::Real(i as f64),
            Numeric::Int(i) => Value::Int(i),
            Numeric::Real(x) if aff == Affinity::Real => Value::Real(if x == 0.0 { 0.0 } else { x }),
            Numeric::Real(x) => int_affinity(x),
        },
    })
}

/// [`store`] made total, for a file's model: a value the model cannot
/// convert becomes one SQLite could have stored there (a real's own
/// rendering as text under TEXT, NULL for unsure text).
pub fn store_total(aff: Affinity, v: &Value) -> Value {
    store(aff, v).unwrap_or_else(|| match v {
        Value::Real(x) if aff == Affinity::Text => Value::Text(format!("{x:?}")),
        _ => Value::Null,
    })
}

/// Whether a value may sit in a column of affinity `aff` (not STRICT):
/// storing it again changes nothing (to the bit), [`store`]'s fixed points.
pub fn settled(aff: Affinity, v: &Value) -> bool {
    match (aff, v) {
        (_, Value::Null | Value::Blob(_)) | (Affinity::None, _) | (Affinity::Text, Value::Text(_)) => true,
        (Affinity::Text, _) | (Affinity::Real, Value::Int(_)) => false,
        (Affinity::Numeric | Affinity::Integer, Value::Int(_)) => true,
        (Affinity::Numeric | Affinity::Integer, Value::Real(x)) => matches!(int_affinity(*x), Value::Real(_)),
        (Affinity::Real, Value::Real(x)) => !(*x == 0.0 && x.is_sign_negative()),
        (_, Value::Text(s)) => numeric_text(s) == Numeric::Not,
    }
}

/// How SQLite stores a value in a column: a real in a column of REAL
/// affinity (STRICT or not) with an integral value in the 6-byte range is
/// stored as that integer (`OP_MakeRecord`'s `MEM_IntReal`), and read back
/// as a real.
pub fn stored(table: &Table, c: usize, v: &Value) -> Value {
    match v {
        Value::Real(x) if real_as_int(*x).is_some() && affinity(table, c) == Affinity::Real => Value::Int(real_as_int(*x).unwrap()),
        _ => v.clone(),
    }
}

fn real_as_int(x: f64) -> Option<i64> {
    let lo = -140_737_488_355_328.0;
    let hi = 140_737_488_355_327.0;
    (x.fract() == 0.0 && (lo..=hi).contains(&x)).then_some(x as i64)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// At least 1.
    pub columns: usize,
    pub key: Key,
    /// Each row has `columns` values. Rowid tables: strictly increasing
    /// rowids. WITHOUT ROWID tables: strictly increasing primary keys, with
    /// no NULL in them, and rowids 1, 2, … (unused).
    pub rows: Vec<Row>,
    pub indexes: Vec<Index>,
    /// A STRICT table: one declared type per column. An INTEGER PRIMARY
    /// KEY column is INTEGER; every value is admitted by its column's type.
    pub strict: Option<Vec<ColType>>,
    /// NOT NULL per column (`columns` of them); such a column has no NULL.
    pub not_null: Vec<bool>,
    /// Per column (`columns` of them), its declared type in a table that is
    /// not STRICT, an index into [`DECLS`] (none for an INTEGER PRIMARY
    /// KEY, declared INTEGER, and in a STRICT table); every value is as
    /// its affinity stores it ([`settled`]).
    pub decl: Vec<Option<usize>>,
    /// Per column (`columns` of them), its `COLLATE` clause, if any
    /// ([`column_coll`]): a WITHOUT ROWID primary key and the table's
    /// indexes order its text by it, UNIQUE tells text apart by it, and a
    /// comparison of the column with a literal uses it.
    pub coll: Vec<Option<Coll>>,
    /// Per column (`columns` of them), the value a row holds there when
    /// no statement wrote it: its `DEFAULT` as the column stores it (see
    /// [`store`]), NULL without one. Only `ALTER TABLE .. ADD COLUMN`
    /// gives one (rows from before it read it back); files have none.
    pub default: Vec<Option<Value>>,
    /// Named `CHECK` constraints (`CONSTRAINT k{j} CHECK(p)`, `j` the
    /// position), added by `ALTER TABLE .. ADD CONSTRAINT`: no row makes
    /// one false. Files have none.
    pub checks: Vec<Pred>,
    /// Foreign keys of the table as a child (`FOREIGN KEY(..) REFERENCES
    /// t{parent}(..)`, see [`ForeignKey`]). Files have none.
    pub fks: Vec<ForeignKey>,
    /// Per column (`columns` of them), the column a generated column
    /// copies (`c{k} AS (c{s}) VIRTUAL`, added by ADD COLUMN, without a
    /// type, so that it holds the value itself); no statement writes one.
    /// Files have none.
    pub generated: Vec<Option<usize>>,
    /// `PRIMARY KEY .. ON CONFLICT x` (an INTEGER PRIMARY KEY or a WITHOUT
    /// ROWID primary key): how a conflict on the key is resolved when the
    /// statement says nothing. Files have none.
    pub key_conflict: Option<crate::statements::Resolve>,
    /// The schema it lives in (main for a file's tables).
    pub db: Db,
    /// The named constraints of its SQL in the order SQLite's
    /// `dropConstraintFunc` meets them (see [`Named`]); the checks
    /// (`checks`) apart. Filled by `crate::cases::read` and the statements
    /// that create or alter it, never by the file reader.
    pub names: Vec<Named>,
}

/// A named constraint of a table's SQL (`CONSTRAINT nn2 NOT NULL`, see
/// `crate::writer::constraint_name`), which `ALTER TABLE .. DROP
/// CONSTRAINT` finds by its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Named {
    /// As written, unquoted (`nn2`).
    pub name: String,
    /// The column it is a constraint of; none for a table constraint
    /// (after every column's in the SQL).
    pub column: Option<usize>,
    pub cons: Cons,
}

/// What dropping a [`Named`] constraint drops.
#[derive(Clone, Debug, PartialEq)]
pub enum Cons {
    /// Its column's NOT NULL.
    NotNull,
    /// The foreign key of these child columns.
    Fk(Vec<usize>),
    /// Only the name: a name before `COLLATE`, `DEFAULT` or `AS`, which
    /// SQLite drops alone.
    Bare,
    /// A PRIMARY KEY, which SQLite refuses to drop (`constraint may not
    /// be dropped`); never drawn.
    Fixed,
}

/// What a foreign key does to its child rows when their parent row is
/// deleted, or its key updated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NoAction,
    Restrict,
    SetNull,
    SetDefault,
    Cascade,
}

impl Action {
    pub const ALL: [Action; 5] = [Action::NoAction, Action::Restrict, Action::SetNull, Action::SetDefault, Action::Cascade];

    pub fn sql(self) -> &'static str {
        match self {
            Action::NoAction => "NO ACTION",
            Action::Restrict => "RESTRICT",
            Action::SetNull => "SET NULL",
            Action::SetDefault => "SET DEFAULT",
            Action::Cascade => "CASCADE",
        }
    }
}

/// `FOREIGN KEY(c..) REFERENCES t{parent}(p..) ON DELETE a ON UPDATE b
/// [DEFERRABLE INITIALLY DEFERRED]`: child columns `columns` (none part
/// of the child's key), parent columns `parent_columns` in the same
/// order, the parent's PRIMARY KEY or the columns of one of its UNIQUE
/// indexes, each pair of the same affinity and collating sequence (so
/// that a child value matches a parent value exactly when they compare
/// equal under it).
#[derive(Clone, Debug, PartialEq)]
pub struct ForeignKey {
    pub columns: Vec<usize>,
    pub parent: usize,
    pub parent_columns: Vec<usize>,
    pub on_delete: Action,
    pub on_update: Action,
    pub deferred: bool,
}

impl Table {
    /// The table without its rows.
    pub fn clone_shape(&self) -> Table {
        Table {
            columns: self.columns,
            key: self.key.clone(),
            rows: Vec::new(),
            indexes: self.indexes.clone(),
            strict: self.strict.clone(),
            not_null: self.not_null.clone(),
            decl: self.decl.clone(),
            coll: self.coll.clone(),
            default: self.default.clone(),
            checks: self.checks.clone(),
            fks: self.fks.clone(),
            generated: self.generated.clone(),
            key_conflict: self.key_conflict,
            db: self.db,
            names: self.names.clone(),
        }
    }

    pub fn new(columns: usize, key: Key) -> Self {
        Table {
            columns,
            key,
            rows: Vec::new(),
            indexes: Vec::new(),
            strict: None,
            not_null: vec![false; columns],
            decl: vec![None; columns],
            coll: vec![None; columns],
            default: vec![None; columns],
            checks: Vec::new(),
            fks: Vec::new(),
            generated: vec![None; columns],
            key_conflict: None,
            db: Db::Main,
            names: Vec::new(),
        }
    }
}

/// How a table is keyed.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// A rowid table.
    Rowid,
    /// A rowid table whose column is an INTEGER PRIMARY KEY, an alias of
    /// the rowid: its value in the model is NULL, as in the record.
    IntegerPrimaryKey(usize),
    /// A WITHOUT ROWID table with this PRIMARY KEY over distinct columns.
    WithoutRowid(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub rowid: i64,
    pub values: Vec<Value>,
}

/// An index over distinct columns, each ascending or descending (`true`).
/// Statements also create two kinds the file generator never draws and
/// the writer never writes ([`normalize`] makes a file's indexes plain):
/// an expression index, whose first term is a function of its first
/// column ([`KeyExpr`]), and a partial index, of the rows its `filter`
/// holds for. SQLite builds and maintains both; the model only needs to
/// know they are there (the planner may use them, VACUUM keeps rowids of
/// a table with any index, `INDEXED BY` a partial one may find no plan).
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub columns: Vec<(usize, bool)>,
    /// A UNIQUE index: no two rows agree on its columns unless one of
    /// them is NULL there ([`unique_ok`]). Only a plain index (no
    /// expression, no filter).
    pub unique: bool,
    /// The first term is this function of the first column.
    pub expr: Option<KeyExpr>,
    /// `WHERE filter`: a partial index.
    pub filter: Option<Pred>,
    /// `COLLATE` clauses of index columns, by column (each a column of
    /// the index, but not the expression's, at most once; see
    /// [`index_coll`]).
    pub coll: Vec<(usize, Coll)>,
}

impl Index {
    /// A plain index over these columns.
    pub fn of(columns: Vec<(usize, bool)>) -> Self {
        Index { columns, unique: false, expr: None, filter: None, coll: Vec::new() }
    }

    /// Whether its `COLLATE` clauses name distinct columns of it, not the
    /// one an expression replaces.
    pub fn coll_ok(&self) -> bool {
        self.coll.iter().enumerate().all(|(k, &(c, _))| {
            self.columns.iter().position(|&(x, _)| x == c).is_some_and(|p| p > 0 || self.expr.is_none()) && !self.coll[..k].iter().any(|&(x, _)| x == c)
        })
    }
}

/// The first term of an expression index: a function of a column `c`
/// that raises no error on any value (so neither creating the index nor
/// writing the table fails), rendered as the queries render it
/// (`crate::expr::key_expr`), so that the planner matches a query's term
/// to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyExpr {
    /// `(c + 1)`: an integer that overflows becomes a real, no error.
    Plus,
    /// `lower(c)`.
    Lower,
    /// `length(c)`.
    Length,
    /// `CAST(c AS INTEGER)`.
    Integer,
    /// `date(c)`: NULL for a value that is no time value (and no value
    /// reads as the current time, [`reads_as_now`], which SQLite refuses
    /// in an index).
    Date,
    /// `unixepoch(c)`.
    Unixepoch,
    /// `strftime(f, c)`, `f` every conversion
    /// (`crate::expr::ALL_CONVERSIONS`).
    Strftime,
    /// `datetime(c, 'start of month', '+1 month', '-1 day', 'subsec')`: the
    /// last moment of the month, to the millisecond.
    Datetime,
    /// `timediff(c, '2024-01-15 12:00:00')`.
    Timediff,
    /// `format(f, c, datetime(c), julianday(c), unixepoch(c), date(c), c,
    /// c, ..)`, `f` every `printf` conversion (`crate::expr::PRINTF_ALL`).
    Format,
    /// `julianday(c, ..)` of ten modifiers (`crate::expr::JULIANDAY_CHAIN`).
    Julianday,
    /// `time(c, ..)` of nine (`crate::expr::TIME_CHAIN`).
    Time,
    /// `rtrim(trim(c, ' a0'), 'é1')`.
    Trim,
    /// `replace(replace(c, 'a', 'Bc'), '1', '')`: longer than `c` where it
    /// holds an `a`, shorter where a `1`.
    Replace,
    /// `quote(c)`.
    Quote,
    /// `concat_ws('-', CAST(c AS TEXT), length(c), octet_length(c),
    /// instr(c, 'a'), substr(CAST(c AS TEXT), -3, 2), soundex(c),
    /// length(unhex(c, ' -')), sign(c), round(c, 2), abs(CAST(c AS REAL)),
    /// char(unicode(c)), json_valid(c, 10), json_error_position(c),
    /// json(CASE WHEN json_valid(CAST(c AS TEXT), 2) THEN CAST(c AS TEXT)
    /// END))`.
    ConcatWs,
}

impl KeyExpr {
    pub const ALL: [KeyExpr; 16] = [
        KeyExpr::Plus,
        KeyExpr::Lower,
        KeyExpr::Length,
        KeyExpr::Integer,
        KeyExpr::Date,
        KeyExpr::Unixepoch,
        KeyExpr::Strftime,
        KeyExpr::Datetime,
        KeyExpr::Timediff,
        KeyExpr::Format,
        KeyExpr::Julianday,
        KeyExpr::Time,
        KeyExpr::Trim,
        KeyExpr::Replace,
        KeyExpr::Quote,
        KeyExpr::ConcatWs,
    ];
}

/// Layout flags, each a deliberate departure from what SQLite's own writer
/// does, all still valid.
pub mod flag {
    /// Cell content is placed on the page in a shuffled order, not the
    /// order of the cell pointer array.
    pub const SHUFFLE: u8 = 1;
    /// Free blocks and fragmented bytes are left between cells.
    pub const GAPS: u8 = 2;
    /// Pages are numbered in a random order instead of allocation order.
    pub const SCATTER: u8 = 4;
    /// Table interior keys are any value in the legal range instead of the
    /// left subtree's largest rowid; freelist trunks are partly filled;
    /// sqlite_schema rowids have gaps.
    pub const LOOSE: u8 = 8;
    /// Integers use any serial type wide enough, not the smallest.
    pub const WIDE: u8 = 16;
    /// Table records omit some trailing NULL columns (schema format >= 2).
    pub const SHORT: u8 = 32;
    /// Unused bytes (free space, free blocks, fragments, reserved bytes,
    /// freelist pages, overflow tails) hold random bytes instead of zeros.
    pub const JUNK: u8 = 64;
    pub const ALL: u8 = 127;
}

/// How a model is laid out in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// A power of two in 512..=65536.
    pub page_size: u32,
    /// Bytes reserved at the end of every page; usable size stays >= 480.
    pub reserved: u8,
    /// Schema format number, 1..=4. Below 4 there are no serial types 8 and
    /// 9 and no descending indexes; below 2 no short records.
    pub format: u8,
    /// Header bytes 18 and 19 say WAL.
    pub wal: bool,
    /// Percent of a leaf page's space a leaf is filled to, 1..=100 (at
    /// least one cell per page).
    pub leaf_fill: u8,
    /// The same for interior pages.
    pub interior_fill: u8,
    /// [`flag`] bits.
    pub flags: u8,
    /// Leaves per freelist trunk, at most usable/4 - 2 when written.
    pub trunk_cap: u8,
    /// Pages on the freelist, trunks included.
    pub free_pages: u32,
    /// 0: no auto-vacuum; 1: auto-vacuum (pointer-map pages); 2: incremental
    /// vacuum.
    pub vacuum: u8,
    /// Drives every random layout decision of the writer.
    pub seed: u32,
}

pub const MAX_TABLES: usize = 8;
pub const MAX_COLUMNS: usize = 8;
pub const MAX_INDEXES: usize = 4;
pub const MAX_ROWS: usize = 4000;
pub const MAX_VIEWS: usize = 20;
pub const MAX_TRIGGERS: usize = 20;

impl Layout {
    pub fn usable(&self) -> usize {
        self.page_size as usize - usize::from(self.reserved)
    }

    pub fn has(&self, f: u8) -> bool {
        self.flags & f != 0
    }

    /// The most free pages for a page size: about 160 KiB of them.
    pub fn max_free(page_size: u32) -> u32 {
        (160 * 1024 / page_size).clamp(4, 256)
    }

    /// Clamp every field into its legal range; idempotent.
    pub fn normalized(mut self) -> Self {
        if !self.page_size.is_power_of_two() || !(512..=65536).contains(&self.page_size) {
            self.page_size = 4096;
        }
        self.reserved = self.reserved.min((self.page_size - 480).min(255) as u8);
        self.format = self.format.clamp(1, 4);
        self.leaf_fill = self.leaf_fill.clamp(1, 100);
        self.interior_fill = self.interior_fill.clamp(1, 100);
        self.flags &= flag::ALL;
        self.trunk_cap = self.trunk_cap.max(1);
        self.free_pages = self.free_pages.min(Self::max_free(self.page_size));
        self.vacuum = self.vacuum.min(2);
        self
    }

    /// The simplicity measure of a layout: 0 for SQLite's defaults (4096
    /// byte pages, nothing reserved, format 4, packed pages, no flags, no
    /// free pages, seed 0).
    pub fn cost(&self) -> u64 {
        let log = u64::from(self.page_size.trailing_zeros());
        log.abs_diff(12)
            + u64::from(self.reserved)
            + u64::from(4 - self.format)
            + u64::from(self.wal)
            + u64::from(100 - self.leaf_fill)
            + u64::from(100 - self.interior_fill)
            + u64::from(self.flags.count_ones())
            + u64::from(self.trunk_cap != 255)
            + u64::from(self.free_pages)
            + u64::from(self.seed != 0)
            + u64::from(self.vacuum != 0)
    }

    pub const SIMPLE: Layout = Layout {
        page_size: 4096,
        reserved: 0,
        format: 4,
        wal: false,
        leaf_fill: 100,
        interior_fill: 100,
        flags: 0,
        trunk_cap: 255,
        free_pages: 0,
        seed: 0,
        vacuum: 0,
    };
}

fn bits(n: u64) -> u64 {
    u64::from(64 - n.leading_zeros())
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

impl Value {
    /// The simplicity measure of a value: NULL is 0; then small integers;
    /// reals above every integer; text and blobs by length in bytes.
    pub fn cost(&self) -> u64 {
        match self {
            Value::Null => 0,
            Value::Int(i) => 1 + bits(zigzag(*i)),
            Value::Real(_) => 67,
            Value::Text(s) => 1 + s.len() as u64,
            Value::Blob(b) => 1 + b.len() as u64,
        }
    }

    fn class(&self) -> u8 {
        match self {
            Value::Null => 0,
            Value::Int(_) | Value::Real(_) => 1,
            Value::Text(_) => 2,
            Value::Blob(_) => 3,
        }
    }
}

impl Model {
    /// The measure every shrink step strictly decreases, together with
    /// [`Layout::cost`].
    pub fn cost(&self) -> u64 {
        let mut c = u64::from(self.encoding != Encoding::Utf8);
        for t in &self.tables {
            c += 2 + t.columns as u64;
            c += match &t.key {
                Key::Rowid => 0,
                Key::IntegerPrimaryKey(_) => 1,
                Key::WithoutRowid(pk) => 1 + pk.len() as u64,
            };
            for i in &t.indexes {
                c += 1 + u64::from(i.unique) + i.columns.iter().map(|&(_, d)| 1 + u64::from(d)).sum::<u64>() + i.coll.len() as u64;
            }
            c += t.coll.iter().filter(|k| k.is_some()).count() as u64;
            for r in &t.rows {
                c += 1 + bits(zigzag(r.rowid)) + r.values.iter().map(Value::cost).sum::<u64>();
            }
            if let Some(types) = &t.strict {
                c += 1 + types.iter().filter(|t| **t != ColType::Any).count() as u64;
            }
            c += t.not_null.iter().filter(|n| **n).count() as u64;
            c += t.decl.iter().filter(|d| d.is_some()).count() as u64;
            c += t.default.iter().flatten().map(|v| 1 + v.cost()).sum::<u64>();
            c += t.checks.len() as u64;
            c += t.generated.iter().flatten().count() as u64;
            c += t.fks.iter().map(|f| 1 + f.columns.len() as u64 + u64::from(f.on_delete != Action::NoAction) + u64::from(f.on_update != Action::NoAction) + u64::from(f.deferred)).sum::<u64>();
        }
        for v in &self.views {
            c += 1 + u64::from(v.table.is_some()) + v.columns.len() as u64 + v.pad as u64;
        }
        for g in &self.triggers {
            c += 1 + u64::from(g.after) + u64::from(g.event) + u64::from(g.column.is_some());
        }
        c
    }

    /// The invariants the writer relies on.
    pub fn well_formed(&self) -> bool {
        let distinct = |cols: &[usize], n: usize| cols.iter().enumerate().all(|(k, &c)| c < n && !cols[..k].contains(&c));
        let views_ok = self.views.iter().all(|v| match v.table {
            Some(t) => t < self.tables.len() && distinct(&v.columns, self.tables[t].columns),
            None => v.columns.is_empty(),
        });
        let triggers_ok = self.triggers.iter().all(|g| {
            g.table < self.tables.len() && g.event <= 2 && g.column.is_none_or(|c| c < self.tables[g.table].columns)
        });
        views_ok && triggers_ok && self.tables.iter().all(|t| {
            let typed = match &t.strict {
                None => true,
                Some(types) => {
                    types.len() == t.columns
                        && !matches!(t.key, Key::IntegerPrimaryKey(k) if types[k] != ColType::Integer)
                        && t.rows.iter().all(|r| r.values.iter().zip(types).all(|(v, ty)| ty.admits(v)))
                        && t.rows.iter().all(|r| {
                            r.values.iter().zip(types).all(|(v, ty)| *ty != ColType::Real || !matches!(v, Value::Real(x) if *x == 0.0 && x.is_sign_negative()))
                        })
                },
            };
            let not_null_ok = t.not_null.len() == t.columns
                && t.rows.iter().all(|r| r.values.iter().zip(&t.not_null).all(|(v, n)| !*n || *v != Value::Null));
            let decl_ok = t.decl.len() == t.columns
                && t.decl.iter().enumerate().all(|(c, d)| {
                    d.is_none_or(|d| d < DECLS.len() && t.strict.is_none() && !matches!(t.key, Key::IntegerPrimaryKey(k) if k == c))
                })
                && (t.strict.is_some()
                    || (0..t.columns).all(|c| {
                        let aff = affinity(t, c);
                        aff == Affinity::None || matches!(t.key, Key::IntegerPrimaryKey(k) if k == c) || t.rows.iter().all(|r| r.values.get(c).is_none_or(|v| settled(aff, v)))
                    }));
            let coll_ok = t.coll.len() == t.columns && t.indexes.iter().all(Index::coll_ok) && t.default.len() == t.columns && t.generated.len() == t.columns;
            typed && not_null_ok && decl_ok && coll_ok && {
            let key_ok = match &t.key {
                Key::Rowid => t.rows.windows(2).all(|w| w[0].rowid < w[1].rowid),
                Key::IntegerPrimaryKey(c) => {
                    *c < t.columns
                        && t.rows.windows(2).all(|w| w[0].rowid < w[1].rowid)
                        && t.rows.iter().all(|r| r.values.get(*c) == Some(&Value::Null))
                }
                Key::WithoutRowid(pk) => {
                    !pk.is_empty()
                        && distinct(pk, t.columns)
                        && t.rows.iter().enumerate().all(|(k, r)| {
                            r.rowid == k as i64 + 1 && pk.iter().all(|&c| r.values.get(c).is_some_and(|v| *v != Value::Null))
                        })
                        && t.rows.windows(2).all(|w| compare_key(t, &w[0], &w[1], pk, self.encoding) == Ordering::Less)
                }
            };
            t.columns >= 1
                && key_ok
                && t.rows.iter().all(|r| {
                    r.values.len() == t.columns && r.values.iter().all(|v| !matches!(v, Value::Real(x) if x.is_nan()))
                })
                && t.indexes.iter().all(|i| {
                    let cols: Vec<usize> = i.columns.iter().map(|&(c, _)| c).collect();
                    !cols.is_empty() && distinct(&cols, t.columns) && (!i.unique || unique_ok(t, i, &t.rows, self.encoding))
                })
            }
        })
    }
}

/// Two rows' order by a WITHOUT ROWID primary key, each column under its
/// collating sequence.
pub fn compare_key(t: &Table, a: &Row, b: &Row, pk: &[usize], enc: Encoding) -> Ordering {
    pk.iter().map(|&c| compare_coll(&a.values[c], &b.values[c], enc, column_coll(t, c))).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal)
}

/// Make a model and a layout agree, and a model well formed: a value in a
/// column with a declared type is as its affinity stores it
/// ([`store_total`]), and only tables that are not STRICT declare types
/// ([`Table::decl`]; an INTEGER PRIMARY KEY is declared INTEGER); indexes are
/// plain (no expression, no filter), and UNIQUE only where the rows allow
/// it; below format 4 there are no
/// descending index columns; an INTEGER PRIMARY KEY column
/// holds NULL (in a STRICT table it is INTEGER, or the key goes); a value
/// its STRICT column does not admit is NULL; a NOT NULL column with a NULL
/// loses the constraint; a WITHOUT ROWID table loses rows with a NULL or
/// repeated primary key and is put in key order; views and triggers lose
/// references that do not resolve, and pads are at most the usable size;
/// no value reads as the current time to SQLite's date functions
/// ([`not_now`]).
/// Never raises the cost, but where an affinity converts a value (text
/// that reads as a number, a number under TEXT).
pub fn normalize(model: &mut Model, layout: &mut Layout) {
    *layout = layout.normalized();
    let enc = model.encoding;
    let tables = model.tables.len();
    model.views.truncate(MAX_VIEWS);
    model.triggers.truncate(MAX_TRIGGERS);
    for v in &mut model.views {
        v.pad = v.pad.min(layout.usable());
        match v.table {
            Some(t) if t < tables => {
                let n = model.tables[t].columns;
                let mut seen = Vec::new();
                v.columns.retain(|&c| {
                    let keep = c < n && !seen.contains(&c);
                    seen.push(c);
                    keep
                });
            },
            _ => {
                v.table = None;
                v.columns.clear();
            },
        }
    }
    model.triggers.retain(|g| g.table < tables && g.event <= 2);
    for g in &mut model.triggers {
        if g.column.is_some_and(|c| c >= model.tables[g.table].columns) {
            g.column = None;
        }
    }
    for t in &mut model.tables {
        for r in &mut t.rows {
            for v in &mut r.values {
                if reads_as_now(v) {
                    *v = not_now(std::mem::replace(v, Value::Null));
                }
            }
        }
        t.not_null.resize(t.columns, false);
        t.decl.resize(t.columns, None);
        t.coll.resize(t.columns, None);
        t.default = vec![None; t.columns];
        t.checks.clear();
        t.fks.clear();
        t.generated = vec![None; t.columns];
        for i in &mut t.indexes {
            let cols: Vec<usize> = i.columns.iter().map(|&(c, _)| c).collect();
            let mut seen = Vec::new();
            i.coll.retain(|&(c, _)| {
                let keep = cols.contains(&c) && !seen.contains(&c);
                seen.push(c);
                keep
            });
        }
        if t.strict.is_some() {
            t.decl.fill(None);
        }
        if let Key::IntegerPrimaryKey(k) = t.key {
            t.decl[k] = None;
        }
        for c in 0..t.columns {
            if t.decl[c].is_some_and(|d| d >= DECLS.len()) {
                t.decl[c] = None;
            }
            let aff = affinity(t, c);
            if t.strict.is_none() && aff != Affinity::None && !matches!(t.key, Key::IntegerPrimaryKey(k) if k == c) {
                for r in &mut t.rows {
                    if !settled(aff, &r.values[c]) {
                        r.values[c] = store_total(aff, &r.values[c]);
                    }
                }
            }
        }
        if let Some(types) = &mut t.strict {
            types.resize(t.columns, ColType::Any);
            if matches!(t.key, Key::IntegerPrimaryKey(k) if types[k] != ColType::Integer) {
                t.key = Key::Rowid;
            }
            for r in &mut t.rows {
                for (v, ty) in r.values.iter_mut().zip(types.iter()) {
                    if !ty.admits(v) {
                        *v = Value::Null;
                    } else if let Value::Real(x) = v {
                        if *ty == ColType::Real && *x == 0.0 {
                            *x = 0.0;
                        }
                    }
                }
            }
        }
        for i in &mut t.indexes {
            i.expr = None;
            i.filter = None;
        }
        if layout.format < 4 {
            for i in &mut t.indexes {
                for c in &mut i.columns {
                    c.1 = false;
                }
            }
        }
        match &t.key {
            Key::Rowid => {},
            Key::IntegerPrimaryKey(c) => {
                for r in &mut t.rows {
                    r.values[*c] = Value::Null;
                }
            },
            Key::WithoutRowid(pk) => {
                let pk = pk.clone();
                let mut rows = std::mem::take(&mut t.rows);
                rows.retain(|r| pk.iter().all(|&c| r.values[c] != Value::Null));
                rows.sort_by(|a, b| compare_key(t, a, b, &pk, enc));
                rows.dedup_by(|b, a| compare_key(t, a, b, &pk, enc) == Ordering::Equal);
                for (k, r) in rows.iter_mut().enumerate() {
                    r.rowid = k as i64 + 1;
                }
                t.rows = rows;
            },
        }
        for (c, n) in t.not_null.iter_mut().enumerate() {
            if *n && t.rows.iter().any(|r| r.values[c] == Value::Null) {
                *n = false;
            }
        }
        for k in 0..t.indexes.len() {
            if t.indexes[k].unique && !unique_ok(t, &t.indexes[k], &t.rows, enc) {
                t.indexes[k].unique = false;
            }
        }
    }
}

/// Whether `rows` of `table` agree on the columns of `index` only where
/// one of them is NULL there: what a UNIQUE index over the columns allows.
/// Values compare as SQLite compares them in the index (`1` and `1.0` are
/// equal; texts under the index's collating sequences, `'a'` and `'A'`
/// under NOCASE).
pub fn unique_ok(table: &Table, index: &Index, rows: &[Row], enc: Encoding) -> bool {
    let colls: Vec<Coll> = index.columns.iter().map(|&(c, _)| index_coll(table, index, c)).collect();
    let mut keys: Vec<Vec<Value>> = rows
        .iter()
        .map(|r| index.columns.iter().map(|&(c, _)| stored(table, c, &column_value(table, r, c))).collect::<Vec<_>>())
        .filter(|k| !k.contains(&Value::Null))
        .collect();
    sort_entries(&mut keys, &vec![false; index.columns.len()], &colls, enc);
    keys.windows(2).all(|w| w[0].iter().zip(&w[1]).zip(&colls).any(|((a, b), k)| compare_coll(a, b, enc, *k) != Ordering::Equal))
}

/// SQLite's integer-to-real comparison (`sqlite3IntFloatCompare`, without
/// long double, as on this platform).
pub fn int_real(i: i64, r: f64) -> Ordering {
    if r < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    if r >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    let y = r as i64;
    match i.cmp(&y) {
        Ordering::Equal => (i as f64).partial_cmp(&r).unwrap_or(Ordering::Equal),
        o => o,
    }
}

/// SQLite's order of values under the BINARY collation: NULL, numbers (by
/// value), text (memcmp of the encoded bytes), blobs (memcmp).
pub fn compare(a: &Value, b: &Value, enc: Encoding) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Real(x), Value::Real(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Int(x), Value::Real(y)) => int_real(*x, *y),
        (Value::Real(x), Value::Int(y)) => int_real(*y, *x).reverse(),
        (Value::Text(x), Value::Text(y)) if enc == Encoding::Utf8 => x.as_bytes().cmp(y.as_bytes()),
        (Value::Text(x), Value::Text(y)) => enc.encode(x).cmp(&enc.encode(y)),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => a.class().cmp(&b.class()),
    }
}

/// A row's value in column `c`: the rowid for an INTEGER PRIMARY KEY.
/// Whether SQLite's date and time functions read `v` as the current time
/// (`parseDateOrTime`: `now`, `subsec` or `subsecond`, in any case): text
/// up to its first NUL, or a blob's bytes, which SQLite reads as text in
/// the database's encoding (any of the three here). In an index, a CHECK
/// constraint or a generated column a date function of such a value is
/// "non-deterministic use of date() in an index" (deterministic.html), and
/// in a query it is the time each statement runs at. No value the
/// generators write reads so ([`not_now`]): text pools spell none, and a
/// random blob that does (three bytes `now` in any case, one in 2^21) is
/// changed.
pub fn reads_as_now(v: &Value) -> bool {
    let word = |s: &str| ["now", "subsec", "subsecond"].iter().any(|w| s.eq_ignore_ascii_case(w));
    match v {
        Value::Text(s) => word(s.split('\0').next().unwrap_or("")),
        Value::Blob(b) => {
            let utf16 = |le: bool| -> String {
                let units = b.as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
                units.take_while(|&u| u != 0).map(|u| char::from_u32(u32::from(u)).unwrap_or('\u{fffd}')).collect()
            };
            word(&String::from_utf8_lossy(b.split(|&x| x == 0).next().unwrap_or(&[]))) || word(&utf16(true)) || word(&utf16(false))
        },
        _ => false,
    }
}

/// `v`, or, when it reads as the current time ([`reads_as_now`]), the
/// same with its first byte one more: then no encoding reads it as a word
/// (each starts with `n` or `s`, a NUL byte first in UTF-16BE).
pub fn not_now(v: Value) -> Value {
    if !reads_as_now(&v) {
        return v;
    }
    match v {
        Value::Blob(mut b) => {
            b[0] = b[0].wrapping_add(1);
            Value::Blob(b)
        },
        Value::Text(s) => Value::Text(format!("+{s}")),
        v => v,
    }
}

pub fn column_value(table: &Table, row: &Row, c: usize) -> Value {
    match table.key {
        Key::IntegerPrimaryKey(k) if k == c => Value::Int(row.rowid),
        _ => row.values[c].clone(),
    }
}

/// The columns of a WITHOUT ROWID table's records: the primary key, then
/// the other columns in order.
pub fn record_columns(table: &Table) -> Vec<usize> {
    match &table.key {
        Key::WithoutRowid(pk) => pk.iter().copied().chain((0..table.columns).filter(|c| !pk.contains(c))).collect(),
        _ => (0..table.columns).collect(),
    }
}

/// The index entries of `index` over `table`, in index order: the indexed
/// values, then the rowid (rowid tables) or the primary key columns not
/// already indexed under their own collating sequence (WITHOUT ROWID
/// tables), which sort ascending.
pub fn index_entries(table: &Table, index: &Index, enc: Encoding) -> Vec<Vec<Value>> {
    let mut desc: Vec<bool> = index.columns.iter().map(|&(_, d)| d).collect();
    let suffix: Vec<usize> = match &table.key {
        // A key column is in the index already only under the key's
        // collating sequence (`isDupColumn`).
        Key::WithoutRowid(pk) => pk
            .iter()
            .copied()
            .filter(|c| !index.columns.iter().any(|&(x, _)| x == *c && index_coll(table, index, x) == column_coll(table, x)))
            .collect(),
        _ => Vec::new(),
    };
    let rowid = !matches!(table.key, Key::WithoutRowid(_));
    desc.extend(std::iter::repeat_n(false, suffix.len() + usize::from(rowid)));
    let colls: Vec<Coll> = index
        .columns
        .iter()
        .map(|&(c, _)| index_coll(table, index, c))
        .chain(suffix.iter().map(|&c| column_coll(table, c)))
        .chain(rowid.then_some(Coll::Binary))
        .collect();
    let mut entries: Vec<Vec<Value>> = table
        .rows
        .iter()
        .map(|r| {
            let mut e: Vec<Value> = index.columns.iter().map(|&(c, _)| stored(table, c, &column_value(table, r, c))).collect();
            e.extend(suffix.iter().map(|&c| stored(table, c, &r.values[c])));
            if rowid {
                e.push(Value::Int(r.rowid));
            }
            e
        })
        .collect();
    sort_entries(&mut entries, &desc, &colls, enc);
    entries
}

/// Sort records by their values, each position ascending or descending
/// and under its collating sequence.
pub fn sort_entries(entries: &mut [Vec<Value>], desc: &[bool], colls: &[Coll], enc: Encoding) {
    // Text compares by its key bytes (encoded, or under its sequence,
    // [`Coll::key`]); compute each once.
    type Keyed = (Vec<Option<Vec<u8>>>, Vec<Value>);
    let mut keyed: Vec<Keyed> = entries
        .iter_mut()
        .map(|e| {
            let keys = e
                .iter()
                .enumerate()
                .map(|(k, v)| match (v, colls.get(k).copied().unwrap_or(Coll::Binary)) {
                    (Value::Text(s), Coll::Binary) if enc != Encoding::Utf8 => Some(enc.encode(s)),
                    (Value::Text(s), coll) if coll != Coll::Binary => Some(coll.key(s.as_bytes()).into_owned()),
                    _ => None,
                })
                .collect();
            (keys, std::mem::take(e))
        })
        .collect();
    keyed.sort_by(|(ka, a), (kb, b)| {
        for k in 0..a.len().min(b.len()) {
            let o = match (&ka[k], &kb[k]) {
                (Some(x), Some(y)) => x.cmp(y),
                _ => compare(&a[k], &b[k], Encoding::Utf8),
            };
            if o != Ordering::Equal {
                return if desc[k] { o.reverse() } else { o };
            }
        }
        Ordering::Equal
    });
    for (slot, (_, e)) in entries.iter_mut().zip(keyed) {
        *slot = e;
    }
}
