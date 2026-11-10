"""Offline repro: a local HTTP server records the query string FastMCP sends.

python standalone.py
  bug present: sent: q=false
  bug absent:  sent: q=1&q=false   (order may differ)
"""
import asyncio
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

try:
    import httpx2 as httpx  # fastmcp 4.x
except ImportError:
    import httpx  # fastmcp 3.x

from fastmcp import Client, FastMCP

seen = []


class H(BaseHTTPRequestHandler):
    def do_GET(self):
        seen.append(self.path)
        body = b"{}"
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


srv = HTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()

spec = {
    "openapi": "3.1.0",
    "info": {"title": "demo", "version": "1"},
    "paths": {
        "/items": {
            "get": {
                "operationId": "list_items",
                "parameters": [
                    # object query parameter, style=form, explode=true (the defaults)
                    {"name": "filter", "in": "query", "schema": {
                        "type": "object", "properties": {"q": {"type": "integer"}}}},
                    {"name": "q", "in": "query", "schema": {"type": "boolean"}},
                ],
                "responses": {"200": {"description": "ok"}},
            }
        }
    },
}


async def main():
    api = httpx.AsyncClient(base_url=f"http://127.0.0.1:{srv.server_port}")
    server = FastMCP.from_openapi(openapi_spec=spec, client=api)
    async with Client(server) as client:
        await client.call_tool("list_items", {"filter": {"q": 1}, "q": False})
    print("sent:", seen[-1].partition("?")[2])


asyncio.run(main())
