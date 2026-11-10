# Posted: https://github.com/PrefectHQ/fastmcp/issues/6299

Filed by alpaylan, 2026-11-03. Text as posted:

**`logging/setLevel` is ignored for `ctx.log()` calls from tools of a mounted server**

### What happened?

A server `parent` mounts a server `child`. A tool on `child` logs at `info` and at `error`. A handshake-era client connects to `parent`, sends `logging/setLevel` with `warning`, and calls the tool. The client still receives the `info` message. Connecting to `child` directly with the same steps delivers only the `error` message.

I expected messages below the client's level to be suppressed for mounted tools too. docs/servers/server.mdx (`client_log_level`) says handshake-era clients "can override this per-session using the MCP `logging/setLevel` request". The same happens over stateful streamable HTTP (`fastmcp.utilities.tests.asgi_client(server, mode="legacy", log_handler=on_log)` in place of `Client(...)` prints the same two lines). This is not the shared-level problem in #5898: there is one client per connection here, and the mounted case runs first.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio

from fastmcp import Client, Context, FastMCP

child = FastMCP("child")


@child.tool
async def say(ctx: Context) -> str:
    await ctx.log("an info message", level="info")
    await ctx.log("an error message", level="error")
    return "done"


parent = FastMCP("parent")
parent.mount(child)


async def levels_received(server):
    levels = []

    async def on_log(msg):
        levels.append(msg.level)

    async with Client(server, mode="legacy", log_handler=on_log) as client:
        await client.set_logging_level("warning")
        await client.call_tool("say")
    return levels


async def main():
    print("mounted:", await levels_received(parent))
    print("direct: ", await levels_received(child))


asyncio.run(main())
```

Output:

```
mounted: ['info', 'error']
direct:  ['error']
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64-arm-64bit
```
