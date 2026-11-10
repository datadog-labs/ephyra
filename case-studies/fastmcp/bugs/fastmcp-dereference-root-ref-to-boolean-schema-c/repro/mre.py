import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool


class AnyArgs(Tool):
    async def run(self, arguments):
        return "ok"


mcp = FastMCP()
mcp.add_tool(
    AnyArgs(
        name="anything",
        parameters={"type": "object", "$ref": "#/$defs/Any", "$defs": {"Any": True}},
    )
)


@mcp.tool
def ping() -> str:
    return "pong"


async def demo():
    async with Client(mcp) as client:
        tools = await client.list_tools()
        print([t.name for t in tools])


if __name__ == "__main__":
    asyncio.run(demo())
