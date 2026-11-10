# repro

`./poc.sh <envoy-static> [oghttp2|nghttp2]`

Starts Envoy (HTTP/2 listener on 127.0.0.1:10000, admin 19901), a Python HTTP/1.1 upstream on 18080, and
sends `request.bin`: one HTTP/2 request whose `:path` comes after the regular header `x-a: b`.

Exit 1: the request was forwarded upstream (bug). Exit 0: not forwarded (expected).
`LOG_LEVEL=debug` also prints Envoy's http/http2/router debug lines.

Checked on 2026-10-28 against Envoy main 849c9283: oghttp2 exits 1, nghttp2 exits 0.
`request.bin` is written again by poc.sh from the printf in the script; the copy here is for inspection.
