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
