# Posted: https://github.com/PrefectHQ/fastmcp/issues/6301

Filed by alpaylan, 2026-11-03. Text as posted:

**Versionless lookup through a mount does not fall back to the enabled version when the child server has list-filtering middleware**

### What happened?

A parent server mounts a child that has prompt `greet` in versions 1 and 2, and the parent disables version 2 with `parent.disable(tags={"beta"})`. The child also has a middleware with an `on_list_prompts` hook that hides prompts from listings (`AuthMiddleware` filters lists the same way). `client.get_prompt("greet")` fails with `Unknown prompt: 'greet'`, and `await parent.get_prompt("greet")` returns `None`.

I expected version 1 to be served: the `get_prompt` docstring says "When the highest version is disabled and no explicit version was requested, falls back to the next-highest enabled version." Without the child middleware the same script prints `hello from v1`, and without the disable rule it prints `hello from v2`. Tools behave the same way (`Unknown tool`). Related but different triggers: #4950 (component-level auth) and #5837 (VersionFilter through a mount).

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio

from fastmcp import Client, FastMCP
from fastmcp.server.middleware import Middleware


class HideFromLists(Middleware):
    """Hides prompts from prompts/list; lookups are untouched."""

    async def on_list_prompts(self, context, call_next):
        return []


child = FastMCP("child", middleware=[HideFromLists()])


@child.prompt(version="1")
def greet() -> str:
    return "hello from v1"


@child.prompt(version="2", tags={"beta"})
def greet() -> str:
    return "hello from v2"


parent = FastMCP("parent")
parent.mount(child)
parent.disable(tags={"beta"})  # turn off v2, so v1 should be served


async def main():
    async with Client(parent) as client:
        try:
            result = await client.get_prompt("greet")
            print("prompts/get greet ->", result.messages[0].content.text)
        except Exception as e:
            print("prompts/get greet -> error:", e)


asyncio.run(main())
```

Output:

```
prompts/get greet -> error: Unknown prompt: 'greet'
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64-arm-64bit
```
