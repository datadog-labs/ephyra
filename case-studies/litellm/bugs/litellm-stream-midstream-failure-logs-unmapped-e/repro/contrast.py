# Same overloaded error, sent as HTTP 529 before any stream starts.
# Run with no argument for a non-stream call, or with "stream" for stream=True.
# In both cases the logged error_class matches the raised exception.
import sys, threading, time
from http.server import BaseHTTPRequestHandler, HTTPServer
import litellm
from litellm.integrations.custom_logger import CustomLogger

BODY = b'{"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}'

class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        self.send_response(529)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)
    def log_message(self, *a): pass

srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()

logged = []
class Log(CustomLogger):
    def log_failure_event(self, kwargs, response_obj, start_time, end_time):
        logged.append(kwargs["standard_logging_object"]["error_information"])
litellm.callbacks = [Log()]

try:
    litellm.completion(
        model="anthropic/claude-sonnet-4-5", api_key="x",
        api_base=f"http://127.0.0.1:{srv.server_address[1]}",
        messages=[{"role": "user", "content": "hi"}], num_retries=0, stream="stream" in sys.argv,
    )
except Exception as e:
    print("raised:", type(e).__name__, e.status_code)

for _ in range(50):
    if logged: break
    time.sleep(0.1)
print("logged:", logged[0]["error_class"], logged[0]["error_code"])
