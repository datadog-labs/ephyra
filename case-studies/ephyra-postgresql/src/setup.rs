// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The generated database a case starts from ([`Setup`]): a [`Model`] and a
//! [`Layout`], the physical choices it is loaded with, none of which
//! changes a row. The analogue of `ephyra-sqlite`'s file generator, whose
//! layouts placed bytes on pages: PostgreSQL's pages are its own, so a
//! layout chooses what shapes them instead (fill factors, TOAST strategies,
//! dead tuples left by deleted rows, new row versions, whether indexes are
//! built over the rows or maintained as they arrive, COPY or INSERT, a
//! VACUUM, VACUUM FULL or CLUSTER after loading, extended statistics
//! before the ANALYZE).
//!
//! A setup is rendered as SQL ([`Setup::script`]) and loaded into a fresh
//! schema ([`load`]). The generator ([`setups`]) draws model and layout;
//! shrinking and mutation change them directly and [`normalize`] restores
//! what the rendering relies on, so every candidate is valid by the same
//! construction as a fresh value ([`setup_ok`]). No hook runs PostgreSQL.

use std::{fmt, ops::ControlFlow};

use ephyra::{Gen, Source};

use crate::{
    model::{
        Column, Compression, PartBound, Partitioning, Strategy, Index, IndexColumn, IndexParams, KeyExpr, Method, Model, Numeric, Row, Storage, Table, Type, Value, DATE_MAX,
        DATE_MIN, MAX_COLUMNS, MAX_INDEXES, MAX_ROWS, MAX_TABLES, MICROS_PER_DAY, TIMESTAMP_MAX, TIMESTAMP_MIN,
    },
    pg::Session,
    semantics::{index_ok, pred_valid, rows_ok},
    statements::{create_index, create_table, CmpOp, Lit, Pred},
};

/// A database to start from: what it holds and how it is loaded.
#[derive(Clone, PartialEq)]
pub struct Setup {
    pub model: Model,
    pub layout: Layout,
}

/// How each table of the model is loaded, by position.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layout {
    pub tables: Vec<TableLayout>,
}

/// How rows reach a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    /// `COPY t (..) FROM STDIN`, text format.
    Copy,
    /// `INSERT INTO t (..) VALUES ..`, this many rows per statement.
    Insert(u16),
}

/// What runs over a table once it is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Nothing,
    Vacuum,
    VacuumFreeze,
    VacuumFull,
    /// `CLUSTER t USING i`: a B-tree index that is not partial.
    Cluster(u32),
}

/// The physical choices of one table; [`TableLayout::SIMPLE`] is none.
#[derive(Clone, Debug, PartialEq)]
pub struct TableLayout {
    /// `fillfactor`, 10 to 100: free space kept on heap pages, which
    /// updates use for new versions on the same page (HOT).
    pub fillfactor: Option<u8>,
    /// `toast_tuple_target`, 128 to 8160: the row size above which values
    /// are compressed or moved out of line.
    pub toast_target: Option<u16>,
    pub unlogged: bool,
    pub load: Load,
    /// The indexes are created before the rows arrive (each row inserted
    /// into each), not built over them afterwards (sorted, bottom up).
    pub indexes_first: bool,
    /// Dead rows: the first `ghosts` rows are loaded and deleted before the
    /// real ones, leaving dead tuples on the pages (and in the indexes when
    /// they come first), or free space once vacuumed (`vacuum_ghosts`).
    pub ghosts: usize,
    pub vacuum_ghosts: bool,
    /// `UPDATE t SET c = c [WHERE p]` once loaded: a new version of each
    /// row it selects, the value unchanged.
    pub touch: Option<(u32, Option<Pred>)>,
    pub after: After,
    /// `CREATE STATISTICS t{t}_s [(kinds)] ON c, .. FROM t` before the
    /// ANALYZE: extended statistics, which change estimates and so plans,
    /// never rows.
    pub stats: Option<ExtStats>,
    pub analyze: bool,
    /// How the table's foreign key (if any) is made.
    pub fk_add: FkAdd,
}

/// How a setup makes a table's foreign key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FkAdd {
    /// `ALTER TABLE .. ADD CONSTRAINT` once every table is loaded (the
    /// whole table checked at once).
    After,
    /// In `CREATE TABLE`, so that every row loaded is checked as it comes
    /// (the parent is loaded before; not for a self-reference).
    Inline,
    /// `ADD CONSTRAINT .. NOT VALID` once loaded.
    NotValid,
    /// `NOT VALID`, then `VALIDATE CONSTRAINT`.
    Validate,
}

/// An extended statistics object: its kinds (`ndistinct`, `dependencies`,
/// `mcv`; none listed is all of them) and its columns, at least two and
/// distinct, of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtStats {
    pub kinds: [bool; 3],
    pub columns: Vec<u32>,
}

impl ExtStats {
    pub const KINDS: [&'static str; 3] = ["ndistinct", "dependencies", "mcv"];

    fn ok(&self, t: &Table) -> bool {
        self.columns.len() >= 2 && self.columns.iter().enumerate().all(|(k, c)| t.column(*c).is_some() && !self.columns[..k].contains(c))
    }

    fn sql(&self, table: u32) -> String {
        let kinds: Vec<&str> = Self::KINDS.iter().zip(self.kinds).filter(|(_, on)| *on).map(|(k, _)| *k).collect();
        let kinds = if kinds.is_empty() { String::new() } else { format!(" ({})", kinds.join(", ")) };
        let cols: Vec<String> = self.columns.iter().map(|c| format!("c{c}")).collect();
        format!("CREATE STATISTICS t{table}_s{kinds} ON {} FROM t{table}", cols.join(", "))
    }
}

impl TableLayout {
    pub const SIMPLE: TableLayout = TableLayout {
        fillfactor: None,
        toast_target: None,
        unlogged: false,
        load: Load::Copy,
        indexes_first: false,
        ghosts: 0,
        vacuum_ghosts: false,
        touch: None,
        after: After::Nothing,
        stats: None,
        analyze: false,
        fk_add: FkAdd::After,
    };

    pub fn cost(&self) -> u64 {
        u64::from(self.fillfactor.is_some())
            + u64::from(self.toast_target.is_some())
            + u64::from(self.unlogged)
            + match self.load {
                Load::Copy => 0,
                Load::Insert(n) => 1 + u64::from(n.min(64)) / 16,
            }
            + u64::from(self.indexes_first)
            + self.ghosts as u64
            + u64::from(self.vacuum_ghosts)
            + self.touch.as_ref().map_or(0, |(_, p)| 1 + p.as_ref().map_or(0, Pred::cost))
            + u64::from(self.after != After::Nothing)
            + self.stats.as_ref().map_or(0, |s| 1 + s.columns.len() as u64 + s.kinds.iter().filter(|k| **k).count() as u64)
            + u64::from(self.analyze)
            + u64::from(self.fk_add != FkAdd::After)
    }
}

impl Layout {
    pub fn cost(&self) -> u64 {
        self.tables.iter().map(TableLayout::cost).sum()
    }

    pub fn simple(tables: usize) -> Layout {
        Layout { tables: vec![TableLayout::SIMPLE; tables] }
    }
}

impl Setup {
    pub fn empty() -> Setup {
        Setup { model: Model::default(), layout: Layout::default() }
    }

    /// The simplicity measure every shrink step strictly decreases.
    pub fn cost(&self) -> u64 {
        self.model.cost() + self.layout.cost()
    }

    /// The SQL that loads it into an empty schema, in order.
    pub fn script(&self) -> Vec<Step> {
        let mut out = Vec::new();
        for (t, l) in self.model.tables.iter().zip(&self.layout.tables) {
            let mut with = Vec::new();
            if let Some(f) = l.fillfactor {
                with.push(format!("fillfactor = {f}"));
            }
            if let Some(k) = l.toast_target {
                with.push(format!("toast_tuple_target = {k}"));
            }
            let bare = Table { fk: if l.fk_add == FkAdd::Inline { t.fk.clone() } else { None }, ..t.clone() };
            out.push(Step::Sql(create_table(&bare, &with.join(", "), l.unlogged)));
            let indexes: Vec<Step> = t.indexes.iter().map(|i| Step::Sql(create_index(t.id, i))).collect();
            if l.indexes_first {
                out.extend(indexes.iter().cloned());
            }
            let all: Vec<usize> = (0..t.columns.len()).collect();
            if l.ghosts > 0 {
                // Every column given, so the sequence does not move.
                out.extend(load_rows(t, &all, &t.rows[..l.ghosts], l.load));
                out.push(Step::Sql(format!("DELETE FROM t{}", t.id)));
                if l.vacuum_ghosts {
                    out.push(Step::Sql(format!("VACUUM t{}", t.id)));
                }
            }
            // The identity column left to its sequence: 1, 2, .. in order.
            let given: Vec<usize> = all.iter().copied().filter(|&k| !t.columns[k].identity).collect();
            out.extend(load_rows(t, &given, &t.rows, l.load));
            if !l.indexes_first {
                out.extend(indexes);
            }
            if let Some((c, p)) = &l.touch {
                out.push(Step::Sql(format!("UPDATE t{} SET c{c} = c{c}{}", t.id, p.as_ref().map_or(String::new(), |p| format!(" WHERE {p}")))));
            }
            match l.after {
                After::Nothing => {},
                After::Vacuum => out.push(Step::Sql(format!("VACUUM t{}", t.id))),
                After::VacuumFreeze => out.push(Step::Sql(format!("VACUUM (FREEZE) t{}", t.id))),
                After::VacuumFull => out.push(Step::Sql(format!("VACUUM (FULL) t{}", t.id))),
                After::Cluster(i) => out.push(Step::Sql(format!("CLUSTER t{} USING t{}_i{i}", t.id, t.id))),
            }
            if let Some(st) = &l.stats {
                out.push(Step::Sql(st.sql(t.id)));
            }
            if l.analyze {
                out.push(Step::Sql(format!("ANALYZE t{}", t.id)));
            }
        }
        // Foreign keys not made inline, once every table is loaded.
        for (t, l) in self.model.tables.iter().zip(&self.layout.tables) {
            let Some(fk) = &t.fk else { continue };
            if l.fk_add == FkAdd::Inline {
                continue;
            }
            let nv = if l.fk_add == FkAdd::After { "" } else { " NOT VALID" };
            out.push(Step::Sql(format!("ALTER TABLE t{} ADD CONSTRAINT t{}_fk {}{nv}", t.id, t.id, fk.sql())));
            if l.fk_add == FkAdd::Validate {
                out.push(Step::Sql(format!("ALTER TABLE t{} VALIDATE CONSTRAINT t{}_fk", t.id, t.id)));
            }
        }
        // Triggers last: the rows are loaded without them.
        for t in &self.model.tables {
            for tr in &t.triggers {
                out.push(Step::Sql(crate::statements::create_trigger(t, tr)));
            }
        }
        out
    }
}

/// One step of a setup's script.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Sql(String),
    /// `COPY .. FROM STDIN` and its data, in text format.
    Copy(String, String),
}

/// A COPY text field: the value's input text with backslash, tab, newline
/// and carriage return escaped; `\N` for NULL.
fn copy_field(v: &Value) -> String {
    match v.text() {
        None => "\\N".into(),
        Some(s) => {
            let mut out = String::with_capacity(s.len());
            for ch in s.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '\t' => out.push_str("\\t"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    c => out.push(c),
                }
            }
            out
        },
    }
}

fn load_rows(t: &Table, cols: &[usize], rows: &[Row], load: Load) -> Vec<Step> {
    if rows.is_empty() {
        return Vec::new();
    }
    if cols.is_empty() {
        return rows.iter().map(|_| Step::Sql(format!("INSERT INTO t{} DEFAULT VALUES", t.id))).collect();
    }
    let names = cols.iter().map(|&k| format!("c{}", t.columns[k].id)).collect::<Vec<_>>().join(", ");
    match load {
        Load::Copy => {
            let mut data = String::new();
            for r in rows {
                data.push_str(&cols.iter().map(|&k| copy_field(&r[k])).collect::<Vec<_>>().join("\t"));
                data.push('\n');
            }
            vec![Step::Copy(format!("COPY t{} ({names}) FROM STDIN", t.id), data)]
        },
        Load::Insert(batch) => rows
            .chunks(usize::from(batch.max(1)))
            .map(|chunk| {
                let values =
                    chunk.iter().map(|r| format!("({})", cols.iter().map(|&k| crate::statements::literal(&r[k])).collect::<Vec<_>>().join(", "))).collect::<Vec<_>>().join(", ");
                Step::Sql(format!("INSERT INTO t{} ({names}) VALUES {values}", t.id))
            })
            .collect(),
    }
}

/// Load a setup into the session's fresh schema.
pub fn load(s: &mut Session, setup: &Setup) -> Result<(), String> {
    for step in setup.script() {
        match &step {
            Step::Sql(sql) => s.execute(sql).map(|_| ()),
            Step::Copy(sql, data) => s.copy_in(sql, data).map(|_| ()),
        }
        .map_err(|e| format!("setup: {}: {e}", match &step {
            Step::Sql(sql) | Step::Copy(sql, _) => sql,
        }))?;
    }
    Ok(())
}

/// FNV-1a over the bytes: two setups print alike only when they are alike.
fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3))
}

/// The model's summary with a digest of the whole script, then the script
/// (long values and COPY data cut short). The digest keeps setups that
/// differ only where the printout is cut apart, which `ephyra check` relies
/// on when it compares values by their rendering.
impl fmt::Debug for Setup {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let rows: usize = self.model.tables.iter().map(|t| t.rows.len()).sum();
        let script = self.script();
        let all: String = script.iter().map(|s| format!("{s:?}")).collect();
        writeln!(f, "Setup {{ {} tables, {rows} rows, digest {:016x}", self.model.tables.len(), digest(all.as_bytes()))?;
        for step in script {
            match step {
                Step::Sql(sql) => writeln!(f, "  {};", cut(&sql, 400))?,
                Step::Copy(sql, data) => {
                    writeln!(f, "  {sql};")?;
                    for line in data.lines().take(40) {
                        writeln!(f, "  {}", cut(line, 200))?;
                    }
                    let n = data.lines().count();
                    if n > 40 {
                        writeln!(f, "  -- .. {} more", n - 40)?;
                    }
                    writeln!(f, "  \\.")?;
                },
            }
        }
        write!(f, "}}")
    }
}

fn cut(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut k = n;
    while !s.is_char_boundary(k) {
        k -= 1;
    }
    format!("{}.. ({} bytes)", &s[..k], s.len())
}

// ------------------------------------------------------------ validity

/// Whether the identity column of `t` holds 1, 2, .. in row order (what
/// loading it through its sequence gives).
fn identity_ok(t: &Table) -> bool {
    match t.identity().and_then(|c| t.pos(c.id)) {
        None => true,
        Some(p) => t.rows.iter().enumerate().all(|(k, r)| r[p] == Value::Int(k as i64 + 1)),
    }
}

/// Whether a table of a setup is well formed: as a new table would be
/// ([`crate::semantics::new_table_ok`], but with rows and indexes), its
/// rows keep its constraints, its indexes are valid.
pub fn table_ok(t: &Table) -> bool {
    let bare = Table { rows: Vec::new(), indexes: Vec::new(), next_index: 0, widest: Default::default(), infinite: Default::default(), unlogged: false, ..t.clone() };
    let mut widths = bare.clone();
    widths.record(&t.rows);
    let ids: Vec<u32> = t.indexes.iter().map(|i| i.id).collect();
    crate::semantics::new_table_ok(&bare)
        && t.rows.len() <= MAX_ROWS
        && t.widest == widths.widest
        && t.infinite == widths.infinite
        && rows_ok(t, &t.rows)
        && identity_ok(t)
        && t.indexes.len() <= MAX_INDEXES
        && ids.iter().enumerate().all(|(k, i)| !ids[..k].contains(i) && *i < t.next_index)
        && t.indexes.iter().all(|i| index_ok(t, i))
}

fn layout_ok(t: &Table, l: &TableLayout) -> bool {
    // No storage parameters and no UNLOGGED for a partitioned table.
    (t.partitioning.is_none() || l.fillfactor.is_none() && l.toast_target.is_none() && !l.unlogged)
        && l.fillfactor.is_none_or(|f| (10..=100).contains(&f))
        && l.toast_target.is_none_or(|k| (128..=8160).contains(&k))
        && match l.load {
            Load::Copy => true,
            Load::Insert(n) => n >= 1 && usize::from(n) <= t.rows.len() && !t.columns.iter().all(|c| c.identity),
        }
        && (l.indexes_first <= (!t.indexes.is_empty() && !t.rows.is_empty()))
        && l.ghosts <= t.rows.len()
        && (l.ghosts > 0 || !l.vacuum_ghosts)
        && l.touch.as_ref().is_none_or(|(c, p)| t.column(*c).is_some() && p.as_ref().is_none_or(|p| pred_valid(t, p)))
        && l.stats.as_ref().is_none_or(|s| s.ok(t))
        && match l.after {
            After::Cluster(i) => t.index(i).is_some_and(|i| i.method == Method::Btree && i.filter.is_none()),
            _ => true,
        }
}

/// A setup is well formed when every table is, the ids are distinct and
/// below the model's counter, each identity sequence is at the value after
/// its rows', no savepoint is open, and the layout matches the tables.
#[ephyra::generator]
pub fn setup_ok(s: &Setup) -> bool {
    let m = &s.model;
    let ids: Vec<u32> = m.tables.iter().map(|t| t.id).collect();
    m.tables.len() <= MAX_TABLES
        && m.savepoints.is_empty()
        && ids.iter().enumerate().all(|(k, i)| !ids[..k].contains(i) && *i < m.next_table)
        && m.tables.iter().all(table_ok)
        && m.sequences.iter().all(|(t, v)| m.table(*t).is_some_and(|t| t.identity().is_some() && *v == t.rows.len() as i64 + 1))
        && m.tables.iter().all(|t| t.identity().is_none() || m.sequences.contains_key(&t.id))
        && s.layout.tables.len() == m.tables.len()
        && m.tables.iter().zip(&s.layout.tables).all(|(t, l)| layout_ok(t, l))
        && m.tables.iter().zip(&s.layout.tables).enumerate().all(|(k, (t, l))| {
            t.unlogged == l.unlogged
                && match &t.fk {
                    None => l.fk_add == FkAdd::After,
                    Some(f) => fk_ok(m, t, f) && (l.fk_add != FkAdd::Inline || inline_ok(m, k, f)),
                }
                && t.triggers.iter().enumerate().all(|(j, tr)| tr.id < t.next_trigger && t.triggers[..j].iter().all(|x| x.id != tr.id) && crate::fk::trigger_ok(&m.tables, t, tr))
        })
}

/// Whether a setup table's foreign key is valid among the setup's tables
/// and its rows keep it.
fn fk_ok(m: &Model, t: &Table, f: &crate::model::ForeignKey) -> bool {
    crate::fk::def_ok(&m.tables, t, f) && crate::fk::holds(&m.tables, t)
}

/// Whether the `k`th table's foreign key may be made in its CREATE TABLE:
/// its parent comes before it.
fn inline_ok(m: &Model, k: usize, f: &crate::model::ForeignKey) -> bool {
    m.tables[..k].iter().any(|p| p.id == f.parent)
}

/// Restore what the rendering relies on after a change: identity columns
/// numbered 1, 2, .. and their sequences after them, the layout's length,
/// its references to what is gone dropped.
pub fn normalize(s: &mut Setup) {
    let m = &mut s.model;
    m.sequences.clear();
    for t in &mut m.tables {
        t.widest.clear();
        t.infinite.clear();
        let rows = t.rows.clone();
        t.record(&rows);
        let bare = Table { indexes: Vec::new(), ..t.clone() };
        let indexes = std::mem::take(&mut t.indexes);
        t.indexes = indexes.into_iter().filter(|i| index_ok(&bare, i)).collect();
        if let Some(p) = t.identity().and_then(|c| t.pos(c.id)) {
            for (k, r) in t.rows.iter_mut().enumerate() {
                r[p] = Value::Int(k as i64 + 1);
            }
            m.sequences.insert(t.id, t.rows.len() as i64 + 1);
        }
        m.next_table = m.next_table.max(t.id + 1);
    }
    s.layout.tables.resize(m.tables.len(), TableLayout::SIMPLE);
    // Persistence as the layout says, then the foreign keys that still
    // hold (each change above may break one: it goes).
    for (t, l) in m.tables.iter_mut().zip(&s.layout.tables) {
        t.unlogged = l.unlogged && t.partitioning.is_none();
    }
    for k in 0..m.tables.len() {
        if let Some(f) = m.tables[k].fk.clone() {
            if !fk_ok(m, &m.tables[k], &f) {
                m.tables[k].fk = None;
            }
        }
    }
    // Triggers still valid (fewer triggers only relax the others').
    let keep: Vec<Vec<bool>> = m.tables.iter().map(|t| t.triggers.iter().enumerate().map(|(j, tr)| t.triggers[..j].iter().all(|x| x.id != tr.id) && crate::fk::trigger_ok(&m.tables, t, tr)).collect()).collect();
    for (t, keep) in m.tables.iter_mut().zip(keep) {
        let mut it = keep.into_iter();
        t.triggers.retain(|_| it.next().unwrap());
        t.next_trigger = t.triggers.iter().map(|x| x.id + 1).max().unwrap_or(0).max(t.next_trigger);
    }
    for k in 0..m.tables.len() {
        let l = &mut s.layout.tables[k];
        match &m.tables[k].fk {
            None => l.fk_add = FkAdd::After,
            Some(f) if l.fk_add == FkAdd::Inline && !inline_ok(m, k, f) => l.fk_add = FkAdd::After,
            _ => {},
        }
    }
    for (t, l) in m.tables.iter().zip(&mut s.layout.tables) {
        l.ghosts = l.ghosts.min(t.rows.len());
        l.vacuum_ghosts &= l.ghosts > 0;
        // Choices the script would not show take their simplest form.
        l.load = match l.load {
            _ if t.rows.is_empty() || t.columns.iter().all(|c| c.identity) => Load::Copy,
            Load::Insert(n) => Load::Insert(n.clamp(1, u16::try_from(t.rows.len()).unwrap_or(u16::MAX))),
            Load::Copy => Load::Copy,
        };
        l.indexes_first &= !t.indexes.is_empty() && !t.rows.is_empty();
        if t.partitioning.is_some() {
            l.fillfactor = None;
            l.toast_target = None;
            l.unlogged = false;
        }
        if l.touch.as_ref().is_some_and(|(c, p)| t.column(*c).is_none() || p.as_ref().is_some_and(|p| !pred_valid(t, p))) {
            l.touch = None;
        }
        if let Some(st) = &mut l.stats {
            st.columns.retain(|c| t.column(*c).is_some());
            if !st.ok(t) {
                l.stats = None;
            }
        }
        if let After::Cluster(i) = l.after {
            if !t.index(i).is_some_and(|i| i.method == Method::Btree && i.filter.is_none()) {
                l.after = After::Nothing;
            }
        }
    }
}

// ------------------------------------------------------------- drawing

/// Setups: up to [`MAX_TABLES`] tables of drawn types, rows, keys, CHECK
/// constraints and indexes, each with a layout.
#[ephyra::generator(setup_ok)]
pub fn setups() -> impl Gen<Value = Setup> {
    Setups
}

pub struct Setups;

/// Characters text is drawn from: ASCII letters of both cases (the C
/// collation orders `B` before `a`), digits, quotes and a backslash (which
/// literals and COPY escape), LIKE's wildcards, and characters of two,
/// three and four bytes.
pub const ALPHABET: [char; 18] = ['a', 'b', 'c', 'A', 'B', 'z', '0', '1', ' ', '\'', '\\', '%', '_', '\t', '\u{e9}', '\u{df}', '\u{20ac}', '\u{1f600}'];

/// A small xorshift generator, for long values expanded from one drawn
/// seed (so that they cost one decision, not one per character).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Values of every type that tables of one setup share, so that columns
/// hold equal values (joins match, groups gather) and literals hit.
pub struct Pool(Vec<(Type, Vec<Value>)>);

impl Pool {
    pub fn draw(g: &mut Source) -> Pool {
        Pool(Type::ALL.iter().map(|&ty| (ty, (0..g.int(2..=8)).map(|_| fresh(g, ty)).collect())).collect())
    }

    pub fn of(&self, ty: Type) -> &[Value] {
        &self.0.iter().find(|(t, _)| *t == ty).unwrap().1
    }
}

/// A value of `ty`: from the pool half the time (when given), else fresh.
pub fn draw_value(g: &mut Source, ty: Type, pool: Option<&Pool>) -> Value {
    match pool {
        Some(p) if g.bool_with(0.5) => g.pick(p.of(ty)).clone(),
        _ => fresh(g, ty),
    }
}

/// Integers: small ones mostly, then wider, then the type's edges.
fn draw_int(g: &mut Source, ty: Type) -> i64 {
    let (lo, hi) = match ty {
        Type::Int2 => (i64::from(i16::MIN), i64::from(i16::MAX)),
        Type::Int4 => (i64::from(i32::MIN), i64::from(i32::MAX)),
        _ => (i64::MIN, i64::MAX),
    };
    match g.int(0..20) {
        0..=10 => g.int(-3i64..=3),
        11..=15 => g.int(-1000i64..=1000),
        16..=17 => g.int(lo / 2..=hi / 2),
        _ => *g.pick(&[lo, hi, lo + 1, hi - 1]),
    }
}

/// Reals: small integers, eighths, wide magnitudes, and the specials
/// (`-0`, the infinities, NaN, the extremes).
fn draw_float(g: &mut Source, single: bool) -> f64 {
    let x = match g.int(0..20) {
        0..=7 => f64::from(g.int(-5i32..=5)),
        8..=12 => f64::from(g.int(-999i32..=999)) / 8.0,
        13..=15 => f64::from(g.int(-99_999i32..=99_999)) * 10f64.powi(g.int(-40i32..=40)),
        _ => *g.pick(&[-0.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 5e-324, f64::MAX, -f64::MAX, f64::MIN_POSITIVE, 0.1]),
    };
    if single {
        f64::from(x as f32)
    } else {
        x
    }
}

fn draw_numeric(g: &mut Source) -> Numeric {
    match g.int(0..30) {
        0 => g.pick(&[Numeric::NaN, Numeric::Inf, Numeric::NegInf]).clone(),
        1..=12 => Numeric::int(g.int(-5i64..=5)),
        13..=24 => {
            // A decimal of up to 4 places, now and then with trailing zeros.
            let scale = g.int(0u16..=4);
            let digits = g.int(0u64..=99_999).to_string();
            let mut digits = digits.into_bytes();
            if g.bool_with(0.3) {
                digits.extend(std::iter::repeat_n(b'0', g.int(1..=3)));
            }
            let extra = digits.len().saturating_sub(5) as u16;
            Numeric::finite(g.bool(), digits, scale + extra)
        },
        _ => {
            // Long ones: up to 40 digits on either side of the point.
            let mut rng = Rng::new(g.int(..));
            let n = g.int(1..=40);
            let digits: Vec<u8> = (0..n).map(|_| b'0' + rng.below(10) as u8).collect();
            Numeric::finite(g.bool(), digits, g.int(0u16..=40))
        },
    }
}

/// Text: short strings of [`ALPHABET`] mostly, now and then long ones
/// (repetitive, which TOAST compresses, or not).
fn draw_text(g: &mut Source) -> String {
    match g.int(0..20) {
        0..=16 => (0..g.int(0..=6)).map(|_| *g.pick(&ALPHABET)).collect(),
        17 | 18 => {
            let n = g.int(7..=300);
            let mut rng = Rng::new(g.int(..));
            (0..n).map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize]).collect()
        },
        _ => {
            let n = g.int(300..=2400);
            let unit: String = (0..g.int(1..=8)).map(|_| *g.pick(&ALPHABET)).collect();
            if g.bool() {
                unit.repeat(n / unit.len().max(1) + 1).chars().take(n).collect()
            } else {
                let mut rng = Rng::new(g.int(..));
                (0..n).map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize]).collect()
            }
        },
    }
}

fn draw_bytes(g: &mut Source) -> Vec<u8> {
    if g.bool_with(0.9) {
        (0..g.int(0..=6)).map(|_| g.int(0u8..=255)).collect()
    } else {
        let n = g.int(7..=2400);
        let mut rng = Rng::new(g.int(..));
        let period = g.int(1u64..=256);
        (0..n).map(|k| if period < 64 { (k as u64 % period) as u8 } else { rng.below(256) as u8 }).collect()
    }
}

/// Dates: a few days of 2024 (so that tables share them), around 2000,
/// before year 1, and the extremes and infinities.
fn draw_date(g: &mut Source) -> i32 {
    match g.int(0..20) {
        0..=9 => 8780 + g.int(0..=40),
        10..=14 => g.int(-400i32..=400),
        15..=16 => g.int(-800_000i32..=-730_000),
        17 => g.int(DATE_MIN..=DATE_MAX),
        _ => *g.pick(&[i32::MIN, i32::MAX, DATE_MIN, DATE_MAX]),
    }
}

fn draw_timestamp(g: &mut Source) -> i64 {
    match g.int(0..20) {
        0..=15 => {
            let day = i64::from(draw_date(g).clamp(DATE_MIN, 106_000_000));
            let time = match g.int(0..4) {
                0 => 0,
                1 => 12 * 3_600_000_000,
                2 => g.int(0..MICROS_PER_DAY / 1_000_000) * 1_000_000,
                _ => g.int(0..MICROS_PER_DAY),
            };
            (day * MICROS_PER_DAY + time).clamp(TIMESTAMP_MIN, TIMESTAMP_MAX)
        },
        _ => *g.pick(&[i64::MIN, i64::MAX, TIMESTAMP_MIN, TIMESTAMP_MAX]),
    }
}

/// A fresh value of `ty` (never NULL).
pub fn fresh(g: &mut Source, ty: Type) -> Value {
    match ty {
        Type::Int2 | Type::Int4 | Type::Int8 => Value::Int(draw_int(g, ty)),
        Type::Float4 => Value::Float(draw_float(g, true)),
        Type::Float8 => Value::Float(draw_float(g, false)),
        Type::Numeric => Value::Numeric(draw_numeric(g)),
        Type::Text => Value::Text(draw_text(g)),
        Type::Bool => Value::Bool(g.bool()),
        Type::Bytea => Value::Bytea(draw_bytes(g)),
        Type::Date => Value::Date(draw_date(g)),
        Type::Timestamp => Value::Timestamp(draw_timestamp(g)),
        t => unreachable!("{t:?} is not a column type"),
    }
}

/// A column type: integers and text most often.
pub fn draw_type(g: &mut Source) -> Type {
    *g.weighted(&[
        (5, Type::Int4),
        (2, Type::Int2),
        (3, Type::Int8),
        (1, Type::Float4),
        (2, Type::Float8),
        (2, Type::Numeric),
        (4, Type::Text),
        (1, Type::Bool),
        (1, Type::Bytea),
        (2, Type::Date),
        (1, Type::Timestamp),
    ])
}

/// A literal for column `c` of `t`: mostly a value in the column, so that
/// comparisons select some rows; sometimes another; now and then of
/// another comparable type, cast.
pub fn draw_literal(g: &mut Source, t: &Table, c: u32, pool: Option<&Pool>) -> Lit {
    let col = t.column(c).unwrap();
    let p = t.pos(c).unwrap();
    let present = (!t.rows.is_empty()).then(|| t.rows[g.index(t.rows.len())][p].clone()).filter(|v| *v != Value::Null);
    let value = match present {
        Some(v) if g.bool_with(0.6) => v,
        _ => draw_value(g, col.ty, pool),
    };
    // Cross-type: a wider integer, a double for a real, a timestamp for a
    // date (at midnight).
    let cast = match (col.ty, &value) {
        (Type::Int2 | Type::Int4, Value::Int(_)) if g.bool_with(0.15) => Some(Type::Int8),
        (Type::Float4, Value::Float(_)) if g.bool_with(0.15) => Some(Type::Float8),
        (Type::Date, Value::Date(d)) if g.bool_with(0.15) && crate::model::date_as_timestamp(*d).is_some() => {
            return Lit { value: Value::Timestamp(crate::model::date_as_timestamp(*d).unwrap()), cast: Some(Type::Timestamp) };
        },
        _ => None,
    };
    Lit { value, cast }
}

/// A predicate of `t`, `depth` levels deep at most.
pub fn draw_pred(g: &mut Source, t: &Table, depth: usize, pool: Option<&Pool>) -> Pred {
    if depth > 1 && g.bool_with(0.35) {
        let a = Box::new(draw_pred(g, t, depth - 1, pool));
        return match g.int(0..5) {
            0 | 1 => Pred::And(a, Box::new(draw_pred(g, t, depth - 1, pool))),
            2 | 3 => Pred::Or(a, Box::new(draw_pred(g, t, depth - 1, pool))),
            _ => Pred::Not(a),
        };
    }
    let leading = t.leading();
    let c = if !leading.is_empty() && g.bool_with(0.5) { *g.pick(&leading) } else { t.columns[g.index(t.columns.len())].id };
    match g.int(0..20) {
        0..=13 => Pred::Cmp(c, *g.pick(&CmpOp::ALL), draw_literal(g, t, c, pool)),
        14..=16 => {
            let (mut lo, mut hi) = (draw_literal(g, t, c, pool), draw_literal(g, t, c, pool));
            if g.bool_with(0.8) && crate::model::compare(&lo.value, &hi.value).is_gt() {
                std::mem::swap(&mut lo, &mut hi);
            }
            Pred::Between(c, lo, hi)
        },
        _ => Pred::IsNull(c, g.bool()),
    }
}

/// A row count at the current size: often small, sometimes up to the cap.
fn draw_row_count(g: &mut Source) -> usize {
    let most = (4 + 20 * g.size()).min(MAX_ROWS);
    match g.int(0..10) {
        0 => 0,
        1..=5 => g.int(0..=most.min(12)),
        6..=8 => g.int(0..=most.min(200)),
        _ => g.int(0..=most),
    }
}

/// Column definitions for a new table (no rows yet): types, NOT NULL,
/// defaults, at most one identity column, TOAST strategies.
pub fn draw_columns(g: &mut Source, n: usize, pool: Option<&Pool>) -> Vec<Column> {
    let mut identity = false;
    (0..n)
        .map(|k| {
            let ty = draw_type(g);
            let mut c = Column::new(k as u32, ty);
            if ty.is_int() && !identity && g.bool_with(0.12) {
                c.identity = true;
                identity = true;
                return c;
            }
            c.not_null = g.bool_with(0.15);
            if g.bool_with(0.1) {
                c.default = Some(draw_value(g, ty, pool));
            }
            if ty.varlena() && g.bool_with(0.25) {
                c.storage = Some(*g.pick(&Storage::ALL));
            }
            if ty.varlena() && g.bool_with(0.15) {
                c.compression = Some(*g.pick(&[Compression::Pglz, Compression::Lz4]));
            }
            c
        })
        .collect()
}

/// An index of `t` with id `id` (not checked against the rows): a B-tree
/// mostly, else hash or BRIN; 1 to 3 key columns, now and then descending,
/// with NULLs placed, INCLUDE columns, an expression, a predicate, storage
/// parameters; UNIQUE now and then.
pub fn draw_index(g: &mut Source, t: &Table, id: u32, pool: Option<&Pool>) -> Index {
    let mut method = *g.weighted(&[(14, Method::Btree), (3, Method::Hash), (3, Method::Brin), (3, Method::Gin), (3, Method::Gist), (2, Method::SpGist)]);
    // SP-GiST indexes text: a text column, or md5() of bytes.
    let texts: Vec<u32> = t.columns.iter().filter(|c| matches!(c.ty, Type::Text | Type::Bytea)).map(|c| c.id).collect();
    if method == Method::SpGist && texts.is_empty() {
        method = Method::Btree;
    }
    let mut cols: Vec<u32> = t.columns.iter().map(|c| c.id).collect();
    if method == Method::SpGist {
        let c = *g.pick(&texts);
        cols.retain(|&x| x != c);
        cols.insert(0, c);
    }
    let n = match method {
        Method::Hash | Method::SpGist => 1,
        _ => g.int(1..=cols.len().min(3)),
    };
    let mut keys = Vec::with_capacity(n);
    for k in 0..n {
        let c = cols.remove(if method == Method::SpGist && k == 0 { 0 } else { g.index(cols.len()) });
        let mut k = IndexColumn::asc(c);
        if method == Method::Btree {
            k.desc = g.bool_with(0.3);
            if g.bool_with(0.15) {
                k.nulls_first = Some(g.bool());
            }
        }
        keys.push(k);
    }
    let mut i = Index { id, method, columns: keys, include: Vec::new(), unique: false, nulls_not_distinct: false, expr: None, filter: None, params: IndexParams::default() };
    if matches!(method, Method::Btree | Method::Gist | Method::SpGist) && !cols.is_empty() && g.bool_with(0.15) {
        i.include.push(cols.remove(g.index(cols.len())));
    }
    let first = t.column(i.columns[0].column).unwrap().ty;
    if method == Method::SpGist && first == Type::Bytea {
        i.expr = Some(KeyExpr::Md5);
    } else if method == Method::SpGist {
        i.expr = g.bool_with(0.2).then(|| *g.pick(&[KeyExpr::Lower, KeyExpr::Md5]));
    } else if g.bool_with(0.2) {
        let fits: Vec<KeyExpr> = KeyExpr::ALL.into_iter().filter(|e| e.applies(first)).collect();
        if !fits.is_empty() {
            i.expr = Some(*g.pick(&fits));
        }
    }
    if g.bool_with(0.2) {
        i.filter = Some(if g.bool_with(0.4) { Pred::IsNull(t.columns[g.index(t.columns.len())].id, true) } else { draw_pred(g, t, 2, pool) });
    }
    if method == Method::Btree && i.expr.is_none() && i.filter.is_none() && g.bool_with(0.25) {
        i.unique = true;
        i.nulls_not_distinct = g.bool_with(0.2);
    }
    match method {
        Method::Btree => {
            if g.bool_with(0.2) {
                i.params.fillfactor = Some(*g.pick(&[10, 30, 50, 70, 90, 100]));
            }
            i.params.no_dedup = g.bool_with(0.1);
        },
        Method::Hash | Method::Gist | Method::SpGist => {
            if g.bool_with(0.2) {
                i.params.fillfactor = Some(*g.pick(&[10, 50, 75, 100]));
            }
        },
        Method::Gin => {},
        Method::Brin => {
            if g.bool_with(0.5) {
                i.params.pages_per_range = Some(*g.pick(&[1, 2, 4, 16, 128]));
            }
        },
    }
    i
}

/// A partitioning of `t` now and then (on a column that is not the identity
/// column): RANGE on split points among values the pool holds (gaps now and
/// then), LIST of groups of such values (NULL in one now and then), HASH of
/// every remainder of a modulus; a DEFAULT partition often. Values drawn from
/// the pool, so that rows land in several partitions.
pub fn draw_partitioning(g: &mut Source, t: &Table, pool: Option<&Pool>) -> Option<Partitioning> {
    let cols: Vec<&Column> = t.columns.iter().filter(|c| !c.identity).collect();
    if cols.is_empty() {
        return None;
    }
    let col = cols[g.index(cols.len())];
    let strategy = *g.weighted(&[(4, Strategy::Range), (3, Strategy::List), (3, Strategy::Hash)]);
    let mut vals: Vec<Value> = Vec::new();
    for _ in 0..8 {
        let v = draw_value(g, col.ty, pool);
        if v != Value::Null && v.width() <= 200 && !matches!(v, Value::Float(x) if x.is_nan()) && !vals.iter().any(|w| crate::model::compare(w, &v).is_eq()) {
            vals.push(v);
        }
    }
    vals.sort_by(crate::model::compare);
    let mut parts = Vec::new();
    match strategy {
        Strategy::Range => {
            let k = g.int(1..=vals.len().clamp(1, 3)).min(vals.len());
            let mut cuts: Vec<Value> = Vec::new();
            for (i, v) in vals.iter().enumerate() {
                if cuts.len() < k && (g.bool() || vals.len() - i <= k - cuts.len()) {
                    cuts.push(v.clone());
                }
            }
            let mut edges: Vec<Option<Value>> = vec![None];
            edges.extend(cuts.into_iter().map(Some));
            edges.push(None);
            let skip = (edges.len() > 2 && g.bool_with(0.2)).then(|| g.index(edges.len() - 1));
            for w in 0..edges.len() - 1 {
                if Some(w) != skip {
                    parts.push(PartBound::Range(edges[w].clone(), edges[w + 1].clone()));
                }
            }
        },
        Strategy::List => {
            if vals.is_empty() {
                return None;
            }
            let groups = g.int(1..=vals.len().min(3));
            let mut lists: Vec<Vec<Value>> = vec![Vec::new(); groups];
            for (k, v) in vals.into_iter().enumerate() {
                let at = if k < groups { k } else { g.index(groups) };
                lists[at].push(v);
            }
            if g.bool_with(0.3) {
                let at = g.index(groups);
                lists[at].push(Value::Null);
            }
            parts.extend(lists.into_iter().map(PartBound::List));
        },
        Strategy::Hash => {
            let m = *g.pick(&[1u32, 2, 3, 4, 8]);
            parts.extend((0..m).map(|r| PartBound::Hash(m, r)));
        },
    }
    if strategy != Strategy::Hash && g.bool_with(if strategy == Strategy::List { 0.5 } else { 0.4 }) {
        parts.push(PartBound::Default);
    }
    let p = Partitioning { strategy, column: col.id, parts };
    crate::model::partitioning_ok(&p, col.ty).then_some(p)
}

/// A table with id `id`: columns, a key now and then, rows that keep the
/// constraints (rows that would break one are left out), a CHECK
/// constraint now and then (true of every row), indexes the rows allow.
fn draw_table(g: &mut Source, id: u32, pool: &Pool, earlier: &[Table]) -> Table {
    let n = g.int(1..=MAX_COLUMNS.min(2 + g.size() / 10));
    let mut t = Table::new(id, &[]);
    t.columns = draw_columns(g, n, Some(pool));
    t.next_column = n as u32;
    if g.bool_with(0.35) {
        let mut cols: Vec<u32> = t.columns.iter().filter(|c| !c.identity || g.bool()).map(|c| c.id).collect();
        let k = g.int(1..=cols.len().clamp(1, 2));
        for _ in 0..k.min(cols.len()) {
            t.key.push(cols.remove(g.index(cols.len())));
        }
    }
    if g.bool_with(0.2) {
        t.partitioning = draw_partitioning(g, &t, Some(pool));
        // A key includes the partition key.
        if let Some(p) = &t.partitioning {
            if !t.key.is_empty() && !t.key.contains(&p.column) {
                t.key.push(p.column);
            }
        }
    }
    // A foreign key to an earlier table now and then (columns of the
    // referenced key's types added when the table has none), or to its
    // own key; each row then references a parent row mostly.
    let fk = draw_setup_fk(g, &mut t, earlier);
    let identity = t.identity().map(|c| c.id);
    for _ in 0..draw_row_count(g) {
        let mut row: Row = t
            .columns
            .iter()
            .map(|c| if (c.not_null || t.key.contains(&c.id)) || !g.bool_with(0.1) { draw_value(g, c.ty, Some(pool)) } else { Value::Null })
            .collect();
        if let Some(p) = identity.and_then(|c| t.pos(c)) {
            row[p] = Value::Int(t.rows.len() as i64 + 1);
        }
        if let Some(f) = &fk {
            let parent = if f.parent == t.id { Some(&t) } else { earlier.iter().find(|p| p.id == f.parent) };
            if let Some(parent) = parent.filter(|p| !p.rows.is_empty()) {
                if g.bool_with(0.9) {
                    let pr = parent.rows[g.index(parent.rows.len())].clone();
                    for (c, pc) in f.columns.iter().zip(&f.parent_columns) {
                        row[t.pos(*c).unwrap()] = pr[parent.pos(*pc).unwrap()].clone();
                    }
                }
            }
        }
        t.rows.push(row);
        if !rows_ok(&t, &t.rows) {
            t.rows.pop();
        }
    }
    if g.bool_with(0.15) {
        // Over a column that is not the identity column, which normalizing
        // renumbers.
        let cols: Vec<&Column> = t.columns.iter().filter(|c| !c.identity).collect();
        if !cols.is_empty() {
            let p = draw_pred(g, &Table { indexes: Vec::new(), ..t.clone() }, 2, Some(pool));
            let mut only = Vec::new();
            p.leaves(&mut only);
            if !only.iter().any(|x| Some(*x) == identity) {
                let mut with = t.clone();
                with.checks.push(p);
                let keep: Vec<Row> = with.rows.iter().filter(|r| with.checks.iter().all(|q| crate::semantics::eval(&with, r, q) != Some(false))).cloned().collect();
                with.rows = keep;
                if with.rows.len() * 2 >= t.rows.len() {
                    t = with;
                    renumber_identity(&mut t);
                }
            }
        }
    }
    // The rows that do not keep the foreign key go (for a self-reference,
    // until none is left without its parent).
    if let Some(f) = fk {
        t.fk = Some(f.clone());
        let mut tables: Vec<Table> = earlier.to_vec();
        for _ in 0..8 {
            tables.retain(|x| x.id != t.id);
            tables.push(t.clone());
            if crate::fk::holds(&tables, &t) {
                break;
            }
            let parent = if f.parent == t.id { t.clone() } else { tables.iter().find(|p| p.id == f.parent).unwrap().clone() };
            let ok = crate::fk::keeps(&t, &parent, &f).unwrap();
            t.rows = t.rows.iter().zip(ok).filter(|(_, k)| *k).map(|(r, _)| r.clone()).collect();
            renumber_identity(&mut t);
        }
    }
    let rows = t.rows.clone();
    t.record(&rows);
    for _ in 0..g.int(0..=3usize.min(MAX_INDEXES)) {
        let def = draw_index(g, &t, t.next_index, Some(pool));
        t.next_index += 1;
        if index_ok(&t, &def) {
            t.indexes.push(def);
        }
    }
    // Rows too wide for a page with their PLAIN columns lose the strategy.
    if !crate::model::sizes_ok(&t, &t.rows) {
        for c in &mut t.columns {
            if c.storage == Some(Storage::Plain) {
                c.storage = None;
            }
        }
    }
    t
}

/// A foreign key for a table being drawn (columns, key and partitioning
/// chosen, no rows yet), to an earlier table's unique constraint or now and
/// then to its own key; the table gains columns of the referenced types
/// when it has none to spare.
fn draw_setup_fk(g: &mut Source, t: &mut Table, earlier: &[Table]) -> Option<crate::model::ForeignKey> {
    if !g.bool_with(0.5) {
        return None;
    }
    let parents: Vec<&Table> = earlier.iter().filter(|p| !p.uniques().is_empty()).collect();
    let own = t.uniques();
    let self_ok = !own.is_empty() && t.identity().is_none();
    let (pid, pcols, ptypes): (u32, Vec<u32>, Vec<Type>) = if self_ok && (parents.is_empty() || g.bool_with(0.15)) {
        let (cols, _) = own[g.index(own.len())].clone();
        let types = cols.iter().map(|c| t.column(*c).unwrap().ty).collect();
        (t.id, cols, types)
    } else if !parents.is_empty() {
        let p = parents[g.index(parents.len())];
        let us = p.uniques();
        let (cols, _) = us[g.index(us.len())].clone();
        let types = cols.iter().map(|c| p.column(*c).unwrap().ty).collect();
        (p.id, cols, types)
    } else {
        return None;
    };
    if pcols.len() > 3 {
        return None;
    }
    let mut columns: Vec<u32> = Vec::new();
    for ty in &ptypes {
        let fits: Vec<u32> = t
            .columns
            .iter()
            .filter(|c| !c.identity && crate::fk::types_match(c.ty, *ty) && !columns.contains(&c.id) && (pid != t.id || !pcols.contains(&c.id)))
            .map(|c| c.id)
            .collect();
        if !fits.is_empty() && g.bool_with(0.5) {
            columns.push(*g.pick(&fits));
        } else if t.columns.len() < MAX_COLUMNS {
            let id = t.next_column;
            t.columns.push(Column::new(id, *ty));
            t.next_column += 1;
            columns.push(id);
        } else if !fits.is_empty() {
            columns.push(*g.pick(&fits));
        } else {
            return None;
        }
    }
    Some(crate::cases::draw_fk_options(g, columns, pid, pcols))
}

fn renumber_identity(t: &mut Table) {
    if let Some(p) = t.identity().and_then(|c| t.pos(c.id)) {
        for (k, r) in t.rows.iter_mut().enumerate() {
            r[p] = Value::Int(k as i64 + 1);
        }
    }
}

/// A layout for `t`.
fn draw_table_layout(g: &mut Source, t: &Table) -> TableLayout {
    let clusterable: Vec<u32> = t.indexes.iter().filter(|i| i.method == Method::Btree && i.filter.is_none()).map(|i| i.id).collect();
    let ghosts = if t.rows.is_empty() || g.bool_with(0.6) { 0 } else { g.int(1..=t.rows.len()) };
    TableLayout {
        fillfactor: g.bool_with(0.3).then(|| *g.pick(&[10, 20, 50, 70, 90, 100])),
        toast_target: g.bool_with(0.2).then(|| *g.pick(&[128, 256, 1024, 2032, 8160])),
        unlogged: g.bool_with(0.15),
        load: if g.bool() { Load::Copy } else { Load::Insert(*g.pick(&[1, 2, 7, 50, 1000])) },
        indexes_first: g.bool_with(0.4),
        ghosts,
        vacuum_ghosts: ghosts > 0 && g.bool(),
        touch: (!t.columns.is_empty() && g.bool_with(0.2)).then(|| (t.columns[g.index(t.columns.len())].id, g.bool_with(0.6).then(|| draw_pred(g, t, 2, None)))),
        after: match g.int(0..10) {
            0..=4 => After::Nothing,
            5 | 6 => After::Vacuum,
            7 => After::VacuumFreeze,
            8 => After::VacuumFull,
            _ if !clusterable.is_empty() => After::Cluster(*g.pick(&clusterable)),
            _ => After::Nothing,
        },
        fk_add: if t.fk.is_none() { FkAdd::After } else { *g.weighted(&[(4, FkAdd::After), (4, FkAdd::Inline), (1, FkAdd::NotValid), (2, FkAdd::Validate)]) },
        stats: (t.columns.len() >= 2 && g.bool_with(0.35)).then(|| {
            let mut ids: Vec<u32> = t.columns.iter().map(|c| c.id).collect();
            let n = g.int(2..=ids.len().min(4));
            let mut columns = Vec::new();
            for _ in 0..n {
                columns.push(ids.remove(g.index(ids.len())));
            }
            let kinds = if g.bool_with(0.4) { [false; 3] } else { [g.bool(), g.bool(), g.bool()] };
            ExtStats { kinds, columns }
        }),
        analyze: g.bool_with(0.5),
    }
}

/// A setup, with the pool its tables drew from.
pub fn draw_setup(g: &mut Source) -> (Setup, Pool) {
    let pool = Pool::draw(g);
    let n = if g.bool_with(0.03) { 0 } else { g.int(1..=MAX_TABLES.min(1 + g.size() / 12)) };
    let mut tables: Vec<Table> = Vec::new();
    for k in 0..n {
        let t = draw_table(g, k as u32, &pool, &tables);
        tables.push(t);
    }
    // Triggers now and then, made once the rows are loaded.
    for k in 0..n {
        if g.bool_with(0.35) {
            let m = Model { tables: tables.clone(), ..Model::default() };
            if let Some(tr) = crate::cases::draw_trigger(g, &m, &tables[k], &pool) {
                if crate::fk::trigger_ok(&tables, &tables[k], &tr) {
                    tables[k].next_trigger = tr.id + 1;
                    tables[k].triggers.push(tr);
                }
            }
        }
    }
    let mut layout = Layout { tables: tables.iter().map(|t| draw_table_layout(g, t)).collect() };
    // A permanent table references only permanent ones: a child of an
    // unlogged table is unlogged too (parents come first).
    for k in 0..n {
        if let Some(f) = &tables[k].fk {
            let p = tables.iter().position(|p| p.id == f.parent).unwrap();
            if layout.tables[p].unlogged && tables[k].partitioning.is_none() {
                layout.tables[k].unlogged = true;
            }
        }
    }
    let mut s = Setup { model: Model { tables, savepoints: Vec::new(), sequences: Default::default(), next_table: n as u32, txn: Default::default() }, layout };
    normalize(&mut s);
    (s, pool)
}

// ----------------------------------------------------------- shrinking

/// Simpler values of a value of its type (not checked against
/// constraints).
pub fn simpler_values(v: &Value) -> Vec<Value> {
    match v {
        Value::Null => vec![],
        Value::Int(0) => vec![Value::Null],
        Value::Int(i) => vec![Value::Null, Value::Int(0), Value::Int(i / 2)],
        Value::Float(x) if *x == 0.0 && x.is_sign_positive() => vec![Value::Null],
        Value::Float(x) => {
            let mut out = vec![Value::Null, Value::Float(0.0)];
            if x.is_finite() && x.fract() != 0.0 {
                out.push(Value::Float(x.trunc()));
            }
            out
        },
        Value::Numeric(Numeric::Finite { digits, scale, .. }) if digits.is_empty() && *scale == 0 => vec![Value::Null],
        Value::Numeric(Numeric::Finite { neg, digits, scale }) => {
            let mut out = vec![Value::Null, Value::Numeric(Numeric::int(0))];
            if *scale > 0 {
                let keep = digits.len().saturating_sub(usize::from(*scale));
                out.push(Value::Numeric(Numeric::finite(*neg, digits[..keep].to_vec(), 0)));
            }
            out
        },
        Value::Numeric(_) => vec![Value::Null, Value::Numeric(Numeric::int(0))],
        Value::Text(s) => {
            let mut half = s.len() / 2;
            while !s.is_char_boundary(half) {
                half -= 1;
            }
            vec![Value::Null, Value::Text(String::new()), Value::Text(s[..half].to_string())]
        },
        Value::Bool(true) => vec![Value::Null, Value::Bool(false)],
        Value::Bool(false) => vec![Value::Null],
        Value::Bytea(b) => vec![Value::Null, Value::Bytea(Vec::new()), Value::Bytea(b[..b.len() / 2].to_vec())],
        Value::Date(0) | Value::Timestamp(0) => vec![Value::Null],
        Value::Date(d) => vec![Value::Null, Value::Date(0), Value::Date(if d.unsigned_abs() > 1 << 20 { 0 } else { d / 2 })],
        Value::Timestamp(t) => vec![Value::Null, Value::Timestamp(0), Value::Timestamp(t.div_euclid(MICROS_PER_DAY).clamp(-(1 << 20), 1 << 20) * MICROS_PER_DAY)],
    }
}

fn model_candidates(m: &Model, visit: &mut dyn FnMut(Model) -> ControlFlow<()>) -> ControlFlow<()> {
    let with = |k: usize, f: &dyn Fn(&mut Table)| {
        let mut c = m.clone();
        f(&mut c.tables[k]);
        c
    };
    for k in (0..m.tables.len()).rev() {
        let mut c = m.clone();
        c.tables.remove(k);
        visit(c)?;
    }
    for (k, t) in m.tables.iter().enumerate() {
        let n = t.rows.len();
        if n > 0 {
            visit(with(k, &|t| t.rows.clear()))?;
        }
        if n > 1 {
            visit(with(k, &|t| t.rows.truncate(n / 2)))?;
            visit(with(k, &|t| {
                t.rows.drain(..n / 2);
            }))?;
        }
        for j in 0..t.indexes.len() {
            visit(with(k, &|t| {
                t.indexes.remove(j);
            }))?;
        }
        for j in 0..t.checks.len() {
            visit(with(k, &|t| {
                t.checks.remove(j);
            }))?;
        }
        if !t.key.is_empty() {
            visit(with(k, &|t| t.key.clear()))?;
        }
        for j in 0..t.triggers.len() {
            visit(with(k, &|t| {
                t.triggers.remove(j);
            }))?;
        }
        if let Some(f) = &t.fk {
            visit(with(k, &|t| t.fk = None))?;
            let simpler = crate::model::ForeignKey {
                on_delete: crate::model::Action::NoAction,
                on_update: crate::model::Action::NoAction,
                delete_set: Vec::new(),
                full: false,
                deferral: crate::model::Deferral::No,
                ..f.clone()
            };
            if simpler != *f {
                visit(with(k, &|t| t.fk = Some(simpler.clone())))?;
            }
        }
        if let Some(p) = &t.partitioning {
            visit(with(k, &|t| t.partitioning = None))?;
            for j in 0..p.parts.len() {
                if p.parts.len() > 1 {
                    visit(with(k, &|t| {
                        t.partitioning.as_mut().unwrap().parts.remove(j);
                    }))?;
                }
            }
        }
        if n > 1 && n <= 16 {
            for r in 0..n {
                visit(with(k, &|t| {
                    t.rows.remove(r);
                }))?;
            }
        }
        // A column less (not of the key), with what reads it.
        for c in t.columns.iter().filter(|c| !t.key.contains(&c.id)) {
            if t.columns.len() > 1 {
                let id = c.id;
                visit(with(k, &|t| {
                    let p = t.pos(id).unwrap();
                    t.columns.remove(p);
                    for r in &mut t.rows {
                        r.remove(p);
                    }
                    t.indexes.retain(|i| !i.columns_read().contains(&id));
                    t.checks.retain(|q| !q.reads(id));
                }))?;
            }
        }
        for (p, c) in t.columns.iter().enumerate() {
            let simpler = Column { not_null: false, default: None, identity: false, storage: None, compression: None, ..c.clone() };
            if simpler != *c {
                visit(with(k, &|t| t.columns[p] = simpler.clone()))?;
            }
        }
        for (j, i) in t.indexes.iter().enumerate() {
            for s in simpler_indexes(i) {
                visit(with(k, &|t| t.indexes[j] = s.clone()))?;
            }
        }
    }
    ControlFlow::Continue(())
}

/// Simpler versions of an index (not checked for validity).
pub fn simpler_indexes(i: &Index) -> Vec<Index> {
    let mut out = Vec::new();
    let mut push = |f: &dyn Fn(&mut Index)| {
        let mut c = i.clone();
        f(&mut c);
        if c != *i {
            out.push(c);
        }
    };
    push(&|c| {
        c.unique = false;
        c.nulls_not_distinct = false;
    });
    push(&|c| c.nulls_not_distinct = false);
    push(&|c| c.expr = None);
    push(&|c| c.filter = None);
    push(&|c| c.include.clear());
    push(&|c| c.params = IndexParams::default());
    push(&|c| c.method = Method::Btree);
    push(&|c| c.columns.iter_mut().for_each(|k| *k = IndexColumn::asc(k.column)));
    if i.columns.len() > 1 {
        for k in 0..i.columns.len() {
            push(&|c| {
                c.columns.remove(k);
            });
        }
    }
    out
}

fn value_candidates(m: &Model, visit: &mut dyn FnMut(Model) -> ControlFlow<()>) -> ControlFlow<()> {
    for (k, t) in m.tables.iter().enumerate() {
        for (r, row) in t.rows.iter().enumerate().take(64) {
            for (p, v) in row.iter().enumerate() {
                for w in simpler_values(v) {
                    let mut c = m.clone();
                    c.tables[k].rows[r][p] = w;
                    visit(c)?;
                }
            }
        }
    }
    ControlFlow::Continue(())
}

fn layout_candidates(l: &Layout, visit: &mut dyn FnMut(Layout) -> ControlFlow<()>) -> ControlFlow<()> {
    for (k, t) in l.tables.iter().enumerate() {
        let mut push = |f: &dyn Fn(&mut TableLayout)| {
            let mut c = l.clone();
            f(&mut c.tables[k]);
            if c != *l {
                visit(c)
            } else {
                ControlFlow::Continue(())
            }
        };
        push(&|t| *t = TableLayout::SIMPLE)?;
        push(&|t| t.fillfactor = None)?;
        push(&|t| t.toast_target = None)?;
        push(&|t| t.unlogged = false)?;
        push(&|t| t.load = Load::Copy)?;
        push(&|t| t.indexes_first = false)?;
        push(&|t| {
            t.ghosts = 0;
            t.vacuum_ghosts = false;
        })?;
        if t.ghosts > 1 {
            push(&|t| t.ghosts /= 2)?;
        }
        push(&|t| t.vacuum_ghosts = false)?;
        push(&|t| t.touch = None)?;
        push(&|t| t.after = After::Nothing)?;
        push(&|t| t.stats = None)?;
        if let Some(st) = &t.stats {
            for k in 0..3 {
                if st.kinds[k] {
                    push(&|t| t.stats.as_mut().unwrap().kinds[k] = false)?;
                }
            }
            if st.columns.len() > 2 {
                for k in 0..st.columns.len() {
                    push(&|t| {
                        t.stats.as_mut().unwrap().columns.remove(k);
                    })?;
                }
            }
        }
        push(&|t| t.analyze = false)?;
        push(&|t| t.fk_add = FkAdd::After)?;
    }
    ControlFlow::Continue(())
}

/// Candidates for a smaller setup, each normalized, well formed and lower in
/// [`Setup::cost`].
pub fn shrink_setup(s: &Setup, visit: &mut dyn FnMut(Setup) -> ControlFlow<()>) -> ControlFlow<()> {
    let cost = s.cost();
    let mut offer = |mut c: Setup| {
        normalize(&mut c);
        if c.cost() < cost && setup_ok(&c) {
            visit(c)
        } else {
            ControlFlow::Continue(())
        }
    };
    offer(Setup::empty())?;
    offer(Setup { model: s.model.clone(), layout: Layout::simple(s.model.tables.len()) })?;
    // Removing a table removes its layout with it.
    for k in (0..s.model.tables.len()).rev() {
        let mut c = s.clone();
        c.model.tables.remove(k);
        c.layout.tables.remove(k);
        offer(c)?;
    }
    model_candidates(&s.model, &mut |m| offer(Setup { model: m, layout: s.layout.clone() }))?;
    layout_candidates(&s.layout, &mut |l| offer(Setup { model: s.model.clone(), layout: l }))?;
    value_candidates(&s.model, &mut |m| offer(Setup { model: m, layout: s.layout.clone() }))
}

/// A nearby setup: rows added, removed or changed, an index added or
/// removed, a layout choice redrawn; a fresh one when no change is valid.
pub fn mutate_setup(s: &Setup, g: &mut Source) -> Setup {
    for _ in 0..8 {
        let mut c = s.clone();
        let pool = Pool::draw(g);
        for _ in 0..g.int(1..=3) {
            if c.model.tables.is_empty() {
                break;
            }
            let k = g.index(c.model.tables.len());
            let t = &mut c.model.tables[k];
            match g.int(0..10) {
                0..=2 => {
                    for _ in 0..g.int(1..=8) {
                        let row: Row = t.columns.iter().map(|col| if g.bool_with(0.1) { Value::Null } else { draw_value(g, col.ty, Some(&pool)) }).collect();
                        t.rows.push(row);
                        renumber_identity(t);
                        if !rows_ok(t, &t.rows) {
                            t.rows.pop();
                        }
                    }
                },
                3 if !t.rows.is_empty() => {
                    let r = g.index(t.rows.len());
                    t.rows.remove(r);
                },
                4 | 5 if !t.rows.is_empty() => {
                    let r = g.index(t.rows.len());
                    let p = g.index(t.columns.len());
                    t.rows[r][p] = draw_value(g, t.columns[p].ty, Some(&pool));
                },
                6 if t.indexes.len() < MAX_INDEXES => {
                    let def = draw_index(g, t, t.next_index, Some(&pool));
                    t.next_index += 1;
                    t.indexes.push(def);
                },
                7 if !t.indexes.is_empty() => {
                    let j = g.index(t.indexes.len());
                    t.indexes.remove(j);
                },
                8 => {
                    let t = c.model.tables[k].clone();
                    c.layout.tables[k] = draw_table_layout(g, &t);
                },
                _ if c.model.tables.len() < MAX_TABLES => {
                    let id = c.model.next_table;
                    let t = draw_table(g, id, &pool, &c.model.tables);
                    c.layout.tables.push(draw_table_layout(g, &t));
                    c.model.tables.push(t);
                    c.model.next_table += 1;
                },
                _ => {},
            }
        }
        normalize(&mut c);
        if setup_ok(&c) {
            return c;
        }
    }
    crate::fell_back(&crate::SETUP_FALLBACKS);
    draw_setup(g).0
}

impl Gen for Setups {
    type Value = Setup;

    fn generate(&self, g: &mut Source) -> Setup {
        draw_setup(g).0
    }

    fn shrink(&self, value: &Setup, visit: &mut dyn FnMut(Setup) -> ControlFlow<()>) -> ControlFlow<()> {
        shrink_setup(value, visit)
    }

    fn mutate(&self, value: &Setup, g: &mut Source) -> Setup {
        mutate_setup(value, g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawn_setups_are_well_formed_and_shrink_and_mutate_so() {
        for seed in 0..200u64 {
            let mut g = Source::new(seed, (seed % 100) as usize);
            let s = Setups.generate(&mut g);
            assert!(setup_ok(&s), "seed {seed}: {s:?}");
            let cost = s.cost();
            let mut seen = 0;
            let shown = format!("{s:?}");
            let _ = Setups.shrink(&s, &mut |c| {
                assert!(setup_ok(&c) && c.cost() < cost, "shrink of seed {seed}");
                assert!(format!("{c:?}") != shown, "seed {seed}: a candidate renders as its input\n{shown}\n{:?}\n{:?}", s.model, c.model);
                seen += 1;
                if seen < 80 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            });
            for _ in 0..3 {
                assert!(setup_ok(&Setups.mutate(&s, &mut g)), "mutant of seed {seed}");
            }
        }
    }

    #[test]
    fn drawn_setups_load() {
        crate::pg::with_session(|session| {
            for seed in 0..150u64 {
                let mut g = Source::new(seed, (seed % 100) as usize);
                let s = Setups.generate(&mut g);
                session.fresh()?;
                load(session, &s).map_err(|e| format!("seed {seed}: {e}\n{s:?}"))?;
                for t in &s.model.tables {
                    let got = session.values(&format!("SELECT * FROM t{}", t.id))?;
                    assert!(crate::semantics::same_rows(&got, &t.rows), "seed {seed}: t{} loaded as {got:?}\n{s:?}", t.id);
                }
            }
            Ok(())
        })
        .unwrap();
    }
}
