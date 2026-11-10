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
    # Run the mounted case first: stdio/in-memory sessions on one server share
    # one stored level, so calling the child first would hide the problem.
    print("mounted:", await levels_received(parent))
    print("direct: ", await levels_received(child))


asyncio.run(main())
