# Posted: https://github.com/PrefectHQ/fastmcp/issues/6302

Filed by alpaylan, 2026-11-03. Text as posted:

**A supporting file named `SKILL.md` in a subfolder is listed as a separate skill and makes `sync_skills()` fail**

### What happened?

Skill `a` ships a supporting file at `sub/SKILL.md` (for example a template in a skill-authoring skill). `Skill(...)` accepts it. Served with `SkillCatalogProvider`, or as a folder with `SkillsDirectoryProvider(root, experimental_skills=True)`, `list_skill_summaries()` reports a second skill `a/sub`, and `sync_skills()` raises and installs nothing, not even `a`. The catalog server fails with `Resource not found: 'skill://a/sub/_manifest'`, the folder server with `SKILL.md must begin with YAML frontmatter`.

I expected one skill, `a`, listed and installed with `sub/SKILL.md` as one of its files. The docs say "`sync_skills()` reads every skill a server lists and writes each one into a local folder". On 4.1.6 the folder variant also lists `a/sub`, but `sync_skills()` still installs `a` without an error; the `Skill` class is not in 4.1.6.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
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
```

Output:

```
catalog: listed ['a', 'a/sub'], sync_skills -> MCPError: Resource not found: 'skill://a/sub/_manifest'
folder: listed ['a', 'a/sub'], sync_skills -> ValueError: SKILL.md must begin with YAML frontmatter
```

### Version Information

```Text
FastMCP version:   main at f32d524c (4.2.0.dev0); 4.1.6 has no fastmcp.Skill
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64-arm-64bit
```
