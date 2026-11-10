import asyncio

import httpx

from fastmcp import Client, FastMCP

spec = {
    "openapi": "3.1.0",
    "info": {"title": "demo", "version": "1"},
    "paths": {
        "/items": {
            "get": {
                "operationId": "list_items",
                "parameters": [
                    # object, style=form, explode=true (the defaults)
                    {"name": "filter", "in": "query", "schema": {
                        "type": "object", "properties": {"q": {"type": "integer"}}}},
                    {"name": "q", "in": "query", "schema": {"type": "boolean"}},
                ],
                "responses": {"200": {"description": "ok"}},
            }
        }
    },
}


def handler(request: httpx.Request) -> httpx.Response:
    print("sent:", request.url.query.decode())
    return httpx.Response(200, json={})


async def main():
    api = httpx.AsyncClient(base_url="http://api", transport=httpx.MockTransport(handler))
    server = FastMCP.from_openapi(openapi_spec=spec, client=api)
    async with Client(server) as client:
        await client.call_tool("list_items", {"filter": {"q": 1}, "q": False})


asyncio.run(main())
