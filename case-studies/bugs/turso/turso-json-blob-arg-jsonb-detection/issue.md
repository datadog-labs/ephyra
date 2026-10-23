DRAFT, not filed
Title: JSON functions take a BLOB for JSONB when json_valid(X, 8) says it isn't, and skip leading whitespace bytes before classifying it

```sql
SELECT json_valid(X'34313233', 8), json(X'34313233');
SELECT json(X'3531652B');
SELECT json(X'09');
SELECT json_extract(X'3400E2D0', '$');
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
0|123
1e+
Runtime error: Parse error: Unexpected end of input
Runtime error: Parse error: Failed to parse integer
```

SQLite 3.50.4 (trunk gives the same):
```
0|4123
Runtime error near line 2: malformed JSON
""
4
```

Expected: SQLite reads a BLOB argument as JSONB only when it passes the check in `jsonArgIsJsonb` (json.c): the header has to claim exactly the whole blob, and when the payload is at most 7 bytes and the first byte is `{`, `[` or a digit, the whole blob must also pass the strict `jsonbValidityCheck`. Anything else is cast to text and parsed as JSON text, which stops at the first NUL. So:
- `X'34313233'` is the text `4123`: as JSONB it would be an INT5 with payload `123`, which is not a valid INT5 (INT5 must be `0x...`). Turso's own `json_valid(X'34313233', 8)` says 0, yet `json()` reads it as JSONB and returns `123`.
- `X'3531652B'` is the text `51e+`, malformed. Turso reads it as a FLOAT `1e+` and returns `1e+`, which is not JSON.
- `X'09'` is a valid JSONB empty TEXT5 (`""`). Turso strips the byte as whitespace first and fails.
- `X'3400E2D0'` is the text `4` followed by a NUL. Turso reads it as an INT5 whose payload is not UTF-8 and fails with an internal-looking "Failed to parse integer".

Likely cause: `convert_ref_dbtype_to_jsonb` (core/json/mod.rs, the `ValueRef::Blob` arm, used by json(), json_extract(), `->`/`->>` and the other functions that take a JSON argument) has its own classification: it skips leading space/tab/LF/CR bytes, then accepts the blob as JSONB when the header length matches and `Jsonb::is_valid()` passes. `is_valid()` is the non-strict validation, which does not check INT5/FLOAT payloads. `looks_like_jsonb_blob` in the same file already implements SQLite's rule (used by json_error_position); using it here, without the whitespace skip, would match SQLite.

Related to #9937 (closed), where json_insert took a non-JSONB BLOB value as JSONB; this is the same classification for the JSON arguments of json(), json_extract() and the other functions.

Tested on Turso main 0f5b651c (also db441331).
