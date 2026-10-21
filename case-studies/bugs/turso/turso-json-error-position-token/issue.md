Filed 2026-10-21 as https://github.com/tursodatabase/turso/issues/10378
Title: json_error_position points one character past SQLite for a bad escape and for a number with no digits after `e` or `0x`

For two kinds of malformed JSON, Turso's `json_error_position` returns a position one greater than SQLite's: a string with an invalid escape, and a number whose exponent (or hex prefix) has no digits. #10023 and #10024 (text after a complete value, keywords followed by a letter) cover other inputs; the open PRs for them (#10323, #10307) do not change these.

```sql
SELECT json_error_position('"\q"');
SELECT json_error_position('"ab\q"');
SELECT json_error_position('["a\q",1]');
SELECT json_error_position('"\x"');
SELECT json_error_position('"\uZZZZ"');
SELECT json_error_position('1e');
SELECT json_error_position('[1e]');
SELECT json_error_position('{"a":1e}');
SELECT json_error_position('[1e+]');
SELECT json_error_position('1ex');
SELECT json_error_position('[0x]');
```

| input | SQLite 3.50.4 (trunk is the same) | Turso |
|---|---|---|
| `'"\q"'` | 3 | 4 |
| `'"ab\q"'` | 5 | 6 |
| `'["a\q",1]'` | 5 | 6 |
| `'"\x"'` | 3 | 4 |
| `'"\uZZZZ"'` | 3 | 4 |
| `'1e'` | 2 | 3 |
| `'[1e]'` | 3 | 4 |
| `'{"a":1e}'` | 7 | 8 |
| `'[1e+]'` | 4 | 5 |
| `'1ex'` | 2 | 3 |
| `'[0x]'` | 3 | 4 |

Expected: SQLite's positions. For a bad escape SQLite reports the character after the backslash, the first character that cannot be part of a valid escape (json.c sets `iErr` there in the string case). For an exponent or hex prefix with no digits it reports the `e` (or the sign after it, or the `x`), the start of the part that is malformed, as #10024 does for keywords. Turso reports the character after it in both cases.

A related difference that #10323 does not change: `json_error_position('0x')` is 1 in SQLite (it stops after `0` and reports the trailing text as #10023 describes) and 3 in Turso.

Tested on Turso main fe4beb8 (2026-10-21), also with #10323 and #10307 applied; SQLite 3.50.4 and trunk fa0247a431bd (2026-10-21).
