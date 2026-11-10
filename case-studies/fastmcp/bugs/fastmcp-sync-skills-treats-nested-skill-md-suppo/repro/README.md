Standalone PoC. Run `./repro.sh /path/to/python` where that python imports fastmcp (main or 4.2.0a1).
Or run `python repro.py` directly.
Bug present: prints `['a', 'a/sub']`, then `MCPError: Resource not found: 'skill://a/sub/_manifest'`.
Bug fixed: prints `['a']` and one installed path.
