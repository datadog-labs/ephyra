-- A min/max aggregate subquery in FROM (index-optimized by planagg) with an
-- outer qual `x = ANY (array_append(.., s.c0))` pushed into it fails in the
-- planner with an internal error.
CREATE TABLE rt(c0 int4);
CREATE INDEX ON rt(c0);
SELECT * FROM (SELECT max(c0) FROM rt) AS s(c0) WHERE 1 = ANY (array_append(ARRAY[2], s.c0));
-- ERROR:  no relation entry for relid 1   (XX000)
-- Without the index, or with ARRAY[s.c0, 2] instead of array_append, it runs.
