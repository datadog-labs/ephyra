# pgsql-bugs report: btree_gist date opclass vs infinite dates

Status: submitted 2026-10-15 as BUG #19761, https://www.postgresql.org/message-id/19761-b72cfc453e487610%40postgresql.org

## Form fields

- PostgreSQL version: choose `18.6` (the body names master and 19rc1)
- Operating system: `macOS 26 arm64 (also Ubuntu 24.04 x86_64)`

## Subject

btree_gist: "cannot subtract infinite dates" when inserting 'infinity' into a GiST index on date

## Body (plain text)

With contrib/btree_gist, a GiST index on a date column rejects the values
'infinity' and '-infinity' once the index has more than one page:

  ERROR:  22008: cannot subtract infinite dates
  LOCATION:  date_mi, date.c:556

Inserting finite dates also fails once the index holds an infinity, and so
does a KNN search (ORDER BY d <-> ...) over such an index. The timestamp and
timestamptz opclasses handle all of these cases.

Versions tested (both behave the same):
  - master at 158b6d5109f (PostgreSQL 20devel), --enable-cassert --enable-debug
  - REL_18_STABLE at a85b5523e67 (reports 18.6)
Platform: macOS 26 (Darwin 25.6.0), Apple M-series arm64, Apple clang 21.0.0.
Rechecked on 2026-10-15 (Ubuntu 24.04, x86_64, gcc 13.3, --enable-cassert
--enable-debug, server under ulimit -v 6 GB): the same five errors on
master aa7b9cf132b, 19rc1 (REL_19_STABLE 0fb192f1b02) and REL_18_STABLE
4d62ef92fda.
Fresh initdb, default configuration except fsync=off; empty ~/.psqlrc.

Script:

\set VERBOSITY verbose
CREATE EXTENSION btree_gist;
CREATE TABLE t (d date);
CREATE INDEX t_d_idx ON t USING gist (d);
-- about 410+ rows are enough for the index to grow past one page
INSERT INTO t SELECT date '2000-01-01' + i FROM generate_series(1, 500) i;
INSERT INTO t VALUES ('infinity');
INSERT INTO t VALUES ('-infinity');
-- an index built over rows that already hold infinities (sorted build)
-- succeeds, but later inserts and KNN searches fail
CREATE TABLE u (d date);
INSERT INTO u SELECT date '2000-01-01' + i FROM generate_series(1, 500) i;
INSERT INTO u VALUES ('infinity'), ('-infinity');
CREATE INDEX u_d_idx ON u USING gist (d);
INSERT INTO u VALUES ('2000-06-01');
SET enable_seqscan = off;
SELECT d FROM u ORDER BY d <-> date '2000-06-01' LIMIT 1;
-- the scalar operator rejects infinity as well
SELECT date 'infinity' <-> date '2000-01-01';
-- for comparison, timestamp handles all of this
CREATE TABLE ts (x timestamp);
CREATE INDEX ts_x_idx ON ts USING gist (x);
INSERT INTO ts SELECT timestamp '2000-01-01' + i * interval '1 day' FROM generate_series(1, 500) i;
INSERT INTO ts VALUES ('infinity'), ('-infinity');
SELECT timestamp 'infinity' <-> timestamp '2000-01-01';

Actual output (master; 18 is identical except date.c:560):

INSERT 0 500
ERROR:  22008: cannot subtract infinite dates      <- INSERT 'infinity'
ERROR:  22008: cannot subtract infinite dates      <- INSERT '-infinity'
CREATE INDEX                                       <- u_d_idx builds fine
ERROR:  22008: cannot subtract infinite dates      <- INSERT '2000-06-01' into u
ERROR:  22008: cannot subtract infinite dates      <- ORDER BY d <-> ... LIMIT 1
ERROR:  22008: cannot subtract infinite dates      <- date 'infinity' <-> date
INSERT 0 2                                         <- timestamp infinities OK
 infinity                                          <- timestamp <-> result

Expected output: the inserts succeed, the KNN query returns 2000-06-01, and
date <-> returns a saturated value instead of an error, as it does for
timestamp. The table accepts these values without the index, and so does a
btree index.

UPDATE ... SET d = 'infinity' and COPY fail the same way. So does
CREATE INDEX ... WITH (buffering = on) over a table that holds infinities,
since that path does not use the sorted build. Before the sorted build for
btree_gist (e4309f73f69, v18), I believe a plain CREATE INDEX over such a
table would fail as well, but I did not test older branches.

Cause: contrib/btree_gist/btree_date.c computes key differences with
DirectFunctionCall2(date_mi, ...). Since date got infinities in 8.4
(a303e4dc431), date_mi raises an error for infinite input
(src/backend/utils/adt/date.c:553-556). The calls are:
  - gbt_date_penalty, lines 212, 218, 228. gistchoose calls it on every
    insert once the root is an internal page, which is why a single-page
    index does not show the problem. Backtrace (backtrace_functions =
    'date_mi'): date_mi <- gbt_date_penalty <- gistchoose <- gistdoinsert.
  - gdb_date_dist, line 95 (KNN distance, via gbt_date_distance and
    gistScanPage).
  - date_dist, line 123 (the date <-> date operator).
btree_ts.c avoids this. gbt_ts_penalty does its own double arithmetic on the
raw int64 values, gbt_ts_dist returns get_float8_infinity() for non-finite
input, and ts_dist/tstz_dist return a saturated interval.

Fix idea: do the same in btree_date.c:
  - gbt_date_penalty: compute the differences as double on the raw DateADT
    values (DATEVAL_NOBEGIN/NOEND = INT32_MIN/MAX then act as very distant
    dates). This also avoids the int32 overflow of "res".
  - gdb_date_dist: return get_float8_infinity() if either input is not
    finite.
  - date_dist: return INT_MAX if either input is not finite, like ts_dist (or
    keep the error for the scalar operator; the index paths must not raise
    it).
I tried this on master (about 30 changed lines, attached). The script then
succeeds. On a 20000-row table with 2% infinities, index and seqscan counts
agree, and the btree_gist regression tests (32) pass. This is the same kind
of fix as 7d3448961da (float NaN/infinity penalties and distances, bugs
#19501/#19524).

(Attach fix.patch when submitting, or paste it inline if using the web form,
which has no attachments.)

## Verification

Throwaway clusters: master on port 5601, REL_18 on port 5602, fsync=off;
since removed. Raw outputs are in verify/.

1. Versions: master 158b6d5109fad9d5f0c30693fa85041a1c621fe1, "PostgreSQL 20devel on aarch64-apple-darwin25.6.0"; REL_18_STABLE a85b5523e679870414bf94e44dee424d73753abb, "PostgreSQL 18.6" (git log -1, SELECT version()).
2. INSERT of ±infinity fails on a multi-page index: verify/pgbg-verify.sql section A and minimal.sql. ERROR on both versions.
3. Threshold: n=405 gives a 1-page index and the insert succeeds; n=410 gives 3 pages and the insert fails (pg_relation_size loop).
4. UPDATE and COPY: sections C1 and C2. ERROR on both (COPY with a CONTEXT line 1).
5. CREATE INDEX over infinities, then a finite insert: section B. The build succeeds (3 pages); INSERT '2000-06-01' gives an ERROR, and so does INSERT 'infinity'.
6. buffering=on build with infinities: B2 gives an ERROR; the control without infinities (B3) succeeds.
7. Scalar 'infinity'::date <-> date: D1 gives an ERROR.
8. KNN scan over an index with infinities (EXPLAIN shows Index Only Scan ... Order By): D2 gives an ERROR.
9. KNN with an infinite probe on a finite-only index: D3 gives an ERROR.
10. timestamp and timestamptz: section E, all succeed; timestamp <-> gives infinity.
11. Call path via backtrace_functions: penalty+72/+96 (lines 212/218) <- gistchoose; gdb_date_dist <- gbt_date_distance <- gistScanPage.
12. Lines on master: grep date_mi in btree_date.c gives lines 95, 123, 212, 218, 228; date.c:556 (on 18, date.c:560).
13. Origin: a303e4dc431 adds the date_mi error (REL8_4_BETA1~873); btree_date.c in REL8_4_0 already calls date_mi in the penalty; the KNN calls came with 8436489c81c (9.1).
14. REL_14_STABLE and REL_19_STABLE btree_date.c have the same five date_mi calls (not run).
15. The sorted build (e4309f73f69, REL_18_BETA1~306) explains why CREATE INDEX works; earlier versions are inferred, not tested.
16. Prototype fix: built via PGXS against a copy of the master install (the main install was not touched). Every case passes; the 20000-row index vs seqscan check matches (400/400/592); installcheck passes 32/32 with no warnings.
17. Rechecked in the main session on master 158b6d5109: minimal.sql gives the five errors listed and timestamp <-> returns infinity.

## Duplicate search

- https://www.postgresql.org/search/?m=1&q=btree_gist+infinity+date gives one unrelated hit, a thread on cross-type integer operators.
- https://www.postgresql.org/search/?m=1&q=cannot+subtract+infinite+dates gives no hits.
- https://www.postgresql.org/search/?m=1&q=gist+infinite+date gives one unrelated hit.
- https://www.postgresql.org/search/?m=1&q=btree_gist+infinity: the closest hit is the float NaN thread https://www.postgresql.org/message-id/CAON2xHNZ903sLWHhC1i9pb0dMDKcVWbDieujAGouX9rs226U_w@mail.gmail.com, about floats rather than dates.
- git log of contrib/btree_gist since 2008: no fix for date infinities. Precedent: 7d3448961da (the NaN fix, 2026-07-01, backpatched through 14).

No duplicate found.
