# Posted: https://github.com/BerriAI/litellm/issues/49936

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: OpenAI HTTP 403 raises APIError or BadRequestError instead of PermissionDeniedError**

### Description

When an OpenAI (or OpenAI-compatible) chat call gets an HTTP 403, litellm raises a generic `litellm.APIError` with `status_code` 403, where the openai SDK raises `openai.PermissionDeniedError` for the same reply. The body I used is OpenAI's unsupported-region reply (`"type": "request_forbidden"`, `"code": "unsupported_country_region_territory"`). On v1.106.6 a 403 whose body has `"type": "invalid_request_error"` raises `litellm.BadRequestError` instead, on main it is `APIError` too

I'd expect `litellm.PermissionDeniedError`, as in the 403 row of the table at https://docs.litellm.ai/docs/exception_mapping. The Anthropic and Vertex AI mappers already raise it for a 403, but the OpenAI mapper in `_map_openai_exception` has no 403 case. #32537 recently made 401 and 404 replies with an `invalid_request_error` body raise `AuthenticationError` and `NotFoundError`, and 403 was not part of that change. Code that does `except litellm.PermissionDeniedError` around an OpenAI call never catches a 403 today

### Config

```yaml
# SDK only, no config.yaml
litellm.completion(model="openai/gpt-4o", messages=[...], api_base="http://127.0.0.1:<port>/v1", api_key="k", max_retries=0)
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

A local server answers every POST with HTTP 403 and one of two OpenAI-style bodies, and the script sends the same request with the openai SDK and with litellm. It runs offline

```python
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import openai, litellm
litellm.suppress_debug_info = True

BODIES = [
    {"error": {"message": "Country, region, or territory not supported", "type": "request_forbidden",
               "param": None, "code": "unsupported_country_region_territory"}},
    {"error": {"message": "Forbidden", "type": "invalid_request_error", "param": None, "code": None}},
]
body = b""

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        self.send_response(403)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args): pass

server = HTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
base = f"http://127.0.0.1:{server.server_port}/v1"
messages = [{"role": "user", "content": "hi"}]

for b in BODIES:
    body = json.dumps(b).encode()
    print("error.type =", b["error"]["type"])
    try:
        openai.OpenAI(base_url=base, api_key="k", max_retries=0).chat.completions.create(model="gpt-4o", messages=messages)
    except openai.APIStatusError as e:
        print("  openai SDK:", type(e).__name__, e.status_code)
    try:
        litellm.completion(model="openai/gpt-4o", messages=messages, api_base=base, api_key="k", max_retries=0)
    except Exception as e:
        print("  litellm:   ", type(e).__name__, getattr(e, "status_code", None),
              "| isinstance PermissionDeniedError:", isinstance(e, litellm.PermissionDeniedError))
```

Output on v1.106.6

```
error.type = request_forbidden
  openai SDK: PermissionDeniedError 403
  litellm:    APIError 403 | isinstance PermissionDeniedError: False
error.type = invalid_request_error
  openai SDK: PermissionDeniedError 403
  litellm:    BadRequestError 403 | isinstance PermissionDeniedError: False
```

Output on main 0aa7079f

```
error.type = request_forbidden
  openai SDK: PermissionDeniedError 403
  litellm:    APIError 403 | isinstance PermissionDeniedError: False
error.type = invalid_request_error
  openai SDK: PermissionDeniedError 403
  litellm:    APIError 403 | isinstance PermissionDeniedError: False
```

### Which part of LiteLLM is this about?

Python SDK: the litellm package itself

### How are you deploying?

pip / Python SDK
