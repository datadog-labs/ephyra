**Title:** A function result that carries a number and its text keeps the old text after REAL affinity: wrong RETURNING output and FOREIGN KEY failures

```sql
CREATE TABLE r(x REAL);
INSERT INTO r VALUES(sqlar_uncompress(5, 1)) RETURNING x || '';
```

Output from the CLI of trunk (check-in 8a675a4b2b, 2026-11-05 00:01), built from the source tarball with zlib. 3.54.1 and /usr/bin/sqlite3 on macOS (3.51.0) give the same:

```
5
```

Expected: 5.0, as with `VALUES(5)`. sqlar_uncompress(X, SZ) returns X unchanged when SZ is the byte length of X, so sqlar_uncompress(5, 1) is the integer 5. It is only a convenient way to get such a value from SQL: any application function that reads its argument as text and then returns it does the same, for example one whose body is `sqlite3_value_text(argv[0]); sqlite3_result_value(ctx, argv[0]);` (repro-fk.c).

The stale text also reaches FOREIGN KEY checks:

```sql
PRAGMA foreign_keys=ON;
CREATE TABLE p(k TEXT UNIQUE);
INSERT INTO p VALUES('5.0');
CREATE TABLE c(x REAL REFERENCES p(k));
INSERT INTO c VALUES(5);
INSERT INTO c VALUES(sqlar_uncompress(5, 1));
```

```
Error near line 6: FOREIGN KEY constraint failed
```

Both INSERTs store the same value, 5.0, and the parent key '5.0' exists. A STRICT table gives the same error, also for an integer outside the 48-bit range such as 140737488355328 with parent key '140737488355328.0'.

**Cause, as far as we can tell.** sqlite3_value_bytes() or sqlite3_value_text() adds a text representation to the argument ("5") and keeps the integer, so the result that sqlite3_result_value() sets has both MEM_Int and MEM_Str. For a REAL column, OP_MakeRecord turns the integer into MEM_IntReal and clears MEM_Int but not MEM_Str, so the register keeps the old text "5". For a STRICT table, OP_TypeCheck does that conversion instead, again without clearing MEM_Str, both for MEM_IntReal and, for integers outside the 48-bit range, for MEM_Real. RETURNING reads that text, and the foreign key lookup copies the register and applies TEXT affinity, which keeps the existing text, so it searches the parent index for '5' instead of '5.0'. OP_Affinity, which converts an integer to MEM_Real for REAL affinity, clears both MEM_Int and MEM_Str.

The patch below clears MEM_Str in those three conversions too, and gives the expected output above for both scripts (5.0, and no error):

```diff
--- a/src/vdbe.c
+++ b/src/vdbe.c
@@ -3495,11 +3495,11 @@
             testcase( pIn1->u.i==-140737488355329LL );
             if( pIn1->u.i<=140737488355327LL && pIn1->u.i>=-140737488355328LL){
               pIn1->flags |= MEM_IntReal;
-              pIn1->flags &= ~MEM_Int;
+              pIn1->flags &= ~(MEM_Int|MEM_Str);
             }else{
               pIn1->u.r = (double)pIn1->u.i;
               pIn1->flags |= MEM_Real;
-              pIn1->flags &= ~MEM_Int;
+              pIn1->flags &= ~(MEM_Int|MEM_Str);
             }
           }else if( (pIn1->flags & (MEM_Real|MEM_IntReal))==0 ){
             goto vdbe_type_error;
@@ -3642,7 +3642,7 @@
       applyAffinity(pRec, zAffinity[0], encoding);
       if( zAffinity[0]==SQLITE_AFF_REAL && (pRec->flags & MEM_Int) ){
         pRec->flags |= MEM_IntReal;
-        pRec->flags &= ~(MEM_Int);
+        pRec->flags &= ~(MEM_Int|MEM_Str);
       }
       REGISTER_TRACE((int)(pRec-aMem), pRec);
       zAffinity++;
```

**Versions**: the same output for both scripts in trunk (8a675a4b2b), 3.54.1, 3.51.0 (/usr/bin/sqlite3 on macOS) and 3.50.4.

We found it with a property-based test generator for SQLite that builds VDBE programs directly and calls application-defined functions that read their arguments through the sqlite3_value interfaces.
