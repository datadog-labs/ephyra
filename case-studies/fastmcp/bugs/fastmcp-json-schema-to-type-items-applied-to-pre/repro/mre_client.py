import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool

mcp = FastMCP()

schema = {
    "type": "object",
    "properties": {
        "row": {"type": "array", "prefixItems": [{"type": "string"}], "items": {"type": "number"}}
    },
}


def get_row() -> dict:
    return {"row": ["a"]}


mcp.add_tool(Tool.from_function(get_row, output_schema=schema))


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("get_row", {})
        print("structured_content:", result.structured_content)
        print("data:", result.data)


asyncio.run(main())
