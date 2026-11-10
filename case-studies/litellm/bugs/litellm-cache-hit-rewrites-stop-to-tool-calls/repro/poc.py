# The same non-streaming litellm.completion() call twice on an anthropic/ model, with the
# in-memory cache on. Anthropic answers with one tool_use block and stop_reason "end_turn".
# The first call returns finish_reason "stop". The second call is a cache hit and should
# return the same response; instead it returns finish_reason "tool_calls".
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
try:
    import litellm
    from litellm.caching.caching import Cache
except ImportError as e:
    print("setup error:", e)
    raise SystemExit(2)

litellm.suppress_debug_info = True
litellm.cache = Cache(type="local")

BODY = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {}}],
        "stop_reason": "end_turn", "stop_sequence": None,
        "usage": {"input_tokens": 1, "output_tokens": 1}}
HITS = []

class MockAnthropic(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        HITS.append(1)
        data = json.dumps(BODY).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockAnthropic)
threading.Thread(target=srv.serve_forever, daemon=True).start()

def finish_reason():
    r = litellm.completion(model="anthropic/claude-sonnet-4-5", max_tokens=100,
                           messages=[{"role": "user", "content": "weather?"}],
                           api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x")
    return r.choices[0].finish_reason, bool(r.choices[0].message.tool_calls)

try:
    first, second = finish_reason(), finish_reason()
except Exception as e:
    print("setup error:", repr(e)[:300])
    raise SystemExit(2)
print("call 1 (finish_reason, has tool_calls):", first)
print("call 2 (finish_reason, has tool_calls):", second)
print("requests that reached the mock:", len(HITS))
if len(HITS) != 1:
    print("setup error: the second call was not a cache hit")
    raise SystemExit(2)
raise SystemExit(0 if first == second else 1)
