# Posted: https://github.com/PrefectHQ/fastmcp/issues/6297

Filed by alpaylan, 2026-11-03. Text as posted:

**OAuth client redirect carries two `iss` parameters when the registered redirect_uri already repeats `iss`**

### What happened?

A client registered the redirect_uri `http://localhost:3000/cb?iss=a&iss=b` with a default `OAuthProxy` (DCR). When the proxy sent the client back after the upstream IdP returned `error=access_denied`, the Location header had two `iss` values: the first was replaced with the server's issuer and the second (`b`) was kept, giving `http://localhost:3000/cb?iss=https%3A%2F%2Fmcp.example%2F&iss=b&error=access_denied&state=xyz`. The same thing happens when calling `build_client_redirect` directly (below).

I expected exactly one `iss`, the server's issuer. The `build_client_redirect` docstring says it returns "`url` with `params` appended and exactly one `iss` query parameter set to `iss`", and RFC 6749 §3.1 says response parameters "MUST NOT be included more than once". A redirect_uri with a single `iss` is handled correctly. Separate issue in the same function: #6296.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from urllib.parse import parse_qs, urlparse

from fastmcp.server.auth.redirect_validation import build_client_redirect

url = build_client_redirect(
    "https://client.example/cb?iss=a&iss=b",
    {"code": "abc", "state": "xyz"},
    iss="https://issuer.example/",
)
print(url)
print("iss values:", parse_qs(urlparse(url).query)["iss"])
```

Output:

```
https://client.example/cb?iss=https%3A%2F%2Fissuer.example%2F&iss=b&code=abc&state=xyz
iss values: ['https://issuer.example/', 'b']
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
