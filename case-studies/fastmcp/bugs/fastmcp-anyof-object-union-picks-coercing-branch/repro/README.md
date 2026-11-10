# repro

`poc.py` converts an `anyOf` of two object schemas with `json_schema_to_type`
and validates `{"a": 0}`. Only the first branch accepts that value.

    PYTHONPATH=<fastmcp checkout>/fastmcp_slim python poc.py

Correct output (second line): `{'x': None}` (first branch; `a` is not one of its properties).
Bug: `{'a': False}` (second branch, with `0` coerced to `False`).

`poc_client.py` shows the same thing through `Client.call_tool(...).data` over the
in-memory transport: `structured_content` is `{'value': {'a': 0}}`, `.data` is
`Root(value=Root(a=False))`.

`run.sh <python> [<fastmcp_slim dir>]` exits 1 when the bug shows, 0 when it does not, 2 when undecided.
Pass the venv python path without resolving symlinks.
