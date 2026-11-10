# litellm.anthropic.messages.acreate(stream=True) twice, same request, in-memory cache on.
# The model is a Gemini model; the mock Gemini server answers with an empty reply (one empty text part).
# litellm does not cache a reply with no output, so the second call should reach the server again.
import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.caching.caching import Cache

litellm.suppress_debug_info = True
litellm.cache = Cache(type="local")

CHUNK = {"candidates": [{"content": {"role": "model", "parts": [{"text": ""}]},
                         "finishReason": "STOP", "index": 0}],
         "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 0, "totalTokenCount": 1},
         "modelVersion": "gemini-2.5-flash", "responseId": "resp-1"}
hits = []

class MockGemini(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        hits.append(self.path)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        self.wfile.write(b"data: " + json.dumps(CHUNK).encode() + b"\r\n\r\n")
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), MockGemini)
threading.Thread(target=srv.serve_forever, daemon=True).start()

async def call():
    s = await litellm.acompletion(
        model="gemini/gemini-2.5-flash", messages=[{"role": "user", "content": "hi"}], max_tokens=16,
        stream=True, api_base="http://127.0.0.1:%d" % srv.server_port, api_key="x")
    return str([c async for c in s])

async def main():
    first = await call()
    await asyncio.sleep(0.5)  # let the stream's cache write finish
    second = await call()
    print("call 1 events:", [l[7:] for l in first.splitlines() if l.startswith("event: ")])
    print("call 2 events:", [l[7:] for l in second.splitlines() if l.startswith("event: ")])
    print("requests to the server:", len(hits))
    return 0 if len(hits) == 2 else 1

raise SystemExit(asyncio.run(main()))
