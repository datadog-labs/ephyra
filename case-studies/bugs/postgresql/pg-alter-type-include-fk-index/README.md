# pg-alter-type-include-fk-index: ALTER COLUMN TYPE of an INCLUDE column of an index a foreign key uses

Found 2026-10-22 by round postgresql/round/alter-1 (qscore, norec). Not reported anywhere. Probably
a limitation rather than a bug; recorded for the owner to judge.

`ALTER TABLE p ALTER COLUMN b TYPE int8` fails with 2BP01 when `b` is an INCLUDE column of the
unique index that a foreign key referencing `p (a)` depends on (`minimal.sql`, master 158b6d5109f).
ALTER COLUMN TYPE rebuilds indexes and constraints on the altered column, including foreign keys
on it, but here the foreign key is not on `b`: the rebuild drops the index RESTRICT, and the
foreign key's dependency on the index stops it. Changing the type of a key column of the same
index works (the foreign key is rebuilt with it).

Fuzzer exclusion: `crate::semantics::alter_type` and `set_domain` skip a column that an index a
foreign key may depend on reads (`fk::may_back`), as DROP COLUMN already does.
