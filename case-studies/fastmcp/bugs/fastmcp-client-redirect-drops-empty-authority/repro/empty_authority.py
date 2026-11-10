from pydantic import AnyUrl

from fastmcp.server.auth.redirect_validation import build_client_redirect, validate_redirect_uri

for uri in ["myapp://", "myapp:///callback", "myapp://callback"]:
    registered = str(AnyUrl(uri))  # what DCR stores
    print(f"registered {registered!r:22} allowed={validate_redirect_uri(AnyUrl(uri), None)}  "
          f"redirect -> {build_client_redirect(registered, {'code': 'abc'}, iss='https://as.example/')}")
