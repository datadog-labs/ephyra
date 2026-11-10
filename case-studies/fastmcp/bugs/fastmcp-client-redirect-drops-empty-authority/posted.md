# Posted: https://github.com/PrefectHQ/fastmcp/issues/6296

Filed by alpaylan, 2026-11-03. Text as posted:

**OAuth client redirect turns a registered `myapp://` redirect_uri into `myapp:?...`**

### What happened?

A native client registered the redirect_uri `myapp://` (and, separately, `myapp:///callback`) with a default `OAuthProxy` (DCR accepts and stores both unchanged). When the proxy sent the client back after the upstream IdP returned `error=access_denied`, the Location header was `myapp:?error=access_denied&state=xyz&iss=...` and `myapp:/callback?error=access_denied&state=xyz&iss=...`, so the empty authority (`//`) was dropped. `build_client_redirect` called directly does the same (below). A redirect_uri with a host, such as `myapp://callback`, is not changed.

I expected the registered URI with the parameters added to its query (`myapp://?code=abc&iss=...`, `myapp:///callback?code=abc&iss=...`). RFC 6749 §4.1.2 says the parameters are added "to the query component of the redirection URI", and the `build_client_redirect` docstring says it returns "`url` with `params` appended".

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from pydantic import AnyUrl

from fastmcp.server.auth.redirect_validation import build_client_redirect, validate_redirect_uri

for uri in ["myapp://", "myapp:///callback", "myapp://callback"]:
    registered = str(AnyUrl(uri))  # what DCR stores
    print(f"registered {registered!r:22} allowed={validate_redirect_uri(AnyUrl(uri), None)}  "
          f"redirect -> {build_client_redirect(registered, {'code': 'abc'}, iss='https://as.example/')}")
```

Output:

```
registered 'myapp://'             allowed=True  redirect -> myapp:?code=abc&iss=https%3A%2F%2Fas.example%2F
registered 'myapp:///callback'    allowed=True  redirect -> myapp:/callback?code=abc&iss=https%3A%2F%2Fas.example%2F
registered 'myapp://callback'     allowed=True  redirect -> myapp://callback?code=abc&iss=https%3A%2F%2Fas.example%2F
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
