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
