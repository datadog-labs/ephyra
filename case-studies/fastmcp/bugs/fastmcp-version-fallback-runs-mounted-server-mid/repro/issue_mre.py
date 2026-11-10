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
