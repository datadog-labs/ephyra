# Repro: [[...]] multi-select elicitation items lack "type": "string"

- `mre.py` (fastmcp 4.x, in-memory): `python mre.py`. Prints the `items` schema for `[["red","blue"]]` and for
  `list[Literal["red","blue"]]`. Bug: the first line has no `"type": "string"`.
- `mre_3x.py`: the same for fastmcp 3.x (no `mode=`, reads `params.requestedSchema`).
- `server.py` + `ts_client.mjs`: the failure as a client sees it. Needs `npm install @modelcontextprotocol/sdk`
  (tested with 1.32.2) in the directory that holds ts_client.mjs, then run `node ts_client.mjs python server.py`. Bug: `isError: true` with a zod
  `invalid_union` error. Fixed: `action='accept' data=['red']`.

Checked on fastmcp main d07877e0, 4.1.4, 3.4.11 (all show the bug). With the candidate fix, both mre lines
have `"type": "string"` and the TS client call succeeds.
