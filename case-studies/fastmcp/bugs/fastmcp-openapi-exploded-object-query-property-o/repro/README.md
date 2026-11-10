# repro

`mre.py` builds a FastMCP server from a two-parameter OpenAPI document, calls the tool
over the in-memory transport, and prints the query string the server sent to the (mocked) API.

    pip install fastmcp        # or an editable install of upstream main
    python mre.py

- Bug present: `sent: q=false`
- Bug absent:  `sent: q=1&q=false`

On 4.x main a FastMCPDeprecationWarning about httpx.AsyncClient is printed; it is unrelated.

Checked (2026-10-28): upstream main 92c4c8d7, v4.1.4 (26bf9080) and 3.4.11 all print `sent: q=false`.
