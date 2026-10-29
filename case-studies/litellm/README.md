# LiteLLM

Testing [LiteLLM](https://github.com/BerriAI/litellm), the Python SDK and proxy
that calls LLM providers through one OpenAI-style interface, with ephyra.

- [`harness/`](harness): the harness ephyra's autonomous agent wrote for LiteLLM
  (October 2026). An ephyra driver generates provider responses, conversations
  and errors, and a Python worker plays them through LiteLLM against mock
  providers (OpenAI, Anthropic, Gemini, Bedrock), checking streaming against
  non-streaming results, request translation, history round trips, caching,
  parameter handling, error mapping and the router. Its README says how it
  builds and runs. Paths under `/agent-run` are the agent host's run directory.
  The harness depends on the library here through a relative path. This is a
  snapshot from the middle of the run.

- [`bugs/`](bugs): the bugs it found that were reported upstream
  ([BerriAI/litellm](https://github.com/BerriAI/litellm/issues?q=is%3Aissue+author%3Aalpaylan)),
  each with the issue as posted (`posted.md`) and its reproduction;
  [`bugs/README.md`](bugs/README.md) lists them with their status.
