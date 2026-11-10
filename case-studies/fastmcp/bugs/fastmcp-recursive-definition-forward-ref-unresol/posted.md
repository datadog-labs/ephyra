# Posted: https://github.com/PrefectHQ/fastmcp/issues/6135

Filed by alpaylan, 2026-10-28. Text as posted:

**Mutually recursive Pydantic return types are hydrated as the wrong class in result.data**

### What happened?

A tool that returns mutually recursive Pydantic models gets the wrong `result.data` on the client, with no error or warning. With `Person.pet: Pet | None` and `Pet.owner: Person | None`, the fourth level (`Max`, a `Pet`) is hydrated with `Person`'s fields: it gets `pet=None` and loses `owner`. `structured_content` is correct.

I expected `result.data` to have the same shape as `structured_content`. This also happens on 3.4.11, and the open PRs #5671 and #5779 (for #5669) don't change it.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from __future__ import annotations

import asyncio

from pydantic import BaseModel

from fastmcp import Client, FastMCP


class Person(BaseModel):
    name: str
    pet: Pet | None = None


class Pet(BaseModel):
    name: str
    owner: Person | None = None


Person.model_rebuild()
Pet.model_rebuild()

mcp = FastMCP()


@mcp.tool
def get_person() -> Person:
    return Person(name="Ann", pet=Pet(name="Rex", owner=Person(name="Bob", pet=Pet(name="Max"))))


async def main():
    async with Client(mcp) as client:
        result = await client.call_tool("get_person", {})
        print(result.structured_content)
        print(result.data)


asyncio.run(main())
```

Output:

```
{'name': 'Ann', 'pet': {'name': 'Rex', 'owner': {'name': 'Bob', 'pet': {'name': 'Max', 'owner': None}}}}
Root(name='Ann', pet=Root(name='Rex', owner=Root(name='Bob', pet=Root(name='Max', pet=None))))
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7 and on 3.4.11)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
