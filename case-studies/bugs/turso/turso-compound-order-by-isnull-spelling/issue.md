Filed as #10395 (https://github.com/tursodatabase/turso/issues/10395) on 2026-10-21.
Title: Compound SELECT: ORDER BY `x ISNULL` does not match a result column written `x IS NULL` (and NOTNULL / NOT NULL / IS NOT NULL)

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES (1), (NULL);
SELECT a IS NULL FROM t UNION ALL SELECT 2 ORDER BY a ISNULL;
SELECT a ISNULL FROM t UNION ALL SELECT 2 ORDER BY a IS NULL;
SELECT a IS NOT NULL FROM t UNION ALL SELECT 2 ORDER BY a NOTNULL;
SELECT a IS NOT NULL FROM t UNION ALL SELECT 2 ORDER BY a NOT NULL;
```

Turso (`tursodb -q -m list :memory: < repro.sql`), for each statement:
```
  × Parse error: 1st ORDER BY term does not match any column in the result set
```

SQLite 3.50.4 (trunk gives the same), for each statement:
```
0
1
2
```

Expected: `x ISNULL` and `x IS NULL` are the same expression, and so are `x NOTNULL`, `x NOT NULL` and `x IS NOT NULL` (SQLite parses all of them to TK_ISNULL / TK_NOTNULL). The ORDER BY term of a compound SELECT matches the result column whichever spelling each side uses. Turso matches only when both sides use the same spelling: `ORDER BY a IS NULL` with `SELECT a IS NULL` works, and so does `ORDER BY a ISNULL` with `SELECT a ISNULL`.

Likely cause: the parser builds `Expr::IsNull` / `Expr::NotNull` for the postfix forms (sqlite/parser/src/parser.rs, the TK_ISNULL / TK_NOTNULL arms) and a `Binary(_, Is / IsNot, Null)` for `IS [NOT] NULL`. `exprs_are_equivalent` (core/util.rs), which `resolve_compound_order_by_expr` (core/translate/select.rs) uses to match the term, compares `IsNull` only with `IsNull` and `NotNull` only with `NotNull`.

Tested on Turso main db441331.
