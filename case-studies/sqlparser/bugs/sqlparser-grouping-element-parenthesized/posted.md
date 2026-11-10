# `ROLLUP`/`CUBE`/`GROUPING SETS`: an element starting with `(` is always read as a sublist (`(SELECT 1)` rejected, `((b))` displayed as `(b)`)

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2806 on 2026-11-02. The text below is the issue as posted.

---

Each element of `ROLLUP`, `CUBE` and `GROUPING SETS` treats a leading `(` as its own sublist delimiter, which breaks parenthesized expressions:

```
SELECT a FROM t GROUP BY ROLLUP (a, (SELECT 1))
  -> sql parser error: Expected: ), found: 1 at Line: 1, Column: 45
SELECT a FROM t GROUP BY CUBE (a, (SELECT 1))
SELECT a FROM t GROUP BY GROUPING SETS (a, (SELECT 1))
  -> same error

SELECT a FROM t GROUP BY ROLLUP (a, ((b)))
  -> displays as: SELECT a FROM t GROUP BY ROLLUP (a, (b))
  -> reparses as ROLLUP (a, b): element [Nested(b)] becomes [b]
```

PostgreSQL 18 runs `SELECT a FROM t GROUP BY ROLLUP (a, (SELECT 1))`. Its docs: "The individual elements of a CUBE or ROLLUP clause may be either individual expressions, or sublists of elements in parentheses" (https://www.postgresql.org/docs/current/queries-table-expressions.html#QUERIES-GROUPING-SETS). The second case changes only the AST, since PostgreSQL treats `((b))` and `(b)` the same.

`parse_group_by_expr` parses each element with `parse_tuple(true, true)`, which consumes the `(` of `(SELECT 1)` and then reads `SELECT` as an identifier. Display for `Rollup`/`Cube` writes a one-element set without its own parentheses, so `[Nested(b)]` prints as `(b)`.

Tested with PostgreSqlDialect on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
