import asyncio

from pydantic import TypeAdapter

from fastmcp import Client, FastMCP
from fastmcp.utilities.json_schema_type import json_schema_to_type

# 1. Direct: JSON true is not equal to the number 1.
for schema in ({"const": 1}, {"enum": [1]}, {"anyOf": [{"const": 1}, {}]}):
    try:
        out = TypeAdapter(json_schema_to_type(schema)).validate_python(True)
        print(schema, "-> accepted, returned", repr(out))
    except Exception as e:
        print(schema, "-> rejected:", type(e).__name__)

# 2. Through the client: a tool whose output field is "1 or anything".
mcp = FastMCP()


@mcp.tool(
    output_schema={
        "type": "object",
        "properties": {"flag": {"anyOf": [{"const": 1}, {}]}},
    }
)
def check() -> dict:
    return {"flag": True}


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("check", {})
        print("structured_content:", result.structured_content)
        print("data:", result.data)


asyncio.run(main())
