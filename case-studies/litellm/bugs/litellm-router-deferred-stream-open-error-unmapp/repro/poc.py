import asyncio
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

import litellm


class Gemini400(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        body = b'{"error": {"code": 400, "message": "bad request", "status": "INVALID_ARGUMENT"}}'
        self.send_response(400)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


server = HTTPServer(("127.0.0.1", 0), Gemini400)
threading.Thread(target=server.serve_forever, daemon=True).start()
params = {
    "model": "gemini/gemini-2.5-flash",
    "api_base": f"http://127.0.0.1:{server.server_port}",
    "api_key": "fake-key",
}
messages = [{"role": "user", "content": "hi"}]


async def consume(stream):
    async for _ in stream:
        pass


async def main():
    # 1. litellm.acompletion: the error comes out mapped
    try:
        await consume(await litellm.acompletion(**params, messages=messages, stream=True))
    except Exception as e:
        print("acompletion:", type(e).__module__, type(e).__name__)

    # 2. Router.acompletion, same deployment: the error comes out raw
    router = litellm.Router(model_list=[{"model_name": "m", "litellm_params": params}], num_retries=0)
    try:
        await consume(await router.acompletion(model="m", messages=messages, stream=True))
    except Exception as e:
        print("router:", type(e).__module__, type(e).__name__)
        print("isinstance(e, litellm.BadRequestError):", isinstance(e, litellm.BadRequestError))


asyncio.run(main())
