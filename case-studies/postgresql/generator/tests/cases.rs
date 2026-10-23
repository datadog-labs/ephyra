// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! The property: PostgreSQL loading a case's setup and running its
//! statements agrees with the model, statement by statement (a query's
//! rows, a write's row count), and with every table's contents at the end;
//! and `amcheck` finds every heap, B-tree and GIN index consistent.

use ephyra_postgresql::{
    cases::{cases, Case},
    model::Value,
    pg::with_session,
    semantics,
    setup::load,
    statements::returns_rows,
};

#[ephyra::test(gen = cases(), tests = 2000)]
fn postgres_agrees_with_the_model(case: Case) {
    with_session(|s| {
        s.fresh()?;
        load(s, &case.setup)?;
        let mut m = case.setup.model.clone();
        for (k, st) in case.statements.iter().enumerate() {
            let sql = st.to_string();
            let effect = semantics::apply(&mut m, st);
            if returns_rows(st) {
                let actual = s.values(&sql).map_err(|e| format!("statement {k}: {sql}: {e}"))?;
                let expected = effect.rows.unwrap();
                assert!(expected.matches(&actual), "statement {k}: {sql}\nexpected {expected:?}\nactual {actual:?}");
            } else {
                let n = s.execute(&sql).map_err(|e| format!("statement {k}: {sql}: {e}"))?;
                if let Some(w) = effect.written {
                    assert_eq!(n, w as u64, "statement {k}: {sql}: rows written");
                }
            }
        }
        for t in &m.tables {
            let actual = s.values(&format!("SELECT * FROM t{}", t.id))?;
            assert!(semantics::same_rows(&actual, &t.rows), "table t{} at the end\nexpected {:?}\nactual {actual:?}", t.id, t.rows);
            // Every leaf relation (the table itself, or its partitions) and
            // its B-tree and GIN indexes, from the catalog.
            let leaves = s.values(&format!("SELECT relid::regclass::text FROM pg_partition_tree('t{}') WHERE isleaf", t.id))?;
            for leaf in leaves {
                let Value::Text(rel) = &leaf[0] else { continue };
                let corrupt = s.query(&format!("SELECT blkno, offnum, msg FROM verify_heapam('{rel}')"))?;
                assert!(corrupt.is_empty(), "verify_heapam({rel}): {corrupt:?}");
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
                    s.execute(&check).map_err(|e| format!("{check}: {e}"))?;
                }
            }
        }
        Ok(())
    })
    .unwrap_or_else(|e| panic!("{e}"));
}
