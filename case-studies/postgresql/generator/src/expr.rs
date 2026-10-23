// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A typed SQL expression and query language over a database's tables, for
//! the SQLancer-style oracles (`tests/sqlancer.rs`). Separate from the
//! model's narrow predicates ([`crate::statements::Pred`]): these
//! expressions are valid against the schema but need not be predictable,
//! since the oracles compare PostgreSQL with itself.
//!
//! PostgreSQL types every expression and refuses what does not type, so
//! expressions are drawn by type ([`draw_expr`]) and checked by type
//! ([`Query::valid`], which recomputes every type). Literals always carry
//! their type (`'5'::int4`), so no literal is of "unknown" type.
//!
//! - [`Expr`]: columns ([`Col`]: a source of the expression's own query, or
//!   of a query around it in a correlated subquery); typed literals;
//!   comparisons (of one type, or of types PostgreSQL compares without a
//!   cast that could fail: integers of any width, integers and numerics,
//!   reals of either width, dates and timestamps); `IS [NOT] DISTINCT
//!   FROM`; `AND`, `OR`, `NOT`; `IS [NOT] NULL`; `IS [NOT] TRUE | FALSE |
//!   UNKNOWN`; `[NOT] BETWEEN [SYMMETRIC]`; `[NOT] IN (list)`; `[NOT] LIKE
//!   | ILIKE` and `~` with literal patterns ([`pattern_ok`]); `+ - * / %`
//!   with PostgreSQL's result types ([`arith_type`]), a divisor always a
//!   literal that is neither 0 nor -1; unary minus; `||`; simple and
//!   searched `CASE`; `coalesce`, `nullif`, `greatest`, `least`; `CAST`
//!   between the pairs [`cast_ok`] allows; the functions of [`Func`];
//!   aggregates (in aggregate queries); `[NOT] EXISTS (q)`, `e [NOT] IN
//!   (q)` and scalar subqueries `(q)`.
//! - Expression-only types ([`Type::internal`]: `jsonb`, `int8[]`,
//!   `text[]`, `interval`): built from the scope's values and read back as
//!   column types inside expressions (`->>`, `cardinality`, `extract`,
//!   `::text`, predicates); never a result, a group key or a `min`/`max`
//!   argument ([`check_select`]), so the oracles read only column types and
//!   no picked value of one shows which of equal values was picked (`'1
//!   day' = '24 hours'`). Constants they need (paths, key sets, interval
//!   literals) render as `CAST('..' AS t)`, which the generic oracle
//!   leaves alone.
//! - Run-time errors: integer and real overflow, values out of range, NaN
//!   or infinity cast to an integer, infinite dates subtracted. Which rows a
//!   plan evaluates an expression on is the planner's, so the oracles skip
//!   a comparison where exactly one side fails so
//!   (`crate::sqlancer::expected_error`). Everything else is excluded by
//!   construction: no division by zero, no negative substring length, no
//!   pattern ending in an escape.
//! - Sources in FROM ([`Src`]): tables, CTEs, set-returning functions
//!   ([`FromFn`]: `generate_series`, `unnest`, `jsonb_each_text`,
//!   `jsonb_array_elements_text`, `ROWS FROM`, `WITH ORDINALITY`, each with
//!   a bound on its rows and no error), and subqueries (derived tables and
//!   `LATERAL`), reading earlier sources only through inner, left and cross
//!   joins ([`from_item_ok`]).
//! - [`Query`]: `[WITH x0 AS [[NOT] MATERIALIZED] (..), ..] SELECT [DISTINCT]
//!   <exprs | *> FROM t AS a0 [[LEFT | RIGHT | FULL | CROSS] JOIN t AS a1
//!   [ON e]].. [WHERE e] [GROUP BY e, .. | ROLLUP | CUBE | GROUPING SETS
//!   ([`GroupSets`]) [HAVING e]] [UNION [ALL] |
//!   INTERSECT [ALL] | EXCEPT [ALL] (SELECT ..)].. [ORDER BY 1, 2, .. [LIMIT
//!   n [OFFSET k]]]`: up to [`MAX_SOURCES`] sources, aliased by nesting level
//!   and position. ORDER BY only at the top level, and only by every result
//!   column, so that LIMIT picks rows determined up to equal values.
//!   Compound members and CTEs only at the top level; a CTE's query is
//!   never DISTINCT, grouped or aggregate, so the rows it gives are
//!   determined.
//!
//! Results are determined by the data up to equal values: rows that a
//! query picks among equal ones (DISTINCT, GROUP BY keys, set operations,
//! `min` and `max`) may differ in values PostgreSQL compares equal (`-0`
//! and `0`, `1.0` and `1.00`), which the oracles compare as equal. So
//! nothing distinguishes such values once picked: a picked value is never
//! read by an enclosing query (grouped, DISTINCT and compound queries are
//! never CTEs or scalar subqueries, and `min`, `max` take no real or
//! numeric), and a grouped query's items are its keys as written or
//! aggregates.
//!
//! The generator ([`Queries`]) never runs PostgreSQL. Shrinking proposes
//! candidates of strictly lower [`Query::cost`] and keeps the valid ones;
//! a smaller schema repairs queries ([`Query::repair`]: a query over a
//! table that is gone is dropped, a column that is gone becomes a NULL of
//! its type).

use std::{borrow::Cow, fmt::Write as _, ops::ControlFlow};

use ephyra::{Gen, Source};

use crate::{
    model::{Model, Numeric, Type, Value},
    setup::fresh,
    statements::{literal, CmpOp},
};

// ---------------------------------------------------------- expressions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

impl Arith {
    fn sql(self) -> &'static str {
        match self {
            Arith::Add => "+",
            Arith::Sub => "-",
            Arith::Mul => "*",
            Arith::Div => "/",
            Arith::Mod => "%",
        }
    }
}

/// The deterministic functions, each with the argument types it takes and
/// the type it returns ([`Func::ret`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    /// `abs` of a number: the same type (overflow for the smallest integer).
    Abs,
    /// `floor`, `ceil`, `sign` of a `float8` or `numeric`.
    Floor,
    Ceil,
    Sign,
    /// `round(x)` of a `float8` or `numeric`; `round(x, k)` of a numeric
    /// and an integer literal.
    Round,
    Trunc,
    /// Text to text.
    Lower,
    Upper,
    Reverse,
    Btrim,
    Initcap,
    /// `md5` of text or bytes: text.
    Md5,
    /// `length` of text or bytes: `int4`.
    Length,
    /// `substr(t, from, n)`, `n` a literal of at least 0.
    Substr,
    /// `replace(t, a, b)`.
    Replace,
    /// `left(t, k)`, `right(t, k)`.
    Left,
    Right,
    /// `strpos(t, s)`: `int4`.
    Strpos,
    /// `starts_with(t, s)`: bool.
    StartsWith,
    /// `concat(a, b, ..)` of any values: text (NULLs left out).
    Concat,
    /// `scale(n)` of a numeric: `int4`.
    Scale,
    /// `to_hex(i)` of an `int4` or `int8`: text.
    ToHex,
    /// `date_trunc('unit', ts)`: the first argument a unit literal.
    DateTrunc,
    /// jsonb builders: `to_jsonb(e)` of any value; `jsonb_build_object(k,
    /// v, ..)`, the keys text literals that are not NULL (a NULL key is an
    /// error); `jsonb_build_array(e, ..)`; `jsonb_strip_nulls(j)`; `j || j`.
    ToJsonb,
    JsonbBuildObject,
    JsonbBuildArray,
    JsonbStripNulls,
    JsonbConcat,
    /// `j -> k` (a text key or an `int4` index), `j #> '{..}'` (a path of
    /// [`JSON_PATHS`]): jsonb, NULL when missing.
    JsonbGet,
    JsonbGetPath(u8),
    /// `j ->> k`, `j #>> '{..}'`, `jsonb_typeof(j)`, `jsonb_pretty(j)`:
    /// text.
    JsonbGetText,
    JsonbGetPathText(u8),
    JsonbTypeof,
    JsonbPretty,
    /// `jsonb_array_length(a)`: `int4`, `a` a `jsonb_build_array` call
    /// (of anything else it may be an error).
    JsonbArrayLength,
    /// `a @> b`, `a <@ b`, `j ? k`, `j ?| '{..}'`, `j ?& '{..}'` (keys of
    /// [`JSON_KEYSETS`]): bool.
    JsonbContains,
    JsonbContained,
    JsonbHasKey,
    JsonbHasAny(u8),
    JsonbHasAll(u8),
    /// `j @? 'path'`, `j @@ 'predicate'` (of [`JSONPATHS`] and
    /// [`JSONPATH_PREDICATES`]; both operators suppress the errors a path
    /// can raise on a value), `jsonb_path_query_first(j, 'path', '{}',
    /// true)` (silent): jsonb.
    JsonbPathExists(u8),
    JsonbPathMatch(u8),
    JsonbPathQueryFirst(u8),
    /// `ARRAY[e, ..]::t[]`, the elements of type `t` (of
    /// [`Type::array`]).
    Array(Type),
    /// `array_append(a, e)`, `array_prepend(e, a)`, `array_cat(a, b)`,
    /// `array_remove(a, e)`, `array_replace(a, e, f)`: arrays.
    ArrayAppend,
    ArrayPrepend,
    ArrayCat,
    ArrayRemove,
    ArrayReplace,
    /// `a[k]`, `k` an `int4`: an element (NULL out of bounds); `a[i:j]`,
    /// `i` and `j` `int4` literals: an array.
    Subscript,
    Slice,
    /// `string_to_array(t, sep)`, `sep` a text literal: `text[]`.
    StringToArray,
    /// `array_to_string(a, sep)`, `sep` a text literal: text.
    ArrayToString,
    /// `cardinality(a)`, `array_length(a, 1)`, `array_upper(a, 1)`,
    /// `array_position(a, e)`: `int4`.
    Cardinality,
    ArrayLength,
    ArrayUpper,
    ArrayPosition,
    /// `e op ANY (a)`, `e op ALL (a)`: bool.
    Any(CmpOp),
    All(CmpOp),
    /// `a @> b`, `a <@ b`, `a && b`: bool.
    ArrayContains,
    ArrayContained,
    ArrayOverlaps,
    /// `CAST('..' AS interval)`, of [`INTERVALS`].
    IntervalLit(u8),
    /// `ts + i`, `ts - i` (a timestamp or a date and an interval: a
    /// timestamp), `ts - ts` (an interval), `age(ts, ts)`; `i + i`, `i - i`,
    /// `i * x` and `i / x` (`x` a `float8` literal, not 0): intervals.
    /// Overflow is an expected error ("out of range").
    TsPlus,
    TsMinus,
    TsDiff,
    Age,
    IntervalAdd,
    IntervalSub,
    IntervalMul,
    IntervalDiv,
    /// `justify_days(i)`, `justify_hours(i)`, `justify_interval(i)`.
    JustifyDays,
    JustifyHours,
    JustifyInterval,
    /// `extract(f FROM x)` (numeric), `date_part('f', x)` (`float8`), of
    /// an interval (fields of [`INTERVAL_FIELDS`]) or a timestamp
    /// ([`TIMESTAMP_FIELDS`]).
    /// (field, of an interval).
    Extract(u8, bool),
    DatePart(u8, bool),
    /// `to_char(x, 'fmt')` of a timestamp or an interval, of
    /// [`TO_CHAR_FORMATS`]: text.
    ToChar(u8),
    /// `date_bin('stride', ts, origin)`, strides of [`BIN_STRIDES`] (no
    /// months, more than 0): a timestamp.
    DateBin(u8),
}

/// Interval literals.
pub const INTERVALS: [&str; 12] = ["1 day", "-3 hours", "1 mon 2 days", "00:00:01.5", "2 years", "1 week", "-1 mon +5 days", "0", "36 hours", "100 days 23:59:59", "1 year -1 sec", "15 minutes"];
/// Fields `extract` takes of an interval.
pub const INTERVAL_FIELDS: [&str; 8] = ["epoch", "day", "hour", "minute", "second", "month", "year", "microseconds"];
/// Fields `extract` takes of a timestamp.
pub const TIMESTAMP_FIELDS: [&str; 14] = ["year", "month", "day", "hour", "minute", "second", "dow", "doy", "epoch", "quarter", "week", "isoyear", "century", "julian"];
/// `to_char` formats.
pub const TO_CHAR_FORMATS: [&str; 6] = ["YYYY-MM-DD HH24:MI:SS", "HH12:MI AM", "Day DD Mon", "DDD IYYY IW", "FMMonth Q", "SSSS.US"];
/// The `to_char` formats an interval takes.
pub const INTERVAL_FORMATS: [usize; 2] = [0, 5];
/// `date_bin` strides.
pub const BIN_STRIDES: [&str; 4] = ["15 minutes", "1 day", "7 days 1 hour", "00:00:01"];

/// Paths for `#>` and `#>>`.
pub const JSON_PATHS: [&str; 6] = ["{a}", "{0}", "{a,0}", "{b,a}", "{1,b}", "{}"];
/// Key sets for `?|` and `?&`.
pub const JSON_KEYSETS: [&str; 4] = ["{a}", "{a,b}", "{b,c}", "{x,a,c}"];
/// SQL/JSON paths: well-formed, errors suppressed by `@?` and silent calls.
pub const JSONPATHS: [&str; 14] = [
    "$.a",
    "$.b[0]",
    "$[*]",
    "$.a ? (@ > 1)",
    "$.** ? (@ == \"a\")",
    "$[last]",
    "$.a.size()",
    "$.type()",
    "$ ? (@.a starts with \"a\")",
    "strict $.a",
    "$.* ? (@ like_regex \"^a\" flag \"i\")",
    "$.a + 1",
    "$.keyvalue().key",
    "$[0 to 1] ? (@ != null)",
];
/// SQL/JSON predicates for `@@`.
pub const JSONPATH_PREDICATES: [&str; 8] = [
    "$.a > 1",
    "$[*] == \"a\"",
    "exists($.b)",
    "$.a starts with \"x\"",
    "$.size() >= 2",
    "($.a + 1) > 0",
    "$.type() == \"object\"",
    "!($.** == null)",
];

/// Units `date_trunc` takes.
pub const UNITS: [&str; 7] = ["microseconds", "second", "minute", "hour", "day", "month", "year"];

impl Func {
    fn name(self) -> &'static str {
        match self {
            Func::Abs => "abs",
            Func::Floor => "floor",
            Func::Ceil => "ceil",
            Func::Sign => "sign",
            Func::Round => "round",
            Func::Trunc => "trunc",
            Func::Lower => "lower",
            Func::Upper => "upper",
            Func::Reverse => "reverse",
            Func::Btrim => "btrim",
            Func::Initcap => "initcap",
            Func::Md5 => "md5",
            Func::Length => "length",
            Func::Substr => "substr",
            Func::Replace => "replace",
            Func::Left => "left",
            Func::Right => "right",
            Func::Strpos => "strpos",
            Func::StartsWith => "starts_with",
            Func::Concat => "concat",
            Func::Scale => "scale",
            Func::ToHex => "to_hex",
            Func::DateTrunc => "date_trunc",
            Func::ToJsonb => "to_jsonb",
            Func::JsonbBuildObject => "jsonb_build_object",
            Func::JsonbBuildArray => "jsonb_build_array",
            Func::JsonbStripNulls => "jsonb_strip_nulls",
            Func::JsonbTypeof => "jsonb_typeof",
            Func::JsonbPretty => "jsonb_pretty",
            Func::JsonbArrayLength => "jsonb_array_length",
            Func::JsonbPathQueryFirst(_) => "jsonb_path_query_first",
            Func::JsonbConcat => "||",
            Func::JsonbGet => "->",
            Func::JsonbGetText => "->>",
            Func::JsonbGetPath(_) => "#>",
            Func::JsonbGetPathText(_) => "#>>",
            Func::JsonbContains => "@>",
            Func::JsonbContained => "<@",
            Func::JsonbHasKey => "?",
            Func::JsonbHasAny(_) => "?|",
            Func::JsonbHasAll(_) => "?&",
            Func::JsonbPathExists(_) => "@?",
            Func::JsonbPathMatch(_) => "@@",
            Func::Array(_) => "ARRAY",
            Func::ArrayAppend => "array_append",
            Func::ArrayPrepend => "array_prepend",
            Func::ArrayCat => "array_cat",
            Func::ArrayRemove => "array_remove",
            Func::ArrayReplace => "array_replace",
            Func::Subscript | Func::Slice => "[]",
            Func::StringToArray => "string_to_array",
            Func::ArrayToString => "array_to_string",
            Func::Cardinality => "cardinality",
            Func::ArrayLength => "array_length",
            Func::ArrayUpper => "array_upper",
            Func::ArrayPosition => "array_position",
            Func::Any(_) => "ANY",
            Func::All(_) => "ALL",
            Func::ArrayContains => "@>",
            Func::ArrayContained => "<@",
            Func::ArrayOverlaps => "&&",
            Func::IntervalLit(_) => "interval",
            Func::TsPlus | Func::IntervalAdd => "+",
            Func::TsMinus | Func::TsDiff | Func::IntervalSub => "-",
            Func::IntervalMul => "*",
            Func::IntervalDiv => "/",
            Func::Age => "age",
            Func::JustifyDays => "justify_days",
            Func::JustifyHours => "justify_hours",
            Func::JustifyInterval => "justify_interval",
            Func::Extract(..) => "extract",
            Func::DatePart(..) => "date_part",
            Func::ToChar(_) => "to_char",
            Func::DateBin(_) => "date_bin",
        }
    }

    /// The call rendered with its rendered arguments.
    fn sql(self, args: &[String]) -> String {
        // `CAST(.. AS ty)`, not `..::ty`: constants of the query, not literals
        // the generic oracle makes parameters (`::text[]` would read as one).
        let lit = |s: &str, ty: &str| format!("CAST({} AS {ty})", literal(&Value::Text(s.into())));
        match (self, args) {
            (Func::JsonbGetPath(k) | Func::JsonbGetPathText(k), [a]) => format!("({a} {} {})", self.name(), lit(JSON_PATHS[k as usize], "text[]")),
            (Func::JsonbHasAny(k) | Func::JsonbHasAll(k), [a]) => format!("({a} {} {})", self.name(), lit(JSON_KEYSETS[k as usize], "text[]")),
            (Func::JsonbPathExists(k), [a]) => format!("({a} @? {})", lit(JSONPATHS[k as usize], "jsonpath")),
            (Func::JsonbPathMatch(k), [a]) => format!("({a} @@ {})", lit(JSONPATH_PREDICATES[k as usize], "jsonpath")),
            (Func::JsonbPathQueryFirst(k), [a]) => format!("jsonb_path_query_first({a}, {}, '{{}}'::jsonb, true)", lit(JSONPATHS[k as usize], "jsonpath")),
            (Func::Array(t), xs) => format!("ARRAY[{}]::{}", xs.join(", "), t.array().map_or("int8[]", |a| a.name())),
            (Func::ArrayLength | Func::ArrayUpper, [a]) => format!("{}({a}, 1)", self.name()),
            (Func::Subscript, [a, k]) => format!("({a})[{k}]"),
            (Func::Slice, [a, i, j]) => format!("({a})[{i}:{j}]"),
            (Func::Any(op) | Func::All(op), [e, a]) => format!("({e} {} {} ({a}))", op.sql(), self.name()),
            (Func::ArrayContains | Func::ArrayContained | Func::ArrayOverlaps, [a, b]) => format!("({a} {} {b})", self.name()),
            (Func::IntervalLit(k), []) => lit(INTERVALS[k as usize], "interval"),
            (Func::TsPlus | Func::TsMinus | Func::TsDiff | Func::IntervalAdd | Func::IntervalSub | Func::IntervalMul | Func::IntervalDiv, [a, b]) => format!("({a} {} {b})", self.name()),
            (Func::Extract(k, iv), [a]) => format!("extract({} FROM {a})", if iv { &INTERVAL_FIELDS[..] } else { &TIMESTAMP_FIELDS[..] }.get(k as usize).unwrap_or(&"epoch")),
            (Func::DatePart(k, iv), [a]) => format!("date_part({}, {a})", lit(if iv { &INTERVAL_FIELDS[..] } else { &TIMESTAMP_FIELDS[..] }.get(k as usize).unwrap_or(&"epoch"), "text")),
            (Func::ToChar(k), [a]) => format!("to_char({a}, {})", lit(TO_CHAR_FORMATS[k as usize], "text")),
            (Func::DateBin(k), [a]) => format!("date_bin({}, {a}, CAST('2001-01-01' AS timestamp))", lit(BIN_STRIDES[k as usize], "interval")),
            (Func::JsonbConcat | Func::JsonbGet | Func::JsonbGetText | Func::JsonbContains | Func::JsonbContained | Func::JsonbHasKey, [a, b]) => format!("({a} {} {b})", self.name()),
            _ => format!("{}({})", self.name(), args.join(", ")),
        }
    }

    /// The type of the call on arguments of these types (and literal
    /// arguments where the function needs one), or None when PostgreSQL
    /// would refuse it or could raise an error this language avoids.
    pub fn ret(self, args: &[Expr], types: &[Type]) -> Option<Type> {
        use Type::*;
        let lit_int = |e: &Expr, least: i64| matches!(e, Expr::Lit(Value::Int(k), Int4) if *k >= least);
        let lit_text = |e: &Expr| matches!(e, Expr::Lit(Value::Text(_), Text));
        match (self, types) {
            (Func::Abs, [t]) if t.is_int() || t.is_float() || *t == Numeric => Some(*t),
            (Func::Floor | Func::Ceil | Func::Sign, [t @ (Float8 | Numeric)]) => Some(*t),
            (Func::Round | Func::Trunc, [t @ (Float8 | Numeric)]) => Some(*t),
            (Func::Round | Func::Trunc, [Numeric, Int4]) if lit_int(&args[1], -20) && matches!(args[1], Expr::Lit(Value::Int(k), _) if k <= 20) => Some(Numeric),
            (Func::Lower | Func::Upper | Func::Reverse | Func::Btrim | Func::Initcap, [Text]) => Some(Text),
            (Func::Md5, [Text | Bytea]) => Some(Text),
            (Func::Length, [Text | Bytea]) => Some(Int4),
            (Func::Substr, [Text, Int4, Int4]) if lit_int(&args[2], 0) => Some(Text),
            (Func::Replace, [Text, Text, Text]) => Some(Text),
            (Func::Left | Func::Right, [Text, Int4]) => Some(Text),
            (Func::Strpos, [Text, Text]) => Some(Int4),
            (Func::StartsWith, [Text, Text]) => Some(Bool),
            (Func::Concat, ts) if !ts.is_empty() && ts.len() <= 4 => Some(Text),
            (Func::Scale, [Numeric]) => Some(Int4),
            (Func::ToHex, [Int4 | Int8]) => Some(Text),
            (Func::DateTrunc, [Text, Timestamp]) if lit_text(&args[0]) && matches!(&args[0], Expr::Lit(Value::Text(u), _) if UNITS.contains(&u.as_str())) => Some(Timestamp),
            (Func::ToJsonb, [_]) => Some(Jsonb),
            (Func::JsonbBuildObject, ts) if ts.len() <= 6 && ts.len() % 2 == 0 => {
                let keys_ok = args.chunks(2).all(|kv| matches!(&kv[0], Expr::Lit(Value::Text(_), Text)));
                keys_ok.then_some(Jsonb)
            },
            (Func::JsonbBuildArray, ts) if ts.len() <= 4 => Some(Jsonb),
            (Func::JsonbStripNulls, [Jsonb]) => Some(Jsonb),
            (Func::JsonbConcat, [Jsonb, Jsonb]) => Some(Jsonb),
            (Func::JsonbGet, [Jsonb, Text | Int4]) => Some(Jsonb),
            (Func::JsonbGetText, [Jsonb, Text | Int4]) => Some(Text),
            (Func::JsonbGetPath(k), [Jsonb]) if (k as usize) < JSON_PATHS.len() => Some(Jsonb),
            (Func::JsonbGetPathText(k), [Jsonb]) if (k as usize) < JSON_PATHS.len() => Some(Text),
            (Func::JsonbTypeof | Func::JsonbPretty, [Jsonb]) => Some(Text),
            (Func::JsonbArrayLength, [Jsonb]) if matches!(&args[0], Expr::Func(Func::JsonbBuildArray, _)) => Some(Int4),
            (Func::JsonbContains | Func::JsonbContained, [Jsonb, Jsonb]) => Some(Bool),
            (Func::JsonbHasKey, [Jsonb, Text]) => Some(Bool),
            (Func::JsonbHasAny(k) | Func::JsonbHasAll(k), [Jsonb]) if (k as usize) < JSON_KEYSETS.len() => Some(Bool),
            (Func::JsonbPathExists(k), [Jsonb]) if (k as usize) < JSONPATHS.len() => Some(Bool),
            (Func::JsonbPathMatch(k), [Jsonb]) if (k as usize) < JSONPATH_PREDICATES.len() => Some(Bool),
            (Func::JsonbPathQueryFirst(k), [Jsonb]) if (k as usize) < JSONPATHS.len() => Some(Jsonb),
            (Func::IntervalLit(k), []) if (k as usize) < INTERVALS.len() => Some(Interval),
            (Func::TsPlus | Func::TsMinus, [Timestamp | Date, Interval]) => Some(Timestamp),
            (Func::TsDiff | Func::Age, [Timestamp, Timestamp]) => Some(Interval),
            (Func::IntervalAdd | Func::IntervalSub, [Interval, Interval]) => Some(Interval),
            (Func::IntervalMul, [Interval, Float8]) if matches!(args[1], Expr::Lit(Value::Float(_), _)) => Some(Interval),
            (Func::IntervalDiv, [Interval, Float8]) if matches!(args[1], Expr::Lit(Value::Float(x), _) if x != 0.0 && x.is_finite()) => Some(Interval),
            (Func::JustifyDays | Func::JustifyHours | Func::JustifyInterval, [Interval]) => Some(Interval),
            (Func::Extract(k, true), [Interval]) if (k as usize) < INTERVAL_FIELDS.len() => Some(Numeric),
            (Func::Extract(k, false), [Timestamp]) if (k as usize) < TIMESTAMP_FIELDS.len() => Some(Numeric),
            (Func::DatePart(k, true), [Interval]) if (k as usize) < INTERVAL_FIELDS.len() => Some(Float8),
            (Func::DatePart(k, false), [Timestamp]) if (k as usize) < TIMESTAMP_FIELDS.len() => Some(Float8),
            (Func::ToChar(k), [Timestamp]) if (k as usize) < TO_CHAR_FORMATS.len() => Some(Text),
            // Of an interval, no AM/PM (an error) and no names.
            (Func::ToChar(k), [Interval]) if INTERVAL_FORMATS.contains(&(k as usize)) => Some(Text),
            (Func::DateBin(k), [Timestamp]) if (k as usize) < BIN_STRIDES.len() => Some(Timestamp),
            (Func::Array(t), ts) if ts.len() <= 4 && ts.iter().all(|u| u == &t) => t.array(),
            (Func::ArrayAppend | Func::ArrayRemove, [a, e]) if a.element() == Some(*e) => Some(*a),
            (Func::ArrayPrepend, [e, a]) if a.element() == Some(*e) => Some(*a),
            (Func::ArrayReplace, [a, e, f]) if a.element() == Some(*e) && e == f => Some(*a),
            (Func::ArrayCat, [a, b]) if a.element().is_some() && a == b => Some(*a),
            (Func::Subscript, [a, Int4]) => a.element(),
            (Func::Slice, [a, Int4, Int4]) if a.element().is_some() && matches!(args[1], Expr::Lit(Value::Int(_), _)) && matches!(args[2], Expr::Lit(Value::Int(_), _)) => Some(*a),
            (Func::StringToArray, [Text, Text]) if lit_text(&args[1]) => Some(TextArray),
            (Func::ArrayToString, [a, Text]) if a.element().is_some() && lit_text(&args[1]) => Some(Text),
            (Func::Cardinality | Func::ArrayLength | Func::ArrayUpper, [a]) if a.element().is_some() => Some(Int4),
            (Func::ArrayPosition, [a, e]) if a.element() == Some(*e) => Some(Int4),
            (Func::Any(_) | Func::All(_), [e, a]) if a.element() == Some(*e) => Some(Bool),
            (Func::ArrayContains | Func::ArrayContained | Func::ArrayOverlaps, [a, b]) if a.element().is_some() && a == b => Some(Bool),
            _ => None,
        }
    }
}

/// Aggregate functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggFunc {
    /// `count(*)` (no argument) or `count([DISTINCT] e)`.
    Count,
    /// `sum` and `avg` of integers and numerics (a real sum depends on the
    /// order rows are summed in).
    Sum,
    Avg,
    /// `min` and `max` of any type but reals and numerics (which of equal
    /// values they return depends on the order of rows), bool and bytes
    /// (which have none).
    Min,
    Max,
    BoolAnd,
    BoolOr,
    /// `string_agg(DISTINCT e, ',' ORDER BY e)`: determined.
    StringAgg,
}

impl AggFunc {
    pub const ALL: [AggFunc; 8] = [AggFunc::Count, AggFunc::Sum, AggFunc::Avg, AggFunc::Min, AggFunc::Max, AggFunc::BoolAnd, AggFunc::BoolOr, AggFunc::StringAgg];

    fn name(self) -> &'static str {
        match self {
            AggFunc::Count => "count",
            AggFunc::Sum => "sum",
            AggFunc::Avg => "avg",
            AggFunc::Min => "min",
            AggFunc::Max => "max",
            AggFunc::BoolAnd => "bool_and",
            AggFunc::BoolOr => "bool_or",
            AggFunc::StringAgg => "string_agg",
        }
    }

    /// The result type for an argument of type `arg` (None for `count(*)`).
    pub fn ret(self, arg: Option<Type>) -> Option<Type> {
        use Type::*;
        match (self, arg) {
            (AggFunc::Count, _) => Some(Int8),
            (AggFunc::Sum, Some(Int2 | Int4)) => Some(Int8),
            (AggFunc::Sum, Some(Int8 | Numeric)) => Some(Numeric),
            (AggFunc::Avg, Some(Int2 | Int4 | Int8 | Numeric)) => Some(Numeric),
            (AggFunc::Min | AggFunc::Max, Some(t)) if !t.is_float() && !t.internal() && !matches!(t, Numeric | Bool | Bytea) => Some(t),
            (AggFunc::BoolAnd | AggFunc::BoolOr, Some(Bool)) => Some(Bool),
            (AggFunc::StringAgg, Some(Text)) => Some(Text),
            _ => None,
        }
    }
}

/// Window functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WinFunc {
    /// `row_number()`: needs a total order.
    RowNumber,
    /// `rank()`, `dense_rank()`: `int8`; `percent_rank()`, `cume_dist()`:
    /// `float8`. Determined by partitions and peers alone.
    Rank,
    DenseRank,
    PercentRank,
    CumeDist,
    /// `ntile(k)`: `int4`; needs a total order.
    Ntile(u32),
    /// `lag(e, k)`, `lead(e, k)`, `first_value(e)`, `last_value(e)`,
    /// `nth_value(e, k)`: the type of `e`; need a total order.
    Lag(u32),
    Lead(u32),
    FirstValue,
    LastValue,
    NthValue(u32),
    /// An aggregate over the frame (no DISTINCT, which PostgreSQL does not
    /// implement for windows; no `string_agg`).
    Agg(AggFunc),
}

impl WinFunc {
    /// Whether its values depend on the order of peers, so that its window
    /// must order by every column of the query's sources.
    pub fn needs_total(self) -> bool {
        !matches!(self, WinFunc::Rank | WinFunc::DenseRank | WinFunc::PercentRank | WinFunc::CumeDist | WinFunc::Agg(_))
    }

    /// The result type for these argument types, or None.
    pub fn ret(self, args: &[Type]) -> Option<Type> {
        use Type::*;
        match (self, args) {
            (WinFunc::RowNumber | WinFunc::Rank | WinFunc::DenseRank, []) => Some(Int8),
            (WinFunc::PercentRank | WinFunc::CumeDist, []) => Some(Float8),
            (WinFunc::Ntile(k), []) if k >= 1 => Some(Int4),
            (WinFunc::Lag(_) | WinFunc::Lead(_) | WinFunc::FirstValue | WinFunc::LastValue, [t]) => Some(*t),
            (WinFunc::NthValue(k), [t]) if k >= 1 => Some(*t),
            (WinFunc::Agg(AggFunc::Count), []) => Some(Int8),
            (WinFunc::Agg(f), [t]) if f != AggFunc::StringAgg => f.ret(Some(*t)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameMode {
    Rows,
    Range,
    Groups,
}

/// A frame bound; offsets only for ROWS and GROUPS (a RANGE offset needs a
/// single numeric or date ORDER BY term).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    UnboundedPreceding,
    Preceding(u32),
    CurrentRow,
    Following(u32),
    UnboundedFollowing,
}

impl Bound {
    fn rank(self) -> u8 {
        match self {
            Bound::UnboundedPreceding => 0,
            Bound::Preceding(_) => 1,
            Bound::CurrentRow => 2,
            Bound::Following(_) => 3,
            Bound::UnboundedFollowing => 4,
        }
    }

    fn sql(self) -> String {
        match self {
            Bound::UnboundedPreceding => "UNBOUNDED PRECEDING".into(),
            Bound::Preceding(k) => format!("{k} PRECEDING"),
            Bound::CurrentRow => "CURRENT ROW".into(),
            Bound::Following(k) => format!("{k} FOLLOWING"),
            Bound::UnboundedFollowing => "UNBOUNDED FOLLOWING".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exclude {
    NoOthers,
    CurrentRow,
    Group,
    Ties,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub mode: FrameMode,
    pub start: Bound,
    pub end: Bound,
    pub exclude: Exclude,
}

impl Frame {
    /// Whether PostgreSQL takes it: the start not after the end, neither
    /// unbounded the wrong way, offsets not in RANGE.
    pub fn ok(&self) -> bool {
        self.start.rank() <= self.end.rank()
            && self.start != Bound::UnboundedFollowing
            && self.end != Bound::UnboundedPreceding
            && (self.mode != FrameMode::Range || ![self.start, self.end].iter().any(|b| matches!(b, Bound::Preceding(_) | Bound::Following(_))))
    }

    /// Whether its rows depend on the order of peers (ROWS does).
    pub fn needs_total(&self) -> bool {
        self.mode == FrameMode::Rows
    }

    fn sql(&self) -> String {
        let mode = match self.mode {
            FrameMode::Rows => "ROWS",
            FrameMode::Range => "RANGE",
            FrameMode::Groups => "GROUPS",
        };
        let ex = match self.exclude {
            Exclude::NoOthers => "",
            Exclude::CurrentRow => " EXCLUDE CURRENT ROW",
            Exclude::Group => " EXCLUDE GROUP",
            Exclude::Ties => " EXCLUDE TIES",
        };
        format!("{mode} BETWEEN {} AND {}{ex}", self.start.sql(), self.end.sql())
    }
}

/// A window function call with its window.
#[derive(Clone, Debug, PartialEq)]
pub struct WinCall {
    pub func: WinFunc,
    pub args: Vec<Expr>,
    pub partition: Vec<Expr>,
    /// ORDER BY terms: (term, desc, nulls first).
    pub order: Vec<(Expr, bool, Option<bool>)>,
    pub frame: Option<Frame>,
}

/// A column reference: column `c{column}` of source `src` of the query
/// `up` levels out (0: the expression's own query), of type `ty`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Col {
    pub up: usize,
    pub src: usize,
    pub column: u32,
    pub ty: Type,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsWhat {
    True,
    False,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Col(Col),
    /// A literal of a type (NULL included): `'..'::type`.
    Lit(Value, Type),
    Cmp(CmpOp, Box<Expr>, Box<Expr>),
    /// `a IS [NOT] DISTINCT FROM b` (`true`: NOT).
    Distinct(bool, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// `e IS [NOT] NULL`.
    IsNull(Box<Expr>, bool),
    /// `e IS [NOT] TRUE | FALSE | UNKNOWN`.
    Is(Box<Expr>, IsWhat, bool),
    /// `e [NOT] BETWEEN [SYMMETRIC] lo AND hi`: (not, symmetric).
    Between { e: Box<Expr>, lo: Box<Expr>, hi: Box<Expr>, not: bool, symmetric: bool },
    InList(Box<Expr>, Vec<Expr>, bool),
    /// `e [NOT] LIKE | ILIKE 'p'` (`insensitive`), the pattern a text
    /// literal.
    Like { e: Box<Expr>, pattern: String, not: bool, insensitive: bool },
    /// `e ~ 'p'`, a pattern of [`REGEXES`].
    Regex(Box<Expr>, usize),
    Arith(Arith, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    /// `a || b`, text or bytes.
    Concat(Box<Expr>, Box<Expr>),
    Case { operand: Option<Box<Expr>>, whens: Vec<(Expr, Expr)>, otherwise: Option<Box<Expr>> },
    Coalesce(Vec<Expr>),
    NullIf(Box<Expr>, Box<Expr>),
    Greatest(Vec<Expr>),
    Least(Vec<Expr>),
    Cast(Box<Expr>, Type),
    Func(Func, Vec<Expr>),
    /// An aggregate: `count(*)` without an argument; DISTINCT when given.
    Agg(AggFunc, bool, Option<Box<Expr>>),
    Exists(Box<Query>, bool),
    InQuery(Box<Expr>, Box<Query>, bool),
    /// A scalar subquery: one aggregate item, no GROUP BY, so one row.
    Scalar(Box<Query>),
    /// A window function, only among the items of a non-aggregate SELECT at
    /// the top level or of a compound member.
    Window(Box<WinCall>),
}

/// Regular expressions that are valid and cheap.
pub const REGEXES: [&str; 8] = ["^a", "b$", "[0-9]", "^$", "a.*b", "(ab|ba)", "^[^a-z]*$", "\\s"];

/// Whether a LIKE pattern raises no error: it does not end with an escape
/// (a backslash not itself escaped).
pub fn pattern_ok(p: &str) -> bool {
    let mut escaped = false;
    for ch in p.chars() {
        escaped = !escaped && ch == '\\';
    }
    !escaped
}

/// The result type of `a op b`, as PostgreSQL resolves it for the types
/// this language combines: integers of any widths (the wider), numerics
/// with numerics or integers, reals of the same width (`float4 op float4`
/// stays `float4`), a date plus or minus an `int4` (a date), a date minus
/// a date (an `int4`). `%` takes no real.
pub fn arith_type(op: Arith, a: Type, b: Type) -> Option<Type> {
    use Type::*;
    let wider = |a: Type, b: Type| if a >= b { a } else { b };
    match (a, b) {
        _ if a.is_int() && b.is_int() => Some(wider(a, b)),
        (Numeric, t) | (t, Numeric) if t == Numeric || t.is_int() => Some(Numeric),
        (Float4, Float4) | (Float8, Float8) if op != Arith::Mod => Some(a),
        (Date, Int4) if matches!(op, Arith::Add | Arith::Sub) => Some(Date),
        (Date, Date) if op == Arith::Sub => Some(Int4),
        _ => None,
    }
}

/// Whether PostgreSQL compares a value of `a` with one of `b` without a
/// cast that could fail.
pub fn comparable(a: Type, b: Type) -> bool {
    a == b
        || a.is_int() && b.is_int()
        || (a.is_int() || a == Type::Numeric) && (b.is_int() || b == Type::Numeric)
        || a.is_float() && b.is_float()
        || matches!((a, b), (Type::Date, Type::Timestamp) | (Type::Timestamp, Type::Date))
}

/// Whether `CAST(x AS to)` of a value of `from` is allowed: PostgreSQL has
/// the cast, and it fails at most with an expected run-time error (an
/// integer out of range, NaN to an integer, a date past the last
/// timestamp). Text is never cast to another type (its parse fails on
/// most values).
pub fn cast_ok(from: Type, to: Type) -> bool {
    use Type::*;
    from == to
        || to == Text
        || from.is_int() && (to.is_int() || to.is_float() || to == Numeric)
        || (from.is_float() || from == Numeric) && (to.is_int() || to.is_float() || to == Numeric)
        || matches!((from, to), (Bool, Int4) | (Int4, Bool) | (Date, Timestamp) | (Timestamp, Date))
}

/// The most nesting of expressions.
pub const MAX_DEPTH: usize = 8;

impl Expr {
    fn bx(self) -> Box<Expr> {
        Box::new(self)
    }

    pub fn null(ty: Type) -> Expr {
        Expr::Lit(Value::Null, ty)
    }

    /// Every direct child expression (not those inside subqueries).
    pub fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Col(_) | Expr::Lit(..) | Expr::Exists(..) | Expr::Scalar(_) => vec![],
            Expr::Cmp(_, a, b) | Expr::Distinct(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::NullIf(a, b) => vec![a, b],
            Expr::Not(a) | Expr::IsNull(a, _) | Expr::Is(a, ..) | Expr::Neg(a) | Expr::Cast(a, _) | Expr::Regex(a, _) | Expr::InQuery(a, ..) => vec![a],
            Expr::Like { e, .. } => vec![e],
            Expr::Between { e, lo, hi, .. } => vec![e, lo, hi],
            Expr::InList(a, list, _) => std::iter::once(&**a).chain(list).collect(),
            Expr::Case { operand, whens, otherwise } => {
                operand.iter().map(|b| &**b).chain(whens.iter().flat_map(|(w, t)| [w, t])).chain(otherwise.iter().map(|b| &**b)).collect()
            },
            Expr::Coalesce(xs) | Expr::Greatest(xs) | Expr::Least(xs) | Expr::Func(_, xs) => xs.iter().collect(),
            Expr::Agg(_, _, a) => a.iter().map(|b| &**b).collect(),
            Expr::Window(w) => w.args.iter().chain(w.partition.iter()).chain(w.order.iter().map(|o| &o.0)).collect(),
        }
    }

    /// The subqueries directly in it.
    pub fn subqueries(&self) -> Vec<&Query> {
        let mut out = Vec::new();
        self.walk(&mut |e| match e {
            Expr::Exists(q, _) | Expr::InQuery(_, q, _) | Expr::Scalar(q) => out.push(&**q),
            _ => {},
        });
        out
    }

    /// Visit this expression and every expression below it, not entering
    /// subqueries.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        f(self);
        for c in self.children() {
            c.walk(f);
        }
    }

    pub fn has_agg(&self) -> bool {
        let mut found = false;
        self.walk(&mut |e| found |= matches!(e, Expr::Agg(..)));
        found
    }

    pub fn has_subquery(&self) -> bool {
        !self.subqueries().is_empty()
    }

    /// A simplicity measure.
    pub fn cost(&self) -> u64 {
        1 + match self {
            Expr::Lit(v, _) => v.cost() / 4,
            Expr::Exists(q, _) | Expr::Scalar(q) => 2 + q.cost(),
            Expr::InQuery(a, q, _) => 2 + a.cost() + q.cost(),
            Expr::Like { e, pattern, .. } => e.cost() + pattern.len() as u64 / 4,
            _ => self.children().iter().map(|c| c.cost()).sum::<u64>(),
        }
    }

    /// Render, the expression being in a query at nesting depth `depth`.
    pub fn sql(&self, depth: usize) -> String {
        let r = |e: &Expr| e.sql(depth);
        let list = |xs: &[Expr]| xs.iter().map(r).collect::<Vec<_>>().join(", ");
        let not = |b: bool| if b { "NOT " } else { "" };
        match self {
            Expr::Col(c) => format!("{}.c{}", alias(depth - c.up.min(depth), c.src), c.column),
            Expr::Lit(v, ty) => format!("{}::{}", literal(v), ty.name()),
            Expr::Cmp(op, a, b) => format!("({} {} {})", r(a), op.sql(), r(b)),
            Expr::Distinct(n, a, b) => format!("({} IS {}DISTINCT FROM {})", r(a), not(*n), r(b)),
            Expr::And(a, b) => format!("({} AND {})", r(a), r(b)),
            Expr::Or(a, b) => format!("({} OR {})", r(a), r(b)),
            Expr::Not(a) => format!("(NOT {})", r(a)),
            Expr::IsNull(a, n) => format!("({} IS {}NULL)", r(a), not(*n)),
            Expr::Is(a, what, n) => format!(
                "({} IS {}{})",
                r(a),
                not(*n),
                match what {
                    IsWhat::True => "TRUE",
                    IsWhat::False => "FALSE",
                    IsWhat::Unknown => "UNKNOWN",
                }
            ),
            Expr::Between { e, lo, hi, not: n, symmetric } => {
                format!("({} {}BETWEEN {}{} AND {})", r(e), not(*n), if *symmetric { "SYMMETRIC " } else { "" }, r(lo), r(hi))
            },
            Expr::InList(a, xs, n) => format!("({} {}IN ({}))", r(a), not(*n), list(xs)),
            Expr::Like { e, pattern, not: n, insensitive } => {
                format!("({} {}{} {})", r(e), not(*n), if *insensitive { "ILIKE" } else { "LIKE" }, literal(&Value::Text(pattern.clone())))
            },
            Expr::Regex(a, k) => format!("({} ~ {})", r(a), literal(&Value::Text(REGEXES[*k].into()))),
            Expr::Arith(op, a, b) => format!("({} {} {})", r(a), op.sql(), r(b)),
            Expr::Neg(a) => format!("(- {})", r(a)),
            Expr::Concat(a, b) => format!("({} || {})", r(a), r(b)),
            Expr::Case { operand, whens, otherwise } => {
                let mut s = String::from("(CASE");
                if let Some(o) = operand {
                    write!(s, " {}", r(o)).unwrap();
                }
                for (w, t) in whens {
                    write!(s, " WHEN {} THEN {}", r(w), r(t)).unwrap();
                }
                if let Some(e) = otherwise {
                    write!(s, " ELSE {}", r(e)).unwrap();
                }
                s.push_str(" END)");
                s
            },
            Expr::Coalesce(xs) => format!("coalesce({})", list(xs)),
            Expr::NullIf(a, b) => format!("nullif({}, {})", r(a), r(b)),
            Expr::Greatest(xs) => format!("greatest({})", list(xs)),
            Expr::Least(xs) => format!("least({})", list(xs)),
            Expr::Cast(a, ty) => format!("CAST({} AS {})", r(a), ty.name()),
            Expr::Func(f, xs) => f.sql(&xs.iter().map(r).collect::<Vec<_>>()),
            Expr::Agg(AggFunc::Count, _, None) => "count(*)".into(),
            Expr::Agg(AggFunc::StringAgg, _, Some(a)) => format!("string_agg(DISTINCT {0}, ',' ORDER BY {0})", r(a)),
            Expr::Agg(f, d, a) => format!("{}({}{})", f.name(), if *d { "DISTINCT " } else { "" }, a.as_ref().map_or(String::new(), |a| r(a))),
            Expr::Exists(q, n) => format!("({}EXISTS ({}))", not(*n), q.sql_at(depth + 1)),
            Expr::InQuery(a, q, n) => format!("({} {}IN ({}))", r(a), not(*n), q.sql_at(depth + 1)),
            Expr::Scalar(q) => format!("({})", q.sql_at(depth + 1)),
            Expr::Window(w) => {
                let call = match w.func {
                    WinFunc::RowNumber => "row_number()".into(),
                    WinFunc::Rank => "rank()".into(),
                    WinFunc::DenseRank => "dense_rank()".into(),
                    WinFunc::PercentRank => "percent_rank()".into(),
                    WinFunc::CumeDist => "cume_dist()".into(),
                    WinFunc::Ntile(k) => format!("ntile({k})"),
                    WinFunc::Lag(k) => format!("lag({}, {k})", list(&w.args)),
                    WinFunc::Lead(k) => format!("lead({}, {k})", list(&w.args)),
                    WinFunc::FirstValue => format!("first_value({})", list(&w.args)),
                    WinFunc::LastValue => format!("last_value({})", list(&w.args)),
                    WinFunc::NthValue(k) => format!("nth_value({}, {k})", list(&w.args)),
                    WinFunc::Agg(AggFunc::Count) if w.args.is_empty() => "count(*)".into(),
                    WinFunc::Agg(f) => format!("{}({})", f.name(), list(&w.args)),
                };
                let mut spec = Vec::new();
                if !w.partition.is_empty() {
                    spec.push(format!("PARTITION BY {}", list(&w.partition)));
                }
                if !w.order.is_empty() {
                    let terms: Vec<String> = w
                        .order
                        .iter()
                        .map(|(e, desc, nulls)| {
                            let n = match nulls {
                                Some(true) => " NULLS FIRST",
                                Some(false) => " NULLS LAST",
                                None => "",
                            };
                            format!("{}{}{n}", r(e), if *desc { " DESC" } else { "" })
                        })
                        .collect();
                    spec.push(format!("ORDER BY {}", terms.join(", ")));
                }
                if let Some(f) = &w.frame {
                    spec.push(f.sql());
                }
                format!("{call} OVER ({})", spec.join(" "))
            },
        }
    }
}

/// The alias of source `src` of a query at nesting depth `depth`.
pub fn alias(depth: usize, src: usize) -> String {
    format!("a{depth}_{src}")
}

// ---------------------------------------------------------------- queries

/// A source in FROM: a table by id, a CTE of the top-level query by
/// position, set-returning functions ([`FromFn`]), or a subquery.
///
/// A function's arguments and a subquery may read the sources before it
/// in the same FROM (a LATERAL reference) when it is joined by an inner,
/// left or cross join (PostgreSQL refuses a LATERAL reference from the
/// right side of a RIGHT or FULL JOIN), and the queries around it (a
/// correlated subquery). A subquery is `LATERAL (SELECT ..) AS a(c0, ..)`
/// (`LATERAL` written wherever it may read the sources before it), in the
/// shape of a CTE's query (rows determined) or of a scalar subquery (one
/// aggregate row).
#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    Table(u32),
    Cte(usize),
    Func(Box<FromFn>),
    Sub(Box<Select>),
}

/// Set-returning functions in FROM: one call, or two as `ROWS FROM (f,
/// g)` (the shorter padded with NULLs), `WITH ORDINALITY` adding an `int8`
/// column; the columns aliased `c0, c1, ..` in order.
#[derive(Clone, Debug, PartialEq)]
pub struct FromFn {
    pub calls: Vec<Call>,
    pub ordinality: bool,
}

/// The steps of a timestamp `generate_series`.
pub const SERIES_STEPS: [&str; 4] = ["1 day", "6 hours", "1 mon", "-2 days"];

/// The most rows a `generate_series` in FROM gives.
pub const SERIES_MAX: u64 = 40;

/// A set-returning function call. Every one gives a number of rows bounded
/// by its form (for the row budget), never an error.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// `generate_series(lo, hi, step)` of `int8` literals, step not 0, at
    /// most [`SERIES_MAX`] rows: `int8`.
    Series(i64, i64, i64),
    /// `generate_series('1'::int8, least(e, 'k'::int8))`, `e` an `int8`
    /// expression (a LATERAL bound mostly), `k` at most 8: `int8`, at most
    /// `k` rows (none for NULL).
    SeriesTo(Expr, u8),
    /// `generate_series(lo, lo + n * step, step)` of timestamps, `step` an
    /// interval of [`SERIES_STEPS`], `n` at most 10: `timestamp`.
    SeriesTs(i64, u8, u8),
    /// `generate_series(lo, hi, step)` of numerics in tenths (`step` not
    /// 0, at most [`SERIES_MAX`] rows): `numeric`.
    SeriesNum(i64, i64, i64),
    /// `string_to_table(t, sep)`, `t` and `sep` text literals of at most 12
    /// characters: text.
    StringTable(String, String),
    /// `unnest(a)` of an `int8[]` or `text[]` expression whose length has a
    /// bound ([`array_bound`]): the element type.
    Unnest(Expr),
    /// `unnest(ARRAY[e, ..])` of 1 to 4 expressions of a column type (not
    /// expression-only): that type.
    UnnestList(Type, Vec<Expr>),
    /// `jsonb_each_text(o)`, `o` a `jsonb_build_object` call (of an object,
    /// else an error): key and value, text.
    JsonbEach(Expr),
    /// `jsonb_array_elements_text(a)`, `a` a `jsonb_build_array` call:
    /// text.
    JsonbElems(Expr),
    /// `JSON_TABLE(a, '$[*]' COLUMNS (c0 .., ..))`, `a` a
    /// `jsonb_build_array` call (a row per element), alone in its
    /// [`FromFn`] (no ROWS FROM, no ordinality): 1 to 4 columns
    /// ([`JtCol`]). A value that does not convert is NULL (the columns'
    /// default `NULL ON ERROR`), so never an error.
    JsonTable(Expr, Vec<JtCol>),
}

/// The paths of JSON_TABLE's columns, relative to an element.
pub const JT_PATHS: [&str; 6] = ["$", "$.a", "$.b", "$.x", "$[0]", "$.a[1]"];

/// A JSON_TABLE column: `FOR ORDINALITY` (`int4`), `t PATH 'p'` of a type
/// of [`JT_TYPES`], `bool EXISTS PATH 'p'`. (No `jsonb` column: an
/// expression-only type is never a source's column.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JtCol {
    Ordinality,
    Path(Type, u8),
    Exists(u8),
}

pub const JT_TYPES: [Type; 7] = [Type::Int4, Type::Int8, Type::Numeric, Type::Float8, Type::Text, Type::Bool, Type::Date];

impl JtCol {
    fn ty(self) -> Type {
        match self {
            JtCol::Ordinality => Type::Int4,
            JtCol::Path(t, _) => t,
            JtCol::Exists(_) => Type::Bool,
        }
    }

    fn ok(self) -> bool {
        match self {
            JtCol::Ordinality => true,
            JtCol::Path(t, p) => JT_TYPES.contains(&t) && (p as usize) < JT_PATHS.len(),
            JtCol::Exists(p) => (p as usize) < JT_PATHS.len(),
        }
    }

    fn sql(self, k: usize) -> String {
        match self {
            JtCol::Ordinality => format!("c{k} FOR ORDINALITY"),
            JtCol::Path(t, p) => format!("c{k} {} PATH '{}'", t.name(), JT_PATHS[p as usize]),
            JtCol::Exists(p) => format!("c{k} bool EXISTS PATH '{}'", JT_PATHS[p as usize]),
        }
    }
}

impl Call {
    fn args(&self) -> Vec<&Expr> {
        match self {
            Call::Series(..) | Call::SeriesTs(..) | Call::SeriesNum(..) | Call::StringTable(..) => vec![],
            Call::SeriesTo(e, _) => vec![e],
            Call::Unnest(e) | Call::JsonbEach(e) | Call::JsonbElems(e) | Call::JsonTable(e, _) => vec![e],
            Call::UnnestList(_, xs) => xs.iter().collect(),
        }
    }

    fn args_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            Call::Series(..) | Call::SeriesTs(..) | Call::SeriesNum(..) | Call::StringTable(..) => vec![],
            Call::SeriesTo(e, _) => vec![e],
            Call::Unnest(e) | Call::JsonbEach(e) | Call::JsonbElems(e) | Call::JsonTable(e, _) => vec![e],
            Call::UnnestList(_, xs) => xs.iter_mut().collect(),
        }
    }

    /// Its column types, as drawn (unchecked).
    fn types(&self) -> Vec<Type> {
        match self {
            Call::Series(..) | Call::SeriesTo(..) => vec![Type::Int8],
            Call::SeriesTs(..) => vec![Type::Timestamp],
            Call::SeriesNum(..) => vec![Type::Numeric],
            Call::StringTable(..) => vec![Type::Text],
            Call::Unnest(e) => vec![type_of(e).element().unwrap_or(Type::Int8)],
            Call::UnnestList(t, _) => vec![*t],
            Call::JsonbEach(_) => vec![Type::Text, Type::Text],
            Call::JsonbElems(_) => vec![Type::Text],
            Call::JsonTable(_, cols) => cols.iter().map(|c| c.ty()).collect(),
        }
    }

    /// The most rows it gives, when bounded.
    fn rows(&self) -> Option<u64> {
        match self {
            &Call::Series(lo, hi, step) => {
                if step == 0 {
                    return None;
                }
                let span = if step > 0 { i128::from(hi) - i128::from(lo) } else { i128::from(lo) - i128::from(hi) };
                let n = if span < 0 { 0 } else { span / i128::from(step).abs() + 1 };
                u64::try_from(n).ok()
            },
            &Call::SeriesNum(lo, hi, step) => Call::Series(lo, hi, step).rows(),
            Call::SeriesTo(_, k) => Some(u64::from(*k)),
            Call::SeriesTs(_, n, _) => Some(u64::from(*n) + 1),
            Call::StringTable(t, _) => Some(t.chars().count() as u64 + 1),
            Call::Unnest(e) => array_bound(e),
            Call::UnnestList(_, xs) => Some(xs.len() as u64),
            Call::JsonbEach(Expr::Func(Func::JsonbBuildObject, args)) => Some(args.len() as u64 / 2),
            Call::JsonbElems(Expr::Func(Func::JsonbBuildArray, args)) | Call::JsonTable(Expr::Func(Func::JsonbBuildArray, args), _) => Some(args.len() as u64),
            _ => None,
        }
    }

    fn sql(&self, depth: usize) -> String {
        match self {
            Call::Series(lo, hi, step) => format!("generate_series('{lo}'::int8, '{hi}'::int8, '{step}'::int8)"),
            Call::SeriesTo(e, k) => format!("generate_series('1'::int8, least({}, '{k}'::int8))", e.sql(depth)),
            Call::SeriesTs(lo, n, step) => {
                let lo = literal(&Value::Timestamp(*lo));
                let st = SERIES_STEPS[*step as usize];
                format!("generate_series({lo}::timestamp, {lo}::timestamp + {n} * CAST('{st}' AS interval), CAST('{st}' AS interval))")
            },
            Call::SeriesNum(lo, hi, step) => {
                let d = |x: i64| format!("'{}{}.{}'::numeric", if x < 0 { "-" } else { "" }, x.abs() / 10, x.abs() % 10);
                format!("generate_series({}, {}, {})", d(*lo), d(*hi), d(*step))
            },
            Call::StringTable(t, sep) => format!("string_to_table({}::text, {}::text)", literal(&Value::Text(t.clone())), literal(&Value::Text(sep.clone()))),
            Call::Unnest(e) => format!("unnest({})", e.sql(depth)),
            Call::UnnestList(_, xs) => format!("unnest(ARRAY[{}])", xs.iter().map(|e| e.sql(depth)).collect::<Vec<_>>().join(", ")),
            Call::JsonbEach(e) => format!("jsonb_each_text({})", e.sql(depth)),
            Call::JsonbElems(e) => format!("jsonb_array_elements_text({})", e.sql(depth)),
            Call::JsonTable(e, cols) => format!("JSON_TABLE({}, '$[*]' COLUMNS ({}))", e.sql(depth), cols.iter().enumerate().map(|(k, c)| c.sql(k)).collect::<Vec<_>>().join(", ")),
        }
    }

    fn cost(&self) -> u64 {
        let cols = if let Call::JsonTable(_, cols) = self { cols.len() as u64 } else { 0 };
        1 + cols + self.args().iter().map(|e| e.cost()).sum::<u64>()
    }
}

/// The most elements an array expression has, when bounded: `ARRAY[..]`
/// and what appends, concatenates, removes, replaces or slices those
/// (`string_to_array` of a value is unbounded).
pub fn array_bound(e: &Expr) -> Option<u64> {
    match e {
        Expr::Func(Func::Array(_), xs) => Some(xs.len() as u64),
        Expr::Func(Func::ArrayAppend, xs) => Some(array_bound(xs.first()?)? + 1),
        Expr::Func(Func::ArrayPrepend, xs) => Some(array_bound(xs.get(1)?)? + 1),
        Expr::Func(Func::ArrayCat, xs) => Some(array_bound(xs.first()?)? + array_bound(xs.get(1)?)?),
        Expr::Func(Func::ArrayRemove | Func::ArrayReplace | Func::Slice, xs) => array_bound(xs.first()?),
        Expr::Case { whens, otherwise, .. } => {
            let mut most = 0;
            for (_, t) in whens {
                most = most.max(array_bound(t)?);
            }
            Some(most.max(array_bound(otherwise.as_deref()?)?))
        },
        Expr::Lit(Value::Null, _) => Some(0),
        _ => None,
    }
}

impl FromFn {
    /// Whether it is a JSON_TABLE (its columns named in it).
    pub fn table(&self) -> bool {
        self.calls.iter().any(|c| matches!(c, Call::JsonTable(..)))
    }

    pub fn types(&self) -> Vec<Type> {
        let mut ts: Vec<Type> = self.calls.iter().flat_map(Call::types).collect();
        if self.ordinality {
            ts.push(Type::Int8);
        }
        ts
    }

    /// The most rows it gives (each call's most), when bounded.
    pub fn rows(&self) -> Option<u64> {
        self.calls.iter().map(Call::rows).try_fold(0u64, |a, r| Some(a.max(r?)))
    }

    pub fn args(&self) -> Vec<&Expr> {
        self.calls.iter().flat_map(Call::args).collect()
    }

    fn sql(&self, depth: usize) -> String {
        let calls: Vec<String> = self.calls.iter().map(|c| c.sql(depth)).collect();
        let body = if calls.len() == 1 { calls[0].clone() } else { format!("ROWS FROM ({})", calls.join(", ")) };
        format!("{body}{}", if self.ordinality { " WITH ORDINALITY" } else { "" })
    }

    fn cost(&self) -> u64 {
        2 + u64::from(self.ordinality) + self.calls.iter().map(Call::cost).sum::<u64>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

impl JoinKind {
    fn sql(self) -> &'static str {
        match self {
            JoinKind::Inner => "JOIN",
            JoinKind::Left => "LEFT JOIN",
            JoinKind::Right => "RIGHT JOIN",
            JoinKind::Full => "FULL JOIN",
            JoinKind::Cross => "CROSS JOIN",
        }
    }
}

/// A join of the next source. ON for every kind but CROSS; for FULL, a
/// conjunction of equalities of a column of the new source with a column
/// of an earlier one (PostgreSQL runs a FULL JOIN only on merge- or
/// hash-joinable conditions).
#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    pub kind: JoinKind,
    pub src: Src,
    pub on: Option<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOp {
    Union,
    UnionAll,
    Intersect,
    IntersectAll,
    Except,
    ExceptAll,
}

impl SetOp {
    pub const ALL: [SetOp; 6] = [SetOp::Union, SetOp::UnionAll, SetOp::Intersect, SetOp::IntersectAll, SetOp::Except, SetOp::ExceptAll];

    pub fn sql(self) -> &'static str {
        match self {
            SetOp::Union => "UNION",
            SetOp::UnionAll => "UNION ALL",
            SetOp::Intersect => "INTERSECT",
            SetOp::IntersectAll => "INTERSECT ALL",
            SetOp::Except => "EXCEPT",
            SetOp::ExceptAll => "EXCEPT ALL",
        }
    }
}

/// How a CTE is planned: as the planner likes, or as written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Materialized {
    Default,
    Yes,
    No,
}

/// One SELECT: what a compound's members and the top level share.
#[derive(Clone, Debug, PartialEq)]
pub struct Select {
    pub distinct: bool,
    /// The result columns; empty for `*`.
    pub items: Vec<Expr>,
    pub from: Src,
    pub joins: Vec<Join>,
    pub filter: Option<Expr>,
    pub group: Vec<Expr>,
    /// Grouping sets over the keys (`group`); None is a plain GROUP BY.
    pub sets: Option<GroupSets>,
    pub having: Option<Expr>,
}

/// Grouping sets of a GROUP BY's keys: `ROLLUP (k, ..)`, `CUBE (k, ..)`,
/// `k0, ROLLUP (k1, ..)` (two keys or more), or `GROUPING SETS ((..), ..)`
/// of key positions, every key in some set (an item may name any key).
/// Each output row is one group of one set, the keys outside the set NULL,
/// so the oracles' partitions of WHERE and HAVING still hold: the union of
/// the partitions' groups, as sets, is the query's (the empty set's row
/// appears whatever the rows).
#[derive(Clone, Debug, PartialEq)]
pub enum GroupSets {
    Rollup,
    Cube,
    Mixed,
    Sets(Vec<Vec<u8>>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cte {
    pub query: Select,
    pub materialized: Materialized,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub count: u32,
    pub offset: u32,
}

/// A query: CTEs, a SELECT, compound members (each joined to everything
/// before it, left to right: rendered in parentheses, so INTERSECT binds no
/// tighter), and an order by every result column (`ORDER BY 1 [DESC] [NULLS
/// FIRST | LAST], 2, ..`) with a limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub with: Vec<Cte>,
    pub select: Select,
    pub compound: Vec<(SetOp, Select)>,
    /// One term per result column when given: (position from 1, desc,
    /// nulls first).
    pub order: Vec<(usize, bool, Option<bool>)>,
    pub limit: Option<Limit>,
}

pub const MAX_SOURCES: usize = 4;
pub const MAX_LEVELS: usize = 3;
pub const MAX_CTES: usize = 2;
pub const MAX_MEMBERS: usize = 3;
/// The most rows a query may read: the product of its sources' rows,
/// times its enclosing queries' for a correlated subquery.
pub const ROW_BUDGET: u64 = 60_000;

/// What a query's sources are: a table's or CTE's columns, its row count
/// and a few values of each column (for literals that select some rows).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub columns: Vec<(u32, Type)>,
    pub rows: usize,
    pub samples: Vec<Vec<Value>>,
}

/// The tables of a database (by id) and the CTEs of the query being drawn
/// or checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub tables: Vec<(u32, SourceInfo)>,
    pub ctes: Vec<SourceInfo>,
}

impl Schema {
    /// The schema of a model: each table's columns, rows and up to six
    /// distinct non-NULL values per column.
    pub fn of(m: &Model) -> Schema {
        let tables = m
            .tables
            .iter()
            .map(|t| {
                let samples = (0..t.columns.len())
                    .map(|p| {
                        let mut vs: Vec<Value> = Vec::new();
                        for r in t.rows.iter().take(512) {
                            if r[p] != Value::Null && !vs.contains(&r[p]) && vs.len() < 6 && r[p].width() < 200 {
                                vs.push(r[p].clone());
                            }
                        }
                        vs
                    })
                    .collect();
                (t.id, SourceInfo { columns: t.columns.iter().map(|c| (c.id, c.ty)).collect(), rows: t.rows.len(), samples })
            })
            .collect();
        Schema { tables, ctes: Vec::new() }
    }

    /// A source's columns and rows (a function's or subquery's columns as
    /// drawn: [`check_select`] checks them).
    pub fn source(&self, s: &Src) -> Option<Cow<'_, SourceInfo>> {
        match s {
            Src::Table(id) => self.tables.iter().find(|(t, _)| t == id).map(|(_, i)| Cow::Borrowed(i)),
            Src::Cte(k) => self.ctes.get(*k).map(Cow::Borrowed),
            Src::Func(f) => {
                let samples = f.calls.iter().flat_map(|c| match c {
                    Call::Series(lo, hi, _) => vec![vec![Value::Int(*lo), Value::Int(*hi)]],
                    Call::SeriesTo(..) => vec![vec![Value::Int(1), Value::Int(2)]],
                    Call::UnnestList(_, xs) => vec![xs.iter().filter_map(|e| match e { Expr::Lit(v, _) if *v != Value::Null => Some(v.clone()), _ => None }).collect()],
                    c => c.types().iter().map(|_| Vec::new()).collect(),
                }).collect();
                Some(Cow::Owned(SourceInfo { columns: (0..).zip(f.types()).collect(), rows: f.rows()?.max(1) as usize, samples }))
            },
            Src::Sub(sel) => {
                let rows = if sel.group.is_empty() && sel.aggregate() { 1 } else { select_budget(sel, self).min(ROW_BUDGET) };
                Some(Cow::Owned(SourceInfo { columns: (0..).zip(sel.items.iter().map(type_of)).collect(), rows: rows as usize, samples: Vec::new() }))
            },
        }
    }

    /// The schema with a query's CTEs as sources (each valid over the
    /// tables and the CTEs before it), or None when one is not valid.
    pub fn with_ctes(&self, q: &Query) -> Option<Schema> {
        let mut s = Schema { tables: self.tables.clone(), ctes: Vec::new() };
        for c in &q.with {
            let types = check_select(&c.query, &s, &[], Role::Cte)?;
            let rows = select_budget(&c.query, &s);
            let ids = (0..types.len() as u32).collect::<Vec<_>>();
            s.ctes.push(SourceInfo { columns: ids.into_iter().zip(types).collect(), rows: rows.min(ROW_BUDGET) as usize, samples: Vec::new() });
        }
        Some(s)
    }
}

/// A source as FROM writes it, at nesting depth `depth`, as source `k`
/// (`lateral`: it may read the sources before it).
fn src_sql(s: &Src, depth: usize, k: usize, lateral: bool) -> String {
    let lat = if lateral { "LATERAL " } else { "" };
    let cols = |n: usize| (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
    match s {
        Src::Table(id) => format!("t{id} AS {}", alias(depth, k)),
        Src::Cte(c) => format!("x{c} AS {}", alias(depth, k)),
        Src::Func(f) if f.table() => format!("{lat}{} AS {}", f.sql(depth), alias(depth, k)),
        Src::Func(f) => format!("{lat}{} AS {}({})", f.sql(depth), alias(depth, k), cols(f.types().len())),
        Src::Sub(sel) => format!("{lat}({}) AS {}({})", sel.sql_at(depth + 1), alias(depth, k), cols(sel.items.len())),
    }
}

/// Whether a join of this kind lets its source read the sources before it.
fn lateral_join(kind: JoinKind) -> bool {
    matches!(kind, JoinKind::Inner | JoinKind::Left | JoinKind::Cross)
}

impl Select {
    pub fn sources(&self) -> Vec<&Src> {
        std::iter::once(&self.from).chain(self.joins.iter().map(|j| &j.src)).collect()
    }

    /// The SELECTs of its subqueries in FROM.
    pub fn from_selects(&self) -> Vec<&Select> {
        self.sources().into_iter().filter_map(|s| match s { Src::Sub(sel) => Some(&**sel), _ => None }).collect()
    }

    /// Whether it is an aggregate query: grouped, or with an aggregate
    /// among its items or in HAVING.
    pub fn aggregate(&self) -> bool {
        !self.group.is_empty() || self.having.is_some() || self.items.iter().any(Expr::has_agg)
    }

    /// `FROM ..` at nesting depth `depth`, without the keyword.
    pub fn from_sql(&self, depth: usize) -> String {
        let mut s = src_sql(&self.from, depth, 0, false);
        for (k, j) in self.joins.iter().enumerate() {
            let lateral = lateral_join(j.kind) && matches!(j.src, Src::Func(_) | Src::Sub(_));
            write!(s, " {} {}", j.kind.sql(), src_sql(&j.src, depth, k + 1, lateral)).unwrap();
            if let Some(on) = &j.on {
                write!(s, " ON {}", on.sql(depth)).unwrap();
            }
        }
        s
    }

    /// The items, `*` written out as every source's columns when
    /// `columns` (the sources' column ids) are given.
    pub fn items_sql(&self, depth: usize) -> String {
        if self.items.is_empty() {
            "*".into()
        } else {
            self.items.iter().map(|e| e.sql(depth)).collect::<Vec<_>>().join(", ")
        }
    }

    /// `WHERE ..` and the rest after FROM: the parts an oracle keeps.
    pub fn group_sql(&self, depth: usize) -> String {
        let mut s = String::new();
        if !self.group.is_empty() {
            let keys: Vec<String> = self.group.iter().map(|e| e.sql(depth)).collect();
            let by = match &self.sets {
                None => keys.join(", "),
                Some(GroupSets::Rollup) => format!("ROLLUP ({})", keys.join(", ")),
                Some(GroupSets::Cube) => format!("CUBE ({})", keys.join(", ")),
                Some(GroupSets::Mixed) => format!("{}, ROLLUP ({})", keys[0], keys[1..].join(", ")),
                Some(GroupSets::Sets(sets)) => {
                    let sets: Vec<String> = sets.iter().map(|set| format!("({})", set.iter().map(|&k| keys[k as usize].as_str()).collect::<Vec<_>>().join(", "))).collect();
                    format!("GROUPING SETS ({})", sets.join(", "))
                },
            };
            write!(s, " GROUP BY {by}").unwrap();
        }
        if let Some(h) = &self.having {
            write!(s, " HAVING {}", h.sql(depth)).unwrap();
        }
        s
    }

    pub fn sql_at(&self, depth: usize) -> String {
        let mut s = format!("SELECT {}{} FROM {}", if self.distinct { "DISTINCT " } else { "" }, self.items_sql(depth), self.from_sql(depth));
        if let Some(w) = &self.filter {
            write!(s, " WHERE {}", w.sql(depth)).unwrap();
        }
        s.push_str(&self.group_sql(depth));
        s
    }

    /// Every expression of it at its own level.
    /// (Function arguments in FROM included; a subquery's in FROM not.)
    pub fn exprs(&self) -> Vec<&Expr> {
        let args = self.sources().into_iter().flat_map(|s| match s { Src::Func(f) => f.args(), _ => Vec::new() });
        self.items.iter().chain(args).chain(self.joins.iter().filter_map(|j| j.on.as_ref())).chain(self.filter.iter()).chain(self.group.iter()).chain(self.having.iter()).collect()
    }

    pub fn cost(&self) -> u64 {
        let from: u64 = self.sources().iter().map(|s| match s { Src::Func(f) => f.cost(), Src::Sub(sel) => 3 + sel.cost(), _ => 0 }).sum();
        let own: u64 = self.items.iter().chain(self.joins.iter().filter_map(|j| j.on.as_ref())).chain(self.filter.iter()).chain(self.group.iter()).chain(self.having.iter()).map(|e| e.cost()).sum();
        1 + u64::from(self.distinct) + u64::from(self.sets.is_some()) + self.joins.len() as u64 * 2 + from + own
    }
}

impl Query {
    pub fn simple(select: Select) -> Query {
        Query { with: Vec::new(), select, compound: Vec::new(), order: Vec::new(), limit: None }
    }

    /// `WITH ..` (with a trailing space), the CTEs hinted as given, or as
    /// `hint` says for every CTE.
    pub fn with_sql_as(&self, hint: Option<Materialized>) -> String {
        if self.with.is_empty() {
            return String::new();
        }
        let ctes: Vec<String> = self
            .with
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let n = query_width(&c.query, self, k);
                let cols = (0..n).map(|i| format!("c{i}")).collect::<Vec<_>>().join(", ");
                let m = match hint.unwrap_or(c.materialized) {
                    Materialized::Default => "",
                    Materialized::Yes => "MATERIALIZED ",
                    Materialized::No => "NOT MATERIALIZED ",
                };
                format!("x{k}({cols}) AS {m}({})", c.query.sql_at(0))
            })
            .collect();
        format!("WITH {} ", ctes.join(", "))
    }

    pub fn with_sql(&self) -> String {
        self.with_sql_as(None)
    }

    /// The query at nesting depth `depth` (subqueries have no CTEs,
    /// compound members or order).
    pub fn sql_at(&self, depth: usize) -> String {
        let mut s = self.select.sql_at(depth);
        for (op, m) in &self.compound {
            s = format!("({s}) {} ({})", op.sql(), m.sql_at(depth));
        }
        s.push_str(&self.order_sql());
        s
    }

    /// ORDER BY and LIMIT, with a leading space.
    pub fn order_sql(&self) -> String {
        let mut s = String::new();
        if !self.order.is_empty() {
            let terms: Vec<String> = self
                .order
                .iter()
                .map(|&(k, desc, nulls)| {
                    let n = match nulls {
                        Some(true) => " NULLS FIRST",
                        Some(false) => " NULLS LAST",
                        None => "",
                    };
                    format!("{k}{}{n}", if desc { " DESC" } else { "" })
                })
                .collect();
            write!(s, " ORDER BY {}", terms.join(", ")).unwrap();
        }
        if let Some(l) = self.limit {
            write!(s, " LIMIT {}", l.count).unwrap();
            if l.offset > 0 {
                write!(s, " OFFSET {}", l.offset).unwrap();
            }
        }
        s
    }

    /// The whole query as SQL.
    pub fn sql(&self) -> String {
        format!("{}{}", self.with_sql(), self.sql_at(0))
    }

    /// Its SELECTs: the query's own and its compound members'.
    pub fn selects(&self) -> Vec<&Select> {
        std::iter::once(&self.select).chain(self.compound.iter().map(|(_, m)| m)).collect()
    }

    pub fn cost(&self) -> u64 {
        self.with.iter().map(|c| 2 + c.query.cost() + u64::from(c.materialized != Materialized::Default)).sum::<u64>()
            + self.select.cost()
            + self.compound.iter().map(|(_, m)| 2 + m.cost()).sum::<u64>()
            + self.order.iter().map(|&(_, d, n)| 1 + u64::from(d) + u64::from(n.is_some())).sum::<u64>()
            + self.limit.map_or(0, |l| 2 + u64::from(l.count.min(16)) / 4 + u64::from(l.offset > 0))
    }

    /// Whether the query is valid against `schema` (a database's, with no
    /// CTEs): every expression types, every rule of the module holds.
    pub fn valid(&self, schema: &Schema) -> bool {
        self.types(schema).is_some()
    }

    /// The result types, when valid.
    pub fn types(&self, schema: &Schema) -> Option<Vec<Type>> {
        if self.with.len() > MAX_CTES || self.compound.len() > MAX_MEMBERS {
            return None;
        }
        let s = schema.with_ctes(self)?;
        let types = check_select(&self.select, &s, &[], Role::Top)?;
        for (_, m) in &self.compound {
            if check_select(m, &s, &[], Role::Member)? != types {
                return None;
            }
        }
        // ORDER BY every result column once, in any order.
        let mut seen: Vec<usize> = self.order.iter().map(|o| o.0).collect();
        seen.sort();
        let order_ok = self.order.is_empty() || seen == (1..=types.len()).collect::<Vec<_>>();
        let limit_ok = self.limit.is_none() || !self.order.is_empty();
        let budget_ok = self.selects().iter().all(|m| select_budget(m, &s) <= ROW_BUDGET);
        (order_ok && limit_ok && budget_ok).then_some(types)
    }

    /// The query against a smaller schema: dropped when a source is gone,
    /// a column that is gone (or has another type) a NULL of its type;
    /// then kept only when valid. Never costlier.
    pub fn repair(&self, schema: &Schema) -> Option<Query> {
        if self.valid(schema) {
            return Some(self.clone());
        }
        let mut q = self.clone();
        let mut s = Schema { tables: schema.tables.clone(), ctes: Vec::new() };
        for c in &mut q.with {
            repair_select(&mut c.query, &s, &mut Vec::new());
            let types = check_select(&c.query, &s, &[], Role::Cte)?;
            s.ctes.push(SourceInfo { columns: (0..types.len() as u32).zip(types).collect(), rows: 0, samples: Vec::new() });
        }
        repair_select(&mut q.select, &s, &mut Vec::new());
        for (_, m) in &mut q.compound {
            repair_select(m, &s, &mut Vec::new());
        }
        (q.valid(schema) && q.cost() <= self.cost()).then_some(q)
    }
}

/// The number of result columns of a CTE's query (always explicit
/// items).
fn query_width(sel: &Select, _q: &Query, _k: usize) -> usize {
    sel.items.len()
}

/// Where a SELECT is: what it may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The top level: anything.
    Top,
    /// A compound member: anything but ORDER BY (which only the whole has).
    Member,
    /// A CTE's query: not DISTINCT, not grouped, no aggregate.
    Cte,
    /// A subquery of EXISTS or IN: no compound, no CTE.
    Sub,
    /// A scalar subquery: one aggregate item, not grouped, not DISTINCT.
    Scalar,
}

/// The scope an expression is checked or drawn in: its query's sources,
/// then those of each enclosing query.
type Scope<'a> = [Vec<&'a SourceInfo>];

/// The rows a SELECT reads: the product of its sources' rows.
fn select_budget(sel: &Select, s: &Schema) -> u64 {
    sel.sources().iter().map(|src| s.source(src).map_or(1, |i| i.rows.max(1) as u64)).fold(1u64, |a, b| a.saturating_mul(b))
}

/// Whether a grouping key keeps a column of its query after PostgreSQL's
/// constant folding (`eval_const_expressions`), the condition for a key
/// under ROLLUP / CUBE / GROUPING SETS.
///
/// Known PostgreSQL bug (`../bugs/will-not-post/pg-grouping-sets-constant-key`;
/// Haibo Yan, pgsql-hackers 2026-10-10, and BUG #19752): a key that is
/// variable-free after folding is nulled, in the row of a set that leaves it
/// out, at only one of its occurrences (not at a second item, inside an
/// expression or in HAVING), but at all of them when the input is empty.
/// Reading a column is not enough: the workspace runs of 2026-10-19..21
/// found keys whose columns fold away, `CASE '3'::int8 WHEN '770'::int8
/// THEN c ELSE .. END`, `CASE WHEN <constant true> THEN 'x' ELSE c END`,
/// `coalesce('-5.0'::float8, c)`, `c > x OR <constant true>`, `c IS NOT
/// NULL` of a NOT NULL column (folded since 19), `c < NULL::t`.
///
/// So this says yes only when a column surely survives: a column of this
/// query; a strict operator or function with an argument that survives
/// and the others surviving or non-NULL literals (a NULL argument folds the
/// call to NULL); AND / OR of survivors or of a survivor and a literal that
/// does not decide it; a CASE whose first arm not ruled out by a literal
/// condition has a surviving condition, or, for a condition that may fold
/// to anything, a surviving result and a surviving rest; COALESCE whose
/// first argument that is not a NULL literal survives; NULLIF whose first
/// argument survives; GREATEST / LEAST with a survivor (folded only when all
/// are constants); IS [NOT] NULL / UNKNOWN only of an expression that
/// PostgreSQL cannot prove non-null ([`maybe_nonnullable`]). Literals,
/// outer columns and subqueries never survive. Anything else, an argument
/// that is variable-free but not a literal included, counts as folding.
pub fn keeps_column(e: &Expr) -> bool {
    let k = keeps_column;
    let lit_or_keeps = |e: &Expr| k(e) || nonnull_literal(e);
    let strict2 = |a: &Expr, b: &Expr| (k(a) && lit_or_keeps(b)) || (k(b) && lit_or_keeps(a));
    let null_lit = |e: &Expr| matches!(e, Expr::Lit(Value::Null, _));
    // A literal that leaves AND (`neutral` true) / OR (false) undecided.
    let neutral = |e: &Expr, neutral: bool| matches!(e, Expr::Lit(Value::Null, _)) || matches!(e, Expr::Lit(Value::Bool(b), _) if *b == neutral);
    let null_test = |a: &Expr| k(a) && !maybe_nonnullable(a);
    match e {
        Expr::Col(c) => c.up == 0,
        Expr::Lit(..) => false,
        Expr::Cmp(_, a, b) | Expr::Arith(_, a, b) | Expr::Concat(a, b) => strict2(a, b),
        // `x IS DISTINCT FROM NULL` is parsed as `x IS NOT NULL`.
        Expr::Distinct(_, a, b) if null_lit(a) => null_test(b),
        Expr::Distinct(_, a, b) if null_lit(b) => null_test(a),
        Expr::Distinct(_, a, b) => strict2(a, b),
        Expr::And(a, b) => (k(a) && (k(b) || neutral(b, true))) || (k(b) && neutral(a, true)),
        Expr::Or(a, b) => (k(a) && (k(b) || neutral(b, false))) || (k(b) && neutral(a, false)),
        Expr::Not(a) | Expr::Neg(a) | Expr::Cast(a, _) => k(a),
        Expr::Like { e, .. } | Expr::Regex(e, _) => k(e),
        Expr::IsNull(a, _) | Expr::Is(a, IsWhat::Unknown, _) => null_test(a),
        // On a non-null argument IS TRUE folds to it, IS FALSE to its NOT.
        Expr::Is(a, _, _) => k(a),
        // `e >= lo AND e <= hi` (and the like for NOT / SYMMETRIC).
        Expr::Between { e, lo, hi, .. } => strict2(e, lo) && strict2(e, hi),
        Expr::InList(e, xs, _) => k(e) && !xs.is_empty() && xs.iter().all(lit_or_keeps),
        Expr::Case { operand, whens, otherwise } => {
            for (w, then) in whens {
                // The arm's test: `w`, or `operand = w`.
                let (test_keeps, ruled_out, decided) = match operand {
                    None => (k(w), matches!(w, Expr::Lit(Value::Null | Value::Bool(false), _)), matches!(w, Expr::Lit(Value::Bool(true), _))),
                    Some(op) => (strict2(op, w), null_lit(op) || null_lit(w), false),
                };
                if test_keeps {
                    return true;
                } else if ruled_out {
                    continue;
                } else if decided || !k(then) {
                    return decided && k(then);
                }
            }
            otherwise.as_deref().is_some_and(k)
        },
        Expr::Coalesce(xs) => xs.iter().find(|x| !null_lit(x)).is_some_and(k),
        Expr::NullIf(a, b) => k(a) || (k(b) && nonnull_literal(a)),
        Expr::Greatest(xs) | Expr::Least(xs) => xs.iter().any(k),
        Expr::Func(_, xs) => xs.iter().any(k) && xs.iter().all(lit_or_keeps),
        Expr::Agg(..) | Expr::Window(_) | Expr::Exists(..) | Expr::InQuery(..) | Expr::Scalar(_) => false,
    }
}

/// A literal (or a cast of one) that is not NULL.
fn nonnull_literal(e: &Expr) -> bool {
    match e {
        Expr::Lit(v, _) => *v != Value::Null,
        Expr::Cast(a, _) => nonnull_literal(a),
        _ => false,
    }
}

/// Whether PostgreSQL might prove `e` non-null once folded
/// (`expr_is_nonnullable`: a NOT NULL column, a non-NULL constant,
/// COALESCE / GREATEST / LEAST with such an argument, a CASE with such
/// results, IS [NOT] NULL and IS TRUE and the like), and so fold `e IS
/// [NOT] NULL` to a constant. Conservative: any column may be NOT NULL
/// (the schema can change after the query is drawn); AND / OR / NOT may
/// fold to an argument; `b = true` folds to `b`.
fn maybe_nonnullable(e: &Expr) -> bool {
    let m = maybe_nonnullable;
    match e {
        Expr::Col(_) => true,
        Expr::Lit(v, _) => *v != Value::Null,
        Expr::Coalesce(xs) | Expr::Greatest(xs) | Expr::Least(xs) => xs.iter().any(m),
        Expr::Case { whens, otherwise, .. } => whens.iter().any(|(_, r)| m(r)) || otherwise.as_deref().is_some_and(m),
        Expr::IsNull(..) | Expr::Is(..) | Expr::Distinct(..) => true,
        Expr::Func(Func::Array(_), _) => true,
        Expr::Cast(a, _) | Expr::Not(a) | Expr::NullIf(a, _) | Expr::InList(a, _, _) => m(a),
        Expr::And(a, b) | Expr::Or(a, b) => m(a) || m(b),
        Expr::Cmp(_, a, b) => (type_of(a) == Type::Bool || type_of(b) == Type::Bool) && (m(a) || m(b)),
        _ => false,
    }
}

/// The result types of a SELECT in `role`, scope `outer` around it, or
/// None when it is not valid.
pub fn check_select(sel: &Select, s: &Schema, outer: &Scope, role: Role) -> Option<Vec<Type>> {
    let srcs: Vec<Cow<SourceInfo>> = sel.sources().iter().map(|src| s.source(src)).collect::<Option<_>>()?;
    if srcs.len() > MAX_SOURCES || outer.len() + 1 > MAX_LEVELS {
        return None;
    }
    // A CTE source only at the top level (subqueries see no CTE).
    if role == Role::Sub || role == Role::Scalar {
        if sel.sources().iter().any(|x| matches!(x, Src::Cte(_))) {
            return None;
        }
    }
    let mut scope: Vec<Vec<&SourceInfo>> = vec![Vec::new()];
    scope.extend(outer.iter().cloned());
    if !from_item_ok(&sel.from, s, &scope, false) {
        return None;
    }
    scope[0].push(&srcs[0]);
    for (k, j) in sel.joins.iter().enumerate() {
        if !from_item_ok(&j.src, s, &scope, lateral_join(j.kind)) {
            return None;
        }
        scope[0].push(&srcs[k + 1]);
        match (j.kind, &j.on) {
            (JoinKind::Cross, None) => {},
            (JoinKind::Cross, Some(_)) | (_, None) => return None,
            (JoinKind::Full, Some(on)) => {
                if !full_on_ok(on, k + 1) || check(on, s, &scope, Ctx::Plain)? != Type::Bool {
                    return None;
                }
            },
            (_, Some(on)) => {
                if check(on, s, &scope, Ctx::Plain)? != Type::Bool {
                    return None;
                }
            },
        }
    }
    if let Some(w) = &sel.filter {
        if check(w, s, &scope, Ctx::Plain)? != Type::Bool {
            return None;
        }
    }
    let aggregate = sel.aggregate();
    let sets_ok = match &sel.sets {
        None => true,
        Some(GroupSets::Rollup | GroupSets::Cube) => !sel.group.is_empty(),
        Some(GroupSets::Mixed) => sel.group.len() >= 2,
        Some(GroupSets::Sets(sets)) => {
            !sets.is_empty() && sets.iter().flatten().all(|&k| (k as usize) < sel.group.len()) && (0..sel.group.len()).all(|k| sets.iter().any(|set| set.contains(&(k as u8))))
        },
    };
    // Known PostgreSQL bug (`../bugs/will-not-post/pg-grouping-sets-constant-key`,
    // already reported upstream): a key that is variable-free after constant
    // folding is nulled in a set that leaves it out only at one item, not at
    // a second one, inside expressions or in HAVING (but everywhere when
    // there are no rows). So under grouping sets every key keeps a column of
    // the query through folding ([`keeps_column`]).
    let column_keys = sel.sets.is_none() || sel.group.iter().all(keeps_column);
    if !sets_ok || !column_keys {
        return None;
    }
    for k in &sel.group {
        if k.has_subquery() {
            return None;
        }
        check(k, s, &scope, Ctx::Plain)?;
    }
    let ctx = match (aggregate, role) {
        (true, _) => Ctx::Agg,
        (false, Role::Top | Role::Member) => Ctx::Items,
        _ => Ctx::Plain,
    };
    let types: Vec<Type> = if sel.items.is_empty() {
        if aggregate {
            return None;
        }
        scope[0].iter().flat_map(|i| i.columns.iter().map(|c| c.1)).collect()
    } else {
        sel.items.iter().map(|e| check(e, s, &scope, ctx)).collect::<Option<_>>()?
    };
    // Expression-only types are never results (nor group keys): the
    // oracles read results as column types, and a picked value of one
    // could show which of equal values was picked.
    if types.iter().any(|t| t.internal()) || sel.group.iter().any(|k| check(k, s, &scope, Ctx::Plain).is_some_and(|t| t.internal())) {
        return None;
    }
    if let Some(h) = &sel.having {
        if check(h, s, &scope, Ctx::Agg)? != Type::Bool {
            return None;
        }
    }
    if aggregate {
        // Items and HAVING: keys as written, or expressions whose columns
        // of this level are inside aggregates; no subquery.
        let ok = |e: &Expr| !e.has_subquery() && grouped_ok(e, &sel.group);
        if !sel.items.iter().all(ok) || !sel.having.iter().all(ok) {
            return None;
        }
    }
    let role_ok = match role {
        Role::Top | Role::Member => true,
        Role::Cte => !sel.distinct && !aggregate && !sel.items.is_empty(),
        Role::Sub => true,
        Role::Scalar => !sel.distinct && sel.group.is_empty() && sel.having.is_none() && sel.items.len() == 1 && sel.items[0].has_agg(),
    } && positional_windows(sel) <= 1;
    role_ok.then_some(types)
}

/// Whether a source in FROM is valid where it is: a function's arguments
/// type in `scope` (its own level the sources before it when `lateral`,
/// else none), each call within its bounds; a subquery is valid in the
/// role of a CTE's query or of a scalar subquery, reads no CTE, and has the
/// column types it was drawn with ([`Schema::source`]).
fn from_item_ok(src: &Src, s: &Schema, scope: &Scope, lateral: bool) -> bool {
    let mut sc: Vec<Vec<&SourceInfo>> = scope.to_vec();
    if !lateral {
        sc[0].clear();
    }
    match src {
        Src::Table(_) | Src::Cte(_) => true,
        Src::Func(f) => (1..=2).contains(&f.calls.len()) && (!f.table() || f.calls.len() == 1 && !f.ordinality) && f.rows().is_some() && f.calls.iter().all(|c| call_ok(c, s, &sc)),
        Src::Sub(sel) => {
            if sel.sources().iter().any(|x| matches!(x, Src::Cte(_))) {
                return false;
            }
            // A known PostgreSQL bug, excluded
            // (`../bugs/pg-minmax-subquery-relid`): a min/max
            // subquery in FROM (index-optimized) with an outer `= ANY
            // (array_append(.., s.c))` pushed into it fails in the planner
            // ("no relation entry for relid 1", XX000). So an aggregate
            // subquery in FROM takes no `min` or `max`.
            let mut minmax = false;
            for e in &sel.items {
                e.walk(&mut |x| minmax |= matches!(x, Expr::Agg(AggFunc::Min | AggFunc::Max, ..)));
            }
            if minmax && sel.aggregate() {
                return false;
            }
            let want: Vec<Type> = sel.items.iter().map(type_of).collect();
            [Role::Cte, Role::Scalar].iter().any(|&r| check_select(sel, s, &sc, r).is_some_and(|ts| ts == want))
        },
    }
}

fn call_ok(c: &Call, s: &Schema, sc: &Scope) -> bool {
    let ty = |e: &Expr| check(e, s, sc, Ctx::Plain);
    match c {
        Call::Series(_, _, step) | Call::SeriesNum(_, _, step) => *step != 0 && c.rows().is_some_and(|n| n <= SERIES_MAX),
        Call::SeriesTo(e, k) => (1..=8).contains(k) && ty(e) == Some(Type::Int8),
        // Finite timestamps far from the ends (no overflow).
        Call::SeriesTs(lo, n, step) => *n <= 10 && (*step as usize) < SERIES_STEPS.len() && (-3_000_000_000_000_000..=3_000_000_000_000_000).contains(lo),
        Call::StringTable(t, sep) => t.chars().count() <= 12 && sep.chars().count() <= 2,
        Call::Unnest(e) => matches!(ty(e), Some(Type::Int8Array | Type::TextArray)) && ty(e) == Some(type_of(e)) && array_bound(e).is_some_and(|n| n <= 16),
        Call::UnnestList(t, xs) => !t.internal() && (1..=4).contains(&xs.len()) && xs.iter().all(|e| ty(e) == Some(*t)),
        Call::JsonbEach(e) => matches!(e, Expr::Func(Func::JsonbBuildObject, a) if !a.is_empty()) && ty(e) == Some(Type::Jsonb),
        Call::JsonbElems(e) => matches!(e, Expr::Func(Func::JsonbBuildArray, a) if !a.is_empty()) && ty(e) == Some(Type::Jsonb),
        Call::JsonTable(e, cols) => {
            matches!(e, Expr::Func(Func::JsonbBuildArray, a) if !a.is_empty())
                && ty(e) == Some(Type::Jsonb)
                && (1..=4).contains(&cols.len())
                && cols.iter().all(|c| c.ok())
                // PostgreSQL takes one FOR ORDINALITY column at most.
                && cols.iter().filter(|c| **c == JtCol::Ordinality).count() <= 1
                && jt_collation_free(e)
        },
    }
}

/// A known PostgreSQL bug, excluded
/// (`../bugs/will-not-post/pg-json-table-context-collation`): JSON_TABLE's
/// context item gets no collations assigned, so a collation-sensitive text
/// operation in it (a comparison, LIKE, `lower`, even with COLLATE) fails
/// with 42P22. So no expression of the argument consumes a text value
/// except a jsonb builder (`to_jsonb`, `jsonb_build_object`,
/// `jsonb_build_array`), subqueries left out (they assign their own).
pub fn jt_collation_free(e: &Expr) -> bool {
    let mut ok = true;
    e.walk(&mut |x| {
        if !matches!(x, Expr::Func(Func::ToJsonb | Func::JsonbBuildObject | Func::JsonbBuildArray, _)) {
            ok &= x.children().iter().all(|c| type_of(c) != Type::Text);
        }
    });
    ok
}

/// The window functions of a SELECT whose values depend on the order of
/// peers (`row_number`, `lag`, a ROWS frame, ..). Each orders by every
/// column, so peers are identical rows and one such window is determined;
/// but two sort the identical rows apart independently, so which row gets
/// which pair of values is the plan's. At most one per SELECT.
fn positional_windows(sel: &Select) -> usize {
    let mut n = 0;
    for item in &sel.items {
        item.walk(&mut |e| {
            if let Expr::Window(w) = e {
                n += usize::from(w.func.needs_total() || w.frame.is_some_and(|f| f.needs_total()));
            }
        });
    }
    n
}

/// Whether a FULL JOIN's ON clause is a conjunction of equalities of a
/// column of source `new` with a column of an earlier source.
fn full_on_ok(on: &Expr, new: usize) -> bool {
    match on {
        Expr::And(a, b) => full_on_ok(a, new) && full_on_ok(b, new),
        Expr::Cmp(CmpOp::Eq, a, b) => match (&**a, &**b) {
            (Expr::Col(x), Expr::Col(y)) => x.up == 0 && y.up == 0 && x.ty == y.ty && (x.src == new && y.src < new || y.src == new && x.src < new),
            _ => false,
        },
        _ => false,
    }
}

/// Whether an item or HAVING term of an aggregate query is a key as
/// written, or reads its own query's columns only inside aggregates (so
/// that nothing reads a key's value, picked among equal ones, but the key
/// itself).
pub fn grouped_ok(e: &Expr, keys: &[Expr]) -> bool {
    fn only_in_aggs(e: &Expr) -> bool {
        match e {
            Expr::Col(c) => c.up > 0,
            Expr::Agg(..) => true,
            _ => e.children().iter().all(|c| only_in_aggs(c)),
        }
    }
    keys.contains(e) || only_in_aggs(e)
}

/// Where an expression is: aggregates allowed or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctx {
    Plain,
    Agg,
    /// The items of a non-aggregate SELECT where windows may be.
    Items,
}

/// Whether every column of `e` is of its own level (`up` 0), subqueries
/// left out.
fn local(e: &Expr) -> bool {
    let mut ok = true;
    e.walk(&mut |x| {
        if let Expr::Col(c) = x {
            ok &= c.up == 0;
        }
        ok &= !matches!(x, Expr::Exists(..) | Expr::InQuery(..) | Expr::Scalar(_));
    });
    ok
}

/// The type of `e` in `scope`, or None when it does not type or breaks a
/// rule.
fn check(e: &Expr, s: &Schema, scope: &Scope, ctx: Ctx) -> Option<Type> {
    use Type::*;
    let c = |x: &Expr| check(x, s, scope, ctx);
    let boolean = |x: &Expr| (c(x)? == Bool).then_some(Bool);
    Some(match e {
        Expr::Col(col) => {
            let info = scope.get(col.up)?.get(col.src)?;
            let ty = info.columns.iter().find(|(id, _)| *id == col.column)?.1;
            (ty == col.ty).then_some(ty)?
        },
        Expr::Lit(v, ty) => ty.admits(v).then_some(*ty)?,
        Expr::Cmp(_, a, b) | Expr::Distinct(_, a, b) => {
            let (x, y) = (c(a)?, c(b)?);
            comparable(x, y).then_some(Bool)?
        },
        Expr::And(a, b) | Expr::Or(a, b) => {
            boolean(a)?;
            boolean(b)?
        },
        Expr::Not(a) | Expr::Is(a, ..) => boolean(a)?,
        Expr::IsNull(a, _) => {
            c(a)?;
            Bool
        },
        Expr::Between { e, lo, hi, .. } => {
            let t = c(e)?;
            (comparable(t, c(lo)?) && comparable(t, c(hi)?)).then_some(Bool)?
        },
        Expr::InList(a, xs, _) => {
            let t = c(a)?;
            let ts: Vec<Type> = xs.iter().map(c).collect::<Option<_>>()?;
            // PostgreSQL resolves the list to one type: keep them alike.
            (!xs.is_empty() && ts.iter().all(|&u| u == t)).then_some(Bool)?
        },
        Expr::Like { e, pattern, .. } => (c(e)? == Text && pattern_ok(pattern) && !pattern.contains('\0')).then_some(Bool)?,
        Expr::Regex(a, k) => (c(a)? == Text && *k < REGEXES.len()).then_some(Bool)?,
        Expr::Arith(op, a, b) => {
            let t = arith_type(*op, c(a)?, c(b)?)?;
            // A divisor is a literal, neither 0 nor -1 (`i / -1` overflows on
            // the smallest integer, which is an expected error, but a
            // literal 0 is an error on every row).
            if matches!(op, Arith::Div | Arith::Mod) && !divisor_ok(b) {
                return None;
            }
            t
        },
        Expr::Neg(a) => {
            let t = c(a)?;
            (t.is_int() || t.is_float() || t == Numeric || t == Interval).then_some(t)?
        },
        Expr::Concat(a, b) => {
            let (x, y) = (c(a)?, c(b)?);
            (x == y && matches!(x, Text | Bytea)).then_some(x)?
        },
        Expr::Case { operand, whens, otherwise } => {
            if whens.is_empty() {
                return None;
            }
            let op = match operand {
                Some(o) => Some(c(o)?),
                None => None,
            };
            let mut out: Option<Type> = None;
            for (w, t) in whens {
                let wt = c(w)?;
                match op {
                    Some(o) if !comparable(o, wt) => return None,
                    None if wt != Bool => return None,
                    _ => {},
                }
                let tt = c(t)?;
                if out.is_some_and(|o| o != tt) {
                    return None;
                }
                out = Some(tt);
            }
            if let Some(x) = otherwise {
                if c(x)? != out? {
                    return None;
                }
            }
            out?
        },
        Expr::Coalesce(xs) | Expr::Greatest(xs) | Expr::Least(xs) => {
            let ts: Vec<Type> = xs.iter().map(c).collect::<Option<_>>()?;
            (!ts.is_empty() && ts.len() <= 4 && ts.iter().all(|&t| t == ts[0])).then_some(ts[0])?
        },
        Expr::NullIf(a, b) => {
            let t = c(a)?;
            (c(b)? == t).then_some(t)?
        },
        Expr::Cast(a, to) => cast_ok(c(a)?, *to).then_some(*to)?,
        Expr::Func(f, xs) => {
            let ts: Vec<Type> = xs.iter().map(c).collect::<Option<_>>()?;
            f.ret(xs, &ts)?
        },
        Expr::Agg(f, distinct, arg) => {
            if ctx != Ctx::Agg {
                return None;
            }
            match arg {
                None => (*f == AggFunc::Count && !distinct).then_some(Int8)?,
                Some(a) => {
                    // No aggregate inside, and only the query's own columns
                    // (an outer column would make it the outer query's).
                    if a.has_agg() || !local(a) {
                        return None;
                    }
                    let t = check(a, s, scope, Ctx::Plain)?;
                    if *f == AggFunc::StringAgg && *distinct {
                        return None;
                    }
                    f.ret(Some(t))?
                },
            }
        },
        Expr::Exists(q, _) => {
            check_sub(q, s, scope, Role::Sub)?;
            Bool
        },
        Expr::InQuery(a, q, _) => {
            let t = c(a)?;
            let ts = check_sub(q, s, scope, Role::Sub)?;
            (ts.len() == 1 && comparable(t, ts[0])).then_some(Bool)?
        },
        Expr::Scalar(q) => {
            let ts = check_sub(q, s, scope, Role::Scalar)?;
            (ts.len() == 1).then_some(ts[0])?
        },
        Expr::Window(w) => {
            if ctx != Ctx::Items || w.order.len() > 24 || w.partition.len() > 3 {
                return None;
            }
            // Arguments and the window: the query's own columns, no
            // aggregate, window or subquery inside.
            let parts = w.args.iter().chain(w.partition.iter()).chain(w.order.iter().map(|o| &o.0));
            for e in parts.clone() {
                if !local(e) || e.has_agg() || has_window(e) {
                    return None;
                }
                check(e, s, scope, Ctx::Plain)?;
            }
            let args: Vec<Type> = w.args.iter().map(|e| check(e, s, scope, Ctx::Plain)).collect::<Option<_>>()?;
            let ty = w.func.ret(&args)?;
            if let Some(f) = &w.frame {
                if !f.ok() || f.mode == FrameMode::Groups && w.order.is_empty() {
                    return None;
                }
            }
            // A total order: every column of every source among the terms.
            let needs_total = w.func.needs_total() || w.frame.is_some_and(|f| f.needs_total());
            if needs_total {
                let total = scope[0].iter().enumerate().all(|(src, info)| {
                    info.columns.iter().all(|&(column, ty)| w.order.iter().any(|(e, ..)| *e == Expr::Col(Col { up: 0, src, column, ty })))
                });
                if !total {
                    return None;
                }
            }
            ty
        },
    })
}

/// Whether a window function is in `e` (not inside subqueries).
pub fn has_window(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |x| found |= matches!(x, Expr::Window(_)));
    found
}

fn divisor_ok(b: &Expr) -> bool {
    match b {
        Expr::Lit(Value::Int(k), _) => *k != 0 && *k != -1,
        Expr::Lit(Value::Float(x), _) => *x != 0.0 && x.is_finite(),
        Expr::Lit(Value::Numeric(Numeric::Finite { digits, .. }), _) => !digits.is_empty(),
        _ => false,
    }
}

/// The result types of a subquery (no CTEs, compound members or order),
/// whose scope is `scope` around it.
fn check_sub(q: &Query, s: &Schema, scope: &Scope, role: Role) -> Option<Vec<Type>> {
    if !q.with.is_empty() || !q.compound.is_empty() || !q.order.is_empty() || q.limit.is_some() {
        return None;
    }
    let types = check_select(&q.select, s, scope, role)?;
    // A correlated subquery runs once per outer row.
    let outer: u64 = scope.iter().flatten().map(|i| i.rows.max(1) as u64).fold(1u64, |a, b| a.saturating_mul(b));
    (select_budget(&q.select, s).saturating_mul(outer) <= ROW_BUDGET * 4).then_some(types)
}

/// Replace what no longer resolves in `sel` by NULLs of its type; drop
/// joins whose source is gone (with their columns), and subqueries that no
/// longer type by NULLs.
fn repair_select(sel: &mut Select, s: &Schema, outer: &mut Vec<Vec<SourceInfo>>) {
    let infos: Vec<Option<SourceInfo>> = sel.sources().iter().map(|src| s.source(src).map(Cow::into_owned)).collect();
    let mut level: Vec<SourceInfo> = infos.iter().map(|i| i.clone().unwrap_or(SourceInfo { columns: Vec::new(), rows: 0, samples: Vec::new() })).collect();
    // A missing source keeps its place (as an empty one) so that the others'
    // positions stay; the query is then invalid and dropped by the caller.
    outer.insert(0, std::mem::take(&mut level));
    for src in std::iter::once(&mut sel.from).chain(sel.joins.iter_mut().map(|j| &mut j.src)) {
        match src {
            Src::Func(f) => {
                for c in &mut f.calls {
                    for e in c.args_mut() {
                        repair_expr(e, outer, s);
                    }
                }
            },
            Src::Sub(sub) => repair_select(sub, s, outer),
            _ => {},
        }
    }
    let fix = |e: &mut Expr, outer: &Vec<Vec<SourceInfo>>| repair_expr(e, outer, s);
    for e in sel.items.iter_mut() {
        fix(e, outer);
    }
    for j in sel.joins.iter_mut() {
        if let Some(on) = &mut j.on {
            fix(on, outer);
        }
    }
    if let Some(w) = &mut sel.filter {
        fix(w, outer);
    }
    for k in sel.group.iter_mut() {
        fix(k, outer);
    }
    if let Some(h) = &mut sel.having {
        fix(h, outer);
    }
    outer.remove(0);
}

fn repair_expr(e: &mut Expr, scope: &Vec<Vec<SourceInfo>>, s: &Schema) {
    match e {
        Expr::Col(c) => {
            let ok = scope.get(c.up).and_then(|l| l.get(c.src)).is_some_and(|i| i.columns.iter().any(|&(id, ty)| id == c.column && ty == c.ty));
            if !ok {
                *e = Expr::null(c.ty);
            }
        },
        Expr::Exists(q, _) | Expr::InQuery(_, q, _) | Expr::Scalar(q) => {
            let mut inner = scope.clone();
            repair_select(&mut q.select, s, &mut inner);
            if let Expr::InQuery(a, ..) = e {
                repair_expr(a, scope, s);
            }
        },
        _ => {
            for c in children_mut(e) {
                repair_expr(c, scope, s);
            }
        },
    }
}

/// Every direct child expression, mutably (not inside subqueries).
pub fn children_mut(e: &mut Expr) -> Vec<&mut Expr> {
    match e {
        Expr::Col(_) | Expr::Lit(..) | Expr::Exists(..) | Expr::Scalar(_) => vec![],
        Expr::Cmp(_, a, b) | Expr::Distinct(_, a, b) | Expr::And(a, b) | Expr::Or(a, b) | Expr::Arith(_, a, b) | Expr::Concat(a, b) | Expr::NullIf(a, b) => vec![a, b],
        Expr::Not(a) | Expr::IsNull(a, _) | Expr::Is(a, ..) | Expr::Neg(a) | Expr::Cast(a, _) | Expr::Regex(a, _) | Expr::InQuery(a, ..) => vec![a],
        Expr::Like { e, .. } => vec![e],
        Expr::Between { e, lo, hi, .. } => vec![e, lo, hi],
        Expr::InList(a, list, _) => std::iter::once(&mut **a).chain(list.iter_mut()).collect(),
        Expr::Case { operand, whens, otherwise } => operand
            .iter_mut()
            .map(|b| &mut **b)
            .chain(whens.iter_mut().flat_map(|(w, t)| [w, t]))
            .chain(otherwise.iter_mut().map(|b| &mut **b))
            .collect(),
        Expr::Coalesce(xs) | Expr::Greatest(xs) | Expr::Least(xs) | Expr::Func(_, xs) => xs.iter_mut().collect(),
        Expr::Agg(_, _, a) => a.iter_mut().map(|b| &mut **b).collect(),
        Expr::Window(w) => {
            let WinCall { args, partition, order, .. } = &mut **w;
            args.iter_mut().chain(partition.iter_mut()).chain(order.iter_mut().map(|o| &mut o.0)).collect()
        },
    }
}

// --------------------------------------------------------------- drawing

/// What drawing an expression knows: the schema, the scope (each level's
/// sources), whether aggregates may appear, how deep it may go, and the
/// rows the enclosing queries read (for correlated subqueries).
pub struct Cx<'a> {
    pub schema: &'a Schema,
    pub scope: Vec<Vec<SourceInfo>>,
    pub agg: bool,
    pub depth: usize,
    pub outer_rows: u64,
    /// Compound expressions still to draw: past it, only leaves (so that a
    /// query stays small whatever its fan-out).
    pub nodes: usize,
}

impl Cx<'_> {

    /// Columns of type `ty` in scope (own level first, then outer ones).
    fn columns(&self, ty: Option<Type>) -> Vec<Col> {
        let mut out = Vec::new();
        for (up, level) in self.scope.iter().enumerate() {
            for (src, info) in level.iter().enumerate() {
                for &(column, t) in &info.columns {
                    if ty.is_none_or(|x| x == t) {
                        out.push(Col { up, src, column, ty: t });
                    }
                }
            }
        }
        out
    }

    fn sample(&self, c: &Col) -> Option<&Vec<Value>> {
        let info = &self.scope[c.up][c.src];
        let p = info.columns.iter().position(|x| x.0 == c.column)?;
        info.samples.get(p).filter(|v| !v.is_empty())
    }
}

/// A column of type `ty`, own level mostly, or None when there is none.
fn draw_col(g: &mut Source, cx: &Cx, ty: Type) -> Option<Expr> {
    let cols = cx.columns(Some(ty));
    if cols.is_empty() {
        return None;
    }
    let local: Vec<Col> = cols.iter().copied().filter(|c| c.up == 0).collect();
    let c = if !local.is_empty() && g.bool_with(0.85) { *g.pick(&local) } else { *g.pick(&cols) };
    Some(Expr::Col(c))
}

/// A literal of `ty`: NULL now and then.
pub fn draw_lit(g: &mut Source, ty: Type) -> Expr {
    if g.bool_with(0.05) {
        return Expr::null(ty);
    }
    // Short text and bytes: the tables hold the long values.
    let v = match ty {
        Type::Text => Value::Text((0..g.int(0..=6)).map(|_| *g.pick(&crate::setup::ALPHABET)).collect()),
        Type::Bytea => Value::Bytea((0..g.int(0..=6)).map(|_| g.int(0u8..=255)).collect()),
        _ => fresh(g, ty),
    };
    Expr::Lit(v, ty)
}

/// A literal to compare with `e`: a value its column holds when it is a
/// column with samples, else a fresh one.
fn lit_for(g: &mut Source, cx: &Cx, e: &Expr, ty: Type) -> Expr {
    if let Expr::Col(c) = e {
        if c.ty == ty {
            if let Some(vs) = cx.sample(c) {
                if g.bool_with(0.7) {
                    return Expr::Lit(g.pick(vs).clone(), ty);
                }
            }
        }
    }
    draw_lit(g, ty)
}

/// A type an expression in this scope can have: mostly a column's.
fn draw_any_type(g: &mut Source, cx: &Cx) -> Type {
    let cols = cx.columns(None);
    if !cols.is_empty() && g.bool_with(0.8) {
        g.pick(&cols).ty
    } else {
        crate::setup::draw_type(g)
    }
}

/// A type comparable with `t`: `t` itself mostly.
fn comparable_type(g: &mut Source, t: Type) -> Type {
    let others: Vec<Type> = Type::ALL.into_iter().filter(|&u| u != t && comparable(t, u)).collect();
    if others.is_empty() || g.bool_with(0.8) {
        t
    } else {
        *g.pick(&others)
    }
}

/// An expression of type `ty`.
pub fn draw_expr(g: &mut Source, cx: &mut Cx, ty: Type) -> Expr {
    if ty == Type::Bool && cx.depth > 0 && cx.nodes > 0 {
        return draw_pred(g, cx);
    }
    if ty == Type::Jsonb {
        return draw_jsonb(g, cx);
    }
    if ty.element().is_some() {
        return draw_array(g, cx, ty);
    }
    if ty == Type::Interval {
        return draw_interval(g, cx);
    }
    if cx.depth == 0 || cx.nodes == 0 || g.bool_with(0.35) {
        return leaf(g, cx, ty);
    }
    cx.nodes -= 1;
    cx.depth -= 1;
    let e = compound(g, cx, ty);
    cx.depth += 1;
    e
}

fn leaf(g: &mut Source, cx: &Cx, ty: Type) -> Expr {
    if ty == Type::Jsonb {
        // No column or literal of it: a column's or a literal's value.
        let t = draw_any_type(g, cx);
        return Expr::Func(Func::ToJsonb, vec![leaf(g, cx, t)]);
    }
    if ty == Type::Interval {
        return Expr::Func(Func::IntervalLit(g.index(INTERVALS.len()) as u8), vec![]);
    }
    if let Some(el) = ty.element() {
        return Expr::Func(Func::Array(el), (0..g.int(1..=2)).map(|_| elem_leaf(g, cx, el)).collect());
    }
    if g.bool_with(0.75) {
        if let Some(c) = draw_col(g, cx, ty) {
            return c;
        }
    }
    draw_lit(g, ty)
}

/// A literal divisor of `ty`: neither 0 nor -1.
fn divisor(g: &mut Source, ty: Type) -> Expr {
    let v = match ty {
        Type::Float4 | Type::Float8 => Value::Float(*g.pick(&[2.0, 0.5, 3.0, -4.0, 1e10])),
        Type::Numeric => Value::Numeric(Numeric::parse(g.pick(&["3", "0.5", "-7", "1.25"])).unwrap()),
        _ => Value::Int(*g.pick(&[2, 3, 7, -2, 100])),
    };
    Expr::Lit(v, ty)
}

fn compound(g: &mut Source, cx: &mut Cx, ty: Type) -> Expr {
    use Type::*;
    let numeric_kind = ty.is_int() || ty.is_float() || ty == Numeric;
    for _ in 0..6 {
        let e = match g.int(0..14) {
            0 | 1 if numeric_kind => {
                let op = *g.pick(&[Arith::Add, Arith::Sub, Arith::Mul, Arith::Div, Arith::Mod]);
                if op == Arith::Mod && ty.is_float() {
                    continue;
                }
                let a = draw_expr(g, cx, ty);
                let b = if matches!(op, Arith::Div | Arith::Mod) { divisor(g, ty) } else { draw_expr(g, cx, ty) };
                Expr::Arith(op, a.bx(), b.bx())
            },
            2 if numeric_kind => Expr::Neg(draw_expr(g, cx, ty).bx()),
            3 if ty == Date => {
                let k = Expr::Lit(Value::Int(g.int(-400i64..=400)), Int4);
                Expr::Arith(*g.pick(&[Arith::Add, Arith::Sub]), draw_expr(g, cx, Date).bx(), k.bx())
            },
            3 if ty == Int4 && g.bool() => Expr::Arith(Arith::Sub, draw_expr(g, cx, Date).bx(), draw_expr(g, cx, Date).bx()),
            4 => {
                // CASE: searched, or simple on a value.
                let n = g.int(1..=2);
                if g.bool() {
                    let whens = (0..n).map(|_| (draw_pred(g, cx), draw_expr(g, cx, ty))).collect();
                    Expr::Case { operand: None, whens, otherwise: g.bool_with(0.7).then(|| draw_expr(g, cx, ty).bx()) }
                } else {
                    let ot = draw_any_type(g, cx);
                    let operand = draw_expr(g, cx, ot);
                    let whens = (0..n).map(|_| (lit_for(g, cx, &operand, ot), draw_expr(g, cx, ty))).collect();
                    Expr::Case { operand: Some(operand.bx()), whens, otherwise: g.bool_with(0.7).then(|| draw_expr(g, cx, ty).bx()) }
                }
            },
            5 => match g.int(0..4) {
                0 => Expr::Coalesce((0..g.int(2..=3)).map(|_| draw_expr(g, cx, ty)).collect()),
                1 => Expr::NullIf(draw_expr(g, cx, ty).bx(), draw_expr(g, cx, ty).bx()),
                2 => Expr::Greatest((0..g.int(2..=3)).map(|_| draw_expr(g, cx, ty)).collect()),
                _ => Expr::Least((0..g.int(2..=3)).map(|_| draw_expr(g, cx, ty)).collect()),
            },
            6 => {
                let froms: Vec<Type> = Type::ALL.into_iter().filter(|&f| f != ty && f != Text && cast_ok(f, ty)).collect();
                if froms.is_empty() {
                    continue;
                }
                let f = *g.pick(&froms);
                Expr::Cast(draw_expr(g, cx, f).bx(), ty)
            },
            7 | 8 => match draw_func(g, cx, ty) {
                Some(e) => e,
                None => continue,
            },
            9 if ty == Text || ty == Bytea => Expr::Concat(draw_expr(g, cx, ty).bx(), draw_expr(g, cx, ty).bx()),
            10 if cx.agg => match draw_agg(g, cx, ty) {
                Some(e) => e,
                None => continue,
            },
            11 if cx.scope.len() < MAX_LEVELS && cx.depth > 1 && g.bool_with(0.5) => match draw_scalar(g, cx, ty) {
                Some(e) => e,
                None => continue,
            },
            _ => leaf(g, cx, ty),
        };
        return e;
    }
    leaf(g, cx, ty)
}

fn int_lit(k: i64) -> Expr {
    Expr::Lit(Value::Int(k), Type::Int4)
}

fn text_lit(s: &str) -> Expr {
    Expr::Lit(Value::Text(s.into()), Type::Text)
}

/// A jsonb key: mostly one the builders use.
fn json_key(g: &mut Source) -> Expr {
    text_lit(g.pick(&["a", "b", "c", "a", "b", "x"]))
}

/// A jsonb value built from the scope's values.
fn draw_jsonb(g: &mut Source, cx: &mut Cx) -> Expr {
    use Type::*;
    if cx.depth == 0 || cx.nodes == 0 {
        return leaf(g, cx, Jsonb);
    }
    cx.nodes -= 1;
    cx.depth -= 1;
    let any = |g: &mut Source, cx: &mut Cx| {
        let t = if g.bool_with(0.15) { Jsonb } else if g.bool_with(0.1) { array_type(g, cx) } else { draw_any_type(g, cx) };
        draw_expr(g, cx, t)
    };
    let e = match g.int(0..12) {
        0 | 1 => Expr::Func(Func::ToJsonb, vec![any(g, cx)]),
        2 | 3 => {
            let n = g.int(0..=3);
            let args = (0..n).flat_map(|_| [json_key(g), any(g, cx)]).collect::<Vec<_>>();
            Expr::Func(Func::JsonbBuildObject, args)
        },
        4 | 5 => Expr::Func(Func::JsonbBuildArray, (0..g.int(0..=3)).map(|_| any(g, cx)).collect()),
        6 => Expr::Func(Func::JsonbStripNulls, vec![draw_jsonb(g, cx)]),
        7 => Expr::Func(Func::JsonbConcat, vec![draw_jsonb(g, cx), draw_jsonb(g, cx)]),
        8 => {
            let j = draw_jsonb(g, cx);
            let k = if g.bool() { json_key(g) } else { int_lit(g.int(-1i64..=2)) };
            Expr::Func(Func::JsonbGet, vec![j, k])
        },
        9 => Expr::Func(Func::JsonbGetPath(g.index(JSON_PATHS.len()) as u8), vec![draw_jsonb(g, cx)]),
        10 => Expr::Func(Func::JsonbPathQueryFirst(g.index(JSONPATHS.len()) as u8), vec![draw_jsonb(g, cx)]),
        _ => {
            let c = draw_pred(g, cx);
            Expr::Case { operand: None, whens: vec![(c, draw_jsonb(g, cx))], otherwise: Some(draw_jsonb(g, cx).bx()) }
        },
    };
    cx.depth += 1;
    e
}

/// An element of type `el` (`int8` or text) from a leaf: an integer column
/// of any width widened.
fn elem_leaf(g: &mut Source, cx: &Cx, el: Type) -> Expr {
    if el == Type::Int8 && g.bool_with(0.6) {
        let t = *g.pick(&[Type::Int2, Type::Int4]);
        if let Some(c) = draw_col(g, cx, t) {
            return Expr::Cast(c.bx(), Type::Int8);
        }
    }
    leaf(g, cx, el)
}

/// An element of type `el`.
fn draw_elem(g: &mut Source, cx: &mut Cx, el: Type) -> Expr {
    if g.bool_with(0.5) { elem_leaf(g, cx, el) } else { draw_expr(g, cx, el) }
}

/// An array type: of `int8` or of text.
fn array_type(g: &mut Source, cx: &Cx) -> Type {
    let texts = !cx.columns(Some(Type::Text)).is_empty();
    if texts && g.bool() { Type::TextArray } else { Type::Int8Array }
}

/// An array of type `ty`.
fn draw_array(g: &mut Source, cx: &mut Cx, ty: Type) -> Expr {
    let el = ty.element().unwrap();
    if cx.depth == 0 || cx.nodes == 0 {
        return leaf(g, cx, ty);
    }
    cx.nodes -= 1;
    cx.depth -= 1;
    let e = match g.int(0..12) {
        0..=3 => Expr::Func(Func::Array(el), (0..g.int(0..=4)).map(|_| draw_elem(g, cx, el)).collect()),
        4 => Expr::Func(Func::ArrayAppend, vec![draw_array(g, cx, ty), draw_elem(g, cx, el)]),
        5 => Expr::Func(Func::ArrayPrepend, vec![draw_elem(g, cx, el), draw_array(g, cx, ty)]),
        6 => Expr::Func(Func::ArrayCat, vec![draw_array(g, cx, ty), draw_array(g, cx, ty)]),
        7 => Expr::Func(Func::ArrayRemove, vec![draw_array(g, cx, ty), draw_elem(g, cx, el)]),
        8 => Expr::Func(Func::ArrayReplace, vec![draw_array(g, cx, ty), draw_elem(g, cx, el), draw_elem(g, cx, el)]),
        9 => {
            let i = g.int(-1i64..=3);
            Expr::Func(Func::Slice, vec![draw_array(g, cx, ty), int_lit(i), int_lit(i + g.int(-1i64..=3))])
        },
        10 if el == Type::Text => Expr::Func(Func::StringToArray, vec![draw_expr(g, cx, Type::Text), text_lit(g.pick(&[",", "a", " ", ""]))]),
        _ => {
            let c = draw_pred(g, cx);
            Expr::Case { operand: None, whens: vec![(c, draw_array(g, cx, ty))], otherwise: Some(draw_array(g, cx, ty).bx()) }
        },
    };
    cx.depth += 1;
    e
}

/// A timestamp from a leaf: a timestamp column, or a date one as a
/// timestamp.
fn ts_leaf(g: &mut Source, cx: &Cx) -> Expr {
    if g.bool_with(0.5) {
        if let Some(c) = draw_col(g, cx, Type::Date) {
            return Expr::Cast(c.bx(), Type::Timestamp);
        }
    }
    leaf(g, cx, Type::Timestamp)
}

fn draw_ts(g: &mut Source, cx: &mut Cx) -> Expr {
    if g.bool_with(0.5) { ts_leaf(g, cx) } else { draw_expr(g, cx, Type::Timestamp) }
}

/// An interval.
fn draw_interval(g: &mut Source, cx: &mut Cx) -> Expr {
    if cx.depth == 0 || cx.nodes == 0 {
        return leaf(g, cx, Type::Interval);
    }
    cx.nodes -= 1;
    cx.depth -= 1;
    let e = match g.int(0..12) {
        0 | 1 => leaf(g, cx, Type::Interval),
        2 | 3 => Expr::Func(Func::TsDiff, vec![draw_ts(g, cx), draw_ts(g, cx)]),
        4 => Expr::Func(Func::Age, vec![draw_ts(g, cx), draw_ts(g, cx)]),
        5 => Expr::Func(*g.pick(&[Func::IntervalAdd, Func::IntervalSub]), vec![draw_interval(g, cx), draw_interval(g, cx)]),
        6 => Expr::Func(Func::IntervalMul, vec![draw_interval(g, cx), Expr::Lit(Value::Float(*g.pick(&[2.0, 0.5, -1.0, 1e6, 0.0, 1.5])), Type::Float8)]),
        7 => Expr::Func(Func::IntervalDiv, vec![draw_interval(g, cx), Expr::Lit(Value::Float(*g.pick(&[2.0, 3.0, -0.25, 7.0])), Type::Float8)]),
        8 | 9 => Expr::Func(*g.pick(&[Func::JustifyDays, Func::JustifyHours, Func::JustifyInterval]), vec![draw_interval(g, cx)]),
        10 => Expr::Neg(draw_interval(g, cx).bx()),
        _ => {
            let c = draw_pred(g, cx);
            Expr::Case { operand: None, whens: vec![(c, draw_interval(g, cx))], otherwise: Some(draw_interval(g, cx).bx()) }
        },
    };
    cx.depth += 1;
    e
}

/// A timestamp built with intervals.
fn draw_ts_func(g: &mut Source, cx: &mut Cx) -> Expr {
    match g.int(0..5) {
        0 | 1 => {
            let a = if g.bool_with(0.4) { match draw_col(g, cx, Type::Date) { Some(c) => c, None => draw_ts(g, cx) } } else { draw_ts(g, cx) };
            Expr::Func(*g.pick(&[Func::TsPlus, Func::TsMinus]), vec![a, draw_interval(g, cx)])
        },
        2 => Expr::Func(Func::DateBin(g.index(BIN_STRIDES.len()) as u8), vec![draw_ts(g, cx)]),
        _ => Expr::Func(Func::DateTrunc, vec![text_lit(g.pick(&UNITS)), draw_ts(g, cx)]),
    }
}

/// A number extracted from a timestamp or an interval.
fn draw_extract(g: &mut Source, cx: &mut Cx, ty: Type) -> Expr {
    let iv = g.bool();
    let (k, arg) = if iv { (g.index(INTERVAL_FIELDS.len()), draw_interval(g, cx)) } else { (g.index(TIMESTAMP_FIELDS.len()), draw_ts(g, cx)) };
    let f = if ty == Type::Numeric { Func::Extract(k as u8, iv) } else { Func::DatePart(k as u8, iv) };
    Expr::Func(f, vec![arg])
}

/// An `int4` read out of an array.
fn draw_array_int(g: &mut Source, cx: &mut Cx) -> Expr {
    let ty = array_type(g, cx);
    let a = draw_array(g, cx, ty);
    match g.int(0..4) {
        0 => Expr::Func(Func::Cardinality, vec![a]),
        1 => Expr::Func(Func::ArrayLength, vec![a]),
        2 => Expr::Func(Func::ArrayUpper, vec![a]),
        _ => {
            let e = draw_elem(g, cx, ty.element().unwrap());
            Expr::Func(Func::ArrayPosition, vec![a, e])
        },
    }
}

/// A predicate on arrays.
fn draw_array_pred(g: &mut Source, cx: &mut Cx) -> Expr {
    let ty = array_type(g, cx);
    let el = ty.element().unwrap();
    match g.int(0..7) {
        0 | 1 => {
            let e = draw_elem(g, cx, el);
            Expr::Func(Func::Any(*g.pick(&CmpOp::ALL)), vec![e, draw_array(g, cx, ty)])
        },
        2 => {
            let e = draw_elem(g, cx, el);
            Expr::Func(Func::All(*g.pick(&CmpOp::ALL)), vec![e, draw_array(g, cx, ty)])
        },
        3 => Expr::Func(Func::ArrayContains, vec![draw_array(g, cx, ty), draw_array(g, cx, ty)]),
        4 => Expr::Func(Func::ArrayContained, vec![draw_array(g, cx, ty), draw_array(g, cx, ty)]),
        5 => Expr::Func(Func::ArrayOverlaps, vec![draw_array(g, cx, ty), draw_array(g, cx, ty)]),
        _ => Expr::Cmp(*g.pick(&CmpOp::ALL), draw_array(g, cx, ty).bx(), draw_array(g, cx, ty).bx()),
    }
}

/// A text read out of a jsonb value.
fn draw_jsonb_text(g: &mut Source, cx: &mut Cx) -> Expr {
    let j = draw_jsonb(g, cx);
    match g.int(0..6) {
        0 | 1 => {
            let k = if g.bool_with(0.7) { json_key(g) } else { int_lit(g.int(-1i64..=2)) };
            Expr::Func(Func::JsonbGetText, vec![j, k])
        },
        2 => Expr::Func(Func::JsonbGetPathText(g.index(JSON_PATHS.len()) as u8), vec![j]),
        3 => Expr::Func(Func::JsonbTypeof, vec![j]),
        4 => Expr::Func(Func::JsonbPretty, vec![j]),
        _ => Expr::Cast(j.bx(), Type::Text),
    }
}

/// A predicate on jsonb values.
fn draw_jsonb_pred(g: &mut Source, cx: &mut Cx) -> Expr {
    let j = draw_jsonb(g, cx);
    match g.int(0..9) {
        0 => Expr::Func(Func::JsonbContains, vec![j, draw_jsonb(g, cx)]),
        1 => Expr::Func(Func::JsonbContained, vec![j, draw_jsonb(g, cx)]),
        2 => Expr::Func(Func::JsonbHasKey, vec![j, json_key(g)]),
        3 => {
            let k = g.index(JSON_KEYSETS.len()) as u8;
            Expr::Func(if g.bool() { Func::JsonbHasAny(k) } else { Func::JsonbHasAll(k) }, vec![j])
        },
        4 | 5 => Expr::Func(Func::JsonbPathExists(g.index(JSONPATHS.len()) as u8), vec![j]),
        6 => Expr::Func(Func::JsonbPathMatch(g.index(JSONPATH_PREDICATES.len()) as u8), vec![j]),
        _ => Expr::Cmp(*g.pick(&CmpOp::ALL), j.bx(), draw_jsonb(g, cx).bx()),
    }
}

/// A function call of type `ty`, or None when no function returns it.
fn draw_func(g: &mut Source, cx: &mut Cx, ty: Type) -> Option<Expr> {
    use Type::*;
    let short = |g: &mut Source| -> String { (0..g.int(0..=2)).map(|_| *g.pick(&crate::setup::ALPHABET)).collect() };
    Some(match ty {
        t if t.is_int() || t.is_float() || t == Numeric => match g.int(0..8) {
            6 if ty == Int4 => draw_array_int(g, cx),
            6 | 7 if matches!(ty, Numeric | Float8) => draw_extract(g, cx, ty),
            7 if ty == Int8 => Expr::Func(Func::Subscript, vec![draw_array(g, cx, Int8Array), int_lit(g.int(0i64..=3))]),
            0 => Expr::Func(Func::Abs, vec![draw_expr(g, cx, ty)]),
            1 if matches!(ty, Float8 | Numeric) => Expr::Func(*g.pick(&[Func::Floor, Func::Ceil, Func::Sign, Func::Round, Func::Trunc]), vec![draw_expr(g, cx, ty)]),
            2 if ty == Numeric => Expr::Func(*g.pick(&[Func::Round, Func::Trunc]), vec![draw_expr(g, cx, Numeric), int_lit(g.int(-3i64..=6))]),
            3 if ty == Int4 => {
                let t = if g.bool_with(0.8) { Text } else { Bytea };
                Expr::Func(Func::Length, vec![draw_expr(g, cx, t)])
            },
            4 if ty == Int4 => match g.bool() {
                true => Expr::Func(Func::Strpos, vec![draw_expr(g, cx, Text), text_lit(&short(g))]),
                false => Expr::Func(Func::Scale, vec![draw_expr(g, cx, Numeric)]),
            },
            5 if ty == Int4 => {
                let args = (0..g.int(0..=3)).map(|_| { let t = draw_any_type(g, cx); draw_expr(g, cx, t) }).collect();
                Expr::Func(Func::JsonbArrayLength, vec![Expr::Func(Func::JsonbBuildArray, args)])
            },
            _ => Expr::Func(Func::Abs, vec![draw_expr(g, cx, ty)]),
        },
        Text => match g.int(0..14) {
            13 => {
                let a = if g.bool() { draw_interval(g, cx) } else { draw_ts(g, cx) };
                let k = if type_of(&a) == Interval { *g.pick(&INTERVAL_FORMATS) } else { g.index(TO_CHAR_FORMATS.len()) };
                if g.bool_with(0.7) { Expr::Func(Func::ToChar(k as u8), vec![a]) } else { Expr::Cast(a.bx(), Text) }
            },
            10 | 11 => draw_jsonb_text(g, cx),
            12 => {
                let ty = array_type(g, cx);
                let a = draw_array(g, cx, ty);
                match g.int(0..3) {
                    0 => Expr::Func(Func::ArrayToString, vec![a, text_lit(g.pick(&[",", "", " | "]))]),
                    1 => Expr::Cast(a.bx(), Text),
                    _ => Expr::Func(Func::Subscript, vec![draw_array(g, cx, TextArray), int_lit(g.int(0i64..=3))]),
                }
            },
            0 => Expr::Func(*g.pick(&[Func::Lower, Func::Upper, Func::Reverse, Func::Btrim, Func::Initcap]), vec![draw_expr(g, cx, Text)]),
            1 => {
                let t = if g.bool() { Text } else { Bytea };
                Expr::Func(Func::Md5, vec![draw_expr(g, cx, t)])
            },
            2 => Expr::Func(Func::Substr, vec![draw_expr(g, cx, Text), int_lit(g.int(-2i64..=4)), int_lit(g.int(0i64..=4))]),
            3 => Expr::Func(Func::Replace, vec![draw_expr(g, cx, Text), text_lit(&short(g)), text_lit(&short(g))]),
            4 => Expr::Func(*g.pick(&[Func::Left, Func::Right]), vec![draw_expr(g, cx, Text), int_lit(g.int(-3i64..=3))]),
            5 => {
                let n = g.int(1..=3);
                Expr::Func(Func::Concat, (0..n).map(|_| { let t = draw_any_type(g, cx); draw_expr(g, cx, t) }).collect())
            },
            6 => {
                let t = if g.bool() { Int4 } else { Int8 };
                Expr::Func(Func::ToHex, vec![draw_expr(g, cx, t)])
            },
            _ => {
                let from = draw_any_type(g, cx);
                Expr::Cast(draw_expr(g, cx, from).bx(), Text)
            },
        },
        Timestamp => draw_ts_func(g, cx),
        _ => return None,
    })
}

/// An aggregate of type `ty` (in an aggregate context), or None.
fn draw_agg(g: &mut Source, cx: &mut Cx, ty: Type) -> Option<Expr> {
    // The argument reads only the query's own columns.
    let own = cx.scope[..1].to_vec();
    let saved = std::mem::replace(&mut cx.scope, own);
    let saved_agg = std::mem::replace(&mut cx.agg, false);
    let out = (|| {
        let funcs: Vec<(AggFunc, Option<Type>)> = AggFunc::ALL
            .into_iter()
            .flat_map(|f| Type::ALL.into_iter().map(move |t| (f, Some(t))).chain(std::iter::once((f, None))))
            .filter(|&(f, a)| (a.is_some() || f == AggFunc::Count) && f.ret(a) == Some(ty))
            .collect();
        if funcs.is_empty() {
            return None;
        }
        let (f, a) = *g.pick(&funcs);
        Some(match a {
            None => Expr::Agg(f, false, None),
            Some(t) if f == AggFunc::Count && g.bool_with(0.3) => {
                let _ = t;
                Expr::Agg(AggFunc::Count, false, None)
            },
            Some(t) => {
                let arg = draw_expr(g, cx, t);
                let distinct = f != AggFunc::StringAgg && g.bool_with(0.2);
                Expr::Agg(f, distinct, Some(arg.bx()))
            },
        })
    })();
    cx.scope = saved;
    cx.agg = saved_agg;
    out
}

/// A scalar subquery of type `ty`: one aggregate over a new FROM, maybe
/// correlated through its WHERE.
fn draw_scalar(g: &mut Source, cx: &mut Cx, ty: Type) -> Option<Expr> {
    let mut sub = Cx { schema: cx.schema, scope: Vec::new(), agg: true, depth: cx.depth.min(3), outer_rows: cx.outer_rows, nodes: cx.nodes.min(10) };
    sub.scope.push(Vec::new());
    sub.scope.extend(cx.scope.iter().cloned());
    let sel = draw_select_body(g, &mut sub, Role::Scalar, Some(&[ty]))?;
    Some(Expr::Scalar(Box::new(Query::simple(sel))))
}

/// A predicate.
pub fn draw_pred(g: &mut Source, cx: &mut Cx) -> Expr {
    if cx.depth == 0 || cx.nodes == 0 {
        return simple_pred(g, cx);
    }
    cx.nodes -= 1;
    cx.depth -= 1;
    let e = match g.int(0..20) {
        0..=3 => Expr::And(draw_pred(g, cx).bx(), draw_pred(g, cx).bx()),
        4..=5 => Expr::Or(draw_pred(g, cx).bx(), draw_pred(g, cx).bx()),
        6 => Expr::Not(draw_pred(g, cx).bx()),
        7 => Expr::Is(draw_pred(g, cx).bx(), *g.pick(&[IsWhat::True, IsWhat::False, IsWhat::Unknown]), g.bool()),
        8 if cx.scope.len() < MAX_LEVELS && cx.depth > 0 => match draw_sub(g, cx, None) {
            Some(q) => Expr::Exists(Box::new(q), g.bool_with(0.3)),
            None => simple_pred(g, cx),
        },
        9 if cx.scope.len() < MAX_LEVELS && cx.depth > 0 => {
            let t = draw_any_type(g, cx);
            let a = draw_expr(g, cx, t);
            match draw_sub(g, cx, Some(t)) {
                Some(q) => Expr::InQuery(a.bx(), Box::new(q), g.bool_with(0.3)),
                None => simple_pred(g, cx),
            }
        },
        _ => simple_pred(g, cx),
    };
    cx.depth += 1;
    e
}

/// A comparison-like predicate over expressions of the scope.
fn simple_pred(g: &mut Source, cx: &mut Cx) -> Expr {
    let depth = cx.depth;
    if depth > 0 && cx.nodes > 0 && g.bool_with(0.18) {
        cx.depth = depth.min(2);
        let e = match g.int(0..10) {
            0..=3 => draw_array_pred(g, cx),
            4..=6 => draw_jsonb_pred(g, cx),
            _ => {
                let a = draw_interval(g, cx);
                Expr::Cmp(*g.pick(&CmpOp::ALL), a.bx(), draw_interval(g, cx).bx())
            },
        };
        cx.depth = depth;
        return e;
    }
    let t = draw_any_type(g, cx);
    cx.depth = depth.min(2);
    let a = draw_expr(g, cx, t);
    let e = match g.int(0..20) {
        0..=8 => {
            let u = comparable_type(g, t);
            let b = if g.bool_with(0.6) { lit_for(g, cx, &a, u) } else { draw_expr(g, cx, u) };
            Expr::Cmp(*g.pick(&CmpOp::ALL), a.bx(), b.bx())
        },
        9 => {
            let b = draw_expr(g, cx, t);
            Expr::Distinct(g.bool(), a.bx(), b.bx())
        },
        10 | 11 => Expr::IsNull(a.bx(), g.bool()),
        12 | 13 => {
            let (lo, hi) = (lit_for(g, cx, &a, t), lit_for(g, cx, &a, t));
            Expr::Between { e: a.bx(), lo: lo.bx(), hi: hi.bx(), not: g.bool_with(0.2), symmetric: g.bool_with(0.3) }
        },
        14 | 15 => {
            let list = (0..g.int(1..=4)).map(|_| lit_for(g, cx, &a, t)).collect();
            Expr::InList(a.bx(), list, g.bool_with(0.2))
        },
        16 | 17 if t == Type::Text => {
            let mut pattern: String = (0..g.int(0..=4)).map(|_| *g.pick(&['a', 'b', '%', '_', 'A', '\u{e9}', ' '])).collect();
            if let (Expr::Col(c), true) = (&a, g.bool()) {
                if let Some(Value::Text(s)) = cx.sample(c).map(|v| g.pick(v).clone()) {
                    pattern = s.chars().take(3).collect::<String>() + "%";
                }
            }
            if !pattern_ok(&pattern) {
                pattern.push('\\');
            }
            Expr::Like { e: a.bx(), pattern, not: g.bool_with(0.2), insensitive: g.bool_with(0.3) }
        },
        18 if t == Type::Text => Expr::Regex(a.bx(), g.index(REGEXES.len())),
        19 if t == Type::Text => Expr::Func(Func::StartsWith, vec![a, text_lit(if g.bool() { "a" } else { "" })]),
        _ => {
            let b = lit_for(g, cx, &a, t);
            Expr::Cmp(*g.pick(&CmpOp::ALL), a.bx(), b.bx())
        },
    };
    cx.depth = depth;
    e
}

/// A subquery for EXISTS or IN (one item of type `item` for IN): a new
/// FROM inside the scope, so that it may be correlated.
fn draw_sub(g: &mut Source, cx: &mut Cx, item: Option<Type>) -> Option<Query> {
    let mut sub = Cx { schema: cx.schema, scope: Vec::new(), agg: false, depth: cx.depth.min(3), outer_rows: cx.outer_rows, nodes: cx.nodes.min(10) };
    sub.scope.push(Vec::new());
    sub.scope.extend(cx.scope.iter().cloned());
    let wanted = item.map(|t| vec![t]);
    let sel = draw_select_body(g, &mut sub, Role::Sub, wanted.as_deref())?;
    Some(Query::simple(sel))
}

/// The sources a SELECT may read: the tables, and the CTEs at the top
/// level.
fn sources_for(cx: &Cx, role: Role) -> Vec<(Src, SourceInfo)> {
    let mut out: Vec<(Src, SourceInfo)> = cx.schema.tables.iter().map(|(id, i)| (Src::Table(*id), i.clone())).collect();
    if matches!(role, Role::Top | Role::Member) {
        out.extend(cx.schema.ctes.iter().enumerate().map(|(k, i)| (Src::Cte(k), i.clone())));
    }
    out
}

/// A join condition between the new source (`new`) and earlier ones:
/// an equality of columns of one type mostly (values the tables share
/// make it match), else any predicate.
fn draw_on(g: &mut Source, cx: &mut Cx, new: usize, full: bool) -> Option<Expr> {
    let level = &cx.scope[0];
    let mut pairs = Vec::new();
    for &(c, t) in &level[new].columns {
        for (src, info) in level.iter().enumerate().take(new) {
            for &(d, u) in &info.columns {
                if t == u {
                    pairs.push((Col { up: 0, src: new, column: c, ty: t }, Col { up: 0, src, column: d, ty: u }));
                }
            }
        }
    }
    if full || (!pairs.is_empty() && g.bool_with(0.75)) {
        if pairs.is_empty() {
            return None;
        }
        let (a, b) = *g.pick(&pairs);
        let eq = Expr::Cmp(CmpOp::Eq, Expr::Col(a).bx(), Expr::Col(b).bx());
        if !full && g.bool_with(0.25) {
            return Some(Expr::And(eq.bx(), draw_pred(g, cx).bx()));
        }
        return Some(eq);
    }
    Some(draw_pred(g, cx))
}

/// A SELECT in `role` with items of the types `wanted` when given, its
/// FROM in `cx.scope[0]` (which is filled), or None when no source fits.
fn draw_select_body(g: &mut Source, cx: &mut Cx, role: Role, wanted: Option<&[Type]>) -> Option<Select> {
    let all = sources_for(cx, role);
    if all.is_empty() {
        return None;
    }
    let budget = (ROW_BUDGET / cx.outer_rows.max(1)).max(1);
    cx.scope[0] = Vec::new();
    let item = if g.bool_with(FROM_ITEM_FIRST) { draw_from_item(g, cx, false).filter(|(_, i)| i.rows.max(1) as u64 <= budget) } else { None };
    let (first, info) = item.unwrap_or_else(|| g.pick(&all).clone());
    let mut rows = info.rows.max(1) as u64;
    cx.scope[0] = vec![info];
    let mut joins = Vec::new();
    let most = if role == Role::Top || role == Role::Member { MAX_SOURCES } else { 2 };
    let n = match g.int(0..10) {
        0..=4 => 0,
        5..=7 => 1,
        8 => 2,
        _ => most - 1,
    }
    .min(most - 1);
    for _ in 0..n {
        let fits: Vec<&(Src, SourceInfo)> = all.iter().filter(|(_, i)| rows.saturating_mul(i.rows.max(1) as u64) <= budget).collect();
        if fits.is_empty() {
            break;
        }
        let kind = *g.weighted(&[(5, JoinKind::Inner), (4, JoinKind::Left), (2, JoinKind::Right), (2, JoinKind::Full), (1, JoinKind::Cross)]);
        let item = if g.bool_with(FROM_ITEM_JOIN) { draw_from_item(g, cx, lateral_join(kind)).filter(|(_, i)| rows.saturating_mul(i.rows.max(1) as u64) <= budget) } else { None };
        let (src, info) = item.unwrap_or_else(|| (*g.pick(&fits)).clone());
        cx.scope[0].push(info.clone());
        let new = cx.scope[0].len() - 1;
        let on = match kind {
            JoinKind::Cross => None,
            _ => match draw_on(g, cx, new, kind == JoinKind::Full) {
                Some(on) => Some(on),
                None => {
                    cx.scope[0].pop();
                    continue;
                },
            },
        };
        rows = rows.saturating_mul(info.rows.max(1) as u64);
        joins.push(Join { kind, src, on });
    }
    cx.outer_rows = cx.outer_rows.saturating_mul(rows);
    let filter = g.bool_with(if role == Role::Scalar { 0.6 } else { 0.8 }).then(|| draw_pred(g, cx));
    let mut sel = Select { distinct: false, items: Vec::new(), from: first, joins, filter, group: Vec::new(), sets: None, having: None };
    match role {
        Role::Scalar => {
            let ty = wanted?[0];
            cx.agg = true;
            let item = draw_agg(g, cx, ty)?;
            sel.items = vec![item];
        },
        Role::Cte => {
            let n = g.int(1..=3);
            sel.items = (0..n).map(|_| { let t = draw_any_type(g, cx); draw_expr(g, cx, t) }).collect();
        },
        _ => draw_items(g, cx, &mut sel, role, wanted),
    }
    Some(sel)
}

/// How often a SELECT's first source, and a joined one, is a function or
/// subquery ([`draw_from_item`]).
const FROM_ITEM_FIRST: f64 = 0.3;
const FROM_ITEM_JOIN: f64 = 0.6;

/// Set-returning functions or a subquery as a source of the SELECT being
/// drawn, reading the sources before it (`cx.scope[0]`) when `lateral`,
/// and the queries around it.
fn draw_from_item(g: &mut Source, cx: &mut Cx, lateral: bool) -> Option<(Src, SourceInfo)> {
    let saved = std::mem::take(&mut cx.scope[0]);
    if lateral {
        cx.scope[0] = saved.clone();
    }
    let (depth, nodes) = (cx.depth, cx.nodes);
    cx.depth = cx.depth.min(2);
    cx.nodes = cx.nodes.min(6);
    let src = if cx.scope.len() < MAX_LEVELS && g.bool_with(0.35) { draw_from_sub(g, cx) } else { draw_from_fn(g, cx) };
    cx.depth = depth;
    cx.nodes = nodes;
    cx.scope[0] = saved;
    let src = src?;
    let refs: Vec<Vec<&SourceInfo>> = cx.scope.iter().map(|l| l.iter().collect()).collect();
    if !from_item_ok(&src, cx.schema, &refs, lateral) {
        return None;
    }
    let info = cx.schema.source(&src)?.into_owned();
    Some((src, info))
}

fn draw_from_sub(g: &mut Source, cx: &mut Cx) -> Option<Src> {
    let mut sub = Cx { schema: cx.schema, scope: vec![Vec::new()], agg: false, depth: 2, outer_rows: cx.outer_rows, nodes: 8 };
    sub.scope.extend(cx.scope.iter().cloned());
    let sel = if g.bool_with(0.4) {
        let t = *g.pick(&[Type::Int8, Type::Numeric, Type::Int4, Type::Text, Type::Date]);
        draw_select_body(g, &mut sub, Role::Scalar, Some(&[t]))?
    } else {
        draw_select_body(g, &mut sub, Role::Cte, None)?
    };
    Some(Src::Sub(Box::new(sel)))
}

fn draw_from_fn(g: &mut Source, cx: &mut Cx) -> Option<Src> {
    if g.bool_with(0.25) {
        // JSON_TABLE over objects mostly (keys its paths read).
        let elems: Vec<Expr> = (0..g.int(1..=4))
            .map(|_| {
                if g.bool_with(0.7) {
                    let args = (0..g.int(1..=3))
                        .flat_map(|_| {
                            let t = draw_any_type(g, cx);
                            [json_key(g), draw_expr(g, cx, t)]
                        })
                        .collect();
                    Expr::Func(Func::JsonbBuildObject, args)
                } else {
                    let t = draw_any_type(g, cx);
                    draw_expr(g, cx, t)
                }
            })
            .collect();
        // The known bug's exclusion (`jt_collation_free`): elements that
        // consume text are left out.
        let mut elems: Vec<Expr> = elems.into_iter().filter(jt_collation_free).collect();
        if elems.is_empty() {
            let t = *g.pick(&[Type::Int4, Type::Int8, Type::Numeric, Type::Date]);
            elems.push(Expr::Func(Func::JsonbBuildObject, vec![json_key(g), draw_expr(g, cx, t)]));
        }
        let path = |g: &mut Source| g.index(JT_PATHS.len()) as u8;
        let mut cols = Vec::new();
        for _ in 0..g.int(1..=4) {
            let ord = cols.contains(&JtCol::Ordinality);
            cols.push(match g.int(0..10) {
                0 if !ord => JtCol::Ordinality,
                1 | 2 => JtCol::Exists(path(g)),
                _ => JtCol::Path(*g.pick(&JT_TYPES), path(g)),
            });
        }
        return Some(Src::Func(Box::new(FromFn { calls: vec![Call::JsonTable(Expr::Func(Func::JsonbBuildArray, elems), cols)], ordinality: false })));
    }
    let n = if g.bool_with(0.2) { 2 } else { 1 };
    let calls = (0..n).map(|_| draw_call(g, cx)).collect();
    Some(Src::Func(Box::new(FromFn { calls, ordinality: g.bool_with(0.3) })))
}

fn draw_call(g: &mut Source, cx: &mut Cx) -> Call {
    match g.int(0..14) {
        10 => {
            let e = if g.bool_with(0.7) { draw_col(g, cx, Type::Int8).unwrap_or_else(|| draw_expr(g, cx, Type::Int8)) } else { draw_expr(g, cx, Type::Int8) };
            Call::SeriesTo(e, g.int(1u8..=6))
        },
        11 => Call::SeriesTs(*g.pick(&[0i64, 86_400_000_000 * 365, -86_400_000_000 * 40, 1_234_567_890_123]), g.int(0u8..=6), g.int(0u8..4)),
        12 => {
            let lo = g.int(-30i64..=30);
            let step = *g.pick(&[5i64, 3, 10, 25, -5]);
            Call::SeriesNum(lo, lo + step * g.int(0i64..=8), step)
        },
        13 => {
            let t: String = (0..g.int(0..=10)).map(|_| *g.pick(&['a', 'b', ',', ' ', 'x', ','])).collect();
            Call::StringTable(t, g.pick(&[",", " ", "", "ab"]).to_string())
        },
        0..=2 => {
            let lo = g.int(-3i64..=10);
            let len = g.int(0i64..=12);
            let step = *g.pick(&[1i64, 1, 1, 2, 3, -1]);
            Call::Series(lo, if step < 0 { lo - len } else { lo + len }, step)
        },
        3..=5 => {
            let mut t = draw_any_type(g, cx);
            if t.internal() {
                t = Type::Int8;
            }
            let xs = (0..g.int(1..=4)).map(|_| draw_expr(g, cx, t)).collect();
            Call::UnnestList(t, xs)
        },
        6 => {
            let t = array_type(g, cx);
            let e = draw_array(g, cx, t);
            if array_bound(&e).is_some_and(|n| n <= 16) {
                Call::Unnest(e)
            } else {
                let el = t.element().unwrap();
                Call::Unnest(Expr::Func(Func::Array(el), vec![draw_elem(g, cx, el)]))
            }
        },
        7 | 8 => {
            let args = (0..g.int(1..=3))
                .flat_map(|_| {
                    let t = draw_any_type(g, cx);
                    [json_key(g), draw_expr(g, cx, t)]
                })
                .collect();
            Call::JsonbEach(Expr::Func(Func::JsonbBuildObject, args))
        },
        _ => {
            let args = (0..g.int(1..=3))
                .map(|_| {
                    let t = draw_any_type(g, cx);
                    draw_expr(g, cx, t)
                })
                .collect();
            Call::JsonbElems(Expr::Func(Func::JsonbBuildArray, args))
        },
    }
}

/// Items, and for an aggregate query its keys and HAVING.
fn draw_items(g: &mut Source, cx: &mut Cx, sel: &mut Select, role: Role, wanted: Option<&[Type]>) {
    let types: Vec<Type> = match wanted {
        Some(ts) => ts.to_vec(),
        None => (0..g.int(1..=3)).map(|_| draw_any_type(g, cx)).collect(),
    };
    let grouped = role != Role::Sub && g.bool_with(0.25);
    if grouped {
        // Keys of the wanted types (items are keys as written, or
        // aggregates).
        let nkeys = g.int(1..=2);
        let mut keys: Vec<Expr> = (0..nkeys).map(|_| { let t = draw_any_type(g, cx); let d = cx.depth; cx.depth = 1; let k = draw_expr(g, cx, t); cx.depth = d; k }).filter(|k| !k.has_subquery()).collect();
        if keys.is_empty() {
            keys.push(leaf(g, cx, types[0]));
        }
        let key_types: Vec<Type> = keys.iter().map(|k| type_of(k)).collect();
        cx.agg = true;
        sel.items = types
            .iter()
            .map(|&t| match key_types.iter().position(|&kt| kt == t) {
                Some(p) if g.bool_with(0.6) => keys[p].clone(),
                _ => draw_agg(g, cx, t).unwrap_or_else(|| keys.iter().find(|k| type_of(k) == t).cloned().unwrap_or(Expr::null(t))),
            })
            .collect();
        if g.bool_with(0.3) {
            let n = Expr::Agg(AggFunc::Count, false, None);
            let lit = Expr::Lit(Value::Int(g.int(0i64..=3)), Type::Int8);
            sel.having = Some(Expr::Cmp(*g.pick(&CmpOp::ALL), n.bx(), lit.bx()));
        }
        cx.agg = false;
        // A known PostgreSQL bug: see `keeps_column` and `check_select`.
        let column_keys = keys.iter().all(keeps_column);
        if column_keys && g.bool_with(0.4) {
            let n = keys.len();
            sel.sets = Some(match g.int(0..4) {
                0 => GroupSets::Rollup,
                1 => GroupSets::Cube,
                2 if n >= 2 => GroupSets::Mixed,
                _ => {
                    let mut sets: Vec<Vec<u8>> = (0..g.int(1..=3)).map(|_| (0..n as u8).filter(|_| g.bool()).collect()).collect();
                    // Every key in some set.
                    for k in 0..n as u8 {
                        if !sets.iter().any(|set| set.contains(&k)) {
                            sets[0].push(k);
                        }
                    }
                    GroupSets::Sets(sets)
                },
            });
        }
        sel.group = keys;
    } else if wanted.is_none() && g.bool_with(0.25) && role == Role::Top {
        // `*`.
        sel.items = Vec::new();
    } else if wanted.is_none() && role == Role::Top && g.bool_with(0.15) {
        // One aggregate over the whole FROM.
        cx.agg = true;
        let t = *g.pick(&[Type::Int8, Type::Numeric, Type::Int4, Type::Text, Type::Date]);
        sel.items = vec![draw_agg(g, cx, t).unwrap_or(Expr::Agg(AggFunc::Count, false, None))];
        cx.agg = false;
    } else {
        let windows = matches!(role, Role::Top | Role::Member) && g.bool_with(0.25);
        sel.items = Vec::new();
        for &t in &types {
            let mut e = if windows && g.bool_with(0.6) { draw_window(g, cx, t).unwrap_or_else(|| draw_expr(g, cx, t)) } else { draw_expr(g, cx, t) };
            // At most one window whose values depend on the order of peers.
            sel.items.push(e.clone());
            if positional_windows(sel) > 1 {
                sel.items.pop();
                e = draw_expr(g, cx, t);
                sel.items.push(e);
            }
        }
    }
    sel.distinct = role != Role::Sub && g.bool_with(0.15);
}

/// Every column of the query's own sources, as order terms.
fn all_columns(cx: &Cx) -> Vec<Expr> {
    cx.scope[0].iter().enumerate().flat_map(|(src, info)| info.columns.iter().map(move |&(column, ty)| Expr::Col(Col { up: 0, src, column, ty }))).collect()
}

/// A window function of type `ty`, or None when none returns it.
fn draw_window(g: &mut Source, cx: &mut Cx, ty: Type) -> Option<Expr> {
    use Type::*;
    let own = cx.scope[..1].to_vec();
    let saved = std::mem::replace(&mut cx.scope, own);
    let out = (|| {
        let mut options: Vec<WinFunc> = Vec::new();
        match ty {
            Int8 => options.extend([WinFunc::RowNumber, WinFunc::Rank, WinFunc::DenseRank, WinFunc::Agg(AggFunc::Count)]),
            Float8 => options.extend([WinFunc::PercentRank, WinFunc::CumeDist]),
            Int4 => options.push(WinFunc::Ntile(g.int(1..=4))),
            _ => {},
        }
        options.extend([WinFunc::Lag(g.int(1..=2)), WinFunc::Lead(g.int(1..=2)), WinFunc::FirstValue, WinFunc::LastValue, WinFunc::NthValue(g.int(1..=3))]);
        for f in [AggFunc::Sum, AggFunc::Avg, AggFunc::Min, AggFunc::Max, AggFunc::BoolAnd, AggFunc::BoolOr] {
            for a in Type::ALL {
                if f.ret(Some(a)) == Some(ty) {
                    options.push(WinFunc::Agg(f));
                    break;
                }
            }
        }
        let func = *g.pick(&options);
        let args = match func {
            WinFunc::Lag(_) | WinFunc::Lead(_) | WinFunc::FirstValue | WinFunc::LastValue | WinFunc::NthValue(_) => vec![draw_expr(g, cx, ty)],
            WinFunc::Agg(AggFunc::Count) if g.bool() => vec![],
            WinFunc::Agg(AggFunc::Count) => {
                let t = draw_any_type(g, cx);
                vec![draw_expr(g, cx, t)]
            },
            WinFunc::Agg(f) => {
                let arg: Vec<Type> = Type::ALL.into_iter().filter(|&a| f.ret(Some(a)) == Some(ty)).collect();
                let a = *g.pick(&arg);
                vec![draw_expr(g, cx, a)]
            },
            _ => vec![],
        };
        let depth = cx.depth;
        cx.depth = 1;
        let mut partition = Vec::new();
        for _ in 0..*g.pick(&[0, 1, 1, 2]) {
            let t = draw_any_type(g, cx);
            partition.push(leaf(g, cx, t));
        }
        let mut order: Vec<(Expr, bool, Option<bool>)> = Vec::new();
        for _ in 0..g.int(0..=2) {
            let t = draw_any_type(g, cx);
            let e = leaf(g, cx, t);
            order.push((e, g.bool_with(0.3), g.bool_with(0.2).then(|| g.bool())));
        }
        cx.depth = depth;
        let frame = g.bool_with(0.4).then(|| {
            let mode = *g.pick(&[FrameMode::Rows, FrameMode::Range, FrameMode::Groups]);
            let offset = mode != FrameMode::Range;
            let bound = |g: &mut Source| match g.int(0..5) {
                0 => Bound::UnboundedPreceding,
                1 if offset => Bound::Preceding(g.int(0..=2)),
                2 => Bound::CurrentRow,
                3 if offset => Bound::Following(g.int(0..=2)),
                _ => Bound::UnboundedFollowing,
            };
            let (a, b) = (bound(g), bound(g));
            let (start, end) = if a.rank() <= b.rank() { (a, b) } else { (b, a) };
            let exclude = *g.pick(&[Exclude::NoOthers, Exclude::NoOthers, Exclude::CurrentRow, Exclude::Group, Exclude::Ties]);
            Frame { mode, start, end, exclude }
        });
        let frame = frame.filter(|f| f.ok() && !(f.mode == FrameMode::Groups && order.is_empty()));
        if func.needs_total() || frame.is_some_and(|f| f.needs_total()) {
            for c in all_columns(cx) {
                if !order.iter().any(|(e, ..)| *e == c) {
                    order.push((c, g.bool_with(0.2), None));
                }
            }
        }
        let frame = frame.filter(|f| !(f.mode == FrameMode::Groups && order.is_empty()));
        Some(Expr::Window(Box::new(WinCall { func, args, partition, order, frame })))
    })();
    cx.scope = saved;
    out
}

/// The type of an expression whose columns and literals carry theirs (a
/// drawn one), without the schema.
pub fn type_of(e: &Expr) -> Type {
    use Type::*;
    match e {
        Expr::Col(c) => c.ty,
        Expr::Lit(_, t) | Expr::Cast(_, t) => *t,
        Expr::Cmp(..) | Expr::Distinct(..) | Expr::And(..) | Expr::Or(..) | Expr::Not(_) | Expr::IsNull(..) | Expr::Is(..) => Bool,
        Expr::Between { .. } | Expr::InList(..) | Expr::Like { .. } | Expr::Regex(..) | Expr::Exists(..) | Expr::InQuery(..) => Bool,
        Expr::Arith(op, a, b) => arith_type(*op, type_of(a), type_of(b)).unwrap_or(Int4),
        Expr::Neg(a) | Expr::Concat(a, _) | Expr::NullIf(a, _) => type_of(a),
        Expr::Case { whens, .. } => whens.first().map_or(Int4, |(_, t)| type_of(t)),
        Expr::Coalesce(xs) | Expr::Greatest(xs) | Expr::Least(xs) => xs.first().map_or(Int4, type_of),
        Expr::Func(f, xs) => f.ret(xs, &xs.iter().map(type_of).collect::<Vec<_>>()).unwrap_or(Text),
        Expr::Agg(f, _, a) => f.ret(a.as_ref().map(|a| type_of(a))).unwrap_or(Int8),
        Expr::Scalar(q) => q.select.items.first().map_or(Int8, type_of),
        Expr::Window(w) => w.func.ret(&w.args.iter().map(type_of).collect::<Vec<_>>()).unwrap_or(Int8),
    }
}

/// A query over `schema`: CTEs now and then, a SELECT, compound members
/// now and then, an order by every column with a limit now and then.
pub fn draw_query(g: &mut Source, schema: &Schema) -> Option<Query> {
    let mut s = Schema { tables: schema.tables.clone(), ctes: Vec::new() };
    let mut with = Vec::new();
    if g.bool_with(0.15) {
        for _ in 0..g.int(1..=MAX_CTES) {
            let mut cx = Cx { schema: &s, scope: vec![Vec::new()], agg: false, depth: 3, outer_rows: 1, nodes: 16 };
            let Some(sel) = draw_select_body(g, &mut cx, Role::Cte, None) else { break };
            let Some(types) = check_select(&sel, &s, &[], Role::Cte) else { break };
            let rows = select_budget(&sel, &s).min(ROW_BUDGET) as usize;
            s.ctes.push(SourceInfo { columns: (0..types.len() as u32).zip(types).collect(), rows, samples: Vec::new() });
            let materialized = *g.pick(&[Materialized::Default, Materialized::Default, Materialized::Yes, Materialized::No]);
            with.push(Cte { query: sel, materialized });
        }
    }
    let mut cx = Cx { schema: &s, scope: vec![Vec::new()], agg: false, depth: 4, outer_rows: 1, nodes: 30 };
    let select = draw_select_body(g, &mut cx, Role::Top, None)?;
    let mut q = Query { with, select, compound: Vec::new(), order: Vec::new(), limit: None };
    let types = q.types(schema)?;
    if !q.select.items.is_empty() && g.bool_with(0.2) {
        for _ in 0..g.int(1..=2) {
            let mut cx = Cx { schema: &s, scope: vec![Vec::new()], agg: false, depth: 3, outer_rows: 1, nodes: 16 };
            if let Some(m) = draw_select_body(g, &mut cx, Role::Member, Some(&types)) {
                q.compound.push((*g.pick(&SetOp::ALL), m));
            }
        }
    }
    if g.bool_with(0.3) {
        let mut cols: Vec<usize> = (1..=types.len()).collect();
        while !cols.is_empty() {
            let k = cols.remove(g.index(cols.len()));
            q.order.push((k, g.bool_with(0.3), g.bool_with(0.2).then(|| g.bool())));
        }
        if g.bool_with(0.5) {
            q.limit = Some(Limit { count: *g.pick(&[0, 1, 2, 3, 5, 10]), offset: if g.bool_with(0.3) { g.int(1..=3) } else { 0 } });
        }
    }
    let q = if q.valid(schema) { q } else { Query { compound: Vec::new(), ..q } };
    q.valid(schema).then_some(q)
}

// -------------------------------------------------------------- shrinking

/// Simpler expressions of the same type, with the scope's columns.
fn simpler_exprs(e: &Expr) -> Vec<Expr> {
    let ty = type_of(e);
    let mut out = Vec::new();
    if !matches!(e, Expr::Lit(Value::Null, _)) {
        out.push(Expr::null(ty));
    }
    for c in e.children() {
        if type_of(c) == ty {
            out.push(c.clone());
        }
    }
    if let Expr::Lit(v, t) = e {
        out.extend(crate::setup::simpler_values(v).into_iter().map(|w| Expr::Lit(w, *t)));
    }
    // Each child simplified in place.
    for k in 0..e.children().len() {
        let child = e.children()[k].clone();
        for s in simpler_exprs(&child) {
            let mut c = e.clone();
            *children_mut(&mut c)[k] = s;
            out.push(c);
        }
    }
    out
}

fn simpler_selects(s: &Select) -> Vec<Select> {
    let mut out = Vec::new();
    let mut push = |f: &dyn Fn(&mut Select)| {
        let mut c = s.clone();
        f(&mut c);
        if c != *s {
            out.push(c);
        }
    };
    push(&|c| c.filter = None);
    push(&|c| c.distinct = false);
    push(&|c| c.having = None);
    push(&|c| c.sets = None);
    push(&|c| {
        c.group.clear();
        c.sets = None;
        c.having = None;
    });
    if !s.joins.is_empty() {
        push(&|c| {
            c.joins.pop();
        });
    }
    for k in 0..s.items.len() {
        if s.items.len() > 1 {
            push(&|c| {
                c.items.remove(k);
            });
        }
    }
    for k in 0..s.joins.len() {
        if s.joins[k].kind != JoinKind::Inner && s.joins[k].on.is_some() {
            push(&|c| c.joins[k].kind = JoinKind::Inner);
        }
    }
    // Expressions simplified, one at a time (a few candidates each).
    let n_items = s.items.len();
    for k in 0..n_items {
        for x in simpler_exprs(&s.items[k]).into_iter().take(12) {
            push(&|c| c.items[k] = x.clone());
        }
    }
    if let Some(w) = &s.filter {
        for x in simpler_exprs(w).into_iter().take(24) {
            push(&|c| c.filter = Some(x.clone()));
        }
    }
    for k in 0..s.joins.len() {
        if let Some(on) = &s.joins[k].on {
            for x in simpler_exprs(on).into_iter().take(12) {
                push(&|c| c.joins[k].on = Some(x.clone()));
            }
        }
    }
    out
}

/// Simpler queries (not checked for validity).
pub fn simpler_queries(q: &Query) -> Vec<Query> {
    let mut out = Vec::new();
    let mut push = |c: Query| {
        if c != *q {
            out.push(c);
        }
    };
    if q.limit.is_some() {
        push(Query { limit: None, ..q.clone() });
    }
    if !q.order.is_empty() {
        push(Query { order: Vec::new(), limit: None, ..q.clone() });
    }
    if !q.compound.is_empty() {
        push(Query { compound: Vec::new(), ..q.clone() });
        let mut c = q.clone();
        c.compound.pop();
        push(c);
    }
    if !q.with.is_empty() {
        // The last CTE gone (valid only when nothing reads it).
        let mut c = q.clone();
        c.with.pop();
        push(c);
        let mut c = q.clone();
        c.with.iter_mut().for_each(|x| x.materialized = Materialized::Default);
        push(c);
    }
    for s in simpler_selects(&q.select) {
        push(Query { select: s, ..q.clone() });
    }
    for (k, (_, m)) in q.compound.iter().enumerate() {
        for s in simpler_selects(m).into_iter().take(8) {
            let mut c = q.clone();
            c.compound[k].1 = s;
            push(c);
        }
    }
    out
}

// -------------------------------------------------------------- generator

/// Queries over a schema: 1 to 6 (none when there is no table).
pub struct Queries(pub Schema);

/// The cost of a list of queries.
pub fn queries_cost(qs: &[Query]) -> u64 {
    qs.iter().map(|q| 1 + q.cost()).sum()
}

impl Gen for Queries {
    type Value = Vec<Query>;

    fn generate(&self, g: &mut Source) -> Vec<Query> {
        if self.0.tables.is_empty() {
            return Vec::new();
        }
        let n = g.int(1..=6);
        (0..n).filter_map(|_| draw_query(g, &self.0)).collect()
    }

    fn shrink(&self, value: &Vec<Query>, visit: &mut dyn FnMut(Vec<Query>) -> ControlFlow<()>) -> ControlFlow<()> {
        let here = queries_cost(value);
        let mut offer = |c: Vec<Query>| if queries_cost(&c) < here && c.iter().all(|q| q.valid(&self.0)) { visit(c) } else { ControlFlow::Continue(()) };
        for k in 0..value.len() {
            let mut c = value.clone();
            c.remove(k);
            offer(c)?;
        }
        for (k, q) in value.iter().enumerate() {
            for s in simpler_queries(q) {
                let mut c = value.clone();
                c[k] = s;
                offer(c)?;
            }
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, value: &Vec<Query>, g: &mut Source) -> Vec<Query> {
        let mut out = value.clone();
        match g.int(0..3) {
            0 if !out.is_empty() => {
                let k = g.index(out.len());
                if let Some(q) = draw_query(g, &self.0) {
                    out[k] = q;
                }
            },
            1 if !out.is_empty() => {
                out.remove(g.index(out.len()));
            },
            _ => {
                if let Some(q) = draw_query(g, &self.0) {
                    out.push(q);
                }
            },
        }
        if out.is_empty() {
            crate::fell_back(&crate::QUERY_FALLBACKS);
            return self.generate(g);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(column: u32, ty: Type) -> Expr {
        Expr::Col(Col { up: 0, src: 0, column, ty })
    }
    fn lit(v: Value, ty: Type) -> Expr {
        Expr::Lit(v, ty)
    }

    /// The folded keys the workspace found (2026-10-19..21) are refused,
    /// keys whose column survives are kept.
    #[test]
    fn grouping_keys_that_fold_away() {
        use Type::*;
        let c = col(0, Int8);
        let b = col(1, Bool);
        let gt = |a: Expr, b: Expr| Expr::Cmp(CmpOp::Gt, a.bx(), b.bx());
        // CASE '3'::int8 WHEN '770'::int8 THEN c ELSE '3'::int8 END
        let simple = |subject: Expr| Expr::Case { operand: Some(subject.bx()), whens: vec![(lit(Value::Int(770), Int8), c.clone())], otherwise: Some(lit(Value::Int(3), Int8).bx()) };
        assert!(!keeps_column(&simple(lit(Value::Int(3), Int8))));
        assert!(keeps_column(&simple(c.clone())));
        // CASE WHEN '689'::int8 > '27'::int8 THEN '5'::int8 ELSE c END
        let searched = |w: Expr| Expr::Case { operand: None, whens: vec![(w, lit(Value::Int(5), Int8))], otherwise: Some(c.clone().bx()) };
        assert!(!keeps_column(&searched(gt(lit(Value::Int(689), Int8), lit(Value::Int(27), Int8)))));
        assert!(keeps_column(&searched(lit(Value::Bool(false), Bool))));
        assert!(keeps_column(&searched(gt(c.clone(), lit(Value::Int(27), Int8)))));
        // coalesce('-5'::int8, c) vs coalesce(NULL::int8, c)
        assert!(!keeps_column(&Expr::Coalesce(vec![lit(Value::Int(-5), Int8), c.clone()])));
        assert!(keeps_column(&Expr::Coalesce(vec![Expr::null(Int8), c.clone()])));
        // c > 1 OR <a constant>; c > 1 OR false
        let c_gt = gt(c.clone(), lit(Value::Int(1), Int8));
        assert!(!keeps_column(&Expr::Or(c_gt.clone().bx(), gt(lit(Value::Int(2), Int8), lit(Value::Int(1), Int8)).bx())));
        assert!(keeps_column(&Expr::Or(c_gt.clone().bx(), lit(Value::Bool(false), Bool).bx())));
        assert!(!keeps_column(&Expr::And(c_gt.clone().bx(), lit(Value::Bool(false), Bool).bx())));
        // c IS NOT NULL (c may be NOT NULL); (c > 1) IS NULL; (b = true) IS UNKNOWN
        assert!(!keeps_column(&Expr::IsNull(c.clone().bx(), true)));
        assert!(keeps_column(&Expr::IsNull(c_gt.clone().bx(), false)));
        assert!(!keeps_column(&Expr::Is(Expr::Cmp(CmpOp::Eq, b.clone().bx(), lit(Value::Bool(true), Bool).bx()).bx(), IsWhat::Unknown, true)));
        // c < NULL::int8
        assert!(!keeps_column(&Expr::Cmp(CmpOp::Lt, c.clone().bx(), Expr::null(Int8).bx())));
        // greatest('1'::int8, c); nullif(c, NULL::int8); nullif(NULL::int8, c)
        assert!(keeps_column(&Expr::Greatest(vec![lit(Value::Int(1), Int8), c.clone()])));
        assert!(keeps_column(&Expr::NullIf(c.clone().bx(), Expr::null(Int8).bx())));
        assert!(!keeps_column(&Expr::NullIf(Expr::null(Int8).bx(), c.clone().bx())));
    }
}
