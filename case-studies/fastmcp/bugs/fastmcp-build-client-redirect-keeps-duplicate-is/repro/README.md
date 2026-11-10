# Repro: client redirect keeps a second `iss` from the registered redirect_uri

Needs FastMCP 4.x (main 1cce9a74 and v4.1.4 reproduce). 3.4.11 has no `iss` in client redirects and no `build_client_redirect`, so it is not affected.

    python mre.py        # the helper alone
    python mre_http.py   # OAuthProxy end to end over httpx.ASGITransport (no network)

Exit 1 = more than one `iss` in the client redirect (bug shows), 0 = exactly one.

Expected (one `iss`, the server's issuer):

    iss values: ['https://issuer.example/']
    iss values: ['https://mcp.example/']

Actual on main 1cce9a74 and v4.1.4:

    iss values: ['https://issuer.example/', 'b']
    iss values: ['https://mcp.example/', 'b']
