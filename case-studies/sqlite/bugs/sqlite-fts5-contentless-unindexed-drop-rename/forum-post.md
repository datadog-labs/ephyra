**Title:** FTS5: DROP TABLE and ALTER TABLE RENAME skip the %_content table of a contentless_unindexed=1 table

```sql
CREATE VIRTUAL TABLE h USING fts5(a, b UNINDEXED, content='', contentless_unindexed=1);
INSERT INTO h(rowid, a, b) VALUES (1, 'x y', 'b1');
ALTER TABLE h RENAME TO h2;
SELECT name FROM sqlite_schema ORDER BY name;
INSERT INTO h2(rowid, a, b) VALUES (2, 'x z', 'b2');
DROP TABLE h2;
SELECT name FROM sqlite_schema;
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.54.1 and 3.51.0 give the same:

```
h2
h2_config
h2_data
h2_docsize
h2_idx
h_content
Error near line 5: database disk image is malformed
h_content
```

Expected:

```
h2
h2_config
h2_content
h2_data
h2_docsize
h2_idx
```

The RENAME leaves `h_content` under its old name, so the renamed table has no %_content table: every INSERT into it fails with "database disk image is malformed", and a SELECT of its columns fails with "no such table: main.h2_content". This stays so after the database is closed and reopened. DROP TABLE leaves `h_content` behind, so a later `CREATE VIRTUAL TABLE h ...` of the same kind fails with "fts5: error creating shadow table h_content: table 'h_content' already exists". It happens for every table with content='', contentless_unindexed=1 and at least one UNINDEXED column, with or without contentless_delete=1, columnsize=0 or locale=1 (without an UNINDEXED column there is no %_content table). Normal, contentless (with or without contentless_delete=1) and external content tables are not affected: DROP and RENAME handle all of their shadow tables.

**Cause, as far as we can tell.** A contentless_unindexed=1 table has a %_content table, which holds the values of its UNINDEXED columns: `sqlite3Fts5StorageOpen()` (fts5_storage.c) creates it when `eContent` is `FTS5_CONTENT_NORMAL` or `FTS5_CONTENT_UNINDEXED`, and the INSERT, DELETE and 'delete-all' code uses it in both modes. `sqlite3Fts5DropAll()` (xDestroy) and `sqlite3Fts5StorageRename()` (xRename) drop and rename it only when `eContent==FTS5_CONTENT_NORMAL`. The INSERT after the RENAME reports SQLITE_CORRUPT_VTAB because `fts5StorageGetStmt()` maps a failure to prepare a statement on a missing shadow table to that error.

Handling %_content in both modes, as `sqlite3Fts5StorageOpen()` and `sqlite3Fts5StorageDelete()` do, gives the expected output above:

```diff
--- a/ext/fts5/fts5_storage.c
+++ b/ext/fts5/fts5_storage.c
@@ -264,7 +264,10 @@
         pConfig->zDb, pConfig->zName
     );
   }
-  if( rc==SQLITE_OK && pConfig->eContent==FTS5_CONTENT_NORMAL ){
+  if( rc==SQLITE_OK
+   && (pConfig->eContent==FTS5_CONTENT_NORMAL
+    || pConfig->eContent==FTS5_CONTENT_UNINDEXED)
+  ){
     rc = fts5ExecPrintf(pConfig->db, 0, 
         "DROP TABLE IF EXISTS %Q.'%q_content';",
         pConfig->zDb, pConfig->zName
@@ -297,7 +300,9 @@
   if( pConfig->bColumnsize ){
     fts5StorageRenameOne(pConfig, &rc, "docsize", zName);
   }
-  if( pConfig->eContent==FTS5_CONTENT_NORMAL ){
+  if( pConfig->eContent==FTS5_CONTENT_NORMAL
+   || pConfig->eContent==FTS5_CONTENT_UNINDEXED
+  ){
     fts5StorageRenameOne(pConfig, &rc, "content", zName);
   }
   return rc;
```

A table dropped by an unpatched build leaves an orphan `h_content`, which `DROP TABLE h_content` removes. A table renamed by an unpatched build can be repaired, with SQLITE_DBCONFIG_DEFENSIVE off, by `ALTER TABLE h_content RENAME TO h2_content`; with the patch, renaming such a table again fails with "SQL logic error" until that repair is done, and DROP TABLE still works on it.

This is offered as a fix. The full patch, including regression tests in ext/fts5/test/fts5alter.test (two new sections, 32 cases, including an attached schema and rollback), is in the attached sqlite-fts5-contentless-unindexed-drop-rename.patch.

Without the fix 25 of the 45 tests in fts5alter.test fail; with it none do. On trunk 185ffd27064b with the patch, all 149 files in ext/fts5/test (fault-injection tests included) give the same 5 pre-existing errors (fts5contentless3, fts5faultI) as without it, veryquick gives 29 errors in the same 7 files, and the new tests and the example run clean under ASan/UBSan. The patch applies cleanly together with the one for the locale=1 report.

The locale=1 report we are posting at the same time depends on this one for its workaround: a table broken by that bug has to be dropped together with its h_content table.

**Versions** (FTS5 builds): the same output in 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_FTS5), 3.51.0 (the macOS /usr/bin/sqlite3), 3.54.1 (Homebrew) and trunk. 3.46.0 rejects contentless_unindexed= ("unrecognized option"), so the bug is as old as the option.

We found it with a property-based test generator for SQLite databases and queries: while checking our fix for the locale=1 report, we saw that DROP TABLE left the h_content table of a contentless_unindexed=1 table behind.
