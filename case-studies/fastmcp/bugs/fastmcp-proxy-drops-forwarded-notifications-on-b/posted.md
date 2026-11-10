# Posted: https://github.com/PrefectHQ/fastmcp/issues/6303

Filed by alpaylan, 2026-11-03. Text as posted:

**A proxy served over stdio forwards only some of the backend's `ctx.log()` messages**

### What happened?

A backend tool sends five `ctx.log()` messages and returns. Served directly over stdio, the client receives all five. Wrapped with `create_proxy(backend)` and served over stdio, the client receives only one of the five, on every run; the tool result itself arrives. With two concurrent calls through the proxy the client gets 1 or 2 of 10. The same proxy with the in-memory transport delivers all five.

I expected the proxy to forward every log notification the backend sends before its result, as it does in-memory.

Possibly a separate cause: with a subprocess backend (`create_proxy(StdioTransport(...))`) the client receives 0 of 5, even with an in-memory client in front of the proxy and with a 0.3 s sleep before the tool returns. In that setup the proxy's forwarding `ctx.log()` runs with request id 1, the client's earlier `server/discover` request, rather than the `tools/call` request (id 2). With an in-process backend it runs with id 2.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio
import sys

from fastmcp import Client, Context, FastMCP
from fastmcp.client.transports import StdioTransport
from fastmcp.server import create_proxy

backend = FastMCP("backend")


@backend.tool
async def chatty(ctx: Context) -> str:
    for i in range(5):
        await ctx.log(f"message {i}", level="info")
    return "done"


proxy = create_proxy(backend)


async def count_logs(args):
    received = []

    async def on_log(msg):
        received.append(msg.data)

    transport = StdioTransport(sys.executable, [__file__, *args])
    async with Client(transport, log_handler=on_log) as client:
        await client.call_tool("chatty")
        await asyncio.sleep(0.5)
    return len(received)


async def main():
    print("backend over stdio:", await count_logs(["backend"]), "of 5 log messages")
    print("proxy over stdio:  ", await count_logs(["proxy"]), "of 5 log messages")


if __name__ == "__main__":
    if sys.argv[1:] == ["backend"]:
        backend.run(show_banner=False)
    elif sys.argv[1:] == ["proxy"]:
        proxy.run(show_banner=False)
    else:
        asyncio.run(main())
```

Output:

```
backend over stdio: 5 of 5 log messages
proxy over stdio:   1 of 5 log messages
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64-arm-64bit
```
