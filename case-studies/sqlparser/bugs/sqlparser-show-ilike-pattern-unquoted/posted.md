# `SHOW ... ILIKE '<pattern>'` is displayed without quotes

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2759 on 2026-10-28. The text below is the issue as posted.

---

`SHOW ... ILIKE '<pattern>'` loses the quotes around the pattern when the statement is displayed, so the output doesn't parse:

```rust
let sql = "SHOW DATABASES ILIKE '%DE%'";
let stmts = Parser::parse_sql(&ClickHouseDialect {}, sql).unwrap();
println!("{}", stmts[0]);
// SHOW DATABASES ILIKE %DE%
// reparsing: sql parser error: Expected: literal string, found: % at Line: 1, Column: 22
```

Expected: `SHOW DATABASES ILIKE '%DE%'`. The `LIKE` arm keeps the quotes; `SHOW TABLES ILIKE 'abc'` displays as `SHOW TABLES ILIKE abc`, which only reparses because a bare word is accepted there.

Tested on main e15e9abf (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip and statement-separation checks); I used AI assistance for triage and verified every case above.
