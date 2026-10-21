use ephyra_sql::ast::{Case, Expr, Literal, Query, Type};
use ephyra_sql::engine::{self, Outcome};
use ephyra_sql::generate::cases;
use ephyra_sql::observe;

fn differential(case: Case) {
    let turso = ephyra::measure("turso", || engine::turso(&case));
    let sqlite = ephyra::measure("sqlite", || engine::sqlite(&case));
    observe::case(&case, &turso);
    assert!(
        engine::agree(case.query.order.is_some(), &turso, &sqlite),
        "turso: {turso:?}\nsqlite: {sqlite:?}"
    );
}

/// Turso and SQLite return the same rows, or both error.
#[ephyra::test(gen = cases(), config = ephyra_sql::config())]
fn turso_agrees_with_sqlite(case: Case) {
    differential(case);
}

/// The same property under the coverage loop.
#[ephyra::test(gen = cases(), config = ephyra_sql::config(), search = ephyra_sql::explore)]
fn turso_agrees_with_sqlite_explored(case: Case) {
    differential(case);
}

/// TLP, in Turso alone: `WHERE p`, `WHERE NOT p` and `WHERE p IS NULL`
/// partition the rows of the unfiltered query.
#[ephyra::test(gen = cases(), config = ephyra_sql::config())]
fn where_partitions_rows(case: Case) {
    let Some(p) = case.query.filter.clone() else {
        return;
    };
    let with = |filter: Option<Expr>| {
        let query = Query { filter, order: None, limit: None, ..case.query.clone() };
        engine::turso(&Case { query, ..case.clone() })
    };
    let all = with(None);
    let parts = [
        with(Some(p.clone())),
        with(Some(Expr::Prefix("NOT", Box::new(p.clone())))),
        with(Some(Expr::IsNull(Box::new(p)))),
    ];
    let mut union = Vec::new();
    for part in &parts {
        match part {
            Outcome::Rows(rows) => union.extend(rows.iter().cloned()),
            // If p fails on some row, every part evaluates it there.
            Outcome::Error(_) => {
                assert!(parts.iter().all(|p| matches!(p, Outcome::Error(_))), "{parts:?}");
                return;
            },
        }
    }
    let union = Outcome::Rows(union);
    assert!(engine::agree(false, &all, &union), "unfiltered: {all:?}\npartitions: {union:?}");
}

#[test]
fn cases_render_as_sql() {
    let case = Case {
        schema: vec![Type::Integer, Type::Text],
        rows: vec![
            vec![Literal::Integer(-1), Literal::Text("it's".into())],
            vec![Literal::Null, Literal::Null],
        ],
        query: Query {
            columns: Some(vec![1]),
            filter: Some(Expr::IsNull(Box::new(Expr::Column(0)))),
            order: Some(true),
            limit: Some(1),
        },
    };
    let script = "CREATE TABLE t(c0 INTEGER, c1 TEXT);\n\
                  INSERT INTO t VALUES ((-1), 'it''s');\n\
                  INSERT INTO t VALUES (NULL, NULL);\n\
                  SELECT c1 FROM t WHERE (c0 IS NULL) ORDER BY c1 DESC LIMIT 1;";
    assert_eq!(case.to_string(), script);
    let outcome = engine::turso(&case);
    assert!(
        engine::agree(true, &outcome, &Outcome::Rows(vec![vec![engine::Value::Null]])),
        "{outcome:?}"
    );
}
