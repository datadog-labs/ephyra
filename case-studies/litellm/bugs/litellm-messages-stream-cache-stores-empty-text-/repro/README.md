# messages-stream-cache-stores-empty-text-

`./repro.sh [python]` runs `poc.py` with an interpreter that can import litellm
(`pip install "litellm[proxy]"`, or an upstream checkout on PYTHONPATH with its deps).
Offline: poc.py starts its own mock Gemini server on 127.0.0.1.

It calls `litellm.anthropic.messages.acreate(model="gemini/...", stream=True)` twice with the
in-memory cache on. The mock answers with an empty reply. Exit 1 = the second call was served
from the cache (bug), 0 = both calls reached the mock, 2 = setup error.
