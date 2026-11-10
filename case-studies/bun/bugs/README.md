# Bugs: bun

The bugs the case studies found in bun. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [bun build lowering of using in a loop rethrows an earlier iteration's error](bun-lowered-using-error-leaks-across-loop-it/) | open | [link](https://github.com/oven-sh/bun/issues/48361) |
| [Bun.XML.parse drops a &#xA; at the end of an NMTOKEN attribute value](bun-xml-tokenized-attr-trims-charref-whitesp/) | open | [link](https://github.com/oven-sh/bun/issues/48401) |
