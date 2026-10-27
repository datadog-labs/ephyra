# BUG #19799: Self-join elimination: "failed to build any 3-way joins" with LATERAL

Posted to pgsql-bugs through the bug form as https://www.postgresql.org/message-id/19799-dab86d769b08dc53%40postgresql.org on 2026-10-26. The text below is the message as archived.

---

The following bug has been logged on the website:

Bug reference:      19799
Logged by:          Alperen Keles
Email address:      alpkeles99(at)gmail(dot)com
PostgreSQL version: 18.6
Operating system:   Ubuntu 24.04 x86_64 (also macOS 26 arm64)
Description:        

Hello,

With self-join elimination, this query fails at plan time with an
internal error:

  CREATE TABLE t (a int PRIMARY KEY);
  INSERT INTO t VALUES (1), (2);
  SELECT s.y
  FROM t d LEFT JOIN
       (t x CROSS JOIN LATERAL (SELECT x.a AS v OFFSET 0) b
            CROSS JOIN LATERAL (SELECT coalesce(b.v, 0) FROM t c
                                WHERE c.a = x.a) s(y))
       ON true;
  ERROR:  XX000: failed to build any 3-way joins
  LOCATION:  standard_join_search, allpaths.c:3537   (18.6)

On builds with --enable-cassert the backend crashes instead:

  TRAP: failed Assert("!bms_is_member(rti, lateral_relids)"),
  File: "initsplan.c", Line: 1370   (master; Line 1086 on REL_18_STABLE)

With SET enable_self_join_elimination = off the query returns the 4
expected rows (each of 1 and 2 twice). The table can be empty. It also
fails with FULL JOIN or RIGHT JOIN in place of the LEFT JOIN, with
(SELECT 1) in place of d, and with b written as LATERAL
generate_series(1, x.a) or a two-row LATERAL VALUES list; with
geqo_threshold = 2 the error is "geqo failed to make a valid plan". It
works without the outer join, without the unique key, with b.v or b.v
+ 1 in place of coalesce(b.v, 0), without OFFSET 0 in b, and with
join_collapse_limit = 1 and from_collapse_limit = 1.

Reproduced on 18.6 (REL_18_6, without assertions, macOS arm64),
REL_18_STABLE 4d62ef92fda, 19rc1 (REL_19_STABLE 1212d5d33b1) and
master 022a80fe6c1 (--enable-cassert, Ubuntu 24.04 x86_64), and master
a261fc2b722 without assertions. 17 and earlier have no self-join
elimination; 14 returns the rows.

The same assertion was reported in the thread "Assert failure in
create_lateral_join_info" and fixed by 3138f248cd3 ("Don't remove an
RTE_RESULT needed by a nested PlaceHolderVar"). That query was
reached through RTE_RESULT removal and works on master 022a80fe6c1,
which contains 3138f248cd3, but this one still fails there.

Found with a query fuzzer.

Regards,
Alperen Keles
