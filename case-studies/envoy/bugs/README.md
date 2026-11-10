# Bugs: envoy

The bugs the case studies found in envoy. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [oghttp2 accepts a pseudo-header after a regular header](envoy-oghttp2-accepts-pseudo-header-after-regu/) | open | [link](https://github.com/envoyproxy/envoy/issues/48807) |
| [UHV builds: oghttp2 accepts duplicate pseudo-headers and joins them (:path becomes /public,/admin)](envoy-uhv-oghttp2-joins-duplicate-pseudo-headers/) | open | [link](https://github.com/envoyproxy/envoy/issues/48808) |
