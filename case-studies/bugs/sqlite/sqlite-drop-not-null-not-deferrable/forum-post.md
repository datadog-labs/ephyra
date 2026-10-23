**Title:** ALTER COLUMN DROP NOT NULL removes NOT DEFERRABLE instead of NOT NULL, or removes a DEFERRABLE clause that follows NOT NULL

```sql
CREATE TABLE p(a PRIMARY KEY);
CREATE TABLE c1(x REFERENCES p NOT DEFERRABLE NOT NULL, y);
ALTER TABLE c1 ALTER x DROP NOT NULL;
SELECT sql FROM sqlite_schema WHERE name='c1';
INSERT INTO c1 VALUES(NULL, 1);

CREATE TABLE c2(x REFERENCES p NOT NULL DEFERRABLE INITIALLY DEFERRED, y);
ALTER TABLE c2 ALTER x DROP NOT NULL;
SELECT sql FROM sqlite_schema WHERE name='c2';
PRAGMA foreign_keys = ON;
BEGIN;
INSERT INTO c2 VALUES(1, 1);    -- x's foreign key is deferred: no error expected here
INSERT INTO p VALUES(1);
COMMIT;
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball. 3.54.0 and 3.53.4 give the same:

```
CREATE TABLE c1(x REFERENCES p NOT NULL, y)
Error near line 5: NOT NULL constraint failed: c1.x
CREATE TABLE c2(x REFERENCES p, y)
Error near line 12: FOREIGN KEY constraint failed
```

Expected: each DROP NOT NULL removes only the NOT NULL, giving the schemas one gets by deleting it by hand, and neither INSERT fails:

```
CREATE TABLE c1(x REFERENCES p NOT DEFERRABLE, y)
CREATE TABLE c2(x REFERENCES p DEFERRABLE INITIALLY DEFERRED, y)
```

Without `NOT DEFERRABLE` in c1, trunk gives `CREATE TABLE c1(x REFERENCES p, y)`. With the deferral clause written before the NOT NULL in c2, `x REFERENCES p DEFERRABLE INITIALLY DEFERRED NOT NULL`, trunk gives the expected c2 schema and the deferred INSERT succeeds.

The two tables show two forms:

- c1: DROP NOT NULL removes the `NOT DEFERRABLE` instead of the `NOT NULL`, reports success, and x is still NOT NULL. The same happens when the NOT NULL is named (`CONSTRAINT nn NOT NULL`), and when the deferral clause is named (`CONSTRAINT d NOT DEFERRABLE`, which is then removed). On a column that has `NOT DEFERRABLE` but no NOT NULL, DROP NOT NULL removes the `NOT DEFERRABLE`, and so does ALTER COLUMN SET NOT NULL, which first removes an existing NOT NULL the same way.
- c2: a deferral clause that follows the NOT NULL is removed with it, so x's foreign key silently changes from deferred to immediate and the INSERT inside the transaction fails. The same happens with a named NOT NULL, with ALTER COLUMN SET NOT NULL on such a column (the result is `x REFERENCES p NOT NULL`), and with ALTER TABLE DROP CONSTRAINT of a named NOT NULL or CHECK constraint that the deferral clause follows, as in `x REFERENCES p CONSTRAINT c CHECK(x>0) DEFERRABLE INITIALLY DEFERRED`.

The cause is in `dropConstraintFunc()` and `getConstraint()` in src/alter.c. In DROP NOT NULL mode, `dropConstraintFunc()` takes the first NOT token in the column definition, or the first named constraint that starts with NOT, to be the NOT NULL constraint without looking at the token after it. `getConstraint()` then finds the end of the constraint being removed by scanning up to the first token that could start another constraint: CONSTRAINT, PRIMARY, NOT, UNIQUE, CHECK, DEFAULT, COLLATE, REFERENCES, FOREIGN, GENERATED, AS, a comma or ")". DEFERRABLE is not in that list, but a deferral clause that follows another column constraint is a column constraint of its own (`ccons ::= defer_subclause` in parse.y) that applies to the most recently defined foreign key, so the scan takes it as part of the preceding NOT NULL or CHECK constraint.

The patch below makes the DROP NOT NULL search accept a NOT only when NULL follows it, and makes `getConstraint()` stop at DEFERRABLE except within a foreign key constraint. For a named `CONSTRAINT d NOT DEFERRABLE` it steps over the DEFERRABLE, so that the clause is not taken for a NOT NULL constraint and DROP CONSTRAINT d still removes all of it:

```diff
--- a/src/alter.c
+++ b/src/alter.c
@@ -2476,6 +2476,10 @@
   **
   **   CONSTRAINT, PRIMARY, NOT, UNIQUE, CHECK, DEFAULT, 
   **   COLLATE, REFERENCES, FOREIGN, GENERATED, AS, RP, or COMMA
+  **
+  ** or DEFERRABLE, unless this is a foreign key constraint. A deferral
+  ** clause that follows some other constraint is a column constraint of
+  ** its own, applying to the most recently defined foreign key.
   **
   ** Also exit the loop if ILLEGAL turns up.
   */
@@ -2485,6 +2489,7 @@
      || t==TK_CHECK || t==TK_DEFAULT || t==TK_COLLATE || t==TK_REFERENCES
      || t==TK_FOREIGN || t==TK_RP || t==TK_COMMA || t==TK_ILLEGAL
      || t==TK_AS || t==TK_GENERATED
+     || (t==TK_DEFERRABLE && eTok!=TK_REFERENCES)
     ){
       break;
     }
@@ -2642,6 +2647,16 @@
           t = TK_CHECK;
         }else{
           iOff += nTok;
+          if( t==TK_NOT ){
+            /* For "NOT DEFERRABLE", step over the DEFERRABLE as well. Such
+            ** a clause is not a NOT NULL constraint.  */
+            int t2 = 0;
+            nTok = getConstraintToken(&zSql[iOff], &t2);
+            if( t2==TK_DEFERRABLE ){
+              iOff += nTok;
+              if( iNotNull>=0 ) t = TK_DEFERRABLE;
+            }
+          }
           iOff += getConstraint(&zSql[iOff], t);
         }
 
@@ -2655,8 +2670,13 @@
         }
 
       }else if( t==TK_NOT && iNotNull==ii ){
-        iEnd = iOff + getConstraint(&zSql[iOff], 0);
-        break;
+        /* Skip the NOT of a "NOT DEFERRABLE" clause */
+        int t2 = 0;
+        getConstraintToken(&zSql[iOff], &t2);
+        if( t2==TK_NULL ){
+          iEnd = iOff + getConstraint(&zSql[iOff], 0);
+          break;
+        }
       }else if( t==TK_RP || t==TK_ILLEGAL ){
         iEnd = -1;
         break;
```

**Versions:** in 3.53.2, 3.53.4, 3.54.0 and trunk. 3.50.4 and 3.51.0 have no ALTER COLUMN DROP NOT NULL.

We found it while working on the report on dropping a foreign key with SET DEFAULT, NOT DEFERRABLE or MATCH (https://sqlite.org/bugs/forumpost/5e80e94512), which came from extending a property-based test generator for SQLite databases and queries to ALTER TABLE ... DROP CONSTRAINT: we read the DROP NOT NULL path of `dropConstraintFunc()` and the stop tokens of `getConstraint()`. This patch and the one in that post change different lines of src/alter.c, and each applies with or without the other.
