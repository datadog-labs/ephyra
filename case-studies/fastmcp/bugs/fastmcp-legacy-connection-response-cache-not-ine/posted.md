# Posted: https://github.com/PrefectHQ/fastmcp/issues/6298

Filed by alpaylan, 2026-11-03. Text as posted:

**`Client(mode="legacy", cache=CacheConfig(default_ttl_ms=...))` serves cached listings, although the cache is documented as inert on legacy connections**

### What happened?

I created a client with `mode="legacy"` and a `CacheConfig` that sets `default_ttl_ms`. On that legacy connection, a second `list_tools()` call is answered from the cache, so a tool added after the first listing does not show up. `list_resources()`, `list_resource_templates()` and `list_prompts()` behave the same way.

The docs (docs/clients/client.mdx, "Response caching") say "a cache is inert on a legacy connection", and the `Client` docstring for `cache` says "Honoring is modern-only, so a cache is inert on legacy connections". I expected the second listing to show both tools, as it does with `cache=None`, with `cache=True`, and with the same `CacheConfig` in the default mode.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio

from mcp.client.caching import CacheConfig

from fastmcp import Client, FastMCP

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
```

Output:

```
protocol: 2025-11-25
before: ['first']
after:  ['first']
```

Expected `after:  ['first', 'second']`.

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64-arm-64bit
```
