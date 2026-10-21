//! The control for a generated file: the same logical contents, written by
//! SQLite the ordinary way. Coverage a generated file reaches beyond its
//! control is what its physical layout adds (page size, packing, free
//! blocks, odd encodings of integers, deep trees, freelists, ...), as
//! opposed to what its contents and the statements reach on any file.
//!
//! The control keeps the text encoding, which is logical (it decides how
//! text compares under BINARY collation), and the file's page size and
//! auto-vacuum mode, which SQLite's own writer produces as readily: what is
//! credited beyond the control is then only how the generated file arranges
//! its pages and cells. Reserved bytes per page are the one setting it does
//! not match, since SQL cannot set them.

use ephyra_sqlite::{
    model::{Encoding, Key, Layout, Model, Value},
    writer::{create_table, create_trigger, create_view},
    SqliteFile,
};
use rusqlite::{types::ToSqlOutput, Connection, ToSql};

struct Bound<'a>(&'a Value);

impl ToSql for Bound<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self.0 {
            Value::Null => ToSqlOutput::from(rusqlite::types::Null),
            Value::Int(i) => ToSqlOutput::from(*i),
            Value::Real(r) => ToSqlOutput::from(*r),
            Value::Text(s) => ToSqlOutput::from(s.as_str()),
            Value::Blob(b) => ToSqlOutput::from(b.as_slice()),
        })
    }
}

pub fn control(model: &Model, layout: &Layout) -> SqliteFile {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(&format!(
        "PRAGMA page_size = {}; PRAGMA auto_vacuum = {}",
        layout.page_size, layout.vacuum
    ))
    .unwrap();
    let encoding = match model.encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf16le => "UTF-16le",
        Encoding::Utf16be => "UTF-16be",
    };
    db.execute_batch(&format!("PRAGMA encoding = '{encoding}'"))
        .unwrap();
    for (ti, t) in model.tables.iter().enumerate() {
        let name = format!("t{ti}");
        db.execute_batch(&create_table(&name, t)).unwrap();
        // An INTEGER PRIMARY KEY column is the rowid: it is written as the
        // rowid and left out of the column list.
        let columns: Vec<usize> = (0..t.columns)
            .filter(|c| !matches!(t.key, Key::IntegerPrimaryKey(k) if k == *c))
            .collect();
        let rowid = !matches!(t.key, Key::WithoutRowid(_));
        let mut names: Vec<String> = columns.iter().map(|c| format!("c{c}")).collect();
        if rowid {
            names.insert(0, "rowid".into());
        }
        let marks = vec!["?"; names.len()].join(", ");
        let mut insert = db
            .prepare(&format!(
                "INSERT INTO {name}({}) VALUES ({marks})",
                names.join(", ")
            ))
            .unwrap();
        for row in &t.rows {
            let id = Value::Int(row.rowid);
            let mut params: Vec<Bound> = columns.iter().map(|c| Bound(&row.values[*c])).collect();
            if rowid {
                params.insert(0, Bound(&id));
            }
            let params: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
            insert.execute(params.as_slice()).unwrap();
        }
        for (ii, idx) in t.indexes.iter().enumerate() {
            let cols: Vec<String> = idx
                .columns
                .iter()
                .map(|&(c, d)| {
                    if d {
                        format!("c{c} DESC")
                    } else {
                        format!("c{c}")
                    }
                })
                .collect();
            db.execute_batch(&format!(
                "CREATE INDEX {name}_i{ii} ON {name}({})",
                cols.join(", ")
            ))
            .unwrap();
        }
    }
    for (k, v) in model.views.iter().enumerate() {
        db.execute_batch(&create_view(&format!("v{k}"), v)).unwrap();
    }
    for (k, g) in model.triggers.iter().enumerate() {
        db.execute_batch(&create_trigger(&format!("g{k}"), g))
            .unwrap();
    }
    SqliteFile(db.serialize(rusqlite::MAIN_DB).unwrap().to_vec())
}
