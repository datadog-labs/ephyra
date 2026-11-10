import asyncio
import tempfile
from pathlib import Path

from fastmcp import Client, FastMCP, Skill
from fastmcp.server.providers.skills import SkillsDirectoryProvider
from fastmcp.skills.provider import SkillCatalogProvider
from fastmcp.utilities.skills import list_skill_summaries, sync_skills

# Skill "a" ships a template file at sub/SKILL.md
skill = Skill(name="a", description="d", instructions="Hi", files={"sub/SKILL.md": "x"})
catalog = FastMCP("catalog", providers=[SkillCatalogProvider([skill])])

# The same skill as a folder
root = Path(tempfile.mkdtemp())
(root / "a" / "sub").mkdir(parents=True)
(root / "a" / "SKILL.md").write_text("---\nname: a\ndescription: d\n---\nHi\n")
(root / "a" / "sub" / "SKILL.md").write_text("x")
folder = FastMCP("folder", providers=[SkillsDirectoryProvider(root, experimental_skills=True)])


async def main():
    for server in (catalog, folder):
        async with Client(server) as client:
            names = [s.name for s in await list_skill_summaries(client)]
            try:
                await sync_skills(client, Path(tempfile.mkdtemp()))
                result = "ok"
            except Exception as e:
                result = f"{type(e).__name__}: {e}"
            print(f"{server.name}: listed {names}, sync_skills -> {result}")


asyncio.run(main())
