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
    show("spark-trailing", &SparkSqlDialect {}, "CACHE TABLE t;");
    show("databricks-next-statement", &DatabricksDialect {}, "CACHE TABLE t; SELECT 1");
    show("databricks-lazy-options", &DatabricksDialect {}, "CACHE LAZY TABLE t OPTIONS ('storageLevel' = 'DISK_ONLY'); SELECT 1");
    show("with-query-ok", &DatabricksDialect {}, "CACHE TABLE t AS SELECT 1; SELECT 1");
}
