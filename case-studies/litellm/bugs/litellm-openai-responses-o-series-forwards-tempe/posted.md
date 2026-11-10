# Posted: https://github.com/BerriAI/litellm/issues/49927

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: responses() sends temperature to OpenAI o-series models even with drop_params=True**

### Description

`litellm.responses(model="openai/o3-mini", temperature=0.5, drop_params=True)` still sends `"temperature": 0.5` to OpenAI. O-series models only accept the default temperature, so the real API rejects the request. The same arguments through `litellm.completion()` drop `temperature` as expected. It is the same with `openai/o1`, `openai/o3` and `openai/o4-mini`

The Azure Responses config already does this drop (`litellm/llms/azure/responses/o_series_transformation.py` says O-series models do not support the temperature parameter in the responses API, so it is dropped when drop_params is enabled). I'd expect the `openai/` Responses path to drop it the same way

### Config

```yaml
# SDK only, no config.yaml
litellm.responses(model="openai/o3-mini", input="hi", temperature=0.5, drop_params=True)
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

The script answers both endpoints from a local mock server, so it runs offline

```python
# Same o-series model and arguments through completion() and responses(), both with
# temperature=0.5 and drop_params=True. A local mock stands in for api.openai.com and
# records the request body litellm sends.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

CHAT = {"id": "c1", "object": "chat.completion", "created": 0, "model": "o3-mini",
        "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
RESP = {"id": "resp_1", "object": "response", "created_at": 0, "status": "completed", "model": "o3-mini",
        "output": [{"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
                    "content": [{"type": "output_text", "text": "hi", "annotations": []}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}
sent = []

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        sent.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
        body = json.dumps(RESP if self.path.endswith("/responses") else CHAT).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
kw = dict(model="openai/o3-mini", temperature=0.5, drop_params=True,
          api_base="http://127.0.0.1:%d/v1" % srv.server_port, api_key="sk-test")

litellm.completion(messages=[{"role": "user", "content": "hi"}], **kw)
print("completion() sent temperature:", "temperature" in sent[-1])
litellm.responses(input="hi", **kw)
print("responses()  sent temperature:", "temperature" in sent[-1], sent[-1])
```

Output on v1.106.6 and on main at 0aa7079f

```
completion() sent temperature: False
responses()  sent temperature: True {'model': 'o3-mini', 'input': 'hi', 'temperature': 0.5}
```

### Which part of LiteLLM is this about?

LLM translation: a specific provider's request or response

### How are you deploying?

pip / Python SDK
