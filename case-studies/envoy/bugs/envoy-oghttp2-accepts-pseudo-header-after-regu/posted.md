*Description*:

With `envoy.reloadable_features.http2_use_oghttp2: true`, Envoy accepts and forwards an HTTP/2 request in which a pseudo-header comes after a regular header (for example `:method :scheme :authority x-a :path`). RFC 9113 section 8.3 says such a request must be treated as malformed, and nghttp2 rejects it. The same happens in the response direction: an upstream response with `x-r: 1` before `:status: 200` is relayed to the client as a 200 with oghttp2, and becomes a 502 with nghttp2. `:method` last and `:path` after `content-length` behave the same.

The missing check appears to be in QUICHE: `HeaderValidator::ValidateSingleHeader` (`quiche/http2/adapter/header_validator.cc`) has no ordering check, both at the version Envoy pins (67bce2eb) and at QUICHE HEAD (7980e3d5, 2026-10-27). Duplicate pseudo-headers and a `host` that conflicts with `:authority` are still rejected in any position on default builds, and Envoy re-encodes the request from its header map, so the upstream receives the same well-formed request as for the correctly ordered one.

This was reported to envoy-security first; they agreed it is not a security issue and asked for a public issue. Tested on the v1.40.2 release and on main at 2cf5f51d (default build). Related: #10646. The UHV duplicate pseudo-header issue is #48808.

*Repro steps*:

Run the config below (default build) and send one raw HTTP/2 connection: client preface, empty SETTINGS, and one HEADERS frame (stream 1, END_STREAM|END_HEADERS) whose HPACK fields are, in order, `:method GET`, `:scheme http`, `:authority example.com`, `x-a b`, `:path /`.

```python
# mkreq.py: writes that connection's bytes to stdout (HPACK literal without indexing, no Huffman)
import sys
fields = [(":method","GET"),(":scheme","http"),(":authority","example.com"),("x-a","b"),(":path","/")]
blk = b"".join(b"\x00" + bytes([len(n)]) + n.encode() + bytes([len(v)]) + v.encode() for n, v in fields)
frame = len(blk).to_bytes(3, "big") + b"\x01\x05" + (1).to_bytes(4, "big") + blk
sys.stdout.buffer.write(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" + b"\x00\x00\x00\x04\x00\x00\x00\x00\x00" + frame)
```

```
python3 mkreq.py > late-path.bin
nc 127.0.0.1 31000 < late-path.bin
# oghttp2: 200, the HTTP/1.1 backend receives "GET / HTTP/1.1" with host: example.com, x-a: b
# nghttp2: rejected, nothing forwarded
# expected: rejected as malformed
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
