# A local OpenAI-style server answers the first POST with 503 and later ones with 200.
# Router(num_retries=1) should retry once. Run the same call through
# router.aresponses() and router.responses() and count the HTTP requests.
import asyncio, json, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import litellm

calls = 0
OK = {"id": "resp_1", "object": "response", "created_at": 0, "status": "completed", "model": "gpt-4o",
      "output": [], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}
ERR = {"error": {"message": "down", "type": "server_error"}}

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        global calls
        self.rfile.read(int(self.headers["content-length"]))
        calls += 1
        status, body = (503, ERR) if calls == 1 else (200, OK)
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *args): pass

server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()

router = litellm.Router(
    model_list=[{"model_name": "my-model", "litellm_params": {
        "model": "openai/gpt-4o", "api_key": "sk-test",
        "api_base": f"http://127.0.0.1:{server.server_port}/v1"}}],
    num_retries=1, retry_after=0, disable_cooldowns=True)

def run(name, call):
    global calls
    calls = 0
    try:
        resp = call()
        result = f"status={resp.status}"
    except Exception as e:
        result = f"raised {type(e).__name__}"
    print(f"{name}: requests={calls} {result}")

run("aresponses", lambda: asyncio.run(router.aresponses(model="my-model", input="hi")))
run("responses", lambda: router.responses(model="my-model", input="hi"))
