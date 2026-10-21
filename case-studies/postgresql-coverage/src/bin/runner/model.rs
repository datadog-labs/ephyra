//! The model property of ephyra-postgresql's `tests/cases.rs` as a function
//! the runner rotates with the oracles: PostgreSQL loading a case's setup
//! and running its statements agrees with the model, statement by statement
//! (a query's rows, a write's row count), and with every table's contents
//! at the end; and `amcheck` finds every heap, B-tree and GIN index
//! consistent. The test's assertions are failures here; the queries of the
//! input are not used.
//!
//! Keep it equivalent to the test when that changes.

use ephyra_postgresql::{cases::Case, expr::Query, model::Value, pg::with_session, semantics, setup::load, sqlancer::Tally, statements::returns_rows};

pub fn check(case: &Case, _queries: &[Query]) -> Result<Tally, String> {
    with_session(|s| {
        let mut tally = Tally::default();
        let mut count = |name: &'static str| *tally.comparisons.entry(name).or_insert(0) += 1;
        s.fresh()?;
        load(s, &case.setup)?;
        let mut m = case.setup.model.clone();
        for (k, st) in case.statements.iter().enumerate() {
            let sql = st.to_string();
            let effect = semantics::apply(&mut m, st);
            if returns_rows(st) {
                let actual = s.values(&sql).map_err(|e| format!("statement {k}: {sql}: {e}"))?;
                let expected = effect.rows.ok_or_else(|| format!("statement {k}: {sql}: the model has no rows for it"))?;
                if !expected.matches(&actual) {
                    return Err(format!("model: rows differ\n  statement {k}: {sql}\n  expected {expected:?}\n  actual {actual:?}"));
                }
                count("model-rows");
            } else {
                let n = s.execute(&sql).map_err(|e| format!("statement {k}: {sql}: {e}"))?;
                if let Some(w) = effect.written {
                    if n != w as u64 {
                        return Err(format!("model: rows written differ\n  statement {k}: {sql}\n  expected {w}, actual {n}"));
                    }
                    count("model-written");
                }
            }
        }
        for t in &m.tables {
            let actual = s.values(&format!("SELECT * FROM t{}", t.id))?;
            if !semantics::same_rows(&actual, &t.rows) {
                return Err(format!("model: table contents differ\n  table t{} at the end\n  expected {:?}\n  actual {actual:?}", t.id, t.rows));
            }
            count("model-contents");
            let leaves = s.values(&format!("SELECT relid::regclass::text FROM pg_partition_tree('t{}') WHERE isleaf", t.id))?;
            for leaf in leaves {
                let Value::Text(rel) = &leaf[0] else { continue };
                let corrupt = s.query(&format!("SELECT blkno, offnum, msg FROM verify_heapam('{rel}')"))?;
                if !corrupt.is_empty() {
                    return Err(format!("model: verify_heapam finds corruption\n  verify_heapam({rel}): {corrupt:?}"));
                }
                count("model-heapam");
                let indexes = s.values(&format!(
                    "SELECT i.indexrelid::regclass::text, am.amname::text FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid JOIN pg_am am ON am.oid = c.relam WHERE i.indrelid = '{rel}'::regclass"
                ))?;
                for ix in indexes {
                    let (Value::Text(name), Value::Text(am)) = (&ix[0], &ix[1]) else { continue };
                    let check = match am.as_str() {
                        "btree" => format!("SELECT bt_index_check('{name}', true)"),
                        "gin" => format!("SELECT gin_index_check('{name}')"),
                        _ => continue,
                    };
                    s.execute(&check).map_err(|e| format!("model: index check fails\n  {check}: {e}"))?;
                    count("model-index-check");
                }
            }
        }
        tally.queries = case.statements.len();
        Ok(tally)
    })
}
