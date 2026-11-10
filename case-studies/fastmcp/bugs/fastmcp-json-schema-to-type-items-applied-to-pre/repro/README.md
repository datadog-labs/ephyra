Run against any fastmcp install (4.x or 3.x) that has pydantic:

    python mre.py          # json_schema_to_type directly
    python mre_client.py   # same bug through Client.call_tool(...).data, in-memory transport

Exit/outcome on an affected build:
- mre.py raises `ValidationError ... list[float] ... unable to parse string as a number` (input 'a').
- mre_client.py logs `Error parsing structured content` and prints `data: None`.

On a fixed build mre.py prints `['a']` and mre_client.py prints `data: Root(row=['a'])`.
To run against a source tree without installing: `PYTHONPATH=<tree>/fastmcp_slim python mre.py`.
