import asyncio, sys
from fastmcp import Client, Context, FastMCP
from fastmcp.client.transports import StdioTransport
from fastmcp.server import create_proxy

backend = FastMCP("backend")

@backend.tool
async def chatty(ctx: Context, pause: float = 0.0) -> str:
    for i in range(5):
        await ctx.log(f"message {i}", level="info")
    await asyncio.sleep(pause)
    return "done"

async def count(client_target, pause):
    got = []
    async def on_log(m): got.append(m.data)
    async with Client(client_target, log_handler=on_log) as c:
        await c.call_tool("chatty", {"pause": pause})
        await asyncio.sleep(0.5)
    return len(got)

async def main():
    st = lambda: StdioTransport(sys.executable, ["-I", __file__, "backend"])
    for pause in (0.0, 0.3):
        print(f"pause={pause}: direct stdio backend {await count(st(), pause)}/5; "
              f"in-memory proxy of stdio backend {await count(create_proxy(st()), pause)}/5; "
              f"in-memory proxy of in-memory backend {await count(create_proxy(backend), pause)}/5")

if __name__ == "__main__":
    if sys.argv[1:] == ["backend"]:
        backend.run(show_banner=False)
    else:
        asyncio.run(main())
