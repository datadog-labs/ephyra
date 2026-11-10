import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool

mcp = FastMCP()

# Only the first branch accepts {"a": 0}: the second requires "a" to be a boolean.
schema = {
    "type": "object",
    "properties": {
        "value": {
            "anyOf": [
                {"type": "object", "properties": {"x": {}}},
                {"type": "object", "properties": {"a": {"type": "boolean"}}},
            ]
        }
    },
}


def get_value() -> dict:
    return {"value": {"a": 0}}


mcp.add_tool(Tool.from_function(get_value, output_schema=schema))


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("get_value", {})
        print("structured_content:", result.structured_content)
        print("data:", result.data)


asyncio.run(main())
