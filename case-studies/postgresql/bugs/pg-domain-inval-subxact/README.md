# pg-domain-inval-subxact: "cannot start a subtransaction when there are unprocessed inval messages"

Found 2026-10-22 by round postgresql/round/alter-1 (`tests/sqlancer.rs`, norec and tlp, two
different shapes). Not reported anywhere.

## Symptom

Inside a transaction block, after `ALTER DOMAIN .. DROP CONSTRAINT` of a domain that a column uses
(domain created and the column changed to it in the same transaction), a later statement that fails
inside a subtransaction (a PL/pgSQL block with an EXCEPTION clause) does not raise its own error
but an internal one:

    ERROR:  cannot start a subtransaction when there are unprocessed inval messages   (XX000)

`minimal.sql` reproduces it every time on master 158b6d5109f (assert-enabled,
`~/Projects/postgres/inst`): the `ALTER DOMAIN .. ADD CONSTRAINT` in the DO block should fail with
23514 and be caught. The second shape found was the same prefix followed by a DO block whose
`ALTER TABLE .. ALTER COLUMN .. TYPE int8` fails with 22003.

Controls (each did not fail): the same without the DROP CONSTRAINT in the transaction; the failing
statement in `SAVEPOINT` / `ROLLBACK TO` with the domain created before the transaction and no
DROP CONSTRAINT in it. Outside a transaction block the DO block works.

## Notes

The error comes from the check that a subtransaction does not start with invalidation messages
queued but not yet processed, which suggests that `ALTER DOMAIN .. DROP CONSTRAINT` queues an
invalidation (for the domain's type, or the dependent column's relation) that no
CommandCounterIncrement processes before the statement ends. Not checked in the source.

## Fuzzer exclusion

ephyra-postgresql (round/alter-1): `ALTER DOMAIN .. DROP CONSTRAINT` runs only outside a transaction
block (`crate::semantics::alter_domain`).
