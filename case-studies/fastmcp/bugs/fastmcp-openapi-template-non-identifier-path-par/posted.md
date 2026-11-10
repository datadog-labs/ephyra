# Posted: https://github.com/PrefectHQ/fastmcp/issues/6150

Filed by alpaylan, 2026-10-29. Text as posted:

**from_openapi lists resource templates for non-identifier path parameters (e.g. {$top}) that can never be read**

### What happened?

With a route map that turns GET routes into resource templates, `FastMCP.from_openapi` lists a template for a path parameter whose name isn't a Python identifier, such as `{$top}` (also `x.y` or `1id`), but reading it always fails with "Resource not found". These names are legal in OpenAPI path templates. A name like `user-id` works, and the same route works when mapped to a tool.

I expected the template to be readable like `resource://plain/{top}`, or not to be listed.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
import asyncio

try:
    import httpx2 as httpx  # fastmcp 4.x
except ImportError:
    import httpx  # fastmcp 3.x

from fastmcp import Client, FastMCP
from fastmcp.server.providers.openapi import MCPType, RouteMap


def get_op(op_id, param):
    return {
        "get": {
            "operationId": op_id,
            "parameters": [
                {"name": param, "in": "path", "required": True,
                 "schema": {"type": "string"}}
            ],
            "responses": {"200": {"description": "ok"}},
        }
    }


spec = {
    "openapi": "3.0.3",
    "info": {"title": "t", "version": "1"},
    "paths": {
        "/a/{top}": get_op("plain", "top"),  # works
        "/b/{$top}": get_op("dollar", "$top"),  # legal OpenAPI name, fails
    },
}


def handler(request):
    return httpx.Response(200, json={"path": request.url.path})


client = httpx.AsyncClient(
    base_url="https://api.example.com", transport=httpx.MockTransport(handler)
)
# The documented route map for GET routes with path parameters.
route_maps = [RouteMap(methods=["GET"], mcp_type=MCPType.RESOURCE_TEMPLATE)]
mcp = FastMCP.from_openapi(spec, client=client, route_maps=route_maps)


async def main():
    async with Client(mcp) as c:
        for t in await c.list_resource_templates():
            print("template:", getattr(t, "uri_template", None) or t.uriTemplate)
        for uri in ["resource://plain/x", "resource://dollar/x"]:
            try:
                print(uri, "->", (await c.read_resource(uri))[0].text)
            except Exception as e:
                print(uri, "-> error:", e)


asyncio.run(main())
```

Output:

```
template: resource://plain/{top}
template: resource://dollar/{$top}
resource://plain/x -> {"path": "/a/x"}
resource://dollar/x -> error: Resource not found: 'resource://dollar/x'
```

### Version Information

```Text
FastMCP version:   4.1.4 (also reproduces on main at 92c4c8d7 and on 3.4.11)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
