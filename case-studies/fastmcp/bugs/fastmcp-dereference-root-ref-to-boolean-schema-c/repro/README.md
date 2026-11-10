# Repro: root `$ref` to a boolean schema crashes tools/list

Run against an upstream fastmcp install or checkout:

    python mre.py         # expected: ['anything', 'ping']; on main/4.1.4/3.4.11: MCPError "Internal server error"
    python one_liner.py   # expected: {} ; on main/4.1.4/3.4.11: AssertionError

With a checkout and no install: `PYTHONPATH=<checkout>/fastmcp_slim python mre.py` (4.x layout).
