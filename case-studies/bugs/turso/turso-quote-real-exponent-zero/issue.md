DRAFT, not filed: held by choice 2026-10-14 (low value; SQLite trunk now prints these numbers differently)

Title: quote() writes a REAL in [1, 10) that needs 16+ digits with exponent "e-00"; SQLite 3.50.4 writes "e+00"

```sql
SELECT quote(1.2345678901234567), quote(-9.473131898840627), quote(0.12345678901234567), quote(12.345678901234567);
```

Turso (`tursodb -q -m list`):

```
1.234567890123456691e-00|-9.47313189884062722e-00|1.234567890123456634e-01|1.234567890123456735e+01
```

SQLite 3.50.4:

```
1.234567890123456691e+00|-9.47313189884062722e+00|1.234567890123456634e-01|1.234567890123456735e+01
```

Expected: `e+00` for a zero exponent, as SQLite 3.50.4's `%!.20e` prints it. The value reads back the same, but the text differs (it shows up in `.dump` output and anywhere quote() text is compared). SQLite trunk now prints the shortest round-trip form here (`1.2345678901234567`), so it agrees with neither.

Likely cause: `format_float_scientific` (core/numeric/mod.rs:876) picks the sign with `if exp.is_positive() { "+" } else { "-" }`, and `0.is_positive()` is false.

Tested on Turso main f8549ca2.
