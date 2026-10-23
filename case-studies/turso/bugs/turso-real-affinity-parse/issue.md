Filed as #9849 (https://github.com/tursodatabase/turso/issues/9849) (line 1 corrected 2026-10-21).

# Text-to-REAL conversion through column affinity and sum()/total()/avg() gives a different double than CAST or a literal

Turso has two decimal-to-double parsers. `CAST(text AS REAL)` and SQL literals use `numeric::str_to_f64`, which matches SQLite 3.50.4's `sqlite3AtoF` bit for bit. REAL/NUMERIC column affinity (`apply_numeric_affinity`) and the text inputs of `sum()`, `total()` and `avg()` use `vdbe::affinity::try_for_float`. For large or small exponents, that parser returns a double one ulp or more away. As a result the same text maps to two different values, and equality checks against the stored value fail.

## Reproduction

```sql
CREATE TABLE r(x REAL);
INSERT INTO r VALUES ('2e218'), (2e218);
SELECT 'q1', x = 2e218, x = CAST('2e218' AS REAL) FROM r;
SELECT 'q2', count(DISTINCT x) FROM r;
SELECT 'q3', sum('2e218') = 2e218, total('2e218') = 2e218, avg('2e218') = 2e218;
SELECT 'q4', sum('1.0e-300') = CAST('1.0e-300' AS REAL);
```

SQLite 3.50.4 (trunk f69fd5f5 gives the same output):
```
q1|1|1
q1|1|1
q2|1
q3|1|1|1
q4|1
```

Turso:
```
q1|0|0
q1|1|1
q2|2
q3|0|0|0
q4|0
```

Expected: SQLite converts text to a number with the same routine (`sqlite3AtoF`) for CAST, literals, column affinity and aggregate inputs. The text '2e218' stored in a REAL column therefore equals the literal 2e218 (https://sqlite.org/datatype3.html#type_affinity), and `sum('2e218')` returns that same value.

Scope: 20,000 random decimal strings were inserted into a REAL column by Turso, and the file was read back with sqlite3 3.50.4. In 539 cases the stored value differs from `CAST(s AS REAL)`, for example `43833.5411857880e249` and `3.158929622800560e-100`. All mismatches have a decimal exponent of roughly |e| >= 99. With exponents in [-60, 60] there were 0 mismatches in 30,000 strings. Turso's own `CAST(s AS REAL)` agreed with SQLite 3.50.4 in all 20,000 cases.

## Likely cause

`try_for_float` / `create_result_from_significand` (core/vdbe/affinity.rs ~379/529) feed the raw significand and exponent straight into the DoubleDouble scaling. `str_to_f64` (core/numeric/mod.rs ~589), like `sqlite3AtoF`, first normalizes: it multiplies the significand by 10 while the exponent is positive and the significand is below MAX_EXACT/10, and strips trailing zeros while the exponent is negative. Without that step the products round differently, e.g. `2e218` (s=2, e=218) and `1.0e-300` (s=10, e=-301).

Tested on Turso main 7641fad2 against SQLite 3.50.4.
