"""Same arguments through litellm.completion(): temperature is dropped."""
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

sent = {}
RESPONSE = {"id": "c1", "object": "chat.completion", "created": 0, "model": "o3-mini",
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        sent.update(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
        body = json.dumps(RESPONSE).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()

litellm.completion(model="openai/o3-mini", messages=[{"role": "user", "content": "hi"}], temperature=0.5,
                   drop_params=True, api_base=f"http://127.0.0.1:{srv.server_port}/v1", api_key="sk-test")
print("request body:", json.dumps(sent))
print("temperature sent:", "temperature" in sent)
