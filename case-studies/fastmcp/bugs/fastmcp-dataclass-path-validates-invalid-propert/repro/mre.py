import asyncio

from pydantic import TypeAdapter

from fastmcp import Client, FastMCP
from fastmcp.utilities.json_schema_type import json_schema_to_type

# The same property, with and without additionalProperties: true.
prop = {"name": {"type": "string", "default": None}}
for schema in (
    {"type": "object", "properties": prop, "additionalProperties": True},
    {"type": "object", "properties": prop},
):
    try:
        TypeAdapter(json_schema_to_type(schema)).validate_python({})
        print(schema, "-> ok")
    except Exception as e:
        print(schema, "->", type(e).__name__, str(e).splitlines()[1:3])

# Through the client: a tool whose output schema has that property.
mcp = FastMCP()


@mcp.tool(output_schema={"type": "object", "properties": prop})
def info() -> dict:
    return {}


async def main():
    async with Client(mcp) as client:
        try:
            result = await client.call_tool("info", {})
            print("data:", result.data)
        except Exception as e:
            print("call_tool:", type(e).__name__, str(e).splitlines()[:3])


asyncio.run(main())
