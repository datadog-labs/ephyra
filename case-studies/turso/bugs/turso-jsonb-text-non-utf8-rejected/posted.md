# JSON functions classify BLOB arguments differently from SQLite: some blobs json_valid(X, 8) rejects are read as JSONB, and JSONB whose text is not UTF-8 is rejected

Posted as https://github.com/tursodatabase/turso/issues/10907 on 2026-10-27 (one issue for this bug and turso-json-blob-arg-jsonb-detection). The text below is the issue as posted.

---

Two problems in how the JSON functions decide whether a BLOB argument is JSONB. Both come from the classification in `convert_ref_dbtype_to_jsonb` (core/json/mod.rs), so they are in one issue.

## 1. Blobs that are not JSONB are read as JSONB, after leading whitespace bytes are skipped

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

## 2. JSONB whose text element is not UTF-8 is rejected, though json_valid(X, 8) accepts it

```sql
SELECT json_valid(X'47E221502E', 8), json_valid(X'29999A', 8);
SELECT json_type(X'47E221502E');
SELECT X'47E221502E' -> '$' IS NOT NULL;
SELECT json_extract(X'29999A', '$') IS NOT NULL;
SELECT json_array_length(X'29999A');
SELECT count(*) FROM json_each(X'47E221502E');
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
1|1
Runtime error: Parse error: malformed JSON
Runtime error: Parse error: malformed JSON
Runtime error: Parse error: malformed JSON
Runtime error: Parse error: malformed JSON
Runtime error: Parse error: malformed JSON
```

SQLite 3.50.4 (trunk gives the same):
```
1|1
text
1
1
0
1
```

`X'47E221502E'` is a JSONB TEXT element (header 0x47: type TEXT, 4-byte payload) and `X'29999A'` is a TEXT5 element (header 0x29: 2-byte payload). Their payloads are not valid UTF-8. SQLite's strict JSONB check works on bytes, so `json_valid(X, 8)` is 1 in both engines. SQLite's other JSON functions then read the blob as JSONB. Turso's `json_valid` agrees with SQLite, but every other function rejects the blob with "malformed JSON".

SQLite can write such a blob itself:

```sql
-- in sqlite3
CREATE TABLE t(j);
INSERT INTO t VALUES (jsonb_array(CAST(X'E221' AS TEXT), 1));
-- then in tursodb, on the same file
SELECT json_array_length(j), j ->> 1 FROM t;
```

SQLite 3.50.4 returns `2|1`. Turso fails with "malformed JSON". The bad bytes sit in an element the query doesn't read.

Expected: the result SQLite gives. Or, if Turso can't hold such text, the U+FFFD substitution that COMPAT.md describes for text made from bytes that are not UTF-8, not an error. In either case, `json_valid(X, 8)` and the other JSON functions should agree on what counts as JSONB.

Likely cause: `convert_ref_dbtype_to_jsonb` (core/json/mod.rs, `ValueRef::Blob` arm) accepts a blob as JSONB only when `Jsonb::is_valid()` passes. That check is `validate_element(.., strict = false)` (core/json/jsonb.rs). For TEXT/TEXTJ/TEXT5/TEXTRAW it requires the payload to be valid UTF-8 ("The lenient document check is stricter than SQLite here: later readers decode text payloads as &str"). When the check fails, the blob is parsed as JSON text instead, and that parse fails with "malformed JSON". `json_valid(X, 8)` uses the strict path (`jsonb_error_position`), which accepts any bytes in text payloads, as SQLite does.

In both cases `json_valid(X, 8)` (the strict path) agrees with SQLite and the other JSON functions do not. Tested on Turso main 222c39296 against SQLite 3.50.4 and trunk.
