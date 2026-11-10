# Posted: https://github.com/PrefectHQ/fastmcp/issues/6125

Filed by alpaylan, 2026-10-28. Text as posted:

**tools/list fails for the whole server when a tool's root $ref points to a boolean schema**

### What happened?

If one tool's parameters have a root `$ref` that points at a boolean schema, e.g. `{"type": "object", "$ref": "#/$defs/Any", "$defs": {"Any": true}}`, `tools/list` fails for the whole server with "Internal server error", so no tool can be listed. The server logs an `AssertionError` from `dereference_refs` (`assert isinstance(merged, dict)`). The same happens through `create_proxy` with a backend that has `dereference_schemas=False`.

`true` is a valid JSON Schema, so I expected the tool to be listed (or, at worst, only that tool to be affected).

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio

from fastmcp import Client, FastMCP
from fastmcp.tools import Tool


class AnyArgs(Tool):
    async def run(self, arguments):
        return "ok"


mcp = FastMCP()
mcp.add_tool(
    AnyArgs(
        name="anything",
        parameters={"type": "object", "$ref": "#/$defs/Any", "$defs": {"Any": True}},
    )
)


@mcp.tool
def ping() -> str:
    return "pong"


async def demo():
    async with Client(mcp) as client:
        tools = await client.list_tools()
        print([t.name for t in tools])


if __name__ == "__main__":
    asyncio.run(demo())
```

Output:

```
mcp.shared.exceptions.MCPError: Internal server error
(server log: AssertionError at 'assert isinstance(merged, dict)' in dereference_refs)
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
