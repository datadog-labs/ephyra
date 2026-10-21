//! A small typed SQL statement language over the schema the writer creates
//! (tables `t{k}`, columns `c{k}`, see [`crate::writer::create_table`]),
//! rendered by `Display` as SQL that SQLite runs as written.
//!
//! Version 0:
//!
//! - `SELECT <c.. | *> FROM t [WHERE p] [ORDER BY c [DESC], ..] [LIMIT n]`
//! - `INSERT INTO t ([rowid,] c..) VALUES (..), ..`
//! - `UPDATE t SET c = v, .. [WHERE p]`
//! - `DELETE FROM t [WHERE p]`
//! - `VACUUM`, `PRAGMA incremental_vacuum(n)`
//! - `PRAGMA integrity_check`, `PRAGMA quick_check`, optionally `(t)`
//! - `SAVEPOINT s{k}`, `ROLLBACK TO s{k}`, `RELEASE s{k}`, `ROLLBACK`
//! - `SELECT count(*) FROM t [WHERE p]`
//! - `CREATE TABLE`, `DROP TABLE` (the last table), `CREATE [UNIQUE] INDEX`
//!   (partial and expression indexes included), `DROP INDEX` (a table's last index)
//! - `ANALYZE [t | t_i]`, `PRAGMA analysis_limit = n`
//! - `PRAGMA optimize[(mask)]`, `PRAGMA automatic_index = 0|1`,
//!   `PRAGMA reverse_unordered_selects = 0|1`
//! - storage: `PRAGMA journal_mode = DELETE|TRUNCATE|PERSIST|MEMORY|WAL|OFF`,
//!   `PRAGMA wal_checkpoint[(mode)]`, `PRAGMA auto_vacuum = m; VACUUM`,
//!   `PRAGMA temp_store`, `mmap_size`, `locking_mode`, `secure_delete`,
//!   `cell_size_check`, `cache_spill`, `journal_size_limit`,
//!   `wal_autocheckpoint`, `synchronous`, `shrink_memory`, `VACUUM INTO ':memory:' | ''`, `BEGIN
//!   DEFERRED|IMMEDIATE|EXCLUSIVE` (with `SAVEPOINT s0`), `COMMIT`
//! - crafted statistics: rows of `sqlite_stat1` replaced by drawn ones
//!   (`ANALYZE sqlite_schema; DELETE FROM sqlite_stat1 WHERE ..; INSERT
//!   INTO sqlite_stat1 VALUES (..); .. ANALYZE sqlite_schema`, see [`Stat`])
//!
//! Predicates compare one column with a literal (`=`, `<>`, `<`, `<=`, `>`,
//! `>=`, `BETWEEN`), test `IS [NOT] NULL`, and combine with `AND`, `OR`,
//! `NOT`. Which statements are valid against a model, and what they do, is
//! [`crate::semantics`]; what is left out of the language, and why, is
//! documented there at each restriction.

use std::fmt;

use crate::{
    expr::{key_expr, pred_expr, spell, At, Sql},
    model::{ColType, Coll, Db, ForeignKey, Index, Rule, Step, Table, Term, Value, DECLS},
    writer::{constraint_name, create_table_named, fk_tail, index_term},
};

#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    Select(Select),
    Insert(Insert),
    Update(Update),
    Delete(Delete),
    Vacuum,
    /// `PRAGMA incremental_vacuum(n)`: frees up to `n` pages (all with 0) in
    /// an incremental-vacuum database, nothing in any other.
    IncrementalVacuum(u32),
    /// `PRAGMA integrity_check` (`quick_check` when `quick`), of one table
    /// when given: answers the single row `ok` on a valid database.
    Check { quick: bool, table: Option<usize> },
    /// `SELECT count(*) FROM t [WHERE p]`.
    Count { table: usize, filter: Option<Pred> },
    /// `CREATE TABLE t{table}(..)` (see [`crate::writer::create_table`]):
    /// a new empty table after the others, `table` their number.
    CreateTable { table: usize, def: Table },
    /// `DROP TABLE t{table}`: the last table, with its indexes and triggers.
    DropTable(usize),
    /// `CREATE INDEX t{table}_i{index} ON t{table}(..) [WHERE p]`: a new
    /// index after the table's others, `index` their number; its first
    /// term an expression of its first column for an expression index,
    /// rendered as queries render expressions (`crate::expr::key_expr`,
    /// `crate::expr::pred_expr`).
    /// `db` is the table's schema: an index of a table in `aux` is named
    /// in it (`CREATE INDEX aux.t5_i0 ON t5(..)`: an unqualified index name
    /// puts the index in main, where `t5` is not), one of a TEMP table
    /// half the time (`temp.t5_i0`).
    CreateIndex { table: usize, index: usize, def: Index, db: Db },
    /// `DROP INDEX t{table}_i{index}`: the table's last index.
    /// `db` the table's schema, which a qualified name names.
    DropIndex { table: usize, index: usize, db: Db },
    /// `PRAGMA cache_size = n`: the page cache's size, in pages (KiB when
    /// negative); nothing logical.
    CacheSize(i32),
    /// `ANALYZE`, of one table (`ANALYZE t{t}`) or one index (`ANALYZE
    /// t{t}_i{i}`) when given: statistics in `sqlite_stat1` and
    /// `sqlite_stat4`, which the planner reads to choose indexes and join
    /// orders; nothing logical (the model has no `sqlite_stat` table, and
    /// neither do the queries' schemas).
    Analyze(Option<(usize, Option<usize>)>),
    /// `PRAGMA analysis_limit = n`: the rows ANALYZE reads of each index
    /// (0: all of them), for the statistics of a partial scan.
    AnalysisLimit(u32),
    /// `PRAGMA optimize`, with a mask when given (`0x1`: report what it
    /// would do; `0x2`: ANALYZE the tables that may benefit, those whose
    /// size is ten times off their statistics or with an index without
    /// any; `0x10`: with a small analysis limit; `0x10000`: every table,
    /// not only those queries used since the database opened): ANALYZE
    /// statements of its own; nothing logical.
    Optimize(Option<u32>),
    /// `PRAGMA automatic_index = 0|1`: whether the planner may build a
    /// temporary index for a lookup (on by default); nothing logical.
    AutomaticIndex(bool),
    /// `PRAGMA reverse_unordered_selects = 0|1`: the planner scans tables
    /// and indexes backwards where no ORDER BY asks for an order; nothing
    /// logical.
    ReverseUnordered(bool),
    /// Crafted statistics: each row of `sqlite_stat1` given replaced by
    /// it, then reloaded for the planner (see [`Stat`]); nothing logical.
    Stats(Vec<Stat>),
    /// `SAVEPOINT s{k}`: a savepoint inside the `k` open ones (starting a
    /// transaction when there are none).
    Savepoint(usize),
    /// `ROLLBACK TO s{k}`: back to the state at savepoint `k`, which stays
    /// open; the savepoints inside it are gone.
    RollbackTo(usize),
    /// `RELEASE s{k}`: savepoint `k` and those inside it are gone; with
    /// `k` 0 the transaction commits.
    Release(usize),
    /// `ROLLBACK`: the transaction the outermost savepoint started is
    /// rolled back, and every savepoint is gone.
    Rollback,
    /// `ALTER TABLE t{table} ADD COLUMN c{n} ..` (see [`NewColumn`]): a
    /// column after the others, `n` their number; rows already there read
    /// its default (SQLite does not rewrite them).
    AddColumn { table: usize, column: usize, def: NewColumn },
    /// `ALTER TABLE t{table} DROP COLUMN c{column}`, then every later column
    /// renamed one down (`ALTER TABLE t RENAME COLUMN c{k+1} TO c{k}`, ..),
    /// so that names stay positional: SQLite rewrites each row without the
    /// column, then the indexes, views, triggers and constraints that name
    /// the renamed ones. `columns` is the table's number of columns before.
    DropColumn { table: usize, column: usize, columns: usize },
    /// `ALTER TABLE t{table} RENAME COLUMN c{column} TO r{column}; ALTER
    /// TABLE t{table} RENAME COLUMN r{column} TO c{column}`: a column renamed
    /// and back (names stay positional), SQLite rewriting the indexes,
    /// views, triggers, checks, generated columns and foreign keys that
    /// name it each time; nothing logical.
    RenameColumn { table: usize, column: usize },
    /// `ALTER TABLE t{table} ALTER COLUMN c{column} SET NOT NULL` (`DROP NOT
    /// NULL` when `on` is false).
    NotNull { table: usize, column: usize, on: bool },
    /// `ALTER TABLE t{table} ADD CONSTRAINT k{j} CHECK(p)`, `j` the table's
    /// number of checks.
    AddCheck { table: usize, index: usize, check: Pred },
    /// `ALTER TABLE t{table} DROP CONSTRAINT k{check}`: its last check.
    DropCheck { table: usize, check: usize },
    /// `ALTER TABLE t{table} DROP CONSTRAINT {name}`: the first of the
    /// table's named constraints by that name (`Table::names`), its NOT
    /// NULL or foreign key dropped, or only its name (before COLLATE,
    /// DEFAULT or AS).
    DropConstraint { table: usize, name: String },
    /// Tables `a` and `b` trade names (`ALTER TABLE t{a} RENAME TO t_swap;
    /// ALTER TABLE t{b} RENAME TO t{a}; ALTER TABLE t_swap RENAME TO t{b}`:
    /// SQLite rewrites their indexes, views and triggers each time), then
    /// their indexes, which keep their names through a rename, are dropped
    /// and made again under the names of their table's new position.
    /// `indexes` are those of `a` and of `b` before it.
    SwapTables { a: usize, b: usize, indexes: (Vec<Index>, Vec<Index>) },
    /// `PRAGMA foreign_keys = 0|1`: whether SQLite enforces foreign keys
    /// and runs their actions (on by default in this build; a no-op inside
    /// a transaction, where it is never drawn).
    ForeignKeys(bool),
    /// `PRAGMA foreign_key_check`, of one table when given: the rows
    /// without a parent (none with foreign keys on).
    FkCheck(Option<usize>),
    /// `CREATE TRIGGER w{id} ..` (see [`Rule`]).
    CreateRule(Rule),
    /// `DROP TRIGGER w{id}`.
    DropRule(usize),
    /// `PRAGMA recursive_triggers = 0|1`.
    RecursiveTriggers(bool),
    /// An INSERT, UPDATE or DELETE of view `v{view}` (its table field
    /// unused), which its INSTEAD OF trigger carries out.
    ViewWrite { view: usize, write: Box<Statement> },
    /// `REINDEX [t{t} | t{t}_i{i} | NOCASE | RTRIM | BINARY]` (see
    /// [`Reindex`]): the indexes rebuilt from their tables' rows; nothing
    /// logical.
    Reindex(Reindex),
    /// `PRAGMA journal_mode = ..` (see [`Journal`]), outside a transaction
    /// only (SQLite refuses a change into or out of WAL inside one).
    JournalMode(Journal),
    /// `PRAGMA wal_checkpoint[(mode)]`, outside a transaction (inside a
    /// write transaction it fails, `SQLITE_LOCKED`): one row; nothing
    /// logical (a database not in WAL mode answers `0, -1, -1`). A WAL file
    /// can refuse it with `SQLITE_LOCKED` outside one too, which the
    /// oracles take as run (`crate::sqlancer`'s `run_statement`).
    WalCheckpoint(Option<Checkpoint>),
    /// `PRAGMA secure_delete = 0|1|FAST` (2 for FAST): whether freed cells
    /// and pages are overwritten with zeros; nothing logical.
    SecureDelete(u8),
    /// `PRAGMA cell_size_check = 0|1`: b-tree pages checked as they are
    /// read; nothing logical.
    CellSizeCheck(bool),
    /// `PRAGMA mmap_size = n`: pages read through the VFS's xFetch (memdb
    /// or the file mapped) up to `n` bytes; nothing logical.
    MmapSize(u32),
    /// `PRAGMA auto_vacuum = NONE|FULL|INCREMENTAL; VACUUM` (see
    /// [`AutoVac`]): the database rebuilt with pointer-map pages (or
    /// without), so that later writes move pages (FULL at every commit,
    /// INCREMENTAL at `PRAGMA incremental_vacuum`); as VACUUM for the rows.
    AutoVacuum(AutoVac),
    /// `PRAGMA temp_store = DEFAULT|FILE|MEMORY` (0, 1, 2): where temporary
    /// b-trees (sorts, DISTINCT, materialized views and CTEs) live. Closes
    /// the TEMP database, so never inside a transaction nor with a TEMP
    /// trigger; nothing logical.
    TempStore(u8),
    /// `PRAGMA locking_mode = EXCLUSIVE|NORMAL`; nothing logical.
    LockingMode(bool),
    /// `PRAGMA journal_size_limit = n`: the journal (or WAL) truncated to
    /// `n` bytes after a commit (-1: no limit); nothing logical.
    JournalSizeLimit(i32),
    /// `PRAGMA wal_autocheckpoint = n`, in frames; nothing logical.
    WalAutocheckpoint(u32),
    /// `PRAGMA cache_spill = n`: dirty pages spill to the journal once the
    /// cache holds `n` (0: never spill); nothing logical.
    CacheSpill(u32),
    /// `VACUUM INTO ':memory:'` (`true`) or `VACUUM INTO ''` (a temporary
    /// file): a copy of the database, never read; nothing logical, and
    /// outside a transaction only.
    VacuumInto(bool),
    /// `BEGIN DEFERRED|IMMEDIATE|EXCLUSIVE; SAVEPOINT s0`: a transaction
    /// started by BEGIN, with savepoint 0 inside it, so that `RELEASE s0`
    /// does not end it (never drawn while [`crate::model::Session::begun`])
    /// and COMMIT or ROLLBACK does.
    Begin(Begin),
    /// `COMMIT`: the transaction commits, every savepoint is gone.
    Commit,
    /// `PRAGMA synchronous = OFF|NORMAL|FULL|EXTRA` (0..=3): how the pager
    /// syncs the journal and the database at a commit, outside a
    /// transaction only; nothing logical.
    Synchronous(u8),
    /// `PRAGMA shrink_memory`: the page caches release what memory they
    /// can; nothing logical.
    ShrinkMemory,
    /// `ATTACH [DATABASE] ':memory:' AS aux`: an empty in-memory database
    /// joins the connection (`attachFunc`), outside a transaction only
    /// ("cannot ATTACH database within transaction"). Tables made in it
    /// after are part of the connection, not of the case's file: every
    /// open of the case runs the statements again, this one included.
    /// With `true`, `ATTACH '' AS aux`: a temporary database in a file of
    /// its own (the unix VFS and the pager's file paths, a rollback
    /// journal), deleted when it is detached or the connection closes.
    Attach(bool),
    /// `DETACH [DATABASE] aux`: the attached database and every object in
    /// it gone (`detachFunc`), outside a transaction only; never while a
    /// TEMP trigger is on or writes one of its tables, and only when its
    /// tables are the last ones (names stay positional).
    Detach,
    /// `ANALYZE {db}`: statistics of every index of one schema
    /// (`analyzeDatabase`); nothing logical.
    AnalyzeDb(Db),
    /// `PRAGMA {db}.{name}[(t{table})]`, [`DB_PRAGMAS`]`[k]`: a pragma of
    /// one schema, of one of its tables when given; nothing logical (the
    /// checks answer `ok`).
    DbPragma { db: Db, k: usize, table: Option<usize> },
    /// A statement on virtual table `vt{k}` ([`crate::vtabs`]), `kind`
    /// that table's (its own for a CREATE).
    Virtual { s: crate::vtabs::VStmt, kind: crate::vtabs::Kind },
}

/// The pragmas [`Statement::DbPragma`] runs on one schema, and whether
/// each takes a table (of that schema).
pub const DB_PRAGMAS: [(&str, bool); 10] = [
    ("integrity_check", false),
    ("quick_check", false),
    ("table_list", false),
    ("page_count", false),
    ("schema_version", false),
    ("table_info", true),
    ("table_xinfo", true),
    ("index_list", true),
    ("foreign_key_list", true),
    ("integrity_check", true),
];

/// A journal mode (`PRAGMA journal_mode`). In a deserialized database
/// (the oracles' default storage) WAL is refused and the mode stays (the
/// pragma answers it); DELETE, TRUNCATE and PERSIST keep a rollback
/// journal file in memdb, MEMORY in memory, OFF none: then ROLLBACK and
/// ROLLBACK TO cannot undo the transaction's writes, and are never drawn
/// (`crate::model::Session::journal_off`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Journal {
    Delete,
    Truncate,
    Persist,
    Memory,
    Wal,
    Off,
}

impl Journal {
    pub const ALL: [Journal; 6] = [Journal::Delete, Journal::Truncate, Journal::Persist, Journal::Memory, Journal::Wal, Journal::Off];

    pub fn name(self) -> &'static str {
        match self {
            Journal::Delete => "DELETE",
            Journal::Truncate => "TRUNCATE",
            Journal::Persist => "PERSIST",
            Journal::Memory => "MEMORY",
            Journal::Wal => "WAL",
            Journal::Off => "OFF",
        }
    }
}

/// A `PRAGMA wal_checkpoint` mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checkpoint {
    Passive,
    Full,
    Restart,
    Truncate,
}

impl Checkpoint {
    pub const ALL: [Checkpoint; 4] = [Checkpoint::Passive, Checkpoint::Full, Checkpoint::Restart, Checkpoint::Truncate];

    pub fn name(self) -> &'static str {
        match self {
            Checkpoint::Passive => "PASSIVE",
            Checkpoint::Full => "FULL",
            Checkpoint::Restart => "RESTART",
            Checkpoint::Truncate => "TRUNCATE",
        }
    }
}

/// An auto-vacuum mode (`PRAGMA auto_vacuum`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoVac {
    None,
    Full,
    Incremental,
}

impl AutoVac {
    pub fn name(self) -> &'static str {
        match self {
            AutoVac::None => "NONE",
            AutoVac::Full => "FULL",
            AutoVac::Incremental => "INCREMENTAL",
        }
    }
}

/// How BEGIN locks (`BEGIN DEFERRED|IMMEDIATE|EXCLUSIVE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begin {
    Deferred,
    Immediate,
    Exclusive,
}

impl Begin {
    pub fn name(self) -> &'static str {
        match self {
            Begin::Deferred => "DEFERRED",
            Begin::Immediate => "IMMEDIATE",
            Begin::Exclusive => "EXCLUSIVE",
        }
    }
}

/// What `REINDEX` rebuilds (`sqlite3Reindex`): every index, a table's
/// indexes, one index, or every index with a column under a collating
/// sequence (`reindexDatabases`, `collationMatch`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reindex {
    All,
    Table(usize),
    Index(usize, usize),
    Coll(Coll),
}

/// Pred `p` over the columns of the row a trigger sees, `NEW.c` or `OLD.c`.
pub fn row_pred(p: &Pred, side: &str) -> String {
    let c = |c: &usize| format!("{side}.c{c}");
    match p {
        Pred::Cmp(k, op, v) => format!("{} {} {}", c(k), op.sql(), literal(v)),
        Pred::IsNull(k, false) => format!("{} IS NULL", c(k)),
        Pred::IsNull(k, true) => format!("{} IS NOT NULL", c(k)),
        Pred::Between(k, lo, hi) => format!("{} BETWEEN {} AND {}", c(k), literal(lo), literal(hi)),
        Pred::And(a, b) => format!("({} AND {})", row_pred(a, side), row_pred(b, side)),
        Pred::Or(a, b) => format!("({} OR {})", row_pred(a, side), row_pred(b, side)),
        Pred::Not(a) => format!("NOT ({})", row_pred(a, side)),
    }
}

/// A reference to a column or table `name` in a trigger or an ALTER
/// TABLE, spelled by `key` (the text around it, [`spell`]): mostly bare,
/// else quoted in `"`, `[]` or `` ` `` or in upper case, which SQLite
/// matches alike (and its rename code finds and rewrites alike).
pub fn ident_ref(name: &str, key: &str) -> String {
    match spell(&format!("{key}/{name}"), 9) {
        0 => format!("\"{name}\""),
        1 => format!("[{name}]"),
        2 => format!("`{name}`"),
        3 => name.to_uppercase(),
        4 => format!("\"{}\"", name.to_uppercase()),
        _ => name.to_string(),
    }
}

/// Pred `p` over the columns of table `t{table}` a trigger step writes,
/// each column spelled by `key` and where it is: a third of them
/// qualified by the table's name (`t2.c1`, `"t2".c1`), the others as
/// [`ident_ref`] spells them; SQLite resolves them alike, and its rename
/// code rewrites the table's name in a qualified one (RENAME TO,
/// `renameTableExprCb`) as it does the column's (RENAME COLUMN).
pub fn step_pred(p: &Pred, table: usize, key: &str) -> String {
    fn go(p: &Pred, table: usize, key: &str, n: &mut usize) -> String {
        let mut col = |c: &usize| {
            *n += 1;
            let here = format!("{key}/{n}");
            let name = format!("c{c}");
            match spell(&here, 6) {
                0 => format!("t{table}.{}", ident_ref(&name, &here)),
                1 => format!("{}.{name}", ident_ref(&format!("t{table}"), &format!("{here}t"))),
                _ => ident_ref(&name, &here),
            }
        };
        match p {
            Pred::Cmp(c, op, v) => format!("{} {} {}", col(c), op.sql(), literal(v)),
            Pred::IsNull(c, false) => format!("{} IS NULL", col(c)),
            Pred::IsNull(c, true) => format!("{} IS NOT NULL", col(c)),
            Pred::Between(c, lo, hi) => format!("{} BETWEEN {} AND {}", col(c), literal(lo), literal(hi)),
            Pred::And(a, b) => format!("({} AND {})", go(a, table, key, n), go(b, table, key, n)),
            Pred::Or(a, b) => format!("({} OR {})", go(a, table, key, n), go(b, table, key, n)),
            Pred::Not(a) => format!("NOT ({})", go(a, table, key, n)),
        }
    }
    go(p, table, key, &mut 0)
}

/// A new column's or table's `name` (ADD COLUMN, RENAME .. TO), likewise
/// but never in another case (which SQLite would keep as its name).
pub fn ident_def(name: &str, key: &str) -> String {
    match spell(&format!("{key}/{name}"), 8) {
        0 => format!("\"{name}\""),
        1 => format!("[{name}]"),
        2 => format!("`{name}`"),
        _ => name.to_string(),
    }
}

/// The named constraints of the column `ALTER TABLE t{table} ADD COLUMN
/// c{column}` adds (as its `Display` names them): COLLATE, NOT NULL, the
/// short AS, DEFAULT and REFERENCES, in that order.
pub fn added_names(table: usize, column: usize, def: &NewColumn) -> Vec<crate::model::Named> {
    use crate::model::{Cons, Named};
    let key = format!("t{table}+{column}");
    let named = |kind: &str| crate::writer::constraint_named(Some(&key), kind, column);
    let mut out = Vec::new();
    let mut add = |kind: &str, on: bool, cons: Cons| {
        if on && named(kind) {
            out.push(Named { name: format!("{kind}{column}"), column: Some(column), cons });
        }
    };
    add("co", def.coll.is_some(), Cons::Bare);
    add("nn", def.not_null, Cons::NotNull);
    add("ge", def.generated.is_some_and(|(_, long)| !long), Cons::Bare);
    add("df", def.default.is_some(), Cons::Bare);
    add("fk", def.references.is_some(), Cons::Fk(vec![column]));
    out
}

/// The trigger's SQL: its column references spelled by [`ident_ref`]
/// (seeded by the trigger's name and where they are).
pub fn create_rule(g: &Rule) -> String {
    let side = if g.event == 2 { "OLD" } else { "NEW" };
    let w = format!("w{}", g.id);
    let col = |c: &usize, at: &str| ident_ref(&format!("c{c}"), &format!("{w}{at}"));
    let term = |t: &Term| match t {
        Term::Lit(v) => literal(v),
        Term::Row(c) => format!("{side}.{}", col(c, side)),
    };
    let event = match g.event {
        0 => "INSERT".to_string(),
        1 if g.of.is_empty() => "UPDATE".to_string(),
        1 => format!("UPDATE OF {}", columns(g.of.iter().map(|c| col(c, "of")))),
        _ => "DELETE".to_string(),
    };
    let (timing, on) = match (g.view, g.before) {
        (true, _) => ("INSTEAD OF", 'v'),
        (_, true) => ("BEFORE", 't'),
        _ => ("AFTER", 't'),
    };
    let temp = if g.temp { "TEMP " } else { "" };
    // A trigger that is not TEMP lives in its table's schema, named there
    // when that is `aux` (unqualified, it would be main's, where the table
    // is not); a TEMP one may name its table's schema (half the time, by
    // its id).
    let (name_db, on_db) = match (g.temp, g.db) {
        (false, Db::Aux) => ("aux.", ""),
        (true, Db::Aux | Db::Temp) if g.id.is_multiple_of(2) => ("", if g.db == Db::Aux { "aux." } else { "temp." }),
        _ => ("", ""),
    };
    let mut out = format!("CREATE {temp}TRIGGER {name_db}w{} {timing} {event} ON {on_db}{on}{} FOR EACH ROW", g.id, g.table);
    if let Some(p) = &g.when {
        out.push_str(&format!(" WHEN {}", row_pred(p, side)));
    }
    out.push_str(" BEGIN ");
    for (n, s) in g.body.iter().enumerate() {
        match s {
            Step::Insert { table, columns: cs, values, or, upsert } => {
                let or = match or {
                    Some(Resolve::ReplaceInto) => "REPLACE".to_string(),
                    Some(r) => format!("INSERT OR {}", r.sql()),
                    None => "INSERT".to_string(),
                };
                out.push_str(&format!("{or} INTO t{table} ({}) VALUES ({}){}", columns(cs.iter().map(|c| col(c, "into"))), columns(values.iter().map(term)), upsert_sql(upsert)))
            },
            Step::Update { table, sets, filter, or } => {
                let or = or.map_or(String::new(), |r| format!("OR {} ", r.sql()));
                out.push_str(&format!("UPDATE {or}t{table} SET {}", columns(sets.iter().map(|(c, t)| format!("{} = {}", col(c, "set"), term(t))))));
                if let Some(p) = filter {
                    out.push_str(&format!(" WHERE {}", step_pred(p, *table, &format!("{w}u{n}"))));
                }
            },
            Step::Delete { table, filter } => {
                out.push_str(&format!("DELETE FROM t{table}"));
                if let Some(p) = filter {
                    out.push_str(&format!(" WHERE {}", step_pred(p, *table, &format!("{w}d{n}"))));
                }
            },
            Step::Ignore => out.push_str("SELECT RAISE(IGNORE)"),
            Step::Abort(p) => out.push_str(&format!("SELECT RAISE(ABORT, 'w{}') WHERE {}", g.id, row_pred(p, side))),
            // Spelled by its place: as written, or the same column of
            // the row inside a CTE, a window's sub-select or an IN, or of
            // the trigger's table in a FROM (its result unused either
            // way), which the rename code walks (`renameWalkWith`,
            // `renameTableSelectCb`, `renameColumnSelectCb`).
            Step::Select(c) => {
                let at = format!("{w}select{n}");
                let c = col(c, &at);
                out.push_str(&match if g.view { 0 } else { spell(&at, 6) } {
                    2 => format!("SELECT * FROM (WITH s(v) AS (SELECT {side}.{c}) SELECT v FROM s)"),
                    3 => format!("SELECT x.{c} FROM t{} AS x LIMIT 1", g.table),
                    4 => format!("SELECT v, row_number() OVER w FROM (SELECT {side}.{c} AS v) WINDOW w AS (ORDER BY v)"),
                    5 => format!("SELECT {side}.{c} WHERE {side}.{c} IN (SELECT {c} FROM t{} LIMIT 2)", g.table),
                    _ => format!("SELECT {side}.{c}"),
                })
            },
        }
        out.push_str("; ");
    }
    out.push_str("END");
    out
}

/// A column `ALTER TABLE .. ADD COLUMN` adds: its declared type (a STRICT
/// type in a STRICT table, else one of [`DECLS`] or none), `COLLATE`, `NOT
/// NULL`, and `DEFAULT` a literal SQLite reads as a constant (a value
/// [`literal`] writes as one token, or a negative number).
#[derive(Clone, Debug, PartialEq)]
pub struct NewColumn {
    pub ty: Option<ColType>,
    pub decl: Option<usize>,
    pub coll: Option<Coll>,
    pub not_null: bool,
    pub default: Option<Value>,
    /// `REFERENCES t{p}(c{q}) ..`: a foreign key of the new column alone.
    pub references: Option<ForeignKey>,
    /// `AS (c{s}) VIRTUAL` (`GENERATED ALWAYS AS (c{s}) VIRTUAL` when the
    /// flag is set): a generated column, a copy of column `s`.
    pub generated: Option<(usize, bool)>,
}

/// Which statistics a row of `sqlite_stat1` holds (its `idx` column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatOf {
    /// `idx` NULL: the table's row count (and row size), which the planner
    /// takes for the table's size when no index of it says otherwise.
    Table,
    /// Index `t{t}_i{k}`.
    Index(usize),
    /// The primary key of a WITHOUT ROWID table, which ANALYZE names
    /// after the table itself.
    PrimaryKey,
}

/// A row of `sqlite_stat1` with drawn numbers, as a table many times its
/// size, a tiny one, a skewed or an inconsistent one would have: the
/// planner's costs (index choice, join order, automatic indexes, Bloom
/// filters, skip-scans) are computed from these, and its results never
/// depend on them. Rendered within [`Statement::Stats`], which deletes the
/// rows of every `(tbl, idx)` it gives in one statement and inserts its
/// own, `('t{t}', .., 'n e1 e2 .. [unordered] [noskipscan] [sz=k]')`, in
/// another.
#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
    pub table: usize,
    pub of: StatOf,
    /// The row count, then for an index the average number of rows per
    /// distinct value of each prefix of its columns, as many as given (at
    /// least the row count; SQLite keeps its defaults for the rest).
    pub counts: Vec<u64>,
    /// `unordered`: the index is not used for a range or an ORDER BY.
    pub unordered: bool,
    /// `noskipscan`: the index is never skip-scanned.
    pub noskipscan: bool,
    /// `sz=k`: the average row size in bytes.
    pub size: Option<u32>,
    /// The `sqlite_stat4` samples of the index (of every index of the
    /// table for [`StatOf::Table`]) are deleted as well, so that the
    /// planner estimates from these numbers alone.
    pub drop_samples: bool,
}

impl Stat {
    /// The `idx` value: NULL, or the index's name as a literal.
    fn idx(&self) -> String {
        match self.of {
            StatOf::Table => "NULL".into(),
            StatOf::Index(k) => format!("'t{}_i{k}'", self.table),
            StatOf::PrimaryKey => format!("'t{}'", self.table),
        }
    }

    /// The row's `sqlite_stat1` key, as a condition.
    fn key(&self) -> String {
        format!("(tbl = 't{}' AND idx IS {})", self.table, self.idx())
    }

    /// The row, as a `VALUES` tuple.
    fn values(&self) -> String {
        let mut stat = self.counts.iter().map(u64::to_string).collect::<Vec<_>>().join(" ");
        for (on, flag) in [(self.unordered, "unordered"), (self.noskipscan, "noskipscan")] {
            if on {
                stat.push(' ');
                stat.push_str(flag);
            }
        }
        if let Some(k) = self.size {
            stat.push_str(&format!(" sz={k}"));
        }
        format!("('t{}', {}, '{stat}')", self.table, self.idx())
    }

    /// The `sqlite_stat4` rows it deletes, as a condition.
    fn samples(&self) -> Option<String> {
        self.drop_samples.then(|| match self.of {
            StatOf::Table => format!("tbl = 't{}'", self.table),
            _ => self.key(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Select {
    pub table: usize,
    /// The selected columns, in order (repeats allowed); empty for `*`.
    pub columns: Vec<usize>,
    pub filter: Option<Pred>,
    /// `ORDER BY` columns, each descending when `true`.
    pub order: Vec<(usize, bool)>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Insert {
    pub table: usize,
    /// The rowid is listed first (only for a rowid table without an INTEGER
    /// PRIMARY KEY, whose rowid has no column).
    pub rowid: bool,
    /// Distinct columns, at least one column or the rowid.
    pub columns: Vec<usize>,
    /// At least one row; each has the rowid (when listed), then one value
    /// per column.
    pub rows: Vec<Vec<Value>>,
    /// `INSERT OR x` (`REPLACE INTO` for [`Resolve::ReplaceInto`]).
    pub or: Option<Resolve>,
    /// `ON CONFLICT ..` clauses, in order (none with `or`).
    pub upsert: Vec<Upsert>,
    /// `RETURNING c..` (`RETURNING *` when empty).
    pub returning: Option<Vec<usize>>,
    /// The rows as `SELECT * FROM (VALUES ..)` (with `WHERE true` before
    /// ON CONFLICT, which would read as a join constraint), in order.
    pub select: bool,
}

/// A conflict resolution: `OR ABORT`, `OR FAIL`, `OR ROLLBACK` (drawn only
/// where nothing conflicts, so as plain), `OR IGNORE` (a conflicting row
/// is skipped), `OR REPLACE` (the rows it conflicts with are deleted
/// first), `REPLACE INTO` (as `OR REPLACE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolve {
    Abort,
    Fail,
    Rollback,
    Ignore,
    Replace,
    ReplaceInto,
}

impl Resolve {
    pub const ALL: [Resolve; 6] = [Resolve::Abort, Resolve::Fail, Resolve::Rollback, Resolve::Ignore, Resolve::Replace, Resolve::ReplaceInto];

    pub fn sql(self) -> &'static str {
        match self {
            Resolve::Abort => "ABORT",
            Resolve::Fail => "FAIL",
            Resolve::Rollback => "ROLLBACK",
            Resolve::Ignore => "IGNORE",
            Resolve::Replace | Resolve::ReplaceInto => "REPLACE",
        }
    }

    /// Whether a conflict deletes the rows in the way.
    pub fn replaces(self) -> bool {
        matches!(self, Resolve::Replace | Resolve::ReplaceInto)
    }
}

/// `ON CONFLICT [(c [COLLATE k], ..)] DO NOTHING | DO UPDATE SET .. [WHERE
/// p]`: its target (the columns of the PRIMARY KEY or of a UNIQUE index;
/// none only in the last clause), and what it does.
#[derive(Clone, Debug, PartialEq)]
pub struct Upsert {
    pub target: Option<Vec<(usize, Option<crate::model::Coll>)>>,
    /// `None` for DO NOTHING; else the sets (`c = excluded.c` for
    /// [`Excluded::Yes`]) and the WHERE over the row already there.
    pub update: Option<DoUpdate>,
}

/// DO UPDATE's sets and WHERE.
pub type DoUpdate = (Vec<(usize, Excluded)>, Option<Pred>);

/// The value DO UPDATE sets a column to: the value the INSERT would have
/// written there (`excluded.c`), or a literal.
#[derive(Clone, Debug, PartialEq)]
pub enum Excluded {
    Yes,
    No(Value),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Update {
    pub table: usize,
    /// Distinct columns, at least one.
    pub sets: Vec<(usize, Value)>,
    pub filter: Option<Pred>,
    /// `UPDATE OR x`.
    pub or: Option<Resolve>,
    pub returning: Option<Vec<usize>>,
    /// How two sets or more are written: `c0 = v0, c1 = v1` (`Plain`), as
    /// one row value `(c0, c1) = (v0, v1)` (`Row`), or from a subquery
    /// `(c0, c1) = (SELECT v0, v1)` (`Select`); the same effect.
    pub vector: VectorSet,
    /// `UPDATE .. FROM` one row of another table ([`UpdateFrom`]).
    pub from: Option<UpdateFrom>,
}

/// `UPDATE t{k} SET .. FROM t{table} AS u WHERE u.rowid = {rowid} AND
/// (p)` (lang_update.html, "UPDATE FROM"): the FROM source is one row of a
/// rowid table, or none, so that every target row the filter selects
/// joins with it at most once, and the sets listed in `reads` (by
/// position) take that row's column (`c = u.c{m}`) instead of their
/// literal. SQLite materializes the join before it writes a row
/// (`updateFromSelect`), so the values are the row's before the UPDATE;
/// the model resolves the statement into the plain UPDATE of those
/// values, or of no row when the FROM row is missing
/// (`crate::semantics::resolve_from`). With `sub`, the source is
/// `(SELECT * FROM t{table} WHERE rowid = {rowid}) AS u`.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateFrom {
    pub table: usize,
    pub rowid: i64,
    pub reads: Vec<(usize, usize)>,
    pub sub: bool,
}

/// The form of an UPDATE's sets ([`Update::vector`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VectorSet {
    #[default]
    Plain,
    Row,
    Select,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Delete {
    pub table: usize,
    pub filter: Option<Pred>,
    pub returning: Option<Vec<usize>>,
}

/// ON CONFLICT clauses, each after a space.
pub fn upsert_sql(clauses: &[Upsert]) -> String {
    let mut out = String::new();
    for u in clauses {
        out.push_str(" ON CONFLICT");
        if let Some(cs) = &u.target {
            let terms = columns(cs.iter().map(|(c, k)| match k {
                Some(k) => format!("c{c} COLLATE {}", k.name()),
                None => format!("c{c}"),
            }));
            out.push_str(&format!(" ({terms})"));
        }
        match &u.update {
            None => out.push_str(" DO NOTHING"),
            Some((sets, filter)) => {
                let sets = columns(sets.iter().map(|(c, e)| match e {
                    Excluded::Yes => format!("c{c} = excluded.c{c}"),
                    Excluded::No(v) => format!("c{c} = {}", literal(v)),
                }));
                out.push_str(&format!(" DO UPDATE SET {sets}"));
                if let Some(p) = filter {
                    out.push_str(&format!(" WHERE {p}"));
                }
            },
        }
    }
    out
}

fn returning(f: &mut fmt::Formatter, r: &Option<Vec<usize>>) -> fmt::Result {
    match r {
        Some(cs) if cs.is_empty() => write!(f, " RETURNING *"),
        Some(cs) => write!(f, " RETURNING {}", columns(cs.iter().map(|c| format!("c{c}")))),
        None => Ok(()),
    }
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

#[derive(Clone, Debug, PartialEq)]
pub enum Pred {
    /// `c op v`.
    Cmp(usize, CmpOp, Value),
    /// `c IS NULL`, or `c IS NOT NULL` when `true`.
    IsNull(usize, bool),
    /// `c BETWEEN lo AND hi`.
    Between(usize, Value, Value),
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
    pub fn leaves(&self, out: &mut Vec<usize>) {
        match self {
            Pred::Cmp(c, ..) | Pred::IsNull(c, _) | Pred::Between(c, ..) => out.push(*c),
            Pred::And(a, b) | Pred::Or(a, b) => {
                a.leaves(out);
                b.leaves(out);
            },
            Pred::Not(a) => a.leaves(out),
        }
    }
}

/// A literal SQLite reads back as exactly `v`.
///
/// - Integers in decimal; SQLite reads `-9223372036854775808` as the
///   smallest integer, not as a real.
/// - Reals in Rust's shortest round-trip decimal when the magnitude is
///   within `1e-50..1e50`: SQLite 3.50's decimal-to-binary conversion is not
///   correctly rounded for every input (outside about `1e-83..1e118` it is
///   off by an ulp for some 17-digit inputs; see the semantics tests), and
///   within this band it was exact on every one of a million samples.
///   Other reals are written exactly in binary, as `(m * 1.0 * 2^k ..)`
///   or `(m * 1.0 / 2^k ..)` with an odd integer `m < 2^53` and factors
///   of at most `2^62`: each step is exact in IEEE arithmetic. Infinities
///   are `9e999` and `-9e999`; `-0.0` is `-0.0`.
/// - Text in single quotes (quotes doubled); it is converted to the
///   database encoding. Text containing NUL is never generated (SQL text
///   ends there).
/// - Blobs as `x'..'`.
pub fn literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Int(i) => i.to_string(),
        Value::Real(x) => real_literal(*x),
        Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Blob(b) => {
            let mut s = String::with_capacity(3 + 2 * b.len());
            s.push_str("x'");
            for byte in b {
                s.push_str(&format!("{byte:02x}"));
            }
            s.push('\'');
            s
        },
    }
}

fn real_literal(x: f64) -> String {
    if x.is_infinite() {
        return if x > 0.0 { "9e999".into() } else { "-9e999".into() };
    }
    if x == 0.0 || (1e-50..1e50).contains(&x.abs()) {
        return format!("{x:?}");
    }
    // x = m * 2^e, m odd.
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1 << 52) - 1);
    let (mut m, mut e) = if exp == 0 { (frac, -1074) } else { (frac | (1 << 52), exp - 1075) };
    while m & 1 == 0 {
        m >>= 1;
        e += 1;
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let mut s = format!("({sign}{m} * 1.0");
    let op = if e < 0 { " / " } else { " * " };
    let mut left = e.unsigned_abs();
    while left > 0 {
        let k = left.min(62);
        s.push_str(op);
        s.push_str(&(1u64 << k).to_string());
        left -= k;
    }
    s.push(')');
    s
}

impl fmt::Display for Pred {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Pred::Cmp(c, op, v) => write!(f, "c{c} {} {}", op.sql(), literal(v)),
            Pred::IsNull(c, false) => write!(f, "c{c} IS NULL"),
            Pred::IsNull(c, true) => write!(f, "c{c} IS NOT NULL"),
            Pred::Between(c, lo, hi) => write!(f, "c{c} BETWEEN {} AND {}", literal(lo), literal(hi)),
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

fn columns(cs: impl Iterator<Item = String>) -> String {
    cs.collect::<Vec<_>>().join(", ")
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Statement::Select(s) => {
                let what = if s.columns.is_empty() { "*".into() } else { columns(s.columns.iter().map(|c| format!("c{c}"))) };
                write!(f, "SELECT {what} FROM t{}", s.table)?;
                where_clause(f, &s.filter)?;
                if !s.order.is_empty() {
                    let by = columns(s.order.iter().map(|&(c, d)| format!("c{c}{}", if d { " DESC" } else { "" })));
                    write!(f, " ORDER BY {by}")?;
                }
                if let Some(n) = s.limit {
                    write!(f, " LIMIT {n}")?;
                }
                Ok(())
            },
            Statement::Insert(s) => {
                let names = columns(s.rowid.then(|| "rowid".to_string()).into_iter().chain(s.columns.iter().map(|c| format!("c{c}"))));
                let mut rows = format!("VALUES {}", columns(s.rows.iter().map(|r| format!("({})", columns(r.iter().map(literal))))));
                if s.select {
                    rows = format!("SELECT * FROM ({rows}){}", if s.upsert.is_empty() { "" } else { " WHERE true" });
                }
                match s.or {
                    Some(Resolve::ReplaceInto) => write!(f, "REPLACE INTO t{} ({names}) {rows}", s.table)?,
                    Some(r) => write!(f, "INSERT OR {} INTO t{} ({names}) {rows}", r.sql(), s.table)?,
                    None => write!(f, "INSERT INTO t{} ({names}) {rows}", s.table)?,
                }
                write!(f, "{}", upsert_sql(&s.upsert))?;
                returning(f, &s.returning)
            },
            Statement::Update(s) => {
                let value = |k: usize, v: &Value| match s.from.as_ref().and_then(|f| f.reads.iter().find(|(i, _)| *i == k)) {
                    Some((_, m)) => format!("u.c{m}"),
                    None => literal(v),
                };
                let names = || columns(s.sets.iter().map(|(c, _)| format!("c{c}")));
                let values = || columns(s.sets.iter().enumerate().map(|(k, (_, v))| value(k, v)));
                let sets = match s.vector {
                    VectorSet::Row if s.sets.len() > 1 => format!("({}) = ({})", names(), values()),
                    VectorSet::Select if s.sets.len() > 1 => format!("({}) = (SELECT {})", names(), values()),
                    _ => columns(s.sets.iter().enumerate().map(|(k, (c, v))| format!("c{c} = {}", value(k, v)))),
                };
                match s.or {
                    Some(r) => write!(f, "UPDATE OR {} t{} SET {sets}", r.sql(), s.table)?,
                    None => write!(f, "UPDATE t{} SET {sets}", s.table)?,
                }
                match &s.from {
                    Some(from) => {
                        if from.sub {
                            write!(f, " FROM (SELECT * FROM t{} WHERE rowid = {}) AS u", from.table, from.rowid)?;
                        } else {
                            write!(f, " FROM t{} AS u WHERE u.rowid = {}", from.table, from.rowid)?;
                        }
                        if let Some(p) = &s.filter {
                            write!(f, " {} {}", if from.sub { "WHERE" } else { "AND" }, row_pred(p, &format!("t{}", s.table)))?;
                        }
                    },
                    None => where_clause(f, &s.filter)?,
                }
                returning(f, &s.returning)
            },
            Statement::Delete(s) => {
                write!(f, "DELETE FROM t{}", s.table)?;
                where_clause(f, &s.filter)?;
                returning(f, &s.returning)
            },
            Statement::Vacuum => write!(f, "VACUUM"),
            Statement::IncrementalVacuum(n) => write!(f, "PRAGMA incremental_vacuum({n})"),
            Statement::Check { quick, table } => {
                write!(f, "PRAGMA {}", if *quick { "quick_check" } else { "integrity_check" })?;
                match table {
                    Some(t) => write!(f, "(t{t})"),
                    None => Ok(()),
                }
            },
            Statement::Count { table, filter } => {
                write!(f, "SELECT count(*) FROM t{table}")?;
                where_clause(f, filter)
            },
            // A TEMP table as `CREATE TEMP TABLE t5` or `CREATE TABLE
            // temp.t5`, by its number; one in aux as `aux.t5`.
            Statement::CreateTable { table, def } => match def.db {
                Db::Main => write!(f, "{}", create_table_named(&format!("t{table}"), def, Some(&format!("t{table}")))),
                Db::Temp if table.is_multiple_of(2) => write!(f, "{}", create_table_named(&format!("t{table}"), def, Some(&format!("t{table}"))).replacen("CREATE TABLE", "CREATE TEMP TABLE", 1)),
                db => write!(f, "{}", create_table_named(&format!("{}.t{table}", db.name()), def, Some(&format!("t{table}")))),
            },
            Statement::DropTable(t) => write!(f, "DROP TABLE t{t}"),
            Statement::CreateIndex { table, index, def, db } => {
                let term = |k: usize, c: usize, d: bool| match def.expr {
                    Some(e) if k == 0 => format!("{}{}", Sql(&key_expr(e, c), At::TOP), if d { " DESC" } else { "" }),
                    _ => index_term(def, c, d),
                };
                let cols = columns(def.columns.iter().enumerate().map(|(k, &(c, d))| term(k, c, d)));
                let unique = if def.unique { "UNIQUE " } else { "" };
                // `IF NOT EXISTS` and the index named in its schema (`main.`)
                // a quarter of the time each, by the columns' text.
                let ine = if spell(&cols, 4) == 0 { "IF NOT EXISTS " } else { "" };
                let schema = match db {
                    Db::Aux => "aux.",
                    Db::Temp if spell(&format!("{cols}{index}"), 2) == 0 => "temp.",
                    Db::Main if spell(&format!("{cols}{index}"), 4) == 0 => "main.",
                    _ => "",
                };
                write!(f, "CREATE {unique}INDEX {ine}{schema}t{table}_i{index} ON t{table}({cols})")?;
                match &def.filter {
                    Some(p) => write!(f, " WHERE {}", Sql(&pred_expr(p), At::TOP)),
                    None => Ok(()),
                }
            },
            Statement::DropIndex { table, index, db } => match spell(&format!("{table}.{index}"), 4) {
                0 => write!(f, "DROP INDEX IF EXISTS {}.t{table}_i{index}", db.name()),
                _ => write!(f, "DROP INDEX t{table}_i{index}"),
            },
            Statement::CacheSize(n) => write!(f, "PRAGMA cache_size = {n}"),
            Statement::Analyze(None) => write!(f, "ANALYZE"),
            Statement::Analyze(Some((t, None))) => write!(f, "ANALYZE t{t}"),
            Statement::Analyze(Some((t, Some(i)))) => write!(f, "ANALYZE t{t}_i{i}"),
            Statement::AnalysisLimit(n) => write!(f, "PRAGMA analysis_limit = {n}"),
            Statement::Optimize(None) => write!(f, "PRAGMA optimize"),
            Statement::Optimize(Some(mask)) => write!(f, "PRAGMA optimize({mask:#x})"),
            Statement::AutomaticIndex(on) => write!(f, "PRAGMA automatic_index = {}", u8::from(*on)),
            Statement::ReverseUnordered(on) => write!(f, "PRAGMA reverse_unordered_selects = {}", u8::from(*on)),
            // `ANALYZE sqlite_schema` creates the statistics tables when no
            // ANALYZE has (it gathers nothing of a system table), and,
            // as every ANALYZE, reloads the statistics after the rows.
            Statement::Stats(rows) => {
                let keys = rows.iter().map(Stat::key).collect::<Vec<_>>().join(" OR ");
                write!(f, "ANALYZE sqlite_schema; DELETE FROM sqlite_stat1 WHERE {keys}; INSERT INTO sqlite_stat1 VALUES {}; ", columns(rows.iter().map(Stat::values)))?;
                let samples: Vec<String> = rows.iter().filter_map(Stat::samples).collect();
                if !samples.is_empty() {
                    write!(f, "DELETE FROM sqlite_stat4 WHERE {}; ", samples.join(" OR "))?;
                }
                write!(f, "ANALYZE sqlite_schema")
            },
            Statement::Savepoint(k) => write!(f, "SAVEPOINT s{k}"),
            Statement::RollbackTo(k) => write!(f, "ROLLBACK TO s{k}"),
            Statement::Release(k) => write!(f, "RELEASE s{k}"),
            Statement::Rollback => write!(f, "ROLLBACK"),
            Statement::AddColumn { table, column, def } => {
                let key = format!("t{table}+{column}");
                let kw = if spell(&key, 4) == 0 { "" } else { "COLUMN " };
                write!(f, "ALTER TABLE {} ADD {kw}{}", ident_ref(&format!("t{table}"), &key), ident_def(&format!("c{column}"), &key))?;
                match (def.ty, def.decl) {
                    (Some(ty), _) => write!(f, " {}", ty.name())?,
                    (None, Some(d)) => write!(f, " {}", DECLS[d].0)?,
                    _ => {},
                }
                // Its constraints named now and then (`constraint_name`).
                let named = |kind: &str| constraint_name(Some(&key), kind, *column);
                if let Some(k) = def.coll {
                    write!(f, " {}COLLATE {}", named("co"), k.name())?;
                }
                if def.not_null {
                    write!(f, " {}NOT NULL", named("nn"))?;
                }
                if let Some((src, long)) = def.generated {
                    // Unnamed in its long form, which SQLite reads as
                    // the end of a type name (`sqlite3AddColumn`).
                    write!(f, " {}AS (c{src}) VIRTUAL", if long { "GENERATED ALWAYS ".to_string() } else { named("ge") })?;
                }
                if let Some(v) = &def.default {
                    write!(f, " {}DEFAULT {}", named("df"), literal(v))?;
                }
                match &def.references {
                    Some(r) => write!(f, " {}REFERENCES t{}(c{}){}", named("fk"), r.parent, r.parent_columns[0], fk_tail(r)),
                    None => Ok(()),
                }
            },
            // The names spelled by [`ident_ref`] and [`ident_def`], and
            // the keyword COLUMN left out now and then (by the same).
            Statement::DropColumn { table, column, columns } => {
                let key = format!("t{table}.{column}.{columns}");
                let kw = |at: &str| if spell(&format!("{key}{at}"), 4) == 0 { "" } else { "COLUMN " };
                write!(f, "ALTER TABLE {} DROP {}{}", ident_ref(&format!("t{table}"), &key), kw("d"), ident_ref(&format!("c{column}"), &key))?;
                for k in column + 1..*columns {
                    let key = format!("{key}.{k}");
                    write!(f, "; ALTER TABLE t{table} RENAME {}{} TO {}", kw(&key), ident_ref(&format!("c{k}"), &key), ident_def(&format!("c{}", k - 1), &key))?;
                }
                Ok(())
            },
            Statement::RenameColumn { table, column } => {
                let key = format!("t{table}~{column}");
                let kw = |at: &str| if spell(&format!("{key}{at}"), 4) == 0 { "" } else { "COLUMN " };
                let (name, away) = (format!("c{column}"), format!("r{column}"));
                write!(f, "ALTER TABLE {} RENAME {}{} TO {}", ident_ref(&format!("t{table}"), &key), kw("a"), ident_ref(&name, &key), ident_def(&away, &key))?;
                write!(f, "; ALTER TABLE {} RENAME {}{} TO {}", ident_ref(&format!("t{table}"), &format!("{key}b")), kw("b"), ident_ref(&away, &format!("{key}b")), ident_def(&name, &format!("{key}b")))
            },
            Statement::NotNull { table, column, on } => {
                let key = format!("t{table}!{column}");
                write!(f, "ALTER TABLE {} ALTER COLUMN {} {} NOT NULL", ident_ref(&format!("t{table}"), &key), ident_ref(&format!("c{column}"), &key), if *on { "SET" } else { "DROP" })
            },
            Statement::AddCheck { table, index, check } => {
                let key = format!("t{table}&{index}");
                write!(f, "ALTER TABLE {} ADD CONSTRAINT {} CHECK({})", ident_ref(&format!("t{table}"), &key), ident_def(&format!("k{index}"), &key), Sql(&pred_expr(check), At::TOP))
            },
            Statement::ForeignKeys(on) => write!(f, "PRAGMA foreign_keys = {}", u8::from(*on)),
            Statement::FkCheck(None) => write!(f, "PRAGMA foreign_key_check"),
            Statement::FkCheck(Some(t)) => write!(f, "PRAGMA foreign_key_check(t{t})"),
            Statement::DropCheck { table, check } => {
                let key = format!("t{table}-{check}");
                write!(f, "ALTER TABLE {} DROP CONSTRAINT {}", ident_ref(&format!("t{table}"), &key), ident_def(&format!("k{check}"), &key))
            },
            Statement::DropConstraint { table, name } => {
                let key = format!("t{table}-{name}");
                write!(f, "ALTER TABLE {} DROP CONSTRAINT {}", ident_ref(&format!("t{table}"), &key), ident_ref(name, &key))
            },
            Statement::CreateRule(g) => write!(f, "{}", create_rule(g)),
            Statement::DropRule(id) => write!(f, "DROP TRIGGER w{id}"),
            Statement::RecursiveTriggers(on) => write!(f, "PRAGMA recursive_triggers = {}", u8::from(*on)),
            Statement::Reindex(Reindex::All) => write!(f, "REINDEX"),
            Statement::Reindex(Reindex::Table(t)) => write!(f, "REINDEX t{t}"),
            Statement::Reindex(Reindex::Index(t, i)) => write!(f, "REINDEX t{t}_i{i}"),
            Statement::Reindex(Reindex::Coll(k)) => write!(f, "REINDEX {}", k.name()),
            Statement::JournalMode(j) => write!(f, "PRAGMA journal_mode = {}", j.name()),
            Statement::WalCheckpoint(None) => write!(f, "PRAGMA wal_checkpoint"),
            Statement::WalCheckpoint(Some(c)) => write!(f, "PRAGMA wal_checkpoint({})", c.name()),
            Statement::SecureDelete(n) => match n {
                2 => write!(f, "PRAGMA secure_delete = FAST"),
                n => write!(f, "PRAGMA secure_delete = {n}"),
            },
            Statement::CellSizeCheck(on) => write!(f, "PRAGMA cell_size_check = {}", u8::from(*on)),
            Statement::MmapSize(n) => write!(f, "PRAGMA mmap_size = {n}"),
            Statement::AutoVacuum(a) => write!(f, "PRAGMA auto_vacuum = {}; VACUUM", a.name()),
            Statement::TempStore(n) => write!(f, "PRAGMA temp_store = {}", ["DEFAULT", "FILE", "MEMORY"][usize::from(*n).min(2)]),
            Statement::LockingMode(x) => write!(f, "PRAGMA locking_mode = {}", if *x { "EXCLUSIVE" } else { "NORMAL" }),
            Statement::JournalSizeLimit(n) => write!(f, "PRAGMA journal_size_limit = {n}"),
            Statement::WalAutocheckpoint(n) => write!(f, "PRAGMA wal_autocheckpoint = {n}"),
            Statement::CacheSpill(n) => write!(f, "PRAGMA cache_spill = {n}"),
            Statement::VacuumInto(memory) => write!(f, "VACUUM INTO '{}'", if *memory { ":memory:" } else { "" }),
            Statement::Begin(b) => write!(f, "BEGIN {}; SAVEPOINT s0", b.name()),
            Statement::Commit => write!(f, "COMMIT"),
            Statement::Synchronous(n) => write!(f, "PRAGMA synchronous = {}", ["OFF", "NORMAL", "FULL", "EXTRA"][usize::from(*n).min(3)]),
            Statement::ShrinkMemory => write!(f, "PRAGMA shrink_memory"),
            Statement::Attach(false) => write!(f, "ATTACH DATABASE ':memory:' AS aux"),
            Statement::Attach(true) => write!(f, "ATTACH '' AS aux"),
            Statement::Detach => write!(f, "DETACH DATABASE aux"),
            Statement::AnalyzeDb(db) => write!(f, "ANALYZE {}", db.name()),
            Statement::Virtual { s, kind } => write!(f, "{}", crate::vtabs::render(s, kind)),
            Statement::DbPragma { db, k, table } => {
                write!(f, "PRAGMA {}.{}", db.name(), DB_PRAGMAS[*k % DB_PRAGMAS.len()].0)?;
                match table {
                    Some(t) => write!(f, "(t{t})"),
                    None => Ok(()),
                }
            },
            Statement::ViewWrite { view, write } => match &**write {
                Statement::Insert(s) => {
                    let rows = columns(s.rows.iter().map(|r| format!("({})", columns(r.iter().map(literal)))));
                    write!(f, "INSERT INTO v{view} ({}) VALUES {rows}", columns(s.columns.iter().map(|c| format!("c{c}"))))
                },
                Statement::Update(s) => {
                    write!(f, "UPDATE v{view} SET {}", columns(s.sets.iter().map(|(c, v)| format!("c{c} = {}", literal(v)))))?;
                    where_clause(f, &s.filter)
                },
                Statement::Delete(s) => {
                    write!(f, "DELETE FROM v{view}")?;
                    where_clause(f, &s.filter)
                },
                s => write!(f, "{s}"),
            },
            Statement::SwapTables { a, b, indexes } => {
                write!(f, "ALTER TABLE t{a} RENAME TO t_swap; ALTER TABLE t{b} RENAME TO t{a}; ALTER TABLE t_swap RENAME TO t{b}")?;
                for (t, defs) in [(*a, &indexes.0), (*b, &indexes.1)] {
                    for k in 0..defs.len() {
                        write!(f, "; DROP INDEX t{t}_i{k}")?;
                    }
                }
                for (t, defs) in [(*b, &indexes.0), (*a, &indexes.1)] {
                    for (k, def) in defs.iter().enumerate() {
                        write!(f, "; {}", Statement::CreateIndex { table: t, index: k, def: def.clone(), db: Db::Main })?;
                    }
                }
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders() {
        let p = Pred::Or(
            Box::new(Pred::And(Box::new(Pred::Cmp(0, CmpOp::Le, Value::Int(-3))), Box::new(Pred::IsNull(1, true)))),
            Box::new(Pred::Not(Box::new(Pred::Between(2, Value::Text("a'b".into()), Value::Blob(vec![0, 255]))))),
        );
        let s = Statement::Select(Select { table: 1, columns: vec![0, 2], filter: Some(p), order: vec![(1, true), (0, false)], limit: Some(4) });
        assert_eq!(
            s.to_string(),
            "SELECT c0, c2 FROM t1 WHERE ((c0 <= -3 AND c1 IS NOT NULL) OR NOT (c2 BETWEEN 'a''b' AND x'00ff')) ORDER BY c1 DESC, c0 LIMIT 4"
        );
        let i = Statement::Insert(Insert { table: 0, rowid: true, columns: vec![1], rows: vec![vec![Value::Int(5), Value::Real(0.5)], vec![Value::Int(6), Value::Null]], ..Default::default() });
        assert_eq!(i.to_string(), "INSERT INTO t0 (rowid, c1) VALUES (5, 0.5), (6, NULL)");
        assert_eq!(real_literal(f64::from_bits(1)), format!("(1 * 1.0{})", " / 4611686018427387904".repeat(17) + " / 1048576"));
        assert_eq!(real_literal(3.0 * 2f64.powi(200)), format!("(3 * 1.0{} * 16384)", " * 4611686018427387904".repeat(3)));
    }

    /// The three forms of an UPDATE's sets render as SQLite reads them and
    /// have the same effect; one set is always plain.
    #[test]
    fn vector_sets_render_and_agree() {
        let base = Update { table: 0, sets: vec![(1, Value::Int(7)), (0, Value::Text("a".into()))], filter: None, or: None, returning: None, vector: VectorSet::Plain, from: None };
        let mut seen = Vec::new();
        for (vector, sql) in [
            (VectorSet::Plain, "UPDATE t0 SET c1 = 7, c0 = 'a'"),
            (VectorSet::Row, "UPDATE t0 SET (c1, c0) = (7, 'a')"),
            (VectorSet::Select, "UPDATE t0 SET (c1, c0) = (SELECT 7, 'a')"),
        ] {
            let u = Statement::Update(Update { vector, ..base.clone() });
            assert_eq!(u.to_string(), sql);
            let db = rusqlite::Connection::open_in_memory().unwrap();
            db.execute_batch(&format!("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 2), (3, 4); {sql};")).unwrap();
            let rows: Vec<(String, i64)> = db.prepare("SELECT c0, c1 FROM t0").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
            seen.push(rows);
        }
        assert!(seen.iter().all(|r| *r == seen[0]));
        let one = Update { sets: vec![(1, Value::Int(7))], vector: VectorSet::Row, ..base };
        assert_eq!(Statement::Update(one).to_string(), "UPDATE t0 SET c1 = 7");
    }
}
