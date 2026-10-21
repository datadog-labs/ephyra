Filed as #10341 (https://github.com/tursodatabase/turso/issues/10341) on 2026-10-21.
Title: VACUUM INTO ':memory:' writes a file named ":memory:" in the current directory; VACUUM INTO '' is rejected

```sql
CREATE TABLE t(a);
INSERT INTO t VALUES (1);
VACUUM INTO ':memory:';
VACUUM INTO ':memory:';
VACUUM INTO '';
```

Turso (run in an empty directory):
```
Parse error: output file already exists: :memory:
Parse error: VACUUM INTO path cannot be empty
```
and the directory now holds files named `:memory:` and `:memory:-wal`.

SQLite 3.50.4 (and trunk): all three statements succeed with no output, and no file is created.

Expected: SQLite opens the VACUUM INTO target the way ATTACH does, so `':memory:'` is an in-memory database and `''` is a private temporary database; in both cases the copy is discarded and nothing is written to the current directory. Turso treats `':memory:'` as a file name, which leaves a stray file behind and makes every later `VACUUM INTO ':memory:'` from that directory fail. If in-memory targets are not meant to be supported (#7230 suggests rejecting `mode=memory`), an error before any file is written would be better than creating `./:memory:`, and COMPAT.md does not mention either case.

Tested on Turso main cbd5ac4e.
