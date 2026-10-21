Filed 2026-10-21 as https://github.com/datadog-labs/sqleq/issues/207 (shapes A and D/E; the empty string as a side note). Text: github-title.txt, github-body.md.

# sqleq-fuzz: 40 "fuzz-missed" pairs from reval-20261021, against current main

Status: triage only. Nothing filed, commented or posted.

## What was tested

- Pairs: the 40 `fuzz-missed` rows of `ephyra-sqleq/out/reval-20261021/{onestep,egraph}/results.tsv`
  (28 onestep, 12 egraph). A pair is fuzz-missed when PostgreSQL shows the two queries differ on the
  case's data and `sqleq-fuzz file` says `NO-COUNTEREXAMPLE`.
- sqleq main at **29c3d05** (2026-10-16, "Merge pull request #205 from datadog-labs/fuzz-boundary-literals"),
  cloned to `/tmp/sqleq-triage/sqleq`. Only `sqleq-fuzz` and `sqleq-frontend` were built (release).
- Baseline: the c23db4d binary in `../sqleq-main/target/release` (the build the run used).
- Both binaries run with their defaults (`--trials 120 --rows 5 --seed 0`, Postgres 17.11 engine,
  `--locale=C`). Two runs of the new binary gave the same answers.

Per-pair results: [rerun.tsv](rerun.tsv).

## Split

| | pairs |
|---|---|
| fixed upstream (c23db4d NO-COUNTEREXAMPLE, 29c3d05 NOT-EQUIVALENT) | 30 |
| still NO-COUNTEREXAMPLE on 29c3d05 | 10 |
| error | 0 |

The 30 fixed ones are boundary changes against a literal compared with a bare column (`c < -4` vs
`-4 >= c`, `c7 >= '0b'` vs `'0b' < c7`, IN-list vs OR chain, etc.): the shape of #187, fixed by
PR #205 ("draw the literals a pair compares its columns with, and their neighbours", commit a368602).

## The 10 still missed, by cause

PR #205 harvests a literal only when it sits directly opposite a **bare column reference** (looking
through parentheses and casts), and adds the literal's **own text** to that column's domain. The 10
remaining pairs fall outside that in three ways.

### A. The literal is compared with an expression of the column, not the column (3 pairs)

Pairs: [onestep--s43_q0_mut2](originals/onestep--s43_q0_mut2.sql),
[egraph--s43_q0_mut2](originals/egraph--s43_q0_mut2.sql) (`NOT (CASE WHEN 0 IS NULL THEN 0 ELSE c4 END > -2)`),
[egraph--s6_q1_mut3](originals/egraph--s6_q1_mut3.sql) (`CASE c0 WHEN '2001-01-13'::date THEN ...`).

Minimal ([A1_coalesce.sql](repros/A1_coalesce.sql); also [A2_case_operand.sql](repros/A2_case_operand.sql),
[A3_case_searched.sql](repros/A3_case_searched.sql), [A4_arith.sql](repros/A4_arith.sql)):

```sql
CREATE TABLE t (c int4);
SELECT * FROM t WHERE coalesce(c, 0) <= -2;
SELECT * FROM t WHERE coalesce(c, 0) < -2;
-- PostgreSQL, row c = -2: query 1 returns 1 row, query 2 returns 0
-- sqleq-fuzz 29c3d05: NO-COUNTEREXAMPLE
```

```sql
CREATE TABLE t (c date);
SELECT * FROM t WHERE (CASE c WHEN '2001-01-13'::date THEN 1 END) = 1;
SELECT * FROM t WHERE false;
-- PostgreSQL, row c = '2001-01-13': 1 row vs 0
-- sqleq-fuzz 29c3d05: NO-COUNTEREXAMPLE
```

`c + 0 <= -2` and `NOT ((CASE WHEN false THEN 0 ELSE c END) > -2)` against `false` are also
NO-COUNTEREXAMPLE. The bare control `NOT (c > -2)` against `false`
([A0_bare_control.sql](repros/A0_bare_control.sql)) is refuted at once (`t=[('-1'); (NULL); ('1'); ('-3'); ('-2')]`).
A simple `CASE c WHEN lit` is an equality with `c` that the harvest does not visit at all.

Original witnesses (PostgreSQL on ephyra-postgresql's cluster):
- s43 (both): `INSERT INTO t1 VALUES (NULL,0,0,NULL,-2)`: query 1 0 rows, query 2 `(0, 0, -3)`.
- s6: `INSERT INTO t0 VALUES ('2001-01-13',1)`: query 1 0 rows, query 2 `(2000-04-05, 2001-01-13)`.

### D/E. A cast that changes the literal's value is treated as transparent (3 pairs)

Pairs: [onestep--s16_q2_mut2](originals/onestep--s16_q2_mut2.sql),
[onestep--s92_q2_mut0](originals/onestep--s92_q2_mut0.sql), [onestep--s92_q2_mut2](originals/onestep--s92_q2_mut2.sql).

PR #205 looks through casts and keeps the inner literal's text. When the cast changes the value,
that text is not the value compared:

```sql
-- D: repros/D_cast_float_to_int.sql
CREATE TABLE t (c int8);
SELECT * FROM t WHERE c < CAST('-1.7305e-12'::float8 AS int8);   -- the cast is 0
SELECT * FROM t WHERE false;
-- PostgreSQL, row c = -1: 1 row vs 0
-- sqleq-fuzz 29c3d05: NO-COUNTEREXAMPLE
-- harvested text '-1.7305e-12' does not parse as an integer, so c gets nothing;
-- with CAST('0'::float8 AS int8) (D0_control.sql) it is refuted: t=[('1'); (NULL); ('1'); ('-1'); ('0')]
```

```sql
-- E: repros/E_cast_float_to_text.sql
CREATE TABLE t (c text);
SELECT * FROM t WHERE CAST('1.7976931348623157e308'::float8 AS text) >= c;  -- '1.7976931348623157e+308'
SELECT * FROM t WHERE false;
-- PostgreSQL, row c = '0': 1 row vs 0
-- sqleq-fuzz 29c3d05: NO-COUNTEREXAMPLE
-- harvested text '1.7976931348623157e308' sorts above '...e+308' (C locale), so it does not satisfy >=;
-- with the literal written as '1.7976931348623157e+308'::text (E0_control.sql) it is refuted.
```

Original witnesses:
- s16_q2_mut2 (`c0 < CAST(...)` OR false, against the same AND false): `INSERT INTO t0 VALUES (-1,NULL)`: query 1 1 row, query 2 0.
- s92_q2_mut0/mut2 (join `a0_2.c3 = a0_0.c0` vs `<>`, under `'1.7976931348623157e+308' >= a0_2.c0`):
  `INSERT INTO t4 VALUES (1); INSERT INTO t0 VALUES ('0',0,0,2)`: query 1 0 rows, query 2 `(2, -852)`.
  With `t0.c0 = 'a'` or `= '1.7976931348623157e308'` (the harvested text) both sides are empty.

### C. The empty string is never drawn (4 pairs)

Pairs: [onestep--s78_q0_mut0](originals/onestep--s78_q0_mut0.sql), [onestep--s78_q0_mut1](originals/onestep--s78_q0_mut1.sql),
[egraph--s78_q0_mut0](originals/egraph--s78_q0_mut0.sql), [egraph--s78_q0_mut3](originals/egraph--s78_q0_mut3.sql).
All four drop a `DISTINCT` over `count(*) ... GROUP BY`; the `WHERE` is
`greatest(substr(c3, -2, 3), '') IN (SELECT a0_0.c3 ...)`, and `substr(x, -2, 3)` is always `''`, so a
row passes only when `t1.c3 = ''`. The text domain is `'a','b','c'`; `''` appears only as an argument
of `greatest`, which nothing harvests.

```sql
-- repros/C_empty_string.sql
CREATE TABLE t (c text NOT NULL);
SELECT * FROM t WHERE substr(c, -2, 3) = c;
SELECT * FROM t WHERE false;
-- PostgreSQL, row c = '': 1 row vs 0
-- sqleq-fuzz 29c3d05: NO-COUNTEREXAMPLE   (also length(c) = 0 vs false: C2_empty_string_plain.sql)
-- c = '' vs false (C3_empty_literal.sql) is refuted, through the literal harvest.
```

Original witness: `INSERT INTO t1 VALUES ('\x00',NULL,NULL,''); INSERT INTO t1 VALUES ('\x00','2000-01-01',NULL,''); INSERT INTO t5 (c0,c3) VALUES ('\x01',1)`:
query 1 one row `(1, NULL)`, query 2 two rows `(1, NULL)`. With `c3 = 'a'` both are empty.

## Duplicate search

`gh issue list --search` and `gh api search/issues` return nothing on this repository (the search API
answers 422 for it), so all 82 issues and 124 PRs were listed with bodies and grepped
(`coalesce`, `CASE`, `empty string`, `''`, `literal`, `domain`, `harvest`, `neighbour`, `negative`).
The only literal-domain items are #187 (closed 2026-10-16) and its fix PR #205. #205's body and
the merged `sqleq-fuzz/src/literals.rs` cover bare columns only ("A column reference counts under
parentheses or a cast"); no issue or PR mentions a literal against a wrapped column, a value-changing
cast, or the empty string.

## Verdicts

| group | pairs | verdict |
|---|---|---|
| fixed | 30 | fixed upstream by #205 (#187) |
| A: literal vs expression of a column (COALESCE, CASE, simple CASE WHEN, arithmetic) | 3 | new, follow-up to #187: same motivation, outside #205's harvest. The README now promises literals "the two statements compare a column with", so by its letter this is a gap in reach, not a broken promise. Worth reporting as an extension. |
| D/E: value-changing cast read as its inner literal's text | 3 | new, a defect in #205's own design: it says it sees through casts, but treats `CAST(float AS int8)` and `CAST(float8 AS text)` as preserving the value. Low practical weight (generator-shaped constants). Could be fixed by asking Postgres for the value of the literal side (`SELECT <expr>`) instead of reading its text. |
| C: no empty string in the text domain | 4 | limitation: no literal is compared with the column, and the documented domain is `'a','b','c'`. Suggesting `''` as a domain value is reasonable, since it is a classic edge value. Not a bug report on its own. |

A and D/E could go in one follow-up to #187: "literals reached through an expression or a
value-changing cast are not drawn".

## Reproducing

- `sqleq-fuzz file repros/<name>.sql` with the 29c3d05 binary (`/tmp/sqleq-triage/sqleq/target/release/sqleq-fuzz`).
- [witness.sh](witness.sh) `<pair.sql> '<INSERT ...>' ...` runs the DDL, the inserts and both queries
  in a rolled-back transaction on ephyra-postgresql's cluster, in a database `sqleqtriage`
  (dropped after this triage; create it with `CREATE DATABASE sqleqtriage TEMPLATE template0 LOCALE 'C'`).
