// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! A small typed SQL statement language over the tables of a [`Model`]
//! (`t{id}`, columns `c{id}`, indexes `t{t}_i{id}`), rendered by `Display`
//! as SQL that PostgreSQL runs as written. Every name is an id, so a
//! statement renders without the model.
//!
//! Version 0:
//!
//! - `SELECT <c.. | *> FROM t [WHERE p] [ORDER BY c [DESC] [NULLS FIRST |
//!   LAST], ..] [LIMIT n]`, `SELECT count(*) FROM t [WHERE p]`
//! - `INSERT INTO t (c..) VALUES (..), ..` (`DEFAULT VALUES` with no
//!   column), `UPDATE t SET c = v, .. [WHERE p]`, `DELETE FROM t [WHERE p]`,
//!   `TRUNCATE t`
//! - `CREATE TABLE`, `DROP TABLE`, `CREATE [UNIQUE] INDEX` (B-tree, hash,
//!   BRIN; partial, expression, INCLUDE, NULLS NOT DISTINCT, storage
//!   parameters), `DROP INDEX`, `ALTER TABLE .. ADD COLUMN` (with a
//!   default: PostgreSQL's fast default, no rewrite), `.. DROP COLUMN`,
//!   `.. ALTER COLUMN .. SET | DROP NOT NULL`, `.. SET | DROP DEFAULT`
//! - `VACUUM [(FULL, FREEZE, ANALYZE)] t, ..`, `ANALYZE t, ..`, `CLUSTER t USING
//!   i`, `REINDEX [CONCURRENTLY] TABLE t | INDEX i`, `CHECKPOINT`
//! - planner and executor settings ([`Setting`]): `SET enable_seqscan =
//!   off`, `SET work_mem = ..`, ..
//! - crafted statistics ([`Stat`]): `pg_restore_relation_stats` and
//!   `pg_restore_attribute_stats` with drawn numbers
//! - `BEGIN; SAVEPOINT s0` (savepoint 0, which opens the transaction),
//!   `SAVEPOINT s{k}`, `ROLLBACK TO s{k}`, `RELEASE s{k}` (`COMMIT` for
//!   savepoint 0), `ROLLBACK`
//!
//! Predicates compare one column with a literal (`=`, `<>`, `<`, `<=`, `>`,
//! `>=`, `BETWEEN`), test `IS [NOT] NULL`, and combine with `AND`, `OR`,
//! `NOT`. A literal is quoted text that PostgreSQL reads as a value of the
//! column's type (an "unknown" literal takes the type of what it meets),
//! or, cast, of another type comparable with it ([`Lit`]). Which statements
//! are valid against a model, and what they do, is [`crate::semantics`].

use std::fmt;

use crate::{
    fk::Code,
    model::{Column, Deferral, ForeignKey, Index, Method, PartBound, Strategy, Table, TrigAction, Trigger, Type, Value},
};

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Select(Select),
    /// `SELECT count(*) FROM t [WHERE p]`.
    Count { table: u32, filter: Option<Pred> },
    Insert(Insert),
    Update(Update),
    Delete(Delete),
    /// `TRUNCATE t`: every row gone (transactionally); the identity
    /// sequence stays where it is.
    Truncate(u32),
    /// `CREATE TABLE t{id}(..)`: a new empty table (see [`create_table`]).
    CreateTable(Table),
    /// `DROP TABLE t{id}`, with its indexes.
    DropTable(u32),
    /// `CREATE INDEX t{table}_i{id} ON t{table} ..`.
    CreateIndex { table: u32, def: Index },
    /// `DROP INDEX t{table}_i{index}`.
    DropIndex { table: u32, index: u32 },
    /// `ALTER TABLE t ADD COLUMN c{id} type [NOT NULL] [DEFAULT v]`: every
    /// row reads the default (or NULL) in it.
    AddColumn { table: u32, column: Column },
    /// `ALTER TABLE t DROP COLUMN c{id}`: the indexes and CHECK constraints
    /// that read it go with it.
    DropColumn { table: u32, column: u32 },
    /// `ALTER TABLE t ALTER COLUMN c SET NOT NULL` (`DROP NOT NULL` when
    /// false).
    SetNotNull { table: u32, column: u32, on: bool },
    /// `ALTER TABLE t ALTER COLUMN c SET DEFAULT v` (`DROP DEFAULT` with
    /// none): only later INSERTs change.
    SetDefault { table: u32, column: u32, value: Option<Value> },
    /// `VACUUM [(FULL, FREEZE, ANALYZE)] t, ..`: outside a transaction only;
    /// nothing logical. Always of named tables: a database-wide VACUUM also
    /// processes the shared catalogs, where sessions of other databases
    /// meet.
    Vacuum { tables: Vec<u32>, full: bool, freeze: bool, analyze: bool },
    /// `ANALYZE t, ..`: statistics in `pg_statistic`; nothing logical.
    /// Always of named tables, as VACUUM.
    Analyze(Vec<u32>),
    /// `CLUSTER t USING i`: the table rewritten in the order of a B-tree
    /// index that is not partial; nothing logical.
    Cluster { table: u32, index: u32 },
    /// `REINDEX [CONCURRENTLY] TABLE t | INDEX i`: indexes rebuilt
    /// (CONCURRENTLY only outside a transaction); nothing logical.
    Reindex { table: u32, index: Option<u32>, concurrently: bool },
    /// `CHECKPOINT`; nothing logical.
    Checkpoint,
    /// `SET name = value` for the session; nothing logical.
    Set(Setting),
    /// Crafted planner statistics; nothing logical (see [`Stat`]).
    Stats(Vec<Stat>),
    /// `SAVEPOINT s{k}` inside the `k` open ones; savepoint 0 opens the
    /// transaction (`BEGIN; SAVEPOINT s0`).
    Savepoint(usize),
    /// `ROLLBACK TO s{k}`: back to the state at savepoint `k`, which stays
    /// open; the savepoints inside it are gone.
    RollbackTo(usize),
    /// `RELEASE s{k}`: savepoint `k` and those inside it are gone; for `k`
    /// 0 the transaction commits (`COMMIT`).
    Release(usize),
    /// `ROLLBACK`: the transaction is rolled back, every savepoint gone.
    Rollback,
    /// A change of how table `t{table}` is stored or estimated; nothing
    /// logical (see [`Phys`]).
    Phys { table: u32, op: Phys },
    /// `ALTER TABLE t ADD CONSTRAINT t{t}_fk FOREIGN KEY .. [NOT VALID]`
    /// (see [`crate::fk`]).
    AddForeignKey { table: u32, fk: ForeignKey, not_valid: bool },
    /// `ALTER TABLE t DROP CONSTRAINT t{t}_fk`.
    DropForeignKey(u32),
    /// `ALTER TABLE t VALIDATE CONSTRAINT t{t}_fk`.
    ValidateForeignKey(u32),
    /// `ALTER TABLE t ALTER CONSTRAINT t{t}_fk [NOT] DEFERRABLE
    /// [INITIALLY DEFERRED | IMMEDIATE]`.
    AlterForeignKey { table: u32, deferral: Deferral },
    /// `SET CONSTRAINTS ALL | t{t}_fk DEFERRED | IMMEDIATE`.
    SetConstraints { table: Option<u32>, deferred: bool },
    /// A statement PostgreSQL must refuse with `code`, run where the
    /// refusal is caught: `DO $$BEGIN s; RAISE EXCEPTION ..; EXCEPTION WHEN
    /// <code> THEN NULL; END$$` (the block's subtransaction undoes it; if
    /// it succeeds, or fails otherwise, the block fails).
    Expect(Code, Box<Statement>),
    /// `CREATE OR REPLACE FUNCTION t{t}_g{k}_f() ..; CREATE TRIGGER
    /// t{t}_g{k} ..` (see [`create_trigger`]).
    CreateTrigger { table: u32, trigger: Trigger },
    /// `DROP TRIGGER t{t}_g{k} ON t{t}`.
    DropTrigger { table: u32, id: u32 },
    /// `ALTER TABLE t {ENABLE | DISABLE} TRIGGER t{t}_g{k}`.
    EnableTrigger { table: u32, id: u32, on: bool },
    /// `INSERT .. ON CONFLICT ..` (see [`OnConflict`]).
    Upsert(Insert, OnConflict),
    /// A write with `RETURNING` (see [`Returning`]): an INSERT, UPSERT,
    /// UPDATE, DELETE or MERGE.
    Returning(Box<Statement>, Returning),
    /// `MERGE INTO t AS tg USING (VALUES ..) AS s(c..) ON .. WHEN ..` (see
    /// [`Merge`]).
    Merge(Merge),
    /// `UPDATE t AS tg SET .. FROM (src) AS s(c..) WHERE tg.k = s.k [AND
    /// cond]` or `DELETE FROM t AS tg USING (src) AS s(c..) WHERE ..`: a
    /// [`Merge`] of one MATCHED clause (UPDATE or DELETE), each target row
    /// joined to one source row at most ([`crate::semantics::merge_write`]
    /// refuses two, where PostgreSQL would pick either).
    JoinWrite(Merge),
    /// `INSERT INTO t (c..) SELECT c.., FROM t2 [WHERE p]`: the other
    /// table's (or its own) rows as the statement found them.
    InsertSelect { table: u32, columns: Vec<u32>, from: FromTable },
}

/// `MERGE INTO t{table} AS tg USING (VALUES (..), ..) AS s(c{c}, ..) ON
/// tg.c{k} = s.c{k} AND .. WHEN ..`: the source rows give values of some of
/// the target's columns (named as them, each value cast to the column's
/// type), joined on equality of the `on` columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Merge {
    pub table: u32,
    /// The source's columns, distinct target columns, with their types
    /// (for the casts).
    pub columns: Vec<(u32, Type)>,
    /// At least one row, one value per column (none with `from`).
    pub rows: Vec<Vec<Value>>,
    /// The source as a query of another table's rows instead of VALUES:
    /// `(SELECT c{s}, .. FROM t{t} [WHERE p]) AS s(c..)`, one source
    /// column of the column's type per target column.
    pub from: Option<FromTable>,
    /// The join columns (among `columns`), at least one.
    pub on: Vec<u32>,
    /// The WHEN clauses in order.
    pub whens: Vec<When>,
}

/// Rows of table `t{table}`: `SELECT c{c}, .. FROM t{table} [WHERE p]`.
#[derive(Clone, Debug, PartialEq)]
pub struct FromTable {
    pub table: u32,
    pub columns: Vec<u32>,
    pub filter: Option<Pred>,
}

impl fmt::Display for FromTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SELECT {} FROM t{}", list(self.columns.iter().map(|c| format!("c{c}"))), self.table)?;
        where_clause(f, &self.filter)
    }
}

/// `WHEN MATCHED | NOT MATCHED [BY TARGET] | NOT MATCHED BY SOURCE [AND
/// cond] THEN action`. The condition of MATCHED and BY SOURCE reads the
/// target row (`tg.c`), of NOT MATCHED the source row (`s.c`, its columns
/// only).
#[derive(Clone, Debug, PartialEq)]
pub struct When {
    pub kind: MergeKind,
    pub cond: Option<Pred>,
    pub action: MergeAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    Matched,
    NotMatched,
    BySource,
}

/// An action: `UPDATE SET c = v | s.c, ..` (no source value BY SOURCE),
/// `DELETE`, `INSERT (c..) VALUES (s.c..)` of every source column (NOT
/// MATCHED only), `DO NOTHING`.
#[derive(Clone, Debug, PartialEq)]
pub enum MergeAction {
    Update(Vec<(u32, SetVal)>),
    Delete,
    Insert,
    Nothing,
}

/// `ON CONFLICT [target] DO NOTHING | DO UPDATE SET c = v, .. [WHERE p]`.
#[derive(Clone, Debug, PartialEq)]
pub struct OnConflict {
    /// None: every unique constraint is an arbiter (DO NOTHING only).
    pub target: Option<Target>,
    /// DO UPDATE: the sets and the condition on the existing row (written
    /// `t{t}.c{c}`); None: DO NOTHING.
    pub update: Option<(Vec<(u32, SetVal)>, Option<Pred>)>,
}

/// The arbiter of an ON CONFLICT: `(c, ..)`, inferring every unique
/// constraint on exactly those columns, or `ON CONSTRAINT t{t}_pkey`.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Columns(Vec<u32>),
    Key,
}

/// A value DO UPDATE sets: a constant, or `EXCLUDED.c{c}` (the proposed
/// row's value, after BEFORE INSERT triggers).
#[derive(Clone, Debug, PartialEq)]
pub enum SetVal {
    Const(Value),
    Excluded(u32),
}

/// `RETURNING` items: `c{c}` (the new row, or the old one of a DELETE),
/// `old.c{c}` or `new.c{c}` (NULL where there is none), each aliased
/// `r{k}`; as the statement itself, or as a data-modifying CTE read by the
/// main query (`WITH w AS (.. RETURNING ..) SELECT * FROM w`), with the
/// target's rows as the statement found them when `snapshot` (`.. UNION ALL
/// SELECT c.. FROM t`: the main query sees the table as it was before the
/// CTE wrote it).
#[derive(Clone, Debug, PartialEq)]
pub struct Returning {
    pub items: Vec<(Which, u32)>,
    pub cte: bool,
    pub snapshot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Plain,
    Old,
    New,
}

impl Statement {
    /// The table a write writes (an INSERT, UPSERT, UPDATE or DELETE).
    pub fn write_table(&self) -> Option<u32> {
        match self {
            Statement::Insert(i) | Statement::Upsert(i, _) => Some(i.table),
            Statement::Update(u) => Some(u.table),
            Statement::Delete(d) => Some(d.table),
            Statement::Merge(mg) | Statement::JoinWrite(mg) => Some(mg.table),
            Statement::InsertSelect { table, .. } => Some(*table),
            _ => None,
        }
    }
}

/// A predicate over a trigger's row: `NEW.c{c}` (or `OLD.`) for `c{c}`.
pub fn pred_sql(p: &Pred, row: &str) -> String {
    match p {
        Pred::Cmp(c, op, v) => format!("{row}.c{c} {} {v}", op.sql()),
        Pred::IsNull(c, false) => format!("{row}.c{c} IS NULL"),
        Pred::IsNull(c, true) => format!("{row}.c{c} IS NOT NULL"),
        Pred::Between(c, lo, hi) => format!("{row}.c{c} BETWEEN {lo} AND {hi}"),
        Pred::And(a, b) => format!("({} AND {})", pred_sql(a, row), pred_sql(b, row)),
        Pred::Or(a, b) => format!("({} OR {})", pred_sql(a, row), pred_sql(b, row)),
        Pred::Not(a) => format!("NOT ({})", pred_sql(a, row)),
    }
}

/// The function and the trigger of table `t{table}`'s trigger `tr` (its
/// column types from `t`, for the casts of constants).
pub fn create_trigger(t: &Table, tr: &Trigger) -> String {
    let table = t.id;
    let name = tr.name(table);
    let row = if tr.events[2] { "OLD" } else { "NEW" };
    let body = match &tr.action {
        TrigAction::Set(c, v) => {
            let ty = t.column(*c).map_or("text", |c| c.ty.name());
            format!("NEW.c{c} := {}::{ty}; RETURN NEW;", literal(v))
        },
        TrigAction::Skip => "RETURN NULL;".to_string(),
        TrigAction::Audit { target, cols } => {
            let dst = list(cols.iter().map(|(_, d)| format!("c{d}")));
            if tr.row {
                format!("INSERT INTO t{target} ({dst}) VALUES ({}); RETURN NULL;", list(cols.iter().map(|(s, _)| format!("{row}.c{s}"))))
            } else {
                format!("INSERT INTO t{target} ({dst}) SELECT {} FROM nt; RETURN NULL;", list(cols.iter().map(|(s, _)| format!("c{s}"))))
            }
        },
    };
    let mut events = Vec::new();
    if tr.events[0] {
        events.push("INSERT".to_string());
    }
    if tr.events[1] {
        events.push(if tr.update_of.is_empty() { "UPDATE".to_string() } else { format!("UPDATE OF {}", list(tr.update_of.iter().map(|c| format!("c{c}")))) });
    }
    if tr.events[2] {
        events.push("DELETE".to_string());
    }
    let mut s = format!(
        "CREATE OR REPLACE FUNCTION {name}_f() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN {body} END$$; CREATE TRIGGER {name} {} {} ON t{table}",
        if tr.before { "BEFORE" } else { "AFTER" },
        events.join(" OR ")
    );
    if !tr.row {
        s.push_str(if tr.events[2] { " REFERENCING OLD TABLE AS nt" } else { " REFERENCING NEW TABLE AS nt" });
    }
    s.push_str(if tr.row { " FOR EACH ROW" } else { " FOR EACH STATEMENT" });
    if let Some(p) = &tr.when {
        s.push_str(&format!(" WHEN ({})", pred_sql(p, row)));
    }
    s.push_str(&format!(" EXECUTE FUNCTION {name}_f()"));
    if !tr.enabled {
        s.push_str(&format!("; ALTER TABLE t{table} DISABLE TRIGGER {name}"));
    }
    s
}

/// Storage and estimation changes of a table, none of which changes a row.
#[derive(Clone, Debug, PartialEq)]
pub enum Phys {
    /// `ALTER TABLE t SET (fillfactor = k)`, `RESET (fillfactor)` with
    /// none: pages written later keep free space. Not of a partitioned
    /// table (no storage of its own).
    Fillfactor(Option<u8>),
    /// `ALTER TABLE t SET LOGGED | UNLOGGED`: the table and its indexes
    /// rewritten. Not of a partitioned table, nor of one with an identity
    /// column (its sequence is rewritten too, so a rollback takes back
    /// the values drawn since).
    Logged(bool),
    /// `ALTER TABLE t ALTER COLUMN c SET STATISTICS k`: what ANALYZE keeps.
    Target(u32, i16),
    /// `ALTER TABLE t ALTER COLUMN c SET (n_distinct = x)`, `x` in
    /// hundredths: the distinct count ANALYZE records.
    NDistinct(u32, i32),
    /// `CREATE STATISTICS IF NOT EXISTS t{t}_x{k} [(kinds)] ON c, .. FROM
    /// t`: kinds of `ndistinct`, `dependencies`, `mcv` (none listed: all),
    /// two columns or more, distinct.
    CreateStats(u8, [bool; 3], Vec<u32>),
    /// `DROP STATISTICS IF EXISTS t{t}_x{k}`.
    DropStats(u8),
    /// `ALTER TABLE t CLUSTER ON i` (a B-tree index that is not partial),
    /// `SET WITHOUT CLUSTER` with none. Not of a partitioned table.
    ClusterOn(Option<u32>),
}

impl Phys {
    /// Whether it applies to the table only when the table is not
    /// partitioned.
    pub fn storage(&self) -> bool {
        matches!(self, Phys::Fillfactor(_) | Phys::Logged(_) | Phys::ClusterOn(_))
    }

    pub fn sql(&self, t: u32) -> String {
        match self {
            Phys::Fillfactor(Some(k)) => format!("ALTER TABLE t{t} SET (fillfactor = {k})"),
            Phys::Fillfactor(None) => format!("ALTER TABLE t{t} RESET (fillfactor)"),
            Phys::Logged(on) => format!("ALTER TABLE t{t} SET {}", if *on { "LOGGED" } else { "UNLOGGED" }),
            Phys::Target(c, k) => format!("ALTER TABLE t{t} ALTER COLUMN c{c} SET STATISTICS {k}"),
            Phys::NDistinct(c, x) => format!("ALTER TABLE t{t} ALTER COLUMN c{c} SET (n_distinct = {})", f64::from(*x) / 100.0),
            Phys::CreateStats(k, kinds, cols) => {
                let names: Vec<&str> = ["ndistinct", "dependencies", "mcv"].iter().zip(kinds).filter(|(_, on)| **on).map(|(n, _)| *n).collect();
                let kinds = if names.is_empty() { String::new() } else { format!(" ({})", names.join(", ")) };
                format!("CREATE STATISTICS IF NOT EXISTS t{t}_x{k}{kinds} ON {} FROM t{t}", list(cols.iter().map(|c| format!("c{c}"))))
            },
            Phys::DropStats(k) => format!("DROP STATISTICS IF EXISTS t{t}_x{k}"),
            Phys::ClusterOn(Some(i)) => format!("ALTER TABLE t{t} CLUSTER ON t{t}_i{i}"),
            Phys::ClusterOn(None) => format!("ALTER TABLE t{t} SET WITHOUT CLUSTER"),
        }
    }

    pub fn cost(&self) -> u64 {
        match self {
            Phys::CreateStats(_, kinds, cols) => 1 + cols.len() as u64 + kinds.iter().filter(|k| **k).count() as u64,
            _ => 1,
        }
    }
}

/// A planner or executor setting for the session: what plan a query gets,
/// never its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// `enable_<x> = on | off`.
    Enable(Enable, bool),
    /// `work_mem` in KiB (64 is the least): sorts and hashes spill.
    WorkMem(u32),
    /// `random_page_cost`, in tenths.
    RandomPageCost(u16),
    /// `cpu_tuple_cost`, in thousandths.
    CpuTupleCost(u16),
    /// `jit = on | off`, with every cost threshold 0 when on (JIT
    /// compilation of every query's expressions, if the server has LLVM).
    Jit(bool),
    /// `debug_parallel_query = on | off`, with parallel setup and tuple
    /// costs 0: parallel plans where the planner can make them.
    Parallel(bool),
}

/// The planner's `enable_*` switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enable {
    SeqScan,
    IndexScan,
    IndexOnlyScan,
    BitmapScan,
    TidScan,
    Sort,
    IncrementalSort,
    HashAgg,
    Material,
    Memoize,
    NestLoop,
    MergeJoin,
    HashJoin,
    GatherMerge,
    PartitionWise,
}

impl Enable {
    pub const ALL: [Enable; 15] = [
        Enable::SeqScan,
        Enable::IndexScan,
        Enable::IndexOnlyScan,
        Enable::BitmapScan,
        Enable::TidScan,
        Enable::Sort,
        Enable::IncrementalSort,
        Enable::HashAgg,
        Enable::Material,
        Enable::Memoize,
        Enable::NestLoop,
        Enable::MergeJoin,
        Enable::HashJoin,
        Enable::GatherMerge,
        Enable::PartitionWise,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Enable::SeqScan => "enable_seqscan",
            Enable::IndexScan => "enable_indexscan",
            Enable::IndexOnlyScan => "enable_indexonlyscan",
            Enable::BitmapScan => "enable_bitmapscan",
            Enable::TidScan => "enable_tidscan",
            Enable::Sort => "enable_sort",
            Enable::IncrementalSort => "enable_incremental_sort",
            Enable::HashAgg => "enable_hashagg",
            Enable::Material => "enable_material",
            Enable::Memoize => "enable_memoize",
            Enable::NestLoop => "enable_nestloop",
            Enable::MergeJoin => "enable_mergejoin",
            Enable::HashJoin => "enable_hashjoin",
            Enable::GatherMerge => "enable_gathermerge",
            Enable::PartitionWise => "enable_partitionwise_join",
        }
    }
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let on = |b: bool| if b { "on" } else { "off" };
        match self {
            Setting::Enable(e, b) => write!(f, "SET {} = {}", e.name(), on(*b)),
            Setting::WorkMem(k) => write!(f, "SET work_mem = '{k}kB'"),
            Setting::RandomPageCost(t) => write!(f, "SET random_page_cost = {}.{}", t / 10, t % 10),
            Setting::CpuTupleCost(t) => write!(f, "SET cpu_tuple_cost = {}.{:03}", t / 1000, t % 1000),
            Setting::Jit(true) => write!(f, "SET jit = on; SET jit_above_cost = 0; SET jit_inline_above_cost = 0; SET jit_optimize_above_cost = 0"),
            Setting::Jit(false) => write!(f, "SET jit = off"),
            Setting::Parallel(true) => write!(f, "SET debug_parallel_query = on; SET parallel_setup_cost = 0; SET parallel_tuple_cost = 0; SET min_parallel_table_scan_size = 0"),
            Setting::Parallel(false) => write!(f, "SET debug_parallel_query = off"),
        }
    }
}

/// Crafted statistics of one table: the planner's costs (row estimates,
/// index choice, join order and method) come from these, and no result
/// depends on them.
#[derive(Clone, Debug, PartialEq)]
pub enum Stat {
    /// `pg_restore_relation_stats(.., 'relpages', p, 'reltuples', n,
    /// 'relallvisible', v)`: the table's size as the planner sees it.
    Relation { table: u32, pages: u32, tuples: f32, all_visible: u32 },
    /// `pg_restore_attribute_stats(.., 'attname', 'c{id}', 'inherited',
    /// false, 'null_frac', f, 'avg_width', w, 'n_distinct', d)`: one
    /// column's.
    Attribute { table: u32, column: u32, null_frac: f32, width: u32, distinct: f32 },
}

impl Stat {
    pub fn table(&self) -> u32 {
        match self {
            Stat::Relation { table, .. } | Stat::Attribute { table, .. } => *table,
        }
    }
}

/// A real as SQL reads it, for statistics (finite by construction).
fn real(x: f32) -> String {
    format!("'{}'::real", x)
}

impl fmt::Display for Stat {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Stat::Relation { table, pages, tuples, all_visible } => write!(
                f,
                "SELECT pg_restore_relation_stats('schemaname', current_schema()::text, 'relname', 't{table}', 'relpages', {pages}::integer, 'reltuples', {}, 'relallvisible', {all_visible}::integer)",
                real(*tuples)
            ),
            Stat::Attribute { table, column, null_frac, width, distinct } => write!(
                f,
                "SELECT pg_restore_attribute_stats('schemaname', current_schema()::text, 'relname', 't{table}', 'attname', 'c{column}', 'inherited', false, 'null_frac', {}, 'avg_width', {width}::integer, 'n_distinct', {})",
                real(*null_frac),
                real(*distinct)
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Select {
    pub table: u32,
    /// The selected columns, in order (repeats allowed); empty for `*`.
    pub columns: Vec<u32>,
    pub filter: Option<Pred>,
    /// `ORDER BY` terms.
    pub order: Vec<OrderTerm>,
    pub limit: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderTerm {
    pub column: u32,
    pub desc: bool,
    /// `NULLS FIRST` (true) or `NULLS LAST` (false) when given.
    pub nulls_first: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Insert {
    pub table: u32,
    /// Distinct columns; none for `DEFAULT VALUES` (then one row, empty).
    pub columns: Vec<u32>,
    /// At least one row, each one value per column.
    pub rows: Vec<Vec<Value>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub table: u32,
    /// Distinct columns, at least one.
    pub sets: Vec<(u32, Value)>,
    pub filter: Option<Pred>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Delete {
    pub table: u32,
    pub filter: Option<Pred>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub const ALL: [CmpOp; 6] = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];

    pub fn sql(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
}

/// A literal in a predicate: a value of the column's type, rendered as
/// quoted text that takes the column's type, or (with `cast`) a value of
/// another type PostgreSQL compares with the column's, rendered with its
/// cast (`'5'::int8` against an `int2` column: a cross-type operator of the
/// same B-tree family, which an index can still use).
#[derive(Clone, Debug, PartialEq)]
pub struct Lit {
    pub value: Value,
    pub cast: Option<Type>,
}

impl Lit {
    pub fn plain(value: Value) -> Self {
        Lit { value, cast: None }
    }

    pub fn cost(&self) -> u64 {
        self.value.cost() + u64::from(self.cast.is_some())
    }
}

impl fmt::Display for Lit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", literal(&self.value))?;
        match self.cast {
            Some(t) => write!(f, "::{}", t.name()),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pred {
    /// `c op v`.
    Cmp(u32, CmpOp, Lit),
    /// `c IS NULL`, or `c IS NOT NULL` when `true`.
    IsNull(u32, bool),
    /// `c BETWEEN lo AND hi`.
    Between(u32, Lit, Lit),
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
    Not(Box<Pred>),
}

impl Pred {
    /// Nesting depth: 1 for a leaf.
    pub fn depth(&self) -> usize {
        match self {
            Pred::Cmp(..) | Pred::IsNull(..) | Pred::Between(..) => 1,
            Pred::And(a, b) | Pred::Or(a, b) => 1 + a.depth().max(b.depth()),
            Pred::Not(a) => 1 + a.depth(),
        }
    }

    /// The columns of the leaves, in order.
    pub fn leaves(&self, out: &mut Vec<u32>) {
        match self {
            Pred::Cmp(c, ..) | Pred::IsNull(c, _) | Pred::Between(c, ..) => out.push(*c),
            Pred::And(a, b) | Pred::Or(a, b) => {
                a.leaves(out);
                b.leaves(out);
            },
            Pred::Not(a) => a.leaves(out),
        }
    }

    pub fn reads(&self, column: u32) -> bool {
        let mut cols = Vec::new();
        self.leaves(&mut cols);
        cols.contains(&column)
    }

    pub fn cost(&self) -> u64 {
        match self {
            Pred::Cmp(_, op, v) => 2 + u64::from(*op != CmpOp::Eq) + v.cost(),
            Pred::IsNull(_, not) => 1 + u64::from(*not),
            Pred::Between(_, lo, hi) => 3 + lo.cost() + hi.cost(),
            Pred::And(a, b) | Pred::Or(a, b) => 1 + a.cost() + b.cost(),
            Pred::Not(a) => 1 + a.cost(),
        }
    }
}

/// A literal PostgreSQL reads back as exactly `v` in a column (or cast) of
/// its type: the value's input text ([`Value::text`]) in single quotes,
/// quotes doubled (`standard_conforming_strings` is on, so a backslash is
/// itself). NULL is `NULL`.
pub fn literal(v: &Value) -> String {
    match v.text() {
        None => "NULL".into(),
        Some(s) => format!("'{}'", s.replace('\'', "''")),
    }
}

impl fmt::Display for Pred {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Pred::Cmp(c, op, v) => write!(f, "c{c} {} {v}", op.sql()),
            Pred::IsNull(c, false) => write!(f, "c{c} IS NULL"),
            Pred::IsNull(c, true) => write!(f, "c{c} IS NOT NULL"),
            Pred::Between(c, lo, hi) => write!(f, "c{c} BETWEEN {lo} AND {hi}"),
            Pred::And(a, b) => write!(f, "({a} AND {b})"),
            Pred::Or(a, b) => write!(f, "({a} OR {b})"),
            Pred::Not(a) => write!(f, "NOT ({a})"),
        }
    }
}

fn where_clause(f: &mut fmt::Formatter, p: &Option<Pred>) -> fmt::Result {
    match p {
        Some(p) => write!(f, " WHERE {p}"),
        None => Ok(()),
    }
}

fn list(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}

/// A column's definition in `CREATE TABLE` or `ADD COLUMN`.
pub fn column_def(c: &Column) -> String {
    let mut s = format!("c{} {}", c.id, c.ty.name());
    // STORAGE and COMPRESSION go before the constraints.
    if let Some(st) = c.storage {
        s.push_str(&format!(" STORAGE {}", st.name()));
    }
    if let Some(cm) = c.compression {
        s.push_str(&format!(" COMPRESSION {}", cm.name()));
    }
    if c.identity {
        s.push_str(" GENERATED BY DEFAULT AS IDENTITY");
    }
    if c.not_null && !c.identity {
        s.push_str(" NOT NULL");
    }
    if let Some(v) = &c.default {
        s.push_str(&format!(" DEFAULT {}::{}", literal(v), c.ty.name()));
    }
    s
}

/// `CREATE TABLE t{id}(..)` for a table's columns, key and CHECK
/// constraints (not its indexes or rows), with storage parameters when
/// given (`WITH (..)`, nothing logical).
/// An `UPDATE .. FROM` or `DELETE .. USING` as SQL (its one clause's
/// action and condition).
pub fn join_write_sql(mg: &Merge) -> String {
    let t = mg.table;
    let names = list(mg.columns.iter().map(|(c, _)| format!("c{c}")));
    let source = match &mg.from {
        Some(ft) => ft.to_string(),
        None => format!("VALUES {}", list(mg.rows.iter().map(|r| format!("({})", list(r.iter().zip(&mg.columns).map(|(v, (_, ty))| format!("{}::{}", literal(v), ty.name()))))))),
    };
    let mut cond = mg.on.iter().map(|c| format!("tg.c{c} = s.c{c}")).collect::<Vec<_>>().join(" AND ");
    let Some(w) = mg.whens.first() else { return String::new() };
    if let Some(p) = &w.cond {
        cond = format!("{cond} AND {}", pred_sql(p, "tg"));
    }
    match &w.action {
        MergeAction::Update(sets) => {
            let sets = list(sets.iter().map(|(c, v)| match v {
                SetVal::Const(v) => format!("c{c} = {}", literal(v)),
                SetVal::Excluded(e) => format!("c{c} = s.c{e}"),
            }));
            format!("UPDATE t{t} AS tg SET {sets} FROM ({source}) AS s({names}) WHERE {cond}")
        },
        _ => format!("DELETE FROM t{t} AS tg USING ({source}) AS s({names}) WHERE {cond}"),
    }
}

/// A MERGE as SQL.
pub fn merge_sql(mg: &Merge) -> String {
    let t = mg.table;
    let rows = list(mg.rows.iter().map(|r| format!("({})", list(r.iter().zip(&mg.columns).map(|(v, (_, ty))| format!("{}::{}", literal(v), ty.name()))))));
    let names = list(mg.columns.iter().map(|(c, _)| format!("c{c}")));
    let on = mg.on.iter().map(|c| format!("tg.c{c} = s.c{c}")).collect::<Vec<_>>().join(" AND ");
    let source = match &mg.from {
        Some(ft) => ft.to_string(),
        None => format!("VALUES {rows}"),
    };
    let mut s = format!("MERGE INTO t{t} AS tg USING ({source}) AS s({names}) ON {on}");
    for w in &mg.whens {
        let (kind, row) = match w.kind {
            MergeKind::Matched => ("MATCHED", "tg"),
            MergeKind::NotMatched => ("NOT MATCHED", "s"),
            MergeKind::BySource => ("NOT MATCHED BY SOURCE", "tg"),
        };
        s.push_str(&format!(" WHEN {kind}"));
        if let Some(p) = &w.cond {
            s.push_str(&format!(" AND {}", pred_sql(p, row)));
        }
        s.push_str(" THEN ");
        s.push_str(&match &w.action {
            MergeAction::Update(sets) => format!(
                "UPDATE SET {}",
                list(sets.iter().map(|(c, v)| match v {
                    SetVal::Const(v) => format!("c{c} = {}", literal(v)),
                    SetVal::Excluded(e) => format!("c{c} = s.c{e}"),
                }))
            ),
            MergeAction::Delete => "DELETE".into(),
            MergeAction::Insert => format!("INSERT ({names}) VALUES ({})", list(mg.columns.iter().map(|(c, _)| format!("s.c{c}")))),
            MergeAction::Nothing => "DO NOTHING".into(),
        });
    }
    s
}

pub fn create_table(t: &Table, with: &str, unlogged: bool) -> String {
    let mut parts: Vec<String> = t.columns.iter().map(column_def).collect();
    if !t.key.is_empty() {
        parts.push(format!("CONSTRAINT t{}_pkey PRIMARY KEY ({})", t.id, list(t.key.iter().map(|c| format!("c{c}")))));
    }
    for p in &t.checks {
        parts.push(format!("CHECK ({p})"));
    }
    if let Some(fk) = &t.fk {
        parts.push(format!("CONSTRAINT t{}_fk {}", t.id, fk.sql()));
    }
    let with = if with.is_empty() { String::new() } else { format!(" WITH ({with})") };
    let mut s = format!("CREATE {}TABLE t{}({})", if unlogged { "UNLOGGED " } else { "" }, t.id, parts.join(", "));
    let Some(p) = &t.partitioning else {
        return s + &with;
    };
    let strategy = match p.strategy {
        Strategy::Range => "RANGE",
        Strategy::List => "LIST",
        Strategy::Hash => "HASH",
    };
    s.push_str(&format!(" PARTITION BY {strategy} (c{})", p.column));
    let edge = |v: &Option<Value>, none: &str| v.as_ref().map_or(none.to_string(), literal);
    for (k, b) in p.parts.iter().enumerate() {
        let bound = match b {
            PartBound::Range(lo, hi) => format!("FOR VALUES FROM ({}) TO ({})", edge(lo, "MINVALUE"), edge(hi, "MAXVALUE")),
            PartBound::List(vs) => format!("FOR VALUES IN ({})", list(vs.iter().map(literal))),
            PartBound::Hash(m, r) => format!("FOR VALUES WITH (MODULUS {m}, REMAINDER {r})"),
            PartBound::Default => "DEFAULT".into(),
        };
        s.push_str(&format!("; CREATE TABLE t{}_p{k} PARTITION OF t{} {bound}", t.id, t.id));
    }
    s
}

/// `CREATE [UNIQUE] INDEX t{table}_i{id} ON t{table} USING m (..) ..`.
pub fn create_index(table: u32, i: &Index) -> String {
    let keys = list(i.columns.iter().enumerate().map(|(k, col)| {
        let term = match i.expr {
            Some(e) if k == 0 => e.sql(col.column),
            _ => format!("c{}", col.column),
        };
        let mut s = term;
        if col.desc {
            s.push_str(" DESC");
        }
        match col.nulls_first {
            Some(true) => s.push_str(" NULLS FIRST"),
            Some(false) => s.push_str(" NULLS LAST"),
            None => {},
        }
        s
    }));
    let mut s = format!("CREATE {}INDEX t{table}_i{} ON t{table} USING {} ({keys})", if i.unique { "UNIQUE " } else { "" }, i.id, i.method.name());
    if !i.include.is_empty() {
        s.push_str(&format!(" INCLUDE ({})", list(i.include.iter().map(|c| format!("c{c}")))));
    }
    if i.nulls_not_distinct {
        s.push_str(" NULLS NOT DISTINCT");
    }
    let mut with = Vec::new();
    if let Some(ff) = i.params.fillfactor {
        with.push(format!("fillfactor = {ff}"));
    }
    if i.params.no_dedup {
        with.push("deduplicate_items = off".to_string());
    }
    if let Some(p) = i.params.pages_per_range {
        with.push(format!("pages_per_range = {p}"));
    }
    if !with.is_empty() {
        s.push_str(&format!(" WITH ({})", with.join(", ")));
    }
    if let Some(p) = &i.filter {
        s.push_str(&format!(" WHERE {p}"));
    }
    s
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Statement::Select(s) => {
                let what = if s.columns.is_empty() { "*".into() } else { list(s.columns.iter().map(|c| format!("c{c}"))) };
                write!(f, "SELECT {what} FROM t{}", s.table)?;
                where_clause(f, &s.filter)?;
                if !s.order.is_empty() {
                    let by = list(s.order.iter().map(|o| {
                        let nulls = match o.nulls_first {
                            Some(true) => " NULLS FIRST",
                            Some(false) => " NULLS LAST",
                            None => "",
                        };
                        format!("c{}{}{nulls}", o.column, if o.desc { " DESC" } else { "" })
                    }));
                    write!(f, " ORDER BY {by}")?;
                }
                if let Some(n) = s.limit {
                    write!(f, " LIMIT {n}")?;
                }
                Ok(())
            },
            Statement::Count { table, filter } => {
                write!(f, "SELECT count(*) FROM t{table}")?;
                where_clause(f, filter)
            },
            Statement::Insert(s) if s.columns.is_empty() => write!(f, "INSERT INTO t{} DEFAULT VALUES", s.table),
            Statement::Insert(s) => {
                let names = list(s.columns.iter().map(|c| format!("c{c}")));
                let rows = list(s.rows.iter().map(|r| format!("({})", list(r.iter().map(literal)))));
                write!(f, "INSERT INTO t{} ({names}) VALUES {rows}", s.table)
            },
            Statement::Upsert(ins, oc) => {
                write!(f, "{} ON CONFLICT", Statement::Insert(ins.clone()))?;
                match &oc.target {
                    Some(Target::Columns(cols)) => write!(f, " ({})", list(cols.iter().map(|c| format!("c{c}"))))?,
                    Some(Target::Key) => write!(f, " ON CONSTRAINT t{}_pkey", ins.table)?,
                    None => {},
                }
                match &oc.update {
                    None => write!(f, " DO NOTHING"),
                    Some((sets, filter)) => {
                        let sets = list(sets.iter().map(|(c, v)| match v {
                            SetVal::Const(v) => format!("c{c} = {}", literal(v)),
                            SetVal::Excluded(e) => format!("c{c} = EXCLUDED.c{e}"),
                        }));
                        write!(f, " DO UPDATE SET {sets}")?;
                        match filter {
                            Some(p) => write!(f, " WHERE {}", pred_sql(p, &format!("t{}", ins.table))),
                            None => Ok(()),
                        }
                    },
                }
            },
            Statement::Merge(mg) => write!(f, "{}", merge_sql(mg)),
            Statement::JoinWrite(mg) => write!(f, "{}", join_write_sql(mg)),
            Statement::InsertSelect { table, columns, from } => write!(f, "INSERT INTO t{table} ({}) {from}", list(columns.iter().map(|c| format!("c{c}")))),
            Statement::Returning(inner, r) => {
                // A MERGE's target is `tg` (a plain name would be the
                // source's too).
                let tg = if matches!(**inner, Statement::Merge(_) | Statement::JoinWrite(_)) { "tg." } else { "" };
                let items = list(r.items.iter().enumerate().map(|(k, (w, c))| match w {
                    Which::Plain => format!("{tg}c{c} AS r{k}"),
                    Which::Old => format!("old.c{c} AS r{k}"),
                    Which::New => format!("new.c{c} AS r{k}"),
                }));
                if !r.cte {
                    return write!(f, "{inner} RETURNING {items}");
                }
                write!(f, "WITH w AS ({inner} RETURNING {items}) SELECT * FROM w")?;
                match (r.snapshot, inner.write_table()) {
                    (true, Some(t)) => write!(f, " UNION ALL SELECT {} FROM t{t}", list(r.items.iter().map(|(_, c)| format!("c{c}")))),
                    _ => Ok(()),
                }
            },
            Statement::Update(s) => {
                let sets = list(s.sets.iter().map(|(c, v)| format!("c{c} = {}", literal(v))));
                write!(f, "UPDATE t{} SET {sets}", s.table)?;
                where_clause(f, &s.filter)
            },
            Statement::Delete(s) => {
                write!(f, "DELETE FROM t{}", s.table)?;
                where_clause(f, &s.filter)
            },
            Statement::Truncate(t) => write!(f, "TRUNCATE t{t} CASCADE"),
            Statement::CreateTable(t) => write!(f, "{}", create_table(t, "", false)),
            Statement::DropTable(t) => write!(f, "DROP TABLE t{t} CASCADE"),
            Statement::CreateIndex { table, def } => write!(f, "{}", create_index(*table, def)),
            Statement::DropIndex { table, index } => write!(f, "DROP INDEX t{table}_i{index}"),
            Statement::AddColumn { table, column } => write!(f, "ALTER TABLE t{table} ADD COLUMN {}", column_def(column)),
            Statement::DropColumn { table, column } => write!(f, "ALTER TABLE t{table} DROP COLUMN c{column}"),
            Statement::SetNotNull { table, column, on } => {
                write!(f, "ALTER TABLE t{table} ALTER COLUMN c{column} {} NOT NULL", if *on { "SET" } else { "DROP" })
            },
            Statement::SetDefault { table, column, value: Some(v) } => write!(f, "ALTER TABLE t{table} ALTER COLUMN c{column} SET DEFAULT {}", literal(v)),
            Statement::SetDefault { table, column, value: None } => write!(f, "ALTER TABLE t{table} ALTER COLUMN c{column} DROP DEFAULT"),
            Statement::Vacuum { tables, full, freeze, analyze } => {
                write!(f, "VACUUM")?;
                let opts: Vec<&str> = [(*full, "FULL"), (*freeze, "FREEZE"), (*analyze, "ANALYZE")].into_iter().filter(|(on, _)| *on).map(|(_, o)| o).collect();
                if !opts.is_empty() {
                    write!(f, " ({})", opts.join(", "))?;
                }
                write!(f, " {}", list(tables.iter().map(|t| format!("t{t}"))))
            },
            Statement::Analyze(tables) => write!(f, "ANALYZE {}", list(tables.iter().map(|t| format!("t{t}")))),
            Statement::Cluster { table, index } => write!(f, "CLUSTER t{table} USING t{table}_i{index}"),
            Statement::Reindex { table, index, concurrently } => {
                let c = if *concurrently { " CONCURRENTLY" } else { "" };
                match index {
                    Some(i) => write!(f, "REINDEX INDEX{c} t{table}_i{i}"),
                    None => write!(f, "REINDEX TABLE{c} t{table}"),
                }
            },
            Statement::Checkpoint => write!(f, "CHECKPOINT"),
            Statement::Set(s) => write!(f, "{s}"),
            Statement::Stats(rows) => write!(f, "{}", list(rows.iter().map(|r| r.to_string())).replace(", SELECT", "; SELECT")),
            Statement::Savepoint(0) => write!(f, "BEGIN; SAVEPOINT s0"),
            Statement::Savepoint(k) => write!(f, "SAVEPOINT s{k}"),
            Statement::RollbackTo(k) => write!(f, "ROLLBACK TO s{k}"),
            Statement::Release(0) => write!(f, "COMMIT"),
            Statement::Release(k) => write!(f, "RELEASE s{k}"),
            Statement::Rollback => write!(f, "ROLLBACK"),
            Statement::Phys { table, op } => write!(f, "{}", op.sql(*table)),
            Statement::AddForeignKey { table, fk, not_valid } => {
                write!(f, "ALTER TABLE t{table} ADD CONSTRAINT t{table}_fk {}{}", fk.sql(), if *not_valid { " NOT VALID" } else { "" })
            },
            Statement::DropForeignKey(t) => write!(f, "ALTER TABLE t{t} DROP CONSTRAINT t{t}_fk"),
            Statement::ValidateForeignKey(t) => write!(f, "ALTER TABLE t{t} VALIDATE CONSTRAINT t{t}_fk"),
            Statement::AlterForeignKey { table, deferral } => write!(
                f,
                "ALTER TABLE t{table} ALTER CONSTRAINT t{table}_fk {}",
                match deferral {
                    Deferral::No => "NOT DEFERRABLE",
                    Deferral::Immediate => "DEFERRABLE INITIALLY IMMEDIATE",
                    Deferral::Deferred => "DEFERRABLE INITIALLY DEFERRED",
                }
            ),
            Statement::SetConstraints { table, deferred } => write!(
                f,
                "SET CONSTRAINTS {} {}",
                table.map_or("ALL".to_string(), |t| format!("t{t}_fk")),
                if *deferred { "DEFERRED" } else { "IMMEDIATE" }
            ),
            Statement::CreateTrigger { .. } => write!(f, "{}", self.trigger_sql().unwrap()),
            Statement::DropTrigger { table, id } => write!(f, "DROP TRIGGER t{table}_g{id} ON t{table}"),
            Statement::EnableTrigger { table, id, on } => write!(f, "ALTER TABLE t{table} {} TRIGGER t{table}_g{id}", if *on { "ENABLE" } else { "DISABLE" }),
            Statement::Expect(code, s) => write!(f, "DO $$BEGIN {s}; RAISE EXCEPTION 'ephyra: no error'; EXCEPTION WHEN {} THEN NULL; END$$", code.condition()),
        }
    }
}

impl Statement {
    /// A CREATE TRIGGER's text. Its Set constants carry their own cast
    /// (the value's natural type), since the statement has no table.
    fn trigger_sql(&self) -> Option<String> {
        let Statement::CreateTrigger { table, trigger } = self else { return None };
        let mut t = Table::new(*table, &[]);
        if let TrigAction::Set(c, v) = &trigger.action {
            let ty = match v {
                Value::Int(_) => Type::Int8,
                Value::Float(_) => Type::Float8,
                Value::Numeric(_) => Type::Numeric,
                Value::Bool(_) => Type::Bool,
                Value::Bytea(_) => Type::Bytea,
                Value::Date(_) => Type::Date,
                Value::Timestamp(_) => Type::Timestamp,
                _ => Type::Text,
            };
            t.columns.push(Column::new(*c, ty));
        }
        Some(create_trigger(&t, trigger))
    }
}

/// Whether the statement only reads, so that it runs as a query and returns
/// rows to compare.
pub fn returns_rows(s: &Statement) -> bool {
    matches!(s, Statement::Select(_) | Statement::Count { .. } | Statement::Returning(..))
}

/// Whether PostgreSQL refuses to run it inside a transaction block.
pub fn outside_transaction(s: &Statement) -> bool {
    matches!(s, Statement::Vacuum { .. } | Statement::Reindex { concurrently: true, .. })
}

/// Whether the index method may hold a column of this type (with the
/// default operator classes, no extension).
pub fn method_takes(m: Method, ty: Type) -> bool {
    match m {
        Method::Btree | Method::Hash | Method::Gin | Method::Gist => true,
        Method::Brin => ty != Type::Bool,
        Method::SpGist => ty == Type::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Numeric;

    #[test]
    fn renders() {
        let lit = |v| Lit::plain(v);
        let p = Pred::Or(
            Box::new(Pred::And(Box::new(Pred::Cmp(0, CmpOp::Le, lit(Value::Int(-3)))), Box::new(Pred::IsNull(1, true)))),
            Box::new(Pred::Not(Box::new(Pred::Between(2, lit(Value::Text("a'b".into())), Lit { value: Value::Bytea(vec![0, 255]), cast: Some(Type::Bytea) })))),
        );
        let s = Statement::Select(Select {
            table: 1,
            columns: vec![0, 2],
            filter: Some(p),
            order: vec![OrderTerm { column: 1, desc: true, nulls_first: Some(false) }, OrderTerm { column: 0, desc: false, nulls_first: None }],
            limit: Some(4),
        });
        assert_eq!(
            s.to_string(),
            "SELECT c0, c2 FROM t1 WHERE ((c0 <= '-3' AND c1 IS NOT NULL) OR NOT (c2 BETWEEN 'a''b' AND '\\x00ff'::bytea)) ORDER BY c1 DESC NULLS LAST, c0 LIMIT 4"
        );
        let i = Statement::Insert(Insert { table: 0, columns: vec![1, 3], rows: vec![vec![Value::Float(-0.0), Value::Null], vec![Value::Numeric(Numeric::parse("1.50").unwrap()), Value::Date(0)]] });
        assert_eq!(i.to_string(), "INSERT INTO t0 (c1, c3) VALUES ('-0', NULL), ('1.50', '2000-01-01')");
        assert_eq!(Statement::Savepoint(0).to_string(), "BEGIN; SAVEPOINT s0");
        assert_eq!(Statement::Release(0).to_string(), "COMMIT");
    }
}
