# repro

`poc.py` starts a local HTTP server that answers every POST with HTTP 403 and OpenAI's
`unsupported_country_region_territory` error body, then sends one chat request with the openai SDK and one
with `litellm.completion`. No network or real key needed.

    ./repro.sh /path/to/python-with-litellm   # exit 1: bug reproduces, exit 0: PermissionDeniedError raised

Results (2026-10-29):
- main 9a68573e9b6b134732e33c53eb40d68f1711aa72 (litellm 1.109.0, from source): exit 1, `APIError 403`
- PyPI litellm 1.106.6: exit 1, `APIError 403`
- pinned base + accepted fix: exit 0, `PermissionDeniedError 403`

Variant: with `"type": "invalid_request_error"` in the body, litellm raises `BadRequestError 403` instead
(main 9a68573e and PyPI 1.106.6).
