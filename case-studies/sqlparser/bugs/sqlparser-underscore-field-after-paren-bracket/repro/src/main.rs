use sqlparser::dialect::*;
use sqlparser::parser::Parser;

/// Parse `sql`, print each statement's Display, and whether the Display
/// parses back to the same AST (AST equality ignores spans).
fn show(name: &str, d: &dyn Dialect, sql: &str) {
    println!("[{name}] input:   {sql}");
    match Parser::parse_sql(d, sql) {
        Err(e) => println!("[{name}] parse:   ERROR {e}"),
        Ok(stmts) => {
            println!("[{name}] parsed:  {} statement(s)", stmts.len());
            for s in &stmts {
                let out = s.to_string();
                println!("[{name}] display: {out}");
                match Parser::parse_sql(d, &out) {
                    Err(e) => println!("[{name}] reparse: ERROR {e}"),
                    Ok(r) if r.len() == 1 && r[0] == *s => println!("[{name}] reparse: same AST"),
                    Ok(r) => println!(
                        "[{name}] reparse: DIFFERENT AST, displays as: {}",
                        r.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ")
                    ),
                }
            }
        }
    }
    println!();
}

fn main() {
    show("postgres-composite", &PostgreSqlDialect {}, "SELECT (c)._x FROM tt");
    show("duckdb-struct", &DuckDbDialect {}, "SELECT ({'_x': 1})._x");
    show("duckdb-list-element", &DuckDbDialect {}, "SELECT ([{'_a': 2}])[1]._a");
    show("bigquery-call", &BigQueryDialect {}, "SELECT f()._x");
    show("without-underscore-ok", &PostgreSqlDialect {}, "SELECT (c).x FROM tt");
}
