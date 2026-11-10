import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool

mcp = FastMCP()

schema = {
    "type": "object",
    "$defs": {
        "Node": {
            "title": "Node",
            "type": "object",
            "properties": {
                "wrapper": {
                    "type": "object",
                    "properties": {"node": {"$ref": "#/$defs/Node"}},
                }
            },
        }
    },
    "properties": {"node": {"$ref": "#/$defs/Node"}},
}


def tree() -> dict:
    return {"node": {"wrapper": {}}}


mcp.add_tool(Tool.from_function(tree, output_schema=schema))


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("tree", {})
        print("structured_content:", result.structured_content)
        print("data:", result.data)


asyncio.run(main())
