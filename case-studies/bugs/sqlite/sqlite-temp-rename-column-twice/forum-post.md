**Title:** ALTER TABLE RENAME COLUMN of a TEMP table fails when a TEMP trigger or view uses the column

```sql
CREATE TEMP TABLE t(a);
CREATE TEMP TRIGGER tr BEFORE DELETE ON t WHEN old.a > 0 BEGIN SELECT 1; END;
ALTER TABLE t RENAME COLUMN a TO b;

CREATE TEMP TABLE t2(a);
CREATE TEMP VIEW v2 AS SELECT a FROM t2;
ALTER TABLE t2 RENAME COLUMN a TO b;

CREATE TABLE t1(x);
CREATE TEMP TABLE t5(a, b);
CREATE TEMP TRIGGER w BEFORE INSERT ON t1 BEGIN UPDATE t5 SET b = 1 WHERE b > 0; END;
ALTER TABLE t5 RENAME COLUMN b TO c;
```

Output from trunk (check-in 5bd093e261a7, 2026-10-16 18:11), built from the source tarball with default options. 3.54.0 gives the same:

```
Error near line 3: error in trigger tr: no such column: old.b
Error near line 7: error in view v2: no such column: b
Error near line 13: error in trigger w: no such column: c
```

Expected: the three statements succeed and the trigger and view are rewritten, as they are when the table is in main ("The column name is changed both within the table definition itself and also within all indexes, triggers, and views that reference the column", lang_altertable.html). With the patch below, `sqlite_temp_schema` then holds:

```
CREATE TABLE t(b)
CREATE TRIGGER tr BEFORE DELETE ON t WHEN old.b > 0 BEGIN SELECT 1; END
CREATE TABLE t2(b)
CREATE VIEW v2 AS SELECT b FROM t2
CREATE TABLE t5(a, c)
CREATE TRIGGER w BEFORE INSERT ON t1 BEGIN UPDATE t5 SET c = 1 WHERE c > 0; END
```

The error names the new column, and the statement is rolled back, so the column cannot be renamed while the trigger or view exists. It takes a table in the temp schema and a temp trigger or temp view (a trigger on the table itself, or on any other table as `w` is) with an expression that refers to the column: `old.a`/`new.a` in the WHEN clause or the body, or the column in a SELECT, a WHERE clause or the right-hand side of a SET. A trigger that names the column only in `UPDATE OF a` or as the target of `SET a = ...` or `INSERT INTO t(a)` is rewritten correctly, as are indexes on the temp table. With the table in main (trigger in main or temp), or in an attached database with a temp trigger on it, the rename works. `PRAGMA legacy_alter_table=ON` makes no difference. RENAME TO and DROP COLUMN of the same temp table work.

**Cause, as far as we can tell.** `sqlite3AlterRenameColumn()` (alter.c) runs two nested UPDATEs before reloading the schema. The first rewrites every row of the table's own schema, `"%w".sqlite_schema` with `zDb` = "temp" here, through `sqlite_rename_column()`. The second rewrites the triggers and views of `temp.sqlite_schema`, so that temp objects that refer to a table in main or an attached database are updated too. When the table itself is in temp the two are the same table, and every temp trigger and view goes through `sqlite_rename_column()` twice. The second time, `renameColumnFunc()` parses the text the first pass wrote (`old.b`) and `renameResolveTrigger()` (or `sqlite3SelectPrep()` for a view) resolves it against the in-memory table, which still has the column `a` until the schema is reloaded after the statement: "no such column: old.b". It is the function's own error (`renameColumnParseError()` with an empty "when"), not the "after rename" check of `renameTestSchema()`. Names that are matched without resolving (`UPDATE OF`, SET and INSERT column lists) are simply not found the second time, which is why those triggers come out right.

`sqlite3AlterRenameTable()` already skips its temp-schema UPDATE when `iDb==1`, and doing the same here gives the expected result for the statements above:

```diff
--- a/src/alter.c
+++ b/src/alter.c
@@ -692,12 +692,17 @@
       pTab->zName
   );
 
-  sqlite3NestedParse(pParse,
-      "UPDATE temp." LEGACY_SCHEMA_TABLE " SET "
-      "sql = sqlite_rename_column(sql, type, name, %Q, %Q, %d, %Q, %d, 1) "
-      "WHERE type IN ('trigger', 'view')",
-      zDb, pTab->zName, iCol, zNew, bQuote
-  );
+  /* If the table is not itself in the temp database, edit the views and
+  ** triggers of the temp database too. If it is, the statement above has
+  ** already edited them.  */
+  if( iSchema!=1 ){
+    sqlite3NestedParse(pParse,
+        "UPDATE temp." LEGACY_SCHEMA_TABLE " SET "
+        "sql = sqlite_rename_column(sql, type, name, %Q, %Q, %d, %Q, %d, 1) "
+        "WHERE type IN ('trigger', 'view')",
+        zDb, pTab->zName, iCol, zNew, bQuote
+    );
+  }
 
   /* Drop and reload the database schema. */
   renameReloadSchema(pParse, iSchema, INITFLAG_AlterRename);
```

With it, alter*.test (altercol, altertab*, alterauth*, alterlegacy and the rest) and trigger*.test pass as before (39 files, 0 errors on both), and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official builds, default options): the same three errors in every release we tried with RENAME COLUMN, 3.25.0 (where it was added) to 3.54.0 and trunk (3.25.0, 3.28.0, 3.30.0, 3.31.0, 3.33.0, 3.35.0, 3.37.0, 3.38.0, 3.39.0, 3.40.0, 3.44.0, 3.50.4, 3.51.0, 3.53.4, 3.54.0).

We found it with a property-based test generator for SQLite databases and queries: a DROP COLUMN followed by a RENAME COLUMN on a TEMP table that a TEMP trigger's WHEN clause read failed with "no such column" for the new name.
