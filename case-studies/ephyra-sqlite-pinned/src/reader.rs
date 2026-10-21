// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! Read a file written by `crate::writer` back into its model and layout,
//! without SQLite: the header, the `sqlite_schema` B-tree, each table's
//! B-tree with its overflow chains, and the records. Index B-trees are not
//! read, since a model's indexes are determined by its tables. The layout
//! fields the file has no place for (fills, flags, trunk capacity, seed) are
//! in the header's user version and application id, which the writer
//! reserves for them.
//!
//! Returns `None` for anything else, never panicking.

use crate::model::{affinity, record_columns, Affinity, ColType, Coll, Encoding, Index, Key, Layout, Model, Row, Table, Trigger, Value, View, DECLS};
use crate::writer::{int_width, local_size};

fn be16(b: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?)))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A varint at `at`: its value and length.
fn varint(b: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for i in 0..9 {
        let x = *b.get(at + i)?;
        if i == 8 {
            return Some(((v << 8) | u64::from(x), 9));
        }
        v = (v << 7) | u64::from(x & 0x7f);
        if x & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

struct File<'a> {
    bytes: &'a [u8],
    page_size: usize,
    usable: usize,
    pages: usize,
}

impl File<'_> {
    fn page(&self, pgno: u32) -> Option<&[u8]> {
        let p = pgno as usize;
        if p == 0 || p > self.pages {
            return None;
        }
        self.bytes.get((p - 1) * self.page_size..p * self.page_size)
    }

    /// A cell's whole payload: the local part, then the overflow chain.
    fn payload(&self, page: &[u8], at: usize, len: usize, table_leaf: bool) -> Option<Vec<u8>> {
        if len > 1 << 30 {
            return None;
        }
        let local = local_size(len, self.usable, table_leaf);
        let mut out = page.get(at..at + local)?.to_vec();
        let mut next = if local < len { be32(page, at + local)? } else { 0 };
        let mut hops = 0;
        while out.len() < len {
            let ov = self.page(next)?;
            hops += 1;
            if hops > self.pages {
                return None;
            }
            let take = (len - out.len()).min(self.usable - 4);
            out.extend_from_slice(ov.get(4..4 + take)?);
            next = be32(ov, 0)?;
        }
        Some(out)
    }

    /// A table B-tree's rows in order, as (rowid, payload).
    fn table(&self, pgno: u32, depth: usize, out: &mut Vec<(i64, Vec<u8>)>) -> Option<()> {
        if depth > 40 {
            return None;
        }
        let page = self.page(pgno)?;
        let off = if pgno == 1 { 100 } else { 0 };
        let kind = *page.get(off)?;
        let n = be16(page, off + 3)?;
        match kind {
            13 => {
                for k in 0..n {
                    let mut at = be16(page, off + 8 + 2 * k)?;
                    let (len, l1) = varint(page, at)?;
                    let (rowid, l2) = varint(page, at + l1)?;
                    at += l1 + l2;
                    out.push((rowid as i64, self.payload(page, at, len as usize, true)?));
                }
            },
            5 => {
                for k in 0..n {
                    let at = be16(page, off + 12 + 2 * k)?;
                    self.table(be32(page, at)?, depth + 1, out)?;
                }
                self.table(be32(page, off + 8)?, depth + 1, out)?;
            },
            _ => return None,
        }
        Some(())
    }
}

impl File<'_> {
    /// An index B-tree's payloads in order.
    fn index(&self, pgno: u32, depth: usize, out: &mut Vec<Vec<u8>>) -> Option<()> {
        if depth > 40 {
            return None;
        }
        let page = self.page(pgno)?;
        let kind = *page.first()?;
        let n = be16(page, 3)?;
        let leaf = match kind {
            10 => true,
            2 => false,
            _ => return None,
        };
        for k in 0..n {
            let mut at = be16(page, if leaf { 8 } else { 12 } + 2 * k)?;
            if !leaf {
                self.index(be32(page, at)?, depth + 1, out)?;
                at += 4;
            }
            let (len, l) = varint(page, at)?;
            out.push(self.payload(page, at + l, len as usize, false)?);
        }
        if !leaf {
            self.index(be32(page, 8)?, depth + 1, out)?;
        }
        Some(())
    }
}

fn decode_record(b: &[u8], enc: Encoding) -> Option<Vec<Value>> {
    let (header_len, mut at) = varint(b, 0)?;
    let header_len = usize::try_from(header_len).ok()?;
    let mut body = header_len;
    let mut values = Vec::new();
    while at < header_len {
        let (t, l) = varint(b, at)?;
        at += l;
        let v = match t {
            0 => Value::Null,
            1..=6 => {
                let w = int_width(t);
                let raw = b.get(body..body + w)?;
                body += w;
                let mut x: i64 = if raw[0] & 0x80 != 0 { -1 } else { 0 };
                for &byte in raw {
                    x = (x << 8) | i64::from(byte);
                }
                Value::Int(x)
            },
            7 => {
                let raw = b.get(body..body + 8)?;
                body += 8;
                Value::Real(f64::from_bits(u64::from_be_bytes(raw.try_into().ok()?)))
            },
            8 => Value::Int(0),
            9 => Value::Int(1),
            10 | 11 => return None,
            _ => {
                let n = usize::try_from((t - 12) / 2).ok()?;
                let raw = b.get(body..body + n)?;
                body += n;
                if t % 2 == 0 {
                    Value::Blob(raw.to_vec())
                } else {
                    Value::Text(enc.decode(raw)?)
                }
            },
        };
        values.push(v);
    }
    (body == b.len()).then_some(values)
}

/// `name(a, b DESC, …)` → (`name`, `a, b DESC, …`).
fn call(s: &str) -> Option<(&str, &str)> {
    let open = s.find('(')?;
    Some((s[..open].trim(), s[open + 1..].strip_suffix(')')?))
}

fn list(s: &str) -> Vec<&str> {
    s.split(',').map(str::trim).collect()
}

fn column(s: &str) -> Option<usize> {
    s.strip_prefix('c')?.parse().ok()
}

/// `t{k}` → k.
fn table_ref(s: &str) -> Option<usize> {
    s.strip_prefix('t')?.parse().ok()
}

/// `CREATE VIEW v AS SELECT …` → the view, given its expected name.
/// `sql` without its `CONSTRAINT name ` clauses.
fn strip_constraint_names(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(at) = rest.find("CONSTRAINT ") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "CONSTRAINT ".len()..];
        rest = match after.find(' ') {
            Some(sp) => &after[sp + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn parse_view(sql: &str, name: &str) -> Option<View> {
    let (head, body) = sql.strip_prefix("CREATE VIEW ")?.split_once(" AS SELECT ")?;
    if head != name {
        return None;
    }
    let (items, table) = match body.rsplit_once(" FROM ") {
        Some((items, t)) => (items, Some(table_ref(t)?)),
        None => (body, None),
    };
    let mut view = View { table, columns: Vec::new(), pad: 0 };
    if items != "1" {
        for item in list(items) {
            match item.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')) {
                Some(x) if !x.is_empty() && x.bytes().all(|b| b == b'x') && view.pad == 0 => view.pad = x.len(),
                Some(_) => return None,
                None if view.pad == 0 => view.columns.push(column(item)?),
                None => return None,
            }
        }
    }
    Some(view)
}

/// `CREATE TRIGGER g AFTER INSERT ON t0 BEGIN SELECT NEW.c0; END`.
fn parse_trigger(sql: &str, name: &str) -> Option<Trigger> {
    let words: Vec<&str> = sql.strip_prefix("CREATE TRIGGER ")?.strip_suffix("; END")?.split(' ').collect();
    let [head, timing, event, "ON", table, "BEGIN", "SELECT", what] = words[..] else {
        return None;
    };
    if head != name {
        return None;
    }
    let after = match timing {
        "AFTER" => true,
        "BEFORE" => false,
        _ => return None,
    };
    let event = ["INSERT", "UPDATE", "DELETE"].iter().position(|e| *e == event)? as u8;
    let column = match what {
        "1" => None,
        w => {
            let (row, c) = w.split_once('.')?;
            if row != if event == 2 { "OLD" } else { "NEW" } {
                return None;
            }
            Some(column(c)?)
        },
    };
    Some(Trigger { table: table_ref(table)?, after, event, column })
}

pub fn read(bytes: &[u8]) -> Option<(Model, Layout)> {
    if bytes.len() < 512 || &bytes[..16] != b"SQLite format 3\0" {
        return None;
    }
    let raw_size = be16(bytes, 16)?;
    let page_size = if raw_size == 1 { 65536 } else { raw_size };
    let user = be32(bytes, 60)?;
    let layout = Layout {
        page_size: page_size as u32,
        reserved: bytes[20],
        format: u8::try_from(be32(bytes, 44)?).ok()?,
        wal: bytes[18] == 2,
        leaf_fill: user as u8,
        interior_fill: (user >> 8) as u8,
        flags: (user >> 16) as u8,
        trunk_cap: (user >> 24) as u8,
        free_pages: be32(bytes, 36)?,
        seed: be32(bytes, 68)?,
        vacuum: match (be32(bytes, 52)?, be32(bytes, 64)?) {
            (0, 0) => 0,
            (0, _) => return None,
            (_, 0) => 1,
            _ => 2,
        },
    };
    if layout.normalized() != layout {
        return None;
    }
    let encoding = Encoding::from_code(be32(bytes, 56)?)?;
    let file = File {
        bytes,
        page_size,
        usable: layout.usable(),
        pages: usize::try_from(be32(bytes, 28)?).ok()?.min(bytes.len() / page_size),
    };
    let mut schema = Vec::new();
    file.table(1, 0, &mut schema)?;
    let mut tables: Vec<Table> = Vec::new();
    let mut views = Vec::new();
    let mut triggers = Vec::new();
    for (_, payload) in schema {
        let values = decode_record(&payload, encoding)?;
        let [Value::Text(kind), Value::Text(name), Value::Text(_), Value::Int(root), Value::Text(sql)] = &values[..] else {
            return None;
        };
        match kind.as_str() {
            "view" => views.push(parse_view(sql, &format!("v{}", views.len())).filter(|_| *name == format!("v{}", views.len()))?),
            "trigger" => {
                triggers.push(parse_trigger(sql, &format!("g{}", triggers.len())).filter(|_| *name == format!("g{}", triggers.len()))?)
            },
            "table" => {
                let (sql, strict) = match sql.strip_suffix(" STRICT") {
                    Some(s) => (s.strip_suffix(',').unwrap_or(s), true),
                    None => (sql.as_str(), false),
                };
                let (sql, without_rowid) = match sql.strip_suffix(" WITHOUT ROWID") {
                    Some(s) => (s, true),
                    None => (sql, false),
                };
                // Named constraints (`crate::writer::constraint_name`) read
                // as the unnamed ones: names without spaces.
                let unnamed = strip_constraint_names(sql);
                let (head, inner) = call(unnamed.strip_prefix("CREATE TABLE ")?)?;
                if head != name || *name != format!("t{}", tables.len()) {
                    return None;
                }
                // Column definitions are separated by ", ", which no declared
                // type holds (`DECIMAL(10,5)` holds a comma).
                let (cols, pk) = match inner.split_once(", PRIMARY KEY(") {
                    Some((cols, pk)) => (cols.split(", ").collect::<Vec<_>>(), Some(list(pk.strip_suffix(')')?))),
                    None => (inner.split(", ").collect::<Vec<_>>(), None),
                };
                let columns = cols.len();
                let mut key = Key::Rowid;
                let mut types = Vec::with_capacity(columns);
                let mut not_null = Vec::with_capacity(columns);
                let mut decl = Vec::with_capacity(columns);
                let mut colls = Vec::with_capacity(columns);
                for (k, c) in cols.iter().enumerate() {
                    let (c, nn) = match c.strip_suffix(" NOT NULL") {
                        Some(c) => (c, true),
                        None => (*c, false),
                    };
                    let (c, coll) = match c.rsplit_once(" COLLATE ") {
                        Some((c, k)) => (c, Some(Coll::from_name(k)?)),
                        None => (c, None),
                    };
                    colls.push(coll);
                    let (c, pk) = match c.strip_suffix(" PRIMARY KEY") {
                        Some(c) if key == Key::Rowid => {
                            key = Key::IntegerPrimaryKey(k);
                            (c, true)
                        },
                        _ => (c, false),
                    };
                    let (c, ty) = match c.split_once(' ') {
                        Some((c, ty)) => (c, Some(ty)),
                        None => (c, None),
                    };
                    match (strict, ty) {
                        (true, Some(ty)) => {
                            types.push(ColType::from_name(ty)?);
                            decl.push(None);
                        },
                        (false, Some("INTEGER")) if pk => decl.push(None),
                        (false, Some(ty)) if !pk => decl.push(Some(DECLS.iter().position(|(name, _)| *name == ty)?)),
                        (false, None) if !pk => decl.push(None),
                        _ => return None,
                    }
                    if column(c) != Some(k) {
                        return None;
                    }
                    not_null.push(nn);
                }
                let strict = strict.then_some(types);
                // A column of REAL affinity reads an integer back as a real.
                let shape = Table { columns, key: key.clone(), rows: Vec::new(), indexes: Vec::new(), strict: strict.clone(), not_null: Vec::new(), decl: decl.clone(), coll: Vec::new(), default: vec![None; columns], checks: Vec::new(), fks: Vec::new(), generated: vec![None; columns], key_conflict: None, db: crate::model::Db::Main };
                let reals: Vec<bool> = (0..columns).map(|c| affinity(&shape, c) == Affinity::Real && !matches!(key, Key::IntegerPrimaryKey(k) if k == c)).collect();
                let real = |c: usize, v: Value| match v {
                    Value::Int(i) if reals[c] => Value::Real(i as f64),
                    v => v,
                };
                let root = u32::try_from(*root).ok()?;
                let rows = match (pk, without_rowid) {
                    (None, false) => {
                        let mut rows = Vec::new();
                        file.table(root, 0, &mut rows)?;
                        rows.into_iter()
                            .map(|(rowid, payload)| {
                                let mut values = decode_record(&payload, encoding)?;
                                if values.is_empty() || values.len() > columns {
                                    return None;
                                }
                                values.resize(columns, Value::Null);
                                let values = values.into_iter().enumerate().map(|(c, v)| real(c, v)).collect();
                                Some(Row { rowid, values })
                            })
                            .collect::<Option<Vec<_>>>()?
                    },
                    (Some(pk), true) if key == Key::Rowid => {
                        key = Key::WithoutRowid(pk.iter().map(|c| column(c)).collect::<Option<Vec<_>>>()?);
                        let order = record_columns(&Table::new(columns, key.clone()));
                        let mut payloads = Vec::new();
                        file.index(root, 0, &mut payloads)?;
                        payloads
                            .into_iter()
                            .enumerate()
                            .map(|(k, payload)| {
                                let stored = decode_record(&payload, encoding)?;
                                if stored.len() != columns {
                                    return None;
                                }
                                let mut values = vec![Value::Null; columns];
                                for (v, &c) in stored.into_iter().zip(&order) {
                                    values[c] = real(c, v);
                                }
                                Some(Row { rowid: k as i64 + 1, values })
                            })
                            .collect::<Option<Vec<_>>>()?
                    },
                    _ => return None,
                };
                tables.push(Table { columns, key, rows, indexes: Vec::new(), strict, not_null, decl, coll: colls, default: vec![None; columns], checks: Vec::new(), fks: Vec::new(), generated: vec![None; columns], key_conflict: None, db: crate::model::Db::Main });
            },
            "index" => {
                let (unique, rest) = match sql.strip_prefix("CREATE UNIQUE INDEX ") {
                    Some(rest) => (true, rest),
                    None => (false, sql.strip_prefix("CREATE INDEX ")?),
                };
                let (iname, on) = rest.split_once(" ON ")?;
                let (tname, cols) = call(on)?;
                let cols = list(cols);
                let t = tables.len().checked_sub(1)?;
                let table = &mut tables[t];
                if iname != name || *iname != format!("t{t}_i{}", table.indexes.len()) || tname != format!("t{t}") {
                    return None;
                }
                let mut coll = Vec::new();
                let columns = cols
                    .iter()
                    .map(|c| {
                        let (c, desc) = match c.strip_suffix(" DESC") {
                            Some(c) => (c, true),
                            None => (*c, false),
                        };
                        let c = match c.split_once(" COLLATE ") {
                            Some((c, k)) => {
                                coll.push((column(c)?, Coll::from_name(k)?));
                                c
                            },
                            None => c,
                        };
                        Some((column(c)?, desc))
                    })
                    .collect::<Option<Vec<_>>>()?;
                table.indexes.push(Index { unique, coll, ..Index::of(columns) });
            },
            _ => return None,
        }
    }
    let model = Model { encoding, tables, views, triggers, savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
    model.well_formed().then_some((model, layout))
}
