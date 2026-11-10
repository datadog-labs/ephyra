# Posted: https://github.com/BerriAI/litellm/issues/49938

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: Sync Router.responses() ignores the Router's num_retries and fallbacks, Router.aresponses() honours them**

### Description

With `litellm.Router(num_retries=1)`, a sync `router.responses(...)` call whose first request gets an HTTP 503 makes exactly one request and raises `ServiceUnavailableError`. The same call through `await router.aresponses(...)` on the same Router retries once and succeeds. Fallbacks behave the same way. With `fallbacks=[{"primary": ["backup"]}]` and `primary` always answering 503, `aresponses` ends up on `backup` and succeeds, while `responses` only ever calls `primary` and raises

I'd expect the sync method to apply the Router's retries and fallbacks like the async one. The Router docstring describes `num_retries` as "Number of retries for failed requests", and the docstring of `Router._generic_api_call_with_fallbacks`, which the sync `responses` goes through, says it "allows you to use retries/fallbacks with litellm router". #44791 notes in passing that this sync path makes one attempt and keeps the SDK level retries from a deployment's own `litellm_params.num_retries`, but the Router level `num_retries` and `fallbacks` are not applied. #35071 is a different gap: per-deployment `num_retries` and `cooldown_time` on the async `aresponses` and `aanthropic_messages` paths With the default `num_retries` (no explicit value) the sync call also makes one request

### Config

```yaml
# SDK only, no config.yaml
router = litellm.Router(model_list=[{"model_name": "my-model", "litellm_params": {"model": "openai/gpt-4o", "api_key": "sk-test", "api_base": "http://127.0.0.1:<port>/v1"}}], num_retries=1, retry_after=0, disable_cooldowns=True)
router.responses(model="my-model", input="hi")
```

`retry_after=0` and `disable_cooldowns=True` only keep the example fast

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

A local OpenAI-style server answers the first `POST /v1/responses` with 503 and every later one with 200. The script runs the same call through `aresponses` and `responses` and counts requests. It runs offline

```python
import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import litellm

calls = 0
OK = {"id": "resp_1", "object": "response", "created_at": 0, "status": "completed", "model": "gpt-4o",
      "output": [], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}
ERR = {"error": {"message": "down", "type": "server_error"}}

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        global calls
        self.rfile.read(int(self.headers["content-length"]))
        calls += 1
        status, body = (503, ERR) if calls == 1 else (200, OK)
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *args): pass

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()

router = litellm.Router(
    model_list=[{"model_name": "my-model", "litellm_params": {
        "model": "openai/gpt-4o", "api_key": "sk-test",
        "api_base": f"http://127.0.0.1:{server.server_port}/v1"}}],
    num_retries=1, retry_after=0, disable_cooldowns=True)

def run(name, call):
    global calls
    calls = 0
    try:
        resp = call()
        result = f"status={resp.status}"
    except Exception as e:
        result = f"raised {type(e).__name__}"
    print(f"{name}: requests={calls} {result}")

run("aresponses", lambda: asyncio.run(router.aresponses(model="my-model", input="hi")))
run("responses", lambda: router.responses(model="my-model", input="hi"))
```

Output on v1.106.6 and on main 0aa7079f (stdout)

```
aresponses: requests=2 status=completed
responses: requests=1 raised ServiceUnavailableError
```

With two model groups, `primary` always 503 and `backup` 200, `num_retries=0` and `fallbacks=[{"primary": ["backup"]}]`, printing which groups got a request

```
aresponses ['primary', 'backup'] status=completed
responses ['primary'] raised ServiceUnavailableError
```

### Which part of LiteLLM is this about?

Routing: load balancing, fallbacks, retries, cooldowns

### How are you deploying?

pip / Python SDK
