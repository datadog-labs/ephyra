"""litellm.responses() forwards temperature to an OpenAI o-series model even
with drop_params=True. A local mock stands in for api.openai.com and prints the
request body litellm sent. Exit 1 if temperature was sent, 0 if dropped."""
import json, sys, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

sent = {}
RESPONSE = {"id": "resp_1", "object": "response", "created_at": 0, "status": "completed",
            "model": "o3-mini", "output": [{"type": "message", "id": "msg_1", "role": "assistant",
            "status": "completed", "content": [{"type": "output_text", "text": "hi", "annotations": []}]}],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        sent.update(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
        body = json.dumps(RESPONSE).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()

litellm.responses(model="openai/o3-mini", input="hi", temperature=0.5, drop_params=True,
                  api_base=f"http://127.0.0.1:{srv.server_port}/v1", api_key="sk-test")
print("request body:", json.dumps(sent))
print("temperature sent:", "temperature" in sent)
sys.exit(1 if "temperature" in sent else 0)
