# ty

Testing [ty](https://github.com/astral-sh/ty), Astral's Python type checker
(developed in [astral-sh/ruff](https://github.com/astral-sh/ruff)), with ephyra.

- [`harness/`](harness): the seed harness (`tyz`) for ephyra's autonomous agent,
  extended by the agent during its run (October 2026); its README says how it
  builds and runs. Paths under `/agent-run` are the agent host's run directory.
  The harness depends on the library here through a relative path.

The bugs it found were reported as
[astral-sh/ty#4825 to #4831](https://github.com/astral-sh/ty/issues?q=is%3Aissue+author%3Aalpaylan);
their folders will be added here.
