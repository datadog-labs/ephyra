# `._name` after `)` or `]` fails with "Unexpected character '_'"

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2803 on 2026-11-02. The text below is the issue as posted.

---

Field access with a name starting with `_` fails to tokenize when it follows `)` or `]`:

```
SELECT (c)._x FROM tt            (PostgreSqlDialect)
  -> sql parser error: Unexpected character '_' at Line: 1, Column: 11
SELECT ({'_x': 1})._x            (DuckDbDialect)
  -> Unexpected character '_' at Line: 1, Column: 19
SELECT ([{'_a': 2}])[1]._a       (DuckDbDialect)
  -> Unexpected character '_' at Line: 1, Column: 24
SELECT f()._x                    (BigQueryDialect)
  -> Unexpected character '_' at Line: 1, Column: 11
SELECT (c).x FROM tt, SELECT t._x FROM t
  -> parse (fine)
```

PostgreSQL 18 runs `SELECT (c)._x FROM tt` for a composite column with a field `_x` (https://www.postgresql.org/docs/current/rowtypes.html#ROWTYPES-ACCESSING), and DuckDB runs `SELECT ({'_x': 1})._x, ([{'_a': 2}])[1]._a` (returns `1 | 2`).

In `Tokenizer::next_token`, the `._` check only accepts a preceding `Token::Word`; after `RParen` or `RBracket` it errors. The block right after it already treats `.` after `Word | RParen | RBracket | Number` as a period when `supports_tuple_element_access()`. Related: #1835 (added the check), #2677 (whitespace case).

Tested on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
