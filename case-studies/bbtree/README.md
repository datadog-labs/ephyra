# bbtree

Testing [bbtree](https://github.com/XiangpengHao/bbtree), a concurrent B-tree
with file, mmap and store backends, with ephyra.

- [`properties/`](properties): hand-written ephyra properties over the file maps,
  mmap, the store (single- and multi-threaded), the sync map's allocation
  refusals and the bulk map's invariants (built with `--cfg fuzzing`). They
  build against a checkout at the repository root:
  `git clone https://github.com/XiangpengHao/bbtree`.
- [`harness/`](harness): the seed harness for ephyra's autonomous agent, which
  then extended it during its run (October 2026); its README says how it builds
  and runs. Paths under `/agent-run` are the agent host's run directory.
- [`bugs/`](bugs): the bugs reported upstream, each with the issue as posted
  (`posted.md`), its reproduction and, where we had one, a fix;
  [`bugs/README.md`](bugs/README.md) lists them with their status.
