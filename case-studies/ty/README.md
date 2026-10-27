# ty

Testing [ty](https://github.com/astral-sh/ty), Astral's Python type checker
(developed in [astral-sh/ruff](https://github.com/astral-sh/ruff)), with ephyra.

- [`harness/`](harness): the seed harness (`tyz`) for ephyra's autonomous agent,
  extended by the agent during its run (October 2026); its README says how it
  builds and runs. Paths under `/agent-run` are the agent host's run directory.
  The harness depends on the library here through a relative path.

- [`bugs/`](bugs): the bugs it found that were reported upstream
  ([astral-sh/ty](https://github.com/astral-sh/ty/issues?q=is%3Aissue+author%3Aalpaylan)),
  each with the issue as posted (`posted.md`) and its reproduction;
  [`bugs/README.md`](bugs/README.md) lists them with their status.
