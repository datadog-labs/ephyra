-- psql -X -d postgres -f repro.sql   (psql on PATH, PGHOST/PGPORT set)
DROP DATABASE IF EXISTS toast_other;
CREATE DATABASE toast_other;
DROP DATABASE IF EXISTS toast_repro;
CREATE DATABASE toast_repro;
\c toast_repro
CREATE TABLE t (id int, b text STORAGE EXTERNAL);
INSERT INTO t VALUES (1, repeat('x', 10000));
SELECT reltoastrelid::regclass AS toast FROM pg_class WHERE oid = 't'::regclass \gset
-- another session, in another database, holds an xid
\! psql -X -d toast_other -Atc "SELECT pg_current_xact_id(), pg_sleep(4)" >/dev/null &
SELECT pg_sleep(1);
UPDATE t SET b = NULL;
VACUUM :toast;   -- its horizon ignores the other database's xid
VACUUM FULL t;
SELECT id, b IS NULL FROM t;
SELECT pg_sleep(4);
