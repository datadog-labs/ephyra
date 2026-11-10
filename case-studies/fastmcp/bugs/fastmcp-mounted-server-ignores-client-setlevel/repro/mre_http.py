import asyncio

from fastmcp import Context, FastMCP
from fastmcp.utilities.tests import asgi_client

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

    # Streamable HTTP, stateful, handshake-era client.
    async with asgi_client(server, mode="legacy", log_handler=on_log) as client:
        await client.set_logging_level("warning")
        await client.call_tool("say")
    return levels


async def main():
    print("mounted:", await levels_received(parent))
    print("direct: ", await levels_received(child))


asyncio.run(main())
