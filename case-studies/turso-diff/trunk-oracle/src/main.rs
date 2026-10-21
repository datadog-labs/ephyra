//! `trunk-oracle DB SQL`: the database file DB opened in memory by SQLite
//! trunk (a WAL header read as rollback mode, as turso-diff does), each
//! statement of the file SQL run in turn (errors before the last one are
//! ignored, as they are for the engines being compared), and the last
//! statement's outcome printed: `OK` and one line per row, cells separated
//! by tabs (`N`, `I:<int>`, `R:<f64 bits, hex>`, `T:<hex>`, `B:<hex>`, or
//! `J:<hex>` for a BLOB that is valid JSONB, as its JSON text); or `E:` and
//! the error message.

use rusqlite::{types::ValueRef, Connection};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn split(sql: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    for c in sql.chars() {
        cur.push(c);
        if c == ';' {
            let z = std::ffi::CString::new(cur.as_str()).unwrap_or_default();
            // SAFETY: a NUL-terminated string that outlives the call.
            if unsafe { rusqlite::ffi::sqlite3_complete(z.as_ptr()) } != 0 {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.retain(|s| !s.trim().is_empty());
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut bytes = std::fs::read(&args[1]).expect("database file");
    let sql = std::fs::read_to_string(&args[2]).expect("SQL file");
    let mut db = Connection::open_in_memory().unwrap();
    if !bytes.is_empty() {
        for at in [18, 19] {
            if bytes[at] == 2 {
                bytes[at] = 1;
            }
        }
        if let Err(e) = db.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), false) {
            println!("E:{e}");
            return;
        }
    }
    let stmts = split(&sql);
    let Some((last, before)) = stmts.split_last() else {
        println!("E:no statements");
        return;
    };
    for s in before {
        let _ = db.execute_batch(s);
    }
    let go = || -> rusqlite::Result<Vec<String>> {
        let mut st = db.prepare(last)?;
        let n = st.column_count();
        let mut json = db.prepare("SELECT json(?1) WHERE json_valid(?1, 8)")?;
        let mut rows = st.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let mut cells = Vec::new();
            for k in 0..n {
                cells.push(match r.get_ref(k)? {
                    ValueRef::Null => "N".to_string(),
                    ValueRef::Integer(i) => format!("I:{i}"),
                    ValueRef::Real(x) => format!("R:{:x}", x.to_bits()),
                    ValueRef::Text(t) => format!("T:{}", hex(t)),
                    ValueRef::Blob(b) => match json.query_row([b], |r| r.get::<_, Option<String>>(0)) {
                        Ok(Some(j)) => format!("J:{}", hex(j.as_bytes())),
                        _ => format!("B:{}", hex(b)),
                    },
                });
            }
            out.push(cells.join("\t"));
        }
        Ok(out)
    };
    match go() {
        Ok(rows) => {
            println!("OK");
            for r in rows {
                println!("{r}");
            }
        },
        Err(e) => println!("E:{e}"),
    }
}
