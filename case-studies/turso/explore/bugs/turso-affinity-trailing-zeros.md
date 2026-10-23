# Column affinity converts `'1.0e-300'` to a different REAL than `CAST` and SQLite do

Status: found 2026-09-30 by `scripts_agree` (coverage-guided). Reported
upstream as [tursodatabase/turso#9849](https://github.com/tursodatabase/turso/issues/9849),
with the wider form turso-diff found later
([turso/bugs/turso-real-affinity-parse](../../bugs/turso-real-affinity-parse/)).

Storing text in a REAL column can produce a double one ulp away from
SQLite's, and from Turso's own `CAST` of the same text, when the significand
has trailing zeros and the exponent is negative.

## Reproduction

Print 17 significant digits: at the shell's default 15, both values render
as `1.0e-300`.

```sql
CREATE TABLE t(c REAL);
INSERT INTO t VALUES ('1.0e-300');
SELECT printf('%!.17g', c), printf('%!.17g', CAST('1.0e-300' AS REAL)),
       c = CAST('1.0e-300' AS REAL) FROM t;
```

| | stored `c` | `CAST(...)` | equal |
|---|---|---|---|
| SQLite 3.50.2 (and 3.51.0) | 9.9999999999999986e-301 | 9.9999999999999986e-301 | 1 |
| Turso 0.8.1 | **1.0e-300** | 9.9999999999999986e-301 | **0** |

`'10e-301'` and `'1.50e-300'` diverge the same way; `'1e-300'` agrees.

## Cause

`sqlite3AtoF` strips trailing zeros from the significand when the exponent
is negative, before scaling (`while( e<0 && (s%10)==0 ){ s /= 10; e++; }`).
`numeric::str_to_f64` (the `CAST` path) does this
(`core/numeric/mod.rs`, `while exponent.is_negative() && significant % 10 == 0`);
`vdbe/affinity.rs::create_result_from_significand`, reached from
`try_for_float` for column affinity, does not. It computes `10 × 10⁻³⁰¹`
instead of `1 × 10⁻³⁰⁰`, and the double-double scaling rounds differently.

## Suggested fix

Normalize in `create_result_from_significand` before the `DoubleDouble`
scaling, as `str_to_f64` does:

```rust
while exponent < 0 && significand % 10 == 0 {
    significand /= 10;
    exponent += 1;
}
```

## Versions

`turso_core` 0.8.1 through its Rust API; the loop is still missing from
`core/vdbe/affinity.rs` on `main` as of 2026-09-30. Reference: SQLite 3.50.2
(the 3.50.3 and 3.50.4 changelogs do not touch float parsing).
