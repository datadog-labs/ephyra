# Repro: version-fallback-runs-mounted-server-mid

    ./run.sh /path/to/python-with-fastmcp     # runs mre.py; exit 1 = bug shows, 0 = it does not
    python poc_server_side.py                 # same setup, plain parent.get_prompt("greet") with no client

Bug present: `prompts/get greet -> error: Unknown prompt: 'greet'`
             (`parent.get_prompt('greet') -> None` for poc_server_side.py)
Bug absent:  `prompts/get greet -> hello from v1`

Controls (both rerun on trunk d07877e0):
- remove `parent.disable(tags={"beta"})`: prints `hello from v2`
- remove `middleware=[HideFromLists()]`: prints `hello from v1`

Results (rerun 2026-10-29, source trees on PYTHONPATH, Python 3.12.10):
- upstream main d07877e0:        Unknown prompt: 'greet'  (bug shows)
- v4.1.4:                        Unknown prompt: 'greet'  (bug shows)
- v3.4.11 (with mcp 1.26.0):     Unknown prompt: 'greet'  (bug shows)
- open PR #5838 head 39ac49dc:   Unknown prompt: 'greet'  (bug shows)
- main + loop fix 032:           hello from v1
