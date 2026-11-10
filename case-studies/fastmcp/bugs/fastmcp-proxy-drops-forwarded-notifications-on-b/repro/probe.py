import asyncio, sys
sys.path.insert(0, "/tmp/verify-fm/side")
from fastmcp.server.context import Context
from fastmcp import Client
from fastmcp.client.transports import StdioTransport
from fastmcp.server import create_proxy
import side

orig_log = Context.log
async def log(self, msg, *a, **k):
    try: rid = self.request_id
    except Exception as e: rid = f"<{type(e).__name__}: {e}>"
    try: sess = id(self.session)
    except Exception as e: sess = f"<{type(e).__name__}>"
    print(f"Context.log({msg!r}) session={type(self.session).__name__}:{sess} request_id={rid} origin={self.origin_request_id} server={self.fastmcp.name} min={self.fastmcp.client_log_level} per-session={dict(self.fastmcp._client_log_levels)}", file=sys.stderr)
    try:
        return await orig_log(self, msg, *a, **k)
    except BaseException as e:
        print("  raised", type(e).__name__, e, file=sys.stderr); raise
Context.log = log

import fastmcp.server.context as CX
orig_send = CX._log_to_server_and_client
async def send(**k):
    print("  _log_to_server_and_client level", k["level"], "min", k["min_level"], "rel", k["related_request_id"], file=sys.stderr)
    return await orig_send(**k)
CX._log_to_server_and_client = send

async def main():
    proxy = create_proxy(StdioTransport(sys.executable, ["-I", side.__file__, "backend"]))
    from fastmcp.server.middleware import Middleware
    class Show(Middleware):
        async def on_request(self, context, call_next):
            from fastmcp.server.dependencies import get_context
            try: rid = get_context().request_id
            except Exception as e: rid = type(e).__name__
            print("front request", context.method, "id", rid, file=sys.stderr)
            return await call_next(context)
    proxy.add_middleware(Show())
    got = []
    async def on_log(m): got.append(m.data)
    async with Client(proxy, log_handler=on_log) as c:
        await c.call_tool("chatty")
        await asyncio.sleep(0.5)
    print("got", len(got))
asyncio.run(main())
