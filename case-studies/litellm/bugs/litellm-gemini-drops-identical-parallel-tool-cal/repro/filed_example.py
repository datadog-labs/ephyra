import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

litellm.suppress_debug_info = True

REPLY = {"candidates": [{"content": {"role": "model", "parts": [{"text": "done"}]}, "index": 0, "finishReason": "STOP"}],
         "modelVersion": "gemini-2.5-flash", "responseId": "r1",
         "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1, "totalTokenCount": 2}}
sent = []

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        sent.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(REPLY).encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()

call = lambda i: {"id": i, "type": "function", "function": {"name": "get_time", "arguments": "{}"}}
litellm.completion(
    model="gemini/gemini-2.5-flash", api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x",
    messages=[
        {"role": "user", "content": "What time is it? Ask twice."},
        {"role": "assistant", "content": None, "tool_calls": [call("call_1"), call("call_2")]},
        {"role": "tool", "tool_call_id": "call_1", "content": "12:00"},
        {"role": "tool", "tool_call_id": "call_2", "content": "12:01"},
    ],
)
parts = [p for c in sent[0]["contents"] for p in c["parts"]]
calls = [p for p in parts if "functionCall" in p or "function_call" in p]
responses = [p for p in parts if "functionResponse" in p or "function_response" in p]
print("contents sent to Gemini:", json.dumps(sent[0]["contents"]))
print("functionCall parts: %d, functionResponse parts: %d" % (len(calls), len(responses)))
