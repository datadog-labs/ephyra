# Minimal OpenAI-compatible mock: answers every POST /chat/completions with a fixed body.
import http.server, json, sys
BODY = {"id": "chatcmpl-1", "object": "chat.completion", "created": 1700000000, "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
CHUNKS = [{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
           "choices": [{"index": 0, "delta": {"role": "assistant", "content": "hi"}, "finish_reason": None}]},
          {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
           "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}]
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if req.get("stream"):
            out = "".join(f"data: {json.dumps(c)}\n\n" for c in CHUNKS) + "data: [DONE]\n\n"
            ctype = "text/event-stream"
        else:
            out, ctype = json.dumps(BODY), "application/json"
        self.send_response(200); self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out.encode())
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
