import asyncio

from pydantic import TypeAdapter

from fastmcp import Client, FastMCP
from fastmcp.utilities.json_schema_type import json_schema_to_type

# 1. JSON true is accepted for const 1 and comes back as 1
print(TypeAdapter(json_schema_to_type({"const": 1})).validate_python(True))

# 2. an object-valued const next to another const raises
try:
    json_schema_to_type({"anyOf": [{"const": {}}, {"const": 1}]})
except TypeError as e:
    print("TypeError:", e)

# 3. through the client
mcp = FastMCP()


@mcp.tool(output_schema={"type": "object", "properties": {"flag": {"anyOf": [{"const": 1}, {}]}}})
def check() -> dict:
    return {"flag": True}


@mcp.tool(output_schema={"type": "object", "properties": {"x": {"anyOf": [{"const": {}}, {"const": 1}]}}})
def get_x() -> dict:
    return {"x": 1}


async def main():
    async with Client(mcp) as client:
        print((await client.call_tool("check", {})).data)
        print((await client.call_tool("get_x", {})).data)


asyncio.run(main())
