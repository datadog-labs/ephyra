# Posted: https://github.com/PrefectHQ/fastmcp/issues/6295

Filed by alpaylan, 2026-11-03. Text as posted:

**Redirect URI helpers accept raw-string URIs that resolve to a different host or path (backslash, trailing space/C0, IPv4 shorthand, percent-encoded or IDNA hosts)**

### What happened?

The public helpers in `fastmcp.server.auth.redirect_validation` accept `str | AnyUrl`. When given a plain string, `validate_redirect_uri` (and `matches_allowed_pattern`) and `is_redirect_uri_allowed_for_application_type` return `True` for URIs that a URL parser following the WHATWG rules reads differently. Examples: `https://evil.com\.example.com/cb` passes the pattern `https://*.example.com/*`, `https://app.example.com/cb/..\steal` passes `https://app.example.com/cb/*`, `http://evil.com\.localhost/cb` passes as a loopback URI for a native client, and `https://127.1/cb` passes as a non-loopback URI for a web client. Passing the same URIs as `AnyUrl` gives `False` for all of them. `AnyUrl` normalizes them first (`\` becomes `/`, `..` is resolved, `127.1` becomes `127.0.0.1`, trailing control characters are removed), the same way the WHATWG URL Standard that browsers follow reads them.

I expected the string form to give the same result as the `AnyUrl` form. The built-in OAuthProxy DCR and authorize paths pass `AnyUrl` and reject all of these URIs (I checked over HTTP), so this only affects code that calls the helpers with strings. `CIMDFetcher.validate_redirect_uri(doc, redirect_uri: str)` also takes a string.

Found with an automated property-based tester. This report was written with AI assistance.

### Example Code

```Python
from pydantic import AnyUrl

from fastmcp.server.auth.redirect_validation import (
    is_redirect_uri_allowed_for_application_type,
    validate_redirect_uri,
)

# validate_redirect_uri: (uri, allowed pattern)
for uri, pattern in [
    ("https://evil.com\\.example.com/cb", "https://*.example.com/*"),
    ("https://app.example.com/cb/..\\steal", "https://app.example.com/cb/*"),
    ("https://app.example.com/cb/.. ", "https://app.example.com/cb/*"),
]:
    print(f"{uri!r:40} str={validate_redirect_uri(uri, [pattern])!s:5} "
          f"AnyUrl={validate_redirect_uri(AnyUrl(uri), [pattern])!s:5} AnyUrl(uri)={AnyUrl(uri)}")

# is_redirect_uri_allowed_for_application_type: (uri, application_type)
for uri, app_type in [
    ("http://evil.com\\.localhost/cb", "native"),
    ("https://127.1/cb", "web"),
    ("https://loc%61lhost/cb", "web"),
    ("https://a。localhost/cb", "web"),
    ("https://localhost\x00", "web"),
]:
    print(f"{app_type:6} {uri!r:33} str={is_redirect_uri_allowed_for_application_type(uri, app_type)!s:5} "
          f"AnyUrl={is_redirect_uri_allowed_for_application_type(AnyUrl(uri), app_type)!s:5} AnyUrl(uri)={AnyUrl(uri)}")
```

Output:

```
'https://evil.com\\.example.com/cb'      str=True  AnyUrl=False AnyUrl(uri)=https://evil.com/.example.com/cb
'https://app.example.com/cb/..\\steal'   str=True  AnyUrl=False AnyUrl(uri)=https://app.example.com/steal
'https://app.example.com/cb/.. '         str=True  AnyUrl=False AnyUrl(uri)=https://app.example.com/
native 'http://evil.com\\.localhost/cb'  str=True  AnyUrl=False AnyUrl(uri)=http://evil.com/.localhost/cb
web    'https://127.1/cb'                str=True  AnyUrl=False AnyUrl(uri)=https://127.0.0.1/cb
web    'https://loc%61lhost/cb'          str=True  AnyUrl=False AnyUrl(uri)=https://localhost/cb
web    'https://a。localhost/cb'          str=True  AnyUrl=False AnyUrl(uri)=https://a.localhost/cb
web    'https://localhost\x00'           str=True  AnyUrl=False AnyUrl(uri)=https://localhost/
```

### Version Information

```Text
FastMCP version:   4.1.6 (also reproduces on main at f32d524c)
MCP version:       2.3.0
Python version:    3.12.15
Platform:          macOS-26.7-arm64
```
