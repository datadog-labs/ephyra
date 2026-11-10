# Posted: https://github.com/PrefectHQ/fastmcp/issues/6300

Filed by alpaylan, 2026-11-03. Text as posted:

**`ctx.elicit()` multi-select with `[[...]]` sends `items` without `"type": "string"`, which the 2025-11-25 schema requires**

### What happened?

The docs show multi-select elicitation as `response_type=[["bug", "feature", "documentation"]]`. For that form FastMCP sends the property schema `{"type": "array", "items": {"enum": [...]}}`. The MCP 2025-11-25 schema defines `UntitledMultiSelectEnumSchema.items` as `{ type: "string"; enum: string[] }` with `type` required, so clients that validate the request reject it (the official TypeScript SDK client fails the call with a zod `invalid_union` error, which is what #3731 saw in the Inspector). `response_type=list[Literal[...]]` already sends `"items": {"type": "string", "enum": [...]}`.

I expected the `[[...]]` form to send the same `items` schema as `list[Literal[...]]`. The example checks the received property against the SDK's own 2025-11-25 model.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio
from typing import Literal

from mcp_types._v2025_11_25 import UntitledMultiSelectEnumSchema

from fastmcp import Client, Context, FastMCP

mcp = FastMCP("demo")


@mcp.tool
async def nested_list(ctx: Context) -> str:
    return str(await ctx.elicit("Pick colors", response_type=[["red", "blue"]]))


@mcp.tool
async def list_literal(ctx: Context) -> str:
    return str(await ctx.elicit("Pick colors", response_type=list[Literal["red", "blue"]]))


async def on_elicit(message, response_type, params, context):
    prop = params.requested_schema["properties"]["value"]
    print("items:", prop["items"])
    try:
        UntitledMultiSelectEnumSchema.model_validate(prop)
        print("  valid UntitledMultiSelectEnumSchema (2025-11-25)")
    except Exception as e:
        print("  invalid:", str(e).splitlines()[1:3])
    return {"value": ["red"]}


async def main():
    async with Client(mcp, mode="legacy", elicitation_handler=on_elicit) as client:
        await client.call_tool("nested_list")
        await client.call_tool("list_literal")


asyncio.run(main())
```

Output:

```
items: {'enum': ['red', 'blue']}
  invalid: ['items.type', "  Field required [type=missing, input_value={'enum': ['red', 'blue']}, input_type=dict]"]
items: {'enum': ['red', 'blue'], 'type': 'string'}
  valid UntitledMultiSelectEnumSchema (2025-11-25)
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64-arm-64bit
```
