// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The logical database a setup creates and the statements change
//! ([`Model`]): tables of typed columns, their rows (a multiset: PostgreSQL
//! has no rowid), indexes, CHECK constraints, identity sequences and open
//! savepoints. Everything is named by a stable id (`t{id}`, `c{id}`,
//! `t{t}_i{id}`), so that statements stay meaningful when other tables,
//! columns or indexes are dropped before them.
//!
//! Values ([`Value`]) are those of the column types in [`Type`], compared as
//! PostgreSQL compares them ([`compare`]): integers of every width alike,
//! reals with `NaN` above every other value and `-0 = 0`, numerics by value
//! (`1.0 = 1.00`, `NaN` above `Infinity`), text by its bytes (the cluster's
//! collation is C), dates and timestamps with their infinities.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::statements::Pred;

/// A column type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Type {
    Int2,
    Int4,
    Int8,
    Float4,
    Float8,
    Numeric,
    Text,
    Bool,
    Bytea,
    Date,
    Timestamp,
    /// Expression-only types (never a column's, never a query's result):
    /// values the query language builds and reads inside expressions.
    Jsonb,
    /// `int8[]` and `text[]`: one-dimensional arrays.
    Int8Array,
    TextArray,
    Interval,
}

impl Type {
    pub const ALL: [Type; 11] = [
        Type::Int2,
        Type::Int4,
        Type::Int8,
        Type::Float4,
        Type::Float8,
        Type::Numeric,
        Type::Text,
        Type::Bool,
        Type::Bytea,
        Type::Date,
        Type::Timestamp,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Type::Int2 => "int2",
            Type::Int4 => "int4",
            Type::Int8 => "int8",
            Type::Float4 => "float4",
            Type::Float8 => "float8",
            Type::Numeric => "numeric",
            Type::Text => "text",
            Type::Bool => "bool",
            Type::Bytea => "bytea",
            Type::Date => "date",
            Type::Timestamp => "timestamp",
            Type::Jsonb => "jsonb",
            Type::Int8Array => "int8[]",
            Type::TextArray => "text[]",
            Type::Interval => "interval",
        }
    }

    /// Whether it is an expression-only type ([`Type::ALL`] has none).
    pub fn internal(self) -> bool {
        matches!(self, Type::Jsonb | Type::Int8Array | Type::TextArray | Type::Interval)
    }

    /// The array type of elements of this type, when the language has it.
    pub fn array(self) -> Option<Type> {
        match self {
            Type::Int8 => Some(Type::Int8Array),
            Type::Text => Some(Type::TextArray),
            _ => None,
        }
    }

    /// The element type of an array type.
    pub fn element(self) -> Option<Type> {
        match self {
            Type::Int8Array => Some(Type::Int8),
            Type::TextArray => Some(Type::Text),
            _ => None,
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self, Type::Int2 | Type::Int4 | Type::Int8)
    }

    pub fn is_float(self) -> bool {
        matches!(self, Type::Float4 | Type::Float8)
    }

    /// Variable-length types, whose values TOAST may compress or move out
    /// of line (and whose STORAGE and COMPRESSION a layout may set).
    pub fn varlena(self) -> bool {
        matches!(self, Type::Numeric | Type::Text | Type::Bytea)
    }

    /// Whether a value of this type is one PostgreSQL stores in a column of
    /// it unchanged: an integer in range, a real a `float4` holds exactly,
    /// and so on. NULL in any.
    pub fn admits(self, v: &Value) -> bool {
        match (self, v) {
            (_, Value::Null) => true,
            (Type::Int2, Value::Int(i)) => i16::try_from(*i).is_ok(),
            (Type::Int4, Value::Int(i)) => i32::try_from(*i).is_ok(),
            (Type::Int8, Value::Int(_)) => true,
            (Type::Float4, Value::Float(x)) => x.is_nan() || f64::from(*x as f32).to_bits() == x.to_bits(),
            (Type::Float8, Value::Float(_)) => true,
            (Type::Numeric, Value::Numeric(_)) => true,
            (Type::Text, Value::Text(s)) => !s.contains('\0'),
            (Type::Bool, Value::Bool(_)) => true,
            (Type::Bytea, Value::Bytea(_)) => true,
            (Type::Date, Value::Date(d)) => date_ok(*d),
            (Type::Timestamp, Value::Timestamp(t)) => timestamp_ok(*t),
            _ => false,
        }
    }

    /// Whether PostgreSQL compares values of `self` with values of `other`
    /// exactly as [`compare`] does: the same type, integers of any width,
    /// reals of either width (a `float4` widens exactly), a date with a
    /// timestamp (at its midnight).
    pub fn comparable(self, other: Type) -> bool {
        self == other
            || self.is_int() && other.is_int()
            || self.is_float() && other.is_float()
            || matches!((self, other), (Type::Date, Type::Timestamp) | (Type::Timestamp, Type::Date))
    }
}

/// An exact decimal as PostgreSQL's `numeric` holds it: its digits and its
/// display scale (`1.50` keeps two), or one of the special values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Numeric {
    NaN,
    Inf,
    NegInf,
    /// `digits` (ASCII, no leading zero; empty for zero) times
    /// `10^-scale`; `neg` is false for zero (PostgreSQL has no negative
    /// zero numeric).
    Finite { neg: bool, digits: Vec<u8>, scale: u16 },
}

impl Numeric {
    /// The integer `i`.
    pub fn int(i: i64) -> Numeric {
        Numeric::finite(i < 0, i.unsigned_abs().to_string().into_bytes(), 0)
    }

    /// A finite numeric, normalized: no leading zeros, zero never negative.
    pub fn finite(neg: bool, digits: Vec<u8>, scale: u16) -> Numeric {
        let first = digits.iter().position(|&d| d != b'0').unwrap_or(digits.len());
        let digits = digits[first..].to_vec();
        Numeric::Finite { neg: neg && !digits.is_empty(), digits, scale }
    }

    /// Parse plain decimal text (`-12.340`), as `numeric_in` reads it.
    pub fn parse(s: &str) -> Option<Numeric> {
        match s {
            "NaN" => return Some(Numeric::NaN),
            "Infinity" => return Some(Numeric::Inf),
            "-Infinity" => return Some(Numeric::NegInf),
            _ => {},
        }
        let (neg, body) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let digits = int.bytes().chain(frac.bytes()).collect();
        Some(Numeric::finite(neg, digits, u16::try_from(frac.len()).ok()?))
    }

    /// Order by value: `-Infinity` < finite < `Infinity` < `NaN`; scale
    /// does not matter.
    pub fn cmp_value(&self, other: &Numeric) -> Ordering {
        let rank = |n: &Numeric| match n {
            Numeric::NegInf => 0,
            Numeric::Finite { .. } => 1,
            Numeric::Inf => 2,
            Numeric::NaN => 3,
        };
        match (self, other) {
            (Numeric::Finite { neg: na, digits: da, scale: sa }, Numeric::Finite { neg: nb, digits: db, scale: sb }) => {
                if na != nb {
                    return if *na { Ordering::Less } else { Ordering::Greater };
                }
                // Both magnitudes scaled to the larger scale.
                let s = (*sa).max(*sb);
                let pad = |d: &[u8], sc: u16| -> Vec<u8> {
                    if d.is_empty() {
                        Vec::new()
                    } else {
                        d.iter().copied().chain(std::iter::repeat_n(b'0', usize::from(s - sc))).collect()
                    }
                };
                let (ma, mb) = (pad(da, *sa), pad(db, *sb));
                let mag = ma.len().cmp(&mb.len()).then_with(|| ma.cmp(&mb));
                if *na {
                    mag.reverse()
                } else {
                    mag
                }
            },
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

impl fmt::Display for Numeric {
    /// As `numeric_out` prints it: every digit of the scale.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Numeric::NaN => write!(f, "NaN"),
            Numeric::Inf => write!(f, "Infinity"),
            Numeric::NegInf => write!(f, "-Infinity"),
            Numeric::Finite { neg, digits, scale } => {
                let scale = usize::from(*scale);
                let mut d: Vec<u8> = digits.clone();
                while d.len() <= scale {
                    d.insert(0, b'0');
                }
                let (int, frac) = d.split_at(d.len() - scale);
                if *neg {
                    write!(f, "-")?;
                }
                write!(f, "{}", std::str::from_utf8(int).unwrap())?;
                if scale > 0 {
                    write!(f, ".{}", std::str::from_utf8(frac).unwrap())?;
                }
                Ok(())
            },
        }
    }
}

/// A column value. Integers of every width are `Int`, reals of both widths
/// `Float` (a `float4` value is the `f64` it widens to exactly).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Numeric(Numeric),
    Text(String),
    Bool(bool),
    Bytea(Vec<u8>),
    /// Days since 2000-01-01; `i32::MIN` is `-infinity`, `i32::MAX`
    /// `infinity` (as PostgreSQL stores them).
    Date(i32),
    /// Microseconds since 2000-01-01 00:00; `i64::MIN` and `i64::MAX` are
    /// the infinities.
    Timestamp(i64),
}

/// The day number of 4714-11-24 BC (Julian day 0), the first date
/// PostgreSQL takes, and of 5874897-12-31, the last.
pub const DATE_MIN: i32 = -2_451_545;
pub const DATE_MAX: i32 = 2_145_031_948;
/// The first and last timestamps: 4714-11-24 00:00 BC and 294276-12-31
/// 23:59:59.999999.
pub const TIMESTAMP_MIN: i64 = -211_813_488_000_000_000;
pub const TIMESTAMP_MAX: i64 = 9_223_371_331_199_999_999;

pub const MICROS_PER_DAY: i64 = 86_400_000_000;

fn date_ok(d: i32) -> bool {
    d == i32::MIN || d == i32::MAX || (DATE_MIN..=DATE_MAX).contains(&d)
}

fn timestamp_ok(t: i64) -> bool {
    t == i64::MIN || t == i64::MAX || (TIMESTAMP_MIN..=TIMESTAMP_MAX).contains(&t)
}

impl Value {
    /// A simplicity measure, for shrinking: NULL is simplest, then short
    /// values.
    pub fn cost(&self) -> u64 {
        fn bits(x: u64) -> u64 {
            u64::from(64 - x.leading_zeros())
        }
        match self {
            Value::Null => 0,
            Value::Int(i) => 1 + bits(i.unsigned_abs()),
            Value::Float(x) => 2 + if x.fract() == 0.0 && x.abs() < 1e15 { bits(x.abs() as u64) } else { 64 },
            Value::Numeric(Numeric::Finite { digits, scale, .. }) => 2 + digits.len() as u64 + u64::from(*scale),
            Value::Numeric(_) => 3,
            Value::Text(s) => 1 + s.len() as u64,
            Value::Bool(b) => 1 + u64::from(*b),
            Value::Bytea(b) => 1 + b.len() as u64,
            Value::Date(d) => 2 + bits(d.unsigned_abs().into()),
            Value::Timestamp(t) => 2 + bits(t.unsigned_abs()),
        }
    }

    /// Bytes the value takes in an index tuple, at most (before any
    /// compression): what the B-tree size limit is checked against.
    pub fn width(&self) -> usize {
        match self {
            Value::Text(s) => s.len() + 4,
            Value::Bytea(b) => b.len() + 4,
            Value::Numeric(_) => 40,
            _ => 8,
        }
    }

    /// The text PostgreSQL reads back as exactly this value, for the type
    /// it is a value of: the input form of a literal (`'..'` around it in
    /// SQL, see [`crate::statements::literal`]) and of a COPY field. None
    /// for NULL.
    pub fn text(&self) -> Option<String> {
        Some(match self {
            Value::Null => return None,
            Value::Int(i) => i.to_string(),
            Value::Float(x) => float_text(*x),
            Value::Numeric(n) => n.to_string(),
            Value::Text(s) => s.clone(),
            Value::Bool(b) => (if *b { "true" } else { "false" }).into(),
            Value::Bytea(b) => {
                let mut s = String::with_capacity(2 + 2 * b.len());
                s.push_str("\\x");
                for byte in b {
                    s.push_str(&format!("{byte:02x}"));
                }
                s
            },
            Value::Date(d) => date_text(*d),
            Value::Timestamp(t) => timestamp_text(*t),
        })
    }
}

/// A real in Rust's shortest round-trip form, which `float8in` (and
/// `float4in`, for a value a `float4` holds) reads back exactly.
fn float_text(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        (if x > 0.0 { "Infinity" } else { "-Infinity" }).into()
    } else if x == 0.0 {
        (if x.is_sign_negative() { "-0" } else { "0" }).into()
    } else {
        format!("{x:?}")
    }
}

/// The Julian day number of 2000-01-01.
const J2000: i64 = 2_451_545;

/// `date2j`: the Julian day of a proleptic Gregorian date (year 0 is 1 BC).
pub fn date2j(y: i64, m: i64, d: i64) -> i64 {
    let (mut y, mut m) = (y, m);
    if m > 2 {
        m += 1;
        y += 4800;
    } else {
        m += 13;
        y += 4799;
    }
    let century = y / 100;
    let mut julian = y * 365 - 32167;
    julian += y / 4 - century + century / 4;
    julian += 7834 * m / 256 + d;
    julian
}

/// `j2date`: the date of a Julian day.
pub fn j2date(jd: i64) -> (i64, i64, i64) {
    let mut julian = jd + 32044;
    let mut quad = julian / 146_097;
    let extra = (julian - quad * 146_097) * 4 + 3;
    julian += 60 + quad * 3 + extra / 146_097;
    quad = julian / 1461;
    julian -= quad * 1461;
    let mut y = julian * 4 / 1461;
    julian = if y != 0 { (julian + 305) % 365 } else { (julian + 306) % 366 } + 123;
    y += quad * 4;
    let y = y - 4800;
    let quad = julian * 2141 / 65536;
    let d = julian - 7834 * quad / 256;
    let m = (quad + 10) % 12 + 1;
    (y, m, d)
}

/// A day number as `date_out` prints it under the default DateStyle
/// (`ISO`): `YYYY-MM-DD`, ` BC` after years before 1.
pub fn date_text(d: i32) -> String {
    match d {
        i32::MIN => "-infinity".into(),
        i32::MAX => "infinity".into(),
        _ => ymd_text(j2date(i64::from(d) + J2000)),
    }
}

fn ymd_text((y, m, d): (i64, i64, i64)) -> String {
    if y > 0 {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{:04}-{m:02}-{d:02} BC", 1 - y)
    }
}

pub fn timestamp_text(t: i64) -> String {
    match t {
        i64::MIN => "-infinity".into(),
        i64::MAX => "infinity".into(),
        _ => {
            let days = t.div_euclid(MICROS_PER_DAY);
            let mut us = t.rem_euclid(MICROS_PER_DAY);
            let (y, m, d) = j2date(days + J2000);
            let h = us / 3_600_000_000;
            us %= 3_600_000_000;
            let min = us / 60_000_000;
            us %= 60_000_000;
            let s = us / 1_000_000;
            us %= 1_000_000;
            // The BC marker goes after the time.
            let (year, bc) = if y > 0 { (y, "") } else { (1 - y, " BC") };
            let mut out = format!("{year:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}");
            if us > 0 {
                let frac = format!("{us:06}");
                out.push('.');
                out.push_str(frac.trim_end_matches('0'));
            }
            out.push_str(bc);
            out
        },
    }
}

/// The first day whose midnight is past the last timestamp (294277-01-01).
pub const DATE_PAST_TIMESTAMPS: i32 = 106_751_983;

/// A date as a timestamp at its midnight (the infinities to the
/// infinities), or None for a date after the last timestamp.
pub fn date_as_timestamp(d: i32) -> Option<i64> {
    match d {
        i32::MIN => Some(i64::MIN),
        i32::MAX => Some(i64::MAX),
        _ if d >= DATE_PAST_TIMESTAMPS => None,
        _ => Some(i64::from(d) * MICROS_PER_DAY),
    }
}

/// A date against a timestamp, as `date_cmp_timestamp_internal` compares
/// them: at the date's midnight, a date after the last timestamp above
/// every finite timestamp and below `infinity`.
fn cmp_date_timestamp(d: i32, t: i64) -> Ordering {
    match date_as_timestamp(d) {
        Some(dt) => dt.cmp(&t),
        None if t == i64::MAX => Ordering::Less,
        None => Ordering::Greater,
    }
}

/// Reals in PostgreSQL's order: `-0 = 0`, `NaN` equal to itself and above
/// everything else.
pub fn cmp_float(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap(),
    }
}

/// PostgreSQL's comparison of two non-NULL values of comparable types
/// ([`Type::comparable`]); NULL is not ordered here (see [`order_nulls`]).
/// Values of types that are not comparable order by kind, a total order no
/// valid statement relies on.
pub fn compare(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => cmp_float(*x, *y),
        (Value::Numeric(x), Value::Numeric(y)) => x.cmp_value(y),
        (Value::Text(x), Value::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Bytea(x), Value::Bytea(y)) => x.cmp(y),
        (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::Timestamp(x), Value::Timestamp(y)) => x.cmp(y),
        (Value::Date(x), Value::Timestamp(y)) => cmp_date_timestamp(*x, *y),
        (Value::Timestamp(x), Value::Date(y)) => cmp_date_timestamp(*y, *x).reverse(),
        _ => kind(a).cmp(&kind(b)),
    }
}

fn kind(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Int(_) => 1,
        Value::Float(_) => 2,
        Value::Numeric(_) => 3,
        Value::Text(_) => 4,
        Value::Bool(_) => 5,
        Value::Bytea(_) => 6,
        Value::Date(_) => 7,
        Value::Timestamp(_) => 8,
    }
}

/// The order of `ORDER BY` for one term: NULL last ascending and first
/// descending by default, or where `nulls_first` says.
pub fn order_nulls(a: &Value, b: &Value, desc: bool, nulls_first: Option<bool>) -> Ordering {
    let nulls_first = nulls_first.unwrap_or(desc);
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => if nulls_first { Ordering::Less } else { Ordering::Greater },
        (_, Value::Null) => if nulls_first { Ordering::Greater } else { Ordering::Less },
        _ => {
            let o = compare(a, b);
            if desc {
                o.reverse()
            } else {
                o
            }
        },
    }
}

/// Exact identity of values, as a result is compared with the model: reals
/// by their bits (but every NaN alike), numerics with their scale.
pub fn identical(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits() || x.is_nan() && y.is_nan(),
        _ => a == b,
    }
}

/// A column.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// Named `c{id}`, unique within its table.
    pub id: u32,
    pub ty: Type,
    pub not_null: bool,
    /// `DEFAULT v`, a constant of the column's type: what an INSERT that
    /// does not list the column writes (NULL without one).
    pub default: Option<Value>,
    /// `GENERATED BY DEFAULT AS IDENTITY` (an integer column, NOT NULL, no
    /// default): an INSERT that does not list it takes the table's
    /// sequence's next value ([`Model::sequences`]).
    pub identity: bool,
    /// `STORAGE ..` (a variable-length type only): how TOAST treats its
    /// values; nothing logical, but a row of `PLAIN` values must fit a page.
    pub storage: Option<Storage>,
    /// `COMPRESSION ..` (a variable-length type only); nothing logical.
    pub compression: Option<Compression>,
}

impl Column {
    pub fn new(id: u32, ty: Type) -> Self {
        Column { id, ty, not_null: false, default: None, identity: false, storage: None, compression: None }
    }
}

/// A column's TOAST strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// Never compressed or moved out of line.
    Plain,
    /// Compressed, out of line only as a last resort.
    Main,
    /// Out of line, never compressed.
    External,
    /// Compressed, then out of line (the default of these types).
    Extended,
}

impl Storage {
    pub const ALL: [Storage; 4] = [Storage::Plain, Storage::Main, Storage::External, Storage::Extended];

    pub fn name(self) -> &'static str {
        match self {
            Storage::Plain => "PLAIN",
            Storage::Main => "MAIN",
            Storage::External => "EXTERNAL",
            Storage::Extended => "EXTENDED",
        }
    }
}

/// A column's compression method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Pglz,
    Lz4,
}

impl Compression {
    pub fn name(self) -> &'static str {
        match self {
            Compression::Pglz => "pglz",
            Compression::Lz4 => "lz4",
        }
    }
}

/// The access method of an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Btree,
    /// One column, never UNIQUE.
    Hash,
    /// Block-range summaries (minmax), of any of the columns but bool.
    Brin,
    /// GIN with `btree_gin`'s operator classes (every type here): bitmap
    /// scans only, never UNIQUE, no INCLUDE.
    Gin,
    /// GiST with `btree_gist`'s operator classes (every type here): INCLUDE
    /// allowed, never UNIQUE.
    Gist,
    /// SP-GiST, of one text value (`text_ops`): INCLUDE allowed, never
    /// UNIQUE.
    SpGist,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Btree => "btree",
            Method::Hash => "hash",
            Method::Brin => "brin",
            Method::Gin => "gin",
            Method::Gist => "gist",
            Method::SpGist => "spgist",
        }
    }
}

/// A key column of an index: ascending or descending, NULLs where the
/// default puts them or where given (B-tree only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexColumn {
    /// The column's id.
    pub column: u32,
    pub desc: bool,
    pub nulls_first: Option<bool>,
}

impl IndexColumn {
    pub fn asc(column: u32) -> Self {
        IndexColumn { column, desc: false, nulls_first: None }
    }
}

/// The first term of an expression index: a function of one column that
/// raises no error on any value of the column's type (so neither building
/// the index nor writing the table fails) and that PostgreSQL deems
/// immutable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyExpr {
    /// `(c % 7)`, of an integer.
    Mod7,
    /// `floor(c)`, of a real or a numeric (`NaN` and the infinities stay).
    Floor,
    /// `abs(c)`, of a real or a numeric.
    Abs,
    /// `lower(c)`, of text.
    Lower,
    /// `length(c)`, of text or bytes.
    Length,
    /// `md5(c)`, of text or bytes.
    Md5,
    /// `(NOT c)`, of a bool.
    Not,
    /// `extract(year from c)`, of a date or a timestamp (`Infinity` for
    /// the infinities).
    Year,
}

impl KeyExpr {
    pub const ALL: [KeyExpr; 8] =
        [KeyExpr::Mod7, KeyExpr::Floor, KeyExpr::Abs, KeyExpr::Lower, KeyExpr::Length, KeyExpr::Md5, KeyExpr::Not, KeyExpr::Year];

    /// Whether it applies to a column of type `ty`.
    pub fn applies(self, ty: Type) -> bool {
        match self {
            KeyExpr::Mod7 => ty.is_int(),
            KeyExpr::Floor | KeyExpr::Abs => ty.is_float() || ty == Type::Numeric,
            KeyExpr::Lower => ty == Type::Text,
            KeyExpr::Length | KeyExpr::Md5 => matches!(ty, Type::Text | Type::Bytea),
            KeyExpr::Not => ty == Type::Bool,
            KeyExpr::Year => matches!(ty, Type::Date | Type::Timestamp),
        }
    }

    /// The type of the term for a column of type `ty`.
    pub fn ret(self, ty: Type) -> Type {
        match self {
            KeyExpr::Mod7 | KeyExpr::Abs => ty,
            KeyExpr::Floor => if ty == Type::Numeric { Type::Numeric } else { Type::Float8 },
            KeyExpr::Lower | KeyExpr::Md5 => Type::Text,
            KeyExpr::Length => Type::Int4,
            KeyExpr::Not => Type::Bool,
            KeyExpr::Year => Type::Numeric,
        }
    }

    /// The term for column `c{column}`.
    pub fn sql(self, column: u32) -> String {
        match self {
            KeyExpr::Mod7 => format!("(c{column} % 7)"),
            KeyExpr::Floor => format!("floor(c{column})"),
            KeyExpr::Abs => format!("abs(c{column})"),
            KeyExpr::Lower => format!("lower(c{column})"),
            KeyExpr::Length => format!("length(c{column})"),
            KeyExpr::Md5 => format!("md5(c{column})"),
            KeyExpr::Not => format!("(NOT c{column})"),
            KeyExpr::Year => format!("extract(year from c{column})"),
        }
    }
}

/// Storage parameters of an index: nothing logical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IndexParams {
    /// `fillfactor` (B-tree 10 to 100, hash 10 to 100).
    pub fillfactor: Option<u8>,
    /// B-tree `deduplicate_items = off`.
    pub no_dedup: bool,
    /// BRIN `pages_per_range`.
    pub pages_per_range: Option<u16>,
}

/// An index.
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    /// Named `t{table}_i{id}`, unique within its table.
    pub id: u32,
    pub method: Method,
    /// At least one, distinct columns.
    pub columns: Vec<IndexColumn>,
    /// `INCLUDE (..)`: B-tree only, columns not among the keys.
    pub include: Vec<u32>,
    /// UNIQUE (B-tree only): no two rows agree on the key columns, unless
    /// one of them is NULL there (or, with `nulls_not_distinct`, unless
    /// they differ in which are NULL).
    pub unique: bool,
    pub nulls_not_distinct: bool,
    /// The first key term is this function of the first key column.
    pub expr: Option<KeyExpr>,
    /// `WHERE filter`: a partial index of the rows it holds for.
    pub filter: Option<Pred>,
    pub params: IndexParams,
}

impl Index {
    /// A plain ascending B-tree index over these columns.
    pub fn btree(id: u32, columns: &[u32]) -> Self {
        Index {
            id,
            method: Method::Btree,
            columns: columns.iter().map(|&c| IndexColumn::asc(c)).collect(),
            include: Vec::new(),
            unique: false,
            nulls_not_distinct: false,
            expr: None,
            filter: None,
            params: IndexParams::default(),
        }
    }

    /// Every column it reads: keys, INCLUDE columns, and those of its
    /// predicate.
    pub fn columns_read(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.columns.iter().map(|k| k.column).chain(self.include.iter().copied()).collect();
        if let Some(p) = &self.filter {
            p.leaves(&mut out);
        }
        out
    }
}

/// A row: one value per column, in column order.
pub type Row = Vec<Value>;

/// How a partitioned table splits its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Range,
    List,
    Hash,
}

/// The bound of one partition (`t{table}_p{k}`, by position).
#[derive(Clone, Debug, PartialEq)]
pub enum PartBound {
    /// `FROM (lo | MINVALUE) TO (hi | MAXVALUE)`: values `lo <= v < hi`.
    Range(Option<Value>, Option<Value>),
    /// `IN (v, ..)`, NULL among them or not.
    List(Vec<Value>),
    /// `WITH (MODULUS m, REMAINDER r)`.
    Hash(u32, u32),
    /// `DEFAULT`: the rows no other partition takes (not with HASH).
    Default,
}

/// `PARTITION BY <strategy> (c)` and the partitions. A row must belong to a
/// partition (PostgreSQL refuses it otherwise): the model computes that for
/// RANGE and LIST ([`routes`]); HASH partitions are always the full set of
/// remainders of one modulus, so every row belongs to one (the model does
/// not know which, and needs not).
#[derive(Clone, Debug, PartialEq)]
pub struct Partitioning {
    pub strategy: Strategy,
    /// The key column's id (never the identity column).
    pub column: u32,
    pub parts: Vec<PartBound>,
}

impl Partitioning {
    pub fn cost(&self) -> u64 {
        2 + self
            .parts
            .iter()
            .map(|p| match p {
                PartBound::Range(lo, hi) => 1 + lo.as_ref().map_or(0, Value::cost) + hi.as_ref().map_or(0, Value::cost),
                PartBound::List(vs) => 1 + vs.iter().map(Value::cost).sum::<u64>(),
                PartBound::Hash(m, _) => 1 + u64::from(*m),
                PartBound::Default => 1,
            })
            .sum::<u64>()
    }
}

/// Whether a row with key value `v` belongs to a partition.
pub fn routes(p: &Partitioning, v: &Value) -> bool {
    let default = p.parts.contains(&PartBound::Default);
    match p.strategy {
        Strategy::Hash => true,
        Strategy::Range => {
            default
                || *v != Value::Null
                    && p.parts.iter().any(|b| match b {
                        PartBound::Range(lo, hi) => lo.as_ref().is_none_or(|lo| compare(lo, v).is_le()) && hi.as_ref().is_none_or(|hi| compare(v, hi).is_lt()),
                        _ => false,
                    })
        },
        Strategy::List => {
            default
                || p.parts.iter().any(|b| match b {
                    PartBound::List(vs) => vs.iter().any(|x| if *v == Value::Null { *x == Value::Null } else { *x != Value::Null && compare(x, v).is_eq() }),
                    _ => false,
                })
        },
    }
}

/// Whether a partitioning is one PostgreSQL takes for a key of type `ty`:
/// bounds of the strategy's kind with short values of the key's type, nonempty
/// ranges, no two partitions sharing a value, at most one DEFAULT (none for
/// HASH), HASH as every remainder of one modulus.
pub fn partitioning_ok(p: &Partitioning, ty: Type) -> bool {
    let defaults = p.parts.iter().filter(|b| **b == PartBound::Default).count();
    let bounded: Vec<&PartBound> = p.parts.iter().filter(|b| **b != PartBound::Default).collect();
    if p.parts.is_empty() || p.parts.len() > 8 || defaults > 1 || bounded.is_empty() && p.strategy == Strategy::Hash {
        return false;
    }
    // A bound is stored in the partition's pg_class row, which has no TOAST
    // table: short values only ("row is too big" otherwise).
    let value_ok = |v: &Value| ty.admits(v) && !matches!(v, Value::Float(x) if x.is_nan()) && v.width() <= 200;
    let short = p.parts.iter().all(|b| match b {
        PartBound::List(vs) => vs.iter().map(Value::width).sum::<usize>() <= 1000,
        _ => true,
    });
    if !short {
        return false;
    }
    match p.strategy {
        Strategy::Range => {
            let ranges: Option<Vec<(&Option<Value>, &Option<Value>)>> = bounded.iter().map(|b| match b {
                PartBound::Range(lo, hi) => Some((lo, hi)),
                _ => None,
            }).collect();
            let Some(ranges) = ranges else { return false };
            let below = |a: &Option<Value>, b: &Option<Value>, a_low: bool, b_low: bool| -> bool {
                // a < b, None meaning MINVALUE (low) or MAXVALUE (high).
                match (a, b) {
                    (None, None) => a_low && !b_low,
                    (None, _) => a_low,
                    (_, None) => !b_low,
                    (Some(x), Some(y)) => compare(x, y).is_lt(),
                }
            };
            ranges.iter().all(|(lo, hi)| lo.iter().chain(hi.iter()).all(|v| *v != Value::Null && value_ok(v)) && below(lo, hi, true, false))
                && ranges.iter().enumerate().all(|(i, a)| ranges[..i].iter().all(|b| !(below(a.0, b.1, true, false) && below(b.0, a.1, true, false))))
        },
        Strategy::List => {
            let mut all: Vec<&Value> = Vec::new();
            for b in &bounded {
                let PartBound::List(vs) = b else { return false };
                if vs.is_empty() || vs.iter().any(|v| *v != Value::Null && !value_ok(v)) {
                    return false;
                }
                all.extend(vs.iter());
            }
            all.iter().enumerate().all(|(i, a)| all[..i].iter().all(|b| match (a, b) {
                (Value::Null, Value::Null) => false,
                (Value::Null, _) | (_, Value::Null) => true,
                _ => compare(a, b).is_ne(),
            }))
        },
        Strategy::Hash => {
            let Some(m) = bounded.iter().map(|b| match b {
                PartBound::Hash(m, _) => Some(*m),
                _ => None,
            }).collect::<Option<Vec<u32>>>() else { return false };
            let modulus = m[0];
            let mut rems: Vec<u32> = bounded.iter().map(|b| if let PartBound::Hash(_, r) = b { *r } else { 0 }).collect();
            rems.sort();
            defaults == 0 && m.iter().all(|&x| x == modulus) && rems == (0..modulus).collect::<Vec<_>>()
        },
    }
}

/// A table.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// Named `t{id}`, unique among the tables.
    pub id: u32,
    pub columns: Vec<Column>,
    /// `PRIMARY KEY (..)`: column ids, empty for none. Its columns are NOT
    /// NULL and no two rows agree on all of them. Named `t{id}_pkey`.
    pub key: Vec<u32>,
    /// A multiset: the order is the order a setup loads them in, nothing
    /// a statement relies on.
    pub rows: Vec<Row>,
    pub indexes: Vec<Index>,
    /// CHECK constraints: no row makes one false (NULL passes).
    pub checks: Vec<Pred>,
    /// The id the next column added gets, and the next index.
    pub next_column: u32,
    pub next_index: u32,
    /// The widest value ([`Value::width`]) each column has held, by column
    /// id: rows a statement replaced or deleted stay in the heap until
    /// vacuumed, and CREATE INDEX indexes them as long as some snapshot may
    /// see them (which depends on other sessions), so a new index must fit
    /// every value its columns have held, not only the live ones.
    pub widest: BTreeMap<u32, usize>,
    /// The columns that have held an infinite date, by id: a GiST index
    /// keeps such keys (in dead entries and in its inner pages' unions)
    /// after the rows are gone, which `btree_gist`'s date penalty cannot
    /// handle (see `crate::semantics::gist_dates_ok`).
    pub infinite: BTreeSet<u32>,
    /// `PARTITION BY ..`: a partitioned table.
    pub partitioning: Option<Partitioning>,
    /// Its foreign key, `t{id}_fk` (at most one per table, see
    /// `crate::fk`).
    pub fk: Option<ForeignKey>,
    /// `UNLOGGED`: which tables a foreign key may link (a permanent table
    /// references only permanent ones).
    pub unlogged: bool,
    /// Its triggers, `t{id}_g{k}` (see `crate::fk`), and the id the next
    /// one gets.
    pub triggers: Vec<Trigger>,
    pub next_trigger: u32,
}

/// A trigger and its PL/pgSQL function `t{t}_g{id}_f()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub id: u32,
    /// BEFORE (a row trigger) or AFTER.
    pub before: bool,
    /// FOR EACH ROW, or FOR EACH STATEMENT (then with a transition table).
    pub row: bool,
    /// INSERT, UPDATE, DELETE.
    pub events: [bool; 3],
    /// `UPDATE OF c, ..`.
    pub update_of: Vec<u32>,
    /// `WHEN (..)` over NEW (OLD for a DELETE trigger).
    pub when: Option<Pred>,
    pub action: TrigAction,
    /// `ALTER TABLE .. DISABLE TRIGGER` makes it false.
    pub enabled: bool,
}

/// What a trigger's function does.
#[derive(Clone, Debug, PartialEq)]
pub enum TrigAction {
    /// `NEW.c := v; RETURN NEW` (BEFORE ROW).
    Set(u32, Value),
    /// `RETURN NULL` (BEFORE ROW INSERT): the row is not inserted.
    Skip,
    /// `INSERT INTO t{target} (dst, ..) VALUES (NEW.src, ..)` (OLD for
    /// DELETE) for a row trigger, `.. SELECT src, .. FROM nt` (the
    /// transition table) for a statement trigger: pairs (src, dst).
    Audit { target: u32, cols: Vec<(u32, u32)> },
}

impl Trigger {
    pub fn cost(&self) -> u64 {
        2 + u64::from(self.before)
            + u64::from(!self.row)
            + self.events.iter().filter(|e| **e).count() as u64
            + self.update_of.len() as u64
            + self.when.as_ref().map_or(0, |p| 1 + p.cost())
            + u64::from(!self.enabled)
            + match &self.action {
                TrigAction::Set(_, v) => 1 + v.cost(),
                TrigAction::Skip => 1,
                TrigAction::Audit { cols, .. } => 1 + cols.len() as u64,
            }
    }

    /// Its name's order among the table's triggers (the order PostgreSQL
    /// fires them in).
    pub fn name(&self, table: u32) -> String {
        format!("t{table}_g{}", self.id)
    }
}

/// What a foreign key does to the referencing rows when a referenced row
/// is deleted or its key changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl Action {
    pub const ALL: [Action; 5] = [Action::NoAction, Action::Restrict, Action::Cascade, Action::SetNull, Action::SetDefault];

    pub fn sql(self) -> &'static str {
        match self {
            Action::NoAction => "NO ACTION",
            Action::Restrict => "RESTRICT",
            Action::Cascade => "CASCADE",
            Action::SetNull => "SET NULL",
            Action::SetDefault => "SET DEFAULT",
        }
    }
}

/// Whether a constraint is `DEFERRABLE`, and then initially deferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deferral {
    No,
    Immediate,
    Deferred,
}

/// `CONSTRAINT t{t}_fk FOREIGN KEY (columns) REFERENCES t{parent}
/// (parent_columns) [MATCH FULL] ON DELETE .. [(delete_set)] ON UPDATE ..
/// [DEFERRABLE [INITIALLY DEFERRED]]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ForeignKey {
    pub columns: Vec<u32>,
    pub parent: u32,
    /// The columns of a unique constraint of the parent, in its order.
    pub parent_columns: Vec<u32>,
    pub on_delete: Action,
    /// The column list of `ON DELETE SET NULL | SET DEFAULT (..)`, a
    /// subset of `columns`; empty for all of them.
    pub delete_set: Vec<u32>,
    pub on_update: Action,
    /// `MATCH FULL` (else `MATCH SIMPLE`, the default).
    pub full: bool,
    pub deferral: Deferral,
}

impl ForeignKey {
    pub fn cost(&self) -> u64 {
        1 + self.columns.len() as u64
            + u64::from(self.on_delete != Action::NoAction)
            + self.delete_set.len() as u64
            + u64::from(self.on_update != Action::NoAction)
            + u64::from(self.full)
            + u64::from(self.deferral != Deferral::No)
    }

    /// The clause after `CONSTRAINT t{t}_fk`.
    pub fn sql(&self) -> String {
        let cols = |v: &[u32]| v.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
        let mut s = format!("FOREIGN KEY ({}) REFERENCES t{} ({})", cols(&self.columns), self.parent, cols(&self.parent_columns));
        if self.full {
            s.push_str(" MATCH FULL");
        }
        if self.on_delete != Action::NoAction {
            s.push_str(&format!(" ON DELETE {}", self.on_delete.sql()));
            if !self.delete_set.is_empty() {
                s.push_str(&format!(" ({})", cols(&self.delete_set)));
            }
        }
        if self.on_update != Action::NoAction {
            s.push_str(&format!(" ON UPDATE {}", self.on_update.sql()));
        }
        match self.deferral {
            Deferral::No => {},
            Deferral::Immediate => s.push_str(" DEFERRABLE"),
            Deferral::Deferred => s.push_str(" DEFERRABLE INITIALLY DEFERRED"),
        }
        s
    }
}

impl Table {
    /// A table of these column types, ids 0, 1, ..
    pub fn new(id: u32, types: &[Type]) -> Self {
        Table {
            id,
            columns: types.iter().enumerate().map(|(k, &ty)| Column::new(k as u32, ty)).collect(),
            key: Vec::new(),
            rows: Vec::new(),
            indexes: Vec::new(),
            checks: Vec::new(),
            next_column: types.len() as u32,
            next_index: 0,
            widest: BTreeMap::new(),
            infinite: BTreeSet::new(),
            partitioning: None,
            fk: None,
            unlogged: false,
            triggers: Vec::new(),
            next_trigger: 0,
        }
    }

    /// Record the widths of values the table now holds.
    pub fn record(&mut self, rows: &[Row]) {
        for r in rows {
            for (c, v) in self.columns.iter().zip(r) {
                let w = self.widest.entry(c.id).or_insert(0);
                *w = (*w).max(v.width());
                if matches!(v, Value::Date(i32::MIN | i32::MAX)) {
                    self.infinite.insert(c.id);
                }
            }
        }
    }

    /// The position of column `c{id}`.
    pub fn pos(&self, id: u32) -> Option<usize> {
        self.columns.iter().position(|c| c.id == id)
    }

    pub fn column(&self, id: u32) -> Option<&Column> {
        self.columns.iter().find(|c| c.id == id)
    }

    pub fn index(&self, id: u32) -> Option<&Index> {
        self.indexes.iter().find(|i| i.id == id)
    }

    /// Row `r`'s value of column `c{id}` (which must exist).
    pub fn value<'a>(&self, r: &'a Row, id: u32) -> &'a Value {
        &r[self.pos(id).expect("a column of the table")]
    }

    /// The identity column, if any.
    pub fn identity(&self) -> Option<&Column> {
        self.columns.iter().find(|c| c.identity)
    }

    /// The columns an index of it (not an expression index) or its key
    /// leads with: where a lookup can seek.
    pub fn leading(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.indexes.iter().filter(|i| i.expr.is_none() && matches!(i.method, Method::Btree | Method::Hash | Method::Gist)).map(|i| i.columns[0].column).collect();
        out.extend(self.key.first().copied());
        out
    }

    /// The unique constraints rows must keep: the key, and every UNIQUE
    /// index (key columns, whether NULLs are distinct).
    pub fn uniques(&self) -> Vec<(Vec<u32>, bool)> {
        let mut out = Vec::new();
        if !self.key.is_empty() {
            out.push((self.key.clone(), false));
        }
        for i in &self.indexes {
            if i.unique && i.filter.is_none() && i.expr.is_none() {
                out.push((i.columns.iter().map(|k| k.column).collect(), i.nulls_not_distinct));
            }
        }
        out
    }
}

/// The tables as a savepoint saw them, and the transaction's constraint
/// state then.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub tables: Vec<Table>,
    pub txn: TxnState,
}

/// What a transaction remembers of its deferred constraints: `SET
/// CONSTRAINTS ALL ..` (`all`, deferred or not), `SET CONSTRAINTS t{t}_fk
/// ..` (`named`, by table), and the tables that may have deferred trigger
/// events pending (`pending`: PostgreSQL refuses DDL on them). Empty
/// outside a transaction.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TxnState {
    pub all: Option<bool>,
    pub named: BTreeMap<u32, bool>,
    pub pending: BTreeSet<u32>,
}

/// The logical database.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Model {
    pub tables: Vec<Table>,
    /// One snapshot per open savepoint, outermost first (savepoint 0 is
    /// the transaction's start).
    pub savepoints: Vec<Snapshot>,
    /// The next value of each identity sequence, by table id. Sequences are
    /// not transactional: a rollback leaves them as they are, so they are
    /// kept apart from the tables (and never forgotten, so a table whose
    /// DROP is rolled back finds its sequence where it was).
    pub sequences: BTreeMap<u32, i64>,
    /// The id the next table created gets (never reused).
    pub next_table: u32,
    /// The open transaction's constraint state.
    pub txn: TxnState,
}

/// Bounds on what setups and statements make.
pub const MAX_TABLES: usize = 8;
pub const MAX_COLUMNS: usize = 8;
pub const MAX_INDEXES: usize = 6;
pub const MAX_ROWS: usize = 2000;

/// The most bytes of one index entry: B-tree's limit is a third of a page
/// (2704 bytes on 8 KiB pages), checked after compression; this is under it
/// before any.
pub const MAX_ENTRY: usize = 2000;
/// The most bytes of the values of a row's columns that a layout keeps
/// from TOAST (`STORAGE PLAIN`): a heap tuple fits a page.
pub const MAX_PLAIN: usize = 6000;

impl Model {
    pub fn table(&self, id: u32) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
    }

    pub fn table_mut(&mut self, id: u32) -> Option<&mut Table> {
        self.tables.iter_mut().find(|t| t.id == id)
    }

    /// A simplicity measure, for shrinking.
    pub fn cost(&self) -> u64 {
        self.tables.iter().map(table_cost).sum()
    }
}

pub fn table_cost(t: &Table) -> u64 {
    1 + t.columns.iter().map(|c| 1 + c.ty as u64 / 4 + u64::from(c.not_null) + u64::from(c.storage.is_some()) + u64::from(c.compression.is_some()) + u64::from(c.identity) + c.default.as_ref().map_or(0, |v| 1 + v.cost())).sum::<u64>()
        + t.key.len() as u64
        + t.rows.iter().map(|r| 1 + r.iter().map(Value::cost).sum::<u64>()).sum::<u64>()
        + t.indexes.iter().map(index_cost).sum::<u64>()
        + t.checks.iter().map(|p| 1 + p.cost()).sum::<u64>()
        + t.partitioning.as_ref().map_or(0, Partitioning::cost)
        + t.fk.as_ref().map_or(0, ForeignKey::cost)
        + t.triggers.iter().map(Trigger::cost).sum::<u64>()
}

pub fn index_cost(i: &Index) -> u64 {
    1 + i.columns.iter().map(|k| 1 + u64::from(k.desc) + u64::from(k.nulls_first.is_some())).sum::<u64>()
        + i.include.len() as u64
        + u64::from(i.method != Method::Btree)
        + u64::from(i.unique)
        + u64::from(i.nulls_not_distinct)
        + u64::from(i.expr.is_some())
        + i.filter.as_ref().map_or(0, |p| 1 + p.cost())
        + u64::from(i.params.fillfactor.is_some())
        + u64::from(i.params.no_dedup)
        + u64::from(i.params.pages_per_range.is_some())
}

/// Whether `rows` keep a unique constraint on `columns` of `t`: no two rows
/// agree on all of them, where a NULL agrees with nothing (or, with
/// `nulls_not_distinct`, with NULL only).
pub fn unique_ok(t: &Table, columns: &[u32], nulls_not_distinct: bool, rows: &[Row]) -> bool {
    let Some(pos) = columns.iter().map(|&c| t.pos(c)).collect::<Option<Vec<usize>>>() else {
        return false;
    };
    let key = |r: &Row| -> Option<Vec<Value>> {
        let k: Vec<Value> = pos.iter().map(|&p| r[p].clone()).collect();
        (nulls_not_distinct || k.iter().all(|v| *v != Value::Null)).then_some(k)
    };
    let mut keys: Vec<Vec<Value>> = rows.iter().filter_map(key).collect();
    let cmp = |a: &Vec<Value>, b: &Vec<Value>| {
        a.iter().zip(b).map(|(x, y)| order_nulls(x, y, false, None)).find(|o| o.is_ne()).unwrap_or(Ordering::Equal)
    };
    keys.sort_by(cmp);
    keys.windows(2).all(|w| cmp(&w[0], &w[1]) != Ordering::Equal)
}

/// Whether every entry an index of `t` would hold for `rows` fits
/// ([`MAX_ENTRY`]) and every row fits its page where its columns keep
/// values inline (`STORAGE PLAIN`).
pub fn sizes_ok(t: &Table, rows: &[Row]) -> bool {
    let plain: Vec<bool> = t.columns.iter().map(|c| c.storage == Some(Storage::Plain)).collect();
    // The key's index, then every other. (An index that exists took every
    // value as it was written; a new one must also fit the values the
    // columns held before, see `crate::semantics::index_ok`.)
    let keys = std::iter::once(t.key.clone()).chain(t.indexes.iter().map(|i| i.columns.iter().map(|k| k.column).chain(i.include.iter().copied()).collect()));
    let entries = keys.into_iter().all(|cols: Vec<u32>| {
        let pos: Vec<usize> = cols.iter().filter_map(|&c| t.pos(c)).collect();
        rows.iter().all(|r| pos.iter().map(|&p| r[p].width()).sum::<usize>() <= MAX_ENTRY)
    });
    entries && rows.iter().all(|r| r.iter().zip(&plain).filter(|(_, &p)| p).map(|(v, _)| v.width()).sum::<usize>() <= MAX_PLAIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numerics_compare_by_value_and_print_their_scale() {
        let n = |s: &str| Numeric::parse(s).unwrap();
        assert_eq!(n("1.0").cmp_value(&n("1.00")), Ordering::Equal);
        assert_eq!(n("-0.00").to_string(), "0.00");
        assert_eq!(n("-1.5").cmp_value(&n("-1.25")), Ordering::Less);
        assert_eq!(n("0.001").cmp_value(&n("-0")), Ordering::Greater);
        assert_eq!(n("NaN").cmp_value(&n("Infinity")), Ordering::Greater);
        assert_eq!(n("007.50").to_string(), "7.50");
        assert_eq!(n(".5").to_string(), "0.5");
        assert_eq!(Numeric::int(-120).to_string(), "-120");
    }

    #[test]
    fn dates_print_as_postgres_does() {
        assert_eq!(date_text(0), "2000-01-01");
        assert_eq!(date_text(DATE_MIN), "4714-11-24 BC");
        assert_eq!(date_text(DATE_MAX), "5874897-12-31");
        assert_eq!(date_text(-1), "1999-12-31");
        assert_eq!(date_text(-730_119), "0001-01-01");
        assert_eq!(date_text(-730_120), "0001-12-31 BC");
        assert_eq!(timestamp_text(TIMESTAMP_MAX), "294276-12-31 23:59:59.999999");
        assert_eq!(timestamp_text(TIMESTAMP_MIN), "4714-11-24 00:00:00 BC");
        assert_eq!(timestamp_text(-1), "1999-12-31 23:59:59.999999");
        assert_eq!(timestamp_text(1_500_000), "2000-01-01 00:00:01.5");
        for d in [DATE_MIN, -800_000, -1, 0, 59, 60, 366, 1_000_000, DATE_MAX] {
            let (y, m, dd) = j2date(i64::from(d) + J2000);
            assert_eq!(date2j(y, m, dd) - J2000, i64::from(d));
        }
    }
}
