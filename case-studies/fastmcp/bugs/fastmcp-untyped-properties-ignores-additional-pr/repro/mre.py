import asyncio

from pydantic import TypeAdapter

from fastmcp import Client, FastMCP
from fastmcp.utilities.json_schema_type import json_schema_to_type

# 1. Direct: "type" is omitted, extra keys are explicitly allowed.
for schema in (
    {"properties": {}, "additionalProperties": True},
    {"type": "object", "properties": {}, "additionalProperties": True},
):
    ta = TypeAdapter(json_schema_to_type(schema))
    print(schema, "->", ta.dump_python(ta.validate_python({"a": 1})))

# 2. Through the client: a tool output schema without "type".
mcp = FastMCP()


@mcp.tool(
    output_schema={
        "properties": {"name": {"type": "string"}},
        "additionalProperties": True,
    }
)
def info() -> dict:
    return {"name": "x", "extra": 1}


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("info", {})
        print("structured_content:", result.structured_content)
        print("data:", result.data)


asyncio.run(main())
