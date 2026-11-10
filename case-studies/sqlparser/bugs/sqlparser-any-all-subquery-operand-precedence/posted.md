# `x = ANY(SELECT ...) <op> y`: the operator after `)` is parsed into the ANY/ALL operand

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2802 on 2026-11-02. The text below is the issue as posted.

---

When the right side of `ANY`/`ALL` is a subquery, parsing continues past its closing parenthesis, so a following operator becomes part of the operand:

```
SELECT 1 = ANY(SELECT 1)::text
  -> displays as: SELECT 1 = ANY((SELECT 1)::TEXT)
SELECT a = ANY(SELECT 1) - 1
  -> displays as: SELECT a = ANY((SELECT 1) - 1)
SELECT a = ALL(SELECT 1) + 1
  -> displays as: SELECT a = ALL((SELECT 1) + 1)
```

PostgreSQL applies the operator to the comparison: `SELECT pg_typeof(1 = ANY(SELECT 1)::text)` returns `text`, and `SELECT 2 = ANY(SELECT 1) + 1` fails with `operator does not exist: boolean + integer` (PostgreSQL 18). The docs say "The right-hand side is a parenthesized subquery": https://www.postgresql.org/docs/current/functions-subquery.html#FUNCTIONS-SUBQUERY-ANY-SOME

The Display side has the reverse problem: `SELECT a = ANY((SELECT 1)) - 1` parses as `(a = ANY(SELECT 1)) - 1` but displays as `SELECT a = ANY(SELECT 1) - 1`, which reparses as the first tree.

The subquery branch in `parse_infix` (from #1459) rewinds to the `(` and calls `parse_subexpr(precedence)`; Display for `AnyOp`/`AllOp` leaves out the parentheses when the right side is an `Expr::Subquery`.

Tested with PostgreSqlDialect on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
