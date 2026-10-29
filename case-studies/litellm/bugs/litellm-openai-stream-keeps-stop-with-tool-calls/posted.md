# [Bug]: OpenAI-compatible streaming keeps finish_reason "stop" when the response has tool calls, non-streaming reports "tool_calls"

Posted as https://github.com/BerriAI/litellm/issues/48716 on 2026-10-28. The text below is the issue as posted.

---

### Description

When an OpenAI-compatible server returns a tool call with `finish_reason: "stop"`, litellm reports `"tool_calls"` without streaming but keeps `"stop"` with `stream=True`. The last streamed chunk says `"stop"` and so does `litellm.stream_chunk_builder(chunks)`

litellm already rewrites stop to tool_calls on both paths, in `convert_dict_to_response.py` for non-streaming and in `CustomStreamWrapper.finish_reason_handler` for streaming, so I'd expect `"tool_calls"` in both cases. In the stream the rewrite depends on `self.tool_call`, and that flag stays False for these chunks, so it never fires

It happens both when the tool call and the finish reason arrive in separate chunks and when they arrive in the same chunk, and it's the same with `hosted_vllm/` instead of `openai/`. Clients that only take the tool path when `finish_reason == "tool_calls"` treat the streamed turn as a normal end of turn

#35663 looks like the same symptom for `ollama_chat`, which goes through the ollama chunk parser rather than this path

The Gemini stream parser has a separate mismatch of the same kind, with MAX_TOKENS, in #48715

### Config

```yaml
# SDK only, no config.yaml
litellm.completion(model="openai/m", api_base="http://127.0.0.1:<port>/v1", api_key="x", messages=[{"role": "user", "content": "weather?"}], stream=True)
```

### LiteLLM Version

v1.106.5, also main at c7c895db and as far back as v1.80.0

### Steps to Repro

The script serves one tool call response from a local mock server, so it runs offline with only litellm installed

```python
# An OpenAI-compatible server answers with one tool call and finish_reason "stop".
# litellm reports "tool_calls" without stream=True, but "stop" with stream=True.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

litellm.suppress_debug_info = True

TOOL_CALL = {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}

def chunk(delta, finish_reason=None):
    return {"id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "m",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]}

BODY = {"id": "c1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "finish_reason": "stop",
                     "message": {"role": "assistant", "content": None, "tool_calls": [TOOL_CALL]}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
SSE = "".join("data: %s\n\n" % json.dumps(c) for c in [
    chunk({"role": "assistant", "content": None, "tool_calls": [dict(TOOL_CALL, index=0)]}),
    chunk({}, "stop"),
]) + "data: [DONE]\n\n"

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        out = SSE if req.get("stream") else json.dumps(BODY)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream" if req.get("stream") else "application/json")
        self.end_headers()
        self.wfile.write(out.encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
args = dict(model="openai/m", api_base="http://127.0.0.1:%d/v1" % srv.server_port, api_key="x",
            messages=[{"role": "user", "content": "weather?"}])

plain = litellm.completion(**args).choices[0].finish_reason
chunks = list(litellm.completion(**args, stream=True))
last = [c.choices[0].finish_reason for c in chunks if c.choices][-1]
built = litellm.stream_chunk_builder(chunks).choices[0].finish_reason
print("stream=False:", plain)
print("stream=True last chunk:", last)
print("stream_chunk_builder:", built)
```

Output on v1.106.5 and on main c7c895db

```
stream=False: tool_calls
stream=True last chunk: stop
stream_chunk_builder: stop
```

### Which part of LiteLLM is this about?

Python SDK: the litellm package itself

### How are you deploying?

pip / Python SDK

