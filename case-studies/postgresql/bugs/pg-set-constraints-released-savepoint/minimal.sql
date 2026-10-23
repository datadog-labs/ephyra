-- SET CONSTRAINTS in a subtransaction that is released survives a
-- ROLLBACK TO an enclosing savepoint; set in the savepoint itself, it is
-- undone.
CREATE TABLE sc (k int PRIMARY KEY, f int REFERENCES sc (k) DEFERRABLE);
BEGIN;
SAVEPOINT s0;
SAVEPOINT s1;
SET CONSTRAINTS ALL DEFERRED;
RELEASE s1;
ROLLBACK TO s0;
INSERT INTO sc VALUES (1, 99);  -- not checked: the constraint is still deferred
ROLLBACK;
BEGIN;
SAVEPOINT s0;
SET CONSTRAINTS ALL DEFERRED;
ROLLBACK TO s0;
INSERT INTO sc VALUES (1, 99);  -- ERROR 23503: immediate again
ROLLBACK;
