# Posted: https://github.com/BerriAI/litellm/issues/49928

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: max_messages is counted on the translated chat messages for responses() and anthropic.messages on bridged providers**

### Description

`Cache(max_messages=...)` (default 4, added in #43878) says requests with more `messages`, or more Responses API `input` items, than the limit are neither looked up nor stored. On the two bridges that call `litellm.completion()` underneath, the limit is checked against the translated chat messages instead of what the caller sent, so it fires in the wrong cases

`litellm.responses()` on `anthropic/` with 5 input items (a user message, two `function_call` items, two `function_call_output` items) is stored and the repeat is served from the cache. The two consecutive calls become one assistant message, so the inner chat request has 4 messages. With the items interleaved (5 chat messages) it is correctly not cached

`litellm.anthropic.messages.create()` on `gemini/` with 3 messages, where the last one carries 3 `tool_result` blocks, is never cached. The adapter emits one `role: "tool"` message per block, so the inner chat request has 5 messages. With 2 tool results it is cached, and async `acreate()` with 3 is cached too

I'd expect the limit to apply to the request the caller made, so the first case reaches the provider twice and the second case once

### Config

```yaml
# SDK only, no config.yaml
litellm.cache = Cache(type="local")  # max_messages defaults to 4
litellm.responses(model="anthropic/claude-sonnet-4-5", input=five_items)
litellm.anthropic.messages.create(model="gemini/gemini-2.5-flash", messages=three_messages, max_tokens=16)
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

The script answers both providers from one local mock server, so it runs offline

```python
# Cache(type="local") with the default max_messages=4. Each request is sent twice and the
# script counts how many requests reach the mock provider.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.caching.caching import Cache

litellm.suppress_debug_info = True
litellm.cache = Cache(type="local")  # max_messages defaults to 4

ANTHROPIC = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
             "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn",
             "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 1}}
GEMINI = {"candidates": [{"content": {"role": "model", "parts": [{"text": "ok"}]}, "finishReason": "STOP", "index": 0}],
          "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2}}
hits = []

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        hits.append(self.path)
        body = json.dumps(ANTHROPIC if self.path.startswith("/v1/messages") else GEMINI).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
base = "http://127.0.0.1:%d" % srv.server_port

# 1. responses() on anthropic/ with 5 input items (over the limit)
five_items = [
    {"role": "user", "content": "hi"},
    {"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{}"},
    {"type": "function_call", "call_id": "call_2", "name": "get_weather", "arguments": "{}"},
    {"type": "function_call_output", "call_id": "call_1", "output": "ok"},
    {"type": "function_call_output", "call_id": "call_2", "output": "ok"},
]
del hits[:]
for _ in range(2):
    r = litellm.responses(model="anthropic/claude-sonnet-4-5", input=five_items, api_base=base, api_key="x")
print("responses(), 5 input items: requests to the provider:", len(hits), "| 2nd cache_hit:", r._hidden_params.get("cache_hit"))

# 2. anthropic.messages.create() on gemini/ with 3 messages (under the limit)
three_messages = [
    {"role": "user", "content": "hi"},
    {"role": "assistant", "content": [{"type": "tool_use", "id": "call_%d" % i, "name": "get_weather",
                                       "input": {"city": "c%d" % i}} for i in range(3)]},
    {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_%d" % i, "content": "sunny"}
                                 for i in range(3)]},
]
del hits[:]
for _ in range(2):
    litellm.anthropic.messages.create(model="gemini/gemini-2.5-flash", messages=three_messages,
                                      max_tokens=16, api_base=base, api_key="x")
print("anthropic.messages.create(), 3 messages: requests to the provider:", len(hits))
```

Output on v1.106.6 and on main at 0aa7079f

```
responses(), 5 input items: requests to the provider: 1 | 2nd cache_hit: True
anthropic.messages.create(), 3 messages: requests to the provider: 2
```

### Which part of LiteLLM is this about?

Caching: response cache, Redis, semantic cache

### How are you deploying?

pip / Python SDK
