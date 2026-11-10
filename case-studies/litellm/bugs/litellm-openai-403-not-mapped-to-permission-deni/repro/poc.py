import json, sys, threading
from http.server import BaseHTTPRequestHandler, HTTPServer

BODY = json.dumps({"error": {"message": "Country, region, or territory not supported", "type": "request_forbidden",
                             "param": None, "code": "unsupported_country_region_territory"}}).encode()

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        self.send_response(403)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)
    def log_message(self, *args):
        pass

server = HTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
base = f"http://127.0.0.1:{server.server_port}/v1"
messages = [{"role": "user", "content": "hi"}]

import openai
try:
    openai.OpenAI(base_url=base, api_key="k", max_retries=0).chat.completions.create(model="gpt-4o", messages=messages)
except openai.APIStatusError as e:
    print("openai SDK:", type(e).__name__, e.status_code)

import litellm
try:
    litellm.completion(model="openai/gpt-4o", messages=messages, api_base=base, api_key="k", max_retries=0)
except Exception as e:
    print("litellm:   ", type(e).__name__, getattr(e, "status_code", None))
    print("message:   ", str(e)[:200])
    sys.exit(0 if isinstance(e, litellm.PermissionDeniedError) else 1)
