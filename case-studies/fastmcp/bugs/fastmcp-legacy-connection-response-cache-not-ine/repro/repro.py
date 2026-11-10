import asyncio
from fastmcp import Client, FastMCP
from mcp.client.caching import CacheConfig

mcp = FastMCP("demo")
mcp.tool(lambda: 1, name="first")


async def main():
    cache = CacheConfig(target_id="demo", default_ttl_ms=60_000)
    async with Client(mcp, mode="legacy", cache=cache) as client:
        print("protocol:", client.session.protocol_version)
        print("before:", [t.name for t in await client.list_tools()])
        mcp.tool(lambda: 2, name="second")
        print("after: ", [t.name for t in await client.list_tools()])


asyncio.run(main())
