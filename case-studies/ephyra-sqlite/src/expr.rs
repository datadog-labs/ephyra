// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A general SQL expression and query language over a database's tables,
//! for the SQLancer-style oracles (`tests/sqlancer.rs`). Separate from the
//! model's narrow predicates ([`crate::statements::Pred`]), which stay as
//! they are: these expressions are valid against the schema but need not be
//! predictable, since the oracles compare SQLite with itself.
//!
//! - [`Expr`]: columns ([`Col`]: a source of the expression's own query, or
//!   of a query around it in a correlated subquery); literals of every
//!   storage class; comparisons; `AND`, `OR`, `NOT`; `IS [NOT] NULL`; `IS
//!   [NOT]`; `[NOT] BETWEEN`; `[NOT] IN (list)`; `+ - * / %`; `||`; `[NOT]
//!   LIKE`, `[NOT] GLOB`; unary minus; simple and searched `CASE`; `CAST`
//!   to INTEGER, REAL, TEXT, BLOB, NUMERIC; `e COLLATE BINARY|NOCASE|RTRIM`
//!   (mostly of a column; never where an aggregate's value would depend on
//!   which of equal texts it picks, [`casts_binary`], and the oracles fold
//!   text where one is, `crate::sqlancer`); the deterministic functions of
//!   [`Func`]; aggregates (in aggregate queries); `[NOT] EXISTS (q)`, `e
//!   [NOT] IN (q)` and scalar subqueries `(q)`.
//! - JSON: the scalar JSON functions and their `jsonb` forms
//!   ([`Func::JSON`]), `->` and `->>`, and counts over `json_each` and
//!   `json_tree` (a subquery `(SELECT count(..) FROM json_each(j, p))`,
//!   [`Func::JsonEach`]). Their arguments raise no error on any row
//!   ([`json_args_ok`]): JSON where JSON is read (JSON text literals,
//!   drawn with the paths of their elements, JSON5 ones, JSON functions'
//!   values, a value `json_valid` guards), well-formed path literals,
//!   values that are no BLOB but JSONB ([`unblob`] guards a column), text
//!   labels. A `jsonb` function's value is only ever read by a JSON
//!   function ([`jsonb_read`]: one value has several JSONB encodings, which
//!   bytes SQLite writes depends on the rows a plan visits before); where
//!   any expression goes it is drawn as `json(..)` (`json_quote(..)` of
//!   `jsonb_extract`).
//! - Aggregates with an ORDER BY of their own ([`Ordered`], SQLite 3.44):
//!   `group_concat`, `string_agg`, `json_group_array`, `json_group_object`,
//!   whose values depend on the order of rows, ordered totally over what
//!   they render ([`ordered_ok`]), DISTINCT where that keeps them
//!   determined; and `count`, `sum`, `min`, `max`, whose values do not.
//!   Wherever an aggregate goes (items and HAVING of an aggregate query, a
//!   scalar subquery's item, a CTE's aggregate items); TLP partitions them
//!   as any nested aggregate (`tlp-having`, else none).
//! - Dates: `date`, `time`, `datetime`, `julianday`, `unixepoch`,
//!   `strftime` and `timediff` ([`Func::DATE`]) of time values (columns of
//!   dates the file generator draws, [`crate::files::draw_date`], time
//!   literals, other date calls, any expression) and modifiers of the
//!   deterministic subset ([`date_args_ok`]: never `now`, `localtime` or
//!   `utc`), anywhere an expression goes, and as the first term of an
//!   expression index (`date(c)`, `unixepoch(c)`, `strftime` of every
//!   conversion, `datetime`, `julianday` and `time` with modifiers,
//!   `timediff`, `format` of a value
//!   and its dates with every `printf` conversion).
//! - [`Query`]: `[WITH x0 AS (..), ..] SELECT [DISTINCT] <exprs | * |
//!   aggregate> FROM t{k} AS a0 [[NATURAL] [LEFT | RIGHT | FULL | CROSS]
//!   JOIN t{j} AS a1 [ON e | USING (c, ..)]].. [WHERE e] [GROUP BY e, ..
//!   [HAVING e]] [UNION [ALL] | INTERSECT | EXCEPT SELECT ..].. [ORDER BY o,
//!   .. [LIMIT n [OFFSET k]]]`: up to five sources ([`MAX_SOURCES`])
//!   (self-joins included), each aliased by its position and nesting level
//!   ([`alias`]). ORDER BY only at the top level and only a total order
//!   ([`Query::total_order`]: by every result column, or by every source's
//!   unique key), so that LIMIT picks determined rows (without one, any of
//!   the rows that tie). Compound members ([`SetOp`]) and common table
//!   expressions ([`Cte`]: a query, or a recursive count, read as source
//!   `x{i}` from [`CTE`] on) only at the top level. [`Query::valid_in`]
//!   lists what a valid query is, including the
//!   rules that keep results determined by the data alone (no bare column
//!   in an aggregate query, aggregates of equal-class-determined values,
//!   scalar subqueries of one row).
//!
//! Every compound expression is rendered in parentheses, so precedence
//! never matters, and a unary minus is followed by a space (`--` starts a
//! comment). Literals are [`crate::statements::literal`]: SQLite reads each
//! back as exactly the value.
//!
//! The generator ([`Queries`], [`queries`]) never runs SQLite. Drawing
//! keeps every query valid by construction (each subquery drawn in the
//! scope it is in); shrinking proposes candidates of strictly lower
//! [`Query::cost`] and keeps the valid ones; a smaller schema repairs
//! queries ([`Query::repair`]: sources whose table is gone dropped,
//! columns out of range NULL).

use std::{fmt, ops::ControlFlow};

use ephyra::{Gen, Source};

use crate::{
    files::draw_date,
    model::{affinity, column_coll, column_value, numeric_text, Affinity, Coll, Db, Key, KeyExpr, Model, Numeric, Value},
    statements::{literal, CmpOp, Pred},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Is,
    IsNot,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Concat,
    Like,
    NotLike,
    Glob,
    NotGlob,
    /// `j -> p`: the JSON text of the element of JSON `j` at path `p`
    /// (a path, a label or an array index; see [`arrow_ok`]).
    Arrow,
    /// `j ->> p`: that element as an SQL value.
    Arrow2,
}

impl BinOp {
    pub const CMP: [BinOp; 6] = [BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge];
    pub const ARITH: [BinOp; 5] = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem];

    pub fn sql(self) -> &'static str {
        match self {
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Is => "IS",
            BinOp::IsNot => "IS NOT",
            BinOp::And => "AND",
            BinOp::Or => "OR",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Concat => "||",
            BinOp::Like => "LIKE",
            BinOp::NotLike => "NOT LIKE",
            BinOp::Glob => "GLOB",
            BinOp::NotGlob => "NOT GLOB",
            BinOp::Arrow => "->",
            BinOp::Arrow2 => "->>",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CastType {
    Integer,
    Real,
    Text,
    Blob,
    Numeric,
}

impl CastType {
    pub const ALL: [CastType; 5] = [CastType::Integer, CastType::Real, CastType::Text, CastType::Blob, CastType::Numeric];

    pub fn sql(self) -> &'static str {
        match self {
            CastType::Integer => "INTEGER",
            CastType::Real => "REAL",
            CastType::Text => "TEXT",
            CastType::Blob => "BLOB",
            CastType::Numeric => "NUMERIC",
        }
    }

    /// Type names of the same affinity (datatype3.html section 3.1): the
    /// rules by `INT`, `CHAR`, `CLOB`, `TEXT`, `BLOB`, `REAL`, `FLOA`,
    /// `DOUB`, else NUMERIC; sizes, several words, lower case.
    pub fn synonyms(self) -> &'static [&'static str] {
        match self {
            CastType::Integer => &["INT", "BIGINT", "UNSIGNED BIG INT", "INT8", "FLOATING POINT", "integer", "SMALLINT(4)"],
            CastType::Real => &["DOUBLE", "DOUBLE PRECISION", "FLOAT", "real", "FLOAT(53)"],
            CastType::Text => &["VARCHAR(255)", "CLOB", "NCHAR(10)", "NATIVE CHARACTER(70)", "text", "VARYING CHARACTER(5)"],
            CastType::Blob => &["blob", "BLOB(8)"],
            CastType::Numeric => &["DECIMAL(10,5)", "BOOLEAN", "DATE", "DATETIME", "NUMBER", "STRING", "numeric", "ANY"],
        }
    }
}

/// Deterministic scalar functions. `min` and `max` take two or more
/// arguments: with one they are the aggregates. Those of [`Func::ALL`]
/// take any arguments; the date and time functions ([`Func::DATE`]) a
/// time value and modifiers of their deterministic subset
/// ([`date_args_ok`]); the JSON functions ([`Func::JSON`]) take JSON,
/// paths and values where they need them ([`json_args_ok`]), and the
/// `jsonb` ones return JSONB blobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Func {
    Abs,
    Length,
    Lower,
    Upper,
    Coalesce,
    Nullif,
    Ifnull,
    Typeof,
    Substr,
    Round,
    Min,
    Max,
    Instr,
    Hex,
    Unicode,
    Iif,
    /// The text functions of [`Func::TEXT`], drawn by [`draw_text_call`].
    /// `trim(x[, set])`, `ltrim`, `rtrim`.
    Trim,
    Ltrim,
    Rtrim,
    /// `replace(x, pattern, with)`.
    Replace,
    /// `char(code, ..)`: out-of-range codes become U+FFFD.
    Char,
    /// `concat(x, ..)`, `concat_ws(sep, x, ..)`: NULL arguments skipped.
    Concat,
    ConcatWs,
    /// `unhex(x[, ignore])`: a BLOB, or NULL for text that is no hex.
    Unhex,
    /// `unistr(x)` of an argument without an invalid escape ([`unistr_ok`]).
    Unistr,
    Soundex,
    Sign,
    OctetLength,
    /// `zeroblob(n)`, `n` bounded ([`zeroblob_ok`]).
    Zeroblob,
    Quote,
    /// `likely(x)`, `unlikely(x)`, `likelihood(x, p)` (`p` a REAL literal
    /// in 0..=1): `x`, planner hints only.
    Likely,
    Unlikely,
    Likelihood,
    /// `date(t, m, ..)`: a time value `t`, then modifiers.
    Date,
    Time,
    Datetime,
    Julianday,
    Unixepoch,
    /// `strftime(f, t, m, ..)`: a format, a time value, modifiers.
    Strftime,
    /// `timediff(a, b)`: two time values.
    Timediff,
    /// `printf(f, ..)` and `format(f, ..)`: a format literal of bounded
    /// widths ([`format_ok`]), then values.
    Printf,
    Format,
    Json,
    Jsonb,
    JsonValid,
    JsonErrorPosition,
    JsonType,
    JsonArrayLength,
    JsonExtract,
    JsonbExtract,
    JsonQuote,
    JsonArray,
    JsonbArray,
    JsonObject,
    JsonbObject,
    JsonInsert,
    JsonbInsert,
    JsonReplace,
    JsonbReplace,
    JsonSet,
    JsonbSet,
    JsonArrayInsert,
    JsonbArrayInsert,
    JsonRemove,
    JsonbRemove,
    JsonPatch,
    JsonbPatch,
    JsonPretty,
    /// `(SELECT count(*) FROM json_each(j[, p]))`, or `count(DISTINCT c)`
    /// of one of its columns `c` (the first argument, [`JSON_EACH_COLUMNS`]).
    JsonEach,
    /// The same of `json_tree`, every element below `j` (or `p`).
    JsonTree,
}

impl Func {
    pub const ALL: [Func; 16] = [
        Func::Abs,
        Func::Length,
        Func::Lower,
        Func::Upper,
        Func::Coalesce,
        Func::Nullif,
        Func::Ifnull,
        Func::Typeof,
        Func::Substr,
        Func::Round,
        Func::Min,
        Func::Max,
        Func::Instr,
        Func::Hex,
        Func::Unicode,
        Func::Iif,
    ];

    pub const TEXT: [Func; 17] = [
        Func::Trim,
        Func::Ltrim,
        Func::Rtrim,
        Func::Replace,
        Func::Char,
        Func::Concat,
        Func::ConcatWs,
        Func::Unhex,
        Func::Unistr,
        Func::Soundex,
        Func::Sign,
        Func::OctetLength,
        Func::Zeroblob,
        Func::Quote,
        Func::Likely,
        Func::Unlikely,
        Func::Likelihood,
    ];

    pub const DATE: [Func; 7] =
        [Func::Date, Func::Time, Func::Datetime, Func::Julianday, Func::Unixepoch, Func::Strftime, Func::Timediff];

    pub const PRINTF: [Func; 2] = [Func::Printf, Func::Format];

    pub const JSON: [Func; 28] = [
        Func::Json,
        Func::Jsonb,
        Func::JsonValid,
        Func::JsonErrorPosition,
        Func::JsonType,
        Func::JsonArrayLength,
        Func::JsonExtract,
        Func::JsonbExtract,
        Func::JsonQuote,
        Func::JsonArray,
        Func::JsonbArray,
        Func::JsonObject,
        Func::JsonbObject,
        Func::JsonInsert,
        Func::JsonbInsert,
        Func::JsonReplace,
        Func::JsonbReplace,
        Func::JsonSet,
        Func::JsonbSet,
        Func::JsonArrayInsert,
        Func::JsonbArrayInsert,
        Func::JsonRemove,
        Func::JsonbRemove,
        Func::JsonPatch,
        Func::JsonbPatch,
        Func::JsonPretty,
        Func::JsonEach,
        Func::JsonTree,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Func::Abs => "abs",
            Func::Length => "length",
            Func::Lower => "lower",
            Func::Upper => "upper",
            Func::Coalesce => "coalesce",
            Func::Nullif => "nullif",
            Func::Ifnull => "ifnull",
            Func::Typeof => "typeof",
            Func::Substr => "substr",
            Func::Round => "round",
            Func::Min => "min",
            Func::Max => "max",
            Func::Instr => "instr",
            Func::Hex => "hex",
            Func::Unicode => "unicode",
            Func::Iif => "iif",
            Func::Trim => "trim",
            Func::Ltrim => "ltrim",
            Func::Rtrim => "rtrim",
            Func::Replace => "replace",
            Func::Char => "char",
            Func::Concat => "concat",
            Func::ConcatWs => "concat_ws",
            Func::Unhex => "unhex",
            Func::Unistr => "unistr",
            Func::Soundex => "soundex",
            Func::Sign => "sign",
            Func::OctetLength => "octet_length",
            Func::Zeroblob => "zeroblob",
            Func::Quote => "quote",
            Func::Likely => "likely",
            Func::Unlikely => "unlikely",
            Func::Likelihood => "likelihood",
            Func::Date => "date",
            Func::Time => "time",
            Func::Datetime => "datetime",
            Func::Julianday => "julianday",
            Func::Unixepoch => "unixepoch",
            Func::Strftime => "strftime",
            Func::Timediff => "timediff",
            Func::Printf => "printf",
            Func::Format => "format",
            Func::Json => "json",
            Func::Jsonb => "jsonb",
            Func::JsonValid => "json_valid",
            Func::JsonErrorPosition => "json_error_position",
            Func::JsonType => "json_type",
            Func::JsonArrayLength => "json_array_length",
            Func::JsonExtract => "json_extract",
            Func::JsonbExtract => "jsonb_extract",
            Func::JsonQuote => "json_quote",
            Func::JsonArray => "json_array",
            Func::JsonbArray => "jsonb_array",
            Func::JsonObject => "json_object",
            Func::JsonbObject => "jsonb_object",
            Func::JsonInsert => "json_insert",
            Func::JsonbInsert => "jsonb_insert",
            Func::JsonReplace => "json_replace",
            Func::JsonbReplace => "jsonb_replace",
            Func::JsonSet => "json_set",
            Func::JsonbSet => "jsonb_set",
            Func::JsonArrayInsert => "json_array_insert",
            Func::JsonbArrayInsert => "jsonb_array_insert",
            Func::JsonRemove => "json_remove",
            Func::JsonbRemove => "jsonb_remove",
            Func::JsonPatch => "json_patch",
            Func::JsonbPatch => "jsonb_patch",
            Func::JsonPretty => "json_pretty",
            Func::JsonEach => "json_each",
            Func::JsonTree => "json_tree",
        }
    }

    /// The argument counts generated (a subset of what SQLite accepts).
    /// For the JSON functions [`json_args_ok`] says more (the edits take
    /// path and value pairs, `json_object` label and value pairs).
    pub fn arity(self) -> (usize, usize) {
        match self {
            Func::Abs | Func::Length | Func::Lower | Func::Upper | Func::Typeof | Func::Hex | Func::Unicode => (1, 1),
            Func::Nullif | Func::Ifnull | Func::Instr => (2, 2),
            Func::Coalesce => (2, 4),
            Func::Min | Func::Max => (2, 3),
            Func::Substr => (2, 3),
            Func::Round => (1, 2),
            Func::Iif => (3, 3),
            Func::Trim | Func::Ltrim | Func::Rtrim | Func::Unhex => (1, 2),
            Func::Replace => (3, 3),
            Func::Char | Func::Concat => (1, 4),
            Func::ConcatWs => (2, 15),
            Func::Unistr | Func::Soundex | Func::Sign | Func::OctetLength | Func::Zeroblob | Func::Quote | Func::Likely | Func::Unlikely => (1, 1),
            Func::Likelihood => (2, 2),
            Func::Date | Func::Time | Func::Datetime | Func::Julianday | Func::Unixepoch => (1, 11),
            Func::Strftime => (2, 12),
            Func::Timediff => (2, 2),
            Func::Printf | Func::Format => (1, 32),
            Func::Json | Func::Jsonb | Func::JsonErrorPosition | Func::JsonQuote => (1, 1),
            Func::JsonValid | Func::JsonType | Func::JsonArrayLength | Func::JsonPretty => (1, 2),
            Func::JsonExtract | Func::JsonbExtract => (2, 4),
            Func::JsonArray | Func::JsonbArray | Func::JsonObject | Func::JsonbObject => (0, 4),
            Func::JsonInsert
            | Func::JsonbInsert
            | Func::JsonReplace
            | Func::JsonbReplace
            | Func::JsonSet
            | Func::JsonbSet
            | Func::JsonArrayInsert
            | Func::JsonbArrayInsert => (3, 5),
            Func::JsonRemove | Func::JsonbRemove => (2, 3),
            Func::JsonPatch | Func::JsonbPatch => (2, 2),
            Func::JsonEach | Func::JsonTree => (2, 3),
        }
    }

    /// One of [`Func::JSON`].
    pub fn json(self) -> bool {
        self >= Func::Json
    }

    /// One of [`Func::DATE`].
    pub fn date(self) -> bool {
        (Func::Date..=Func::Timediff).contains(&self)
    }

    /// The position of the time value of a date and time function (its
    /// first argument, `strftime`'s second): the arguments after it are
    /// modifiers ([`date_args_ok`]).
    pub fn time_at(self) -> usize {
        usize::from(self == Func::Strftime)
    }

    /// A `jsonb` function: its value is a JSONB blob (or, for
    /// `jsonb_extract` of a scalar, an SQL value, or NULL).
    pub fn jsonb(self) -> bool {
        matches!(
            self,
            Func::Jsonb
                | Func::JsonbExtract
                | Func::JsonbArray
                | Func::JsonbObject
                | Func::JsonbInsert
                | Func::JsonbReplace
                | Func::JsonbSet
                | Func::JsonbArrayInsert
                | Func::JsonbRemove
                | Func::JsonbPatch
        )
    }

    /// Its value is always well-formed JSON (text, or a JSONB blob), or
    /// NULL; `json_extract` of two or more paths too (an array of their
    /// values), not of one (that is the element as an SQL value).
    pub fn makes_json(self) -> bool {
        self.json()
            && !matches!(
                self,
                Func::JsonValid
                    | Func::JsonErrorPosition
                    | Func::JsonType
                    | Func::JsonArrayLength
                    | Func::JsonExtract
                    | Func::JsonbExtract
                    | Func::JsonEach
                    | Func::JsonTree
            )
    }

    /// The `jsonb` form of a JSON function that has one, else itself.
    pub fn binary(self) -> Func {
        match self {
            Func::Json => Func::Jsonb,
            Func::JsonExtract => Func::JsonbExtract,
            Func::JsonArray => Func::JsonbArray,
            Func::JsonObject => Func::JsonbObject,
            Func::JsonInsert => Func::JsonbInsert,
            Func::JsonReplace => Func::JsonbReplace,
            Func::JsonSet => Func::JsonbSet,
            Func::JsonArrayInsert => Func::JsonbArrayInsert,
            Func::JsonRemove => Func::JsonbRemove,
            Func::JsonPatch => Func::JsonbPatch,
            f => f,
        }
    }
}

/// A column reference: column `col` of source `src` (in FROM order) of the
/// query `up` levels out from the expression's own query (0: its own; 1:
/// the query a subquery is in, a correlated reference).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Col {
    pub up: usize,
    pub src: usize,
    pub col: usize,
}

impl Col {
    /// Column `col` of source `src` of the expression's own query.
    pub fn here(src: usize, col: usize) -> Col {
        Col { up: 0, src, col }
    }
}

#[derive(Clone, PartialEq)]
pub enum Expr {
    Col(Col),
    Lit(Value),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// `e IS NULL`, `e IS NOT NULL` when `true`.
    IsNull(Box<Expr>, bool),
    /// `e BETWEEN lo AND hi`, `NOT BETWEEN` when `true`.
    Between(Box<Expr>, Box<Expr>, Box<Expr>, bool),
    /// `e IN (list)`, non-empty list; `NOT IN` when `true`.
    In(Box<Expr>, Vec<Expr>, bool),
    /// A row value comparison `(a, b, ..) op (x, y, ..)`: two or three
    /// terms on each side, `op` one of [`BinOp::CMP`], IS or IS NOT
    /// (lang_rowvalue.html: compared left to right, as ORDER BY does).
    RowCmp(BinOp, Vec<Expr>, Vec<Expr>),
    /// `(a, b, ..) [NOT] BETWEEN (lo, ..) AND (hi, ..)`.
    RowBetween(Vec<Expr>, Vec<Expr>, Vec<Expr>, bool),
    /// `(a, b, ..) [NOT] IN ((x, y, ..), ..)`, two rows or more (SQLite
    /// reads the list as VALUES, `sqlite3ExprListToValues`).
    RowIn(Vec<Expr>, Vec<Vec<Expr>>, bool),
    /// `(a, b, ..) [NOT] IN (q)`, `q` of as many columns ([`Kind::Row`]).
    RowInSelect(Vec<Expr>, Box<Query>, bool),
    /// `CASE [operand] WHEN a THEN b .. [ELSE e] END`, at least one WHEN.
    Case(Option<Box<Expr>>, Vec<(Expr, Expr)>, Option<Box<Expr>>),
    Cast(Box<Expr>, CastType),
    /// `e COLLATE x`: `e` under collating sequence `x`, which an
    /// expression around it takes ([`expr_coll`]); never where the oracles
    /// rely on equal values being identical ([`casts_binary`]).
    Collate(Box<Expr>, Coll),
    Call(Func, Vec<Expr>),
    /// An aggregate of the expression's own query: `count(*)` is `(Count,
    /// None)`. Only in the items and HAVING of an aggregate query, with an
    /// argument of its own query's rows whose value is determined by its
    /// equivalence class (see [`Query::valid_in`]).
    Agg(Agg, Option<Box<Expr>>),
    /// An aggregate with an ORDER BY of its own ([`Ordered`]), where
    /// [`Expr::Agg`] may be.
    Ordered(Box<Ordered>),
    /// `EXISTS (q)`, `NOT EXISTS` when `true`.
    Exists(Box<Query>, bool),
    /// `e IN (q)`, `q` of one column; `NOT IN` when `true`.
    InSelect(Box<Expr>, Box<Query>, bool),
    /// `(q)`: a scalar subquery, one aggregate column and no GROUP BY, so
    /// exactly one row.
    Scalar(Box<Query>),
    /// A window function, only as an item of a top-level query that does
    /// not aggregate (see [`Window`]).
    Window(Box<Window>),
}

fn bx(e: Expr) -> Box<Expr> {
    Box::new(e)
}

/// The column number of a source's rowid in a [`Col`]: `a0.rowid` (in a
/// window's ORDER BY, which orders by every source's key).
pub const ROWID: usize = 1 << 16;

/// A window function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WinFunc {
    RowNumber,
    Rank,
    DenseRank,
    PercentRank,
    CumeDist,
    /// `ntile(n)`, n from 1.
    Ntile(u8),
    FirstValue,
    LastValue,
    /// `nth_value(x, n)`, n from 1.
    NthValue(u8),
    /// `lag(x, n [, default])`, n from 1 (`lag(x)` for 1 without a default).
    Lag(u8),
    Lead(u8),
    Count,
    Sum,
    Min,
    Max,
    /// `avg` and `total` of a small integer ([`small_int`]).
    Avg,
    Total,
    /// `group_concat(x [, sep])`, the separator [`GROUP_SEPS`]`[k]` (the
    /// default `,` for 0).
    GroupConcat(u8),
    /// `json_group_array(CAST(x AS INTEGER))`.
    JsonGroupArray,
}

/// The separators of a `group_concat` window ([`WinFunc::GroupConcat`]).
pub const GROUP_SEPS: [&str; 3] = [",", "", " | "];

impl WinFunc {
    pub const ALL: [WinFunc; 19] = [
        WinFunc::RowNumber,
        WinFunc::Rank,
        WinFunc::DenseRank,
        WinFunc::PercentRank,
        WinFunc::CumeDist,
        WinFunc::Ntile(3),
        WinFunc::FirstValue,
        WinFunc::LastValue,
        WinFunc::NthValue(2),
        WinFunc::Lag(1),
        WinFunc::Lead(1),
        WinFunc::Count,
        WinFunc::Sum,
        WinFunc::Min,
        WinFunc::Max,
        WinFunc::Avg,
        WinFunc::Total,
        WinFunc::GroupConcat(0),
        WinFunc::JsonGroupArray,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WinFunc::RowNumber => "row_number",
            WinFunc::Rank => "rank",
            WinFunc::DenseRank => "dense_rank",
            WinFunc::PercentRank => "percent_rank",
            WinFunc::CumeDist => "cume_dist",
            WinFunc::Ntile(_) => "ntile",
            WinFunc::FirstValue => "first_value",
            WinFunc::LastValue => "last_value",
            WinFunc::NthValue(_) => "nth_value",
            WinFunc::Lag(_) => "lag",
            WinFunc::Lead(_) => "lead",
            WinFunc::Count => "count",
            WinFunc::Sum => "sum",
            WinFunc::Min => "min",
            WinFunc::Max => "max",
            WinFunc::Avg => "avg",
            WinFunc::Total => "total",
            WinFunc::GroupConcat(_) => "group_concat",
            WinFunc::JsonGroupArray => "json_group_array",
        }
    }

    /// An aggregate over a frame (the others rank rows or read one row,
    /// and take no FILTER).
    pub fn aggregate(self) -> bool {
        matches!(
            self,
            WinFunc::Count | WinFunc::Sum | WinFunc::Min | WinFunc::Max | WinFunc::Avg | WinFunc::Total | WinFunc::GroupConcat(_) | WinFunc::JsonGroupArray
        )
    }

    /// Whether it reads a frame: the aggregates and `first_value`,
    /// `last_value`, `nth_value` (the others ignore one).
    pub fn framed(self) -> bool {
        self.aggregate() || matches!(self, WinFunc::FirstValue | WinFunc::LastValue | WinFunc::NthValue(_))
    }

    /// Whether its value depends on the order of rows within peers, so
    /// that it needs a total order ([`keyed`]): `row_number`, `ntile`, the
    /// functions reading one row of the frame or partition, and the
    /// aggregates that concatenate.
    pub fn needs_total(self) -> bool {
        matches!(
            self,
            WinFunc::RowNumber
                | WinFunc::Ntile(_)
                | WinFunc::FirstValue
                | WinFunc::LastValue
                | WinFunc::NthValue(_)
                | WinFunc::Lag(_)
                | WinFunc::Lead(_)
                | WinFunc::GroupConcat(_)
                | WinFunc::JsonGroupArray
        )
    }
}

/// How a window is written: inline (`OVER (..)`), or through a named
/// window of the query's WINDOW clause named by its definition's text
/// ([`window_name`]): `OVER w` (all of it named), `OVER (w ORDER BY ..
/// frame)` (its PARTITION BY named), `OVER (w frame)` (its PARTITION BY and
/// ORDER BY named).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Named {
    #[default]
    Inline,
    All,
    Partition,
    PartitionOrder,
}

/// How a frame counts: rows, values (peers), or groups of peers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameMode {
    Rows,
    Range,
    Groups,
}

/// A frame bound; offsets are rows (or groups) away from the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    UnboundedPreceding,
    Preceding(u8),
    CurrentRow,
    Following(u8),
    UnboundedFollowing,
}

impl Bound {
    /// Its kind's place in the order SQLite requires of a frame's start and
    /// end (a start of a later kind than the end is an error).
    pub fn rank(self) -> u8 {
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
            Bound::UnboundedPreceding => "UNBOUNDED PRECEDING".to_string(),
            Bound::Preceding(n) => format!("{n} PRECEDING"),
            Bound::CurrentRow => "CURRENT ROW".to_string(),
            Bound::Following(n) => format!("{n} FOLLOWING"),
            Bound::UnboundedFollowing => "UNBOUNDED FOLLOWING".to_string(),
        }
    }
}

/// `EXCLUDE ..`: rows left out of every frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exclude {
    NoOthers,
    CurrentRow,
    Group,
    Ties,
}

/// `ROWS|RANGE|GROUPS BETWEEN start AND end [EXCLUDE ..]`, or `ROWS|..
/// start` (`short`, when `end` is the current row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub mode: FrameMode,
    pub start: Bound,
    pub end: Bound,
    pub exclude: Option<Exclude>,
    pub short: bool,
}

impl Frame {
    /// A frame SQLite accepts: the start of no later kind than the end
    /// ([`Bound::rank`]), neither unbounded the wrong way; RANGE with
    /// offsets only when `range_offsets` (an offset needs one numeric ORDER
    /// BY term: the window's is an integer, [`int_key`]); offsets up to 5
    /// (100 for RANGE, whose offsets are values); the short form only to
    /// the current row.
    pub fn ok(&self, range_offsets: bool) -> bool {
        let most = if self.mode == FrameMode::Range { 100 } else { 5 };
        let offset = |b: Bound| matches!(b, Bound::Preceding(n) | Bound::Following(n) if n > most);
        self.start.rank() <= self.end.rank()
            && self.start != Bound::UnboundedFollowing
            && self.end != Bound::UnboundedPreceding
            && !offset(self.start)
            && !offset(self.end)
            && !(self.mode == FrameMode::Range && self.offsets() && !range_offsets)
            && (!self.short || self.end == Bound::CurrentRow)
    }

    /// Whether a bound is `n PRECEDING` or `n FOLLOWING`.
    pub fn offsets(&self) -> bool {
        [self.start, self.end].iter().any(|b| matches!(b, Bound::Preceding(_) | Bound::Following(_)))
    }

    fn sql(&self) -> String {
        let mode = match self.mode {
            FrameMode::Rows => "ROWS",
            FrameMode::Range => "RANGE",
            FrameMode::Groups => "GROUPS",
        };
        let bounds = if self.short { self.start.sql() } else { format!("BETWEEN {} AND {}", self.start.sql(), self.end.sql()) };
        let exclude = match self.exclude {
            None => "",
            Some(Exclude::NoOthers) => " EXCLUDE NO OTHERS",
            Some(Exclude::CurrentRow) => " EXCLUDE CURRENT ROW",
            Some(Exclude::Group) => " EXCLUDE GROUP",
            Some(Exclude::Ties) => " EXCLUDE TIES",
        };
        format!("{mode} {bounds}{exclude}")
    }
}

/// `func([arg | *] [, n] [, default]) [FILTER (WHERE f)] OVER ([PARTITION
/// BY e, ..] [ORDER BY e, ..] [frame])`, or through a named window
/// ([`Named`]). The argument as an aggregate's ([`Expr::Agg`]): any for
/// `count` (or `count(*)`), a `CAST` to INTEGER or TEXT for `min` and
/// `max`, to INTEGER for `sum` and `json_group_array`, a small integer for
/// `avg` and `total` ([`small_int`], so that the sum is exact and its real
/// does not depend on the order rows are added in); any row expression for
/// the functions that read one row (`first_value`, `last_value`,
/// `nth_value`, `lag`, `lead`, whose `default` is another); for
/// `group_concat` a column, a blob as its hex ([`unblob`]: valid text);
/// none for the ranking functions. FILTER only on an aggregate. The ORDER
/// BY terms are [`OrderKey::Expr`]s; the functions of
/// [`WinFunc::needs_total`] and a ROWS frame need them to order rows
/// totally (they end with every source's key: rowid as column [`ROWID`],
/// or the primary key's columns), so that the values are determined; the
/// others depend on peers only. A RANGE frame with offsets only over one
/// integer ORDER BY term ([`int_key`]).
#[derive(Clone, PartialEq)]
pub struct Window {
    pub func: WinFunc,
    pub arg: Option<Expr>,
    pub default: Option<Expr>,
    pub filter: Option<Expr>,
    pub partition: Vec<Expr>,
    pub order: Vec<Order>,
    pub frame: Option<Frame>,
    pub named: Named,
}

impl Window {
    /// The ORDER BY terms' expressions.
    pub fn keys(&self) -> Vec<&Expr> {
        self.order.iter().filter_map(|o| if let OrderKey::Expr(e) = &o.key { Some(e) } else { None }).collect()
    }

    /// The most rows its frame holds whatever the rows, for a function
    /// that reads a frame ([`WinFunc::framed`]): a ROWS frame between
    /// offsets and the current row, or a RANGE one over the rowid of the
    /// query's one source (`single`), whose values are distinct, holds as
    /// many rows as its bounds are apart; every other (none: RANGE from the
    /// partition's start to the current row's peers; GROUPS, RANGE over
    /// values that repeat, an UNBOUNDED bound) may hold the whole
    /// partition: `None`.
    pub fn frame_rows(&self, single: bool) -> Option<u64> {
        let f = self.frame?;
        let rowid = single && matches!(self.keys().as_slice(), [Expr::Col(Col { up: 0, src: 0, col: ROWID })]);
        if !(f.mode == FrameMode::Rows || f.mode == FrameMode::Range && rowid) {
            return None;
        }
        let at = |b: Bound| match b {
            Bound::Preceding(n) => Some(-i64::from(n)),
            Bound::CurrentRow => Some(0),
            Bound::Following(n) => Some(i64::from(n)),
            Bound::UnboundedPreceding | Bound::UnboundedFollowing => None,
        };
        Some((at(f.end)? - at(f.start)? + 1).max(0) as u64)
    }

    /// Whether it reads its frames within their budget over `rows` rows
    /// ([`FRAME_ROWS`], [`CONCAT_FRAME_ROWS`]): the rows times what its
    /// frame holds ([`Window::frame_rows`], else the rows). A function that
    /// reads no frame always does.
    pub fn frames_fit(&self, rows: u64, single: bool) -> bool {
        if !self.func.framed() {
            return true;
        }
        let read = rows.saturating_mul(self.frame_rows(single).unwrap_or(rows));
        let concat = matches!(self.func, WinFunc::GroupConcat(_) | WinFunc::JsonGroupArray);
        read <= if concat { CONCAT_FRAME_ROWS } else { FRAME_ROWS }
    }
}

impl Expr {
    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
        Expr::Bin(op, bx(a), bx(b))
    }

    /// A column of source `src` of the expression's own query.
    pub fn col(src: usize, col: usize) -> Expr {
        Expr::Col(Col::here(src, col))
    }

    /// The direct subexpressions, in rendering order; a subquery's are not
    /// among them (see [`Expr::subquery`]).
    pub fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Col(_) | Expr::Lit(_) | Expr::Exists(..) | Expr::Scalar(_) | Expr::Agg(_, None) => vec![],
            Expr::Ordered(o) => o.args.iter().chain(o.keys()).collect(),
            Expr::Not(a) | Expr::Neg(a) | Expr::IsNull(a, _) | Expr::Cast(a, _) | Expr::Collate(a, _) | Expr::InSelect(a, ..) => vec![a],
            Expr::Agg(_, Some(a)) => vec![a],
            Expr::Bin(_, a, b) => vec![a, b],
            Expr::Between(a, b, c, _) => vec![a, b, c],
            Expr::In(a, l, _) => std::iter::once(&**a).chain(l.iter()).collect(),
            Expr::RowCmp(_, a, b) => a.iter().chain(b.iter()).collect(),
            Expr::RowBetween(a, lo, hi, _) => a.iter().chain(lo.iter()).chain(hi.iter()).collect(),
            Expr::RowIn(a, l, _) => a.iter().chain(l.iter().flatten()).collect(),
            Expr::RowInSelect(a, ..) => a.iter().collect(),
            Expr::Case(op, ws, el) => {
                let mut v: Vec<&Expr> = op.iter().map(|b| &**b).collect();
                for (w, t) in ws {
                    v.push(w);
                    v.push(t);
                }
                v.extend(el.iter().map(|b| &**b));
                v
            },
            Expr::Call(_, args) => args.iter().collect(),
            Expr::Window(w) => w.arg.iter().chain(w.default.iter()).chain(w.filter.iter()).chain(w.partition.iter()).chain(w.keys()).collect(),
        }
    }

    pub fn children_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            Expr::Col(_) | Expr::Lit(_) | Expr::Exists(..) | Expr::Scalar(_) | Expr::Agg(_, None) => vec![],
            Expr::Ordered(o) => {
                let Ordered { args, order, .. } = &mut **o;
                let keys = order.iter_mut().filter_map(|o| if let OrderKey::Expr(e) = &mut o.key { Some(e) } else { None });
                args.iter_mut().chain(keys).collect()
            },
            Expr::Not(a) | Expr::Neg(a) | Expr::IsNull(a, _) | Expr::Cast(a, _) | Expr::Collate(a, _) | Expr::InSelect(a, ..) => vec![a],
            Expr::Agg(_, Some(a)) => vec![a],
            Expr::Bin(_, a, b) => vec![a, b],
            Expr::Between(a, b, c, _) => vec![a, b, c],
            Expr::In(a, l, _) => std::iter::once(&mut **a).chain(l.iter_mut()).collect(),
            Expr::RowCmp(_, a, b) => a.iter_mut().chain(b.iter_mut()).collect(),
            Expr::RowBetween(a, lo, hi, _) => a.iter_mut().chain(lo.iter_mut()).chain(hi.iter_mut()).collect(),
            Expr::RowIn(a, l, _) => a.iter_mut().chain(l.iter_mut().flatten()).collect(),
            Expr::RowInSelect(a, ..) => a.iter_mut().collect(),
            Expr::Case(op, ws, el) => {
                let mut v: Vec<&mut Expr> = op.iter_mut().map(|b| &mut **b).collect();
                for (w, t) in ws {
                    v.push(w);
                    v.push(t);
                }
                v.extend(el.iter_mut().map(|b| &mut **b));
                v
            },
            Expr::Call(_, args) => args.iter_mut().collect(),
            Expr::Window(w) => {
                let Window { arg, default, filter, partition, order, .. } = &mut **w;
                let keys = order.iter_mut().filter_map(|o| if let OrderKey::Expr(e) = &mut o.key { Some(e) } else { None });
                arg.iter_mut().chain(default.iter_mut()).chain(filter.iter_mut()).chain(partition.iter_mut()).chain(keys).collect()
            },
        }
    }

    /// The subquery of an EXISTS, IN or scalar subquery node.
    pub fn subquery(&self) -> Option<&Query> {
        match self {
            Expr::Exists(q, _) | Expr::InSelect(_, q, _) | Expr::Scalar(q) | Expr::RowInSelect(_, q, _) => Some(q),
            _ => None,
        }
    }

    pub fn subquery_mut(&mut self) -> Option<&mut Query> {
        match self {
            Expr::Exists(q, _) | Expr::InSelect(_, q, _) | Expr::Scalar(q) | Expr::RowInSelect(_, q, _) => Some(q),
            _ => None,
        }
    }

    /// Nesting depth: 1 for a leaf (a subquery counts as a leaf; its own
    /// expressions have their own depth).
    pub fn depth(&self) -> usize {
        1 + self.children().iter().map(|c| c.depth()).max().unwrap_or(0)
    }

    /// Nodes in the tree, subqueries' not counted.
    pub fn nodes(&self) -> usize {
        1 + self.children().iter().map(|c| c.nodes()).sum::<usize>()
    }

    /// Visit every node, preorder, subqueries' expressions included.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        for c in self.children() {
            c.walk(f);
        }
        if let Some(q) = self.subquery() {
            for e in q.exprs() {
                e.walk(f);
            }
        }
    }

    /// The simplicity measure every shrink step strictly decreases: a
    /// literal NULL is 1, a column 2 or more, every compound node 1 more
    /// than its parts, a subquery node 2 more than its query's cost.
    pub fn cost(&self) -> u64 {
        let own = match self {
            Expr::Col(c) => return 2 + (c.up + c.src + c.col.min(ROWID - 1)) as u64,
            Expr::Lit(v) => return 1 + v.cost(),
            Expr::IsNull(_, not) | Expr::Between(.., not) | Expr::In(_, _, not) => 1 + u64::from(*not),
            Expr::Case(op, ws, el) => 1 + u64::from(op.is_some()) + ws.len() as u64 + u64::from(el.is_some()),
            Expr::Agg(a, _) => 1 + u64::from(*a != Agg::Count),
            Expr::Ordered(o) => {
                let order: u64 = o.order.iter().map(|t| 1 + u64::from(t.desc) + u64::from(t.nulls.is_some())).sum();
                2 + o.func as u64 + u64::from(o.distinct) + order
            },
            Expr::Exists(q, not) | Expr::InSelect(_, q, not) => 2 + u64::from(*not) + q.cost(),
            Expr::RowInSelect(_, q, not) => 3 + u64::from(*not) + q.cost(),
            Expr::RowCmp(..) => 2,
            Expr::RowBetween(.., not) | Expr::RowIn(.., not) => 2 + u64::from(*not),
            Expr::Scalar(q) => 2 + q.cost(),
            Expr::Window(w) => {
                let frame = w.frame.map_or(0, |f| 1 + u64::from(f.exclude.is_some()) + u64::from(f.short));
                let order: u64 = w.order.iter().map(|o| 1 + u64::from(o.desc) + u64::from(o.nulls.is_some())).sum();
                let named = u64::from(w.named != Named::Inline);
                2 + u64::from(w.func != WinFunc::Count) + w.partition.len() as u64 + order + frame + named
            },
            _ => 1,
        };
        own + self.children().iter().map(|c| c.cost()).sum::<u64>()
    }

    /// Whether an aggregate of the expression's own query is in it (not
    /// one of a subquery).
    pub fn has_agg(&self) -> bool {
        matches!(self, Expr::Agg(..) | Expr::Ordered(_)) || self.children().iter().any(|c| c.has_agg())
    }

    /// Whether the expression refers to a column of the query `n` levels
    /// out (subqueries included, one level further out from inside them).
    pub fn refers(&self, n: usize) -> bool {
        match self {
            Expr::Col(c) => c.up == n,
            _ => self.children().iter().any(|c| c.refers(n)) || self.subquery().is_some_and(|q| q.refers(n + 1)),
        }
    }

    /// Every column reference to the query `n` levels out mapped by `f`
    /// (to NULL where `f` gives `None`), in subqueries too.
    pub fn map_level(&self, n: usize, f: &dyn Fn(Col) -> Option<Col>) -> Expr {
        match self {
            Expr::Col(c) if c.up == n => f(*c).map_or(Expr::Lit(Value::Null), Expr::Col),
            _ => {
                let mut e = self.clone();
                for c in e.children_mut() {
                    *c = c.map_level(n, f);
                }
                if let Some(q) = e.subquery_mut() {
                    q.map_level(n + 1, f);
                }
                e
            },
        }
    }
}

/// The deepest expression generated or accepted; SQLite allows 1000.
pub const MAX_DEPTH: usize = 12;

/// A literal SQL can carry exactly: text without NUL, a real that is not
/// NaN.
pub fn value_ok(v: &Value) -> bool {
    match v {
        Value::Text(s) => !s.contains('\0'),
        Value::Real(x) => !x.is_nan(),
        _ => true,
    }
}

/// The deepest nesting of queries: the top level and two levels of
/// subqueries.
pub const MAX_LEVELS: usize = 3;

/// The alias prefix of the sources of a query at each nesting level:
/// `a0, a1, ..` at the top, `b0, ..` in its subqueries, `d0, ..` in
/// theirs. Distinct per level, so a correlated reference names its query.
const ALIASES: [&str; MAX_LEVELS] = ["a", "b", "d"];

/// The alias of source `src` of a query at nesting `depth`.
pub fn alias(depth: usize, src: usize) -> String {
    format!("{}{src}", ALIASES[depth.min(MAX_LEVELS - 1)])
}

/// Where an expression is rendered: the nesting depth of its query (0 at
/// the top) and how many sources that query has. A column of the
/// expression's own query is written bare (`c3`) when that query has one
/// source (SQLite resolves a bare name in the innermost query that has
/// it, and that source has the column); every other column is qualified
/// by its source's alias (`a1.c3`, `a0.c2` from a subquery).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct At {
    pub depth: usize,
    pub sources: usize,
    /// Subqueries rendered with `NOT INDEXED` on every source.
    pub noindex: bool,
    /// Common table expressions referred to by a marker, for
    /// [`Query::sql_inline`] to put each one's query in place of.
    pub inline: bool,
    /// Items named `c0, c1, ..` (`e AS c0`): the first SELECT of a common
    /// table expression's query.
    pub named: bool,
    /// The sources of the query (one bit each, the first eight) that are
    /// tables of the database, whose columns may be written with the
    /// schema too (`main.a0.c0`).
    pub main: u8,
    /// Of those, the ones in TEMP and in `aux` (the rest are in main):
    /// `temp.a0.c0`, `aux.a1.c2`.
    pub temp: u8,
    pub aux: u8,
}

impl At {
    pub const TOP: At = At { depth: 0, sources: 1, noindex: false, inline: false, named: false, main: 0, temp: 0, aux: 0 };

    /// The schema of source `k`'s table, as a prefix: `main.`, `temp.`,
    /// `aux.`.
    fn db(self, k: usize) -> &'static str {
        if self.aux >> k & 1 == 1 {
            "aux."
        } else if self.temp >> k & 1 == 1 {
            "temp."
        } else {
            "main."
        }
    }

    /// `self` with the flags that subqueries and compound members keep
    /// (`noindex`, `inline`) of `outer`.
    fn within(self, outer: At) -> At {
        At { noindex: outer.noindex, inline: outer.inline, named: false, ..self }
    }
}

/// An expression rendered at a place ([`At`]).
pub struct Sql<'a>(pub &'a Expr, pub At);

fn list(f: &mut fmt::Formatter, items: &[Expr], at: At) -> fmt::Result {
    for (k, e) in items.iter().enumerate() {
        if k > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", Sql(e, at))?;
    }
    Ok(())
}

impl fmt::Display for Sql<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let at = self.1;
        let s = |e: &Expr| Sql(e, at).to_string();
        let not = |b: bool| if b { "NOT " } else { "" };
        let sub = |q: &Query| {
            let inner = q.at(at.depth + 1).within(at);
            let hints = if at.noindex { vec![" NOT INDEXED".to_string(); inner.sources] } else { Vec::new() };
            q.render_at(inner, &[hints])
        };
        match self.0 {
            Expr::Col(c) if c.col == ROWID && c.up == 0 && at.sources == 1 => write!(f, "rowid"),
            Expr::Col(c) if c.col == ROWID => write!(f, "{}.rowid", alias(at.depth.saturating_sub(c.up), c.src)),
            Expr::Col(c) if c.up == 0 && at.sources == 1 => write!(f, "{}", quoted(&format!("c{}", c.col))),
            Expr::Col(c) => {
                let a = alias(at.depth.saturating_sub(c.up), c.src);
                let name = quoted(&format!("c{}", c.col));
                // Of a table of this query's, with the schema an eighth of
                // the time: `lookupName` matches the source's schema.
                let schema = if c.up == 0 && c.src < 8 && at.main >> c.src & 1 == 1 && spell(&format!("{a}.{name}"), 8) == 0 { at.db(c.src) } else { "" };
                write!(f, "{schema}{a}.{name}")
            },
            Expr::Lit(v) => write!(f, "{}", lit_sql(v)),
            Expr::Not(a) => write!(f, "(NOT {})", s(a)),
            Expr::Neg(a) => write!(f, "(- {})", s(a)),
            Expr::Bin(op @ (BinOp::Like | BinOp::NotLike), a, b) if matches!(&**b, Expr::Lit(Value::Text(p)) if spell(p, 3) == 0) => {
                let Expr::Lit(Value::Text(p)) = &**b else { unreachable!() };
                write!(f, "({} {} {} ESCAPE '!')", s(a), op.sql(), literal(&Value::Text(escaped(p))))
            },
            Expr::Bin(op, a, b) => {
                let (a, b) = (s(a), s(b));
                write!(f, "({a} {} {b})", op_sql(*op, &a))
            },
            Expr::IsNull(a, n) => {
                let a = s(a);
                let op = match (n, spell(&a, 6)) {
                    (false, 0 | 1) => "ISNULL",
                    (false, _) => "IS NULL",
                    (true, 0) => "NOTNULL",
                    (true, 1) => "NOT NULL",
                    (true, _) => "IS NOT NULL",
                };
                write!(f, "({a} {op})")
            },
            Expr::Between(a, lo, hi, n) => write!(f, "({} {}BETWEEN {} AND {})", s(a), not(*n), s(lo), s(hi)),
            Expr::In(a, l, n) => {
                write!(f, "({} {}IN (", s(a), not(*n))?;
                list(f, l, at)?;
                write!(f, "))")
            },
            Expr::Case(op, ws, el) => {
                write!(f, "(CASE")?;
                if let Some(op) = op {
                    write!(f, " {}", s(op))?;
                }
                for (w, t) in ws {
                    write!(f, " WHEN {} THEN {}", s(w), s(t))?;
                }
                if let Some(el) = el {
                    write!(f, " ELSE {}", s(el))?;
                }
                write!(f, " END)")
            },
            // A third of the time (by the operand, `spell`) a type
            // name of the same affinity (`sqlite3AffinityType`); never of a
            // bare column, the form of an expression index's term, which
            // SQLite matches by its type name too.
            Expr::Cast(a, ty) => {
                // Keyed by the operand as a top-level query renders it, so
                // that it is spelled alike wherever it is (an ORDER BY term
                // of a compound matches its result column by its text).
                let key = a.to_string();
                let name = if matches!(**a, Expr::Col(_)) || spell(&key, 3) != 0 { ty.sql() } else { ty.synonyms()[spell(&format!("{key}:"), ty.synonyms().len() as u64) as usize] };
                write!(f, "CAST({} AS {name})", s(a))
            },
            Expr::Collate(a, k) => write!(f, "({} COLLATE {})", s(a), k.name()),
            Expr::Call(func @ (Func::JsonEach | Func::JsonTree), args) => {
                let count = match args.first() {
                    Some(Expr::Lit(Value::Text(c))) if c != "*" => format!("count(DISTINCT {c})"),
                    _ => "count(*)".to_string(),
                };
                write!(f, "(SELECT {count} FROM {}(", func.name())?;
                list(f, args.get(1..).unwrap_or(&[]), at)?;
                write!(f, "))")
            },
            Expr::Call(func, args) => {
                write!(f, "{}(", func.name())?;
                list(f, args, at)?;
                write!(f, ")")
            },
            Expr::Agg(a, None) => write!(f, "{}(*)", a.name()),
            Expr::Agg(a, Some(e)) => write!(f, "{}({})", a.name(), s(e)),
            Expr::Ordered(o) => {
                // Qualified columns in its ORDER BY, as in a query's.
                let qualified = At { sources: at.sources.max(2), ..at };
                write!(f, "{}({}", o.func.name(), if o.distinct { "DISTINCT " } else { "" })?;
                list(f, &o.args, at)?;
                write!(f, " ORDER BY {})", order_sql(&o.order, &|e| Sql(e, qualified).to_string()))
            },
            Expr::Exists(q, n) => write!(f, "({}EXISTS ({}))", not(*n), sub(q)),
            Expr::InSelect(a, q, n) => write!(f, "({} {}IN ({}))", s(a), not(*n), sub(q)),
            Expr::RowCmp(op, a, b) => write!(f, "({} {} {})", row_sql(a, &s), op.sql(), row_sql(b, &s)),
            Expr::RowBetween(a, lo, hi, n) => write!(f, "({} {}BETWEEN {} AND {})", row_sql(a, &s), not(*n), row_sql(lo, &s), row_sql(hi, &s)),
            Expr::RowIn(a, l, n) => {
                write!(f, "({} {}IN ({}))", row_sql(a, &s), not(*n), l.iter().map(|r| row_sql(r, &s)).collect::<Vec<_>>().join(", "))
            },
            Expr::RowInSelect(a, q, n) => write!(f, "({} {}IN ({}))", row_sql(a, &s), not(*n), sub(q)),
            Expr::Scalar(q) => write!(f, "({})", sub(q)),
            Expr::Window(w) => {
                let mut args: Vec<String> = match (&w.arg, w.func) {
                    (Some(a), _) => vec![s(a)],
                    (None, WinFunc::Count) => vec!["*".to_string()],
                    (None, _) => vec![],
                };
                match w.func {
                    WinFunc::Ntile(n) => args.push(n.to_string()),
                    WinFunc::NthValue(n) => args.push(n.to_string()),
                    WinFunc::Lag(n) | WinFunc::Lead(n) if n != 1 || w.default.is_some() => {
                        args.push(n.to_string());
                        args.extend(w.default.iter().map(s));
                    },
                    WinFunc::GroupConcat(k) if k > 0 => args.push(literal(&Value::Text(GROUP_SEPS[k as usize % GROUP_SEPS.len()].into()))),
                    _ => {},
                }
                let filter = w.filter.as_ref().map_or(String::new(), |e| format!(" FILTER (WHERE {})", s(e)));
                write!(f, "{}({}){filter} OVER {}", w.func.name(), args.join(", "), window_over(w, &s).0)
            },
        }
    }
}

/// A number from 0 to `n - 1` that a rendered text `s` picks: the
/// spelling of a form that has several (`ISNULL` or `IS NULL`, a quoted
/// or a bare name), the same wherever the text is, so that every rewrite
/// of an oracle spells it alike.
///
/// The unit tests of this crate render one spelling (the first: `IS NULL`,
/// bare names) unless a test asks for them all ([`set_spellings`]); every
/// other build renders them all.
pub fn spell(s: &str, n: u64) -> u64 {
    if !SPELLINGS.with(std::cell::Cell::get) {
        return n - 1;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    // FNV mixes the last bytes into the low bits only.
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h % n
}

thread_local! {
    static SPELLINGS: std::cell::Cell<bool> = const { std::cell::Cell::new(!cfg!(test)) };
}

/// Whether this thread renders every spelling of [`spell`].
pub fn set_spellings(on: bool) {
    SPELLINGS.with(|c| c.set(on));
}

/// Column name `name` as an expression writes it: bare, else (one name in
/// eight each) `"name"` or `[name]` (the tokenizer's two quoting paths).
fn quoted(name: &str) -> String {
    match spell(name, 16) {
        0 | 1 => format!("\"{name}\""),
        2 => format!("[{name}]"),
        _ => name.to_string(),
    }
}

/// A binary operator, its spelling picked by its left operand's text `a`:
/// `==` for `=`, `!=` for `<>`, `IS NOT DISTINCT FROM` for `IS`, `IS
/// DISTINCT FROM` for `IS NOT` (the same operators, `parse.y`) a quarter
/// of the time.
fn op_sql(op: BinOp, a: &str) -> &'static str {
    match (op, spell(a, 4)) {
        (BinOp::Eq, 0) => "==",
        (BinOp::Ne, 0) => "!=",
        (BinOp::Is, 0) => "IS NOT DISTINCT FROM",
        (BinOp::IsNot, 0) => "IS DISTINCT FROM",
        _ => op.sql(),
    }
}

/// LIKE pattern `p` written for `ESCAPE '!'` to match what `p` matches
/// without it: each `!` doubled (an escaped `!`), and the first letter or
/// digit escaped too (an escaped ordinary character is itself), so that
/// `patternCompare` reads escapes of both kinds; `%` and `_` stay
/// wildcards.
fn escaped(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 2);
    let mut first = true;
    for ch in p.chars() {
        if ch == '!' {
            out.push('!');
        } else if first && ch.is_ascii_alphanumeric() {
            out.push('!');
            first = false;
        }
        out.push(ch);
    }
    out
}

/// A literal as an expression writes it: [`literal`], but a non-negative
/// integer written in hexadecimal (`0x1f`) or, from 1000, with digit
/// separators (`1_000`) a sixth of the time each, and a text that no name
/// could be (it has a character no identifier the harness writes has)
/// double-quoted a quarter of the time: SQLite reads `"a b"` as the text
/// `'a b'` when no column of that name is in scope (the default build
/// allows double-quoted strings in DML, `SQLITE_DQS`;
/// `areDoubleQuotedStringsEnabled` in `lookupName`).
fn lit_sql(v: &Value) -> String {
    match v {
        Value::Int(i) if *i >= 0 => {
            let dec = i.to_string();
            match spell(&dec, 6) {
                0 => format!("0x{i:x}"),
                1 if *i >= 1000 => {
                    let mut out = String::new();
                    for (k, ch) in dec.chars().enumerate() {
                        if k > 0 && (dec.len() - k) % 3 == 0 {
                            out.push('_');
                        }
                        out.push(ch);
                    }
                    out
                },
                2 => format!("00{dec}"),
                _ => dec,
            }
        },
        // A real as the same decimal number spelled another way now and
        // then (by its text, `spell`): [`respelled_real`].
        Value::Real(_) => {
            let text = literal(v);
            match spell(&text, 9) {
                k @ 0..=4 => respelled_real(&text, k).unwrap_or(text),
                _ => text,
            }
        },
        Value::Text(t) if !t.is_empty()
            && t.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
            && t.chars().all(|c| c != '\0')
            && spell(t, 4) == 0 =>
        {
            format!("\"{}\"", t.replace('"', "\"\""))
        },
        _ => literal(v),
    }
}

/// The decimal number `text` (`[-]digits[.digits][e[-]digits]`, as
/// `literal` renders a real) spelled another way, the same number: its
/// digits as an integer and an exponent (`15e-1`), trailing zeros
/// (`1.500`), leading zeros (`001.5`), an upper-case `E` and a signed
/// exponent (`1.5E+0`), no zero before the point (`.5`); `None` for any
/// other text (a real past the plain range is a product, `real_literal`).
fn respelled_real(text: &str, k: u64) -> Option<String> {
    let (sign, rest) = text.strip_prefix('-').map_or(("", text), |r| ("-", r));
    let (mant, exp) = match rest.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (rest, 0),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let frac_or_0 = if frac.is_empty() { "0" } else { frac };
    Some(match k {
        0 => {
            let digits = format!("{int}{frac}").trim_start_matches('0').to_string();
            let digits = if digits.is_empty() { "0".to_string() } else { digits };
            format!("{sign}{digits}e{}", exp - frac.len() as i32)
        },
        1 => format!("{sign}{int}.{frac_or_0}00{}", if exp != 0 { format!("e{exp}") } else { String::new() }),
        2 => format!("{sign}00{int}.{frac_or_0}{}", if exp != 0 { format!("e{exp}") } else { String::new() }),
        3 => format!("{sign}{int}.{frac_or_0}E{exp:+}"),
        _ if int == "0" => format!("{sign}.{frac_or_0}{}", if exp != 0 { format!("e{exp}") } else { String::new() }),
        _ => return None,
    })
}

/// A row value `(a, b, ..)`, each term rendered by `s`.
fn row_sql(r: &[Expr], s: &dyn Fn(&Expr) -> String) -> String {
    format!("({})", r.iter().map(s).collect::<Vec<_>>().join(", "))
}

/// A window's `OVER` clause, and the named window it reads (`(name,
/// definition)`) when [`Window::named`] says so, each expression rendered
/// by `s`. The name is a hash of the definition, so that windows defined
/// alike share one and the WINDOW clause ([`Query::windows_at`]) has each
/// name once.
fn window_over(w: &Window, s: &dyn Fn(&Expr) -> String) -> (String, Option<(String, String)>) {
    let partition =
        (!w.partition.is_empty()).then(|| format!("PARTITION BY {}", w.partition.iter().map(s).collect::<Vec<_>>().join(", ")));
    let order = (!w.order.is_empty()).then(|| format!("ORDER BY {}", order_sql(&w.order, s)));
    let frame = w.frame.map(|f| f.sql());
    let join = |parts: &[&Option<String>]| parts.iter().filter_map(|p| p.as_deref()).collect::<Vec<_>>().join(" ");
    let (def, rest) = match w.named {
        Named::Inline => return (format!("({})", join(&[&partition, &order, &frame])), None),
        Named::All => (join(&[&partition, &order, &frame]), None),
        Named::Partition => (join(&[&partition]), Some(join(&[&order, &frame]))),
        Named::PartitionOrder => (join(&[&partition, &order]), Some(join(&[&frame]))),
    };
    let name = window_name(&def);
    let over = match rest {
        None => name.clone(),
        Some(r) if r.is_empty() => format!("({name})"),
        Some(r) => format!("({name} {r})"),
    };
    (over, Some((name, def)))
}

/// The name of a named window defined as `def`: `w` and a hash of it.
fn window_name(def: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in def.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    format!("w{:x}", h & 0xff_ffff)
}

/// ORDER BY terms of expressions (a window's, an aggregate's), each
/// rendered by `s`: `e [DESC] [NULLS FIRST | NULLS LAST], ..`.
fn order_sql(order: &[Order], s: &dyn Fn(&Expr) -> String) -> String {
    let terms: Vec<String> = order
        .iter()
        .map(|o| {
            let key = if let OrderKey::Expr(e) = &o.key { s(e) } else { String::new() };
            let nulls = match o.nulls {
                None => "",
                Some(true) => " NULLS FIRST",
                Some(false) => " NULLS LAST",
            };
            format!("{key}{}{nulls}", direction(&key, o.desc))
        })
        .collect();
    terms.join(", ")
}

/// An ORDER BY term's direction: ` DESC`, else a quarter of the time (by
/// the term's text, [`spell`]) ` ASC`, which SQLite reads as no direction.
fn direction(key: &str, desc: bool) -> &'static str {
    if desc {
        " DESC"
    } else if spell(&format!("{key} asc"), 4) == 0 {
        " ASC"
    } else {
        ""
    }
}

/// A join's keyword `kw` (of [`Join::keyword`]), a third of the time (by
/// the text of the source it joins, [`spell`]) with the `INNER` or
/// `OUTER` that SQLite reads alike (`sqlite3JoinType`).
fn join_spelled(kw: &'static str, key: &str) -> &'static str {
    if spell(&format!("{key} join"), 3) != 0 {
        return kw;
    }
    match kw {
        "JOIN" => "INNER JOIN",
        "LEFT JOIN" => "LEFT OUTER JOIN",
        "RIGHT JOIN" => "RIGHT OUTER JOIN",
        "FULL JOIN" => "FULL OUTER JOIN",
        "NATURAL JOIN" => "NATURAL INNER JOIN",
        "NATURAL LEFT JOIN" => "NATURAL LEFT OUTER JOIN",
        "NATURAL RIGHT JOIN" => "NATURAL RIGHT OUTER JOIN",
        "NATURAL FULL JOIN" => "NATURAL FULL OUTER JOIN",
        kw => kw,
    }
}

/// An expression of a top-level query with one source.
impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", Sql(self, At::TOP))
    }
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self}")
    }
}

// ------------------------------------------------------------------ queries

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Agg {
    Count,
    Min,
    Max,
    Sum,
}

impl Agg {
    pub fn name(self) -> &'static str {
        match self {
            Agg::Count => "count",
            Agg::Min => "min",
            Agg::Max => "max",
            Agg::Sum => "sum",
        }
    }
}

/// An aggregate that takes an ORDER BY of its own (SQLite 3.44): those
/// whose value depends on the order of rows, and the [`Agg`]s, whose value
/// does not (a path of its own in the planner and the code generator).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OrdFunc {
    Count,
    Sum,
    Min,
    Max,
    GroupConcat,
    StringAgg,
    JsonGroupArray,
    JsonGroupObject,
}

impl OrdFunc {
    pub fn name(self) -> &'static str {
        match self {
            OrdFunc::Count => "count",
            OrdFunc::Sum => "sum",
            OrdFunc::Min => "min",
            OrdFunc::Max => "max",
            OrdFunc::GroupConcat => "group_concat",
            OrdFunc::StringAgg => "string_agg",
            OrdFunc::JsonGroupArray => "json_group_array",
            OrdFunc::JsonGroupObject => "json_group_object",
        }
    }

    /// The [`Agg`] of the same name, for the four whose value the order
    /// does not change.
    pub fn agg(self) -> Option<Agg> {
        match self {
            OrdFunc::Count => Some(Agg::Count),
            OrdFunc::Sum => Some(Agg::Sum),
            OrdFunc::Min => Some(Agg::Min),
            OrdFunc::Max => Some(Agg::Max),
            _ => None,
        }
    }
}

/// `func([DISTINCT] args ORDER BY o, ..)`: an aggregate of the expression's
/// own query with an ORDER BY of its own, as [`Expr::Agg`] in the items and
/// HAVING of an aggregate query. Its value is determined by the rows
/// ([`ordered_ok`]): the order is total over what it renders.
#[derive(Clone, PartialEq)]
pub struct Ordered {
    pub func: OrdFunc,
    pub distinct: bool,
    pub args: Vec<Expr>,
    /// At least one term, each an [`OrderKey::Expr`].
    pub order: Vec<Order>,
}

impl Ordered {
    /// The ORDER BY terms' expressions.
    pub fn keys(&self) -> Vec<&Expr> {
        self.order.iter().filter_map(|o| if let OrderKey::Expr(e) = &o.key { Some(e) } else { None }).collect()
    }

    /// The terms that make the order total over what it renders (see
    /// [`ordered_ok`]): `CAST(x AS TEXT)` for `group_concat` and
    /// `string_agg` (the text each value is appended as), `json_quote(x)`
    /// for `json_group_array` (the JSON it is appended as, NULL as `null`),
    /// `CAST(k AS TEXT)` and `json_quote(v)` for `json_group_object`; none
    /// for the others.
    pub fn ties(func: OrdFunc, args: &[Expr]) -> Vec<Expr> {
        let text = |x: &Expr| Expr::Cast(bx(x.clone()), CastType::Text);
        let quote = |x: &Expr| Expr::Call(Func::JsonQuote, vec![x.clone()]);
        match (func, args) {
            (OrdFunc::GroupConcat | OrdFunc::StringAgg, [x, ..]) => vec![text(x)],
            (OrdFunc::JsonGroupArray, [x]) => vec![quote(x)],
            (OrdFunc::JsonGroupObject, [k, v]) => vec![text(k), quote(v)],
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum Items {
    /// `*`.
    Star,
    /// One or more expressions (with aggregates in an aggregate query).
    Exprs(Vec<Expr>),
    /// One aggregate of any argument, in a top-level query without GROUP
    /// BY: `count(*)` is `(Count, None)`; the others take an argument.
    Agg(Agg, Option<Expr>),
}

impl Items {
    pub fn exprs(&self) -> Vec<&Expr> {
        match self {
            Items::Star | Items::Agg(_, None) => vec![],
            Items::Exprs(es) => es.iter().collect(),
            Items::Agg(_, Some(e)) => vec![e],
        }
    }

    fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            Items::Star | Items::Agg(_, None) => vec![],
            Items::Exprs(es) => es.iter_mut().collect(),
            Items::Agg(_, Some(e)) => vec![e],
        }
    }
}

/// The kind of a join with USING or NATURAL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinOp {
    Inner,
    Left,
    Right,
    Full,
}

impl JoinOp {
    pub const ALL: [JoinOp; 4] = [JoinOp::Inner, JoinOp::Left, JoinOp::Right, JoinOp::Full];

    pub fn keyword(self) -> &'static str {
        match self {
            JoinOp::Inner => "JOIN",
            JoinOp::Left => "LEFT JOIN",
            JoinOp::Right => "RIGHT JOIN",
            JoinOp::Full => "FULL JOIN",
        }
    }
}

/// How a source is joined to the ones before it.
#[derive(Clone, PartialEq)]
pub enum JoinKind {
    /// `JOIN .. ON e`.
    Inner(Expr),
    /// `LEFT JOIN .. ON e`.
    Left(Expr),
    /// `CROSS JOIN ..`, no ON.
    Cross,
    /// `RIGHT JOIN .. ON e`.
    Right(Expr),
    /// `FULL JOIN .. ON e`.
    Full(Expr),
    /// `op .. USING (c.., ..)`: the columns (distinct, in both sources) the
    /// two sources must agree on; only for the first join.
    Using(JoinOp, Vec<usize>),
    /// `NATURAL op ..`: USING every column both have (`c0` up to the
    /// narrower one's last); only for the first join.
    Natural(JoinOp),
}

impl JoinKind {
    /// A RIGHT or FULL join (with ON, USING or NATURAL).
    pub fn outer_right(&self) -> bool {
        matches!(self.op(), Some(JoinOp::Right | JoinOp::Full))
    }

    /// The kind of join, whatever its condition: `None` for CROSS JOIN.
    pub fn op(&self) -> Option<JoinOp> {
        match self {
            JoinKind::Inner(_) => Some(JoinOp::Inner),
            JoinKind::Left(_) => Some(JoinOp::Left),
            JoinKind::Right(_) => Some(JoinOp::Right),
            JoinKind::Full(_) => Some(JoinOp::Full),
            JoinKind::Using(op, _) | JoinKind::Natural(op) => Some(*op),
            JoinKind::Cross => None,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Join {
    pub table: usize,
    pub kind: JoinKind,
}

impl Join {
    pub fn on(&self) -> Option<&Expr> {
        match &self.kind {
            JoinKind::Inner(e) | JoinKind::Left(e) | JoinKind::Right(e) | JoinKind::Full(e) => Some(e),
            _ => None,
        }
    }

    fn on_mut(&mut self) -> Option<&mut Expr> {
        match &mut self.kind {
            JoinKind::Inner(e) | JoinKind::Left(e) | JoinKind::Right(e) | JoinKind::Full(e) => Some(e),
            _ => None,
        }
    }

    /// The keyword before the source (`NATURAL ..` included).
    pub fn keyword(&self) -> &'static str {
        match self.kind {
            JoinKind::Cross => "CROSS JOIN",
            JoinKind::Natural(JoinOp::Inner) => "NATURAL JOIN",
            JoinKind::Natural(JoinOp::Left) => "NATURAL LEFT JOIN",
            JoinKind::Natural(JoinOp::Right) => "NATURAL RIGHT JOIN",
            JoinKind::Natural(JoinOp::Full) => "NATURAL FULL JOIN",
            _ => self.kind.op().map_or("", JoinOp::keyword),
        }
    }

    /// The columns of a USING or NATURAL join, given the two sources'
    /// column counts; none for any other.
    pub fn using(&self, left: usize, right: usize) -> Vec<usize> {
        match &self.kind {
            JoinKind::Using(_, cols) => cols.clone(),
            JoinKind::Natural(_) => (0..left.min(right)).collect(),
            _ => Vec::new(),
        }
    }
}

/// What an ORDER BY term orders by.
#[derive(Clone, PartialEq)]
pub enum OrderKey {
    /// Result column `k` (from 0), written as its number `k + 1`.
    Column(usize),
    /// `a{src}.rowid`, of a source whose table has a rowid.
    Rowid(usize),
    /// An expression of the query's rows, written with its columns
    /// qualified (a bare name in ORDER BY could name a result column).
    Expr(Expr),
}

/// An ORDER BY term: `key [DESC] [NULLS FIRST | NULLS LAST]`.
#[derive(Clone, PartialEq)]
pub struct Order {
    pub key: OrderKey,
    pub desc: bool,
    /// `NULLS FIRST` (`Some(true)`) or `NULLS LAST` (`Some(false)`); the
    /// default (`None`) puts NULLs first ascending, last descending.
    pub nulls: Option<bool>,
    /// How a query's ORDER BY writes a result column's term
    /// ([`OrderKey::Column`]); [`By::Number`] for any other.
    pub by: By,
}

/// How an ORDER BY term names result column `k`: all three order by the
/// same values (`resolveOrderGroupBy`, `resolveCompoundOrderBy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// Its number, `k + 1`.
    Number,
    /// Its alias: the items of a query with such a term are named `e AS
    /// r{k}` (`c{k}` in a CTE's body, whose items are so named): the name
    /// matched before any column's (`resolveAsName`, `sqlite3MatchEName`).
    Alias,
    /// Its expression, qualified, compared with the items
    /// (`resolveOrderGroupBy`), only an item no earlier item renders the
    /// same as; in a compound (whose ORDER BY SQLite matches with the
    /// first SELECT's items, `resolveOrderByTermToExprList`) never an item
    /// with a COLLATE (the "no such column" of plans seeds 7106 and 9640
    /// of 0..10000 was bugs/sqlite/sqlite-compound-orderby-collate-groupby,
    /// fixed after the vendored SQLite); never an item
    /// SQLite would read as a number, nor one with a window, an aggregate
    /// or a subquery (a count over `json_each` too: `NC_NoSelect`), its
    /// number in place of such a one.
    Expr,
}

impl Order {
    /// Whether NULLs come first under this term.
    pub fn nulls_first(&self) -> bool {
        self.nulls.unwrap_or(!self.desc)
    }
}

/// `LIMIT count OFFSET offset`, or `LIMIT offset, count` when `comma`. A
/// negative count is no limit, a negative offset 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub count: i64,
    pub offset: i64,
    pub comma: bool,
}

impl Limit {
    /// The rows of an ordered result `rows` this limit keeps.
    pub fn slice<T: Clone>(&self, rows: &[T]) -> Vec<T> {
        let start = (self.offset.max(0) as usize).min(rows.len());
        let end = if self.count < 0 { rows.len() } else { start.saturating_add(self.count as usize).min(rows.len()) };
        rows[start..end].to_vec()
    }
}

/// A common table expression's query.
#[derive(Clone, PartialEq)]
pub enum CteBody {
    /// A query over the database's tables (see [`Query::scope`] for what it
    /// may be); its first SELECT's items are named `c0, c1, ..`.
    Select(Box<Query>),
    /// `SELECT start AS c0 UNION ALL SELECT c0 + 1 FROM x{i} WHERE c0 <
    /// start + count - 1`, recursive: `count` (1 or more) integers from
    /// `start`, in ascending order; `form` one of [`COUNT_FORMS`] (the
    /// same rows in the same order).
    Count { start: i64, count: i64, form: u8 },
    /// `SELECT col AS c0, .. FROM vtab(..)`: the rows of a table-valued
    /// function or eponymous virtual table ([`Vtab`]).
    Vtab(Vtab),
}

/// A table-valued function or eponymous virtual table read as a CTE's
/// body, all its columns but the hidden ones as `c0, c1, ..`. Each is
/// read-only and its rows are determined by the database (its schema,
/// its pages) or by its literal arguments, so the oracles compare them as
/// any table: the pragma functions over a table of the schema (`t{table}`
/// exists, [`Cte::info`]), `json_each`/`json_tree` over a JSON text literal
/// (`atom` in place of `value`, which carries the JSON subtype for an
/// object or array, a subtype a materialized CTE drops), `dbstat` and
/// `main.sqlite_schema`.
#[derive(Clone, Debug, PartialEq)]
pub enum Vtab {
    /// `pragma_table_info('t{table}')` (`pragma_table_xinfo` with `xinfo`).
    TableInfo { table: usize, xinfo: bool, arg: VtabArg },
    /// `pragma_index_list('t{table}')`.
    IndexList { table: usize, arg: VtabArg },
    /// `pragma_index_list('t{table}') AS v0, pragma_index_[x]info(v0.name)
    /// AS v1`: every index's columns, the second function's argument a
    /// column of the first (a join of two virtual tables, one constraining
    /// the other's hidden column).
    IndexInfo { table: usize, xinfo: bool },
    /// `pragma_foreign_key_list('t{table}')`.
    ForeignKeys { table: usize, arg: VtabArg },
    /// `pragma_table_list`: every table of every schema.
    TableList,
    /// `json_each(doc[, path])` (`json_tree` with `tree`), `doc` a JSON
    /// text, `path` one of its elements' paths.
    Json { tree: bool, doc: String, path: Option<String>, arg: VtabArg },
    /// `dbstat` (`dbstat('main', 1)`, a row per b-tree, with `aggregate`).
    Dbstat { aggregate: bool, arg: VtabArg },
    /// `main.sqlite_schema`, as [`SCHEMA_FORMS`]`[form]` names it and its
    /// columns (`main.sqlite_master.type`: a column qualified by the
    /// database and the schema table's other name, `isValidSchemaTableName`).
    SqliteSchema { form: usize },
    /// `pragma_{name}`, [`PRAGMAS`]`[k]`, of `t{table}` when given (a check
    /// of that table).
    Pragma { k: usize, table: Option<usize> },
    /// A virtual table of the database, `vt{k}` ([`crate::vtabs::VQuery`]).
    Virtual(crate::vtabs::VQuery),
}

/// The pragmas read as table-valued functions by [`Vtab::Pragma`]: their
/// columns, and whether they take a table. Those whose values are
/// determined by the case's database (its header, its pages) and what is
/// built in; never one storage changes (`journal_mode`, `cache_size`,
/// `synchronous`, `database_list`'s file name), nor a connection's flag
/// (`foreign_keys`, `recursive_triggers`, ..: a statement sets it, and the
/// storage oracle's reopened connection has it as a new one does).
/// Never `module_list`, which lists the pragma modules the connection has
/// registered so far: they register when first read (`pragma_page_count`
/// is listed only after a query has read it).
pub const PRAGMAS: [(&str, &[&str], bool); 13] = [
    ("page_count", &["page_count"], false),
    ("freelist_count", &["freelist_count"], false),
    ("page_size", &["page_size"], false),
    ("encoding", &["encoding"], false),
    ("schema_version", &["schema_version"], false),
    ("user_version", &["user_version"], false),
    ("application_id", &["application_id"], false),
    ("auto_vacuum", &["auto_vacuum"], false),
    ("collation_list", &["seq", "name"], false),
    ("function_list", &["name", "builtin", "type", "enc", "narg", "flags"], false),
    ("compile_options", &["compile_options"], false),
    ("integrity_check", &["integrity_check"], true),
    ("quick_check", &["quick_check"], true),
];

/// How a table-valued function takes its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtabArg {
    /// `f(a, ..)`.
    Call,
    /// `f(a, .., 'main')` for a pragma (its schema argument), `main.f(a)`
    /// for the others.
    Schema,
    /// `f AS v0 WHERE v0.arg = a ..`: constraints on its hidden columns.
    Hidden,
}

/// The FROM clause and the columns of [`Vtab::SqliteSchema`].
/// The last two read other schemas' tables: the TEMP schema's under its
/// own name (`sqlite_temp_schema`, which `isValidSchemaTableName` takes
/// for the TEMP schema's table, its columns qualified by it), and `aux`'s
/// (only while it is attached, [`aux_attached`]).
pub const SCHEMA_FORMS: [(&str, [&str; 5]); 6] = [
    ("main.sqlite_schema", ["type", "name", "tbl_name", "rootpage", "sql"]),
    ("main.sqlite_master", ["main.sqlite_master.type", "main.sqlite_master.name", "main.sqlite_master.tbl_name", "main.sqlite_master.rootpage", "main.sqlite_master.sql"]),
    ("sqlite_schema", ["main.sqlite_schema.type", "main.sqlite_schema.name", "main.sqlite_schema.tbl_name", "main.sqlite_schema.rootpage", "main.sqlite_schema.sql"]),
    ("main.sqlite_schema", ["sqlite_master.type", "sqlite_master.name", "sqlite_master.tbl_name", "sqlite_master.rootpage", "sqlite_master.sql"]),
    ("sqlite_temp_schema", ["sqlite_temp_schema.type", "sqlite_temp_schema.name", "temp.sqlite_temp_schema.tbl_name", "sqlite_temp_schema.rootpage", "sqlite_temp_schema.sql"]),
    ("aux.sqlite_master", ["type", "aux.sqlite_master.name", "sqlite_master.tbl_name", "rootpage", "aux.sqlite_master.sql"]),
];

/// The forms of [`SCHEMA_FORMS`] that read main's.
pub const MAIN_SCHEMA_FORMS: usize = 4;

/// What [`Vtab`]s read of a table of the schema: its columns' declared
/// types, its indexes' column counts, its foreign keys' column counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub types: Vec<&'static str>,
    pub indexes: Vec<usize>,
    pub fks: Vec<usize>,
    /// An upper bound of the pages its b-trees (the table's and its
    /// indexes') take, from the bytes of its values, as if in 512-byte
    /// pages: large values spill to overflow pages, which a bound by row
    /// count alone misses ([`Vtab::info`]'s page bound).
    pub pages: usize,
}

impl Vtab {
    /// The source table it introspects, if any.
    pub fn table(&self) -> Option<usize> {
        match self {
            Vtab::TableInfo { table, .. } | Vtab::IndexList { table, .. } | Vtab::IndexInfo { table, .. } | Vtab::ForeignKeys { table, .. } => Some(*table),
            Vtab::Pragma { table, .. } => *table,
            _ => None,
        }
    }

    fn arg(&self) -> VtabArg {
        match self {
            Vtab::TableInfo { arg, .. } | Vtab::IndexList { arg, .. } | Vtab::ForeignKeys { arg, .. } | Vtab::Json { arg, .. } | Vtab::Dbstat { arg, .. } => *arg,
            _ => VtabArg::Call,
        }
    }

    /// Its columns as the body reads them (`v0.` and `v1.` qualified for
    /// [`Vtab::IndexInfo`]).
    pub fn columns(&self) -> Vec<String> {
        if let Vtab::Virtual(v) = self {
            return virtual_kind(v.vt()).map_or_else(Vec::new, |k| v.columns(&k));
        }
        self.static_columns().into_iter().map(String::from).collect()
    }

    fn static_columns(&self) -> Vec<&'static str> {
        match self {
            Vtab::TableInfo { xinfo, .. } => {
                let mut c = vec!["cid", "name", "type", "\"notnull\"", "dflt_value", "pk"];
                if *xinfo {
                    c.push("hidden");
                }
                c
            },
            Vtab::IndexList { .. } => vec!["seq", "name", "\"unique\"", "origin", "partial"],
            Vtab::IndexInfo { xinfo: false, .. } => vec!["v0.name", "v1.seqno", "v1.cid", "v1.name"],
            Vtab::IndexInfo { xinfo: true, .. } => vec!["v0.name", "v1.seqno", "v1.cid", "v1.name", "v1.\"desc\"", "v1.coll", "v1.\"key\""],
            Vtab::ForeignKeys { .. } => vec!["id", "seq", "\"table\"", "\"from\"", "\"to\"", "on_update", "on_delete", "\"match\""],
            Vtab::TableList => vec!["schema", "name", "type", "ncol", "wr", "strict"],
            Vtab::Json { .. } => vec!["key", "atom", "type", "id", "parent", "fullkey", "path"],
            Vtab::Dbstat { aggregate: false, .. } => vec!["name", "path", "pageno", "pagetype", "ncell", "payload", "unused", "mx_payload", "pgoffset", "pgsize"],
            Vtab::Dbstat { aggregate: true, .. } => vec!["name", "pageno", "ncell", "payload", "unused", "mx_payload", "pgsize"],
            Vtab::SqliteSchema { form } => SCHEMA_FORMS[*form % SCHEMA_FORMS.len()].1.to_vec(),
            Vtab::Pragma { k, .. } => PRAGMAS[*k % PRAGMAS.len()].1.to_vec(),
            Vtab::Virtual(_) => Vec::new(),
        }
    }

    /// `FROM ..` of its body, with constraints on hidden columns.
    fn source_sql(&self) -> String {
        let q = |s: &str| literal(&Value::Text(s.to_string()));
        // The schema argument the table's own schema.
        let pragma = |f: &str, t: usize, arg: VtabArg| match arg {
            VtabArg::Call => format!("{f}('t{t}')"),
            VtabArg::Schema => format!("{f}('t{t}', '{}')", table_db(t).name()),
            VtabArg::Hidden => format!("{f} AS v0 WHERE v0.arg = 't{t}' AND v0.schema = '{}'", table_db(t).name()),
        };
        match self {
            Vtab::TableInfo { table, xinfo, arg } => pragma(if *xinfo { "pragma_table_xinfo" } else { "pragma_table_info" }, *table, *arg),
            Vtab::IndexList { table, arg } => pragma("pragma_index_list", *table, *arg),
            Vtab::IndexInfo { table, xinfo } => {
                format!("pragma_index_list('t{table}') AS v0, pragma_index_{}info(v0.name) AS v1", if *xinfo { "x" } else { "" })
            },
            Vtab::ForeignKeys { table, arg } => pragma("pragma_foreign_key_list", *table, *arg),
            Vtab::TableList => "pragma_table_list".to_string(),
            Vtab::Json { tree, doc, path, arg } => {
                let f = if *tree { "json_tree" } else { "json_each" };
                match (arg, path) {
                    (VtabArg::Hidden, None) => format!("{f} AS v0 WHERE v0.json = {}", q(doc)),
                    (VtabArg::Hidden, Some(p)) => format!("{f} AS v0 WHERE v0.json = {} AND v0.root = {}", q(doc), q(p)),
                    (VtabArg::Schema, None) => format!("main.{f}({})", q(doc)),
                    (VtabArg::Schema, Some(p)) => format!("main.{f}({}, {})", q(doc), q(p)),
                    (VtabArg::Call, None) => format!("{f}({})", q(doc)),
                    (VtabArg::Call, Some(p)) => format!("{f}({}, {})", q(doc), q(p)),
                }
            },
            Vtab::Dbstat { aggregate, arg } => match (arg, aggregate) {
                (VtabArg::Hidden, false) => "dbstat AS v0 WHERE v0.schema = 'main'".to_string(),
                (VtabArg::Hidden, true) => "dbstat AS v0 WHERE v0.schema = 'main' AND v0.aggregate = 1".to_string(),
                (VtabArg::Call, true) => "dbstat('main', 1)".to_string(),
                (VtabArg::Schema, true) => "main.dbstat('main', 1)".to_string(),
                (VtabArg::Schema, false) => "dbstat('main')".to_string(),
                (VtabArg::Call, false) => "dbstat".to_string(),
            },
            Vtab::SqliteSchema { form } => SCHEMA_FORMS[*form % SCHEMA_FORMS.len()].0.to_string(),
            Vtab::Pragma { k, table: None } => format!("pragma_{}", PRAGMAS[*k % PRAGMAS.len()].0),
            Vtab::Pragma { k, table: Some(t) } => format!("pragma_{}('t{t}')", PRAGMAS[*k % PRAGMAS.len()].0),
            Vtab::Virtual(_) => String::new(),
        }
    }

    /// `SELECT col AS c0, .. FROM ..`; `+col` for `dbstat`, whose
    /// `xBestIndex` takes any equality it cannot use (one with a source
    /// after it, as a CROSS JOIN puts it) for no plan at all
    /// (`statBestIndex` wants to be the right-most table of a join): an
    /// expression of a column is no constraint on it.
    pub fn sql(&self, noindex: bool) -> String {
        if let Vtab::Virtual(v) = self {
            return virtual_kind(v.vt()).map_or_else(String::new, |k| v.sql(&k, noindex));
        }
        let plus = if matches!(self, Vtab::Dbstat { .. }) { "+" } else { "" };
        let items: Vec<String> = self.columns().iter().enumerate().map(|(k, c)| format!("{plus}{c} AS c{k}")).collect();
        format!("SELECT {} FROM {}", items.join(", "), self.source_sql())
    }

    /// Its part of a CTE's cost: 1, its argument form, its flags, the JSON
    /// text's length.
    fn cost(&self) -> u64 {
        let flags = match self {
            Vtab::TableInfo { xinfo, .. } | Vtab::IndexInfo { xinfo, .. } => u64::from(*xinfo),
            Vtab::Json { tree, doc, path, .. } => u64::from(*tree) + doc.len() as u64 + path.as_ref().map_or(0, |p| 1 + p.len() as u64),
            Vtab::Dbstat { aggregate, .. } => u64::from(*aggregate),
            Vtab::Pragma { k, table } => *k as u64 + u64::from(table.is_some()),
            Vtab::SqliteSchema { form } => *form as u64,
            Vtab::Virtual(v) => v.cost(),
            _ => 0,
        };
        let arg = match self.arg() {
            VtabArg::Call => 0,
            VtabArg::Schema => 1,
            VtabArg::Hidden => 2,
        };
        1 + arg + flags
    }

    /// As a source: its columns, about how many rows it has (an upper
    /// bound for `dbstat`'s pages), values its columns hold; `None` when
    /// the table it reads is not in `schema`.
    fn info(&self, schema: &Schema) -> Option<TableInfo> {
        if let Vtab::Virtual(v) = self {
            let (rows, by_column, width) = VINFO.with(|i| v.info(&i.borrow()))?;
            let mut samples = Vec::new();
            for x in by_column.iter().flatten() {
                if samples.len() < 16 && !samples.contains(x) && short(x) && value_ok(x) {
                    samples.push(x.clone());
                }
            }
            let by_column = by_column.into_iter().map(|c| c.into_iter().filter(|x| short(x) && value_ok(x)).collect()).collect();
            return Some(TableInfo { columns: width, rows, samples, by_column, shared: Vec::new(), key: RowKey::None, exprs: Vec::new(), partial: Vec::new(), leading: Vec::new(), coll: vec![Some(Coll::Binary); width], aff: vec![Affinity::None; width] });
        }
        let text = |s: &str| Value::Text(s.to_string());
        let ints = |n: usize| (0..n.min(6) as i64).map(Value::Int).collect::<Vec<_>>();
        let width = self.columns().len();
        let mut by_column = vec![Vec::new(); width];
        let none = Catalog::default();
        let cat = |t: usize| schema.catalog.get(t).unwrap_or(&none);
        if let Some(t) = self.table() {
            schema.tables.get(t)?;
        }
        let indexes = |t: usize| (0..cat(t).indexes.len().min(6)).map(|i| text(&format!("t{t}_i{i}"))).collect::<Vec<_>>();
        // The virtual tables' shadow tables, and pages for their rows.
        let (shadows, shadow_pages) = VINFO.with(|i| crate::vtabs::shadows(&i.borrow()));
        let pages = || {
            let trees: usize = 1 + schema.tables.len() + shadows + schema.catalog.iter().map(|c| c.indexes.len()).sum::<usize>();
            trees + shadow_pages + schema.tables.iter().enumerate().map(|(k, t)| ((t.rows * t.columns.max(1)).div_ceil(16) * 2).max(schema.catalog.get(k).map_or(0, |c| c.pages))).sum::<usize>()
        };
        let rows = match self {
            Vtab::Virtual(_) => return None,
            Vtab::Pragma { k, table } => {
                let (name, _, tabled) = PRAGMAS[*k % PRAGMAS.len()];
                if *k >= PRAGMAS.len() || table.is_some() && !tabled {
                    return None;
                }
                match name {
                    "collation_list" => 3,
                    "function_list" => 300,
                    "compile_options" => 80,
                    // One row, but each read checks every page.
                    "integrity_check" | "quick_check" => pages(),
                    _ => 1,
                }
            },
            Vtab::TableInfo { table, xinfo, .. } => {
                let n = schema.tables[*table].columns;
                by_column[0] = ints(n);
                by_column[1] = (0..n.min(6)).map(|c| text(&format!("c{c}"))).collect();
                by_column[2] = cat(*table).types.iter().take(6).map(|s| text(s)).collect();
                by_column[3] = ints(2);
                by_column[5] = ints(2);
                if *xinfo {
                    by_column[6] = ints(1);
                }
                n
            },
            Vtab::IndexList { table, .. } => {
                by_column[0] = ints(cat(*table).indexes.len());
                by_column[1] = indexes(*table);
                by_column[2] = ints(2);
                by_column[3] = vec![text("c"), text("pk"), text("u")];
                by_column[4] = ints(2);
                cat(*table).indexes.len()
            },
            Vtab::IndexInfo { table, xinfo } => {
                by_column[0] = indexes(*table);
                by_column[1] = ints(3);
                by_column[2] = vec![Value::Int(-2), Value::Int(-1), Value::Int(0), Value::Int(1)];
                by_column[3] = vec![text("c0"), text("c1")];
                if *xinfo {
                    by_column[4] = ints(2);
                    by_column[5] = vec![text("BINARY"), text("NOCASE"), text("RTRIM")];
                    by_column[6] = ints(2);
                }
                cat(*table).indexes.iter().map(|k| k + if *xinfo { 2 } else { 0 }).sum()
            },
            Vtab::ForeignKeys { table, .. } => {
                by_column[0] = ints(cat(*table).fks.len());
                by_column[1] = ints(2);
                by_column[2] = (0..schema.tables.len().min(6)).map(|t| text(&format!("t{t}"))).collect();
                by_column[3] = vec![text("c0"), text("c1")];
                by_column[5] = vec![text("NO ACTION"), text("CASCADE"), text("SET NULL")];
                cat(*table).fks.iter().sum()
            },
            Vtab::TableList => {
                by_column[0] = vec![text("main"), text("temp")];
                by_column[1] = (0..schema.tables.len().min(5)).map(|t| text(&format!("t{t}"))).chain([text("sqlite_schema")]).collect();
                by_column[2] = vec![text("table"), text("view")];
                by_column[3] = ints(6);
                schema.tables.len() + 4 + shadows
            },
            Vtab::Json { tree, doc, .. } => {
                by_column[2] = vec![text("integer"), text("text"), text("object"), text("array"), text("null"), text("real")];
                by_column[3] = ints(6);
                // At most one row per value of the text.
                let values = 1 + doc.matches([',', ':', '[', '{']).count();
                if *tree {
                    values
                } else {
                    values.min(1 + doc.matches(',').count())
                }
            },
            Vtab::Dbstat { aggregate, .. } => {
                by_column[0] = (0..schema.tables.len().min(5)).map(|t| text(&format!("t{t}"))).chain([text("sqlite_schema")]).collect();
                if !*aggregate {
                    by_column[1] = vec![text("/"), text("/000/")];
                    by_column[3] = vec![text("leaf"), text("internal"), text("overflow")];
                }
                if *aggregate {
                    1 + schema.tables.len() + shadows + schema.catalog.iter().map(|c| c.indexes.len()).sum::<usize>()
                } else {
                    // A page per 8 values, at least, and the trees' roots.
                    pages()
                }
            },
            Vtab::SqliteSchema { form } => {
                if *form >= SCHEMA_FORMS.len() || *form == SCHEMA_FORMS.len() - 1 && !aux_attached() {
                    return None;
                }
                by_column[0] = vec![text("table"), text("index"), text("view"), text("trigger")];
                by_column[1] = (0..schema.tables.len().min(6)).map(|t| text(&format!("t{t}"))).collect();
                by_column[2] = by_column[1].clone();
                schema.tables.len() + shadows + schema.catalog.iter().map(|c| c.indexes.len()).sum::<usize>()
            },
        };
        // The model does not see what these list beyond its tables and
        // indexes (views, triggers, the stat tables, automatic indexes,
        // overflow and sparse pages), so their counts are read as at least
        // the most SQLite gave on the cases of 8000 inputs from seed 9300000
        // with --queries 1-300 (`index_list` 8, `index_xinfo` 33,
        // `table_list` 32, `sqlite_schema` 61, `dbstat` 1873, 1130 at the
        // 99.9th percentile), so that a join of them stays within
        // ROW_BUDGET ([`Query::fits`]): a drawn count of 49 pages where
        // dbstat had 551, and `sqlite_schema` of 33 rows where 1 was
        // drawn, multiplied through joins of three of them to millions of
        // rows (run 20261017-0438 and 20261018-0438: 8 workers over 8 GB;
        // `dbstat` three times joined, 3.6 GB, `pragma_table_list` three
        // times with `pragma_index_xinfo`, 3.9 GB, collecting the rows).
        let rows = rows.max(match self {
            Vtab::IndexList { .. } => 8,
            Vtab::IndexInfo { .. } => 40,
            // 41 at seed 9300047 once Geopoly tables were filled (round
            // rtree-geopoly): fifteen views, the stat tables of main and
            // aux, two virtual tables and their shadow tables.
            Vtab::TableList => 48,
            // 66 at seed 9300147 of virtual_tables_count_what_sqlite_lists
            // once the ALTER TABLE draws moved (round schema-refs).
            Vtab::SqliteSchema { .. } => 80,
            Vtab::Dbstat { aggregate: true, .. } => 64,
            Vtab::Dbstat { aggregate: false, .. } => 2000,
            _ => 0,
        });
        let aff = match self {
            Vtab::Dbstat { aggregate: false, .. } => [Affinity::Text, Affinity::Text, Affinity::Integer, Affinity::Text].into_iter().chain([Affinity::Integer; 6]).collect(),
            Vtab::Dbstat { aggregate: true, .. } => [Affinity::Text].into_iter().chain([Affinity::Integer; 6]).collect(),
            Vtab::SqliteSchema { .. } => vec![Affinity::Text, Affinity::Text, Affinity::Text, Affinity::Integer, Affinity::Text],
            _ => vec![Affinity::None; width],
        };
        let mut samples = Vec::new();
        for v in by_column.iter().flatten() {
            if samples.len() < 16 && !samples.contains(v) {
                samples.push(v.clone());
            }
        }
        Some(TableInfo { columns: width, rows, samples, by_column, shared: Vec::new(), key: RowKey::None, exprs: Vec::new(), partial: Vec::new(), leading: Vec::new(), coll: vec![Some(Coll::Binary); width], aff })
    }
}

/// `x{i}[(c0, ..)] AS [[NOT] MATERIALIZED] (body)`, CTE `i` of a top-level
/// query's WITH clause.
#[derive(Clone, PartialEq)]
pub struct Cte {
    pub body: CteBody,
    /// `MATERIALIZED` (`Some(true)`), `NOT MATERIALIZED` (`Some(false)`), or
    /// neither.
    pub hint: Option<bool>,
    /// With the column list `(c0, ..)` (always for [`CteBody::Count`]).
    pub columns: bool,
}

/// The forms of a [`CteBody::Count`]'s recursion, each giving its
/// integers in ascending order: the compound's operator and what follows
/// the recursive SELECT (`{end}` the last integer, `{count}` how many,
/// `{more}` a limit past them). UNION keeps a table of the rows to drop
/// repeats; an ORDER BY of the compound makes its queue a priority queue;
/// a LIMIT stops the recursion (`generateWithRecursiveQuery`).
pub const COUNT_FORMS: [(&str, &str); 6] = [
    ("UNION ALL", " WHERE c0 < {end}"),
    ("UNION", " WHERE c0 < {end}"),
    ("UNION ALL", " WHERE c0 < {end} ORDER BY 1"),
    ("UNION ALL", " LIMIT {count}"),
    ("UNION", " ORDER BY 1 LIMIT {count} OFFSET 0"),
    ("UNION ALL", " WHERE c0 < {end} LIMIT {more}"),
];

/// The most rows a [`CteBody::Count`] generates.
pub const MAX_COUNT: i64 = 20;

/// The marker of CTE `i` in a query rendered with [`At::inline`]: NUL never
/// appears in SQL text otherwise ([`value_ok`]).
fn cte_marker(i: usize) -> String {
    format!("\0{i}\0")
}

impl Cte {
    /// The CTE as a source against the database's `schema`, `None` when its
    /// query is not valid:
    ///
    /// - [`CteBody::Count`]: 1 to [`MAX_COUNT`] integers from a start within
    ///   1000 of 0, [`Cte::columns`] unset (its column list is always
    ///   written, so the flag would only make two values of one query);
    /// - [`CteBody::Select`]: a valid top-level query without a WITH clause
    ///   whose first SELECT has expression items; every SELECT of it neither
    ///   DISTINCT nor grouped nor one aggregate ([`Items::Agg`]), its
    ///   compound operators UNION ALL, a LIMIT only with an order by every
    ///   source's key. Its rows are then determined exactly, values and
    ///   multiplicities (aggregates without GROUP BY are of determined
    ///   values by construction), so a query reading it is as determined as
    ///   one reading a table.
    ///
    /// Its rows count as the product of its sources' (the sum over a
    /// compound's SELECTs, 1 for an aggregate), what reading it costs; its
    /// sample values are those of its tables; a column that is a column of
    /// a table has that column's values and shares its values' pairs
    /// ([`TableInfo::shared`]). No key ([`RowKey::None`]).
    pub fn info(&self, schema: &Schema) -> Option<TableInfo> {
        match &self.body {
            CteBody::Count { start, count, form } => {
                // `columns` unset: the list is always written (the same SQL).
                if !(1..=MAX_COUNT).contains(count) || start.abs() > 1000 || self.columns || usize::from(*form) >= COUNT_FORMS.len() {
                    return None;
                }
                let values: Vec<Value> = (0..*count.min(&6)).map(|k| Value::Int(start + k)).collect();
                let rows = *count as usize;
                let by_column = vec![values.clone()];
                // Its column, of a literal (`SELECT {start} AS c0 UNION ALL
                // ..`), has no declared sequence: a reference to it has
                // BINARY (`sqlite3ExprCollSeq` of a column).
                Some(TableInfo { columns: 1, rows, samples: values, by_column, shared: Vec::new(), key: RowKey::None, exprs: Vec::new(), partial: Vec::new(), leading: Vec::new(), coll: vec![Some(Coll::Binary)], aff: Vec::new() })
            },
            // `columns` unset: its items are always named.
            CteBody::Vtab(v) => if self.columns { None } else { v.info(schema) },
            CteBody::Select(q) => {
                let cores = q.cores();
                let ok = q.with.is_empty()
                    && matches!(q.items, Items::Exprs(_))
                    && q.valid(schema)
                    && cores.iter().all(|c| !c.distinct && c.group.is_empty() && !matches!(c.items, Items::Agg(..)))
                    && q.compound.iter().all(|(op, _)| *op == SetOp::UnionAll)
                    && (q.limit.is_none() || q.total_order(schema) == Some(true));
                if !ok {
                    return None;
                }
                let rows = cores
                    .iter()
                    .map(|c| {
                        if c.aggregate() {
                            1
                        } else {
                            c.sources().iter().map(|t| schema.info(*t).map_or(1, |i| i.rows.max(1))).product()
                        }
                    })
                    .sum();
                let mut samples = Vec::new();
                for t in q.sources() {
                    for v in schema.info(t).map_or(&[][..], |i| &i.samples) {
                        if samples.len() < 16 && !samples.contains(v) {
                            samples.push(v.clone());
                        }
                    }
                }
                let (mut by_column, mut shared) = (Vec::new(), Vec::new());
                for (k, e) in q.items.exprs().into_iter().enumerate() {
                    match e {
                        Expr::Col(c) if c.up == 0 => {
                            let t = q.sources()[c.src];
                            let info = schema.info(t).unwrap();
                            by_column.push(info.by_column.get(c.col).cloned().unwrap_or_default());
                            shared.push((k, t, c.col));
                            shared.extend(info.shared.iter().filter(|s| s.0 == c.col).map(|&(_, u, d)| (k, u, d)));
                        },
                        _ => by_column.push(Vec::new()),
                    }
                }
                // A CTE's column has its first SELECT's item's collating
                // sequence (`sqlite3SubqueryColumnTypes`).
                let levels = [q.sources()];
                let coll = q.items.exprs().iter().map(|e| Some(expr_coll(e, schema, &levels))).collect();
                let aff = q
                    .items
                    .exprs()
                    .iter()
                    .map(|e| match e {
                        Expr::Col(c) if c.up == 0 => schema.info(levels[0][c.src]).and_then(|i| i.aff.get(c.col).copied()).unwrap_or(Affinity::None),
                        _ => Affinity::None,
                    })
                    .collect();
                Some(TableInfo { columns: by_column.len(), rows, samples, by_column, shared, key: RowKey::None, exprs: Vec::new(), partial: Vec::new(), leading: Vec::new(), coll, aff })
            },
        }
    }

    /// The body's SQL, with `NOT INDEXED` on its sources when `noindex`.
    fn body_sql(&self, i: usize, noindex: bool) -> String {
        match &self.body {
            CteBody::Count { start, count, form } => {
                let end = start + count - 1;
                let (union, tail) = COUNT_FORMS[usize::from(*form) % COUNT_FORMS.len()];
                let tail = tail.replace("{end}", &end.to_string()).replace("{count}", &count.to_string()).replace("{more}", &(count + 5).to_string());
                format!("SELECT {start} AS c0 {union} SELECT c0 + 1 FROM x{i}{tail}")
            },
            CteBody::Select(q) => {
                let hint = |c: &Query| vec![" NOT INDEXED".to_string(); 1 + c.joins.len()];
                let hints: Vec<Vec<String>> = if noindex { q.cores().iter().map(hint).collect() } else { Vec::new() };
                q.render_at(At { noindex, named: true, ..q.at(0) }, &hints)
            },
            CteBody::Vtab(v) => v.sql(noindex),
        }
    }

    /// The query in place of a reference to it, as a subquery in FROM: the
    /// body, or for a [`CteBody::Count`] its integers as a UNION ALL of
    /// literals.
    fn inline_sql(&self, i: usize, noindex: bool) -> String {
        match &self.body {
            CteBody::Count { start, count, .. } => {
                let values: Vec<String> =
                    (0..*count).map(|k| format!("SELECT {}{}", start + k, if k == 0 { " AS c0" } else { "" })).collect();
                format!("({})", values.join(" UNION ALL "))
            },
            CteBody::Select(_) | CteBody::Vtab(_) => format!("({})", self.body_sql(i, noindex)),
        }
    }

    /// Its part of [`Query::cost`]: 2, its hint, its column list, and its
    /// query's cost (for [`CteBody::Count`]: 1, the count, the start's
    /// magnitude).
    pub fn cost(&self) -> u64 {
        let body = match &self.body {
            CteBody::Count { start, count, form } => 1 + count.unsigned_abs() + start.unsigned_abs() + u64::from(*form),
            CteBody::Select(q) => q.cost(),
            CteBody::Vtab(v) => v.cost(),
        };
        2 + u64::from(self.hint.is_some()) + u64::from(self.columns) + body
    }

    /// How many columns it has.
    pub fn width(&self, schema: &Schema) -> usize {
        match &self.body {
            CteBody::Count { .. } => 1,
            CteBody::Select(q) => q.width(schema),
            CteBody::Vtab(v) => v.columns().len(),
        }
    }
}

/// A compound operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetOp {
    UnionAll,
    Union,
    Intersect,
    Except,
}

impl SetOp {
    pub const ALL: [SetOp; 4] = [SetOp::UnionAll, SetOp::Union, SetOp::Intersect, SetOp::Except];

    pub fn sql(self) -> &'static str {
        match self {
            SetOp::UnionAll => "UNION ALL",
            SetOp::Union => "UNION",
            SetOp::Intersect => "INTERSECT",
            SetOp::Except => "EXCEPT",
        }
    }
}

/// `SELECT [DISTINCT] items FROM t{table} AS a0 [joins] [WHERE filter]
/// [GROUP BY group [HAVING having]] [op SELECT ..].. [ORDER BY order [LIMIT
/// limit]]`. Its
/// sources are `table`, then each join's table, aliased by position
/// ([`alias`]). An ON clause sees the sources up to its own.
#[derive(Clone, PartialEq)]
pub struct Query {
    /// The WITH clause, only at the top level: CTE `i` is the source table
    /// [`CTE`]` + i`, of the query, its subqueries and compound members.
    pub with: Vec<Cte>,
    /// The first source's table (a CTE's from [`CTE`] on).
    pub table: usize,
    pub joins: Vec<Join>,
    /// Not with [`Items::Agg`].
    pub distinct: bool,
    pub items: Items,
    pub filter: Option<Expr>,
    /// GROUP BY terms; none for no GROUP BY.
    pub group: Vec<Expr>,
    /// Only with GROUP BY.
    pub having: Option<Expr>,
    /// Compound members, only at the top level: `op SELECT ..` after this
    /// query's own SELECT (its core), left to right; each member a core (no
    /// compound, ORDER BY or LIMIT of its own) of the same width. The ORDER
    /// BY and LIMIT apply to the whole compound.
    pub compound: Vec<(SetOp, Query)>,
    /// ORDER BY terms, only at the top level and only a total order
    /// ([`Query::total_order`]); none for no ORDER BY.
    pub order: Vec<Order>,
    /// Only with ORDER BY.
    pub limit: Option<Limit>,
}

/// What a query is for: the top level, or the subquery of an EXISTS, IN or
/// scalar subquery node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Top,
    Exists,
    In,
    Scalar,
    /// The subquery of a row value IN ([`Expr::RowInSelect`]): this many
    /// columns.
    Row(usize),
}

impl Query {
    /// A query on one table, no GROUP BY.
    pub fn simple(table: usize, distinct: bool, items: Items, filter: Option<Expr>) -> Query {
        Query {
            with: Vec::new(),
            table,
            joins: Vec::new(),
            distinct,
            items,
            filter,
            group: Vec::new(),
            having: None,
            compound: Vec::new(),
            order: Vec::new(),
            limit: None,
        }
    }

    /// The table of each source, in FROM order.
    pub fn sources(&self) -> Vec<usize> {
        std::iter::once(self.table).chain(self.joins.iter().map(|j| j.table)).collect()
    }

    /// Whether the query aggregates: a GROUP BY, or an aggregate among its
    /// items.
    pub fn aggregate(&self) -> bool {
        !self.group.is_empty() || matches!(self.items, Items::Agg(..)) || self.items.exprs().iter().any(|e| e.has_agg())
    }

    /// Every expression of the query, in rendering order: items, ON
    /// clauses, filter, GROUP BY terms, HAVING, ORDER BY terms.
    pub fn exprs(&self) -> Vec<&Expr> {
        let mut v = self.items.exprs();
        v.extend(self.joins.iter().filter_map(Join::on));
        v.extend(self.filter.iter());
        v.extend(self.group.iter());
        v.extend(self.having.iter());
        v.extend(self.order.iter().filter_map(|o| if let OrderKey::Expr(e) = &o.key { Some(e) } else { None }));
        v
    }

    pub fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut v = self.items.exprs_mut();
        v.extend(self.joins.iter_mut().filter_map(Join::on_mut));
        v.extend(self.filter.iter_mut());
        v.extend(self.group.iter_mut());
        v.extend(self.having.iter_mut());
        v.extend(self.order.iter_mut().filter_map(|o| if let OrderKey::Expr(e) = &mut o.key { Some(e) } else { None }));
        v
    }

    /// Whether a window function is among the items.
    pub fn has_window(&self) -> bool {
        self.items.exprs().iter().any(|e| matches!(e, Expr::Window(_)))
    }

    /// Whether the query neither aggregates nor has DISTINCT nor is a
    /// compound: each result row comes from one row of its sources.
    pub fn plain(&self) -> bool {
        !self.distinct && !self.aggregate() && self.compound.is_empty()
    }

    /// The query's own SELECT, without compound members, ORDER BY and LIMIT.
    pub fn core(&self) -> Query {
        Query { compound: Vec::new(), order: Vec::new(), limit: None, ..self.clone() }
    }

    /// The SELECTs of a compound, each a query of its own (with the WITH
    /// clause): [`Query::core`], then each member's.
    pub fn cores(&self) -> Vec<Query> {
        std::iter::once(self.core()).chain(self.compound.iter().map(|(_, m)| Query { with: self.with.clone(), ..m.clone() })).collect()
    }

    /// How many columns the result has (`*`: every column of every
    /// source, a USING or NATURAL join's columns once).
    pub fn width(&self, schema: &Schema) -> usize {
        match &self.items {
            Items::Star => {
                let cols: Vec<usize> = self.sources().iter().map(|t| schema.info(*t).map_or(0, |t| t.columns)).collect();
                let shared = self.joins.first().map_or(0, |j| j.using(cols[0], cols[1]).len());
                cols.iter().sum::<usize>().saturating_sub(shared)
            },
            Items::Exprs(es) => es.len(),
            Items::Agg(..) => 1,
        }
    }

    /// Whether the ORDER BY orders the result rows totally: `Some(true)`
    /// when the query is plain and the terms include the unique key of
    /// every source (its rowid, or every column of a WITHOUT ROWID table's
    /// primary key), so no two rows tie and the order is fixed;
    /// `Some(false)` when the terms include every result column, so rows
    /// that tie are equal in every column as SQLite compares values (`1`
    /// and `1.0` may still come in either order); `None` otherwise (no
    /// ORDER BY included).
    pub fn total_order(&self, schema: &Schema) -> Option<bool> {
        if self.order.is_empty() {
            return None;
        }
        let keyed = self.plain()
            && self.sources().iter().enumerate().all(|(k, t)| match schema.info(*t) {
                Some(TableInfo { key: RowKey::Rowid, .. }) => self.order.iter().any(|o| o.key == OrderKey::Rowid(k)),
                Some(TableInfo { key: RowKey::Columns(pk), .. }) => {
                    pk.iter().all(|c| self.order.iter().any(|o| o.key == OrderKey::Expr(Expr::col(k, *c))))
                },
                _ => false,
            });
        if keyed {
            return Some(true);
        }
        let all = (0..self.width(schema)).all(|c| self.order.iter().any(|o| o.key == OrderKey::Column(c)));
        all.then_some(false)
    }

    /// Whether the query refers to a column of the query `n` levels out
    /// from it (its own expressions at `n`, its subqueries' at `n + 1`).
    pub fn refers(&self, n: usize) -> bool {
        self.exprs().iter().any(|e| e.refers(n))
    }

    /// [`Expr::map_level`] on every expression.
    pub fn map_level(&mut self, n: usize, f: &dyn Fn(Col) -> Option<Col>) {
        for e in self.exprs_mut() {
            *e = e.map_level(n, f);
        }
    }

    /// Subqueries in the query, nested ones and compound members' included.
    pub fn subqueries(&self) -> usize {
        let mut n = 0;
        for e in self.exprs() {
            e.walk(&mut |x| n += usize::from(x.subquery().is_some()));
        }
        n + self.compound.iter().map(|(_, m)| m.subqueries()).sum::<usize>()
    }

    /// The deepest nesting of subqueries in the query (compound members'
    /// included): 0 without any.
    pub fn nesting(&self) -> usize {
        let mut d = 0;
        for e in self.exprs() {
            e.walk(&mut |x| {
                if let Some(q) = x.subquery() {
                    d = d.max(1 + q.nesting());
                }
            });
        }
        self.compound.iter().map(|(_, m)| m.nesting()).fold(d, usize::max)
    }

    /// The database tables the query reads, its subqueries', compound
    /// members' and common table expressions' included.
    pub fn tables(&self) -> Vec<usize> {
        let mut v = self.sources();
        for e in self.exprs() {
            e.walk(&mut |x| {
                if let Some(q) = x.subquery() {
                    v.extend(q.tables());
                }
            });
        }
        v.extend(self.compound.iter().flat_map(|(_, m)| m.tables()));
        for cte in &self.with {
            if let CteBody::Select(b) = &cte.body {
                v.extend(b.tables());
            }
        }
        v.retain(|t| *t < CTE);
        v.sort_unstable();
        v.dedup();
        v
    }

    // ------------------------------------------------------------ rendering

    /// Where the query's expressions render at nesting `depth`.
    pub fn at(&self, depth: usize) -> At {
        let main = self
            .sources()
            .iter()
            .take(8)
            .enumerate()
            .filter(|(k, t)| **t < CTE && !wrap_pick(**t, depth, *k))
            .fold(0, |m, (k, _)| m | 1 << k);
        let (temp, aux) = DBS.with(|d| {
            let d = d.borrow();
            if d.0.iter().all(|&db| db == Db::Main) {
                return (0, 0);
            }
            let of = |db: Db| self.sources().iter().take(8).enumerate().filter(|(_, t)| d.0.get(**t) == Some(&db)).fold(0, |m, (k, _)| m | 1 << k) & main;
            (of(Db::Temp), of(Db::Aux))
        });
        At { depth, sources: 1 + self.joins.len(), main, temp, aux, ..At::TOP }
    }

    /// `e`, an expression of this query, rendered at nesting `depth`.
    pub fn e<'a>(&self, e: &'a Expr, depth: usize) -> Sql<'a> {
        Sql(e, self.at(depth))
    }

    /// Whether an ORDER BY term names its result column by alias
    /// ([`By::Alias`]): the items are then named `r{k}`.
    fn aliased(&self) -> bool {
        self.order.iter().any(|o| o.by == By::Alias && matches!(o.key, OrderKey::Column(_)))
    }

    /// The expression of result column `k` that a [`By::Expr`] term writes
    /// in its place, when it may.
    fn by_expr(&self, k: usize) -> Option<&Expr> {
        let Items::Exprs(es) = &self.items else { return None };
        let e = es.get(k)?;
        let mut bad = int_const(e) || matches!(e, Expr::Lit(_));
        // In a compound, never with a COLLATE: SQLite then rewrites the
        // compound as a subquery and moves its last SELECT's GROUP BY,
        // HAVING and DISTINCT out of it (bugs/sqlite/
        // sqlite-compound-orderby-collate-groupby, fixed after the vendored
        // SQLite).
        if !self.compound.is_empty() {
            e.walk(&mut |x| bad = bad || matches!(x, Expr::Collate(..)));
        }
        e.walk(&mut |x| bad = bad || matches!(x, Expr::Window(_) | Expr::Agg(..) | Expr::Ordered(_) | Expr::Call(Func::JsonEach | Func::JsonTree, _)) || x.subquery().is_some());
        let text = Sql(e, self.at(0)).to_string();
        let earlier = es[..k].iter().any(|x| Sql(x, self.at(0)).to_string() == text);
        (!bad && !earlier).then_some(e)
    }

    /// The items at `at`.
    fn items_at(&self, at: At) -> String {
        match &self.items {
            Items::Star => "*".to_string(),
            Items::Exprs(es) if at.named => {
                es.iter().enumerate().map(|(k, e)| format!("{} AS c{k}", Sql(e, At { named: false, ..at }))).collect::<Vec<_>>().join(", ")
            },
            Items::Exprs(es) if self.aliased() => es.iter().enumerate().map(|(k, e)| format!("{} AS r{k}", Sql(e, at))).collect::<Vec<_>>().join(", "),
            Items::Exprs(es) => es.iter().map(|e| Sql(e, at).to_string()).collect::<Vec<_>>().join(", "),
            Items::Agg(a, None) => format!("{}(*)", a.name()),
            Items::Agg(a, Some(e)) => format!("{}({})", a.name(), Sql(e, at)),
        }
    }

    /// The FROM clause at `at`, a hint per source.
    ///
    /// With joins, a fifth of the time the first `m` sources (two or more,
    /// all of them included) are written in parentheses, `(t0 AS a0 JOIN
    /// t1 AS a1 ON e) JOIN ..`: SQLite joins left to right, so the group is
    /// the same join, read as a subquery in FROM whose columns keep their
    /// sources' names (`isNestedFrom`, resolved by `lookupName` through
    /// its items) and which the flattener then undoes. Picked by the
    /// unparenthesized text, so that the oracles' variants of the query
    /// that keep its FROM clause group it alike.
    fn sources_at(&self, at: At, hints: &[String]) -> String {
        let hint = |k: usize| hints.get(k).map_or("", |h| h.as_str());
        let first = self.source_at(0, at, hint(0));
        let joins: Vec<String> = (0..self.joins.len()).map(|k| self.join_at(k, at, hint(k + 1))).collect();
        let flat = first.clone() + &joins.concat();
        if joins.is_empty() || spell(&flat, 5) != 0 {
            return flat;
        }
        // With inner joins only, half of the groups are the last two
        // sources instead, `.. JOIN (t1 AS a1 JOIN t2 AS a2 ON e2) ON e1`,
        // where `e2` reads no earlier source: inner joins are associative
        // and their ON terms filters, so the rows and the columns' order
        // stay; a nested FROM on the right of a join.
        let n = joins.len();
        if n >= 2 && spell(&flat, 2) == 0 && self.right_group_ok() {
            let j = &self.joins[n - 2];
            let inner = self.source_at(n - 1, at, hint(n - 1));
            let kw = join_spelled(j.keyword(), &inner);
            let on = j.on().map_or(String::new(), |e| format!(" ON {}", Sql(e, at)));
            let last = &joins[n - 1];
            return format!("{first}{} {kw} ({inner}{last}){on}", joins[..n - 2].concat());
        }
        let m = 1 + spell(&joins.concat(), joins.len() as u64) as usize;
        format!("({first}{}){}", joins[..m].concat(), joins[m..].concat())
    }

    /// `t AS a{k}{hint}`: source `k` at `at`; for a table of the database
    /// whose rowid the query never reads, a sixth of the time (by table,
    /// depth and position: [`wrap_pick`]) `(SELECT * FROM t{hint}) AS a{k}`,
    /// the same rows and columns (their affinities and collating sequences
    /// with them) as a subquery in FROM, which the flattener mostly undoes
    /// (`flattenSubquery`, under each join kind's conditions) and which
    /// has no rowid.
    fn source_at(&self, k: usize, at: At, hint: &str) -> String {
        let t = self.sources()[k];
        if self.wraps(k, at.depth) {
            // Half of them with `LIMIT -1` (no limit), which keeps a
            // subquery on the right of a join, or in an aggregate or a
            // compound, from being flattened: a co-routine or a
            // materialized view, and the WHERE terms pushed down into it.
            let limit = if spell(&format!("limit{t}.{}.{k}", at.depth), 2) == 0 { " LIMIT -1" } else { "" };
            format!("(SELECT * FROM {}{hint}{limit}) AS {}", source_name(t, at, k), alias(at.depth, k))
        } else {
            let name = source_name(t, at, k);
            let a = alias(at.depth, k);
            // `AS` left out a quarter of the time (by the source's text).
            let as_ = if spell(&format!("{name} {a}"), 4) == 0 { "" } else { "AS " };
            format!("{name} {as_}{a}{hint}")
        }
    }

    /// Whether the last two sources may be grouped on the right
    /// ([`Query::sources_at`]): every join inner or cross, the last one's
    /// ON reading neither an earlier source nor anything at all of the
    /// query's from a subquery (a correlated column is fine).
    fn right_group_ok(&self) -> bool {
        let n = self.joins.len();
        if n < 2 || !self.joins.iter().all(|j| matches!(j.kind, JoinKind::Inner(_) | JoinKind::Cross)) {
            return false;
        }
        fn early(e: &Expr, d: usize, first: usize) -> bool {
            match e {
                Expr::Col(c) => c.up == d && c.src < first,
                _ => e.children().iter().any(|c| early(c, d, first)) || e.subquery().is_some_and(|q| q.exprs().iter().any(|x| early(x, d + 1, first))),
            }
        }
        self.joins[n - 1].on().is_none_or(|e| !early(e, 0, n - 1))
    }

    /// Whether the query reads the rowid of its source `k` (an ORDER BY
    /// key, a window's, an expression's, its subqueries' included).
    fn reads_rowid(&self, k: usize) -> bool {
        fn expr(e: &Expr, d: usize, k: usize) -> bool {
            match e {
                Expr::Col(c) => c.col == ROWID && c.up == d && c.src == k,
                _ => e.children().iter().any(|c| expr(c, d, k)) || e.subquery().is_some_and(|q| q.exprs().iter().any(|x| expr(x, d + 1, k))),
            }
        }
        self.order.iter().any(|o| o.key == OrderKey::Rowid(k)) || self.exprs().iter().any(|e| expr(e, 0, k))
    }

    /// The sources the query at nesting `depth` reads through a subquery
    /// in FROM ([`Query::source_at`]), bit `k` for source `k`.
    pub fn wrapped(&self, depth: usize) -> u64 {
        (0..self.sources().len().min(64)).filter(|&k| self.wraps(k, depth)).fold(0, |m, k| m | 1 << k)
    }

    /// Whether source `k` of the query at nesting `depth` is read through
    /// a subquery in FROM ([`Query::source_at`]).
    fn wraps(&self, k: usize, depth: usize) -> bool {
        let t = self.sources()[k];
        t < CTE && wrap_pick(t, depth, k) && !self.reads_rowid(k)
    }

    /// ` KEYWORD t AS a{k + 1}{hint} [ON e | USING (..)]`: join `k` at `at`.
    fn join_at(&self, k: usize, at: At, hint: &str) -> String {
        let j = &self.joins[k];
        let source = self.source_at(k + 1, at, hint);
        let mut out = format!(" {} {source}", join_spelled(j.keyword(), &source));
        if let Some(on) = j.on() {
            out += &format!(" ON {}", Sql(on, at));
        }
        if let JoinKind::Using(_, cols) = &j.kind {
            out += &format!(" USING ({})", cols.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", "));
        }
        out
    }

    /// Joins `from..` of the top-level query, as in its FROM clause.
    pub fn joins_sql(&self, from: usize) -> String {
        (from..self.joins.len()).map(|k| self.join_at(k, self.at(0), "")).collect()
    }

    /// WHERE, GROUP BY and HAVING at `at`.
    fn rest_at(&self, at: At) -> String {
        let mut out = self.filter.as_ref().map_or(String::new(), |p| format!(" WHERE {}", Sql(p, at)));
        if !self.group.is_empty() {
            // With its items named `r{k}` ([`By::Alias`]), a term that is an
            // item by its name: SQLite reads the item in its place
            // (`resolveAlias`).
            let items = match &self.items {
                Items::Exprs(es) if self.aliased() && !at.named => &es[..],
                _ => &[],
            };
            let term = |k: &Expr| match items.iter().position(|e| e == k) {
                Some(i) => format!("r{i}"),
                None => Sql(k, at).to_string(),
            };
            out += &format!(" GROUP BY {}", self.group.iter().map(term).collect::<Vec<_>>().join(", "));
        }
        if let Some(h) = &self.having {
            out += &format!(" HAVING {}", Sql(h, at));
        }
        out
    }

    /// A rowid or expression ORDER BY key as an expression at `at`
    /// (qualified when `at` has several sources); a result column's number.
    fn key_at(&self, key: &OrderKey, at: At) -> String {
        match key {
            OrderKey::Column(k) => format!("{}", k + 1),
            OrderKey::Rowid(k) => format!("{}.rowid", alias(at.depth, *k)),
            OrderKey::Expr(e) => Sql(e, at).to_string(),
        }
    }

    /// An ORDER BY key of the top-level query as an expression, its columns
    /// qualified (for a result column: its number).
    pub fn key_sql(&self, key: &OrderKey) -> String {
        self.key_at(key, At { sources: 2, ..self.at(0) })
    }

    /// ORDER BY and LIMIT at `at`.
    /// ` WINDOW w AS (..), ..`: the named windows its items read
    /// ([`window_over`]), rendered at `at` as the items are; or nothing.
    pub fn windows_at(&self, at: At) -> String {
        let s = |e: &Expr| Sql(e, at).to_string();
        let mut defs: Vec<(String, String)> = Vec::new();
        for e in self.items.exprs() {
            if let Expr::Window(w) = e {
                if let Some(d) = window_over(w, &s).1 {
                    if !defs.contains(&d) {
                        defs.push(d);
                    }
                }
            }
        }
        if defs.is_empty() {
            return String::new();
        }
        format!(" WINDOW {}", defs.iter().map(|(n, d)| format!("{n} AS ({d})")).collect::<Vec<_>>().join(", "))
    }

    fn tail_at(&self, at: At) -> String {
        let windows = self.windows_at(at);
        if self.order.is_empty() {
            return windows;
        }
        // Qualified columns: a bare name could be read as a result column's.
        let qualified = At { sources: at.sources.max(2), ..at };
        let terms: Vec<String> = self
            .order
            .iter()
            .map(|o| {
                let key = match (&o.key, o.by) {
                    (OrderKey::Column(k), By::Alias) if matches!(self.items, Items::Exprs(_)) => format!("{}{k}", if at.named { "c" } else { "r" }),
                    (OrderKey::Column(k), By::Expr) => match self.by_expr(*k) {
                        Some(e) => Sql(e, qualified).to_string(),
                        None => format!("{}", k + 1),
                    },
                    (OrderKey::Column(k), _) => format!("{}", k + 1),
                    _ => self.key_at(&o.key, qualified),
                };
                let nulls = match o.nulls {
                    None => "",
                    Some(true) => " NULLS FIRST",
                    Some(false) => " NULLS LAST",
                };
                format!("{key}{}{nulls}", direction(&key, o.desc))
            })
            .collect();
        let mut out = format!("{windows} ORDER BY {}", terms.join(", "));
        match self.limit {
            Some(Limit { count, offset, comma: true }) => out += &format!(" LIMIT {offset}, {count}"),
            Some(Limit { count, offset, comma: false }) => out += &format!(" LIMIT {count} OFFSET {offset}"),
            None => {},
        }
        out
    }

    /// The query at `at`: `hints[0]` a hint per source of its core,
    /// `hints[k]` per source of compound member `k` (none past the ends).
    fn render_at(&self, at: At, hints: &[Vec<String>]) -> String {
        let hint = |k: usize| hints.get(k).map_or(&[][..], |h| h.as_slice());
        let core = |q: &Query, at: At, hints: &[String]| {
            let items = q.items_at(at);
            let distinct = if q.distinct {
                "DISTINCT "
            } else if spell(&items, 8) == 0 {
                "ALL "
            } else {
                ""
            };
            format!("SELECT {distinct}{items} FROM {}{}", q.sources_at(at, hints), q.rest_at(at))
        };
        let mut out = core(self, at, hint(0));
        for (k, (op, m)) in self.compound.iter().enumerate() {
            out += &format!(" {} {}", op.sql(), core(m, m.at(at.depth).within(at), hint(k + 1)));
        }
        out += &self.tail_at(at);
        if self.with.is_empty() {
            out
        } else if at.inline {
            for (i, cte) in self.with.iter().enumerate() {
                out = out.replace(&cte_marker(i), &cte.inline_sql(i, at.noindex));
            }
            out
        } else {
            self.with_at(at.noindex) + &out
        }
    }

    /// `WITH [RECURSIVE] x0[(c0, ..)] AS [[NOT] MATERIALIZED] (..), .. `, or
    /// nothing; `NOT INDEXED` on the CTEs' sources when `noindex`.
    fn with_at(&self, noindex: bool) -> String {
        if self.with.is_empty() {
            return String::new();
        }
        let recursive = self.with.iter().any(|c| matches!(c.body, CteBody::Count { .. }));
        let ctes: Vec<String> = self
            .with
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let width = match &c.body {
                    CteBody::Count { .. } => 1,
                    CteBody::Select(q) => q.items.exprs().len(),
                    CteBody::Vtab(v) => v.columns().len(),
                };
                let columns = if c.columns || matches!(c.body, CteBody::Count { .. }) {
                    format!("({})", (0..width).map(|k| format!("c{k}")).collect::<Vec<_>>().join(", "))
                } else {
                    String::new()
                };
                let hint = match c.hint {
                    None => "",
                    Some(true) => "MATERIALIZED ",
                    Some(false) => "NOT MATERIALIZED ",
                };
                format!("x{i}{columns} AS {hint}({})", c.body_sql(i, noindex))
            })
            .collect();
        format!("WITH {}{} ", if recursive { "RECURSIVE " } else { "" }, ctes.join(", "))
    }

    /// The WITH clause and a space, or nothing: the prefix of any statement
    /// built from the query's clauses.
    pub fn with_sql(&self) -> String {
        self.with_at(false)
    }

    /// The top-level query with every reference to a common table
    /// expression replaced by its query, as a subquery in FROM, and no
    /// WITH clause.
    pub fn sql_inline(&self) -> String {
        self.render_at(At { inline: true, ..self.at(0) }, &[])
    }

    /// The top-level query, `hints` as for [`Query::render_at`].
    pub fn sql_hinted(&self, hints: &[Vec<String>]) -> String {
        self.render_at(self.at(0), hints)
    }

    /// ` ORDER BY .. [LIMIT ..]`, or nothing.
    pub fn tail_sql(&self, depth: usize) -> String {
        self.tail_at(self.at(depth))
    }

    /// The items, rendered at nesting `depth`.
    pub fn items_sql(&self, depth: usize) -> String {
        self.items_at(self.at(depth))
    }

    /// `SELECT [DISTINCT] items`.
    pub fn select_sql(&self, depth: usize) -> String {
        format!("SELECT {}{}", if self.distinct { "DISTINCT " } else { "" }, self.items_sql(depth))
    }

    /// `t0 AS a0{hint} JOIN t1 AS a1{hint} ON e ..`, a hint (such as
    /// ` NOT INDEXED`) per source, none past the end of `hints`.
    pub fn from_sql(&self, depth: usize, hints: &[String]) -> String {
        self.sources_at(self.at(depth), hints)
    }

    /// ` WHERE e`, or nothing.
    pub fn where_sql(&self, depth: usize) -> String {
        self.filter.as_ref().map_or(String::new(), |p| format!(" WHERE {}", self.e(p, depth)))
    }

    /// ` GROUP BY k, ..`, or nothing.
    pub fn group_sql(&self, depth: usize) -> String {
        if self.group.is_empty() {
            return String::new();
        }
        format!(" GROUP BY {}", self.group.iter().map(|k| self.e(k, depth).to_string()).collect::<Vec<_>>().join(", "))
    }

    /// ` HAVING e`, or nothing.
    pub fn having_sql(&self, depth: usize) -> String {
        self.having.as_ref().map_or(String::new(), |h| format!(" HAVING {}", self.e(h, depth)))
    }

    /// The query at nesting `depth`, with a hint per source of its core.
    pub fn render_with(&self, depth: usize, hints: &[String]) -> String {
        self.render_at(self.at(depth), &[hints.to_vec()])
    }

    /// The query at nesting `depth`.
    pub fn render(&self, depth: usize) -> String {
        self.render_with(depth, &[])
    }

    /// The top-level query with `NOT INDEXED` on every source, its
    /// subqueries' and compound members' included.
    pub fn sql_not_indexed(&self) -> String {
        let hints: Vec<Vec<String>> = self.cores().iter().map(|q| vec![" NOT INDEXED".to_string(); 1 + q.joins.len()]).collect();
        self.render_at(At { noindex: true, ..self.at(0) }, &hints)
    }

    /// The top-level query as SQL.
    pub fn sql(&self) -> String {
        self.render(0)
    }

    // ------------------------------------------------------------- measure

    /// The simplicity measure: 1, plus DISTINCT, the items, each join (2,
    /// its kind, its ON clause), the filter, each GROUP BY term, the
    /// HAVING clause, each compound member (2, its operator: 0 for UNION
    /// ALL to 3 for EXCEPT, its cost), each ORDER BY term (1, its key,
    /// DESC, NULLS) and the LIMIT (1, its count and offset up to 1000 each,
    /// the comma form); and each CTE's ([`Cte::cost`]).
    pub fn cost(&self) -> u64 {
        let items = match &self.items {
            Items::Star => 0,
            Items::Exprs(es) => es.iter().map(|e| 1 + e.cost()).sum(),
            Items::Agg(a, e) => 1 + u64::from(*a != Agg::Count) + e.as_ref().map_or(0, Expr::cost),
        };
        let joins: u64 = self
            .joins
            .iter()
            .map(|j| {
                2 + match &j.kind {
                    JoinKind::Cross => 0,
                    JoinKind::Inner(e) => 1 + e.cost(),
                    JoinKind::Left(e) | JoinKind::Right(e) => 2 + e.cost(),
                    JoinKind::Full(e) => 3 + e.cost(),
                    JoinKind::Using(op, cols) => 1 + *op as u64 + cols.len() as u64,
                    JoinKind::Natural(op) => 1 + *op as u64,
                }
            })
            .sum();
        let group: u64 = self.group.iter().map(|k| 1 + k.cost()).sum();
        let opt = |e: &Option<Expr>| e.as_ref().map_or(0, |e| 1 + e.cost());
        let order: u64 = self
            .order
            .iter()
            .map(|o| {
                let key = match &o.key {
                    OrderKey::Column(_) => 1,
                    OrderKey::Rowid(k) => 1 + *k as u64,
                    OrderKey::Expr(e) => e.cost(),
                };
                1 + key + u64::from(o.desc) + u64::from(o.nulls.is_some()) + u64::from(o.by != By::Number)
            })
            .sum();
        let limit = self.limit.map_or(0, |l| 1 + l.count.unsigned_abs().min(1000) + l.offset.unsigned_abs().min(1000) + u64::from(l.comma));
        let compound: u64 = self.compound.iter().map(|(op, m)| 2 + *op as u64 + m.cost()).sum();
        let with: u64 = self.with.iter().map(Cte::cost).sum();
        with + 1 + u64::from(self.distinct) + items + joins + opt(&self.filter) + group + opt(&self.having) + compound + order + limit
    }

    // ------------------------------------------------------------ validity

    /// Valid as a top-level query against `schema`.
    pub fn valid(&self, schema: &Schema) -> bool {
        self.valid_in(schema, &[], Kind::Top)
    }

    /// Valid as a query of kind `kind` inside queries whose visible
    /// sources' tables are `outer` (outermost first):
    ///
    /// - at most [`MAX_LEVELS`] levels; every table exists; every column
    ///   reference names a visible source of its level and a column of its
    ///   table (an ON clause sees the sources up to its own);
    /// - each expression valid ([`MAX_DEPTH`], arities, non-empty lists, no
    ///   NUL in text, no NaN);
    /// - `*` only in a query that does not aggregate; [`Items::Agg`] only at
    ///   the top level, without GROUP BY, DISTINCT or HAVING;
    /// - aggregates ([`Expr::Agg`]) only in the items and HAVING of an
    ///   aggregate query, never nested, their argument over the query's own
    ///   rows (no subquery, no outer column), `min`/`max` of a `CAST` to
    ///   INTEGER or TEXT and `sum` of a `CAST` to INTEGER (see
    ///   [`agg_arg_ok`]); so are those with an ORDER BY ([`Expr::Ordered`],
    ///   see [`ordered_ok`]);
    /// - in the items and HAVING of an aggregate query, a column of the
    ///   query only inside a GROUP BY term or an aggregate ([`grouped_ok`]);
    /// - no GROUP BY term an integer constant (SQLite reads `GROUP BY 2` as
    ///   the second result column); HAVING only with GROUP BY;
    /// - an IN subquery has one expression item; a scalar subquery one item
    ///   with an aggregate and no GROUP BY (so exactly one row);
    /// - compound members only at the top level: each a valid top-level
    ///   query of the same width, with no compound, window, ORDER BY or
    ///   LIMIT of its own; neither it nor the first SELECT one aggregate
    ///   `sum()`;
    /// - window functions ([`Expr::Window`]) only as items of a top-level
    ///   query that neither aggregates nor has DISTINCT nor is a compound,
    ///   each as [`window_ok`] says;
    /// - ORDER BY only at the top level, not with [`Items::Agg`] (one row)
    ///   but in a compound,
    ///   and only a total order
    ///   ([`Query::total_order`]); its terms result column numbers in range,
    ///   or in a plain query ([`Query::plain`]) rowids of sources with one
    ///   and row expressions without a subquery that are not integer
    ///   constants (a column number); LIMIT only with ORDER BY.
    pub fn valid_in(&self, schema: &Schema, outer: &[Vec<usize>], kind: Kind) -> bool {
        if self.with.is_empty() {
            return self.valid_scoped(schema, outer, kind);
        }
        kind == Kind::Top && self.scope(schema).is_some_and(|scoped| self.valid_scoped(&scoped, outer, kind))
    }

    /// USING and NATURAL only in the first join, USING of distinct columns
    /// both sources have; and not `*` with one when the first source is a
    /// left operand of a RIGHT or FULL join (that join, or a later one) and
    /// a later source has a column of a USING column's name. SQLite writes
    /// such a column of `*` as the bare name (`selectExpander`, for a table
    /// flagged `JT_LTORJ`, so that the name resolves to the USING columns
    /// coalesced), which the later source's column makes ambiguous:
    /// "ambiguous column name: c0" for `SELECT * FROM t0 FULL JOIN t1 USING
    /// (c0) CROSS JOIN t2`, a query SQLite should accept
    /// (bugs/sqlite/sqlite-right-join-using-star-ambiguous).
    fn using_ok(&self, schema: &Schema) -> bool {
        let columns: Vec<usize> = self.sources().iter().map(|t| schema.info(*t).map_or(0, |i| i.columns)).collect();
        self.joins.iter().enumerate().all(|(k, j)| match &j.kind {
            JoinKind::Using(_, cols) => {
                let mut seen = cols.clone();
                seen.sort_unstable();
                seen.dedup();
                k == 0 && !cols.is_empty() && seen.len() == cols.len() && cols.iter().all(|c| *c < columns[0].min(columns[1]))
            },
            JoinKind::Natural(_) => k == 0,
            _ => true,
        }) && match self.joins.first() {
            Some(j) if self.items == Items::Star && self.joins.iter().any(|j| j.kind.outer_right()) => {
                let using = j.using(columns[0], columns[1]);
                columns[2..].iter().all(|&n| using.iter().all(|c| *c >= n))
            },
            _ => true,
        }
    }

    /// The schema the query's sources see: `schema` (which has no CTEs of
    /// its own) with the query's common table expressions ([`Schema::ctes`],
    /// see [`Cte::info`]); `None` when one is not valid.
    pub fn scope(&self, schema: &Schema) -> Option<Schema> {
        if !schema.ctes.is_empty() {
            return None;
        }
        let ctes = self.with.iter().map(|c| c.info(schema)).collect::<Option<Vec<_>>>()?;
        Some(Schema { ctes, ..schema.clone() })
    }

    /// Whether the query reads within the row budget it is drawn with: its
    /// sources' row product within [`ROW_BUDGET`], each subquery within what
    /// [`draw_sub`] gives it (the budget over that product when it is
    /// correlated, at least [`ROW_BUDGET`] / 10 when not), and each compound
    /// member and CTE body within [`ROW_BUDGET`]; each window's frames
    /// within their budget ([`Window::frames_fit`]). Every drawn query fits; a
    /// query kept after its case's tables grew may not.
    pub fn fits(&self, schema: &Schema) -> bool {
        let Some(scoped) = self.scope(schema) else {
            return false;
        };
        let bodies = self.with.iter().all(|c| match &c.body {
            CteBody::Select(b) => b.fits_in(schema, ROW_BUDGET),
            CteBody::Count { .. } | CteBody::Vtab(_) => true,
        });
        bodies && self.fits_in(&scoped, ROW_BUDGET) && self.compound.iter().all(|(_, m)| m.fits_in(&scoped, ROW_BUDGET)) && self.concat_ctes_fit(schema, &scoped)
    }

    /// A CTE body that concatenates its rows (`group_concat`, `string_agg`,
    /// `json_group_array`, `json_group_object` with an ORDER BY of their
    /// own) gives a text as long as the rows it reads, which every row of a
    /// query reading the CTE carries: the rows it reads times the outer
    /// query's sources' rows within [`CONCAT_FRAME_ROWS`], as for such a
    /// window. Past it, examples/heavy.rs tlp seed 7500113 with --queries
    /// 1-300 --mutate 4 (q222: a `json_group_array` over a 20000-row RIGHT
    /// JOIN, read by a DISTINCT cross join of the same two tables): 2.9 GB.
    fn concat_ctes_fit(&self, schema: &Schema, scoped: &Schema) -> bool {
        fn concat(e: &Expr) -> bool {
            matches!(e, Expr::Ordered(o) if matches!(o.func, OrdFunc::GroupConcat | OrdFunc::StringAgg | OrdFunc::JsonGroupArray | OrdFunc::JsonGroupObject)) || e.children().into_iter().any(concat)
        }
        let rows = |q: &Query, sc: &Schema| q.sources().iter().map(|t| sc.info(*t).map_or(u64::MAX, |i| i.rows.max(1) as u64)).fold(1u64, u64::saturating_mul);
        self.with.iter().enumerate().all(|(i, c)| match &c.body {
            CteBody::Select(b) if b.exprs().into_iter().any(concat) => {
                let width = rows(b, schema);
                [self].into_iter().chain(self.compound.iter().map(|(_, m)| m)).filter(|q| q.reads(CTE + i)).all(|q| rows(q, scoped).saturating_mul(width) <= CONCAT_FRAME_ROWS)
            },
            _ => true,
        })
    }

    /// [`Query::fits`] for this query's own SELECT, within `budget`.
    fn fits_in(&self, schema: &Schema, budget: u64) -> bool {
        let product = self
            .sources()
            .iter()
            .map(|t| schema.info(*t).map_or(u64::MAX, |i| i.rows.max(1) as u64))
            .fold(1u64, u64::saturating_mul);
        if product > budget {
            return false;
        }
        let single = self.joins.is_empty();
        if !self.items.exprs().iter().all(|e| !matches!(e, Expr::Window(w) if !w.frames_fit(product, single))) {
            return false;
        }
        let inner = budget / product;
        fn subqueries<'a>(e: &'a Expr, out: &mut Vec<&'a Query>) {
            out.extend(e.subquery());
            for c in e.children() {
                subqueries(c, out);
            }
        }
        let mut subs = Vec::new();
        for e in self.exprs() {
            subqueries(e, &mut subs);
        }
        subs.iter().all(|q| q.fits_in(schema, if correlated(q) { inner } else { inner.max(ROW_BUDGET / 10) }))
    }

    /// [`Query::valid_in`] once the WITH clause is in `schema`.
    fn valid_scoped(&self, schema: &Schema, outer: &[Vec<usize>], kind: Kind) -> bool {
        if outer.len() >= MAX_LEVELS {
            return false;
        }
        let sources = self.sources();
        if sources.iter().any(|t| schema.info(*t).is_none()) || !self.using_ok(schema) {
            return false;
        }
        let mut levels = outer.to_vec();
        for k in 0..self.joins.len() {
            if let Some(on) = self.joins[k].on() {
                levels.push(sources[..k + 2].to_vec());
                let ok = valid_expr(on, schema, &levels, false);
                levels.pop();
                if !ok {
                    return false;
                }
            }
        }
        levels.push(sources);
        let row = |e: &Expr| valid_expr(e, schema, &levels, false);
        let grouped = |e: &Expr| valid_expr(e, schema, &levels, true) && grouped_ok(e, &self.group);
        let aggregate = self.aggregate();
        let shape = match &self.items {
            Items::Star => !aggregate,
            Items::Exprs(es) => {
                // Windows only in a top-level query that is not DISTINCT nor
                // a compound (nor aggregates: `aggregate` is false with one).
                let windows = kind == Kind::Top && !self.distinct && self.compound.is_empty();
                !es.is_empty()
                    && es.iter().all(|e| match e {
                        Expr::Window(w) => windows && !aggregate && window_ok(w, schema, &levels),
                        _ if aggregate => grouped(e),
                        _ => row(e),
                    })
            },
            Items::Agg(a, e) => {
                kind == Kind::Top
                    && self.group.is_empty()
                    && !self.distinct
                    && match e {
                        None => *a == Agg::Count,
                        Some(e) => row(e),
                    }
            },
        };
        let kind_ok = match kind {
            Kind::Top | Kind::Exists => true,
            Kind::In => matches!(&self.items, Items::Exprs(es) if es.len() == 1),
            Kind::Row(n) => matches!(&self.items, Items::Exprs(es) if es.len() == n),
            Kind::Scalar => self.group.is_empty() && matches!(&self.items, Items::Exprs(es) if es.len() == 1 && es[0].has_agg()),
        };
        let (plain, width) = (self.plain(), self.width(schema));
        let order_ok = |o: &Order| match &o.key {
            OrderKey::Column(c) => *c < width,
            OrderKey::Rowid(k) => {
                plain && levels.last().unwrap().get(*k).is_some_and(|t| schema.info(*t).is_some_and(|i| i.key == RowKey::Rowid))
            },
            OrderKey::Expr(e) => plain && row(e) && !int_const(e) && !has_subquery(e),
        };
        // A compound's members: cores of the same width, not one aggregate
        // `sum()` (whose real sum depends on the order rows are summed in).
        let summed = |q: &Query| matches!(q.items, Items::Agg(Agg::Sum, _));
        let compound = self.compound.is_empty()
            || (kind == Kind::Top
                && !summed(self)
                && self.compound.iter().all(|(_, m)| {
                    m.compound.is_empty()
                        && !m.has_window()
                        && m.order.is_empty()
                        && m.limit.is_none()
                        && !summed(m)
                        && m.valid_in(schema, outer, Kind::Top)
                        && m.width(schema) == width
                }));
        let ordered = (self.order.is_empty() && self.limit.is_none())
            || (kind == Kind::Top
                && (!matches!(self.items, Items::Agg(..)) || !self.compound.is_empty())
                && self.order.iter().all(order_ok)
                && self.total_order(schema).is_some());
        shape
            && kind_ok
            && self.filter.iter().all(row)
            && self.group.iter().all(|k| row(k) && !int_const(k))
            && self.having.iter().all(|h| !self.group.is_empty() && grouped(h))
            && compound
            && ordered
    }

    // -------------------------------------------------------------- repair

    /// The query without source `k` (`None` when it is the only one):
    /// its join and ON clause go (the first source's when `k` is 0, the
    /// next source becoming the first), its columns become NULL and later
    /// sources' columns are renumbered, in subqueries too; so is its rowid
    /// among the ORDER BY terms. Cheaper. (Result column numbers in ORDER
    /// BY are left as they are: with `*` some may now be out of range.)
    pub fn drop_source(&self, k: usize) -> Option<Query> {
        if self.joins.is_empty() || k > self.joins.len() {
            return None;
        }
        let mut q = self.clone();
        if k == 0 {
            q.table = q.joins.remove(0).table;
        } else {
            q.joins.remove(k - 1);
        }
        q.map_level(0, &|c| match c.src.cmp(&k) {
            std::cmp::Ordering::Less => Some(c),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(Col { src: c.src - 1, ..c }),
        });
        q.order.retain(|o| o.key != OrderKey::Rowid(k));
        for o in &mut q.order {
            if let OrderKey::Rowid(s) = &mut o.key {
                *s -= usize::from(*s > k);
            }
        }
        Some(q)
    }

    /// Every source table number `t` (of the query, its subqueries and
    /// compound members, not its CTEs' queries) mapped by `f`.
    fn map_tables(&mut self, f: &dyn Fn(usize) -> usize) {
        self.table = f(self.table);
        for j in &mut self.joins {
            j.table = f(j.table);
        }
        for (_, m) in &mut self.compound {
            m.map_tables(f);
        }
        for e in self.exprs_mut() {
            map_expr_tables(e, f);
        }
    }

    /// Whether the query, its subqueries or compound members read source
    /// table `t`.
    fn reads(&self, t: usize) -> bool {
        self.count_reads(t) > 0
    }

    /// How many sources of the query, its subqueries and compound members
    /// read source table `t`.
    pub fn count_reads(&self, t: usize) -> usize {
        let mut n = self.sources().iter().filter(|s| **s == t).count();
        n += self.compound.iter().map(|(_, m)| m.count_reads(t)).sum::<usize>();
        for e in self.exprs() {
            e.walk(&mut |x| n += x.subquery().map_or(0, |q| q.sources().iter().filter(|s| **s == t).count()));
        }
        n
    }

    /// The query without CTE `i`, when nothing reads it (later CTEs
    /// renumbered). Cheaper.
    pub fn drop_cte(&self, i: usize) -> Option<Query> {
        if i >= self.with.len() || self.reads(CTE + i) {
            return None;
        }
        let mut q = self.clone();
        q.with.remove(i);
        q.map_tables(&|t| if t > CTE + i { t - 1 } else { t });
        Some(q)
    }

    /// The query against a smaller schema: `None` when no source's table
    /// is left or what is left is not valid; else sources whose table is
    /// gone dropped ([`Query::drop_source`]), columns out of range NULL,
    /// subqueries repaired the same way (a subquery with no table left
    /// becomes NULL), compound members repaired (dropped with no table
    /// left or of another width), ORDER BY terms out of range dropped (the
    /// whole ORDER BY and LIMIT when the rest is not a total order). Never
    /// costlier.
    pub fn repair(&self, schema: &Schema) -> Option<Query> {
        self.repair_in(schema, &[]).filter(|q| q.valid(schema))
    }

    fn repair_in(&self, schema: &Schema, outer: &[Vec<usize>]) -> Option<Query> {
        let mut q = self.clone();
        // The CTEs' queries first, against the database's tables; a CTE with
        // no table left, or not valid now, takes the query with it.
        let scoped;
        let schema = if q.with.is_empty() {
            schema
        } else {
            for cte in &mut q.with {
                if let CteBody::Select(b) = &mut cte.body {
                    **b = b.repair_in(schema, &[])?;
                }
            }
            scoped = q.scope(schema)?;
            &scoped
        };
        for k in (0..=q.joins.len()).rev() {
            if schema.info(q.sources()[k]).is_none() {
                q = q.drop_source(k)?;
            }
        }
        let mut levels = outer.to_vec();
        levels.push(q.sources());
        for e in q.exprs_mut() {
            *e = repair_expr(e, schema, &levels);
        }
        // Compound members are repaired as queries of their own; those with no
        // table left, or of another width now, go.
        let width = q.width(schema);
        let members = std::mem::take(&mut q.compound);
        q.compound = members
            .into_iter()
            .filter_map(|(op, m)| m.repair_in(schema, outer).filter(|m| m.width(schema) == width).map(|m| (op, m)))
            .collect();
        // ORDER BY terms that no longer apply go, and the ORDER BY with them
        // when what is left is not a total order.
        let (plain, sources) = (q.plain(), q.sources());
        q.order.retain(|o| match &o.key {
            OrderKey::Column(c) => *c < width,
            OrderKey::Rowid(k) => plain && sources.get(*k).is_some_and(|t| schema.info(*t).is_some_and(|i| i.key == RowKey::Rowid)),
            OrderKey::Expr(e) => plain && !int_const(e),
        });
        if q.total_order(schema).is_none() {
            q.order.clear();
            q.limit = None;
        }
        Some(q)
    }
}

/// Whether column `c` names a source of its level in `levels` (outermost
/// first, the expression's own last) and a column of that source's table.
fn col_ok(c: &Col, schema: &Schema, levels: &[Vec<usize>]) -> bool {
    let info = if c.up < levels.len() { levels[levels.len() - 1 - c.up].get(c.src).and_then(|t| schema.info(*t)) } else { None };
    info.is_some_and(|t| if c.col == ROWID { t.key == RowKey::Rowid } else { c.col < t.columns })
}

/// Whether the ORDER BY terms `keys` order the rows of a query with the
/// sources `sources` totally: they include every source's key (its rowid,
/// [`ROWID`], or every column of its primary key).
fn keyed(keys: &[&Expr], schema: &Schema, sources: &[usize]) -> bool {
    sources.iter().enumerate().all(|(k, t)| match schema.info(*t).map(|i| &i.key) {
        Some(RowKey::Rowid) => keys.contains(&&Expr::col(k, ROWID)),
        Some(RowKey::Columns(pk)) => pk.iter().all(|c| keys.contains(&&Expr::col(k, *c))),
        _ => false,
    })
}

/// A window item of a query whose visible sources' tables are `levels`
/// (its own last): its argument as [`Window`] says, of the query's own
/// rows ([`agg_arg_ok`]); its PARTITION BY and ORDER BY terms row
/// expressions without subqueries; ORDER BY terms expressions only; a
/// valid frame ([`Frame::ok`]) only for an aggregate; for `row_number`
/// and a ROWS frame a total order ([`keyed`]).
fn window_ok(w: &Window, schema: &Schema, levels: &[Vec<usize>]) -> bool {
    let row = |e: &Expr| valid_expr(e, schema, levels, false) && !has_subquery(e);
    let value = |a: &Expr| local(a) && row(a);
    let arg = match (w.func, &w.arg) {
        (WinFunc::RowNumber | WinFunc::Rank | WinFunc::DenseRank | WinFunc::PercentRank | WinFunc::CumeDist, None) => true,
        (WinFunc::Ntile(n), None) => (1..=8).contains(&n),
        (WinFunc::FirstValue | WinFunc::LastValue, Some(a)) => value(a),
        (WinFunc::NthValue(n), Some(a)) => (1..=4).contains(&n) && value(a),
        (WinFunc::Lag(n) | WinFunc::Lead(n), Some(a)) => (1..=3).contains(&n) && value(a),
        (WinFunc::Count, None) => true,
        (WinFunc::Count, Some(a)) => agg_arg_ok(Agg::Count, Some(a)) && row(a),
        (WinFunc::Sum | WinFunc::JsonGroupArray, Some(a)) => agg_arg_ok(Agg::Sum, Some(a)) && row(a),
        (WinFunc::Min | WinFunc::Max, Some(a)) => agg_arg_ok(Agg::Min, Some(a)) && row(a),
        (WinFunc::Avg | WinFunc::Total, Some(a)) => small_int(a) && value(a),
        (WinFunc::GroupConcat(k), Some(a)) => (k as usize) < GROUP_SEPS.len() && matches!(a, Expr::Case(_, _, Some(x)) if matches!(**x, Expr::Col(_)) && *a == unblob((**x).clone())) && value(a),
        _ => false,
    };
    let default = match (w.func, &w.default) {
        (_, None) => true,
        (WinFunc::Lag(_) | WinFunc::Lead(_), Some(d)) => value(d),
        _ => false,
    };
    let filter = w.filter.as_ref().is_none_or(|f| w.func.aggregate() && value(f));
    let keys = w.keys();
    let total = w.func.needs_total() || w.frame.is_some_and(|f| f.mode == FrameMode::Rows);
    let range_offsets = matches!(keys.as_slice(), [k] if int_key(k));
    arg && default && filter
        && w.arg.as_ref().is_none_or(|a| casts_binary(a, schema, levels))
        && w.partition.iter().all(row)
        && keys.len() == w.order.len()
        && keys.iter().all(|e| row(e))
        && w.frame.is_none_or(|f| w.func.framed() && f.ok(range_offsets))
        && (!total || keyed(&keys, schema, levels.last().map_or(&[][..], |l| l.as_slice())))
}

/// `CAST(x AS INTEGER) % n`, 1 <= n <= 1000: an integer (or NULL) small
/// enough that a sum of a table's rows of it is exact, so that `avg` and
/// `total` do not depend on the order of rows.
pub fn small_int(e: &Expr) -> bool {
    matches!(e, Expr::Bin(BinOp::Rem, x, n) if matches!(**x, Expr::Cast(_, CastType::Integer)) && matches!(**n, Expr::Lit(Value::Int(1..=1000))))
}

/// A window ORDER BY term whose values are integers or NULL, as a RANGE
/// frame's offsets need: a rowid, or a `CAST(.. AS INTEGER)`.
pub fn int_key(e: &Expr) -> bool {
    matches!(e, Expr::Col(Col { col: ROWID, .. }) | Expr::Cast(_, CastType::Integer))
}

fn repair_expr(e: &Expr, schema: &Schema, levels: &[Vec<usize>]) -> Expr {
    match e {
        Expr::Col(c) if !col_ok(c, schema, levels) => Expr::Lit(Value::Null),
        _ => {
            let mut e = e.clone();
            for c in e.children_mut() {
                *c = repair_expr(c, schema, levels);
            }
            if let Some(q) = e.subquery() {
                match q.repair_in(schema, levels) {
                    Some(r) => *e.subquery_mut().unwrap() = r,
                    None => return Expr::Lit(Value::Null),
                }
            }
            e
        },
    }
}

/// An integer constant as SQLite's `sqlite3ExprIsInteger` reads one after
/// parsing: an integer literal, under unary minuses; or an AND with an
/// operand that is the literal 0 (or such an AND), which the parser folds
/// into the integer 0 (`sqlite3ExprAnd`: an operand flagged `EP_IsFalse`,
/// as an integer literal 0 is, makes the AND a literal 0; `GROUP BY (0
/// AND c0)` is "1st GROUP BY term out of range"); or `IS [NOT] NULL` of a
/// non-NULL literal, which the parser folds into 0 or 1 (`GROUP BY ('a' IS
/// NULL)` is out of range, and `GROUP BY (1 IS NOT NULL)` would mean the
/// first result column; `NULL IS NULL` and `NULL IS NOT 0` are not
/// folded), written as `IS [NOT] NULL` or as `IS`/`IS NOT` with a NULL right
/// operand, which the parser reads the same way. The folds nest: the
/// operand of `IS NULL` may be under unary minuses and itself folded
/// (`sqlite3PExprIsNull` skips unary operators and folds any integer
/// node, `((' 2 ' IS NOT NULL) IS NULL)` is 0), and a folded 0 is false to
/// the AND (`(c0 AND ('?' IS NULL))` is 0). Over-approximate (SQLite does
/// not fold an AND with a function call in it, nor `IS NULL` of a real
/// written as an expression): such a term is never generated, no more.
fn int_const(e: &Expr) -> bool {
    // A node the parser leaves as an integer literal, or folds into one.
    let folded = |x: &Expr| !matches!(x, Expr::Neg(_)) && int_const(x);
    // The operand of `IS [NOT] NULL` that makes it fold: under unary
    // minuses, a non-NULL literal or a folded integer.
    let foldable = |x: &Expr| {
        let mut x = x;
        while let Expr::Neg(a) = x {
            x = a;
        }
        matches!(x, Expr::Lit(v) if *v != Value::Null) || folded(x)
    };
    // A node flagged always false: the literal 0, or a fold into 0.
    let zero = |x: &Expr| match x {
        Expr::Lit(Value::Int(0)) => true,
        Expr::IsNull(a, false) | Expr::Bin(BinOp::Is, a, _) => folded(x) && foldable(a),
        Expr::Bin(BinOp::And, ..) => folded(x),
        _ => false,
    };
    match e {
        Expr::Lit(Value::Int(_)) => true,
        // GROUP BY and ORDER BY read a term without its COLLATE
        // (`sqlite3ExprSkipCollateAndLikely`): `GROUP BY (0 COLLATE RTRIM)`
        // is out of range. Over-approximate below a unary minus.
        Expr::Collate(a, _) => int_const(a),
        Expr::IsNull(a, _) => foldable(a),
        // `x IS NULL` and `x IS NOT NULL` parse alike however written.
        Expr::Bin(BinOp::Is | BinOp::IsNot, a, b) => matches!(&**b, Expr::Lit(Value::Null)) && foldable(a),
        Expr::Neg(a) => int_const(a),
        Expr::Bin(BinOp::And, a, b) => zero(a) || zero(b),
        _ => false,
    }
}

/// [`Query::map_tables`] on the subqueries in `e`.
fn map_expr_tables(e: &mut Expr, f: &dyn Fn(usize) -> usize) {
    if let Some(q) = e.subquery_mut() {
        q.map_tables(f);
    }
    for c in e.children_mut() {
        map_expr_tables(c, f);
    }
}

/// Whether a subquery is anywhere in `e`.
fn has_subquery(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |n| found |= n.subquery().is_some());
    found
}

/// Valid where `levels` gives the visible sources' tables of each level,
/// the expression's own query last; aggregates allowed when `aggs`.
fn valid_expr(e: &Expr, schema: &Schema, levels: &[Vec<usize>], aggs: bool) -> bool {
    e.depth() <= MAX_DEPTH && jsonb_read(e, false) && zeroblobs_read_as_blobs(e) && nodes_ok(e, schema, levels, aggs)
}

/// The functions that read a `zeroblob(n)` as the BLOB it is
/// ([`zeroblob_ok`]'s placement): its length, its type, its hex digits,
/// its literal.
pub const BLOB_READERS: [Func; 5] = [Func::Hex, Func::Length, Func::OctetLength, Func::Typeof, Func::Quote];

/// Whether every `zeroblob(n)` in `e` (its subqueries' too) is the
/// argument of one of [`BLOB_READERS`]: never text (`CAST(.. AS TEXT)`,
/// a text function, `||`), a key (GROUP BY, DISTINCT, a compound's) or a
/// comparison, where SQLite gives text with NUL characters no defined
/// result (nulinstr.html) and its zero-filled BLOB (`MEM_Zero`) came out
/// by the plan: `CAST(zeroblob(2) AS TEXT)` as two NULs inline and one
/// materialized (run 20261017-0438 cte-materialized w2-440158), `GROUP BY
/// upper(zeroblob(length(c0)))` as two groups and one with the
/// optimizations off (run 20261016-2205 plans-noopt w3-147506).
fn zeroblobs_read_as_blobs(e: &Expr) -> bool {
    let (mut all, mut read) = (0usize, 0usize);
    e.walk(&mut |n| match n {
        Expr::Call(Func::Zeroblob, _) => all += 1,
        Expr::Call(f, args) if BLOB_READERS.contains(f) => read += args.iter().filter(|a| matches!(a, Expr::Call(Func::Zeroblob, _))).count(),
        _ => {},
    });
    all == read
}

fn nodes_ok(e: &Expr, schema: &Schema, levels: &[Vec<usize>], aggs: bool) -> bool {
    let here = match e {
        Expr::Col(c) => col_ok(c, schema, levels),
        Expr::Lit(v) => value_ok(v),
        Expr::In(_, l, _) => !l.is_empty(),
        Expr::Case(_, ws, _) => !ws.is_empty(),
        Expr::Call(f, args) if f.json() => json_args_ok(*f, args),
        Expr::Call(f, args) if f.date() => date_args_ok(*f, args),
        Expr::Call(f @ (Func::Printf | Func::Format), args) => {
            let (lo, hi) = f.arity();
            (lo..=hi).contains(&args.len()) && matches!(&args[0], Expr::Lit(Value::Text(f)) if format_ok(f))
        },
        Expr::Call(f, args) => {
            let (lo, hi) = f.arity();
            (lo..=hi).contains(&args.len())
                && match f {
                    Func::Unistr => unistr_ok(&args[0]),
                    Func::Zeroblob => zeroblob_ok(&args[0]),
                    Func::Likelihood => matches!(args[1], Expr::Lit(Value::Real(p)) if p.is_sign_positive() && (p == 0.0 || (1e-3..=1.0).contains(&p))),
                    _ => true,
                }
        },
        Expr::Bin(BinOp::Arrow | BinOp::Arrow2, j, p) => json_ok(j) && arrow_ok(p),
        Expr::Agg(a, arg) => {
            return aggs
                && agg_arg_ok(*a, arg.as_deref())
                && casts_binary(e, schema, levels)
                && arg.iter().all(|x| nodes_ok(x, schema, levels, false))
        },
        Expr::Ordered(o) => {
            return aggs && ordered_ok(o) && casts_binary(e, schema, levels) && e.children().iter().all(|x| nodes_ok(x, schema, levels, false))
        },
        Expr::Exists(q, _) => q.valid_in(schema, levels, Kind::Exists),
        Expr::InSelect(_, q, _) => q.valid_in(schema, levels, Kind::In),
        Expr::Scalar(q) => q.valid_in(schema, levels, Kind::Scalar),
        Expr::RowCmp(op, a, b) => {
            (2..=3).contains(&a.len()) && a.len() == b.len() && (BinOp::CMP.contains(op) || matches!(op, BinOp::Is | BinOp::IsNot))
        },
        Expr::RowBetween(a, lo, hi, _) => (2..=3).contains(&a.len()) && lo.len() == a.len() && hi.len() == a.len(),
        // No aggregate in the list: SQLite reads it as VALUES, a query of its
        // own, which `count(*)` would belong to ("misuse of aggregate").
        Expr::RowIn(a, l, _) => (2..=3).contains(&a.len()) && l.len() >= 2 && l.iter().all(|r| r.len() == a.len() && r.iter().all(|e| !e.has_agg())),
        Expr::RowInSelect(a, q, _) => (2..=3).contains(&a.len()) && q.valid_in(schema, levels, Kind::Row(a.len())),
        // Only as an item, checked there ([`window_ok`]).
        Expr::Window(_) => false,
        _ => true,
    };
    here && e.children().iter().all(|c| nodes_ok(c, schema, levels, aggs))
}

/// An aggregate's argument: any for `count`, `CAST(x AS INTEGER)` or
/// `CAST(x AS TEXT)` for `min` and `max`, `CAST(x AS INTEGER)` for `sum`;
/// `x` with no aggregate, no subquery and no column of an outer query.
///
/// The casts make the aggregate's value a function of the equivalence
/// class of its inputs: `min()` and `max()` return the first of equal
/// extremes in visiting order (`1` or `1.0`), which depends on the plan,
/// and expressions on them could tell which (`typeof`); a cast to INTEGER
/// or TEXT maps equal values to identical ones, a cast to TEXT only of a
/// value under BINARY ([`casts_binary`], checked with the schema: `'a'`
/// and `'A'` stay equal under NOCASE). A REAL `sum()` depends on
/// summation order; an INTEGER one does not (overflow aside, see
/// [`crate::sqlancer`]). An outer column in an argument would make the
/// aggregate the outer query's (SQLite assigns an aggregate to the
/// innermost query among its columns'), and a subquery could hide one.
fn agg_arg_ok(a: Agg, arg: Option<&Expr>) -> bool {
    match (a, arg) {
        (Agg::Count, None) => true,
        (Agg::Count, Some(x)) => local(x),
        (Agg::Min | Agg::Max, Some(Expr::Cast(x, CastType::Integer | CastType::Text))) => local(x),
        (Agg::Sum, Some(Expr::Cast(x, CastType::Integer))) => local(x),
        _ => false,
    }
}

/// An expression of its own query's rows: no aggregate, window, subquery
/// or column of an outer query (see [`agg_arg_ok`]).
fn local(x: &Expr) -> bool {
    let mut ok = true;
    x.walk(&mut |n| {
        ok &= !matches!(
            n,
            Expr::Agg(..)
                | Expr::Ordered(_)
                | Expr::Window(_)
                | Expr::Exists(..)
                | Expr::InSelect(..)
                | Expr::RowInSelect(..)
                | Expr::Scalar(_)
                | Expr::Col(Col { up: 1.., .. })
        )
    });
    ok
}

/// An aggregate with an ORDER BY whose value the rows determine. Its
/// arguments and ORDER BY terms are of its own query's rows ([`local`]),
/// with no `jsonb` function (whose bytes for one value depend on the order
/// rows are visited, see [`crate::sqlancer`]), and:
///
/// - `count`, `sum`, `min`, `max`: the argument [`agg_arg_ok`] allows (not
///   `*`, which takes no ORDER BY); any terms, the order changing nothing;
/// - `group_concat(x[, sep])`, `string_agg(x, sep)`: `sep` a text literal
///   (one separator for every row); `json_group_array(x)`: `x` a JSON value
///   ([`value_arg_ok`]); `json_group_object(k, v)`: `k` never a BLOB, `v` a
///   JSON value;
/// - for these four, the terms include [`Ordered::ties`]: rows that tie in
///   every term are appended as the same text (`1` and `1.0` compare equal
///   but render apart; `-0.0` renders as `0.0`), so the order of what is
///   rendered is fixed;
/// - DISTINCT only with one argument (SQLite's rule); for `group_concat`
///   and `json_group_array` only of a `CAST` to INTEGER or TEXT, ordered by
///   that value or its tie terms alone: SQLite keeps the first of equal
///   values in the order rows are visited, and sorts the one it kept by
///   that row's terms.
pub fn ordered_ok(o: &Ordered) -> bool {
    let mut jsonb = false;
    for e in o.args.iter().chain(o.keys()) {
        e.walk(&mut |n| jsonb |= matches!(n, Expr::Call(f, _) if f.jsonb()));
    }
    let sep = |e: &Expr| matches!(e, Expr::Lit(Value::Text(_)));
    let args = match (o.func, o.args.as_slice()) {
        (f, [x]) if f.agg().is_some() => agg_arg_ok(f.agg().unwrap(), Some(x)),
        (OrdFunc::GroupConcat, [_]) => true,
        (OrdFunc::GroupConcat | OrdFunc::StringAgg, [_, s]) => sep(s),
        (OrdFunc::JsonGroupArray, [x]) => value_arg_ok(x),
        (OrdFunc::JsonGroupObject, [k, v]) => never_blob(k) && value_arg_ok(v),
        _ => false,
    };
    let keys = o.keys();
    let ties = Ordered::ties(o.func, &o.args);
    let distinct = !o.distinct
        || (o.args.len() == 1
            && (o.func.agg().is_some()
                || (matches!(o.args[0], Expr::Cast(_, CastType::Integer | CastType::Text))
                    && keys.iter().all(|k| **k == o.args[0] || ties.contains(k)))));
    !jsonb
        && args
        && o.args.iter().chain(keys.iter().copied()).all(local)
        && !keys.is_empty()
        && keys.len() == o.order.len()
        && ties.iter().all(|t| keys.contains(&t))
        && distinct
}

/// What a JSON function's argument must be, by position (see
/// [`json_args_ok`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonArg {
    /// Well-formed JSON or NULL ([`json_ok`]).
    Json,
    /// A well-formed path literal or NULL ([`path_ok`]); for
    /// `json_array_insert` one that ends at an array element.
    Path { array: bool },
    /// A value that is not a BLOB, or is a JSONB one ([`value_arg_ok`]).
    Value,
    /// A text literal: `json_object` raises an error on a label of any
    /// other type.
    Label,
    /// `json_valid`'s flags, an integer literal 1 to 15.
    Flags,
    /// The column [`Func::JsonEach`] and [`Func::JsonTree`] count, a text
    /// literal of [`JSON_EACH_COLUMNS`].
    Column,
    /// Anything: `json_valid` and `json_error_position` take any value.
    Any,
    /// `json_pretty`'s indent: NULL (the default) or a text literal of at
    /// most 4 spaces and tabs ([`JSON_INDENTS`]). Only with whitespace is
    /// the output JSON ([`Func::makes_json`]): any other indent (`0`, a
    /// JSONB blob, a column) puts its characters between the elements, and
    /// a JSON function reading the output raises "malformed JSON". And an
    /// indent from a row may be any length, which the output repeats on
    /// every line at every level: past SQLite's length limits (as a LIKE
    /// pattern, SQLITE_LIMIT_LIKE_PATTERN_LENGTH). No other JSON function's
    /// output grows faster than its arguments.
    Indent,
}

/// The indents `json_pretty` is drawn with ([`JsonArg::Indent`]).
pub const JSON_INDENTS: [&str; 6] = ["", " ", "  ", "    ", "\t", " \t"];

/// The kind of argument `k` of JSON function `f`.
fn json_arg(f: Func, k: usize) -> JsonArg {
    match (f, k) {
        (Func::JsonValid | Func::JsonErrorPosition, 0) => JsonArg::Any,
        (Func::JsonValid, _) => JsonArg::Flags,
        (Func::JsonPretty, 1) => JsonArg::Indent,
        (Func::JsonEach | Func::JsonTree, 0) => JsonArg::Column,
        (Func::JsonEach | Func::JsonTree, 1) => JsonArg::Json,
        (Func::JsonArray | Func::JsonbArray | Func::JsonQuote, _) => JsonArg::Value,
        (Func::JsonObject | Func::JsonbObject, k) => if k % 2 == 0 { JsonArg::Label } else { JsonArg::Value },
        (_, 0) => JsonArg::Json,
        (Func::JsonPatch | Func::JsonbPatch, _) => JsonArg::Json,
        (Func::JsonArrayInsert | Func::JsonbArrayInsert, k) => if k % 2 == 1 { JsonArg::Path { array: true } } else { JsonArg::Value },
        (Func::JsonInsert | Func::JsonbInsert | Func::JsonReplace | Func::JsonbReplace | Func::JsonSet | Func::JsonbSet, k) => {
            if k % 2 == 1 { JsonArg::Path { array: false } } else { JsonArg::Value }
        },
        _ => JsonArg::Path { array: false },
    }
}

/// A call of JSON function `f` that cannot raise an error, whatever the
/// rows: the argument count generated ([`Func::arity`]), label and value
/// pairs for `json_object`, path and value pairs for the edits, and each
/// argument of its kind ([`json_arg`]). SQLite raises "malformed JSON",
/// "bad JSON path", "JSON cannot hold BLOB values" and the like on the
/// rows whose arguments are wrong, and which rows a query evaluates an
/// expression on depends on the plan (see [`crate::sqlancer`]); so every
/// argument is right by its form. The arguments of [`Func::JsonEach`] and
/// [`Func::JsonTree`], rendered in a subquery, have no aggregate (it
/// would be the subquery's) and no rowid (it would be `json_each`'s).
pub fn json_args_ok(f: Func, args: &[Expr]) -> bool {
    let (lo, hi) = f.arity();
    let pairs = match f {
        Func::JsonObject | Func::JsonbObject => args.len().is_multiple_of(2),
        Func::JsonRemove | Func::JsonbRemove => true,
        _ if f.makes_json() && lo == 3 => args.len() % 2 == 1,
        _ => true,
    };
    // An aggregate or rowid of a subquery in an argument is that
    // subquery's: only the argument's own nodes count.
    fn own_ok(e: &Expr) -> bool {
        !matches!(e, Expr::Agg(..) | Expr::Ordered(_) | Expr::Col(Col { col: ROWID, .. })) && e.children().iter().all(|c| own_ok(c))
    }
    let inner_ok = !matches!(f, Func::JsonEach | Func::JsonTree) || args.iter().all(own_ok);
    (lo..=hi).contains(&args.len())
        && pairs
        && inner_ok
        && args.iter().enumerate().all(|(k, a)| match json_arg(f, k) {
            JsonArg::Json => json_ok(a),
            JsonArg::Path { array } => matches!(a, Expr::Lit(Value::Null)) || matches!(a, Expr::Lit(Value::Text(s)) if path_ok(s, array)),
            JsonArg::Value => value_arg_ok(a),
            JsonArg::Label => matches!(a, Expr::Lit(Value::Text(_))),
            JsonArg::Flags => matches!(a, Expr::Lit(Value::Int(1..=15))),
            JsonArg::Column => matches!(a, Expr::Lit(Value::Text(c)) if JSON_EACH_COLUMNS.contains(&c.as_str())),
            JsonArg::Any => true,
            JsonArg::Indent => {
                matches!(a, Expr::Lit(Value::Null)) || matches!(a, Expr::Lit(Value::Text(s)) if s.len() <= 4 && s.chars().all(|c| c == ' ' || c == '\t'))
            },
        })
}

/// A call of date and time function `f` whose value its arguments fix:
/// the argument count generated ([`Func::arity`]: a time value at least,
/// so never the current time), every modifier a text literal of the
/// deterministic subset ([`modifier_ok`]) or NULL, and no time value with
/// a literal `now` in it, nor a literal SQLite reads as the current time
/// (`crate::model::reads_as_now`: `subsec`, a blob of `now`). SQLite reads `now` as the current time (fixed
/// per statement, but the oracles compare statements) and the modifiers
/// `localtime` and `utc` by the host's timezone; any other argument gives
/// a value or NULL, never an error (an unknown modifier, a time value it
/// cannot read or one out of range is NULL; so is an unknown conversion in
/// a format, which may be any value). No value the generators draw reads
/// as the current time (no text pool spells `now`, and a random blob that
/// does is changed, `crate::model::not_now`), so a column or an expression
/// as a time value is as fixed.
pub fn date_args_ok(f: Func, args: &[Expr]) -> bool {
    let (lo, hi) = f.arity();
    let t = f.time_at();
    let now = |e: &Expr| {
        let mut found = false;
        e.walk(&mut |n| found |= matches!(n, Expr::Lit(Value::Text(s)) if s.to_ascii_lowercase().contains("now")));
        // A literal SQLite reads as the current time, but a nested date
        // call's modifier (`subsec` is one; that call is checked itself).
        fn current(e: &Expr) -> bool {
            match e {
                Expr::Lit(v) => crate::model::reads_as_now(v),
                Expr::Call(f, args) if f.date() => args.get(f.time_at()).is_some_and(current) || (*f == Func::Timediff && args.iter().any(current)),
                _ => e.children().iter().any(|c| current(c)),
            }
        }
        found |= current(e);
        found
    };
    (lo..=hi).contains(&args.len())
        && args.iter().enumerate().all(|(k, a)| match k {
            _ if f == Func::Timediff || k == t => !now(a),
            _ if k < t => true,
            _ => matches!(a, Expr::Lit(Value::Null)) || matches!(a, Expr::Lit(Value::Text(m)) if modifier_ok(m)),
        })
}

/// A format for `printf` whose output stays short: no `*` (a width or
/// precision from an argument, any size) and no run of more than two
/// digits (a width or precision past 99). SQLite formats every argument
/// some way, a missing one as NULL, and raises no error.
pub fn format_ok(f: &str) -> bool {
    let mut run = 0;
    f.chars().all(|c| {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        c != '*' && run <= 2
    })
}

/// A modifier of the deterministic subset: no `local`, `utc` or `now` in
/// it, in any case (SQLite matches `localtime` and `utc` whole, so this is
/// stricter than it must be).
pub fn modifier_ok(m: &str) -> bool {
    let m = m.to_ascii_lowercase();
    !["local", "utc", "now"].iter().any(|w| m.contains(w))
}

/// Well-formed JSON or NULL, by its form: a text literal that is JSON
/// ([`json_text_ok`]), JSON5 ([`json5_text_ok`]) or one of [`JSON5_DOCS`];
/// a call whose value is JSON ([`Func::makes_json`]); `->` (the JSON text
/// of an element); or `CASE WHEN json_valid(x) THEN x END`, any value that
/// SQLite reads as JSON (`json_valid` with its default flags accepts only
/// RFC 8259 text, which every JSON function reads), else NULL; or `CASE
/// WHEN json_valid(x, 8) THEN x END`, a blob that is JSONB by SQLite's
/// strict check (which every JSON function reads), else NULL.
pub fn json_ok(e: &Expr) -> bool {
    match e {
        Expr::Lit(Value::Null) => true,
        Expr::Lit(Value::Text(s)) => json_text_ok(s) || JSON5_DOCS.contains(&s.as_str()) || json5_text_ok(s),
        Expr::Call(Func::JsonExtract, args) => args.len() > 2,
        Expr::Call(f, _) => f.makes_json(),
        Expr::Bin(BinOp::Arrow, ..) => true,
        Expr::Ordered(o) => matches!(o.func, OrdFunc::JsonGroupArray | OrdFunc::JsonGroupObject),
        Expr::Case(None, ws, None) => matches!(ws.as_slice(), [(Expr::Call(Func::JsonValid, a), x)]
            if !a.is_empty() && a[0] == *x && (a.len() == 1 || a.len() == 2 && matches!(a[1], Expr::Lit(Value::Int(f)) if guard_flags(f)))),
        _ => false,
    }
}

/// Flags of `json_valid` that guard a JSON argument: any but those with
/// 4 (a blob whose header only looks like JSONB passes then, which a JSON
/// function may find malformed). Without it a blob that looks like JSONB
/// is checked strictly (8) or is not valid; any other value is read as
/// text, RFC 8259 (1) or JSON5 too (2), as every JSON function reads it.
fn guard_flags(f: i64) -> bool {
    (1..=15).contains(&f) && f & 4 == 0
}

/// The columns of `json_each` and `json_tree` that [`Func::JsonEach`] and
/// [`Func::JsonTree`] count the distinct values of, or `*` for the rows.
pub const JSON_EACH_COLUMNS: [&str; 9] = ["*", "key", "value", "type", "atom", "id", "parent", "fullkey", "path"];

/// JSON5 texts every JSON function reads (SQLite accepts JSON5 input;
/// [`json_text_ok`] accepts only RFC 8259): unquoted and single-quoted
/// labels and strings, trailing commas, hexadecimal, `Infinity`, `NaN`,
/// signs and bare decimal points, comments, escapes JSON lacks.
pub const JSON5_DOCS: [&str; 12] = [
    "{a:1,'b':[2,],}",
    "[0x1F,-0xa,+1,.5,5.,]",
    "'single \\x41'",
    "Infinity",
    "[-Infinity,NaN]",
    "/* c */ {\"a\":1} // d",
    "{$_b:[0x10,],c:'d'}",
    "\"a\\\nb\"",
    "{\"a\":{b:+.5e1}}",
    "[1,\n// x\n2]",
    "\"\\v\\x41\\'\"",
    "{a:[{b:null,},],}",
];

/// RFC 8259 JSON text, strictly: one value, whitespace around. SQLite
/// accepts more (JSON5), so every text accepted here is well-formed to it.
pub fn json_text_ok(s: &str) -> bool {
    fn ws(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    }
    fn string(b: &[u8], mut i: usize) -> Option<usize> {
        i += 1;
        while i < b.len() {
            match b[i] {
                b'"' => return Some(i + 1),
                b'\\' => match b.get(i + 1)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => i += 2,
                    b'u' if b.len() >= i + 6 && b[i + 2..i + 6].iter().all(u8::is_ascii_hexdigit) => i += 6,
                    _ => return None,
                },
                0..=0x1f => return None,
                _ => i += 1,
            }
        }
        None
    }
    fn number(b: &[u8], mut i: usize) -> Option<usize> {
        let digits = |i: usize| b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if b.get(i) == Some(&b'-') {
            i += 1;
        }
        match digits(i) {
            0 => return None,
            n if n > 1 && b[i] == b'0' => return None,
            n => i += n,
        }
        if b.get(i) == Some(&b'.') {
            match digits(i + 1) {
                0 => return None,
                n => i += 1 + n,
            }
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            match digits(i) {
                0 => return None,
                n => i += n,
            }
        }
        Some(i)
    }
    fn value(b: &[u8], i: usize, depth: usize) -> Option<usize> {
        let i = ws(b, i);
        let close = |open: u8| if open == b'{' { b'}' } else { b']' };
        match *b.get(i)? {
            open @ (b'{' | b'[') if depth < 100 => {
                let mut j = ws(b, i + 1);
                if b.get(j) == Some(&close(open)) {
                    return Some(j + 1);
                }
                loop {
                    if open == b'{' {
                        j = ws(b, j);
                        if b.get(j) != Some(&b'"') {
                            return None;
                        }
                        j = ws(b, string(b, j)?);
                        if b.get(j) != Some(&b':') {
                            return None;
                        }
                        j += 1;
                    }
                    j = ws(b, value(b, j, depth + 1)?);
                    match b.get(j) {
                        Some(b',') => j += 1,
                        Some(&c) if c == close(open) => return Some(j + 1),
                        _ => return None,
                    }
                }
            },
            b'"' => string(b, i),
            b'-' | b'0'..=b'9' => number(b, i),
            _ => ["true", "false", "null"].iter().find(|w| b[i..].starts_with(w.as_bytes())).map(|w| i + w.len()),
        }
    }
    let b = s.as_bytes();
    value(b, 0, 0).is_some_and(|i| ws(b, i) == b.len())
}

/// JSON5 text of the forms [`draw_json5_text`] draws, strictly (SQLite
/// reads more): RFC 8259 JSON with labels also unquoted (`[A-Za-z_$]` then
/// `[A-Za-z0-9_$]`) or in single quotes; strings also in single quotes
/// (no `"` in them unescaped) and with the escapes `\'`, `\xHH`, `\v`,
/// `\0` (no digit after it) and a backslash before a line break (`\n`,
/// `\r\n`, U+2028, U+2029); numbers also with a sign `+`, in hexadecimal
/// (`0x`, signed too, any length: past 64 bits SQLite reads it as
/// infinity), with no digit before or after the decimal point (one at
/// least), `Infinity` (signed too) and `NaN` (unsigned: `-NaN` is
/// malformed); a comma after an object's or array's last element; and
/// between tokens also U+000B, U+000C, U+00A0, U+1680, U+2000 to
/// U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF, `/* .. */` and
/// `// ..` ending at a line feed, a carriage return, U+2028 or U+2029
/// (`json5Whitespace`), each comment of
/// lowercase letters and spaces only. Every text [`json_text_ok`] accepts
/// is accepted.
pub fn json5_text_ok(s: &str) -> bool {
    fn ws(s: &str, mut i: usize) -> Option<usize> {
        let comment = |body: &str| body.bytes().all(|c| c == b' ' || c.is_ascii_lowercase());
        loop {
            let rest = &s[i..];
            if let Some(c) = rest.chars().next().filter(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')) {
                i += c.len_utf8();
            } else if let Some(r) = rest.strip_prefix("/*") {
                let end = r.find("*/")?;
                if !comment(&r[..end]) {
                    return None;
                }
                i += 2 + end + 2;
            } else if let Some(r) = rest.strip_prefix("//") {
                let end = r.find(['\n', '\r', '\u{2028}', '\u{2029}'])?;
                if !comment(&r[..end]) {
                    return None;
                }
                i += 2 + end + r[end..].chars().next()?.len_utf8();
            } else {
                return Some(i);
            }
        }
    }
    fn string(s: &str, i: usize) -> Option<usize> {
        let b = s.as_bytes();
        let q = b[i];
        let mut i = i + 1;
        while i < b.len() {
            match b[i] {
                c if c == q => return Some(i + 1),
                b'"' => return None,
                b'\\' => {
                    let r = &s[i + 1..];
                    let hex = |k: usize| r.len() > k && r.as_bytes()[..=k].iter().skip(1).all(u8::is_ascii_hexdigit);
                    i += 1 + match *b.get(i + 1)? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' | b'\'' | b'v' => 1,
                        b'0' if !b.get(i + 2).is_some_and(u8::is_ascii_digit) => 1,
                        b'u' if hex(4) => 5,
                        b'x' if hex(2) => 3,
                        b'\r' if b.get(i + 2) == Some(&b'\n') => 2,
                        _ if r.starts_with('\n') => 1,
                        _ if r.starts_with('\u{2028}') || r.starts_with('\u{2029}') => 3,
                        _ => return None,
                    };
                },
                0..=0x1f => return None,
                _ => i += 1,
            }
        }
        None
    }
    fn number(b: &[u8], mut i: usize) -> Option<usize> {
        let digits = |i: usize| b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        let signed = matches!(b.get(i), Some(b'-' | b'+'));
        if signed {
            i += 1;
        }
        let rest = &b[i..];
        if rest.starts_with(b"Infinity") {
            return Some(i + 8);
        }
        if rest.starts_with(b"NaN") {
            return (!signed).then_some(i + 3);
        }
        if rest.len() > 2 && rest[0] == b'0' && matches!(rest[1], b'x' | b'X') {
            let n = rest[2..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
            return (n > 0).then_some(i + 2 + n);
        }
        let whole = digits(i);
        if whole > 1 && b[i] == b'0' {
            return None;
        }
        i += whole;
        let mut frac = 0;
        if b.get(i) == Some(&b'.') {
            frac = digits(i + 1);
            i += 1 + frac;
        }
        if whole + frac == 0 {
            return None;
        }
        if matches!(b.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(b.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            match digits(i) {
                0 => return None,
                n => i += n,
            }
        }
        Some(i)
    }
    fn value(s: &str, i: usize, depth: usize) -> Option<usize> {
        let b = s.as_bytes();
        let i = ws(s, i)?;
        let close = |open: u8| if open == b'{' { b'}' } else { b']' };
        match *b.get(i)? {
            open @ (b'{' | b'[') if depth < 100 => {
                let mut j = ws(s, i + 1)?;
                if b.get(j) == Some(&close(open)) {
                    return Some(j + 1);
                }
                loop {
                    if open == b'{' {
                        j = ws(s, j)?;
                        j = match *b.get(j)? {
                            b'"' | b'\'' => string(s, j)?,
                            c if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                                j + b[j..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'_' || **c == b'$').count()
                            },
                            _ => return None,
                        };
                        j = ws(s, j)?;
                        if b.get(j) != Some(&b':') {
                            return None;
                        }
                        j += 1;
                    }
                    j = ws(s, value(s, j, depth + 1)?)?;
                    match b.get(j) {
                        Some(b',') => {
                            j = ws(s, j + 1)?;
                            if b.get(j) == Some(&close(open)) {
                                return Some(j + 1);
                            }
                        },
                        Some(&c) if c == close(open) => return Some(j + 1),
                        _ => return None,
                    }
                }
            },
            b'"' | b'\'' => string(s, i),
            b'-' | b'+' | b'.' | b'0'..=b'9' | b'I' | b'N' => number(b, i),
            _ => ["true", "false", "null"].iter().find(|w| b[i..].starts_with(w.as_bytes())).map(|w| i + w.len()),
        }
    }
    value(s, 0, 0).and_then(|i| ws(s, i)).is_some_and(|i| i == s.len())
}

/// A well-formed JSON path: `$` then steps `.label` (unquoted, without
/// `.`, `[`, `]`, `"` or `\`; or in double quotes, without `"` or `\`),
/// `[n]`, `[#]`, `[#-n]`; ending at an array element when `array`
/// (`json_array_insert` raises an error on any other path that reaches an
/// element, and on `$`). SQLite raises "bad JSON path" on a malformed one.
pub fn path_ok(s: &str, array: bool) -> bool {
    let Some(mut rest) = s.strip_prefix('$') else {
        return false;
    };
    let mut last_array = false;
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            if let Some(q) = r.strip_prefix('"') {
                let Some(end) = q.find('"') else {
                    return false;
                };
                if q[..end].contains('\\') {
                    return false;
                }
                rest = &q[end + 1..];
            } else {
                let end = r.find(['.', '[']).unwrap_or(r.len());
                if end == 0 || r[..end].contains([']', '"', '\\']) {
                    return false;
                }
                rest = &r[end..];
            }
            last_array = false;
        } else if let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else {
                return false;
            };
            let inner = &r[..end];
            if !(digits(inner) || inner == "#" || inner.strip_prefix("#-").is_some_and(digits)) {
                return false;
            }
            rest = &r[end + 1..];
            last_array = true;
        } else {
            return false;
        }
    }
    !array || last_array
}

/// A right operand of `->` or `->>`: NULL; an integer (`$[n]`, from the
/// end when negative); a path ([`path_ok`]); `[..]`, read as `$[..]`;
/// any other non-empty text without `"` or `\`, read as a label
/// (`$."text"`).
pub fn arrow_ok(e: &Expr) -> bool {
    match e {
        Expr::Lit(Value::Null | Value::Int(_)) => true,
        Expr::Lit(Value::Text(s)) if s.starts_with('$') => path_ok(s, false),
        Expr::Lit(Value::Text(s)) if s.len() >= 3 && s.starts_with('[') && s.ends_with(']') => path_ok(&format!("${s}"), false),
        Expr::Lit(Value::Text(s)) => !s.is_empty() && !s.contains(['"', '\\']),
        _ => false,
    }
}

/// Whether every `jsonb` function's value in `e` (its own nodes; a
/// subquery's are checked as its own expressions) is read by a JSON
/// function: an argument of one, or of `->` / `->>` on the left, directly
/// or through what passes a value on unchanged (`coalesce` and `ifnull`
/// arguments, `iif` and `CASE` results, `COLLATE`); `read` when `e` itself
/// is one such argument. SQLite may encode one value as JSONB in several
/// ways, by its per-statement JSON parse cache, so by the rows a plan
/// visits before (an INT5 `0x10` copied from a JSON5 input, or the INT
/// `16` of the same value parsed again from text once the cache let it
/// go; `json()` of both is one text): any other function, operator,
/// comparison, cast, aggregate, sort, group or set sees the bytes or a
/// value of them (`lower`, `length`, `||`, `=`, truthiness), and so does a
/// query's result (a whole item, in any query). The JSON functions read
/// JSONB as the value it encodes. So no oracle ever sees an encoding (the
/// runner's tlp-where of `lower(jsonb_replace(json_patch(..), ..))`, run
/// 20261012-0317, w0-261227: one row of six had the other bytes). The
/// generator wraps a `jsonb` value drawn where any expression goes in
/// `json()` (`json_quote()` for `jsonb_extract`, [`draw_json`]).
pub fn jsonb_read(e: &Expr, read: bool) -> bool {
    match e {
        Expr::Call(f, args) if f.json() => (read || !f.jsonb()) && args.iter().all(|a| jsonb_read(a, true)),
        Expr::Call(Func::Coalesce | Func::Ifnull, args) => args.iter().all(|a| jsonb_read(a, read)),
        Expr::Call(Func::Iif, args) => args.iter().enumerate().all(|(k, a)| jsonb_read(a, read && k > 0)),
        Expr::Case(op, ws, el) => {
            op.as_deref().is_none_or(|o| jsonb_read(o, false))
                && ws.iter().all(|(w, t)| jsonb_read(w, false) && jsonb_read(t, read))
                && el.as_deref().is_none_or(|x| jsonb_read(x, read))
        },
        Expr::Collate(x, _) => jsonb_read(x, read),
        Expr::Bin(BinOp::Arrow | BinOp::Arrow2, j, p) => jsonb_read(j, true) && jsonb_read(p, false),
        _ => e.children().iter().all(|c| jsonb_read(c, false)),
    }
}

/// A value a JSON function takes as one: never a BLOB ([`never_blob`]),
/// or a `jsonb` call, whose BLOB is JSONB. Any other BLOB is "JSON cannot
/// hold BLOB values" (or, when it happens to look like JSONB, read as
/// such).
pub fn value_arg_ok(e: &Expr) -> bool {
    !cast_of_jsonb(e) && (never_blob(e) || matches!(e, Expr::Call(f, _) if f.jsonb()))
}

/// A `CAST` of a value that may be a `jsonb` function's (through casts,
/// `coalesce` and the like, `CASE` results, the aggregates `min` and
/// `max`, a scalar subquery's item): the cast keeps the JSON subtype, so a
/// JSON function takes the cast's text (the blob's bytes) as JSON and
/// raises "malformed JSON" or "JSON cannot hold BLOB values" (the runner's
/// `CAST(jsonb_array(..) AS TEXT)`). The aggregates `min` and `max` keep
/// the subtype of the value they return (the runner's `max(CAST(
/// jsonb_object(..) AS TEXT))` in a HAVING), and
/// so does a scalar subquery, even guarded by [`unblob`], up to SQLite
/// check-in 8ab55e0185 (trunk of 2026-10-12, after 3.54.0 and the vendored
/// 797081cb705c), from which a scalar subquery's value has no subtype. The
/// rule keeps such a scalar subquery out on every version: the generator
/// never runs SQLite, so it draws the same queries whichever SQLite runs
/// them (the runner's replays, the frozen score); on a newer SQLite the
/// rule is only stricter than it must be, for this one narrow shape.
fn cast_of_jsonb(e: &Expr) -> bool {
    fn jsonb(e: &Expr) -> bool {
        match e {
            Expr::Call(f, _) if f.jsonb() => true,
            Expr::Call(Func::Coalesce | Func::Ifnull | Func::Min | Func::Max | Func::Nullif | Func::Iif | Func::Likely | Func::Unlikely | Func::Likelihood, args) => args.iter().any(jsonb),
            Expr::Cast(x, _) | Expr::Agg(Agg::Min | Agg::Max, Some(x)) => jsonb(x),
            Expr::Ordered(o) if matches!(o.func, OrdFunc::Min | OrdFunc::Max) => o.args.iter().any(jsonb),
            Expr::Case(_, ws, el) => ws.iter().any(|(_, t)| jsonb(t)) || el.as_deref().is_some_and(jsonb),
            Expr::Scalar(q) => q.items.exprs().into_iter().any(jsonb),
            _ => false,
        }
    }
    match e {
        Expr::Cast(x, _) => jsonb(x),
        Expr::Call(Func::Coalesce | Func::Ifnull | Func::Min | Func::Max | Func::Nullif | Func::Iif | Func::Likely | Func::Unlikely | Func::Likelihood, args) => args.iter().any(cast_of_jsonb),
        Expr::Agg(Agg::Min | Agg::Max, Some(x)) => cast_of_jsonb(x),
        Expr::Ordered(o) if matches!(o.func, OrdFunc::Min | OrdFunc::Max) => o.args.iter().any(cast_of_jsonb),
        Expr::Scalar(q) => q.items.exprs().into_iter().any(cast_of_jsonb),
        Expr::Case(_, ws, el) => ws.iter().any(|(_, t)| cast_of_jsonb(t)) || el.as_deref().is_some_and(cast_of_jsonb),
        _ => false,
    }
}

/// Never a BLOB, by its form: a literal of another class; any operator
/// (arithmetic, comparisons, `||` give no BLOB); a `CAST` to anything but
/// BLOB; a function of [`Func::ALL`] that makes a new value, or one that
/// returns an argument when every such argument is never a BLOB; a JSON
/// function but the `jsonb` ones; an aggregate (of a `CAST` to INTEGER or
/// TEXT for `min` and `max`); `EXISTS` and `IN`; `CASE` with every result
/// never a BLOB, or `CASE typeof(x) WHEN 'blob' THEN y ELSE x END` with
/// `y` never one ([`unblob`]). Never a column or a scalar subquery.
pub fn never_blob(e: &Expr) -> bool {
    match e {
        Expr::Lit(v) => !matches!(v, Value::Blob(_)),
        Expr::Col(_) | Expr::Scalar(_) | Expr::Window(_) => false,
        Expr::Cast(_, ty) => *ty != CastType::Blob,
        Expr::Case(op, ws, el) => {
            let guard = matches!(
                (op.as_deref(), ws.as_slice(), el.as_deref()),
                (Some(Expr::Call(Func::Typeof, a)), [(Expr::Lit(Value::Text(t)), y)], Some(x))
                    if a.len() == 1 && a[0] == *x && t == "blob" && never_blob(y)
            );
            guard || (ws.iter().all(|(_, t)| never_blob(t)) && el.as_deref().is_none_or(never_blob))
        },
        Expr::Call(f, args) => match f {
            Func::Coalesce | Func::Ifnull | Func::Min | Func::Max => args.iter().all(never_blob),
            Func::Nullif | Func::Substr | Func::Likely | Func::Unlikely | Func::Likelihood => args.first().is_some_and(never_blob),
            Func::Iif => args.iter().skip(1).all(never_blob),
            Func::Unhex | Func::Zeroblob => false,
            f => !f.jsonb(),
        },
        _ => true,
    }
}

/// `CASE typeof(x) WHEN 'blob' THEN hex(x) ELSE x END`: `x` as a JSON
/// value ([`value_arg_ok`]) without dropping its type for any other class.
pub fn unblob(x: Expr) -> Expr {
    let typeof_x = Expr::Call(Func::Typeof, vec![x.clone()]);
    let hex = Expr::Call(Func::Hex, vec![x.clone()]);
    Expr::Case(Some(bx(typeof_x)), vec![(Expr::Lit(Value::Text("blob".into())), hex)], Some(bx(x)))
}

/// In the items or HAVING of an aggregate query with GROUP BY terms
/// `keys`: every column of the query is inside a term or an aggregate,
/// and no subquery refers to the query outside one. SQLite evaluates any
/// other column on one arbitrary row of the group (lang_select.html,
/// "evaluated against a single arbitrarily chosen row from within the
/// group"), which depends on the plan.
pub fn grouped_ok(e: &Expr, keys: &[Expr]) -> bool {
    if keys.contains(e) {
        return true;
    }
    match e {
        Expr::Col(c) => c.up != 0,
        Expr::Agg(..) | Expr::Ordered(_) => true,
        Expr::Window(_) => false,
        Expr::Exists(q, _) | Expr::Scalar(q) => !q.refers(1),
        Expr::InSelect(a, q, _) => grouped_ok(a, keys) && !q.refers(1),
        Expr::RowInSelect(a, q, _) => a.iter().all(|x| grouped_ok(x, keys)) && !q.refers(1),
        _ => e.children().iter().all(|c| grouped_ok(c, keys)),
    }
}

impl fmt::Debug for Query {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.sql())
    }
}

/// What the queries may refer to: per table, its column count, its row
/// count (to keep joins and subqueries cheap) and a few values it holds
/// (for literals that select some rows).
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub tables: Vec<TableInfo>,
    /// The common table expressions of the query being drawn or checked,
    /// as sources: CTE `i` is source table [`CTE`]` + i`. None in a
    /// database's schema.
    pub ctes: Vec<TableInfo>,
    /// Per table, what the virtual tables that introspect the schema read
    /// of it ([`Vtab`]); empty for a schema not of a database. Shared:
    /// a query's scope ([`Query::scope`]) is a copy of the schema.
    pub catalog: std::sync::Arc<Vec<Catalog>>,
}

/// The source table number of the first common table expression of a
/// query (`x0`); table numbers below it are the database's tables (`t0`,
/// ..).
pub const CTE: usize = 1 << 20;

/// Whether source `k` of a query at nesting `depth`, of table `t`, is read
/// through a subquery in FROM ([`Query::source_at`]), when it may be.
fn wrap_pick(t: usize, depth: usize, k: usize) -> bool {
    spell(&format!("wrap{t}.{depth}.{k}"), 6) == 0
}

thread_local! {
    /// The schema of each table of the database the queries of this
    /// thread are drawn and run against, and whether `aux` is attached:
    /// set by [`Schema::of`], which every caller that draws, checks or
    /// renders a case's queries runs on the case's final model first.
    /// Names are unique over the schemas, so a query names a table bare
    /// wherever it is; it names the schema now and then ([`source_name`],
    /// a column's `main.a0.c0`, a pragma's schema argument), which must be
    /// the table's.
    static DBS: std::cell::RefCell<(Vec<Db>, bool)> = const { std::cell::RefCell::new((Vec::new(), false)) };
}

thread_local! {
    /// The virtual tables of the database of this thread's queries, set by
    /// [`Schema::of`] as [`DBS`] is.
    static VINFO: std::cell::RefCell<Vec<crate::vtabs::VInfo>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The kind of virtual table `vt{k}` of this thread's database.
pub fn virtual_kind(k: usize) -> Option<crate::vtabs::Kind> {
    VINFO.with(|i| i.borrow().get(k).and_then(|i| i.kind.clone()))
}

/// Whether this thread's database has a virtual table.
pub fn has_virtual() -> bool {
    VINFO.with(|i| !i.borrow().is_empty())
}

/// The schema of table `t` (main for one not in this thread's database).
pub fn table_db(t: usize) -> Db {
    DBS.with(|d| d.borrow().0.get(t).copied().unwrap_or_default())
}

/// Whether `aux` is attached to this thread's database.
pub fn aux_attached() -> bool {
    DBS.with(|d| d.borrow().1)
}

/// Source table `t`, source `k` of a query at `at`, as the query names it:
/// its name, or the marker of its CTE ([`At::inline`]).
fn source_name(t: usize, at: At, k: usize) -> String {
    if t >= CTE && at.inline {
        cte_marker(t - CTE)
    } else if t < CTE && spell(&format!("{t}.{}.{k}", at.depth), 4) == 0 {
        // Schema-qualified a quarter of the time, in the table's schema.
        format!("{}.{}", table_db(t).name(), table_name(t))
    } else {
        table_name(t)
    }
}

/// The name of source table `t`: `t{t}`, or `x{i}` for CTE `i`.
pub fn table_name(t: usize) -> String {
    if t >= CTE {
        format!("x{}", t - CTE)
    } else {
        format!("t{t}")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableInfo {
    pub columns: usize,
    pub rows: usize,
    pub samples: Vec<Value>,
    /// Up to 6 values each column holds, for comparisons of that column
    /// with a literal that selects some of its rows.
    pub by_column: Vec<Vec<Value>>,
    /// Pairs `(column, other table, other column)` whose columns share a
    /// non-NULL value (among each table's first 256 rows): equalities that
    /// make a join match.
    pub shared: Vec<(usize, usize, usize)>,
    /// What makes its rows unique.
    pub key: RowKey,
    /// The first terms of its expression indexes ([`key_expr`], of source
    /// 0): a query compares one with a literal for the planner to seek
    /// the index.
    pub exprs: Vec<Expr>,
    /// The predicates of its partial indexes ([`pred_expr`], of source 0):
    /// a query's WHERE clause with one as a term implies it, so the
    /// planner may use the index.
    pub partial: Vec<Expr>,
    /// The columns an index of it (not an expression index) or its key
    /// leads with: an equality of one is a lookup the planner can make
    /// ([`draw_on`] joins on them).
    pub leading: Vec<usize>,
    /// Each column's collating sequence as SQLite gives a reference to it
    /// (`sqlite3ExprCollSeq`): a table column's declared one, BINARY where
    /// none is declared, none for an INTEGER PRIMARY KEY (a reference to it
    /// is one to the rowid, column -1, which has none); a CTE column's
    /// item's ([`expr_coll`]).
    pub coll: Vec<Option<Coll>>,
    /// Each column's affinity (none where none is given): a table column's,
    /// a CTE column's that is a table column's. Literals compared with a
    /// column of an affinity are now and then of a class it converts
    /// ([`converted_lit`]).
    pub aff: Vec<Affinity>,
}

impl TableInfo {
    /// Column `c`'s collating sequence, BINARY where it has none.
    pub fn coll_of(&self, c: usize) -> Coll {
        self.seq_of(c).unwrap_or(Coll::Binary)
    }

    /// Column `c`'s collating sequence, none for an INTEGER PRIMARY KEY or
    /// the rowid ([`ROWID`]): a compound takes a column's from its first
    /// member whose item has one ([`member_coll`]).
    pub fn seq_of(&self, c: usize) -> Option<Coll> {
        self.coll.get(c).copied().flatten()
    }

    /// Whether a column has a collating sequence other than BINARY.
    pub fn collated(&self) -> bool {
        self.coll.iter().any(|k| k.is_some_and(|k| k != Coll::Binary))
    }
}

/// The collating sequence SQLite gives `e` (`sqlite3ExprCollSeq`, BINARY
/// where it gives none): an explicit `COLLATE`'s, which expressions around
/// it take through their operands and arguments ([`explicit_coll`]); else
/// a column's, through `CAST`; else BINARY. `levels` as for
/// [`valid_expr`].
pub fn expr_coll(e: &Expr, schema: &Schema, levels: &[Vec<usize>]) -> Coll {
    if let Some(k) = explicit_coll(e) {
        return k;
    }
    match e {
        Expr::Col(c) => col_seq(c, schema, levels).unwrap_or(Coll::Binary),
        Expr::Cast(x, _) => expr_coll(x, schema, levels),
        _ => Coll::Binary,
    }
}

/// Column `c`'s collating sequence, none for an INTEGER PRIMARY KEY or the
/// rowid ([`TableInfo::seq_of`]).
fn col_seq(c: &Col, schema: &Schema, levels: &[Vec<usize>]) -> Option<Coll> {
    let l = levels.len().checked_sub(1 + c.up)?;
    schema.info(*levels[l].get(c.src)?)?.seq_of(c.col)
}

/// The collating sequence a compound takes from a member's item `e`
/// (`multiSelectCollSeq`: the first member whose item has one, BINARY when
/// none has): `Some(None)` where `e` has none (an expression without a
/// column or an explicit `COLLATE` it takes, an INTEGER PRIMARY KEY or the
/// rowid of a table, [`TableInfo::seq_of`]), `None` where the model does
/// not tell (a subquery). A column always has one, BINARY where none is
/// declared, a CTE's, a view's and a subquery's in FROM too (its
/// declared one its first SELECT's item's, `sqlite3SubqueryColumnTypes`):
/// so the INTEGER PRIMARY KEY of a source of `levels`' last level read
/// through a subquery in FROM (bit `k` of `wrapped` for source `k`,
/// [`Query::wrapped`]) has BINARY, the subquery's column a column.
pub fn member_coll(e: &Expr, schema: &Schema, levels: &[Vec<usize>], wrapped: u64) -> Option<Option<Coll>> {
    if let Some(k) = explicit_coll(e) {
        return Some(Some(k));
    }
    match e {
        Expr::Col(c) if c.up == 0 && c.src < 64 && wrapped >> c.src & 1 == 1 => Some(Some(col_seq(c, schema, levels).unwrap_or(Coll::Binary))),
        Expr::Col(c) => Some(col_seq(c, schema, levels)),
        Expr::Cast(x, _) => member_coll(x, schema, levels, wrapped),
        _ if e.subquery().is_some() => None,
        _ => Some(None),
    }
}

/// The sequence of the explicit `COLLATE` that `e` takes, if any: its own,
/// or the one `sqlite3ExprCollSeq` reaches through the nodes the parser
/// marks as holding one (`EP_Collate`, which it carries up from operands
/// and arguments): the left operand's, else the first argument's or list
/// element's (`CASE`, `BETWEEN`, `IN`) holding one, else the right
/// operand's. `x LIKE p` is `like(p, x)`, its pattern first; `x IN (q)`
/// takes `x`'s. None comes out of a subquery, out of an ordered
/// aggregate's ORDER BY (`sqlite3ExprAddFunctionOrderBy` hangs it on a
/// `TK_ORDER` node it sets no flags on) or out of a window's PARTITION BY
/// and ORDER BY (`sqlite3WindowAttach`), only out of their arguments.
fn explicit_coll(e: &Expr) -> Option<Coll> {
    match e {
        Expr::Collate(_, k) => Some(*k),
        Expr::Exists(..) | Expr::Scalar(_) => None,
        Expr::InSelect(x, ..) => explicit_coll(x),
        Expr::Ordered(o) => o.args.iter().find_map(explicit_coll),
        Expr::Window(w) => w.arg.as_ref().and_then(explicit_coll),
        Expr::Bin(BinOp::Like | BinOp::NotLike | BinOp::Glob | BinOp::NotGlob, x, p) => explicit_coll(p).or_else(|| explicit_coll(x)),
        _ => e.children().into_iter().find_map(explicit_coll),
    }
}

/// Whether `e` has no `CAST(x AS TEXT)` of a collated `x`: a cast to TEXT
/// keeps its operand's collating sequence, so under NOCASE or RTRIM it no
/// longer maps equal values to identical ones, which aggregates' arguments
/// and tie terms rely on ([`agg_arg_ok`], [`ordered_ok`]).
fn casts_binary(e: &Expr, schema: &Schema, levels: &[Vec<usize>]) -> bool {
    let mut ok = true;
    e.walk(&mut |n| {
        if let Expr::Cast(x, CastType::Text) = n {
            ok &= expr_coll(x, schema, levels) == Coll::Binary;
        }
    });
    ok
}

/// What makes a source's rows unique: its rowid, the primary key of a
/// WITHOUT ROWID table, or nothing (a common table expression).
#[derive(Clone, Debug, PartialEq)]
pub enum RowKey {
    Rowid,
    Columns(Vec<usize>),
    None,
}

impl Schema {
    /// Source table `t`: a table, or a CTE ([`CTE`]).
    pub fn info(&self, t: usize) -> Option<&TableInfo> {
        if t >= CTE {
            self.ctes.get(t - CTE)
        } else {
            self.tables.get(t)
        }
    }

    /// Every source table number: the tables, then the CTEs.
    pub fn ids(&self) -> Vec<usize> {
        (0..self.tables.len()).chain((0..self.ctes.len()).map(|i| CTE + i)).collect()
    }

    /// The tables of `m`, with up to 16 short values from rows spread over
    /// each table.
    pub fn of(m: &Model) -> Schema {
        let tables = m
            .tables
            .iter()
            .map(|t| {
                let mut samples = Vec::new();
                let step = (t.rows.len() / 8).max(1);
                for (k, row) in t.rows.iter().step_by(step).take(8).enumerate() {
                    for c in [k % t.columns, (k + 1) % t.columns] {
                        let v = column_value(t, row, c);
                        if short(&v) && value_ok(&v) && !samples.contains(&v) {
                            samples.push(v);
                        }
                    }
                }
                let mut by_column = vec![Vec::new(); t.columns];
                let step = (t.rows.len() / 6).max(1);
                for row in t.rows.iter().step_by(step).take(6) {
                    for (c, seen) in by_column.iter_mut().enumerate() {
                        let v = column_value(t, row, c);
                        if short(&v) && value_ok(&v) && !seen.contains(&v) {
                            seen.push(v);
                        }
                    }
                }
                let key = match &t.key {
                    Key::WithoutRowid(pk) => RowKey::Columns(pk.clone()),
                    _ => RowKey::Rowid,
                };
                TableInfo {
                    columns: t.columns,
                    rows: t.rows.len(),
                    samples,
                    by_column,
                    shared: Vec::new(),
                    key,
                    exprs: t.indexes.iter().filter_map(|i| Some(key_expr(i.expr?, i.columns[0].0))).collect(),
                    partial: t.indexes.iter().filter_map(|i| i.filter.as_ref().map(pred_expr)).filter(literals_ok).collect(),
                    leading: t
                        .indexes
                        .iter()
                        .filter(|i| i.expr.is_none())
                        .map(|i| i.columns[0].0)
                        .chain(match &t.key {
                            Key::IntegerPrimaryKey(c) => Some(*c),
                            Key::WithoutRowid(pk) => Some(pk[0]),
                            Key::Rowid => None,
                        })
                        .collect(),
                    coll: (0..t.columns).map(|c| (!matches!(t.key, Key::IntegerPrimaryKey(k) if k == c)).then(|| column_coll(t, c))).collect(),
                    aff: (0..t.columns).map(|c| affinity(t, c)).collect(),
                }
            })
            .collect::<Vec<_>>();
        let catalog = m
            .tables
            .iter()
            .map(|t| Catalog {
                types: (0..t.columns)
                    .map(|c| match (&t.strict, t.decl[c]) {
                        (Some(ts), _) => ts[c].name(),
                        (None, Some(d)) => crate::model::DECLS[d].0,
                        (None, None) if matches!(t.key, Key::IntegerPrimaryKey(k) if k == c) => "INTEGER",
                        (None, None) => "",
                    })
                    .collect(),
                indexes: t.indexes.iter().map(|i| i.columns.len()).collect(),
                fks: t.fks.iter().map(|f| f.columns.len()).collect(),
                pages: {
                    let width = if m.encoding == crate::model::Encoding::Utf8 { 1 } else { 2 };
                    let size = |v: &Value| match v {
                        Value::Null => 1,
                        Value::Int(_) | Value::Real(_) => 9,
                        Value::Text(x) => 2 + width * x.len(),
                        Value::Blob(b) => 2 + b.len(),
                    };
                    // An expression index's first term as long as its
                    // value may get: `format` and `strftime` of every
                    // conversion are a few hundred characters (one
                    // overflow page an entry: seed 9300093 of
                    // `virtual_tables_count_what_sqlite_lists`, two
                    // `format` indexes of 1253 rows, 1397 pages each).
                    let term = |i: &crate::model::Index, k: usize, v: Option<&Value>| {
                        let own = v.map_or(1, size);
                        match (k, &i.expr) {
                            (0, Some(KeyExpr::Format)) => 2 + width * 512 + 3 * own,
                            (0, Some(KeyExpr::Strftime)) => 2 + width * 320,
                            (0, Some(KeyExpr::Quote | KeyExpr::Replace)) => 32 + 2 * own,
                            (0, Some(KeyExpr::ConcatWs)) => 32 + 3 * own,
                            (0, Some(_)) => own.max(2 + width * 40),
                            _ => own,
                        }
                    };
                    let bytes: usize = t
                        .rows
                        .iter()
                        .map(|r| 12 + r.values.iter().map(size).sum::<usize>() + t.indexes.iter().map(|i| 12 + i.columns.iter().enumerate().map(|(k, &(c, _))| term(i, k, r.values.get(c))).sum::<usize>()).sum::<usize>())
                        .sum();
                    bytes.div_ceil(256)
                },
            })
            .collect::<Vec<_>>();
        DBS.with(|d| *d.borrow_mut() = (m.tables.iter().map(|t| t.db).collect(), m.session.aux));
        VINFO.with(|i| *i.borrow_mut() = crate::vtabs::summary(&m.vtabs));
        let mut schema = Schema { tables, ctes: Vec::new(), catalog: catalog.into() };
        // Each column's distinct non-NULL values among its first 256 rows,
        // as hashes that keep storage classes apart.
        let key = |v: &Value| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            match v {
                Value::Null => 0u8.hash(&mut h),
                Value::Int(i) => (1u8, i).hash(&mut h),
                Value::Real(r) => (2u8, r.to_bits()).hash(&mut h),
                Value::Text(s) => (3u8, s).hash(&mut h),
                Value::Blob(b) => (4u8, b).hash(&mut h),
            }
            h.finish()
        };
        let keys: Vec<Vec<std::collections::HashSet<u64>>> = m
            .tables
            .iter()
            .map(|t| {
                (0..t.columns)
                    .map(|c| {
                        t.rows
                            .iter()
                            .take(256)
                            .map(|row| column_value(t, row, c))
                            .filter(|v| *v != Value::Null)
                            .map(|v| key(&v))
                            .collect()
                    })
                    .collect()
            })
            .collect();
        for (t, cols) in keys.iter().enumerate() {
            for (c, here) in cols.iter().enumerate() {
                for (u, other) in keys.iter().enumerate() {
                    for (d, there) in other.iter().enumerate() {
                        if (t, c) != (u, d) && !here.is_disjoint(there) {
                            schema.tables[t].shared.push((c, u, d));
                        }
                    }
                }
            }
        }
        schema
    }
}

/// The first term of an expression index over column `c` of source 0
/// (see [`KeyExpr`]): `CREATE INDEX` renders it as a query of one source
/// renders it, so its tree is the one the planner compares a query's term
/// with.
pub fn key_expr(k: KeyExpr, c: usize) -> Expr {
    let col = Expr::col(0, c);
    let text = |s: &str| Expr::Lit(Value::Text(s.to_string()));
    match k {
        KeyExpr::Plus => Expr::bin(BinOp::Add, col, Expr::Lit(Value::Int(1))),
        KeyExpr::Lower => Expr::Call(Func::Lower, vec![col]),
        KeyExpr::Length => Expr::Call(Func::Length, vec![col]),
        KeyExpr::Integer => Expr::Cast(bx(col), CastType::Integer),
        KeyExpr::Date => Expr::Call(Func::Date, vec![col]),
        KeyExpr::Unixepoch => Expr::Call(Func::Unixepoch, vec![col]),
        KeyExpr::Strftime => Expr::Call(Func::Strftime, vec![text(ALL_CONVERSIONS), col]),
        KeyExpr::Datetime => Expr::Call(Func::Datetime, vec![col, text("start of month"), text("+1 month"), text("-1 day"), text("subsec")]),
        KeyExpr::Timediff => Expr::Call(Func::Timediff, vec![col, text("2024-01-15 12:00:00")]),
        KeyExpr::Julianday => Expr::Call(Func::Julianday, std::iter::once(col.clone()).chain(JULIANDAY_CHAIN.iter().map(|m| text(m))).collect()),
        KeyExpr::Time => Expr::Call(Func::Time, std::iter::once(col.clone()).chain(TIME_CHAIN.iter().map(|m| text(m))).collect()),
        // `rtrim` of a set of a two-byte character too.
        KeyExpr::Trim => Expr::Call(Func::Rtrim, vec![Expr::Call(Func::Trim, vec![col, text(" a0")]), text("é1")]),
        // Then every `1` dropped: `replaceFunc` shrinks its output.
        KeyExpr::Replace => Expr::Call(Func::Replace, vec![Expr::Call(Func::Replace, vec![col, text("a"), text("Bc")]), text("1"), text("")]),
        KeyExpr::Quote => Expr::Call(Func::Quote, vec![col]),
        // The column as text: concat_ws() of a BLOB in a UTF-16 database
        // is wrong on the first row it converts (the known SQLite bug
        // sqlite-concat-blob-utf16-length), so CREATE INDEX and a later
        // read would compute different keys.
        // Text and numbers of the column besides (never a BLOB, as above):
        // `octet_length`, `instr`, `substr` of its text from the end,
        // `soundex`, the length of `unhex`, `sign`, `round` to two digits,
        // `abs` of its real (no integer, which may overflow), the character
        // of its first code point (`char(unicode(c))`), whether it is JSON5
        // text or strict JSONB (`json_valid(c, 10)`), and where its JSON
        // goes wrong (`json_error_position`), and its text as the JSON
        // SQLite makes of it where it is JSON5 (`json` of it guarded by
        // `json_valid(.., 2)`: the minified RFC 8259 text, by
        // `jsonTranslateTextToBlob` and `jsonTranslateBlobToText`).
        KeyExpr::ConcatWs => {
            let as_text = Expr::Cast(bx(col.clone()), CastType::Text);
            let as_json = as_text.clone();
            let int = |i: i64| Expr::Lit(Value::Int(i));
            Expr::Call(
                Func::ConcatWs,
                vec![
                    text("-"),
                    as_text.clone(),
                    Expr::Call(Func::Length, vec![col.clone()]),
                    Expr::Call(Func::OctetLength, vec![col.clone()]),
                    Expr::Call(Func::Instr, vec![col.clone(), text("a")]),
                    Expr::Call(Func::Substr, vec![as_text, int(-3), int(2)]),
                    Expr::Call(Func::Soundex, vec![col.clone()]),
                    Expr::Call(Func::Length, vec![Expr::Call(Func::Unhex, vec![col.clone(), text(" -")])]),
                    Expr::Call(Func::Sign, vec![col.clone()]),
                    Expr::Call(Func::Round, vec![col.clone(), int(2)]),
                    Expr::Call(Func::Abs, vec![Expr::Cast(bx(col.clone()), CastType::Real)]),
                    Expr::Call(Func::Char, vec![Expr::Call(Func::Unicode, vec![col.clone()])]),
                    Expr::Call(Func::JsonValid, vec![col.clone(), int(10)]),
                    Expr::Call(Func::JsonErrorPosition, vec![col.clone()]),
                    Expr::Call(Func::Json, vec![Expr::Case(None, vec![(Expr::Call(Func::JsonValid, vec![as_json.clone(), int(2)]), as_json)], None)]),
                ],
            )
        },
        KeyExpr::Format => {
            let date = |f: Func| Expr::Call(f, vec![col.clone()]);
            let mut args = vec![text(PRINTF_ALL), col.clone(), date(Func::Datetime), date(Func::Julianday), date(Func::Unixepoch), date(Func::Date)];
            args.extend(std::iter::repeat_n(col, PRINTF_ARGS));
            Expr::Call(Func::Format, args)
        },
    }
}

/// A statement predicate as an expression over source 0: a partial
/// index's `WHERE`, rendered by `CREATE INDEX` as a query of one source
/// renders it.
pub fn pred_expr(p: &Pred) -> Expr {
    match p {
        Pred::Cmp(c, op, v) => {
            let op = match op {
                CmpOp::Eq => BinOp::Eq,
                CmpOp::Ne => BinOp::Ne,
                CmpOp::Lt => BinOp::Lt,
                CmpOp::Le => BinOp::Le,
                CmpOp::Gt => BinOp::Gt,
                CmpOp::Ge => BinOp::Ge,
            };
            Expr::bin(op, Expr::col(0, *c), Expr::Lit(v.clone()))
        },
        Pred::IsNull(c, not) => Expr::IsNull(bx(Expr::col(0, *c)), *not),
        Pred::Between(c, lo, hi) => Expr::Between(bx(Expr::col(0, *c)), bx(Expr::Lit(lo.clone())), bx(Expr::Lit(hi.clone())), false),
        Pred::And(a, b) => Expr::bin(BinOp::And, pred_expr(a), pred_expr(b)),
        Pred::Or(a, b) => Expr::bin(BinOp::Or, pred_expr(a), pred_expr(b)),
        Pred::Not(a) => Expr::Not(bx(pred_expr(a))),
    }
}

/// Every literal of `e` one SQL can carry ([`value_ok`]).
fn literals_ok(e: &Expr) -> bool {
    let mut ok = true;
    e.walk(&mut |n| ok &= !matches!(n, Expr::Lit(v) if !value_ok(v)));
    ok
}

/// A value short enough to be a literal (text and blobs of 40 bytes or
/// fewer).
fn short(v: &Value) -> bool {
    match v {
        Value::Text(s) => s.len() <= 40,
        Value::Blob(b) => b.len() <= 40,
        _ => true,
    }
}

// ------------------------------------------------------------------ drawing

const TEXTS: [&str; 24] = [
    "", "a", "A", "abc", "ABC", "é", "É", "中文", "😀", "0", "1", "-1", " 2 ", "1e3", "0x10", "3.0", "a%", "%", "_", "*", "?", "[a]",
    "NULL", "9223372036854775808",
];
const ALPHABET: [char; 12] = ['a', 'b', 'z', 'A', 'Z', '0', '1', ' ', 'é', 'ж', '中', '😀'];
const REALS: [f64; 18] = [
    -0.0,
    0.0,
    0.5,
    -1.5,
    1.0,
    3.0,
    1e308,
    -1e308,
    f64::MAX,
    5e-324,
    2.2250738585072014e-308,
    1e-300,
    9_223_372_036_854_775_808.0,
    -9_223_372_036_854_775_808.0,
    1e15 + 0.5,
    f64::INFINITY,
    f64::NEG_INFINITY,
    4503599627370497.0,
];
const INTS: [i64; 10] =
    [i64::MIN, i64::MAX, i64::MIN + 1, i64::MAX - 1, 1 << 53, (1 << 53) + 1, 1 << 31, -(1 << 31), 4_294_967_296, 255];

fn short_text(g: &mut Source) -> String {
    (0..g.int(0..=4)).map(|_| *g.pick(&ALPHABET)).collect()
}

/// A literal: a value the table holds now and then, else one of any
/// storage class, extremes included.
pub fn draw_value(g: &mut Source, t: &TableInfo) -> Value {
    if !t.samples.is_empty() && g.bool_with(0.4) {
        return g.pick(&t.samples).clone();
    }
    if g.bool_with(0.1) {
        return draw_date(g);
    }
    match g.int(0..20) {
        0..=1 => Value::Null,
        2..=6 => Value::Int(match g.int(0..10) {
            0..=5 => g.int(-3..=3),
            6..=7 => g.int(-1000..=1000),
            _ => *g.pick(&INTS),
        }),
        7..=10 => Value::Real(if g.bool_with(0.6) { *g.pick(&REALS) } else { ((g.f64() * 200.0 - 100.0) * 4.0).round() / 4.0 }),
        11..=15 => Value::Text(if g.bool_with(0.6) { (*g.pick(&TEXTS)).to_string() } else { short_text(g) }),
        _ => Value::Blob(match g.int(0..3) {
            0 => g.pick(&[&b""[..], b"\x00", b"\x01", b"\xff", b"ab", b"12"]).to_vec(),
            _ => match crate::model::not_now(Value::Blob((0..g.int(0..=4)).map(|_| g.int(0u8..=255)).collect())) {
                Value::Blob(b) => b,
                _ => unreachable!("a blob stays one"),
            },
        }),
    }
}

/// Rows a query may read in all: the product of its sources' row counts,
/// times the rows of the queries a correlated subquery runs for. Keeps a
/// join or a subquery from making a query slow; a table of 0 rows counts
/// as 1.
pub const ROW_BUDGET: u64 = 20_000;

/// Rows a window function that reads a frame ([`WinFunc::framed`]) may
/// read in all its frames ([`Window::frames_fit`]): with EXCLUDE (but NO
/// OTHERS), and in plans-noopt for every frame (the WindowFunc
/// optimization off), SQLite computes the function over the whole frame
/// for every row (`windowFullScan`), where it otherwise adds and removes
/// the rows that come and go. Over the default frame of a 20000-row join
/// that is 2*10^8 rows read, 3 to 10 seconds for each window (run
/// 20261016-1540's timeouts); at this budget, 2000 rows.
pub const FRAME_ROWS: u64 = 4_000_000;

/// Rows `group_concat` and `json_group_array` windows may read in all
/// their frames: each returns, for every row, a text of its frame's
/// values, which the oracles keep for each variant of the query; over the
/// default frame of a 20000-row join, gigabytes of text (two workers of
/// run 20261016-1540 killed over 8 GB). At this budget, 500 rows over the
/// default frame, a ROWS frame of 11 rows over 20000 rows.
pub const CONCAT_FRAME_ROWS: u64 = 250_000;

/// What an expression being drawn may use.
#[derive(Clone)]
pub struct Cx<'a> {
    pub schema: &'a Schema,
    /// The visible sources' tables of each level, outermost first; the
    /// last is the query the expression is in.
    pub levels: Vec<Vec<usize>>,
    /// The outermost level whose columns may be drawn: below the last
    /// level in a correlated subquery.
    pub min_level: usize,
    /// Rows a subquery drawn here may read (0: no subquery).
    pub budget: u64,
    /// In the items or HAVING of an aggregate query: its GROUP BY terms
    /// (none without GROUP BY). Leaves are then terms, aggregates and
    /// literals, never bare columns ([`grouped_ok`]).
    pub grouped: Option<Vec<Expr>>,
}

impl Cx<'_> {
    fn here(&self) -> &[usize] {
        self.levels.last().map_or(&[], |l| l.as_slice())
    }

    /// The table a column of this context names.
    fn table_of(&self, c: &Col) -> &TableInfo {
        self.schema.info(self.levels[self.levels.len() - 1 - c.up][c.src]).unwrap()
    }

    /// A table of the query's sources, for literals.
    fn some_table(&self, g: &mut Source) -> &TableInfo {
        self.schema.info(*g.pick(self.here())).unwrap()
    }

    /// A column: of the expression's own query mostly, of an enclosing
    /// one (from `min_level`) a third of the time when there is one.
    fn draw_col(&self, g: &mut Source) -> Col {
        let top = self.levels.len() - 1;
        let level = if self.min_level < top && g.bool_with(0.35) { g.int(self.min_level..=top - 1) } else { top };
        let src = g.index(self.levels[level].len());
        let columns = self.schema.info(self.levels[level][src]).unwrap().columns;
        Col { up: top - level, src, col: g.index(columns) }
    }

    /// Without aggregates or subqueries: for an aggregate's argument.
    fn rows_only(&self) -> Self {
        Cx { min_level: self.levels.len() - 1, budget: 0, grouped: None, ..self.clone() }
    }

    fn can_nest(&self) -> bool {
        self.levels.len() < MAX_LEVELS && self.schema.tables.iter().chain(&self.schema.ctes).any(|t| (t.rows.max(1) as u64) <= self.budget)
    }
}

/// An aggregate of the context's query rows: `count(*)`, `count(e)`,
/// `min`/`max` of a `CAST(e AS INTEGER|TEXT)`, `sum` of a `CAST(e AS
/// INTEGER)` (see [`agg_arg_ok`]).
fn draw_agg(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    let e = draw_agg_any(g, cx, depth);
    // A cast to TEXT of a collated value maps equal values apart.
    if casts_binary(&e, cx.schema, &cx.levels) {
        e
    } else {
        Expr::Agg(Agg::Count, None)
    }
}

fn draw_agg_any(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if g.bool_with(0.3) {
        return draw_ordered(g, cx, depth);
    }
    let rows = cx.rows_only();
    let arg = |g: &mut Source| if g.bool_with(0.6) { Expr::Col(rows.draw_col(g)) } else { draw_expr(g, &rows, depth.max(2)) };
    match g.int(0..20) {
        0..=4 => Expr::Agg(Agg::Count, None),
        5..=8 => Expr::Agg(Agg::Count, Some(bx(arg(g)))),
        9..=15 => {
            let ty = if g.bool_with(0.6) { CastType::Integer } else { CastType::Text };
            Expr::Agg(if g.bool() { Agg::Min } else { Agg::Max }, Some(bx(Expr::Cast(bx(arg(g)), ty))))
        },
        _ => Expr::Agg(Agg::Sum, Some(bx(Expr::Cast(bx(arg(g)), CastType::Integer)))),
    }
}

/// The separators of `group_concat` and `string_agg`.
const SEPARATORS: [&str; 6] = [",", "", " | ", "-", "é", ", "];

/// An aggregate with an ORDER BY of its own ([`ordered_ok`]) of the
/// context's query rows: mostly `group_concat` (with a separator 60% of the
/// time), `string_agg`, `json_group_array`, `json_group_object` (labels
/// mostly a `CAST` to TEXT, else a date call or a text literal), else
/// `count`, `sum`, `min` or `max` of the arguments they take; the values
/// mostly columns, else date calls or row expressions (made JSON values by
/// [`unblob`] where they must be). The ORDER BY: up to two leading terms
/// (columns, date calls, row expressions) and the tie terms
/// ([`Ordered::ties`]) in random order, each descending 30% of the time and
/// with NULLS FIRST or LAST 20%. DISTINCT 25% of the time with one
/// argument: of a `CAST` to INTEGER or TEXT ordered by itself for the
/// order-dependent ones.
fn draw_ordered(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    let rows = cx.rows_only();
    let d = depth.max(2);
    let value = |g: &mut Source| match g.int(0..10) {
        0..=4 => Expr::Col(rows.draw_col(g)),
        5..=6 => draw_date_call(g, &rows, d.min(3)),
        _ => draw_expr(g, &rows, d),
    };
    let json = |g: &mut Source| {
        let x = value(g);
        if value_arg_ok(&x) {
            x
        } else {
            unblob(x)
        }
    };
    let func = *g.weighted(&[
        (6, OrdFunc::GroupConcat),
        (3, OrdFunc::StringAgg),
        (4, OrdFunc::JsonGroupArray),
        (3, OrdFunc::JsonGroupObject),
        (1, OrdFunc::Count),
        (1, OrdFunc::Sum),
        (1, OrdFunc::Min),
        (1, OrdFunc::Max),
    ]);
    let sep = |g: &mut Source| Expr::Lit(Value::Text((*g.pick(&SEPARATORS)).to_string()));
    let mut args = match func {
        OrdFunc::GroupConcat if g.bool_with(0.6) => vec![value(g), sep(g)],
        OrdFunc::GroupConcat => vec![value(g)],
        OrdFunc::StringAgg => vec![value(g), sep(g)],
        OrdFunc::JsonGroupArray => vec![json(g)],
        OrdFunc::JsonGroupObject => {
            let k = match g.int(0..10) {
                0..=6 => Expr::Cast(bx(value(g)), CastType::Text),
                7..=8 => draw_date_call(g, &rows, 2),
                _ => Expr::Lit(Value::Text((*g.pick(&JSON_LABELS)).to_string())),
            };
            vec![if never_blob(&k) { k } else { Expr::Cast(bx(k), CastType::Text) }, json(g)]
        },
        OrdFunc::Count => vec![value(g)],
        OrdFunc::Sum => vec![Expr::Cast(bx(value(g)), CastType::Integer)],
        OrdFunc::Min | OrdFunc::Max => vec![Expr::Cast(bx(value(g)), if g.bool_with(0.6) { CastType::Integer } else { CastType::Text })],
    };
    let distinct = args.len() == 1 && g.bool_with(0.25);
    let ordered = func.agg().is_none();
    if distinct && ordered && !matches!(args[0], Expr::Cast(_, CastType::Integer | CastType::Text)) {
        args[0] = Expr::Cast(bx(args[0].clone()), if g.bool() { CastType::Text } else { CastType::Integer });
    }
    let mut keys: Vec<Expr> = Vec::new();
    if distinct && ordered {
        keys.push(args[0].clone());
    } else {
        for _ in 0..*g.weighted(&[(3, 0usize), (5, 1), (2, 2)]) {
            keys.push(match g.int(0..10) {
                0..=5 => Expr::Col(rows.draw_col(g)),
                6..=7 => draw_date_call(g, &rows, 2),
                _ => draw_expr(g, &rows, 2),
            });
        }
    }
    let mut ties = Ordered::ties(func, &args);
    if distinct && ordered && g.bool() {
        keys.clear();
    }
    shuffle(g, &mut ties);
    for t in ties {
        let k = g.int(0..=keys.len());
        keys.insert(k, t);
    }
    if keys.is_empty() {
        keys.push(Expr::Col(rows.draw_col(g)));
    }
    let order = keys.into_iter().map(|e| Order { key: OrderKey::Expr(e), desc: g.bool_with(0.3), nulls: g.bool_with(0.2).then(|| g.bool()), by: By::Number }).collect();
    let o = Ordered { func, distinct, args, order };
    if ordered_ok(&o) {
        Expr::Ordered(Box::new(o))
    } else {
        // A `jsonb` call drawn somewhere in it: a count instead.
        Expr::Agg(Agg::Count, None)
    }
}

/// A term: a column, or in an aggregate query a GROUP BY term or an
/// aggregate.
fn term(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    match &cx.grouped {
        Some(keys) if !keys.is_empty() && g.bool_with(0.55) => g.pick(keys).clone(),
        Some(_) => draw_agg(g, cx, depth),
        None => Expr::Col(cx.draw_col(g)),
    }
}

fn leaf(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if g.bool_with(0.55) {
        term(g, cx, depth)
    } else {
        let t = cx.some_table(g);
        Expr::Lit(draw_value(g, t))
    }
}

/// A LIKE or GLOB pattern: mostly a literal from a value with wildcards
/// put in (or none), now and then any expression.
fn draw_pattern(g: &mut Source, cx: &Cx, glob: bool, depth: usize) -> Expr {
    if g.bool_with(0.25) {
        return draw_expr(g, cx, depth);
    }
    let t = cx.some_table(g);
    let texts: Vec<&String> = t.samples.iter().filter_map(|v| if let Value::Text(s) = v { Some(s) } else { None }).collect();
    let base = if !texts.is_empty() && g.bool_with(0.6) { (*g.pick(&texts)).clone() } else { short_text(g) };
    let mut chars: Vec<String> = base.chars().map(String::from).collect();
    let wild: &[&str] = if glob { &["*", "?", "[a-z]", "[^a]", "*"] } else { &["%", "_", "%"] };
    let edits = if g.bool_with(0.3) { 0 } else { g.int(1..=2) };
    for _ in 0..edits {
        let w = (*g.pick(wild)).to_string();
        if !chars.is_empty() && g.bool() {
            let k = g.index(chars.len());
            chars[k] = w;
        } else {
            let k = g.int(0..=chars.len());
            chars.insert(k, w);
        }
    }
    // Half the time (by [`Side`]: no draw) a form more ([`GLOB_MORE`],
    // [`LIKE_MORE`]) in place of a character or put in, or (a LIKE) the
    // case of its ASCII letters changed, which LIKE folds.
    let mut side = Side::of(&chars.concat(), if glob { 0x676c } else { 0x6c6b });
    match side.take(6) {
        0..=1 if !chars.is_empty() => {
            let k = side.take(chars.len());
            chars[k] = (*side.pick(if glob { &GLOB_MORE[..] } else { &LIKE_MORE[..] })).to_string();
        },
        0..=2 => {
            let k = side.take(chars.len() + 1);
            chars.insert(k, (*side.pick(if glob { &GLOB_MORE[..] } else { &LIKE_MORE[..] })).to_string());
        },
        3 if !glob => {
            for c in &mut chars {
                if side.take(2) == 0 {
                    *c = if c.chars().all(|x| x.is_ascii_lowercase()) { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
                }
            }
        },
        _ => {},
    }
    Expr::Lit(Value::Text(chars.concat()))
}

/// GLOB forms [`draw_pattern`] puts in now and then: classes of ranges, a
/// `]` or `-` as a member, negated classes, the wildcards as members (a
/// class is GLOB's only escape), runs of wildcards, a class never closed
/// (which matches nothing), a range of a non-ASCII end.
const GLOB_MORE: [&str; 16] =
    ["[0-9]", "[]]", "[^]a]", "[a-c-]", "[-z]", "[*]", "[?]", "[[]", "[A-Za-z]", "[^0-9]", "**", "*?*", "?*?", "[a-é]", "[", "[^-]"];

/// LIKE forms [`draw_pattern`] puts in now and then: runs of wildcards, an
/// upper-case letter (LIKE folds ASCII case), the escape character
/// (written escaped when the pattern has an ESCAPE clause, [`escaped`]),
/// characters GLOB reads as wildcards (plain in LIKE).
const LIKE_MORE: [&str; 12] = ["%%", "_%", "%_%", "__", "A", "Z", "!", "*", "[a]", "%a%", "_!", "?"];

/// A short text from a value the tables hold (a character or a few of
/// one, so that `replace` and `trim` find it), else a short text.
fn text_piece(g: &mut Source, cx: &Cx) -> String {
    let t = cx.some_table(g);
    let texts: Vec<&String> = t.samples.iter().filter_map(|v| if let Value::Text(s) = v { Some(s) } else { None }).filter(|s| !s.is_empty()).collect();
    if texts.is_empty() || g.bool_with(0.3) {
        return short_text(g);
    }
    let chars: Vec<char> = g.pick(&texts).chars().collect();
    let n = g.int(1..=3.min(chars.len()));
    let at = g.int(0..=chars.len() - n);
    chars[at..at + n].iter().collect()
}

/// Escapes `unistr` reads (lang_corefunc.html#unistr), and plain text.
const UNISTR_PIECES: [&str; 9] = ["\\u00e9", "\\+01F600", "\\0041", "\\U0001F600", "\\\\", "a\\u0062c", "\\00E9", "x", "\\uFFFD"];

/// A call of one of [`Func::TEXT`] at most `depth` levels deep (`depth` >=
/// 2): its main argument any expression (columns most often), the others
/// mostly literals from the tables' text, so that `trim` and `replace`
/// change something.
fn draw_text_call(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    let d = depth - 1;
    let sub = |g: &mut Source| draw_expr(g, cx, d);
    let text = |s: String| Expr::Lit(Value::Text(s));
    let f = *g.weighted(&[
        (3, Func::Trim),
        (2, Func::Ltrim),
        (2, Func::Rtrim),
        (4, Func::Replace),
        (2, Func::Char),
        (3, Func::Concat),
        (2, Func::ConcatWs),
        (2, Func::Unhex),
        (2, Func::Unistr),
        (1, Func::Soundex),
        (2, Func::Sign),
        (2, Func::OctetLength),
        (1, Func::Zeroblob),
        (2, Func::Quote),
        (2, Func::Likely),
        (2, Func::Unlikely),
        (2, Func::Likelihood),
    ]);
    let args = match f {
        Func::Trim | Func::Ltrim | Func::Rtrim => {
            let x = sub(g);
            match g.int(0..10) {
                0..=4 => vec![x],
                5..=8 => {
                    let mut set = text_piece(g, cx);
                    if g.bool_with(0.3) {
                        set.push(' ');
                    }
                    vec![x, text(set)]
                },
                _ => vec![x, sub(g)],
            }
        },
        Func::Replace => {
            let x = sub(g);
            let pat = match g.int(0..20) {
                0 => text(String::new()),
                1..=14 => text(text_piece(g, cx)),
                _ => sub(g),
            };
            let with = if g.bool_with(0.7) { text(text_piece(g, cx)) } else { draw_expr(g, cx, 1) };
            vec![x, pat, with]
        },
        Func::Char => (0..g.int(1..=3))
            .map(|_| match g.int(0..10) {
                0..=3 => Expr::Lit(Value::Int(*g.pick(&[65, 97, 0x20, 0xe9, 0x3a9, 0x4e2d, 0x1f600, 0x10ffff, 0x110000, -1]))),
                4..=6 => Expr::Call(Func::Coalesce, vec![Expr::Call(Func::Unicode, vec![sub(g)]), Expr::Lit(Value::Int(97))]),
                _ => sub(g),
            })
            .collect(),
        Func::Concat => (0..g.int(1..=4)).map(|_| sub(g)).collect(),
        Func::ConcatWs => {
            let sep = if g.bool_with(0.75) { text((*g.pick(&[",", " ", "", "-", "ab"])).to_string()) } else { sub(g) };
            std::iter::once(sep).chain((0..g.int(1..=4)).map(|_| sub(g))).collect()
        },
        Func::Unhex => {
            let x = match g.int(0..4) {
                0 | 1 => Expr::Call(Func::Hex, vec![sub(g)]),
                2 => text((*g.pick(&["41 42", "4a4B", "zz", "123", "", "00ff", "-41-", "e9"])).to_string()),
                _ => sub(g),
            };
            if g.bool_with(0.35) {
                let ignore = if g.bool_with(0.8) { text((*g.pick(&[" ", "-", " -", "z"])).to_string()) } else { sub(g) };
                vec![x, ignore]
            } else {
                vec![x]
            }
        },
        Func::Unistr => {
            let part = |g: &mut Source| {
                if g.bool() {
                    text((*g.pick(&UNISTR_PIECES)).to_string())
                } else {
                    Expr::Call(Func::Replace, vec![sub(g), text("\\".into()), text("\\\\".into())])
                }
            };
            let first = part(g);
            vec![if g.bool_with(0.4) { Expr::bin(BinOp::Concat, first, part(g)) } else { first }]
        },
        // Read as a BLOB only ([`zeroblobs_read_as_blobs`]).
        Func::Zeroblob => {
            let n = match g.int(0..3) {
                0 => Expr::Lit(Value::Int(g.int(0..=40))),
                1 => Expr::Call(Func::Length, vec![sub(g)]),
                _ => Expr::Call(Func::OctetLength, vec![sub(g)]),
            };
            return Expr::Call(*g.pick(&BLOB_READERS), vec![Expr::Call(Func::Zeroblob, vec![n])]);
        },
        Func::Likelihood => vec![sub(g), Expr::Lit(Value::Real(*g.pick(&[0.0, 0.0625, 0.5, 0.9375, 1.0])))],
        _ => vec![sub(g)],
    };
    // A third of the `concat` and `concat_ws` calls (by [`Side`]: no draw)
    // a `printf` or `format` of the same values, under a format of one
    // conversion an argument ([`draw_format`]).
    if matches!(f, Func::Concat | Func::ConcatWs) {
        let mut side = Side::of(&format!("{args:?}"), 0x7066);
        if side.take(3) == 0 {
            let format = draw_format(&mut side, args.len());
            let pf = if side.take(2) == 0 { Func::Printf } else { Func::Format };
            return Expr::Call(pf, std::iter::once(text(format)).chain(args).collect());
        }
    }
    Expr::Call(f, args)
}

/// The conversions of `printf` (lang_corefunc.html#printf, and `%j` and
/// `%J`, JSON strings, of trunk), the flags, and widths and precisions
/// that [`draw_format`] takes from.
const PRINTF_CONVERSIONS: [char; 24] = ['d', 'i', 'u', 'f', 'e', 'E', 'g', 'G', 'x', 'X', 'o', 'c', 's', 'z', 'q', 'Q', 'w', 'j', 'J', 'r', 'p', 'n', 'd', 's'];
const PRINTF_FLAGS: [&str; 12] = ["", "", "", "-", "+", " ", "#", "!", "0", ",", "-0", "+#"];
const PRINTF_TEXT: [&str; 8] = ["", "", "", "|", " ", "x=", "%%", "é"];

/// A `printf` format of `n` conversions ([`format_ok`]: no `*`, widths
/// and precisions of at most two digits), chosen by `side`: each with
/// flags, a width, a precision and a length (`l`, `ll`) now and then, text
/// between them; now and then a format SQLite stops reading at (an
/// unknown conversion, a lone `%` at the end), which leaves what it read.
fn draw_format(side: &mut Side, n: usize) -> String {
    let mut out = String::new();
    for _ in 0..n {
        out += side.pick(&PRINTF_TEXT);
        out.push('%');
        out += side.pick(&PRINTF_FLAGS);
        if side.take(3) == 0 {
            // Two digits at most with a `0` flag before them.
            out += &side.take(if out.ends_with('0') { 10 } else { 25 }).to_string();
        }
        if side.take(3) == 0 {
            out += &format!(".{}", side.take(21));
        }
        match side.take(8) {
            0 => out.push('l'),
            1 => out += "ll",
            _ => {},
        }
        out.push(*side.pick(&PRINTF_CONVERSIONS));
    }
    match side.take(12) {
        0 => out += "%y",
        1 => out.push('%'),
        2 => out += " %s",
        _ => {},
    }
    out
}

/// Whether `unistr` reads text `s` without an invalid escape: a backslash
/// is followed by another, by 4 hex digits, `+` and 6, `u` and 4, or `U`
/// and 8 (`unistrFunc`).
fn unistr_text_ok(s: &str) -> bool {
    let b = s.as_bytes();
    let hex = |at: usize, n: usize| b.len() >= at + n && b[at..at + n].iter().all(u8::is_ascii_hexdigit);
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let (ok, skip) = match b.get(i + 1) {
            Some(b'\\') => (true, 2),
            Some(c) if c.is_ascii_hexdigit() => (hex(i + 1, 4), 5),
            Some(b'+') => (hex(i + 2, 6), 8),
            Some(b'u') => (hex(i + 2, 4), 6),
            Some(b'U') => (hex(i + 2, 8), 10),
            _ => (false, 0),
        };
        if !ok {
            return false;
        }
        i += skip;
    }
    true
}

/// An argument of `unistr` that never holds an invalid escape: a text
/// literal without one, `replace(x, '\', '\\')` (every backslash doubled),
/// NULL, or a concatenation of such (each escape is whole within its part).
pub fn unistr_ok(e: &Expr) -> bool {
    match e {
        Expr::Lit(Value::Text(s)) => unistr_text_ok(s),
        Expr::Lit(Value::Null) => true,
        Expr::Call(Func::Replace, a) => {
            matches!(a.as_slice(), [_, Expr::Lit(Value::Text(p)), Expr::Lit(Value::Text(w))] if p == "\\" && w == "\\\\")
        },
        Expr::Bin(BinOp::Concat, a, b) => unistr_ok(a) && unistr_ok(b),
        _ => false,
    }
}

/// An argument of `zeroblob` of a bounded size: a small integer literal,
/// NULL, or the length of a value (`length`, `octet_length`), never one
/// SQLite refuses as too big.
pub fn zeroblob_ok(e: &Expr) -> bool {
    match e {
        Expr::Lit(Value::Int(n)) => (0..=64).contains(n),
        Expr::Lit(Value::Null) => true,
        Expr::Call(Func::Length | Func::OctetLength, a) => a.len() == 1,
        _ => false,
    }
}

/// Object labels, and the labels of paths: drawn from the same few, a
/// path finds an element now and then.
const JSON_LABELS: [&str; 6] = ["a", "b", "c", "x y", "é", "0"];
/// JSON strings' contents, as written in JSON (escapes included).
const JSON_STRINGS: [&str; 12] =
    ["", "a", "x\\\"y", "\\\\", "\\n", "\\u00e9", "é", "\\ud83d\\ude00", "\\t\\/", "1", "null", "a b"];
const JSON_NUMBERS: [&str; 16] = [
    "0",
    "-1",
    "1",
    "42",
    "1.5",
    "-0.0",
    "1e3",
    "1E-2",
    "-2.5e-3",
    "0.1",
    "9223372036854775807",
    "-9223372036854775808",
    "9223372036854775808",
    "1e308",
    "1e999",
    "123456789012345678901234567890",
];
/// Texts for the arguments that take anything (`json_valid`,
/// `json_error_position`): JSON5, and text that is not JSON at all.
const NOT_JSON: [&str; 10] = ["", "[1,", "{\"a\"}", "tru", "\"\\x\"", "{a:1}", "0x1F", "[1,2,]", "'s'", " 2 "];

/// A JSON text (RFC 8259, [`json_text_ok`]) of at most `depth` levels of
/// objects and arrays: strings with escapes (now and then long, past the
/// one-byte JSONB size), integers past 64 bits, reals past the double
/// range, literals, now and then whitespace. The path of every element
/// (`at` for the text itself) is pushed on `paths`.
pub fn draw_json_text(g: &mut Source, depth: usize, out: &mut String, at: &str, paths: &mut Vec<String>) {
    draw_json_in(g, depth, out, at, paths, false);
}

/// A JSON5 text ([`json5_text_ok`]) as [`draw_json_text`] draws JSON, each
/// part now and then in a form only JSON5 has: labels unquoted or in
/// single quotes, strings in single quotes and with JSON5's escapes
/// ([`JSON5_STRINGS`]), numbers in hexadecimal, signed, with a bare
/// decimal point, `Infinity` and `NaN` ([`JSON5_NUMBERS`]), trailing
/// commas, JSON5's whitespace and comments ([`JSON5_SPACES`]). SQLite
/// reads it as the JSON it converts it to; its elements have the paths
/// they would in that JSON.
pub fn draw_json5_text(g: &mut Source, depth: usize, out: &mut String, at: &str, paths: &mut Vec<String>) {
    draw_json_in(g, depth, out, at, paths, true);
}

/// The share of JSON text literals drawn as JSON5 ([`json_literal`]).
const JSON5_SHARE: f64 = 0.5;

/// Whitespace and comments [`json5_text_ok`] reads between tokens.
pub const JSON5_SPACES: [&str; 10] = [" ", "\n", "\u{a0}", "\u{2028}", "\u{2029}", "\u{feff}", "\u{b}\u{c}", "/* c */", "// c\n", "\r\n\t"];
/// More of each part of a JSON or JSON5 text, which a drawn part gives way
/// to now and then (by the length of the text so far: [`more`], no draw),
/// so that the draws stay those of the parts before: whitespace
/// `json5Whitespace` reads by other branches (U+1680, U+2000, U+200A,
/// U+202F, U+205F, U+3000, an empty comment, line comments ended by a
/// carriage return or U+2028); JSON5 escapes `\b`, `\f`, `\r`, `\/`, `\"`,
/// a line continuation by U+2029, `\0` before a letter; JSON5 numbers
/// `0X`, a signed bare decimal point, signed exponents, hexadecimal zero
/// and past `i64`; JSON strings with control characters, CJK and the
/// last surrogate pair escaped, an escaped quote; JSON numbers `-0`,
/// `0e0`, `1E+2`, subnormal, underflowing to 0, past `u64`.
pub const JSON5_SPACES_MORE: [&str; 11] = ["\t", "\r", "\u{1680}", "\u{2000}", "\u{200a}", "\u{202f}", "\u{205f}", "\u{3000}", "/**/", "// c\r", "// c\u{2028}"];
pub const JSON5_STRINGS_MORE: [&str; 5] = ["\\b\\f\\r", "\\u00e9\\x7f", "\\/\\\"", "a\\\u{2029}b", "\\0z"];
pub const JSON5_NUMBERS_MORE: [&str; 8] = ["0X1f", "-.0", "+.5e+3", "0x0", ".0e0", "-0x8000000000000001", "+0.0", "5.e2"];
const JSON_STRINGS_MORE: [&str; 8] = ["\\b\\f\\r", "\\u0001\\u001F", "\\u4E2D\\u20ac", "中文", "\\\"\\\\\\/", "a\\u0022b", "\\uDBFF\\uDFFF", "\\u00ff\\u0080"];
const JSON_NUMBERS_MORE: [&str; 10] =
    ["-0", "0e0", "1E+2", "-1.5E+300", "4.9e-324", "1e-400", "18446744073709551616", "-9223372036854775809", "0.000001", "100000000000000000000.5"];

/// `drawn`, or a third of the time (by `at`, the length of the text so
/// far) one of `more`; never one with a single quote in place of one
/// without (whether a string is single-quoted is drawn only then).
fn more<'a>(drawn: &'a str, more: &[&'a str], at: usize) -> &'a str {
    if at.is_multiple_of(3) && !drawn.contains('\'') {
        more[(at / 3) % more.len()]
    } else {
        drawn
    }
}

/// String contents with the escapes JSON5 has and JSON lacks (a line
/// continuation, `\x`, `\v`, `\0`, `\'`), as [`json5_text_ok`] reads them.
pub const JSON5_STRINGS: [&str; 8] = ["\\x41", "\\v", "a\\0", "\\'", "a\\\nb", "a\\\r\nb", "x\\\u{2028}y", "\\x7e\\xC3"];
/// Numbers only JSON5 has.
pub const JSON5_NUMBERS: [&str; 16] = [
    "0x1F", "-0xa", "+0x0", "+1", "-.5e2", ".5", "5.", "+5.E-1", "Infinity", "-Infinity", "+Infinity", "NaN", "0xFFFFFFFFFFFFFFFFFFFF",
    "0x8000000000000000", "-0x8000000000000000", "+1.5e3",
];

fn draw_json_in(g: &mut Source, depth: usize, out: &mut String, at: &str, paths: &mut Vec<String>, json5: bool) {
    let space = |g: &mut Source, out: &mut String| {
        if g.bool_with(0.15) {
            if json5 && g.bool() {
                let sp = *g.pick(&JSON5_SPACES);
                out.push_str(more(sp, &JSON5_SPACES_MORE, out.len()));
            } else {
                out.push_str(g.pick(&[" ", "\n", " \t "]));
            }
        }
    };
    let string = |g: &mut Source, out: &mut String, s: &str| {
        let q = if json5 && !s.contains('\'') && g.bool_with(0.3) { '\'' } else { '"' };
        out.push(q);
        out.push_str(s);
        out.push(q);
    };
    let trailing = |g: &mut Source, out: &mut String, n: usize| {
        if json5 && n > 0 && g.bool_with(0.3) {
            out.push(',');
        }
    };
    paths.push(at.to_string());
    match g.int(0..10) {
        0..=3 if depth > 0 => {
            out.push('{');
            let mut labels = Vec::new();
            let n = g.int(0..=3);
            for k in 0..n {
                if k > 0 {
                    out.push(',');
                }
                space(g, out);
                let label = *g.pick(&JSON_LABELS);
                if json5 && label.len() == 1 && label.as_bytes()[0].is_ascii_alphabetic() && g.bool_with(0.4) {
                    out.push_str(label);
                } else {
                    let quoted = if label == "é" && g.bool() { "\\u00e9" } else { label };
                    string(g, out, quoted);
                }
                out.push(':');
                space(g, out);
                let step = if label == "x y" { format!(".\"{label}\"") } else { format!(".{label}") };
                // A path finds the first of duplicate labels only.
                let mut shadowed = Vec::new();
                let to = if labels.contains(&label) { &mut shadowed } else { &mut *paths };
                draw_json_in(g, depth - 1, out, &format!("{at}{step}"), to, json5);
                labels.push(label);
            }
            trailing(g, out, n);
            out.push('}');
        },
        4..=6 if depth > 0 => {
            out.push('[');
            let n = g.int(0..=4);
            for k in 0..n {
                if k > 0 {
                    out.push(',');
                }
                space(g, out);
                let step = if g.bool_with(0.3) { format!("[#-{}]", n - k) } else { format!("[{k}]") };
                draw_json_in(g, depth - 1, out, &format!("{at}{step}"), paths, json5);
            }
            trailing(g, out, n);
            space(g, out);
            out.push(']');
        },
        0..=7 if g.bool_with(0.1) => {
            let n = *g.pick(&[12, 40, 300]);
            string(g, out, &"ab\\n".repeat(n / 4 + 1));
        },
        0..=7 if json5 && g.bool_with(0.4) => {
            let s = more(g.pick(&JSON5_STRINGS), &JSON5_STRINGS_MORE, out.len());
            string(g, out, s);
        },
        0..=7 => {
            let s = more(g.pick(&JSON_STRINGS), &JSON_STRINGS_MORE, out.len());
            string(g, out, s);
        },
        8 if json5 && g.bool() => {
            let n = more(g.pick(&JSON5_NUMBERS), &JSON5_NUMBERS_MORE, out.len());
            out.push_str(n);
        },
        8 => {
            let n = more(g.pick(&JSON_NUMBERS), &JSON_NUMBERS_MORE, out.len());
            out.push_str(n);
        },
        _ => out.push_str(g.pick(&["true", "false", "null"])),
    }
}

/// An object for `json_patch` (RFC 8259, [`json_text_ok`]) of at most
/// `depth` levels: labels of [`JSON_LABELS`], so that a target and its
/// patch share some; values NULL (which the patch removes the label by),
/// objects (which it merges), else any JSON ([`draw_json_text`]).
fn draw_patch_text(g: &mut Source, depth: usize, out: &mut String) {
    out.push('{');
    for k in 0..g.int(1..=3) {
        if k > 0 {
            out.push(',');
        }
        out.push_str(&format!("\"{}\":", g.pick(&JSON_LABELS)));
        match g.int(0..10) {
            0..=2 => out.push_str("null"),
            3..=6 if depth > 1 => draw_patch_text(g, depth - 1, out),
            _ => draw_json_text(g, 1, out, "$", &mut Vec::new()),
        }
    }
    out.push('}');
}

/// A JSONB blob of at most `depth` levels of arrays and objects, its
/// elements of every type JSONB has (the JSON5 ones and the text forms
/// too: INT5, FLOAT5, TEXTJ, TEXT5, TEXTRAW), a fifth of the headers but
/// NULL's, true's and false's with a larger size field than the payload
/// needs (1, 2, 4 or 8 bytes), which SQLite reads as well. The path of every element is pushed on `paths`.
pub fn draw_jsonb(g: &mut Source, depth: usize, out: &mut Vec<u8>, at: &str, paths: &mut Vec<String>, ascii: bool) {
    fn header(g: &mut Source, out: &mut Vec<u8>, t: u8, payload: &[u8]) {
        let n = payload.len();
        // SQLite's strict check takes NULL, true and false only with the
        // size in the header byte.
        let wide = t > 2 && g.bool_with(0.2);
        let code: u8 = match n {
            0..=11 if !wide => n as u8,
            0..=255 if !wide || g.bool() => 12,
            0..=65535 if !wide || g.bool() => 13,
            _ if !wide || g.bool() => 14,
            _ => 15,
        };
        out.push(code << 4 | t);
        let size = n as u64;
        match code {
            12 => out.push(size as u8),
            13 => out.extend_from_slice(&(size as u16).to_be_bytes()),
            14 => out.extend_from_slice(&(size as u32).to_be_bytes()),
            15 => out.extend_from_slice(&size.to_be_bytes()),
            _ => {},
        }
        out.extend_from_slice(payload);
    }
    paths.push(at.to_string());
    match g.int(0..10) {
        0..=2 if depth > 0 => {
            let mut body = Vec::new();
            let n = g.int(0..=4);
            for k in 0..n {
                let step = if g.bool_with(0.3) { format!("[#-{}]", n - k) } else { format!("[{k}]") };
                draw_jsonb(g, depth - 1, &mut body, &format!("{at}{step}"), paths, ascii);
            }
            header(g, out, 11, &body);
        },
        3..=4 if depth > 0 => {
            let mut body = Vec::new();
            let mut labels = Vec::new();
            for _ in 0..g.int(0..=3) {
                let label = *g.pick(&["a", "b", "c"]);
                let t = *g.pick(&[7, 8, 9, 10]);
                header(g, &mut body, t, label.as_bytes());
                let mut shadowed = Vec::new();
                let to = if labels.contains(&label) { &mut shadowed } else { &mut *paths };
                draw_jsonb(g, depth - 1, &mut body, &format!("{at}.{label}"), to, ascii);
                labels.push(label);
            }
            header(g, out, 12, &body);
        },
        k => {
            let (t, payload): (u8, &str) = match k % 6 {
                0 => (g.int(0..=2), ""),
                1 => (3, g.pick(&["0", "42", "-7", "9223372036854775807", "123456789012345678901"])),
                2 => (4, g.pick(&["0x1F", "-0xa", "0xFFFFFFFFFFFFFFFFFF"])),
                3 => (*g.pick(&[5, 6]), g.pick(&["1.5", "-0.25", "1e3", "1e999"])),
                4 => (7, g.pick(if ascii { &["", "ab", "x y", "a"] } else { &["", "ab", "é", "x y"] })),
                _ => *g.pick(&[(8, "a\\nb"), (8, "\\u00e9\\\""), (9, "\\x41\\v"), (9, "a\\\nb"), (10, "a\"b\\"), (10, "\n")]),
            };
            let payload = if t == 6 && g.bool() { *g.pick(&[".5", "5.", "-.5e2"]) } else { payload };
            header(g, out, t, payload.as_bytes());
        },
    }
}

/// A JSONB blob ([`draw_jsonb`]) of up to three levels, and its elements'
/// paths; when `damage`, now and then (of ASCII payloads then) a byte of
/// it changed, cut short or with a byte past it, which SQLite may then
/// read as no JSONB (the paths are those of the blob before).
fn jsonb_literal(g: &mut Source, damage: bool) -> (Value, Vec<String>) {
    let (mut b, mut paths) = (Vec::new(), Vec::new());
    let levels = g.int(1..=3);
    // Damaged, a blob SQLite's check still passes may hold the changed
    // byte in a text: an ASCII one in ASCII, so that the text stays UTF-8.
    let damage = damage && g.bool_with(0.4);
    draw_jsonb(g, levels, &mut b, "$", &mut paths, damage);
    if damage {
        match g.int(0..3) {
            0 => {
                let k = g.index(b.len());
                b[k] = *g.pick(&[0x0b, 0x0c, 0x13, 0x2c, 0x5b, 0x0f, 0x7c, b'0', b'\\']);
            },
            1 => b.truncate(g.index(b.len())),
            _ => b.push(*g.pick(&[0x00, 0x01, 0x13])),
        }
    }
    (Value::Blob(b), paths)
}

/// A path: `$`, then up to three steps, labels from [`JSON_LABELS`]
/// (quoted when they must be, and now and then when not) and array
/// elements from the start or the end; ending at an array element when
/// `array`.
fn draw_path(g: &mut Source, array: bool) -> String {
    let mut s = "$".to_string();
    let steps = g.int(usize::from(array)..=3);
    for k in 0..steps {
        if g.bool() && !(array && k + 1 == steps) {
            match *g.pick(&JSON_LABELS) {
                l @ "x y" => s.push_str(&format!(".\"{l}\"")),
                l if g.bool_with(0.15) => s.push_str(&format!(".\"{l}\"")),
                l => s.push_str(&format!(".{l}")),
            }
        } else {
            s.push_str(g.pick(&["[0]", "[1]", "[2]", "[#]", "[#-1]", "[#-2]", "[9]"]));
        }
    }
    s
}

/// A path argument ([`JsonArg::Path`]): mostly the path of an element of
/// the JSON argument (`found`, when it is a literal), else any path, now
/// and then with a step past the element, or NULL.
fn path_arg(g: &mut Source, array: bool, found: &[String]) -> Expr {
    let found: Vec<&String> = found.iter().filter(|p| !array || p.ends_with(']')).collect();
    let p = match g.int(0..20) {
        0 => return Expr::Lit(Value::Null),
        1..=11 if !found.is_empty() => (*g.pick(&found)).clone(),
        12..=14 if !found.is_empty() => {
            let tail = draw_path(g, array);
            format!("{}{}", g.pick(&found), &tail[1..])
        },
        _ => draw_path(g, array),
    };
    Expr::Lit(Value::Text(p))
}

/// A JSON text literal of 1 to `levels` levels, and its elements' paths:
/// half of them JSON5 ([`draw_json5_text`]).
fn json_literal(g: &mut Source, levels: usize) -> (Expr, Vec<String>) {
    let (mut s, mut paths) = (String::new(), Vec::new());
    let levels = g.int(1..=levels);
    if g.bool_with(JSON5_SHARE) {
        draw_json5_text(g, levels, &mut s, "$", &mut paths);
    } else {
        draw_json_text(g, levels, &mut s, "$", &mut paths);
    }
    (Expr::Lit(Value::Text(s)), paths)
}

/// A JSON argument ([`json_ok`]) of at most `depth` levels, and the paths
/// of its elements when it is a JSON text literal: mostly such a literal
/// (now and then JSON5), else a JSON function's value, a value of the
/// query that `json_valid` guards, or NULL.
fn draw_json_doc(g: &mut Source, cx: &Cx, depth: usize) -> (Expr, Vec<String>) {
    let (doc, paths) = draw_json_doc_in(g, cx, depth);
    match &doc {
        Expr::Lit(Value::Text(s)) => match json_column(cx, s) {
            Some(guarded) => (guarded, paths),
            None => (doc, paths),
        },
        _ => (doc, paths),
    }
}

/// A third of the time (by [`Side`] of a drawn JSON text literal `s`: no
/// draw moves) in its place a column of the query's sources that holds
/// JSON (by its values, `TableInfo::by_column`: an object or array
/// [`json5_text_ok`] reads), guarded by `json_valid` (`CASE WHEN
/// json_valid(c, 2) THEN c END`, of flags 2, 10, 11 or 3 that read JSON5
/// text too, or none, RFC 8259 only, and 10 and 11 strict JSONB too:
/// [`json_ok`]; a blob column holds JSONB, `crate::files`), so that the JSON function
/// parses and converts a value of every row it reads
/// (`jsonTranslateTextToBlob`, `json5Whitespace`) and looks up its paths
/// there (`jsonLookupStep`). Not in an aggregate query's items or HAVING
/// (no bare column there).
fn json_column(cx: &Cx, s: &str) -> Option<Expr> {
    let mut side = Side::of(s, 0x6a63);
    if cx.grouped.is_some() || side.take(3) != 0 {
        return None;
    }
    let holds = |v: &Value| match v {
        Value::Text(t) => t.trim_start().starts_with(['{', '[']) && json5_text_ok(t),
        // JSONB of an array or an object by its header, which the guard
        // checks strictly (flags 10 and 11; none other passes a blob).
        Value::Blob(b) => b.first().is_some_and(|h| matches!(h & 0x0f, 11 | 12)),
        _ => false,
    };
    let cols: Vec<Col> = cx
        .here()
        .iter()
        .enumerate()
        .filter_map(|(src, &t)| cx.schema.info(t).map(|i| (src, i)))
        .flat_map(|(src, i)| i.by_column.iter().enumerate().filter(|(_, vs)| vs.iter().any(holds)).map(move |(col, _)| Col { up: 0, src, col }))
        .collect();
    if cols.is_empty() {
        return None;
    }
    let x = Expr::Col(*side.pick(&cols));
    let mut args = vec![x.clone()];
    if let &Some(f) = side.pick(&[Some(2), Some(10), Some(3), None, Some(10), Some(11)]) {
        args.push(Expr::Lit(Value::Int(f)));
    }
    Some(Expr::Case(None, vec![(Expr::Call(Func::JsonValid, args), x)], None))
}

fn draw_json_doc_in(g: &mut Source, cx: &Cx, depth: usize) -> (Expr, Vec<String>) {
    match g.int(0..20) {
        _ if depth <= 1 => json_literal(g, 2),
        10..=11 => (Expr::Lit(Value::Text((*g.pick(&JSON5_DOCS)).to_string())), Vec::new()),
        12 => (Expr::Lit(Value::Null), Vec::new()),
        // A JSONB blob, which `json_valid(x, 8)` guards: damaged now and
        // then, NULL then.
        0..=1 => {
            let (b, paths) = jsonb_literal(g, true);
            let x = Expr::Lit(b);
            (Expr::Case(None, vec![(Expr::Call(Func::JsonValid, vec![x.clone(), Expr::Lit(Value::Int(8))]), x)], None), paths)
        },
        13..=14 if depth >= 3 => {
            let x = if depth >= 4 && g.bool_with(0.3) { draw_expr(g, cx, depth - 2) } else { term(g, cx, 1) };
            (Expr::Case(None, vec![(Expr::Call(Func::JsonValid, vec![x.clone()]), x)], None), Vec::new())
        },
        15.. => (draw_json_maker(g, cx, depth), Vec::new()),
        _ => json_literal(g, 3),
    }
}

/// An argument that may be anything ([`JsonArg::Any`]): a term of the
/// query (its values as SQLite reads them as JSON), a JSON text cut short
/// or with a byte replaced, text that is JSON5 or not JSON, a JSONB blob
/// literal (damaged now and then, [`jsonb_literal`]), or JSON (text or,
/// from a `jsonb` function, a JSONB blob).
fn draw_any(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    match g.int(0..20) {
        0..=5 => term(g, cx, 1),
        6..=8 => {
            let mut b = match json_literal(g, 3).0 {
                Expr::Lit(Value::Text(s)) => s.into_bytes(),
                _ => unreachable!("a JSON literal is text"),
            };
            if g.bool() {
                b.truncate(g.index(b.len()));
            } else {
                let k = g.index(b.len());
                b[k] = *g.pick(b"{}[]\",:\\ x0-.e");
            }
            Expr::Lit(Value::Text(String::from_utf8_lossy(&b).into_owned()))
        },
        9 => Expr::Lit(Value::Text((*g.pick(&NOT_JSON)).to_string())),
        10..=12 => Expr::Lit(jsonb_literal(g, true).0),
        13 => Expr::Lit(Value::Text((*g.pick(&JSON5_DOCS)).to_string())),
        14..=16 if depth >= 2 => {
            let f = *g.pick(&[Func::Jsonb, Func::JsonbArray, Func::JsonbObject, Func::JsonbSet, Func::JsonbPatch]);
            draw_json_call(g, cx, f, depth)
        },
        _ => draw_json_doc(g, cx, depth).0,
    }
}

/// A value argument ([`value_arg_ok`]) of at most `depth` levels: a
/// literal not a BLOB, a term of the query (a column, in an aggregate
/// query a GROUP BY term or an aggregate) or any expression, each guarded
/// by [`unblob`] (or at depth 2 cast) unless never a BLOB; or a JSON
/// function's value (JSON text is embedded as JSON, not as a string).
fn draw_json_value(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    let literal = |g: &mut Source| {
        let t = cx.some_table(g);
        match draw_value(g, t) {
            Value::Blob(_) => Expr::Lit(Value::Text(short_text(g))),
            v => Expr::Lit(v),
        }
    };
    let e = match g.int(0..20) {
        7..=12 if depth >= 2 => {
            let x = term(g, cx, 1);
            if never_blob(&x) {
                x
            } else if depth >= 3 {
                unblob(x)
            } else {
                Expr::Cast(bx(x), *g.pick(&[CastType::Text, CastType::Numeric, CastType::Integer, CastType::Real]))
            }
        },
        13..=15 if depth >= 2 => draw_json_maker(g, cx, depth),
        16.. if depth >= 4 => {
            let e = draw_expr(g, cx, depth - 2);
            if value_arg_ok(&e) {
                e
            } else {
                unblob(e)
            }
        },
        _ => literal(g),
    };
    // A cast of a JSONB value, even unblobbed, keeps its JSON subtype
    // ([`value_arg_ok`]): a literal instead.
    if value_arg_ok(&e) {
        e
    } else {
        literal(g)
    }
}

/// A call of JSON function `f` with arguments of the kinds it takes
/// ([`json_args_ok`]): one or two label and value pairs for
/// `json_object`, one or two path and value pairs for the edits; paths
/// mostly of elements of its JSON argument ([`path_arg`]).
fn draw_json_call(g: &mut Source, cx: &Cx, f: Func, depth: usize) -> Expr {
    let d = depth.saturating_sub(1).max(1);
    debug_assert!(depth >= 2, "a call is two levels");
    let (lo, hi) = f.arity();
    let n = match f {
        Func::JsonObject | Func::JsonbObject => 2 * g.int(0..=2),
        _ if f.makes_json() && lo == 3 => 1 + 2 * g.int(1..=2),
        Func::JsonExtract | Func::JsonbExtract => if g.bool_with(0.8) { 2 } else { g.int(3..=hi) },
        Func::JsonValid | Func::JsonType | Func::JsonArrayLength | Func::JsonPretty => if g.bool_with(0.6) { 1 } else { 2 },
        _ => g.int(lo..=hi),
    };
    let mut found = Vec::new();
    let mut args = Vec::with_capacity(n);
    for k in 0..n {
        args.push(match json_arg(f, k) {
            JsonArg::Json if matches!(f, Func::JsonPatch | Func::JsonbPatch) && g.bool_with(0.6) => {
                let mut s = String::new();
                draw_patch_text(g, 3, &mut s);
                Expr::Lit(Value::Text(s))
            },
            JsonArg::Json => {
                let (doc, paths) = draw_json_doc(g, cx, d);
                if found.is_empty() {
                    found = paths;
                }
                doc
            },
            JsonArg::Path { array } => path_arg(g, array, &found),
            JsonArg::Value => draw_json_value(g, cx, d),
            JsonArg::Label => Expr::Lit(Value::Text((*g.pick(&JSON_LABELS)).to_string())),
            JsonArg::Flags => Expr::Lit(Value::Int(g.int(1..=15))),
            JsonArg::Column => Expr::Lit(Value::Text((*g.pick(&JSON_EACH_COLUMNS)).to_string())),
            JsonArg::Any => {
                let x = draw_any(g, cx, d);
                // A blob is checked as JSONB only with flags 4 or 8.
                if f == Func::JsonValid && n == 1 && matches!(x, Expr::Lit(Value::Blob(_))) && g.bool_with(0.8) {
                    args.push(x);
                    args.push(Expr::Lit(Value::Int(*g.pick(&[4, 8, 12, 6, 9, 13]))));
                    continue;
                }
                x
            },
            JsonArg::Indent => Expr::Lit(Value::Text((*g.pick(&JSON_INDENTS)).to_string())),
        });
    }
    Expr::Call(f, args)
}

/// `j -> p` or `j ->> p` (`text`), `p` mostly the path of an element of
/// `j` when it is a literal ([`draw_arrow_path`]).
fn draw_arrow(g: &mut Source, cx: &Cx, text: bool, depth: usize) -> Expr {
    let (doc, found) = draw_json_doc(g, cx, depth - 1);
    let p = draw_arrow_path(g, &found);
    Expr::bin(if text { BinOp::Arrow2 } else { BinOp::Arrow }, doc, p)
}

/// A JSON function whose value is JSON ([`json_ok`]): a constructor, an
/// edit, `json()`, `json_quote`, `->`; a quarter of them in `jsonb` form.
/// A JSON text literal below two levels.
fn draw_json_maker(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if depth < 2 {
        return json_literal(g, 2).0;
    }
    let f = *g.weighted(&[
        (4, Func::JsonArray),
        (4, Func::JsonObject),
        (2, Func::Json),
        (2, Func::JsonQuote),
        (2, Func::JsonSet),
        (1, Func::JsonInsert),
        (1, Func::JsonReplace),
        (1, Func::JsonArrayInsert),
        (1, Func::JsonRemove),
        (1, Func::JsonPatch),
        (2, Func::JsonExtract),
    ]);
    match f {
        Func::JsonExtract if g.bool() => draw_arrow(g, cx, false, depth),
        Func::JsonExtract => {
            let (doc, found) = draw_json_doc(g, cx, depth - 1);
            let paths = g.int(2..=3);
            let mut args = vec![doc];
            args.extend((0..paths).map(|_| path_arg(g, false, &found)));
            Expr::Call(Func::JsonExtract, args)
        },
        f => {
            let f = if g.bool_with(0.25) { f.binary() } else { f };
            draw_json_call(g, cx, f, depth)
        },
    }
}

/// A right operand of `->` or `->>` ([`arrow_ok`]): a path (mostly of
/// an element of the JSON argument, `found`), a label, an array index
/// (from the end when negative), `[n]`.
fn draw_arrow_path(g: &mut Source, found: &[String]) -> Expr {
    match g.int(0..20) {
        0..=9 => path_arg(g, false, found),
        10..=14 => Expr::Lit(Value::Text((*g.pick(&JSON_LABELS)).to_string())),
        15..=18 => Expr::Lit(Value::Int(g.int(-2..=3))),
        _ => Expr::Lit(Value::Text(g.pick(&["[0]", "[#-1]"]).to_string())),
    }
}

/// A JSON expression: an element of a JSON argument (`json_extract`,
/// `->`, `->>`, mostly), its type or array length, the validity of any
/// value (a column's, mostly), a count over `json_each` or `json_tree`, or
/// a JSON function's value; a quarter of the functions that have one in
/// `jsonb` form, whose value is then made JSON text ([`jsonb_read`]).
fn draw_json(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if depth < 2 {
        return leaf(g, cx, depth);
    }
    // A `jsonb` function's value goes to a JSON function ([`jsonb_read`]):
    // here, where any expression may take it, its JSON text (`json`, or
    // `json_quote` for `jsonb_extract`, whose value may be an SQL value).
    match draw_json_any(g, cx, depth) {
        Expr::Call(Func::JsonbExtract, args) => Expr::Call(Func::JsonQuote, vec![Expr::Call(Func::JsonbExtract, args)]),
        e @ Expr::Call(f, _) if f.jsonb() => Expr::Call(Func::Json, vec![e]),
        e => e,
    }
}

/// [`draw_json`] before its `jsonb` values are made text.
fn draw_json_any(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    match g.int(0..24) {
        0..=3 => {
            let f = if g.bool_with(0.25) { Func::JsonbExtract } else { Func::JsonExtract };
            draw_json_call(g, cx, f, depth)
        },
        4..=8 => {
            let text = g.bool();
            draw_arrow(g, cx, text, depth)
        },
        9 => draw_json_call(g, cx, Func::JsonType, depth),
        10 => draw_json_call(g, cx, Func::JsonArrayLength, depth),
        11..=12 => draw_json_call(g, cx, Func::JsonValid, depth),
        13..=14 => draw_json_call(g, cx, Func::JsonErrorPosition, depth),
        15 => draw_json_call(g, cx, Func::JsonPretty, depth),
        16..=17 => {
            let f = if g.bool() { Func::JsonEach } else { Func::JsonTree };
            if cx.grouped.is_none() {
                return draw_json_call(g, cx, f, depth);
            }
            // In an aggregate query a term may be an aggregate, which
            // the subquery may not hold ([`json_args_ok`]): a literal.
            let (doc, found) = json_literal(g, 3);
            let mut args = vec![Expr::Lit(Value::Text((*g.pick(&JSON_EACH_COLUMNS)).to_string())), doc];
            if g.bool() {
                args.push(path_arg(g, false, &found));
            }
            Expr::Call(f, args)
        },
        _ => draw_json_maker(g, cx, depth),
    }
}

/// Units of the `+N unit` modifiers, plural and not.
const DATE_UNITS: [&str; 12] = ["days", "hours", "minutes", "seconds", "months", "years", "day", "hour", "minute", "second", "month", "year"];

/// `strftime` formats: every conversion SQLite has, alone and in the
/// usual layouts, `%%`, text around them, an unknown conversion and a
/// trailing `%` (both NULL); a drawn format joins up to four.
const DATE_FORMATS: [&str; 24] = [
    "%Y-%m-%d",
    "%H:%M:%S",
    "%Y-%m-%d %H:%M:%f",
    "%Y",
    "%m",
    "%d",
    "%s",
    "%J",
    "%j",
    "%w %u",
    "%W",
    "%U",
    "%V %G %g",
    "%e",
    "%k %l",
    "%I:%M %p",
    "%P",
    "%R",
    "%T",
    "%F",
    "%Y%m%d%%",
    "%q",
    "%",
    "day %j of %Y",
];

/// The modifiers of `KeyExpr::Julianday`, each of which SQLite reads on any
/// time value it reads (`auto` first, which reads a number as a julian day
/// or a unix epoch by its magnitude), so that every one runs.
pub const JULIANDAY_CHAIN: [&str; 10] =
    ["auto", "weekday 3", "start of year", "+14 months", "end of month", "-1.5 hours", "+00:30:15.5", "-0001-02-03 04:05", "floor", "subsec"];

/// The modifiers of `KeyExpr::Time`, likewise.
pub const TIME_CHAIN: [&str; 9] =
    ["+12 hours", "start of day", "+90 minutes", "+3600 seconds", "+0.5 days", "weekday 0", "start of month", "+1 years", "ceiling"];

/// Every conversion of `printf` (`%s %d %i %u %f %e %E %g %G %x %X %o %c
/// %q %Q %w %j %J %r %p %n %%`) with its flags, widths and precisions, for
/// a value, its date and time, julian day, unix epoch and date, then the
/// value for each of the others (`KeyExpr::Format`, [`PRINTF_ARGS`]): the
/// text ones with a precision, so that the output stays bounded.
pub const PRINTF_ALL: &str = "%-12s|%.10s|%+.3f|%05d|%,d|%x|%#X|%#o|%.3e|%10.4E|%g|%!.20G|%c|%.8q|%.8Q|%.6w|%u|% i|%.3s|%%|%.6j|%!.6J|%r|%p|%n|%#.6q|%!.4s|%.3c|%lld|%#.0e|%!.3g|%-+8.2e";

/// The values of [`PRINTF_ALL`]'s conversions after the first five.
pub const PRINTF_ARGS: usize = 25;

/// Every conversion of `strftime` in one format (but an unknown one),
/// drawn now and then: one call on a time value takes them all.
pub const ALL_CONVERSIONS: &str = "%Y-%m-%d %H:%M:%S.%f %j %J %s %w %u %W %U %V %G %g %e %k %l %I %p %P %R %T %F %%";

/// A choice stream of its own, seeded by a hash of what was drawn
/// (`key`, `salt`): content chosen from it moves no draw of the source.
struct Side(u64);

impl Side {
    fn of(key: &str, salt: u64) -> Side {
        Side(key.bytes().fold(0xcbf2_9ce4_8422_2325u64 ^ salt, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)))
    }

    /// A choice of `n`.
    fn take(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.take(xs.len())]
    }
}

/// Modifiers SQLite reads at any place, taken now and then (by [`Side`])
/// besides the drawn ones: fractional months and years, `±YYYY-MM-DD
/// HH:MM[:SS]` and five-digit years, `±HH:MM:SS.SSS` shifts, the edges of
/// the month and day ranges of a date shift, exponents and bare decimal
/// points, singular and upper-case units, negative weekdays, a shift past
/// a unit's limit (NULL), and the others of [`draw_modifier`].
pub const MODIFIERS_MORE: [&str; 32] = [
    "+1.5 months",
    "-2.75 years",
    "+0001-02-03 04:05:06",
    "-0000-11-30",
    "+00010-00-00",
    "-0002-06-15 23:59",
    "+12:00",
    "-00:00:00.001",
    "+23:59:59.999",
    "-36:00",
    "+1e1 days",
    "+.5 days",
    "-0.000001 days",
    "+1 MONTH",
    "+2 Years",
    "-86400 seconds",
    "+1.0005 seconds",
    "+59.9999 minutes",
    "weekday -3",
    "weekday 6.0",
    "end of day",
    "end of year",
    "start of year",
    "ceiling",
    "floor",
    "subsec",
    "+1000 years",
    "-1440 minutes",
    "+6000000 days",
    "-11 months",
    "+13 months",
    "-0.5 hours",
];

/// Time values taken now and then (by [`Side`]) in place of a drawn time
/// literal: ISO-week edges (week 53, week 1 of the next year), the hours
/// 0 and 12 (`%I`, `%l`, `%p`), more fractional digits than
/// milliseconds, `24:00`, timezones `z`, `Z` after a space, `-00:00` and
/// without seconds, the first and last instants SQLite reads, and numbers
/// as text.
pub const TIMES_MORE: [&str; 20] = [
    "2024-12-30 00:00:00",
    "2021-01-03",
    "2020-12-31 12:00",
    "2026-01-01 00:00:00.000",
    "2024-03-10 00:30:00.123456",
    "2024-01-15 23:59:59.9999",
    "2024-01-15T12:00:00+05:30",
    "2024-01-15 12:00:00 Z",
    "2024-01-15 12:00:00z",
    "12:00:00-00:00",
    "24:00",
    "2024-01-15 24:00:00",
    "2024-02-29T00:00",
    "-4713-11-24 12:00:00",
    "9999-12-31 23:59:59.999",
    "2460000.25",
    "2023-06-15 13:45:00-02:30",
    "00:00:00.5",
    "2019-12-29 11:59:59",
    "2000-01-01 12:00:00+00:00",
];

/// Shifts of a `timediff` argument from the other: months and years
/// either way, fractional days, month ends.
pub const TIMEDIFF_SHIFTS: [&str; 12] =
    ["+1 month", "-1 month", "+13 months", "-25 months", "+1 year", "-3 years", "+0.5 days", "-1.25 days", "end of month", "start of month", "+29 days", "-0001-02-03 04:05:06"];

/// Conversions of `strftime` in groups, added now and then to a drawn
/// format: the 12-hour clock and its marks, the ISO week date, fractional
/// and julian seconds, padded and unpadded fields.
pub const STRFTIME_MORE: [&str; 10] =
    ["%I %l %p %P", "%G-W%V-%u", "%f %s", "%J", "%j %U %W", "%e %k", "%H%M%S", "%S.%M", "%d.%m.%Y %R", "%g %w %%"];

/// A modifier ([`modifier_ok`]): mostly `+N unit` (signed, fractional,
/// large), `start of` and `end of` a month, year or day, `weekday N`;
/// else `unixepoch`, `julianday` or `auto` (only as the `first` modifier,
/// which is the only place SQLite takes them), `subsec`, `ceiling` and
/// `floor`, `±HH:MM[:SS.SSS]` and `±YYYY-MM-DD[ HH:MM]` shifts, other
/// cases and spacing, and ones SQLite does not read (NULL, and no modifier
/// after it is read: only as the `last`).
fn draw_modifier(g: &mut Source, first: bool, last: bool) -> String {
    let shift = |g: &mut Source| {
        let n = *g.pick(&["1", "-1", "2", "7", "-30", "12", "0", "1.5", "-0.25", "366", "+3", "100000"]);
        let sign = if n.starts_with(['-', '+']) || g.bool_with(0.4) { "" } else { "+" };
        format!("{sign}{n} {}", g.pick(&DATE_UNITS))
    };
    match g.int(0..24) {
        8..=10 => format!("start of {}", g.pick(&["month", "year", "day"])),
        11 => format!("end of {}", g.pick(&["month", "year", "day"])),
        12..=13 => format!("weekday {}", g.int(-6..=6)),
        14..=15 if first => (*g.pick(&["unixepoch", "julianday", "auto"])).to_string(),
        16 => (*g.pick(&["subsec", "subsecond"])).to_string(),
        17 => (*g.pick(&["ceiling", "floor"])).to_string(),
        18 => format!("{}{:02}:{:02}{}", g.pick(&["+", "-", ""]), g.int(0..=25), g.pick(&[0, 30, 59]), g.pick(&["", ":15", ":00.500"])),
        19 => format!("{}{:04}-{:02}-{:02}{}", g.pick(&["+", "-"]), g.int(0..=2), g.int(0..=12), g.pick(&[0, 1, 15, 31]), g.pick(&["", " 01:30"])),
        20 => (*g.pick(&["START OF MONTH", "+1 DAYS", "Weekday 0", "SUBSEC", " +1 day", "+1 day ", "-1 Months", "End Of Year"])).to_string(),
        21 if last => (*g.pick(&["start of week", "weekday 7", "+1 fortnight", "", "+1", "1 day", "+1e2 days", "unixepoch"])).to_string(),
        _ => shift(g),
    }
}

/// Whether `v` reads as a time value of the data's: text that starts as a
/// date or a time, a unix epoch of these decades, a julian day of these
/// centuries.
fn timelike(v: &Value) -> bool {
    let digits = |b: &[u8]| b.iter().all(u8::is_ascii_digit);
    match v {
        Value::Text(s) => {
            let b = s.as_bytes();
            b.len() >= 5 && ((digits(&b[..4]) && b[4] == b'-') || (digits(&b[..2]) && b[2] == b':'))
        },
        Value::Int(i) => (0..4_000_000_000).contains(i),
        Value::Real(x) => (2_000_000.0..3_000_000.0).contains(x),
        _ => false,
    }
}

/// Whether the sampled values of column `c` are numbers (and some).
fn numbers(cx: &Cx, c: &Col) -> bool {
    let values = &cx.table_of(c).by_column[c.col];
    !values.is_empty() && values.iter().all(|v| matches!(v, Value::Int(_) | Value::Real(_) | Value::Null))
}

/// A term for a time value: in a query that does not aggregate, mostly a
/// column of its own whose values read as time values ([`timelike`]),
/// else a term ([`term`]).
fn time_term(g: &mut Source, cx: &Cx) -> Expr {
    if cx.grouped.is_none() && g.bool_with(0.75) {
        let mut dated = Vec::new();
        for (src, t) in cx.here().iter().enumerate() {
            for (col, values) in cx.schema.info(*t).unwrap().by_column.iter().enumerate() {
                if values.iter().any(timelike) {
                    dated.push(Expr::col(src, col));
                }
            }
        }
        if !dated.is_empty() {
            return g.pick(&dated).clone();
        }
    }
    term(g, cx, 1)
}

/// A time value of at most `depth` levels: mostly a term
/// ([`time_term`]), a time literal ([`draw_date`]) or another date and
/// time call; else any expression.
fn draw_time(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    match g.int(0..20) {
        0..=9 => time_term(g, cx),
        10..=13 => {
            let v = draw_date(g);
            let mut side = Side::of(&format!("{v:?}"), 0x7173);
            if side.take(3) == 0 {
                Expr::Lit(Value::Text((*side.pick(&TIMES_MORE)).to_string()))
            } else {
                Expr::Lit(v)
            }
        },
        14..=16 if depth >= 3 => draw_date_call(g, cx, depth - 1),
        17..=18 if depth >= 2 => draw_expr(g, cx, depth - 1),
        _ => term(g, cx, 1),
    }
}

/// A date and time call ([`date_args_ok`]): `date`, `datetime`, `time`,
/// `julianday` or `unixepoch` of a time value ([`draw_time`]) and up to
/// three modifiers ([`draw_modifier`]); `strftime` the same after a format
/// of [`DATE_FORMATS`] (now and then two); `timediff` of two time values.
fn draw_date_call(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if depth < 2 {
        return leaf(g, cx, depth);
    }
    let d = depth - 1;
    let f = *g.weighted(&[
        (4, Func::Date),
        (3, Func::Datetime),
        (2, Func::Time),
        (2, Func::Julianday),
        (2, Func::Unixepoch),
        (4, Func::Strftime),
        (2, Func::Timediff),
    ]);
    if f == Func::Timediff {
        let (a, b) = (draw_time(g, cx, d), draw_time(g, cx, d));
        // A third of the time (by [`Side`]: no draw) the second the first
        // shifted by modifiers of [`TIMEDIFF_SHIFTS`]: differences of
        // months and years either way, across month ends and leap days.
        let mut side = Side::of(&format!("{a},{b}"), 0x7464);
        if side.take(3) == 0 {
            let (m1, m2) = (side.pick(&TIMEDIFF_SHIFTS), side.pick(&TIMEDIFF_SHIFTS));
            let b = Expr::Call(Func::Datetime, vec![a.clone(), Expr::Lit(Value::Text((*m1).to_string())), Expr::Lit(Value::Text((*m2).to_string()))]);
            return if side.take(2) == 0 { Expr::Call(f, vec![a, b]) } else { Expr::Call(f, vec![b, a]) };
        }
        return Expr::Call(f, vec![a, b]);
    }
    let mut args = Vec::new();
    if f == Func::Strftime && g.bool_with(0.2) {
        args.push(Expr::Lit(Value::Text(ALL_CONVERSIONS.to_string())));
    } else if f == Func::Strftime {
        // An unknown conversion or a trailing `%` makes it NULL: rarely
        // joined with others.
        let mut parts: Vec<&str> = Vec::new();
        for _ in 0..*g.weighted(&[(4, 1usize), (3, 2), (2, 3), (1, 4)]) {
            let p = *g.pick(&DATE_FORMATS);
            if parts.is_empty() || !matches!(p, "%q" | "%") || g.bool_with(0.2) {
                parts.push(p);
            }
        }
        let mut format = parts.join(*g.pick(&[" ", "", "T", "/"]));
        // Half the time (by [`Side`]: no draw) more conversions after them,
        // of [`STRFTIME_MORE`].
        let mut side = Side::of(&format, 0x7366);
        if side.take(2) == 0 {
            format.push(' ');
            format.push_str(side.pick(&STRFTIME_MORE));
        }
        args.push(Expr::Lit(Value::Text(format)));
    }
    let time = draw_time(g, cx, d);
    // A number reads as a julian day unless the first modifier says unix
    // epoch (or `auto`, by its magnitude): half the time it does.
    let number = matches!(&time, Expr::Lit(Value::Int(_) | Value::Real(_))) || matches!(&time, Expr::Col(c) if numbers(cx, c));
    args.push(time);
    let n = *g.weighted(&[(3, 0usize), (4, 1), (5, 2), (4, 3), (3, 4), (2, 5)]);
    for k in 0..n {
        let m = if k == 0 && number && g.bool() {
            (*g.pick(&["unixepoch", "auto", "julianday", "unixepoch"])).to_string()
        } else {
            draw_modifier(g, k == 0, k + 1 == n)
        };
        args.push(Expr::Lit(Value::Text(m)));
    }
    // Three quarters of the time one to four modifiers more (by [`Side`]: no draw), before
    // the last drawn one (which may be one SQLite does not read) and after
    // the first (which may be one SQLite reads only there).
    let key: String = args.iter().map(|a| format!("{a},")).collect();
    let mut side = Side::of(&key, f as u64);
    let (_, hi) = f.arity();
    if side.take(4) != 0 {
        for _ in 0..=side.take(3) {
            if args.len() >= hi {
                break;
            }
            let at = if n == 0 { args.len() } else { (args.len() - 1).max(f.time_at() + 2) };
            args.insert(at, Expr::Lit(Value::Text((*side.pick(&MODIFIERS_MORE)).to_string())));
        }
    }
    Expr::Call(f, args)
}

/// Depth for a query drawn at `size`: 3, growing by one every 3 sizes to
/// [`MAX_DEPTH`] - 5, which leaves room for mutation to deepen it.
fn max_depth(size: usize) -> usize {
    (3 + size / 3).min(MAX_DEPTH - 5)
}

/// A subquery node of kind `kind` (EXISTS, IN, scalar) drawn in `cx`, or
/// `None` when no table fits the budget or no level is left.
fn draw_sub(g: &mut Source, cx: &Cx, kind: Kind, depth: usize) -> Option<Expr> {
    if !cx.can_nest() {
        return None;
    }
    // Correlated (it may refer to the queries around it) two thirds of the
    // time, never from an aggregate query's items or HAVING: there a
    // column of the query is a bare column ([`grouped_ok`]).
    let correlated = cx.grouped.is_none() && g.bool_with(0.65);
    let (min_level, budget) =
        if correlated { (cx.min_level, cx.budget) } else { (cx.levels.len(), cx.budget.max(ROW_BUDGET / 10)) };
    let q = draw_select(g, cx.schema, &cx.levels, min_level, budget, kind, depth.saturating_sub(1).max(2))?;
    let not = g.bool_with(0.3);
    Some(match kind {
        Kind::Exists => Expr::Exists(Box::new(q), not),
        Kind::In => {
            let rows = Cx { budget: 0, ..cx.clone() };
            let left = if g.bool_with(0.7) { term(g, cx, depth) } else { draw_expr(g, &rows, depth.saturating_sub(1)) };
            Expr::InSelect(bx(left), Box::new(q), not)
        },
        _ => Expr::Scalar(Box::new(q)),
    })
}

/// Any expression of at most `depth` levels (a leaf at 1).
pub fn draw_expr(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if depth <= 1 || g.bool_with(0.25) {
        return leaf(g, cx, depth);
    }
    let d = depth - 1;
    let sub = |g: &mut Source| draw_expr(g, cx, d);
    let w = |base: u32| if cx.can_nest() { base } else { 0 };
    let choice = *g.weighted(&[
        (12, 0u8),
        (8, 1),
        (4, 2),
        (4, 3),
        (4, 4),
        (4, 5),
        (4, 6),
        (10, 7),
        (4, 8),
        (5, 9),
        (5, 10),
        (5, 11),
        (2, 12),
        (9, 13),
        (w(2), 14),
        (w(2), 15),
        (w(3), 16),
        (20, 17),
        (6, 18),
        (1, 19),
        (5, 20),
    ]);
    match choice {
        0 => Expr::bin(*g.pick(&BinOp::CMP), sub(g), sub(g)),
        1 => Expr::bin(if g.bool() { BinOp::And } else { BinOp::Or }, sub(g), sub(g)),
        2 => Expr::Not(bx(sub(g))),
        3 => Expr::IsNull(bx(sub(g)), g.bool()),
        4 => Expr::bin(if g.bool() { BinOp::Is } else { BinOp::IsNot }, sub(g), sub(g)),
        5 => Expr::Between(bx(sub(g)), bx(sub(g)), bx(sub(g)), g.bool_with(0.3)),
        6 => {
            let l = (0..g.int(1..=4)).map(|_| sub(g)).collect();
            Expr::In(bx(sub(g)), l, g.bool_with(0.3))
        },
        7 => Expr::bin(*g.pick(&BinOp::ARITH), sub(g), sub(g)),
        8 => Expr::bin(BinOp::Concat, sub(g), sub(g)),
        9 => {
            let glob = g.bool_with(0.4);
            let op = match (glob, g.bool_with(0.25)) {
                (false, false) => BinOp::Like,
                (false, true) => BinOp::NotLike,
                (true, false) => BinOp::Glob,
                (true, true) => BinOp::NotGlob,
            };
            Expr::bin(op, sub(g), draw_pattern(g, cx, glob, d))
        },
        10 => {
            let op = g.bool().then(|| bx(sub(g)));
            let ws = (0..g.int(1..=3)).map(|_| (sub(g), sub(g))).collect();
            let el = g.bool_with(0.6).then(|| bx(sub(g)));
            Expr::Case(op, ws, el)
        },
        11 => Expr::Cast(bx(sub(g)), *g.pick(&CastType::ALL)),
        12 => Expr::Neg(bx(sub(g))),
        13 => {
            let f = *g.pick(&Func::ALL);
            let (lo, hi) = f.arity();
            Expr::Call(f, (0..g.int(lo..=hi)).map(|_| sub(g)).collect())
        },
        14 => draw_sub(g, cx, Kind::Exists, d).unwrap_or_else(|| leaf(g, cx, d)),
        15 => draw_sub(g, cx, Kind::In, d).unwrap_or_else(|| leaf(g, cx, d)),
        16 => draw_sub(g, cx, Kind::Scalar, d).unwrap_or_else(|| leaf(g, cx, d)),
        18 => draw_date_call(g, cx, depth),
        20 => draw_text_call(g, cx, depth),
        // Mostly a column, which an index under that sequence serves.
        19 => {
            let e = if cx.grouped.is_none() && g.bool_with(0.6) && !cx.here().is_empty() { Expr::Col(cx.rows_only().draw_col(g)) } else { sub(g) };
            Expr::Collate(bx(e), *g.weighted(&[(1, Coll::Binary), (2, Coll::Nocase), (2, Coll::Rtrim)]))
        },
        _ => draw_json(g, cx, depth),
    }
}

/// A literal for a comparison with `against`: a value its column holds
/// most of the time, so that the predicate selects some rows and the
/// oracles' partitions are not trivial.
fn lit_for(g: &mut Source, cx: &Cx, against: &Expr, depth: usize) -> Expr {
    let column = match against {
        Expr::Col(c) => Some(*c),
        Expr::Agg(Agg::Min | Agg::Max, Some(a)) => match &**a {
            Expr::Cast(x, _) => match &**x {
                Expr::Col(c) => Some(*c),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };
    match column {
        Some(c) if !cx.table_of(&c).by_column[c.col].is_empty() && g.bool_with(0.7) => {
            let info = cx.table_of(&c);
            let v = g.pick(&info.by_column[c.col]).clone();
            let aff = info.aff.get(c.col).copied().unwrap_or(Affinity::None);
            Expr::Lit(if aff != Affinity::None && matches!(against, Expr::Col(_)) && g.bool_with(0.3) { converted_lit(g, v, aff) } else { v })
        },
        // A date and time call: the same call of a time literal now and
        // then (a constant the planner can seek an index on it with), else
        // a time literal.
        _ if matches!(against, Expr::Call(f, _) if f.date()) && g.bool_with(0.8) => match against {
            Expr::Call(f, args) if *f != Func::Timediff && g.bool_with(0.3) => {
                let mut args = args.clone();
                args[f.time_at()] = Expr::Lit(draw_date(g));
                Expr::Call(*f, args)
            },
            _ => Expr::Lit(draw_date(g)),
        },
        _ if matches!(against, Expr::Agg(..)) && g.bool_with(0.6) => Expr::Lit(Value::Int(g.int(-1..=4))),
        _ if g.bool_with(0.75) => {
            let t = cx.some_table(g);
            Expr::Lit(draw_value(g, t))
        },
        _ => draw_expr(g, cx, depth),
    }
}

/// A value a column of affinity `aff` holds, as a literal of a class the
/// comparison converts back to it (datatype3.html section 4.2): under a
/// numeric affinity a number as text (`'5'`, `' 5 '`, `'5.0'` up to 2^53,
/// past which it reads as a rounded real, `'+5'` and `'-5'` with the sign
/// written, a real's shortest text that reads back as it, Rust's `{x:?}`:
/// `0.1`, `1e300`, `5e-324`, which `sqlite3AtoF` reads exactly), under TEXT
/// a text that reads as a number as that number; any other value as it is.
fn converted_lit(g: &mut Source, v: Value, aff: Affinity) -> Value {
    match (&v, aff) {
        (Value::Int(i), a) if a.numeric() => Value::Text(match g.int(0..4) {
            0 => i.to_string(),
            1 => format!(" {i} "),
            2 if i.unsigned_abs() <= 1 << 53 => format!("{i}.0"),
            _ => format!("{i:+}"),
        }),
        (Value::Real(x), a) if a.numeric() && x.is_finite() => Value::Text(format!("{x:?}")),
        (Value::Text(s), Affinity::Text) => match numeric_text(s) {
            Numeric::Int(i) => Value::Int(i),
            Numeric::Real(x) => Value::Real(x),
            _ => v,
        },
        _ => v,
    }
}

/// The first terms of the expression indexes of the sources of the query
/// of `cx` ([`TableInfo::exprs`]), each of its source; none in the items
/// or HAVING of an aggregate query, where a bare column may not be.
fn indexed_terms(cx: &Cx) -> Vec<Expr> {
    if cx.grouped.is_some() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (src, &t) in cx.here().iter().enumerate() {
        out.extend(cx.schema.info(t).unwrap().exprs.iter().map(|e| e.map_level(0, &|c| Some(Col { src, ..c }))));
    }
    out
}

/// A WHERE, ON or HAVING clause: mostly a comparison of a term (a column,
/// in an aggregate query a GROUP BY term or an aggregate, or the first
/// term of an expression index, [`indexed_terms`]), or a boolean
/// combination; subqueries (EXISTS, IN, comparison with a scalar
/// subquery) when the budget allows; a comparison of a JSON expression
/// ([`draw_json`]); any expression now and then.
pub fn draw_pred(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    if depth <= 1 {
        return leaf(g, cx, depth);
    }
    let d = depth - 1;
    let indexed = indexed_terms(cx);
    let col = |g: &mut Source| {
        if !indexed.is_empty() && g.bool_with(0.3) {
            g.pick(&indexed).clone()
        } else if g.bool_with(0.75) {
            term(g, cx, d)
        } else {
            draw_expr(g, cx, d)
        }
    };
    let w = |base: u32| if cx.can_nest() { base } else { 0 };
    let choice =
        *g.weighted(&[(30, 0u8), (15, 1), (5, 2), (6, 3), (4, 4), (8, 5), (8, 6), (6, 7), (18, 8), (w(5), 9), (w(5), 10), (w(4), 11), (12, 12), (6, 13), (10, 14)]);
    match choice {
        0 => {
            let c = col(g);
            let l = lit_for(g, cx, &c, d);
            Expr::bin(*g.pick(&BinOp::CMP), c, l)
        },
        1 => Expr::bin(if g.bool() { BinOp::And } else { BinOp::Or }, draw_pred(g, cx, d), draw_pred(g, cx, d)),
        2 => Expr::Not(bx(draw_pred(g, cx, d))),
        3 => Expr::IsNull(bx(col(g)), g.bool()),
        4 => {
            let c = col(g);
            let l = lit_for(g, cx, &c, d);
            Expr::bin(if g.bool() { BinOp::Is } else { BinOp::IsNot }, c, l)
        },
        5 => {
            let c = col(g);
            let (lo, hi) = (lit_for(g, cx, &c, d), lit_for(g, cx, &c, d));
            Expr::Between(bx(c), bx(lo), bx(hi), g.bool_with(0.25))
        },
        6 => {
            let c = col(g);
            let l = (0..g.int(1..=4)).map(|_| lit_for(g, cx, &c, d)).collect();
            Expr::In(bx(c), l, g.bool_with(0.25))
        },
        7 => {
            let glob = g.bool_with(0.4);
            let op = if glob { BinOp::Glob } else { BinOp::Like };
            Expr::bin(op, col(g), draw_pattern(g, cx, glob, d))
        },
        9 => draw_sub(g, cx, Kind::Exists, d).unwrap_or_else(|| leaf(g, cx, d)),
        10 => draw_sub(g, cx, Kind::In, d).unwrap_or_else(|| leaf(g, cx, d)),
        11 => match draw_sub(g, cx, Kind::Scalar, d) {
            Some(s) => {
                let c = col(g);
                Expr::bin(*g.pick(&BinOp::CMP), c, s)
            },
            None => leaf(g, cx, d),
        },
        12 => {
            let j = draw_json(g, cx, d);
            let l = lit_for(g, cx, &j, d);
            Expr::bin(*g.pick(&BinOp::CMP), j, l)
        },
        14 => draw_row_pred(g, cx, d),
        13 => {
            let t = draw_date_call(g, cx, d);
            let l = lit_for(g, cx, &t, d);
            if g.bool_with(0.25) {
                let hi = lit_for(g, cx, &t, d);
                Expr::Between(bx(t), bx(l), bx(hi), g.bool_with(0.2))
            } else {
                Expr::bin(*g.pick(&BinOp::CMP), t, l)
            }
        },
        _ => draw_expr(g, cx, depth),
    }
}

/// A row value predicate ([`Expr::RowCmp`], [`Expr::RowBetween`],
/// [`Expr::RowIn`], [`Expr::RowInSelect`]) of two terms (or three, a
/// quarter of the time): half the time, outside an aggregate query, a
/// source's primary key of several columns in order (an index the planner
/// can compare the row value with, `codeVectorCompare` on its columns) or
/// a column an index or key leads with first; else terms as comparisons
/// draw them. Compared with literals for those terms (now and then with
/// another source's or an outer query's columns), a list of two to four
/// rows of literals, or a subquery of as many columns.
fn draw_row_pred(g: &mut Source, cx: &Cx, depth: usize) -> Expr {
    let n = if g.bool_with(0.75) { 2 } else { 3 };
    let mut left: Vec<Expr> = Vec::new();
    if cx.grouped.is_none() && g.bool() {
        let here = cx.here();
        let k = g.int(0..=here.len() - 1);
        if let Some(info) = cx.schema.info(here[k]) {
            match &info.key {
                RowKey::Columns(pk) if pk.len() >= 2 => left.extend(pk.iter().take(n).map(|&c| Expr::col(k, c))),
                _ if !info.leading.is_empty() => left.push(Expr::col(k, *g.pick(&info.leading))),
                _ => {},
            }
        }
    }
    while left.len() < n {
        left.push(if g.bool_with(0.8) { term(g, cx, depth) } else { draw_expr(g, cx, depth.min(2)) });
    }
    let lits = |g: &mut Source, left: &[Expr]| left.iter().map(|c| lit_for(g, cx, c, depth)).collect::<Vec<_>>();
    match g.int(0..10) {
        0..=4 => {
            let op = *g.pick(&[BinOp::Eq, BinOp::Ne, BinOp::Lt, BinOp::Le, BinOp::Gt, BinOp::Ge, BinOp::Lt, BinOp::Ge, BinOp::Is, BinOp::IsNot]);
            let right = if g.bool_with(0.2) { (0..n).map(|_| term(g, cx, depth)).collect() } else { lits(g, &left) };
            Expr::RowCmp(op, left, right)
        },
        5 => {
            let (lo, hi) = (lits(g, &left), lits(g, &left));
            Expr::RowBetween(left, lo, hi, g.bool_with(0.25))
        },
        6 | 7 => {
            let plain = |g: &mut Source, e: Expr| if e.has_agg() { Expr::Lit(Value::Int(g.int(0..=3))) } else { e };
            let rows = (0..g.int(2..=4)).map(|_| lits(g, &left).into_iter().map(|e| plain(g, e)).collect()).collect();
            Expr::RowIn(left, rows, g.bool_with(0.25))
        },
        _ => {
            let correlated = cx.grouped.is_none() && g.bool_with(0.5);
            let (min_level, budget) =
                if correlated { (cx.min_level, cx.budget) } else { (cx.levels.len(), cx.budget.max(ROW_BUDGET / 10)) };
            match cx.can_nest().then(|| draw_select(g, cx.schema, &cx.levels, min_level, budget, Kind::Row(n), depth.saturating_sub(1).max(2))).flatten() {
                Some(q) => Expr::RowInSelect(left, Box::new(q), g.bool_with(0.3)),
                None => {
                    let right = lits(g, &left);
                    Expr::RowCmp(BinOp::Eq, left, right)
                },
            }
        },
    }
}

/// An ON clause for join `k` (source `k + 1`) in `cx`, whose last level
/// is the sources up to it: an equality of a column of the new source
/// with one of a source before it (or itself) most of the time, now and
/// then with another predicate; else any predicate. The two columns are
/// mostly a pair that shares a value ([`TableInfo::shared`]), so that the
/// join matches some rows, half the time (when there is one) a pair whose
/// new column an index or the key of its table leads with
/// ([`TableInfo::leading`]), so that the planner can search the new source
/// for each row of the others (and weigh a Bloom filter, statistics
/// permitting) or the other way round. Now and then two such links ANDed,
/// a key of two columns (from one source or two) for an index of two.
fn draw_on(g: &mut Source, cx: &Cx, k: usize, depth: usize) -> Expr {
    if g.bool_with(0.35) {
        return draw_pred(g, cx, depth);
    }
    let eq = draw_link(g, cx, k);
    if g.bool_with(0.18) {
        // Two links, as one row value half the time when both are `=` (or
        // both IS): `(a1.c0, a1.c1) = (a0.c2, a0.c0)`, which the planner
        // splits into its terms (`exprAnalyze`), a key of two columns.
        match (eq, draw_link(g, cx, k)) {
            (Expr::Bin(o, a, b), Expr::Bin(p, c, d)) if o == p && matches!(o, BinOp::Eq | BinOp::Is) && g.bool() => {
                Expr::RowCmp(o, vec![*a, *c], vec![*b, *d])
            },
            (x, y) => Expr::bin(BinOp::And, x, y),
        }
    } else if g.bool_with(0.3) {
        Expr::bin(if g.bool_with(0.8) { BinOp::And } else { BinOp::Or }, eq, draw_pred(g, cx, depth.saturating_sub(1).max(2)))
    } else {
        eq
    }
}

/// A comparison linking source `k + 1` of the last level of `cx` with a
/// source before it (or itself), as [`draw_on`] draws one: mostly an
/// equality of two columns that share a value, half the time (when there
/// is one) a lookup into the new source.
fn draw_link(g: &mut Source, cx: &Cx, k: usize) -> Expr {
    let here = cx.here();
    let info = |src: usize| cx.schema.info(here[src]).unwrap();
    // The other column is of a source before the new one: two columns of
    // the same row rarely agree, so a comparison within the new source is
    // left to the rare remainder.
    let mut matching = Vec::new();
    for &(ca, table, cb) in &info(k + 1).shared {
        for (other, _) in here[..=k].iter().enumerate().filter(|(_, t)| **t == table) {
            matching.push((ca, other, cb));
        }
    }
    let lookups: Vec<(usize, usize, usize)> = matching.iter().copied().filter(|m| info(k + 1).leading.contains(&m.0)).collect();
    let (a, b) = if !lookups.is_empty() && g.bool_with(0.5) {
        let (ca, other, cb) = *g.pick(&lookups);
        (Expr::col(k + 1, ca), Expr::col(other, cb))
    } else if !matching.is_empty() && g.bool_with(0.75) {
        let (ca, other, cb) = *g.pick(&matching);
        (Expr::col(k + 1, ca), Expr::col(other, cb))
    } else {
        let other = if g.bool_with(0.9) { g.int(0..=k) } else { k + 1 };
        (Expr::col(k + 1, g.index(info(k + 1).columns)), Expr::col(other, g.index(info(other).columns)))
    };
    let op = *g.weighted(&[(8, BinOp::Eq), (1, BinOp::Is), (1, BinOp::Lt), (1, BinOp::Ge), (1, BinOp::Ne)]);
    Expr::bin(op, a, b)
}

/// Join `k` of the query of `cx` (whose last level is the sources up to
/// it) of kind `kd`: 0 inner, 1 LEFT, 2 CROSS, 3 RIGHT, 4 FULL. The first
/// join of any kind but CROSS is 20% of the time USING one or two columns
/// both sources have (mostly ones that share values,
/// [`TableInfo::shared`]), or a third of those times NATURAL; else an ON
/// clause ([`draw_on`]).
fn draw_join(g: &mut Source, cx: &Cx, k: usize, kd: u8, depth: usize) -> JoinKind {
    let op = match kd {
        0 => JoinOp::Inner,
        1 => JoinOp::Left,
        3 => JoinOp::Right,
        4 => JoinOp::Full,
        _ => return JoinKind::Cross,
    };
    if k == 0 && g.bool_with(0.2) {
        if g.bool_with(0.33) {
            return JoinKind::Natural(op);
        }
        let here = cx.here();
        let (left, right) = (cx.schema.info(here[0]).unwrap(), cx.schema.info(here[1]).unwrap());
        let both = left.columns.min(right.columns);
        let shared: Vec<usize> = right.shared.iter().filter(|s| s.1 == here[0] && s.0 == s.2 && s.0 < both).map(|s| s.0).collect();
        let mut cols = Vec::new();
        for _ in 0..g.int(1..=2) {
            let c = if !shared.is_empty() && g.bool_with(0.7) { *g.pick(&shared) } else { g.index(both) };
            if !cols.contains(&c) {
                cols.push(c);
            }
        }
        return JoinKind::Using(op, cols);
    }
    let on = draw_on(g, cx, k, depth);
    match op {
        JoinOp::Inner => JoinKind::Inner(on),
        JoinOp::Left => JoinKind::Left(on),
        JoinOp::Right => JoinKind::Right(on),
        JoinOp::Full => JoinKind::Full(on),
    }
}

/// The most sources of a query (its FROM table and its joins).
pub const MAX_SOURCES: usize = 5;

/// At the top level, the chance of another join after `k + 1` joins: a
/// query of 4 or 5 sources gives the planner join orders to choose from
/// (`wherePathSolver`), and small tables keep their product in budget.
const JOIN_ON: [f64; MAX_SOURCES - 2] = [0.5, 0.42, 0.32];

/// A query of kind `kind` inside queries whose visible sources' tables
/// are `outer`, its columns from level `min_level` on, its sources'
/// rows (with what subqueries read per row) within `budget`. `None` when
/// no table fits.
///
/// Sources: one table; a join (inner, left or cross, self-joins
/// included) 42% of the time at the top and 15% in subqueries, at the top
/// more after it ([`JOIN_ON`]) up to [`MAX_SOURCES`], while the product of
/// row counts fits. Then by kind:
///
/// - top: `*` (or DISTINCT), expressions, one aggregate ([`Items::Agg`]),
///   or GROUP BY of 1 or 2 terms: a quarter of the time with expressions
///   of the terms as items, else with terms and aggregates as items and a
///   HAVING clause 85% of the time;
/// - EXISTS: `*`, a literal or expressions, sometimes grouped;
/// - IN: one expression, or one GROUP BY term;
/// - scalar: one aggregate expression, no GROUP BY.
pub fn draw_select(
    g: &mut Source,
    schema: &Schema,
    outer: &[Vec<usize>],
    min_level: usize,
    budget: u64,
    kind: Kind,
    depth: usize,
) -> Option<Query> {
    let rows = |t: usize| schema.info(t).unwrap().rows.max(1) as u64;
    let fits: Vec<usize> = schema.ids().into_iter().filter(|&t| rows(t) <= budget).collect();
    if fits.is_empty() {
        return None;
    }
    let table = *g.pick(&fits);
    let mut sources = vec![table];
    let mut product = rows(table);
    let top = kind == Kind::Top;
    let mut want = usize::from(g.bool_with(if top { 0.42 } else { 0.15 }));
    while top && want > 0 && want + 1 < MAX_SOURCES && g.bool_with(JOIN_ON[want - 1]) {
        want += 1;
    }
    let mut kinds = Vec::new();
    for _ in 0..want {
        let fit: Vec<usize> = schema.ids().into_iter().filter(|&t| product * rows(t) <= budget).collect();
        if fit.is_empty() {
            break;
        }
        let t = *g.pick(&fit);
        product *= rows(t);
        sources.push(t);
        kinds.push(*g.weighted(&[(40, 0u8), (28, 1), (15, 2), (8, 3), (9, 4)]));
    }
    let mut levels = outer.to_vec();
    levels.push(sources.clone());
    let cx = Cx { schema, levels, min_level, budget: budget / product, grouped: None };
    let mut joins = Vec::new();
    for (k, kd) in kinds.iter().enumerate() {
        let mut on_cx = cx.clone();
        *on_cx.levels.last_mut().unwrap() = sources[..k + 2].to_vec();
        joins.push(Join { table: sources[k + 1], kind: draw_join(g, &on_cx, k, *kd, depth) });
    }
    let mut q = Query::simple(table, false, Items::Star, None);
    q.joins = joins;
    let grouped = match kind {
        Kind::Top => g.bool_with(0.2),
        Kind::Exists | Kind::In => g.bool_with(0.15),
        Kind::Scalar | Kind::Row(_) => false,
    };
    if grouped {
        let n = if kind == Kind::In || g.bool_with(0.6) { 1 } else { 2 };
        // Of the query's own columns or, in a subquery, an enclosing query's
        // (SQLite 3.50.2 rejected the latter; trunk accepts it).
        let kcx = cx.clone();
        q.group = (0..n)
            .map(|_| {
                let k = match g.int(0..10) {
                    0..=6 => Expr::Col(kcx.draw_col(g)),
                    7 => draw_date_call(g, &kcx, 2),
                    _ => draw_expr(g, &kcx, 2),
                };
                if int_const(&k) {
                    Expr::col(0, 0)
                } else {
                    k
                }
            })
            .collect();
    }
    let gcx = Cx { grouped: Some(q.group.clone()), ..cx.clone() };
    let item = |g: &mut Source, d: usize| draw_expr(g, &cx, d);
    // A GROUP BY query without aggregates, and so without HAVING.
    let mut plain = false;
    match kind {
        // A quarter of GROUP BY queries have no aggregate (and no HAVING):
        // the GROUP BY terms and expressions of them, as `tlp-group`
        // partitions them.
        Kind::Top if grouped && g.bool_with(0.25) => {
            let items = (0..g.int(1..=2))
                .map(|_| {
                    let e = draw_expr(g, &gcx, 3);
                    if e.has_agg() {
                        g.pick(&q.group).clone()
                    } else {
                        e
                    }
                })
                .collect();
            q.items = Items::Exprs(items);
            q.distinct = g.bool_with(0.15);
            plain = true;
        },
        Kind::Top if grouped => {
            let mut items: Vec<Expr> = Vec::new();
            if g.bool_with(0.6) {
                items.push(g.pick(&q.group).clone());
            }
            for _ in 0..g.int(1..=2) {
                items.push(if g.bool_with(0.5) { draw_agg(g, &gcx, depth) } else { draw_expr(g, &gcx, depth - 1) });
            }
            q.items = Items::Exprs(items);
            q.distinct = g.bool_with(0.15);
        },
        Kind::Top => match g.int(0..16) {
            0..=5 => q.distinct = g.bool_with(0.3),
            6..=11 => {
                let mut items: Vec<Expr> = (0..g.int(1..=3)).map(|_| item(g, depth - 1)).collect();
                // Window functions in half of these (none with DISTINCT).
                if g.bool_with(0.5) {
                    for _ in 0..g.int(1..=3) {
                        let w = draw_window(g, &cx);
                        let k = g.int(0..=items.len());
                        items.insert(k, w);
                    }
                } else {
                    q.distinct = g.bool_with(0.3);
                }
                q.items = Items::Exprs(items);
            },
            _ => {
                let agg = *g.pick(&[Agg::Count, Agg::Count, Agg::Min, Agg::Max, Agg::Sum, Agg::Sum]);
                let arg = if agg == Agg::Count && g.bool() {
                    None
                } else {
                    let e = item(g, depth - 1);
                    // An integer-valued argument half the time, for an
                    // exact SUM.
                    Some(if agg == Agg::Sum && g.bool() { Expr::Cast(bx(e), CastType::Integer) } else { e })
                };
                q.items = Items::Agg(agg, arg);
            },
        },
        Kind::Exists if grouped => q.items = Items::Exprs(vec![if g.bool() { g.pick(&q.group).clone() } else { draw_agg(g, &gcx, 2) }]),
        Kind::Exists => match g.int(0..4) {
            0 | 1 => {},
            2 => q.items = Items::Exprs(vec![Expr::Lit(Value::Int(1))]),
            _ => q.items = Items::Exprs(vec![item(g, 2)]),
        },
        Kind::In if grouped => q.items = Items::Exprs(vec![q.group[0].clone()]),
        Kind::Row(n) => {
            q.items = Items::Exprs((0..n).map(|_| if g.bool_with(0.7) { Expr::Col(cx.draw_col(g)) } else { item(g, depth - 1) }).collect());
            q.distinct = g.bool_with(0.2);
        },
        Kind::In => {
            q.items = Items::Exprs(vec![if g.bool_with(0.6) { Expr::Col(cx.draw_col(g)) } else { item(g, depth - 1) }]);
            q.distinct = g.bool_with(0.3);
        },
        Kind::Scalar => {
            let acx = Cx { grouped: Some(Vec::new()), ..cx.clone() };
            // An aggregate with an ORDER BY of its own half the time when the
            // subquery is not correlated (SQLite runs it once; a subquery in
            // a WHERE clause runs under every oracle), else as `draw_agg`
            // draws one (a correlated one sorts once per outer row).
            let ordered = if cx.min_level + 1 == cx.levels.len() { 0.5 } else { 0.15 };
            let e = if g.bool_with(ordered) {
                Some(draw_ordered(g, &acx, 2)).filter(|e| casts_binary(e, acx.schema, &acx.levels)).unwrap_or(Expr::Agg(Agg::Count, None))
            } else if g.bool_with(0.6) {
                draw_agg(g, &acx, 2)
            } else {
                draw_expr(g, &acx, 3)
            };
            q.items = Items::Exprs(vec![if e.has_agg() { e } else { draw_agg(g, &acx, 2) }]);
        },
    }
    let filter = match kind {
        Kind::Top if grouped => 0.7,
        Kind::Top => 0.92,
        _ => 0.85,
    };
    q.filter = g.bool_with(filter).then(|| draw_pred(g, &cx, depth));
    // A partial index's predicate as a WHERE term 40% of the time, so that
    // the WHERE clause implies it and the planner may use the index
    // (`whereUsablePartialIndex`); `c IS NOT NULL` half the time as a
    // comparison of `c`, which implies it too (`exprImpliesNotNull`).
    for (k, &t) in sources.iter().enumerate() {
        let partial = &schema.info(t).unwrap().partial;
        if partial.is_empty() || !g.bool_with(0.4) {
            continue;
        }
        let p = g.pick(partial).map_level(0, &|c| Some(Col { src: k, ..c }));
        let p = match p {
            Expr::IsNull(c, true) if g.bool() => {
                let l = lit_for(g, &cx, &c, 1);
                Expr::bin(*g.pick(&BinOp::CMP), *c, l)
            },
            p => p,
        };
        let f = match q.filter.clone() {
            Some(f) if g.bool() => Expr::bin(BinOp::And, f, p),
            Some(f) => Expr::bin(BinOp::And, p, f),
            None => p,
        };
        if f.depth() <= MAX_DEPTH {
            q.filter = Some(f);
        }
    }
    // The join condition of a CROSS join (60%), an inner one (15%) or a
    // LEFT one (10%: on its right source, the join becomes an inner one)
    // as a WHERE term ([`draw_link`]), as implicit joins put it: a CROSS
    // JOIN keeps its sources' order, so its key may come from a source
    // further out than the one just before. Now and then two links ANDed,
    // as [`draw_on`] draws them.
    for (k, kd) in kinds.iter().enumerate() {
        let p = match kd {
            2 => 0.6,
            0 => 0.15,
            1 => 0.1,
            _ => 0.0,
        };
        if !g.bool_with(p) {
            continue;
        }
        let mut on_cx = cx.clone();
        *on_cx.levels.last_mut().unwrap() = sources[..k + 2].to_vec();
        let mut link = draw_link(g, &on_cx, k);
        if g.bool_with(0.12) {
            link = Expr::bin(BinOp::And, link, draw_link(g, &on_cx, k));
        }
        let f = match q.filter.clone() {
            Some(f) if g.bool() => Expr::bin(BinOp::And, f, link),
            Some(f) => Expr::bin(BinOp::And, link, f),
            None => link,
        };
        if f.depth() <= MAX_DEPTH {
            q.filter = Some(f);
        }
    }
    if grouped && !plain && g.bool_with(if top { 0.85 } else { 0.5 }) {
        q.having = Some(draw_pred(g, &gcx, depth.min(4)));
    }
    // `*` with a USING join left of a RIGHT or FULL join and a later source
    // of the same column ([`Query::using_ok`]): CROSS JOIN instead.
    if !q.using_ok(schema) {
        q.joins[0].kind = JoinKind::Cross;
    }
    if top && !q.has_window() && g.bool_with(0.15) {
        draw_compound(g, schema, &mut q, depth);
    }
    if top && (!matches!(q.items, Items::Agg(..)) || !q.compound.is_empty()) && g.bool_with(0.3) {
        draw_order(g, schema, &mut q);
    }
    Some(q)
}

/// One or two compound members for the top-level query `q` (an aggregate
/// `sum()` in it becoming `count(*)`, see [`Query::valid_in`]): half the
/// time `q`'s own SELECT with another filter or none (so that INTERSECT and
/// EXCEPT meet common rows), else a fresh query made `q`'s width
/// ([`fit_width`]). The operator UNION ALL 35% of the time, UNION 25%,
/// INTERSECT and EXCEPT 20% each.
fn draw_compound(g: &mut Source, schema: &Schema, q: &mut Query, depth: usize) {
    if matches!(q.items, Items::Agg(Agg::Sum, _)) {
        q.items = Items::Agg(Agg::Count, None);
    }
    let width = q.width(schema);
    for _ in 0..(if g.bool_with(0.7) { 1 } else { 2 }) {
        let m = if g.bool() {
            let mut m = q.core();
            m.filter = g.bool_with(0.8).then(|| draw_pred(g, &top_cx(schema, &m), depth));
            m
        } else {
            let Some(mut m) = draw_select(g, schema, &[], 0, ROW_BUDGET, Kind::Top, depth) else {
                break;
            };
            m = m.core();
            if let Items::Exprs(es) = &mut m.items {
                for e in es.iter_mut().filter(|e| matches!(e, Expr::Window(_))) {
                    *e = Expr::Lit(Value::Null);
                }
            }
            fit_width(g, schema, &mut m, width, depth);
            m
        };
        let op = *g.weighted(&[(35, SetOp::UnionAll), (25, SetOp::Union), (20, SetOp::Intersect), (20, SetOp::Except)]);
        q.compound.push((op, m));
    }
}

/// `q` (a top-level core) made `width` columns wide: its expression items
/// kept up to `width` and more drawn (row expressions, or in an aggregate
/// query of its GROUP BY terms and aggregates); one aggregate `sum()`
/// becomes `count(*)`.
fn fit_width(g: &mut Source, schema: &Schema, q: &mut Query, width: usize, depth: usize) {
    if matches!(q.items, Items::Agg(Agg::Sum, _)) {
        q.items = Items::Agg(Agg::Count, None);
    }
    if q.width(schema) == width {
        return;
    }
    let cx = top_cx(schema, q);
    let cx = if q.aggregate() { Cx { grouped: Some(q.group.clone()), ..cx } } else { cx };
    let mut items = match &q.items {
        Items::Exprs(es) => es.clone(),
        _ => Vec::new(),
    };
    items.truncate(width);
    while items.len() < width {
        items.push(draw_expr(g, &cx, depth.saturating_sub(1).max(1)));
    }
    q.items = Items::Exprs(items);
}

/// The keys of every source in `cx`'s query (its last level), as ORDER BY
/// terms ([`keyed`]); `None` when one has no key (a CTE).
fn source_keys(cx: &Cx) -> Option<Vec<Expr>> {
    let mut keys = Vec::new();
    for (k, t) in cx.here().iter().enumerate() {
        match &cx.schema.info(*t)?.key {
            RowKey::Rowid => keys.push(Expr::col(k, ROWID)),
            RowKey::Columns(pk) => keys.extend(pk.iter().map(|c| Expr::col(k, *c))),
            RowKey::None => return None,
        }
    }
    Some(keys)
}

/// A window function item for the query of `cx` ([`Window`]): the
/// functions of [`WinFunc::needs_total`] only when every source has a key;
/// up to two PARTITION BY and up to two ORDER BY terms (mostly columns),
/// then for those functions and a ROWS frame, and half the other times,
/// every source's key; or, a quarter of the time for a function that reads
/// a frame, one integer ORDER BY term ([`int_key`]: the rowid of a single
/// source, or a `CAST(.. AS INTEGER)` where the order need not be total)
/// for a RANGE frame with offsets. A frame for those functions 60% of the
/// time (ROWS only with keys), FILTER for an aggregate a fifth of the
/// time, a named window a quarter of the time. One that would read more
/// rows in its frames than their budget ([`Window::frames_fit`]) is
/// [`narrowed`].
pub fn draw_window(g: &mut Source, cx: &Cx) -> Expr {
    let rows = cx.rows_only();
    let keys = source_keys(cx);
    let term = |g: &mut Source| if g.bool_with(0.7) { Expr::Col(rows.draw_col(g)) } else { draw_expr(g, &rows, 2) };
    let funcs: Vec<(u32, WinFunc)> = WinFunc::ALL
        .iter()
        .map(|&f| {
            let w = match f {
                _ if f.needs_total() && keys.is_none() => 0,
                WinFunc::Count | WinFunc::Sum | WinFunc::Min | WinFunc::Max => 3,
                WinFunc::RowNumber | WinFunc::Rank | WinFunc::DenseRank => 2,
                _ => 2,
            };
            (w, f)
        })
        .collect();
    let func = match *g.weighted(&funcs) {
        WinFunc::Ntile(_) => WinFunc::Ntile(g.int(1..=5)),
        WinFunc::NthValue(_) => WinFunc::NthValue(g.int(1..=3)),
        WinFunc::Lag(_) => WinFunc::Lag(if g.bool() { 1 } else { g.int(1..=3) }),
        WinFunc::Lead(_) => WinFunc::Lead(if g.bool() { 1 } else { g.int(1..=3) }),
        WinFunc::GroupConcat(_) => WinFunc::GroupConcat(g.int(0..=GROUP_SEPS.len() as u8 - 1)),
        f => f,
    };
    let order_term = |g: &mut Source, e: Expr| {
        let desc = g.bool_with(0.3);
        Order { key: OrderKey::Expr(e), desc, nulls: g.bool_with(0.2).then(|| g.bool()), by: By::Number }
    };
    let single_rowid = cx.here().len() == 1 && keys.as_deref() == Some(&[Expr::col(0, ROWID)][..]);
    let ranged = func.framed() && (single_rowid || !func.needs_total()) && g.bool_with(0.25);
    let partition = (0..g.int(0..=2)).map(|_| term(g)).collect();
    let mut order: Vec<Order>;
    let frame;
    if ranged {
        let key = if single_rowid && (func.needs_total() || g.bool()) { Expr::col(0, ROWID) } else { Expr::Cast(bx(term(g)), CastType::Integer) };
        order = vec![order_term(g, key)];
        frame = Some(draw_frame(g, false, true));
    } else {
        order = (0..g.int(0..=2))
            .map(|_| {
                let e = term(g);
                order_term(g, e)
            })
            .collect();
        frame = (func.framed() && g.bool_with(0.6)).then(|| draw_frame(g, keys.is_some(), false));
        let total = func.needs_total() || frame.is_some_and(|f| f.mode == FrameMode::Rows);
        if let Some(mut keys) = keys.clone() {
            if total || g.bool() {
                shuffle(g, &mut keys);
                for k in keys {
                    order.push(order_term(g, k));
                }
            }
        }
    }
    let arg = |g: &mut Source| if g.bool_with(0.6) { Expr::Col(rows.draw_col(g)) } else { draw_expr(g, &rows, 2) };
    let arg = match func {
        WinFunc::Count => g.bool().then(|| arg(g)),
        WinFunc::Sum | WinFunc::JsonGroupArray => Some(Expr::Cast(bx(arg(g)), CastType::Integer)),
        WinFunc::Min | WinFunc::Max => {
            let x = arg(g);
            // A cast to TEXT keeps a collated value's sequence ([`casts_binary`]).
            let text = g.bool_with(0.4) && casts_binary(&Expr::Cast(bx(x.clone()), CastType::Text), cx.schema, &cx.levels);
            Some(Expr::Cast(bx(x), if text { CastType::Text } else { CastType::Integer }))
        },
        WinFunc::Avg | WinFunc::Total => {
            let n = *g.pick(&[10, 100, 1000]);
            Some(Expr::bin(BinOp::Rem, Expr::Cast(bx(arg(g)), CastType::Integer), Expr::Lit(Value::Int(n))))
        },
        WinFunc::FirstValue | WinFunc::LastValue | WinFunc::NthValue(_) | WinFunc::Lag(_) | WinFunc::Lead(_) => Some(arg(g)),
        // A column, a blob as its hex: a blob read as text need not be valid
        // text (of odd length in a UTF-16 database), and groupConcatInverse
        // removes a value by the length of its text converted again, which
        // then need not be what it appended (plans-noopt-WindowFunc, qscore
        // seed 0 input 9193; garbage in, garbage out, invalidutf.html).
        WinFunc::GroupConcat(_) => Some(unblob(Expr::Col(rows.draw_col(g)))),
        _ => None,
    };
    let default = (matches!(func, WinFunc::Lag(_) | WinFunc::Lead(_)) && g.bool_with(0.4))
        .then(|| if g.bool() { Expr::Lit(g.pick(&[Value::Int(-1), Value::Int(0), Value::Null]).clone()) } else { Expr::Col(rows.draw_col(g)) });
    let filter = (func.aggregate() && g.bool_with(0.2)).then(|| draw_expr(g, &rows, 2));
    let named = if g.bool_with(0.25) { *g.pick(&[Named::All, Named::Partition, Named::PartitionOrder]) } else { Named::Inline };
    let w = Window { func, arg, default, filter, partition, order, frame, named };
    let product = cx.here().iter().map(|&t| cx.schema.info(t).map_or(1, |i| i.rows.max(1) as u64)).fold(1u64, u64::saturating_mul);
    let single = cx.here().len() == 1;
    Expr::Window(Box::new(if w.frames_fit(product, single) { w } else { narrowed(g, w, keys) }))
}

/// A window that reads too many rows in its frames over its query's rows
/// ([`Window::frames_fit`]) made one that reads few: given the sources'
/// keys, a ROWS frame of a few rows ([`draw_narrow_frame`]) with the keys
/// ending its ORDER BY; without them (the function then depends on peers
/// only), a ranking function that reads no frame.
fn narrowed(g: &mut Source, mut w: Window, keys: Option<Vec<Expr>>) -> Window {
    match keys {
        Some(mut keys) => {
            w.frame = Some(draw_narrow_frame(g));
            if !keys.iter().all(|k| w.keys().contains(&k)) {
                shuffle(g, &mut keys);
                for k in keys {
                    let desc = g.bool_with(0.3);
                    w.order.push(Order { key: OrderKey::Expr(k), desc, nulls: g.bool_with(0.2).then(|| g.bool()), by: By::Number });
                }
            }
            w
        },
        None => Window {
            func: *g.pick(&[WinFunc::Rank, WinFunc::DenseRank, WinFunc::PercentRank, WinFunc::CumeDist]),
            arg: None,
            default: None,
            filter: None,
            frame: None,
            ..w
        },
    }
}

/// A frame SQLite accepts ([`Frame::ok`]): ROWS (only when `rows`), RANGE
/// (with offsets only when `range`, then always RANGE) or GROUPS; bounds of
/// kinds in order, offsets 0 to 3 (for RANGE 0 to 100); EXCLUDE a quarter
/// of the time; the short form a fifth of the time when it applies.
fn draw_frame(g: &mut Source, rows: bool, range: bool) -> Frame {
    let modes: &[FrameMode] =
        if rows { &[FrameMode::Rows, FrameMode::Range, FrameMode::Groups] } else { &[FrameMode::Range, FrameMode::Groups] };
    let mode = if range { FrameMode::Range } else { *g.pick(modes) };
    let offsets = mode != FrameMode::Range || range;
    let n = |g: &mut Source| if range { *g.pick(&[0, 1, 2, 5, 20, 100]) } else { g.int(0..=3) };
    let bound = |g: &mut Source, rank: u8| match rank {
        0 => Bound::UnboundedPreceding,
        1 if offsets => Bound::Preceding(n(g)),
        3 if offsets => Bound::Following(n(g)),
        4 => Bound::UnboundedFollowing,
        _ => Bound::CurrentRow,
    };
    // With offsets for RANGE, at least one bound is an offset.
    let (a, b) = loop {
        let a = g.int(0..=3u8);
        let b = g.int(a.max(1)..=4u8);
        if !range || a == 1 || a == 3 || b == 1 || b == 3 {
            break (a, b);
        }
    };
    let (start, end) = (bound(g, a), bound(g, b));
    let exclude = g.bool_with(0.25).then(|| *g.pick(&[Exclude::NoOthers, Exclude::CurrentRow, Exclude::Group, Exclude::Ties]));
    let short = end == Bound::CurrentRow && g.bool_with(0.2);
    Frame { mode, start, end, exclude, short }
}

/// A frame of at most 7 rows ([`Window::frame_rows`]): ROWS from `n
/// PRECEDING` or the current row to the current row or `n FOLLOWING`,
/// offsets 0 to 3; EXCLUDE and the short form as [`draw_frame`] draws
/// them.
fn draw_narrow_frame(g: &mut Source) -> Frame {
    let start = if g.bool_with(0.7) { Bound::Preceding(g.int(0..=3)) } else { Bound::CurrentRow };
    let end = if start == Bound::CurrentRow || g.bool() { Bound::Following(g.int(0..=3)) } else { Bound::CurrentRow };
    let exclude = g.bool_with(0.25).then(|| *g.pick(&[Exclude::NoOthers, Exclude::CurrentRow, Exclude::Group, Exclude::Ties]));
    let short = end == Bound::CurrentRow && g.bool_with(0.2);
    Frame { mode: FrameMode::Rows, start, end, exclude, short }
}

/// Shuffle `v` in place.
fn shuffle<T>(g: &mut Source, v: &mut [T]) {
    for k in (1..v.len()).rev() {
        let j = g.int(0..=k);
        v.swap(k, j);
    }
}

/// A total ORDER BY ([`Query::total_order`]) for a top-level query, and a
/// LIMIT 60% of the time. In a plain query: up to two leading terms (a
/// result column or a row expression), then, 60% of the time when every
/// source has one, the unique key of every source (rowid or primary key),
/// else every result column;
/// in any other query every result column. The terms in random order,
/// each descending 30% of the time and with NULLS FIRST or LAST 20%.
pub fn draw_order(g: &mut Source, schema: &Schema, q: &mut Query) {
    let width = q.width(schema);
    let cx = Cx { schema, levels: vec![q.sources()], min_level: 0, budget: 0, grouped: None };
    let mut keys = Vec::new();
    if q.plain() {
        for _ in 0..g.int(0..=2) {
            keys.push(if g.bool_with(0.4) {
                OrderKey::Column(g.index(width))
            } else {
                let depth = g.int(1..=3);
                let e = draw_expr(g, &cx, depth);
                OrderKey::Expr(if int_const(&e) { Expr::Col(cx.draw_col(g)) } else { e })
            });
        }
    }
    let mut total = Vec::new();
    let keyed = q.sources().iter().all(|t| schema.info(*t).is_some_and(|i| i.key != RowKey::None));
    if q.plain() && keyed && g.bool_with(0.6) {
        for (k, t) in q.sources().iter().enumerate() {
            match &schema.info(*t).map_or(RowKey::None, |i| i.key.clone()) {
                RowKey::Rowid => total.push(OrderKey::Rowid(k)),
                RowKey::Columns(pk) => total.extend(pk.iter().map(|c| OrderKey::Expr(Expr::col(k, *c)))),
                RowKey::None => {},
            }
        }
    } else {
        total.extend((0..width).map(OrderKey::Column));
    }
    shuffle(g, &mut total);
    keys.extend(total);
    // A result column by number, alias or expression alike; one alias
    // names every item.
    let by = |g: &mut Source| match g.int(0..10) {
        0..=2 => By::Alias,
        3..=5 => By::Expr,
        _ => By::Number,
    };
    q.order = keys
        .into_iter()
        .map(|key| {
            let by = if matches!(key, OrderKey::Column(_)) && matches!(q.items, Items::Exprs(_)) { by(g) } else { By::Number };
            Order { key, desc: g.bool_with(0.3), nulls: g.bool_with(0.2).then(|| g.bool()), by }
        })
        .collect();
    q.limit = g.bool_with(0.6).then(|| Limit {
        count: if g.bool_with(0.1) { -1 } else { g.int(0..=8) },
        offset: if g.bool_with(0.5) { 0 } else { g.int(-1..=5) },
        comma: g.bool_with(0.2),
    });
}

/// A top-level query with a WITH clause of one CTE (two 30% of the time),
/// that reads one of them: each a virtual table ([`draw_vtab`]) 8% of the
/// time (every one when `vtab`), else recursive ([`CteBody::Count`]) 30%
/// of the time, else a query ([`draw_body`]); MATERIALIZED or NOT MATERIALIZED a
/// third of the time each, with a column list 30% of the time. The query
/// is drawn as any other, its sources from the tables and the CTEs; `None`
/// when four tries read no CTE.
fn draw_with(g: &mut Source, schema: &Schema, depth: usize, vtab: bool) -> Option<Query> {
    let mut with = Vec::new();
    for _ in 0..(if g.bool_with(if vtab { 0.5 } else { 0.7 }) { 1 } else { 2 }) {
        // A WITH clause drawn for a virtual table has virtual tables only.
        let body = if vtab || g.bool_with(0.08) {
            CteBody::Vtab(draw_vtab(g, schema))
        } else if g.bool_with(0.3) {
            CteBody::Count { start: g.int(-2..=2), count: g.int(1..=12), form: if g.bool() { 0 } else { g.int(1..COUNT_FORMS.len() as u8) } }
        } else {
            CteBody::Select(Box::new(draw_body(g, schema, depth)?))
        };
        let hint = match g.int(0..3) {
            0 => None,
            k => Some(k == 1),
        };
        // A column list only for a query: a recursive count always has one.
        let columns = matches!(body, CteBody::Select(_)) && g.bool_with(0.3);
        with.push(Cte { body, hint, columns });
    }
    let probe = Query { with: with.clone(), ..Query::simple(0, false, Items::Star, None) };
    let scoped = probe.scope(schema)?;
    for _ in 0..4 {
        let mut q = draw_select(g, &scoped, &[], 0, ROW_BUDGET, Kind::Top, depth)?;
        q.with = with.clone();
        if (0..with.len()).any(|i| q.reads(CTE + i)) && q.valid(schema) && q.concat_ctes_fit(schema, &scoped) {
            return Some(q);
        }
    }
    None
}

/// A table-valued function or eponymous virtual table ([`Vtab`]) with rows:
/// a pragma function of a table (of its indexes only when it has one, of
/// its foreign keys only when it has one), `pragma_table_list`,
/// `json_each` or `json_tree` of a JSON text (now and then at the path of
/// one of its elements), `dbstat`, `main.sqlite_schema`; its arguments as
/// written, with the schema, or as constraints on its hidden columns.
pub fn draw_vtab(g: &mut Source, schema: &Schema) -> Vtab {
    // The database's own virtual tables most of the time when it has any.
    if has_virtual() && g.bool_with(0.7) {
        if let Some(v) = VINFO.with(|i| crate::vtabs::draw_query(g, &i.borrow())) {
            return Vtab::Virtual(v);
        }
    }
    let arg = match g.int(0..4) {
        0 => VtabArg::Schema,
        1 => VtabArg::Hidden,
        _ => VtabArg::Call,
    };
    let table = g.index(schema.tables.len());
    let none = Catalog::default();
    let cat = schema.catalog.get(table).unwrap_or(&none);
    loop {
        match g.int(0..20) {
            0..=4 => return Vtab::TableInfo { table, xinfo: g.bool(), arg },
            5..=6 if !cat.indexes.is_empty() => return Vtab::IndexList { table, arg },
            7..=8 if !cat.indexes.is_empty() => return Vtab::IndexInfo { table, xinfo: g.bool() },
            9 if !cat.fks.is_empty() => return Vtab::ForeignKeys { table, arg },
            10 => return Vtab::TableList,
            11..=14 => {
                let (mut doc, mut paths) = (String::new(), Vec::new());
                let levels = g.int(1..=3);
                draw_json_text(g, levels, &mut doc, "$", &mut paths);
                if !value_ok(&Value::Text(doc.clone())) {
                    continue;
                }
                let path = (g.bool_with(0.4) && !paths.is_empty()).then(|| g.pick(&paths).clone());
                return Vtab::Json { tree: g.bool(), doc, path, arg };
            },
            15..=16 => return Vtab::Dbstat { aggregate: g.bool_with(0.3), arg },
            17 => return Vtab::SqliteSchema { form: g.index(SCHEMA_FORMS.len() - usize::from(!aux_attached())) },
            18..=19 => {
                let k = g.index(PRAGMAS.len());
                return Vtab::Pragma { k, table: (PRAGMAS[k].2 && g.bool()).then_some(table) };
            },
            _ => {},
        }
    }
}

/// A CTE's query ([`Cte::info`]): a query's FROM and WHERE, with 1 to 3
/// items (mostly columns, else row expressions, and a window 30% of the
/// time; aggregates without GROUP BY 20% of the time); 25% of the time an ORDER BY of every source's key
/// with a LIMIT, else 15% of the time a UNION ALL of the same with another
/// filter. `None` when no table fits the budget.
pub fn draw_body(g: &mut Source, schema: &Schema, depth: usize) -> Option<Query> {
    let mut q = draw_select(g, schema, &[], 0, ROW_BUDGET, Kind::Top, depth)?.core();
    q.distinct = false;
    q.group.clear();
    q.having = None;
    let cx = top_cx(schema, &q);
    let n = g.int(1..=3);
    q.items = Items::Exprs(if g.bool_with(0.2) {
        let acx = Cx { grouped: Some(Vec::new()), ..cx.clone() };
        (0..n).map(|_| draw_agg(g, &acx, 2)).collect()
    } else {
        let d = depth.saturating_sub(1).max(1);
        let mut items: Vec<Expr> = (0..n).map(|_| if g.bool_with(0.6) { Expr::Col(cx.draw_col(g)) } else { draw_expr(g, &cx, d) }).collect();
        // A window item 30% of the time: read through the CTE, inlined as
        // a subquery in FROM whose window the outer WHERE may not push
        // into (`pushDownWindowCheck`), or materialized.
        if g.bool_with(0.3) {
            let w = draw_window(g, &cx);
            let k = g.int(0..=items.len());
            items.insert(k, w);
        }
        items
    });
    let windowed = q.has_window();
    if q.plain() && g.bool_with(0.25) {
        draw_order(g, schema, &mut q);
        if q.total_order(schema) != Some(true) {
            q.order.clear();
        }
        q.limit = (!q.order.is_empty()).then(|| Limit { count: g.int(0..=6), offset: g.int(0..=3), comma: g.bool_with(0.2) });
    } else if !windowed && g.bool_with(0.15) {
        let other = Query { filter: g.bool_with(0.8).then(|| draw_pred(g, &cx, depth)), ..q.core() };
        q.compound.push((SetOp::UnionAll, other));
    }
    Some(q)
}

/// A top-level query on `schema` (which has a table): 15% of the time
/// with a WITH clause ([`draw_with`]).
pub fn draw_top(g: &mut Source, schema: &Schema) -> Query {
    let depth = max_depth(g.size());
    // A query of the database's virtual tables a third of the time when
    // it has any (joined with its tables).
    if g.bool_with(if has_virtual() { 0.9 } else { 0.15 }) {
        if let Some(q) = draw_with(g, schema, depth, true) {
            return q;
        }
    }
    if g.bool_with(0.15) {
        if let Some(q) = draw_with(g, schema, depth, false) {
            return q;
        }
    }
    draw_select(g, schema, &[], 0, ROW_BUDGET, Kind::Top, depth).unwrap_or_else(|| {
        // Every table holds more rows than the budget: a plain query.
        let t = g.index(schema.tables.len());
        let cx = Cx { schema, levels: vec![vec![t]], min_level: 0, budget: 0, grouped: None };
        Query::simple(t, false, Items::Star, g.bool_with(0.92).then(|| draw_pred(g, &cx, depth)))
    })
}

// ---------------------------------------------------------------- shrinking

type Visit<'a, T> = &'a mut dyn FnMut(T) -> ControlFlow<()>;

fn simpler_values(v: &Value) -> Vec<Value> {
    match v {
        Value::Null => vec![],
        Value::Int(0) => vec![Value::Null],
        Value::Int(i) => vec![Value::Null, Value::Int(0), Value::Int(i / 2)],
        Value::Real(x) => vec![Value::Null, Value::Int(0), Value::Int(x.clamp(-1e18, 1e18) as i64)],
        Value::Text(s) => {
            let mut half = s.len() / 2;
            while !s.is_char_boundary(half) {
                half -= 1;
            }
            vec![Value::Null, Value::Text(String::new()), Value::Text(s[..half].to_string())]
        },
        Value::Blob(b) => vec![Value::Null, Value::Blob(Vec::new()), Value::Blob(b[..b.len() / 2].to_vec())],
    }
}

/// Simpler expressions, each strictly lower in [`Expr::cost`]: NULL, the
/// column `c0` of the first source, a simpler literal; each child; a
/// clause or list element or optional argument dropped, a NOT flag
/// cleared, an aggregate made `count`; a child replaced by one of its own
/// simpler forms; a subquery replaced by a simpler one
/// ([`shrink_query_raw`]). Not all are valid where `e` is (a bare column
/// in an aggregate query, a subquery of the wrong shape):
/// [`shrink_query`] keeps the valid ones.
pub fn shrink_expr(e: &Expr, visit: Visit<Expr>) -> ControlFlow<()> {
    let cost = e.cost();
    if cost > 1 {
        visit(Expr::Lit(Value::Null))?;
    }
    if cost > 2 {
        visit(Expr::col(0, 0))?;
    }
    if let Expr::Lit(v) = e {
        for w in simpler_values(v) {
            if 1 + w.cost() < cost && w != Value::Null {
                visit(Expr::Lit(w))?;
            }
        }
        return ControlFlow::Continue(());
    }
    for c in e.children() {
        if c.cost() > 1 {
            visit(c.clone())?;
        }
    }
    match e {
        Expr::IsNull(a, true) => visit(Expr::IsNull(a.clone(), false))?,
        Expr::Between(a, b, c, true) => visit(Expr::Between(a.clone(), b.clone(), c.clone(), false))?,
        Expr::In(a, l, not) => {
            if *not {
                visit(Expr::In(a.clone(), l.clone(), false))?;
            }
            if l.len() > 1 {
                for k in 0..l.len() {
                    let mut l = l.clone();
                    l.remove(k);
                    visit(Expr::In(a.clone(), l, *not))?;
                }
            }
        },
        // A row value: a scalar comparison of its first terms, the
        // negation dropped, a row of the list dropped, a term dropped from
        // every row (three to two; a subquery's item with it).
        Expr::RowCmp(op, a, b) => {
            visit(Expr::bin(*op, a[0].clone(), b[0].clone()))?;
            if a.len() > 2 {
                for k in 0..a.len() {
                    let (mut a, mut b) = (a.clone(), b.clone());
                    a.remove(k);
                    b.remove(k);
                    visit(Expr::RowCmp(*op, a, b))?;
                }
            }
        },
        Expr::RowBetween(a, lo, hi, not) => {
            visit(Expr::Between(bx(a[0].clone()), bx(lo[0].clone()), bx(hi[0].clone()), *not))?;
            if *not {
                visit(Expr::RowBetween(a.clone(), lo.clone(), hi.clone(), false))?;
            }
        },
        Expr::RowIn(a, l, not) => {
            visit(Expr::In(bx(a[0].clone()), l.iter().map(|r| r[0].clone()).collect(), *not))?;
            if *not {
                visit(Expr::RowIn(a.clone(), l.clone(), false))?;
            }
            if l.len() > 2 {
                for k in 0..l.len() {
                    let mut l = l.clone();
                    l.remove(k);
                    visit(Expr::RowIn(a.clone(), l, *not))?;
                }
            }
            if a.len() > 2 {
                for k in 0..a.len() {
                    let mut a = a.clone();
                    a.remove(k);
                    let l = l.iter().map(|r| r.iter().enumerate().filter(|(i, _)| *i != k).map(|(_, e)| e.clone()).collect()).collect();
                    visit(Expr::RowIn(a, l, *not))?;
                }
            }
        },
        Expr::RowInSelect(a, q, not) => {
            if *not {
                visit(Expr::RowInSelect(a.clone(), q.clone(), false))?;
            }
            if let Items::Exprs(items) = &q.items {
                if a.len() > 2 && items.len() == a.len() && q.compound.is_empty() {
                    for k in 0..a.len() {
                        let mut a = a.clone();
                        a.remove(k);
                        let mut q = q.clone();
                        if let Items::Exprs(items) = &mut q.items {
                            items.remove(k);
                        }
                        visit(Expr::RowInSelect(a, q, *not))?;
                    }
                }
                if let (Some(x), [item, ..]) = (a.first(), items.as_slice()) {
                    let mut one = q.clone();
                    one.items = Items::Exprs(vec![item.clone()]);
                    visit(Expr::InSelect(bx(x.clone()), one, *not))?;
                }
            }
        },
        Expr::Case(op, ws, el) => {
            if op.is_some() {
                visit(Expr::Case(None, ws.clone(), el.clone()))?;
            }
            if el.is_some() {
                visit(Expr::Case(op.clone(), ws.clone(), None))?;
            }
            if ws.len() > 1 {
                for k in 0..ws.len() {
                    let mut ws = ws.clone();
                    ws.remove(k);
                    visit(Expr::Case(op.clone(), ws, el.clone()))?;
                }
            }
        },
        Expr::Call(f, args) if args.len() > f.arity().0 => {
            for k in 0..args.len() {
                let mut args = args.clone();
                args.remove(k);
                visit(Expr::Call(*f, args))?;
            }
        },
        Expr::Agg(a, arg) => {
            if arg.is_some() {
                visit(Expr::Agg(Agg::Count, None))?;
            }
            if *a != Agg::Count {
                visit(Expr::Agg(Agg::Count, arg.clone()))?;
            }
        },
        Expr::Ordered(o) => {
            visit(Expr::Agg(Agg::Count, None))?;
            if let Some(x) = o.args.first() {
                visit(Expr::Agg(Agg::Count, Some(bx(x.clone()))))?;
            }
            let with = |f: &dyn Fn(&mut Ordered)| {
                let mut v = (**o).clone();
                f(&mut v);
                Expr::Ordered(Box::new(v))
            };
            if o.distinct {
                visit(with(&|v| v.distinct = false))?;
            }
            for k in 0..o.order.len() {
                if o.order.len() > 1 {
                    visit(with(&|v| {
                        v.order.remove(k);
                    }))?;
                }
                if o.order[k].desc {
                    visit(with(&|v| v.order[k].desc = false))?;
                }
                if o.order[k].nulls.is_some() {
                    visit(with(&|v| v.order[k].nulls = None))?;
                }
            }
        },
        Expr::Exists(q, true) => visit(Expr::Exists(q.clone(), false))?,
        Expr::Window(w) => {
            let with = |f: &dyn Fn(&mut Window)| {
                let mut v = (**w).clone();
                f(&mut v);
                Expr::Window(Box::new(v))
            };
            if w.filter.is_some() {
                visit(with(&|v| v.filter = None))?;
            }
            if w.default.is_some() {
                visit(with(&|v| v.default = None))?;
            }
            if w.named != Named::Inline {
                visit(with(&|v| v.named = Named::Inline))?;
            }
            if let Some(fr) = w.frame {
                visit(with(&|v| v.frame = None))?;
                if fr.exclude.is_some() {
                    visit(with(&|v| v.frame = Some(Frame { exclude: None, ..fr })))?;
                }
                if fr.short {
                    visit(with(&|v| v.frame = Some(Frame { short: false, ..fr })))?;
                }
            }
            for k in 0..w.partition.len() {
                visit(with(&|v| {
                    v.partition.remove(k);
                }))?;
            }
            for k in 0..w.order.len() {
                visit(with(&|v| {
                    v.order.remove(k);
                }))?;
                if w.order[k].desc {
                    visit(with(&|v| v.order[k].desc = false))?;
                }
                if w.order[k].nulls.is_some() {
                    visit(with(&|v| v.order[k].nulls = None))?;
                }
            }
            if w.func != WinFunc::Count || w.arg.is_some() {
                visit(with(&|v| {
                    v.func = WinFunc::Count;
                    v.arg = None;
                    v.default = None;
                }))?;
            }
        },
        Expr::InSelect(a, q, true) => visit(Expr::InSelect(a.clone(), q.clone(), false))?,
        _ => {},
    }
    for k in 0..e.children().len() {
        let child = e.children()[k].clone();
        shrink_expr(&child, &mut |s| {
            let mut next = e.clone();
            *next.children_mut()[k] = s;
            visit(next)
        })?;
    }
    if let Some(q) = e.subquery() {
        shrink_query_raw(q, &mut |s| {
            let mut next = e.clone();
            *next.subquery_mut().unwrap() = s;
            visit(next)
        })?;
    }
    ControlFlow::Continue(())
}

/// Simpler queries, each strictly lower in [`Query::cost`], not all valid
/// ([`shrink_query`] keeps those that are): the filter, HAVING dropped; a
/// CTE no query reads dropped, its hint or column list dropped, its count
/// halved or start 0, its query shrunk; the
/// LIMIT dropped or its offset 0, count halved, comma form dropped; a
/// compound member dropped, or alone (a member's cost is below the
/// compound's), or its operator UNION ALL; the ORDER BY dropped, an ORDER
/// BY term dropped or its DESC or NULLS dropped; DISTINCT dropped; GROUP BY dropped (with `*` for the items, or the items kept),
/// a GROUP BY term dropped; a source dropped ([`Query::drop_source`]); a
/// LEFT join made inner, an inner join made cross; `*` for the items, an
/// item dropped, `count(*)` for an aggregate; then each expression shrunk
/// ([`shrink_expr`]), then each compound member shrunk.
/// Cheaper forms of a virtual table: its arguments as written, without
/// its flags, a JSON text without its path, or `[1]`.
fn shrink_vtab(v: &Vtab) -> Vec<Vtab> {
    let mut out = Vec::new();
    let mut plain = v.clone();
    match &mut plain {
        Vtab::TableInfo { arg, .. } | Vtab::IndexList { arg, .. } | Vtab::ForeignKeys { arg, .. } | Vtab::Json { arg, .. } | Vtab::Dbstat { arg, .. } => *arg = VtabArg::Call,
        _ => {},
    }
    if plain != *v {
        out.push(plain);
    }
    match v {
        Vtab::TableInfo { table, xinfo: true, arg } => out.push(Vtab::TableInfo { table: *table, xinfo: false, arg: *arg }),
        Vtab::IndexInfo { table, xinfo: true } => out.push(Vtab::IndexInfo { table: *table, xinfo: false }),
        Vtab::Dbstat { aggregate: true, arg } => out.push(Vtab::Dbstat { aggregate: false, arg: *arg }),
        Vtab::SqliteSchema { form } if *form > 0 => out.push(Vtab::SqliteSchema { form: 0 }),
        Vtab::Pragma { k, table } => {
            if table.is_some() {
                out.push(Vtab::Pragma { k: *k, table: None });
            }
            if *k > 0 && PRAGMAS[0].2 == table.is_some() {
                out.push(Vtab::Pragma { k: 0, table: *table });
            }
        },
        Vtab::Json { tree, doc, path, arg } => {
            if path.is_some() {
                out.push(Vtab::Json { tree: *tree, doc: doc.clone(), path: None, arg: *arg });
            }
            if *tree {
                out.push(Vtab::Json { tree: false, doc: doc.clone(), path: path.clone(), arg: *arg });
            }
            if doc.len() > 2 {
                out.push(Vtab::Json { tree: *tree, doc: "[1]".to_string(), path: None, arg: *arg });
            }
        },
        Vtab::Virtual(q) => out.extend(q.simpler().into_iter().map(Vtab::Virtual)),
        _ => {},
    }
    out.retain(|s| s.cost() < v.cost());
    out
}

pub fn shrink_query_raw(q: &Query, visit: Visit<Query>) -> ControlFlow<()> {
    let with = |f: &dyn Fn(&mut Query)| {
        let mut c = q.clone();
        f(&mut c);
        c
    };
    if q.filter.is_some() {
        visit(with(&|c| c.filter = None))?;
    }
    if q.having.is_some() {
        visit(with(&|c| c.having = None))?;
    }
    if let Some(l) = q.limit {
        visit(with(&|c| c.limit = None))?;
        if l.offset != 0 {
            visit(with(&|c| c.limit = Some(Limit { offset: 0, ..l })))?;
        }
        if l.count != 0 {
            visit(with(&|c| c.limit = Some(Limit { count: l.count / 2, ..l })))?;
        }
        if l.comma {
            visit(with(&|c| c.limit = Some(Limit { comma: false, ..l })))?;
        }
    }
    for (i, cte) in q.with.iter().enumerate() {
        if let Some(c) = q.drop_cte(i) {
            visit(c)?;
        }
        if cte.hint.is_some() {
            visit(with(&|c| c.with[i].hint = None))?;
        }
        if cte.columns {
            visit(with(&|c| c.with[i].columns = false))?;
        }
        match &cte.body {
            CteBody::Count { start, count, form } => {
                if *count > 1 {
                    visit(with(&|c| c.with[i].body = CteBody::Count { start: *start, count: count / 2, form: *form }))?;
                }
                if *start != 0 {
                    visit(with(&|c| c.with[i].body = CteBody::Count { start: 0, count: *count, form: *form }))?;
                }
                if *form != 0 {
                    visit(with(&|c| c.with[i].body = CteBody::Count { start: *start, count: *count, form: 0 }))?;
                }
            },
            CteBody::Select(b) => {
                shrink_query_raw(b, &mut |s| {
                    let mut c = q.clone();
                    c.with[i].body = CteBody::Select(Box::new(s));
                    visit(c)
                })?;
            },
            CteBody::Vtab(v) => {
                for s in shrink_vtab(v) {
                    visit(with(&|c| c.with[i].body = CteBody::Vtab(s.clone())))?;
                }
            },
        }
    }
    for (k, (op, m)) in q.compound.iter().enumerate() {
        visit(with(&|c| {
            c.compound.remove(k);
        }))?;
        visit(Query { with: q.with.clone(), ..m.clone() })?;
        if *op != SetOp::UnionAll {
            visit(with(&|c| c.compound[k].0 = SetOp::UnionAll))?;
        }
    }
    if !q.order.is_empty() {
        visit(with(&|c| {
            c.order.clear();
            c.limit = None;
        }))?;
        for k in 0..q.order.len() {
            if q.order.len() > 1 {
                visit(with(&|c| {
                    c.order.remove(k);
                }))?;
            }
            if q.order[k].desc {
                visit(with(&|c| c.order[k].desc = false))?;
            }
            if q.order[k].nulls.is_some() {
                visit(with(&|c| c.order[k].nulls = None))?;
            }
            if q.order[k].by != By::Number {
                // The number the term may already be written as: another
                // value of one query is no shrink.
                let c = with(&|c| c.order[k].by = By::Number);
                if c.sql() != q.sql() {
                    visit(c)?;
                }
            }
        }
    }
    if !q.group.is_empty() {
        if q.items != Items::Star {
            visit(with(&|c| {
                c.group.clear();
                c.having = None;
                c.items = Items::Star;
            }))?;
        }
        visit(with(&|c| {
            c.group.clear();
            c.having = None;
        }))?;
        if q.group.len() > 1 {
            for k in 0..q.group.len() {
                visit(with(&|c| {
                    c.group.remove(k);
                }))?;
            }
        }
    }
    for k in 0..=q.joins.len() {
        if let Some(c) = q.drop_source(k) {
            visit(c)?;
        }
    }
    for k in 0..q.joins.len() {
        match &q.joins[k].kind {
            JoinKind::Left(e) | JoinKind::Right(e) => visit(with(&|c| c.joins[k].kind = JoinKind::Inner(e.clone())))?,
            JoinKind::Full(e) => visit(with(&|c| c.joins[k].kind = JoinKind::Left(e.clone())))?,
            JoinKind::Inner(_) => visit(with(&|c| c.joins[k].kind = JoinKind::Cross))?,
            JoinKind::Cross => {},
            JoinKind::Using(op, cols) => {
                visit(with(&|c| c.joins[k].kind = JoinKind::Cross))?;
                if *op != JoinOp::Inner {
                    visit(with(&|c| c.joins[k].kind = JoinKind::Using(JoinOp::Inner, cols.clone())))?;
                }
                if cols.len() > 1 {
                    for d in 0..cols.len() {
                        let mut fewer = cols.clone();
                        fewer.remove(d);
                        visit(with(&|c| c.joins[k].kind = JoinKind::Using(*op, fewer.clone())))?;
                    }
                }
            },
            JoinKind::Natural(op) => {
                visit(with(&|c| c.joins[k].kind = JoinKind::Cross))?;
                if *op != JoinOp::Inner {
                    visit(with(&|c| c.joins[k].kind = JoinKind::Natural(JoinOp::Inner)))?;
                }
            },
        }
    }
    if q.distinct {
        visit(with(&|c| c.distinct = false))?;
    }
    match &q.items {
        Items::Star => {},
        Items::Exprs(es) => {
            visit(with(&|c| c.items = Items::Star))?;
            if es.len() > 1 {
                for k in 0..es.len() {
                    visit(with(&|c| {
                        if let Items::Exprs(es) = &mut c.items {
                            es.remove(k);
                        }
                    }))?;
                }
            }
        },
        Items::Agg(Agg::Count, None) => visit(with(&|c| c.items = Items::Star))?,
        Items::Agg(a, Some(e)) => {
            visit(with(&|c| c.items = Items::Agg(Agg::Count, None)))?;
            if *a != Agg::Count {
                visit(with(&|c| c.items = Items::Agg(Agg::Count, Some(e.clone()))))?;
            }
        },
        Items::Agg(_, None) => {},
    }
    let n = q.exprs().len();
    for k in 0..n {
        let e = q.exprs()[k].clone();
        shrink_expr(&e, &mut |s| {
            let mut c = q.clone();
            *c.exprs_mut()[k] = s;
            visit(c)
        })?;
    }
    for k in 0..q.compound.len() {
        shrink_query_raw(&q.compound[k].1, &mut |s| {
            let mut c = q.clone();
            c.compound[k].1 = s;
            visit(c)
        })?;
    }
    ControlFlow::Continue(())
}

/// The valid candidates of [`shrink_query_raw`] against `schema`: each
/// lower in [`Query::cost`] and valid.
pub fn shrink_query(q: &Query, schema: &Schema, visit: Visit<Query>) -> ControlFlow<()> {
    shrink_query_raw(q, &mut |c| if c.valid(schema) { visit(c) } else { ControlFlow::Continue(()) })
}

/// The total cost of a list of queries: dropping one or simplifying one
/// lowers it.
pub fn queries_cost(qs: &[Query]) -> u64 {
    qs.iter().map(Query::cost).sum()
}

/// Simpler lists, each lower in [`queries_cost`]: a query dropped, then a
/// query simplified ([`shrink_query`]). Valid against `schema` where `qs`
/// is.
pub fn shrink_queries(qs: &[Query], schema: &Schema, visit: Visit<Vec<Query>>) -> ControlFlow<()> {
    for k in 0..qs.len() {
        let mut c = qs.to_vec();
        c.remove(k);
        visit(c)?;
    }
    for k in 0..qs.len() {
        shrink_query(&qs[k], schema, &mut |q| {
            let mut c = qs.to_vec();
            c[k] = q;
            visit(c)
        })?;
    }
    ControlFlow::Continue(())
}

// ----------------------------------------------------------------- mutation

/// Replace the `n`th node, in preorder, by `new`.
fn replace_nth(e: &mut Expr, n: &mut usize, new: &mut Option<Expr>) {
    if new.is_none() {
        return;
    }
    if *n == 0 {
        *e = new.take().unwrap();
        return;
    }
    *n -= 1;
    for c in e.children_mut() {
        replace_nth(c, n, new);
        if new.is_none() {
            return;
        }
    }
}

/// The drawing context of a top-level query's WHERE clause.
fn top_cx<'a>(schema: &'a Schema, q: &Query) -> Cx<'a> {
    let product: u64 = q.sources().iter().map(|t| schema.info(*t).map_or(1, |i| i.rows.max(1) as u64)).product();
    Cx { schema, levels: vec![q.sources()], min_level: 0, budget: ROW_BUDGET / product.max(1), grouped: None }
}

/// A local change to `q`: a new or extended filter, DISTINCT toggled, a
/// join added or the last one dropped (each of these two dropping the ORDER
/// BY, which may no longer be total), an ORDER BY drawn or dropped,
/// compound members drawn or dropped (with the ORDER BY), or a random
/// subtree of one of its
/// expressions redrawn (as a WHERE clause's, or in the items and HAVING
/// of an aggregate query as theirs). Valid against `schema` and within
/// the row budget ([`Query::fits`]; falls back to a fresh query).
pub fn mutate_query(g: &mut Source, base: &Schema, q: &Query) -> Query {
    let Some(scoped) = q.scope(base) else {
        crate::fell_back(&crate::QUERY_FALLBACKS);
        return draw_top(g, base);
    };
    let schema = &scoped;
    let mut q = q.clone();
    let cx = top_cx(schema, &q);
    match g.int(0..8) {
        0 | 1 => {
            let p = draw_pred(g, &cx, 3);
            q.filter = Some(match q.filter.take() {
                Some(old) if g.bool() => Expr::bin(if g.bool() { BinOp::And } else { BinOp::Or }, old, p),
                Some(old) if g.bool() => Expr::Not(bx(old)),
                _ => p,
            });
        },
        2 if !matches!(q.items, Items::Agg(..)) => {
            q.distinct = !q.distinct;
            q.order.clear();
            q.limit = None;
        },
        3 if !q.joins.is_empty() && g.bool() => {
            q = q.drop_source(q.joins.len()).unwrap_or(q);
            q.order.clear();
            q.limit = None;
        },
        3 if q.joins.len() + 1 < MAX_SOURCES && cx.budget > 0 => {
            let fit: Vec<usize> = schema.ids().into_iter().filter(|&t| schema.info(t).unwrap().rows.max(1) as u64 <= cx.budget).collect();
            if fit.is_empty() {
                crate::fell_back(&crate::QUERY_FALLBACKS);
                return draw_top(g, base);
            }
            let t = *g.pick(&fit);
            let mut sources = q.sources();
            sources.push(t);
            let on_cx = Cx { levels: vec![sources], budget: 0, ..cx.clone() };
            let k = q.joins.len();
            let kd = g.int(0..5);
            let kind = draw_join(g, &on_cx, k, kd, 3);
            q.joins.push(Join { table: t, kind });
            if !q.using_ok(schema) {
                q.joins[k].kind = JoinKind::Cross;
            }
            q.order.clear();
            q.limit = None;
        },
        5 if g.bool_with(0.3) && !q.has_window() => {
            q.order.clear();
            q.limit = None;
            if q.compound.is_empty() {
                draw_compound(g, schema, &mut q, 3);
            } else {
                q.compound.clear();
            }
        },
        6 if !q.with.is_empty() => {
            let k = g.index(q.with.len());
            q.with[k].hint = match q.with[k].hint {
                None => Some(g.bool()),
                Some(_) => None,
            };
        },
        4 if g.bool_with(0.3) && (!matches!(q.items, Items::Agg(..)) || !q.compound.is_empty()) => {
            if q.order.is_empty() {
                draw_order(g, schema, &mut q);
            } else {
                q.order.clear();
                q.limit = None;
            }
        },
        _ => {
            let aggregate = q.aggregate();
            let gcx = Cx { grouped: Some(q.group.clone()), ..cx.clone() };
            let mut es: Vec<(bool, &mut Expr)> = Vec::new();
            let grouped_items = aggregate && !matches!(q.items, Items::Agg(..));
            let mut items = q.items.exprs_mut();
            es.extend(items.drain(..).map(|e| (grouped_items, e)));
            es.extend(q.filter.iter_mut().map(|e| (false, e)));
            es.extend(q.having.iter_mut().map(|e| (true, e)));
            if es.is_empty() {
                crate::fell_back(&crate::QUERY_FALLBACKS);
                return draw_top(g, base);
            }
            let k = g.index(es.len());
            let (grouped, e) = &mut es[k];
            let mut n = g.index(e.nodes());
            let depth = g.int(1..=3);
            let new = draw_expr(g, if *grouped { &gcx } else { &cx }, depth);
            replace_nth(e, &mut n, &mut Some(new));
        },
    }
    if q.valid(base) && q.fits(base) {
        q
    } else {
        crate::fell_back(&crate::QUERY_FALLBACKS);
        draw_top(g, base)
    }
}

/// Queries on the tables of a schema: 1 to 3 (more with size), none when
/// it has no table. Implements `Gen` directly: shrinking is
/// [`shrink_queries`], mutation redraws, adds, drops or changes a query.
pub struct Queries(pub Schema);

impl Gen for Queries {
    type Value = Vec<Query>;

    fn generate(&self, g: &mut Source) -> Vec<Query> {
        if self.0.tables.is_empty() {
            return Vec::new();
        }
        let n = g.int(1..=(1 + g.size() / 4).min(6));
        (0..n).map(|_| draw_top(g, &self.0)).collect()
    }

    fn shrink(&self, value: &Vec<Query>, visit: &mut dyn FnMut(Vec<Query>) -> ControlFlow<()>) -> ControlFlow<()> {
        shrink_queries(value, &self.0, visit)
    }

    fn mutate(&self, value: &Vec<Query>, g: &mut Source) -> Vec<Query> {
        let tables = self.0.tables.len();
        if tables == 0 || value.is_empty() {
            crate::fell_back(&crate::QUERY_FALLBACKS);
            return self.generate(g);
        }
        let mut qs = value.clone();
        match g.int(0..6) {
            0 => {
                let k = g.index(qs.len());
                qs[k] = draw_top(g, &self.0);
            },
            1 if qs.len() < 4 => qs.push(draw_top(g, &self.0)),
            2 if qs.len() > 1 => {
                qs.remove(g.index(qs.len()));
            },
            _ => {
                let k = g.index(qs.len());
                qs[k] = mutate_query(g, &self.0, &qs[k]);
            },
        }
        qs
    }
}

// ------------------------------------------------- the standalone generator

/// Schemas of 1 to 4 tables of 1 to 6 columns and 0 to 300 rows, each
/// with up to 8 sample values: what the query generator needs to know of a
/// database.
pub struct Schemas;

impl Gen for Schemas {
    type Value = Schema;

    fn generate(&self, g: &mut Source) -> Schema {
        let tables = (0..g.int(1..=4))
            .map(|_| {
                let columns = g.int(1..=6);
                let rows = g.int(0..=300);
                // A WITHOUT ROWID table a quarter of the time, keyed by its
                // first one or two columns.
                let key = if g.bool_with(0.25) { RowKey::Columns((0..g.int(1..=columns.min(2))).collect()) } else { RowKey::Rowid };
                let empty = TableInfo { columns, rows, samples: Vec::new(), by_column: vec![Vec::new(); columns], shared: Vec::new(), key, exprs: Vec::new(), partial: Vec::new(), leading: Vec::new(), coll: Vec::new(), aff: Vec::new() };
                let samples: Vec<Value> = (0..g.int(0..=8)).map(|_| draw_value(g, &empty)).collect();
                let by_column = (0..columns).map(|_| samples.iter().filter(|_| g.bool()).cloned().collect()).collect();
                TableInfo { samples, by_column, ..empty }
            })
            .collect();
        Schema { tables, ctes: Vec::new(), catalog: Default::default() }
    }
}

/// Every query is valid against the schema.
#[ephyra::generator]
pub fn scoped_valid(v: &(Schema, Vec<Query>)) -> bool {
    v.1.iter().all(|q| q.valid(&v.0))
}

/// Queries over a random schema: the expression language on its own.
#[ephyra::generator(scoped_valid, metrics = scoped_metrics)]
pub fn queries() -> impl Gen<Value = (Schema, Vec<Query>)> {
    ephyra::gens::dependent(Schemas, |s: &Schema| Queries(s.clone()))
}

fn class(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Int(_) => 1,
        Value::Real(_) => 2,
        Value::Text(_) => 3,
        Value::Blob(_) => 4,
    }
}

/// A node's kind, for counting the operators used: 0 and 1 are leaves.
fn kind(e: &Expr) -> usize {
    match e {
        Expr::Col(_) => 0,
        Expr::Lit(_) => 1,
        Expr::Not(_) => 2,
        Expr::Neg(_) => 3,
        Expr::IsNull(..) => 4,
        Expr::Between(..) => 5,
        Expr::In(..) => 6,
        Expr::RowCmp(..) => 37,
        Expr::RowBetween(..) => 38,
        Expr::RowIn(..) => 39,
        Expr::RowInSelect(..) => 40,
        Expr::Case(..) => 7,
        Expr::Cast(..) => 8,
        Expr::Call(..) => 9,
        Expr::Bin(op, ..) => 10 + *op as usize,
        Expr::Agg(..) => 32,
        Expr::Exists(..) => 33,
        Expr::InSelect(..) => 34,
        Expr::Scalar(_) => 35,
        Expr::Window(_) => 36,
        Expr::Ordered(_) => 37,
        Expr::Collate(..) => 38,
    }
}

/// Whether a subquery refers to a column of a query around it.
fn correlated(q: &Query) -> bool {
    (1..MAX_LEVELS).any(|n| q.refers(n))
}

/// Numbers about a list of queries, from the syntax alone (subqueries'
/// expressions included):
///
/// - queries: how many (0 without tables).
/// - max depth: the deepest expression (0 without any), 1 for a leaf.
/// - nodes: expression nodes over all queries.
/// - operators used: distinct operator kinds (binary operators each, NOT,
///   unary minus, IS NULL, BETWEEN, IN, CASE, CAST, call, aggregate,
///   EXISTS, IN subquery, scalar subquery); low means a narrow mix.
/// - storage classes mixed: distinct storage classes among the literals,
///   0 to 5 (NULL, integer, real, text, blob).
/// - function calls: calls over all queries; functions used: distinct
///   functions; json calls: JSON function calls and `->`, `->>`.
/// - aggregates, distinct queries, filtered queries: top-level queries of
///   each sort.
/// - joins: joins over the top-level queries; joins per query: that over
///   the queries (0 without any), 0 to 2; left joins, right joins, full
///   joins, cross joins: joins of each kind (with ON, USING or NATURAL);
///   using joins, natural joins: those with USING, with NATURAL; self
///   joins: queries joining a table with itself.
/// - grouped queries: top-level queries with GROUP BY; having: those with
///   HAVING too.
/// - subqueries: subquery nodes, nested ones included; correlated
///   subqueries: those referring to a query around them; subquery depth:
///   the deepest nesting (0 without any, at most 2).
/// - ordered queries: top-level queries with ORDER BY; limited queries:
///   those with LIMIT too; keyed orders: those ordering by a rowid or an
///   expression (in a plain query, a source key or a leading expression
///   term); order terms: ORDER BY terms over all queries.
/// - window queries: top-level queries with a window function; windows:
///   window function items over all queries; window functions used:
///   distinct functions among them, 0 to 7; frames: those with a frame.
/// - cte queries: top-level queries with a WITH clause; ctes: CTEs over
///   all queries; recursive ctes, hinted ctes: those recursive, those
///   MATERIALIZED or NOT MATERIALIZED; cte reads: sources (of queries,
///   their subqueries and compound members) that are CTEs, where reading
///   one twice shows.
/// - compound queries: top-level queries with set operations; set
///   operations: compound members over all queries; set operators used:
///   distinct operators among them, 0 to 4.
pub fn ast_metrics(qs: &[Query]) -> Vec<(&'static str, f64)> {
    let (mut depth, mut nodes, mut calls, mut json, mut subs, mut corr) = (0, 0, 0, 0, 0, 0);
    let mut kinds = [false; 41];
    let mut classes = [false; 5];
    let mut funcs = [false; Func::ALL.len() + Func::TEXT.len() + Func::DATE.len() + Func::PRINTF.len() + Func::JSON.len()];
    for q in qs {
        for e in q.exprs() {
            depth = depth.max(e.depth());
            e.walk(&mut |n| {
                nodes += 1;
                kinds[kind(n)] = true;
                match n {
                    Expr::Lit(v) => classes[class(v)] = true,
                    Expr::Call(f, _) => {
                        calls += 1;
                        json += usize::from(f.json());
                        funcs[*f as usize] = true;
                    },
                    Expr::Bin(BinOp::Arrow | BinOp::Arrow2, ..) => json += 1,
                    _ => {},
                }
                if let Some(s) = n.subquery() {
                    subs += 1;
                    corr += usize::from(correlated(s));
                }
            });
        }
    }
    let count = |b: &[bool]| b.iter().filter(|x| **x).count() as f64;
    let queries = |f: &dyn Fn(&Query) -> bool| qs.iter().filter(|q| f(q)).count() as f64;
    let joins: usize = qs.iter().map(|q| q.joins.len()).sum();
    let windows: Vec<&Window> =
        qs.iter().flat_map(|q| q.items.exprs()).filter_map(|e| if let Expr::Window(w) = e { Some(&**w) } else { None }).collect();
    let join_kind = |f: &dyn Fn(&JoinKind) -> bool| qs.iter().flat_map(|q| &q.joins).filter(|j| f(&j.kind)).count() as f64;
    vec![
        ("queries", qs.len() as f64),
        ("max depth", depth as f64),
        ("nodes", nodes as f64),
        ("operators used", count(&kinds[2..])),
        ("storage classes mixed", count(&classes)),
        ("function calls", calls as f64),
        ("functions used", count(&funcs)),
        ("json calls", json as f64),
        ("aggregates", queries(&|q| matches!(q.items, Items::Agg(..)))),
        ("distinct queries", queries(&|q| q.distinct)),
        ("filtered queries", queries(&|q| q.filter.is_some())),
        ("joins", joins as f64),
        ("joins per query", if qs.is_empty() { 0.0 } else { joins as f64 / qs.len() as f64 }),
        ("left joins", join_kind(&|k| k.op() == Some(JoinOp::Left))),
        ("right joins", join_kind(&|k| k.op() == Some(JoinOp::Right))),
        ("full joins", join_kind(&|k| k.op() == Some(JoinOp::Full))),
        ("using joins", join_kind(&|k| matches!(k, JoinKind::Using(..)))),
        ("natural joins", join_kind(&|k| matches!(k, JoinKind::Natural(_)))),
        ("cross joins", join_kind(&|k| matches!(k, JoinKind::Cross))),
        ("self joins", queries(&|q| {
            let mut t = q.sources();
            t.sort_unstable();
            t.windows(2).any(|w| w[0] == w[1])
        })),
        ("grouped queries", queries(&|q| !q.group.is_empty())),
        ("having", queries(&|q| q.having.is_some())),
        ("subqueries", subs as f64),
        ("correlated subqueries", corr as f64),
        ("subquery depth", qs.iter().map(Query::nesting).max().unwrap_or(0) as f64),
        ("ordered queries", queries(&|q| !q.order.is_empty())),
        ("limited queries", queries(&|q| q.limit.is_some())),
        ("keyed orders", queries(&|q| q.order.iter().any(|o| matches!(o.key, OrderKey::Rowid(_) | OrderKey::Expr(_))))),
        ("order terms", qs.iter().map(|q| q.order.len()).sum::<usize>() as f64),
        ("window queries", queries(&|q| q.has_window())),
        ("windows", windows.len() as f64),
        ("window functions used", {
            let mut used = [false; WinFunc::ALL.len()];
            for w in &windows {
                if let Some(k) = WinFunc::ALL.iter().position(|f| f.name() == w.func.name()) {
                    used[k] = true;
                }
            }
            count(&used)
        }),
        ("frames", windows.iter().filter(|w| w.frame.is_some()).count() as f64),
        ("cte queries", queries(&|q| !q.with.is_empty())),
        ("ctes", qs.iter().map(|q| q.with.len()).sum::<usize>() as f64),
        ("recursive ctes", qs.iter().flat_map(|q| &q.with).filter(|c| matches!(c.body, CteBody::Count { .. })).count() as f64),
        ("hinted ctes", qs.iter().flat_map(|q| &q.with).filter(|c| c.hint.is_some()).count() as f64),
        ("cte reads", qs.iter().map(|q| (0..q.with.len()).map(|i| q.count_reads(CTE + i)).sum::<usize>()).sum::<usize>() as f64),
        ("compound queries", queries(&|q| !q.compound.is_empty())),
        ("set operations", qs.iter().map(|q| q.compound.len()).sum::<usize>() as f64),
        ("set operators used", {
            let mut used = [false; 4];
            for q in qs {
                for (op, _) in &q.compound {
                    used[*op as usize] = true;
                }
            }
            count(&used)
        }),
    ]
}

/// [`ast_metrics`], and the schema's table count.
pub fn scoped_metrics(v: &(Schema, Vec<Query>), _size: usize) -> Vec<(&'static str, f64)> {
    let mut m = ast_metrics(&v.1);
    m.push(("tables", v.0.tables.len() as f64));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every spelling [`spell`] picks reads back as the plain one: `ISNULL`,
    /// `NOT NULL`, `IS [NOT] DISTINCT FROM`, `==`, `!=`, quoted names,
    /// hexadecimal and separated integers, double-quoted texts, `SELECT
    /// ALL`, `main.` tables and parenthesized FROM groups.
    #[test]
    fn drawn_formats_print_every_value() {
        // Every drawn format is of bounded output and SQLite prints values
        // of every class under it, the same twice.
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let values = ["NULL", "-7", "3.25", "-1e300", "'a''b\u{7}\"é'", "x'00ff'", "9223372036854775807", "''", "'12abc'"];
        let mut convs = std::collections::BTreeSet::new();
        for k in 0..3000u64 {
            let mut side = Side(k);
            let n = 1 + (k % 4) as usize;
            let f = draw_format(&mut side, n);
            assert!(format_ok(&f), "{f}");
            convs.extend(f.chars().filter(char::is_ascii_alphabetic));
            let args: Vec<&str> = (0..n).map(|i| values[(k as usize + i * 3) % values.len()]).collect();
            let sql = format!("SELECT printf({}, {}), format({0}, {1})", lit_sql(&Value::Text(f.clone())), args.join(", "));
            let bytes = |v: rusqlite::types::ValueRef| v.as_bytes().map(<[u8]>::to_vec).ok();
            let one = || db.query_row(&sql, [], |r| Ok((bytes(r.get_ref(0)?), bytes(r.get_ref(1)?)))).unwrap();
            let first = one();
            assert_eq!(first, one(), "{sql}");
            assert_eq!(first.0, first.1, "{sql}");
        }
        assert!(PRINTF_CONVERSIONS.iter().all(|c| convs.contains(c)));
    }

    #[test]
    fn spellings_read_as_the_plain_forms() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); CREATE TABLE t1(c0, c1); CREATE TABLE t2(c0); CREATE TABLE t3(c0); INSERT INTO t0 VALUES (1, 'a b'), (NULL, 'x'); INSERT INTO t1 VALUES (1, 2);").unwrap();
        let rows = |sql: &str| -> Vec<String> {
            let mut st = db.prepare(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
            let n = st.column_count();
            st.query_map([], |r| Ok((0..n).map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap())).collect::<Vec<_>>().join("|")))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let c = |k| Expr::col(0, k);
        let mut exprs: Vec<Expr> = Vec::new();
        for k in 0..2 {
            exprs.push(Expr::IsNull(bx(c(k)), false));
            exprs.push(Expr::IsNull(bx(c(k)), true));
            for op in [BinOp::Eq, BinOp::Ne, BinOp::Is, BinOp::IsNot] {
                exprs.push(Expr::bin(op, c(k), Expr::Lit(Value::Text("a b".into()))));
                for i in 0..12 {
                    exprs.push(Expr::bin(op, Expr::bin(BinOp::Add, c(k), Expr::Lit(Value::Int(i))), Expr::Lit(Value::Int(1))));
                }
            }
        }
        exprs.extend((0..40).map(|i| Expr::Lit(Value::Int(i * 997))));
        for p in ["a%", "_!b", "%A B%", "x!!", "%", "!", "a_", "%b", "A b", "!a%"] {
            exprs.extend((0..2).map(|k| Expr::bin(BinOp::Like, c(k), Expr::Lit(Value::Text(p.into())))));
            exprs.extend((0..2).map(|k| Expr::bin(BinOp::NotLike, c(k), Expr::Lit(Value::Text(p.into())))));
        }
        exprs.extend(["x\"y", "1.5", "c0 ", "-"].map(|t| Expr::Lit(Value::Text(t.into()))));
        exprs.extend((0..16).map(|i| Expr::Lit(Value::Text(format!("a b{i}")))));
        let mut spelled = std::collections::BTreeSet::new();
        for e in &exprs {
            for q in [Query::simple(0, false, Items::Exprs(vec![e.clone()]), None), Query {
                joins: vec![Join { table: 1, kind: JoinKind::Left(Expr::bin(BinOp::Or, Expr::bin(BinOp::Eq, c(0), Expr::col(1, 0)), e.clone())) }],
                ..Query::simple(0, false, Items::Exprs(vec![e.clone()]), Some(e.clone()))
            }] {
                set_spellings(false);
                let plain = q.sql();
                set_spellings(true);
                let sql = q.sql();
                if sql != plain {
                    spelled.insert(sql.clone());
                }
                assert_eq!(rows(&sql), rows(&plain), "{sql} against {plain}");
            }
        }
        set_spellings(false);
        let all = spelled.iter().cloned().collect::<Vec<_>>().join("\n");
        for form in ["ISNULL", "NOTNULL", " NOT NULL", "IS NOT DISTINCT FROM", "IS DISTINCT FROM", "==", "!=", "0x", "_", "\"a b", "SELECT ALL", "FROM (", "ESCAPE '!'"] {
            assert!(all.contains(form), "{form} never drawn:\n{all}");
        }
        set_spellings(true);
        let main: Vec<String> = (0..4).flat_map(|t| (0..4).map(move |k| source_name(t, At::TOP, k))).filter(|n| n.starts_with("main.")).collect();
        set_spellings(false);
        assert!(!main.is_empty() && main.len() < 12, "{main:?}");
        assert_eq!(rows(&format!("SELECT * FROM {} AS a0", main[0])), rows(&format!("SELECT * FROM {} AS a0", &main[0][5..])));
    }

    #[test]
    fn renders() {
        let e = Expr::Case(
            Some(bx(Expr::col(0, 1))),
            vec![(Expr::Lit(Value::Int(-5)), Expr::Neg(bx(Expr::Lit(Value::Int(-1)))))],
            Some(bx(Expr::Call(Func::Substr, vec![Expr::Lit(Value::Text("a'b".into())), Expr::Lit(Value::Int(2))]))),
        );
        assert_eq!(e.to_string(), "(CASE c1 WHEN -5 THEN (- -1) ELSE substr('a''b', 2) END)");
        let q = Query::simple(
            2,
            true,
            Items::Exprs(vec![Expr::Cast(bx(Expr::col(0, 0)), CastType::Numeric), e]),
            Some(Expr::In(bx(Expr::col(0, 0)), vec![Expr::Lit(Value::Blob(vec![0xab])), Expr::Lit(Value::Real(-0.0))], true)),
        );
        assert_eq!(
            q.render_with(0, &[" NOT INDEXED".to_string()]),
            "SELECT DISTINCT CAST(c0 AS NUMERIC), (CASE c1 WHEN -5 THEN (- -1) ELSE substr('a''b', 2) END) FROM t2 AS a0 NOT INDEXED WHERE (c0 NOT IN (x'ab', -0.0))"
        );
    }

    /// `zeroblob(n)` is valid only as the argument of a function that
    /// reads it as a BLOB ([`BLOB_READERS`]), never where it turns into
    /// text, a key or an operand (runs 20261016-2205 plans-noopt w3-147506,
    /// 20261017-0438 cte-materialized w2-440158), and is drawn so.
    #[test]
    fn zeroblobs_are_read_as_blobs() {
        let z = || Expr::Call(Func::Zeroblob, vec![Expr::Lit(Value::Int(2))]);
        for f in BLOB_READERS {
            assert!(zeroblobs_read_as_blobs(&Expr::Call(f, vec![z()])), "{f:?}");
        }
        for e in [
            z(),
            Expr::Cast(bx(z()), CastType::Text),
            Expr::Call(Func::Upper, vec![z()]),
            Expr::bin(BinOp::Concat, z(), z()),
            Expr::bin(BinOp::Eq, Expr::col(0, 0), z()),
            Expr::Call(Func::Hex, vec![Expr::Call(Func::Upper, vec![z()])]),
        ] {
            assert!(!zeroblobs_read_as_blobs(&e), "{e}");
        }
        let cx_schema = Schema { tables: vec![TableInfo {
            columns: 1, rows: 2, samples: vec![], by_column: vec![vec![Value::Text("ab".into())]], shared: vec![], key: RowKey::Rowid,
            exprs: vec![], partial: vec![], leading: vec![], coll: vec![None], aff: Vec::new(),
        }], ctes: vec![], catalog: Default::default() };
        let cx = Cx { schema: &cx_schema, levels: vec![vec![0]], min_level: 0, budget: 0, grouped: None };
        let mut drawn = 0;
        for seed in 0..20000 {
            let e = draw_text_call(&mut Source::new(seed, 50), &cx, 3);
            e.walk(&mut |n| drawn += usize::from(matches!(n, Expr::Call(Func::Zeroblob, _))));
            assert!(zeroblobs_read_as_blobs(&e), "{e}");
        }
        assert!(drawn > 100, "{drawn}");
    }

    #[test]
    fn renders_joins_groups_and_correlated_subqueries() {
        // EXISTS (SELECT * FROM t1 AS b0 WHERE b0.c0 = a1.c1): correlated
        // with the outer query's second source.
        let sub = Query::simple(1, false, Items::Star, Some(Expr::bin(BinOp::Eq, Expr::col(0, 0), Expr::Col(Col { up: 1, src: 1, col: 1 }))));
        let mut q = Query::simple(0, false, Items::Exprs(vec![Expr::col(1, 0), Expr::Agg(Agg::Count, None)]), None);
        q.joins.push(Join { table: 0, kind: JoinKind::Left(Expr::bin(BinOp::Eq, Expr::col(1, 0), Expr::col(0, 0))) });
        q.filter = Some(Expr::Exists(Box::new(sub), false));
        q.group = vec![Expr::col(1, 0)];
        q.having = Some(Expr::bin(BinOp::Gt, Expr::Agg(Agg::Count, None), Expr::Lit(Value::Int(1))));
        let schema = Schema { tables: vec![TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] }; 2], ctes: vec![], catalog: Default::default() };
        assert!(q.valid(&schema));
        assert_eq!(
            q.sql(),
            "SELECT a1.c0, count(*) FROM t0 AS a0 LEFT JOIN t0 AS a1 ON (a1.c0 = a0.c0) \
             WHERE (EXISTS (SELECT * FROM t1 AS b0 WHERE (c0 = a1.c1))) GROUP BY a1.c0 HAVING (count(*) > 1)"
        );
        assert_eq!(
            q.sql_not_indexed(),
            "SELECT a1.c0, count(*) FROM t0 AS a0 NOT INDEXED LEFT JOIN t0 AS a1 NOT INDEXED ON (a1.c0 = a0.c0) \
             WHERE (EXISTS (SELECT * FROM t1 AS b0 NOT INDEXED WHERE (c0 = a1.c1))) GROUP BY a1.c0 HAVING (count(*) > 1)"
        );
        // A bare column in an aggregate query is not valid; nor is a GROUP BY
        // term that SQLite reads as a column number.
        let mut bare = q.clone();
        bare.items = Items::Exprs(vec![Expr::col(0, 1)]);
        assert!(!bare.valid(&schema));
        let mut number = q.clone();
        number.group = vec![Expr::bin(BinOp::And, Expr::col(0, 0), Expr::Lit(Value::Int(0)))];
        number.items = Items::Exprs(vec![Expr::Agg(Agg::Count, None)]);
        assert!(!number.valid(&schema));
        // Dropping the first source: its columns become NULL, the second's
        // are renumbered, in the subquery too.
        let dropped = q.drop_source(0).unwrap();
        assert_eq!(
            dropped.sql(),
            "SELECT c0, count(*) FROM t0 AS a0 WHERE (EXISTS (SELECT * FROM t1 AS b0 WHERE (c0 = a0.c1))) GROUP BY c0 HAVING (count(*) > 1)"
        );
        assert!(dropped.cost() < q.cost());
    }

    #[test]
    fn renders_orders_and_limits() {
        let info = |key| TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info(RowKey::Rowid), info(RowKey::Columns(vec![1]))], ctes: vec![], catalog: Default::default() };
        let mut q = Query::simple(0, false, Items::Star, None);
        q.joins.push(Join { table: 1, kind: JoinKind::Cross });
        let term = |key| Order { key, desc: false, nulls: None, by: By::Number };
        q.order = vec![
            Order { key: OrderKey::Expr(Expr::bin(BinOp::Add, Expr::col(0, 0), Expr::Lit(Value::Int(1)))), desc: true, nulls: Some(false), by: By::Number },
            term(OrderKey::Column(3)),
            term(OrderKey::Expr(Expr::col(1, 1))),
            Order { key: OrderKey::Rowid(0), desc: false, nulls: Some(true), by: By::Number },
        ];
        q.limit = Some(Limit { count: 3, offset: 2, comma: true });
        assert!(q.valid(&schema));
        assert_eq!(q.total_order(&schema), Some(true));
        assert_eq!(
            q.sql(),
            "SELECT * FROM t0 AS a0 CROSS JOIN t1 AS a1 ORDER BY (a0.c0 + 1) DESC NULLS LAST, 4, a1.c1, a0.rowid NULLS FIRST LIMIT 2, 3"
        );
        // One source: ORDER BY expressions stay qualified.
        let one = q.drop_source(0).unwrap();
        assert_eq!(one.sql(), "SELECT * FROM t1 AS a0 ORDER BY (NULL + 1) DESC NULLS LAST, 4, a0.c1 LIMIT 2, 3");
        // Without the first source's rowid the order is not total; neither is
        // it with a result column out of range.
        assert!(!one.valid(&schema));
        let repaired = one.repair(&schema).unwrap();
        assert_eq!(repaired.sql(), "SELECT * FROM t1 AS a0 ORDER BY (NULL + 1) DESC NULLS LAST, a0.c1 LIMIT 2, 3");
        assert_eq!(repaired.total_order(&schema), Some(true));
        assert!(repaired.cost() < one.cost());
        // A DISTINCT query orders by result columns only.
        let mut d = Query::simple(1, true, Items::Star, None);
        d.order = vec![term(OrderKey::Column(1)), term(OrderKey::Column(0))];
        assert_eq!(d.total_order(&schema), Some(false));
        d.order.push(term(OrderKey::Expr(Expr::col(0, 1))));
        assert!(!d.valid(&schema));
        assert_eq!(Limit { count: 2, offset: -1, comma: false }.slice(&[1, 2, 3]), [1, 2]);
        assert_eq!(Limit { count: -1, offset: 1, comma: false }.slice(&[1, 2, 3]), [2, 3]);
        assert_eq!(Limit { count: 5, offset: 9, comma: false }.slice(&[1, 2, 3]), [0; 0]);
    }

    #[test]
    fn renders_compounds() {
        let info = TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info.clone(), TableInfo { columns: 1, ..info }], ctes: vec![], catalog: Default::default() };
        let mut q = Query::simple(0, false, Items::Star, Some(Expr::IsNull(bx(Expr::col(0, 1)), false)));
        let member = Query::simple(1, true, Items::Exprs(vec![Expr::col(0, 0), Expr::Lit(Value::Int(1))]), None);
        q.compound = vec![(SetOp::Intersect, member.clone()), (SetOp::UnionAll, Query::simple(0, false, Items::Star, None))];
        q.order = vec![Order { key: OrderKey::Column(1), desc: true, nulls: None, by: By::Number }, Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Number }];
        q.limit = Some(Limit { count: 2, offset: 0, comma: false });
        assert!(q.valid(&schema));
        assert_eq!(q.total_order(&schema), Some(false));
        assert_eq!(
            q.sql(),
            "SELECT * FROM t0 AS a0 WHERE (c1 IS NULL) INTERSECT SELECT DISTINCT c0, 1 FROM t1 AS a0 UNION ALL SELECT * FROM t0 AS a0 ORDER BY 2 DESC, 1 LIMIT 2 OFFSET 0"
        );
        assert_eq!(
            q.sql_not_indexed(),
            "SELECT * FROM t0 AS a0 NOT INDEXED WHERE (c1 IS NULL) INTERSECT SELECT DISTINCT c0, 1 FROM t1 AS a0 NOT INDEXED \
             UNION ALL SELECT * FROM t0 AS a0 NOT INDEXED ORDER BY 2 DESC, 1 LIMIT 2 OFFSET 0"
        );
        assert_eq!(q.tables(), [0, 1]);
        // Members of another width are not valid; nor is a member with an
        // ORDER BY of its own.
        let mut wide = q.clone();
        wide.compound[0].1 = Query::simple(1, false, Items::Star, None);
        assert!(!wide.valid(&schema));
        let mut ordered = q.clone();
        ordered.compound[1].1.order = vec![Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Number }];
        assert!(!ordered.valid(&schema));
        // Against a schema where t0 has one column, `*` is one column wide:
        // the members of two columns go, and the ORDER BY by column 2.
        let narrow = Schema { tables: vec![TableInfo { columns: 1, ..schema.tables[1].clone() }; 2], ctes: vec![], catalog: Default::default() };
        let r = q.repair(&narrow).unwrap();
        assert_eq!(r.sql(), "SELECT * FROM t0 AS a0 WHERE (NULL IS NULL) UNION ALL SELECT * FROM t0 AS a0 ORDER BY 1 LIMIT 2 OFFSET 0");
        assert!(r.cost() < q.cost());
        let mut cands = Vec::new();
        let _ = shrink_query(&q, &schema, &mut |c| {
            cands.push(c);
            ControlFlow::Continue(())
        });
        assert!(cands.iter().all(|c| c.cost() < q.cost()));
        assert!(cands.contains(&member));
    }

    #[test]
    fn integer_constants_fold_as_the_parser_folds_them() {
        let lit = |v: Value| Expr::Lit(v);
        let null = |e: Expr, not: bool| Expr::IsNull(bx(e), not);
        let and = |a: Expr, b: Expr| Expr::bin(BinOp::And, a, b);
        // `((' 2 ' IS NOT NULL) IS NULL)`: the inner fold is an integer, so
        // the outer folds too; `(c0 AND ('?' IS NULL))` and `(c0 AND (-0.0
        // IS NULL))` are 0.
        assert!(int_const(&null(null(lit(Value::Text(" 2 ".into())), true), false)));
        assert!(int_const(&and(Expr::col(0, 0), null(lit(Value::Text("?".into())), false))));
        assert!(int_const(&and(Expr::col(0, 0), null(Expr::Neg(bx(lit(Value::Real(-0.0)))), false))));
        assert!(int_const(&and(Expr::col(0, 0), Expr::bin(BinOp::Is, lit(Value::Int(3)), lit(Value::Null)))));
        // `IS NOT NULL` folds to 1, not false; `NULL IS NULL` does not fold;
        // nor does a unary minus make a false operand.
        assert!(!int_const(&and(Expr::col(0, 0), null(lit(Value::Int(3)), true))));
        assert!(!int_const(&null(lit(Value::Null), false)));
        assert!(!int_const(&and(Expr::col(0, 0), Expr::Neg(bx(lit(Value::Int(0)))))));
        assert!(!int_const(&null(Expr::col(0, 0), false)));
    }

    #[test]
    fn renders_outer_and_using_joins() {
        let info = |columns| TableInfo { columns, rows: 3, samples: vec![], by_column: vec![vec![]; columns], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info(3), info(2)], ctes: vec![], catalog: Default::default() };
        let mut q = Query::simple(0, false, Items::Star, None);
        q.joins.push(Join { table: 1, kind: JoinKind::Using(JoinOp::Full, vec![1, 0]) });
        assert!(q.valid(&schema));
        assert_eq!(q.width(&schema), 3);
        assert_eq!(q.sql(), "SELECT * FROM t0 AS a0 FULL JOIN t1 AS a1 USING (c1, c0)");
        q.joins[0].kind = JoinKind::Natural(JoinOp::Right);
        assert_eq!(q.sql(), "SELECT * FROM t0 AS a0 NATURAL RIGHT JOIN t1 AS a1");
        assert_eq!(q.width(&schema), 3);
        // A third source with a column `c0`: `*` is ambiguous to SQLite (see
        // `using_ok`), and so it is with a later RIGHT JOIN; not with items.
        q.joins.push(Join { table: 1, kind: JoinKind::Cross });
        assert!(!q.valid(&schema));
        q.joins[0].kind = JoinKind::Natural(JoinOp::Left);
        assert!(q.valid(&schema));
        q.joins[1].kind = JoinKind::Right(Expr::bin(BinOp::Eq, Expr::col(2, 0), Expr::col(0, 2)));
        assert!(!q.valid(&schema));
        q.items = Items::Exprs(vec![Expr::col(2, 1)]);
        assert!(q.valid(&schema));
        assert_eq!(q.sql(), "SELECT a2.c1 FROM t0 AS a0 NATURAL LEFT JOIN t1 AS a1 RIGHT JOIN t1 AS a2 ON (a2.c0 = a0.c2)");
        // USING only in the first join, of columns both sources have.
        let mut late = q.clone();
        late.joins[1].kind = JoinKind::Using(JoinOp::Inner, vec![0]);
        assert!(!late.valid(&schema));
        let mut wide = q.clone();
        wide.joins[0].kind = JoinKind::Using(JoinOp::Inner, vec![2]);
        assert!(!wide.valid(&schema));
        let mut cands = Vec::new();
        let _ = shrink_query(&q, &schema, &mut |c| {
            assert!(c.cost() < q.cost());
            cands.push(c);
            ControlFlow::Continue(())
        });
        assert!(cands.iter().any(|c| c.joins.len() == 2 && c.joins[1].kind.op() == Some(JoinOp::Inner)));
    }

    #[test]
    fn renders_json() {
        let doc = Expr::Lit(Value::Text("{\"a\":[1,'x']}".into()));
        let e = Expr::bin(BinOp::Arrow2, doc.clone(), Expr::Lit(Value::Text("$.a[#-1]".into())));
        assert_eq!(e.to_string(), "('{\"a\":[1,''x'']}' ->> '$.a[#-1]')");
        let set = Expr::Call(Func::JsonbSet, vec![doc, Expr::Lit(Value::Text("$.b".into())), unblob(Expr::col(0, 1))]);
        assert_eq!(set.to_string(), "jsonb_set('{\"a\":[1,''x'']}', '$.b', (CASE typeof(c1) WHEN 'blob' THEN hex(c1) ELSE c1 END))");
        let each = Expr::Call(Func::JsonTree, vec![Expr::Lit(Value::Text("fullkey".into())), set, Expr::Lit(Value::Text("$.a".into()))]);
        assert!(json_args_ok(Func::JsonTree, if let Expr::Call(_, a) = &each { a } else { unreachable!() }));
        assert!(each.to_string().starts_with("(SELECT count(DISTINCT fullkey) FROM json_tree(jsonb_set("), "{each}");
        let rows = Expr::Call(Func::JsonEach, vec![Expr::Lit(Value::Text("*".into())), Expr::Lit(Value::Null)]);
        assert_eq!(rows.to_string(), "(SELECT count(*) FROM json_each(NULL))");
        let agg = Expr::Agg(Agg::Max, Some(bx(Expr::Cast(bx(Expr::col(0, 0)), CastType::Text))));
        assert!(!json_args_ok(Func::JsonEach, &[Expr::Lit(Value::Text("*".into())), Expr::Call(Func::JsonQuote, vec![agg])]));
    }

    /// `json_pretty`'s indent is a short literal of spaces and tabs, or
    /// NULL: never a column, whose value may be long enough to make the
    /// output pass SQLite's length limits (a finding of the runner).
    #[test]
    fn json_pretty_indents_are_short_literals() {
        let doc = Expr::Lit(Value::Text("[1,{\"a\":2}]".into()));
        let ok = |indent: Expr| json_args_ok(Func::JsonPretty, &[doc.clone(), indent]);
        assert!(JSON_INDENTS.iter().all(|s| ok(Expr::Lit(Value::Text(s.to_string())))));
        assert!(ok(Expr::Lit(Value::Null)));
        assert!(!ok(Expr::col(0, 1)));
        assert!(!ok(Expr::Lit(Value::Text("     ".into()))));
        assert!(!ok(Expr::Lit(Value::Text("ab".into()))));
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for s in JSON_INDENTS {
            let sql = format!("SELECT length(json_pretty('[1,[2]]', {})) < 40", literal(&Value::Text(s.to_string())));
            assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 1, "{sql}");
            let sql = format!("SELECT json_extract(json_pretty('[{{}},1e999,{{\"a\":\"\\\\\"}}]', {}), '$[1]')", literal(&Value::Text(s.to_string())));
            db.query_row(&sql, [], |r| r.get::<_, f64>(0)).unwrap();
        }
        // The runner's mutants: a
        // json_pretty with a JSONB or integer indent read as JSON. SQLite
        // finds it malformed; the rules now reject it, in a query too.
        for indent in ["jsonb_array(3, '0ж')", "0"] {
            let sql = format!("SELECT json_extract(json_pretty('[{{}},1e999]', {indent}), '$')");
            let e = db.query_row(&sql, [], |r| r.get::<_, Option<String>>(0)).unwrap_err().to_string();
            assert!(e.contains("malformed JSON"), "{sql}: {e}");
        }
        for indent in [Expr::Call(Func::JsonbArray, vec![Expr::Lit(Value::Int(3))]), Expr::Lit(Value::Int(0))] {
            let pretty = Expr::Call(Func::JsonPretty, vec![doc.clone(), indent]);
            let read = Expr::Call(Func::JsonExtract, vec![pretty, Expr::Lit(Value::Text("$".into()))]);
            let q = Query::simple(0, false, Items::Exprs(vec![read]), None);
            let schema = Schema { tables: vec![TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] }], ctes: vec![], catalog: Default::default() };
            assert!(!q.valid(&schema), "{}", q.sql());
        }
    }

    /// A `CAST` of a `jsonb` value keeps its JSON subtype, so as a JSON
    /// function's value it is read as JSON text (a finding of the runner):
    /// never a value argument, through `coalesce` or `CASE` either.
    #[test]
    fn casts_of_jsonb_are_no_json_values() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let e = db.query_row("SELECT json_array(jsonb_array(CAST(jsonb_object('', NULL) AS TEXT)))", [], |r| r.get::<_, String>(0)).unwrap_err();
        assert!(e.to_string().contains("BLOB values"), "SQLite may clear the subtype now: {e}");
        let jsonb = Expr::Call(Func::JsonbObject, vec![Expr::Lit(Value::Text(String::new())), Expr::Lit(Value::Null)]);
        let cast = Expr::Cast(bx(jsonb.clone()), CastType::Text);
        assert!(!value_arg_ok(&cast));
        assert!(!value_arg_ok(&unblob(cast.clone())));
        assert!(!value_arg_ok(&Expr::Call(Func::Coalesce, vec![cast, Expr::Lit(Value::Int(1))])));
        assert!(value_arg_ok(&jsonb));
        assert!(value_arg_ok(&Expr::Cast(bx(Expr::col(0, 0)), CastType::Text)));
    }

    /// The aggregates `min` and `max` return their value with its subtype,
    /// so `max(CAST(jsonb_object(..) AS TEXT))` is a cast of a `jsonb`
    /// value too: never a value argument (the runner's HAVING, a
    /// tlp-having failure; the variant
    /// evaluates the call on groups the original's `AND` skipped). Nor a
    /// scalar subquery of one, which keeps the subtype up to SQLite
    /// check-in 8ab55e0185 and has none from it on (there the call returns
    /// the cast's text as a string; the rule stays, see [`cast_of_jsonb`]).
    /// Which of the two this SQLite is, SQLite tells: a scalar subquery of
    /// `json('[1]')` goes into `json_array` as JSON while it keeps the
    /// subtype, as a string once it does not.
    #[test]
    fn aggregates_of_casts_of_jsonb_are_no_json_values() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 2);").unwrap();
        let jsonb = Expr::Call(
            Func::JsonbObject,
            vec![Expr::Lit(Value::Text("b".into())), Expr::Lit(Value::Int(-87157)), Expr::Lit(Value::Text("é".into())), Expr::Lit(Value::Int(-1))],
        );
        let cast = Expr::Cast(bx(jsonb), CastType::Text);
        for agg in [Agg::Max, Agg::Min] {
            let value = Expr::Agg(agg, Some(bx(cast.clone())));
            assert!(!value_arg_ok(&value));
            let call = Expr::Call(Func::JsonArrayInsert, vec![Expr::Lit(Value::Text("[1]".into())), Expr::Lit(Value::Text("$[0]".into())), value]);
            assert!(!json_args_ok(Func::JsonArrayInsert, match &call {
                Expr::Call(_, a) => a,
                _ => unreachable!(),
            }));
            let q = Query {
                group: vec![Expr::col(0, 0)],
                having: Some(Expr::IsNull(bx(call), false)),
                ..Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0)]), None)
            };
            let e = db.query_row(&q.sql(), [], |r| r.get::<_, rusqlite::types::Value>(0)).unwrap_err();
            assert!(e.to_string().contains("malformed JSON"), "SQLite may clear the subtype now: {e}");
            let schema = Schema { tables: vec![TableInfo { columns: 2, rows: 1, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] }], ctes: vec![], catalog: Default::default() };
            assert!(!q.valid(&schema), "{}", q.sql());
        }
        // A scalar subquery of one, even unblobbed: never a value argument.
        let scalar = Expr::Scalar(Box::new(Query::simple(0, false, Items::Exprs(vec![Expr::Agg(Agg::Max, Some(bx(cast.clone())))]), None)));
        assert!(!value_arg_ok(&scalar));
        assert!(!value_arg_ok(&unblob(scalar.clone())));
        let args = vec![Expr::Lit(Value::Text("[1]".into())), Expr::Lit(Value::Text("$[0]".into())), unblob(scalar)];
        assert!(!json_args_ok(Func::JsonArrayInsert, &args));
        let call = Expr::Call(Func::JsonArrayInsert, args);
        let embedded: String = db.query_row("SELECT json_array((SELECT json('[1]')))", [], |r| r.get(0)).unwrap();
        let keeps = match embedded.as_str() {
            "[[1]]" => true,
            "[\"[1]\"]" => false,
            other => panic!("a scalar subquery's JSON went into json_array as neither JSON nor a string: {other}"),
        };
        // Its type: the text holds the JSONB bytes, no UTF-8.
        let got = db.query_row(&format!("SELECT typeof({call})"), [], |r| r.get::<_, String>(0));
        if keeps {
            let e = got.unwrap_err();
            assert!(e.to_string().contains("malformed JSON"), "{e}");
        } else {
            assert_eq!(got.unwrap(), "text");
            let sql = format!("SELECT json_extract({call}, '$[0]') IS {cast} AND json_extract({call}, '$[1]') = 1");
            assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 1, "{sql}");
        }
    }

    /// Every form of a counting CTE ([`COUNT_FORMS`]) gives its integers
    /// in ascending order, as its inlined UNION ALL of literals does.
    #[test]
    fn counting_ctes_count_in_every_form() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        for form in 0..COUNT_FORMS.len() as u8 {
            for (start, count) in [(-2, 1), (0, 3), (2, 12)] {
                let cte = Cte { body: CteBody::Count { start, count, form }, hint: None, columns: false };
                let all = |sql: String| -> String { db.query_row(&format!("SELECT group_concat(c0, ',') FROM {sql}"), [], |r| r.get(0)).unwrap() };
                let want: Vec<String> = (start..start + count).map(|k| k.to_string()).collect();
                let got = db
                    .query_row(&format!("WITH RECURSIVE x0(c0) AS ({}) SELECT group_concat(c0, ',') FROM x0", cte.body_sql(0, false)), [], |r| r.get::<_, String>(0))
                    .unwrap();
                assert_eq!(got, want.join(","), "{form} {start} {count}");
                assert_eq!(all(cte.inline_sql(0, false)), want.join(","), "{form}");
            }
        }
    }

    /// The JSON arguments' rules ([`json_args_ok`]) against SQLite: the
    /// texts drawn are JSON to it, JSON5 too, the paths well formed, and
    /// the rules reject what SQLite raises an error on.
    #[test]
    fn json_arguments_are_what_sqlite_takes() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let one = |sql: &str| db.query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0));
        let text = |s: &str| literal(&Value::Text(s.to_string()));
        for seed in 0..300u64 {
            let mut g = Source::new(seed, 50);
            let (mut s, mut paths) = (String::new(), Vec::new());
            draw_json_text(&mut g, 3, &mut s, "$", &mut paths);
            assert!(json_text_ok(&s), "{s}");
            assert_eq!(one(&format!("SELECT json_valid({})", text(&s))).unwrap(), rusqlite::types::Value::Integer(1), "{s}");
            for p in &paths {
                assert!(path_ok(p, false), "{p}");
                let found = one(&format!("SELECT json_type({}, {})", text(&s), text(p))).unwrap();
                assert!(found != rusqlite::types::Value::Null, "{s} {p}");
            }
            let array = g.bool();
            let p = draw_path(&mut g, array);
            assert!(path_ok(&p, array), "{p}");
            let f = if array { "json_array_insert" } else { "json_set" };
            one(&format!("SELECT {f}({}, {}, 1), json_extract({}, {})", text(&s), text(&p), text(&s), text(&p))).unwrap();
        }
        for seed in 0..400u64 {
            let mut g = Source::new(seed, 50);
            let (mut s, mut paths) = (String::new(), Vec::new());
            draw_json5_text(&mut g, 3, &mut s, "$", &mut paths);
            assert!(json5_text_ok(&s), "{s}");
            assert_eq!(one(&format!("SELECT json_valid({}, 2)", text(&s))).unwrap(), rusqlite::types::Value::Integer(1), "{s}");
            for p in &paths {
                let found = one(&format!("SELECT json_type({}, {})", text(&s), text(p))).unwrap();
                assert!(found != rusqlite::types::Value::Null, "{s} {p}");
            }
            // As SQLite reads it: what it is cut short to or a byte of it
            // replaced by is JSON5 to the rules only if it is to SQLite.
            for k in 0..s.len() {
                for t in [s.get(..k).map(str::to_string), s.get(..k).zip(s.get(k + 1..)).map(|(a, b)| format!("{a}{}{b}", ['x', ',', '\'', '0', '.', '/', '\\'][k % 7]))] {
                    if let Some(t) = t.filter(|t| json5_text_ok(t)) {
                        assert_eq!(one(&format!("SELECT json_valid({}, 2)", text(&t))).unwrap(), rusqlite::types::Value::Integer(1), "{t}");
                    }
                }
            }
        }
        for d in ["[-NaN]", "[1//x]", "\"\\01\"", "0x", "[,]", "{,}", "[1,,]", ".", "[0x1f.5]", "{é:1}", "'a\"b'", "[01]", "/* A */1", "// x", "\"\\x4\""] {
            assert!(!json5_text_ok(d), "{d}");
            assert!(!json_text_ok(d), "{d}");
        }
        for seed in 0..100u64 {
            let mut g = Source::new(seed, 50);
            let mut s = String::new();
            draw_patch_text(&mut g, 3, &mut s);
            assert!(json_text_ok(&s), "{s}");
        }
        // JSONB blobs: valid by SQLite's strict check undamaged, their
        // paths found; damaged or not, JSON functions read what the check
        // passes, and `json_valid` and `json_error_position` take any.
        let (mut valid, mut damaged) = (0, 0);
        for seed in 0..600u64 {
            let mut g = Source::new(seed, 50);
            let (b, paths) = jsonb_literal(&mut g, seed % 2 == 1);
            let x = literal(&b);
            let ok = one(&format!("SELECT json_valid({x}, 8)")).unwrap() == rusqlite::types::Value::Integer(1);
            if seed % 2 == 0 {
                assert!(ok, "{x}");
            }
            for k in 1..=15 {
                one(&format!("SELECT json_valid({x}, {k}), json_error_position({x})")).unwrap();
            }
            if ok {
                valid += 1;
                let out = one(&format!("SELECT json({x}) || json_pretty({x}) || (SELECT count(*) FROM json_tree({x})) || json_set({x}, '$.a', 1)")).unwrap();
                assert!(matches!(out, rusqlite::types::Value::Text(_)), "{x}");
                for p in &paths {
                    let found = one(&format!("SELECT json_type({x}, {})", text(p))).unwrap();
                    assert!(seed % 2 == 1 || found != rusqlite::types::Value::Null, "{x} {p}");
                }
            } else {
                damaged += 1;
            }
        }
        assert!(valid > 300 && damaged > 30, "{valid} {damaged}");
        for d in JSON5_DOCS {
            assert!(!json_text_ok(d), "{d}");
            assert_eq!(one(&format!("SELECT json_valid({}, 2)", text(d))).unwrap(), rusqlite::types::Value::Integer(1), "{d}");
        }
        for d in ["", " ", "01", "1.", ".5", "-", "[1,]", "{\"a\":1,}", "{a:1}", "'a'", "\"\\x41\"", "\"\t\"", "[1]]", "nul", "1e", "\"\\ud8\""] {
            assert!(!json_text_ok(d), "{d}");
        }
        for p in ["$", "$.a", "$.\"x y\"", "$[0]", "$[#]", "$[#-1].b[2]", "$.é.c"] {
            assert!(path_ok(p, false), "{p}");
            one(&format!("SELECT json_extract('[]', {})", text(p))).unwrap();
        }
        for p in ["", "a", "$.", "$..a", "$[", "$[a]", "$[-1]", "$[#-]", "$.\"a", "$a"] {
            assert!(!path_ok(p, false), "{p}");
            // SQLite reads a path as far as the document goes: an object
            // and an array between them reach every step's error.
            let err = |doc: &str| one(&format!("SELECT json_extract('{doc}', {})", text(p))).is_err();
            assert!(err("{\"a\":[1]}") || err("[1]"), "{p}");
        }
        assert!(!path_ok("$.a", true) && !path_ok("$", true) && path_ok("$.a[0]", true));
        assert!(one("SELECT json_array_insert('{\"a\":1}', '$.a', 1)").is_err());
        let col = Expr::col(0, 0);
        let path = Expr::Lit(Value::Text("$".into()));
        assert!(!json_args_ok(Func::JsonExtract, &[col.clone(), path.clone()]));
        assert!(!json_args_ok(Func::JsonArray, std::slice::from_ref(&col)));
        assert!(json_args_ok(Func::JsonArray, &[unblob(col.clone()), Expr::Call(Func::Jsonb, vec![Expr::Lit(Value::Null)])]));
        assert!(!json_args_ok(Func::JsonSet, &[Expr::Lit(Value::Null), path.clone()]));
        assert!(!json_args_ok(Func::JsonObject, &[Expr::Lit(Value::Int(1)), Expr::Lit(Value::Int(1))]));
        let guarded = Expr::Case(None, vec![(Expr::Call(Func::JsonValid, vec![col.clone()]), col.clone())], None);
        assert!(json_args_ok(Func::JsonExtract, &[guarded, path]));
        for (sql, err) in [
            ("json_extract('x', '$')", "malformed JSON"),
            ("json_array(x'')", "JSON cannot hold BLOB values"),
            ("json_object(1, 1)", "labels must be TEXT"),
            ("json_extract('[]', '$.')", "bad JSON path"),
        ] {
            assert!(one(&format!("SELECT {sql}")).unwrap_err().to_string().contains(err), "{sql}");
        }
        for (sql, v) in [
            ("'[1,2]' -> -1", "2"),
            ("'{\"x y\":3}' ->> 'x y'", "3"),
            ("'[[0,5]]' ->> '[0][1]'", "5"),
            ("json_array(CASE WHEN json_valid('[1]') THEN '[1]' END, jsonb('[2]'))", "[\"[1]\",[2]]"),
        ] {
            assert_eq!(one(&format!("SELECT CAST({sql} AS TEXT)")).unwrap(), rusqlite::types::Value::Text(v.into()), "{sql}");
        }
    }

    /// Row values render as SQLite reads them and run: a comparison, a
    /// BETWEEN, a list (as VALUES) and a subquery of two columns; a list
    /// with `count(*)` (which VALUES would own), rows of unequal width and a
    /// one-term row are invalid; shrinking reaches the scalar forms.
    #[test]
    fn renders_row_values() {
        let info = TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Columns(vec![1, 0]), exprs: vec![], partial: vec![], leading: vec![1], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info.clone(), info], ctes: vec![], catalog: Default::default() };
        let (c0, c1) = (Expr::col(0, 0), Expr::col(0, 1));
        let int = |i| Expr::Lit(Value::Int(i));
        let sub = Query::simple(1, false, Items::Exprs(vec![Expr::col(0, 1), Expr::col(0, 0)]), None);
        let preds = [
            (Expr::RowCmp(BinOp::Lt, vec![c1.clone(), c0.clone()], vec![int(1), int(2)]), "((c1, c0) < (1, 2))"),
            (Expr::RowBetween(vec![c1.clone(), c0.clone()], vec![int(0), int(0)], vec![int(5), int(5)], true), "((c1, c0) NOT BETWEEN (0, 0) AND (5, 5))"),
            (Expr::RowIn(vec![c1.clone(), c0.clone()], vec![vec![int(1), int(2)], vec![int(3), Expr::Lit(Value::Null)]], false), "((c1, c0) IN ((1, 2), (3, NULL)))"),
            (Expr::RowInSelect(vec![c1.clone(), c0.clone()], Box::new(sub), true), "((c1, c0) NOT IN (SELECT c1, c0 FROM t1 AS b0))"),
        ];
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1, PRIMARY KEY(c1, c0)); CREATE TABLE t1(c0, c1, PRIMARY KEY(c1, c0)); INSERT INTO t0 VALUES (1, 1), (2, 3); INSERT INTO t1 VALUES (1, 1);").unwrap();
        for (p, sql) in &preds {
            let q = Query::simple(0, false, Items::Star, Some(p.clone()));
            assert!(q.valid(&schema), "{sql}");
            assert!(q.sql().contains(sql), "{} lacks {sql}", q.sql());
            db.prepare(&q.sql()).unwrap().query([]).unwrap().next().unwrap();
            let mut scalar = false;
            let _ = shrink_expr(p, &mut |c| {
                scalar |= matches!(c, Expr::Bin(..) | Expr::Between(..) | Expr::In(..) | Expr::InSelect(..));
                ControlFlow::Continue(())
            });
            assert!(scalar, "{sql}");
        }
        let bad = [
            Expr::RowIn(vec![c1.clone(), c0.clone()], vec![vec![int(1), Expr::Agg(Agg::Count, None)], vec![int(3), int(4)]], false),
            Expr::RowIn(vec![c1.clone(), c0.clone()], vec![vec![int(1)], vec![int(3), int(4)]], false),
            Expr::RowCmp(BinOp::Eq, vec![c1.clone()], vec![int(1)]),
        ];
        for p in bad {
            assert!(!Query::simple(0, false, Items::Star, Some(p)).valid(&schema));
        }
    }

    #[test]
    fn renders_windows() {
        let info = |key| TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info(RowKey::Rowid), info(RowKey::Columns(vec![1]))], ctes: vec![], catalog: Default::default() };
        let term = |e: Expr, desc| Order { key: OrderKey::Expr(e), desc, nulls: None, by: By::Number };
        let w = Window {
            func: WinFunc::Sum,
            arg: Some(Expr::Cast(bx(Expr::col(1, 0)), CastType::Integer)),
            partition: vec![Expr::col(0, 1)],
            order: vec![term(Expr::col(1, 0), true), term(Expr::col(0, ROWID), false), term(Expr::col(1, 1), false)],
            frame: Some(Frame { mode: FrameMode::Rows, start: Bound::Preceding(2), end: Bound::CurrentRow, exclude: Some(Exclude::Ties), short: true }),
            default: None,
            filter: None,
            named: Named::Inline,
        };
        let mut q = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0), Expr::Window(Box::new(w.clone()))]), None);
        q.joins.push(Join { table: 1, kind: JoinKind::Cross });
        assert!(q.valid(&schema));
        assert_eq!(
            q.sql(),
            "SELECT a0.c0, sum(CAST(a1.c0 AS INTEGER)) OVER (PARTITION BY a0.c1 ORDER BY a1.c0 DESC, a0.rowid, a1.c1 \
             ROWS 2 PRECEDING EXCLUDE TIES) FROM t0 AS a0 CROSS JOIN t1 AS a1"
        );
        // A ROWS frame needs every source's key; a RANGE frame does not.
        let mut partial = w.clone();
        partial.order.pop();
        let with = |w: Window| Query { items: Items::Exprs(vec![Expr::Window(Box::new(w))]), ..q.clone() };
        assert!(!with(partial.clone()).valid(&schema));
        partial.frame = Some(Frame { mode: FrameMode::Range, start: Bound::UnboundedPreceding, end: Bound::CurrentRow, exclude: None, short: false });
        assert!(with(partial.clone()).valid(&schema));
        // Not with DISTINCT, nor in a WHERE clause, nor with a frame that
        // SQLite rejects.
        assert!(!Query { distinct: true, ..q.clone() }.valid(&schema));
        assert!(!Query { filter: Some(Expr::Window(Box::new(w.clone()))), ..q.clone() }.valid(&schema));
        partial.frame = Some(Frame { mode: FrameMode::Groups, start: Bound::Following(1), end: Bound::Preceding(1), exclude: None, short: false });
        assert!(!with(partial).valid(&schema));
        // Dropping a source leaves the other's key: still total.
        assert!(q.drop_source(1).unwrap().valid(&schema));
        let mut cands = 0;
        let _ = shrink_query(&q, &schema, &mut |c| {
            assert!(c.cost() < q.cost());
            cands += 1;
            ControlFlow::Continue(())
        });
        assert!(cands > 10);
    }

    #[test]
    fn renders_ctes() {
        let info = TableInfo { columns: 2, rows: 3, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info.clone(); 2], ctes: vec![], catalog: Default::default() };
        let body = Query::simple(1, false, Items::Exprs(vec![Expr::col(0, 1)]), Some(Expr::IsNull(bx(Expr::col(0, 0)), true)));
        let sub = Query::simple(CTE + 1, false, Items::Star, Some(Expr::bin(BinOp::Eq, Expr::col(0, 0), Expr::Col(Col { up: 1, src: 0, col: 0 }))));
        let mut q = Query::simple(CTE, false, Items::Star, Some(Expr::Exists(Box::new(sub), false)));
        q.joins.push(Join { table: 0, kind: JoinKind::Inner(Expr::bin(BinOp::Eq, Expr::col(1, 1), Expr::col(0, 0))) });
        q.with = vec![
            Cte { body: CteBody::Select(Box::new(body)), hint: Some(true), columns: false },
            Cte { body: CteBody::Count { start: 2, count: 3, form: 0 }, hint: None, columns: false },
        ];
        assert!(q.valid(&schema));
        assert_eq!(
            q.sql(),
            "WITH RECURSIVE x0 AS MATERIALIZED (SELECT c1 AS c0 FROM t1 AS a0 WHERE (c0 IS NOT NULL)), \
             x1(c0) AS (SELECT 2 AS c0 UNION ALL SELECT c0 + 1 FROM x1 WHERE c0 < 4) \
             SELECT * FROM x0 AS a0 JOIN t0 AS a1 ON (a1.c1 = a0.c0) WHERE (EXISTS (SELECT * FROM x1 AS b0 WHERE (c0 = a0.c0)))"
        );
        assert_eq!(
            q.sql_inline(),
            "SELECT * FROM (SELECT c1 AS c0 FROM t1 AS a0 WHERE (c0 IS NOT NULL)) AS a0 JOIN t0 AS a1 ON (a1.c1 = a0.c0) \
             WHERE (EXISTS (SELECT * FROM (SELECT 2 AS c0 UNION ALL SELECT 3 UNION ALL SELECT 4) AS b0 WHERE (c0 = a0.c0)))"
        );
        assert_eq!(q.tables(), [0, 1]);
        assert_eq!(q.count_reads(CTE + 1), 1);
        // A column past the CTE's one is not valid; a CTE nothing reads can go.
        let mut wide = q.clone();
        wide.joins[0].kind = JoinKind::Inner(Expr::col(0, 1));
        assert!(!wide.valid(&schema));
        assert!(q.drop_cte(0).is_none());
        let mut unread = q.clone();
        unread.filter = None;
        let dropped = unread.drop_cte(1).unwrap();
        assert!(dropped.valid(&schema) && dropped.cost() < unread.cost());
        assert_eq!(dropped.with.len(), 1);
        // A CTE's query must be determined: no DISTINCT, no LIMIT without an
        // order by every source's key.
        let mut distinct = q.clone();
        if let CteBody::Select(b) = &mut distinct.with[0].body {
            b.distinct = true;
        }
        assert!(!distinct.valid(&schema));
        // Without t1, the CTE's query has no table left: the query goes.
        let small = Schema { tables: vec![info], ctes: vec![], catalog: Default::default() };
        assert!(q.repair(&small).is_none());
        let mut cands = 0;
        let _ = shrink_query(&q, &schema, &mut |c| {
            assert!(c.cost() < q.cost());
            cands += 1;
            ControlFlow::Continue(())
        });
        assert!(cands > 10);
    }

    #[test]
    fn date_calls_are_fixed_by_their_arguments() {
        let text = |s: &str| Expr::Lit(Value::Text(s.into()));
        let c = Expr::col(0, 0);
        assert!(date_args_ok(Func::Date, &[c.clone(), text("+1 day"), text("start of month")]));
        assert!(date_args_ok(Func::Strftime, &[c.clone(), c.clone()]));
        // The current time, the host's timezone, and too few arguments
        // (`strftime(f)` and `date()` read the current time).
        assert!(!date_args_ok(Func::Date, &[text(" Now ")]));
        assert!(!date_args_ok(Func::Timediff, &[c.clone(), Expr::Call(Func::Lower, vec![text("NOW")])]));
        assert!(!date_args_ok(Func::Datetime, &[c.clone(), text("localtime")]));
        assert!(!date_args_ok(Func::Unixepoch, &[c.clone(), text("UTC")]));
        assert!(!date_args_ok(Func::Date, &[c.clone(), c.clone()]));
        assert!(!date_args_ok(Func::Strftime, &[text("%Y")]));
        assert!(!date_args_ok(Func::Date, &[]));
        // Drawn calls of time literals: SQLite answers each without an
        // error, the same in two statements, and not mostly NULL.
        let schema = Schema { tables: vec![TableInfo { columns: 1, rows: 1, samples: vec![], by_column: vec![vec![]], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] }], ctes: vec![], catalog: Default::default() };
        let cx = Cx { schema: &schema, levels: vec![vec![0]], min_level: 0, budget: 0, grouped: None };
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0); INSERT INTO t0 VALUES (NULL)").unwrap();
        let (mut calls, mut values) = (0, 0);
        for seed in 0..2000u64 {
            let mut g = Source::new(seed, 50);
            let v = draw_date(&mut g);
            assert!(!matches!(&v, Value::Text(s) if s.to_ascii_lowercase().contains("now")), "{v:?}");
            let e = draw_date_call(&mut g, &cx, 4);
            if !matches!(e, Expr::Call(f, _) if f.date()) || e.refers(0) {
                continue;
            }
            assert!(date_args_ok_deep(&e), "{e}");
            let sql = format!("SELECT {e}");
            let one = || db.query_row(&sql, [], |r| r.get::<_, rusqlite::types::Value>(0)).unwrap();
            let first = one();
            assert_eq!(first, one(), "{sql}");
            calls += 1;
            values += usize::from(first != rusqlite::types::Value::Null);
        }
        assert!(calls > 300 && values * 2 > calls, "{values} of {calls} calls not NULL");
        // Formats of bounded output; every expression index SQLite builds on
        // time values of every form, and writes after.
        assert!(format_ok(PRINTF_ALL) && format_ok("%-99.99f") && !format_ok("%*d") && !format_ok("%100d"));
        let mut g = Source::new(1, 50);
        for (k, key) in KeyExpr::ALL.iter().enumerate() {
            let sql = format!("CREATE INDEX i{k} ON t0({}); INSERT INTO t0 VALUES ({})", Sql(&key_expr(*key, 0), At::TOP), literal(&draw_date(&mut g)));
            db.execute_batch(&sql).unwrap();
        }
        for _ in 0..300 {
            db.execute_batch(&format!("INSERT INTO t0 VALUES ({})", literal(&draw_date(&mut g)))).unwrap();
        }
        // The modifier chains run to the end on a date.
        for key in [KeyExpr::Julianday, KeyExpr::Time] {
            let sql = format!("SELECT {} FROM (SELECT '2024-01-15 12:30:00' AS c0)", Sql(&key_expr(key, 0), At::TOP));
            assert!(db.query_row(&sql, [], |r| r.get::<_, rusqlite::types::Value>(0)).unwrap() != rusqlite::types::Value::Null, "{sql}");
        }
        let ok: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(ok, "ok");
    }

    /// Every date and time call in `e` is valid ([`date_args_ok`]).
    fn date_args_ok_deep(e: &Expr) -> bool {
        let mut ok = true;
        e.walk(&mut |n| {
            if let Expr::Call(f, args) = n {
                ok &= !f.date() || date_args_ok(*f, args);
            }
        });
        ok
    }

    #[test]
    fn ordered_aggregates_render_one_order() {
        let (x, y) = (Expr::col(0, 0), Expr::col(0, 1));
        let term = |e: &Expr| Order { key: OrderKey::Expr(e.clone()), desc: false, nulls: None, by: By::Number };
        let agg = |func, distinct, args: Vec<Expr>, keys: &[Expr]| Ordered { func, distinct, args, order: keys.iter().map(term).collect() };
        let text = |e: &Expr| Expr::Cast(bx(e.clone()), CastType::Text);
        let comma = Expr::Lit(Value::Text(",".into()));
        let ok = agg(OrdFunc::GroupConcat, false, vec![x.clone(), comma.clone()], &[y.clone(), text(&x)]);
        assert!(ordered_ok(&ok));
        assert_eq!(Expr::Ordered(Box::new(ok)).to_string(), "group_concat(c0, ',' ORDER BY a0.c1, CAST(a0.c0 AS TEXT))");
        // Without the tie term, `1` and `1.0` (tied on c1) come in either order.
        assert!(!ordered_ok(&agg(OrdFunc::GroupConcat, false, vec![x.clone()], std::slice::from_ref(&y))));
        assert!(!ordered_ok(&agg(OrdFunc::StringAgg, false, vec![x.clone(), y.clone()], &[text(&x)])));
        // DISTINCT: one argument, a cast, ordered by itself alone.
        assert!(!ordered_ok(&agg(OrdFunc::GroupConcat, true, vec![x.clone(), comma], &[text(&x)])));
        assert!(!ordered_ok(&agg(OrdFunc::GroupConcat, true, vec![x.clone()], &[text(&x)])));
        assert!(ordered_ok(&agg(OrdFunc::GroupConcat, true, vec![text(&x)], &[text(&text(&x))])));
        assert!(!ordered_ok(&agg(OrdFunc::GroupConcat, true, vec![text(&x)], &[y.clone(), text(&text(&x))])));
        assert!(ordered_ok(&agg(OrdFunc::Count, true, vec![x.clone()], std::slice::from_ref(&y))));
        // No JSONB bytes, no BLOB as a JSON value.
        let jsonb = Expr::Call(Func::JsonbArray, vec![x.clone()]);
        assert!(!ordered_ok(&agg(OrdFunc::JsonGroupArray, false, vec![jsonb.clone()], &Ordered::ties(OrdFunc::JsonGroupArray, &[jsonb]))));
        assert!(!ordered_ok(&agg(OrdFunc::JsonGroupArray, false, vec![x.clone()], &Ordered::ties(OrdFunc::JsonGroupArray, std::slice::from_ref(&x)))));
        // Drawn ones: the same on rows in two orders and two plans, rows that
        // compare equal but render apart included.
        let rows = "(1, 'x'), (1.0, 'x'), (-0.0, 'y'), (0, 'y'), (NULL, 'z'), (x'61', 'a'), ('a', 'a'), ('2024-01-15', 1), (1705276800, 1), (1705276800.0, NULL)";
        let open = |reverse: bool| {
            let db = rusqlite::Connection::open_in_memory().unwrap();
            db.execute_batch("CREATE TABLE t0(c0, c1)").unwrap();
            let mut values: Vec<&str> = rows.split("), (").collect();
            if reverse {
                values.reverse();
                db.execute_batch("CREATE INDEX i0 ON t0(c1, c0)").unwrap();
            }
            let values: Vec<String> = values.iter().map(|v| format!("({})", v.trim_matches(['(', ')']))).collect();
            db.execute_batch(&format!("INSERT INTO t0 VALUES {}", values.join(", "))).unwrap();
            db
        };
        let (a, b) = (open(false), open(true));
        let info = TableInfo { columns: 2, rows: 10, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info], ctes: vec![], catalog: Default::default() };
        let cx = Cx { schema: &schema, levels: vec![vec![0]], min_level: 0, budget: 0, grouped: Some(vec![]) };
        let mut ordered = 0;
        for seed in 0..600u64 {
            let e = draw_ordered(&mut Source::new(seed, 50), &cx, 3);
            let Expr::Ordered(o) = &e else { continue };
            assert!(ordered_ok(o), "{e}");
            for group in ["", " GROUP BY c1"] {
                let sql = format!("SELECT {e} FROM t0 AS a0{group} ORDER BY 1");
                // Values as their bytes: a blob cast to text need not be UTF-8.
                let run = |db: &rusqlite::Connection| -> Vec<String> {
                    let mut stmt = db.prepare(&sql).unwrap();
                    stmt.query_map([], |r| Ok(format!("{:?}", r.get_ref(0)?))).unwrap().map(|v| v.unwrap()).collect()
                };
                // JSON refused (a JSON value from a row): the same error.
                match (a.prepare(&sql).map(|_| ()), run_ok(&a, &sql)) {
                    (Ok(()), true) => assert_eq!(run(&a), run(&b), "{sql}"),
                    _ => assert!(!run_ok(&b, &sql), "{sql}"),
                }
            }
            ordered += 1;
        }
        assert!(ordered > 300);
    }

    /// The collating sequence of an expression ([`expr_coll`], which
    /// ORDER BY sorts it under) and the one a compound takes from it as an
    /// item ([`member_coll`]) are SQLite's: an explicit COLLATE reaches the
    /// expressions around it through operands and arguments (a LIKE's
    /// pattern before its subject; `IN (q)`'s left operand; a `CASE`'s
    /// conditions), but not out of an ordered aggregate's ORDER BY, a
    /// window's PARTITION BY or ORDER BY, or a subquery; an INTEGER PRIMARY
    /// KEY has none (its declared one is ignored), so a compound takes its
    /// column's from a later member.
    #[test]
    fn collating_sequences_are_sqlites() {
        use crate::model::{compare_coll, Encoding};
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE t0(c0, c1, c2); INSERT INTO t0 VALUES ('B', 'x', 1), ('a', 'y', 2);
             CREATE TABLE t1(c0 INTEGER PRIMARY KEY COLLATE NOCASE, c1 COLLATE NOCASE, c2); INSERT INTO t1 VALUES (1, 'B', 1), (2, 'a', 2);",
        )
        .unwrap();
        let info = |coll| TableInfo { columns: 3, rows: 2, samples: vec![], by_column: vec![vec![]; 3], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll, aff: vec![] };
        let schema = Schema { tables: vec![info(vec![Some(Coll::Binary); 3]), info(vec![None, Some(Coll::Nocase), Some(Coll::Binary)])], ctes: vec![], catalog: Default::default() };
        let (c0, c1, c2) = (Expr::col(0, 0), Expr::col(0, 1), Expr::col(0, 2));
        let under = |e: &Expr, k| Expr::Collate(bx(e.clone()), k);
        let term = |e: &Expr| Order { key: OrderKey::Expr(e.clone()), desc: false, nulls: None, by: By::Number };
        let comma = Expr::Lit(Value::Text(",".into()));
        let concat = |args: Vec<Expr>, keys: &[Expr]| Expr::Ordered(Box::new(Ordered { func: OrdFunc::GroupConcat, distinct: false, args, order: keys.iter().map(term).collect() }));
        let window = |arg: &Expr, partition: Vec<Expr>, keys: &[Expr]| {
            Expr::Window(Box::new(Window { func: WinFunc::Max, arg: Some(arg.clone()), default: None, filter: None, partition, order: keys.iter().map(term).collect(), frame: None, named: Named::Inline }))
        };
        let xs = Query::simple(0, false, Items::Exprs(vec![Expr::Lit(Value::Text("x".into()))]), None);
        let nocase = Some(Coll::Nocase);
        // (table, item, grouped by c2, the item's sequence, the one a compound takes from it).
        let cases: Vec<(usize, Expr, bool, Coll, Option<Coll>)> = vec![
            (0, concat(vec![c0.clone(), comma.clone()], &[under(&c1, Coll::Nocase)]), true, Coll::Binary, None),
            (0, concat(vec![under(&c0, Coll::Nocase), comma], std::slice::from_ref(&c1)), true, Coll::Nocase, nocase),
            (0, window(&c0, vec![], &[under(&c1, Coll::Nocase)]), false, Coll::Binary, None),
            (0, window(&c0, vec![under(&c1, Coll::Nocase)], &[]), false, Coll::Binary, None),
            (0, window(&under(&c0, Coll::Nocase), vec![c2.clone()], &[]), false, Coll::Nocase, nocase),
            (0, Expr::Case(None, vec![(Expr::IsNull(bx(Expr::InSelect(bx(under(&c1, Coll::Nocase)), Box::new(xs), false)), true), c0.clone())], None), false, Coll::Nocase, nocase),
            (0, Expr::Case(None, vec![(Expr::bin(BinOp::Eq, under(&c1, Coll::Nocase), c1.clone()), c0.clone())], None), false, Coll::Nocase, nocase),
            (0, Expr::bin(BinOp::Concat, Expr::bin(BinOp::Like, under(&c0, Coll::Rtrim), under(&c0, Coll::Nocase)), c0.clone()), false, Coll::Nocase, nocase),
            (0, Expr::bin(BinOp::Concat, Expr::bin(BinOp::NotGlob, under(&c0, Coll::Nocase), under(&c0, Coll::Rtrim)), c0.clone()), false, Coll::Rtrim, Some(Coll::Rtrim)),
            (0, Expr::bin(BinOp::Concat, c0.clone(), c1.clone()), false, Coll::Binary, None),
            (1, c0.clone(), false, Coll::Binary, None),
            (1, Expr::Cast(bx(c0.clone()), CastType::Text), false, Coll::Binary, None),
            (1, c1.clone(), false, Coll::Nocase, nocase),
        ];
        let rows = |sql: &str| -> Vec<Value> {
            let mut stmt = db.prepare(sql).unwrap();
            let rows = stmt.query_map([], |r| {
                Ok(match r.get_ref(0)? {
                    rusqlite::types::ValueRef::Integer(i) => Value::Int(i),
                    rusqlite::types::ValueRef::Text(t) => Value::Text(String::from_utf8(t.to_vec()).unwrap()),
                    v => panic!("{v:?}"),
                })
            });
            rows.unwrap().map(Result::unwrap).collect()
        };
        for (t, e, grouped, coll, member) in cases {
            let q = Query { group: if grouped { vec![c2.clone()] } else { vec![] }, ..Query::simple(t, false, Items::Exprs(vec![e.clone()]), None) };
            let scope = [vec![t]];
            assert_eq!((expr_coll(&e, &schema, &scope), member_coll(&e, &schema, &scope, 0)), (coll, Some(member)), "{}", q.sql());
            let sorted = rows(&format!("{} ORDER BY 1", q.sql()));
            assert!(sorted.windows(2).all(|w| compare_coll(&w[0], &w[1], Encoding::Utf8, coll).is_le()), "{} sorts as {sorted:?}", q.sql());
            // Its first row against texts that NOCASE or RTRIM take for it
            // (with one under a sequence of its own, which a compound takes
            // where no member before has one): one row for each class of
            // values equal under the compound's sequence.
            let first = Query { filter: Some(Expr::bin(BinOp::Eq, c2.clone(), Expr::Lit(Value::Int(1)))), ..q.clone() };
            let v = rows(&first.sql()).remove(0);
            let s = match &v {
                Value::Text(s) => s.clone(),
                _ => "B".to_string(),
            };
            let probes = [
                vec![(s.to_ascii_lowercase(), None)],
                vec![(format!("{s} "), None)],
                vec![(s.clone(), None), (s.to_ascii_lowercase(), Some(Coll::Nocase))],
                vec![(s.to_ascii_lowercase(), None), (format!("{s} "), Some(Coll::Rtrim))],
                vec![("B".to_string(), None), ("b".to_string(), Some(Coll::Nocase))],
                vec![("b".to_string(), None), ("b ".to_string(), Some(Coll::Rtrim))],
            ];
            for probe in probes {
                let k = member.or(probe.iter().find_map(|p| p.1)).unwrap_or(Coll::Binary);
                let mut values = vec![v.clone()];
                values.extend(probe.iter().map(|p| Value::Text(p.0.clone())));
                let classes = values.iter().enumerate().filter(|(i, x)| values[..*i].iter().all(|y| compare_coll(x, y, Encoding::Utf8, k).is_ne())).count();
                let members: Vec<String> =
                    probe.iter().map(|(p, c)| format!(" UNION SELECT {}", c.map_or(literal(&Value::Text(p.clone())), |c| format!("({} COLLATE {})", literal(&Value::Text(p.clone())), c.name())))).collect();
                let sql = format!("SELECT count(*) FROM ({}{})", first.sql(), members.concat());
                assert_eq!(rows(&sql), [Value::Int(classes as i64)], "{sql} (under {k:?})");
            }
        }
    }

    /// The texts [`converted_lit`] draws for a value a column of a numeric
    /// affinity holds compare equal with it on SQLite: integers with their
    /// sign written (`'+5'`, `'-5'`), `'5.0'` only where the real is the
    /// integer, reals in Rust's shortest text, which `sqlite3AtoF` reads
    /// back exactly (subnormals, the largest finite, 1e300, and 3000
    /// random ones).
    #[test]
    fn converted_literals_compare_equal_with_the_value() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let mut s: u64 = 7;
        let mut next = || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            s ^ (s >> 29)
        };
        let mut values: Vec<Value> = [0, 5, -5, 300, -300, 1 << 53, -(1 << 53), (1 << 53) + 1, -(1 << 53) - 1, i64::MAX, i64::MIN].map(Value::Int).to_vec();
        let reals = [0.5, -2.25, 0.1, -0.0, 1e300, -1e300, 5e-324, -5e-324, f64::MAX, f64::MIN, f64::MIN_POSITIVE, 1e16, 123_456.789, 1e-7];
        values.extend(reals.map(Value::Real));
        values.extend((0..3000).map(|_| f64::from_bits(next())).filter(|x| x.is_finite()).map(Value::Real));
        for (decl, aff) in [("INTEGER", Affinity::Integer), ("NUMERIC", Affinity::Numeric), ("REAL", Affinity::Real)] {
            db.execute_batch(&format!("DROP TABLE IF EXISTS t; CREATE TABLE t(c {decl})")).unwrap();
            for v in &values {
                db.execute_batch(&format!("DELETE FROM t; INSERT INTO t VALUES ({})", literal(v))).unwrap();
                // The value as the column holds it.
                let held = db
                    .query_row("SELECT c FROM t", [], |r| {
                        Ok(match r.get_ref(0)? {
                            rusqlite::types::ValueRef::Integer(i) => Value::Int(i),
                            rusqlite::types::ValueRef::Real(x) => Value::Real(x),
                            v => panic!("{v:?}"),
                        })
                    })
                    .unwrap();
                let mut seen = Vec::new();
                for seed in 0..32 {
                    let lit = converted_lit(&mut Source::new(seed, 0), held.clone(), aff);
                    if seen.contains(&lit) {
                        continue;
                    }
                    let sql = format!("SELECT c = {} FROM t", literal(&lit));
                    assert_eq!(db.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(), 1, "{sql}, c holding {held:?} under {decl}");
                    seen.push(lit);
                }
            }
        }
    }

    /// Whether `sql` runs on `db` to the end.
    fn run_ok(db: &rusqlite::Connection, sql: &str) -> bool {
        db.prepare(sql).and_then(|mut s| s.query_map([], |_| Ok(())).map(|r| r.collect::<Result<Vec<_>, _>>().is_ok())).unwrap_or(false)
    }

    /// No JSONB bytes reach anything but a JSON function ([`jsonb_read`]).
    /// On SQLite one `jsonb_set` of a per-row constant has two encodings in
    /// one statement, by its JSON parse cache: after four `json_valid`
    /// parses evict the cached parse of `json_patch`'s value, it is parsed
    /// again from text (the INT `16` for the JSON5 INT5 `0x10`). `length()`
    /// shows it; `json()` does not. The rule rejects the runner's
    /// `lower(jsonb_replace(json_patch(..), ..))` (run 20261012-0317,
    /// tlp-where, w0-261227) and takes it under `json()`; drawn queries
    /// keep their `jsonb` calls, always inside a JSON function.
    #[test]
    fn jsonb_values_are_read_by_json_functions_only() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0); INSERT INTO t0 VALUES (NULL), (0), (1), (2), (3), (NULL);").unwrap();
        let set = "jsonb_set(json_patch('{a:0x10}', '{}'), '$.b', c0)";
        let col = |sql: &str| -> Vec<String> {
            let mut st = db.prepare(sql).unwrap();
            st.query_map([], |r| r.get::<_, rusqlite::types::Value>(0).map(|v| format!("{v:?}"))).unwrap().map(Result::unwrap).collect()
        };
        let lengths = col(&format!("SELECT length({set}) FROM t0 WHERE json_valid(c0) IS NULL"));
        assert_eq!(lengths, ["Integer(11)", "Integer(9)"], "SQLite may encode JSONB alike now: the rule may go");
        assert_eq!(col(&format!("SELECT length({set}) FROM t0 WHERE c0 IS NULL")), ["Integer(11)", "Integer(11)"]);
        let texts = col(&format!("SELECT json({set}) FROM t0 WHERE json_valid(c0) IS NULL"));
        assert_eq!(texts[0], texts[1]);
        let text = |s: &str| Expr::Lit(Value::Text(s.into()));
        let patch = Expr::Call(Func::JsonPatch, vec![text("{$_b:[0x10,],c:'d'}"), text("{\"c\":\"\\n\",\"x y\":\"a b\"}")]);
        let typeof_c0 = Expr::Call(Func::Typeof, vec![Expr::col(0, 0)]);
        let replace = Expr::Call(Func::JsonbReplace, vec![patch, Expr::Lit(Value::Null), typeof_c0]);
        let info = TableInfo { columns: 1, rows: 6, samples: vec![], by_column: vec![vec![]], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let schema = Schema { tables: vec![info], ctes: vec![], catalog: Default::default() };
        let item = |e: Expr| Query::simple(0, false, Items::Exprs(vec![e]), None).valid(&schema);
        let json = |e: Expr| Expr::Call(Func::Json, vec![e]);
        for bad in [
            Expr::Call(Func::Lower, vec![replace.clone()]),
            Expr::Call(Func::Length, vec![replace.clone()]),
            replace.clone(),
            Expr::bin(BinOp::Concat, replace.clone(), text("a")),
            Expr::Cast(bx(replace.clone()), CastType::Text),
            Expr::Call(Func::Ifnull, vec![replace.clone(), text("a")]),
            Expr::Call(Func::Lower, vec![Expr::Call(Func::Coalesce, vec![replace.clone(), text("a")])]),
        ] {
            assert!(!item(bad.clone()), "{bad}");
        }
        for good in [
            json(replace.clone()),
            Expr::Call(Func::Lower, vec![json(replace.clone())]),
            Expr::Call(Func::JsonArray, vec![replace.clone()]),
            Expr::Call(Func::JsonValid, vec![Expr::Call(Func::Ifnull, vec![replace.clone(), text("a")])]),
            Expr::bin(BinOp::Arrow2, replace.clone(), text("$.c")),
        ] {
            assert!(item(good.clone()), "{good}");
        }
        let (mut drawn, mut jsonb) = (0, 0);
        for seed in 0..400u64 {
            let mut g = Source::new(seed, (seed % 101) as usize);
            let (schema, qs) = queries().generate(&mut g);
            for q in &qs {
                assert!(q.valid(&schema), "seed {seed}: {q:?}");
                drawn += 1;
                let sql = q.sql();
                jsonb += usize::from(sql.contains("jsonb"));
            }
        }
        assert!(jsonb * 50 > drawn, "{jsonb} of {drawn} drawn queries call a jsonb function");
    }

    #[test]
    fn generated_queries_are_valid_and_shrink_and_mutate_so() {
        for seed in 0..500u64 {
            let mut g = Source::new(seed, (seed % 101) as usize);
            let (schema, qs) = queries().generate(&mut g);
            assert!(scoped_valid(&(schema.clone(), qs.clone())), "seed {seed}: {qs:?}");
            let gen = Queries(schema.clone());
            let here = queries_cost(&qs);
            let _ = gen.shrink(&qs, &mut |c| {
                assert!(c.iter().all(|q| q.valid(&schema)), "shrink of seed {seed}: {c:?}");
                // Lower, and different as SQL too (two values of one query
                // would look like a shrinker returning its input).
                assert!(queries_cost(&c) < here && format!("{c:?}") != format!("{qs:?}"), "shrink of seed {seed}: {c:?}");
                ControlFlow::Continue(())
            });
            for _ in 0..4 {
                let m = gen.mutate(&qs, &mut g);
                assert!(m.iter().all(|q| q.valid(&schema)), "mutant of seed {seed}: {m:?}");
            }
        }
    }

    /// Drawn windows read their frames within budget ([`Window::frames_fit`],
    /// as [`Query::fits`] checks): over a table of 5000 rows the
    /// concatenating aggregates are still drawn, over narrow frames, and
    /// without a key the aggregates over the whole partition give way to
    /// ranking; over 50 rows frames of the whole partition are drawn.
    #[test]
    fn windows_read_their_frames_within_budget() {
        let table = |rows, key| TableInfo { columns: 2, rows, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        for (rows, key) in [(5000, RowKey::Rowid), (5000, RowKey::None), (50, RowKey::Rowid)] {
            let schema = Schema { tables: vec![table(rows, key.clone())], ctes: vec![], catalog: Default::default() };
            let cx = Cx { schema: &schema, levels: vec![vec![0]], min_level: 0, budget: 0, grouped: None };
            let (mut whole, mut concat, mut framed) = (0, 0, 0);
            for seed in 0..3000 {
                let Expr::Window(w) = draw_window(&mut Source::new(seed, 50), &cx) else { panic!("a window") };
                let q = Query::simple(0, false, Items::Exprs(vec![Expr::Window(w.clone())]), None);
                assert!(q.valid(&schema) && q.fits(&schema), "{}", q.sql());
                framed += usize::from(w.func.framed());
                whole += usize::from(w.func.framed() && w.frame_rows(true).is_none());
                concat += usize::from(matches!(w.func, WinFunc::GroupConcat(_) | WinFunc::JsonGroupArray));
            }
            assert_eq!(framed > 1000, key == RowKey::Rowid, "{rows} rows, {key:?}: {framed} windows reading frames");
            assert_eq!(whole > 0, rows == 50, "{rows} rows, {key:?}: {whole} over the whole partition");
            assert_eq!(concat > 100, key == RowKey::Rowid, "{rows} rows, {key:?}: {concat} concatenating");
        }
        let w = |frame| Window { func: WinFunc::Sum, arg: None, default: None, filter: None, partition: vec![], order: vec![Order { key: OrderKey::Expr(Expr::col(0, ROWID)), desc: false, nulls: None, by: By::Number }], frame, named: Named::Inline };
        let f = |mode, start, end| Some(Frame { mode, start, end, exclude: None, short: false });
        assert_eq!(w(None).frame_rows(true), None);
        assert_eq!(w(f(FrameMode::Rows, Bound::Preceding(2), Bound::Following(3))).frame_rows(false), Some(6));
        assert_eq!(w(f(FrameMode::Rows, Bound::Following(3), Bound::Following(1))).frame_rows(false), Some(0));
        assert_eq!(w(f(FrameMode::Range, Bound::Preceding(100), Bound::CurrentRow)).frame_rows(true), Some(101));
        assert_eq!(w(f(FrameMode::Range, Bound::Preceding(100), Bound::CurrentRow)).frame_rows(false), None);
        assert_eq!(w(f(FrameMode::Groups, Bound::Preceding(1), Bound::CurrentRow)).frame_rows(true), None);
        assert_eq!(w(f(FrameMode::Rows, Bound::UnboundedPreceding, Bound::CurrentRow)).frame_rows(true), None);
    }

    /// Every virtual table form reads rows on SQLite as its CTE says:
    /// its width, its rows within the estimate (and some), under each
    /// argument form alike; its cheaper forms cheaper and still valid.
    #[test]
    fn virtual_tables_read_as_ctes() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE t0(c0 INT UNIQUE, c1 TEXT COLLATE NOCASE, c2); CREATE INDEX t0_i0 ON t0(c1, c0 DESC); CREATE INDEX t0_i1 ON t0(lower(c1)) WHERE c0 > 1;
             CREATE TABLE t1(c0 INTEGER PRIMARY KEY, c1 REFERENCES t0(c0)); INSERT INTO t0 VALUES (1, 'a', 2), (3, 'B', x'00');",
        )
        .unwrap();
        let info = |columns, rows| TableInfo { columns, rows, samples: vec![], by_column: vec![vec![]; columns], shared: vec![], key: RowKey::Rowid, exprs: vec![], partial: vec![], leading: vec![], coll: vec![], aff: vec![] };
        let catalog = vec![Catalog { types: vec!["INT", "TEXT", ""], indexes: vec![2, 1, 1], fks: vec![], pages: 0 }, Catalog { types: vec!["INTEGER", ""], indexes: vec![], fks: vec![1], pages: 0 }];
        let schema = Schema { tables: vec![info(3, 2), info(2, 0)], ctes: vec![], catalog: catalog.into() };
        let doc = r#"{"a":[1,2,{"b":null}],"c":"x"}"#.to_string();
        let mut forms = vec![Vtab::IndexInfo { table: 0, xinfo: false }, Vtab::IndexInfo { table: 0, xinfo: true }, Vtab::TableList];
        forms.extend((0..MAIN_SCHEMA_FORMS).map(|form| Vtab::SqliteSchema { form }));
        forms.extend((0..PRAGMAS.len()).map(|k| Vtab::Pragma { k, table: None }));
        forms.extend((0..PRAGMAS.len()).filter(|k| PRAGMAS[*k].2).map(|k| Vtab::Pragma { k, table: Some(1) }));
        for arg in [VtabArg::Call, VtabArg::Schema, VtabArg::Hidden] {
            forms.extend([
                Vtab::TableInfo { table: 0, xinfo: false, arg },
                Vtab::TableInfo { table: 0, xinfo: true, arg },
                Vtab::IndexList { table: 0, arg },
                Vtab::ForeignKeys { table: 1, arg },
                Vtab::Json { tree: false, doc: doc.clone(), path: None, arg },
                Vtab::Json { tree: true, doc: doc.clone(), path: Some("$.a".into()), arg },
                Vtab::Dbstat { aggregate: false, arg },
                Vtab::Dbstat { aggregate: true, arg },
            ]);
        }
        for v in &forms {
            let cte = Cte { body: CteBody::Vtab(v.clone()), hint: None, columns: false };
            let t = cte.info(&schema).unwrap_or_else(|| panic!("{v:?}"));
            let q = Query { with: vec![cte.clone()], ..Query::simple(CTE, false, Items::Star, None) };
            assert!(q.valid(&schema), "{}", q.sql());
            for sql in [q.sql(), q.sql_inline()] {
                let mut st = db.prepare(&sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
                assert_eq!(st.column_count(), t.columns, "{sql}");
                let rows = st.query_map([], |_| Ok(())).unwrap().count();
                assert!(rows > 0 && rows <= t.rows.max(1), "{sql}: {rows} rows, {} estimated", t.rows);
            }
            for s in shrink_vtab(v) {
                assert!(s.cost() < v.cost());
                assert!(Cte { body: CteBody::Vtab(s), ..cte.clone() }.info(&schema).is_some());
            }
        }
        // A table that is gone: no such CTE.
        let gone = Cte { body: CteBody::Vtab(Vtab::TableInfo { table: 2, xinfo: false, arg: VtabArg::Call }), hint: None, columns: false };
        assert!(gone.info(&schema).is_none());
    }

    /// ORDER BY a result column by number, alias or expression orders
    /// alike, in a compound too (each member's own column), and a GROUP BY
    /// term that is an item is written by its alias when the items are
    /// named; an item SQLite would read otherwise is written by number.
    #[test]
    fn order_by_names_result_columns_alike() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 'b'), (2, 'a'), (3, 'a'); CREATE TABLE t1(c0, c1); INSERT INTO t1 VALUES (5, 'c'), (0, 'z');").unwrap();
        let rows = |sql: &str| db.prepare(sql).unwrap_or_else(|e| panic!("{sql}: {e}")).query_map([], |r| Ok(format!("{:?}{:?}", r.get_ref(0)?, r.get_ref(1)?))).unwrap().map(|r| r.unwrap()).collect::<Vec<_>>();
        let lower = Expr::Call(Func::Lower, vec![Expr::col(0, 1)]);
        let items = Items::Exprs(vec![Expr::bin(BinOp::Add, Expr::col(0, 0), Expr::Lit(Value::Int(1))), lower.clone()]);
        let member = Query::simple(1, false, items.clone(), None);
        let mut answers = Vec::new();
        for by in [By::Number, By::Alias, By::Expr] {
            let mut q = Query::simple(0, false, items.clone(), None);
            q.compound.push((SetOp::UnionAll, member.clone()));
            q.order = vec![Order { key: OrderKey::Column(1), desc: true, nulls: None, by }, Order { key: OrderKey::Column(0), desc: false, nulls: None, by }];
            let sql = q.sql();
            match by {
                By::Number => assert!(sql.ends_with("ORDER BY 2 DESC, 1"), "{sql}"),
                By::Alias => assert!(sql.contains("AS r1") && sql.ends_with("ORDER BY r1 DESC, r0"), "{sql}"),
                By::Expr => assert!(sql.ends_with("ORDER BY lower(a0.c1) DESC, (a0.c0 + 1)"), "{sql}"),
            }
            answers.push(rows(&sql));
        }
        assert_eq!(answers[0], answers[1]);
        assert_eq!(answers[0], answers[2]);
        // In a compound never an item with a COLLATE (the SQLite bug of
        // bugs/sqlite/sqlite-compound-orderby-collate-groupby): its number.
        let collated = Items::Exprs(vec![Expr::Collate(bx(Expr::col(0, 1)), Coll::Nocase), Expr::col(0, 0)]);
        let mut q = Query::simple(0, false, collated.clone(), None);
        q.compound.push((SetOp::Union, Query::simple(1, false, collated, None)));
        q.order = vec![Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Expr }, Order { key: OrderKey::Column(1), desc: false, nulls: None, by: By::Expr }];
        assert!(q.sql().ends_with("ORDER BY 1, a0.c0"), "{}", q.sql());
        // Its expression in a plain query.
        let mut plain = Query::simple(0, false, items.clone(), None);
        plain.order = vec![Order { key: OrderKey::Column(1), desc: true, nulls: None, by: By::Expr }, Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Expr }];
        assert!(plain.sql().ends_with("ORDER BY lower(a0.c1) DESC, (a0.c0 + 1)"), "{}", plain.sql());
        plain.order.iter_mut().for_each(|o| o.by = By::Number);
        assert_eq!(rows(&plain.sql()), rows(&{ let mut p = plain.clone(); p.order.iter_mut().for_each(|o| o.by = By::Expr); p.sql() }));
        // GROUP BY by alias.
        let mut g = Query::simple(0, false, Items::Exprs(vec![lower.clone(), Expr::Agg(Agg::Count, None)]), None);
        g.group = vec![lower];
        g.order = vec![Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Alias }, Order { key: OrderKey::Column(1), desc: false, nulls: None, by: By::Expr }];
        let sql = g.sql();
        assert!(sql.contains("GROUP BY r0") && sql.ends_with("ORDER BY r0, 2"), "{sql}");
        assert_eq!(rows(&sql), ["Text([97])Integer(2)", "Text([98])Integer(1)"]);
        // A literal item: by number.
        let lit = Query { order: vec![Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Expr }], ..Query::simple(0, false, Items::Exprs(vec![Expr::Lit(Value::Int(2))]), None) };
        assert!(lit.sql().ends_with("ORDER BY 1"), "{}", lit.sql());
    }
}
