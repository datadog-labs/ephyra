# responses() bridge: over-limit request is cached

- `poc.py`: two identical `litellm.responses()` calls to `anthropic/` with 5 input items
  (user, 2 function_call, 2 function_call_output). A local mock stands in for the Anthropic API.
  `Cache(type="local")` has `max_messages=4`, so neither call should touch the cache.
  Bug: the second call is served from the cache (1 provider request, `cache_hit: True`). Exit 1.
- `contrast_interleaved.py`: the same 5 items, interleaved (user, call_1, output_1, call_2, output_2).
  Not cached (2 provider requests). Exit 0.
- `repro.sh [python]`: runs poc.py, exit 1 = reproduces, 0 = fixed, 2 = cannot decide.

Needs `litellm[proxy]` (plain `litellm` from PyPI fails the cache lookup with a missing `fastapi` import).
