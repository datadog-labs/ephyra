"""litellm.anthropic.messages.acreate(stream=True) on Bedrock: a mid-stream
Bedrock exception event escapes as a raw BedrockError, not a mapped litellm
exception. Runs offline against a local mock of bedrock-runtime."""
import asyncio, json, struct, threading, zlib
from http.server import BaseHTTPRequestHandler, HTTPServer

import litellm, openai


def exception_frame(exc_type, payload):
    # one AWS event-stream message with :message-type exception
    hs = b""
    for k, v in ((":exception-type", exc_type), (":content-type", "application/json"), (":message-type", "exception")):
        hs += bytes([len(k)]) + k.encode() + struct.pack("!BH", 7, len(v)) + v.encode()
    prelude = struct.pack("!II", 16 + len(hs) + len(payload), len(hs))
    msg = prelude + struct.pack("!I", zlib.crc32(prelude)) + hs + payload
    return msg + struct.pack("!I", zlib.crc32(msg))


BODY = exception_frame("internalServerException", b'{"message":"boom"}')


class Mock(BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["content-length"]))
        self.send_response(200)
        self.send_header("content-type", "application/vnd.amazon.eventstream")
        self.send_header("content-length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)

    def log_message(self, *a):
        pass


srv = HTTPServer(("127.0.0.1", 0), Mock)
threading.Thread(target=srv.serve_forever, daemon=True).start()


async def main():
    stream = await litellm.acompletion(
        model="bedrock/invoke/us.anthropic.claude-sonnet-4-5-20250929-v1:0",
        messages=[{"role": "user", "content": "hi"}],
        max_tokens=16,
        stream=True,
        api_base=f"http://127.0.0.1:{srv.server_port}",
        aws_access_key_id="x", aws_secret_access_key="x", aws_region_name="us-east-1",
    )
    try:
        async for _ in stream:
            pass
    except Exception as e:
        print(type(e).__module__ + "." + type(e).__name__, getattr(e, "status_code", None), str(e)[:300])
        return isinstance(e, openai.OpenAIError)
    print("no exception")
    return False


raise SystemExit(0 if asyncio.run(main()) else 1)
