poc.py starts a mock Anthropic /v1/messages server on 127.0.0.1, turns on the in-memory
cache, and sends the same litellm.responses() call twice. No network or API key needed.

    pip install litellm mcp      # or put an upstream checkout on PYTHONPATH
    ./repro.sh                   # or ./repro.sh /path/to/venv/bin/python

Exit 1: the second (cached) call raised APIError (bug). Exit 0: both calls returned the
same response. Exit 2: the first call failed (setup problem).
