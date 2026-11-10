poc.py starts a mock Anthropic /v1/messages server on 127.0.0.1, turns on the in-memory cache,
and sends the same non-streaming litellm.completion() call twice on an anthropic/ model. It needs
no network and no API key. The mock answers with one tool_use block and stop_reason "end_turn".

    pip install 'litellm[proxy]'   # or put an upstream checkout on PYTHONPATH (uv sync --extra proxy)
    ./repro.sh                     # or ./repro.sh /path/to/venv/bin/python

Exit 1 means the cached (second) call returned a different finish_reason than the first (the bug).
Exit 0 means both calls returned the same finish_reason. Exit 2 means a call raised, or the second
call reached the mock (no cache hit).

The proxy extra is needed even for SDK use: without it, the cache-hit path imports a proxy module
and raises ModuleNotFoundError. That is a separate problem.
