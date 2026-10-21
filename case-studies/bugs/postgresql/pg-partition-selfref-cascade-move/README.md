# pg-partition-selfref-cascade-move (candidate, not reported)

Found by ephyra-postgresql round fk-1 (`set_operations_agree`, the case's
own statements), 2026-10-18. Confirmed on master 158b6d5109
(`~/Projects/postgres/inst`, assertions on).

A partitioned table with a self-referencing foreign key `ON UPDATE CASCADE`:
an UPDATE of the referenced key that moves a row to another partition fails
with 23503 when a row (here the moved row itself) references the old key,
although the cascade would update the reference to the new key. The same
UPDATE on a plain table succeeds and cascades (`minimal.sql`).

Likely cause: a cross-partition UPDATE is a DELETE plus an INSERT; the
INSERT into the new partition queues a referencing-side check event for the
new row before the root's UPDATE event (whose action trigger cascades) is
queued, so the check fires first and sees the old key gone. On a plain
table both triggers belong to one UPDATE event and the action trigger
(`RI_ConstraintTrigger_a_*`) fires before the check (`_c_*`).

Possibly intended (ordering of row movement events); needs a look at
`ExecCrossPartitionUpdateForeignKey` and the docs before any report.

Exclusion in ephyra-postgresql: no self-referencing foreign key on a
partitioned table (`fk::def_ok`).
