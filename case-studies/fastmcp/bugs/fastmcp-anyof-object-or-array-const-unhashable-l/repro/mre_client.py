import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool

mcp = FastMCP()

schema = {
    "type": "object",
    "properties": {"x": {"anyOf": [{"const": {}}, {"const": 1}]}},
}


def get_x() -> dict:
    return {"x": 1}


mcp.add_tool(Tool.from_function(get_x, output_schema=schema))


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("get_x", {})
        print("structured_content:", result.structured_content)
        print("data:", result.data)


asyncio.run(main())
