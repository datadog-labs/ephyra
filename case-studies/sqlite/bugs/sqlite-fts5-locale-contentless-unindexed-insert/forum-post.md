**Title:** FTS5: INSERT into a locale=1, contentless_unindexed=1 table with an UNINDEXED column fails with "database disk image is malformed"

```sql
CREATE VIRTUAL TABLE h USING fts5(a, b UNINDEXED, locale=1, content='', contentless_unindexed=1);
INSERT INTO h(rowid, a, b) VALUES (4, 'p q', 'r');
SELECT rowid, * FROM h;
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.54.1 and 3.51.0 give the same:

```
Error near line 2: database disk image is malformed
```

Expected, and what the same table without locale=1 prints:

```
4||r
```

The database is fresh, and `INSERT INTO h(h) VALUES('integrity-check')` and PRAGMA integrity_check pass before the INSERT. Every INSERT into such a table fails, with plain text or with fts5_locale() values, so it can never hold a row. Without locale=1, or without an UNINDEXED column (then there is no %_content table), the INSERT works.

**Cause, as far as we can tell.** With contentless_unindexed=1 the %_content table stores only the UNINDEXED columns. `sqlite3Fts5StorageOpen()` (fts5_storage.c) creates it, and when locale=1 it also adds an "l" column for each indexed column, whatever the content mode, so the table here is `h_content(id INTEGER PRIMARY KEY, c1, l0)`. The rest of FTS5 uses those locale columns only for normal content tables: `fts5StorageGetStmt()` adds their bindings to the INSERT_CONTENT statement only when `eContent==FTS5_CONTENT_NORMAL`, and the same is true of `fts5ConfigMakeExprlist()` and `fts5TextFromStmt()`. So FTS5 prepares `INSERT INTO 'main'.'h_content' VALUES(?1,?3)`, two values for three columns. That prepare fails with SQLITE_ERROR, and `fts5StorageGetStmt()` reports that as SQLITE_CORRUPT_VTAB, the error it gives for a missing shadow table. The locale of an indexed column has nowhere to go in such a table anyway, since the column's text is not stored (`fts5_get_locale()` returns NULL for it).

Creating the locale columns only for normal content tables, as the other places do, gives the expected output above:

```diff
--- a/ext/fts5/fts5_storage.c
+++ b/ext/fts5/fts5_storage.c
@@ -377,7 +377,7 @@
           sqlite3_str_appendf(pDefn, ", c%d", i);
         }
       }
-      if( pConfig->bLocale ){
+      if( pConfig->bLocale && pConfig->eContent==FTS5_CONTENT_NORMAL ){
         for(i=0; i<pConfig->nCol; i++){
           if( pConfig->abUnindexed[i]==0 ){
             sqlite3_str_appendf(pDefn, ", l%d", i);
```

A table created by an unpatched build keeps its extra columns, so INSERT still fails on it with the patch. No row can have been inserted into such a table, so it can be dropped and created again; because of a separate bug we are reporting at the same time (DROP TABLE and ALTER TABLE RENAME skip the %_content table of every contentless_unindexed=1 table), its h_content table has to be dropped too: DROP TABLE h; DROP TABLE h_content; A table created by a patched build also works in unpatched builds, because their INSERT statement matches the new schema.

This is offered as a fix. The full patch, including regression tests in ext/fts5/test/fts5unindexed2.test (two new sections, 11 cases), is in the attached sqlite-fts5-locale-contentless-unindexed-insert.patch.

Without the fix 9 of the 58 tests in fts5unindexed2.test fail; with it none do. With this patch and the one for the tokendata=1 report we are posting at the same time applied together to trunk 185ffd27064b, all 149 files in ext/fts5/test (fault-injection tests included) give the same 5 pre-existing errors (fts5contentless3, fts5faultI) as without them, veryquick gives 29 errors in the same 7 files, and the new tests and the example run clean under ASan/UBSan. Each patch alone fixes only its own tests.

**Versions** (FTS5 builds): the same error in 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_FTS5), 3.51.0 (the macOS /usr/bin/sqlite3), 3.54.1 (Homebrew) and trunk. 3.46.0 rejects locale= ("unrecognized option"); neither locale= nor contentless_unindexed= is in its source.

We found it with a property-based test generator for SQLite databases and queries: an INSERT into an FTS5 table with locale=1, content='', contentless_unindexed=1 and an UNINDEXED column failed with "database disk image is malformed".
