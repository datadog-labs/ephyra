# Variant (verification 2026-10-28): the stream sends the functionCall and finishReason MAX_TOKENS
# in ONE chunk. Stream then reports "tool_calls", same as non-stream.
import json, threading, litellm
from http.server import BaseHTTPRequestHandler, HTTPServer

CALL = {"functionCall": {"name": "get_weather", "args": {}}}
USAGE = {"promptTokenCount": 5, "candidatesTokenCount": 5, "totalTokenCount": 10}
BODY = {"candidates": [{"content": {"role": "model", "parts": [CALL, {"text": ""}]},
                        "finishReason": "MAX_TOKENS", "index": 0}], "usageMetadata": USAGE}
SSE = [{"candidates": [{"content": {"role": "model", "parts": [CALL]}, "finishReason": "MAX_TOKENS", "index": 0}], "usageMetadata": USAGE}]

class Mock(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        if "streamGenerateContent" in self.path:
            data = "".join(f"data: {json.dumps(o)}\r\n\r\n" for o in SSE).encode()
            ctype = "text/event-stream"
        else:
            data, ctype = json.dumps(BODY).encode(), "application/json"
        self.send_response(200)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()
kw = dict(model="gemini/gemini-2.5-flash", messages=[{"role": "user", "content": "weather?"}],
          api_base=f"http://127.0.0.1:{srv.server_port}", api_key="x")

r = litellm.completion(**kw)
print("non-stream finish_reason:", r.choices[0].finish_reason)
reasons = [c.choices[0].finish_reason for c in litellm.completion(stream=True, **kw)
           if c.choices and c.choices[0].finish_reason]
print("stream finish_reason:    ", reasons[-1])
