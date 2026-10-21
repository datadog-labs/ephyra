Follow-up to #187. #205 adds the literals a pair compares a column with to that column's value domain, and that fixes the boundary pairs from #187. Two shapes still report `NO-COUNTEREXAMPLE` for pairs that PostgreSQL tells apart with a single row: the literal is compared with an expression of the column instead of the bare column, or it sits behind a cast that changes its value.

### 1. The literal is compared with an expression of the column

```sql
-- truth: not equivalent
CREATE TABLE t (c int4);
SELECT * FROM t WHERE coalesce(c, 0) <= -2;
SELECT * FROM t WHERE coalesce(c, 0) < -2;
```

`sqleq-fuzz file pair.sql` on `main` (`29c3d05`): `NO-COUNTEREXAMPLE`. On PostgreSQL the row `c = -2` separates them (1 row against 0). With the bare column (`NOT (c > -2)` against `false`), `sqleq-fuzz` reports `NOT-EQUIVALENT`.

The same happens with other expressions around the column:

| pair | witness row |
|---|---|
| `(CASE c WHEN '2001-01-13'::date THEN 1 END) = 1` vs `false` (`c date`) | `c = '2001-01-13'` |
| `NOT ((CASE WHEN false THEN 0 ELSE c END) > -2)` vs `false` (`c int4`) | `c = -2` |
| `c + 0 <= -2` vs `false` (`c int4`) | `c = -2` |

### 2. A cast that changes the value is read as the literal's text

#205 sees through casts, but it adds the inner literal's text rather than the cast's value:

```sql
-- truth: not equivalent
CREATE TABLE t (c int8);
SELECT * FROM t WHERE c < CAST('-1.7305e-12'::float8 AS int8);
SELECT * FROM t WHERE false;
```

The cast is `0`, but `-1.7305e-12` does not parse as an integer, so nothing is added and no generated row is below `0`: `NO-COUNTEREXAMPLE`. On PostgreSQL the row `c = -1` gives 1 row against 0. With `CAST('0'::float8 AS int8)` it reports `NOT-EQUIVALENT`.

```sql
-- truth: not equivalent
CREATE TABLE t (c text);
SELECT * FROM t WHERE CAST('1.7976931348623157e308'::float8 AS text) >= c;
SELECT * FROM t WHERE false;
```

The cast's value is `'1.7976931348623157e+308'`; the added text `'1.7976931348623157e308'` sorts above it, so every generated value fails the comparison: `NO-COUNTEREXAMPLE`. On PostgreSQL the row `c = '0'` gives 1 row against 0. Written as `'1.7976931348623157e+308'::text >= c` it reports `NOT-EQUIVALENT`.

**Suggestion.** Take the literal side of a comparison whatever the other side is (a column inside `coalesce`, `CASE`, arithmetic, ...) and add it to the domains of the columns that side reads; and add a cast's value rather than its operand's text, for instance by asking the engine for `SELECT <literal side>` once.

### Side note: the empty string

A third shape has no literal to harvest: `length(c) = 0` against `false` (`c text NOT NULL`), or `substr(c, -2, 3) = c` against `false`, is `NO-COUNTEREXAMPLE` because `''` is not in the text domain (`'a'`, `'b'`, `'c'`); the row `c = ''` separates them. `c = ''` against `false` is refuted, since `''` is then a literal. Adding `''` to the text domain would cover it.

Found by ephyra-sqleq (equivalent query pairs from an e-graph over PostgreSQL seeds, and one-operator mutants). Of 40 pairs that PostgreSQL showed to differ and that `sqleq-fuzz` at `c23db4d` missed, 30 are refuted at `29c3d05`; the other 10 are the shapes above (3 of shape 1, 3 of shape 2, 4 of the empty string). All runs with the default engine (PostgreSQL 17.11, C locale); witness rows checked on PostgreSQL master.
