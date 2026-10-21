//! Test cases for SQLite: a generated database file and a sequence of SQL
//! statements over it ([`Case`]), with the predicate [`well_formed`], the
//! generator [`cases`] and its metrics [`case_metrics`].
//!
//! The generator draws a file with [`Files`], reads its model back, and
//! draws each statement against the model the statements before it leave
//! ([`crate::semantics`]). Its hooks never run SQLite.

use std::{cell::RefCell, fmt, ops::ControlFlow};

use ephyra::{Gen, Source};

use crate::{
    files::Files,
    integrity_ok,
    model::{column_value, Action, Coll, ColType, Db, Encoding, ForeignKey, Index, Key, KeyExpr, Layout, Model, Rule, Step, Table, Term, Value, DECLS},
    reader,
    semantics::{self, affinity, comparable, convert, is_key, Affinity},
    statements::{AutoVac, Begin, Checkpoint, CmpOp, Delete, Journal, Excluded, Insert, NewColumn, Pred, Reindex, Resolve, Select, Stat, StatOf, Statement, Update, UpdateFrom, Upsert, VectorSet},
    writer::Rng,
    SqliteFile,
};

/// A database file and the statements to run on it, in order.
#[derive(Clone, PartialEq)]
pub struct Case {
    pub file: SqliteFile,
    pub statements: Vec<Statement>,
}

/// The file's summary (see [`SqliteFile`]'s `Debug`), then the statements
/// as a SQL script.
impl fmt::Debug for Case {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Case {{")?;
        writeln!(f, "  file: {:?}", self.file)?;
        writeln!(f, "  -- {} statements", self.statements.len())?;
        for s in &self.statements {
            writeln!(f, "  {s};")?;
        }
        write!(f, "}}")
    }
}

fn empty_model() -> Model {
    Model { encoding: Encoding::Utf8, tables: Vec::new(), views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session { memory_main: true, ..Default::default() }, vtabs: Vec::new() }
}

/// Table `t`, or, with probability `p` when `t` is not `wanted` and others
/// are, one of those.
fn fk_target(g: &mut Source, m: &Model, t: usize, p: f64, wanted: impl Fn(&Model, usize) -> bool) -> usize {
    if wanted(m, t) {
        return t;
    }
    let others: Vec<usize> = (0..m.tables.len()).filter(|&k| wanted(m, k)).collect();
    if others.is_empty() || !g.bool_with(p) {
        return t;
    }
    *g.pick(&others)
}

/// Whether a foreign key with an ON DELETE (`update` false) or ON UPDATE
/// action names table `k` as its parent.
fn acted(m: &Model, k: usize, update: bool) -> bool {
    m.tables.iter().any(|c| c.fks.iter().any(|f| f.parent == k && if update { f.on_update } else { f.on_delete } != Action::NoAction))
}

/// How many constraints `ALTER TABLE t{o} DROP CONSTRAINT` may drop: its
/// last check and its named constraints SQLite drops as the model has it
/// (`semantics::droppable_constraint`).
fn draw_drop_constraint(m: &Model, o: usize) -> usize {
    let t = &m.tables[o];
    usize::from(!t.checks.is_empty()) + t.names.iter().filter(|n| semantics::droppable_constraint(m, o, t, &n.name)).count()
}

/// The model and layout of a file the file generator writes; the empty file
/// is an empty database (and has no layout).
pub fn read(file: &SqliteFile) -> Option<(Model, Option<Layout>)> {
    if file.0.is_empty() {
        return Some((empty_model(), None));
    }
    reader::read(&file.0).map(|(mut m, l)| {
        // The writer names their constraints by the tables' names.
        for (k, t) in m.tables.iter_mut().enumerate() {
            t.names = crate::writer::table_names(t, Some(&format!("t{k}")));
        }
        (m, Some(l))
    })
}

/// The model a file holds (see [`read`]).
pub fn model_of(file: &SqliteFile) -> Option<Model> {
    read(file).map(|(m, _)| m)
}

thread_local! {
    /// The last few files `integrity_ok` judged, with the verdicts: shrink
    /// and mutation candidates mostly share their file.
    static JUDGED: RefCell<Vec<(Vec<u8>, bool)>> = const { RefCell::new(Vec::new()) };
}

fn integrity_ok_cached(file: &SqliteFile) -> bool {
    if let Some(v) = JUDGED.with(|j| j.borrow().iter().find(|(b, _)| *b == file.0).map(|(_, v)| *v)) {
        return v;
    }
    let v = integrity_ok(file);
    JUDGED.with(|j| {
        let mut j = j.borrow_mut();
        if j.len() >= 4 {
            j.remove(0);
        }
        j.push((file.0.clone(), v));
    });
    v
}

/// A case is well formed when its file is valid by SQLite's own check, the
/// file reader recovers the model from it, and every statement is valid
/// against the model the statements before it leave.
#[ephyra::generator]
pub fn well_formed(case: &Case) -> bool {
    integrity_ok_cached(&case.file)
        && model_of(&case.file).is_some_and(|mut m| {
            case.statements.iter().all(|s| {
                let ok = semantics::valid(&m, s);
                if ok {
                    semantics::apply(&mut m, s);
                }
                ok
            })
        })
}

/// Cases: a generated file and up to about 30 statements over it.
#[ephyra::generator(well_formed, metrics = case_metrics)]
pub fn cases() -> impl Gen<Value = Case> {
    Cases
}

pub struct Cases;

// ------------------------------------------------------------------ drawing


const MAX_STATEMENTS: usize = 50;

fn draw_statements(g: &mut Source, mut m: Model) -> Vec<Statement> {
    let most = (4 + 4 * g.size()).min(50);
    let n = if g.bool_with(0.01) { 0 } else { g.int(1..=most) };
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        // The last statement is a VACUUM now and then: it reads the whole
        // file as it rewrites it, after the others have run on the file's
        // own layout.
        let s = if k + 1 == n && g.bool_with(0.85) {
            // Outside any transaction, where VACUUM runs.
            // (rolled back half the time: VACUUM then reads the pages
            // the transaction wrote as they were).
            if !m.savepoints.is_empty() {
                // Committed (COMMIT inside a BEGIN) where a rollback is
                // not drawn (journal_mode OFF); with a deferred foreign
                // key's orphan it cannot commit: then no VACUUM.
                let commit = if m.session.begun || g.bool_with(0.2) { Statement::Commit } else { Statement::Release(0) };
                let end = if (g.bool_with(0.85) || !semantics::valid(&m, &commit)) && semantics::valid(&m, &Statement::Rollback) { Statement::Rollback } else { commit };
                if !semantics::valid(&m, &end) {
                    let s = draw_statement(g, &m);
                    semantics::apply(&mut m, &s);
                    out.push(s);
                    continue;
                }
                semantics::apply(&mut m, &end);
                out.push(end);
            }
            // Statistics of the final rows before it 45% of the time, for
            // the queries over the final state to be planned with.
            if g.bool_with(0.45) {
                out.push(Statement::Analyze(None));
            }
            // Crafted statistics of some tables after that 40% of the
            // time: the queries are planned for tables of other sizes
            // and shapes than they have.
            if !m.tables.is_empty() && g.bool_with(0.6) {
                out.push(draw_final_stats(g, &m));
            }
            Statement::Vacuum
        } else if k == 0 && n > 2 && g.bool_with(0.9) {
            // Storage settings first a third of the time, for the
            // transaction's writes and its rollback (a journal mode, the
            // auto-vacuum mode, temporary files, memory-mapped reads, ..).
            if g.bool_with(0.65) {
                for _ in 0..*g.weighted(&[(3, 1), (2, 2), (1, 3)]) {
                    let s = draw_storage(g, &m);
                    semantics::apply(&mut m, &s);
                    out.push(s);
                }
            }
            // `aux` attached 30% of the time, with a
            // table or two made in it (the second maybe a child of the
            // first): the statements after write them as any table.
            if g.bool_with(0.3) {
                let attach = Statement::Attach(g.bool_with(0.3));
                semantics::apply(&mut m, &attach);
                out.push(attach);
                for _ in 0..g.int(1..=2) {
                    let s = Statement::CreateTable { table: m.tables.len(), def: draw_new_table(g, &m, Some(Db::Aux)) };
                    if semantics::valid(&m, &s) {
                        semantics::apply(&mut m, &s);
                        out.push(s);
                    }
                }
            }
            // Virtual tables 75% of the time, made and filled (an
            // R-Tree, a Geopoly or FTS5 table, an fts5vocab over one),
            // two of them two thirds of the time (half before): the
            // statements after write them, the queries read them.
            if g.bool_with(0.75) {
                for _ in 0..*g.weighted(&[(1, 1), (2, 2)]) {
                    for s in crate::vtabs::draw_made(g, &m.vtabs) {
                        let s = Statement::Virtual { kind: crate::vtabs::target(&m.vtabs, &s).expect("a table"), s };
                        if semantics::valid(&m, &s) {
                            semantics::apply(&mut m, &s);
                            out.push(s);
                        }
                    }
                }
            }
            // A transaction around most of the statements now and then,
            // begun by BEGIN a quarter of the time.
            if g.bool_with(0.25) {
                Statement::Begin(*g.weighted(&[(2, Begin::Deferred), (2, Begin::Immediate), (1, Begin::Exclusive)]))
            } else {
                Statement::Savepoint(0)
            }
        } else {
            draw_statement(g, &m)
        };
        semantics::apply(&mut m, &s);
        out.push(s);
    }
    out
}

/// A statement valid against `m`.
pub fn draw_statement(g: &mut Source, m: &Model) -> Statement {
    // A virtual table's statement: often once there is one, rarely else
    // (a new one).
    if g.bool_with(if m.vtabs.is_empty() { 0.01 } else { 0.15 }) {
        for _ in 0..4 {
            if let Some(s) = crate::vtabs::draw(g, &m.vtabs, !m.savepoints.is_empty()) {
                if let Some(kind) = crate::vtabs::target(&m.vtabs, &s) {
                    let s = Statement::Virtual { s, kind };
                    if semantics::valid(m, &s) {
                        return s;
                    }
                }
            }
        }
    }
    if m.tables.is_empty() || g.bool_with(0.18) {
        return match g.int(0..22) {
            // A VACUUM mid-sequence is rare: what runs after it runs on
            // SQLite's own layout (and the last statement is often one).
            0 if m.savepoints.is_empty() && g.bool_with(0.25) => Statement::Vacuum,
            0 => draw_check(g, m),
            1..=3 => Statement::IncrementalVacuum(*g.pick(&[0, 1, 2, 5, 100])),
            4..=5 => draw_check(g, m),
            13 | 14 => Statement::Analyze(None),
            15 => Statement::AnalysisLimit(*g.pick(&[0, 1, 2, 10, 100])),
            16 => Statement::Optimize(g.bool_with(0.8).then(|| *g.pick(&OPTIMIZE_MASKS))),
            // Off only a fifth of the time: off persists to the queries, and
            // automatic indexes are a planner path worth keeping (constructed
            // for 11.4% of inputs before these statements, 9.5% at half off).
            17 => Statement::AutomaticIndex(g.bool_with(0.8)),
            18 => Statement::ReverseUnordered(g.bool_with(0.7)),
            19..=21 => draw_storage(g, m),
            _ => Statement::CacheSize(*g.pick(&[1, 2, 5, 10, 20, 100, -1, -2000])),
        };
    }
    let fk_weight = if m.tables.iter().any(|t| !t.fks.is_empty()) { 3 } else { 1 };
    // Triggers with bodies: there must be another table to write.
    let rule_weight = if m.tables.len() > 1 && m.rules.len() < semantics::MAX_RULES { 10 } else { 0 };
    // Tables foreign keys link (children, and their parents) a third of
    // the time when there are any: writes to them run the checks and
    // actions.
    let linked: Vec<usize> = (0..m.tables.len()).filter(|&k| !m.tables[k].fks.is_empty() || m.tables.iter().any(|t| t.fks.iter().any(|f| f.parent == k))).collect();
    // Tables with triggers 30% of the time otherwise: writes to them fire
    // the triggers.
    let ruled: Vec<usize> = (0..m.tables.len()).filter(|&k| m.rules.iter().any(|r| !r.view && r.table == k)).collect();
    for _ in 0..8 {
        let t = if !linked.is_empty() && g.bool_with(0.33) {
            *g.pick(&linked)
        } else if !ruled.is_empty() && g.bool_with(0.3) {
            *g.pick(&ruled)
        } else {
            g.index(m.tables.len())
        };
        let table = &m.tables[t];
        let s = match *g.weighted(&[(35, 0u8), (25, 1), (20, 2), (12, 3), (10, 4), (5, 5), (28, 6), (10, 7), (12, 8), (12, 9), (25, 10), (8, 11), (3, 12), (8, 13), (8, 14), (5, 15), (5, 16), (2, 17), (4, 18), (fk_weight, 19), (2, 20), (rule_weight, 21), (u32::from(!m.rules.is_empty()) * 3, 22), (1, 23), (u32::from(!m.views.is_empty() && m.rules.len() < semantics::MAX_RULES) * 6, 24), (u32::from(m.rules.iter().any(|r| r.view)) * 15, 25), (u32::from(!table.indexes.is_empty()) * 4, 26), (u32::from(!m.session.aux && m.savepoints.is_empty()) * 4, 27), (u32::from(m.session.aux) * 6, 28), (2, 29), (3, 30), (3, 31)]) {
            0 => Statement::Select(draw_select(g, table, t)),
            // A child of a foreign key a third of the time when this table
            // is none: its check of the parent (`sqlite3FkCheck`).
            1 => {
                let t = fk_target(g, m, t, 0.35, |m, k| !m.tables[k].fks.is_empty());
                match draw_insert(g, m, t) {
                    Some(s) => Statement::Insert(s),
                    None => continue,
                }
            },
            // A parent whose key a child's ON UPDATE action follows (or a
            // child) a third of the time when this table is neither.
            2 => {
                let t = fk_target(g, m, t, 0.35, |m, k| acted(m, k, true) || !m.tables[k].fks.is_empty());
                match if g.bool_with(0.1) { draw_clashing_update(g, m, t) } else { draw_update(g, m, t) } {
                    Some(s) => Statement::Update(s),
                    None => continue,
                }
            },
            // A parent's rows a child refers to 40% of the time, for the
            // foreign key's action (or its failure).
            // A parent whose children's ON DELETE action runs, half the
            // time when this table is none.
            3 => {
                let t = fk_target(g, m, t, 0.5, |m, k| acted(m, k, false));
                let table = &m.tables[t];
                match referenced(g, m, t).filter(|_| g.bool_with(0.4)) {
                    Some((c, v)) => Statement::Delete(Delete { table: t, filter: Some(Pred::Cmp(c, CmpOp::Eq, v)), returning: draw_returning(g, table) }),
                    None => Statement::Delete(Delete { table: t, filter: (!g.bool_with(0.05)).then(|| draw_pred(g, table, 3)), returning: draw_returning(g, table) }),
                }
            },
            4 => draw_check(g, m),
            6 => Statement::CreateIndex { table: t, index: table.indexes.len(), def: draw_new_index(g, table), db: table.db },
            7 if !table.indexes.is_empty() => Statement::DropIndex { table: t, index: table.indexes.len() - 1, db: table.db },
            8 => Statement::CreateTable { table: m.tables.len(), def: draw_new_table(g, m, None) },
            9 => Statement::DropTable(m.tables.len() - 1),
            // Savepoints: a ROLLBACK TO brings the pages it wrote back as
            // they were, generated layout included, for later statements.
            10 => {
                let depth = m.savepoints.len();
                match g.int(0..6) {
                    _ if depth == 0 && g.bool_with(0.2) => Statement::Begin(*g.pick(&[Begin::Deferred, Begin::Immediate, Begin::Exclusive])),
                    _ if depth == 0 => Statement::Savepoint(0),
                    4 if g.bool_with(0.3) || m.session.begun => Statement::Commit,
                    0 => Statement::Savepoint(depth),
                    1 | 2 => Statement::RollbackTo(g.index(depth)),
                    3 => Statement::Rollback,
                    _ => Statement::Release(g.index(depth)),
                }
            },
            7 => continue,
            // Statistics of one table, or of one of its indexes.
            11 => match table.indexes.len() {
                n if n > 0 && g.bool_with(0.3) => Statement::Analyze(Some((t, Some(g.index(n))))),
                _ => Statement::Analyze(Some((t, None))),
            },
            12 => Statement::Stats(draw_stats(g, table, t)),
            13 => Statement::AddColumn { table: t, column: table.columns, def: draw_new_column(g, m, t) },
            14 => match draw_drop_column(g, m, t) {
                Some(s) => s,
                None => continue,
            },
            15 => {
                let c = g.index(table.columns);
                Statement::NotNull { table: t, column: c, on: !table.not_null[c] }
            },
            16 => match draw_check_constraint(g, table, m.encoding) {
                Some(p) => Statement::AddCheck { table: t, index: table.checks.len(), check: p },
                None => continue,
            },
            // Its last check or one of its named constraints (half the
            // time each when it has both); of another table with either,
            // three quarters of the time, when this one has neither: a
            // constraint that exists.
            17 => {
                let drops = |o: usize| draw_drop_constraint(m, o);
                let o = if drops(t) > 0 {
                    t
                } else {
                    let others: Vec<usize> = (0..m.tables.len()).filter(|&o| drops(o) > 0).collect();
                    if others.is_empty() || !g.bool_with(0.75) {
                        continue;
                    }
                    *g.pick(&others)
                };
                let ot = &m.tables[o];
                let names: Vec<&str> = ot.names.iter().map(|n| n.name.as_str()).filter(|n| semantics::droppable_constraint(m, o, ot, n)).collect();
                if names.is_empty() || (!ot.checks.is_empty() && g.bool_with(0.5)) {
                    Statement::DropCheck { table: o, check: ot.checks.len() - 1 }
                } else {
                    Statement::DropConstraint { table: o, name: g.pick(&names).to_string() }
                }
            },
            // A column renamed and back: three quarters of the time one
            // the schema names (by how much, `semantics::column_refs`).
            31 => {
                let named: Vec<(u32, usize)> = (0..table.columns).filter(|&c| semantics::renamable(m, t, c)).map(|c| (semantics::column_refs(m, t, c) as u32, c)).filter(|&(w, _)| w > 0).collect();
                let c = if !named.is_empty() && g.bool_with(0.75) { *g.weighted(&named) } else { g.index(table.columns) };
                if !semantics::renamable(m, t, c) {
                    continue;
                }
                Statement::RenameColumn { table: t, column: c }
            },
            18 if m.tables.len() > 1 => {
                let b = (t + 1 + g.index(m.tables.len() - 1)) % m.tables.len();
                Statement::SwapTables { a: t, b, indexes: (table.indexes.clone(), m.tables[b].indexes.clone()) }
            },
            18 => continue,
            // Foreign keys off now and then (on by default), then on again.
            19 => Statement::ForeignKeys(!m.foreign_keys || g.bool_with(0.2)),
            20 => Statement::FkCheck(g.bool_with(0.3).then_some(t)),
            21 => match draw_rule(g, m, t) {
                Some(r) => Statement::CreateRule(r),
                None => continue,
            },
            22 => Statement::DropRule(g.pick(&m.rules).id),
            23 => Statement::RecursiveTriggers(!m.recursive_triggers),
            24 => match draw_view_rule(g, m) {
                Some(r) => Statement::CreateRule(r),
                None => continue,
            },
            25 => match draw_view_write(g, m) {
                Some(s) => s,
                None => continue,
            },
            // REINDEX of a table with indexes: of it, of one of its indexes,
            // of a collating sequence one of its columns or index columns
            // has, of everything.
            26 => Statement::Reindex(match g.int(0..10) {
                0..=3 => Reindex::Table(t),
                4..=6 => Reindex::Index(t, g.index(table.indexes.len())),
                7 | 8 => {
                    let colls: Vec<Coll> = table.coll.iter().flatten().copied().chain(table.indexes.iter().flat_map(|i| i.coll.iter().map(|&(_, k)| k))).collect();
                    Reindex::Coll(if colls.is_empty() { *g.pick(&Coll::ALL) } else { *g.pick(&colls) })
                },
                _ => Reindex::All,
            }),
            27 => Statement::Attach(g.bool_with(0.3)),
            28 => Statement::Detach,
            29 => Statement::AnalyzeDb(if m.session.aux && g.bool_with(0.6) { Db::Aux } else if g.bool() { Db::Temp } else { Db::Main }),
            // A pragma of the table's schema, of the table itself half the
            // time when it takes one.
            30 => {
                let k = g.index(crate::statements::DB_PRAGMAS.len());
                let tabled = crate::statements::DB_PRAGMAS[k].1;
                let db = if tabled { table.db } else if m.session.aux && g.bool() { Db::Aux } else { *g.pick(&[Db::Main, Db::Temp]) };
                Statement::DbPragma { db, k, table: tabled.then_some(t) }
            },
            _ => Statement::Count { table: t, filter: g.bool_with(0.3).then(|| draw_pred(g, table, 2)) },
        };
        if semantics::valid(m, &s) {
            return s;
        }
    }
    Statement::Select(Select { table: g.index(m.tables.len()), columns: Vec::new(), filter: None, order: Vec::new(), limit: None })
}

/// A storage statement valid against `m` (see [`Statement::JournalMode`]
/// and those after it): a setting the statements after it write or read
/// under, a checkpoint after WAL was asked for, a copy of the database.
/// Those that are invalid inside a transaction only outside one.
pub fn draw_storage(g: &mut Source, m: &Model) -> Statement {
    // Inside a transaction only the settings that change nothing there
    // but what later pages do.
    let open = !m.savepoints.is_empty();
    let out = |b: bool| u32::from(!open || b);
    let kind = *g.weighted(&[
        (out(false) * 35, 0u8),
        (out(false) * 18, 1),
        (out(false) * u32::from(semantics::valid(m, &Statement::TempStore(1))) * 10, 2),
        (8, 3),
        (8, 4),
        (6, 5),
        (4, 6),
        (5, 7),
        (3, 8),
        (2, 9),
        (out(false) * 8, 10),
        (out(false) * u32::from(m.session.wal) * 12, 11),
        (6, 12),
        (out(false) * 6, 13),
        (3, 14),
    ]);
    let s = match kind {
        0 => Statement::JournalMode(*g.weighted(&[(3, Journal::Delete), (2, Journal::Truncate), (3, Journal::Persist), (1, Journal::Memory), (1, Journal::Off), (1, Journal::Wal)])),
        1 => Statement::AutoVacuum(*g.weighted(&[(3, AutoVac::Full), (3, AutoVac::Incremental), (1, AutoVac::None)])),
        2 => Statement::TempStore(*g.weighted(&[(4, 1), (1, 2), (1, 0)])),
        3 => Statement::MmapSize(*g.pick(&[4096, 65536, 1 << 20, 1 << 26])),
        4 => Statement::LockingMode(!m.session.exclusive),
        5 => Statement::SecureDelete(*g.pick(&[1, 2, 2, 0])),
        6 => Statement::CellSizeCheck(true),
        7 => Statement::CacheSpill(*g.pick(&[0, 1, 2, 5, 20])),
        8 => Statement::JournalSizeLimit(*g.pick(&[0, 512, 4096, -1])),
        9 => Statement::WalAutocheckpoint(*g.pick(&[1, 4, 100])),
        10 => Statement::VacuumInto(g.bool_with(0.3)),
        // A small cache, for the writes after it to spill to the journal.
        12 => Statement::CacheSize(*g.pick(&[1, 2, 5, 10])),
        13 => Statement::Synchronous(*g.weighted(&[(2, 0), (2, 1), (1, 2), (2, 3)])),
        14 => Statement::ShrinkMemory,
        _ => Statement::WalCheckpoint(g.bool_with(0.8).then(|| *g.pick(&Checkpoint::ALL))),
    };
    debug_assert!(semantics::valid(m, &s), "{s}");
    s
}

/// Masks of `PRAGMA optimize` (see [`Statement::Optimize`]): the default,
/// every table, with the small limit, reporting only (the same decisions
/// without the cost of the ANALYZE they report), nothing.
const OPTIMIZE_MASKS: [u32; 7] = [0xfffe, 0x10003, 0x10013, 0x3, 0x10012, 0x12, 0x10000];

/// Crafted statistics of some of `m`'s tables (each 60% of the time, at
/// least one).
fn draw_final_stats(g: &mut Source, m: &Model) -> Statement {
    let mut rows = Vec::new();
    for (k, t) in m.tables.iter().enumerate() {
        if g.bool_with(0.6) {
            rows.extend(draw_stats(g, t, k));
        }
    }
    if rows.is_empty() {
        let k = g.index(m.tables.len());
        rows = draw_stats(g, &m.tables[k], k);
    }
    Statement::Stats(rows)
}

/// A row count for crafted statistics of a table of `rows` rows: a big
/// table's, a tiny one's, a multiple or a tenth of the true count, or any.
fn draw_row_count(g: &mut Source, rows: usize) -> u64 {
    let rows = rows as u64;
    match g.int(0..8) {
        0..=2 => *g.pick(&[10_000, 100_000, 1_000_000, 50_000_000, 1 << 32, 1_000_000_000_000]),
        3..=4 => *g.pick(&[0, 1, 2, 3, 5, 8]),
        5 => rows / 10,
        6 => rows.saturating_mul(*g.pick(&[1, 2, 10, 100, 1000])).max(1),
        _ => g.int(1..=1_000_000),
    }
}

/// Crafted `sqlite_stat1` rows of table `k` (see [`Stat`]): its own row
/// (always without indexes), some of its indexes' and its primary key's,
/// mostly for one row count. Per column prefix, rows per distinct value
/// that make a lookup selective, unselective or neither, now and then
/// inconsistent (zero, more than the rows, larger than the shorter
/// prefix's); now and then the flags SQLite reads after the numbers.
fn draw_stats(g: &mut Source, t: &Table, k: usize) -> Vec<Stat> {
    let mut targets = Vec::new();
    if t.indexes.is_empty() || g.bool_with(0.3) {
        targets.push(StatOf::Table);
    }
    for i in 0..t.indexes.len() {
        if g.bool_with(0.6) {
            targets.push(StatOf::Index(i));
        }
    }
    if matches!(t.key, Key::WithoutRowid(_)) && g.bool_with(0.5) {
        targets.push(StatOf::PrimaryKey);
    }
    if targets.is_empty() {
        targets.push(StatOf::Index(g.index(t.indexes.len())));
    }
    let n = draw_row_count(g, t.rows.len());
    targets
        .into_iter()
        .map(|of| {
            let columns = match (of, &t.key) {
                (StatOf::Index(i), _) => t.indexes[i].columns.len(),
                (StatOf::PrimaryKey, Key::WithoutRowid(pk)) => pk.len(),
                _ => 0,
            };
            let n = if g.bool_with(0.85) { n } else { draw_row_count(g, t.rows.len()) };
            let mut counts = vec![n];
            let mut last = n.max(1);
            let given = if g.bool_with(0.85) { columns } else { g.int(0..=columns) };
            for _ in 0..given {
                let e = match g.int(0..10) {
                    0..=3 => g.int(1..=2),
                    4..=5 => (last / *g.pick(&[2, 10])).max(1),
                    6..=7 => last.min(*g.pick(&[10, 20, 100, 1000])),
                    8 => last,
                    _ => *g.pick(&[0, last.saturating_mul(10), u64::from(u32::MAX)]),
                };
                counts.push(e);
                last = e.max(1);
            }
            let flags = of != StatOf::Table;
            Stat {
                table: k,
                of,
                counts,
                unordered: flags && g.bool_with(0.08),
                noskipscan: flags && g.bool_with(0.08),
                size: g.bool_with(0.15).then(|| *g.pick(&[2, 8, 40, 200, 1000, 10000])),
                drop_samples: g.bool_with(0.3),
            }
        })
        .collect()
}

/// The parent keys of table `p` a foreign key may name (see
/// [`semantics::parent_unique`]): its INTEGER PRIMARY KEY, its WITHOUT
/// ROWID primary key, the columns of its UNIQUE indexes.
fn parent_keys(p: &Table) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = match &p.key {
        Key::IntegerPrimaryKey(k) => vec![vec![*k]],
        Key::WithoutRowid(pk) => vec![pk.clone()],
        Key::Rowid => Vec::new(),
    };
    for i in &p.indexes {
        let cols: Vec<usize> = i.columns.iter().map(|&(c, _)| c).collect();
        if semantics::parent_unique(&Table { indexes: vec![i.clone()], rows: Vec::new(), key: Key::Rowid, ..p.clone_shape() }, &cols) {
            out.push(cols);
        }
    }
    out
}

/// Column `c` of `child` declared as parent column `pc` of `p` is, as
/// far as a foreign key cares (affinity, collating sequence): a STRICT
/// type or declared type of the same affinity; `None` when there is none
/// (NUMERIC in a STRICT table).
fn mirror(child: &mut Table, c: usize, p: &Table, pc: usize) -> Option<()> {
    let aff = affinity(p, pc);
    match &mut child.strict {
        Some(types) => {
            types[c] = match (&p.strict, aff) {
                (Some(pt), _) if !matches!(p.key, Key::IntegerPrimaryKey(k) if k == pc) => pt[pc],
                (_, Affinity::Integer) => ColType::Integer,
                (_, Affinity::Real) => ColType::Real,
                (_, Affinity::Text) => ColType::Text,
                (_, Affinity::None) => ColType::Any,
                (_, Affinity::Numeric) => return None,
            };
        },
        None => {
            child.decl[c] = match (&p.strict, aff) {
                (None, _) if !matches!(p.key, Key::IntegerPrimaryKey(k) if k == pc) => p.decl[pc],
                (_, Affinity::Integer) => Some(DECLS.iter().position(|d| d.0 == "INTEGER").unwrap()),
                (_, Affinity::Real) => Some(DECLS.iter().position(|d| d.0 == "REAL").unwrap()),
                (_, Affinity::Text) => Some(DECLS.iter().position(|d| d.0 == "TEXT").unwrap()),
                (_, Affinity::None) => None,
                (_, Affinity::Numeric) => Some(DECLS.iter().position(|d| d.0 == "NUMERIC").unwrap()),
            };
        },
    }
    child.coll[c] = p.coll[pc];
    (affinity(child, c) == aff).then_some(())
}

/// Actions for a foreign key, NO ACTION and CASCADE the most.
fn draw_action(g: &mut Source) -> Action {
    *g.weighted(&[(3, Action::NoAction), (1, Action::Restrict), (2, Action::SetNull), (1, Action::SetDefault), (3, Action::Cascade)])
}

/// A foreign key of `child` over its columns `cols` to parent key `pk` of
/// table `parent`, the columns declared to match.
fn draw_fk(g: &mut Source, m: &Model, child: &mut Table, cols: Vec<usize>, parent: usize, pk: Vec<usize>) -> Option<ForeignKey> {
    let mut probe = child.clone();
    for (&c, &pc) in cols.iter().zip(&pk) {
        mirror(&mut probe, c, &m.tables[parent], pc)?;
    }
    let f = ForeignKey { columns: cols, parent, parent_columns: pk, on_delete: draw_action(g), on_update: draw_action(g), deferred: g.bool_with(0.35) };
    semantics::fk_ok(m, &probe, &f).then(|| {
        *child = probe;
        f
    })
}

/// A new empty table's definition: 80% of the time (when there are
/// tables) with a foreign key to one of them, now and then two; in `db`
/// when given, else in its parent's schema (REFERENCES names a table of
/// the child's), and without a parent in `aux` half the time while it is
/// attached, TEMP a quarter of the time: the statements and queries after
/// it read and write it as any other (its name unqualified resolves to it,
/// `sqlite3LocateTable` searching every schema).
fn draw_new_table(g: &mut Source, m: &Model, db: Option<Db>) -> Table {
    let mut t = draw_plain_table(g);
    // `PRIMARY KEY .. ON CONFLICT x` a fifth of the time.
    if t.key != Key::Rowid && g.bool_with(0.2) {
        t.key_conflict = Some(*g.weighted(&[(3, Resolve::Replace), (3, Resolve::Ignore), (1, Resolve::Abort), (1, Resolve::Fail), (1, Resolve::Rollback)]));
    }
    let same: Vec<usize> = (0..m.tables.len()).filter(|&k| db.is_none_or(|d| m.tables[k].db == d)).collect();
    if !same.is_empty() && g.bool_with(0.8) {
        for _ in 0..if g.bool_with(0.3) { 2 } else { 1 } {
            let parent = if same.len() == m.tables.len() { g.index(m.tables.len()) } else { *g.pick(&same) };
            let keys = parent_keys(&m.tables[parent]);
            if keys.is_empty() {
                continue;
            }
            let pk = g.pick(&keys).clone();
            let mut free: Vec<usize> = (0..t.columns).filter(|&c| !is_key(&t, c) && !t.fks.iter().any(|f| f.columns.contains(&c))).collect();
            if free.len() < pk.len() {
                continue;
            }
            let cols: Vec<usize> = (0..pk.len()).map(|_| free.remove(g.index(free.len()))).collect();
            if t.fks.is_empty() {
                t.db = m.tables[parent].db;
            }
            if let Some(f) = draw_fk(g, m, &mut t, cols, parent, pk) {
                t.fks.push(f);
            }
        }
    }
    t.db = match db {
        Some(d) => d,
        None if !t.fks.is_empty() => t.db,
        None if m.session.aux && g.bool() => Db::Aux,
        None if g.bool_with(0.25) => Db::Temp,
        None => Db::Main,
    };
    t
}

fn draw_plain_table(g: &mut Source) -> Table {
    let columns = g.int(1..=4);
    let key = crate::files::draw_key(g, columns);
    let mut t = Table::new(columns, key.clone());
    if g.bool_with(0.3) {
        t.strict = Some(crate::files::draw_types(g, &t));
    } else if g.bool_with(crate::files::DECLARED) {
        t.decl = crate::files::draw_decls(g, &t);
    }
    if g.bool_with(crate::files::COLLATED) {
        t.coll = crate::files::draw_colls(g, &t);
    }
    for c in 0..columns {
        t.not_null[c] = !matches!(key, Key::IntegerPrimaryKey(k) if k == c) && g.bool_with(0.15);
    }
    t
}

/// An index for `CREATE INDEX` (ascending: see [`semantics::valid`]): 30%
/// of the time a partial one, its predicate `c IS NOT NULL` (implied by a
/// query's comparison of `c`) or a predicate as statements draw them
/// (comparisons with values the table holds); 40% of the time an
/// expression index ([`KeyExpr`]); some both; a plain one UNIQUE 30% of
/// the time.
fn draw_new_index(g: &mut Source, t: &Table) -> Index {
    let mut def = crate::files::draw_index(g, t.columns);
    def.columns.iter_mut().for_each(|c| c.1 = false);
    if g.bool_with(0.3) {
        def.filter = Some(if g.bool_with(0.45) { Pred::IsNull(g.index(t.columns), true) } else { draw_pred(g, t, 2) });
    }
    // 40% of the time over the sixteen forms: each of the twelve date,
    // arithmetic and case forms as often as at 30% over them alone.
    if g.bool_with(0.4) {
        def.expr = Some(*g.pick(&KeyExpr::ALL));
    }
    def.coll = crate::files::draw_index_colls(g, &def);
    // A plain one UNIQUE now and then (valid only on rows that allow it).
    def.unique = def.expr.is_none() && def.filter.is_none() && g.bool_with(0.3);
    def
}

/// A column for `ALTER TABLE .. ADD COLUMN` (see [`NewColumn`]): a type
/// (in a STRICT table always), now and then a `COLLATE`, a default 60% of
/// the time (a value as writes draw them, of the column's type), NOT NULL
/// with a default now and then.
fn draw_new_column(g: &mut Source, m: &Model, k: usize) -> NewColumn {
    let t = &m.tables[k];
    let mut d = NewColumn { ty: None, decl: None, coll: None, not_null: false, default: None, references: None, generated: None };
    // A generated column 15% of the time (not in a STRICT table): a copy
    // of a column, now and then collated.
    if t.strict.is_none() && g.bool_with(0.15) {
        let src = g.index(t.columns);
        if t.generated[src].is_none() {
            d.generated = Some((src, g.bool()));
            if g.bool_with(0.25) {
                d.coll = Some(*g.pick(&crate::model::Coll::ALL));
            }
            return d;
        }
    }
    // A REFERENCES column 20% of the time, without a default.
    if g.bool_with(0.3) && t.columns < crate::model::MAX_COLUMNS {
        let parent = g.index(m.tables.len());
        let keys: Vec<Vec<usize>> = parent_keys(&m.tables[parent]).into_iter().filter(|k| k.len() == 1).collect();
        if parent != k && !keys.is_empty() && !semantics::fk_reaches(m, parent, k) {
            let mut probe = Table { rows: Vec::new(), indexes: Vec::new(), ..t.clone_shape() };
            probe.columns += 1;
            probe.not_null.push(false);
            probe.decl.push(None);
            probe.coll.push(None);
            probe.default.push(None);
            probe.generated.push(None);
            if let Some(types) = &mut probe.strict {
                types.push(ColType::Any);
            }
            let pk = g.pick(&keys).clone();
            if let Some(f) = draw_fk(g, m, &mut probe, vec![t.columns], parent, pk) {
                d.ty = probe.strict.as_ref().map(|ty| ty[t.columns]);
                d.decl = probe.decl[t.columns];
                d.coll = probe.coll[t.columns];
                d.references = Some(f);
                return d;
            }
        }
    }
    if t.strict.is_some() {
        d.ty = Some(*g.pick(&ColType::ALL));
    } else if g.bool_with(0.6) {
        d.decl = Some(g.index(crate::model::DECLS.len()));
    }
    if g.bool_with(0.25) {
        d.coll = Some(*g.pick(&crate::model::Coll::ALL));
    }
    if g.bool_with(0.6) {
        let probe = semantics::new_column_table(t, &d);
        let v = draw_write(g, &probe, 0);
        if semantics::constant_default(&v) && semantics::default_ok(m.encoding, t, &v) {
            d.default = Some(v);
        }
    }
    let valued = d.default.as_ref().is_some_and(|v| *v != Value::Null);
    d.not_null = (valued && g.bool_with(0.3)) || (t.rows.is_empty() && g.bool_with(0.1));
    d
}

/// A predicate for `ADD CONSTRAINT .. CHECK`, one no row makes false: as
/// statements draw them, or its negation.
fn draw_check_constraint(g: &mut Source, t: &Table, enc: Encoding) -> Option<Pred> {
    for _ in 0..4 {
        let p = draw_pred(g, t, 2);
        for p in [p.clone(), Pred::Not(Box::new(p))] {
            if t.rows.iter().all(|r| semantics::eval(t, r, &p, enc) != Some(false)) {
                return Some(p);
            }
        }
    }
    None
}

/// An integrity check of the whole database or of one table.
fn draw_check(g: &mut Source, m: &Model) -> Statement {
    let table = (!m.tables.is_empty() && g.bool_with(0.3)).then(|| g.index(m.tables.len()));
    Statement::Check { quick: g.bool_with(0.3), table }
}

fn draw_select(g: &mut Source, t: &Table, k: usize) -> Select {
    let columns = if g.bool_with(0.4) { Vec::new() } else { (0..g.int(1..=3)).map(|_| g.index(t.columns)).collect() };
    let filter = g.bool_with(0.8).then(|| draw_pred(g, t, 3));
    let mut order = Vec::new();
    if g.bool_with(0.4) {
        let mut cols: Vec<usize> = (0..t.columns).collect();
        for _ in 0..g.int(1..=t.columns.min(2)) {
            order.push((cols.remove(g.index(cols.len())), g.bool_with(0.3)));
        }
    }
    let limit = g.bool_with(if order.is_empty() { 0.08 } else { 0.35 }).then(|| *g.pick(&[0, 1, 2, 3, 5, 10]));
    Select { table: k, columns, filter, order, limit }
}

/// The columns an index or the key leads with: where a lookup can seek.
fn leading_columns(t: &Table) -> Vec<usize> {
    let mut out: Vec<usize> = t.indexes.iter().filter(|i| i.expr.is_none()).map(|i| i.columns[0].0).collect();
    match &t.key {
        Key::IntegerPrimaryKey(c) => out.push(*c),
        Key::WithoutRowid(pk) => out.push(pk[0]),
        Key::Rowid => {},
    }
    out
}

fn draw_pred(g: &mut Source, t: &Table, depth: usize) -> Pred {
    if depth > 1 && g.bool_with(0.35) {
        let a = Box::new(draw_pred(g, t, depth - 1));
        return match g.int(0..5) {
            0 | 1 => Pred::And(a, Box::new(draw_pred(g, t, depth - 1))),
            2 | 3 => Pred::Or(a, Box::new(draw_pred(g, t, depth - 1))),
            _ => Pred::Not(a),
        };
    }
    let leading = leading_columns(t);
    let c = if !leading.is_empty() && g.bool_with(0.5) { *g.pick(&leading) } else { g.index(t.columns) };
    match g.int(0..20) {
        0..=13 => Pred::Cmp(c, *g.pick(&CmpOp::ALL), draw_literal(g, t, c)),
        14..=16 => {
            let (mut lo, mut hi) = (draw_literal(g, t, c), draw_literal(g, t, c));
            if g.bool_with(0.8) && crate::model::compare(&lo, &hi, Encoding::Utf8).is_gt() {
                std::mem::swap(&mut lo, &mut hi);
            }
            Pred::Between(c, lo, hi)
        },
        _ => Pred::IsNull(c, g.bool()),
    }
}

/// A literal to compare column `c` with: mostly a value in the column, so
/// that lookups hit; sometimes a neighbour of one, or another value.
fn draw_literal(g: &mut Source, t: &Table, c: usize) -> Value {
    let present = (!t.rows.is_empty()).then(|| column_value(t, &t.rows[g.index(t.rows.len())], c));
    let v = match (present, g.int(0..10)) {
        (Some(Value::Text(s)), 0..=2) if crate::model::column_coll(t, c) != crate::model::Coll::Binary => collated_twin(g, &s),
        (Some(v), 0..=5) => v,
        (Some(v), 6..=7) => neighbour(g, &v),
        _ => fresh_for(g, affinity(t, c)),
    };
    if comparable(t, c, &v) {
        return v;
    }
    (0..3).map(|_| fresh_for(g, affinity(t, c))).find(|v| comparable(t, c, v)).unwrap_or(Value::Null)
}

const ALPHABET: [char; 10] = ['a', 'b', 'z', 'A', '0', ' ', 'é', 'ж', '中', '😀'];

fn short_text(g: &mut Source) -> String {
    (0..g.int(0..=4)).map(|_| *g.pick(&ALPHABET)).collect()
}

fn small_int(g: &mut Source) -> i64 {
    match g.int(0..10) {
        0..=6 => g.int(-3..=3),
        7..=8 => g.int(-100_000..=100_000),
        _ => *g.pick(&[i64::MIN, i64::MAX, 1 << 53, -(1 << 31)]),
    }
}

fn small_real(g: &mut Source) -> f64 {
    match g.int(0..6) {
        0..=2 => f64::from(g.int(-50i32..=50)) / 4.0,
        3 => g.f64() * 100.0 - 50.0,
        4 => *g.pick(&[-0.0, 1e300, -1e-300, 5e-324, f64::INFINITY, f64::NEG_INFINITY, 9.223_372_036_854_776e18]),
        _ => {
            let x = g.f64_bits();
            if x.is_nan() {
                0.25
            } else {
                x
            }
        },
    }
}

/// A value a column of this affinity compares with; NULL now and then; a
/// time value ([`crate::files::draw_date`]) now and then; for a column with
/// an affinity, a quarter of the time one of a class it converts
/// ([`converted`]).
pub(crate) fn fresh_for(g: &mut Source, aff: Affinity) -> Value {
    if g.bool_with(0.03) {
        return Value::Null;
    }
    if aff != Affinity::None && g.bool_with(0.25) {
        return converted(g, aff);
    }
    if !aff.numeric() && g.bool_with(0.15) {
        match crate::files::draw_date(g) {
            v @ Value::Text(_) => return v,
            v if aff == Affinity::None => return v,
            _ => {},
        }
    }
    let class = match aff {
        Affinity::Integer | Affinity::Real | Affinity::Numeric => g.int(0..2),
        Affinity::Text => 2,
        Affinity::None => g.int(0..4),
    };
    match class {
        0 => Value::Int(small_int(g)),
        1 => Value::Real(small_real(g)),
        2 => Value::Text(short_text(g)),
        _ => Value::Blob((0..g.int(0..=4)).map(|_| g.int(0u8..=255)).collect()),
    }
}

/// A value of a class an affinity converts, or compares across, as the
/// model follows it ([`crate::model::store`], [`semantics::comparable`]):
/// under a numeric affinity mostly text that reads as a number
/// ([`number_text`]), else text that does not, or a blob; under TEXT an
/// integer or a real it renders, or a blob.
pub(crate) fn converted(g: &mut Source, aff: Affinity) -> Value {
    if aff.numeric() {
        match g.int(0..10) {
            0..=5 => Value::Text(number_text(g)),
            6..=8 => Value::Text(short_text(g)),
            _ => Value::Blob((0..g.int(0..=4)).map(|_| g.int(0u8..=255)).collect()),
        }
    } else {
        match g.int(0..10) {
            0..=4 => Value::Int(small_int(g)),
            5..=8 => Value::Real(f64::from(g.int(-4000i32..=4000)) / *g.pick(&[1.0, 2.0, 8.0, 64.0])),
            _ => Value::Blob((0..g.int(0..=4)).map(|_| g.int(0u8..=255)).collect()),
        }
    }
}

/// Text that SQLite reads as a number under a numeric affinity, in the
/// forms its reader takes (`sqlite3AtoF`): plain and signed integers,
/// leading zeros, whitespace around, decimals with the point first or
/// last, exponents; and a few that it reads as no number (`0x10`, `1e`,
/// `12abc`, `- 5`), which stay text.
pub(crate) fn number_text(g: &mut Source) -> String {
    let i: i64 = if g.bool_with(0.8) { g.int(-300..=300) } else { g.int(-1_000_000_000_000..=1_000_000_000_000) };
    let x = f64::from(g.int(-4000i32..=4000)) / *g.pick(&[2.0, 4.0, 8.0, 10.0, 100.0]);
    match g.int(0..12) {
        0..=2 => i.to_string(),
        3 => format!("{}{i}{}", g.pick(&[" ", "\t", "  ", "\n"]), g.pick(&["", " ", "\r\n"])),
        4 => format!("+{}", i.abs()),
        5 => format!("{i}.0"),
        6 => format!("{x}"),
        7 => format!("{i}e{}", g.int(-3..=3)),
        8 => format!("{}{}", g.pick(&["", "-", "+"]), g.pick(&[".5", "5.", "0.", ".25e1", "1.5E+2", "00.0", "007", "-0", "0e0"])),
        9 => format!("{x:e}"),
        _ => g.pick(&["0x10", "1e", "12abc", "- 5", "1.2.3", "+", ".", "e5", "1e+", "١٢"]).to_string(),
    }
}

/// A text a collating sequence may take for `s`: its ASCII letters in
/// the other case, or trailing spaces added or taken away (equal to `s`
/// under NOCASE or RTRIM, apart under BINARY).
fn collated_twin(g: &mut Source, s: &str) -> Value {
    Value::Text(match g.int(0..4) {
        0 => s.to_ascii_uppercase(),
        1 => s.chars().map(|ch| if ch.is_ascii_uppercase() { ch.to_ascii_lowercase() } else { ch.to_ascii_uppercase() }).collect(),
        2 => format!("{s}{}", g.pick(&[" ", "  ", "   "])),
        _ => s.trim_end_matches(' ').to_string(),
    })
}

/// A value next to `v` in SQLite's order, or of a neighbouring class.
fn neighbour(g: &mut Source, v: &Value) -> Value {
    match v {
        Value::Null => Value::Int(0),
        Value::Int(i) => match g.int(0..3) {
            0 => Value::Int(i.saturating_add(1)),
            1 => Value::Int(i.saturating_sub(1)),
            _ => Value::Real(*i as f64 + 0.5),
        },
        Value::Real(x) if x.is_finite() => match g.int(0..3) {
            0 => Value::Real(x.next_up()),
            1 => Value::Real(x.next_down()),
            _ => Value::Int(x.clamp(-9e18, 9e18) as i64),
        },
        Value::Real(x) => Value::Real(-x),
        Value::Text(s) => {
            let mut s = s.clone();
            if g.bool() || s.is_empty() {
                s.push(*g.pick(&ALPHABET));
            } else {
                s.pop();
            }
            Value::Text(s)
        },
        Value::Blob(b) => {
            let mut b = b.clone();
            if g.bool() || b.is_empty() {
                b.push(g.int(0u8..=255));
            } else {
                b.pop();
            }
            Value::Blob(b)
        },
    }
}

/// A value to write to column `c` (not a key the statement may not
/// write): mostly one already in the column, else a fresh value of a class
/// the column takes, now and then a long one that overflows a page.
fn draw_write(g: &mut Source, t: &Table, c: usize) -> Value {
    if !t.not_null[c] && g.bool_with(0.08) {
        return Value::Null;
    }
    let v = crate::model::not_now(if !t.rows.is_empty() && g.bool_with(0.4) {
        match t.rows[g.index(t.rows.len())].values[c].clone() {
            Value::Text(s) if crate::model::column_coll(t, c) != crate::model::Coll::Binary && g.bool() => collated_twin(g, &s),
            v => v,
        }
    } else if g.bool_with(0.05) {
        let n = g.int(100..=3000usize);
        let mut rng = Rng::new(g.int(..));
        match t.strict.as_ref().map(|ty| ty[c]) {
            Some(ColType::Blob) => Value::Blob((0..n).map(|_| rng.below(256) as u8).collect()),
            _ => Value::Text((0..n).map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize]).collect()),
        }
    } else {
        match t.strict.as_ref().map(|ty| ty[c]) {
            Some(ColType::Int | ColType::Integer) if g.bool_with(0.9) => Value::Int(small_int(g)),
            Some(ColType::Int | ColType::Integer) => Value::Real(f64::from(g.int(-9i32..=9))),
            Some(ColType::Real) if g.bool_with(0.8) => Value::Real(small_real(g)),
            Some(ColType::Real) => Value::Int(small_int(g)),
            Some(ColType::Text) => Value::Text(short_text(g)),
            Some(ColType::Blob) => Value::Blob((0..g.int(0..=4)).map(|_| g.int(0u8..=255)).collect()),
            // A declared type: half the time a value its affinity converts.
            None if affinity(t, c) != Affinity::None && g.bool() => converted(g, affinity(t, c)),
            _ => fresh_for(g, Affinity::None),
        }
    });
    if convert(t, c, &v).is_some() {
        return v;
    }
    match t.strict.as_ref().map(|ty| ty[c]) {
        Some(ColType::Real) => Value::Real(0.5),
        Some(ColType::Text) => Value::Text(String::new()),
        Some(ColType::Blob) => Value::Blob(Vec::new()),
        _ => Value::Int(0),
    }
}

/// A fresh rowid: next to the largest, or anywhere.
fn draw_rowid(g: &mut Source, t: &Table) -> Value {
    let top = t.rows.last().map_or(0, |r| r.rowid);
    match g.int(0..10) {
        0 => Value::Null,
        1..=6 if top < i64::MAX - 10 => Value::Int(top + g.int(1..=3)),
        7 => Value::Int(*g.pick(&[i64::MIN, i64::MAX, 0, -1])),
        _ => Value::Int(g.int(..)),
    }
}

/// A fresh value for a WITHOUT ROWID primary key column, unlikely to be
/// taken.
fn draw_key_value(g: &mut Source, t: &Table, c: usize) -> Value {
    if !t.rows.is_empty() && g.bool_with(0.15) {
        let r = g.index(t.rows.len());
        return match &t.rows[r].values[c] {
            Value::Text(s) if crate::model::column_coll(t, c) != crate::model::Coll::Binary && g.bool() => collated_twin(g, s),
            v => neighbour(g, v),
        };
    }
    let class = match t.strict.as_ref().map(|ty| ty[c]) {
        Some(ColType::Int | ColType::Integer) => 0,
        Some(ColType::Real) => 1,
        Some(ColType::Text) => 2,
        Some(ColType::Blob) => 3,
        _ => g.int(0..4),
    };
    let n: i64 = g.int(-1_000_000_000..=1_000_000_000);
    match class {
        0 => Value::Int(n),
        1 => Value::Real(n as f64 / 4.0 + 0.125),
        2 => Value::Text(format!("n{n}")),
        _ => Value::Blob(n.to_be_bytes().to_vec()),
    }
}

/// The key of a random row of foreign key `f`'s parent, for its columns.
fn parent_values(g: &mut Source, m: &Model, f: &ForeignKey) -> Option<Vec<Value>> {
    let p = &m.tables[f.parent];
    if p.rows.is_empty() {
        return None;
    }
    let r = &p.rows[g.index(p.rows.len())];
    Some(f.parent_columns.iter().map(|&c| column_value(p, r, c)).collect())
}

fn draw_insert(g: &mut Source, m: &Model, k: usize) -> Option<Insert> {
    let (t, enc) = (&m.tables[k], m.encoding);
    let ipk = match t.key {
        Key::IntegerPrimaryKey(c) => Some(c),
        _ => None,
    };
    // The required columns, then others, in a random order.
    let mut columns: Vec<usize> = Vec::new();
    let mut rest: Vec<usize> = (0..t.columns).collect();
    while !rest.is_empty() {
        let c = rest.remove(g.index(rest.len()));
        let fk = t.fks.iter().any(|f| f.columns.contains(&c));
        if t.generated[c].is_some() {
            continue;
        }
        if t.not_null[c] || (is_key(t, c) && ipk != Some(c)) || g.bool_with(if fk { 0.85 } else { 0.6 }) {
            columns.push(c);
        }
    }
    let full = t.rows.last().is_some_and(|r| r.rowid == i64::MAX);
    if ipk.is_some_and(|c| full && !columns.contains(&c)) {
        columns.push(ipk.unwrap());
    }
    let rowid = t.key == Key::Rowid && (full || g.bool_with(0.25));
    if columns.is_empty() && !rowid {
        columns.push(g.index(t.columns));
        columns.retain(|&c| t.generated[c].is_none());
        if columns.is_empty() {
            return None;
        }
    }
    let n = if g.bool_with(0.8) { g.int(1..=3) } else { g.int(4..=8) };
    let (or, upsert) = draw_conflict(g, t);
    let conflicting = or.is_some() || !upsert.is_empty() || t.key_conflict.is_some();
    let mut ins = Insert { table: k, rowid, columns: columns.clone(), rows: Vec::new(), or, upsert, returning: None, select: g.bool_with(0.12) };
    for _ in 0..n {
        let mut row = Vec::with_capacity(columns.len() + 1);
        if rowid {
            row.push(draw_rowid(g, t));
        }
        for &c in &columns {
            row.push(if Some(c) == ipk {
                match draw_rowid(g, t) {
                    Value::Null if t.not_null[c] => Value::Int(g.int(..)),
                    v => v,
                }
            } else if is_key(t, c) {
                // A neighbour of a key may read as the current time.
                crate::model::not_now(draw_key_value(g, t, c))
            } else {
                draw_write(g, t, c)
            });
        }
        // A foreign key's columns mostly hold a parent's key.
        for f in &t.fks {
            if g.bool_with(0.85) {
                if let Some(vals) = parent_values(g, m, f) {
                    for (&c, v) in f.columns.iter().zip(vals) {
                        if let Some(at) = columns.iter().position(|&x| x == c) {
                            row[at + usize::from(rowid)] = v;
                        }
                    }
                }
            }
        }
        // Under a conflict clause, a row with the key of a row there, or
        // its values in a UNIQUE index, half the time.
        if conflicting && !t.rows.is_empty() && g.bool() {
            let there = &t.rows[g.index(t.rows.len())];
            let cons = constraints(t);
            let cols = &cons[g.index(cons.len())];
            for &c in cols {
                if Some(c) == ipk || !is_key(t, c) || matches!(t.key, Key::WithoutRowid(_)) {
                    if let Some(at) = columns.iter().position(|&x| x == c) {
                        row[at + usize::from(rowid)] = column_value(t, there, c);
                    }
                }
            }
            if cols.is_empty() && rowid {
                row[0] = Value::Int(there.rowid);
            }
        }
        // Keep the rows that fit; drop one that conflicts.
        ins.rows.push(row);
        let fits = if conflicting {
            semantics::valid(m, &Statement::Insert(ins.clone()))
        } else {
            semantics::inserted_rows(t, &ins, enc).is_some() && semantics::fk_rows_ok(m, k, &ins)
        };
        if !fits {
            ins.rows.pop();
        }
    }
    ins.returning = draw_returning(g, t);
    (!ins.rows.is_empty()).then_some(ins)
}

/// The columns of each constraint an INSERT may conflict with: the key
/// (none for a rowid table without an INTEGER PRIMARY KEY: its rowid),
/// then each UNIQUE index.
fn constraints(t: &Table) -> Vec<Vec<usize>> {
    let key = match &t.key {
        Key::Rowid => Vec::new(),
        Key::IntegerPrimaryKey(c) => vec![*c],
        Key::WithoutRowid(pk) => pk.clone(),
    };
    std::iter::once(key).chain(t.indexes.iter().filter(|i| i.unique).map(|i| i.columns.iter().map(|&(c, _)| c).collect())).collect()
}

/// A value of column `c` of table `b` a trigger step writes: a column of
/// the row it sees 40% of the time (one that holds values the column
/// takes), else a literal.
fn draw_term(g: &mut Source, t: &Table, b: &Table, c: usize) -> Term {
    if g.bool_with(0.4) {
        let fit: Vec<usize> = (0..t.columns)
            .filter(|&r| t.generated[r].is_none() && !matches!(t.key, Key::IntegerPrimaryKey(k) if k == r) && t.rows.iter().take(8).all(|row| convert(b, c, &column_value(t, row, r)).is_some()))
            .collect();
        if !fit.is_empty() {
            return Term::Row(*g.pick(&fit));
        }
    }
    Term::Lit(draw_write(g, b, c))
}

/// A trigger on table `k` writing another table (see [`Rule`]).
/// `ALTER TABLE .. DROP COLUMN` of a column `droppable` (then the later
/// ones renamed one down): three quarters of the time one whose later
/// columns the schema names (weighted by how many objects name them,
/// [`semantics::column_refs`]), of `k` or, when none of `k`'s is, of
/// another table, so that the renames rewrite indexes, checks, views,
/// triggers and foreign keys; else any of `k`'s.
fn draw_drop_column(g: &mut Source, m: &Model, k: usize) -> Option<Statement> {
    let refs = |t: usize, c: usize| (c + 1..m.tables[t].columns).map(|r| semantics::column_refs(m, t, r)).sum::<usize>();
    let named = |t: usize| -> Vec<(u32, usize)> { (0..m.tables[t].columns).filter(|&c| semantics::droppable(m, t, c)).map(|c| (refs(t, c) as u32, c)).filter(|&(w, _)| w > 0).collect() };
    if g.bool_with(0.75) {
        let mut t = k;
        let mut cs = named(k);
        if cs.is_empty() {
            let others: Vec<usize> = (0..m.tables.len()).filter(|&o| o != k && !named(o).is_empty()).collect();
            if !others.is_empty() {
                t = *g.pick(&others);
                cs = named(t);
            }
        }
        if !cs.is_empty() {
            return Some(Statement::DropColumn { table: t, column: *g.weighted(&cs), columns: m.tables[t].columns });
        }
    }
    let cs: Vec<usize> = (0..m.tables[k].columns).filter(|&c| semantics::droppable(m, k, c)).collect();
    (!cs.is_empty()).then(|| Statement::DropColumn { table: k, column: *g.pick(&cs), columns: m.tables[k].columns })
}

fn draw_rule(g: &mut Source, m: &Model, k: usize) -> Option<Rule> {
    let t = &m.tables[k];
    let event = g.int(0u8..=2);
    let before = g.bool();
    let of = if event == 1 && g.bool_with(0.3) { (0..g.int(1..=2)).map(|_| g.index(t.columns)).collect::<Vec<_>>() } else { Vec::new() };
    let mut of = of;
    of.dedup();
    // Not on a table foreign keys link: the model leaves out its writes
    // (its steps may write one).
    let linked = |x: usize| !m.tables[x].fks.is_empty() || m.tables.iter().any(|o| o.fks.iter().any(|f| f.parent == x));
    if linked(k) {
        return None;
    }
    let others: Vec<usize> = (0..m.tables.len()).filter(|&b| b != k).collect();
    if others.is_empty() {
        return None;
    }
    let mut body = Vec::new();
    let mut when = g.bool_with(0.4).then(|| draw_pred(g, t, 2));
    for _ in 0..*g.weighted(&[(6, 1usize), (3, 2), (1, 3)]) {
        let bk = *g.pick(&others);
        let b = &m.tables[bk];
        let step = match *g.weighted(&[(35, 0u8), (25, 1), (20, 2), (if before { 10 } else { 0 }, 3), (5, 4), (5, 5)]) {
            0 => {
                // Required columns, then others; the rowid left to SQLite.
                let mut columns = Vec::new();
                for c in 0..b.columns {
                    if b.generated[c].is_some() || matches!(b.key, Key::IntegerPrimaryKey(x) if x == c) && !b.not_null[c] {
                        continue;
                    }
                    if b.not_null[c] || is_key(b, c) || g.bool_with(0.5) {
                        columns.push(c);
                    }
                }
                if columns.is_empty() {
                    continue;
                }
                let values = columns.iter().map(|&c| draw_term(g, t, b, c)).collect();
                let (or, upsert) = match g.int(0..10) {
                    0 | 1 => (Some(*g.pick(&[Resolve::Ignore, Resolve::Replace, Resolve::ReplaceInto, Resolve::Abort])), Vec::new()),
                    2 => {
                        let free: Vec<usize> = (0..b.columns).filter(|&c| !is_key(b, c) && b.generated[c].is_none()).collect();
                        let update = (!free.is_empty() && g.bool()).then(|| (vec![(*g.pick(&free), Excluded::Yes)], None));
                        (None, vec![Upsert { target: None, update }])
                    },
                    _ => (None, Vec::new()),
                };
                Step::Insert { table: bk, columns, values, or, upsert }
            },
            1 => {
                let mut free: Vec<usize> = (0..b.columns).filter(|&c| !is_key(b, c) && b.generated[c].is_none()).collect();
                if free.is_empty() {
                    continue;
                }
                let sets = (0..g.int(1..=free.len().min(2)))
                    .map(|_| {
                        let c = free.remove(g.index(free.len()));
                        (c, if g.bool_with(0.6) { Term::Lit(draw_write(g, b, c)) } else { draw_term(g, t, b, c) })
                    })
                    .collect();
                let or = g.bool_with(0.15).then(|| *g.pick(&[Resolve::Ignore, Resolve::Replace, Resolve::Fail]));
                Step::Update { table: bk, sets, filter: g.bool_with(0.8).then(|| draw_pred(g, b, 2)), or }
            },
            2 => Step::Delete { table: bk, filter: g.bool_with(0.85).then(|| draw_pred(g, b, 2)) },
            3 => {
                if when.is_none() && g.bool_with(0.8) {
                    when = Some(draw_pred(g, t, 2));
                }
                Step::Ignore
            },
            4 => {
                let c = g.index(t.columns);
                Step::Abort(Pred::Cmp(c, CmpOp::Eq, draw_write(g, t, c)))
            },
            _ => Step::Select(g.index(t.columns)),
        };
        body.push(step);
        if matches!(body.last(), Some(Step::Ignore)) {
            break;
        }
    }
    if body.is_empty() {
        return None;
    }
    // TEMP when the body writes a table of another schema (a trigger that
    // is not TEMP resolves its body's tables in its own).
    let temp = g.bool_with(0.2) || body.iter().any(|s| step_table(s).is_some_and(|b| m.tables[b].db != t.db));
    Some(Rule { id: semantics::next_rule_id(m), table: k, view: false, temp, db: t.db, before, event, of, when, body })
}

/// An INSTEAD OF trigger on a view statements may write, its steps
/// writing tables (the view's own half the time), with the view's row's
/// values for INSERT and DELETE.
fn draw_view_rule(g: &mut Source, m: &Model) -> Option<Rule> {
    let views: Vec<usize> = (0..m.views.len()).filter(|&v| semantics::writable_view(m, v).is_some()).collect();
    if views.is_empty() {
        return None;
    }
    let v = *g.pick(&views);
    let (k, cols) = semantics::writable_view(m, v)?;
    let cols = cols.to_vec();
    let t = &m.tables[k];
    let event = g.int(0u8..=2);
    let mut body = Vec::new();
    for _ in 0..*g.weighted(&[(7, 1usize), (3, 2)]) {
        let bk = if g.bool() { k } else { g.index(m.tables.len()) };
        let b = &m.tables[bk];
        let term = |g: &mut Source, c: usize| {
            let fit: Vec<usize> = cols.iter().copied().filter(|&r| t.generated[r].is_none() && !matches!(t.key, Key::IntegerPrimaryKey(x) if x == r) && b.columns > c).collect();
            if event != 1 && !fit.is_empty() && g.bool_with(0.6) {
                Term::Row(*g.pick(&fit))
            } else {
                Term::Lit(draw_write(g, b, c))
            }
        };
        let step = match g.int(0..3) {
            0 => {
                let mut columns = Vec::new();
                for c in 0..b.columns {
                    if b.generated[c].is_some() || matches!(b.key, Key::IntegerPrimaryKey(x) if x == c) && !b.not_null[c] {
                        continue;
                    }
                    if b.not_null[c] || is_key(b, c) || g.bool_with(0.6) {
                        columns.push(c);
                    }
                }
                if columns.is_empty() {
                    continue;
                }
                let values = columns.iter().map(|&c| term(g, c)).collect();
                Step::Insert { table: bk, columns, values, or: g.bool_with(0.2).then_some(Resolve::Ignore), upsert: Vec::new() }
            },
            1 => {
                let free: Vec<usize> = (0..b.columns).filter(|&c| !is_key(b, c) && b.generated[c].is_none()).collect();
                if free.is_empty() {
                    continue;
                }
                let c = *g.pick(&free);
                Step::Update { table: bk, sets: vec![(c, term(g, c))], filter: g.bool_with(0.8).then(|| draw_pred(g, b, 2)), or: None }
            },
            _ => Step::Delete { table: bk, filter: g.bool_with(0.9).then(|| draw_pred(g, b, 2)) },
        };
        body.push(step);
    }
    if body.is_empty() {
        return None;
    }
    let temp = g.bool_with(0.2) || body.iter().any(|s| step_table(s).is_some_and(|b| m.tables[b].db != Db::Main));
    Some(Rule { id: semantics::next_rule_id(m), table: v, view: true, temp, db: Db::Main, before: false, event, of: Vec::new(), when: None, body })
}

/// The table a trigger step writes, if it writes one.
fn step_table(s: &Step) -> Option<usize> {
    match s {
        Step::Insert { table, .. } | Step::Update { table, .. } | Step::Delete { table, .. } => Some(*table),
        _ => None,
    }
}

/// An INSERT, UPDATE or DELETE of a view an INSTEAD OF trigger carries.
fn draw_view_write(g: &mut Source, m: &Model) -> Option<Statement> {
    let rules: Vec<&Rule> = m.rules.iter().filter(|r| r.view).collect();
    if rules.is_empty() {
        return None;
    }
    let r = *g.pick(&rules);
    let (k, cols) = semantics::writable_view(m, r.table)?;
    let t = &m.tables[k];
    let mut cols = cols.to_vec();
    let write = match r.event {
        0 => {
            let mut columns = Vec::new();
            for _ in 0..g.int(1..=cols.len()) {
                columns.push(cols.remove(g.index(cols.len())));
            }
            let rows = (0..*g.weighted(&[(6, 1usize), (3, 2), (1, 3)])).map(|_| columns.iter().map(|&c| draw_write(g, t, c)).collect()).collect();
            Statement::Insert(Insert { table: k, columns, rows, ..Default::default() })
        },
        1 => {
            let c = *g.pick(&cols);
            let filter = g.bool_with(0.85).then(|| Pred::Cmp(*g.pick(&cols), CmpOp::Eq, draw_write(g, t, c)));
            Statement::Update(Update { table: k, sets: vec![(c, draw_write(g, t, c))], filter, ..Default::default() })
        },
        _ => {
            let c = *g.pick(&cols);
            let filter = g.bool_with(0.9).then(|| if g.bool() { Pred::Cmp(c, CmpOp::Eq, draw_write(g, t, c)) } else { draw_pred(g, t, 2) });
            Statement::Delete(Delete { table: k, filter, returning: None })
        },
    };
    Some(Statement::ViewWrite { view: r.table, write: Box::new(write) })
}

/// `RETURNING` 15% of the time: `*`, or some columns.
fn draw_returning(g: &mut Source, t: &Table) -> Option<Vec<usize>> {
    g.bool_with(0.15).then(|| if g.bool() { Vec::new() } else { (0..g.int(1..=3)).map(|_| g.index(t.columns)).collect() })
}

/// A conflict clause for an INSERT into `t` a quarter of the time: OR
/// IGNORE, OR REPLACE or REPLACE INTO, ON CONFLICT clauses, or (rarely,
/// as nothing may conflict under them) OR ABORT, FAIL, ROLLBACK.
fn draw_conflict(g: &mut Source, t: &Table) -> (Option<Resolve>, Vec<Upsert>) {
    if !g.bool_with(0.25) {
        return (None, Vec::new());
    }
    match g.int(0..10) {
        0..=1 => (Some(Resolve::Ignore), Vec::new()),
        2..=3 => (Some(if g.bool_with(0.7) { Resolve::Replace } else { Resolve::ReplaceInto }), Vec::new()),
        4 => (Some(*g.pick(&[Resolve::Abort, Resolve::Fail, Resolve::Rollback])), Vec::new()),
        _ => {
            let mut targets: Vec<Vec<usize>> = constraints(t).into_iter().filter(|cs| !cs.is_empty()).collect();
            let mut out = Vec::new();
            for _ in 0..g.int(1..=2) {
                let last = targets.is_empty() || g.bool_with(0.3);
                let target = (!last).then(|| {
                    let cs = targets.remove(g.index(targets.len()));
                    cs.into_iter().map(|c| (c, None)).collect()
                });
                let free: Vec<usize> = (0..t.columns).filter(|&c| !is_key(t, c) && t.generated[c].is_none()).collect();
                let update = (!free.is_empty() && g.bool_with(0.75)).then(|| {
                    let mut free = free.clone();
                    let sets = (0..g.int(1..=free.len().min(2)))
                        .map(|_| {
                            let c = free.remove(g.index(free.len()));
                            (c, if g.bool_with(0.6) { Excluded::Yes } else { Excluded::No(draw_write(g, t, c)) })
                        })
                        .collect();
                    (sets, g.bool_with(0.3).then(|| draw_pred(g, t, 2)))
                });
                out.push(Upsert { target, update });
                if last {
                    break;
                }
            }
            (None, out)
        },
    }
}

/// A parent key column of table `k` and a value of it a child row refers
/// to (by the first column of its foreign key).
fn referenced(g: &mut Source, m: &Model, k: usize) -> Option<(usize, Value)> {
    let links: Vec<(usize, usize)> = m.tables.iter().enumerate().flat_map(|(c, t)| t.fks.iter().enumerate().filter(|(_, f)| f.parent == k).map(move |(i, _)| (c, i))).collect();
    if links.is_empty() {
        return None;
    }
    let (c, i) = *g.pick(&links);
    let (child, f) = (&m.tables[c], &m.tables[c].fks[i]);
    let vals: Vec<Value> = child.rows.iter().map(|r| column_value(child, r, f.columns[0])).filter(|v| *v != Value::Null).collect();
    if vals.is_empty() {
        return None;
    }
    Some((f.parent_columns[0], g.pick(&vals).clone()))
}

fn draw_update(g: &mut Source, m: &Model, k: usize) -> Option<Update> {
    let t = &m.tables[k];
    // A parent key a child refers to set anew 40% of the time (a UNIQUE
    // index's column; key columns never change).
    if let Some((c, v)) = referenced(g, m, k).filter(|(c, _)| !is_key(t, *c) && g.bool_with(0.4)) {
        let to = if g.bool_with(0.15) { Value::Null } else { draw_write(g, t, c) };
        return Some(Update { table: k, sets: vec![(c, to)], filter: Some(Pred::Cmp(c, CmpOp::Eq, v)), ..Default::default() });
    }
    let mut free: Vec<usize> = (0..t.columns).filter(|&c| !is_key(t, c) && t.generated[c].is_none()).collect();
    if free.is_empty() {
        return None;
    }
    let mut sets = Vec::new();
    for _ in 0..g.int(1..=free.len().min(3)) {
        let c = free.remove(g.index(free.len()));
        // A column of a foreign key of one column mostly a parent's key.
        let parent = t.fks.iter().find(|f| f.columns == [c]).filter(|_| g.bool_with(0.7)).and_then(|f| parent_values(g, m, f));
        sets.push((c, parent.map_or_else(|| draw_write(g, t, c), |mut v| v.remove(0))));
    }
    let returning = draw_returning(g, t);
    let vector = draw_vector_set(g);
    let from = g.bool_with(0.2).then(|| draw_update_from(g, m, k, &mut sets)).flatten();
    Some(Update { table: k, sets, filter: (!g.bool_with(0.05)).then(|| draw_pred(g, t, 3)), or: draw_update_or(g), returning, vector, from })
}

/// `UPDATE .. FROM` one row of another rowid table (see
/// [`crate::statements::UpdateFrom`]): a row it holds 90% of the time, its
/// subquery a third of the time; each set reads one of the row's columns
/// half the time (at least one), its literal set to that value (what the
/// UPDATE writes, and what is left when shrinking drops the FROM).
fn draw_update_from(g: &mut Source, m: &Model, k: usize, sets: &mut [(usize, Value)]) -> Option<UpdateFrom> {
    let sources: Vec<usize> = (0..m.tables.len()).filter(|&j| j != k && !matches!(m.tables[j].key, Key::WithoutRowid(_))).collect();
    if sources.is_empty() {
        return None;
    }
    let j = *g.pick(&sources);
    let src = &m.tables[j];
    let row = (!src.rows.is_empty() && g.bool_with(0.9)).then(|| &src.rows[g.index(src.rows.len())]);
    let rowid = row.map_or_else(|| g.int(-1..=20), |r| r.rowid);
    let mut reads = Vec::new();
    let n = sets.len();
    for (i, set) in sets.iter_mut().enumerate() {
        if g.bool() || (i + 1 == n && reads.is_empty()) {
            let c = g.index(src.columns);
            if let Some(r) = row {
                set.1 = column_value(src, r, c);
            }
            reads.push((i, c));
        }
    }
    Some(UpdateFrom { table: j, rowid, reads, sub: g.bool_with(0.33) })
}

/// The form of an UPDATE's sets ([`Update::vector`]): a row value a
/// quarter of the time, a subquery of one row a quarter.
fn draw_vector_set(g: &mut Source) -> VectorSet {
    *g.weighted(&[(2, VectorSet::Plain), (1, VectorSet::Row), (1, VectorSet::Select)])
}

/// `UPDATE OR x` 12% of the time, mostly IGNORE or REPLACE.
fn draw_update_or(g: &mut Source) -> Option<Resolve> {
    g.bool_with(0.12).then(|| *g.weighted(&[(3, Resolve::Ignore), (4, Resolve::Replace), (1, Resolve::Abort), (1, Resolve::Fail), (1, Resolve::Rollback)]))
}

/// One row of `k` given the values of another in a UNIQUE index (`UPDATE
/// OR IGNORE|REPLACE` of it, which conflicts), the row picked by a column
/// it may alone hold that value in.
fn draw_clashing_update(g: &mut Source, m: &Model, k: usize) -> Option<Update> {
    let t = &m.tables[k];
    let uniques: Vec<&Index> = t.indexes.iter().filter(|i| i.unique && i.columns.iter().all(|&(c, _)| !is_key(t, c) && t.generated[c].is_none())).collect();
    if uniques.is_empty() || t.rows.len() < 2 {
        return None;
    }
    let i = uniques[g.index(uniques.len())];
    let (a, b) = (g.index(t.rows.len()), g.index(t.rows.len()));
    if a == b {
        return None;
    }
    let sets = i.columns.iter().map(|&(c, _)| (c, column_value(t, &t.rows[b], c))).collect();
    let c = g.index(t.columns);
    let filter = Some(Pred::Cmp(c, CmpOp::Eq, column_value(t, &t.rows[a], c)));
    let or = Some(if g.bool_with(0.6) { Resolve::Replace } else { Resolve::Ignore });
    let vector = draw_vector_set(g);
    Some(Update { table: k, sets, filter, or, returning: draw_returning(g, t), vector, from: None })
}

// ---------------------------------------------------------------- measure

fn bits(n: u64) -> u64 {
    u64::from(64 - n.leading_zeros())
}

fn pred_cost(p: &Pred) -> u64 {
    match p {
        Pred::Cmp(_, op, v) => 2 + u64::from(*op != CmpOp::Eq) + v.cost(),
        Pred::IsNull(_, not) => 2 + u64::from(*not),
        Pred::Between(_, lo, hi) => 3 + lo.cost() + hi.cost(),
        Pred::And(a, b) | Pred::Or(a, b) => 1 + pred_cost(a) + pred_cost(b),
        Pred::Not(a) => 1 + pred_cost(a),
    }
}

fn filter_cost(p: &Option<Pred>) -> u64 {
    p.as_ref().map_or(0, pred_cost)
}

/// A statement's simplicity measure, a natural number.
pub fn statement_cost(s: &Statement) -> u64 {
    1 + match s {
        Statement::Select(s) => {
            s.columns.len() as u64
                + s.order.iter().map(|&(_, d)| 1 + u64::from(d)).sum::<u64>()
                + s.limit.map_or(0, |n| 1 + bits(u64::from(n)))
                + filter_cost(&s.filter)
        },
        Statement::Insert(i) => {
            1 + u64::from(i.rowid)
                + i.columns.len() as u64
                + i.rows.iter().map(|r| 1 + r.iter().map(Value::cost).sum::<u64>()).sum::<u64>()
                + u64::from(i.or.is_some())
                + i.upsert.iter().map(upsert_cost).sum::<u64>()
                + returning_cost(&i.returning)
                + u64::from(i.select)
        },
        Statement::Update(u) => {
            1 + u.sets.iter().map(|(_, v)| 1 + v.cost()).sum::<u64>() + filter_cost(&u.filter) + u64::from(u.or.is_some()) + returning_cost(&u.returning) + u.from.as_ref().map_or(0, |f| 2 + f.reads.len() as u64 + u64::from(f.sub))
        },
        Statement::Delete(d) => 1 + filter_cost(&d.filter) + returning_cost(&d.returning),
        Statement::Vacuum => 0,
        Statement::IncrementalVacuum(n) => 1 + bits(u64::from(*n)),
        Statement::Check { quick, table } => u64::from(*quick) + u64::from(table.is_some()),
        Statement::Count { filter, .. } => 1 + filter_cost(filter),
        Statement::CreateTable { def, .. } => {
            Model { encoding: Encoding::Utf8, tables: vec![def.clone()], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() }.cost() + u64::from(def.db != Db::Main)
        },
        Statement::CreateIndex { def, .. } => index_cost(def),
        Statement::Attach(file) => 1 + u64::from(*file),
        Statement::Detach => 1,
        Statement::AnalyzeDb(db) => 1 + u64::from(*db != Db::Main),
        Statement::DbPragma { k, table, .. } => 1 + *k as u64 + u64::from(table.is_some()),
        Statement::Virtual { s, .. } => crate::vtabs::cost(s),
        Statement::DropTable(_) | Statement::DropIndex { .. } => 1,
        Statement::CacheSize(_) | Statement::Savepoint(_) | Statement::RollbackTo(_) | Statement::Release(_) | Statement::Rollback => 1,
        Statement::Analyze(target) => target.map_or(0, |(_, i)| 1 + u64::from(i.is_some())),
        Statement::AnalysisLimit(n) => 1 + bits(u64::from(*n)),
        Statement::Optimize(mask) => mask.map_or(0, |m| 1 + bits(u64::from(m))),
        Statement::AutomaticIndex(on) => u64::from(!*on),
        Statement::ReverseUnordered(on) => u64::from(*on),
        Statement::Stats(rows) => rows.iter().map(stat_cost).sum(),
        Statement::AddColumn { def, .. } => {
            1 + u64::from(def.generated.is_some()) + u64::from(def.ty.is_some() || def.decl.is_some()) + u64::from(def.coll.is_some()) + u64::from(def.not_null) + def.default.as_ref().map_or(0, |v| 1 + v.cost())
        },
        Statement::DropColumn { column, columns, .. } => 1 + (*columns - *column) as u64,
        Statement::NotNull { .. } | Statement::DropCheck { .. } | Statement::DropConstraint { .. } | Statement::RenameColumn { .. } => 1,
        Statement::ForeignKeys(on) => u64::from(!*on),
        Statement::FkCheck(t) => u64::from(t.is_some()),
        Statement::AddCheck { check, .. } => 1 + pred_cost(check),
        Statement::SwapTables { indexes, .. } => 2 + indexes.0.iter().chain(&indexes.1).map(index_cost).sum::<u64>(),
        Statement::CreateRule(g) => rule_cost(g),
        Statement::DropRule(_) => 1,
        Statement::RecursiveTriggers(on) => u64::from(*on),
        Statement::Reindex(r) => match r {
            Reindex::All => 0,
            Reindex::Table(_) | Reindex::Coll(_) => 1,
            Reindex::Index(..) => 2,
        },
        Statement::ViewWrite { write, .. } => 1 + statement_cost(write),
        Statement::JournalMode(j) => 1 + u64::from(*j != Journal::Delete),
        Statement::WalCheckpoint(c) => 1 + u64::from(c.is_some()),
        Statement::SecureDelete(n) => 1 + u64::from(*n),
        Statement::CellSizeCheck(_) | Statement::TempStore(_) | Statement::LockingMode(_) | Statement::VacuumInto(_) | Statement::Commit | Statement::ShrinkMemory => 1,
        Statement::Synchronous(n) => 1 + u64::from(*n != 2),
        Statement::MmapSize(n) | Statement::WalAutocheckpoint(n) | Statement::CacheSpill(n) => 1 + bits(u64::from(*n)),
        Statement::JournalSizeLimit(n) => 1 + bits(n.unsigned_abs().into()),
        Statement::AutoVacuum(a) => 1 + u64::from(*a != AutoVac::None),
        Statement::Begin(b) => 2 + u64::from(*b != Begin::Deferred),
    }
}

fn term_cost(x: &Term) -> u64 {
    match x {
        Term::Lit(v) => 1 + v.cost(),
        Term::Row(_) => 1,
    }
}

fn rule_cost(g: &Rule) -> u64 {
    2 + u64::from(!g.before) + g.of.len() as u64 + g.when.as_ref().map_or(0, pred_cost)
        + g.body
            .iter()
            .map(|s| match s {
                Step::Insert { values, or, upsert, .. } => 2 + u64::from(or.is_some()) + upsert.iter().map(upsert_cost).sum::<u64>() + values.iter().map(|x| 1 + term_cost(x)).sum::<u64>(),
                Step::Update { sets, filter, or, .. } => 2 + u64::from(or.is_some()) + sets.iter().map(|(_, x)| 1 + term_cost(x)).sum::<u64>() + filter_cost(filter),
                Step::Delete { filter, .. } => 2 + filter_cost(filter),
                Step::Ignore | Step::Select(_) => 1,
                Step::Abort(p) => 1 + pred_cost(p),
            })
            .sum::<u64>()
}

fn returning_cost(r: &Option<Vec<usize>>) -> u64 {
    r.as_ref().map_or(0, |cs| 1 + cs.len() as u64)
}

fn upsert_cost(u: &Upsert) -> u64 {
    1 + u.target.as_ref().map_or(0, |cs| cs.iter().map(|(_, k)| 1 + u64::from(k.is_some())).sum())
        + u.update.as_ref().map_or(0, |(sets, p)| 1 + sets.iter().map(|(_, e)| 1 + if let Excluded::No(v) = e { v.cost() } else { 0 }).sum::<u64>() + filter_cost(p))
}

fn index_cost(def: &Index) -> u64 {
    1 + def.columns.iter().map(|&(_, d)| 1 + u64::from(d)).sum::<u64>() + u64::from(def.unique) + u64::from(def.expr.is_some()) + filter_cost(&def.filter)
}

fn stat_cost(r: &Stat) -> u64 {
    1 + r.counts.iter().map(|&c| 1 + bits(c)).sum::<u64>()
        + u64::from(r.unordered)
        + u64::from(r.noskipscan)
        + r.size.map_or(0, |k| 1 + bits(u64::from(k)))
        + u64::from(r.drop_samples)
}

fn file_measure(file: &SqliteFile) -> u64 {
    match reader::read(&file.0) {
        Some((m, l)) => 1 + m.cost() + l.cost(),
        None => 0,
    }
}

/// The measure every shrink step strictly decreases, lexicographically:
/// the file's (as for [`Files`]), then the statements' total cost.
pub fn measure(c: &Case) -> (u64, u64) {
    (file_measure(&c.file), c.statements.iter().map(statement_cost).sum())
}

// ----------------------------------------------------------------- repair

fn repair_pred(t: &Table, p: &Pred) -> Option<Pred> {
    match p {
        Pred::And(a, b) | Pred::Or(a, b) => match (repair_pred(t, a), repair_pred(t, b)) {
            (Some(x), Some(y)) => Some(if matches!(p, Pred::And(..)) { Pred::And(Box::new(x), Box::new(y)) } else { Pred::Or(Box::new(x), Box::new(y)) }),
            (x, None) => x,
            (None, y) => y,
        },
        Pred::Not(a) => repair_pred(t, a).map(|x| Pred::Not(Box::new(x))),
        leaf => semantics::pred_valid(t, leaf).then(|| leaf.clone()),
    }
}

/// `s` itself when valid against `m`; otherwise what is left of it once the
/// parts that no longer resolve are gone, or nothing. Never costlier than
/// `s`.
pub fn repair(m: &Model, s: &Statement) -> Option<Statement> {
    if semantics::valid(m, s) {
        return Some(s.clone());
    }
    let fixed = match s {
        Statement::Select(s) => {
            let t = m.tables.get(s.table)?;
            Statement::Select(Select {
                table: s.table,
                columns: s.columns.iter().copied().filter(|&c| c < t.columns).collect(),
                filter: s.filter.as_ref().and_then(|p| repair_pred(t, p)),
                order: s.order.iter().copied().filter(|&(c, _)| c < t.columns).collect(),
                limit: s.limit,
            })
        },
        Statement::Insert(i) => {
            let t = m.tables.get(i.table)?;
            let rowid = i.rowid && t.key == Key::Rowid;
            let skip = usize::from(i.rowid);
            let keep: Vec<usize> = (0..i.columns.len()).filter(|&k| i.columns[k] < t.columns && t.generated[i.columns[k]].is_none()).collect();
            let cols: Vec<usize> = keep.iter().map(|&k| i.columns[k]).collect();
            let fits = |c: usize| c < t.columns && t.generated[c].is_none() && !is_key(t, c);
            let upsert: Vec<Upsert> = i
                .upsert
                .iter()
                .filter(|u| {
                    u.target.as_ref().is_none_or(|cs| cs.iter().all(|&(c, _)| c < t.columns)) && u.update.as_ref().is_none_or(|(sets, p)| sets.iter().all(|(c, _)| fits(*c)) && p.as_ref().is_none_or(|p| semantics::pred_valid(t, p)))
                })
                .cloned()
                .collect();
            let returning = i.returning.as_ref().map(|cs| cs.iter().copied().filter(|&c| c < t.columns).collect());
            let extras = i.or.is_some() || !upsert.is_empty();
            let mut fixed = Insert { table: i.table, rowid, columns: cols, rows: Vec::new(), or: i.or, upsert, returning, select: i.select };
            for r in &i.rows {
                let mut row: Vec<Value> = Vec::with_capacity(keep.len() + 1);
                if rowid {
                    row.push(r[0].clone());
                }
                row.extend(keep.iter().map(|&k| r[skip + k].clone()));
                fixed.rows.push(row);
                let ok = if extras { semantics::valid(m, &Statement::Insert(fixed.clone())) } else { semantics::inserted_rows(t, &fixed, m.encoding).is_some() };
                if !ok {
                    fixed.rows.pop();
                }
            }
            if fixed.rows.is_empty() {
                return None;
            }
            Statement::Insert(fixed)
        },
        Statement::Update(u) => {
            let t = m.tables.get(u.table)?;
            let kept: Vec<usize> = (0..u.sets.len())
                .filter(|&k| {
                    let (c, v) = &u.sets[k];
                    *c < t.columns && !is_key(t, *c) && t.generated[*c].is_none() && convert(t, *c, v).is_some()
                })
                .collect();
            let sets: Vec<(usize, Value)> = kept.iter().map(|&k| u.sets[k].clone()).collect();
            let filter = match &u.filter {
                Some(p) => Some(repair_pred(t, p)?),
                None => None,
            };
            if sets.is_empty() {
                return None;
            }
            let returning = u.returning.as_ref().map(|cs| cs.iter().copied().filter(|&c| c < t.columns).collect());
            // The FROM row's table still a rowid table with the columns
            // read, else the literals.
            let from = u.from.clone().filter(|f| m.tables.get(f.table).is_some_and(|s| !matches!(s.key, Key::WithoutRowid(_)) && f.table != u.table && f.reads.iter().all(|&(_, c)| c < s.columns)));
            let from = from.map(|mut f| {
                f.reads = f.reads.iter().filter_map(|&(k, c)| Some((kept.iter().position(|&x| x == k)?, c))).collect();
                f
            });
            Statement::Update(Update { table: u.table, sets, filter, or: u.or, returning, vector: u.vector, from })
        },
        Statement::Delete(d) => {
            let t = m.tables.get(d.table)?;
            let filter = match &d.filter {
                Some(p) => Some(repair_pred(t, p)?),
                None => None,
            };
            let returning = d.returning.as_ref().map(|cs| cs.iter().copied().filter(|&c| c < t.columns).collect());
            Statement::Delete(Delete { table: d.table, filter, returning })
        },
        // Only invalid inside a transaction, where it stays invalid.
        Statement::Vacuum => return None,
        Statement::IncrementalVacuum(_)
        | Statement::CacheSize(_)
        | Statement::AnalysisLimit(_)
        | Statement::Optimize(_)
        | Statement::AutomaticIndex(_)
        | Statement::ReverseUnordered(_) => return Some(s.clone()),
        Statement::Analyze(_) => Statement::Analyze(None),
        // The rows whose index is still there.
        Statement::Stats(rows) => {
            let rows: Vec<Stat> = rows.iter().filter(|r| semantics::stat_valid(m, r)).cloned().collect();
            if rows.is_empty() {
                return None;
            }
            Statement::Stats(rows)
        },
        Statement::Savepoint(_) => Statement::Savepoint(m.savepoints.len()),
        Statement::RollbackTo(k) => Statement::RollbackTo((*k).min(m.savepoints.len().checked_sub(1)?)),
        Statement::Release(k) => Statement::Release((*k).min(m.savepoints.len().checked_sub(1)?)),
        Statement::Rollback => return None,
        Statement::Check { quick, .. } => Statement::Check { quick: *quick, table: None },
        Statement::Count { table, filter } => {
            let t = m.tables.get(*table)?;
            let filter = match filter {
                Some(p) => Some(repair_pred(t, p)?),
                None => None,
            };
            Statement::Count { table: *table, filter }
        },
        // In main once aux is gone.
        Statement::CreateTable { def, .. } => {
            let db = if def.db == Db::Aux && !m.session.aux { Db::Main } else { def.db };
            Statement::CreateTable { table: m.tables.len(), def: Table { db, ..def.clone() } }
        },
        Statement::DropTable(_) => Statement::DropTable(m.tables.len().checked_sub(1)?),
        // The columns that are left; the expression only while its column
        // still leads, what resolves of the predicate.
        Statement::CreateIndex { table, def, .. } => {
            let t = m.tables.get(*table)?;
            let columns: Vec<(usize, bool)> = def.columns.iter().copied().filter(|&(c, _)| c < t.columns).collect();
            let expr = def.expr.filter(|_| columns.first() == def.columns.first());
            let filter = def.filter.as_ref().and_then(|p| repair_pred(t, p));
            if columns.is_empty() {
                return None;
            }
            let coll: Vec<(usize, crate::model::Coll)> = def.coll.iter().copied().filter(|&(c, _)| columns.iter().any(|&(x, _)| x == c)).collect();
            let shape = Index { columns, unique: false, expr, filter, coll };
            let unique = def.unique && crate::model::unique_ok(t, &shape, &t.rows, m.encoding);
            Statement::CreateIndex { table: *table, index: t.indexes.len(), def: Index { unique, ..shape }, db: t.db }
        },
        Statement::DropIndex { table, .. } => {
            let t = m.tables.get(*table)?;
            Statement::DropIndex { table: *table, index: t.indexes.len().checked_sub(1)?, db: t.db }
        },
        // At the end of the table; plainer when it no longer fits.
        Statement::AddColumn { table, def, .. } => {
            let t = m.tables.get(*table)?;
            let fixed = Statement::AddColumn { table: *table, column: t.columns, def: def.clone() };
            if semantics::valid(m, &fixed) {
                fixed
            } else {
                Statement::AddColumn { table: *table, column: t.columns, def: NewColumn { not_null: false, default: None, references: None, generated: None, ..def.clone() } }
            }
        },
        Statement::DropColumn { table, column, .. } => Statement::DropColumn { table: *table, column: *column, columns: m.tables.get(*table)?.columns },
        Statement::NotNull { .. } | Statement::DropConstraint { .. } | Statement::ForeignKeys(_) | Statement::RenameColumn { .. } => return None,
        Statement::FkCheck(_) => Statement::FkCheck(None),
        Statement::AddCheck { table, check, .. } => {
            let t = m.tables.get(*table)?;
            Statement::AddCheck { table: *table, index: t.checks.len(), check: repair_pred(t, check)? }
        },
        Statement::DropCheck { table, .. } => Statement::DropCheck { table: *table, check: m.tables.get(*table)?.checks.len().checked_sub(1)? },
        Statement::SwapTables { a, b, .. } => {
            Statement::SwapTables { a: *a, b: *b, indexes: (m.tables.get(*a)?.indexes.clone(), m.tables.get(*b)?.indexes.clone()) }
        },
        // Of its table's schema now, TEMP when its body writes another's.
        Statement::CreateRule(g) => {
            let db = if g.view { Db::Main } else { m.tables.get(g.table)?.db };
            let temp = g.temp || g.body.iter().any(|s| step_table(s).is_some_and(|b| m.tables.get(b).is_some_and(|b| b.db != db)));
            Statement::CreateRule(Rule { id: semantics::next_rule_id(m), db, temp, ..g.clone() })
        },
        Statement::Attach(_) | Statement::Detach => return None,
        Statement::Virtual { s, .. } => {
            let kind = crate::vtabs::target(&m.vtabs, s)?;
            let s = crate::vtabs::repair(&m.vtabs, s, !m.savepoints.is_empty())?;
            Statement::Virtual { kind: crate::vtabs::target(&m.vtabs, &s).unwrap_or(kind), s }
        },
        Statement::AnalyzeDb(_) => Statement::AnalyzeDb(Db::Main),
        Statement::DbPragma { k, table, .. } => match table {
            // Of the table's schema now.
            Some(t) => Statement::DbPragma { db: m.tables.get(*t)?.db, k: *k, table: Some(*t) },
            None => Statement::DbPragma { db: Db::Main, k: *k, table: None },
        },
        Statement::DropRule(_) => return None,
        Statement::RecursiveTriggers(_) => return Some(s.clone()),
        Statement::ViewWrite { .. } => return None,
        Statement::Reindex(_) => Statement::Reindex(Reindex::All),
        // Invalid only inside a transaction, where they stay invalid.
        Statement::JournalMode(_) | Statement::WalCheckpoint(_) | Statement::AutoVacuum(_) | Statement::VacuumInto(_) | Statement::TempStore(_) | Statement::Commit => return None,
        Statement::SecureDelete(_)
        | Statement::CellSizeCheck(_)
        | Statement::MmapSize(_)
        | Statement::LockingMode(_)
        | Statement::JournalSizeLimit(_)
        | Statement::WalAutocheckpoint(_)
        | Statement::CacheSpill(_)
        | Statement::ShrinkMemory => return Some(s.clone()),
        Statement::Synchronous(_) => return None,
        // Inside a transaction: a savepoint of it.
        Statement::Begin(_) => Statement::Savepoint(m.savepoints.len()),
    };
    (semantics::valid(m, &fixed) && statement_cost(&fixed) <= statement_cost(s)).then_some(fixed)
}

/// Statements repaired one after another against the model each leaves:
/// every statement of the result is valid where it runs.
fn settle(mut m: Model, statements: impl IntoIterator<Item = Statement>) -> Vec<Statement> {
    let mut out = Vec::new();
    for s in statements {
        if let Some(s) = repair(&m, &s) {
            semantics::apply(&mut m, &s);
            out.push(s);
        }
    }
    out
}

/// The model before each statement, and after the last.
fn models(m: Model, statements: &[Statement]) -> Vec<Model> {
    let mut out = vec![m];
    for s in statements {
        let mut next = out.last().unwrap().clone();
        semantics::apply(&mut next, s);
        out.push(next);
    }
    out
}

// -------------------------------------------------------------- shrinking

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

fn simpler_preds(p: &Pred) -> Vec<Pred> {
    let mut out = Vec::new();
    match p {
        Pred::And(a, b) | Pred::Or(a, b) => {
            out.push((**a).clone());
            out.push((**b).clone());
            let and = matches!(p, Pred::And(..));
            let join = |x: Pred, y: Pred| if and { Pred::And(Box::new(x), Box::new(y)) } else { Pred::Or(Box::new(x), Box::new(y)) };
            for x in simpler_preds(a) {
                out.push(join(x, (**b).clone()));
            }
            for y in simpler_preds(b) {
                out.push(join((**a).clone(), y));
            }
        },
        Pred::Not(a) => {
            out.push((**a).clone());
            out.extend(simpler_preds(a).into_iter().map(|x| Pred::Not(Box::new(x))));
        },
        Pred::Cmp(c, op, v) => {
            if *op != CmpOp::Eq {
                out.push(Pred::Cmp(*c, CmpOp::Eq, v.clone()));
            }
            out.push(Pred::IsNull(*c, false));
            out.extend(simpler_values(v).into_iter().map(|w| Pred::Cmp(*c, *op, w)));
        },
        Pred::Between(c, lo, hi) => {
            out.push(Pred::Cmp(*c, CmpOp::Ge, lo.clone()));
            out.push(Pred::Cmp(*c, CmpOp::Le, hi.clone()));
            out.extend(simpler_values(lo).into_iter().map(|w| Pred::Between(*c, w, hi.clone())));
            out.extend(simpler_values(hi).into_iter().map(|w| Pred::Between(*c, lo.clone(), w)));
        },
        Pred::IsNull(c, true) => out.push(Pred::IsNull(*c, false)),
        Pred::IsNull(_, false) => {},
    }
    out
}

fn simpler_filters(p: &Option<Pred>) -> Vec<Option<Pred>> {
    match p {
        Some(p) => std::iter::once(None).chain(simpler_preds(p).into_iter().map(Some)).collect(),
        None => Vec::new(),
    }
}

/// Simpler versions of one statement (not checked for validity).
fn simpler_statements(s: &Statement) -> Vec<Statement> {
    let mut out = Vec::new();
    match s {
        Statement::Select(s) => {
            let with = |f: &dyn Fn(&mut Select)| {
                let mut c = s.clone();
                f(&mut c);
                Statement::Select(c)
            };
            if *s != (Select { table: s.table, columns: Vec::new(), filter: None, order: Vec::new(), limit: None }) {
                out.push(with(&|c| *c = Select { table: s.table, columns: Vec::new(), filter: None, order: Vec::new(), limit: None }));
            }
            if s.limit.is_some() {
                out.push(with(&|c| c.limit = None));
            }
            if !s.columns.is_empty() {
                out.push(with(&|c| c.columns.clear()));
            }
            for k in 0..s.order.len() {
                out.push(with(&|c| {
                    c.order.remove(k);
                }));
            }
            for f in simpler_filters(&s.filter) {
                out.push(with(&|c| c.filter = f.clone()));
            }
        },
        Statement::Insert(i) => {
            if i.or.is_some() {
                out.push(Statement::Insert(Insert { or: None, ..i.clone() }));
            }
            for k in 0..i.upsert.len() {
                let mut c = i.clone();
                c.upsert.remove(k);
                out.push(Statement::Insert(c));
            }
            if i.returning.is_some() {
                out.push(Statement::Insert(Insert { returning: None, ..i.clone() }));
            }
            if i.select {
                out.push(Statement::Insert(Insert { select: false, ..i.clone() }));
            }
            if i.rows.len() > 1 {
                for k in 0..i.rows.len() {
                    let mut c = i.clone();
                    c.rows.remove(k);
                    out.push(Statement::Insert(c));
                }
            }
            if i.rowid {
                let mut c = i.clone();
                c.rowid = false;
                for r in &mut c.rows {
                    r.remove(0);
                }
                out.push(Statement::Insert(c));
            }
            let skip = usize::from(i.rowid);
            for k in 0..i.columns.len() {
                let mut c = i.clone();
                c.columns.remove(k);
                for r in &mut c.rows {
                    r.remove(skip + k);
                }
                out.push(Statement::Insert(c));
            }
            for (r, row) in i.rows.iter().enumerate() {
                for (k, v) in row.iter().enumerate() {
                    for w in simpler_values(v) {
                        let mut c = i.clone();
                        c.rows[r][k] = w;
                        out.push(Statement::Insert(c));
                    }
                }
            }
        },
        Statement::Update(u) => {
            if u.or.is_some() {
                out.push(Statement::Update(Update { or: None, ..u.clone() }));
            }
            if u.returning.is_some() {
                out.push(Statement::Update(Update { returning: None, ..u.clone() }));
            }
            if u.vector != VectorSet::Plain {
                out.push(Statement::Update(Update { vector: VectorSet::Plain, ..u.clone() }));
            }
            if let Some(f) = &u.from {
                out.push(Statement::Update(Update { from: None, ..u.clone() }));
                if f.sub {
                    out.push(Statement::Update(Update { from: Some(UpdateFrom { sub: false, ..f.clone() }), ..u.clone() }));
                }
                if f.reads.len() > 1 {
                    for k in 0..f.reads.len() {
                        let mut f = f.clone();
                        f.reads.remove(k);
                        out.push(Statement::Update(Update { from: Some(f), ..u.clone() }));
                    }
                }
            }
            if u.sets.len() > 1 {
                for k in 0..u.sets.len() {
                    let mut c = u.clone();
                    c.sets.remove(k);
                    if let Some(f) = &mut c.from {
                        f.reads = f.reads.iter().filter(|&&(i, _)| i != k).map(|&(i, col)| (if i > k { i - 1 } else { i }, col)).collect();
                    }
                    out.push(Statement::Update(c));
                }
            }
            for f in simpler_filters(&u.filter) {
                out.push(Statement::Update(Update { filter: f, ..u.clone() }));
            }
            for (k, (_, v)) in u.sets.iter().enumerate() {
                for w in simpler_values(v) {
                    let mut c = u.clone();
                    c.sets[k].1 = w;
                    out.push(Statement::Update(c));
                }
            }
        },
        Statement::CreateRule(g) => {
            let with = |f: &dyn Fn(&mut Rule)| {
                let mut c = g.clone();
                f(&mut c);
                Statement::CreateRule(c)
            };
            if g.when.is_some() {
                out.push(with(&|c| c.when = None));
            }
            if !g.of.is_empty() {
                out.push(with(&|c| c.of.clear()));
            }
            if g.body.len() > 1 {
                for k in 0..g.body.len() {
                    out.push(with(&|c| {
                        c.body.remove(k);
                    }));
                }
            }
        },
        Statement::RecursiveTriggers(true) => out.push(Statement::RecursiveTriggers(false)),
        Statement::ViewWrite { view, write } => {
            out.extend(simpler_statements(write).into_iter().map(|w| Statement::ViewWrite { view: *view, write: Box::new(w) }));
        },
        Statement::Delete(d) => {
            if d.returning.is_some() {
                out.push(Statement::Delete(Delete { returning: None, ..d.clone() }));
            }
            for f in simpler_filters(&d.filter) {
                out.push(Statement::Delete(Delete { filter: f, ..d.clone() }));
            }
        },
        Statement::IncrementalVacuum(n) if *n > 0 => {
            out.push(Statement::IncrementalVacuum(0));
            out.push(Statement::IncrementalVacuum(n / 2));
        },
        Statement::Analyze(Some((t, i))) => {
            out.push(Statement::Analyze(None));
            if i.is_some() {
                out.push(Statement::Analyze(Some((*t, None))));
            }
        },
        Statement::AnalysisLimit(n) if *n > 0 => {
            out.push(Statement::AnalysisLimit(0));
            out.push(Statement::AnalysisLimit(n / 2));
        },
        Statement::Analyze(None) | Statement::AnalysisLimit(_) => {},
        Statement::Optimize(Some(_)) => out.push(Statement::Optimize(None)),
        Statement::AutomaticIndex(false) => out.push(Statement::AutomaticIndex(true)),
        Statement::ReverseUnordered(true) => out.push(Statement::ReverseUnordered(false)),
        Statement::Optimize(None) | Statement::AutomaticIndex(true) | Statement::ReverseUnordered(false) => {},
        // A row less; a row with a number less, halved, or without a flag.
        Statement::Stats(rows) => {
            let with = |k: usize, f: &dyn Fn(&mut Stat)| {
                let mut c = rows.clone();
                f(&mut c[k]);
                Statement::Stats(c)
            };
            for k in 0..rows.len() {
                if rows.len() > 1 {
                    let mut c = rows.clone();
                    c.remove(k);
                    out.push(Statement::Stats(c));
                }
                let r = &rows[k];
                if r.counts.len() > 1 {
                    out.push(with(k, &|r| {
                        r.counts.pop();
                    }));
                }
                for j in 0..r.counts.len() {
                    if r.counts[j] > 0 {
                        out.push(with(k, &|r| r.counts[j] /= 2));
                    }
                }
                if r.unordered {
                    out.push(with(k, &|r| r.unordered = false));
                }
                if r.noskipscan {
                    out.push(with(k, &|r| r.noskipscan = false));
                }
                if r.size.is_some() {
                    out.push(with(k, &|r| r.size = None));
                }
                if r.drop_samples {
                    out.push(with(k, &|r| r.drop_samples = false));
                }
            }
        },
        Statement::IncrementalVacuum(_)
        | Statement::Vacuum
        | Statement::CacheSize(_)
        | Statement::Savepoint(_)
        | Statement::RollbackTo(_)
        | Statement::Release(_)
        | Statement::Rollback => {},
        Statement::Check { quick, table } => {
            if *quick {
                out.push(Statement::Check { quick: false, table: *table });
            }
            if table.is_some() {
                out.push(Statement::Check { quick: *quick, table: None });
            }
        },
        Statement::Count { table, filter } => {
            for f in simpler_filters(filter) {
                out.push(Statement::Count { table: *table, filter: f });
            }
        },
        Statement::CreateIndex { table, index, def, db } => {
            let db = *db;
            if def.unique {
                out.push(Statement::CreateIndex { table: *table, index: *index, def: Index { unique: false, ..def.clone() }, db });
            }
            if def.expr.is_some() {
                out.push(Statement::CreateIndex { table: *table, index: *index, def: Index { expr: None, ..def.clone() }, db });
            }
            for f in simpler_filters(&def.filter) {
                out.push(Statement::CreateIndex { table: *table, index: *index, def: Index { filter: f, ..def.clone() }, db });
            }
            for k in 0..def.columns.len() {
                let mut d = def.clone();
                if d.columns.len() == 1 {
                    continue;
                }
                d.columns.remove(k);
                out.push(Statement::CreateIndex { table: *table, index: *index, def: d, db });
            }
        },
        Statement::AddColumn { table, column, def } => {
            let with = |d: NewColumn| Statement::AddColumn { table: *table, column: *column, def: d };
            if def.default.is_some() {
                out.push(with(NewColumn { default: None, not_null: false, ..def.clone() }));
            }
            if def.references.is_some() {
                out.push(with(NewColumn { references: None, ..def.clone() }));
            }
            if def.generated.is_some() {
                out.push(with(NewColumn { generated: None, ..def.clone() }));
            }
            if def.not_null {
                out.push(with(NewColumn { not_null: false, ..def.clone() }));
            }
            if def.coll.is_some() {
                out.push(with(NewColumn { coll: None, ..def.clone() }));
            }
            if def.decl.is_some() {
                out.push(with(NewColumn { decl: None, ..def.clone() }));
            }
            if let Some(v) = &def.default {
                for w in simpler_values(v) {
                    out.push(with(NewColumn { default: Some(w), ..def.clone() }));
                }
            }
        },
        Statement::AddCheck { table, index, check } => {
            for p in simpler_preds(check) {
                out.push(Statement::AddCheck { table: *table, index: *index, check: p });
            }
        },
        Statement::CreateTable { table, def } if def.db != Db::Main => out.push(Statement::CreateTable { table: *table, def: Table { db: Db::Main, ..def.clone() } }),
        Statement::AnalyzeDb(db) if *db != Db::Main => out.push(Statement::AnalyzeDb(Db::Main)),
        Statement::Attach(true) => out.push(Statement::Attach(false)),
        Statement::Virtual { s, kind } => out.extend(crate::vtabs::simpler(s).into_iter().map(|s| Statement::Virtual { s, kind: kind.clone() })),
        Statement::Attach(_) | Statement::Detach | Statement::AnalyzeDb(_) | Statement::DbPragma { .. } => {},
        Statement::CreateTable { .. }
        | Statement::DropTable(_)
        | Statement::DropIndex { .. }
        | Statement::DropColumn { .. }
        | Statement::NotNull { .. }
        | Statement::RenameColumn { .. }
        | Statement::DropCheck { .. }
        | Statement::DropConstraint { .. }
        | Statement::ForeignKeys(_)
        | Statement::SwapTables { .. } => {},
        Statement::FkCheck(Some(_)) => out.push(Statement::FkCheck(None)),
        Statement::FkCheck(None) | Statement::DropRule(_) | Statement::RecursiveTriggers(false) | Statement::Reindex(Reindex::All) => {},
        Statement::Reindex(Reindex::Index(t, _)) => {
            out.push(Statement::Reindex(Reindex::All));
            out.push(Statement::Reindex(Reindex::Table(*t)));
        },
        Statement::Reindex(_) => out.push(Statement::Reindex(Reindex::All)),
        Statement::JournalMode(Journal::Delete) => {},
        Statement::JournalMode(_) => out.push(Statement::JournalMode(Journal::Delete)),
        Statement::WalCheckpoint(Some(_)) => out.push(Statement::WalCheckpoint(None)),
        Statement::SecureDelete(n) if *n > 0 => out.push(Statement::SecureDelete(0)),
        Statement::MmapSize(n) if *n > 0 => out.push(Statement::MmapSize(n / 2)),
        Statement::CacheSpill(n) if *n > 0 => out.push(Statement::CacheSpill(n / 2)),
        Statement::WalAutocheckpoint(n) if *n > 0 => out.push(Statement::WalAutocheckpoint(n / 2)),
        // As VACUUM, then without changing the mode.
        Statement::AutoVacuum(a) => {
            out.push(Statement::Vacuum);
            if *a != AutoVac::None {
                out.push(Statement::AutoVacuum(AutoVac::None));
            }
        },
        // A SAVEPOINT's transaction (COMMIT ends it too), a DEFERRED one.
        Statement::Begin(b) => {
            out.push(Statement::Savepoint(0));
            if *b != Begin::Deferred {
                out.push(Statement::Begin(Begin::Deferred));
            }
        },
        Statement::Commit => out.push(Statement::Release(0)),
        Statement::WalCheckpoint(None)
        | Statement::SecureDelete(_)
        | Statement::MmapSize(_)
        | Statement::CacheSpill(_)
        | Statement::WalAutocheckpoint(_)
        | Statement::CellSizeCheck(_)
        | Statement::TempStore(_)
        | Statement::LockingMode(_)
        | Statement::JournalSizeLimit(_)
        | Statement::VacuumInto(_)
        | Statement::ShrinkMemory => {},
        Statement::Synchronous(2) => {},
        Statement::Synchronous(_) => out.push(Statement::Synchronous(2)),
    }
    out
}

impl Gen for Cases {
    type Value = Case;

    fn generate(&self, g: &mut Source) -> Case {
        let file = crate::files::draw_for_statements(g);
        let m = model_of(&file).unwrap_or_else(empty_model);
        let statements = draw_statements(g, m);
        Case { file, statements }
    }

    /// Every candidate is lower in [`measure`] (lexicographic on two
    /// natural numbers, so well founded), checked before it is offered:
    /// statement steps keep the file and remove or simplify statements;
    /// file steps are [`Files`]' own (each lower in its measure) with the
    /// statements repaired against the smaller model. Every candidate's
    /// statements are settled ([`settle`]), so each is well formed.
    fn shrink(&self, value: &Case, visit: &mut dyn FnMut(Case) -> ControlFlow<()>) -> ControlFlow<()> {
        let Some(m) = model_of(&value.file) else {
            return ControlFlow::Continue(());
        };
        let ms = models(m.clone(), &value.statements);
        let here = measure(value);
        let file_here = here.0;
        let mut offer = |c: Case| if measure(&c) < here && c != *value { visit(c) } else { ControlFlow::Continue(()) };
        let n = value.statements.len();
        let same_file = |statements: Vec<Statement>| Case { file: value.file.clone(), statements };
        // Fewer statements: none, either half, one less.
        if n > 0 {
            offer(same_file(Vec::new()))?;
        }
        if n > 1 {
            offer(same_file(value.statements[..n / 2].to_vec()))?;
            offer(same_file(settle(m.clone(), value.statements[n / 2..].iter().cloned())))?;
        }
        for k in (0..n).rev() {
            let rest = settle(ms[k].clone(), value.statements[k + 1..].iter().cloned());
            offer(same_file(value.statements[..k].iter().cloned().chain(rest).collect()))?;
        }
        // A smaller file, the statements repaired against it.
        Files.shrink(&value.file, &mut |file| {
            let Some(fm) = model_of(&file) else {
                return ControlFlow::Continue(());
            };
            if file_measure(&file) >= file_here {
                return ControlFlow::Continue(());
            }
            let statements = settle(fm, value.statements.iter().cloned());
            offer(Case { file, statements })
        })?;
        // Simpler statements.
        for (k, before) in ms.iter().enumerate().take(n) {
            for s in simpler_statements(&value.statements[k]) {
                if !semantics::valid(before, &s) {
                    continue;
                }
                let mut next = before.clone();
                semantics::apply(&mut next, &s);
                let rest = settle(next, value.statements[k + 1..].iter().cloned());
                offer(same_file(value.statements[..k].iter().cloned().chain(std::iter::once(s)).chain(rest).collect()))?;
            }
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, value: &Case, g: &mut Source) -> Case {
        let mut file = value.file.clone();
        let mut statements = value.statements.clone();
        for _ in 0..g.int(1..=3) {
            let Some(m) = model_of(&file) else {
                crate::fell_back(&crate::FILE_FALLBACKS);
                return self.generate(g);
            };
            let n = statements.len();
            match g.int(0..10) {
                0 | 1 => file = Files.mutate(&file, g),
                // A new statement somewhere.
                2..=4 if n < MAX_STATEMENTS => {
                    let k = g.int(0..=n);
                    let ms = models(m.clone(), &statements[..k]);
                    let s = draw_statement(g, ms.last().unwrap());
                    statements.insert(k, s);
                },
                // A statement replaced by a new one.
                5..=7 if n > 0 => {
                    let k = g.index(n);
                    let ms = models(m.clone(), &statements[..k]);
                    statements[k] = draw_statement(g, ms.last().unwrap());
                },
                8 if n > 0 => {
                    statements.remove(g.index(n));
                },
                // Two statements swapped.
                _ if n > 1 => {
                    let (a, b) = (g.index(n), g.index(n));
                    statements.swap(a, b);
                },
                _ => file = Files.mutate(&file, g),
            }
            let m = model_of(&file).unwrap_or_else(empty_model);
            statements = settle(m, statements);
        }
        Case { file, statements }
    }
}

// ---------------------------------------------------------------- metrics

/// Numbers about one case, from the model and the statements' semantics
/// (no SQLite):
///
/// - statements: how many; selects, inserts, updates, deletes, vacuums
///   (`VACUUM` and `PRAGMA incremental_vacuum`): how many of each kind.
/// - predicate leaves: comparisons and NULL tests over all statements;
///   indexed predicate leaves: those on a column an index or the key leads
///   with, where SQLite can seek; max predicate depth (0 without any).
/// - rows returned: rows all SELECTs return together; empty selects: SELECTs
///   that return nothing (with selects, the share of misses).
/// - rows written: rows inserted, updated and deleted together.
/// - ordered selects: SELECTs with ORDER BY.
/// - From the file's model: tables, without rowid tables, strict tables,
///   indexes, rows (at the start), utf16 (0/1), page size (0 for the empty
///   file).
pub fn case_metrics(case: &Case, _size: usize) -> Vec<(&'static str, f64)> {
    let (mut m, layout) = read(&case.file).unwrap_or((empty_model(), None));
    let initial_rows: usize = m.tables.iter().map(|t| t.rows.len()).sum();
    let file = [
        ("tables", m.tables.len() as f64),
        ("without rowid tables", m.tables.iter().filter(|t| matches!(t.key, Key::WithoutRowid(_))).count() as f64),
        ("strict tables", m.tables.iter().filter(|t| t.strict.is_some()).count() as f64),
        ("indexes", m.tables.iter().map(|t| t.indexes.len()).sum::<usize>() as f64),
        ("rows", initial_rows as f64),
        ("utf16", f64::from(u8::from(m.encoding != Encoding::Utf8))),
        ("page size", layout.map_or(0.0, |l| f64::from(l.page_size))),
    ];
    let mut kinds = [0usize; 5];
    let (mut leaves, mut indexed, mut depth, mut returned, mut empty, mut written, mut ordered) = (0, 0, 0, 0, 0, 0, 0);
    for s in &case.statements {
        let (filter, table) = match s {
            Statement::Select(x) => (&x.filter, Some(x.table)),
            Statement::Update(x) => (&x.filter, Some(x.table)),
            Statement::Delete(x) => (&x.filter, Some(x.table)),
            Statement::Count { table, filter } => (filter, Some(*table)),
            _ => (&None, None),
        };
        if let (Some(p), Some(t)) = (filter, table.and_then(|t| m.tables.get(t))) {
            let mut cols = Vec::new();
            p.leaves(&mut cols);
            let lead = leading_columns(t);
            leaves += cols.len();
            indexed += cols.iter().filter(|c| lead.contains(c)).count();
            depth = depth.max(p.depth());
        }
        kinds[match s {
            Statement::Select(x) => {
                ordered += usize::from(!x.order.is_empty());
                0
            },
            Statement::Count { .. } => 0,
            Statement::Insert(_) => 1,
            Statement::Update(_) => 2,
            Statement::Delete(_) => 3,
            _ => 4,
        }] += 1;
        if !semantics::valid(&m, s) {
            continue;
        }
        let e = semantics::apply(&mut m, s);
        written += e.written;
        if let Some(rows) = e.rows {
            returned += rows.len();
            empty += usize::from(rows.is_empty());
        }
    }
    let mut out = vec![
        ("statements", case.statements.len() as f64),
        ("selects", kinds[0] as f64),
        ("inserts", kinds[1] as f64),
        ("updates", kinds[2] as f64),
        ("deletes", kinds[3] as f64),
        ("vacuums", kinds[4] as f64),
        ("predicate leaves", leaves as f64),
        ("indexed predicate leaves", indexed as f64),
        ("max predicate depth", depth as f64),
        ("rows returned", returned as f64),
        ("empty selects", empty as f64),
        ("ordered selects", ordered as f64),
        ("rows written", written as f64),
    ];
    out.extend(file);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cases_are_well_formed_and_shrink_and_mutate_so() {
        for seed in 0..60u64 {
            let mut g = Source::new(seed, (seed % 12) as usize);
            let case = Cases.generate(&mut g);
            assert!(well_formed(&case), "seed {seed}: {case:?}");
            let here = measure(&case);
            let mut seen = 0;
            let _ = Cases.shrink(&case, &mut |c| {
                assert!(well_formed(&c), "shrink of seed {seed}: {c:?}");
                assert!(measure(&c) < here);
                seen += 1;
                if seen < 40 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            });
            for _ in 0..4 {
                let c = Cases.mutate(&case, &mut g);
                assert!(well_formed(&c), "mutant of seed {seed}: {c:?}");
            }
        }
    }

    /// An UPDATE FROM whose FROM row is missing still has its filter
    /// resolve against the target: SQLite prepares it either way. Found as
    /// run 20261016-1828 (novel profile, all eight oracles): mutation left
    /// `UPDATE t0 SET c0 = u.c0 FROM t2 AS u WHERE u.rowid = 8 AND t0.c4 <
    /// -3` over `t0(c0)` and an empty new t2, which the model took for the
    /// UPDATE of no row and SQLite rejects ("no such column: t0.c4").
    #[test]
    fn update_from_filter_resolves_without_the_from_row() {
        let mut m = empty_model();
        m.tables.push(Table::new(1, Key::Rowid));
        m.tables.push(Table::new(1, Key::Rowid));
        let u = Update {
            table: 0,
            sets: vec![(0, Value::Int(1))],
            filter: Some(Pred::And(Box::new(Pred::IsNull(0, true)), Box::new(Pred::Cmp(4, CmpOp::Lt, Value::Int(-3))))),
            from: Some(UpdateFrom { table: 1, rowid: 8, reads: vec![(0, 0)], sub: false }),
            ..Update::default()
        };
        let s = Statement::Update(u);
        assert!(!semantics::valid(&m, &s), "{s}");
        let fixed = repair(&m, &s).expect("the filter's valid leaf is left");
        assert!(semantics::valid(&m, &fixed), "{fixed}");
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0); CREATE TABLE t1(c0);").unwrap();
        assert!(db.execute_batch(&s.to_string()).is_err());
        db.execute_batch(&fixed.to_string()).unwrap_or_else(|e| panic!("{fixed}: {e}"));
    }
}
