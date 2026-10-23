DRAFT, not filed. Closed issue #6089 reports exactly this ("BLOB || BLOB returns BLOB instead of SQLite-compatible TEXT", closed as completed 2026-03-22, no linked PR). It still reproduces on main, so file it as a regression referencing #6089, or ask for #6089 to be reopened.

Title: BLOB || BLOB still returns a BLOB (#6089 reproduces on main)

```sql
SELECT typeof(x'41' || x'42'), x'41' || x'42' = 'AB';
SELECT typeof(x'' || x'');
```

Turso:
```
blob|0
blob
```
SQLite 3.50.4 (and trunk):
```
text|1
text
```

Expected: the concatenation operator returns text (https://sqlite.org/datatypes.html: "The string concatenation operator ("||") returns a text result").

Cause: `Value::exec_concat` (core/vdbe/value.rs:1272) still has the `(Value::Blob, Value::Blob)` special case that #6089 pointed at. Blob array columns take the separate `ArrayConcat` path (chosen by `expr_is_array` at translation time), so that special case isn't needed for arrays.

Effect seen in practice: `json_set(..., x'' || x'')` raises "JSON cannot hold BLOB values" in Turso, because the argument is a BLOB rather than the empty text SQLite passes.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
