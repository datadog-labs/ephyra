# Candidate: "corrupt MVNDistinct entry" (XX000) from GROUP BY with grouping sets over a column and an expression of it

Found 2026-10-18 by the oracle tests on round/open-1 (extended statistics + grouping sets, both new in that
round): every oracle's original query failed. Not reported, not validated beyond the check below.

## Repro (minimal.sql, default settings)

    CREATE TABLE t0 (c0 int8, c1 int8);
    INSERT INTO t0 VALUES (NULL, NULL);
    CREATE STATISTICS t0_s ON c0, c1 FROM t0;
    ANALYZE t0;
    SELECT c0 FROM t0 GROUP BY c0, ROLLUP (abs(c0));

    ERROR:  XX000: corrupt MVNDistinct entry
    LOCATION:  estimate_multivariate_ndistinct, selfuncs.c:4820     (18.6: selfuncs.c:4485)

The error is raised by the planner's group-count estimate (estimate_num_groups ->
estimate_multivariate_ndistinct), so it happens at plan time.

## Versions checked (2026-10-18, macOS arm64, cassert)

- master 158b6d5109: error.
- REL_18_STABLE a85b5523e67 (18.6): error.

## Not yet done

Validation as for the other candidates: fresh master / 19 / 18 / 14 builds, the extent (other grouping-set
forms, other expressions, ndistinct only vs all kinds, without the NULL row), the cause in selfuncs.c, a
duplicate search, a draft report.
