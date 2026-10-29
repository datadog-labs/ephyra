# Variant (verification 2026-10-28): the tool call delta and finish_reason "stop" arrive in ONE chunk.
# The stream still reports "stop"; non-stream reports "tool_calls".
import json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm

litellm.suppress_debug_info = True

TOOL_CALL = {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}

def chunk(delta, finish_reason=None):
    return {"id": "c1", "object": "chat.completion.chunk", "created": 1, "model": "m",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]}

BODY = {"id": "c1", "object": "chat.completion", "created": 1, "model": "m",
        "choices": [{"index": 0, "finish_reason": "stop",
                     "message": {"role": "assistant", "content": None, "tool_calls": [TOOL_CALL]}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
SSE = "".join("data: %s\n\n" % json.dumps(c) for c in [
    chunk({"role": "assistant", "content": None, "tool_calls": [dict(TOOL_CALL, index=0)]}, "stop"),
]) + "data: [DONE]\n\n"

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        out = SSE if req.get("stream") else json.dumps(BODY)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream" if req.get("stream") else "application/json")
        self.end_headers()
        self.wfile.write(out.encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
args = dict(model="openai/m", api_base="http://127.0.0.1:%d/v1" % srv.server_port, api_key="x",
            messages=[{"role": "user", "content": "weather?"}])

plain = litellm.completion(**args).choices[0].finish_reason
chunks = list(litellm.completion(**args, stream=True))
last = [c.choices[0].finish_reason for c in chunks if c.choices][-1]
built = litellm.stream_chunk_builder(chunks).choices[0].finish_reason
print("stream=False:", plain)
print("stream=True last chunk:", last)
print("stream_chunk_builder:", built)
raise SystemExit(1 if (plain, last, built) != ("tool_calls",) * 3 else 0)
