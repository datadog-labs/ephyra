# DuckDB: `f(a == b)` is displayed as `f(a = b)`, which reparses as a named argument

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2808 on 2026-11-02. The text below is the issue as posted.

---

`==` is parsed as `BinaryOperator::Eq` and displayed as `=`. Inside a function call with DuckDbDialect, `a = b` is a named argument (`supports_named_fn_args_with_eq_operator`, #1182), so the output means something else:

```
SELECT coalesce(a == b) FROM t
  -> AST arg: Unnamed(BinaryOp { a Eq b })
  -> displays as: SELECT coalesce(a = b) FROM t
  -> reparse arg: Named { name: a, arg: b, operator: Equals }
SELECT a == b FROM t
  -> displays as SELECT a = b FROM t, same AST (fine)
```

DuckDB documents `=` and `==` as equal (https://duckdb.org/docs/stable/sql/expressions/comparison_operators) and evaluates `coalesce(a == b)` as a comparison. This may count as accepted normalisation; the only effect is the AST change inside call arguments.

Tested on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
