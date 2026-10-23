// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The control holds exactly the rows of the file it stands in for.

use ephyra::{Gen, Source};
use ephyra_sqlite::{files::Files, integrity_ok, reader, SqliteFile};
use sqlite_coverage::{control, open};

/// `sql` without its constraint names (`CONSTRAINT nn3 NOT NULL` as `NOT NULL`): the control is
/// built from the model, which keeps no names, and SQLite treats a named constraint as the same
/// constraint.
fn unnamed(sql: &str) -> String {
    let mut out = String::new();
    let mut rest = sql;
    while let Some(at) = rest.find("CONSTRAINT ") {
        out.push_str(&rest[..at]);
        let name = &rest[at + "CONSTRAINT ".len()..];
        let end = match name.chars().next() {
            Some(q @ ('[' | '"' | '`')) => {
                let close = if q == '[' { ']' } else { q };
                name[1..].find(close).map_or(name.len(), |i| i + 2)
            }
            _ => name.find(' ').unwrap_or(name.len()),
        };
        rest = name[end..].trim_start();
    }
    out.push_str(rest);
    out
}

/// Every table's rows, rowid first, as SQLite reads them from `file`.
fn contents(file: &SqliteFile) -> Vec<Vec<String>> {
    if file.0.is_empty() {
        return Vec::new();
    }
    let db = open(file, true);
    let tables: Vec<(String, String)> = db
        .prepare("SELECT name, sql FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .iter()
        .map(|(t, sql)| {
            let key = if sql.contains("WITHOUT ROWID") {
                ""
            } else {
                "rowid, "
            };
            let mut stmt = db.prepare(&format!("SELECT {key}* FROM {t}")).unwrap();
            let n = stmt.column_count();
            let mut out = vec![format!("{t}: {}", unnamed(sql))];
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                out.push(format!(
                    "{:?}",
                    (0..n).map(|c| row.get_ref(c).unwrap()).collect::<Vec<_>>()
                ));
            }
            out.sort();
            out
        })
        .collect()
}

#[test]
fn controls_hold_the_rows_of_their_files() {
    for i in 0..3000u64 {
        let size = (u64::BITS - (i + 1).leading_zeros() - 1) as usize;
        let file = Files.generate(&mut Source::new(i, size));
        let Some((model, layout)) = reader::read(&file.0) else {
            assert!(file.0.is_empty(), "file {i} does not read back");
            continue;
        };
        let control = control(&model, &layout);
        assert!(integrity_ok(&control), "control {i} fails integrity_check");
        let page_size = u32::from(u16::from_be_bytes([control.0[16], control.0[17]]));
        assert_eq!(
            if page_size == 1 { 65536 } else { page_size },
            layout.page_size,
            "control {i} page size"
        );
        let be32 = |at: usize| {
            u32::from_be_bytes([
                control.0[at],
                control.0[at + 1],
                control.0[at + 2],
                control.0[at + 3],
            ])
        };
        let vacuum = if be32(52) == 0 {
            0
        } else if be32(64) == 0 {
            1
        } else {
            2
        };
        assert_eq!(vacuum, layout.vacuum, "control {i} auto-vacuum mode");
        assert_eq!(contents(&file), contents(&control), "file {i}");
    }
}
