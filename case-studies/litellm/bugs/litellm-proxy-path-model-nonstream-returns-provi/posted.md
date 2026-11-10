# Posted: https://github.com/BerriAI/litellm/issues/49932

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: /engines/{model}/chat/completions and /openai/deployments/{model}/chat/completions return the provider model name without streaming**

### Description

The proxy normally answers with the model name the client asked for. A non-streaming `POST /chat/completions` with `"model": "team-model"` returns `"model": "team-model"`, and so does a streaming request on the Azure style routes. But a non-streaming request on `/engines/team-model/chat/completions` or `/openai/deployments/team-model/chat/completions`, with the model only in the path and not in the body, returns the deployment's underlying model (`"model": "gpt-4o"`)

I'd expect `"model": "team-model"` in the non-streaming response too, the same as streaming on these routes and the same as `/chat/completions`. Clients that read `model` from the response see a name they never asked for, and it exposes which provider model sits behind the alias. #38761 was the same kind of mismatch on `/v1/messages`

### Config

```yaml
model_list:
  - model_name: team-model
    litellm_params:
      model: openai/gpt-4o
      api_base: http://127.0.0.1:18080/v1
      api_key: sk-fake
general_settings:
  master_key: sk-1234567890abcdef
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

A local OpenAI compatible mock (`mock_openai.py`, below) always answers with `"model": "gpt-4o"`, so this runs offline

```python
# Minimal OpenAI-compatible mock: answers every POST /chat/completions with a fixed body.
import http.server, json, sys
BODY = {"id": "chatcmpl-1", "object": "chat.completion", "created": 1700000000, "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
CHUNKS = [{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
           "choices": [{"index": 0, "delta": {"role": "assistant", "content": "hi"}, "finish_reason": None}]},
          {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
           "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}]
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if req.get("stream"):
            out = "".join(f"data: {json.dumps(c)}\n\n" for c in CHUNKS) + "data: [DONE]\n\n"
            ctype = "text/event-stream"
        else:
            out, ctype = json.dumps(BODY), "application/json"
        self.send_response(200); self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out.encode())
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
```

```bash
python mock_openai.py 18080 &
litellm --config config.yaml --port 14000 &
H=(-H 'Authorization: Bearer sk-1234567890abcdef' -H 'Content-Type: application/json')
for R in /engines/team-model/chat/completions /openai/deployments/team-model/chat/completions; do
  echo "--- $R non-stream, no model in body"; curl -s localhost:14000$R "${H[@]}" -d '{"messages":[{"role":"user","content":"hi"}]}' | grep -o '"model":"[^"]*"'
  echo "--- $R stream, no model in body"; curl -s localhost:14000$R "${H[@]}" -d '{"messages":[{"role":"user","content":"hi"}],"stream":true}' | grep -o '"model":"[^"]*"' | sort | uniq -c
done
echo "--- /chat/completions non-stream, model team-model in body"; curl -s localhost:14000/chat/completions "${H[@]}" -d '{"model":"team-model","messages":[{"role":"user","content":"hi"}]}' | grep -o '"model":"[^"]*"'
```

Output on v1.106.6 and on main at 0aa7079f

```
--- /engines/team-model/chat/completions non-stream, no model in body
"model":"gpt-4o"
--- /engines/team-model/chat/completions stream, no model in body
   2 "model":"team-model"
--- /openai/deployments/team-model/chat/completions non-stream, no model in body
"model":"gpt-4o"
--- /openai/deployments/team-model/chat/completions stream, no model in body
   2 "model":"team-model"
--- /chat/completions non-stream, model team-model in body
"model":"team-model"
```

### Which part of LiteLLM is this about?

Proxy core: startup, config, health checks, endpoints

### How are you deploying?

pip / Python SDK
