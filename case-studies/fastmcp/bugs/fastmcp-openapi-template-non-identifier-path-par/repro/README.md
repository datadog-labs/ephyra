# Repro: from_openapi resource template with a path parameter named `$top` is unreadable

Run with any Python that imports the fastmcp build under test (4.x uses httpx2, 3.x httpx):

    python mre.py

Bug present:
    template: resource://plain/{top}
    template: resource://dollar/{$top}
    resource://plain/x -> {"path": "/a/x"}
    resource://dollar/x -> error: Resource not found: 'resource://dollar/x'

Bug absent:
    template: resource://plain/{top}
    template: resource://dollar/{$top}
    resource://plain/x -> {"path": "/a/x"}
    resource://dollar/x -> {"path": "/b/x"}

Checked 2026-10-28: reproduces on main 92c4c8d7, v4.1.4, 3.4.11. Not on the patched build.
