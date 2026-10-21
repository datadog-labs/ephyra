`sqleq-fuzz` draws column values from a small fixed domain (`0, 1, 2` for numbers, three dates, ...) and, as its README says, does not add the queries' literals to it. So two queries that differ only at a literal's boundary are never told apart: no generated row equals the literal, both sides agree on every trial, and a non-equivalent pair is reported `NO-COUNTEREXAMPLE`.

```sql
-- truth: not equivalent
CREATE TABLE t (c int8 NOT NULL);
SELECT * FROM t WHERE 334 <= c;
SELECT * FROM t WHERE 334 < c;
```

`sqleq-fuzz file pair.sql` on `main` (`bbb86f3`): `NO-COUNTEREXAMPLE`, also with `--trials 2000 --rows 8`. With `1` in place of `334` (inside the domain) it reports `NOT-EQUIVALENT` at once. On PostgreSQL the single row `c = 334` separates them (1 row against 0).

The same happens with other types and with a constant off by one:

| pair | witness row |
|---|---|
| `c1 > '2000-01-01'::date` vs `c1 >= '2000-01-01'::date` | `c1 = '2000-01-01'` |
| `c0 >= '-820'::numeric` vs `c0 > '-820'::numeric` | `c0 = -820` |
| `334 <= c1` vs `335 <= c1` (int8) | `c1 = 334` |

This is the documented limitation in `sqleq-fuzz/README.md` ("The generated value domain"), which suggests writing pairs against the domain. That works for hand-written pairs, but not for pairs that come from real queries or a generator, where constants are whatever the query names; and boundary changes (`<` vs `<=`, an off-by-one constant) are among the most common wrong rewrites.

**Suggestion.** Add each literal a column is compared with (and, for integers and dates, its neighbours `k-1` and `k+1`) to that column's value domain, as parameters are already biased toward values a column holds. The domain stays small (a few values per literal), so the collision behaviour the small domain exists for is kept.

Found by ephyra-sqleq's near-miss pairs (one deliberate semantic change each): of 28 such pairs that PostgreSQL showed to differ, `sqleq-fuzz` reported 12 as `NO-COUNTEREXAMPLE`, 10 of them a boundary literal outside the domain; each of those reproduces with a single witness row.
