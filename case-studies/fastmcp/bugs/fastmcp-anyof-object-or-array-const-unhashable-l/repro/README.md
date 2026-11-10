# Repro

Run against any fastmcp checkout or install (needs the `fastmcp` package importable):

    python mre.py          # one-liner: json_schema_to_type raises TypeError: unhashable type: 'dict'
    python mre_client.py   # same schema as a tool output_schema over the in-memory transport: result.data is None

From a source checkout without installing: `PYTHONPATH=<checkout>/fastmcp_slim python mre.py`.

Expected on a fixed build: mre.py exits 0 with no output; mre_client.py prints `data: Root(x=1)`.
Checked: trunk 92c4c8d7, v4.1.4 and 3.4.11 reproduce; the build with fix 004 does not.
