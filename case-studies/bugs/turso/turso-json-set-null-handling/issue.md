DRAFT, not filed

Title: json_set/json_insert/json_replace/json_remove: a JSON `null` result becomes SQL NULL, and NULL arguments aren't handled like SQLite (follow-up to #5750)

#5750 asked that these functions return SQL NULL when the *first argument* is SQL NULL. The current code does something else: it returns SQL NULL whenever the *resulting document* is JSON `null`, and still evaluates SQL-NULL input as the JSON `null` literal. Both directions are now wrong, and NULL path arguments are handled differently from SQLite too.

```sql
SELECT quote(json_set('null', '$.a', 1));      -- 1
SELECT quote(json_replace('{}', '$', NULL));   -- 2
SELECT quote(jsonb_set('[1]', '$', NULL));     -- 3
SELECT quote(json_set(NULL, '$', 5));          -- 4
SELECT quote(json_set('[1]', NULL, x'ff'));    -- 5
SELECT quote(json_remove('[1]', NULL));        -- 6
```

| # | SQLite 3.50.4 (and trunk) | Turso |
|---|---|---|
| 1 | `'null'` | `NULL` |
| 2 | `'null'` | `NULL` |
| 3 | `X'00'` | `NULL` |
| 4 | `NULL` | `'5'` |
| 5 | `'[1]'` | error: JSON cannot hold BLOB values |
| 6 | `NULL` | `'[1]'` |

The same applies to json_insert/json_replace/json_remove and the jsonb_ variants: `json_insert('null','$.a',1)`, `json_remove('null','$.a')` and `json_remove('null')` are `'null'` in SQLite and NULL in Turso. `json_replace(NULL,'$',1)` and `json_set(NULL,'$',json('[1]'))` return a value in Turso and NULL in SQLite.

Expected (SQLite 3.50.4 `jsonInsertIntoBlob` / `jsonRemoveFunc`):
1. A SQL NULL first argument gives SQL NULL, whatever the paths are.
2. Otherwise the result is the edited document, so a document that is JSON `null` comes back as the text `'null'` (`X'00'` for jsonb_*). Only `json_remove(j, '$')` returns SQL NULL.
3. In json_set/insert/replace a PATH/VALUE pair whose PATH is SQL NULL is skipped entirely, so its VALUE isn't converted or checked: no BLOB error.
4. In json_remove a SQL NULL path makes the whole result SQL NULL.

Likely cause: `json_string_to_db_type` (core/json/mod.rs:683) returns `Value::Null` for any `ElementType::NULL`, even with `OutputVariant::String`/`Binary`, which json_set, jsonb_set and the ops.rs functions use for their result. The NULL first argument is never checked before `json_cache.get_or_insert_with`. In `json_set` (mod.rs ~431) `ensure_blob_arg_is_jsonb(second)` runs before the path is known to be NULL. `json_remove` (core/json/ops.rs:87) skips NULL paths instead of returning NULL.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
