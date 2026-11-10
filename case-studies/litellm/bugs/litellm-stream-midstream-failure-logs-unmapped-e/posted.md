# Posted: https://github.com/BerriAI/litellm/issues/49940

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: A stream that fails after it opens logs the raw provider exception class (e.g. AnthropicError) in error_information**

### Description

When a `completion(stream=True)` stream opens with HTTP 200 and then gets a provider error event, the caller receives `MidStreamFallbackError` wrapping the mapped `InternalServerError`, but the failure callbacks get the unmapped provider exception. `standard_logging_object["error_information"]["error_class"]` is `AnthropicError`, an internal class the caller never sees, so a logger or dashboard grouping failures by `error_class` sees a different class than the application caught

I'd expect `error_class` to name the exception litellm raises, which is what happens for the same overloaded error in the other two shapes I tried. Sent as HTTP 529 on a non-stream call, or as HTTP 529 on a stream that fails to open, the error is raised and logged as `InternalServerError` both times. The logging spec (https://docs.litellm.ai/docs/proxy/logging_spec) gives `"RateLimitError"` as the example `error_class`. The async path (`acompletion` with `async_log_failure_event`) logs `AnthropicError` too

### Config

```yaml
# SDK only, no config.yaml
litellm.callbacks = [Log()]   # CustomLogger that records kwargs["standard_logging_object"]["error_information"]
litellm.completion(model="anthropic/claude-sonnet-4-5", api_key="x", api_base="http://127.0.0.1:<port>", messages=[...], stream=True)
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

A local server stands in for Anthropic. It answers every POST with HTTP 200, `content-type: text/event-stream` and a single `event: error` with an `overloaded_error`. It runs offline

```python
import json, threading, time
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.integrations.custom_logger import CustomLogger

SSE = (
    'event: error\n'
    'data: {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}\n\n'
)

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        self.wfile.write(SSE.encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()

logged = []
class Log(CustomLogger):
    def log_failure_event(self, kwargs, response_obj, start_time, end_time):
        logged.append(kwargs["standard_logging_object"]["error_information"])
litellm.callbacks = [Log()]

try:
    for _ in litellm.completion(
        model="anthropic/claude-sonnet-4-5", api_key="x",
        api_base=f"http://127.0.0.1:{srv.server_address[1]}",
        messages=[{"role": "user", "content": "hi"}], stream=True,
    ):
        pass
except Exception as e:
    raised = getattr(e, "original_exception", None) or e
    print("raised:", type(e).__name__, "->", type(raised).__name__, raised.status_code)

for _ in range(50):
    if logged: break
    time.sleep(0.1)
info = logged[0]
print("logged:", info["error_class"], info["error_code"])
raise SystemExit(1 if info["error_class"] != type(raised).__name__ else 0)
```

Output on v1.106.6 and on main 0aa7079f (stdout)

```
raised: MidStreamFallbackError -> InternalServerError 500
logged: AnthropicError 500
```

With the server answering HTTP 529 and the same JSON error body instead (no stream opened), both a non-stream call and a `stream=True` call print

```
raised: InternalServerError 500
logged: InternalServerError 500
```

### Which part of LiteLLM is this about?

Logging: callbacks, Langfuse, Datadog, OTel, Prometheus, alerting

### How are you deploying?

pip / Python SDK
