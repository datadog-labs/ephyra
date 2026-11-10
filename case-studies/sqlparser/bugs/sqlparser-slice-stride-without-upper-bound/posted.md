# Slice with a stride and no upper bound (`arr[1: :4]`) is displayed as `arr[1::4]`, which doesn't parse

Posted as https://github.com/apache/datafusion-sqlparser-rs/issues/2807 on 2026-11-02. The text below is the issue as posted.

---

`Subscript::Slice` with `upper_bound: None` and a stride prints the two colons next to each other, and the tokenizer then reads `::` (cast):

```
SELECT ([1, 2, 3, 4, 5, 6, 7, 8, 9])[1: :4]
  -> displays as: SELECT ([1, 2, 3, 4, 5, 6, 7, 8, 9])[1::4]
  -> reparse: Expected: a data type name, found: 4 at Line: 1, Column: 41
SELECT ([1, 2, 3, 4, 5, 6, 7, 8, 9])[: :4]
  -> displays as: SELECT ([1, 2, 3, 4, 5, 6, 7, 8, 9])[::4]
  -> reparse: Expected: an expression, found: :: at Line: 1, Column: 38
SELECT ([1, 2, 3, 4, 5, 6, 7, 8, 9])[1:-:4]
  -> round-trips (fine)
```

DuckDB documents `[1:-:4]` for this (https://duckdb.org/docs/stable/sql/functions/list#slicing), but it also runs `[1: :4]` (returns `[1, 5, 9]`), and the parser accepts it. Something like `[1: :4]` in Display would round-trip.

Tested with DuckDbDialect on main ce12f1f6 (also 0.63.0 and 0.58.0).

Found with a property-based fuzzer (round trip check); I used AI assistance for triage and verified every case above.
