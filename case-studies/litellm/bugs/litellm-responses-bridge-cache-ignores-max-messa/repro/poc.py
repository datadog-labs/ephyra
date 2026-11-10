# Cache(max_messages=4): "Requests with more `messages` (or Responses API `input` items) than this are
# neither looked up nor stored". A 5-item responses() call bridged to anthropic/ is still cached.
# Usage: python poc.py   (offline: a local mock stands in for the Anthropic API)
import json, sys, threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import litellm
from litellm.caching.caching import Cache

ANTHROPIC_REPLY = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
                   "content": [{"type": "text", "text": "hello"}], "stop_reason": "end_turn",
                   "stop_sequence": None, "usage": {"input_tokens": 1, "output_tokens": 1}}
provider_requests = 0


class MockAnthropic(BaseHTTPRequestHandler):
    def do_POST(self):
        global provider_requests
        provider_requests += 1
        self.rfile.read(int(self.headers["content-length"]))
        body = json.dumps(ANTHROPIC_REPLY).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


server = HTTPServer(("127.0.0.1", 0), MockAnthropic)
threading.Thread(target=server.serve_forever, daemon=True).start()

litellm.cache = Cache(type="local")  # max_messages defaults to 4
five_items = [
    {"role": "user", "content": "hi"},
    {"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{}"},
    {"type": "function_call", "call_id": "call_2", "name": "get_weather", "arguments": "{}"},
    {"type": "function_call_output", "call_id": "call_1", "output": "ok"},
    {"type": "function_call_output", "call_id": "call_2", "output": "ok"},
]
try:
    for _ in range(2):
        r = litellm.responses(model="anthropic/claude-sonnet-4-5", input=five_items, api_key="x",
                              api_base=f"http://127.0.0.1:{server.server_address[1]}")
except Exception as e:
    print("cannot decide:", type(e).__name__, e)
    sys.exit(2)

print("provider requests:", provider_requests)
print("second call cache_hit:", r._hidden_params.get("cache_hit"))
# expected: provider requests: 2, cache_hit: None
sys.exit(1 if provider_requests < 2 else 0)
