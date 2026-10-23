# pg-statext-bool-domain: "expression not found in statistics object" for a boolean domain column

Found 2026-10-22 by round postgresql/round/alter-1 (tlp oracle, `tests/sqlancer.rs`), once
ephyra-postgresql started giving columns domain types. Not reported anywhere.

## Symptom

A query with a clause on a column of a domain over `bool`, together with another clause on a
column of the same extended statistics object (MCV list), fails at planning:

    ERROR:  expression not found in statistics object   (XX000)

`minimal.sql` reproduces it every time on master 158b6d5109f (assert-enabled build in
`~/Projects/postgres/inst`). The same shape with a domain over `int4` works, as does a plain `bool`
column.

## Likely cause (not verified in the source)

A boolean clause on the column (`b`, `NOT b`, `b = true`, which the planner simplifies to `b`)
is the column under a RelabelType (domain to bool). The extended-statistics clause matching seems to
accept it as an expression of the statistics object, and the MCV code then fails to find that
expression among the object's (it has none: `b` is a plain column there).

## Fuzzer exclusion

`crate::semantics::set_domain` (ephyra-postgresql, branch round/alter-1) does not give a column a
domain over bool.
