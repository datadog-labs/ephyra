import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import litellm
hits = []
OK = {"id": "resp_1", "object": "response", "created_at": 0, "status": "completed", "model": "gpt-4o",
      "output": [], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}
class H(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"])); hits.append(self.path.split("/")[1])
        st, body = (503, {"error": {"message": "down", "type": "server_error"}}) if self.path.startswith("/primary") else (200, OK)
        d = json.dumps(body).encode(); self.send_response(st); self.send_header("content-type","application/json")
        self.send_header("content-length", str(len(d))); self.end_headers(); self.wfile.write(d)
    def log_message(self, *a): pass
s = ThreadingHTTPServer(("127.0.0.1", 0), H); threading.Thread(target=s.serve_forever, daemon=True).start()
r = litellm.Router(model_list=[{"model_name": g, "litellm_params": {"model": "openai/gpt-4o", "api_key": "sk-test",
    "api_base": f"http://127.0.0.1:{s.server_port}/{g}/v1"}} for g in ("primary", "backup")],
    num_retries=0, fallbacks=[{"primary": ["backup"]}], disable_cooldowns=True)
for name, call in [("aresponses", lambda: asyncio.run(r.aresponses(model="primary", input="hi"))), ("responses", lambda: r.responses(model="primary", input="hi"))]:
    hits.clear()
    try: res = "status=" + call().status
    except Exception as e: res = "raised " + type(e).__name__
    print(name, hits, res)
