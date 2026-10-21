// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The generator of database files: a [`Model`] and a [`Layout`] drawn from
//! the [`Source`], written byte by byte by [`crate::writer`]. Shrinking and
//! mutation read a file back into its model and layout ([`crate::reader`]),
//! change those, and write again, so every candidate is valid by the same
//! construction as a fresh value. No hook runs SQLite.

use std::ops::ControlFlow;

use ephyra::{Gen, Source};

use crate::{
    model::{
        self, flag, ColType, Encoding, Index, Key, Layout, Model, Row, Table, Trigger, Value, View, MAX_COLUMNS, MAX_INDEXES,
        MAX_ROWS, MAX_TABLES, MAX_TRIGGERS, MAX_VIEWS,
    },
    reader,
    writer::{self, Rng},
    SqliteFile,
};

/// Database files written at the byte level from a model and a layout.
#[ephyra::generator(crate::integrity_ok, metrics = crate::file_metrics)]
pub fn sqlite_files() -> impl Gen<Value = SqliteFile> {
    Files
}

pub struct Files;

fn emit(mut m: Model, mut l: Layout) -> SqliteFile {
    model::normalize(&mut m, &mut l);
    SqliteFile(writer::write(&m, &l))
}

impl Gen for Files {
    type Value = SqliteFile;

    fn generate(&self, g: &mut Source) -> SqliteFile {
        draw(g, false)
    }

    /// Candidates are simpler files, each strictly lower in the measure
    /// `μ`: 0 for the empty file, `1 + Model::cost + Layout::cost` (a
    /// natural number) for a written one. The empty file is offered first
    /// and has no candidates; every other candidate's cost is checked
    /// before it is offered. So every shrink chain terminates.
    fn shrink(&self, value: &SqliteFile, visit: &mut dyn FnMut(SqliteFile) -> ControlFlow<()>) -> ControlFlow<()> {
        let Some((m, l)) = reader::read(&value.0) else {
            return ControlFlow::Continue(());
        };
        visit(SqliteFile(Vec::new()))?;
        let cost = m.cost() + l.cost();
        let mut offer = |mut cm: Model, mut cl: Layout| {
            model::normalize(&mut cm, &mut cl);
            if cm.cost() + cl.cost() < cost {
                visit(SqliteFile(writer::write(&cm, &cl)))
            } else {
                ControlFlow::Continue(())
            }
        };
        // The simplest written file next: an empty database laid out as
        // SQLite would. Then the same model laid out simply, then smaller
        // steps.
        offer(Model { encoding: Encoding::Utf8, tables: Vec::new(), views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() }, Layout::SIMPLE)?;
        offer(m.clone(), Layout::SIMPLE)?;
        model_candidates(&m, &mut |cm| offer(cm, l))?;
        layout_candidates(&l, &mut |cl| offer(m.clone(), cl))?;
        value_candidates(&m, &mut |cm| offer(cm, l))
    }

    fn mutate(&self, value: &SqliteFile, g: &mut Source) -> SqliteFile {
        let Some((mut m, mut l)) = reader::read(&value.0) else {
            crate::fell_back(&crate::FILE_FALLBACKS);
            return self.generate(g);
        };
        for _ in 0..g.int(1..=3) {
            if g.bool_with(0.35) {
                mutate_layout(&mut l, g);
            } else {
                mutate_model(&mut m, &l, g);
            }
        }
        emit(m, l)
    }
}

// ---------------------------------------------------------------- drawing

/// A file; for a case (`case`), drawn as [`draw_layout`] says.
fn draw(g: &mut Source, case: bool) -> SqliteFile {
    if g.bool_with(0.01) {
        // The empty file, which SQLite reads as an empty database.
        return SqliteFile(Vec::new());
    }
    let packed = g.bool_with(0.4);
    draw_file(g, packed, case)
}

/// A file for a case's statements to run on: as [`Files`] draws them, but
/// more often with a freelist, which the statements' new pages come from
/// (a file with free pages is larger, and slower for the file generator's
/// own check).
pub fn draw_for_statements(g: &mut Source) -> SqliteFile {
    draw(g, true)
}

/// A layout; for a case's file (`case`), more often with a freelist, with
/// wide integers, with auto-vacuum, less often with 8192-byte pages, never
/// with pages of 16384 bytes or more (files big for what they hold) and
/// never with 4096-byte pages (the control's).
fn draw_layout(g: &mut Source, case: bool) -> Layout {
    let freelist = if case { 0.95 } else { 0.5 };
    let page_size = *g.weighted(&[
        (6, 512u32),
        (6, 1024),
        (4, 2048),
        (if case { 0 } else { 4 }, 4096),
        (if case { 1 } else { 2 }, 8192),
        (if case { 0 } else { 1 }, 16384),
        (if case { 0 } else { 1 }, 32768),
        (if case { 0 } else { 1 }, 65536),
    ]);
    let max_reserved = (page_size - 480).min(255);
    let reserved = if g.bool_with(0.5) { 0 } else { g.int(1..=max_reserved) as u8 };
    // Sparse pages make deep trees, which want small pages: a sparse large
    // page only makes the file big and slow to check.
    let least = (page_size / 1024).clamp(1, 50) as u8;
    let fill = |g: &mut Source| match g.int(0..4) {
        0 => 100,
        1 => g.int(least..=(least + 15).min(100)),
        _ => g.int(least..=100),
    };
    let free_pages = if !g.bool_with(freelist) { 0 } else { g.int(1..=Layout::max_free(page_size).min(4 + 5 * g.size() as u32)) };
    Layout {
        page_size,
        reserved,
        format: g.int(1u8..=4),
        wal: g.bool(),
        leaf_fill: fill(g),
        // Interior pages of a few cells make deep trees from few leaves.
        interior_fill: if g.bool_with(0.65) { g.int(least..=(least + 3).min(100)) } else { fill(g) },
        flags: if g.bool_with(0.2) { 0 } else { g.int(0u8..=flag::ALL) } | if case && g.bool_with(0.5) { flag::WIDE } else { 0 },
        trunk_cap: if g.bool_with(0.5) { 255 } else { g.int(1u8..=255) },
        free_pages,
        seed: g.int(..),
        vacuum: if case && g.bool_with(0.85) { g.int(1u8..=2) } else { g.int(0u8..=2) },
    }
}

/// A file drawn from a layout and a model; when `packed`, with full pages
/// and no gaps, its tables cut to end on full leaves and page 1 filled.
fn draw_file(g: &mut Source, packed: bool, case: bool) -> SqliteFile {
    let mut layout = draw_layout(g, case);
    if packed {
        layout.leaf_fill = 100;
        if g.bool() {
            layout.interior_fill = 100;
        }
        layout.flags &= !flag::GAPS;
    }
    let mut model = draw_model(g, &layout, if packed { 0.5 } else { 0.2 }, case);
    if packed {
        pack(g, &mut model, &mut layout);
    }
    emit(model, layout)
}

/// Pack a model's pages, for a layout with full pages and no gaps: cut
/// every table to end on a full leaf, then fill page 1 with a view padded
/// to its free space (when the schema fits there and a view can be added).
fn pack(g: &mut Source, m: &mut Model, l: &mut Layout) {
    model::normalize(m, l);
    for t in 0..m.tables.len() {
        let keep = writer::packed_rows(m, l, t);
        m.tables[t].rows.truncate(keep);
    }
    if m.views.len() >= MAX_VIEWS {
        return;
    }
    let table = (!m.tables.is_empty() && g.bool()).then(|| g.index(m.tables.len()));
    m.views.push(View { table, columns: Vec::new(), pad: 0 });
    let k = m.views.len() - 1;
    if writer::schema_slack(m, l).is_some() {
        // The largest pad that keeps the schema on page 1. Below this bound
        // the view's row never overflows, so the slack falls as pad grows.
        let per_char = if m.encoding == Encoding::Utf8 { 1 } else { 2 };
        let (mut lo, mut hi) = (0, (l.usable() - 108) / per_char);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            m.views[k].pad = mid;
            if writer::schema_slack(m, l).is_some() {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        // Mostly the largest pad; now and then a page 1 a little less full.
        m.views[k].pad = if g.bool_with(0.7) { lo } else { g.int(lo * 3 / 4..=lo) };
    }
}

/// A short value admitted by `ty` (text and blobs of at most `longest`
/// characters or bytes), NULL now and then.
fn draw_short(g: &mut Source, ty: ColType, longest: usize) -> Value {
    if g.bool_with(0.05) {
        return Value::Null;
    }
    let ty = if ty == ColType::Any { *g.pick(&[ColType::Int, ColType::Real, ColType::Text, ColType::Blob]) } else { ty };
    match ty {
        ColType::Int | ColType::Integer | ColType::Any => {
            Value::Int(if g.bool_with(0.7) { g.int(-3..=3) } else { g.int(-100_000..=100_000) })
        },
        ColType::Real => Value::Real(if g.bool() { f64::from(g.int(-50i32..=50)) } else { f64::from(g.int(-999i32..=999)) / 8.0 }),
        ColType::Text if longest <= 6 => {
            let n = g.int(0..=longest);
            Value::Text((0..n).map(|_| *g.pick(&ALPHABET)).collect())
        },
        ColType::Blob if longest <= 6 => Value::Blob((0..g.int(0..=longest)).map(|_| g.int(0u8..=255)).collect()),
        // Longer values are expanded from one drawn seed.
        ColType::Text => {
            let n = g.int(0..=longest);
            let mut rng = Rng::new(g.int(..));
            Value::Text((0..n).map(|_| ALPHABET[rng.below(ALPHABET.len() as u64) as usize]).collect())
        },
        ColType::Blob => {
            let n = g.int(0..=longest);
            let mut rng = Rng::new(g.int(..));
            Value::Blob((0..n).map(|_| rng.below(256) as u8).collect())
        },
    }
}

/// The chance that a table that is not STRICT declares column types.
pub(crate) const DECLARED: f64 = 0.35;

/// Declared types for a table that is not STRICT ([`model::DECLS`]): each
/// column but an INTEGER PRIMARY KEY 70% of the time, of every affinity.
pub(crate) fn draw_decls(g: &mut Source, table: &Table) -> Vec<Option<usize>> {
    (0..table.columns)
        .map(|c| match table.key {
            Key::IntegerPrimaryKey(k) if k == c => None,
            _ => g.bool_with(0.7).then(|| g.index(model::DECLS.len())),
        })
        .collect()
}

/// The chance that a table gives its columns `COLLATE` clauses.
pub(crate) const COLLATED: f64 = 0.06;

/// `COLLATE` clauses for a table's columns ([`Table::coll`]): each column
/// but an INTEGER PRIMARY KEY 60% of the time, NOCASE and RTRIM twice as
/// often as an explicit BINARY.
pub(crate) fn draw_colls(g: &mut Source, table: &Table) -> Vec<Option<model::Coll>> {
    (0..table.columns)
        .map(|c| match table.key {
            Key::IntegerPrimaryKey(k) if k == c => None,
            _ => g.bool_with(0.6).then(|| *g.weighted(&[(1, model::Coll::Binary), (2, model::Coll::Nocase), (2, model::Coll::Rtrim)])),
        })
        .collect()
}

/// `COLLATE` clauses for some of an index's columns (none 92% of the
/// time), of any sequence, the column's own included.
pub(crate) fn draw_index_colls(g: &mut Source, i: &Index) -> Vec<(usize, model::Coll)> {
    if !g.bool_with(0.08) {
        return Vec::new();
    }
    let skip = usize::from(i.expr.is_some());
    let mut out = Vec::new();
    for &(c, _) in i.columns.iter().skip(skip) {
        if g.bool_with(0.6) {
            out.push((c, *g.pick(&model::Coll::ALL)));
        }
    }
    out
}

/// Column types for a STRICT table; an INTEGER PRIMARY KEY is INTEGER.
pub(crate) fn draw_types(g: &mut Source, table: &Table) -> Vec<ColType> {
    (0..table.columns)
        .map(|c| match table.key {
            Key::IntegerPrimaryKey(k) if k == c => ColType::Integer,
            _ => *g.pick(&ColType::ALL),
        })
        .collect()
}

fn draw_view(g: &mut Source, m: &Model) -> View {
    let table = (!m.tables.is_empty() && g.bool_with(0.8)).then(|| g.index(m.tables.len()));
    let mut columns = Vec::new();
    if let Some(t) = table {
        let mut cols: Vec<usize> = (0..m.tables[t].columns).collect();
        for _ in 0..g.int(0..=cols.len().min(3)) {
            columns.push(cols.remove(g.index(cols.len())));
        }
    }
    let pad = match g.int(0..4) {
        0 | 1 => 0,
        2 => g.int(1..=20),
        _ => g.int(1..=300),
    };
    View { table, columns, pad }
}

/// A trigger on one of the model's tables, which it must have.
fn draw_trigger(g: &mut Source, m: &Model) -> Trigger {
    let table = g.index(m.tables.len());
    let column = g.bool_with(0.7).then(|| g.index(m.tables[table].columns));
    Trigger { table, after: g.bool(), event: g.int(0u8..=2), column }
}

/// A row count for a table at the current size: often small, sometimes up
/// to the cap.
fn draw_row_count(g: &mut Source) -> usize {
    let most = (4 + 12 * g.size()).min(160);
    match g.int(0..10) {
        0 => 0,
        1..=5 => g.int(0..=most.min(12)),
        _ => g.int(0..=most),
    }
}

/// A model; `dense` is the chance that a table is dense; `case` for a
/// case's file.
fn draw_model(g: &mut Source, layout: &Layout, dense: f64, case: bool) -> Model {
    let encoding = *g.weighted(&[(2, Encoding::Utf8), (1, Encoding::Utf16le), (1, Encoding::Utf16be)]);
    // Few trees on large pages, whose files are big for what they hold
    // (and every byte of a file is hashed several times per check).
    // (and every byte of a file is hashed several times per check); now
    // and then as many as on small pages.
    let most = match layout.page_size {
        _ if g.bool_with(0.7) => MAX_TABLES,
        16384 => 2,
        32768.. => 1,
        _ => MAX_TABLES,
    };
    let tables = if g.bool_with(0.03) { 0 } else { g.int(1..=most.min(1 + g.size() / 2)) };
    let mut budget = MODEL_BYTES;
    let mut index_budget = INDEX_BYTES;
    // Dense tables on sparse leaves are capped to keep their pages few.
    // Now and then every table is empty: a schema without data (less
    // often for a case's file, where statements read the generated pages).
    let most_dense = 16 * usize::from(layout.leaf_fill) / tables.max(1);
    let dense = if g.bool_with(if case { 0.03 } else { 0.12 }) { (0, 0.0) } else { (most_dense, dense) };
    let tables = (0..tables).map(|_| draw_table(g, layout, &mut budget, dense, &mut index_budget)).collect();
    let mut m = Model { encoding, tables, views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
    // A schema without data is often a large one.
    let (odds, most) = if dense.0 == 0 { (0.8, 20) } else { (0.55, 15) };
    if g.bool_with(odds) {
        for _ in 0..g.int(1..=most) {
            let v = draw_view(g, &m);
            m.views.push(v);
        }
    }
    if !m.tables.is_empty() && g.bool_with((odds + 0.15f64).min(1.0)) {
        for _ in 0..g.int(1..=most) {
            let t = draw_trigger(g, &m);
            m.triggers.push(t);
        }
    }
    m
}

/// An index over 1 to 4 distinct columns (often several, so that indexes
/// of one table share a leading column and differ after it, and the
/// planner chooses between them), each descending 40% of the time.
pub(crate) fn draw_index(g: &mut Source, columns: usize) -> Index {
    let mut cols: Vec<usize> = (0..columns).collect();
    let n = g.int(1..=columns.min(4));
    let mut picked = Vec::with_capacity(n);
    for _ in 0..n {
        let c = cols.remove(g.index(cols.len()));
        picked.push((c, g.bool_with(0.4)));
    }
    Index::of(picked)
}

pub(crate) fn draw_key(g: &mut Source, columns: usize) -> Key {
    match g.int(0..20) {
        0..=11 => Key::Rowid,
        12..=14 => Key::IntegerPrimaryKey(g.index(columns)),
        _ => {
            let mut cols: Vec<usize> = (0..columns).collect();
            let n = g.int(1..=columns.min(2));
            Key::WithoutRowid((0..n).map(|_| cols.remove(g.index(cols.len()))).collect())
        },
    }
}

/// A table; with the chance `dense.1` a dense one, of up to `dense.0` short
/// rows; an empty one when `dense.0` is 0. Its indexes, up to
/// [`MAX_INDEXES`] (one on pages of 32768 bytes or more), come out of
/// `index_budget`, the bytes of index entries the file may still hold
/// ([`index_bytes`]): the planner chooses between several indexes on one
/// table, and the budget, not the table count, keeps the file small.
/// STRICT 30% of the time; otherwise with declared types ([`draw_decls`])
/// [`DECLARED`] of the time, whose affinities convert the drawn values as
/// SQLite would have stored them (`model::normalize`).
fn draw_table(g: &mut Source, layout: &Layout, budget: &mut usize, dense: (usize, f64), index_budget: &mut usize) -> Table {
    let columns = g.int(1..=MAX_COLUMNS.min(6));
    let key = draw_key(g, columns);
    let empty = dense.0 == 0;
    let dense = (dense.0 >= 50 && g.bool_with(dense.1)).then(|| g.int(50..=dense.0));
    // Some dense tables have longer values, for fat keys and deep trees.
    let longest = if dense.is_some() && g.bool_with(if layout.page_size >= 16384 { 0.8 } else { 0.4 }) { g.int(20..=(layout.usable() / 4).max(160)) } else { 6 };
    // Their bytes are capped, more loosely now and then on large pages.
    let cap = if layout.page_size >= 16384 && g.bool_with(0.3) { 150_000 } else { 30_000 };
    let dense = dense.map(|n| if longest > 6 && g.bool_with(0.3) { g.int(10..=50) } else { n.min(cap / longest) });
    let n = if empty { 0 } else { dense.unwrap_or_else(|| draw_row_count(g)) };
    let mut table = Table::new(columns, key.clone());
    if g.bool_with(0.3) {
        table.strict = Some(draw_types(g, &table));
    } else if g.bool_with(DECLARED) {
        table.decl = draw_decls(g, &table);
    }
    if g.bool_with(COLLATED) {
        table.coll = draw_colls(g, &table);
    }
    for c in 0..columns {
        table.not_null[c] = !matches!(key, Key::IntegerPrimaryKey(k) if k == c) && g.bool_with(0.15);
    }
    // A STRICT column draws what its type admits (ANY: anything); a TEXT
    // one is a column of dates now and then.
    let kinds: Vec<u8> = (0..columns)
        .map(|c| match table.strict.as_ref().map(|t| t[c]) {
            Some(ColType::Blob) => 4,
            Some(ColType::Int | ColType::Integer) => 1,
            Some(ColType::Real) => 2,
            Some(ColType::Text) => if g.bool_with(0.3) { 7 } else { 3 },
            _ => g.int(0u8..=6),
        })
        .collect();
    let rowids = draw_rowids(g, n);
    let pk: &[usize] = match &key {
        Key::WithoutRowid(pk) => pk,
        _ => &[],
    };
    let rows = rowids
        .into_iter()
        .map(|rowid| {
            let values = (0..columns)
                .map(|c| {
                    let v = if pk.contains(&c) && g.bool_with(0.7) {
                        // Mostly distinct primary keys, of every type,
                        // derived from the distinct rowids.
                        let class = match kinds[c] {
                            k @ 1..=4 if table.strict.is_some() => k - 1,
                            7 => 2,
                            _ => g.int(0..4),
                        };
                        match class {
                            0 => Value::Int(rowid),
                            1 => Value::Real(rowid as f64 + 0.5),
                            2 => Value::Text(format!("k{rowid}")),
                            _ => Value::Blob(rowid.to_be_bytes().to_vec()),
                        }
                    } else {
                        match dense {
                            Some(_) if kinds[c] >= 6 => dated(g, kinds[c]),
                            Some(_) => draw_short(g, short_type(kinds[c]), longest),
                            None => draw_value(g, kinds[c], layout, budget),
                        }
                    };
                    match v {
                        Value::Null if table.not_null[c] => non_null(kinds[c]),
                        v => v,
                    }
                })
                .collect();
            Row { rowid, values }
        })
        .collect();
    table.rows = rows;
    let most = if layout.page_size >= 32768 { 1 } else { MAX_INDEXES };
    for _ in 0..g.int(0..=most) {
        let mut i = draw_index(g, columns);
        i.coll = draw_index_colls(g, &i);
        // UNIQUE a third of the time, half of those times with the key
        // columns (an INTEGER PRIMARY KEY, a WITHOUT ROWID primary key) in
        // it, which makes it unique whatever the rows; otherwise it stays
        // UNIQUE only where the rows allow (`model::normalize`).
        if g.bool_with(0.33) {
            i.unique = true;
            let key: &[usize] = match &key {
                Key::IntegerPrimaryKey(k) => std::slice::from_ref(k),
                Key::WithoutRowid(pk) => pk,
                Key::Rowid => &[],
            };
            if g.bool() {
                for &c in key {
                    if !i.columns.iter().any(|&(x, _)| x == c) {
                        i.columns.push((c, false));
                    }
                }
            }
        }
        let bytes = index_bytes(&table, &i);
        if bytes > *index_budget {
            break;
        }
        *index_budget -= bytes;
        table.indexes.push(i);
    }
    table
}

/// About the bytes of an index's entries: its columns' values and a few
/// bytes of key and header per row.
fn index_bytes(t: &Table, i: &Index) -> usize {
    let size = |v: &Value| match v {
        Value::Null => 1,
        Value::Int(_) | Value::Real(_) => 8,
        Value::Text(s) => s.len(),
        Value::Blob(b) => b.len(),
    };
    t.rows.iter().map(|r| 6 + i.columns.iter().map(|&(c, _)| size(&r.values[c])).sum::<usize>()).sum()
}

/// The type of a short value for a column kind (see [`draw_value`]).
fn short_type(kind: u8) -> ColType {
    match kind {
        1 => ColType::Int,
        2 => ColType::Real,
        3 | 7 => ColType::Text,
        4 => ColType::Blob,
        _ => ColType::Any,
    }
}

/// A small non-NULL value of a column kind (see [`draw_value`]).
fn non_null(kind: u8) -> Value {
    match kind {
        2 => Value::Real(0.5),
        3 | 7 => Value::Text(String::new()),
        4 => Value::Blob(Vec::new()),
        _ => Value::Int(0),
    }
}

/// A time value for a column of dates ([`draw_date`]): of any class for
/// kind 6, text for kind 7 (a STRICT TEXT column), NULL now and then.
fn dated(g: &mut Source, kind: u8) -> Value {
    if g.bool_with(0.08) {
        return Value::Null;
    }
    match draw_date(g) {
        Value::Text(s) => Value::Text(s),
        _ if kind == 7 => Value::Text(format!("{}-01-01", g.pick(&["2024", "2023", "2000"]))),
        v => v,
    }
}

/// Strictly increasing rowids: dense from 1, sparse, or anywhere in i64.
fn draw_rowids(g: &mut Source, n: usize) -> Vec<i64> {
    let mut ids = Vec::with_capacity(n);
    match g.int(0..4) {
        0 | 1 => ids.extend(1..=n as i64),
        2 => {
            let mut at: i64 = g.int(-1000..=1000);
            for _ in 0..n {
                ids.push(at);
                at += g.int(1..=1_000_000);
            }
        },
        _ => {
            let mut set = std::collections::BTreeSet::new();
            while set.len() < n {
                let id: i64 = if g.bool_with(0.1) { *g.pick(&[i64::MIN, i64::MAX, 0, -1]) } else { g.int(..) };
                set.insert(id);
            }
            ids.extend(set);
        },
    }
    ids
}

const ALPHABET: [char; 12] = ['a', 'b', 'c', 'z', 'A', '0', ' ', 'é', 'ж', '中', '😀', '\u{ff5e}'];

/// A text or blob length. Long lengths, enough to overflow, sometimes over
/// several pages, come out of `budget`, which bounds the bytes of a file so
/// that checking stays fast.
fn draw_len(g: &mut Source, layout: &Layout, budget: &mut usize) -> usize {
    let n = match g.int(0..20) {
        0..=13 => g.int(0..=8),
        14..=17 => g.int(0..=200),
        _ => g.int(0..=(3 * layout.page_size as usize).min(12_000)),
    };
    if n <= 8 {
        return n;
    }
    let n = n.min(*budget);
    *budget -= n;
    n
}

/// Days from 1970-01-01 to `y-m-d` in the proleptic Gregorian calendar
/// (a day past the month's end runs into the next month, as SQLite reads
/// it).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// Malformed time values and edges SQLite reads as NULL or normalizes: a
/// month or day out of range, two-digit years, slashes, a stray space,
/// hours past 24, a timezone without the time; and ones it reads that the
/// drawn forms do not take: a negative year, numbers as text (a julian
/// day, a unix epoch only with the `unixepoch` or `auto` modifier), a
/// timezone without a colon, a leap second.
const ODD_DATES: [&str; 20] = [
    "-0044-03-15",
    "-4713-11-24 12:00:00",
    "2460324.5",
    "1705276800",
    "2024-01-15 12:00:00-0500",
    "2024-02-29 23:59:60.5",
    "2024-01-15T12:00Z",
    "  12:30:45  ",
    "2024-13-01",
    "2024-02-30",
    "2023-02-29",
    "24-01-15",
    "2024/01/15",
    " 2024-01-15",
    "2024-01-15 24:00",
    "2024-01-15Z",
    "0000-01-01",
    "9999-12-31 23:59:59.999",
    "2024-1-5",
    "12:60",
];

/// A time value as SQLite's date and time functions read one (none is
/// `now`, the only value whose meaning is not fixed): mostly text of a few
/// dates (four years, four months, days at a month's start, middle and
/// end, so that rows of different tables share them and fall in the same
/// day or month), as `YYYY-MM-DD`, with a time to the minute, second or
/// millisecond, a `T` separator, a timezone suffix (`Z`, `+05:30`,
/// `-08:00`), or a bare time (`HH:MM[:SS]`); else the same instants as unix
/// epoch integers or reals and julian day numbers, edges (day 0, the last
/// day SQLite reads), and malformed text ([`ODD_DATES`]).
pub fn draw_date(g: &mut Source) -> Value {
    let y: i64 = *g.weighted(&[(6, 2024), (2, 2023), (1, 2000), (1, 1970)]);
    let m: i64 = *g.pick(&[1, 2, 3, 12]);
    let d: i64 = *g.pick(&[1, 15, 28, 29, 31]);
    let (h, mi, s): (i64, i64, i64) = (*g.pick(&[0, 9, 12, 23]), *g.pick(&[0, 30, 59]), *g.pick(&[0, 15, 59]));
    let day = format!("{y:04}-{m:02}-{d:02}");
    let secs = days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + s;
    Value::Text(match g.int(0..40) {
        0..=11 => day,
        12..=16 => format!("{day} {h:02}:{mi:02}:{s:02}"),
        17..=18 => format!("{day} {h:02}:{mi:02}"),
        19..=20 => format!("{day}T{h:02}:{mi:02}:{s:02}.{:03}", g.pick(&[0, 5, 123, 999])),
        21..=22 => format!("{day} {h:02}:{mi:02}{}", g.pick(&["Z", "+05:30", "-08:00", "+00:00", " +01:00"])),
        23..=24 => {
            if g.bool() {
                format!("{h:02}:{mi:02}")
            } else {
                format!("{h:02}:{mi:02}:{s:02}.{:03}", g.pick(&[0, 250, 999]))
            }
        },
        25..=28 => return Value::Int(secs),
        29..=30 => return Value::Real(secs as f64 + 0.5),
        31..=34 => return Value::Real(secs as f64 / 86_400.0 + 2_440_587.5),
        35 => return g.pick(&[Value::Int(0), Value::Real(5_373_484.499), Value::Int(-1), Value::Real(2_451_545.0)]).clone(),
        _ => (*g.pick(&ODD_DATES)).to_string(),
    })
}

/// The byte budget of a freshly drawn model.
const MODEL_BYTES: usize = 32_000;
/// The byte budget of one mutation.
const MUTATION_BYTES: usize = 20_000;
/// The bytes of index entries a freshly drawn model may hold.
const INDEX_BYTES: usize = 40_000;

/// A value; `kind` biases a column: 0 mixed (now and then a time value),
/// 1 integers, 2 reals, 3 text, 4 blobs, 5 mostly NULL, 6 time values of
/// any class and 7 as text ([`dated`]).
fn draw_value(g: &mut Source, kind: u8, layout: &Layout, budget: &mut usize) -> Value {
    if kind >= 6 || (kind == 0 && g.bool_with(0.15)) {
        return dated(g, kind);
    }
    let class = match kind {
        0 => g.int(0u8..=4),
        5 => {
            if g.bool_with(0.8) {
                0
            } else {
                g.int(1u8..=4)
            }
        },
        k => {
            if g.bool_with(0.1) {
                0
            } else {
                k
            }
        },
    };
    match class {
        0 => Value::Null,
        1 => Value::Int(match g.int(0..6) {
            0..=2 => g.int(-3..=3),
            3 => g.int(-100_000..=100_000),
            4 => *g.pick(&[i64::MIN, i64::MAX, 127, 128, -129, 32767, 32768, 8_388_608, 1 << 31, 1 << 47, 1 << 48]),
            _ => g.int(..),
        }),
        2 => Value::Real(match g.int(0..5) {
            0 => f64::from(g.int(-3i32..=3)),
            1 => g.f64() * 100.0 - 50.0,
            2 => *g.pick(&[-0.0, 0.5, 1e300, -1e-300, f64::INFINITY, f64::NEG_INFINITY, 9.223_372_036_854_776e18]),
            _ => {
                let x = g.f64_bits();
                if x.is_nan() {
                    0.25
                } else {
                    x
                }
            },
        }),
        // The contents of a text or blob are expanded from one drawn seed:
        // drawing every byte from the source is slow for long values, and
        // their individual bytes matter little.
        3 => {
            let n = draw_len(g, layout, budget);
            let mut rng = Rng::new(g.int(..));
            let mut s = String::with_capacity(n);
            while s.len() < n {
                s.push(ALPHABET[rng.below(ALPHABET.len() as u64) as usize]);
            }
            Value::Text(s)
        },
        _ => {
            let n = draw_len(g, layout, budget);
            let mut rng = Rng::new(g.int(..));
            Value::Blob((0..n.div_ceil(8)).flat_map(|_| rng.next_u64().to_le_bytes()).take(n).collect())
        },
    }
}

// --------------------------------------------------------------- shrinking

type Visit<'a, T> = &'a mut dyn FnMut(T) -> ControlFlow<()>;

/// Structural candidates: fewer tables, views, triggers, indexes, rows,
/// columns, constraints; UTF-8; rowids renumbered from 1.
fn model_candidates(m: &Model, visit: Visit<Model>) -> ControlFlow<()> {
    for t in 0..m.tables.len() {
        visit(remove_table(m, t))?;
    }
    for k in 0..m.views.len() {
        let mut c = m.clone();
        c.views.remove(k);
        visit(c)?;
    }
    for k in 0..m.triggers.len() {
        let mut c = m.clone();
        c.triggers.remove(k);
        visit(c)?;
    }
    for (k, v) in m.views.iter().enumerate() {
        let with = |f: &dyn Fn(&mut View)| {
            let mut c = m.clone();
            f(&mut c.views[k]);
            c
        };
        if v.pad > 0 {
            visit(with(&|v| v.pad = 0))?;
            visit(with(&|v| v.pad /= 2))?;
        }
        if v.table.is_some() {
            visit(with(&|v| {
                v.table = None;
                v.columns.clear();
            }))?;
        }
        for i in 0..v.columns.len() {
            visit(with(&|v| {
                v.columns.remove(i);
            }))?;
        }
    }
    for (k, g) in m.triggers.iter().enumerate() {
        let with = |f: &dyn Fn(&mut Trigger)| {
            let mut c = m.clone();
            f(&mut c.triggers[k]);
            c
        };
        if g.column.is_some() {
            visit(with(&|g| g.column = None))?;
        }
        if g.after {
            visit(with(&|g| g.after = false))?;
        }
        if g.event > 0 {
            visit(with(&|g| g.event = 0))?;
        }
    }
    for (t, table) in m.tables.iter().enumerate() {
        if let Some(types) = &table.strict {
            let mut c = m.clone();
            c.tables[t].strict = None;
            visit(c)?;
            for (k, ty) in types.iter().enumerate() {
                if *ty != ColType::Any {
                    let mut c = m.clone();
                    c.tables[t].strict.as_mut().unwrap()[k] = ColType::Any;
                    visit(c)?;
                }
            }
        }
        for k in 0..table.columns {
            if table.not_null[k] {
                let mut c = m.clone();
                c.tables[t].not_null[k] = false;
                visit(c)?;
            }
        }
        if table.coll.iter().any(Option::is_some) {
            let mut c = m.clone();
            c.tables[t].coll.fill(None);
            visit(c)?;
            for k in 0..table.columns {
                if table.coll[k].is_some() {
                    let mut c = m.clone();
                    c.tables[t].coll[k] = None;
                    visit(c)?;
                }
            }
        }
        for i in 0..table.indexes.len() {
            if !table.indexes[i].coll.is_empty() {
                let mut c = m.clone();
                c.tables[t].indexes[i].coll.clear();
                visit(c)?;
            }
        }
        if table.decl.iter().any(Option::is_some) {
            let mut c = m.clone();
            c.tables[t].decl.fill(None);
            visit(c)?;
            for k in 0..table.columns {
                if table.decl[k].is_some() {
                    let mut c = m.clone();
                    c.tables[t].decl[k] = None;
                    visit(c)?;
                }
            }
        }
    }
    for (t, table) in m.tables.iter().enumerate() {
        for i in 0..table.indexes.len() {
            let mut c = m.clone();
            c.tables[t].indexes.remove(i);
            visit(c)?;
        }
    }
    for (t, table) in m.tables.iter().enumerate() {
        let n = table.rows.len();
        if n >= 2 {
            for keep in [0..n / 2, n / 2..n] {
                let mut c = m.clone();
                c.tables[t].rows = table.rows[keep].to_vec();
                visit(c)?;
            }
        }
    }
    if m.encoding != Encoding::Utf8 {
        let mut c = m.clone();
        c.encoding = Encoding::Utf8;
        visit(c)?;
    }
    for (t, table) in m.tables.iter().enumerate() {
        if let Key::WithoutRowid(pk) = &table.key {
            for k in 0..pk.len() {
                let mut c = m.clone();
                let mut fewer = pk.clone();
                fewer.remove(k);
                c.tables[t].key = if fewer.is_empty() { Key::Rowid } else { Key::WithoutRowid(fewer) };
                visit(c)?;
            }
        } else if table.key != Key::Rowid {
            let mut c = m.clone();
            c.tables[t].key = Key::Rowid;
            visit(c)?;
        }
    }
    for (t, table) in m.tables.iter().enumerate() {
        if table.columns > 1 {
            for col in 0..table.columns {
                visit(drop_column(m, t, col))?;
            }
        }
    }
    for (t, table) in m.tables.iter().enumerate() {
        if table.rows.iter().enumerate().any(|(k, r)| r.rowid != k as i64 + 1) {
            let mut c = m.clone();
            for (k, r) in c.tables[t].rows.iter_mut().enumerate() {
                r.rowid = k as i64 + 1;
            }
            visit(c)?;
        }
        for i in 0..table.indexes.len() {
            if table.indexes[i].unique {
                let mut c = m.clone();
                c.tables[t].indexes[i].unique = false;
                visit(c)?;
            }
            for k in 0..table.indexes[i].columns.len() {
                let mut c = m.clone();
                if c.tables[t].indexes[i].columns[k].1 {
                    c.tables[t].indexes[i].columns[k].1 = false;
                    visit(c.clone())?;
                }
                if c.tables[t].indexes[i].columns.len() > 1 {
                    c.tables[t].indexes[i].columns.remove(k);
                    visit(c)?;
                }
            }
        }
        for r in 0..table.rows.len() {
            let mut c = m.clone();
            c.tables[t].rows.remove(r);
            visit(c)?;
        }
    }
    ControlFlow::Continue(())
}

/// The model without table `t`: its triggers go, its views select from no
/// table, and later tables' references move down.
fn remove_table(m: &Model, t: usize) -> Model {
    let mut c = m.clone();
    c.tables.remove(t);
    c.triggers.retain(|g| g.table != t);
    for g in &mut c.triggers {
        if g.table > t {
            g.table -= 1;
        }
    }
    for v in &mut c.views {
        match v.table {
            Some(x) if x == t => {
                v.table = None;
                v.columns.clear();
            },
            Some(x) if x > t => v.table = Some(x - 1),
            _ => {},
        }
    }
    c
}

/// Table `t` without column `col`; indexes, views and triggers lose it,
/// and an index left with no column goes.
fn drop_column(m: &Model, t: usize, col: usize) -> Model {
    let mut c = m.clone();
    for v in &mut c.views {
        if v.table == Some(t) {
            v.columns.retain(|&x| x != col);
            for x in &mut v.columns {
                if *x > col {
                    *x -= 1;
                }
            }
        }
    }
    for g in &mut c.triggers {
        if g.table == t {
            g.column = match g.column {
                Some(x) if x == col => None,
                Some(x) => Some(x - usize::from(x > col)),
                None => None,
            };
        }
    }
    let table = &mut c.tables[t];
    table.columns -= 1;
    table.not_null.remove(col);
    table.decl.remove(col);
    table.coll.remove(col);
    table.default.remove(col);
    table.generated.remove(col);
    if let Some(types) = &mut table.strict {
        types.remove(col);
    }
    for r in &mut table.rows {
        r.values.remove(col);
    }
    for i in &mut table.indexes {
        i.columns.retain(|&(x, _)| x != col);
        for x in &mut i.columns {
            if x.0 > col {
                x.0 -= 1;
            }
        }
        i.coll.retain(|&(x, _)| x != col);
        for x in &mut i.coll {
            if x.0 > col {
                x.0 -= 1;
            }
        }
    }
    table.indexes.retain(|i| !i.columns.is_empty());
    table.key = match &table.key {
        Key::IntegerPrimaryKey(k) if *k == col => Key::Rowid,
        Key::IntegerPrimaryKey(k) => Key::IntegerPrimaryKey(k - usize::from(*k > col)),
        Key::WithoutRowid(pk) => {
            let pk: Vec<usize> = pk.iter().filter(|&&k| k != col).map(|&k| k - usize::from(k > col)).collect();
            if pk.is_empty() {
                Key::Rowid
            } else {
                Key::WithoutRowid(pk)
            }
        },
        Key::Rowid => Key::Rowid,
    };
    c
}

/// Layout candidates, each a step toward SQLite's defaults.
fn layout_candidates(l: &Layout, visit: Visit<Layout>) -> ControlFlow<()> {
    let mut steps: Vec<Layout> = Vec::new();
    let with = |f: &dyn Fn(&mut Layout)| {
        let mut c = *l;
        f(&mut c);
        c
    };
    steps.push(with(&|c| c.free_pages = 0));
    steps.push(with(&|c| c.free_pages /= 2));
    steps.push(with(&|c| c.flags = 0));
    for b in 0..7 {
        steps.push(with(&|c| c.flags &= !(1 << b)));
    }
    steps.push(with(&|c| c.leaf_fill = 100));
    steps.push(with(&|c| c.interior_fill = 100));
    steps.push(with(&|c| c.reserved = 0));
    steps.push(with(&|c| c.reserved /= 2));
    steps.push(with(&|c| c.page_size = 4096));
    steps.push(with(&|c| {
        c.page_size = if c.page_size > 4096 { c.page_size / 2 } else { (c.page_size * 2).min(4096) }
    }));
    steps.push(with(&|c| c.format = 4));
    steps.push(with(&|c| c.wal = false));
    steps.push(with(&|c| c.trunk_cap = 255));
    steps.push(with(&|c| c.seed = 0));
    steps.push(with(&|c| c.vacuum = 0));
    for s in steps {
        if s != *l {
            visit(s)?;
        }
    }
    ControlFlow::Continue(())
}

/// Simpler values: NULL, half an integer, an integer for a real, half a
/// text or blob.
fn value_candidates(m: &Model, visit: Visit<Model>) -> ControlFlow<()> {
    for (t, table) in m.tables.iter().enumerate() {
        for (r, row) in table.rows.iter().enumerate() {
            for (k, v) in row.values.iter().enumerate() {
                let simpler: Vec<Value> = match v {
                    Value::Null => vec![],
                    Value::Int(0) => vec![Value::Null],
                    Value::Int(i) => vec![Value::Null, Value::Int(i / 2)],
                    Value::Real(x) => vec![Value::Null, Value::Int(x.clamp(-1e18, 1e18) as i64)],
                    Value::Text(s) => {
                        let mut half = s.len() / 2;
                        while !s.is_char_boundary(half) {
                            half -= 1;
                        }
                        vec![Value::Null, Value::Text(s[..half].to_string())]
                    },
                    Value::Blob(b) => vec![Value::Null, Value::Blob(b[..b.len() / 2].to_vec())],
                };
                for s in simpler {
                    let mut c = m.clone();
                    c.tables[t].rows[r].values[k] = s;
                    visit(c)?;
                }
            }
        }
    }
    ControlFlow::Continue(())
}

// ---------------------------------------------------------------- mutation

fn mutate_layout(l: &mut Layout, g: &mut Source) {
    match g.int(0..9) {
        0 => l.seed = g.int(..),
        1 => l.flags ^= 1 << g.int(0..7),
        2 => l.page_size = *g.pick(&[512, 1024, 2048, 4096, 8192, 16384, 32768, 65536]),
        3 => l.reserved = g.int(0..=255),
        4 => l.leaf_fill = g.int(1..=100),
        5 => l.interior_fill = g.int(1..=100),
        6 => l.free_pages = g.int(0..=Layout::max_free(l.page_size).min(64)),
        7 => {
            l.format = g.int(1..=4);
            l.wal = g.bool_with(0.2);
            l.vacuum = g.int(0..=2);
        },
        _ => l.trunk_cap = g.int(1..=255),
    }
    *l = l.normalized();
}

fn mutate_model(m: &mut Model, l: &Layout, g: &mut Source) {
    if m.tables.is_empty() || g.bool_with(0.05) {
        if m.tables.len() < MAX_TABLES {
            m.tables.push(draw_table(g, l, &mut MUTATION_BYTES.clone(), (500, 0.2), &mut MUTATION_BYTES.clone()));
        }
        return;
    }
    if g.bool_with(0.05) {
        m.encoding = *g.pick(&[Encoding::Utf8, Encoding::Utf16le, Encoding::Utf16be]);
        return;
    }
    if g.bool_with(0.1) {
        if m.views.len() < MAX_VIEWS && g.bool() {
            let v = draw_view(g, m);
            m.views.push(v);
        } else if !m.views.is_empty() {
            m.views.remove(g.index(m.views.len()));
        }
        return;
    }
    if g.bool_with(0.1) {
        if m.triggers.len() < MAX_TRIGGERS && g.bool() {
            let t = draw_trigger(g, m);
            m.triggers.push(t);
        } else if !m.triggers.is_empty() {
            m.triggers.remove(g.index(m.triggers.len()));
        }
        return;
    }
    if g.bool_with(0.1) {
        // STRICT or not, a column's type, a NOT NULL. Values the change
        // makes invalid become NULL (or the constraint goes) on writing; a
        // declared type converts them as its affinity does.
        let t = g.index(m.tables.len());
        let table = &mut m.tables[t];
        match g.int(0..4) {
            0 => table.strict = if table.strict.is_some() { None } else { Some(draw_types(g, table)) },
            3 => {
                let k = g.index(table.columns);
                table.coll[k] = if table.coll[k].is_some() && g.bool() { None } else { Some(*g.pick(&model::Coll::ALL)) };
            },
            1 => {
                let k = g.index(table.columns);
                if let Some(types) = &mut table.strict {
                    types[k] = *g.pick(&ColType::ALL);
                } else {
                    table.decl[k] = if table.decl[k].is_some() && g.bool() { None } else { Some(g.index(model::DECLS.len())) };
                }
            },
            _ => {
                let k = g.index(table.columns);
                table.not_null[k] = !table.not_null[k];
            },
        }
        return;
    }
    let t = g.index(m.tables.len());
    let mut budget = MUTATION_BYTES;
    let table = &mut m.tables[t];
    match g.int(0..10) {
        // Insert a row at a free rowid.
        0 | 1 => {
            if table.rows.len() < MAX_ROWS {
                let rowid = match table.rows.last() {
                    Some(r) if r.rowid < i64::MAX && g.bool() => r.rowid + 1,
                    _ => g.int(..),
                };
                if let Err(at) = table.rows.binary_search_by_key(&rowid, |r| r.rowid) {
                    let kind = g.int(0u8..=6);
                    let values = (0..table.columns).map(|_| draw_value(g, kind, l, &mut budget)).collect();
                    table.rows.insert(at, Row { rowid, values });
                }
            }
        },
        2 => {
            if !table.rows.is_empty() {
                table.rows.remove(g.index(table.rows.len()));
            }
        },
        3 | 4 => {
            if !table.rows.is_empty() {
                let r = g.index(table.rows.len());
                let k = g.index(table.columns);
                let kind = g.int(0u8..=6);
                table.rows[r].values[k] = draw_value(g, kind, l, &mut budget);
            }
        },
        5 => {
            if table.columns < MAX_COLUMNS {
                table.columns += 1;
                table.not_null.push(false);
                table.decl.push(None);
                table.coll.push(None);
                table.default.push(None);
                table.generated.push(None);
                if let Some(types) = &mut table.strict {
                    types.push(*g.pick(&ColType::ALL));
                }
                let kind = g.int(0u8..=6);
                for r in &mut table.rows {
                    r.values.push(draw_value(g, kind, l, &mut budget));
                }
            }
        },
        6 => {
            if table.indexes.len() < MAX_INDEXES {
                let mut i = draw_index(g, table.columns);
                i.unique = g.bool_with(0.3);
                i.coll = draw_index_colls(g, &i);
                table.indexes.push(i);
            } else {
                table.indexes.remove(g.index(table.indexes.len()));
            }
        },
        7 => {
            if table.columns > 1 {
                *m = drop_column(m, t, g.index(m.tables[t].columns));
            }
        },
        8 => {
            let columns = table.columns;
            table.key = draw_key(g, columns);
            if let (Key::IntegerPrimaryKey(k), Some(types)) = (&table.key, &mut table.strict) {
                types[*k] = ColType::Integer;
            }
        },
        _ => {
            if m.tables.len() > 1 {
                *m = remove_table(m, t);
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{file_metrics, integrity_ok};
    use rusqlite::{Connection, OpenFlags};

    /// SQLite's own messages for a file, for diagnosing writer bugs.
    fn messages(bytes: &[u8]) -> Vec<String> {
        let path = std::env::temp_dir().join(format!("ephyra-sqlite-diag-{}-{}.db", std::process::id(), rand_id()));
        std::fs::write(&path, bytes).unwrap();
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
        let out = match Connection::open_with_flags(format!("file:{}?immutable=1", path.display()), flags) {
            Ok(db) => match db.prepare("PRAGMA integrity_check(20)") {
                Ok(mut s) => s
                    .query_map([], |r| r.get::<_, String>(0))
                    .map(|rows| rows.map(|r| r.unwrap_or_else(|e| e.to_string())).collect())
                    .unwrap_or_else(|e| vec![e.to_string()]),
                Err(e) => vec![e.to_string()],
            },
            Err(e) => vec![e.to_string()],
        };
        let _ = std::fs::remove_file(&path);
        out
    }

    fn rand_id() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn check(file: &SqliteFile, what: &str) {
        assert!(integrity_ok(file), "{what}: {:?}\n{:?}", messages(&file.0), reader::read(&file.0).map(|(_, l)| l));
    }

    #[test]
    fn generated_files_are_valid_and_read_back() {
        for seed in 0..400u64 {
            let mut g = Source::new(seed, (seed % 14) as usize);
            let layout = draw_layout(&mut g, false);
            let mut m = draw_model(&mut g, &layout, 0.2, false);
            let mut l = layout;
            model::normalize(&mut m, &mut l);
            let file = SqliteFile(writer::write(&m, &l));
            check(&file, &format!("seed {seed}"));
            let (m2, l2) = reader::read(&file.0).expect("reads back");
            assert_eq!((&m2, &l2), (&m, &l), "seed {seed}");
        }
    }

    #[test]
    fn packed_files_are_valid_full_and_read_back() {
        let mut best: f64 = 0.0;
        for seed in 0..200u64 {
            let file = draw_file(&mut Source::new(seed, 10), true, false);
            check(&file, &format!("packed seed {seed}"));
            let (m, l) = reader::read(&file.0).expect("reads back");
            assert_eq!(writer::write(&m, &l), file.0, "seed {seed}");
            let fill = file_metrics(&file, 10).into_iter().find(|(k, _)| *k == "fill").unwrap().1;
            best = best.max(fill);
        }
        assert!(best >= 0.95, "best fill {best}");
    }

    fn round_trip(mut m: Model, mut l: Layout, what: &str) -> Model {
        model::normalize(&mut m, &mut l);
        let file = SqliteFile(writer::write(&m, &l));
        check(&file, what);
        let (m2, l2) = reader::read(&file.0).expect("reads back");
        assert_eq!((&m2, &l2), (&m, &l), "{what}");
        m
    }

    #[test]
    fn edge_schemas_are_valid() {
        let small = Layout { page_size: 512, ..Layout::SIMPLE };
        // One schema row too big for page 1: a cell-less root over a leaf.
        let mut t = Table::new(8, Key::Rowid);
        t.strict = Some(vec![ColType::Integer; 8]);
        t.not_null = vec![true; 8];
        t.rows = (1..=3).map(|r| Row { rowid: r, values: (0..8).map(Value::Int).collect() }).collect();
        let m = Model { encoding: Encoding::Utf16be, tables: vec![t], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        assert!(writer::schema_slack(&m, &small).is_none());
        round_trip(m.clone(), small, "big table row");
        // A view row that overflows, alone.
        let v = View { table: None, columns: Vec::new(), pad: 512 };
        let lone = Model { encoding: Encoding::Utf16le, tables: Vec::new(), views: vec![v], triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        round_trip(lone, small, "big view row");
        // STRICT REAL: integral reals stored as integers, -0.0 as 0.
        let mut t = Table::new(2, Key::WithoutRowid(vec![0]));
        t.strict = Some(vec![ColType::Real, ColType::Text]);
        let reals = [-0.5, 0.0, 3.0, 140_737_488_355_327.0, 140_737_488_355_328.0, 1e300, f64::INFINITY];
        t.rows = reals.iter().enumerate().map(|(k, x)| Row { rowid: k as i64 + 1, values: vec![Value::Real(*x), Value::Text("a".into())] }).collect();
        t.indexes = vec![Index::of(vec![(1, false), (0, true)])];
        let mut u = Table::new(3, Key::IntegerPrimaryKey(1));
        u.strict = Some(vec![ColType::Real, ColType::Integer, ColType::Any]);
        u.rows = (1..=5).map(|r| Row { rowid: r, values: vec![Value::Real(-0.0), Value::Null, Value::Int(r)] }).collect();
        u.indexes = vec![Index::of(vec![(0, false)])];
        let views = vec![View { table: Some(1), columns: vec![2, 0], pad: 3 }, View { table: Some(0), columns: Vec::new(), pad: 0 }];
        let triggers = (0..3).map(|e| Trigger { table: e as usize % 2, after: e == 1, event: e, column: Some(0) }).collect();
        let m = Model { encoding: Encoding::Utf8, tables: vec![t, u], views, triggers, savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        let m = round_trip(m, Layout::SIMPLE, "strict reals");
        assert!(matches!(m.tables[1].rows[0].values[0], Value::Real(x) if x == 0.0 && x.is_sign_positive()));
    }

    #[test]
    fn row_counts_match_sqlite() {
        for seed in 0..60u64 {
            let mut g = Source::new(seed, 10);
            let file = Files.generate(&mut g);
            let Some((m, _)) = reader::read(&file.0) else {
                assert!(file.0.is_empty());
                continue;
            };
            let rows: usize = m.tables.iter().map(|t| t.rows.len()).sum();
            let metrics: std::collections::BTreeMap<_, _> = file_metrics(&file, 10).into_iter().collect();
            assert_eq!(metrics["rows"], rows as f64);
            assert_eq!(metrics["tables"], m.tables.len() as f64);
        }
    }

    #[test]
    fn every_feature_appears() {
        let mut seen: std::collections::BTreeMap<&str, f64> = Default::default();
        for seed in 0..200u64 {
            let file = Files.generate(&mut Source::new(seed, 10));
            check(&file, "generated");
            for (k, v) in file_metrics(&file, 10) {
                *seen.entry(k).or_default() += f64::from(u8::from(v > 0.0));
            }
            let Some((m, _)) = reader::read(&file.0) else {
                assert!(file.0.is_empty());
                continue;
            };
            let ipk = m.tables.iter().any(|t| matches!(t.key, Key::IntegerPrimaryKey(_)));
            *seen.entry("integer primary key").or_default() += f64::from(u8::from(ipk));
        }
        for k in [
            "without rowid tables",
            "integer primary key",
            "auto vacuum",
            "incremental vacuum",
            "utf16",
            "wal",
            "freelist pages",
            "overflow pages",
            "interior pages",
            "reserved bytes",
            "indexes",
            "strict tables",
            "views",
            "triggers",
        ] {
            assert!(seen[k] >= 5.0, "{k}: {seen:?}");
        }
    }

    #[test]
    fn shrink_and_mutate_stay_valid() {
        for seed in 0..40u64 {
            let mut g = Source::new(seed, 8);
            let file = Files.generate(&mut g);
            let Some((m, l)) = reader::read(&file.0) else {
                assert!(file.0.is_empty());
                continue;
            };
            let cost = m.cost() + l.cost();
            let candidates = Files.shrink_candidates(&file);
            assert!(candidates[0].0.is_empty(), "the empty file comes first");
            for c in candidates.into_iter().skip(1).take(40) {
                check(&c, "shrink");
                let (cm, cl) = reader::read(&c.0).unwrap();
                assert!(cm.cost() + cl.cost() < cost);
            }
            for _ in 0..4 {
                check(&Files.mutate(&file, &mut g), "mutate");
            }
        }
    }
}
