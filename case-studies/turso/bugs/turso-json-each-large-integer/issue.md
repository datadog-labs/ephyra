Filed as #9839 (https://github.com/tursodatabase/turso/issues/9839) (line 1 corrected 2026-10-21).

Title: json_each/json_tree error "number too large to fit in target type" on integers outside the i64 range

```sql
SELECT value FROM json_each('[9223372036854775808]');
SELECT atom FROM json_tree('123456789012345678901234567890');
SELECT value FROM json_each('-9223372036854775809');
```

Turso:
```
Runtime error: Parse error: number too large to fit in target type
Runtime error: Parse error: number too large to fit in target type
Runtime error: Parse error: number too small to fit in target type
```

SQLite 3.50.4:
```
9.22337203685478e+18
1.23456789012346e+29
-9.22337203685478e+18
```
(trunk prints the same reals with 17 significant digits.)

`type` is `integer` in both engines. Only `value` and `atom` fail. Other JSON functions already convert such literals to REAL in Turso: `SELECT json_extract('9223372036854775808','$'), '[9223372036854775808]' ->> 0;` gives `9.22337203685478e+18` twice, like SQLite.

Expected: the same REAL value that json_extract returns for that element. https://sqlite.org/json1.html describes json_each's `value`/`atom` as the SQL value of the element, the same conversion json_extract applies.

Likely cause: `jsonb_to_integer` in core/json/vtab.rs (~line 622) does `string.parse::<i64>()?` on the element text for `ElementType::INT`/`INT5`. A digit string outside i64 should fall back to f64, as `json_string_to_db_type` does.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
