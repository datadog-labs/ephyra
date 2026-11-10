# Posted: https://github.com/BerriAI/litellm/issues/49925

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: Cached responses() call on an Anthropic model fails with "provider returned a response with no 'choices'"**

### Description

With the response cache on, `litellm.responses()` on a model that goes through the Responses to chat completions bridge (here `anthropic/`) works the first time and stores the response. The same call a second time is a cache hit and raises `litellm.APIError: LiteLLM: provider returned a response with no 'choices'`. Every repeat of the request fails while the entry is in the cache

The stored entry is a Responses API object (`"object": "response"` with `output`), but its `id` is the bridge's `chatcmpl-...` id. The cache hit path seems to take that id prefix as a sign of a chat completion and rebuilds it as one, which needs `choices`

I'd expect the second call to return the cached response. `responses` is in the default supported call types of the cache

### Config

```yaml
# SDK only, no config.yaml
litellm.cache = Cache(type="local")
litellm.responses(model="anthropic/claude-sonnet-4-5", input="hi", api_base="http://127.0.0.1:<port>", api_key="x")  # twice
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

The script serves the Anthropic reply from a local mock server, so it runs offline

```python
# The same litellm.responses() call twice with the in-memory cache on, on an Anthropic model
# (served through the Responses to chat completions bridge). The second call should be a cache hit.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.caching.caching import Cache

litellm.suppress_debug_info = True
litellm.cache = Cache(type="local")

BODY = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "hello"}], "stop_reason": "end_turn", "stop_sequence": None,
        "usage": {"input_tokens": 1, "output_tokens": 1}}

class MockAnthropic(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        data = json.dumps(BODY).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockAnthropic)
threading.Thread(target=srv.serve_forever, daemon=True).start()

for i in (1, 2):
    try:
        r = litellm.responses(model="anthropic/claude-sonnet-4-5", input="hi",
                              api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x")
        print("call %d:" % i, r.id[:9], r.object, r.output_text)
    except Exception as e:
        print("call %d:" % i, type(e).__name__, str(e)[:110])
```

Output on v1.106.6 and on main at 0aa7079f

```
call 1: chatcmpl- response hello
call 2: APIError litellm.APIError: LiteLLM: provider returned a response with no 'choices'. Raw keys: ['id', 'created_at', 'err
```

### Which part of LiteLLM is this about?

Caching: response cache, Redis, semantic cache

### How are you deploying?

pip / Python SDK
