# [Bug]: Gemini streaming reports finish_reason "length" for a tool call that ends with MAX_TOKENS, non-streaming reports "tool_calls"

Posted as https://github.com/BerriAI/litellm/issues/48715 on 2026-10-28. The text below is the issue as posted.

---

### Description

When Gemini returns a function call and the reply ends with `finishReason: "MAX_TOKENS"`, `litellm.completion` returns `finish_reason="tool_calls"` without streaming but `"length"` with `stream=True`, for the same reply

Within streaming the answer also depends on where the chunk boundary falls. If the function call and MAX_TOKENS arrive in the same chunk the stream reports `"tool_calls"`, and if Gemini sends them in two chunks (the usual shape) it reports `"length"`

I'd expect one answer for one reply. The Gemini stream parser tracks `has_seen_tool_calls` so that it can report `"tool_calls"` when the call and the finish reason come in separate chunks, but the override for an empty-text final chunk only rewrites `"stop"`. With STOP both paths give `"tool_calls"` and with SAFETY both give `"content_filter"`, so only MAX_TOKENS disagrees

Related, #21577 added `has_seen_tool_calls` and #23895 added the empty-text final chunk override. Open PR #31634 changes finish_reason on the intermediate tool call chunks and doesn't affect this case

The same stream vs non-stream mismatch for OpenAI-compatible providers, in the generic stream wrapper, is #48716

### Config

```yaml
# SDK only, no config.yaml
litellm.completion(model="gemini/gemini-2.5-flash", messages=[{"role": "user", "content": "weather?"}], api_base="http://127.0.0.1:<port>", api_key="x", stream=True)
```

### LiteLLM Version

v1.106.5, also main at c7c895db and as far back as v1.80.0

### Steps to Repro

The script serves one Gemini reply from a local mock server, so it runs offline with only litellm installed

```python
# Gemini replies with a tool call, then hits MAX_TOKENS. A local mock server
# returns the same reply as JSON (non-streaming) and as SSE (streaming).
import json, threading, litellm
from http.server import BaseHTTPRequestHandler, HTTPServer

CALL = {"functionCall": {"name": "get_weather", "args": {}}}
USAGE = {"promptTokenCount": 5, "candidatesTokenCount": 5, "totalTokenCount": 10}
BODY = {"candidates": [{"content": {"role": "model", "parts": [CALL, {"text": ""}]},
                        "finishReason": "MAX_TOKENS", "index": 0}], "usageMetadata": USAGE}
SSE = [{"candidates": [{"content": {"role": "model", "parts": [CALL]}, "index": 0}]},
       {"candidates": [{"content": {"role": "model", "parts": [{"text": ""}]},
                        "finishReason": "MAX_TOKENS", "index": 0}], "usageMetadata": USAGE}]

class Mock(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        if "streamGenerateContent" in self.path:
            data = "".join(f"data: {json.dumps(o)}\r\n\r\n" for o in SSE).encode()
            ctype = "text/event-stream"
        else:
            data, ctype = json.dumps(BODY).encode(), "application/json"
        self.send_response(200)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
kw = dict(model="gemini/gemini-2.5-flash", messages=[{"role": "user", "content": "weather?"}],
          api_base=f"http://127.0.0.1:{srv.server_port}", api_key="x")

r = litellm.completion(**kw)
print("non-stream finish_reason:", r.choices[0].finish_reason)
reasons = [c.choices[0].finish_reason for c in litellm.completion(stream=True, **kw)
           if c.choices and c.choices[0].finish_reason]
print("stream finish_reason:    ", reasons[-1])
```

Output on v1.106.5 and on main c7c895db

```
non-stream finish_reason: tool_calls
stream finish_reason:     length
```

### Which part of LiteLLM is this about?

LLM translation: a specific provider's request or response

### How are you deploying?

pip / Python SDK

