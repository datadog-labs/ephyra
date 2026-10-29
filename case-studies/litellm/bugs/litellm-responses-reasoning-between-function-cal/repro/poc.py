# Turn 1: litellm.responses(stream=True) against a mock Gemini server whose reply is
# [functionCall, thought, functionCall]. Turn 2: send the output back with both tool
# results, as in OpenAI's function-calling guide. Turn 2 fails before any HTTP request.
import json, threading, litellm
from http.server import BaseHTTPRequestHandler, HTTPServer

# One SSE event per part, in this order.
PARTS = [{"functionCall": {"name": "get_weather", "args": {"city": "Paris"}}},
         {"text": "Now Rome too.", "thought": True},
         {"functionCall": {"name": "get_weather", "args": {"city": "Rome"}}}]
CHUNKS = [{"candidates": [{"content": {"role": "model", "parts": [p]}}]} for p in PARTS]
CHUNKS[-1]["candidates"][0]["finishReason"] = "STOP"
CHUNKS[-1]["usageMetadata"] = {"promptTokenCount": 5, "candidatesTokenCount": 5, "totalTokenCount": 10}
FINAL = {"candidates": [{"content": {"role": "model", "parts": [{"text": "Both sunny."}]},
                         "finishReason": "STOP"}],
         "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 5, "totalTokenCount": 10}}

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        self.send_response(200)
        if "alt=sse" in self.path:  # turn 1 (streaming)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for c in CHUNKS:
                self.wfile.write(b"data: " + json.dumps(c).encode() + b"\n\n")
        else:  # turn 2 (never reached on affected versions)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps(FINAL).encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
args = dict(model="gemini/gemini-2.5-flash", api_key="x",
            api_base=f"http://127.0.0.1:{srv.server_port}",
            tools=[{"type": "function", "name": "get_weather",
                    "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}])

history = [{"role": "user", "content": "Weather in Paris and Rome?"}]
out = None
for ev in litellm.responses(input=history, stream=True, **args):
    if ev.type == "response.completed":
        out = ev.response.output
items = [i.model_dump() if hasattr(i, "model_dump") else dict(i) for i in out]
print("turn 1 output:", [i["type"] for i in items])
history += items
history += [{"type": "function_call_output", "call_id": i["call_id"], "output": "sunny"}
            for i in items if i["type"] == "function_call"]
try:
    r = litellm.responses(input=history, stream=False, **args)
    print("turn 2 OK:", r.output[-1].content[0].text)
    raise SystemExit(20)
except Exception as e:
    print("turn 2 FAILED:", str(e).splitlines()[0][:200])
    raise SystemExit(10 if "Missing corresponding tool call" in str(e) else 20)
