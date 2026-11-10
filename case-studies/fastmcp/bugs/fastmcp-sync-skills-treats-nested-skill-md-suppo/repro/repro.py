import asyncio
import tempfile
from pathlib import Path

from fastmcp import Client, FastMCP, Skill
from fastmcp.skills.provider import SkillCatalogProvider
from fastmcp.utilities.skills import list_skill_summaries, sync_skills

skill = Skill(
    name="a",
    description="d",
    instructions="Hi",
    files={"sub/SKILL.md": "x"},
)
mcp = FastMCP("demo", providers=[SkillCatalogProvider([skill])])


async def demo():
    async with Client(mcp) as client:
        print([s.name for s in await list_skill_summaries(client)])
        print(await sync_skills(client, Path(tempfile.mkdtemp())))


if __name__ == "__main__":
    asyncio.run(demo())
