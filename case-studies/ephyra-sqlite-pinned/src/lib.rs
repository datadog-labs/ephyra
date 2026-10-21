//! What a valid SQLite database file is, for ephyra to synthesize
//! generators of: [`SqliteFile`] (a file's bytes), the predicate
//! [`integrity_ok`], and [`file_metrics`], numbers about a file's structure
//! for a generator to declare.
//!
//! SQLite itself is the judge. The bytes are deserialized into an in-memory,
//! read-only database, and the file is valid when `PRAGMA integrity_check`
//! answers exactly `ok`. Two valid files need care, since deserialization
//! rejects them: the empty file, which SQLite reads as an empty database, is
//! opened as one; and a header saying WAL mode is presented as rollback mode
//! (bytes 18 and 19 set to 1). With no `-wal` file beside it, SQLite reads
//! such a file's pages exactly as written, so the check sees the same
//! B-trees.

use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use rusqlite::Connection;

pub mod cases;
pub mod expr;
pub mod files;
pub mod model;
pub mod probes;
pub mod reader;
pub mod semantics;
pub mod sqlancer;
pub mod statements;
pub mod vtabs;
pub mod writer;

/// Mutations of a file or case that fell back to a fresh draw (the file did
/// not read back), counted for a fuzz loop's measurements.
pub static FILE_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Mutations of queries that fell back to fresh queries (no edit applied
/// validly, or none was left after a case's mutation), counted likewise.
pub static QUERY_FALLBACKS: AtomicU64 = AtomicU64::new(0);

fn fell_back(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// A database file, as bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct SqliteFile(pub Vec<u8>);

/// A file valid by SQLite's own check: it opens, and `PRAGMA
/// integrity_check` reports `ok`.
#[ephyra::generator]
pub fn integrity_ok(file: &SqliteFile) -> bool {
    with_open(&file.0, |db| {
        let first: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        Ok(first == "ok")
    })
    .unwrap_or(false)
}

/// Run `f` on `bytes` opened as a read-only in-memory database.
fn with_open<T>(bytes: &[u8], f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Option<T> {
    let mut db = Connection::open_in_memory().ok()?;
    if !bytes.is_empty() {
        let mut bytes = bytes.to_vec();
        for at in [18, 19] {
            if bytes.get(at) == Some(&2) {
                bytes[at] = 1;
            }
        }
        db.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), true).ok()?;
    }
    f(&db).ok()
}

/// The 100-byte header's fields, when the file has one.
struct Header {
    page_size: u32,
    wal: bool,
    reserved: u8,
    page_count: u32,
    freelist_pages: u32,
    schema_format: u32,
    auto_vacuum: bool,
    incremental_vacuum: bool,
    encoding: u32,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn header(bytes: &[u8]) -> Option<Header> {
    if bytes.len() < 100 || &bytes[..16] != b"SQLite format 3\0" {
        return None;
    }
    let size = u16::from_be_bytes([bytes[16], bytes[17]]);
    Some(Header {
        page_size: if size == 1 { 65536 } else { u32::from(size) },
        wal: bytes[18] == 2 || bytes[19] == 2,
        reserved: bytes[20],
        page_count: be32(bytes, 28),
        freelist_pages: be32(bytes, 36),
        schema_format: be32(bytes, 44),
        auto_vacuum: be32(bytes, 52) != 0,
        incremental_vacuum: be32(bytes, 64) != 0,
        encoding: be32(bytes, 56),
    })
}

/// What SQLite reports about a file's contents.
#[derive(Default)]
struct Contents {
    tables: f64,
    without_rowid: f64,
    strict: f64,
    indexes: f64,
    views: f64,
    triggers: f64,
    rows: f64,
    interior_pages: f64,
    leaf_pages: f64,
    overflow_pages: f64,
    depth: f64,
    btree_bytes: f64,
    unused_bytes: f64,
}

fn contents(bytes: &[u8]) -> Contents {
    with_open(bytes, |db| {
        let mut c = Contents::default();
        let mut stmt = db.prepare("SELECT type, name, coalesce(sql, '') FROM sqlite_schema")?;
        let objects: Vec<(String, String, String)> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
        for (kind, name, sql) in &objects {
            let sql = sql.to_uppercase();
            match kind.as_str() {
                "table" => {
                    c.tables += 1.0;
                    c.without_rowid += f64::from(u8::from(sql.contains("WITHOUT ROWID")));
                    c.strict += f64::from(u8::from(sql.trim_end().ends_with("STRICT")));
                    let count = format!("SELECT count(*) FROM \"{}\"", name.replace('"', "\"\""));
                    c.rows += db.query_row(&count, [], |r| r.get::<_, i64>(0)).unwrap_or(0) as f64;
                },
                "index" => c.indexes += 1.0,
                "view" => c.views += 1.0,
                "trigger" => c.triggers += 1.0,
                _ => {},
            }
        }
        let mut stmt = db.prepare("SELECT pagetype, path, pgsize, unused FROM dbstat")?;
        let pages: Vec<(String, String, i64, i64)> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        for (kind, path, size, unused) in &pages {
            match kind.as_str() {
                "internal" => c.interior_pages += 1.0,
                "leaf" => c.leaf_pages += 1.0,
                "overflow" => c.overflow_pages += 1.0,
                _ => {},
            }
            if kind != "overflow" {
                // `dbstat` reads a page header's cell-content offset of 0 as 0,
                // but on a 65536-byte page it means 65536, so an empty page 1
                // reports -108 unused bytes. Only that case is negative.
                let unused = if *unused < 0 { unused + 65536 } else { *unused };
                c.btree_bytes += *size as f64;
                c.unused_bytes += unused as f64;
                // A path is `/`, then one `NNN/` per level below the root.
                c.depth = c.depth.max(path.matches('/').count() as f64);
            }
        }
        Ok(c)
    })
    .unwrap_or_default()
}

/// Numbers about one valid file. All are 0 for the empty file, which has no
/// header and no pages.
///
/// From the header:
/// - bytes: the file's length.
/// - page size: bytes per page, 512 to 65536.
/// - pages: the header's page count.
/// - reserved bytes: bytes reserved at the end of every page (0 unless an
///   extension asked for them).
/// - freelist pages: pages on the freelist, free for reuse.
/// - schema format: 1 to 4.
/// - utf16: 1 when text is UTF-16 (little- or big-endian), 0 for UTF-8.
/// - wal: 1 when the header says WAL mode.
/// - auto vacuum, incremental vacuum: indicators.
///
/// From SQLite reading the file:
/// - tables, without rowid tables, strict tables, indexes, views, triggers:
///   counts of schema objects.
/// - rows: total rows over all tables.
/// - interior pages, leaf pages, overflow pages: B-tree pages by kind
///   (`dbstat`).
/// - depth: the deepest B-tree, in levels; 1 is a root that is a leaf.
/// - fill: used bytes over all bytes on B-tree pages (not overflow); 0 with
///   none, near 1 for packed pages.
pub fn file_metrics(file: &SqliteFile, _size: usize) -> Vec<(&'static str, f64)> {
    let bytes = &file.0;
    let mut m = vec![("bytes", bytes.len() as f64)];
    let h = header(bytes);
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    m.extend([
        ("page size", h.as_ref().map_or(0.0, |h| f64::from(h.page_size))),
        ("pages", h.as_ref().map_or(0.0, |h| f64::from(h.page_count))),
        ("reserved bytes", h.as_ref().map_or(0.0, |h| f64::from(h.reserved))),
        ("freelist pages", h.as_ref().map_or(0.0, |h| f64::from(h.freelist_pages))),
        ("schema format", h.as_ref().map_or(0.0, |h| f64::from(h.schema_format))),
        ("utf16", h.as_ref().map_or(0.0, |h| flag(h.encoding == 2 || h.encoding == 3))),
        ("wal", h.as_ref().map_or(0.0, |h| flag(h.wal))),
        ("auto vacuum", h.as_ref().map_or(0.0, |h| flag(h.auto_vacuum))),
        ("incremental vacuum", h.as_ref().map_or(0.0, |h| flag(h.incremental_vacuum))),
    ]);
    let c = contents(bytes);
    m.extend([
        ("tables", c.tables),
        ("without rowid tables", c.without_rowid),
        ("strict tables", c.strict),
        ("indexes", c.indexes),
        ("views", c.views),
        ("triggers", c.triggers),
        ("rows", c.rows),
        ("interior pages", c.interior_pages),
        ("leaf pages", c.leaf_pages),
        ("overflow pages", c.overflow_pages),
        ("depth", c.depth),
        ("fill", if c.btree_bytes > 0.0 { 1.0 - c.unused_bytes / c.btree_bytes } else { 0.0 }),
    ]);
    m
}

/// FNV-1a over the bytes: two files print alike only when they are alike.
fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3))
}

/// A summary rather than the bytes: the header's shape, a digest of the
/// contents, and the schema. The digest keeps files that differ only inside
/// their pages apart, which `ephyra check` relies on when it compares values
/// by their rendering.
impl fmt::Debug for SqliteFile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Some(h) = header(&self.0) else {
            return write!(f, "SqliteFile({} bytes, digest {:016x}, no header)", self.0.len(), digest(&self.0));
        };
        write!(
            f,
            "SqliteFile({} bytes, digest {:016x}, page size {}, {} pages, {} free",
            self.0.len(),
            digest(&self.0),
            h.page_size,
            h.page_count,
            h.freelist_pages
        )?;
        let schema = with_open(&self.0, |db| {
            let mut stmt = db.prepare("SELECT coalesce(sql, name) FROM sqlite_schema")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        });
        match schema {
            Some(sql) => write!(f, ")\n  {}", sql.join(";\n  ")),
            None => write!(f, ", does not open)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(sql: &str) -> SqliteFile {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(sql).unwrap();
        SqliteFile(db.serialize(rusqlite::MAIN_DB).unwrap().to_vec())
    }

    #[test]
    fn files_sqlite_writes_are_valid() {
        assert!(integrity_ok(&built("CREATE TABLE t(a, b); INSERT INTO t VALUES (1, randomblob(5000)); CREATE INDEX i ON t(a);")));
        assert!(integrity_ok(&built("PRAGMA page_size = 512; CREATE TABLE t(a PRIMARY KEY, b) WITHOUT ROWID;")));
    }

    #[test]
    fn the_empty_file_is_an_empty_database() {
        assert!(integrity_ok(&SqliteFile(Vec::new())));
        assert!(file_metrics(&SqliteFile(Vec::new()), 0).iter().all(|(_, v)| *v == 0.0));
    }

    #[test]
    fn a_wal_mode_file_is_valid() {
        let path = std::env::temp_dir().join(format!("ephyra-sqlite-wal-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(a); INSERT INTO t VALUES (1); PRAGMA wal_checkpoint(TRUNCATE);")
                .unwrap();
        }
        let file = SqliteFile(std::fs::read(&path).unwrap());
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        assert!(integrity_ok(&file));
        assert!(file_metrics(&file, 0).contains(&("wal", 1.0)));
    }

    #[test]
    fn damage_and_garbage_are_not() {
        let good = built("CREATE TABLE t(a, b); INSERT INTO t VALUES (1, randomblob(5000)); CREATE INDEX i ON t(a);");
        let mut damaged = good.clone();
        let n = damaged.0.len();
        damaged.0[n - 3000] ^= 0xff;
        damaged.0[4096 + 9] ^= 0x55;
        assert!(!integrity_ok(&damaged));
        assert!(!integrity_ok(&SqliteFile(vec![7; 4096])));
        assert!(!integrity_ok(&SqliteFile(good.0[..100].to_vec())));
    }

    #[test]
    fn fill_stays_within_bounds_on_large_pages() {
        let file = built("PRAGMA page_size = 65536; CREATE TABLE t(a);");
        let fill = file_metrics(&file, 0).into_iter().find(|(k, _)| *k == "fill").unwrap().1;
        assert!((0.0..=1.0).contains(&fill), "fill {fill}");
    }

    #[test]
    fn metrics_describe_the_file() {
        let file = built("PRAGMA page_size = 1024; CREATE TABLE t(a, b); CREATE INDEX i ON t(a); WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 300) INSERT INTO t SELECT x, randomblob(x % 7 * 400) FROM n;");
        let m: std::collections::BTreeMap<_, _> = file_metrics(&file, 0).into_iter().collect();
        assert_eq!(m["page size"], 1024.0);
        assert_eq!((m["tables"], m["indexes"], m["rows"]), (1.0, 1.0, 300.0));
        assert!(m["overflow pages"] > 0.0 && m["interior pages"] > 0.0 && m["depth"] >= 2.0, "{m:?}");
        assert!(m.values().all(|v| v.is_finite()));
    }
}
