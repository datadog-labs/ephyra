# BUG #19798: "failed to build any 4-way joins" with ROLLUP over a LATERAL subquery

Posted to pgsql-bugs through the bug form as https://www.postgresql.org/message-id/19798-5a5da856f5f71118%40postgresql.org on 2026-10-26. The text below is the message as archived.

---

The following bug has been logged on the website:

Bug reference:      19798
Logged by:          Alperen Keles
Email address:      alpkeles99(at)gmail(dot)com
PostgreSQL version: 18.6
Operating system:   Ubuntu 24.04 x86_64 (also macOS 26 arm64)
Description:        

Hello,

A query with grouping sets fails at plan time with an internal error
when a LATERAL subquery over two tables returns a column of the
nullable side of an earlier LEFT JOIN. The same query without
grouping sets runs fine.

  CREATE TABLE t (x int);
  INSERT INTO t VALUES (1), (2);
  SELECT s.y
  FROM t b LEFT JOIN t a ON a.x = b.x
       CROSS JOIN LATERAL (SELECT a.x AS y FROM t c, t d) s
  GROUP BY ROLLUP (s.y);
  ERROR:  XX000: failed to build any 4-way joins
  LOCATION:  standard_join_search, allpaths.c:3538   (18.6)

The table can be empty. CUBE (s.y), GROUPING SETS ((s.y), ()) and a
RIGHT JOIN in place of the LEFT JOIN fail the same way. With three
tables in the subquery it fails with "5-way joins", and with
"t b LEFT JOIN (t a JOIN t f ON a.x = f.x) ON a.x = b.x" and three
tables in the subquery with "6-way joins". The query works with a
plain GROUP BY s.y or no GROUP BY, with one table in the subquery,
with an inner join instead of the LEFT JOIN, when the subquery
references b instead of a, or with join_collapse_limit = 1 and
from_collapse_limit = 1.

Reproduced on 18.6 (REL_18_STABLE 4d62ef92fda), 19rc1 (REL_19_STABLE
1212d5d33b1), 14.24 (REL_14_STABLE 14904052e65) and master 022a80fe6c1,
--enable-cassert, Ubuntu 24.04 x86_64; also on master and 18.6 on
macOS arm64. It is a plain ERROR, not an assertion failure. All of
these builds include the fix for the similar case in the thread
"failed to build any N-way joins" from a five-relation query
(commit b84ad0cd904 on master), and that thread's query works on
them, but this one still fails.

The first failing commit is 90947674fc9 ("Fix incorrect handling of
subquery pullup in the presence of grouping sets", 2018), which was
back-patched to 9.5. It fails, and its parent ca4587f3f94 returns the
rows.

As far as I can tell, since that commit pull_up_simple_subquery()
wraps the subquery output a.x, a lateral reference, in a
PlaceHolderVar evaluated at {c,d} when the query has grouping sets. So
c and d must be joined first, and the only legal 4-way join is
{b,a} + {c,d}. No join clause connects these two. In the bushy loop of
join_search_one_level() the pair is considered only with {b,a} as
old_rel, since {b,a} comes first in the level-2 list. {b,a} is skipped
there because it has no join clauses and has_join_restriction() is
false for it: it contains its outer join, and a joinrel's
lateral_referencers is always NULL. The 5-way and 6-way forms fail the
same way, because there the pair is considered only with the smaller
side, {b,a} or {b,a,f}, as old_rel.

Found with a query fuzzer.

Regards,
Alperen Keles
