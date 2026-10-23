Filed as #9843 (https://github.com/tursodatabase/turso/issues/9843) (line 1 corrected 2026-10-21).

Title: json_patch/jsonb_patch with a repeated key in the patch apply members out of order, and jsonb_patch returns a malformed blob

When a patch object repeats a key and an earlier occurrence has an object value, Turso applies the later occurrence first. If the later one is `null` (remove), the deferred object merge then writes into the removed member. jsonb_patch produces a blob Turso can't read back, and json_patch keeps a member SQLite removed.

```sql
SELECT json(jsonb_patch('{}', '{"e":{"f":{"a":1}},"e":null}'));
SELECT json_patch('{}', '{"e":{"f":1},"e":null}');
SELECT json_patch('{"e":[1]}', '{"e":{"x":1},"e":null}');
SELECT json(jsonb_patch('{"e":[1]}', '{"e":{"0":{"a":1},"0":{}}}'));
```

| query | SQLite 3.50.4 (and trunk) | Turso |
|---|---|---|
| 1 | `{}` | error: malformed JSON |
| 2 | `{}` | `{"e":{"f":1}}` |
| 3 | `{}` | `{"e":{"x":1}}` |
| 4 | `{"e":{"0":{"a":1}}}` | `{"e":{"0":{"a":1},"0":{}}}` |

Without repeated keys the results match, as do repeated scalar keys (`json_patch('{}','{"a":1,"a":null}')` gives `{}` in both).

Expected: https://sqlite.org/json1.html (json_patch section) says json_patch implements the RFC 7396 MergePatch algorithm, which walks the patch's members in order and recurses into each object value as it reaches it. SQLite 3.50.4 does that (`jsonMergePatch`). The docs leave it open whether *json()* keeps duplicate labels, but either way a patch function must not return a malformed JSONB blob (query 1).

Likely cause: the patch loop in core/json/jsonb.rs (~3320-3365) handles an object-valued member by queueing `(key_path, value_cursor)` on `work_stack` for later. Scalar and `null` members are applied right away. Members are therefore not applied in document order. A later `"e":null` deletes `e` before its queued object merge runs, and that merge then navigates a path that no longer exists, which corrupts the blob.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
