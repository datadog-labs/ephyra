// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Serialize a [`Model`] with a [`Layout`] into the bytes of a database file,
//! following SQLite's "Database File Format" document: the 100-byte header,
//! table and index B-trees (leaf and interior pages, cell pointer arrays,
//! free blocks, fragments), records, overflow chains, the freelist, and the
//! `sqlite_schema` table rooted on page 1.
//!
//! Writing is a pure function of the model and the layout: every random
//! layout decision comes from a [`Rng`] seeded by `layout.seed`.

use crate::model::{flag, index_entries, record_columns, stored, Encoding, Index, Key, Layout, Model, Table, Trigger, Value, View, DECLS};

/// SplitMix64: a small deterministic generator for layout decisions.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`; `n` > 0.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

pub fn varint_len(v: u64) -> usize {
    if v >> 56 != 0 {
        return 9;
    }
    let mut n = 1;
    let mut v = v >> 7;
    while v != 0 {
        n += 1;
        v >>= 7;
    }
    n
}

pub fn put_varint(out: &mut Vec<u8>, v: u64) {
    if v >> 56 != 0 {
        let hi = v >> 8;
        for i in (0..8).rev() {
            out.push((((hi >> (7 * i)) & 0x7f) as u8) | 0x80);
        }
        out.push(v as u8);
        return;
    }
    let n = varint_len(v);
    for i in (0..n).rev() {
        let b = ((v >> (7 * i)) & 0x7f) as u8;
        out.push(if i == 0 { b } else { b | 0x80 });
    }
}

/// Byte width of integer serial types 1..=6.
pub fn int_width(serial: u64) -> usize {
    [0, 1, 2, 3, 4, 6, 8][serial as usize]
}

/// The smallest integer serial type in 1..=6 holding `v`.
fn min_int_type(v: i64) -> u64 {
    match v {
        -0x80..=0x7f => 1,
        -0x8000..=0x7fff => 2,
        -0x80_0000..=0x7f_ffff => 3,
        -0x8000_0000..=0x7fff_ffff => 4,
        -0x8000_0000_0000..=0x7fff_ffff_ffff => 5,
        _ => 6,
    }
}

/// Integer encoding choices for one record.
#[derive(Clone, Copy)]
struct IntStyle {
    format4: bool,
    wide: bool,
}

impl IntStyle {
    fn serial(self, v: i64, rng: &mut Rng) -> u64 {
        let min = min_int_type(v);
        let constant = self.format4 && (v == 0 || v == 1);
        if self.wide && rng.chance(1, 2) {
            // Any wider type, or type 1 for 0 and 1 even when 8 and 9 exist.
            return min + rng.below(7 - min);
        }
        if constant {
            8 + v as u64
        } else {
            min
        }
    }
}

/// A record: header (its size, then one serial type per value) and body.
/// An integer in `fixed` positions is written with the given serial type.
fn record(values: &[Value], enc: Encoding, style: IntStyle, rng: &mut Rng, fixed: &[(usize, u64)]) -> Vec<u8> {
    let mut types = Vec::with_capacity(values.len());
    let mut body = Vec::new();
    for (k, v) in values.iter().enumerate() {
        match v {
            Value::Null => types.push(0),
            Value::Int(i) => {
                let t = fixed.iter().find(|(p, _)| *p == k).map_or_else(|| style.serial(*i, rng), |&(_, t)| t);
                types.push(t);
                if (1..=6).contains(&t) {
                    let w = int_width(t);
                    body.extend_from_slice(&i.to_be_bytes()[8 - w..]);
                }
            },
            Value::Real(r) => {
                types.push(7);
                body.extend_from_slice(&r.to_bits().to_be_bytes());
            },
            Value::Text(s) => {
                let b = enc.encode(s);
                types.push(13 + 2 * b.len() as u64);
                body.extend_from_slice(&b);
            },
            Value::Blob(b) => {
                types.push(12 + 2 * b.len() as u64);
                body.extend_from_slice(b);
            },
        }
    }
    let types_len: usize = types.iter().map(|&t| varint_len(t)).sum();
    let mut header_len = types_len + 1;
    while varint_len(header_len as u64) + types_len != header_len {
        header_len = varint_len(header_len as u64) + types_len;
    }
    let mut out = Vec::with_capacity(header_len + body.len());
    put_varint(&mut out, header_len as u64);
    for t in types {
        put_varint(&mut out, t);
    }
    out.extend_from_slice(&body);
    out
}

/// How much of a `p`-byte payload stays on a B-tree page (the spec's
/// formulas with U the usable size).
pub fn local_size(p: usize, u: usize, table_leaf: bool) -> usize {
    let x = if table_leaf { u - 35 } else { (u - 12) * 64 / 255 - 23 };
    if p <= x {
        return p;
    }
    let m = (u - 12) * 32 / 255 - 23;
    let k = m + (p - m) % (u - 4);
    if k <= x {
        k
    } else {
        m
    }
}

/// Page types (the first byte of a B-tree page header).
const INDEX_INTERIOR: u8 = 2;
const TABLE_INTERIOR: u8 = 5;
const INDEX_LEAF: u8 = 10;
const TABLE_LEAF: u8 = 13;

/// A cell: an optional left child, the bytes, an optional first overflow
/// page. Pages are referred to by index into the planner's page list until
/// they are numbered.
struct Cell {
    child: Option<usize>,
    body: Vec<u8>,
    overflow: Option<usize>,
}

impl Cell {
    fn size(&self) -> usize {
        4 * usize::from(self.child.is_some()) + self.body.len() + 4 * usize::from(self.overflow.is_some())
    }
}

struct Node {
    kind: u8,
    cells: Vec<Cell>,
    right: Option<usize>,
}

enum Page {
    Placeholder,
    Btree(Node),
    Overflow { next: Option<usize>, data: Vec<u8> },
    Trunk { next: Option<usize>, leaves: Vec<usize> },
    FreeLeaf,
}

struct Planner {
    layout: Layout,
    u: usize,
    pages: Vec<Page>,
}

/// Split items (with their space on a page) into runs. With `promote`,
/// one item between consecutive runs is left out of both (it goes up a
/// level), so every run is followed by a promoted item except the last.
/// Every run has at least one item and fits `cap`; a run stops growing at
/// `budget`. Returns the runs as ranges.
fn partition(sizes: &[usize], cap: usize, budget: usize, promote: bool) -> Vec<std::ops::Range<usize>> {
    let n = sizes.len();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < n {
        let start = i;
        let mut used = sizes[i];
        i += 1;
        while i < n && used + sizes[i] <= budget.min(cap) {
            used += sizes[i];
            i += 1;
        }
        if promote && n - i == 1 {
            // One item left: it cannot be promoted, as no run would follow.
            if used + sizes[i] <= cap {
                runs.push(start..n);
            } else {
                assert!(i - start >= 2, "two cells always fit an interior page");
                runs.push(start..i - 1);
                runs.push(i..n);
            }
            return runs;
        }
        runs.push(start..i);
        if promote {
            i += 1;
        }
    }
    runs
}

impl Planner {
    fn alloc(&mut self, page: Page) -> usize {
        self.pages.push(page);
        self.pages.len() - 1
    }

    fn place(&mut self, node: Node, slot: Option<usize>) -> usize {
        match slot {
            Some(s) => {
                self.pages[s] = Page::Btree(node);
                s
            },
            None => self.alloc(Page::Btree(node)),
        }
    }

    /// The overflow chain for `rest`, if any.
    fn chain(&mut self, rest: &[u8]) -> Option<usize> {
        if rest.is_empty() {
            return None;
        }
        let chunks: Vec<&[u8]> = rest.chunks(self.u - 4).collect();
        let first = self.pages.len();
        let n = chunks.len();
        for (k, c) in chunks.into_iter().enumerate() {
            let next = (k + 1 < n).then_some(first + k + 1);
            self.alloc(Page::Overflow { next, data: c.to_vec() });
        }
        Some(first)
    }

    fn payload_cell(&mut self, payload: &[u8], rowid: Option<i64>) -> Cell {
        let local = local_size(payload.len(), self.u, rowid.is_some());
        let mut body = Vec::with_capacity(local + 18);
        put_varint(&mut body, payload.len() as u64);
        if let Some(r) = rowid {
            put_varint(&mut body, r as u64);
        }
        body.extend_from_slice(&payload[..local]);
        let overflow = self.chain(&payload[local..]);
        Cell { child: None, body, overflow }
    }

    /// A table interior key between `lo` and `hi` inclusive.
    fn separator(&self, lo: i64, hi: i64, rng: &mut Rng) -> i64 {
        if !self.layout.has(flag::LOOSE) || lo == hi {
            return lo;
        }
        match rng.below(3) {
            0 => lo,
            1 => hi,
            _ => {
                let span = (i128::from(hi) - i128::from(lo)) as u64;
                let r = if span == u64::MAX { rng.next_u64() } else { rng.below(span + 1) };
                (i128::from(lo) + i128::from(r)) as i64
            },
        }
    }

    /// Build a B-tree bottom up from its leaf cells, in key order. For a
    /// table, `keys` are the cells' rowids. `extra` is header space taken
    /// on every page of the tree (100 for the tree rooted on page 1). The
    /// root goes to `slot` when given. Returns the root's page index.
    fn tree(&mut self, cells: Vec<Cell>, keys: Option<&[i64]>, extra: usize, slot: Option<usize>, rng: &mut Rng) -> usize {
        let table = keys.is_some();
        let (leaf_kind, interior_kind) = if table { (TABLE_LEAF, TABLE_INTERIOR) } else { (INDEX_LEAF, INDEX_INTERIOR) };
        // Every page has the whole usable size except the root, which
        // loses `extra`.
        let leaf_cap = self.u - 8;
        let interior_cap = self.u - 12;
        let leaf_budget = leaf_cap * usize::from(self.layout.leaf_fill) / 100;
        let interior_budget = interior_cap * usize::from(self.layout.interior_fill) / 100;

        let sizes: Vec<usize> = cells.iter().map(|c| c.size().max(4) + 2).collect();
        let mut runs = partition(&sizes, leaf_cap, leaf_budget, !table);
        if runs.len() <= 1 && sizes.iter().sum::<usize>() > leaf_cap - extra {
            if cells.len() == 1 {
                // One cell too big for page 1: a root with no cell over a
                // leaf, as SQLite's balance leaves it (only page 1 may be
                // an interior page without cells).
                assert!(table && extra > 0);
                let leaf = self.alloc(Page::Btree(Node { kind: leaf_kind, cells, right: None }));
                return self.place(Node { kind: interior_kind, cells: Vec::new(), right: Some(leaf) }, slot);
            }
            runs = partition(&sizes, leaf_cap - extra, leaf_budget, !table);
        }
        if runs.len() <= 1 {
            return self.place(Node { kind: leaf_kind, cells, right: None }, slot);
        }
        let mut cells: Vec<Option<Cell>> = cells.into_iter().map(Some).collect();
        let mut children = Vec::with_capacity(runs.len());
        let mut separators = Vec::with_capacity(runs.len() - 1);
        for (k, r) in runs.iter().enumerate() {
            let leaf_cells = r.clone().map(|i| cells[i].take().unwrap()).collect();
            children.push(self.alloc(Page::Btree(Node { kind: leaf_kind, cells: leaf_cells, right: None })));
            if k + 1 < runs.len() {
                match keys {
                    Some(keys) => {
                        let key = self.separator(keys[r.end - 1], keys[runs[k + 1].start] - 1, rng);
                        let mut body = Vec::new();
                        put_varint(&mut body, key as u64);
                        separators.push(Cell { child: None, body, overflow: None });
                    },
                    None => separators.push(cells[r.end].take().unwrap()),
                }
            }
        }
        loop {
            let sizes: Vec<usize> = separators.iter().map(|c| c.size() + 4 + 2).collect();
            let mut runs = partition(&sizes, interior_cap, interior_budget, true);
            if runs.len() == 1 && sizes.iter().sum::<usize>() > interior_cap - extra {
                assert!(sizes.len() >= 2, "a separator fits page 1");
                runs = partition(&sizes, interior_cap - extra, interior_budget, true);
            }
            let mut seps: Vec<Option<Cell>> = separators.into_iter().map(Some).collect();
            let mut nodes = Vec::with_capacity(runs.len());
            let mut promoted = Vec::new();
            for (k, r) in runs.iter().enumerate() {
                let node_cells = r
                    .clone()
                    .map(|i| {
                        let mut c = seps[i].take().unwrap();
                        c.child = Some(children[i]);
                        c
                    })
                    .collect();
                nodes.push(Node { kind: interior_kind, cells: node_cells, right: Some(children[r.end]) });
                if k + 1 < runs.len() {
                    promoted.push(seps[r.end].take().unwrap());
                }
            }
            if nodes.len() == 1 {
                return self.place(nodes.pop().unwrap(), slot);
            }
            children = nodes.into_iter().map(|n| self.alloc(Page::Btree(n))).collect();
            separators = promoted;
        }
    }

    fn style(&self) -> IntStyle {
        IntStyle { format4: self.layout.format >= 4, wide: self.layout.has(flag::WIDE) }
    }

    /// The table's B-tree; returns its root. A rowid table is a table
    /// B-tree keyed by rowid; a WITHOUT ROWID table is an index B-tree of
    /// records that start with the primary key.
    fn table(&mut self, enc: Encoding, table: &Table, rng: &mut Rng) -> usize {
        if let Key::WithoutRowid(_) = table.key {
            let order = record_columns(table);
            let entries: Vec<Vec<Value>> =
                table.rows.iter().map(|r| order.iter().map(|&c| stored(table, c, &r.values[c])).collect()).collect();
            return self.index(enc, &entries, rng);
        }
        let (cells, keys) = self.table_cells(enc, table, rng);
        self.tree(cells, Some(&keys), 0, None, rng)
    }

    /// A rowid table's leaf cells and their rowids.
    fn table_cells(&mut self, enc: Encoding, table: &Table, rng: &mut Rng) -> (Vec<Cell>, Vec<i64>) {
        let style = self.style();
        let short = self.layout.has(flag::SHORT) && self.layout.format >= 2;
        let mut cells = Vec::with_capacity(table.rows.len());
        let mut keys = Vec::with_capacity(table.rows.len());
        for row in &table.rows {
            let values: Vec<Value> = row.values.iter().enumerate().map(|(c, v)| stored(table, c, v)).collect();
            let mut n = values.len();
            if short {
                let nulls = values.iter().rev().take_while(|v| **v == Value::Null).count().min(n - 1);
                n -= rng.below(nulls as u64 + 1) as usize;
            }
            let payload = record(&values[..n], enc, style, rng, &[]);
            cells.push(self.payload_cell(&payload, Some(row.rowid)));
            keys.push(row.rowid);
        }
        (cells, keys)
    }

    /// An index B-tree over sorted entries; returns its root.
    fn index(&mut self, enc: Encoding, entries: &[Vec<Value>], rng: &mut Rng) -> usize {
        let style = self.style();
        let cells = entries
            .iter()
            .map(|e| {
                let payload = record(e, enc, style, rng, &[]);
                self.payload_cell(&payload, None)
            })
            .collect();
        self.tree(cells, None, 0, None, rng)
    }

    /// The sqlite_schema table into page index 0.
    fn schema(&mut self, model: &Model, roots: &[(u32, Vec<u32>)]) {
        let mut rng = Rng::new(u64::from(self.layout.seed) ^ 0x5c4e_3a00);
        let style = self.style();
        let enc = model.encoding;
        let mut rows: Vec<(i64, Vec<u8>)> = Vec::new();
        let mut rowid = 0i64;
        let mut push = |kind: &str, name: String, tbl: String, root: u32, sql: String, rng: &mut Rng| {
            rowid += 1;
            if self.layout.has(flag::LOOSE) {
                rowid += rng.below(3) as i64;
            }
            // The root page number has a fixed width, so both passes of
            // `write` lay the schema out alike whatever the numbers are.
            let width = if style.wide { 4 + rng.below(3) } else { 4 };
            let values = [Value::Text(kind.into()), Value::Text(name), Value::Text(tbl), Value::Int(root.into()), Value::Text(sql)];
            rows.push((rowid, record(&values, enc, style, rng, &[(3, width)])));
        };
        for (ti, t) in model.tables.iter().enumerate() {
            let name = format!("t{ti}");
            push("table", name.clone(), name.clone(), roots[ti].0, create_table_named(&name, t, Some(&name)), &mut rng);
            for (ii, idx) in t.indexes.iter().enumerate() {
                let iname = format!("{name}_i{ii}");
                let cols: Vec<String> = idx.columns.iter().map(|&(c, d)| index_term(idx, c, d)).collect();
                let unique = if idx.unique { "UNIQUE " } else { "" };
                let sql = format!("CREATE {unique}INDEX {iname} ON {name}({})", cols.join(", "));
                push("index", iname, name.clone(), roots[ti].1[ii], sql, &mut rng);
            }
        }
        for (k, v) in model.views.iter().enumerate() {
            let name = format!("v{k}");
            push("view", name.clone(), name.clone(), 0, create_view(&name, v), &mut rng);
        }
        for (k, g) in model.triggers.iter().enumerate() {
            let name = format!("g{k}");
            push("trigger", name.clone(), format!("t{}", g.table), 0, create_trigger(&name, g), &mut rng);
        }
        let mut cells = Vec::with_capacity(rows.len());
        let mut keys = Vec::with_capacity(rows.len());
        for (rowid, payload) in &rows {
            cells.push(self.payload_cell(payload, Some(*rowid)));
            keys.push(*rowid);
        }
        self.tree(cells, Some(&keys), 100, Some(0), &mut rng);
    }

    fn freelist(&mut self, count: u32, rng: &mut Rng) -> Option<usize> {
        let cap = usize::from(self.layout.trunk_cap).min(self.u / 4 - 2);
        let mut remaining = count as usize;
        let mut first = None;
        let mut prev: Option<usize> = None;
        while remaining > 0 {
            let t = self.alloc(Page::Trunk { next: None, leaves: Vec::new() });
            remaining -= 1;
            match prev {
                Some(p) => {
                    if let Page::Trunk { next, .. } = &mut self.pages[p] {
                        *next = Some(t);
                    }
                },
                None => first = Some(t),
            }
            let most = cap.min(remaining);
            let k = if self.layout.has(flag::LOOSE) { rng.below(most as u64 + 1) as usize } else { most };
            let leaves: Vec<usize> = (0..k).map(|_| self.alloc(Page::FreeLeaf)).collect();
            remaining -= k;
            if let Page::Trunk { leaves: l, .. } = &mut self.pages[t] {
                *l = leaves;
            }
            prev = Some(t);
        }
        first
    }
}

/// `CREATE TABLE t(c0, c1 INTEGER PRIMARY KEY, c2 NOT NULL, c3 VARCHAR(20),
/// …)`, or with `PRIMARY KEY(…)) WITHOUT ROWID`; a STRICT table declares
/// every column's type (`c0 REAL`) and ends with `STRICT`, another declares
/// the types it has ([`Table::decl`]); a column's `COLLATE` clause follows
/// its type (`c3 TEXT COLLATE NOCASE NOT NULL`).
pub fn create_table(name: &str, t: &Table) -> String {
    create_table_named(name, t, None)
}

/// `CONSTRAINT {kind}{at} ` half the time when `key` is given (by
/// [`crate::expr::spell`] of it), the name now and then quoted: a named
/// constraint, which SQLite takes as the unnamed one (and keeps in the
/// table's SQL, where `ALTER TABLE .. DROP CONSTRAINT`, `DROP NOT NULL`,
/// `SET NOT NULL` and `ADD CONSTRAINT` search the column definitions for
/// theirs). Never a `k{j}` (the checks' names, which ADD CONSTRAINT
/// gives and DROP CONSTRAINT drops).
pub fn constraint_name(key: Option<&str>, kind: &str, at: usize) -> String {
    let Some(key) = key else {
        return String::new();
    };
    let here = format!("{key}/{kind}{at}");
    if crate::expr::spell(&here, 2) != 0 {
        return String::new();
    }
    let name = format!("{kind}{at}");
    match crate::expr::spell(&format!("{here}/q"), 6) {
        0 => format!("CONSTRAINT \"{name}\" "),
        1 => format!("CONSTRAINT [{name}] "),
        2 => format!("CONSTRAINT `{name}` "),
        _ => format!("CONSTRAINT {name} "),
    }
}

/// [`create_table`], its constraints named now and then ([`constraint_name`]
/// keyed by `key`) when `key` is given.
pub fn create_table_named(name: &str, t: &Table, key: Option<&str>) -> String {
    let named = |kind: &str, at: usize| constraint_name(key, kind, at);
    let mut cols: Vec<String> = (0..t.columns)
        .map(|c| {
            let mut def = format!("c{c}");
            let ipk = matches!(t.key, Key::IntegerPrimaryKey(k) if k == c);
            match &t.strict {
                Some(types) => {
                    def.push(' ');
                    def.push_str(types[c].name());
                },
                None if ipk => def.push_str(" INTEGER"),
                None => {
                    if let Some(d) = t.decl[c] {
                        def.push(' ');
                        def.push_str(DECLS[d].0);
                    }
                },
            }
            if ipk {
                def.push(' ');
                def.push_str(&named("pk", c));
                def.push_str("PRIMARY KEY");
                def.push_str(&key_conflict(t));
            }
            if let Some(k) = t.coll[c] {
                def.push(' ');
                def.push_str(&named("co", c));
                def.push_str("COLLATE ");
                def.push_str(k.name());
            }
            if t.not_null[c] {
                def.push(' ');
                def.push_str(&named("nn", c));
                def.push_str("NOT NULL");
            }
            // A foreign key of one column, as a column constraint.
            for f in t.fks.iter().filter(|f| f.columns == [c]) {
                def.push_str(&format!(" {}REFERENCES t{}(c{}){}", named("fk", c), f.parent, f.parent_columns[0], fk_tail(f)));
            }
            def
        })
        .collect();
    for (j, f) in t.fks.iter().enumerate().filter(|(_, f)| f.columns.len() > 1) {
        let names = |cs: &[usize]| cs.iter().map(|c| format!("c{c}")).collect::<Vec<_>>().join(", ");
        cols.push(format!("{}FOREIGN KEY({}) REFERENCES t{}({}){}", named("fks", j), names(&f.columns), f.parent, names(&f.parent_columns), fk_tail(f)));
    }
    let mut tail = String::new();
    if let Key::WithoutRowid(pk) = &t.key {
        let pk: Vec<String> = pk.iter().map(|c| format!("c{c}")).collect();
        cols.push(format!("{}PRIMARY KEY({}){}", named("pk", t.columns), pk.join(", "), key_conflict(t)));
        tail.push_str(" WITHOUT ROWID");
    }
    if t.strict.is_some() {
        tail.push_str(if tail.is_empty() { " STRICT" } else { ", STRICT" });
    }
    format!("CREATE TABLE {name}({}){tail}", cols.join(", "))
}

/// ` ON CONFLICT x` of the table's primary key, if any.
fn key_conflict(t: &Table) -> String {
    t.key_conflict.map_or(String::new(), |r| format!(" ON CONFLICT {}", r.sql()))
}

/// A foreign key's actions and deferral, as written after its
/// `REFERENCES` clause (NO ACTION, the default, left out).
pub fn fk_tail(f: &crate::model::ForeignKey) -> String {
    let mut out = String::new();
    for (on, a) in [("DELETE", f.on_delete), ("UPDATE", f.on_update)] {
        if a != crate::model::Action::NoAction {
            out.push_str(&format!(" ON {on} {}", a.sql()));
        }
    }
    if f.deferred {
        out.push_str(" DEFERRABLE INITIALLY DEFERRED");
    }
    out
}

/// An index's term for column `c`: `c1`, `c1 COLLATE NOCASE`, `c1 DESC`,
/// `c1 COLLATE RTRIM DESC`.
pub fn index_term(i: &Index, c: usize, desc: bool) -> String {
    let coll = i.coll.iter().find(|(x, _)| *x == c).map_or(String::new(), |(_, k)| format!(" COLLATE {}", k.name()));
    format!("c{c}{coll}{}", if desc { " DESC" } else { "" })
}

/// `CREATE VIEW v AS SELECT c0, 'xx' FROM t0` (see [`View`]).
pub fn create_view(name: &str, v: &View) -> String {
    let mut items: Vec<String> = v.columns.iter().map(|c| format!("c{c}")).collect();
    if v.pad > 0 {
        items.push(format!("'{}'", "x".repeat(v.pad)));
    }
    if items.is_empty() {
        items.push("1".into());
    }
    let from = v.table.map_or(String::new(), |t| format!(" FROM t{t}"));
    format!("CREATE VIEW {name} AS SELECT {}{from}", items.join(", "))
}

/// `CREATE TRIGGER g AFTER INSERT ON t0 BEGIN SELECT NEW.c0; END` (see
/// [`Trigger`]).
pub fn create_trigger(name: &str, g: &Trigger) -> String {
    let timing = if g.after { "AFTER" } else { "BEFORE" };
    let event = ["INSERT", "UPDATE", "DELETE"][usize::from(g.event)];
    let what = match g.column {
        Some(c) => format!("{}.c{c}", if g.event == 2 { "OLD" } else { "NEW" }),
        None => "1".into(),
    };
    format!("CREATE TRIGGER {name} {timing} {event} ON t{} BEGIN SELECT {what}; END", g.table)
}

/// For packing table `t` of `model`: how many of its first rows to keep so
/// that its last leaf is as full as it gets, laid out as `write` lays it
/// out (the tables before it, and their indexes, take the seeded
/// generator's first draws). Candidates are the ends of its leaves from
/// half the rows on; all rows when the table is one leaf.
pub fn packed_rows(model: &Model, layout: &Layout, t: usize) -> usize {
    let mut p = Planner { layout: *layout, u: layout.usable(), pages: vec![Page::Placeholder] };
    let mut rng = Rng::new(u64::from(layout.seed));
    let enc = model.encoding;
    for prev in &model.tables[..t] {
        p.table(enc, prev, &mut rng);
        for i in &prev.indexes {
            p.index(enc, &index_entries(prev, i, enc), &mut rng);
        }
    }
    let table = &model.tables[t];
    let style = p.style();
    let (sizes, promote): (Vec<usize>, bool) = match table.key {
        Key::WithoutRowid(_) => {
            let order = record_columns(table);
            let sizes = table
                .rows
                .iter()
                .map(|r| {
                    let e: Vec<Value> = order.iter().map(|&c| stored(table, c, &r.values[c])).collect();
                    let payload = record(&e, enc, style, &mut rng, &[]);
                    p.payload_cell(&payload, None).size().max(4) + 2
                })
                .collect();
            (sizes, true)
        },
        _ => {
            let (cells, _) = p.table_cells(enc, table, &mut rng);
            (cells.iter().map(|c| c.size().max(4) + 2).collect(), false)
        },
    };
    let cap = p.u - 8;
    let budget = cap * usize::from(layout.leaf_fill) / 100;
    let runs = partition(&sizes, cap, budget, promote);
    if runs.len() <= 1 && table.indexes.is_empty() {
        return sizes.len();
    }
    // The index trees, estimated: each row's entry size (drawn with a
    // throwaway generator) and the rows in index order.
    let probe = match table.key {
        Key::WithoutRowid(_) => Table { key: Key::Rowid, ..table.clone() },
        _ => table.clone(),
    };
    let mut scratch = Rng::new(0);
    let indexes: Vec<Vec<(usize, usize)>> = probe
        .indexes
        .iter()
        .map(|i| {
            index_entries(&probe, i, enc)
                .iter()
                .map(|e| {
                    let Some(Value::Int(rowid)) = e.last() else { unreachable!("a rowid ends the entry") };
                    let pos = probe.rows.binary_search_by_key(rowid, |r| r.rowid).unwrap_or(0);
                    let payload = record(e, enc, style, &mut scratch, &[]);
                    (pos, p.payload_cell(&payload, None).size().max(4) + 2)
                })
                .collect()
        })
        .collect();
    // Unused leaf bytes and leaf bytes of a tree over `sizes`.
    let slack = |sizes: &[usize], promote: bool| -> (usize, usize) {
        let runs = partition(sizes, cap, budget, promote);
        let used: usize = runs.iter().map(|r| sizes[r.clone()].iter().sum::<usize>()).sum();
        (runs.len().max(1) * cap - used, runs.len().max(1) * cap)
    };
    let half = (sizes.len() / 2).max(1);
    let mut candidates: Vec<usize> = runs.iter().map(|r| r.end).filter(|&e| e >= half).collect();
    if candidates.is_empty() || runs.len() <= 1 {
        candidates = (half..=sizes.len()).rev().step_by((sizes.len() / 40).max(1)).collect();
    }
    let mut best = (f64::INFINITY, sizes.len());
    for k in candidates {
        let (mut free, mut all) = slack(&sizes[..k], promote);
        for idx in &indexes {
            let kept: Vec<usize> = idx.iter().filter(|(pos, _)| *pos < k).map(|(_, s)| *s).collect();
            let (f, a) = slack(&kept, true);
            free += f;
            all += a;
        }
        let ratio = free as f64 / all as f64;
        if ratio < best.0 {
            best = (ratio, k);
        }
    }
    best.1
}

/// The free bytes on page 1 when the whole schema fits there, else `None`.
pub fn schema_slack(model: &Model, layout: &Layout) -> Option<usize> {
    let mut p = Planner { layout: *layout, u: layout.usable(), pages: vec![Page::Placeholder] };
    let zeros: Vec<(u32, Vec<u32>)> = model.tables.iter().map(|t| (0, vec![0; t.indexes.len()])).collect();
    p.schema(model, &zeros);
    match &p.pages[0] {
        Page::Btree(node) if node.right.is_none() => {
            let used: usize = node.cells.iter().map(|c| c.size().max(4) + 2).sum();
            Some(p.u - 108 - used)
        },
        _ => None,
    }
}

fn put16(b: &mut [u8], at: usize, v: usize) {
    b[at..at + 2].copy_from_slice(&(v as u16).to_be_bytes());
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

/// Fill `b` with random bytes: windows of `block` (8 KiB drawn once per
/// file) at random offsets, which is fast even unoptimized.
fn junk(b: &mut [u8], block: &[u8], rng: &mut Rng) {
    for chunk in b.chunks_mut(block.len() / 2) {
        let at = rng.below(block.len() as u64 / 2) as usize;
        chunk.copy_from_slice(&block[at..at + chunk.len()]);
    }
}

/// Write a B-tree page: header at `off`, cell pointers in key order, cell
/// content from the end of the usable area down, possibly shuffled and
/// with free blocks and fragments between cells (never before the first,
/// never two free blocks without a cell between them).
fn write_node(page: &mut [u8], off: usize, node: &Node, num: &[u32], layout: &Layout, rng: &mut Rng) {
    let u = layout.usable();
    let leaf = node.right.is_none();
    let n = node.cells.len();
    let ptrs = off + if leaf { 8 } else { 12 };
    let bytes: Vec<Vec<u8>> = node
        .cells
        .iter()
        .map(|c| {
            let mut b = Vec::with_capacity(c.size());
            if let Some(ch) = c.child {
                b.extend_from_slice(&num[ch].to_be_bytes());
            }
            b.extend_from_slice(&c.body);
            if let Some(o) = c.overflow {
                b.extend_from_slice(&num[o].to_be_bytes());
            }
            // SQLite counts a cell as at least 4 bytes (a one-column record
            // of 0 or 1 makes a 3-byte index cell).
            b.resize(b.len().max(4), 0);
            b
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    if layout.has(flag::SHUFFLE) {
        for i in (1..n).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
    }
    let used: usize = bytes.iter().map(Vec::len).sum();
    let mut slack = u - ptrs - 2 * n - used;
    let mut frag = 0usize;
    // (cell index, gap after it: 0 none, 1..=3 fragment, >= 4 free block)
    let mut items = Vec::with_capacity(n);
    for &c in &order {
        let mut gap = 0;
        if layout.has(flag::GAPS) && rng.chance(1, 3) {
            if slack >= 4 && rng.chance(1, 2) {
                gap = 4 + rng.below((slack - 4).min(80) as u64 + 1) as usize;
            } else if slack >= 1 && frag < 60 {
                gap = 1 + rng.below(slack.min(3).min(60 - frag) as u64) as usize;
                frag += gap;
            }
        }
        slack -= gap;
        items.push((c, gap));
    }
    let content: usize = used + items.iter().map(|&(_, g)| g).sum::<usize>();
    let start = u - content;
    let mut at = start;
    let mut offsets = vec![0; n];
    let mut freeblocks = Vec::new();
    for &(c, gap) in &items {
        offsets[c] = at;
        page[at..at + bytes[c].len()].copy_from_slice(&bytes[c]);
        at += bytes[c].len();
        if gap >= 4 {
            freeblocks.push((at, gap));
        }
        at += gap;
    }
    for (k, &(pos, size)) in freeblocks.iter().enumerate() {
        put16(page, pos, freeblocks.get(k + 1).map_or(0, |f| f.0));
        put16(page, pos + 2, size);
    }
    page[off] = node.kind;
    put16(page, off + 1, freeblocks.first().map_or(0, |f| f.0));
    put16(page, off + 3, n);
    put16(page, off + 5, if start == 65536 { 0 } else { start });
    page[off + 7] = frag as u8;
    if let Some(r) = node.right {
        put32(page, off + 8, num[r]);
    }
    for (k, &o) in offsets.iter().enumerate() {
        put16(page, ptrs + 2 * k, o);
    }
}

/// The bytes of the database file for `model` laid out by `layout`. Both
/// must be normalized (see [`crate::model::normalize`]) and the model well
/// formed.
pub fn write(model: &Model, layout: &Layout) -> Vec<u8> {
    debug_assert!(model.well_formed());
    let layout = *layout;
    let mut p = Planner { layout, u: layout.usable(), pages: vec![Page::Placeholder] };
    let mut rng = Rng::new(u64::from(layout.seed));
    let enc = model.encoding;
    let mut roots = Vec::with_capacity(model.tables.len());
    for t in &model.tables {
        let root = p.table(enc, t, &mut rng);
        let idx = t.indexes.iter().map(|i| p.index(enc, &index_entries(t, i, enc), &mut rng)).collect::<Vec<_>>();
        roots.push((root, idx));
    }
    // Two passes over the schema: the first, with placeholder root numbers,
    // counts its pages; the numbering then fixes the real ones, and as root
    // numbers have a fixed width the second pass lays out the same pages.
    let mark = p.pages.len();
    let zeros: Vec<(u32, Vec<u32>)> = roots.iter().map(|(_, i)| (0, vec![0; i.len()])).collect();
    p.schema(model, &zeros);
    let planned = p.pages.len() + layout.free_pages as usize;
    // Page numbers: 1 for the schema root, then the others in allocation
    // order or scattered, skipping pointer-map pages under auto-vacuum.
    let per_map = layout.usable() / 5 + 1;
    let is_map = |pgno: usize| layout.vacuum != 0 && pgno >= 2 && (pgno - 2).is_multiple_of(per_map);
    let mut slots: Vec<u32> = Vec::with_capacity(planned + planned / per_map + 2);
    let mut pgno = 1;
    while slots.len() < planned {
        if !is_map(pgno) {
            slots.push(pgno as u32);
        }
        pgno += 1;
    }
    let total = pgno - 1;
    let mut nrng = Rng::new(u64::from(layout.seed) ^ 0x6e75_6d00);
    let num = if layout.vacuum == 0 {
        if layout.has(flag::SCATTER) {
            for i in (2..planned).rev() {
                slots.swap(i, 1 + nrng.below(i as u64) as usize);
            }
        }
        slots
    } else {
        // Under auto-vacuum every root page comes before every other page
        // (as SQLite's CREATE keeps it): committing a change moves pages
        // from the end of the file into free ones, and a root page in the
        // way makes SQLite report the file corrupt. Scattering permutes the
        // roots among themselves and the other pages among themselves.
        let mut rootset: Vec<usize> = roots.iter().flat_map(|(r, i)| std::iter::once(*r).chain(i.iter().copied())).collect();
        let mut others: Vec<usize> = (1..planned).filter(|i| !rootset.contains(i)).collect();
        if layout.has(flag::SCATTER) {
            for group in [&mut rootset, &mut others] {
                for i in (1..group.len()).rev() {
                    group.swap(i, nrng.below(i as u64 + 1) as usize);
                }
            }
        }
        let mut num = vec![0u32; planned];
        for (k, idx) in std::iter::once(0).chain(rootset).chain(others).enumerate() {
            num[idx] = slots[k];
        }
        num
    };
    p.pages.truncate(mark);
    p.pages[0] = Page::Placeholder;
    let real: Vec<(u32, Vec<u32>)> =
        roots.iter().map(|(r, i)| (num[*r], i.iter().map(|&x| num[x]).collect())).collect();
    p.schema(model, &real);
    let mut frng = Rng::new(u64::from(layout.seed) ^ 0x6672_6500);
    let first_trunk = p.freelist(layout.free_pages, &mut frng);
    assert_eq!(p.pages.len(), planned);

    let ps = layout.page_size as usize;
    let u = layout.usable();
    let mut out = vec![0u8; total * ps];
    let mut block = vec![0u8; if layout.has(flag::JUNK) { 8192 } else { 0 }];
    let mut brng = Rng::new(u64::from(layout.seed) ^ 0x6a75_6e6b);
    for chunk in block.chunks_mut(8) {
        chunk.copy_from_slice(&brng.next_u64().to_le_bytes());
    }
    for (idx, page) in p.pages.iter().enumerate() {
        let pgno = num[idx] as usize;
        let buf = &mut out[(pgno - 1) * ps..pgno * ps];
        let mut prng = Rng::new(u64::from(layout.seed) ^ ((idx as u64) << 32) ^ 0x7061_6765);
        if layout.has(flag::JUNK) {
            junk(buf, &block, &mut prng);
        }
        match page {
            Page::Placeholder => unreachable!("every page is planned"),
            Page::Btree(node) => write_node(buf, if pgno == 1 { 100 } else { 0 }, node, &num, &layout, &mut prng),
            Page::Overflow { next, data } => {
                put32(buf, 0, next.map_or(0, |n| num[n]));
                buf[4..4 + data.len()].copy_from_slice(data);
                debug_assert!(4 + data.len() <= u);
            },
            Page::Trunk { next, leaves } => {
                put32(buf, 0, next.map_or(0, |n| num[n]));
                put32(buf, 4, leaves.len() as u32);
                for (k, &l) in leaves.iter().enumerate() {
                    put32(buf, 8 + 4 * k, num[l]);
                }
            },
            Page::FreeLeaf => {},
        }
    }
    if layout.vacuum != 0 {
        // Pointer-map entries: each page's type and parent.
        let mut entry = |child: u32, kind: u8, parent: u32| {
            let child = child as usize;
            let map = (child - 2) / per_map * per_map + 2;
            let at = (map - 1) * ps + 5 * (child - map - 1);
            out[at] = kind;
            put32(&mut out, at + 1, parent);
        };
        for (idx, page) in p.pages.iter().enumerate() {
            let me = num[idx];
            match page {
                Page::Btree(node) => {
                    for c in &node.cells {
                        if let Some(ch) = c.child {
                            entry(num[ch], 5, me);
                        }
                        if let Some(o) = c.overflow {
                            entry(num[o], 3, me);
                        }
                    }
                    if let Some(r) = node.right {
                        entry(num[r], 5, me);
                    }
                },
                Page::Overflow { next: Some(n), .. } => entry(num[*n], 4, me),
                Page::Trunk { .. } | Page::FreeLeaf => entry(me, 2, 0),
                _ => {},
            }
        }
        for (r, idx) in &roots {
            for &x in std::iter::once(r).chain(idx) {
                entry(num[x], 1, 0);
            }
        }
    }
    let h = &mut out[..100];
    h[..16].copy_from_slice(b"SQLite format 3\0");
    put16(h, 16, if ps == 65536 { 1 } else { ps });
    let version = if layout.wal { 2 } else { 1 };
    h[18] = version;
    h[19] = version;
    h[20] = layout.reserved;
    h[21] = 64;
    h[22] = 32;
    h[23] = 32;
    let counter = 1 + layout.seed % 1000;
    put32(h, 24, counter);
    put32(h, 28, total as u32);
    put32(h, 32, first_trunk.map_or(0, |t| num[t]));
    put32(h, 36, layout.free_pages);
    put32(h, 40, 1 + (layout.seed >> 8) % 1000);
    put32(h, 44, layout.format.into());
    h[48..52].fill(0);
    let largest_root = roots.iter().flat_map(|(r, i)| std::iter::once(num[*r]).chain(i.iter().map(|&x| num[x]))).max().unwrap_or(1);
    put32(h, 52, if layout.vacuum != 0 { largest_root } else { 0 });
    put32(h, 56, enc.code());
    let user = u32::from(layout.leaf_fill)
        | u32::from(layout.interior_fill) << 8
        | u32::from(layout.flags) << 16
        | u32::from(layout.trunk_cap) << 24;
    put32(h, 60, user);
    put32(h, 64, u32::from(layout.vacuum == 2));
    put32(h, 68, layout.seed);
    h[72..92].fill(0);
    put32(h, 92, counter);
    put32(h, 96, 3_050_002);
    out
}
