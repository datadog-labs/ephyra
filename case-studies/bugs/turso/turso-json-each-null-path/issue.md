DRAFT, not filed

Title: json_each/json_tree with a NULL path error "root path should be text" (SQLite returns no rows)

```sql
SELECT count(*) FROM json_each('[1,2]', NULL);
SELECT count(*) FROM json_tree('[1,2]', NULL);
SELECT count(*) FROM (SELECT NULL AS p) JOIN json_each('[1,2]', p);
```

Turso:
```
Runtime error: root path should be text
```
SQLite 3.50.4 (and trunk): `0` for each.

Expected: a NULL PATH gives an empty result, the same as a NULL JSON argument (which Turso already handles). In SQLite 3.50.4's `jsonEachFilter`, the JSON is parsed first (so `json_each('not json', NULL)` is still "malformed JSON", which Turso matches), then `zRoot = sqlite3_value_text(argv[1]); if( zRoot==0 ) return SQLITE_OK;`. A non-NULL, non-text path is converted to text and must start with `$`, so `json_each('[1]', 1)` is "bad JSON path: '1'" in SQLite. That one is also an error in Turso, only with a different message.

Likely cause: `JsonEachCursor::filter` (core/json/vtab.rs:281) requires `args[1]` to be `Value::Text` and returns `InvalidArgument("root path should be text")` for anything else, NULL included. NULL should return `Ok(false)` after the JSON is parsed.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
