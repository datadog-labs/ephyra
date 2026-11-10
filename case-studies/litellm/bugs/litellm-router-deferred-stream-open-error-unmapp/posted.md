# Posted: https://github.com/BerriAI/litellm/issues/49933

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: Router stream=True with a Gemini deployment raises the raw VertexAIError (async) or a TypeError (sync) when the stream fails to open**

### Description

When a `gemini/` deployment answers a streaming request with an HTTP error, `litellm.completion(stream=True)` and `litellm.acompletion(stream=True)` raise the mapped `litellm.BadRequestError`. Going through `litellm.Router` with the same deployment and `num_retries=0` gives something else. `Router.acompletion(stream=True)` raises `litellm.llms.vertex_ai.common_utils.VertexAIError`, which is not a litellm or openai exception class, and the sync `Router.completion(stream=True)` raises `TypeError: can't concat str to bytes`, so the provider error never reaches the caller at all. In both cases `except litellm.BadRequestError` (or `except openai.APIError`) does not catch it

I'd expect the Router to raise the same mapped exception as `litellm.completion` and `litellm.acompletion`. These are the streams that are opened lazily by `fetch_stream()` / `fetch_sync_stream()` in `Router._acompletion` and `Router._completion`, after `litellm.acompletion` has returned, so the error does not go through the mapping in `CustomStreamWrapper`. #42757 (open) is about the same call site but a different effect (no failure callbacks and no cooldown)

The sync traceback ends in `async_function_with_fallbacks_common_utils`, at `original_exception.message += format_fallback_outcome_message(...)`, because the unmapped exception's `message` is the raw bytes response body. With `litellm.expose_router_debug_in_errors = False` the TypeError goes away and the raw `MaskedHTTPStatusError` comes out instead

### Config

```yaml
# SDK only, no config.yaml
router = litellm.Router(model_list=[{"model_name": "m", "litellm_params": {"model": "gemini/gemini-2.5-flash", "api_base": "http://127.0.0.1:<port>", "api_key": "fake-key"}}], num_retries=0)
router.completion(model="m", messages=[...], stream=True)          # or await router.acompletion(...)
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

A local server stands in for the Gemini API and answers every request with HTTP 400, so this runs offline

```python
import asyncio, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
litellm.suppress_debug_info = True

class Gemini400(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        body = b'{"error": {"code": 400, "message": "bad request", "status": "INVALID_ARGUMENT"}}'
        self.send_response(400)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass

server = HTTPServer(("127.0.0.1", 0), Gemini400)
threading.Thread(target=server.serve_forever, daemon=True).start()
params = {"model": "gemini/gemini-2.5-flash", "api_base": f"http://127.0.0.1:{server.server_port}", "api_key": "fake-key"}
messages = [{"role": "user", "content": "hi"}]
router = litellm.Router(model_list=[{"model_name": "m", "litellm_params": params}], num_retries=0)

def show(label, e):
    print(f"{label:20} {type(e).__module__}.{type(e).__name__}: {str(e)[:60]!r}")
    print(f"{'':20} isinstance(e, litellm.BadRequestError) = {isinstance(e, litellm.BadRequestError)}")

async def consume(stream):
    async for _ in stream:
        pass

async def acomp():
    await consume(await litellm.acompletion(**params, messages=messages, stream=True))

async def arouter():
    await consume(await router.acompletion(model="m", messages=messages, stream=True))

def srouter():
    for _ in router.completion(model="m", messages=messages, stream=True):
        pass

for label, fn in [
    ("litellm.completion", lambda: list(litellm.completion(**params, messages=messages, stream=True))),
    ("litellm.acompletion", lambda: asyncio.run(acomp())),
    ("Router.completion", srouter),
    ("Router.acompletion", lambda: asyncio.run(arouter())),
]:
    try:
        fn()
        print(label, "no exception")
    except Exception as e:
        show(label, e)
```

Output on v1.106.6 and on main 0aa7079f (stdout)

```
litellm.completion   litellm.exceptions.BadRequestError: 'litellm.BadRequestError: Vertex_ai_betaException BadRequestE'
                     isinstance(e, litellm.BadRequestError) = True
litellm.acompletion  litellm.exceptions.BadRequestError: 'litellm.BadRequestError: Vertex_ai_betaException BadRequestE'
                     isinstance(e, litellm.BadRequestError) = True
Router.completion    builtins.TypeError: "can't concat str to bytes"
                     isinstance(e, litellm.BadRequestError) = False
Router.acompletion   litellm.llms.vertex_ai.common_utils.VertexAIError: 'b\'{"error": {"code": 400, "message": "bad request", "status"'
                     isinstance(e, litellm.BadRequestError) = False
```

### Which part of LiteLLM is this about?

Routing: load balancing, fallbacks, retries, cooldowns

### How are you deploying?

pip / Python SDK
