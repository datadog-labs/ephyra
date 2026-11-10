# Posted: https://github.com/BerriAI/litellm/issues/49935

Filed by alpaylan, 2026-11-03. Text as posted:

**[Bug]: anthropic.messages.acreate(stream=True) on Bedrock raises the raw BedrockError for an exception event in the stream**

### Description

I call `litellm.anthropic.messages.acreate(..., stream=True)` with a `bedrock/` Claude model, and the Bedrock event stream carries an exception event (here `internalServerException`). Iterating the returned stream raises `litellm.llms.bedrock.common_utils.BedrockError`. That is litellm's internal provider class, not one of the mapped exceptions, so `except litellm.ServiceUnavailableError` or `except openai.APIError` around the stream does not catch it

I'd expect a mapped litellm exception, as for the same event on `litellm.acompletion(model="bedrock/invoke/...", stream=True)`, which raises `litellm.MidStreamFallbackError` wrapping `litellm.ServiceUnavailableError`. Errors that `anthropic.messages.acreate` raises before it returns (an HTTP error status, for example) are already mapped, only errors raised while the returned stream is iterated come out raw. #24609 described the same thing for the proxy's `/v1/messages` path in March and was closed as stale

### Config

```yaml
# SDK only, no config.yaml
await litellm.anthropic.messages.acreate(model="bedrock/us.anthropic.claude-sonnet-4-5-20250929-v1:0", messages=[...], max_tokens=16, stream=True, api_base="http://127.0.0.1:<port>", aws_access_key_id="x", aws_secret_access_key="x", aws_region_name="us-east-1")
```

### LiteLLM Version

v1.106.6, also main at 0aa7079f

### Steps to Repro

A local server stands in for bedrock-runtime. It answers every POST with HTTP 200 and one AWS event-stream frame whose `:message-type` is `exception` and `:exception-type` is `internalServerException`, so this runs offline

```python
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
    stream = await litellm.anthropic.messages.acreate(
        model="bedrock/us.anthropic.claude-sonnet-4-5-20250929-v1:0",
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
        return isinstance(e, openai.OpenAIError)  # litellm exceptions subclass openai errors
    print("no exception")
    return False


raise SystemExit(0 if asyncio.run(main()) else 1)
```

Output on v1.106.6 and on main 0aa7079f

```
litellm.llms.bedrock.common_utils.BedrockError 500 internalServerException {"message":"boom"}
```

The same mock with `litellm.acompletion(model="bedrock/invoke/us.anthropic.claude-sonnet-4-5-20250929-v1:0", ..., stream=True)` raises the mapped exception

```
litellm.exceptions.MidStreamFallbackError 503 litellm.MidStreamFallbackError: litellm.ServiceUnavailableError: BedrockException - internalServerException {"message":"boom"} ...
```

### Which part of LiteLLM is this about?

LLM translation: a specific provider's request or response

### How are you deploying?

pip / Python SDK
