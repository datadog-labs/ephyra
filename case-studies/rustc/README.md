# rustc

Testing [rustc](https://github.com/rust-lang/rust), the Rust compiler, on generated MIR programs, with ephyra.

- [`harness/`](harness): the test harness, written by ephyra's autonomous agent during
  its runs (October 2026), with its README on how it builds and runs. Paths
  under `/agent-run` are the agent host's run directory: the target's checkout
  (`/agent-run/work/src`), reference implementations (`/agent-run/work/ref`) and
  caches. Set the environment variables the scripts read to point elsewhere. The
  harness depends on the library here through a relative path.
