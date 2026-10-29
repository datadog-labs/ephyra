# sqlparser

Testing [sqlparser-rs](https://github.com/apache/datafusion-sqlparser-rs), the
SQL parser behind DataFusion, with ephyra: a coding agent wrote property tests
starting from [STARTUP.md](../../STARTUP.md) alone (October 2026), checking that
displaying a statement and parsing it back gives the same tree, that a
statement doesn't swallow the next one, and that comments and keyword case
don't change the tree.

- [`bugs/`](bugs): the bugs reported upstream so far, each with the issue as
  posted (`posted.md`) and its reproduction; [`bugs/README.md`](bugs/README.md)
  lists them with their status. The harness follows once the remaining
  findings are reported.
