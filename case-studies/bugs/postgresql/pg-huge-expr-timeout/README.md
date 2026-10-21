# Candidate: a 240 KB query runs ~3 minutes past a 10 s statement_timeout, its backend killed by the OS

Found 2026-10-13 by ephyra-postgresql's query-validity test (`sqlancer::tests::drawn_queries_run`,
seed 56, before the generator got a node budget). PostgreSQL master 158b6d5109 (2026-10-13),
built with `--enable-cassert --enable-debug`, CFLAGS `-O2 -g`, at ~/Projects/postgres/inst.

- `setup.sql`: the tables (load into an empty schema; COPY data inline, run with psql).
- `query.sql`: the query (one line, 240113 bytes): a CTE, then `SELECT * FROM t2 WHERE <deeply nested
  boolean expression of constants>`.

Server log: the backend ran from 11:49:28 to 11:52:37 and was "terminated by signal 9: Killed: 9"
(macOS memory pressure), although the session had `statement_timeout = '10s'`.

Open questions: which phase (parse, analyze, eval_const_expressions, canonicalize_qual, ...) takes
the time and memory; whether it is exponential in the nesting; whether a CHECK_FOR_INTERRUPTS is
missing there; whether it is already known.
