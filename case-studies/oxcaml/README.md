# OxCaml

Testing [OxCaml](https://github.com/oxcaml/oxcaml), OxCaml, Jane Street's OCaml compiler branch, with ephyra.

- [`harness/`](harness): the test harness, written by ephyra's autonomous agent during
  its runs (October 2026), with its README on how it builds and runs. Paths
  under `/agent-run` are the agent host's run directory: the target's checkout
  (`/agent-run/work/src`), reference implementations (`/agent-run/work/ref`) and
  caches. Set the environment variables the scripts read to point elsewhere. The
  harness depends on the library here through a relative path.
- [`bugs/`](bugs): the bugs it found and that were reported upstream, each
  with the issue as posted (`posted.md`, or the report as filed), its
  reproduction and, where we had one, a fix; [`bugs/README.md`](bugs/README.md)
  lists them with their status.
