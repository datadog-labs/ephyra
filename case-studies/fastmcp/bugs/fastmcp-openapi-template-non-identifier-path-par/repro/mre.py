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
