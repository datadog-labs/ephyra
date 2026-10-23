# Planner: "no relation entry for relid 1" with min/max subquery and ScalarArrayOp (candidate)

Found 2026-10-21 by round postgresql/round/dml-1 (ephyra-postgresql, NoREC's
original query over a derived-table source). XX000 internal error from the
planner for a min/max aggregate subquery in FROM (the index-scan min/max
optimization applies: needs a B-tree index on the column) whose outer WHERE
is `const op ANY|ALL (array_append(<array>, s.c0))`, a qual pushed down into
the subquery. Plain `ARRAY[s.c0, 2]` or `s.c0 IN (..)` does not fail.

Confirmed on ~/Projects/postgres/inst (master 158b6d5109, assert-enabled).
Run: `psql -X -f minimal.sql` (in a scratch database).

Not reported upstream (round rule). Excluded in ephyra-postgresql
(`expr.rs`, `draw_from_sub` / `from_item_ok`): an aggregate-row subquery in
FROM takes no `min` or `max`.
