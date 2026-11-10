# Async variant of poc.py: acompletion(stream=True) and async_log_failure_event.
# The mock stream opens (HTTP 200) and then sends an Anthropic error event.
import json, threading, time
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.integrations.custom_logger import CustomLogger

SSE = (
    'event: error\n'
    'data: {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}\n\n'
)

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        self.wfile.write(SSE.encode())
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()

logged = []
class Log(CustomLogger):
    async def async_log_failure_event(self, kwargs, response_obj, start_time, end_time):
        logged.append(kwargs["standard_logging_object"]["error_information"])
litellm.callbacks = [Log()]

import asyncio
async def go():
  global raised
  try:
    async for _ in await litellm.acompletion(
        model="anthropic/claude-sonnet-4-5", api_key="x",
        api_base=f"http://127.0.0.1:{srv.server_address[1]}",
        messages=[{"role": "user", "content": "hi"}], stream=True,
    ):
        pass
  except Exception as e:
    raised = getattr(e, "original_exception", None) or e
    print("raised:", type(e).__name__, "->", type(raised).__name__, raised.status_code)

  for _ in range(50):
    if logged: break
    await asyncio.sleep(0.1)
asyncio.run(go())
info = logged[0]
print("logged:", info["error_class"], info["error_code"])
raise SystemExit(1 if info["error_class"] != type(raised).__name__ else 0)
