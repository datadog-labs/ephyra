**Title:** ALTER TABLE DROP CONSTRAINT on a foreign key with SET DEFAULT, NOT DEFERRABLE or MATCH fails or leaves part of the constraint behind

```sql
CREATE TABLE p(a PRIMARY KEY);
CREATE TABLE c1(x CONSTRAINT f REFERENCES p(a) ON DELETE SET DEFAULT ON UPDATE CASCADE, y);
ALTER TABLE c1 DROP CONSTRAINT f;
SELECT sql FROM sqlite_schema WHERE name='c1';

CREATE TABLE c2(w REFERENCES p DEFERRABLE INITIALLY DEFERRED, x CONSTRAINT f REFERENCES p NOT DEFERRABLE);
ALTER TABLE c2 DROP CONSTRAINT f;
SELECT sql FROM sqlite_schema WHERE name='c2';
PRAGMA foreign_keys = ON;
BEGIN;
INSERT INTO c2(w) VALUES(1);    -- w's foreign key is deferred: no error expected here
INSERT INTO p VALUES(1);
COMMIT;
PRAGMA foreign_keys = OFF;

CREATE TABLE c3(x, y, CONSTRAINT f FOREIGN KEY(x) REFERENCES p MATCH generated);
ALTER TABLE c3 DROP CONSTRAINT f;
SELECT sql FROM sqlite_schema WHERE name='c3';
```

Output from trunk (check-in 3769f60be38b, 2026-10-21 16:05), built from the source tarball. 3.54.0 gives the same:

```
Error near line 3: error in table c1 after drop constraint: near "ON": syntax error
CREATE TABLE c1(x CONSTRAINT f REFERENCES p(a) ON DELETE SET DEFAULT ON UPDATE CASCADE, y)
CREATE TABLE c2(w REFERENCES p DEFERRABLE INITIALLY DEFERRED, x NOT DEFERRABLE)
Error near line 11: FOREIGN KEY constraint failed
CREATE TABLE c3(x, y, generated)
```

Expected: each statement removes exactly the constraint it names, giving the schemas one gets by deleting that text by hand, and the deferred INSERT succeeds:

```
CREATE TABLE c1(x, y)
CREATE TABLE c2(w REFERENCES p DEFERRABLE INITIALLY DEFERRED, x)
CREATE TABLE c3(x, y)
```

Without the `SET DEFAULT`, `NOT DEFERRABLE` and `MATCH generated` clauses the same statements work: with `ON DELETE CASCADE` in c1, `REFERENCES p` alone in c2 and `MATCH full` in c3, trunk gives exactly these three schemas.

The three tables show three forms:

- c1: the DROP CONSTRAINT fails for any foreign key with `ON DELETE SET DEFAULT` or `ON UPDATE SET DEFAULT`, column or table constraint. The error is near "ON", ",", ")" or "DEFAULT", depending on what follows.
- c2: the DROP CONSTRAINT succeeds but leaves `NOT DEFERRABLE` behind as a column constraint of x. A bare deferral clause applies to the foreign key defined last before it, which after the drop is the one on w, so w's foreign key silently changes from deferred to immediate and the INSERT inside the transaction fails. For a table constraint `FOREIGN KEY(x) REFERENCES p NOT DEFERRABLE` the leftover does not parse and the statement fails.
- c3: `MATCH generated` (GENERATED can be used as a name). The table constraint form leaves `generated` behind as a new column, so the table now has three columns. The column constraint form `x CONSTRAINT f REFERENCES p MATCH generated` leaves `x generated`, which gives x the declared type "generated" and NUMERIC affinity instead of BLOB.

The cause is in `getConstraint()` in src/alter.c, which finds the end of a constraint by scanning tokens up to the first one that could start another constraint: CONSTRAINT, PRIMARY, NOT, UNIQUE, CHECK, DEFAULT, COLLATE, REFERENCES, FOREIGN, GENERATED, AS, a comma or ")". Three of these can occur inside a foreign key clause: the DEFAULT of `SET DEFAULT`, the NOT of `NOT DEFERRABLE`, and GENERATED as the name after MATCH. The scan stops there, and `dropConstraintFunc()` removes only the text before that token.

The patch below makes `getConstraint()`, inside a REFERENCES constraint, consume `SET DEFAULT`, `NOT DEFERRABLE` and `MATCH name` as units:

```diff
--- a/src/alter.c
+++ b/src/alter.c
@@ -2481,6 +2481,21 @@
   */
   while( 1 ){
     int n = getConstraintToken(&z[iOff], &t);
+    if( eTok==TK_REFERENCES && (t==TK_SET || t==TK_NOT || t==TK_MATCH) ){
+      /* Within a foreign key constraint, "SET DEFAULT", "NOT DEFERRABLE"
+      ** and "MATCH name" are each consumed as a unit. Otherwise the
+      ** DEFAULT, the NOT, or a name that is the fallback token GENERATED
+      ** would be taken for the start of the next constraint.  */
+      int t2 = 0;
+      int n2 = getConstraintToken(&z[iOff+n], &t2);
+      if( t==TK_MATCH
+       || (t==TK_SET && t2==TK_DEFAULT)
+       || (t==TK_NOT && t2==TK_DEFERRABLE)
+      ){
+        iOff += n + n2;
+        continue;
+      }
+    }
     if( t==TK_CONSTRAINT || t==TK_PRIMARY || t==TK_NOT || t==TK_UNIQUE
      || t==TK_CHECK || t==TK_DEFAULT || t==TK_COLLATE || t==TK_REFERENCES
      || t==TK_FOREIGN || t==TK_RP || t==TK_COMMA || t==TK_ILLEGAL
```

**Versions:** in 3.54.0 and trunk. 3.53.2 and 3.53.4 refuse to drop a foreign key ("constraint may not be dropped"). 3.51.0 has no DROP CONSTRAINT.

We found it with a property-based test generator for SQLite databases and queries: while extending the generator to ALTER TABLE ... DROP CONSTRAINT, we read `getConstraint()` and saw that its list of stop tokens includes keywords that can appear inside a foreign key clause.
