**Title:** NOT INDEXED is ignored on WITHOUT ROWID tables: the planner still uses secondary and UNIQUE indexes

Query results are not affected. Only the plan is: a table named with NOT INDEXED is still read through its indexes when it is a WITHOUT ROWID table.

```sql
CREATE TABLE u(a PRIMARY KEY, b UNIQUE) WITHOUT ROWID;
EXPLAIN QUERY PLAN SELECT * FROM u NOT INDEXED WHERE b=1;
```

Output from trunk (check-in 74a6182a0c, 2026-10-13 18:36), built from the source tarball with default options:

```
QUERY PLAN
`--SEARCH u USING INDEX sqlite_autoindex_u_2 (b=?)
```

With the same table as an ordinary rowid table (`CREATE TABLE u(a PRIMARY KEY, b UNIQUE);`), the plan is `SCAN u`.

The documentation for INDEXED BY says that NOT INDEXED means "no index shall be used when accessing the preceding table, including implied indices create by UNIQUE and PRIMARY KEY constraints", and that the rowid can still be used. Here the implied index of the UNIQUE constraint is used. (Seeking on the PRIMARY KEY of a WITHOUT ROWID table, as in `WHERE a=1`, seems fine to us, since that b-tree is the table itself.)

The same happens with an ordinary `CREATE INDEX ti ON t(b)` on a WITHOUT ROWID table, for `b IN (...)`, for a covering scan (`SELECT b FROM t NOT INDEXED` gives `SCAN t USING COVERING INDEX ti`), for `ORDER BY b` (the index replaces the sort), for `min(b)`, for `b=1 OR a=2` (MULTI-INDEX OR), and when t is the inner table of a join. The rowid version of each of these gives a plain `SCAN`.

Where it does change results is an index that no longer matches its collating sequence, for example after an application changes a collation without a REINDEX. Here an application registers a collation "app" with `sqlite3_create_collation()`, case-sensitive (memcmp) in its first version, case-insensitive (strncasecmp) in its second:

```sql
-- with version 1 of "app":
CREATE TABLE w(k TEXT PRIMARY KEY, name TEXT COLLATE app) WITHOUT ROWID;
CREATE INDEX wi ON w(name);
CREATE TABLE r(k TEXT PRIMARY KEY, name TEXT COLLATE app);
CREATE INDEX ri ON r(name);
INSERT INTO w VALUES('1','alice'),('2','Bob'),('3','carol'),('4','Dave'),('5','erin');
INSERT INTO r SELECT * FROM w;
-- reopened with version 2 of "app":
SELECT count(*) FROM w NOT INDEXED WHERE name = 'bob';   -- 0, via SEARCH w USING COVERING INDEX wi (name=?)
SELECT count(*) FROM r NOT INDEXED WHERE name = 'bob';   -- 1, via SCAN r
```

`name = 'dave'` gives the same 0 and 1. Reading the table, as NOT INDEXED promises, finds the row under the new collation; on the WITHOUT ROWID table the stale index is searched instead.

**Cause, as far as we can tell.** In `whereLoopAddBtree()` (where.c), the rowid branch drops the table's indexes when NOT INDEXED is given (`sPk.pNext = notIndexed ? 0 : pTab->pIndex`), but the `!HasRowid(pTab)` branch sets `pProbe = pTab->pIndex` without looking at `notIndexed`, and the loop then visits every index. This change fixes all the shapes above for us; with it they become `SCAN t` (ORDER BY with a temp b-tree), while `a=1` still seeks on the primary key:

```diff
   }else if( !HasRowid(pTab) ){
-    pProbe = pTab->pIndex;
+    pProbe = pSrc->fg.notIndexed ? sqlite3PrimaryKeyIndex(pTab) : pTab->pIndex;
...
   for(; rc==SQLITE_OK && pProbe;
-      pProbe=(pSrc->fg.isIndexedBy ? 0 : pProbe->pNext), iSortIdx++
+      pProbe=((pSrc->fg.isIndexedBy || pSrc->fg.notIndexed) ? 0 : pProbe->pNext), iSortIdx++
```

Query results with it matched unpatched trunk in our checks (composite and expression indexes, partial index, joins, OR, DISTINCT, upsert), but we have not run the test suite, so please take it as a pointer to the cause.

The count() optimization already honours NOT INDEXED (count.test 7.x, 2020-05-08), and it does so on WITHOUT ROWID tables too: `SELECT count(*) FROM t NOT INDEXED` gives `SCAN t`, while without the clause it gives `SCAN t USING COVERING INDEX ti`.

**Versions:** every release we tried from 3.8.2 (the first with WITHOUT ROWID) through 3.53.4, 3.54.0 and trunk uses `sqlite_autoindex_u_2` for the query above.

We noticed this while triaging another report.
