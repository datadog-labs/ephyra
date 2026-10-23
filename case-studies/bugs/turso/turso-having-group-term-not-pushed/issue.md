Filed as #9880 (https://github.com/tursodatabase/turso/issues/9880) (line 1 corrected 2026-10-21).

# Aggregates of groups removed by a GROUP BY-only HAVING term still run, so they raise errors SQLite never reaches

## Repro (tursodb)
```sql
CREATE TABLE t(a, g);
INSERT INTO t VALUES (9223372036854775807, 1), (1, 1), (5, 2);
SELECT g, sum(a) FROM t GROUP BY g HAVING g = 2;
SELECT g, max(json(CASE WHEN g = 1 THEN '{' ELSE '1' END)) FROM t GROUP BY g HAVING g = 2;
SELECT sum(a) FROM t GROUP BY NULL HAVING NULL;
```

| query | Turso | SQLite 3.50.4 (and trunk) |
|---|---|---|
| `... sum(a) ... HAVING g = 2` | `integer overflow` | `2\|5` |
| `... max(json(...)) ... HAVING g = 2` | `malformed JSON` | `2\|1` |
| `... GROUP BY NULL HAVING NULL` | `integer overflow` | no rows |

## Expected
The SQLite results. Group `g = 1` is removed by the HAVING clause and never appears in the output. SQLite moves every HAVING term that uses only constants and GROUP BY terms into the WHERE clause (`havingToWhere()` in src/select.c), so the rows of group 1 are never aggregated, and the `sum()` overflow (lang_aggfunc.html: sum() "will throw an 'integer overflow' exception") or the `json()` error never happens. Turso aggregates every group and only then applies HAVING, so it fails.

SQLite does not move a term that is literally false (`HAVING 0`), a term with a subquery, or a HAVING without GROUP BY; in those cases both engines raise the error, as they do when no HAVING is present.

This is not the same as #9851 (scalar subquery hoisted before the scan); here the extra work is the aggregate step for rows SQLite filters out before grouping.

## Likely cause
Turso has no HAVING-to-WHERE transfer: a HAVING predicate on GROUP BY terms is evaluated after the aggregates of each group are computed.

Tested on main f8549ca2.
