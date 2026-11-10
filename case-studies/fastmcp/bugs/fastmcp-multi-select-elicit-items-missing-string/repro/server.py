from fastmcp import Context, FastMCP

mcp = FastMCP("demo")


@mcp.tool
async def pick(ctx: Context) -> str:
    result = await ctx.elicit("Pick colors", response_type=[["red", "blue"]])
    return str(result)


if __name__ == "__main__":
    mcp.run()
