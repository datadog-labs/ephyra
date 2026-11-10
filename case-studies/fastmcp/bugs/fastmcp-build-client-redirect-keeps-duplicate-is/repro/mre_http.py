import asyncio
from urllib.parse import parse_qs, urlparse

import httpx
from key_value.aio.stores.memory import MemoryStore
from starlette.applications import Starlette

from fastmcp.server.auth import OAuthProxy
from fastmcp.server.auth.providers.jwt import StaticTokenVerifier

proxy = OAuthProxy(
    upstream_authorization_endpoint="https://idp.example/authorize",
    upstream_token_endpoint="https://idp.example/token",
    upstream_client_id="upstream-id",
    upstream_client_secret="upstream-secret",
    token_verifier=StaticTokenVerifier(tokens={}),
    base_url="https://mcp.example",
    jwt_signing_key="a-long-test-secret",
    client_storage=MemoryStore(),
)
app = Starlette(routes=proxy.get_routes())

# A client registers a redirect_uri that already carries `iss` twice.
REDIRECT = "http://localhost:3000/cb?iss=a&iss=b"


async def main():
    async with httpx.AsyncClient(
        transport=httpx.ASGITransport(app=app), base_url="https://mcp.example"
    ) as http:
        reg = await http.post(
            "/register",
            json={"redirect_uris": [REDIRECT], "token_endpoint_auth_method": "none"},
        )
        r = await http.get(
            "/authorize",
            params={
                "response_type": "code",
                "client_id": reg.json()["client_id"],
                "redirect_uri": REDIRECT,
                "state": "xyz",
                "code_challenge": "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
                "code_challenge_method": "S256",
            },
        )
        txn_id = parse_qs(urlparse(r.headers["location"]).query)["txn_id"][0]

        # The upstream IdP sends the user back with an error.
        r = await http.get(f"/auth/callback?error=access_denied&state={txn_id}")
        location = r.headers["location"]
        print(location)
        iss = parse_qs(urlparse(location).query)["iss"]
        print("iss values:", iss)
        return len(iss) > 1


raise SystemExit(asyncio.run(main()))  # exit 1 = bug shows
