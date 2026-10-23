Filed as #10343 (https://github.com/tursodatabase/turso/issues/10343) on 2026-10-21.
Title: JSON5 // comment ends only at LF; SQLite also ends it at CR, U+2028 and U+2029

```sql
SELECT json('[1,// c'||char(13)||'2]');
SELECT json('[1,// c'||char(0x2028)||'2]');
SELECT json('[1,// c'||char(0x2029)||'2]');
SELECT json('[1,// c'||char(10)||'2]');
```

Turso:
```
Parse error: malformed JSON
Parse error: malformed JSON
Parse error: malformed JSON
[1,2]
```
SQLite 3.50.4 (and trunk):
```
[1,2]
[1,2]
[1,2]
[1,2]
```

Expected: a JSON5 single-line comment ends at any line terminator: LF, CR, U+2028 or U+2029 (SQLite's `json5Whitespace` checks all four). #7753 made these characters count as whitespace, but the `//` comment scanner still stops only at `\n`, so the rest of the document is swallowed as comment text.

Likely cause: the `b'/'` line-comment branch of `skip_whitespace_and_comments` in `core/json/jsonb.rs` loops `while input[pos] != b'\n'`.

Tested on Turso main cbd5ac4e.
