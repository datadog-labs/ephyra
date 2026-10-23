# Bugs: postgresql

The bugs the case studies found in postgresql. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [ALTER COLUMN TYPE fails on an INCLUDE column of an index used by a foreign key](pg-alter-type-include-fk-index/) | open | BUG #19791 |
| [btree_gist date_ops: inserting infinity/-infinity into a multi-page GiST index on a date fails with "cannot subtract infinite dates"](pg-btree-gist-infinite-date/) | open | [link](https://www.postgresql.org/message-id/19761-b72cfc453e487610%40postgresql.org) |
| [CLUSTER / REPACK silently loses a row updated in a rolled-back subtransaction](pg-cluster-subxact-row-loss/) | open | [link](https://www.postgresql.org/message-id/19760-8f9a0eba17bc8c92%40postgresql.org) |
| ["cannot start a subtransaction when there are unprocessed inval messages" after ALTER DOMAIN DROP CONSTRAINT](pg-domain-inval-subxact/) | open | BUG #19790 |
| [Hashed set operations and recursive UNION pre-size their hash table from the planner's estimate without the hash_mem cap (PostgreSQL 19 regression)](pg-hashsetop-out-of-memory/) | open | [link](https://www.postgresql.org/message-id/19759-acd3f33402e54f36%40postgresql.org) |
| [Nested BETWEEN grows exponentially in parse analysis and statement_timeout cannot stop it](pg-huge-expr-timeout/) | open | [link](https://www.postgresql.org/message-id/19758-d074798b35cc9215%40postgresql.org) |
| [JSON_TABLE with a subquery in PASSING fails with "unexpected non-SELECT command in SubLink"](pg-json-table-passing-sublink/) | open | [link](https://www.postgresql.org/message-id/19786-70758103a4b2bff2%40postgresql.org) |
| [pg-minmax-subquery-relid](pg-minmax-subquery-relid/) |  |  |
| ["corrupt MVNDistinct entry" (XX000) from GROUP BY with grouping sets over a column and an expression of it](pg-mvndistinct-grouping-sets/) | open | BUG #19772 |
| [EXPLAIN ANALYZE: Assert("toc->toc_entry[i].key != key") in shm_toc_insert (master only, since a241bc4fede)](pg-parallel-material-dup-toc/) | open | [link](https://www.postgresql.org/message-id/19766-c3d105a4e0ec5487%40postgresql.org) |
| [UPDATE moving a row of a self-referencing partitioned table fails with 23503 although ON UPDATE CASCADE would fix the reference](pg-partition-selfref-cascade-move/) | open | BUG #19781 |
| ["could not find pathkey item to sort" for a Gather Merge over a merge join after outer-join reordering](pg-pathkey-item-norec-join/) | open | [link](https://www.postgresql.org/message-id/19787-d14d7f0f320f4495%40postgresql.org) |
| [PostgreSQL: candidate from today's round, report drafted, not reported](pg-plans-sortagg-cte/) | open | BUG #19783 |
| [SET CONSTRAINTS in a released savepoint survives ROLLBACK TO an enclosing savepoint](pg-set-constraints-released-savepoint/) | open | BUG #19782 |
| ["expression not found in statistics object" with MCV statistics on a domain over boolean](pg-statext-bool-domain/) | open | BUG #19789 |
| [pg_get_viewdef prints a negated integer constant in ORDER BY / GROUP BY as a column position](pg-viewdef-negated-sort-constant/) | open | [link](https://www.postgresql.org/message-id/19767-f1189fd6a83254b7%40postgresql.org) |
