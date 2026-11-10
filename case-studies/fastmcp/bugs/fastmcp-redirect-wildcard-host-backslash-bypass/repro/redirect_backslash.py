from fastmcp.server.auth.redirect_validation import (
    matches_allowed_pattern,
    validate_redirect_uri,
)

pattern = "https://*.example.com/*"
uri = "https://evil.com\\.example.com/cb"  # one backslash after evil.com

print("matches_allowed_pattern:", matches_allowed_pattern(uri, pattern))
print("validate_redirect_uri:  ", validate_redirect_uri(uri, [pattern]))

# Controls: the plain off-site URI is rejected, a real subdomain is accepted.
print("control evil.com/cb:     ", matches_allowed_pattern("https://evil.com/cb", pattern))
print("control app.example.com: ", matches_allowed_pattern("https://app.example.com/cb", pattern))
