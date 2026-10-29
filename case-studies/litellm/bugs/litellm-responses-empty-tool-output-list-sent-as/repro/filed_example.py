# litellm.responses() on an Anthropic model. The tool output is [{"type": "input_text", "text": ""}]:
# one text part, empty. Anthropic should get an empty tool_result, the same as for output "".
# It gets the JSON text of the part list instead.
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

litellm.suppress_debug_info = True
SENT = []

BODY = {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn", "stop_sequence": None,
        "usage": {"input_tokens": 1, "output_tokens": 1}}

class MockAnthropic(BaseHTTPRequestHandler):
    def do_POST(self):
        SENT.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(BODY).encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockAnthropic)
threading.Thread(target=srv.serve_forever, daemon=True).start()

def tool_result_sent(output):
    SENT.clear()
    litellm.responses(
        model="anthropic/claude-sonnet-4-5", api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x",
        input=[
            {"role": "user", "content": "hi"},
            {"type": "function_call", "call_id": "call_1", "name": "get_weather", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "call_1", "output": output},
        ],
    )
    return SENT[0]["messages"][-1]["content"][0]

try:
    a = tool_result_sent("")
    b = tool_result_sent([{"type": "input_text", "text": ""}])
except Exception as e:
    print("setup error:", repr(e)[:300])
    raise SystemExit(2)
print('output "":                                ', json.dumps(a))
print('output [{"type":"input_text","text":""}]: ', json.dumps(b))
