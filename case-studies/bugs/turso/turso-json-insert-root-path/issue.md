DRAFT, not filed

Title: json_insert / jsonb_insert with path '$' replace the whole document; SQLite leaves it unchanged

json_insert only adds values that don't exist yet. The root always exists, so in SQLite `json_insert(j, '$', v)` returns `j` unchanged. Turso replaces the document with `v`.

```sql
SELECT json_insert('[]', '$', 1);
SELECT json_insert('{"a":2}', '$', 1);
SELECT json_insert('[5]', '$', json('{"b":1}'));
SELECT json(jsonb_insert('[5]', '$', 1));
SELECT json_insert('{"a":2}', '$.a', 1);
```

Turso (`tursodb -q -m list`):

```
1
1
{"b":1}
1
{"a":2}
```

SQLite 3.50.4 (trunk is the same):

```
[]
{"a":2}
[5]
[5]
{"a":2}
```

Expected: the SQLite output. An existing object key (last query) is already left alone, so only the root path is affected.

Likely cause: `navigate_to_segment` (core/json/jsonb.rs ~2725) returns the `PathElement::Root` segment as a `DocumentRoot` target without checking the mode. With `PathOperationMode::InsertNew`, the root should count as an existing value and the insert should be skipped. `InsertOperation::execute` then overwrites the root value.

Tested on Turso main f8549ca2.
