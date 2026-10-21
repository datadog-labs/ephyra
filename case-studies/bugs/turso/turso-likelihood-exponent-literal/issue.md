Filed as #10338 (https://github.com/tursodatabase/turso/issues/10338) on 2026-10-21.
Title: likelihood() rejects a float literal written with an exponent (625e-4, 1e0) or in parentheses

```sql
SELECT likelihood(1, 625e-4);
SELECT likelihood(1, 1e0);
SELECT likelihood(1, (0.5));
```

Turso:
```
Parse error: second argument to likelihood() must be a floating point number with decimal point
Parse error: second argument to likelihood() must be a floating point number with decimal point
Parse error: second argument to likelihood() must be a constant between 0.0 and 1.0
```
SQLite 3.50.4 (and trunk):
```
1
1
1
```

Expected: SQLite accepts any floating-point literal between 0.0 and 1.0 as the second argument, including the exponent form (`625e-4`, `1e0`) and a parenthesized literal. It rejects only non-float tokens (`1`, `'0.5'`, `0x1`, `+0.5`, `0.5+0`), so the integer case Turso already rejects stays an error.

Likely cause: the `ScalarFunc::Likelihood` branch in `core/translate/expr/translator.rs` requires the numeric literal text to contain a `.` and does not look through parentheses.

Tested on Turso main cbd5ac4e.
