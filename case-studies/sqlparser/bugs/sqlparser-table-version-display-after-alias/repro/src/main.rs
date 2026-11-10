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
    show("snowflake-doc-example", &SnowflakeDialect {}, "SELECT * FROM my_table AT(OFFSET => -60*5) AS T WHERE T.flag = 'valid'");
    show("snowflake-no-as", &SnowflakeDialect {}, "SELECT * FROM t1 AT(TIMESTAMP => '2024-06-05 17:50:00'::TIMESTAMP_LTZ) h");
    show("mssql-alias-hints", &MsSqlDialect {}, "SELECT * FROM t FOR SYSTEM_TIME AS OF '2014-01-01' AS x WITH (NOLOCK)");
    show("databricks", &DatabricksDialect {}, "SELECT * FROM events VERSION AS OF 123 AS e");
    show("bigquery-documented-order", &BigQueryDialect {}, "SELECT * FROM t AS x FOR SYSTEM_TIME AS OF '2020-01-01'");
    show("bigquery-other-order", &BigQueryDialect {}, "SELECT * FROM t FOR SYSTEM_TIME AS OF '2020-01-01' AS x");
}
