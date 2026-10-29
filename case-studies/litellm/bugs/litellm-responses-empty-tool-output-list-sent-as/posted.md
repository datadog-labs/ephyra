# [Bug]: responses() sends a function_call_output of empty text parts to the provider as the JSON text of the list

Posted as https://github.com/BerriAI/litellm/issues/48832 on 2026-10-28. The text below is the issue as posted.

---

### Description

With `litellm.responses()` on a chat provider (Anthropic, Gemini), a `function_call_output` whose `output` is a list of only empty text parts, e.g. `[{"type": "input_text", "text": ""}]`, reaches the provider as the JSON text of that list. Anthropic gets a `tool_result` with content `'[{"type": "input_text", "text": ""}]'`, and the model reads that as the tool's output. `output: []` becomes `"[]"` the same way

I'd expect an empty tool result, which is what litellm sends for `output: ""`. A list of non-empty text parts is already turned into its text, and the docstring of `_normalize_function_call_output_to_tool_content` says text-only outputs keep their string form, so this looks like the empty case falling through to the "last resort" JSON dump

Most clients send the tool output as a string, so this is low impact, but when it happens nothing errors and the model sees the wire format as the tool result

### Config

```yaml
# SDK only, no config.yaml
litellm.responses(model="anthropic/claude-sonnet-4-5", input=[...], api_base="http://127.0.0.1:<port>", api_key="x")
```

### LiteLLM Version

v1.106.5 and main at c7c895db. On a plain v1.106.5 install `litellm.responses()` also needs `pip install mcp` (#46045, fixed on main)

### Steps to Repro

The script sends the same conversation twice to a local mock of the Anthropic Messages API, once with `output: ""` and once with the empty part list, and prints the `tool_result` litellm sent each time

```python
# litellm.responses() on an Anthropic model. The tool output is [{"type": "input_text", "text": ""}]:
# one text part, empty. Anthropic should get an empty tool_result, the same as for output "".
# It gets the JSON text of the part list instead.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

litellm.suppress_debug_info = True
SENT = []

BODY = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn", "stop_sequence": None,
        "usage": {"input_tokens": 1, "output_tokens": 1}}

class MockAnthropic(BaseHTTPRequestHandler):
    def do_POST(self):
        SENT.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(BODY).encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockAnthropic)
threading.Thread(target=srv.serve_forever, daemon=True).start()

def tool_result_sent(output):
    SENT.clear()
    litellm.responses(
        model="anthropic/claude-sonnet-4-5", api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x",
        input=[
            {"role": "user", "content": "hi"},
            {"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "call_1", "output": output},
        ],
    )
    return SENT[0]["messages"][-1]["content"][0]

try:
    a = tool_result_sent("")
    b = tool_result_sent([{"type": "input_text", "text": ""}])
except Exception as e:
    print("setup error:", repr(e)[:300])
    raise SystemExit(2)
print('output "":                                ', json.dumps(a))
print('output [{"type":"input_text","text":""}]: ', json.dumps(b))
```

Output on v1.106.5 and on main c7c895db

```
output "":                                 {"type": "tool_result", "tool_use_id": "call_1", "content": ""}
output [{"type":"input_text","text":""}]:  {"type": "tool_result", "tool_use_id": "call_1", "content": "[{\"type\": \"input_text\", \"text\": \"\"}]"}
```

### Which part of LiteLLM is this about?

LLM translation: a specific provider's request or response

### How are you deploying?

pip / Python SDK

