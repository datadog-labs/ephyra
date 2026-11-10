from urllib.parse import parse_qs, urlparse

from fastmcp.server.auth.redirect_validation import build_client_redirect

url = build_client_redirect(
    "https://client.example/cb?iss=a&iss=b",
    {"code": "abc", "state": "xyz"},
    iss="https://issuer.example/",
)
print(url)
iss = parse_qs(urlparse(url).query)["iss"]
print("iss values:", iss)
raise SystemExit(len(iss) > 1)  # exit 1 = bug shows
