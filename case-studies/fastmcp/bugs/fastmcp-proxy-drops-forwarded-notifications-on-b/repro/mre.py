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
