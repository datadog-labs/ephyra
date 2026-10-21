DRAFT, not filed

Title: `->`/`->>` ignore the `'[N]'` abbreviated path form, so `'["a","b"]' ->> '[0]'` returns NULL

SQLite accepts three abbreviated forms on the right of `->` and `->>`: an integer (`$[N]`, or `$[#N]` when negative), a label (`$.label`), and a bracketed array index such as `'[0]'`, `'[#-1]'` or `'[2][1]'`, which is taken as `$` followed by that text. Turso handles the first two but treats `'[...]'` as an object label.

```sql
SELECT '["a","b",["c","d"]]' ->> '[0]';
SELECT '["a","b",["c","d"]]' ->> '[#-1]';
SELECT '["a","b",["c","d"]]' ->> '[2][1]';
SELECT '{"[0]":5}' ->> '[0]';
SELECT '{"a":1}' ->> '';
```

| query | SQLite 3.50.4 (and trunk) | Turso |
|---|---|---|
| `->> '[0]'` | `a` | NULL |
| `->> '[#-1]'` | `["c","d"]` | NULL |
| `->> '[2][1]'` | `d` | NULL |
| `'{"[0]":5}' ->> '[0]'` | NULL | `5` |
| `->> ''` | error: bad JSON path: '' | NULL |

`'$[#-1]'` and `'$[0]'` work in Turso; only the short form is affected.

Expected: https://sqlite.org/json1.html#the_and_operators says the right operand can be "a text label X or an integer N" and, besides those PostgreSQL forms, SQLite's `jsonExtractFunc` (3.50.4) documents and implements:

```
NUMBER   ==>  $[NUMBER]     // PG compatible  ($[#NUMBER] when negative)
LABEL    ==>  $.LABEL       // PG compatible
[NUMBER] ==>  $[NUMBER]     // Not PG.  Purely for convenience
```

The rule is: a text that starts with `[`, ends with `]`, and is at least 3 bytes long gets `$` prepended. An alphanumeric text becomes `$.text`, which is why `''` becomes `$.` and is a path error. Anything else becomes `$."text"`.

Smaller deviations in the same function (`json_path_from_db_value`, non-strict branch, core/json/mod.rs:778):
- An integer operand is cast with `i as i32`, so large values wrap: `SELECT '[1,2]' -> -4294967295;` gives `2` in Turso and NULL in SQLite, and `'[1,2]' ->> 9223372036854775807` gives `2` in Turso and NULL in SQLite.
- A BLOB operand is an error in Turso ("JSON path error near: ..."). SQLite reads it as text: `SELECT '{"a":1}' -> x'61';` gives `1`.

Likely cause: in that non-strict branch, a TEXT operand that doesn't start with `$` always becomes `Root, Key(text)`. The `[`...`]` case and the empty-text error are missing.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
