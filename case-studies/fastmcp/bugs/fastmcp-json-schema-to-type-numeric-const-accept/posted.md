# Posted: https://github.com/PrefectHQ/fastmcp/issues/6121

Filed by alpaylan, 2026-10-28. Text as posted:

**json_schema_to_type: const/enum accept true for 1, and an object-valued const in a union raises TypeError**

### What happened?

`json_schema_to_type` mishandles `const` and `enum` in two ways.

JSON `true` is accepted for `{"const": 1}` (and `{"enum": [1]}`) and comes back as `1`, because `True == 1` in Python. Through the client, a tool whose output schema has `{"anyOf": [{"const": 1}, {}]}` returns `{"flag": True}` as structured content but `result.data.flag == 1`.

An object- or array-valued `const` in a union, e.g. `{"anyOf": [{"const": {}}, {"const": 1}]}`, raises `TypeError: unhashable type: 'dict'`. Through the client this logs "Error parsing structured content" and `result.data` is `None`.

I expected `true` to be rejected for `const: 1` (JSON Schema treats booleans and numbers as different values), and the union to build a working type.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
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
```

Output:

```
1
TypeError: unhashable type: 'dict'
Root(flag=1)
None
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
