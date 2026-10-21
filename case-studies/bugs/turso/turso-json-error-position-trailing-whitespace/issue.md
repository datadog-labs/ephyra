DRAFT, not filed

Title: json_error_position('') is an internal error, and whitespace before end of input shifts the position

```sql
SELECT json_error_position('');
SELECT json_error_position(' ');
SELECT json_error_position('[1, ');
SELECT json_error_position('{"a": ');
SELECT json_error_position('/* c */');
```

| input | SQLite 3.50.4 (and trunk) | Turso |
|---|---|---|
| `''` | 1 | `Internal error: failed to determine json error position` |
| `' '` | 1 | 2 |
| `'[1, '` | 4 | 5 |
| `'{"a": '` | 6 | 7 |
| `'/* c */'` | 1 | 8 |
| `char(0)` | 1 | internal error |

Expected: https://sqlite.org/json1.html (json_error_position): the function returns the 1-based character position of the first syntax error. When the input ends where a value is expected, SQLite reports the position where that value would start, i.e. the start of the whitespace or comment before the end of input. It does not report the end of input. An empty string is an error at position 1. Cases with a real bad character after whitespace (`'[1, x'` gives 5, `'  x'` gives 3) already match.

Likely cause:
- `Jsonb::from_str_tracking` (core/json/jsonb.rs ~2515) returns `location: None` for empty input, and `json_error_position` (core/json/mod.rs ~840) maps a missing location to `InternalError`. Text cut at the first NUL is empty for `char(0)`, so the same thing happens.
- For the off-by-whitespace cases, the parser skips whitespace and comments before it notices end of input, so the reported location is past them.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
