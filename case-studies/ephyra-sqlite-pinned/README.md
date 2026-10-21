# ephyra-sqlite-pinned

The generator of [ephyra-sqlite](../ephyra-sqlite) as of 2026-10-13, frozen
for [turso-diff](../turso-diff). turso-diff draws its inputs (a database
file, statements, queries) from `ephyra_sqlite::probes::case_queries` and
shrinks findings with the same generator; depending on this copy instead of
`../ephyra-sqlite` keeps its runs reproducible by seed and keeps changes in
progress in ephyra-sqlite from breaking its build. The package name is
still `ephyra-sqlite`, so the code that uses it is the same.

It is the same design as ephyra-sqlite (generated files, a model of the
statements, SQLancer-style oracles; see that README), at an earlier point:
no virtual tables (`vtabs.rs`: R-Tree, Geopoly, FTS5) and a smaller
statement and expression language. Nothing is developed here; when
turso-diff moves to a newer generator, this directory is replaced by a copy
of ephyra-sqlite.

## Build and run

Run `case-studies/vendor/fetch.sh` first. Built on its own, the crate
patches in SQLite trunk (`case-studies/vendor/libsqlite3-sys-trunk`) as
ephyra-sqlite does; built as turso-diff's dependency, turso-diff's own patch
applies and the generator judges cases with SQLite 3.50.4, the version
Turso tracks.

```bash
cargo test --release                     # the model property and the oracles, as in ephyra-sqlite
cargo run --release --example sqlancer -- tlp 0 10000
```
