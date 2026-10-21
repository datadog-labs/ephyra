# Candidate: btree_gist date_ops fails to insert infinite dates

Found 2026-10-13 by ephyra-postgresql's setup loader (setup test seed 89), the day GiST indexes joined
the generator. `gbt_date_penalty` (contrib/btree_gist/btree_date.c, lines ~212-228) computes key
distances with `DirectFunctionCall2(date_mi, ...)`, and `date_mi` raises "cannot subtract infinite
dates" for infinite inputs (since a303e4dc431, PostgreSQL 8.4, 2008). Single-page indexes never call
the penalty, so small tests pass. timestamp_ops is not affected (its penalty does its own arithmetic).
The distance functions (lines ~95, ~123, the `<->` operator) call date_mi too. Not yet checked:
whether known/fixed, releases, inserting finite dates into an index that already holds infinities.
ephyra-postgresql excludes it (semantics::rows_ok).
