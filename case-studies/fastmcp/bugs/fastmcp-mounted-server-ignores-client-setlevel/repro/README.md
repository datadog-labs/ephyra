# Repro: mounted-server-ignores-client-setlevel

Run against an upstream fastmcp 4.x install:

    python mre.py        # in-memory, handshake-era client
    python mre_http.py   # stateful streamable HTTP (in-process ASGI), handshake-era client
    python mre_3x.py     # same as mre.py without mode="legacy", for fastmcp 3.x

Bug present: `mounted: ['info', 'error']` vs `direct:  ['error']`.
Bug absent:  both lines print `['error']`.

Seen on main 92c4c8d7 and 4.1.4 (both scripts). 3.4.11 prints `['error']` for both.
Run order matters in mre.py. stdio and in-memory connections to one server share one stored level, so the mounted case runs first.
