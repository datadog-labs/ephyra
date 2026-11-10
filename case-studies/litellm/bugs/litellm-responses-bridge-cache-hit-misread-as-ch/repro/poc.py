# The same litellm.responses() call twice with the in-memory cache on. The model is an
# Anthropic model, which litellm serves through its chat-completion bridge. The first call
# works and is cached. The second call should return the cached response; instead it raises.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.caching.caching import Cache

litellm.suppress_debug_info = True
litellm.cache = Cache(type="local")

BODY = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "hello"}],
        "stop_reason": "end_turn", "stop_sequence": None,
        "usage": {"input_tokens": 1, "output_tokens": 1}}

class MockAnthropic(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(BODY).encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockAnthropic)
threading.Thread(target=srv.serve_forever, daemon=True).start()

def call():
    return litellm.responses(model="anthropic/claude-sonnet-4-5", input="hi",
                             api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x")

try:
    first = call()
except Exception as e:
    print("setup error:", repr(e)[:300])
    raise SystemExit(2)
print("call 1:", first.id, first.object, first.output_text)
try:
    second = call()
except Exception as e:
    print("call 2:", type(e).__name__, str(e)[:200])
    raise SystemExit(1)
print("call 2:", second.id, second.object, second.output_text)
raise SystemExit(0)
