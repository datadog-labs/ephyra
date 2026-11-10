import asyncio
from mcp.types import Tool as MT
from fastmcp import Client, FastMCP
from fastmcp.server import create_proxy
from fastmcp.tools import Tool
class AnyArgs(Tool):
    async def run(self, arguments): return "ok"
backend = FastMCP(dereference_schemas=False)
backend.add_tool(AnyArgs(name="anything", parameters={"type": "object", "$ref": "#/$defs/Any", "$defs": {"Any": True}}))
proxy = create_proxy(backend)
async def demo():
    async with Client(proxy) as c:
        print([t.name for t in await c.list_tools()])
asyncio.run(demo())
