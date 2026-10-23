// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Test cases for PostgreSQL: a generated database ([`Setup`]) and a
//! sequence of SQL statements over it ([`Case`]), with the predicate
//! [`well_formed`], the generator [`cases`] and its metrics
//! [`case_metrics`].
//!
//! The generator draws a setup, then each statement against the model the
//! statements before it leave ([`crate::semantics`]). Its hooks never run
//! PostgreSQL.

use std::{fmt, ops::ControlFlow};

use ephyra::{Gen, Source};

use crate::{
    fk,
    model::{Action, Column, Compression, Deferral, ForeignKey, Index, Method, Model, Storage, Table, TrigAction, Trigger, Value},
    semantics::{self, inserted_rows, next_value},
    setup::{self, draw_columns, draw_index, draw_pred, draw_type, draw_value, fresh, setup_ok, simpler_indexes, simpler_values, Pool, Setup, Setups},
    statements::{CmpOp, Delete, DomOp, Enable, FromTable, Insert, Lit, Merge, MergeAction, MergeKind, OnConflict, OrderTerm, Phys, Pred, Returning, Select, SetVal, Setting, Stat, Statement, Target, Update, When, Which},
};

/// A database to start from and the statements to run on it, in order.
#[derive(Clone, PartialEq)]
pub struct Case {
    pub setup: Setup,
    pub statements: Vec<Statement>,
}

/// The setup's script, then the statements as a SQL script.
impl fmt::Debug for Case {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "Case {{")?;
        writeln!(f, "  setup: {:?}", self.setup)?;
        writeln!(f, "  -- {} statements", self.statements.len())?;
        for s in &self.statements {
            writeln!(f, "  {s};")?;
        }
        write!(f, "}}")
    }
}

/// A case is well formed when its setup is, and every statement is valid
/// against the model the statements before it leave.
#[ephyra::generator]
pub fn well_formed(case: &Case) -> bool {
    setup_ok(&case.setup) && {
        let mut m = case.setup.model.clone();
        case.statements.iter().all(|s| {
            let ok = semantics::valid(&m, s);
            if ok {
                semantics::apply(&mut m, s);
            }
            ok
        })
    }
}

/// Cases: a generated setup and up to about 50 statements over it.
#[ephyra::generator(well_formed, metrics = case_metrics)]
pub fn cases() -> impl Gen<Value = Case> {
    Cases
}

pub struct Cases;

/// The model at the end of a case's statements.
pub fn final_model(case: &Case) -> Model {
    let mut m = case.setup.model.clone();
    for s in &case.statements {
        semantics::apply(&mut m, s);
    }
    m
}

// ------------------------------------------------------------------ drawing

const MAX_STATEMENTS: usize = 50;

fn draw_statements(g: &mut Source, mut m: Model, pool: &Pool) -> Vec<Statement> {
    let most = (4 + 4 * g.size()).min(MAX_STATEMENTS);
    let n = if g.bool_with(0.01) { 0 } else { g.int(1..=most) };
    let mut out = Vec::with_capacity(n);
    for k in 0..n {
        // The last statements close the transaction now and then (rolled
        // back mostly, so that what it wrote is undone in the pages),
        // gather statistics, craft some, and end with a VACUUM or a
        // CHECKPOINT: what the oracles' queries are planned and run on.
        if k + 1 == n && g.bool_with(0.6) {
            if !m.savepoints.is_empty() {
                let end = if g.bool_with(0.7) { Statement::Rollback } else { Statement::Release(0) };
                semantics::apply(&mut m, &end);
                out.push(end);
            }
            if g.bool_with(0.45) {
                if !m.tables.is_empty() {
                    out.push(Statement::Analyze(all_tables(&m)));
                }
            }
            if !m.tables.is_empty() && g.bool_with(0.35) {
                let s = Statement::Stats(draw_stats(g, &m));
                if semantics::valid(&m, &s) {
                    out.push(s);
                }
            }
            let s = match g.int(0..4) {
                0 => Statement::Checkpoint,
                1 if !m.tables.is_empty() => Statement::Vacuum { tables: all_tables(&m), full: false, freeze: false, analyze: g.bool() },
                _ => draw_statement(g, &m, pool),
            };
            semantics::apply(&mut m, &s);
            out.push(s);
            break;
        }
        let s = if k == 0 && n > 2 && g.bool_with(0.6) {
            // A transaction around most of the statements now and then.
            Statement::Savepoint(0)
        } else {
            draw_statement(g, &m, pool)
        };
        semantics::apply(&mut m, &s);
        out.push(s);
    }
    out
}

/// Every table of `m`, by id.
fn all_tables(m: &Model) -> Vec<u32> {
    m.tables.iter().map(|t| t.id).collect()
}

/// A planner or executor setting.
fn draw_setting(g: &mut Source) -> Setting {
    match g.int(0..10) {
        0..=5 => Setting::Enable(*g.pick(&Enable::ALL), g.bool_with(0.2)),
        6 => Setting::WorkMem(*g.pick(&[64, 128, 1024, 65536])),
        7 => Setting::RandomPageCost(*g.pick(&[1, 10, 40, 400])),
        8 => Setting::CpuTupleCost(*g.pick(&[0, 1, 10, 1000])),
        _ => Setting::Parallel(g.bool_with(0.8)),
    }
}

/// Crafted statistics of some of `m`'s tables: a size many times off the
/// true one (or tiny), and per column a NULL fraction, width and distinct
/// count that may contradict the rows.
fn draw_stats(g: &mut Source, m: &Model) -> Vec<Stat> {
    let mut out = Vec::new();
    for t in &m.tables {
        if !g.bool_with(0.5) {
            continue;
        }
        let rows = t.rows.len() as f32;
        let tuples = match g.int(0..6) {
            0 | 1 => *g.pick(&[1e4, 1e6, 1e9]),
            2 => *g.pick(&[0.0, 1.0, 3.0]),
            3 => -1.0,
            4 => rows * *g.pick(&[2.0, 100.0]),
            _ => rows / 10.0,
        };
        // At most 1000 tuples per page: the planner scales the stored
        // density by the table's actual pages, and a hash index build sizes
        // its buckets by that estimate (10^9 tuples on one page made
        // gigabytes of empty buckets).
        let least = (tuples.max(0.0) / 1000.0).ceil() as u32;
        let pages = (*g.pick(&[0u32, 1, 10, 1000, 1_000_000])).max(least);
        out.push(Stat::Relation { table: t.id, pages, tuples, all_visible: if g.bool() { 0 } else { pages } });
        for c in &t.columns {
            if g.bool_with(0.4) {
                out.push(Stat::Attribute {
                    table: t.id,
                    column: c.id,
                    null_frac: *g.pick(&[0.0, 0.01, 0.5, 1.0]),
                    width: *g.pick(&[1, 4, 8, 100, 2000]),
                    distinct: *g.pick(&[-1.0, -0.5, -0.01, 1.0, 2.0, 100.0, 1e6]),
                });
            }
        }
    }
    if out.is_empty() {
        let t = &m.tables[g.index(m.tables.len())];
        out.push(Stat::Relation { table: t.id, pages: 1000, tuples: 1e6, all_visible: 0 });
    }
    out
}

/// A statement valid against `m`.
pub fn draw_statement(g: &mut Source, m: &Model, pool: &Pool) -> Statement {
    if m.tables.is_empty() || g.bool_with(0.15) {
        return match g.int(0..10) {
            0 if m.savepoints.is_empty() && !m.tables.is_empty() => Statement::Vacuum { tables: all_tables(m), full: g.bool_with(0.2), freeze: g.bool_with(0.2), analyze: g.bool_with(0.3) },
            1 if !m.tables.is_empty() => Statement::Analyze(all_tables(m)),
            2 => Statement::Checkpoint,
            3 if m.tables.len() < crate::model::MAX_TABLES => {
                // Its foreign key may still be refused here (a pending
                // parent): then none.
                let mut t = draw_new_table(g, m, pool);
                if !semantics::valid(m, &Statement::CreateTable(t.clone())) {
                    t.fk = None;
                }
                Statement::CreateTable(t)
            },
            _ => Statement::Set(draw_setting(g)),
        };
    }
    for _ in 0..8 {
        let t = &m.tables[g.index(m.tables.len())];
        let k = *g.weighted(&[
            (35, 0u8),
            (25, 1),
            (20, 2),
            (12, 3),
            (10, 4),
            (18, 5),
            (6, 6),
            (8, 7),
            (3, 8),
            (22, 9),
            (6, 10),
            (5, 11),
            (3, 12),
            (6, 13),
            (3, 14),
            (4, 15),
            (4, 16),
            (4, 17),
            (4, 18),
            (6, 19),
            (10, 20),
            (8, 21),
            (2, 22),
            (2, 23),
            (2, 24),
            (6, 25),
            (8, 26),
            (1, 27),
            (2, 28),
            (14, 29),
            (ALTER_TYPES, 30),
            (TRIPS, 31),
            (CONSTRAINTS, 32),
            (PARTITIONS, 33),
            (DOMAINS, 34),
            (KEYS, 35),
        ]);
        let s = match k {
            0 => Statement::Select(draw_select(g, t, pool)),
            1 if g.bool_with(INSERT_SELECTS) => match draw_insert_select(g, m, t, pool) {
                Some(s) => s,
                None => continue,
            },
            1 if g.bool_with(MERGES) => match draw_merge(g, m, t, pool, false) {
                Some(s) => s,
                None => continue,
            },
            2 | 3 if g.bool_with(JOIN_WRITES) => match draw_merge(g, m, t, pool, true) {
                Some(s) => s,
                None => continue,
            },
            1 if g.bool_with(UPSERTS) => match draw_upsert(g, m, t, pool) {
                Some(s) => s,
                None => continue,
            },
            1 => match draw_insert(g, m, t, pool) {
                Some(s) => Statement::Insert(s),
                None => continue,
            },
            2 => Statement::Update(draw_update(g, m, t, pool)),
            3 => Statement::Delete(Delete { table: t.id, filter: (!g.bool_with(0.05)).then(|| draw_pred(g, t, 3, Some(pool))) }),
            4 => Statement::Count { table: t.id, filter: g.bool_with(0.7).then(|| draw_pred(g, t, 2, Some(pool))) },
            5 => Statement::CreateIndex { table: t.id, def: draw_index(g, t, t.next_index, Some(pool)) },
            6 if !t.indexes.is_empty() => Statement::DropIndex { table: t.id, index: t.indexes[g.index(t.indexes.len())].id },
            7 => Statement::CreateTable(draw_new_table(g, m, pool)),
            8 => Statement::DropTable(t.id),
            // Savepoints: a ROLLBACK TO undoes what the subtransaction
            // wrote (its rows stay as dead tuples).
            9 => {
                let depth = m.savepoints.len();
                match g.int(0..6) {
                    _ if depth == 0 => Statement::Savepoint(0),
                    0 => Statement::Savepoint(depth),
                    1 | 2 => Statement::RollbackTo(g.index(depth)),
                    3 => Statement::Rollback,
                    _ => Statement::Release(g.index(depth)),
                }
            },
            10 => Statement::Analyze(vec![t.id]),
            11 => Statement::Stats(draw_stats(g, &Model { tables: vec![t.clone()], ..Model::default() })),
            12 => Statement::Truncate(t.id),
            13 => Statement::AddColumn { table: t.id, column: draw_new_column(g, t, pool) },
            14 => {
                let cols: Vec<u32> = t.columns.iter().filter(|c| !t.key.contains(&c.id)).map(|c| c.id).collect();
                if cols.is_empty() {
                    continue;
                }
                Statement::DropColumn { table: t.id, column: *g.pick(&cols) }
            },
            15 => {
                let c = &t.columns[g.index(t.columns.len())];
                Statement::SetNotNull { table: t.id, column: c.id, on: !c.not_null }
            },
            16 => {
                let c = &t.columns[g.index(t.columns.len())];
                Statement::SetDefault { table: t.id, column: c.id, value: g.bool_with(0.7).then(|| draw_value(g, c.ty, Some(pool))) }
            },
            17 => {
                let ok: Vec<u32> = t.indexes.iter().filter(|i| i.method == Method::Btree && i.filter.is_none()).map(|i| i.id).collect();
                if ok.is_empty() {
                    continue;
                }
                Statement::Cluster { table: t.id, index: *g.pick(&ok) }
            },
            18 => Statement::Reindex {
                table: t.id,
                index: (!t.indexes.is_empty() && g.bool()).then(|| t.indexes[g.index(t.indexes.len())].id),
                concurrently: m.savepoints.is_empty() && g.bool_with(0.3),
            },
            20 => Statement::Phys { table: t.id, op: draw_phys(g, t) },
            21 => match draw_fk(g, m, t) {
                Some(fk) => Statement::AddForeignKey { table: t.id, fk, not_valid: g.bool_with(0.3) },
                None => continue,
            },
            22 => Statement::DropForeignKey(t.id),
            23 => Statement::ValidateForeignKey(t.id),
            24 => Statement::AlterForeignKey { table: t.id, deferral: *g.pick(&[Deferral::No, Deferral::Immediate, Deferral::Deferred]) },
            25 => {
                let named: Vec<u32> = m.tables.iter().filter(|x| x.fk.as_ref().is_some_and(|f| f.deferral != Deferral::No)).map(|x| x.id).collect();
                if m.savepoints.len() != 1 || named.is_empty() && g.bool_with(0.7) {
                    continue;
                }
                Statement::SetConstraints { table: (!named.is_empty() && g.bool_with(0.4)).then(|| *g.pick(&named)), deferred: g.bool() }
            },
            26 => match draw_trigger(g, m, t, pool) {
                Some(trigger) => Statement::CreateTrigger { table: t.id, trigger },
                None => continue,
            },
            27 | 28 if !t.triggers.is_empty() => {
                let tr = &t.triggers[g.index(t.triggers.len())];
                if k == 27 { Statement::DropTrigger { table: t.id, id: tr.id } } else { Statement::EnableTrigger { table: t.id, id: tr.id, on: !tr.enabled } }
            },
            27 | 28 => continue,
            // A referenced row deleted, or its key changed: the foreign
            // keys' actions and checks.
            29 => match draw_parent_write(g, m, pool) {
                Some(s) => s,
                None => continue,
            },
            32 => match g.int(0..5) {
                0 | 1 => Statement::AddCheck { table: t.id, id: t.next_check, pred: draw_pred(g, t, 2, Some(pool)), not_valid: g.bool_with(0.3) },
                2 if !t.named.is_empty() => Statement::DropCheck { table: t.id, id: t.named[g.index(t.named.len())].0 },
                3 if !t.named.is_empty() => Statement::ValidateCheck { table: t.id, id: t.named[g.index(t.named.len())].0 },
                _ => {
                    let c = &t.columns[g.index(t.columns.len())];
                    Statement::SetNotNull { table: t.id, column: c.id, on: true }
                },
            },
            35 => match draw_key_change(g, t) {
                Some(s) if semantics::valid(m, &s) => s,
                Some(s @ Statement::AddKey { .. }) => match &s {
                    Statement::AddKey { table, columns, using } => match semantics::add_key(m, *table, columns, *using) {
                        Some(Err(code)) => Statement::Expect(code, Box::new(s.clone())),
                        _ => continue,
                    },
                    _ => continue,
                },
                _ => continue,
            },
            34 => match draw_domain(g, m, t, pool) {
                Some(s) => s,
                None => continue,
            },
            33 => match draw_partition_change(g, m, t, pool) {
                Some(s) => s,
                None => continue,
            },
            31 => match draw_trip(g, t) {
                Some(op) => Statement::Trip { table: t.id, op },
                None => continue,
            },
            30 => match draw_alter_type(g, m, t) {
                Some(s) => s,
                None => continue,
            },
            _ => Statement::Vacuum { tables: vec![t.id], full: g.bool_with(0.2), freeze: g.bool_with(0.2), analyze: g.bool_with(0.3) },
        };
        if semantics::valid(m, &s) {
            return returning(g, m, s);
        }
        // A write PostgreSQL refuses with one error: run where it is caught.
        if (matches!(s, Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::AddForeignKey { not_valid: false, .. } | Statement::AddCheck { not_valid: false, .. }) || CONSTRAINTS > 0 && matches!(s, Statement::SetNotNull { on: true, .. })) && g.bool_with(0.8) {
            let code = match &s {
                Statement::AddForeignKey { .. } => Some(fk::Code::ForeignKey),
                Statement::AddCheck { .. } => Some(fk::Code::Check),
                Statement::SetNotNull { .. } => Some(fk::Code::NotNull),
                _ => semantics::dml_write(m, &s).and_then(|(w, _)| w.errors.code()),
            };
            if let Some(code) = code {
                let e = Statement::Expect(code, Box::new(s.clone()));
                if semantics::valid(m, &e) {
                    return e;
                }
            }
        }
    }
    let t = &m.tables[g.index(m.tables.len())];
    Statement::Count { table: t.id, filter: None }
}

/// The weight of primary keys added and dropped.
const KEYS: u32 = 6;

/// `DROP CONSTRAINT t{t}_pkey` of a table with a key; for one without,
/// `ADD .. PRIMARY KEY` of 1 or 2 columns (the partition key among them),
/// or USING INDEX of a unique index now and then.
fn draw_key_change(g: &mut Source, t: &Table) -> Option<Statement> {
    if !t.key.is_empty() {
        return Some(Statement::DropKey(t.id));
    }
    let uniques: Vec<&Index> = t.indexes.iter().filter(|i| i.unique).collect();
    if !uniques.is_empty() && g.bool_with(0.4) {
        let i = uniques[g.index(uniques.len())];
        return Some(Statement::AddKey { table: t.id, columns: i.columns.iter().map(|c| c.column).collect(), using: Some(i.id) });
    }
    let mut columns = vec![t.columns[g.index(t.columns.len())].id];
    if let Some(p) = &t.partitioning {
        columns = vec![p.column];
    }
    if t.columns.len() > 1 && g.bool_with(0.35) {
        let c = t.columns[g.index(t.columns.len())].id;
        if !columns.contains(&c) {
            columns.push(c);
        }
    }
    Some(Statement::AddKey { table: t.id, columns, using: None })
}

/// The weight of domains: a column changed to a new domain over its type,
/// or its domain altered.
const DOMAINS: u32 = 10;

fn draw_domain(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Option<Statement> {
    let c = &t.columns[g.index(t.columns.len())];
    let at = t.pos(c.id)?;
    let shape = Table { columns: vec![c.clone()], rows: t.rows.iter().map(|r| vec![r[at].clone()]).collect(), ..Table::new(t.id, &[]) };
    let s = match &c.dom {
        Some(d) if g.bool_with(0.7) => {
            let op = match g.int(0..4) {
                0 => DomOp::AddCheck(draw_pred(g, &shape, 2, Some(pool)), g.bool_with(0.3)),
                1 => DomOp::DropCheck,
                2 => DomOp::Validate,
                _ => DomOp::NotNull(!d.not_null),
            };
            let s = Statement::AlterDomain { table: t.id, column: c.id, name: d.name, op };
            // NOT VALID is taken by any rows; drawn only on rows that keep it.
            if let Statement::AlterDomain { op: DomOp::AddCheck(_, true), .. } = &s {
                return semantics::valid(m, &s).then_some(s);
            }
            s
        },
        _ => {
            let dom = crate::model::Domain { name: t.next_check, not_null: g.bool_with(0.3), check: g.bool_with(0.75).then(|| draw_pred(g, &shape, 2, Some(pool))) };
            Statement::SetDomain { table: t.id, column: c.id, ty: c.ty, dom }
        },
    };
    if semantics::valid(m, &s) {
        return Some(s);
    }
    let code = match &s {
        Statement::SetDomain { table, column, dom, .. } => semantics::set_domain(m, *table, *column, dom)?.err()?,
        Statement::AlterDomain { table, column, op, .. } => semantics::alter_domain(m, *table, *column, op)?.err()?,
        _ => return None,
    };
    let e = Statement::Expect(code, Box::new(s));
    semantics::valid(m, &e).then_some(e)
}

/// The weight of partitions dropped and added (of a partitioned table).
const PARTITIONS: u32 = 0;

/// A partition of `t` detached and dropped, or a new one created (run
/// where its one error is caught when the default partition holds rows
/// of its bound).
fn draw_partition_change(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Option<Statement> {
    use crate::model::{PartBound, Strategy};
    let p = t.partitioning.as_ref()?;
    if g.bool_with(0.4) {
        return Some(Statement::DropPartition { table: t.id, name: p.name(g.index(p.parts.len())) });
    }
    let ty = t.column(p.column)?.ty;
    let name = (0..p.parts.len()).map(|k| p.name(k) + 1).max().unwrap_or(0);
    let bound = match p.strategy {
        Strategy::Hash => return None,
        _ if g.bool_with(0.15) => PartBound::Default,
        Strategy::Range => {
            let a = draw_value(g, ty, Some(pool));
            let b = draw_value(g, ty, Some(pool));
            if a == Value::Null || b == Value::Null {
                return None;
            }
            let (lo, hi) = if crate::model::compare(&a, &b).is_le() { (a, b) } else { (b, a) };
            PartBound::Range((!g.bool_with(0.1)).then_some(lo), (!g.bool_with(0.1)).then_some(hi))
        },
        Strategy::List => PartBound::List((0..g.int(1..=3)).map(|_| draw_value(g, ty, Some(pool))).collect()),
    };
    let s = Statement::AddPartition { table: t.id, name, bound };
    if semantics::valid(m, &s) {
        return Some(s);
    }
    let e = Statement::Expect(fk::Code::Check, Box::new(s));
    semantics::valid(m, &e).then_some(e)
}

/// The weight of named CHECK constraints added, dropped and validated,
/// and SET NOT NULL (refused on a column holding NULL).
const CONSTRAINTS: u32 = 0;

/// The weight of round trips (renames there and back, a partition
/// detached and attached) among the statements.
const TRIPS: u32 = 0;

/// A round trip through a schema change of one of `t`'s objects.
fn draw_trip(g: &mut Source, t: &Table) -> Option<crate::statements::Trip> {
    use crate::statements::Trip;
    let mut ops = vec![Trip::Column(t.columns[g.index(t.columns.len())].id), Trip::Table];
    if !t.indexes.is_empty() {
        ops.push(Trip::Index(t.indexes[g.index(t.indexes.len())].id));
    }
    if !t.key.is_empty() {
        ops.push(Trip::Key);
    }
    if t.fk.is_some() {
        ops.push(Trip::ForeignKey);
    }
    if !t.triggers.is_empty() {
        ops.push(Trip::Trigger(t.triggers[g.index(t.triggers.len())].id));
    }
    if let Some(p) = &t.partitioning {
        let k = g.index(p.parts.len());
        ops.push(Trip::Partition(k, p.clone()));
        ops.push(Trip::Partition(k, p.clone()));
    }
    Some(g.pick(&ops).clone())
}

/// The weight of `ALTER COLUMN .. TYPE` among the statements.
const ALTER_TYPES: u32 = 10;

/// `ALTER COLUMN .. TYPE` of a column of `t` to a type it converts to,
/// with USING when the conversion needs it (now and then without, or
/// with it when it does not); run where its one error is caught when it
/// must fail (see [`semantics::alter_type`]).
fn draw_alter_type(g: &mut Source, m: &Model, t: &Table) -> Option<Statement> {
    let c = &t.columns[g.index(t.columns.len())];
    let to: Vec<crate::model::Type> = crate::model::Type::ALL.iter().copied().filter(|&ty| semantics::cast_needs_using(c.ty, ty).is_some()).collect();
    if to.is_empty() {
        return None;
    }
    let ty = *g.pick(&to);
    let using = if semantics::cast_needs_using(c.ty, ty) == Some(true) { g.bool_with(0.85) } else { g.bool_with(0.15) };
    let s = Statement::AlterType { table: t.id, column: c.id, ty, using };
    match semantics::alter_type(m, t.id, c.id, ty, using)? {
        Ok(_) => Some(s),
        Err(code) => Some(Statement::Expect(code, Box::new(s))),
    }
}

/// A storage or estimation change of `t` (maybe invalid for it: the caller
/// checks).
fn draw_phys(g: &mut Source, t: &Table) -> Phys {
    let col = |g: &mut Source| t.columns[g.index(t.columns.len())].id;
    match g.int(0..8) {
        0 => Phys::Fillfactor(g.bool_with(0.8).then(|| *g.pick(&[10, 30, 50, 90, 100]))),
        1 => Phys::Logged(g.bool()),
        2 => Phys::Target(col(g), *g.pick(&[-1, 0, 1, 5, 100, 10000])),
        3 => Phys::NDistinct(col(g), *g.pick(&[-100, -50, -1, 100, 300, 100_000])),
        4 | 5 => {
            let mut ids: Vec<u32> = t.columns.iter().map(|c| c.id).collect();
            let mut cols = Vec::new();
            for _ in 0..g.int(2..=3) {
                if ids.is_empty() {
                    break;
                }
                cols.push(ids.remove(g.index(ids.len())));
            }
            let kinds = if g.bool_with(0.4) { [false; 3] } else { [g.bool(), g.bool(), g.bool()] };
            Phys::CreateStats(g.int(0u8..=2), kinds, cols)
        },
        6 => Phys::DropStats(g.int(0u8..=2)),
        _ => {
            let ok: Vec<u32> = t.indexes.iter().filter(|i| i.method == Method::Btree && i.filter.is_none()).map(|i| i.id).collect();
            Phys::ClusterOn((!ok.is_empty() && g.bool_with(0.8)).then(|| *g.pick(&ok)))
        },
    }
}

/// A new empty table: 1 to 4 columns, a key now and then, a CHECK
/// constraint now and then.
fn draw_new_table(g: &mut Source, m: &Model, pool: &Pool) -> Table {
    let n = g.int(1..=4);
    let mut t = Table::new(m.next_table, &[]);
    t.columns = draw_columns(g, n, Some(pool));
    t.next_column = n as u32;
    if g.bool_with(0.35) {
        t.key.push(t.columns[g.index(n)].id);
    }
    if g.bool_with(0.15) {
        t.checks.push(draw_pred(g, &t, 2, Some(pool)));
    }
    if g.bool_with(0.2) {
        t.partitioning = crate::setup::draw_partitioning(g, &t, Some(pool));
        if let Some(p) = &t.partitioning {
            if !t.key.is_empty() && !t.key.contains(&p.column) {
                t.key.push(p.column);
            }
        }
    }
    if g.bool_with(0.35) {
        t.fk = draw_fk(g, m, &t).filter(|f| fk::def_ok(&m.tables, &t, f));
    }
    t
}

/// A foreign key for `child` (not checked against the rows): a unique
/// constraint of a table of `m` or of `child` itself as the parent, child
/// columns of matching types, drawn actions, MATCH FULL and deferrability
/// now and then. None when no parent fits.
pub fn draw_fk(g: &mut Source, m: &Model, child: &Table) -> Option<ForeignKey> {
    let mut parents: Vec<&Table> = m.tables.iter().filter(|p| p.id != child.id && !p.uniques().is_empty()).collect();
    if !child.uniques().is_empty() && (parents.is_empty() || g.bool_with(0.2)) {
        parents = vec![child];
    }
    if parents.is_empty() {
        return None;
    }
    let parent = parents[g.index(parents.len())];
    let uniques = parent.uniques();
    let (pcols, _) = &uniques[g.index(uniques.len())];
    if pcols.len() > 3 {
        return None;
    }
    let mut columns = Vec::new();
    for pc in pcols {
        let pty = parent.column(*pc)?.ty;
        let fits: Vec<u32> = child
            .columns
            .iter()
            .filter(|c| !c.identity && fk::types_match(c.ty, pty) && !columns.contains(&c.id) && (parent.id != child.id || !pcols.contains(&c.id)))
            .map(|c| c.id)
            .collect();
        if fits.is_empty() {
            return None;
        }
        columns.push(*g.pick(&fits));
    }
    Some(draw_fk_options(g, columns, parent.id, pcols.clone()))
}

/// A trigger for `t` (checked by the caller): a BEFORE ROW trigger that
/// sets a column (on INSERT, UPDATE or both, UPDATE OF a column now and
/// then) or skips inserted rows, or an AFTER trigger that copies columns
/// into another table of the same types (a row trigger from NEW or OLD, a
/// statement trigger from its transition table); WHEN now and then.
pub fn draw_trigger(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Option<Trigger> {
    let id = t.next_trigger;
    let col_ids: Vec<u32> = t.columns.iter().map(|c| c.id).collect();
    let mut tr = Trigger { id, before: true, row: true, events: [true, false, false], update_of: Vec::new(), when: None, action: TrigAction::Skip, enabled: true };
    match g.int(0..10) {
        0..=3 => {
            let cols: Vec<&Column> = t.columns.iter().filter(|c| !c.identity && t.partitioning.as_ref().is_none_or(|p| p.column != c.id)).collect();
            if cols.is_empty() {
                return None;
            }
            let c = cols[g.index(cols.len())];
            tr.action = TrigAction::Set(c.id, draw_value(g, c.ty, Some(pool)));
            tr.events = *g.pick(&[[true, false, false], [false, true, false], [true, true, false]]);
            if tr.events[1] && g.bool_with(0.3) {
                tr.update_of.push(*g.pick(&col_ids));
            }
            if g.bool_with(0.5) {
                tr.when = Some(draw_pred(g, t, 2, Some(pool)));
            }
        },
        4 => tr.when = g.bool_with(0.85).then(|| draw_pred(g, t, 2, Some(pool))),
        _ => {
            let targets: Vec<&Table> = m
                .tables
                .iter()
                .filter(|x| x.id != t.id && x.fk.is_none() && x.triggers.is_empty() && x.identity().is_none() && !m.tables.iter().any(|y| y.fk.as_ref().is_some_and(|f| f.parent == x.id)))
                .collect();
            if targets.is_empty() {
                return None;
            }
            let target = targets[g.index(targets.len())];
            let mut cols = Vec::new();
            for d in &target.columns {
                let srcs: Vec<u32> = t.columns.iter().filter(|c| c.ty == d.ty).map(|c| c.id).collect();
                if !srcs.is_empty() && (cols.is_empty() || g.bool_with(0.7)) {
                    cols.push((*g.pick(&srcs), d.id));
                }
            }
            if cols.is_empty() {
                return None;
            }
            tr.before = false;
            tr.action = TrigAction::Audit { target: target.id, cols };
            tr.row = g.bool_with(0.6);
            tr.events = if tr.row { *g.pick(&[[true, false, false], [false, true, false], [false, false, true], [true, true, false]]) } else { *g.pick(&[[true, false, false], [false, true, false], [false, false, true]]) };
            if tr.row && tr.events[1] && g.bool_with(0.2) {
                tr.update_of.push(*g.pick(&col_ids));
            }
            if tr.row && g.bool_with(0.3) {
                tr.when = Some(draw_pred(g, t, 2, Some(pool)));
            }
        },
    }
    tr.enabled = !g.bool_with(0.05);
    Some(tr)
}

/// A DELETE of referenced rows, or an UPDATE of a referenced key column,
/// of a table some foreign key references, selecting rows by a key value
/// a referencing row holds (mostly) so that the actions find rows.
fn draw_parent_write(g: &mut Source, m: &Model, pool: &Pool) -> Option<Statement> {
    let kids: Vec<&Table> = m.tables.iter().filter(|c| c.fk.is_some()).collect();
    if kids.is_empty() {
        return None;
    }
    let child = kids[g.index(kids.len())];
    let f = child.fk.as_ref().unwrap();
    let parent = m.table(f.parent)?;
    if parent.rows.is_empty() {
        return None;
    }
    let k = g.index(f.parent_columns.len());
    let pc = f.parent_columns[k];
    let p = parent.pos(pc)?;
    let referenced: Vec<&Value> = child.rows.iter().filter_map(|r| child.pos(f.columns[k]).map(|x| &r[x])).filter(|v| **v != Value::Null).collect();
    let v = if !referenced.is_empty() && g.bool_with(0.8) { referenced[g.index(referenced.len())].clone() } else { parent.rows[g.index(parent.rows.len())][p].clone() };
    if v == Value::Null {
        return None;
    }
    let filter = Some(Pred::Cmp(pc, CmpOp::Eq, Lit::plain(v)));
    if g.bool() {
        return Some(Statement::Delete(Delete { table: parent.id, filter }));
    }
    let ty = parent.columns[p].ty;
    let col = &parent.columns[p];
    let new = if !col.not_null && !parent.key.contains(&pc) && g.bool_with(0.15) { Value::Null } else { draw_value(g, ty, Some(pool)) };
    Some(Statement::Update(Update { table: parent.id, sets: vec![(pc, new)], filter }))
}

/// Actions, match type and deferrability for a foreign key.
pub fn draw_fk_options(g: &mut Source, columns: Vec<u32>, parent: u32, parent_columns: Vec<u32>) -> ForeignKey {
    let action = |g: &mut Source| *g.weighted(&[(3, Action::NoAction), (2, Action::Restrict), (4, Action::Cascade), (3, Action::SetNull), (2, Action::SetDefault)]);
    let on_delete = action(g);
    let full = columns.len() > 1 && g.bool_with(0.2);
    let mut delete_set = Vec::new();
    if matches!(on_delete, Action::SetNull | Action::SetDefault) && !full && columns.len() > 1 && g.bool_with(0.4) {
        delete_set.push(*g.pick(&columns));
    }
    ForeignKey {
        columns,
        parent,
        parent_columns,
        on_delete,
        delete_set,
        on_update: action(g),
        full,
        deferral: *g.weighted(&[(6, Deferral::No), (2, Deferral::Immediate), (2, Deferral::Deferred)]),
    }
}

/// A new column for ADD COLUMN: a default half the time (PostgreSQL's fast
/// default: no rewrite, every row reads it), NOT NULL with one now and
/// then.
fn draw_new_column(g: &mut Source, t: &Table, pool: &Pool) -> Column {
    let ty = draw_type(g);
    let mut c = Column::new(t.next_column, ty);
    if g.bool() {
        c.default = Some(draw_value(g, ty, Some(pool)));
        c.not_null = g.bool_with(0.3);
    }
    if ty.varlena() && g.bool_with(0.2) {
        c.storage = Some(*g.pick(&[Storage::Main, Storage::External, Storage::Extended]));
    }
    if ty.varlena() && g.bool_with(0.1) {
        c.compression = Some(*g.pick(&[Compression::Pglz, Compression::Lz4]));
    }
    c
}

fn draw_select(g: &mut Source, t: &Table, pool: &Pool) -> Select {
    let ids: Vec<u32> = t.columns.iter().map(|c| c.id).collect();
    let columns = if g.bool_with(0.4) { Vec::new() } else { (0..g.int(1..=3)).map(|_| *g.pick(&ids)).collect() };
    let filter = g.bool_with(0.8).then(|| draw_pred(g, t, 3, Some(pool)));
    let mut order = Vec::new();
    if g.bool_with(0.4) {
        let mut cols = ids.clone();
        for _ in 0..g.int(1..=cols.len().min(2)) {
            let column = cols.remove(g.index(cols.len()));
            order.push(OrderTerm { column, desc: g.bool_with(0.3), nulls_first: g.bool_with(0.2).then(|| g.bool()) });
        }
    }
    let limit = g.bool_with(if order.is_empty() { 0.08 } else { 0.35 }).then(|| *g.pick(&[0, 1, 2, 3, 5, 10]));
    Select { table: t.id, columns, filter, order, limit }
}

/// An INSERT of 1 to 4 rows (or `DEFAULT VALUES`), the identity column left
/// to its sequence mostly; rows that would break a constraint are left
/// out, and None when none is left.
fn draw_insert(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Option<Insert> {
    let mut columns: Vec<u32> = Vec::new();
    for c in &t.columns {
        let needed = (c.not_null || t.key.contains(&c.id)) && c.default.is_none() && !c.identity;
        if needed || (if c.identity { g.bool_with(0.15) } else { g.bool_with(0.8) }) {
            columns.push(c.id);
        }
    }
    if columns.is_empty() && !g.bool_with(0.1) {
        columns = t.columns.iter().filter(|c| !c.identity).map(|c| c.id).collect();
    }
    let mut ins = Insert { table: t.id, columns: columns.clone(), rows: Vec::new() };
    let n = if columns.is_empty() { 1 } else { g.int(1..=4) };
    let next = next_value(m, t.id);
    for _ in 0..n {
        let row: Vec<Value> = columns
            .iter()
            .map(|&id| {
                let c = t.column(id).unwrap();
                if !(c.not_null || c.identity || t.key.contains(&id)) && g.bool_with(0.1) {
                    Value::Null
                } else if g.bool_with(0.15) {
                    fresh(g, c.ty)
                } else {
                    draw_value(g, c.ty, Some(pool))
                }
            })
            .collect();
        ins.rows.push(row);
        reference(g, m, t, &ins.columns.clone(), ins.rows.last_mut().unwrap());
        if inserted_rows(t, next, &ins).is_none() {
            ins.rows.pop();
        }
    }
    (!ins.rows.is_empty()).then_some(ins)
}

/// How often an INSERT is an `INSERT .. ON CONFLICT`, and a write has
/// RETURNING.
const UPSERTS: f64 = 0.45;
const RETURNINGS: f64 = 0.3;

/// How often an INSERT is a MERGE instead, and an UPDATE or DELETE one
/// joined to a source (`UPDATE .. FROM`, `DELETE .. USING`).
const MERGES: f64 = 0.3;
const JOIN_WRITES: f64 = 0.25;
/// How often an INSERT is an INSERT .. SELECT, and a MERGE's source a
/// table's rows.
const INSERT_SELECTS: f64 = 0.15;
const MERGE_FROM_TABLE: f64 = 0.35;

/// A query of the rows of a table of `m` (`t` itself now and then) as
/// values of `columns` of `t`: for each column a column of the same type,
/// a filter mostly. None when some column has no match.
fn draw_from_table(g: &mut Source, m: &Model, t: &Table, columns: &[u32], pool: &Pool) -> Option<FromTable> {
    let fits = |s: &Table| columns.iter().all(|c| t.column(*c).is_some_and(|col| s.columns.iter().any(|x| x.ty == col.ty)));
    let mut cands: Vec<&Table> = m.tables.iter().filter(|s| s.id != t.id && fits(s)).collect();
    if cands.is_empty() || g.bool_with(0.25) {
        cands.push(t);
    }
    let s = cands[g.index(cands.len())];
    let mut cols = Vec::new();
    for c in columns {
        let ty = t.column(*c)?.ty;
        let same: Vec<u32> = s.columns.iter().filter(|x| x.ty == ty).map(|x| x.id).collect();
        if same.is_empty() {
            return None;
        }
        cols.push(*g.pick(&same));
    }
    Some(FromTable { table: s.id, columns: cols, filter: g.bool_with(0.8).then(|| draw_pred(g, s, 2, Some(pool))) })
}

/// `INSERT INTO t (c..) SELECT ..` of another table's rows (or its own):
/// columns of types some table shares, the identity column among them.
fn draw_insert_select(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Option<Statement> {
    let mut columns: Vec<u32> = t.columns.iter().filter(|c| c.identity || c.not_null || t.key.contains(&c.id) || g.bool_with(0.6)).map(|c| c.id).collect();
    if columns.is_empty() {
        columns.push(t.columns[g.index(t.columns.len())].id);
    }
    let from = draw_from_table(g, m, t, &columns, pool)?;
    Some(Statement::InsertSelect { table: t.id, columns, from })
}

/// A MERGE into `t` (none for a table with triggers): a source of 1 to 4
/// rows over some of its columns (the key or identity among them), many
/// taking an existing row's values so that they match, joined on one or
/// two of them; 1 to 3 WHEN clauses. Rows and clauses are dropped from the
/// end until it is valid; None when nothing is.
fn draw_merge(g: &mut Source, m: &Model, t: &Table, pool: &Pool, join: bool) -> Option<Statement> {
    if !t.triggers.is_empty() {
        return None;
    }
    let uniques = t.uniques();
    let mut columns: Vec<u32> = Vec::new();
    for c in &t.columns {
        let needed = c.identity || ((c.not_null || t.key.contains(&c.id)) && c.default.is_none());
        if needed || g.bool_with(0.6) {
            columns.push(c.id);
        }
    }
    if columns.is_empty() {
        columns.push(t.columns[g.index(t.columns.len())].id);
    }
    let pos: Vec<usize> = columns.iter().map(|&c| t.pos(c).unwrap()).collect();
    // Join on a unique constraint's columns mostly (when the source has
    // them), else on one or two columns.
    let keyed: Vec<&Vec<u32>> = uniques.iter().map(|(c, _)| c).filter(|u| u.iter().all(|c| columns.contains(c))).collect();
    let on: Vec<u32> = if !keyed.is_empty() && g.bool_with(0.7) {
        keyed[g.index(keyed.len())].clone()
    } else {
        let mut cs = columns.clone();
        (0..g.int(1..=2usize).min(cs.len())).map(|_| cs.remove(g.index(cs.len()))).collect()
    };
    let mut rows = Vec::new();
    for _ in 0..g.int(1..=4) {
        let mut row: Vec<Value> = columns.iter().map(|&id| draw_value(g, t.column(id).unwrap().ty, Some(pool))).collect();
        if !t.rows.is_empty() && g.bool_with(0.65) {
            let old = &t.rows[g.index(t.rows.len())];
            for (k, &p) in pos.iter().enumerate() {
                if on.contains(&columns[k]) || g.bool_with(0.5) {
                    row[k] = old[p].clone();
                }
            }
        }
        reference(g, m, t, &columns, &mut row);
        rows.push(row);
    }
    let src = Table { columns: t.columns.iter().filter(|c| columns.contains(&c.id)).cloned().collect(), rows: Vec::new(), indexes: Vec::new(), key: Vec::new(), checks: Vec::new(), named: Vec::new(), ..t.clone() };
    let mut whens: Vec<When> = Vec::new();
    let mut terminal = [false; 3];
    for _ in 0..if join { 1 } else { g.int(1..=3) } {
        let kind = if join { MergeKind::Matched } else { *g.weighted(&[(5, MergeKind::Matched), (4, MergeKind::NotMatched), (2, MergeKind::BySource)]) };
        if terminal[kind as usize] {
            continue;
        }
        let cond = g.bool_with(0.4).then(|| if kind == MergeKind::NotMatched { draw_pred(g, &src, 2, Some(pool)) } else { draw_pred(g, t, 2, Some(pool)) });
        terminal[kind as usize] |= cond.is_none();
        let action = match kind {
            MergeKind::NotMatched => {
                if g.bool_with(0.85) {
                    MergeAction::Insert
                } else {
                    MergeAction::Nothing
                }
            },
            _ => match g.int(0..10) {
                0..=5 => {
                    let mut cols: Vec<&Column> = t.columns.iter().filter(|c| !c.identity).collect();
                    let mut sets = Vec::new();
                    for _ in 0..g.int(1..=2) {
                        if cols.is_empty() {
                            break;
                        }
                        let c = cols.remove(g.index(cols.len()));
                        let v = if kind == MergeKind::Matched && columns.contains(&c.id) && g.bool_with(0.6) { SetVal::Excluded(c.id) } else { SetVal::Const(draw_value(g, c.ty, Some(pool))) };
                        sets.push((c.id, v));
                    }
                    if sets.is_empty() {
                        MergeAction::Delete
                    } else {
                        MergeAction::Update(sets)
                    }
                },
                6..=8 => MergeAction::Delete,
                _ if join => MergeAction::Delete,
                _ => MergeAction::Nothing,
            },
        };
        whens.push(When { kind, cond, action });
    }
    if whens.is_empty() {
        return None;
    }
    let types = columns.iter().map(|&c| (c, t.column(c).unwrap().ty)).collect();
    let from = if g.bool_with(MERGE_FROM_TABLE) { draw_from_table(g, m, t, &columns, pool) } else { None };
    if from.is_some() {
        let mg = Merge { table: t.id, columns: types, rows: Vec::new(), from, on: on.clone(), whens: whens.clone() };
        let s = if join { Statement::JoinWrite(mg) } else { Statement::Merge(mg) };
        if semantics::valid(m, &s) {
            return Some(s);
        }
    }
    let types = columns.iter().map(|&c| (c, t.column(c).unwrap().ty)).collect();
    let mut mg = Merge { table: t.id, columns: types, rows, from: None, on, whens };
    loop {
        let s = if join { Statement::JoinWrite(mg.clone()) } else { Statement::Merge(mg.clone()) };
        if semantics::valid(m, &s) {
            return Some(s);
        }
        if mg.rows.len() > 1 {
            mg.rows.pop();
        } else if mg.whens.len() > 1 {
            mg.whens.pop();
        } else {
            return None;
        }
    }
}

/// A valid write with RETURNING now and then: 1 to 3 items, old and new
/// rows now and then, as a data-modifying CTE now and then.
fn returning(g: &mut Source, m: &Model, s: Statement) -> Statement {
    let Some(t) = s.write_table().and_then(|t| m.table(t)) else { return s };
    if !g.bool_with(RETURNINGS) {
        return s;
    }
    let items = (0..g.int(1..=3)).map(|_| (*g.weighted(&[(6, Which::Plain), (2, Which::Old), (2, Which::New)]), t.columns[g.index(t.columns.len())].id)).collect();
    let cte = g.bool_with(0.35);
    let r = Statement::Returning(Box::new(s.clone()), Returning { items, cte, snapshot: cte && g.bool() });
    if semantics::valid(m, &r) { r } else { s }
}

/// An `INSERT .. ON CONFLICT` of 1 to 4 rows into `t`, many of them taking
/// an existing row's values (so that they conflict): DO NOTHING (with or
/// without a target) or DO UPDATE of the arbiter's conflicting row. Rows
/// are dropped from the end until it is valid; None when none is.
fn draw_upsert(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Option<Statement> {
    let uniques = t.uniques();
    let mut columns: Vec<u32> = Vec::new();
    for c in &t.columns {
        let needed = (c.not_null || t.key.contains(&c.id) || uniques.iter().any(|(u, _)| u.contains(&c.id))) && c.default.is_none() && !c.identity;
        if needed || (if c.identity { g.bool_with(0.1) } else { g.bool_with(0.75) }) {
            columns.push(c.id);
        }
    }
    let pos: Vec<usize> = columns.iter().map(|&c| t.pos(c).unwrap()).collect();
    let mut ins = Insert { table: t.id, columns: columns.clone(), rows: Vec::new() };
    let n = if columns.is_empty() { 1 } else { g.int(1..=4) };
    for _ in 0..n {
        let mut row: Vec<Value> = columns.iter().map(|&id| draw_value(g, t.column(id).unwrap().ty, Some(pool))).collect();
        if !t.rows.is_empty() && g.bool_with(0.6) {
            let old = &t.rows[g.index(t.rows.len())];
            for (k, &p) in pos.iter().enumerate() {
                if g.bool_with(0.8) {
                    row[k] = old[p].clone();
                }
            }
        }
        reference(g, m, t, &columns, &mut row);
        ins.rows.push(row);
    }
    let target = match g.int(0..10) {
        0..=1 => None,
        2 if !t.key.is_empty() => Some(Target::Key),
        _ if !uniques.is_empty() => {
            let (cols, _) = &uniques[g.index(uniques.len())];
            let mut cols = cols.clone();
            // Inference takes the columns in any order.
            if g.bool_with(0.3) {
                cols.reverse();
            }
            Some(Target::Columns(cols))
        },
        _ => None,
    };
    let update = (target.is_some() && g.bool_with(0.65)).then(|| {
        let mut cols: Vec<&Column> = t.columns.iter().filter(|c| !c.identity).collect();
        let mut sets = Vec::new();
        for _ in 0..g.int(1..=2) {
            if cols.is_empty() {
                break;
            }
            let c = cols.remove(g.index(cols.len()));
            let same: Vec<u32> = t.columns.iter().filter(|x| x.ty == c.ty).map(|x| x.id).collect();
            let v = if g.bool_with(0.6) { SetVal::Excluded(if g.bool_with(0.7) { c.id } else { *g.pick(&same) }) } else { SetVal::Const(draw_value(g, c.ty, Some(pool))) };
            sets.push((c.id, v));
        }
        (sets, g.bool_with(0.3).then(|| draw_pred(g, t, 2, Some(pool))))
    });
    let oc = OnConflict { target, update };
    while !ins.rows.is_empty() {
        let s = Statement::Upsert(ins.clone(), oc.clone());
        if semantics::valid(m, &s) {
            return Some(s);
        }
        if ins.columns.is_empty() {
            break;
        }
        ins.rows.pop();
    }
    None
}

/// Values of a row's foreign key columns (those among `columns`) from a
/// row of the parent now and then, so that most writes keep the foreign
/// key and some do not.
fn reference(g: &mut Source, m: &Model, t: &Table, columns: &[u32], row: &mut [Value]) {
    let Some(f) = &t.fk else { return };
    let parent = if f.parent == t.id { Some(t) } else { m.table(f.parent) };
    let Some(parent) = parent else { return };
    if parent.rows.is_empty() || !g.bool_with(0.85) {
        return;
    }
    let pr = &parent.rows[g.index(parent.rows.len())];
    for (c, pc) in f.columns.iter().zip(&f.parent_columns) {
        if let (Some(k), Some(pp)) = (columns.iter().position(|x| x == c), parent.pos(*pc)) {
            row[k] = pr[pp].clone();
        }
    }
}

fn draw_update(g: &mut Source, m: &Model, t: &Table, pool: &Pool) -> Update {
    let mut cols: Vec<&Column> = t.columns.iter().collect();
    let mut sets = Vec::new();
    for _ in 0..g.int(1..=cols.len().min(2)) {
        let c = cols.remove(g.index(cols.len()));
        let v = if !c.not_null && !c.identity && !t.key.contains(&c.id) && g.bool_with(0.1) { Value::Null } else { draw_value(g, c.ty, Some(pool)) };
        sets.push((c.id, v));
    }
    let ids: Vec<u32> = sets.iter().map(|(c, _)| *c).collect();
    let mut vals: Vec<Value> = sets.iter().map(|(_, v)| v.clone()).collect();
    reference(g, m, t, &ids, &mut vals);
    for (s, v) in sets.iter_mut().zip(vals) {
        s.1 = v;
    }
    Update { table: t.id, sets, filter: (!g.bool_with(0.1)).then(|| draw_pred(g, t, 3, Some(pool))) }
}

// ------------------------------------------------------------------ measure

fn bits(x: u64) -> u64 {
    u64::from(64 - x.leading_zeros())
}

fn filter_cost(p: &Option<Pred>) -> u64 {
    p.as_ref().map_or(0, Pred::cost)
}

/// A statement's simplicity measure, a natural number.
pub fn statement_cost(s: &Statement) -> u64 {
    1 + match s {
        Statement::Select(s) => {
            s.columns.len() as u64
                + s.order.iter().map(|o| 1 + u64::from(o.desc) + u64::from(o.nulls_first.is_some())).sum::<u64>()
                + s.limit.map_or(0, |n| 1 + bits(u64::from(n)))
                + filter_cost(&s.filter)
        },
        Statement::Count { filter, .. } => 1 + filter_cost(filter),
        Statement::Insert(i) => 1 + i.columns.len() as u64 + i.rows.iter().map(|r| 1 + r.iter().map(Value::cost).sum::<u64>()).sum::<u64>(),
        Statement::Update(u) => 1 + u.sets.iter().map(|(_, v)| 1 + v.cost()).sum::<u64>() + filter_cost(&u.filter),
        Statement::Delete(d) => 1 + filter_cost(&d.filter),
        Statement::Truncate(_) | Statement::DropTable(_) | Statement::DropIndex { .. } | Statement::DropColumn { .. } => 1,
        Statement::CreateTable(t) => crate::model::table_cost(t),
        Statement::CreateIndex { def, .. } => crate::model::index_cost(def),
        Statement::AddColumn { column, .. } => 1 + u64::from(column.not_null) + column.default.as_ref().map_or(0, |v| 1 + v.cost()) + u64::from(column.storage.is_some()) + u64::from(column.compression.is_some()),
        Statement::SetNotNull { on, .. } => 1 + u64::from(*on),
        Statement::SetDefault { value, .. } => 1 + value.as_ref().map_or(0, |v| 1 + v.cost()),
        Statement::AlterType { using, .. } => 2 + u64::from(*using),
        Statement::DropPartition { .. } => 1,
        Statement::AddKey { columns, using, .. } => 1 + columns.len() as u64 + u64::from(using.is_some()),
        Statement::DropKey(_) => 1,
        Statement::SetDomain { dom, .. } => 2 + u64::from(dom.not_null) + dom.check.as_ref().map_or(0, |p| 1 + p.cost()),
        Statement::AlterDomain { op, .. } => 1 + match op {
            DomOp::AddCheck(p, nv) => p.cost() + u64::from(*nv),
            _ => 1,
        },
        Statement::AddPartition { bound, .. } => crate::model::Partitioning { strategy: crate::model::Strategy::List, column: 0, parts: vec![bound.clone()], names: Vec::new() }.cost(),
        Statement::Trip { .. } | Statement::DropCheck { .. } | Statement::ValidateCheck { .. } => 1,
        Statement::AddCheck { pred, not_valid, .. } => 1 + pred.cost() + u64::from(*not_valid),
        Statement::Vacuum { tables, full, freeze, analyze } => tables.len() as u64 + u64::from(*full) + u64::from(*freeze) + u64::from(*analyze),
        Statement::Analyze(tables) => tables.len() as u64,
        Statement::Cluster { .. } => 2,
        Statement::Reindex { index, concurrently, .. } => 1 + u64::from(index.is_some()) + u64::from(*concurrently),
        Statement::Checkpoint => 0,
        Statement::Set(s) => match s {
            Setting::Enable(_, on) => 1 + u64::from(!*on),
            _ => 2,
        },
        Statement::Stats(rows) => rows.len() as u64 * 4,
        Statement::Savepoint(_) | Statement::RollbackTo(_) | Statement::Release(_) | Statement::Rollback => 1,
        Statement::Phys { op, .. } => op.cost(),
        Statement::AddForeignKey { fk, not_valid, .. } => fk.cost() + u64::from(*not_valid),
        Statement::DropForeignKey(_) | Statement::ValidateForeignKey(_) | Statement::AlterForeignKey { .. } | Statement::SetConstraints { .. } => 1,
        Statement::Expect(_, s) => 1 + statement_cost(s),
        Statement::Upsert(i, oc) => {
            let target = match &oc.target {
                None => 0,
                Some(Target::Key) => 1,
                Some(Target::Columns(c)) => c.len() as u64,
            };
            let update = oc.update.as_ref().map_or(0, |(sets, filter)| 1 + sets.iter().map(|(_, v)| 1 + if let SetVal::Const(v) = v { v.cost() } else { 0 }).sum::<u64>() + filter_cost(filter));
            1 + statement_cost(&Statement::Insert(i.clone())) + target + update
        },
        Statement::Merge(mg) | Statement::JoinWrite(mg) => {
            1 + mg.columns.len() as u64
                + mg.on.len() as u64
                + mg.from.as_ref().map_or(0, |f| 1 + filter_cost(&f.filter))
                + mg.rows.iter().map(|r| 1 + r.iter().map(Value::cost).sum::<u64>()).sum::<u64>()
                + mg.whens
                    .iter()
                    .map(|w| {
                        1 + filter_cost(&w.cond)
                            + match &w.action {
                                MergeAction::Update(sets) => 1 + sets.iter().map(|(_, v)| 1 + if let SetVal::Const(v) = v { v.cost() } else { 0 }).sum::<u64>(),
                                MergeAction::Delete | MergeAction::Insert => 1,
                                MergeAction::Nothing => 0,
                            }
                    })
                    .sum::<u64>()
        },
        Statement::InsertSelect { columns, from, .. } => 2 + columns.len() as u64 + filter_cost(&from.filter),
        Statement::Returning(inner, r) => 1 + statement_cost(inner) + r.items.iter().map(|(w, _)| 1 + u64::from(*w != Which::Plain)).sum::<u64>() + u64::from(r.cte) + u64::from(r.snapshot),
        Statement::CreateTrigger { trigger, .. } => trigger.cost(),
        Statement::DropTrigger { .. } | Statement::EnableTrigger { .. } => 1,
    }
}

/// The measure every shrink step strictly decreases, lexicographically:
/// the setup's ([`Setup::cost`]), then the statements' total cost.
pub fn measure(c: &Case) -> (u64, u64) {
    (c.setup.cost(), c.statements.iter().map(statement_cost).sum())
}

// ------------------------------------------------------------------- repair

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

fn repair_filter(t: &Table, f: &Option<Pred>) -> Option<Option<Pred>> {
    match f {
        Some(p) => Some(Some(repair_pred(t, p)?)),
        None => Some(None),
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
            let t = m.table(s.table)?;
            Statement::Select(Select {
                table: s.table,
                columns: s.columns.iter().copied().filter(|&c| t.column(c).is_some()).collect(),
                filter: s.filter.as_ref().and_then(|p| repair_pred(t, p)),
                order: s.order.iter().copied().filter(|o| t.column(o.column).is_some()).collect(),
                limit: s.limit,
            })
        },
        Statement::Count { table, filter } => {
            let t = m.table(*table)?;
            Statement::Count { table: *table, filter: filter.as_ref().and_then(|p| repair_pred(t, p)) }
        },
        Statement::Insert(i) => {
            let t = m.table(i.table)?;
            let keep: Vec<usize> = (0..i.columns.len()).filter(|&k| t.column(i.columns[k]).is_some_and(|c| c.ty.admits(&i.rows[0][k]) || i.rows.iter().any(|r| c.ty.admits(&r[k])))).collect();
            let mut fixed = Insert { table: i.table, columns: keep.iter().map(|&k| i.columns[k]).collect(), rows: Vec::new() };
            let next = next_value(m, t.id);
            for r in &i.rows {
                fixed.rows.push(keep.iter().map(|&k| r[k].clone()).collect());
                if inserted_rows(t, next, &fixed).is_none() {
                    fixed.rows.pop();
                }
            }
            if fixed.rows.is_empty() {
                return None;
            }
            Statement::Insert(fixed)
        },
        Statement::Update(u) => {
            let t = m.table(u.table)?;
            let sets: Vec<(u32, Value)> = u.sets.iter().filter(|(c, v)| t.column(*c).is_some_and(|c| c.ty.admits(v))).cloned().collect();
            if sets.is_empty() {
                return None;
            }
            Statement::Update(Update { table: u.table, sets, filter: repair_filter(t, &u.filter)? })
        },
        Statement::Delete(d) => {
            let t = m.table(d.table)?;
            Statement::Delete(Delete { table: d.table, filter: repair_filter(t, &d.filter)? })
        },
        // The columns that are left, the expression only while its column
        // still leads, what resolves of the predicate, a fresh id.
        Statement::CreateIndex { table, def } => {
            let t = m.table(*table)?;
            let columns: Vec<_> = def.columns.iter().copied().filter(|k| t.column(k.column).is_some()).collect();
            if columns.is_empty() {
                return None;
            }
            let expr = def.expr.filter(|_| columns.first() == def.columns.first());
            let include = def.include.iter().copied().filter(|c| t.column(*c).is_some()).collect();
            let filter = def.filter.as_ref().and_then(|p| repair_pred(t, p));
            let id = if t.index(def.id).is_some() { t.next_index } else { def.id };
            let mut d = Index { id, columns, include, expr, filter, ..def.clone() };
            if d.unique && !semantics::index_ok(t, &d) {
                d.unique = false;
                d.nulls_not_distinct = false;
            }
            Statement::CreateIndex { table: *table, def: d }
        },
        Statement::CreateTable(def) if m.table(def.id).is_some() => Statement::CreateTable(Table { id: m.next_table, ..def.clone() }),
        Statement::AddColumn { table, column } => {
            let t = m.table(*table)?;
            Statement::AddColumn { table: *table, column: Column { id: t.next_column.max(column.id), not_null: column.not_null && (column.default.is_some() || t.rows.is_empty()), ..column.clone() } }
        },
        // The tables that are left.
        Statement::Analyze(tables) => Statement::Analyze(tables.iter().copied().filter(|&t| m.table(t).is_some()).collect()),
        Statement::Vacuum { tables, full, freeze, analyze } => {
            Statement::Vacuum { tables: tables.iter().copied().filter(|&t| m.table(t).is_some()).collect(), full: *full, freeze: *freeze, analyze: *analyze }
        },
        Statement::Reindex { table, index: Some(_), concurrently } => Statement::Reindex { table: *table, index: None, concurrently: *concurrently },
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
        Statement::SetNotNull { table, column, on: true } => Statement::SetNotNull { table: *table, column: *column, on: false },
        // The insert repaired, with its ON CONFLICT while still valid.
        Statement::Upsert(i, oc) => match repair(m, &Statement::Insert(i.clone()))? {
            Statement::Insert(fixed) => {
                let up = Statement::Upsert(fixed.clone(), oc.clone());
                if semantics::valid(m, &up) { up } else { Statement::Insert(fixed) }
            },
            other => other,
        },
        // The write repaired, with its RETURNING while still valid.
        Statement::Returning(inner, r) => {
            let fixed = repair(m, inner)?;
            let with = Statement::Returning(Box::new(fixed.clone()), r.clone());
            if semantics::valid(m, &with) { with } else { fixed }
        },
        _ => return None,
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

// ---------------------------------------------------------------- shrinking

fn simpler_lits(l: &Lit) -> Vec<Lit> {
    let mut out: Vec<Lit> = simpler_values(&l.value).into_iter().filter(|v| *v != Value::Null).map(|value| Lit { value, cast: l.cast }).collect();
    if l.cast.is_some() && !matches!(l.value, Value::Timestamp(_)) {
        out.push(Lit::plain(l.value.clone()));
    }
    out
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
            out.extend(simpler_lits(v).into_iter().map(|w| Pred::Cmp(*c, *op, w)));
        },
        Pred::Between(c, lo, hi) => {
            out.push(Pred::Cmp(*c, CmpOp::Ge, lo.clone()));
            out.push(Pred::Cmp(*c, CmpOp::Le, hi.clone()));
            out.extend(simpler_lits(lo).into_iter().map(|w| Pred::Between(*c, w, hi.clone())));
            out.extend(simpler_lits(hi).into_iter().map(|w| Pred::Between(*c, lo.clone(), w)));
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
            let bare = Select { table: s.table, columns: Vec::new(), filter: None, order: Vec::new(), limit: None };
            if *s != bare {
                out.push(Statement::Select(bare));
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
                if s.order[k].desc || s.order[k].nulls_first.is_some() {
                    out.push(with(&|c| c.order[k] = OrderTerm { column: c.order[k].column, desc: false, nulls_first: None }));
                }
            }
            for f in simpler_filters(&s.filter) {
                out.push(with(&|c| c.filter = f.clone()));
            }
        },
        Statement::Count { table, filter } => {
            for f in simpler_filters(filter) {
                out.push(Statement::Count { table: *table, filter: f });
            }
        },
        Statement::Insert(i) => {
            if i.rows.len() > 1 {
                for k in 0..i.rows.len() {
                    let mut c = i.clone();
                    c.rows.remove(k);
                    out.push(Statement::Insert(c));
                }
            }
            for k in 0..i.columns.len() {
                let mut c = i.clone();
                c.columns.remove(k);
                for r in &mut c.rows {
                    r.remove(k);
                }
                if c.columns.is_empty() {
                    c.rows.truncate(1);
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
            if u.sets.len() > 1 {
                for k in 0..u.sets.len() {
                    let mut c = u.clone();
                    c.sets.remove(k);
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
        Statement::Delete(d) => {
            for f in simpler_filters(&d.filter) {
                out.push(Statement::Delete(Delete { table: d.table, filter: f }));
            }
        },
        Statement::CreateIndex { table, def } => {
            for d in simpler_indexes(def) {
                out.push(Statement::CreateIndex { table: *table, def: d });
            }
        },
        Statement::Upsert(i, oc) => {
            out.push(Statement::Insert(i.clone()));
            if let Some((sets, filter)) = &oc.update {
                out.push(Statement::Upsert(i.clone(), OnConflict { target: oc.target.clone(), update: None }));
                if sets.len() > 1 {
                    for k in 0..sets.len() {
                        let mut c = sets.clone();
                        c.remove(k);
                        out.push(Statement::Upsert(i.clone(), OnConflict { target: oc.target.clone(), update: Some((c, filter.clone())) }));
                    }
                }
                for f in simpler_filters(filter) {
                    out.push(Statement::Upsert(i.clone(), OnConflict { target: oc.target.clone(), update: Some((sets.clone(), f)) }));
                }
            }
            for x in simpler_statements(&Statement::Insert(i.clone())) {
                if let Statement::Insert(x) = x {
                    out.push(Statement::Upsert(x, oc.clone()));
                }
            }
        },
        Statement::InsertSelect { table, columns, from } => {
            for f in simpler_filters(&from.filter) {
                out.push(Statement::InsertSelect { table: *table, columns: columns.clone(), from: FromTable { filter: f, ..from.clone() } });
            }
        },
        Statement::Merge(mg) => {
            if let Some(ft) = &mg.from {
                for f in simpler_filters(&ft.filter) {
                    out.push(Statement::Merge(Merge { from: Some(FromTable { filter: f, ..ft.clone() }), ..mg.clone() }));
                }
            }
            let mut push = |f: &dyn Fn(&mut Merge)| {
                let mut c = mg.clone();
                f(&mut c);
                if c != *mg {
                    out.push(Statement::Merge(c));
                }
            };
            for k in 0..mg.whens.len() {
                if mg.whens.len() > 1 {
                    push(&|c| {
                        c.whens.remove(k);
                    });
                }
                push(&|c| c.whens[k].cond = None);
                push(&|c| c.whens[k].action = MergeAction::Nothing);
            }
            if mg.rows.len() > 1 {
                for k in 0..mg.rows.len() {
                    push(&|c| {
                        c.rows.remove(k);
                    });
                }
            }
            for (r, row) in mg.rows.iter().enumerate() {
                for (k, v) in row.iter().enumerate() {
                    for w in simpler_values(v).into_iter().take(3) {
                        push(&|c| c.rows[r][k] = w.clone());
                    }
                }
            }
        },
        Statement::Returning(inner, r) => {
            out.push((**inner).clone());
            if r.items.len() > 1 {
                for k in 0..r.items.len() {
                    let mut c = r.clone();
                    c.items.remove(k);
                    out.push(Statement::Returning(inner.clone(), c));
                }
            }
            if r.cte {
                out.push(Statement::Returning(inner.clone(), Returning { cte: false, snapshot: false, ..r.clone() }));
            }
            for x in simpler_statements(inner) {
                out.push(Statement::Returning(Box::new(x), r.clone()));
            }
        },
        Statement::CreateTable(t) => {
            let mut push = |f: &dyn Fn(&mut Table)| {
                let mut c = t.clone();
                f(&mut c);
                if c != *t {
                    out.push(Statement::CreateTable(c));
                }
            };
            push(&|c| c.checks.clear());
            push(&|c| c.key.clear());
            push(&|c| c.partitioning = None);
            for k in 0..t.columns.len() {
                push(&|c| c.columns[k] = Column::new(c.columns[k].id, c.columns[k].ty));
                if t.columns.len() > 1 && !t.key.contains(&t.columns[k].id) {
                    push(&|c| {
                        let id = c.columns[k].id;
                        c.columns.remove(k);
                        c.checks.retain(|p| !p.reads(id));
                    });
                }
            }
        },
        Statement::AddColumn { table, column } => {
            let bare = Column::new(column.id, column.ty);
            if bare != *column {
                out.push(Statement::AddColumn { table: *table, column: bare });
            }
            if column.not_null {
                out.push(Statement::AddColumn { table: *table, column: Column { not_null: false, ..column.clone() } });
            }
        },
        Statement::SetDefault { table, column, value: Some(v) } => {
            out.push(Statement::SetDefault { table: *table, column: *column, value: None });
            for w in simpler_values(v) {
                out.push(Statement::SetDefault { table: *table, column: *column, value: Some(w) });
            }
        },
        Statement::Vacuum { tables, full, freeze, analyze } => {
            let t = tables.clone();
            for (on, c) in [
                (*full, Statement::Vacuum { tables: t.clone(), full: false, freeze: *freeze, analyze: *analyze }),
                (*freeze, Statement::Vacuum { tables: t.clone(), full: *full, freeze: false, analyze: *analyze }),
                (*analyze, Statement::Vacuum { tables: t.clone(), full: *full, freeze: *freeze, analyze: false }),
            ] {
                if on {
                    out.push(c);
                }
            }
            for k in 0..tables.len() {
                if tables.len() > 1 {
                    let mut fewer = tables.clone();
                    fewer.remove(k);
                    out.push(Statement::Vacuum { tables: fewer, full: *full, freeze: *freeze, analyze: *analyze });
                }
            }
        },
        Statement::Analyze(tables) if tables.len() > 1 => {
            for k in 0..tables.len() {
                let mut fewer = tables.clone();
                fewer.remove(k);
                out.push(Statement::Analyze(fewer));
            }
        },
        Statement::Reindex { table, index, concurrently } => {
            if *concurrently {
                out.push(Statement::Reindex { table: *table, index: *index, concurrently: false });
            }
            if index.is_some() {
                out.push(Statement::Reindex { table: *table, index: None, concurrently: *concurrently });
            }
        },
        Statement::Set(Setting::Enable(e, false)) => out.push(Statement::Set(Setting::Enable(*e, true))),
        Statement::AddForeignKey { table, fk, not_valid } => {
            let mut push = |f: &dyn Fn(&mut ForeignKey), nv: bool| {
                let mut c = fk.clone();
                f(&mut c);
                if c != *fk || nv != *not_valid {
                    out.push(Statement::AddForeignKey { table: *table, fk: c, not_valid: nv });
                }
            };
            push(&|_| {}, false);
            push(&|c| c.on_delete = Action::NoAction, *not_valid);
            push(&|c| c.on_update = Action::NoAction, *not_valid);
            push(&|c| c.delete_set.clear(), *not_valid);
            push(&|c| c.full = false, *not_valid);
            push(&|c| c.deferral = Deferral::No, *not_valid);
        },
        Statement::Stats(rows) if rows.len() > 1 => {
            for k in 0..rows.len() {
                let mut c = rows.clone();
                c.remove(k);
                out.push(Statement::Stats(c));
            }
        },
        _ => {},
    }
    out
}

impl Gen for Cases {
    type Value = Case;

    fn generate(&self, g: &mut Source) -> Case {
        let (setup, pool) = setup::draw_setup(g);
        let statements = draw_statements(g, setup.model.clone(), &pool);
        Case { setup, statements }
    }

    /// Every candidate is lower in [`measure`] (lexicographic on two
    /// natural numbers, so well founded), checked before it is offered:
    /// statement steps keep the setup and remove or simplify statements;
    /// setup steps are [`Setups`]' own (each lower in its cost) with the
    /// statements repaired against the smaller model. Every candidate's
    /// statements are settled ([`settle`]), so each is well formed.
    fn shrink(&self, value: &Case, visit: &mut dyn FnMut(Case) -> ControlFlow<()>) -> ControlFlow<()> {
        let m = value.setup.model.clone();
        let ms = models(m.clone(), &value.statements);
        let here = measure(value);
        let mut offer = |c: Case| if measure(&c) < here && c != *value { visit(c) } else { ControlFlow::Continue(()) };
        let n = value.statements.len();
        let same_setup = |statements: Vec<Statement>| Case { setup: value.setup.clone(), statements };
        // Fewer statements: none, either half, one less.
        if n > 0 {
            offer(same_setup(Vec::new()))?;
        }
        if n > 1 {
            offer(same_setup(value.statements[..n / 2].to_vec()))?;
            offer(same_setup(settle(m.clone(), value.statements[n / 2..].iter().cloned())))?;
        }
        for k in (0..n).rev() {
            let rest = settle(ms[k].clone(), value.statements[k + 1..].iter().cloned());
            offer(same_setup(value.statements[..k].iter().cloned().chain(rest).collect()))?;
        }
        // A smaller setup, the statements repaired against it.
        Setups.shrink(&value.setup, &mut |setup| {
            let statements = settle(setup.model.clone(), value.statements.iter().cloned());
            offer(Case { setup, statements })
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
                offer(same_setup(value.statements[..k].iter().cloned().chain(std::iter::once(s)).chain(rest).collect()))?;
            }
        }
        ControlFlow::Continue(())
    }

    fn mutate(&self, value: &Case, g: &mut Source) -> Case {
        let mut setup = value.setup.clone();
        let mut statements = value.statements.clone();
        let pool = Pool::draw(g);
        for _ in 0..g.int(1..=3) {
            let m = setup.model.clone();
            let n = statements.len();
            match g.int(0..10) {
                0 | 1 => setup = Setups.mutate(&setup, g),
                // A new statement somewhere.
                2..=4 if n < MAX_STATEMENTS => {
                    let k = g.int(0..=n);
                    let ms = models(m, &statements[..k]);
                    let s = draw_statement(g, ms.last().unwrap(), &pool);
                    statements.insert(k, s);
                },
                // A statement replaced by a new one.
                5..=7 if n > 0 => {
                    let k = g.index(n);
                    let ms = models(m, &statements[..k]);
                    statements[k] = draw_statement(g, ms.last().unwrap(), &pool);
                },
                8 if n > 0 => {
                    statements.remove(g.index(n));
                },
                // Two statements swapped.
                _ if n > 1 => {
                    let (a, b) = (g.index(n), g.index(n));
                    statements.swap(a, b);
                },
                _ => setup = Setups.mutate(&setup, g),
            }
            statements = settle(setup.model.clone(), statements);
        }
        Case { setup, statements }
    }
}

// ------------------------------------------------------------------ metrics

/// Numbers about one case, from the model and the statements' semantics
/// (no PostgreSQL):
///
/// - statements: how many; selects (with counts), inserts, updates,
///   deletes, ddl (CREATE, DROP, ALTER, TRUNCATE), maintenance (VACUUM,
///   ANALYZE, CLUSTER, REINDEX, CHECKPOINT, settings, statistics),
///   transaction (savepoints, rollbacks, commits): how many of each kind.
/// - predicate leaves: comparisons and NULL tests over all statements;
///   indexed predicate leaves: those on a column an index or the key leads
///   with; max predicate depth (0 without any).
/// - rows returned, empty selects, ordered selects, rows written.
/// - From the setup: tables, indexes, rows, keyed tables, identity tables,
///   ghost rows, tables with indexes created before their rows.
pub fn case_metrics(case: &Case, _size: usize) -> Vec<(&'static str, f64)> {
    let mut m = case.setup.model.clone();
    let layout = &case.setup.layout;
    let start = [
        ("tables", m.tables.len() as f64),
        ("indexes", m.tables.iter().map(|t| t.indexes.len()).sum::<usize>() as f64),
        ("rows", m.tables.iter().map(|t| t.rows.len()).sum::<usize>() as f64),
        ("keyed tables", m.tables.iter().filter(|t| !t.key.is_empty()).count() as f64),
        ("identity tables", m.tables.iter().filter(|t| t.identity().is_some()).count() as f64),
        ("ghost rows", layout.tables.iter().map(|l| l.ghosts).sum::<usize>() as f64),
        ("indexes first", layout.tables.iter().filter(|l| l.indexes_first).count() as f64),
    ];
    let mut kinds = [0usize; 7];
    let (mut leaves, mut indexed, mut depth, mut returned, mut empty, mut written, mut ordered) = (0, 0, 0, 0, 0, 0, 0);
    for s in &case.statements {
        let (filter, table) = match s {
            Statement::Select(x) => (&x.filter, Some(x.table)),
            Statement::Update(x) => (&x.filter, Some(x.table)),
            Statement::Delete(x) => (&x.filter, Some(x.table)),
            Statement::Count { table, filter } => (filter, Some(*table)),
            _ => (&None, None),
        };
        if let (Some(p), Some(t)) = (filter, table.and_then(|t| m.table(t))) {
            let mut cols = Vec::new();
            p.leaves(&mut cols);
            let lead = t.leading();
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
            Statement::Insert(_) | Statement::Upsert(..) | Statement::Merge(_) | Statement::InsertSelect { .. } => 1,
            Statement::JoinWrite(_) => 2,
            Statement::Update(_) => 2,
            Statement::Delete(_) => 3,
            Statement::Truncate(_)
            | Statement::CreateTable(_)
            | Statement::DropTable(_)
            | Statement::CreateIndex { .. }
            | Statement::DropIndex { .. }
            | Statement::AddColumn { .. }
            | Statement::DropColumn { .. }
            | Statement::SetNotNull { .. }
            | Statement::SetDefault { .. }
            | Statement::AlterType { .. } => 4,
            Statement::Savepoint(_) | Statement::RollbackTo(_) | Statement::Release(_) | Statement::Rollback => 6,
            _ => 5,
        }] += 1;
        if !semantics::valid(&m, s) {
            continue;
        }
        let e = semantics::apply(&mut m, s);
        written += e.written.unwrap_or(0);
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
        ("ddl", kinds[4] as f64),
        ("maintenance", kinds[5] as f64),
        ("transaction", kinds[6] as f64),
        ("predicate leaves", leaves as f64),
        ("indexed predicate leaves", indexed as f64),
        ("max predicate depth", depth as f64),
        ("rows returned", returned as f64),
        ("empty selects", empty as f64),
        ("ordered selects", ordered as f64),
        ("rows written", written as f64),
    ];
    out.extend(start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cases_are_well_formed_and_shrink_and_mutate_so() {
        for seed in 0..80u64 {
            let mut g = Source::new(seed, (seed % 40) as usize);
            let case = Cases.generate(&mut g);
            assert!(well_formed(&case), "seed {seed}: {case:?}");
            let here = measure(&case);
            let mut seen = 0;
            let shown = format!("{case:?}");
            let _ = Cases.shrink(&case, &mut |c| {
                assert!(well_formed(&c), "shrink of seed {seed}: {c:?}");
                assert!(format!("{c:?}") != shown, "seed {seed}: a candidate renders as its input\n{shown}");
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
}
