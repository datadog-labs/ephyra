# pg-set-constraints-released-savepoint (candidate, not reported)

Found by ephyra-postgresql round fk-1 (`tests/cases.rs`), 2026-10-18;
confirmed on master 158b6d5109 (`~/Projects/postgres/inst`).

`SET CONSTRAINTS` run in a subtransaction that is then released is not
undone by `ROLLBACK TO` an enclosing savepoint, while the same command run
directly in that savepoint is (`minimal.sql`). The deferred-trigger state
is saved per subtransaction level only when `SET CONSTRAINTS` runs at that
level (`AfterTriggerSetState` saves `trans_stack[my_level].state`), and
`AfterTriggerEndSubXact` on commit of the inner level does not hand the
saved copy to the parent, so the parent's rollback has nothing to restore.
The docs say SET CONSTRAINTS is transactional like other settings.

Exclusion in ephyra-postgresql: SET CONSTRAINTS only with one savepoint
open (`semantics::valid`).
