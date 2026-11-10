# Posted: https://github.com/BerriAI/litellm/issues/49931

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: anthropic.messages.acreate(stream=True) caches an empty reply from a non-Anthropic model and replays it on every repeat**

### Description

With the response cache on, `litellm.anthropic.messages.acreate(stream=True)` on a `gemini/` model stores a reply that has no content. The adapter turns the empty Gemini reply into a stream with one empty text block, and that stream is cached. Every later identical async call is answered from the cache with the same empty reply and never reaches the provider again

The response cache skips answers with no output on the other paths. The same empty reply through `litellm.acompletion(stream=True)` or the sync `litellm.anthropic.messages.create(stream=True)` is not cached, and the repeat reaches the provider. I'd expect the async Messages stream to behave the same, so a transient empty answer does not stick. #47576 is about the empty text block itself, this report is only about it being cached

### Config

```yaml
# SDK only, no config.yaml
litellm.cache = Cache(type="local")
await litellm.anthropic.messages.acreate(model="gemini/gemini-2.5-flash", messages=[{"role": "user", "content": "hi"}], max_tokens=16, stream=True)  # twice
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

The script serves the empty Gemini reply from a local mock server, so it runs offline

```python
# litellm.anthropic.messages.acreate(stream=True) twice, same request, in-memory cache on.
# The model is a Gemini model; the mock Gemini server answers with an empty reply (one empty text part).
# litellm does not cache a reply with no output, so the second call should reach the server again.
import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.caching.caching import Cache

litellm.suppress_debug_info = True
litellm.cache = Cache(type="local")

CHUNK = {"candidates": [{"content": {"role": "model", "parts": [{"text": ""}]},
                         "finishReason": "STOP", "index": 0}],
         "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 0, "totalTokenCount": 1},
         "modelVersion": "gemini-2.5-flash", "responseId": "resp-1"}
hits = []

class MockGemini(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        hits.append(self.path)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        self.wfile.write(b"data: " + json.dumps(CHUNK).encode() + b"\r\n\r\n")
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockGemini)
threading.Thread(target=srv.serve_forever, daemon=True).start()

async def call():
    s = await litellm.anthropic.messages.acreate(
        model="gemini/gemini-2.5-flash", messages=[{"role": "user", "content": "hi"}], max_tokens=16,
        stream=True, api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x")
    return b"".join([e if isinstance(e, bytes) else str(e).encode() async for e in s]).decode()

async def main():
    try:
        first = await call()
        await asyncio.sleep(0.5)  # let the stream's cache write finish
        second = await call()
    except Exception as e:
        print("setup error:", repr(e)[:300])
        return 2
    print("call 1 events:", [l[7:] for l in first.splitlines() if l.startswith("event: ")])
    print("call 2 events:", [l[7:] for l in second.splitlines() if l.startswith("event: ")])
    print("call 2 content_block_start:", [l for l in second.splitlines() if '"content_block_start"' in l])
    print("requests to the server:", len(hits))
    return 0 if len(hits) == 2 else 1

raise SystemExit(asyncio.run(main()))
```

Output on v1.106.6 and on main at 0aa7079f

```
call 1 events: ['message_start', 'content_block_start', 'content_block_stop', 'message_delta', 'message_stop']
call 2 events: ['message_start', 'content_block_start', 'content_block_stop', 'message_delta', 'message_stop']
call 2 content_block_start: ['data: {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}']
requests to the server: 1
```

With `litellm.acompletion(stream=True)` or the sync `litellm.anthropic.messages.create(stream=True)` in place of `acreate`, the same script prints `requests to the server: 2`

### Which part of LiteLLM is this about?

Caching: response cache, Redis, semantic cache

### How are you deploying?

pip / Python SDK
