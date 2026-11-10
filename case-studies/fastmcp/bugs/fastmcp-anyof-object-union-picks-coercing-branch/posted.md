# Posted: https://github.com/PrefectHQ/fastmcp/issues/6136

Filed by alpaylan, 2026-10-28. Text as posted:

**result.data is built from an anyOf branch that coerces the value (0 becomes False)**

### What happened?

When a tool's output schema has an `anyOf` of object schemas, `result.data` can be built from a branch that only accepts the value after coercing it. With `anyOf: [{properties: {x: {}}}, {properties: {a: {type: boolean}}}]`, the value `{"a": 0}` is valid only for the first branch (`0` is not a boolean), but `result.data` comes from the second branch with `a` changed to `False`. `structured_content` still shows `{"a": 0}`.

I expected `result.data` to keep the value the server sent (`0`). The same happens with `json_schema_to_type` directly, and a string `"5"` is coerced to an integer the same way.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool

mcp = FastMCP()

# Only the first branch accepts {"a": 0}; the second requires "a" to be a boolean
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
```

Output:

```
structured_content: {'value': {'a': 0}}
data: Root(value=Root(a=False))
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7 and on 3.4.11)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
