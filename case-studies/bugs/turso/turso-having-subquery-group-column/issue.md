DRAFT, not filed
Title: Comment on #8221: the GROUP BY column itself is read from the wrong row too, when a HAVING subquery uses it and it isn't selected

Another case of this issue: it also happens for the GROUP BY column, not only for bare columns. If the column isn't in the result list, a subquery in HAVING that uses it (including the left side of `IN (SELECT ...)`) reads it from the table cursor after the scan. Every group is then dropped.

```sql
CREATE TABLE t(a, b);
INSERT INTO t VALUES (1, 10), (2, 20), (2, 30);
SELECT 'A', count(*) FROM t GROUP BY a HAVING a IN (SELECT 2);
SELECT 'B', count(*) FROM t GROUP BY a HAVING a NOT IN (SELECT 2);
SELECT 'C', count(*) FROM t GROUP BY a HAVING EXISTS (SELECT 1 WHERE a = 2);
SELECT 'D', a, count(*) FROM t GROUP BY a HAVING a IN (SELECT 2);
SELECT 'E', count(*) FROM t GROUP BY a HAVING a IN (2);
```

Turso (`tursodb -q -m list`):

```
D|2|2
E|2
```

SQLite 3.50.4:

```
A|2
B|1
C|2
D|2|2
E|2
```

D (the column is selected) and E (no subquery) are already correct. In `EXPLAIN` for A, the IN's left side is `Column` on the table cursor of `t` inside the group output subroutine, where the sorter's value of `a` should be used.

Tested on Turso main abf885b2.
